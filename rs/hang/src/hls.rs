//! HLS (HTTP Live Streaming) importer built on top of CMAF.
//!
//! This module provides reusable logic to ingest HLS master/media playlists and
//! feed their CMAF segments into a `hang` broadcast. It is designed to be
//! independent of any particular HTTP client; callers provide an implementation
//! of [`HlsFetcher`] to perform the actual network I/O.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use m3u8_rs::{
	AlternativeMedia, AlternativeMediaType, Map, MasterPlaylist, MediaPlaylist, MediaSegment, Resolution, VariantStream,
};
use tracing::{debug, info, warn};
use url::Url;

use crate::{cmaf, moq_lite::BroadcastProducer, CatalogProducer, Error, Result};

/// Configuration for the single-rendition HLS ingest loop.
#[derive(Clone)]
pub struct HlsConfig {
	/// The master or media playlist URL to ingest.
	pub playlist: Url,
	/// Number of media segments to buffer before announcing the broadcast.
	pub preroll_segments: usize,
	/// Fraction of target duration to wait after new data is ingested.
	pub refresh_ratio: f32,
	/// Optional list of allowed output resolutions (width, height).
	/// When set, only variants matching these resolutions will be ingested.
	pub allowed_resolutions: Option<Vec<(u32, u32)>>,
}

impl HlsConfig {
	pub fn new(playlist: Url, preroll_segments: usize, refresh_ratio: f32) -> Self {
		Self {
			playlist,
			preroll_segments,
			refresh_ratio,
			allowed_resolutions: None,
		}
	}
}

/// Result of a single ingest step.
pub struct StepOutcome {
	/// Number of media segments written during this step.
	pub wrote_segments: usize,
	/// Target segment duration (in seconds) from the playlist, if known.
	pub target_duration: Option<f32>,
}

/// Asynchronous byte fetcher used by the HLS importer.
///
/// This allows the core HLS ingest logic to stay independent of any particular
/// HTTP client implementation. Callers (such as `hang-cli`) provide a concrete
/// implementation that performs the actual network I/O.
pub trait HlsFetcher: Send + Sync {
	fn fetch_bytes(&self, url: &Url) -> Pin<Box<dyn Future<Output = Result<Bytes>> + Send + '_>>;
}

/// Pulls an HLS media playlist and feeds the bytes into the CMAF importer.
pub struct HlsIngest<F: HlsFetcher> {
	/// Broadcast that all CMAF importers write into.
	broadcast: BroadcastProducer,

	/// Shared catalog handle used when multiple CMAF importers contribute to
	/// the same `catalog.json` track.
	shared_catalog: Option<CatalogProducer>,

	/// CMAF importers for each discovered video rendition.
	/// Each importer feeds a separate MoQ track but shares the same catalog.
	video_importers: Vec<cmaf::Import>,

	/// CMAF importer for the selected audio rendition, if any.
	audio_importer: Option<cmaf::Import>,

	fetcher: F,
	cfg: HlsConfig,
	/// All discovered video variants (one per HLS rendition).
	video: Vec<TrackState>,
	/// Optional audio track shared across variants.
	audio: Option<TrackState>,
}

#[derive(Clone)]
struct TrackState {
	label: &'static str,
	playlist: Url,
	next_sequence: Option<u64>,
	init_ready: bool,
}

impl TrackState {
	fn new(label: &'static str, playlist: Url) -> Self {
		Self {
			label,
			playlist,
			next_sequence: None,
			init_ready: false,
		}
	}
}

impl<F: HlsFetcher> HlsIngest<F> {
	/// Create a new HLS importer that will write into the given broadcast.
	pub fn new(broadcast: BroadcastProducer, cfg: HlsConfig, fetcher: F) -> Self {
		Self {
			broadcast,
			shared_catalog: None,
			video_importers: Vec::new(),
			audio_importer: None,
			fetcher,
			cfg,
			video: Vec::new(),
			audio: None,
		}
	}

