// OpenTelemetry observability module
// Uses tracing-opentelemetry bridge for seamless integration

use opentelemetry::{
	global,
	metrics::{Counter, Histogram, Meter, UpDownCounter},
	KeyValue,
};
use opentelemetry_sdk::{
	propagation::TraceContextPropagator,
	trace,
	Resource,
};
use opentelemetry_otlp::WithExportConfig;

const SERVICE_NAME: &str = "service.name";

/// Initialize OpenTelemetry SDK with OTLP exporters
/// Sets up the global trace context propagator, meter provider, and logger provider
/// Note: Tracer provider initialization is handled by tracing-opentelemetry bridge in init_logging_with_otel()
pub fn init_otel(service_name: &str, otlp_endpoint: Option<&str>) -> anyhow::Result<()> {
	// Set up trace context propagator for W3C trace context support
	global::set_text_map_propagator(TraceContextPropagator::new());

	let endpoint = otlp_endpoint.unwrap_or("http://localhost:4317");
	let resource = Resource::new(vec![KeyValue::new(SERVICE_NAME, service_name.to_string())]);

	// Initialize meter provider with OTLP exporter for metrics
	// Use opentelemetry-otlp pipeline similar to tracing
	let meter_provider = opentelemetry_otlp::new_pipeline()
		.metrics(opentelemetry_sdk::runtime::Tokio)
		.with_exporter(
			opentelemetry_otlp::new_exporter()
				.tonic()
				.with_endpoint(endpoint),
		)
		.with_resource(resource.clone())
		.build()?;

	global::set_meter_provider(meter_provider);

	// Note: Logs are handled via tracing-subscriber JSON output
	// The OTel Collector can receive logs via filebeat or similar, or we can
	// implement OTLP logs export separately if needed.
	// For now, JSON logs are written to stderr and can be collected by log shippers.

	Ok(())
}

/// Get the global meter for creating metrics instruments
pub fn get_meter() -> Meter {
	global::meter("moq-relay")
}

/// Create RelayMetrics instance using the global meter
pub fn create_relay_metrics() -> RelayMetrics {
	RelayMetrics::new(&get_meter())
}

