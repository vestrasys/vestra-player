import type { PlayerService } from "./player.ts";
import type { ScopeImpl } from "./scope.ts";
import type { ResolvedProducer } from "../source-resolvers.ts";
import type { ResolvedStream, Stream, StreamOptions } from "./types.ts";
export declare function openStream(scope: ScopeImpl, service: PlayerService, options: StreamOptions): Promise<Stream>;
/**
 * A file resolver's stream (`ResolvedStream`): prepared and prerolled while
 * the open waits, then handed to the host as the answer. Once the host
 * selected it, the start control is acknowledged and `started` gets the
 * running stream, synchronously, before any `onStop`. Cancelled before or
 * during activation (the open was replaced or closed, the plugin stopped),
 * the producer is released and `started` is never called.
 */
export declare function resolvedStream(scope: ScopeImpl, service: PlayerService, stream: ResolvedStream, signal: AbortSignal): Promise<ResolvedProducer>;
