use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct ObservabilityConfig {
	/// OpenTelemetry OTLP endpoint (default: http://localhost:4317)
	#[arg(
		long = "otel-endpoint",
		env = "OTEL_EXPORTER_OTLP_ENDPOINT",
		default_value = "http://localhost:4317"
	)]
	#[serde(default = "default_otlp_endpoint")]
	pub otlp_endpoint: String,

	// Note: qlog configuration is handled by ServerConfig.qlog (moq-native::QlogConfig)
	// to avoid duplicate command-line arguments
}

fn default_otlp_endpoint() -> String {
	"http://localhost:4317".to_string()
}

impl Default for ObservabilityConfig {
	fn default() -> Self {
		Self {
			otlp_endpoint: default_otlp_endpoint(),
		}
	}
}
