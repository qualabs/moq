// qlog support for QUIC connections
// qlog is enabled via quinn's qlog feature flag

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// qlog configuration
#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QlogConfig {
	/// Enable qlog output
	#[arg(long = "qlog-enabled", env = "MOQ_QLOG_ENABLED")]
	#[serde(default)]
	pub enabled: bool,
	/// Directory for qlog files
	#[arg(long = "qlog-dir", env = "MOQ_QLOG_DIR", default_value = "./qlog")]
	pub dir: PathBuf,
	/// Sampling rate (0.0-1.0)
	#[arg(long = "qlog-sample-rate", env = "MOQ_QLOG_SAMPLE_RATE", default_value = "0.1")]
	pub sample_rate: f64,
}

impl Default for QlogConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			dir: PathBuf::from("./qlog"),
			sample_rate: 0.1, // 10% sampling by default
		}
	}
}

impl QlogConfig {
	/// Create qlog file path for a connection
	pub fn file_path(&self, connection_id: &str) -> PathBuf {
		self.dir.join(connection_id).join("trace.json")
	}

	/// Check if qlog should be enabled for this connection (based on sampling)
	pub fn should_log(&self) -> bool {
		if !self.enabled {
			return false;
		}
		// Simple sampling: use connection_id hash
		// TODO: Implement proper sampling logic
		true
	}
}

/// Configure qlog for a quinn connection
/// TODO: Implement actual qlog configuration once quinn API is verified
pub fn configure_qlog(
	_connection: &quinn::Connection,
	_config: &QlogConfig,
	_connection_id: &str,
) -> anyhow::Result<()> {
	// TODO: Use quinn's qlog API to configure logging
	// Example (needs verification):
	// connection.set_qlog(
	//     Box::new(File::create(config.file_path(connection_id))?),
	//     "moq-relay".to_string(),
	//     format!("Connection {}", connection_id),
	// );
	Ok(())
}
