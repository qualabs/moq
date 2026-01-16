# MoQ Observability Metrics Checklist (Implementation + Correctness)

This checklist categorizes each metric into one of the following implementation states.

## Categories

- **✅ Done**: Implemented and trustworthy.
- **⚠️ Approx**: Implemented, but simplified/approximate.
- **🟡 Not wired**: Exists, but not actually recorded yet.
- **🧱 Big work**: Needs major instrumentation/architecture work.
- **🚫 N/A**: Not meaningful/feasible in this deployment.

---

## Summary table (presentable)

**How to verify quickly:** the ✅/⚠️ tables below include **Source / Trigger / Sanity check** columns. The “Implementation validation (✅/⚠️ only)” section further below expands each item with more detail.

### Relay (MoQ layer)

#### Measured now (✅/⚠️)

| Metric | Subcat | Status | Source (Where) | Trigger / Computation (What) | Quick sanity check |
|---|---|---:|---|---|---|
| `moq_relay_active_streams` | Capacity | ✅ | `rs/moq-relay/src/cluster.rs` → `MetricsTracker::{increment_streams,decrement_streams}`; exported by `rs/moq-relay/src/observability.rs` | UpDownCounter exported as delta every 10s from `active_streams()` | Publish 1 stream ⇒ ~1 until stopped |
| `moq_relay_active_subscribers` | Capacity | ✅ | `rs/moq-relay/src/connection.rs` increments/decrements; exported by `observability.rs` | UpDownCounter exported as delta every 10s from `active_subscribers()` | Start N viewers ⇒ ~N |
| `moq_relay_active_connections` | Conn | ⚠️ | `rs/moq-relay/src/connection.rs` → `MetricsTracker::{increment_connections,decrement_connections}` | **QUIC/WebTransport connection count** (not MoQ sessions) | Fanout: origin may show ~1 with many viewers |
| `moq_relay_connections_total` | Conn | ⚠️ | `MetricsTracker::increment_connections` increments `total_connections` | Monotonic total QUIC connections | Increases per new QUIC conn |
| `moq_relay_active_sessions_by_transport` | Capacity | ✅ | WT: `rs/moq-relay/src/connection.rs`; WS: `rs/moq-relay/src/web.rs`; exported by `observability.rs` w/ `transport` | UpDownCounter; active MoQ sessions split by `transport` | Start N viewers ⇒ 해당 `transport` ~N |
| `moq_relay_sessions_total_by_transport` | Capacity | ✅ | Same as above | Counter; total MoQ sessions split by `transport` | Monotonic; +1 per new session |
| `moq_relay_app_bytes_sent_total` | AppTP | ✅ | moq-lite `Stats.add_tx_bytes` via `TransportStats` passed in `connection.rs` + `web.rs` | Counter; sums app payload bytes sent | No viewers ⇒ output Mbps ~0 |
| `moq_relay_app_bytes_received_total` | AppTP | ✅ | moq-lite `Stats.add_rx_bytes` via `TransportStats` | Counter; sums app payload bytes received | Tracks publisher input rate |
| `moq_relay_app_bytes_sent_total_by_transport` | AppTP | ✅ | `TransportStats` attributes bytes by `transport`; exported by `observability.rs` | Counter; app bytes sent split by transport | Only QUIC ⇒ `webtransport` dominates |
| `moq_relay_app_bytes_received_total_by_transport` | AppTP | ✅ | Same | Counter; app bytes received split by transport | Only QUIC ⇒ `webtransport` dominates |
| `moq_relay_errors_total` | Reliability | ✅ | `rs/moq-relay/src/connection.rs` on auth reject + session error | Counter; total errors | Bad JWT ⇒ increments |
| `moq_relay_fanout` | Effect | ⚠️ | `rs/moq-relay/src/observability.rs` periodic compute | Hist records `active_subscribers/active_streams` sample | 1 stream + N subs ⇒ ~N |

#### Not yet / incomplete (🟡/🧱/🚫)

