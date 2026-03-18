/**
 * `<hang-telemetry>` — web component that sends metrics to an OpenTelemetry collector.
 *
 * The SDK is loaded only when a valid `endpoint` attribute is present,
 * so bundle size is zero when the element is not used.
 *
 * Attributes:
 *   endpoint         OTLP HTTP URL, e.g. "http://localhost:4318"    (required)
 *   service-name     identifies your app in dashboards              (default: "moq-client")
 *   service-version  app version label                              (optional)
 *   session-id       unique id per player, used to filter one viewer (auto-generated)
 *   interval         how often to send metrics, in ms               (default: 30000)
 *
 * Usage:
 *   ```html
 *   import "@moq/hang/telemetry/element";
 *   <hang-telemetry endpoint="http://localhost:4318" service-name="my-app"></hang-telemetry>
 *   ```
 */

import { clearProvider, setProvider } from "./bridge.ts";

/** All metric names, descriptions, and units in one place. Change here, change everywhere. */
const METRICS = {
    connections: { name: "moq_client_connections_total", description: "Total MoQ connections opened." },
    activeConnections: { name: "moq_client_active_connections", description: "Currently active MoQ connections." },
    startupTime: { name: "moq_client_startup_time_seconds", description: "Time from subscription start to first decoded frame.", unit: "s" },
    framesDecoded: { name: "moq_client_frames_decoded_total", description: "Total frames decoded by VideoDecoder / AudioDecoder." },
    bytesReceived: { name: "moq_client_bytes_received_total", description: "Compressed bytes received from the relay.", unit: "By" },
    bitrate: { name: "moq_client_bitrate_bytes_per_second", description: "Estimated receive bitrate.", unit: "By/s" },
    stalls: { name: "moq_client_stall_total", description: "Number of video playback stall entries." },
};

// The library name shown in dashboards under "instrumentation scope".
const INSTRUMENTATION_SCOPE = "@moq/hang";

const OBSERVED = ["endpoint", "service-name", "service-version", "session-id", "interval"];
type Observed = (typeof OBSERVED)[number];

class HangTelemetry extends HTMLElement {
    static readonly observedAttributes = OBSERVED;

    #meterProvider: { shutdown(): Promise<void> } | undefined;
    // Keep the same session id across attribute changes so metrics stay grouped.
    #sessionId: string | undefined;
    // Run teardown and setup one at a time, never in parallel.
    #chain: Promise<void> = Promise.resolve();
    // Prevent multiple restarts when several attributes change at the same time.
    #restartPending = false;

    connectedCallback(): void {
        this.#restart();
    }

    disconnectedCallback(): void {
        this.#chain = this.#chain.then(() => this.#teardown());
    }

    attributeChangedCallback(_name: Observed, oldValue: string | null, newValue: string | null): void {
        if (oldValue === newValue)
            return;
        if (this.isConnected)
            this.#restart();
    }

