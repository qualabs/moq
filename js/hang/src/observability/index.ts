/**
 * OpenTelemetry observability for MoQ client
 *
 * Provides metrics, traces, and events aligned with CMCD vocabulary
 * (mapping only, not wire format)
 */

import { metrics, trace, context, propagation } from "@opentelemetry/api";
import type { Meter, Counter, Histogram, Tracer } from "@opentelemetry/api";
import { MeterProvider, PeriodicExportingMetricReader } from "@opentelemetry/sdk-metrics";
import { OTLPMetricExporter } from "@opentelemetry/exporter-metrics-otlp-http";
import { WebTracerProvider } from "@opentelemetry/sdk-trace-web";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http";
import { BatchSpanProcessor } from "@opentelemetry/sdk-trace-web";
import { Resource } from "@opentelemetry/resources";

let tracer: Tracer | undefined;
let meter: Meter | undefined;
let initialized = false;
let sessionId: string | undefined;

export interface ObservabilityConfig {
	/** OTLP endpoint URL (default: http://localhost:4318) */
	otlpEndpoint?: string;
	/** Service name (default: moq-client) */
	serviceName?: string;
	/** Enable observability (default: true if endpoint provided) */
	enabled?: boolean;
	/** Per-player session id (defaults to a random UUID) */
	sessionId?: string;
}

function createSessionId(): string {
	try {
		return globalThis.crypto?.randomUUID?.() ?? `sid-${Math.random().toString(16).slice(2)}-${Date.now()}`;
	} catch {
		return `sid-${Math.random().toString(16).slice(2)}-${Date.now()}`;
	}
}

/**
 * Initialize OpenTelemetry SDK for browser
 */
export function initObservability(config: ObservabilityConfig = {}): void {
	if (initialized) {
		return;
	}

	const endpoint = config.otlpEndpoint || "http://localhost:4318";
	const serviceName = config.serviceName || "moq-client";
	const enabled = config.enabled ?? !!config.otlpEndpoint;
	sessionId = config.sessionId || createSessionId();

	if (!enabled) {
		console.log("[Observability] Disabled - no endpoint configured");
		return;
	}

	try {
		const resource = new Resource({
			"service.name": serviceName,
			"service.instance.id": sessionId,
			"moq.player.session_id": sessionId,
		});

		// Common headers for OTLP exporters (no auth to avoid CORS credentials issues)
		const exporterHeaders = {
			"Content-Type": "application/json",
		};

		// Initialize trace provider
		const traceExporter = new OTLPTraceExporter({
			url: `${endpoint}/v1/traces`,
			headers: exporterHeaders,
		});
		const tracerProvider = new WebTracerProvider({ resource });
		tracerProvider.addSpanProcessor(new BatchSpanProcessor(traceExporter));
		tracerProvider.register();
		tracer = trace.getTracer(serviceName);

		// Initialize meter provider with periodic export
		const metricExporter = new OTLPMetricExporter({
			url: `${endpoint}/v1/metrics`,
			headers: exporterHeaders,
		});

		const metricReader = new PeriodicExportingMetricReader({
			exporter: metricExporter,
			exportIntervalMillis: 10000, // Export every 10 seconds
		});

		const meterProvider = new MeterProvider({
			resource,
			readers: [metricReader],
		});

		// Register the meter provider globally
		metrics.setGlobalMeterProvider(meterProvider);
		meter = metrics.getMeter(serviceName);

		// Initialize client metrics instance with actual instruments
		clientMetricsInstance = new ClientMetrics(meter, metricExporter);

		console.log(`[Observability] Metrics will export every 10s to ${endpoint}/v1/metrics`);

		// Setup connection type tracking
		setupConnectionTracking();

		console.log(`[Observability] Initialized with endpoint: ${endpoint}`);
		initialized = true;
	} catch (error) {
		console.warn("Failed to initialize OpenTelemetry:", error);
	}
}

/**
 * Setup connection type tracking from the lite module
 */
function setupConnectionTracking() {
	// Dynamically import to avoid circular dependencies
	import("@moq/lite").then((Moq) => {
		if (Moq.Connection?.onConnectionType) {
			Moq.Connection.onConnectionType((type: "webtransport" | "websocket") => {
				getClientMetrics()?.recordConnection(type, sessionId ? { "moq.player.session_id": sessionId } : undefined);
			});
			console.log("[Observability] Connection type tracking enabled");
		}
	}).catch((err) => {
		console.debug("[Observability] Connection tracking not available:", err);
	});
}

