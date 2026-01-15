# MoQ Observability - Implementation Status

Last updated: January 2026

## ✅ Completed

### Infrastructure
- [x] Docker Compose stack with all services
- [x] OpenTelemetry Collector configured
- [x] Prometheus for metrics storage
- [x] Tempo for trace storage
- [x] Loki for log storage
- [x] Grafana with provisioned datasources
- [x] Grafana Alloy for Docker log collection
- [x] Dashboard import script

### Relay Instrumentation (Rust)
- [x] OpenTelemetry SDK integration
- [x] Active subscribers metric (`moq_relay_active_subscribers`)
- [x] Active streams metric (`moq_relay_active_streams`)
- [x] Active connections metric (`moq_relay_active_connections`)
- [x] Total connections counter (`moq_relay_connections_total`)
- [x] Bytes sent/received counters
- [x] Frames sent/received counters
- [x] Error counter (`moq_relay_errors_total`)
- [x] Metrics export via OTLP/gRPC

### Client Instrumentation (Browser/TypeScript)
- [x] OpenTelemetry JS SDK integration
- [x] Buffer length histogram (`moq_client_buffer_length_seconds`)
- [x] Startup time histogram (`moq_client_startup_time_seconds`)
- [x] Quality switches counter (`moq_client_quality_switches_total`)
- [x] Connection type tracking (`moq_client_connections_total`)
- [x] Rebuffer counter (`moq_client_rebuffer_count_total`)
- [x] Auto-initialization when connecting to relay
- [x] CORS configuration for browser → collector

### Dashboards
- [x] MoQ Overview - main operational dashboard
- [x] MoQ Pipeline - detailed metrics
- [x] Node Exporter Full - system metrics
- [x] OpenTelemetry Collector health

## 🔄 Partial / In Progress

### QUIC-specific Metrics
- [ ] RTT histogram - *blocked on WebTransport support*
- [ ] Packet loss ratio - *blocked on WebTransport support*
- [x] Connection stats polling infrastructure (ready for when WebTransport works)

### Traces
- [x] Basic trace provider setup
- [ ] End-to-end trace correlation (publisher → relay → subscriber)
- [ ] Span links for multi-hop traces

### Logs
- [x] Alloy configured for Docker log scraping
- [ ] Structured JSON logging in relay with trace_id injection
- [ ] Log ↔ trace correlation in Grafana

## ❌ Not Yet Implemented

### Alerting
- [ ] Alert rules for critical conditions
- [ ] Grafana alerting configuration
- [ ] Alert notification channels

### Per-Broadcast Metrics
- [ ] Stream/broadcast labels on metrics
- [ ] Per-broadcast dashboard views

### Geographic Distribution
- [ ] Region labels on client metrics
- [ ] Geographic heatmap dashboard

### Advanced Features
- [ ] qlog integration for QUIC deep debugging
- [ ] Recording rules for derived metrics
- [ ] Long-term storage configuration
- [ ] Multi-relay aggregation

## Architecture Decisions Made

| Decision | Rationale |
|----------|-----------|
| No session_id in Prometheus | High cardinality - use traces/logs instead |
| OpenTelemetry over custom | Vendor-neutral, standard protocol |
| Grafana stack (Prometheus/Tempo/Loki) | Unified ecosystem, proven at scale |
| OTLP for all telemetry | Single protocol, flexible routing |
| 10s export interval | Balance between latency and overhead |

## Known Limitations

1. **WebSocket fallback**: Most browsers currently fall back to WebSocket due to WebTransport configuration. QUIC-specific metrics only work with WebTransport.

2. **No per-session metrics**: By design, to avoid Prometheus cardinality issues. Use Tempo traces for per-session debugging.

3. **Local development only**: Current CORS config allows localhost. Production needs proper origin configuration.

4. **No authentication**: Grafana runs with anonymous admin access for development. Production needs proper auth.

## Files Modified

### Rust (Relay)
- `rs/moq-relay/src/observability.rs` - OTel SDK init and metrics export
- `rs/moq-relay/src/metrics.rs` - MetricsTracker with atomic counters
- `rs/moq-relay/src/connection.rs` - Connection lifecycle instrumentation
- `rs/moq-relay/src/main.rs` - OTel initialization on startup

### TypeScript (Client)
- `js/hang/src/observability/index.ts` - OTel browser SDK and metrics
- `js/hang/src/watch/element.ts` - Auto-init observability
- `js/lite/src/connection/connect.ts` - Transport type tracking

### Infrastructure
- `observability/docker-compose.yml` - All services
- `observability/otel-collector-config.yaml` - Collector pipelines
- `observability/prometheus.yml` - Scrape config
- `observability/grafana/dashboards/*.json` - Dashboard definitions