	/// Fetch the latest playlist, download the init segment, and prime the importer with a buffer of segments.
	pub async fn prime(&mut self) -> Result<usize> {
		self.ensure_tracks().await?;

		let mut buffered = 0usize;

		// Prime all discovered video variants.
		//
		// Move the video track states out of `self` so we can safely mutate both
		// the ingest and the tracks without running into borrow checker issues.
		let video_tracks = std::mem::take(&mut self.video);
		for (index, mut track) in video_tracks.into_iter().enumerate() {
			let url = track.playlist.clone();
			let playlist = self.fetch_media_playlist(&url).await?;
			let count = self
				.consume_segments(index, &mut track, &playlist, Some(self.cfg.preroll_segments))
				.await?;
			buffered += count;
			self.video.push(track);
		}

		// Prime the shared audio track, if any.
		if let Some(url) = self.audio.as_ref().map(|track| track.playlist.clone()) {
			let playlist = self.fetch_media_playlist(&url).await?;
			if let Some(mut track) = self.audio.take() {
				let count = self
					.consume_segments(usize::MAX, &mut track, &playlist, Some(self.cfg.preroll_segments))
					.await?;
				buffered += count;
				self.audio = Some(track);
			}
		}

		if buffered == 0 {
			warn!("HLS playlist had no new segments during prime step");
		} else {
			info!(count = buffered, "buffered initial HLS segments");
		}

		Ok(buffered)
	}

	/// Perform a single ingest step for all active tracks.
	///
	/// This fetches the current media playlists, consumes any fresh segments,
	/// and returns how many segments were written along with the target
	/// duration to guide scheduling of the next step.
	pub async fn step(&mut self) -> Result<StepOutcome> {
		self.ensure_tracks().await?;

		let mut wrote = 0usize;
		let mut target_duration = None;

		// Ingest a step from all active video variants.
		let video_tracks = std::mem::take(&mut self.video);
		for (index, mut track) in video_tracks.into_iter().enumerate() {
			let url = track.playlist.clone();
			let playlist = self.fetch_media_playlist(&url).await?;
			// Use the first video's target duration as the base.
			if target_duration.is_none() {
				target_duration = Some(playlist.target_duration);
			}
			let count = self.consume_segments(index, &mut track, &playlist, None).await?;
			wrote += count;
			self.video.push(track);
		}

		// Ingest from the shared audio track, if present.
		if let Some(url) = self.audio.as_ref().map(|track| track.playlist.clone()) {
			let playlist = self.fetch_media_playlist(&url).await?;
			if target_duration.is_none() {
				target_duration = Some(playlist.target_duration);
			}
			if let Some(mut track) = self.audio.take() {
				let count = self
					.consume_segments(usize::MAX, &mut track, &playlist, None)
					.await?;
				wrote += count;
				self.audio = Some(track);
			}
		}

		Ok(StepOutcome {
			wrote_segments: wrote,
			target_duration,
		})
	}

	/// Compute the delay before the next ingest step should run.
	pub fn refresh_delay(&self, target_duration: Option<f32>, wrote_segments: usize) -> Duration {
		let base = target_duration
			.map(|dur| Duration::from_secs_f32(dur.max(0.5)))
			.unwrap_or_else(|| Duration::from_millis(500));
		if wrote_segments == 0 {
			return base;
		}

		base.mul_f32(self.cfg.refresh_ratio)
	}

	async fn fetch_media_playlist(&self, url: &Url) -> Result<MediaPlaylist> {
		let body = self.fetch_bytes(url).await?;
		let (_, playlist) = m3u8_rs::parse_media_playlist(&body)
			.map_err(|err| Error::Hls(format!("failed to parse media playlist: {err}")))?;
		Ok(playlist)
	}

