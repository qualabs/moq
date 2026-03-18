import type { Announced } from "../announced.ts";
import type { Broadcast } from "../broadcast.ts";
import type * as Path from "../path.ts";

// Both moq-lite and moq-ietf implement this.
export interface Established {
	readonly url: URL;

	/** The transport layer that was used to establish this connection. */
	readonly transport: "webtransport" | "websocket";

	announced(prefix?: Path.Valid): Announced;
	publish(path: Path.Valid, broadcast: Broadcast): void;
	consume(broadcast: Path.Valid): Broadcast;
	close(): void;
	closed: Promise<void>;
}
