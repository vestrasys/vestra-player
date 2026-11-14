import type { ScopeImpl } from "./scope.ts";
import type { ResolvedProducer } from "../source-resolvers.ts";
import type { SourceIntent, SourceState, SourceTicket } from "../source.ts";
import type { Wire } from "../wire.ts";
import type { ResolvedStream, Stream, StreamOptions } from "./types.ts";
/**
 * The player's source as a stream uses it: submit and close it, read it
 * again. `player.ts` passes its state copy (`PlayerService`), which follows
 * the source while a stream exists; this file depends only on this shape,
 * not on the copy (refactor plan §2.1).
 */
export interface StreamSource {
    readonly runtime: {
        readonly host: Wire;
        /** `plugin.player.read()` was called: the state is read after own actions. */
        readonly reads: boolean;
    };
    /** The newest source state seen. */
    readonly raw: SourceState;
    /** The same, as the plugin sees it (`error` in the player's words). */
    readonly state: {
        readonly error: string | null;
    };
    refresh(): Promise<unknown>;
    submit(intent: SourceIntent, options?: {
        accepted?: (ticket: SourceTicket) => void;
    }): Promise<SourceTicket>;
    closeIfCurrent(generation: number): Promise<void>;
}
export declare function openStream(scope: ScopeImpl, service: StreamSource, options: StreamOptions): Promise<Stream>;
/**
 * A file resolver's stream (`ResolvedStream`): prepared and prerolled while
 * the open waits, then handed to the host as the answer. Once the host
 * selected it, the start control is acknowledged and `started` gets the
 * running stream, synchronously, before any `onStop`. Cancelled before or
 * during activation (the open was replaced or closed, the plugin stopped),
 * the producer is released and `started` is never called.
 */
export declare function resolvedStream(scope: ScopeImpl, service: StreamSource, stream: ResolvedStream, signal: AbortSignal): Promise<ResolvedProducer>;
