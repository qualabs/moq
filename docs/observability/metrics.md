# MoQ Metrics Reference

Complete reference of all metrics collected by the MoQ observability stack.

Metrics are organized into two layers to align with MoQ's "relay stays dumb" philosophy:
- **MoQ Layer**: Media-agnostic relay metrics (objects, groups, cache, dedup)
- **Hang Layer**: Media-aware client metrics (frames, decode, buffer, sync)

## MoQ Layer (Relay - Media Agnostic)

The relay operates on MoQ-native units without understanding media semantics.

### Connection Metrics

#### moq_relay_active_subscribers

**Type:** Gauge (UpDownCounter)
**Description:** Current number of active subscribers (viewers).

**Labels:**
| Label | Values | Description |
|-------|--------|-------------|
| `relay_instance` | e.g., `relay-1` | Relay identifier |
| `namespace` | e.g., `default` | Stream namespace |
| `region` | e.g., `us-east-1` | Deployment region |

**Example queries:**
```promql
# Total viewers
sum(moq_relay_active_subscribers)

# Viewers by region
sum by (region) (moq_relay_active_subscribers)
```

---

#### moq_relay_active_streams

**Type:** Gauge (UpDownCounter)
**Description:** Current number of active streams being published.

**Labels:** Same as `moq_relay_active_subscribers`

---

#### moq_relay_active_connections

**Type:** Gauge (UpDownCounter)
**Description:** Current number of active connections (publishers + subscribers).

---

#### moq_relay_connections_total

**Type:** Counter
**Description:** Total number of connections over time.

**Example queries:**
```promql
# Connections per minute
sum(rate(moq_relay_connections_total[1m])) * 60
```

---

### Object/Group Metrics

#### moq_relay_objects_sent_total

**Type:** Counter
**Description:** Total MoQ objects transmitted to subscribers.

**Example queries:**
```promql
# Objects per second
sum(rate(moq_relay_objects_sent_total[1m]))
```

---

#### moq_relay_objects_received_total

**Type:** Counter
**Description:** Total MoQ objects received from publishers.

---

#### moq_relay_groups_sent_total

**Type:** Counter
**Description:** Total MoQ groups transmitted (a group is a collection of objects, typically a GOP).

---

#### moq_relay_groups_received_total

**Type:** Counter
**Description:** Total MoQ groups received from publishers.

---

### Bandwidth Metrics

#### moq_relay_bytes_sent_total

**Type:** Counter
**Unit:** bytes
**Description:** Total bytes sent to clients (transport-level; may include retransmissions depending on transport/stats source).

**Example queries:**
```promql
# Bandwidth in Mbps
sum(rate(moq_relay_bytes_sent_total[1m])) * 8 / 1000000
```

---

#### moq_relay_bytes_received_total

**Type:** Counter
**Unit:** bytes
**Description:** Total bytes received from publishers (transport-level; may include retransmissions depending on transport/stats source).

---

#### moq_relay_app_bytes_sent_total

**Type:** Counter
**Unit:** bytes
**Description:** Application-level payload bytes sent (MoQ frame chunks; excludes retransmissions).

---

#### moq_relay_app_bytes_received_total

**Type:** Counter
**Unit:** bytes
**Description:** Application-level payload bytes received (MoQ frame chunks; excludes retransmissions).

**Example queries:**
```promql
# Relay amplification ratio (application-level)
sum(rate(moq_relay_app_bytes_sent_total[1m])) /
clamp_min(sum(rate(moq_relay_app_bytes_received_total[1m])), 1)
```

**Interpretation:**
- ~1.0: mostly passthrough (little/no fanout)
- >1.0: fanout/dedup amplification (one input feeds many outputs)

---

### Cache & Dedup Metrics

These metrics represent the relay's core value proposition.

#### moq_relay_cache_hits_total

**Type:** Counter
**Description:** Objects served without triggering an upstream fetch.

**Caveat (fanout + definition):** In a Producer/Consumer fanout relay, “cache hit” is ambiguous unless we specify *what* is being cached and *what* the alternative would have been.
Depending on definition, a “hit” could mean:
- **Per-consumer delivery hit**: a subscriber received an object that was already buffered (often dominated by fanout and not very informative).
- **Per-upstream work hit**: the relay avoided creating an additional upstream subscription/fetch for an already-requested track/object (usually the useful definition for relays).
- **Late-join/rewind hit**: a subscriber requested older data that was retained (requires explicit retention/history buffers).

**Status:** This metric is **not yet wired to relay events** (placeholder); treat dashboards/alerts based on it as **experimental** until the definition and instrumentation are finalized.

**Example queries:**
```promql
# Cache hit rate
sum(rate(moq_relay_cache_hits_total[5m])) /
(sum(rate(moq_relay_cache_hits_total[5m])) + sum(rate(moq_relay_cache_misses_total[5m]))) * 100
```

**Notes on “hit rate”:** Thresholds depend heavily on the chosen definition and workload (live vs VOD, late join behavior, fanout). Until the definition is finalized, avoid hard SLO thresholds on this ratio.

---

#### moq_relay_cache_misses_total

**Type:** Counter
**Description:** Objects that required upstream work (fetch/subscribe) because they were not already available.

**Caveat:** “Miss” should be the complement of whatever “hit” means. If “hit” is defined per-upstream-work, then “miss” should count the *first* upstream subscription/fetch for a given object/track, not per-subscriber deliveries.

**Status:** This metric is **not yet wired to relay events** (placeholder); treat as **experimental** until instrumentation is finalized.

---

#### moq_relay_dedup_upstream_saved_total

**Type:** Counter
**Description:** Upstream fetches avoided due to subscription deduplication.

This metric shows how many times the relay served multiple subscribers from a single upstream subscription.