/// Start a background task to periodically export MetricsTracker values to OTel metrics
pub fn start_metrics_export_task(
	metrics_tracker: crate::MetricsTracker,
	relay_metrics: RelayMetrics,
	relay_instance: String,
	region: Option<String>,
	namespace: Option<String>,
) -> tokio::task::JoinHandle<()> {
	// Clone strings for the async task
	let region_str = region.unwrap_or_else(|| "unknown".to_string());
	let namespace_str = namespace.unwrap_or_else(|| "default".to_string());

	tokio::spawn(async move {
		let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
		let mut last_bytes_sent = 0u64;
		let mut last_bytes_received = 0u64;
		let mut last_app_bytes_sent = 0u64;
		let mut last_app_bytes_received = 0u64;
		let mut last_objects_sent = 0u64;
		let mut last_objects_received = 0u64;
		let mut last_groups_sent = 0u64;
		let mut last_groups_received = 0u64;
		let mut last_connections = 0u64;
		let mut last_errors = 0u64;
		let mut last_active_streams = 0i64;
		let mut last_active_subscribers = 0i64;
		let mut last_active_connections = 0i64;
		let mut last_active_sessions_ws = 0i64;
		let mut last_active_sessions_wt = 0i64;
		// Cache and dedup tracking
		let mut last_cache_hits = 0u64;
		let mut last_cache_misses = 0u64;
		let mut last_dedup_saved = 0u64;
		let mut last_drops = 0u64;
		let mut last_queue_depth = 0i64;
		let mut last_sessions_total_ws = 0u64;
		let mut last_sessions_total_wt = 0u64;
		let mut last_app_bytes_sent_ws = 0u64;
		let mut last_app_bytes_sent_wt = 0u64;
		let mut last_app_bytes_received_ws = 0u64;
		let mut last_app_bytes_received_wt = 0u64;

		loop {
			interval.tick().await;

			let labels = &[
				KeyValue::new("relay_instance", relay_instance.clone()),
				KeyValue::new("region", region_str.clone()),
				KeyValue::new("namespace", namespace_str.clone()),
			];

			// Export active streams (UpDownCounter - add delta from last value)
			let current_streams = metrics_tracker.active_streams() as i64;
			let delta_streams = current_streams - last_active_streams;
			if delta_streams != 0 {
				relay_metrics.active_streams.add(delta_streams as f64, labels);
				last_active_streams = current_streams;
			}

			// Export active subscribers (UpDownCounter - add delta from last value)
			let current_subscribers = metrics_tracker.active_subscribers() as i64;
			let delta_subscribers = current_subscribers - last_active_subscribers;
			if delta_subscribers != 0 {
				relay_metrics.active_subscribers.add(delta_subscribers as f64, labels);
				last_active_subscribers = current_subscribers;
			}

			// Export active connections (UpDownCounter - add delta)
			let current_active_conns = metrics_tracker.active_connections() as i64;
			let delta_active_conns = current_active_conns - last_active_connections;
			if delta_active_conns != 0 {
				relay_metrics.active_connections.add(delta_active_conns as f64, labels);
				last_active_connections = current_active_conns;
			}

			// Export active sessions by transport (UpDownCounter - add delta)
			let current_active_sessions_ws =
				metrics_tracker.active_sessions(crate::metrics::Transport::WebSocket) as i64;
			let delta_active_sessions_ws = current_active_sessions_ws - last_active_sessions_ws;
			if delta_active_sessions_ws != 0 {
				let labels_ws = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "websocket"),
				];
				relay_metrics
					.active_sessions_by_transport
					.add(delta_active_sessions_ws as f64, labels_ws);
				last_active_sessions_ws = current_active_sessions_ws;
			}

			let current_active_sessions_wt =
				metrics_tracker.active_sessions(crate::metrics::Transport::WebTransport) as i64;
			let delta_active_sessions_wt = current_active_sessions_wt - last_active_sessions_wt;
			if delta_active_sessions_wt != 0 {
				let labels_wt = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "webtransport"),
				];
				relay_metrics
					.active_sessions_by_transport
					.add(delta_active_sessions_wt as f64, labels_wt);
				last_active_sessions_wt = current_active_sessions_wt;
			}

			// Export total connections (Counter - add delta)
			let current_total_conns = metrics_tracker.total_connections();
			let delta_total_conns = current_total_conns.saturating_sub(last_connections);
			if delta_total_conns > 0 {
				relay_metrics.connections_total.add(delta_total_conns, labels);
				last_connections = current_total_conns;
			}

			// Export total sessions by transport (Counter - add delta)
			let current_sessions_total_ws =
				metrics_tracker.total_sessions(crate::metrics::Transport::WebSocket);
			let delta_sessions_total_ws = current_sessions_total_ws.saturating_sub(last_sessions_total_ws);
			if delta_sessions_total_ws > 0 {
				let labels_ws = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "websocket"),
				];
				relay_metrics
					.sessions_total_by_transport
					.add(delta_sessions_total_ws, labels_ws);
				last_sessions_total_ws = current_sessions_total_ws;
			}

			let current_sessions_total_wt =
				metrics_tracker.total_sessions(crate::metrics::Transport::WebTransport);
			let delta_sessions_total_wt = current_sessions_total_wt.saturating_sub(last_sessions_total_wt);
			if delta_sessions_total_wt > 0 {
				let labels_wt = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "webtransport"),
				];
				relay_metrics
					.sessions_total_by_transport
					.add(delta_sessions_total_wt, labels_wt);
				last_sessions_total_wt = current_sessions_total_wt;
			}

			// Export errors (Counter - add delta)
			let current_errors = metrics_tracker.total_errors();
			let delta_errors = current_errors.saturating_sub(last_errors);
			if delta_errors > 0 {
				relay_metrics.errors_total.add(delta_errors, labels);
				last_errors = current_errors;
			}

			// Export bytes sent/received (counters - track deltas)
			let current_bytes_sent = metrics_tracker.total_bytes_sent();
			let delta_sent = current_bytes_sent.saturating_sub(last_bytes_sent);
			if delta_sent > 0 {
				relay_metrics.bytes_sent_total.add(delta_sent, labels);
				last_bytes_sent = current_bytes_sent;
			}

			let current_bytes_received = metrics_tracker.total_bytes_received();
			let delta_received = current_bytes_received.saturating_sub(last_bytes_received);
			if delta_received > 0 {
				relay_metrics.bytes_received_total.add(delta_received, labels);
				last_bytes_received = current_bytes_received;
			}

			// Export application-level payload bytes (counters - track deltas)
			let current_app_bytes_sent = metrics_tracker.total_app_bytes_sent();
			let delta_app_sent = current_app_bytes_sent.saturating_sub(last_app_bytes_sent);
			if delta_app_sent > 0 {
				relay_metrics.app_bytes_sent_total.add(delta_app_sent, labels);
				last_app_bytes_sent = current_app_bytes_sent;
			}

			let current_app_bytes_received = metrics_tracker.total_app_bytes_received();
			let delta_app_received = current_app_bytes_received.saturating_sub(last_app_bytes_received);
			if delta_app_received > 0 {
				relay_metrics.app_bytes_received_total.add(delta_app_received, labels);
				last_app_bytes_received = current_app_bytes_received;
			}

			// Export application-level payload bytes by transport (counters - track deltas)
			let current_app_bytes_sent_ws =
				metrics_tracker.total_app_bytes_sent_by_transport(crate::metrics::Transport::WebSocket);
			let delta_app_bytes_sent_ws = current_app_bytes_sent_ws.saturating_sub(last_app_bytes_sent_ws);
			if delta_app_bytes_sent_ws > 0 {
				let labels_ws = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "websocket"),
				];
				relay_metrics
					.app_bytes_sent_total_by_transport
					.add(delta_app_bytes_sent_ws, labels_ws);
				last_app_bytes_sent_ws = current_app_bytes_sent_ws;
			}

			let current_app_bytes_sent_wt =
				metrics_tracker.total_app_bytes_sent_by_transport(crate::metrics::Transport::WebTransport);
			let delta_app_bytes_sent_wt = current_app_bytes_sent_wt.saturating_sub(last_app_bytes_sent_wt);
			if delta_app_bytes_sent_wt > 0 {
				let labels_wt = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "webtransport"),
				];
				relay_metrics
					.app_bytes_sent_total_by_transport
					.add(delta_app_bytes_sent_wt, labels_wt);
				last_app_bytes_sent_wt = current_app_bytes_sent_wt;
			}

			let current_app_bytes_received_ws =
				metrics_tracker.total_app_bytes_received_by_transport(crate::metrics::Transport::WebSocket);
			let delta_app_bytes_received_ws =
				current_app_bytes_received_ws.saturating_sub(last_app_bytes_received_ws);
			if delta_app_bytes_received_ws > 0 {
				let labels_ws = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "websocket"),
				];
				relay_metrics
					.app_bytes_received_total_by_transport
					.add(delta_app_bytes_received_ws, labels_ws);
				last_app_bytes_received_ws = current_app_bytes_received_ws;
			}

			let current_app_bytes_received_wt =
				metrics_tracker.total_app_bytes_received_by_transport(crate::metrics::Transport::WebTransport);
			let delta_app_bytes_received_wt =
				current_app_bytes_received_wt.saturating_sub(last_app_bytes_received_wt);
			if delta_app_bytes_received_wt > 0 {
				let labels_wt = &[
					KeyValue::new("relay_instance", relay_instance.clone()),
					KeyValue::new("region", region_str.clone()),
					KeyValue::new("namespace", namespace_str.clone()),
					KeyValue::new("transport", "webtransport"),
				];
				relay_metrics
					.app_bytes_received_total_by_transport
					.add(delta_app_bytes_received_wt, labels_wt);
				last_app_bytes_received_wt = current_app_bytes_received_wt;
			}

			// Export MoQ objects sent/received (counters - track deltas)
			let current_objects_sent = metrics_tracker.total_objects_sent();
			let delta_objects_sent = current_objects_sent.saturating_sub(last_objects_sent);
			if delta_objects_sent > 0 {
				relay_metrics.objects_sent_total.add(delta_objects_sent, labels);
				last_objects_sent = current_objects_sent;
			}

			let current_objects_received = metrics_tracker.total_objects_received();
			let delta_objects_received = current_objects_received.saturating_sub(last_objects_received);
			if delta_objects_received > 0 {
				relay_metrics.objects_received_total.add(delta_objects_received, labels);
				last_objects_received = current_objects_received;
			}

			// Export MoQ groups sent/received (counters - track deltas)
			let current_groups_sent = metrics_tracker.total_groups_sent();
			let delta_groups_sent = current_groups_sent.saturating_sub(last_groups_sent);
			if delta_groups_sent > 0 {
				relay_metrics.groups_sent_total.add(delta_groups_sent, labels);
				last_groups_sent = current_groups_sent;
			}

			let current_groups_received = metrics_tracker.total_groups_received();
			let delta_groups_received = current_groups_received.saturating_sub(last_groups_received);
			if delta_groups_received > 0 {
				relay_metrics.groups_received_total.add(delta_groups_received, labels);
				last_groups_received = current_groups_received;
			}

			// Export cache hits (Counter - add delta)
			let current_cache_hits = metrics_tracker.total_cache_hits();
			let delta_cache_hits = current_cache_hits.saturating_sub(last_cache_hits);
			if delta_cache_hits > 0 {
				relay_metrics.cache_hits_total.add(delta_cache_hits, labels);
				last_cache_hits = current_cache_hits;
			}

			// Export cache misses (Counter - add delta)
			let current_cache_misses = metrics_tracker.total_cache_misses();
			let delta_cache_misses = current_cache_misses.saturating_sub(last_cache_misses);
			if delta_cache_misses > 0 {
				relay_metrics.cache_misses_total.add(delta_cache_misses, labels);
				last_cache_misses = current_cache_misses;
			}

			// Export dedup savings (Counter - add delta)
			let current_dedup_saved = metrics_tracker.total_dedup_saved();
			let delta_dedup_saved = current_dedup_saved.saturating_sub(last_dedup_saved);
			if delta_dedup_saved > 0 {
				relay_metrics.dedup_upstream_saved_total.add(delta_dedup_saved, labels);
				last_dedup_saved = current_dedup_saved;
			}

			// Export drops (Counter - add delta)
			let current_drops = metrics_tracker.total_drops();
			let delta_drops = current_drops.saturating_sub(last_drops);
			if delta_drops > 0 {
				relay_metrics.drops_total.add(delta_drops, labels);
				last_drops = current_drops;
			}

			// Export queue depth (UpDownCounter - add delta)
			let current_queue_depth = metrics_tracker.queue_depth() as i64;
			let delta_queue_depth = current_queue_depth - last_queue_depth;
			if delta_queue_depth != 0 {
				relay_metrics.queue_depth.add(delta_queue_depth as f64, labels);
				last_queue_depth = current_queue_depth;
			}

			// Compute and record fanout (subscribers per stream)
			// This is a derived metric showing the relay's amplification factor
			if current_streams > 0 {
				let fanout = current_subscribers as f64 / current_streams as f64;
				relay_metrics.fanout.record(fanout, labels);
			}

			// Log metrics for debugging
			tracing::trace!(
				active_streams = current_streams,
				active_subscribers = current_subscribers,
				active_connections = current_active_conns,
				total_connections = current_total_conns,
				errors = current_errors,
				bytes_sent = current_bytes_sent,
				bytes_received = current_bytes_received,
				objects_sent = current_objects_sent,
				objects_received = current_objects_received,
				groups_sent = current_groups_sent,
				groups_received = current_groups_received,
				cache_hits = current_cache_hits,
				cache_misses = current_cache_misses,
				dedup_saved = current_dedup_saved,
				drops = current_drops,
				queue_depth = current_queue_depth,
				"Metrics exported"
			);
		}
	})
}