	async fn ensure_tracks(&mut self) -> Result<()> {
		// Tracks already discovered.
		if !self.video.is_empty() {
			return Ok(());
		}

		let body = self.fetch_bytes(&self.cfg.playlist).await?;
		if let Ok((_, master)) = m3u8_rs::parse_master_playlist(&body) {
			let mut variants = select_variants(&master);
			if !variants.is_empty() {
				// Optionally filter by explicitly allowed resolutions.
				if let Some(ref allowed) = self.cfg.allowed_resolutions {
					let original = variants.clone();
					variants.retain(|variant| {
						if let Some(Resolution { width, height }) = variant.resolution {
							allowed
								.iter()
								.any(|(w, h)| *w == width as u32 && *h == height as u32)
						} else {
							false
						}
					});

					if variants.is_empty() {
						warn!(
							?allowed,
							"no HLS variants matched requested resolutions; falling back to all H.264 renditions"
						);
						variants = original;
					}
				}

				// Create a video track state for every usable variant.
				for variant in &variants {
					let video_url = resolve_uri(&self.cfg.playlist, &variant.uri)
						.map_err(|err| Error::Hls(format!("failed to resolve video rendition URL: {err}")))?;
					self.video.push(TrackState::new("video", video_url));
				}

				// Choose an audio rendition based on the first variant with an audio group.
				if let Some(variant) = variants.iter().find(|v| v.audio.is_some()) {
					if let Some(group_id) = variant.audio.as_deref() {
						if let Some(audio_tag) = select_audio(&master, group_id) {
							if let Some(uri) = &audio_tag.uri {
								let audio_url = resolve_uri(&self.cfg.playlist, uri).map_err(|err| {
									Error::Hls(format!("failed to resolve audio rendition URL: {err}"))
								})?;
								self.audio = Some(TrackState::new("audio", audio_url));
							} else {
								warn!(%group_id, "audio rendition missing URI");
							}
						} else {
							warn!(%group_id, "audio group not found in master playlist");
						}
					}
				}

				let audio_url = self.audio.as_ref().map(|a| a.playlist.to_string());
				info!(
					video_variants = variants.len(),
					audio = audio_url.as_deref().unwrap_or("none"),
					"selected master playlist renditions"
				);

				return Ok(());
			}
		}

		// Fallback: treat the provided URL as a single media playlist.
		self.video.push(TrackState::new("video", self.cfg.playlist.clone()));
		Ok(())
	}

	async fn consume_segments(
		&mut self,
		index: usize,
		track: &mut TrackState,
		playlist: &MediaPlaylist,
		limit: Option<usize>,
	) -> Result<usize> {
		self.ensure_init_segment(index, track, playlist).await?;

		let mut consumed = 0usize;
		let mut sequence = playlist.media_sequence;
		let max = limit.unwrap_or(usize::MAX);

		for segment in &playlist.segments {
			if let Some(next) = track.next_sequence {
				if sequence < next {
					sequence += 1;
					continue;
				}
			}

			if consumed >= max {
				break;
			}

			self.push_segment(index, track, segment, sequence).await?;
			consumed += 1;
			sequence += 1;
		}

		if limit.is_none() && consumed == 0 {
			debug!(track = track.label, "no fresh HLS segments available");
		}

		Ok(consumed)
	}

	async fn ensure_init_segment(
		&mut self,
		index: usize,
		track: &mut TrackState,
		playlist: &MediaPlaylist,
	) -> Result<()> {
		if track.init_ready {
			return Ok(());
		}

		let map = self
			.find_map(playlist)
			.ok_or_else(|| Error::Hls("playlist missing EXT-X-MAP".to_string()))?;

		let url = resolve_uri(&track.playlist, &map.uri)
			.map_err(|err| Error::Hls(format!("failed to resolve init segment URL: {err}")))?;
		let bytes = match self.fetch_bytes(&url).await {
			Ok(bytes) => bytes,
			Err(err) => return Err(err),
		};
		let importer = match track.label {
			"video" => self.ensure_video_importer_for(index)?,
			"audio" => self.ensure_audio_importer()?,
			_ => unreachable!("unexpected HLS track label"),
		};

		importer
			.parse(&bytes)
			.map_err(|err| Error::Hls(format!("failed to parse init segment as CMAF: {err}")))?;

		track.init_ready = true;
		info!(track = track.label, "loaded HLS init segment");
		Ok(())
	}

