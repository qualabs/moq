use std::collections::HashSet;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::process::Command as ProcessCommand;
use std::str::FromStr;

use anyhow::{bail, Context};
use bytes::Bytes;
use hang::hls::{HlsFetcher, HlsIngest};
use hang::{moq_lite::BroadcastProducer, Error as HangError, Result as HangResult};
use reqwest::Client;
use tokio::time::sleep;
use tracing::debug;
use url::Url;

/// Re-export the core HLS configuration so CLI code can build it directly.
pub use hang::hls::HlsConfig;

/// HTTP client implementation of `HlsFetcher` using `reqwest`.
struct ReqwestHlsFetcher {
	client: Client,
}

impl HlsFetcher for ReqwestHlsFetcher {
	fn fetch_bytes(&self, url: &Url) -> Pin<Box<dyn Future<Output = HangResult<Bytes>> + Send + '_>> {
		let client = self.client.clone();
		let url = url.clone();

		Box::pin(async move {
			let response = client
				.get(url.clone())
				.send()
				.await
				.map_err(|err| HangError::Hls(format!("failed to download {url}: {err}")))?;

			let response = response
				.error_for_status()
				.map_err(|err| HangError::Hls(format!("request for {url} failed: {err}")))?;

			response
				.bytes()
				.await
				.map_err(|err| HangError::Hls(format!("failed to read segment body from {url}: {err}")))
		})
	}
}

/// Pulls an HLS media playlist and feeds the bytes into the CMAF importer.
///
/// This is a thin CLI wrapper around the core `hang::hls::HlsIngest` type,
/// responsible only for wiring up the HTTP client and running the ingest loop.
pub struct HlsImporter {
	ingest: HlsIngest<ReqwestHlsFetcher>,
}

impl HlsImporter {
	pub fn new(broadcast: BroadcastProducer, cfg: HlsConfig) -> anyhow::Result<Self> {
		let client = Client::builder()
			.user_agent("hang-hls-ingest/0.1")
			.build()
			.context("failed to build HTTP client")?;

		let fetcher = ReqwestHlsFetcher { client };
		let ingest = HlsIngest::new(broadcast, cfg, fetcher);

		Ok(Self { ingest })
	}

	/// Fetch the latest playlist, download the init segment, and prime
	/// the importer with a buffer of segments.
	pub async fn prime(&mut self) -> anyhow::Result<()> {
		let buffered = self.ingest.prime().await.map_err(anyhow::Error::from)?;
		if buffered == 0 {
			debug!("HLS playlist had no new segments during prime step");
		}
		Ok(())
	}

	/// Run the ingest loop until cancelled.
	pub async fn run(&mut self) -> anyhow::Result<()> {
		loop {
			let outcome = self.ingest.step().await.map_err(anyhow::Error::from)?;
			let delay = self
				.ingest
				.refresh_delay(outcome.target_duration, outcome.wrote_segments);

			debug!(
				wrote = outcome.wrote_segments,
				delay = ?delay,
				"HLS ingest step complete"
			);

			sleep(delay).await;
		}
	}
}

/// Parsed resolution from `--hls-resolution WIDTHxHEIGHT`.
#[derive(Clone, Debug)]
pub struct HlsResolution {
	pub width: u32,
	pub height: u32,
}

impl FromStr for HlsResolution {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut parts = s.split('x');
		let width = parts
			.next()
			.ok_or_else(|| "missing width".to_string())?
			.parse::<u32>()
			.map_err(|_| "invalid width".to_string())?;
		let height = parts
			.next()
			.ok_or_else(|| "missing height".to_string())?
			.parse::<u32>()
			.map_err(|_| "invalid height".to_string())?;
		if parts.next().is_some() {
			return Err("too many components in resolution".to_string());
		}

		Ok(Self { width, height })
	}
}

/// Run `ffprobe` on the HLS URL to list available video renditions and let the
/// user interactively choose which resolutions to ingest. The chosen
/// resolutions are written back into `resolutions` so they can be applied to
/// `HlsConfig` afterwards.
pub fn run_ffprobe_and_select(url: &Url, resolutions: &mut Vec<HlsResolution>) -> anyhow::Result<()> {
	let mut cmd = ProcessCommand::new("ffprobe");
	cmd.arg("-v")
		.arg("error")
		.arg("-select_streams")
		.arg("v")
		.arg("-show_entries")
		.arg("stream=width,height,codec_name,bit_rate")
		.arg("-of")
		.arg("csv=p=0")
		// Limit how much media ffprobe reads from live HLS so it terminates
		// quickly even for infinite streams.
		.arg("-read_intervals")
		.arg("0%+1")
		.arg(url.as_str());

	let output = cmd.output().context("failed to run ffprobe")?;
	if !output.status.success() {
		eprintln!("ffprobe failed:\n{}", String::from_utf8_lossy(&output.stderr));
		eprint!("ffprobe failed, continue without preview? [y/N]: ");
		io::stdout().flush().ok();

		let mut line = String::new();
		io::stdin().read_line(&mut line)?;
		if !line.trim().to_lowercase().starts_with('y') {
			bail!("aborted by user after ffprobe failure");
		}

		return Ok(());
	}

	let stdout = String::from_utf8_lossy(&output.stdout);

	// First parse all rows from ffprobe.
	let mut rows: Vec<(u32, u32, String, String)> = Vec::new();
	for line in stdout.lines() {
		let parts: Vec<_> = line.split(',').collect();
		if parts.len() < 3 {
			continue;
		}

		let width: u32 = parts[0].parse().unwrap_or(0);
		let height: u32 = parts[1].parse().unwrap_or(0);
		let codec = parts[2].to_string();
		let bitrate = parts.get(3).unwrap_or(&"").to_string();
		rows.push((width, height, codec, bitrate));
	}

	// Deduplicate by (width, height) so ladders with many variants at the
	// same resolution don't overwhelm the menu. The ingest pipeline only
	// cares about resolution; per-resolution variant selection happens later.
	let mut seen = HashSet::new();
	let mut renditions: Vec<(usize, u32, u32, String, String)> = Vec::new();
	for (width, height, codec, bitrate) in rows {
		let key = (width, height);
		if seen.insert(key) {
			let idx = renditions.len();
			renditions.push((idx, width, height, codec, bitrate));
		}
	}

	if renditions.is_empty() {
		eprintln!("No video renditions discovered by ffprobe");
		return Ok(());
	}

	eprintln!("Available video renditions from ffprobe:");
	for (idx, width, height, codec, bitrate) in &renditions {
		eprintln!("[{idx}] {width}x{height} {codec} {bitrate}");
	}

	eprintln!("Enter a comma-separated list of indexes to ingest (empty = all, q = abort): ");
	io::stdout().flush().ok();
	let mut line = String::new();
	io::stdin().read_line(&mut line)?;

	let input = line.trim().to_lowercase();
	if input == "q" {
		bail!("aborted by user");
	}

	// If empty, keep existing behavior (all resolutions).
	if input.is_empty() {
		resolutions.clear();
		return Ok(());
	}

	let mut selected = Vec::new();
	for token in input.split(',') {
		let token = token.trim();
		if token.is_empty() {
			continue;
		}
		let idx: usize = token
			.parse()
			.map_err(|_| anyhow::anyhow!("invalid index: {token}"))?;
		let (_, width, height, _, _) = renditions
			.iter()
			.find(|(i, _, _, _, _)| *i == idx)
			.ok_or_else(|| anyhow::anyhow!("no rendition at index {idx}"))?;
		selected.push(HlsResolution {
			width: *width,
			height: *height,
		});
	}

	*resolutions = selected;

	Ok(())
}