/**
 * Get the tracer instance
 */
export function getTracer(): Tracer | undefined {
	return tracer;
}

/**
 * Get the meter instance
 */
export function getMeter(): Meter | undefined {
	return meter;
}

/**
 * Generate W3C trace context for WebTransport CONNECT headers
 */
export function generateTraceContext(): { traceparent: string; tracestate?: string } {
	if (!tracer) {
		return { traceparent: "" };
	}

	// Create a new span for the connection
	const span = tracer.startSpan("webtransport.connect");
	const ctx = trace.setSpan(context.active(), span);

	// Extract trace context
	const carrier: { traceparent?: string; tracestate?: string } = {};
	propagation.inject(ctx, carrier);

	span.end();

	return {
		traceparent: carrier.traceparent || "",
		tracestate: carrier.tracestate,
	};
}

/**
 * Client metrics aligned with CMCD vocabulary
 *
 * Includes both MoQ-agnostic client experience metrics and
 * media-specific metrics that belong in the hang layer (not the relay).
 */
export class ClientMetrics {
	// Client experience metrics (CMCD-aligned)
	private bufferLengthHistogram?: Histogram;
	private latencyHistogram?: Histogram;
	private startupTimeHistogram?: Histogram;
	private bitrateGauge?: Histogram;
	private rebufferCounter?: Counter;
	private qualitySwitchCounter?: Counter;
	private connectionCounter?: Counter;

	// Media-specific metrics (hang layer - not relay)
	// These track decode/render performance which is media-aware
	private framesDecodedCounter?: Counter;
	private framesDroppedCounter?: Counter;
	private keyframeIntervalHistogram?: Histogram;
	private decodeTimeHistogram?: Histogram;
	private avSyncDriftHistogram?: Histogram;

	private exporter?: OTLPMetricExporter;

	constructor(meter?: Meter, exporter?: OTLPMetricExporter) {
		this.exporter = exporter;
		if (meter) {
			// Client experience metrics (CMCD-aligned)
			this.bufferLengthHistogram = meter.createHistogram("moq_client_buffer_length_seconds", {
				description: "Client buffer length in seconds (CMCD bl)",
				unit: "s",
			});

			this.latencyHistogram = meter.createHistogram("moq_client_latency_seconds", {
				description: "Client latency to live edge in seconds (CMCD dl)",
				unit: "s",
			});

			this.startupTimeHistogram = meter.createHistogram("moq_client_startup_time_seconds", {
				description: "Time to first frame in seconds (CMCD st)",
				unit: "s",
			});

			this.bitrateGauge = meter.createHistogram("moq_client_bitrate_bps", {
				description: "Current playback bitrate in bits per second (CMCD br)",
				unit: "bps",
			});

			this.rebufferCounter = meter.createCounter("moq_client_rebuffer_count_total", {
				description: "Number of rebuffer events (CMCD rt)",
			});

			this.qualitySwitchCounter = meter.createCounter("moq_client_quality_switches_total", {
				description: "Number of quality/rendition switches",
			});

			this.connectionCounter = meter.createCounter("moq_client_connections_total", {
				description: "Total client connections by transport type",
			});

			// Media-specific metrics (hang layer)
			this.framesDecodedCounter = meter.createCounter("moq_client_frames_decoded_total", {
				description: "Total video frames successfully decoded",
			});

			this.framesDroppedCounter = meter.createCounter("moq_client_frames_dropped_total", {
				description: "Video frames dropped (decode failure, late arrival, or congestion)",
			});

			this.keyframeIntervalHistogram = meter.createHistogram("moq_client_keyframe_interval_seconds", {
				description: "Time between keyframes (IDR frames)",
				unit: "s",
			});

			this.decodeTimeHistogram = meter.createHistogram("moq_client_decode_time_seconds", {
				description: "Video frame decode latency",
				unit: "s",
			});

			this.avSyncDriftHistogram = meter.createHistogram("moq_client_av_sync_drift_seconds", {
				description: "Audio/video synchronization drift (positive = video ahead)",
				unit: "s",
			});
		}
	}