/// Initialize logging with optional OpenTelemetry layer
/// When OTel is enabled, adds tracing-opentelemetry layer and uses JSON formatting
pub fn init_logging_with_otel(log_config: &moq_native::Log, otel_enabled: bool) -> anyhow::Result<()> {
	use tracing_subscriber::layer::SubscriberExt;
	use tracing_subscriber::util::SubscriberInitExt;
	use tracing_subscriber::{EnvFilter, Layer};

	let filter = EnvFilter::builder()
		.with_default_directive(log_config.level().into())
		.from_env_lossy()
		.add_directive("h2=warn".parse().unwrap())
		.add_directive("quinn=info".parse().unwrap())
		// Note: Keep span logging off to reduce noise, but spans are still created and exported
		.add_directive("tracing::span=off".parse().unwrap())
		.add_directive("tracing::span::active=off".parse().unwrap())
		.add_directive("tokio=info".parse().unwrap())
		.add_directive("runtime=info".parse().unwrap());

	if otel_enabled {
		// Use JSON formatting when OTel is enabled for structured logs with trace_id/span_id
		let fmt_layer = tracing_subscriber::fmt::layer()
			.json()
			.with_writer(std::io::stderr)
			.with_filter(filter.clone());

		// Configure OTLP exporter for tracing-opentelemetry
		let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
			.unwrap_or_else(|_| "http://localhost:4317".to_string());

		tracing::debug!(endpoint = %otlp_endpoint, "Initializing OTLP tracer provider");

		// Create OTLP tracer provider
		// install_batch sets it as global and configures automatic batching with reasonable defaults
		// Batch configuration: max batch size 512, export timeout 30s, scheduled delay 5s
		opentelemetry_otlp::new_pipeline()
			.tracing()
			.with_exporter(
				opentelemetry_otlp::new_exporter()
					.tonic()
					.with_endpoint(&otlp_endpoint),
			)
			.with_trace_config(
				trace::Config::default().with_resource(Resource::new(vec![KeyValue::new(
					SERVICE_NAME,
					"moq-relay".to_string(),
				)])),
			)
			.install_batch(opentelemetry_sdk::runtime::Tokio)?;

		tracing::debug!("OTLP tracer provider installed successfully - spans will be batched and exported");

		// Add tracing-opentelemetry layer to export spans to OTel
		// The layer will automatically use the global tracer provider set by install_batch
		let otel_layer = tracing_opentelemetry::layer();

		#[cfg(feature = "tokio-console")]
		{
			let console_layer = console_subscriber::spawn();
			tracing_subscriber::registry()
				.with(fmt_layer)
				.with(otel_layer)
				.with(console_layer)
				.init();
		}

		#[cfg(not(feature = "tokio-console"))]
		{
			tracing_subscriber::registry()
				.with(fmt_layer)
				.with(otel_layer)
				.init();
		}
	} else {
		// Standard text formatting when OTel is disabled
		let fmt_layer = tracing_subscriber::fmt::layer()
			.with_writer(std::io::stderr)
			.with_filter(filter);

		#[cfg(feature = "tokio-console")]
		{
			let console_layer = console_subscriber::spawn();
			tracing_subscriber::registry()
				.with(fmt_layer)
				.with(console_layer)
				.init();
		}

		#[cfg(not(feature = "tokio-console"))]
		{
			tracing_subscriber::registry().with(fmt_layer).init();
		}
	}

	Ok(())
}

