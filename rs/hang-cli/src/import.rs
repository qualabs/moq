use anyhow::Context;
use clap::ValueEnum;
use hang::moq_lite::BroadcastProducer;
use tokio::io::AsyncRead;

use crate::hls::{HlsConfig, HlsImporter};

#[derive(ValueEnum, Clone, Copy, PartialEq, Eq)]
pub enum ImportType {
	AnnexB,
	Cmaf,
	Hls,
}

pub enum Import {
	AnnexB(hang::annexb::Import),
	Cmaf(Box<hang::cmaf::Import>),
	Hls(Box<HlsImporter>),
}

impl Import {
	pub fn new(broadcast: BroadcastProducer, format: ImportType, hls: Option<HlsConfig>) -> anyhow::Result<Self> {
		let import = match format {
			ImportType::AnnexB => Self::AnnexB(hang::annexb::Import::new(broadcast)),
			ImportType::Cmaf => Self::Cmaf(Box::new(hang::cmaf::Import::new(broadcast))),
			ImportType::Hls => Self::Hls(Box::new(HlsImporter::new(
				broadcast,
				hls.context("--hls-url is required when format=hls")?,
			)?)),
		};

		Ok(import)
	}
}

impl Import {
	pub async fn init_from<T: AsyncRead + Unpin>(&mut self, input: Option<&mut T>) -> anyhow::Result<()> {
		match self {
			Self::AnnexB(_import) => {}
			Self::Cmaf(import) => {
				let reader = input.context("media input is required for CMAF format")?;
				import.init_from(reader).await.context("failed to parse CMAF headers")?;
			}
			Self::Hls(import) => import.prime().await?,
		};

		Ok(())
	}

	pub async fn read_from<T: AsyncRead + Unpin>(&mut self, input: Option<&mut T>) -> anyhow::Result<()> {
		match self {
			Self::AnnexB(import) => {
				let reader = input.context("media input is required for AnnexB format")?;
				import.read_from(reader).await.map_err(anyhow::Error::from)?
			}
			Self::Cmaf(import) => {
				let reader = input.context("media input is required for CMAF format")?;
				import.read_from(reader).await.map_err(anyhow::Error::from)?
			}
			Self::Hls(import) => import.run().await?,
		};

		Ok(())
	}
}