| Metric | Subcat | Status | What’s missing (short) |
|---|---|---:|---|
| `moq_relay_bytes_sent_total` | TransportTP | 🟡 | QUIC-only stats polling |
| `moq_relay_bytes_received_total` | TransportTP | 🟡 | QUIC-only stats polling |
| `moq_relay_objects_sent_total` | Protocol | 🟡 | Not hooked to send path |
| `moq_relay_objects_received_total` | Protocol | 🟡 | Not hooked to receive path |
| `moq_relay_groups_sent_total` | Protocol | 🟡 | Not hooked to send path |
| `moq_relay_groups_received_total` | Protocol | 🟡 | Not hooked to receive path |
| `moq_relay_cache_hits_total` | Cache/Dedup | 🟡 | Placeholder; definition+hooks missing |
| `moq_relay_cache_misses_total` | Cache/Dedup | 🟡 | Placeholder; definition+hooks missing |
| `moq_relay_dedup_upstream_saved_total` | Cache/Dedup | 🟡 | Not hooked to dedup logic |
| `moq_relay_queue_depth` | Backpressure | 🟡 | Not fed from queue |
| `moq_relay_drops_total` | Backpressure | 🟡 | Not hooked to drop points |
| `moq_relay_publish_to_delivery_seconds` | Latency | 🧱 | Needs publish→delivery attribution |
| `moq_relay_quic_rtt_seconds` | QUIC | 🧱 | Needs RTT samples exported |
| `moq_relay_quic_packet_loss_ratio` | QUIC | 🧱 | Needs loss samples exported |

### Client (Hang layer)

#### Measured now (✅/⚠️)

| Metric | Subcat | Status | Source (Where) | Trigger / Computation (What) | Quick sanity check |
|---|---|---:|---|---|---|
| `moq_client_connections_total` | Conn | ✅ | `js/hang/src/observability/index.ts` `Connection.onConnectionType(...)` | Counter; increments on connect; `transport` label | Each player adds ~1 |
| `moq_client_startup_time_seconds` | Startup | ✅ | `js/hang/src/watch/broadcast.ts`, `js/hang/src/watch/video/source.ts`, `js/hang/src/watch/audio/source.ts` | Hist; seconds since `performance.now()` start markers | Local dev small; regressions shift upward |
| `moq_client_rebuffer_count_total` | Quality | ⚠️ | Video+Audio `consumer.decode()` wait loops | Counter; increments when waitDuration > 100ms after first frame | Should be ~0 on healthy playback |
| `moq_client_quality_switches_total` | Adapt | ⚠️ | `js/hang/src/watch/video/source.ts` | Counter; increments at (re)subscribe start | ~1 at start; grows on restarts |
| `moq_client_frames_decoded_total` | Render | ✅ | `js/hang/src/watch/video/source.ts` | Counter; per rendered frame | `rate()` ≈ FPS |
| `moq_client_keyframe_interval_seconds` | Encode | ✅ | `js/hang/src/watch/video/source.ts` | Hist; `(ts - last_ts)/1e6` on keyframes | Matches GOP (~1–2s) |
| `moq_client_buffer_length_seconds` | Buffer | ⚠️ | `js/hang/src/watch/video/source.ts` | Hist; records `sleep/1000` (scheduled render delay) | Near target latency when ahead |
| `moq_client_decode_time_seconds` | Decode | ⚠️ | `js/hang/src/watch/video/source.ts` | Hist; time to call `decoder.decode()` (submission) | Low ms; not true decode latency |

#### Not yet / incomplete (🟡/🧱/🚫)

| Metric | Subcat | Status | What’s missing (short) |
|---|---|---:|---|
| `moq_client_frames_dropped_total` | Render | 🟡 | Not hooked to drop reasons |
| `moq_client_latency_seconds` | E2E | 🧱 | Needs shared timeline/definition |
| `moq_client_bitrate_bps` | Bitrate | 🧱 | Needs bitrate model + hooks |
| `moq_client_av_sync_drift_seconds` | A/V | 🧱 | Needs A/V timeline correlation |

### qlog / protocol event coverage

