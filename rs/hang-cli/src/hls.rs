use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use hang::{cmaf, moq_lite::BroadcastProducer};
use m3u8_rs::{Map, MediaPlaylist, MediaSegment};
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
	next_sequence: Option<u64>,
	init_ready: bool,
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
			next_sequence: None,
			init_ready: false,
		})
	}

	/// Fetch the latest playlist, download the init segment, and prime the importer with a buffer of segments.
	pub async fn prime(&mut self) -> Result<()> {
		let playlist = self.fetch_playlist().await?;
		self.ensure_init_segment(&playlist).await?;
		let buffered = self
			.consume_segments(&playlist, Some(self.cfg.preroll_segments))
			.await?;
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
			let playlist = self.fetch_playlist().await?;

			let wrote = self.consume_segments(&playlist, None).await?;
			let delay = self.refresh_delay(&playlist, wrote);

			debug!(wrote, delay = ?delay, "HLS ingest step complete");
			sleep(delay).await;
		}
	}

	async fn fetch_playlist(&self) -> Result<MediaPlaylist> {
		let response = self
			.client
			.get(self.cfg.playlist.clone())
			.send()
			.await
			.context("failed to fetch HLS playlist")?
			.error_for_status()
			.context("playlist request failed")?;

		let body = response.bytes().await.context("failed to read playlist body")?;
		let (_, playlist) = m3u8_rs::parse_media_playlist(&body).map_err(|err| anyhow!(err.to_string()))?;
		Ok(playlist)
	}

	async fn consume_segments(&mut self, playlist: &MediaPlaylist, limit: Option<usize>) -> Result<usize> {
		self.ensure_init_segment(playlist).await?;

		let mut consumed = 0usize;
		let mut sequence = playlist.media_sequence;
		let max = limit.unwrap_or(usize::MAX);

		for segment in &playlist.segments {
			if let Some(next) = self.next_sequence {
				if sequence < next {
					sequence += 1;
					continue;
				}
			}

			if consumed >= max {
				break;
			}

			self.push_segment(segment, sequence).await?;
			consumed += 1;
			sequence += 1;
		}

		if limit.is_none() && consumed == 0 {
			debug!("no fresh HLS segments available");
		}

		Ok(consumed)
	}

	async fn ensure_init_segment(&mut self, playlist: &MediaPlaylist) -> Result<()> {
		if self.init_ready {
			return Ok(());
		}

		let map = self
			.find_map(playlist)
			.ok_or_else(|| anyhow::anyhow!("playlist missing EXT-X-MAP"))?;

		let url = resolve_uri(&self.cfg.playlist, &map.uri).context("failed to resolve init segment URL")?;
		let bytes = self.fetch_bytes(url).await?;
		self.importer.parse(&bytes)?;

		self.init_ready = true;
		info!("loaded HLS init segment");
		Ok(())
	}

	async fn push_segment(&mut self, segment: &MediaSegment, sequence: u64) -> Result<()> {
		if segment.uri.is_empty() {
			bail!("encountered segment with empty URI");
		}

		let url = resolve_uri(&self.cfg.playlist, &segment.uri).context("failed to resolve segment URL")?;
		let bytes = self.fetch_bytes(url).await?;

		self.importer.parse(bytes.as_ref())?;
		self.next_sequence = Some(sequence + 1);

		Ok(())
	}

	fn find_map<'a>(&self, playlist: &'a MediaPlaylist) -> Option<&'a Map> {
		playlist.segments.iter().find_map(|segment| segment.map.as_ref())
	}

	fn refresh_delay(&self, playlist: &MediaPlaylist, wrote_segments: usize) -> Duration {
		let base = Duration::from_secs_f32(playlist.target_duration.max(0.5));
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

fn resolve_uri(base: &Url, value: &str) -> Result<Url> {
	if let Ok(url) = Url::parse(value) {
		return Ok(url);
	}

	base.join(value).context("failed to join URL")
}