	async fn push_segment(
		&mut self,
		index: usize,
		track: &mut TrackState,
		segment: &MediaSegment,
		sequence: u64,
	) -> Result<()> {
		if segment.uri.is_empty() {
			return Err(Error::Hls("encountered segment with empty URI".to_string()));
		}

		let url = resolve_uri(&track.playlist, &segment.uri)
			.map_err(|err| Error::Hls(format!("failed to resolve segment URL: {err}")))?;
		let bytes = match self.fetch_bytes(&url).await {
			Ok(bytes) => bytes,
			Err(err) => return Err(err),
		};

		let importer = match track.label {
			"video" => self.ensure_video_importer_for(index)?,
			"audio" => self.ensure_audio_importer()?,
			_ => unreachable!("unexpected HLS track label"),
		};

		importer
			.parse(bytes.as_ref())
			.map_err(|err| Error::Hls(format!("failed to parse media segment as CMAF: {err}")))?;
		track.next_sequence = Some(sequence + 1);

		Ok(())
	}

	fn find_map<'a>(&self, playlist: &'a MediaPlaylist) -> Option<&'a Map> {
		playlist.segments.iter().find_map(|segment| segment.map.as_ref())
	}

	async fn fetch_bytes(&self, url: &Url) -> Result<Bytes> {
		self.fetcher.fetch_bytes(url).await
	}

	/// Lazily create or retrieve the CMAF importer for a specific video rendition.
	///
	/// Each video variant gets its own importer so that their tracks remain
	/// independent while still contributing to the same shared catalog.
	fn ensure_video_importer_for(&mut self, index: usize) -> Result<&mut cmaf::Import> {
		// Audio paths pass usize::MAX; they should never request a video importer.
		debug_assert!(index != usize::MAX, "audio track must not use video importer");

		while self.video_importers.len() <= index {
			let importer = match &self.shared_catalog {
				// First importer for this broadcast, create a fresh catalog track.
				None => {
					let importer = cmaf::Import::new(self.broadcast.clone());
					let catalog = importer.catalog();
					self.shared_catalog = Some(catalog);
					importer
				}
				// Reuse the existing catalog track for this broadcast.
				Some(catalog) => cmaf::Import::with_catalog(self.broadcast.clone(), catalog.clone()),
			};

			self.video_importers.push(importer);
		}

		Ok(self
			.video_importers
			.get_mut(index)
			.expect("video_importer must be initialized"))
	}

	/// Lazily create or retrieve the CMAF importer for the audio rendition.
	fn ensure_audio_importer(&mut self) -> Result<&mut cmaf::Import> {
		if self.audio_importer.is_none() {
			let importer = match &self.shared_catalog {
				// First importer for this broadcast, create a fresh catalog track.
				None => {
					let importer = cmaf::Import::new(self.broadcast.clone());
					let catalog = importer.catalog();
					self.shared_catalog = Some(catalog);
					importer
				}
				// Reuse the existing catalog track for this broadcast.
				Some(catalog) => cmaf::Import::with_catalog(self.broadcast.clone(), catalog.clone()),
			};

			self.audio_importer = Some(importer);
		}

		Ok(self.audio_importer.as_mut().expect("audio_importer must be initialized"))
	}

	#[cfg(test)]
	fn has_video_importer(&self) -> bool {
		!self.video_importers.is_empty()
	}

	#[cfg(test)]
	fn has_audio_importer(&self) -> bool {
		self.audio_importer.is_some()
	}
}

fn select_audio<'a>(master: &'a MasterPlaylist, group_id: &str) -> Option<&'a AlternativeMedia> {
	let mut first = None;
	let mut default = None;

	for alternative in master
		.alternatives
		.iter()
		.filter(|alt| alt.media_type == AlternativeMediaType::Audio && alt.group_id == group_id)
	{
		if first.is_none() {
			first = Some(alternative);
		}
		if alternative.default {
			default = Some(alternative);
			break;
		}
	}

	default.or(first)
}