| Area | Category | Why / Notes |
|---|---:|---|
| MOQT qlog events (e.g. control messages, fetch/object events) | 🧱 | qlog is **structured event logging**, not Prometheus metrics. It’s most relevant on the primary QUIC/WebTransport path and requires explicit event emission + a logs/traces pipeline (see [draft-pardue-moq-qlog-moq-events](https://datatracker.ietf.org/doc/draft-pardue-moq-qlog-moq-events/)). |

---

## Implementation validation (✅/⚠️ only)

This section is intended for quick correctness review (“is this stat what it claims?”). Each item includes:
- **Where**: code location(s)
- **What it measures**: as implemented
- **Sanity check**: simple test/expectation

### Relay

#### ✅ `moq_relay_active_streams`
- **Where**: `rs/moq-relay/src/cluster.rs` calls `MetricsTracker::{increment_streams,decrement_streams}`; exported in `rs/moq-relay/src/observability.rs` periodic exporter.
- **What it measures**: count of active published broadcasts (streams) tracked by lifecycle.
- **Sanity check**: publish 1 broadcast ⇒ metric goes to 1 and stays until publish stops; publish 2 broadcasts ⇒ 2.

#### ✅ `moq_relay_active_subscribers`
- **Where**: `rs/moq-relay/src/connection.rs` increments/decrements subscribers for sessions that have subscribe permissions; exported in `rs/moq-relay/src/observability.rs`.
- **What it measures**: number of accepted *subscriber* sessions (not publishers).
- **Sanity check**: start N viewers ⇒ ~N; disconnect viewers ⇒ decreases back toward 0.

#### ⚠️ `moq_relay_active_connections`
- **Where**: `rs/moq-relay/src/connection.rs` calls `MetricsTracker::{increment_connections,decrement_connections}`; exported in `rs/moq-relay/src/observability.rs`.
- **What it measures**: **WebTransport/QUIC connection count** (not a MoQ session count).
- **Sanity check**: on primary QUIC path, should track the number of QUIC connections accepted; if you have a fanout topology, origin relay may show ~1 even with many downstream viewers.

#### ⚠️ `moq_relay_connections_total`
- **Where**: same as above (`MetricsTracker::increment_connections` increments both active+total); exported in `rs/moq-relay/src/observability.rs`.
- **What it measures**: total accepted QUIC/WebTransport connections over time.
- **Sanity check**: monotonically increases with each new QUIC connection (not per viewer if relays are chained).

#### ✅ `moq_relay_active_sessions_by_transport` / `moq_relay_sessions_total_by_transport`
- **Where**:
  - WebTransport session: `rs/moq-relay/src/connection.rs` calls `MetricsTracker::increment_sessions(Transport::WebTransport)` and decrements on close.
  - WebSocket fallback session: `rs/moq-relay/src/web.rs` calls `increment_sessions(Transport::WebSocket)` and decrements via a guard on close.
  - Exported in `rs/moq-relay/src/observability.rs` with label `transport=webtransport|websocket`.
- **What it measures**: active/total **MoQ sessions** split by transport.
- **Sanity check**: if you start N viewers (sessions) on a given relay, `active_sessions_by_transport{transport=...}` should approach N.

#### ✅ `moq_relay_app_bytes_sent_total` / `moq_relay_app_bytes_received_total`
- **Where**:
  - moq-lite calls the `Stats` trait hooks `add_tx_bytes/add_rx_bytes` on the stats object passed at accept time.
  - Relay passes a transport-aware stats wrapper (`TransportStats`) in both accept paths: `rs/moq-relay/src/connection.rs` and `rs/moq-relay/src/web.rs`.
  - Exported in `rs/moq-relay/src/observability.rs`.
- **What it measures**: application payload bytes (MoQ objects/chunks) sent/received at the session level; excludes retransmits (by definition of the Stats hook usage).
- **Sanity check**: output rate should be ≥ input rate when fanout > 1; if no viewers, output should go ~0.

#### ✅ `moq_relay_app_bytes_sent_total_by_transport` / `moq_relay_app_bytes_received_total_by_transport`
- **Where**: same Stats hooks as above; `TransportStats` attributes bytes to `transport`; exported with label `transport=...` in `rs/moq-relay/src/observability.rs`.
- **What it measures**: application payload bytes split by transport.
- **Sanity check**: when running only QUIC viewers, websocket series should be ~0; when only WS fallback, webtransport series ~0.

#### ✅ `moq_relay_errors_total`
- **Where**: `rs/moq-relay/src/connection.rs` increments on auth reject and on session close error; exported in `rs/moq-relay/src/observability.rs`.
- **What it measures**: total connection/session errors seen by the relay.
- **Sanity check**: invalid JWT/path should increment; normal closes should not (unless treated as error upstream).

#### ⚠️ `moq_relay_fanout`
- **Where**: computed periodically in `rs/moq-relay/src/observability.rs` as `active_subscribers / active_streams` and recorded as a histogram sample.
- **What it measures**: a **point-in-time approximation** of fanout (not per-group attribution).
- **Sanity check**: with 1 active stream and N subscribers, should hover around N; if streams change rapidly, can jump.

### Client (Hang layer)

#### ✅ `moq_client_connections_total`
- **Where**: `js/hang/src/observability/index.ts` registers `@moq/lite` `Connection.onConnectionType(...)` and records `recordConnection(type)` on connect.
- **What it measures**: number of client connections by `transport` (webtransport/websocket).
- **Sanity check**: each player session should add ~1; transport label should match the chosen connection type.

#### ✅ `moq_client_startup_time_seconds`
- **Where**:
  - “Announce became active” startup time: `js/hang/src/watch/broadcast.ts` records once when the first active broadcast is observed.
  - Time-to-first-video-frame: `js/hang/src/watch/video/source.ts` records on first rendered frame.
  - Time-to-first-audio: `js/hang/src/watch/audio/source.ts` records on first decoded audio.
- **What it measures**: a few related “time to start” notions; the label set differs between the above call sites.
- **Sanity check**: should be small in local dev; increases if relay is slow to announce or if decode is delayed.

#### ⚠️ `moq_client_rebuffer_count_total`
- **Where**:
  - Video: `js/hang/src/watch/video/source.ts` increments when `consumer.decode()` takes > `REBUFFER_THRESHOLD_MS` (100ms) after the first frame.
  - Audio: `js/hang/src/watch/audio/source.ts` increments when `consumer.decode()` takes > 100ms after first audio.
- **What it measures**: “data starvation / decode wait exceeded threshold” events (can be triggered by jitter, GC pauses, etc.).
- **Sanity check**: should be near 0 on healthy steady playback; spikes correlate with stalls/jitter but are not a perfect “stall” detector.

#### ⚠️ `moq_client_quality_switches_total`
- **Where**: `js/hang/src/watch/video/source.ts` increments on track subscription start.
- **What it measures**: “track (re)subscription events”, which may overcount true rendition switches.
- **Sanity check**: should be 1 on initial start; increases when switching tracks or restarting.

#### ✅ `moq_client_frames_decoded_total`
- **Where**: `js/hang/src/watch/video/source.ts` increments on each frame emitted to the renderer (`recordFrameDecoded`).
- **What it measures**: frames successfully rendered/decoded by the client.
- **Sanity check**: `rate(frames_decoded_total)` should approximate displayed FPS.

#### ✅ `moq_client_keyframe_interval_seconds`
- **Where**: `js/hang/src/watch/video/source.ts` records interval between keyframes using encoded timestamps (`(next.timestamp - lastKeyframeTime) / 1_000_000`).
- **What it measures**: GOP duration / keyframe cadence.
- **Sanity check**: should align with encoder GOP config (e.g., ~2s).

#### ⚠️ `moq_client_buffer_length_seconds`
- **Where**: `js/hang/src/watch/video/source.ts` records `sleep/1000`, where `sleep` is computed from timestamp alignment + configured latency.
- **What it measures**: “scheduled delay until render” (how far ahead the frame is relative to playback), not a true buffered duration.
- **Sanity check**: tends toward your configured latency when frames arrive early; collapses toward 0 when near-starvation.

#### ⚠️ `moq_client_decode_time_seconds`
- **Where**: `js/hang/src/watch/video/source.ts` measures only the time to call `decoder.decode(chunk)` (submission), not async decode completion.
- **What it measures**: submission/queueing overhead, not actual decode latency.
- **Sanity check**: should be low (sub-ms to few ms) and does not correlate with true decoder load.