    #restart(): void {
        if (this.#restartPending)
            return;
        this.#restartPending = true;
        // Wait for all synchronous attribute changes to finish before restarting once.
        queueMicrotask(() => {
            this.#restartPending = false;
            if (!this.isConnected)
                return;
            this.#chain = this.#chain
                .then(() => this.#teardown())
                .then(() => this.#setup())
                .catch((err) => console.error("[hang-telemetry]", err));
        });
    }

    async #setup(): Promise<void> {
        const endpoint = this.getAttribute("endpoint");
        if (!endpoint)
            return;

        const serviceName = this.getAttribute("service-name") ?? "moq-client";
        const serviceVersion = this.getAttribute("service-version");
        this.#sessionId ??= this.getAttribute("session-id") ?? crypto.randomUUID();
        // Minimum 1s to avoid sending too many requests to the collector.
        const intervalMs = Math.max(1000, Number(this.getAttribute("interval") ?? "30000"));

        // All three modules are needed together — fail fast if any import fails.
        const [
            { MeterProvider, PeriodicExportingMetricReader, ExplicitBucketHistogramAggregation, View },
            { OTLPMetricExporter, AggregationTemporalityPreference },
            { Resource },
        ] = await Promise.all([
            import("@opentelemetry/sdk-metrics"),
            import("@opentelemetry/exporter-metrics-otlp-http"),
            import("@opentelemetry/resources"),
        ]);

        const resourceAttrs: Record<string, string> = {
            "service.name": serviceName,
            "moq.player.session_id": this.#sessionId,
        };

        if (serviceVersion)
            resourceAttrs["service.version"] = serviceVersion;

        const exporter = new OTLPMetricExporter({
            url: `${endpoint}/v1/metrics`,
            temporalityPreference: AggregationTemporalityPreference.CUMULATIVE,
        });

        const reader = new PeriodicExportingMetricReader({ exporter, exportIntervalMillis: intervalMs });

        // Finer buckets for startup time (50ms–60s).
        // The SDK defaults ([0, 5, 10, …, 10000]) are too coarse for sub-second values.
        const startupBuckets = new ExplicitBucketHistogramAggregation(
            [0.05, 0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 5, 10, 30, 60],
        );

        const meterProvider = new MeterProvider({
            resource: new Resource(resourceAttrs),
            readers: [reader],
            views: [new View({ aggregation: startupBuckets, instrumentName: METRICS.startupTime.name })],
        });

        // Use getMeter() directly on the provider, not via the global API.
        // The global API is a singleton that breaks if called more than once per page.
        const meter = meterProvider.getMeter(INSTRUMENTATION_SCOPE);

        // Pass each METRICS entry as the options object — the extra `name` field is ignored by the SDK.
        const connections = meter.createCounter(METRICS.connections.name, METRICS.connections);
        const activeConnections = meter.createUpDownCounter(METRICS.activeConnections.name, METRICS.activeConnections);
        const startupTime = meter.createHistogram(METRICS.startupTime.name, METRICS.startupTime);
        const framesDecoded = meter.createCounter(METRICS.framesDecoded.name, METRICS.framesDecoded);
        const bytesReceived = meter.createCounter(METRICS.bytesReceived.name, METRICS.bytesReceived);
        const stallCount = meter.createCounter(METRICS.stalls.name, METRICS.stalls);

        // The gauge reads bitrateState on each export cycle instead of receiving push updates.
        const bitrateState = new Map<string, { ema: number; lastTime: number; attrs: Record<string, string> }>();
        const bitrateGauge = meter.createObservableGauge(METRICS.bitrate.name, METRICS.bitrate);
        bitrateGauge.addCallback((result) => {
            for (const { ema, attrs } of bitrateState.values())
                result.observe(ema, attrs);
        });

        // Send data immediately for important events, but no more than once every 2s.
        // High-frequency events (frames, bytes) skip this — the periodic reader handles them.
        let lastFlush = 0;
        const tryFlush = () => {
            const now = Date.now();
            if (now - lastFlush > 2000) {
                lastFlush = now;
                meterProvider.forceFlush().catch((err) => console.error("[hang-telemetry] flush:", err));
            }
        };

        setProvider({
            recordConnection(transport, attrs) {
                connections.add(1, { transport, ...attrs });
                activeConnections.add(1, { transport, ...attrs });
                tryFlush();
            },
            recordDisconnect(transport, attrs) {
                activeConnections.add(-1, { transport, ...attrs });
                tryFlush();
            },
            recordStartupTime(ms, attrs) {
                startupTime.record(ms / 1000, attrs);
                tryFlush();
            },
            recordFrameDecoded(count, attrs) {
                framesDecoded.add(count, attrs);
            },
            recordBytesReceived(bytes, trackType, attrs) {
                const finalAttrs = { track_type: trackType, ...attrs };
                bytesReceived.add(bytes, finalAttrs);

                const now = performance.now();
                const key = `${trackType}:${JSON.stringify(attrs ?? {})}`;
                const last = bitrateState.get(key) ?? { ema: 0, lastTime: now, attrs: finalAttrs };
                const dt = (now - last.lastTime) / 1000;
                if (dt > 0.01) {
                    bitrateState.set(key, { ema: last.ema * 0.7 + (bytes / dt) * 0.3, lastTime: now, attrs: finalAttrs });
                }
            },
            recordStall(count, attrs) {
                stallCount.add(count, attrs);
                tryFlush();
            },
        });

        this.#meterProvider = meterProvider;
    }

    async #teardown(): Promise<void> {
        if (!this.#meterProvider) return;
        clearProvider();
        await this.#meterProvider.shutdown().catch((err) => console.error("[hang-telemetry] shutdown:", err));
        this.#meterProvider = undefined;
    }
}

customElements.define("hang-telemetry", HangTelemetry);

export default HangTelemetry;

declare global {
    interface HTMLElementTagNameMap {
        "hang-telemetry": HangTelemetry;
    }
}