/// Observability metrics for MoQ relay
/// These metrics follow Prometheus naming conventions and low-cardinality labels
/// Note: Uses MoQ-native terminology (objects/groups) not media terminology (frames)
pub struct RelayMetrics {
	pub active_streams: UpDownCounter<f64>,
	pub active_subscribers: UpDownCounter<f64>,
	pub active_connections: UpDownCounter<f64>,
	pub connections_total: Counter<u64>,
	// Transport-split session metrics (WS vs WebTransport)
	pub active_sessions_by_transport: UpDownCounter<f64>,
	pub sessions_total_by_transport: Counter<u64>,
	// Transport-split application payload bytes (from moq-lite Stats hooks)
	pub app_bytes_sent_total_by_transport: Counter<u64>,
	pub app_bytes_received_total_by_transport: Counter<u64>,
	pub errors_total: Counter<u64>,
	pub bytes_sent_total: Counter<u64>,
	pub bytes_received_total: Counter<u64>,
	/// Application-level payload bytes (frame chunks). Ignores retransmissions.
	pub app_bytes_sent_total: Counter<u64>,
	/// Application-level payload bytes (frame chunks). Ignores retransmissions.
	pub app_bytes_received_total: Counter<u64>,
	// MoQ objects (individual data units within a group)
	pub objects_sent_total: Counter<u64>,
	pub objects_received_total: Counter<u64>,
	// MoQ groups (collections of objects, typically a GOP or similar)
	pub groups_sent_total: Counter<u64>,
	pub groups_received_total: Counter<u64>,
	// Cache and dedup metrics (relay's core value proposition)
	pub cache_hits_total: Counter<u64>,
	pub cache_misses_total: Counter<u64>,
	pub dedup_upstream_saved_total: Counter<u64>,
	pub drops_total: Counter<u64>,
	pub queue_depth: UpDownCounter<f64>,
	// Fanout: subscribers per group (computed as histogram for distribution)
	pub fanout: Histogram<f64>,
	pub publish_to_delivery_seconds: Histogram<f64>,
	pub quic_packet_loss_ratio: UpDownCounter<f64>,
	pub quic_rtt_seconds: Histogram<f64>,
}

