/**
 * Telemetry bridge for @moq/hang.
 *
 * This module has zero dependencies.
 * It defines a small interface that any metrics backend can implement.
 * All calls are silent no-ops until a backend is registered with {@link setProvider}.
 *
 * Example with a custom backend:
 *
 *   import * as Bridge from "@moq/hang/telemetry/bridge";
 *
 *   Bridge.setProvider({
 *     recordConnection(transport) { myMetrics.increment(`connections.${transport}`); },
 *     recordStartupTime(ms)       { myMetrics.timing("startup_ms", ms); },
 *     recordFrameDecoded(count)   { myMetrics.count("frames_decoded", count); },
 *     recordBytesReceived(n, t)   { myMetrics.count(`bytes.${t}`, n); },
 *     recordStall()               { myMetrics.increment("stalls"); },
 *     recordDisconnect()          { myMetrics.decrement("active_connections"); },
 *   });
 */

/** Any metrics backend must implement this interface. */
export interface TelemetryProvider {
	/** Called when a session is established. */
	recordConnection(transport: "webtransport" | "websocket", attrs?: Record<string, string>): void;

	/** Called when the first frame is decoded. `ms` is time since playback started. */
	recordStartupTime(ms: number, attrs?: Record<string, string>): void;

	/** Called for each decoded frame. */
	recordFrameDecoded(count: number, attrs?: Record<string, string>): void;

	/** Called for each chunk of compressed data received. */
	recordBytesReceived(bytes: number, trackType: "video" | "audio", attrs?: Record<string, string>): void;

	/** Called once when video playback starts buffering. Not called every frame during the stall. */
	recordStall(count: number, attrs?: Record<string, string>): void;

	/** Called when a session closes. */
	recordDisconnect(transport: "webtransport" | "websocket", attrs?: Record<string, string>): void;
}

let provider: TelemetryProvider | undefined;

// Events that arrive before a provider is registered are held here and replayed on setProvider().
// Connection and startup events keep their order (they are rare and order matters).
// High-frequency events (frames, bytes, stalls) are aggregated by attribute key so
// the buffer never grows unbounded.

type PendingEvent =
	| { type: "connection"; transport: "webtransport" | "websocket"; attrs?: Record<string, string> }
	| { type: "startupTime"; ms: number; attrs?: Record<string, string> };

type CountEntry = { count: number; attrs: Record<string, string> | undefined };
type BytesEntry = { bytes: number; trackType: "video" | "audio"; attrs: Record<string, string> | undefined };

// Safety cap: avoid unbounded growth if many connections happen before a provider loads.
const MAX_PENDING = 50;
const pending: PendingEvent[] = [];

const pendingFrames = new Map<string, CountEntry>();
const pendingBytes = new Map<string, BytesEntry>();
const pendingStalls = new Map<string, CountEntry>();

// Produces a stable string key from an attribute map so {b,a} and {a,b} give the same key.
const toKey = (attrs?: Record<string, string>): string =>
	JSON.stringify(attrs ? Object.fromEntries(Object.entries(attrs).sort()) : null);

/**
 * Register a backend. Any events buffered before this call are replayed immediately.
 */
export function setProvider(p: TelemetryProvider): void {
	provider = p;

	for (const ev of pending.splice(0)) {
		if (ev.type === "connection") p.recordConnection(ev.transport, ev.attrs);
		else p.recordStartupTime(ev.ms, ev.attrs);
	}

	for (const { count, attrs } of pendingFrames.values()) p.recordFrameDecoded(count, attrs);
	pendingFrames.clear();

	for (const { bytes, trackType, attrs } of pendingBytes.values()) p.recordBytesReceived(bytes, trackType, attrs);
	pendingBytes.clear();

	for (const { count, attrs } of pendingStalls.values()) p.recordStall(count, attrs);
	pendingStalls.clear();
}

/** Remove the current backend. All subsequent calls become no-ops. */
export function clearProvider(): void {
	provider = undefined;
}

/** Return the active backend, or `undefined` if none is registered. */
export function getProvider(): TelemetryProvider | undefined {
	return provider;
}

/** Record a new session. `transport` is `"webtransport"` (QUIC) or `"websocket"` (fallback). */
export function recordConnection(transport: "webtransport" | "websocket", attrs?: Record<string, string>): void {
	if (provider) {
		provider.recordConnection(transport, attrs);
	} else if (pending.length < MAX_PENDING) {
		pending.push({ type: "connection", transport, attrs });
	}
}

/** Record time-to-first-frame. `ms` is milliseconds from subscription start to first output. */
export function recordStartupTime(ms: number, attrs?: Record<string, string>): void {
	if (provider) {
		provider.recordStartupTime(ms, attrs);
	} else if (pending.length < MAX_PENDING) {
		pending.push({ type: "startupTime", ms, attrs });
	}
}

/** Record decoded frames. Call once per frame from the decoder output callback. */
export function recordFrameDecoded(count = 1, attrs?: Record<string, string>): void {
	if (provider) {
		provider.recordFrameDecoded(count, attrs);
		return;
	}
	const key = toKey(attrs);
	const entry = pendingFrames.get(key);
	if (entry)
		entry.count += count;
	else
		pendingFrames.set(key, { count, attrs });
}

/** Record compressed bytes received. Pass the raw byte length before decoding. */
export function recordBytesReceived(bytes: number, trackType: "video" | "audio", attrs?: Record<string, string>): void {
	if (provider) {
		provider.recordBytesReceived(bytes, trackType, attrs);
		return;
	}
	const key = `${trackType}:${toKey(attrs)}`;
	const entry = pendingBytes.get(key);
	if (entry)
		entry.bytes += bytes;
	else
		pendingBytes.set(key, { bytes, trackType, attrs });
}

/** Record a stall. Call once when buffering starts, not on every frame during the stall. */
export function recordStall(count = 1, attrs?: Record<string, string>): void {
	if (provider) {
		provider.recordStall(count, attrs);
		return;
	}
	const key = toKey(attrs);
	const entry = pendingStalls.get(key);
	if (entry)
		entry.count += count;
	else
		pendingStalls.set(key, { count, attrs });
}

/**
 * Record that a session has closed.
 * Disconnects are not buffered: if no backend is registered the event is dropped.
 */
export function recordDisconnect(transport: "webtransport" | "websocket", attrs?: Record<string, string>): void {
	provider?.recordDisconnect(transport, attrs);
}

