// Metrics tracking for MoQ relay
// Tracks active streams, subscribers, connections, bytes, objects, groups, and errors
// Note: Uses MoQ-native terminology (objects/groups) not media terminology (frames)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Global metrics tracker
/// Uses atomic counters for thread-safe metrics collection
#[derive(Clone, Default)]
pub struct MetricsTracker {
	active_streams: Arc<AtomicU64>,
	active_subscribers: Arc<AtomicU64>,
	active_connections: Arc<AtomicU64>,
	total_connections: Arc<AtomicU64>,
	bytes_sent: Arc<AtomicU64>,
	bytes_received: Arc<AtomicU64>,
	connection_errors: Arc<AtomicU64>,
	// MoQ objects (individual data units within a group)
	objects_sent: Arc<AtomicU64>,
	objects_received: Arc<AtomicU64>,
	// MoQ groups (collections of objects, typically a GOP or similar)
	groups_sent: Arc<AtomicU64>,
	groups_received: Arc<AtomicU64>,
	// Cache effectiveness metrics (relay's core value proposition)
	cache_hits: Arc<AtomicU64>,
	cache_misses: Arc<AtomicU64>,
	// Deduplication: upstream objects saved by serving from cache
	dedup_upstream_saved: Arc<AtomicU64>,
	// Objects dropped due to backpressure or queue overflow
	drops: Arc<AtomicU64>,
	// Current queue depth (pending objects awaiting delivery)
	queue_depth: Arc<AtomicU64>,
}

impl MetricsTracker {
	pub fn new() -> Self {
		Self::default()
	}

	/// Increment active streams count
	pub fn increment_streams(&self) {
		self.active_streams.fetch_add(1, Ordering::Relaxed);
	}

	/// Decrement active streams count
	pub fn decrement_streams(&self) {
		self.active_streams.fetch_sub(1, Ordering::Relaxed);
	}

	/// Increment active subscribers count
	pub fn increment_subscribers(&self) {
		self.active_subscribers.fetch_add(1, Ordering::Relaxed);
	}

	/// Decrement active subscribers count
	pub fn decrement_subscribers(&self) {
		self.active_subscribers.fetch_sub(1, Ordering::Relaxed);
	}

	/// Increment active connections (call on connect)
	pub fn increment_connections(&self) {
		self.active_connections.fetch_add(1, Ordering::Relaxed);
		self.total_connections.fetch_add(1, Ordering::Relaxed);
	}

	/// Decrement active connections (call on disconnect)
	pub fn decrement_connections(&self) {
		self.active_connections.fetch_sub(1, Ordering::Relaxed);
	}

	/// Record a connection error
	pub fn record_error(&self) {
		self.connection_errors.fetch_add(1, Ordering::Relaxed);
	}

	/// Record bytes sent
	pub fn record_bytes_sent(&self, bytes: u64) {
		self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
	}

	/// Record bytes received
	pub fn record_bytes_received(&self, bytes: u64) {
		self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
	}

	/// Record a MoQ object sent
	pub fn record_object_sent(&self) {
		self.objects_sent.fetch_add(1, Ordering::Relaxed);
	}

	/// Record a MoQ object received
	pub fn record_object_received(&self) {
		self.objects_received.fetch_add(1, Ordering::Relaxed);
	}

	/// Record a MoQ group sent
	pub fn record_group_sent(&self) {
		self.groups_sent.fetch_add(1, Ordering::Relaxed);
	}

	/// Record a MoQ group received
	pub fn record_group_received(&self) {
		self.groups_received.fetch_add(1, Ordering::Relaxed);
	}

	/// Record a cache hit (object served from cache)
	pub fn record_cache_hit(&self) {
		self.cache_hits.fetch_add(1, Ordering::Relaxed);
	}

	/// Record a cache miss (object not in cache, fetched from upstream)
	pub fn record_cache_miss(&self) {
		self.cache_misses.fetch_add(1, Ordering::Relaxed);
	}

	/// Record dedup savings (upstream fetch avoided due to existing subscription)
	pub fn record_dedup_saved(&self) {
		self.dedup_upstream_saved.fetch_add(1, Ordering::Relaxed);
	}

	/// Record an object drop (due to backpressure or queue overflow)
	pub fn record_drop(&self) {
		self.drops.fetch_add(1, Ordering::Relaxed);
	}

	/// Set current queue depth
	pub fn set_queue_depth(&self, depth: u64) {
		self.queue_depth.store(depth, Ordering::Relaxed);
	}

	/// Increment queue depth
	pub fn increment_queue_depth(&self) {
		self.queue_depth.fetch_add(1, Ordering::Relaxed);
	}

	/// Decrement queue depth
	pub fn decrement_queue_depth(&self) {
		self.queue_depth.fetch_sub(1, Ordering::Relaxed);
	}

	/// Get current active streams count
	pub fn active_streams(&self) -> u64 {
		self.active_streams.load(Ordering::Relaxed)
	}

	/// Get current active subscribers count
	pub fn active_subscribers(&self) -> u64 {
		self.active_subscribers.load(Ordering::Relaxed)
	}

	/// Get current active connections count
	pub fn active_connections(&self) -> u64 {
		self.active_connections.load(Ordering::Relaxed)
	}

	/// Get total connections ever
	pub fn total_connections(&self) -> u64 {
		self.total_connections.load(Ordering::Relaxed)
	}

	/// Get total connection errors
	pub fn total_errors(&self) -> u64 {
		self.connection_errors.load(Ordering::Relaxed)
	}

	/// Get total bytes sent
	pub fn total_bytes_sent(&self) -> u64 {
		self.bytes_sent.load(Ordering::Relaxed)
	}

	/// Get total bytes received
	pub fn total_bytes_received(&self) -> u64 {
		self.bytes_received.load(Ordering::Relaxed)
	}

	/// Get total MoQ objects sent
	pub fn total_objects_sent(&self) -> u64 {
		self.objects_sent.load(Ordering::Relaxed)
	}

	/// Get total MoQ objects received
	pub fn total_objects_received(&self) -> u64 {
		self.objects_received.load(Ordering::Relaxed)
	}

	/// Get total MoQ groups sent
	pub fn total_groups_sent(&self) -> u64 {
		self.groups_sent.load(Ordering::Relaxed)
	}

	/// Get total MoQ groups received
	pub fn total_groups_received(&self) -> u64 {
		self.groups_received.load(Ordering::Relaxed)
	}

	/// Get total cache hits
	pub fn total_cache_hits(&self) -> u64 {
		self.cache_hits.load(Ordering::Relaxed)
	}

	/// Get total cache misses
	pub fn total_cache_misses(&self) -> u64 {
		self.cache_misses.load(Ordering::Relaxed)
	}

	/// Get total dedup upstream savings
	pub fn total_dedup_saved(&self) -> u64 {
		self.dedup_upstream_saved.load(Ordering::Relaxed)
	}

	/// Get total drops
	pub fn total_drops(&self) -> u64 {
		self.drops.load(Ordering::Relaxed)
	}

	/// Get current queue depth
	pub fn queue_depth(&self) -> u64 {
		self.queue_depth.load(Ordering::Relaxed)
	}
}
