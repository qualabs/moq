mod client;
mod hls;
mod import;
mod server;

use std::path::PathBuf;

use client::*;
use import::*;
use server::*;

use clap::{Args, Parser, Subcommand};
use hls::HlsConfig;
use url::Url;

#[derive(Parser, Clone)]
pub struct Cli {
	#[command(flatten)]
	log: moq_native::Log,

	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand, Clone)]
pub enum Command {
	Serve {
		#[command(flatten)]
		config: moq_native::ServerConfig,

		/// The name of the broadcast to serve.
		#[arg(long)]
		name: String,

		/// Optionally serve static files from the given directory.
		#[arg(long)]
		dir: Option<PathBuf>,

		/// The format of the input media.
		#[arg(long, value_enum, default_value_t = ImportType::Cmaf)]
		format: ImportType,
		#[command(flatten)]
		hls: HlsArgs,
	},
	Publish {
		/// The MoQ client configuration.
		#[command(flatten)]
		config: moq_native::ClientConfig,

		/// The URL of the MoQ server.
		///
		/// The URL must start with `https://` or `http://`.
		/// - If `http` is used, a HTTP fetch to "/certificate.sha256" is first made to get the TLS certificiate fingerprint (insecure).
		/// - If `https` is used, then A WebTransport connection is made via QUIC to the provided host/port.
		///
		/// The `?jwt=` query parameter is used to provide a JWT token from moq-token-cli.
		/// Otherwise, the public path (if any) is used instead.
		///
		/// The path currently must be `/` or you'll get an error on connect.
		#[arg(long)]
		url: Url,

		/// The name of the broadcast to publish.
		#[arg(long)]
		name: String,

		/// The format of the input media.
		#[arg(long, value_enum, default_value_t = ImportType::Cmaf)]
		format: ImportType,
		#[command(flatten)]
		hls: HlsArgs,
	},
}

#[derive(Args, Clone, Default)]
pub struct HlsArgs {
	/// The media playlist to ingest (required when --format hls).
	#[arg(long, value_name = "URL", required_if_eq("format", "hls"))]
	hls_url: Option<Url>,

	/// Number of segments to buffer before announcing the broadcast.
	#[arg(long, default_value_t = 3)]
	hls_preroll: usize,

	/// Fraction of target duration to wait after new data is ingested.
	#[arg(long, default_value_t = 0.5)]
	hls_refresh_ratio: f32,
}

impl HlsArgs {
	fn into_config(self) -> Option<HlsConfig> {
		self.hls_url
			.map(|url| HlsConfig::new(url, self.hls_preroll, self.hls_refresh_ratio))
	}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let cli = Cli::parse();
	cli.log.init();

	match cli.command {
		Command::Serve {
			config,
			dir,
			name,
			format,
			hls,
		} => server(config, name, dir, format, hls.into_config(), &mut tokio::io::stdin()).await,
		Command::Publish {
			config,
			url,
			name,
			format,
			hls,
		} => client(config, url, name, format, hls.into_config(), &mut tokio::io::stdin()).await,
	}
}
