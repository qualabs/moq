use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use hang::{cmaf, moq_lite::BroadcastProducer};
use m3u8_rs::{
	AlternativeMedia, AlternativeMediaType, Map, MasterPlaylist, MediaPlaylist, MediaSegment, VariantStream,
};
use reqwest::Client;
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

/// Configuration for the single-rendition HLS ingest loop.
#[derive(Clone)]
pub struct HlsConfig {
	pub playlist: Url,
	pub preroll_segments: usize,
	pub refresh_ratio: f32,
}

impl HlsConfig {
	pub fn new(playlist: Url, preroll_segments: usize, refresh_ratio: f32) -> Self {
		Self {
			playlist,
			preroll_segments,
			refresh_ratio,
		}
	}
}

/// Pulls an HLS media playlist and feeds the bytes into the CMAF importer.
pub struct HlsImporter {
	importer: cmaf::Import,
	client: Client,
	cfg: HlsConfig,
	video: Option<TrackState>,
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

impl HlsImporter {
	pub fn new(broadcast: BroadcastProducer, cfg: HlsConfig) -> Result<Self> {
		let client = Client::builder()
			.user_agent("hang-hls-ingest/0.1")
			.build()
			.context("failed to build HTTP client")?;

		Ok(Self {
			importer: cmaf::Import::new(broadcast),
			client,
			cfg,
			video: None,
			audio: None,
		})
	}

	/// Fetch the latest playlist, download the init segment, and prime the importer with a buffer of segments.
	pub async fn prime(&mut self) -> Result<()> {
		self.ensure_tracks().await?;

		let mut buffered = 0usize;

		if let Some(url) = self.video.as_ref().map(|track| track.playlist.clone()) {
			let playlist = self.fetch_media_playlist(&url).await?;
			if let Some(mut track) = self.video.take() {
				let count = self
					.consume_segments(&mut track, &playlist, Some(self.cfg.preroll_segments))
					.await;
				self.video = Some(track);
				buffered += count?;
			}
		}

		if let Some(url) = self.audio.as_ref().map(|track| track.playlist.clone()) {
			let playlist = self.fetch_media_playlist(&url).await?;
			if let Some(mut track) = self.audio.take() {
				let count = self
					.consume_segments(&mut track, &playlist, Some(self.cfg.preroll_segments))
					.await;
				self.audio = Some(track);
				buffered += count?;
			}
		}

		if buffered == 0 {
			warn!("HLS playlist had no new segments during prime step");
		} else {
			info!(count = buffered, "buffered initial HLS segments");
		}
		Ok(())
	}

	/// Run the ingest loop until cancelled.
	pub async fn run(&mut self) -> Result<()> {
		loop {
			self.ensure_tracks().await?;

			let mut wrote = 0usize;
			let mut target_duration = None;

			if let Some(url) = self.video.as_ref().map(|track| track.playlist.clone()) {
				let playlist = self.fetch_media_playlist(&url).await?;
				target_duration = Some(playlist.target_duration);
				if let Some(mut track) = self.video.take() {
					let count = self.consume_segments(&mut track, &playlist, None).await;
					self.video = Some(track);
					wrote += count?;
				}
			}

			if let Some(url) = self.audio.as_ref().map(|track| track.playlist.clone()) {
				let playlist = self.fetch_media_playlist(&url).await?;
				if target_duration.is_none() {
					target_duration = Some(playlist.target_duration);
				}
				if let Some(mut track) = self.audio.take() {
					let count = self.consume_segments(&mut track, &playlist, None).await;
					self.audio = Some(track);
					wrote += count?;
				}
			}

			let delay = self.refresh_delay(target_duration, wrote);

			debug!(wrote, delay = ?delay, "HLS ingest step complete");
			sleep(delay).await;
		}
	}

	async fn fetch_media_playlist(&self, url: &Url) -> Result<MediaPlaylist> {
		let body = self.fetch_bytes(url.clone()).await?;
		let (_, playlist) = m3u8_rs::parse_media_playlist(&body).map_err(|err| anyhow!(err.to_string()))?;
		Ok(playlist)
	}

