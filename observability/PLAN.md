# Observability Implementation Plan

## Goal

Production-ready live stream analyzer with:
- Real-time monitoring of all active users
- Aggregated metrics (latency, rebuffers, bandwidth)
- Debugging capabilities (logs + traces with correlation)
- Performance monitoring and alerting

## Phase 1: Data Collection (~4 hours)

### 1.1 Instrument Bytes/Bandwidth in moq-lite (1.5h)

**Location**: `rs/moq-lite/src/` (session handling)

**Metrics to add**:
```rust
moq_relay_bytes_sent_total{broadcast, track_type, direction}
moq_relay_bytes_received_total{broadcast, track_type, direction}
```

**Where to instrument**:
- [ ] Find where frames/groups are sent to subscribers
- [ ] Find where frames/groups are received from publishers
- [ ] Call `metrics.record_bytes_sent(bytes)` at send points
- [ ] Call `metrics.record_bytes_received(bytes)` at receive points

**Labels** (low cardinality):
- `broadcast`: namespace/name (e.g., "anon/bbb")
- `track_type`: "video" | "audio" | "catalog"
- `direction`: "publish" | "subscribe"

### 1.2 Add Stream Labels to Existing Metrics (0.5h)

**Current**: 
```rust
moq_relay_active_streams  // no labels
moq_relay_active_subscribers  // no labels
```

**Target**:
```rust
moq_relay_active_streams{namespace}
moq_relay_active_subscribers{namespace, broadcast}
```

**Location**: `rs/moq-relay/src/connection.rs`

- [ ] Pass broadcast name to metrics increment/decrement
- [ ] Update `MetricsTracker` to accept labels
- [ ] Update `RelayMetrics` OTel instruments with attributes

### 1.3 Get Client Metrics Flowing (1h)

**Test in browser**:
- [ ] Open http://localhost:5173 with DevTools
- [ ] Check console for `[Observability] Initialized`
- [ ] Play video, check for CORS errors
- [ ] Verify metrics appear in Prometheus:
  ```bash
  curl 'http://localhost:9090/api/v1/query?query=moq_client_buffer_length_seconds'
  ```

**If CORS errors**:
- [ ] Verify OTel Collector CORS config allows browser origin
- [ ] Test with `curl` to rule out collector issues

**Metrics expected**:
```promql
moq_client_startup_time_seconds
moq_client_buffer_length_seconds
moq_client_rebuffer_count_total
moq_client_quality_switches_total
```

### 1.4 Enable Log Collection (1h)

**Option A: Dockerize relay for dev** (Recommended)
- [ ] Create `dev` service in docker-compose.yml
- [ ] Mount source code for hot reload
- [ ] Update `just dev` to use Docker
- [ ] Alloy automatically collects logs

**Option B: OTLP log export from Rust**
- [ ] Add `tracing-opentelemetry` log bridge
- [ ] Configure OTLP log exporter
- [ ] More code, but works without Docker

**Verification**:
- [ ] Query Loki: `{service_name="moq-relay"}`
- [ ] Verify `trace_id` appears in log lines

## Phase 2: Visualization (~3 hours)

### 2.1 Production Dashboard (2h)

**Panels to create**:

**Row 1: Overview**
- Total active streams (stat)
- Total viewers (stat)  
- Bandwidth in/out (graph)
- System health (gauge)

**Row 2: Per-Stream Table**
- Broadcast name
- Viewer count
- Bandwidth
- Avg latency
- Health status

**Row 3: Client Health**
- Startup time histogram
- Buffer length distribution
- Rebuffer rate over time
- Quality switch frequency

**Row 4: Latency**
- P50/P95/P99 latency graph
- Latency heatmap by stream

### 2.2 Debug Views (1h)

- [ ] Log panel with trace_id filter
- [ ] Link from log → trace view
- [ ] Per-session detail view (if trace_id known)

## Phase 3: Alerting (~2 hours)

### 3.1 Define Alert Rules (1h)

**File**: `observability/prometheus-alerts.yml`

```yaml
groups:
  - name: moq-alerts
    rules:
      # High latency
      - alert: HighClientLatency
        expr: histogram_quantile(0.95, rate(moq_client_latency_seconds_bucket[5m])) > 0.5
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "High client latency detected"
          
      # Rebuffer spike
      - alert: HighRebufferRate
        expr: rate(moq_client_rebuffer_count_total[5m]) > 0.1
        for: 1m
        labels:
          severity: critical
          
      # No active streams (if expected)
      - alert: NoActiveStreams
        expr: moq_relay_active_streams == 0
        for: 5m
        labels:
          severity: warning
          
      # High CPU
      - alert: HighCPUUsage
        expr: 100 - (avg(irate(node_cpu_seconds_total{mode="idle"}[5m])) * 100) > 80
        for: 5m
        labels:
          severity: warning
```

### 3.2 Configure Grafana Alerting (0.5h)

- [ ] Set up notification channel (email, Slack, etc.)
- [ ] Import alert rules to Grafana
- [ ] Test with simulated conditions

### 3.3 Documentation (0.5h)

- [ ] Document alert meanings
- [ ] Create runbook for each alert
- [ ] Document escalation procedures

## Success Criteria

After all phases:

- [ ] Can see real-time viewer count per stream
- [ ] Can see bandwidth usage per stream
- [ ] Can see P50/P95/P99 latency from clients
- [ ] Can search logs by trace_id
- [ ] Can trace a session from client to relay
- [ ] Alerts fire for high latency, rebuffers
- [ ] Dashboard loads in <2s

## Files to Modify

**Phase 1**:
- `rs/moq-lite/src/session/` - bytes instrumentation
- `rs/moq-relay/src/connection.rs` - stream labels
- `rs/moq-relay/src/metrics.rs` - label support
- `rs/moq-relay/src/observability.rs` - OTel attributes
- `observability/docker-compose.yml` - relay service (if Option A)

**Phase 2**:
- `observability/grafana/dashboards/moq-pipeline.json` - new panels

**Phase 3**:
- `observability/prometheus-alerts.yml` - new file
- `observability/prometheus.yml` - include alerts
- `observability/grafana/provisioning/alerting/` - notification channels

## Notes

- Keep labels LOW CARDINALITY (no session_id, user_id in Prometheus)
- Put high-cardinality data in traces/logs only
- Test with `just dev` after each change
- Commit working increments