impl RelayMetrics {
	pub fn new(meter: &Meter) -> Self {
		Self {
			active_streams: meter
				.f64_up_down_counter("moq_relay_active_streams")
				.with_description("Number of active broadcasts")
				.init(),
			active_subscribers: meter
				.f64_up_down_counter("moq_relay_active_subscribers")
				.with_description("Number of connected clients")
				.init(),
			active_connections: meter
				.f64_up_down_counter("moq_relay_active_connections")
				.with_description("Number of active WebTransport/QUIC connections")
				.init(),
			connections_total: meter
				.u64_counter("moq_relay_connections_total")
				.with_description("Total connections ever accepted")
				.init(),
			active_sessions_by_transport: meter
				.f64_up_down_counter("moq_relay_active_sessions_by_transport")
				.with_description("Number of active MoQ sessions by transport (websocket/webtransport)")
				.init(),
			sessions_total_by_transport: meter
				.u64_counter("moq_relay_sessions_total_by_transport")
				.with_description("Total MoQ sessions accepted by transport (websocket/webtransport)")
				.init(),
			app_bytes_sent_total_by_transport: meter
				.u64_counter("moq_relay_app_bytes_sent_total_by_transport")
				.with_description("Application payload bytes sent by transport (excludes retransmits)")
				.init(),
			app_bytes_received_total_by_transport: meter
				.u64_counter("moq_relay_app_bytes_received_total_by_transport")
				.with_description("Application payload bytes received by transport (excludes retransmits)")
				.init(),
			errors_total: meter
				.u64_counter("moq_relay_errors_total")
				.with_description("Total connection errors")
				.init(),
			bytes_sent_total: meter
				.u64_counter("moq_relay_bytes_sent_total")
				.with_description("Total bytes transmitted")
				.init(),
			bytes_received_total: meter
				.u64_counter("moq_relay_bytes_received_total")
				.with_description("Total bytes received")
				.init(),
			app_bytes_sent_total: meter
				.u64_counter("moq_relay_app_bytes_sent_total")
				.with_description("Application-level payload bytes sent (frame chunks; excludes retransmissions)")
				.init(),
			app_bytes_received_total: meter
				.u64_counter("moq_relay_app_bytes_received_total")
				.with_description("Application-level payload bytes received (frame chunks; excludes retransmissions)")
				.init(),
			objects_sent_total: meter
				.u64_counter("moq_relay_objects_sent_total")
				.with_description("Total MoQ objects transmitted")
				.init(),
			objects_received_total: meter
				.u64_counter("moq_relay_objects_received_total")
				.with_description("Total MoQ objects received")
				.init(),
			groups_sent_total: meter
				.u64_counter("moq_relay_groups_sent_total")
				.with_description("Total MoQ groups transmitted")
				.init(),
			groups_received_total: meter
				.u64_counter("moq_relay_groups_received_total")
				.with_description("Total MoQ groups received")
				.init(),
			cache_hits_total: meter
				.u64_counter("moq_relay_cache_hits_total")
				.with_description("Objects served from cache")
				.init(),
			cache_misses_total: meter
				.u64_counter("moq_relay_cache_misses_total")
				.with_description("Objects fetched from upstream (cache miss)")
				.init(),
			dedup_upstream_saved_total: meter
				.u64_counter("moq_relay_dedup_upstream_saved_total")
				.with_description("Upstream fetches avoided due to subscription deduplication")
				.init(),
			drops_total: meter
				.u64_counter("moq_relay_drops_total")
				.with_description("Objects dropped due to backpressure or queue overflow")
				.init(),
			queue_depth: meter
				.f64_up_down_counter("moq_relay_queue_depth")
				.with_description("Current number of objects pending delivery")
				.init(),
			fanout: meter
				.f64_histogram("moq_relay_fanout")
				.with_description("Number of subscribers per published group")
				.init(),
			publish_to_delivery_seconds: meter
				.f64_histogram("moq_relay_publish_to_delivery_seconds")
				.with_description("Time from publish to delivery in seconds")
				.init(),
			quic_packet_loss_ratio: meter
				.f64_up_down_counter("moq_relay_quic_packet_loss_ratio")
				.with_description("QUIC packet loss percentage (0.0-1.0)")
				.init(),
			quic_rtt_seconds: meter
				.f64_histogram("moq_relay_quic_rtt_seconds")
				.with_description("QUIC round-trip time in seconds")
				.init(),
		}
	}
}

/// Generate a stable connection ID
pub fn generate_connection_id(conn_id: u64) -> String {
	format!("conn-{}", conn_id)
}

/// Extract W3C trace context from headers (when available)
/// Note: This is a placeholder - actual implementation depends on web-transport-quinn API
pub fn extract_trace_context(_headers: &[(&str, &str)]) -> Option<opentelemetry::trace::SpanContext> {
	// TODO: Extract traceparent header and parse into SpanContext
	// For now, return None - this will be implemented when header access is available
	None
}