	async fn ensure_tracks(&mut self) -> Result<()> {
		if self.video.is_some() {
			return Ok(());
		}

		let body = self.fetch_bytes(self.cfg.playlist.clone()).await?;
		if let Ok((_, master)) = m3u8_rs::parse_master_playlist(&body) {
			if let Some(variant) = select_variant(&master) {
				let video_url =
					resolve_uri(&self.cfg.playlist, &variant.uri).context("failed to resolve video rendition URL")?;
				self.video = Some(TrackState::new("video", video_url));

				if let Some(group_id) = variant.audio.as_deref() {
					if let Some(audio_tag) = select_audio(&master, group_id) {
						if let Some(uri) = &audio_tag.uri {
							let audio_url = resolve_uri(&self.cfg.playlist, uri)
								.context("failed to resolve audio rendition URL")?;
							self.audio = Some(TrackState::new("audio", audio_url));
						} else {
							warn!(%group_id, "audio rendition missing URI");
						}
					} else {
						warn!(%group_id, "audio group not found in master playlist");
					}
				}

				let audio_url = self.audio.as_ref().map(|a| a.playlist.to_string());
				if let Some(selected) = self.video.as_ref() {
					info!(
						video = %selected.playlist,
						audio = audio_url.as_deref().unwrap_or("none"),
						bandwidth = variant.bandwidth,
						"selected master playlist renditions"
					);
				}
				return Ok(());
			}
		}

		// Fallback: treat the provided URL as a media playlist.
		self.video = Some(TrackState::new("video", self.cfg.playlist.clone()));
		Ok(())
	}

	async fn consume_segments(
		&mut self,
		track: &mut TrackState,
		playlist: &MediaPlaylist,
		limit: Option<usize>,
	) -> Result<usize> {
		self.ensure_init_segment(track, playlist).await?;

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

			self.push_segment(track, segment, sequence).await?;
			consumed += 1;
			sequence += 1;
		}

		if limit.is_none() && consumed == 0 {
			debug!(track = track.label, "no fresh HLS segments available");
		}

		Ok(consumed)
	}

	async fn ensure_init_segment(&mut self, track: &mut TrackState, playlist: &MediaPlaylist) -> Result<()> {
		if track.init_ready {
			return Ok(());
		}

		let map = self
			.find_map(playlist)
			.ok_or_else(|| anyhow::anyhow!("playlist missing EXT-X-MAP"))?;

		let url = resolve_uri(&track.playlist, &map.uri).context("failed to resolve init segment URL")?;
		let bytes = self.fetch_bytes(url).await?;
		self.importer.parse(&bytes)?;

		track.init_ready = true;
		info!(track = track.label, "loaded HLS init segment");
		Ok(())
	}

	async fn push_segment(&mut self, track: &mut TrackState, segment: &MediaSegment, sequence: u64) -> Result<()> {
		if segment.uri.is_empty() {
			bail!("encountered segment with empty URI");
		}

		let url = resolve_uri(&track.playlist, &segment.uri).context("failed to resolve segment URL")?;
		let bytes = self.fetch_bytes(url).await?;

		self.importer.parse(bytes.as_ref())?;
		track.next_sequence = Some(sequence + 1);

		Ok(())
	}

	fn find_map<'a>(&self, playlist: &'a MediaPlaylist) -> Option<&'a Map> {
		playlist.segments.iter().find_map(|segment| segment.map.as_ref())
	}

	fn refresh_delay(&self, target_duration: Option<f32>, wrote_segments: usize) -> Duration {
		let base = target_duration
			.map(|dur| Duration::from_secs_f32(dur.max(0.5)))
			.unwrap_or_else(|| Duration::from_millis(500));
		if wrote_segments == 0 {
			return base;
		}

		base.mul_f32(self.cfg.refresh_ratio)
	}

	async fn fetch_bytes(&self, url: Url) -> Result<Bytes> {
		let response = self
			.client
			.get(url.clone())
			.send()
			.await
			.with_context(|| format!("failed to download {}", url))?
			.error_for_status()
			.with_context(|| format!("request for {} failed", url))?;

		response.bytes().await.context("failed to read segment body")
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

fn select_variant<'a>(master: &'a MasterPlaylist) -> Option<&'a VariantStream> {
	master
		.variants
		.iter()
		.filter(|variant| !variant.is_i_frame && !variant.uri.is_empty())
		.min_by_key(|variant| variant.average_bandwidth.unwrap_or(variant.bandwidth))
		.or_else(|| master.variants.iter().find(|variant| !variant.uri.is_empty()))
}

fn resolve_uri(base: &Url, value: &str) -> Result<Url> {
	if let Ok(url) = Url::parse(value) {
		return Ok(url);
	}

	base.join(value).context("failed to join URL")
}
