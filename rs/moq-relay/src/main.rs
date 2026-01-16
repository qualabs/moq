//! MoQ relay server connecting publishers to subscribers.
//!
//! Content-agnostic relay that works with any live data, not just media.
//!
//! Features:
//! - Clustering: connect multiple relays for global distribution
//! - Authentication: JWT-based access control via [`moq_token`]
//! - WebSocket fallback: for restrictive networks
//! - HTTP API: health checks and metrics via [`Web`]

mod auth;
mod cluster;
mod config;
mod connection;
mod metrics;
mod observability;
mod observability_config;
mod web;

pub use auth::*;
pub use cluster::*;
pub use config::*;
pub use connection::*;
pub use metrics::*;
pub use observability::*;
pub use observability_config::*;
pub use web::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// TODO: It would be nice to remove this and rely on feature flags only.
	// However, some dependency is pulling in `ring` and I don't know why, so meh for now.
	rustls::crypto::aws_lc_rs::default_provider()
		.install_default()
		.expect("failed to install default crypto provider");

	// Load config without initializing logging yet (we need to add OTel layer first)
	let config = Config::load_with_init(false)?;

	// Initialize OpenTelemetry (enabled by default with http://localhost:4317)
	// Can be disabled by setting otlp_endpoint to empty string or "disabled"
	let otlp_endpoint = config.observability.otlp_endpoint.trim();
	let otel_enabled = !otlp_endpoint.is_empty() && otlp_endpoint != "disabled";
	if otel_enabled {
		if let Err(e) = observability::init_otel("moq-relay", Some(otlp_endpoint)) {
			eprintln!("Warning: Failed to initialize OpenTelemetry: {}", e);
		}
	}

	// Initialize logging with optional OTel layer
	observability::init_logging_with_otel(&config.log, otel_enabled)?;

	if otel_enabled {
		tracing::info!("OpenTelemetry initialized with OTLP endpoint");
		
		// Create a test span to verify trace export is working
		let test_span = tracing::info_span!("otel_init_test", test = true);
		test_span.in_scope(|| {
			tracing::info!("Test span created - this should appear in Tempo if export is working");
		});
		tracing::debug!("Test span should be exported to OTel Collector");
	}

	let addr = config.server.bind.unwrap_or("[::]:443".parse().unwrap());
	let mut server = config.server.init()?;

	#[allow(unused_mut)]
	let mut client = config.client.init()?;

	#[cfg(feature = "iroh")]
	{
		let iroh = config.iroh.bind().await?;
		server.with_iroh(iroh.clone());
		client.with_iroh(iroh);
	}

	let auth = config.auth.init()?;

	let cluster = Cluster::new(config.cluster, client);
	let cloned = cluster.clone();
	tokio::spawn(async move { cloned.run().await.expect("cluster failed") });

	// Start metrics export task if OTel is enabled
	if otel_enabled {
		let relay_metrics = observability::create_relay_metrics();
		let metrics_tracker = cluster.metrics.clone();
		observability::start_metrics_export_task(
			metrics_tracker,
			relay_metrics,
			"relay-1".to_string(),
			None, // region
			None, // namespace
		);
		tracing::info!("Metrics export task started");
	}

	// Create a web server too.
	let web = Web::new(
		WebState {
			auth: auth.clone(),
			cluster: cluster.clone(),
			tls_info: server.tls_info(),
			conn_id: Default::default(),
		},
		config.web,
	);

	tokio::spawn(async move {
		web.run().await.expect("failed to run web server");
	});

	tracing::info!(%addr, "listening");

	#[cfg(unix)]
	// Notify systemd that we're ready after all initialization is complete
	let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

	let mut conn_id = 0;

	while let Some(request) = server.accept().await {
		// Generate qlog path if qlog is enabled and should be logged for this connection
		let qlog_config = server.qlog_config();
		let qlog_path = if qlog_config.should_log() {
			let connection_id = crate::generate_connection_id(conn_id);
			Some(qlog_config.file_path(&connection_id))
		} else {
			None
		};

		let conn = Connection {
			id: conn_id,
			request,
			cluster: cluster.clone(),
			auth: auth.clone(),
			qlog_path,
		};

		conn_id += 1;
		tokio::spawn(async move {
			let err = conn.run().await;
			if let Err(err) = err {
				tracing::warn!(%err, "connection closed");
			}
		});
	}

	Ok(())
}
