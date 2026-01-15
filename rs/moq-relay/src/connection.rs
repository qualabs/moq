use crate::{generate_connection_id, Auth, Cluster};

use moq_native::Request;
use std::path::PathBuf;
use tokio::sync::watch;

pub struct Connection {
	pub id: u64,
	pub request: Request,
	pub cluster: Cluster,
	pub auth: Auth,
	/// Optional qlog path for correlation with QUIC traces
	pub qlog_path: Option<PathBuf>,
}

impl Connection {
	#[tracing::instrument("conn", skip_all, fields(id = self.id, connection_id, qlog_path))]
	pub async fn run(self) -> anyhow::Result<()> {
		let connection_id = generate_connection_id(self.id);

		// Track connection start
		self.cluster.metrics.increment_connections();

		// Get current span and add connection_id and qlog_path attributes
		let span = tracing::Span::current();
		span.record("connection_id", &connection_id.as_str());
		
		// Record qlog_path if available for correlation with QUIC traces
		if let Some(ref qlog_path) = self.qlog_path {
			span.record("qlog_path", qlog_path.to_string_lossy().as_ref());
			tracing::debug!(connection_id = %connection_id, qlog_path = %qlog_path.display(), "Connection span with qlog correlation");
		} else {
			tracing::debug!(connection_id = %connection_id, "Connection span created and will be exported");
		}

		// Get the QUIC connection for stats polling (before accept consumes the request)
		let quic_conn = self.request.connection();

		// TODO: Extract W3C trace context from WebTransport CONNECT headers
		let (path, token) = match &self.request {
			Request::WebTransport(request) => {
				let path = request.url().path();
				let token = request
					.url()
					.query_pairs()
					.find(|(k, _)| k == "jwt")
					.map(|(_, v)| v.to_string());
				(path, token)
			}
			Request::Quic(_conn) => ("", None),
		};

		// Verify the URL before accepting the connection.
		let token = match self.auth.verify(path, token.as_deref()) {
			Ok(token) => token,
			Err(err) => {
				self.cluster.metrics.record_error();
				self.cluster.metrics.decrement_connections();
				let _ = self.request.reject(err.clone().into()).await;
				return Err(err.into());
			}
		};

		let publish = self.cluster.publisher(&token);
		let subscribe = self.cluster.subscriber(&token);

		// Track if this is a subscriber before moving subscribe
		let is_subscriber = subscribe.is_some();

		match (&publish, &subscribe) {
			(Some(publish), Some(subscribe)) => {
				tracing::info!(root = %token.root, publish = %publish.allowed().map(|p| p.as_str()).collect::<Vec<_>>().join(","), subscribe = %subscribe.allowed().map(|p| p.as_str()).collect::<Vec<_>>().join(","), "session accepted");
				self.cluster.metrics.increment_subscribers();
			}
			(Some(publish), None) => {
				tracing::info!(root = %token.root, publish = %publish.allowed().map(|p| p.as_str()).collect::<Vec<_>>().join(","), "publisher accepted");
			}
			(None, Some(subscribe)) => {
				tracing::info!(root = %token.root, subscribe = %subscribe.allowed().map(|p| p.as_str()).collect::<Vec<_>>().join(","), "subscriber accepted");
				self.cluster.metrics.increment_subscribers();
			}
			_ => anyhow::bail!("invalid session; no allowed paths"),
		}

		// Accept the connection.
		let stats: std::sync::Arc<dyn moq_native::moq_lite::Stats> =
			std::sync::Arc::new(self.cluster.metrics.clone());
		let session = self
			.request
			.accept_with_stats(subscribe, publish, Some(stats))
			.await?;

		// Track connection closure for metrics
		let metrics = self.cluster.metrics.clone();

		// Add session info to current span
		tracing::Span::current().record("session_started", true);

		// Create a channel to signal when the session closes
		let (close_tx, close_rx) = watch::channel(false);

		// Spawn QUIC stats polling task if we have a connection handle
		let stats_task = if let Some(conn) = quic_conn {
			let metrics_clone = metrics.clone();
			let mut close_rx = close_rx.clone();
			Some(tokio::spawn(async move {
				let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
				let mut last_bytes_sent = 0u64;
				let mut last_bytes_received = 0u64;

				loop {
					tokio::select! {
						_ = interval.tick() => {
							let stats = conn.stats();
							let path_stats = &stats.path;

							// Calculate packet loss ratio
							let total_packets = path_stats.lost_packets + path_stats.sent_packets;
							let packet_loss_ratio = if total_packets > 0 {
								path_stats.lost_packets as f64 / total_packets as f64
							} else {
								0.0
							};

							// Log stats for debugging
							tracing::trace!(
								rtt_ms = path_stats.rtt.as_millis(),
								bytes_sent = stats.udp_tx.bytes,
								bytes_received = stats.udp_rx.bytes,
								packet_loss = %format!("{:.2}%", packet_loss_ratio * 100.0),
								"QUIC connection stats"
							);

							// Track bytes delta
							let delta_sent = stats.udp_tx.bytes.saturating_sub(last_bytes_sent);
							let delta_received = stats.udp_rx.bytes.saturating_sub(last_bytes_received);

							if delta_sent > 0 {
								metrics_clone.record_bytes_sent(delta_sent);
								last_bytes_sent = stats.udp_tx.bytes;
							}
							if delta_received > 0 {
								metrics_clone.record_bytes_received(delta_received);
								last_bytes_received = stats.udp_rx.bytes;
							}
						}
						_ = close_rx.changed() => {
							// Session closed, stop polling
							break;
						}
					}
				}
			}))
		} else {
			None
		};

		// Wait until the session is closed.
		let result: anyhow::Result<()> = session.closed().await.map_err(Into::into);

		// Signal the stats task to stop
		let _ = close_tx.send(true);

		// Wait for stats task to finish
		if let Some(task) = stats_task {
			let _ = task.await;
		}

		// Track errors
		if result.is_err() {
			metrics.record_error();
		}

		// Decrement subscriber count
		if is_subscriber {
			tracing::debug!("Subscriber session closed");
			metrics.decrement_subscribers();
		}

		// Decrement active connections
		metrics.decrement_connections();

		result
	}
}