**Why this is usually the “real cache hit” for fanout relays:** In a fanout architecture, the most meaningful “cache effectiveness” number is often “how much upstream work was avoided?” (dedup), not “how many downstream reads came from memory?” (which mostly tracks fanout).

---

#### moq_relay_fanout

**Type:** Histogram
**Description:** Effective fanout (how many subscribers are served per stream), recorded periodically.

**Caveat (definition vs implementation):** A true “subscribers per *published group*” metric would be measured at publish/send time with group-level attribution. The current relay implementation records a derived value (\(active\_subscribers / active\_streams\)) on a timer, which is a useful *approximation* but not group-accurate.

**Example queries:**
```promql
# Median fanout
histogram_quantile(0.5, sum(rate(moq_relay_fanout_bucket[5m])) by (le))

# 95th percentile fanout
histogram_quantile(0.95, sum(rate(moq_relay_fanout_bucket[5m])) by (le))
```

---

### Backpressure Metrics

#### moq_relay_queue_depth

**Type:** Gauge (UpDownCounter)
**Description:** Current number of objects pending delivery.

**Health indicators:**
- ✅ Good: < 100
- ⚠️ Warning: 100-1000
- 🔴 Critical: > 1000 (backpressure building)

---

#### moq_relay_drops_total

**Type:** Counter
**Description:** Objects dropped due to backpressure or queue overflow.

**Health indicators:**
- ✅ Good: 0
- ⚠️ Warning: Any drops
- 🔴 Critical: Sustained drops

---

#### moq_relay_errors_total

**Type:** Counter
**Description:** Connection errors.

---

## Hang Layer (Client - Media Aware)

Media-specific metrics collected in the browser player.

### Client Experience (CMCD-aligned)

#### moq_client_buffer_length_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Current video buffer length.

**Labels:**
| Label | Values | Description |
|-------|--------|-------------|
| `track_type` | `video`, `audio` | Media track type |
| `codec` | e.g., `avc1.64001f` | Codec identifier |

**Health indicators:**
- ✅ Good: > 2 seconds
- ⚠️ Warning: 0.5-2 seconds
- 🔴 Critical: < 0.5 seconds

---

#### moq_client_startup_time_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Time from play request to first frame rendered (CMCD `st`).

**Health indicators:**
- ✅ Good: < 1 second
- ⚠️ Warning: 1-3 seconds
- 🔴 Critical: > 3 seconds

---

#### moq_client_latency_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Latency to live edge (CMCD `dl`).

---

#### moq_client_bitrate_bps

**Type:** Histogram
**Unit:** bits per second
**Description:** Current playback bitrate (CMCD `br`).

---

#### moq_client_quality_switches_total

**Type:** Counter
**Description:** Number of quality/bitrate switches.

**Health indicators:**
- ✅ Good: < 2/minute
- ⚠️ Warning: 2-5/minute
- 🔴 Critical: > 5/minute

---

#### moq_client_connections_total

**Type:** Counter
**Description:** Connections by transport type.

**Labels:**
| Label | Values | Description |
|-------|--------|-------------|
| `transport` | `webtransport`, `websocket` | Connection transport |

---

#### moq_client_rebuffer_count_total

**Type:** Counter
**Description:** Rebuffering events (playback stalls) (CMCD `bs`).

**Health indicators:**
- ✅ Good: 0
- ⚠️ Warning: 1-2/session
- 🔴 Critical: > 2/session

---

### Decode/Render Metrics

#### moq_client_frames_decoded_total

**Type:** Counter
**Description:** Successfully decoded video frames.

---

#### moq_client_frames_dropped_total

**Type:** Counter
**Description:** Video frames dropped (decode failure, late arrival, or congestion).

**Health indicators:**
- ✅ Good: < 0.1% of frames
- ⚠️ Warning: 0.1-1%
- 🔴 Critical: > 1%

---

#### moq_client_keyframe_interval_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Time between keyframes (IDR frames).

---

#### moq_client_decode_time_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Video frame decode latency.

---

#### moq_client_av_sync_drift_seconds

**Type:** Histogram
**Unit:** seconds
**Description:** Audio/video synchronization drift. Positive = video ahead, negative = audio ahead.

**Health indicators:**
- ✅ Good: < 40ms drift
- ⚠️ Warning: 40-80ms
- 🔴 Critical: > 80ms (noticeable lip sync issues)

---

## CMCD Mapping

These metrics align with [Common Media Client Data (CMCD)](https://cdn.cta.tech/cta/media/media/resources/standards/pdfs/cta-5004-final.pdf) concepts:

| CMCD Field | Our Metric | Description |
|------------|------------|-------------|
| `bl` (buffer length) | `moq_client_buffer_length_seconds` | Buffer level in seconds |
| `st` (startup time) | `moq_client_startup_time_seconds` | Time to first frame |
| `bs` (buffer starvation) | `moq_client_rebuffer_count_total` | Rebuffering events |
| `br` (bitrate) | `moq_client_bitrate_bps` | Current bitrate |
| `dl` (deadline) | `moq_client_latency_seconds` | Latency to live edge |

Note: CMCD is used as a **vocabulary reference**, not as wire protocol (MoQ is not HTTP-based).

---

## Label Cardinality Guidelines

To keep Prometheus healthy, we follow these rules:

### Safe Labels (low cardinality)
- `transport`: 2-3 values
- `codec`: ~10 values
- `track_type`: 2-3 values
- `region`: ~10 values
- `relay_instance`: ~10-100 values

### Avoided Labels (high cardinality)
- `session_id`: Millions of values → Use traces/logs instead
- `user_id`: Millions of values → Use traces/logs instead
- `stream_id`: Could be high → Keep for traces only

For per-session debugging, use **Tempo traces** or **Loki logs** which handle high cardinality.
