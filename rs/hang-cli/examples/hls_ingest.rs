use clap::Parser;
use hang::cmaf::hls::HlsIngest;
use hang::moq_lite::{self, Broadcast, Origin};
use url::Url;

#[derive(Parser, Debug, Clone)]
struct Args {
	#[command(flatten)]
	log: moq_native::Log,

	#[command(flatten)]
	client: moq_native::ClientConfig,

	/// The MoQ relay URL to publish into (ex: http://localhost:4443/anon)
	#[arg(long)]
	publish_url: Url,

	/// Broadcast name that viewers will subscribe to.
	#[arg(long)]
	name: String,

	/// URL to the HLS media playlist you want to ingest.
	#[arg(long)]
	playlist_url: Url,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	args.log.init();

	let broadcast = Broadcast::produce();
	let client = args.client.clone().init()?;

	tracing::info!(url = %args.publish_url, name = %args.name, "connecting to relay");
	let connection = client.connect(args.publish_url.clone()).await?;

	let origin = Origin::produce();
	let session = moq_lite::Session::connect(connection, origin.consumer, None).await?;

	origin.producer.publish_broadcast(&args.name, broadcast.consumer);
	let ingest = HlsIngest::new(broadcast.producer, args.playlist_url.clone());

	tokio::select! {
		res = ingest.start() => res,
		res = session.closed() => res.map_err(Into::into),
		_ = tokio::signal::ctrl_c() => {
			tracing::info!("Ctrl+C received; closing session");
			session.close(moq_lite::Error::Cancel);
			Ok(())
		}
	}
}