	recordBufferLength(seconds: number, attributes?: Record<string, string>): void {
		this.bufferLengthHistogram?.record(seconds, attributes);
	}

	recordStartupTime(seconds: number, attributes?: Record<string, string>): void {
		this.startupTimeHistogram?.record(seconds, attributes);
	}

	recordBitrate(bps: number, attributes?: Record<string, string>): void {
		this.bitrateGauge?.record(bps, attributes);
	}

	incrementRebuffer(attributes?: Record<string, string>): void {
		this.rebufferCounter?.add(1, attributes);
	}

	/**
	 * Record a connection event with transport type
	 */
	recordConnection(transportType: "webtransport" | "websocket", attributes?: Record<string, string>): void {
		this.connectionCounter?.add(1, { transport: transportType, ...attributes });
		console.log(`[Observability] Connection recorded: ${transportType}`);
	}

	recordLatency(seconds: number, attributes?: Record<string, string>): void {
		this.latencyHistogram?.record(seconds, attributes);
	}

	incrementQualitySwitch(attributes?: Record<string, string>): void {
		this.qualitySwitchCounter?.add(1, attributes);
	}

	// ============================================================
	// Media-specific metrics (hang layer - decode/render aware)
	// ============================================================

	/**
	 * Record a successfully decoded video frame
	 */
	recordFrameDecoded(attributes?: Record<string, string>): void {
		this.framesDecodedCounter?.add(1, attributes);
	}

	/**
	 * Record multiple decoded frames at once (batch reporting)
	 */
	recordFramesDecoded(count: number, attributes?: Record<string, string>): void {
		if (count > 0) {
			this.framesDecodedCounter?.add(count, attributes);
		}
	}

	/**
	 * Record a dropped video frame
	 */
	recordFrameDropped(attributes?: Record<string, string>): void {
		this.framesDroppedCounter?.add(1, attributes);
	}

	/**
	 * Record multiple dropped frames at once (batch reporting)
	 */
	recordFramesDropped(count: number, attributes?: Record<string, string>): void {
		if (count > 0) {
			this.framesDroppedCounter?.add(count, attributes);
		}
	}

	/**
	 * Record the interval between keyframes
	 */
	recordKeyframeInterval(seconds: number, attributes?: Record<string, string>): void {
		this.keyframeIntervalHistogram?.record(seconds, attributes);
	}

	/**
	 * Record video frame decode time
	 */
	recordDecodeTime(seconds: number, attributes?: Record<string, string>): void {
		this.decodeTimeHistogram?.record(seconds, attributes);
	}

	/**
	 * Record audio/video sync drift
	 * Positive values mean video is ahead of audio
	 * Negative values mean audio is ahead of video
	 */
	recordAvSyncDrift(seconds: number, attributes?: Record<string, string>): void {
		this.avSyncDriftHistogram?.record(seconds, attributes);
	}

	/**
	 * Force flush metrics to the exporter
	 */
	async flush(): Promise<void> {
		try {
			await this.exporter?.forceFlush?.();
		} catch (error) {
			console.warn("Failed to flush metrics:", error);
		}
	}
}

/**
 * Get or create client metrics instance
 */
let clientMetricsInstance: ClientMetrics | undefined;

export function getClientMetrics(): ClientMetrics | undefined {
	if (!clientMetricsInstance) {
		// Return a no-op instance if not initialized
		clientMetricsInstance = new ClientMetrics();
	}
	return clientMetricsInstance;
}

/**
 * Shutdown observability (call on page unload)
 */
export async function shutdownObservability(): Promise<void> {
	if (clientMetricsInstance) {
		await clientMetricsInstance.flush();
	}
}

/**
 * Helper for recording metrics without repetitive dynamic imports.
 * Use this instead of `import("...observability").then(...)` pattern.
 *
 * @example
 * ```ts
 * import { recordMetric } from "../../observability";
 * recordMetric((m) => m.recordFrameDecoded({ codec, track_type: "video" }));
 * ```
 */
export function recordMetric(fn: (metrics: ClientMetrics) => void): void {
	const metrics = getClientMetrics();
	if (metrics) {
		fn(metrics);
	}
}