fn select_variants<'a>(master: &'a MasterPlaylist) -> Vec<&'a VariantStream> {
	// Helper to extract the first video codec token from the CODECS attribute.
	fn first_video_codec(variant: &VariantStream) -> Option<&str> {
		let codecs = variant.codecs.as_deref()?;
		codecs
			.split(',')
			.map(|s| s.trim())
			.find(|s| !s.is_empty())
	}

	// Map codec strings into a coarse "family" so we can prefer H.264 over others.
	fn codec_family(codec: &str) -> Option<&'static str> {
		if codec.starts_with("avc1.") || codec.starts_with("avc3.") {
			Some("h264")
		} else {
			None
		}
	}

	// Consider only non-i-frame variants with a URI and a known codec family.
	let candidates: Vec<(&VariantStream, &str, &str)> = master
		.variants
		.iter()
		.filter(|variant| !variant.is_i_frame && !variant.uri.is_empty())
		.filter_map(|variant| {
			let codec = first_video_codec(variant)?;
			let family = codec_family(codec)?;
			Some((variant, codec, family))
		})
		.collect();

	if candidates.is_empty() {
		return Vec::new();
	}

	// Prefer families in this order, falling back to the first available.
	const FAMILY_PREFERENCE: &[&str] = &["h264", "h265", "vp9", "av1"];

	let families_present: Vec<&str> = candidates.iter().map(|(_, _, fam)| *fam).collect();

	let target_family = FAMILY_PREFERENCE
		.iter()
		.find(|fav| families_present.iter().any(|fam| fam == *fav))
		.copied()
		.unwrap_or(families_present[0]);

	// Keep only variants in the chosen family.
	let family_variants: Vec<&VariantStream> = candidates
		.into_iter()
		.filter(|(_, _, fam)| *fam == target_family)
		.map(|(variant, _, _)| variant)
		.collect();

	// Deduplicate by resolution, keeping the lowest-bandwidth variant for each size.
	let mut by_resolution: HashMap<Option<Resolution>, &VariantStream> = HashMap::new();

	for variant in family_variants {
		let key = variant.resolution;
		let bandwidth = variant.average_bandwidth.unwrap_or(variant.bandwidth);

		use std::collections::hash_map::Entry;
		match by_resolution.entry(key) {
			Entry::Vacant(entry) => {
				entry.insert(variant);
			}
			Entry::Occupied(mut entry) => {
				let existing = entry.get();
				let existing_bw = existing.average_bandwidth.unwrap_or(existing.bandwidth);
				if bandwidth < existing_bw {
					entry.insert(variant);
				}
			}
		}
	}

	by_resolution.values().cloned().collect()
}

fn resolve_uri(base: &Url, value: &str) -> std::result::Result<Url, url::ParseError> {
	if let Ok(url) = Url::parse(value) {
		return Ok(url);
	}

	base.join(value)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hls_config_new_sets_fields() {
		let url = Url::parse("https://example.com/stream.m3u8").unwrap();
		let cfg = HlsConfig::new(url.clone(), 3, 0.5);
		assert_eq!(cfg.playlist, url);
		assert_eq!(cfg.preroll_segments, 3);
		assert!((cfg.refresh_ratio - 0.5).abs() < f32::EPSILON);
	}

	struct NoopFetcher;

	impl HlsFetcher for NoopFetcher {
		fn fetch_bytes(&self, _url: &Url) -> Pin<Box<dyn Future<Output = Result<Bytes>> + Send + '_>> {
			Box::pin(async { Err(Error::Hls("noop".to_string())) })
		}
	}

	#[test]
	fn hls_ingest_starts_without_importers() {
		let broadcast = BroadcastProducer::default();
		let url = Url::parse("https://example.com/master.m3u8").unwrap();
		let cfg = HlsConfig::new(url, 0, 1.0);
		let ingest: HlsIngest<NoopFetcher> = HlsIngest::new(broadcast, cfg, NoopFetcher);

		assert!(!ingest.has_video_importer());
		assert!(!ingest.has_audio_importer());
	}
}
