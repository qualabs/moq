use std::future::Future;
use std::pin::Pin;

use anyhow::Context;
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
