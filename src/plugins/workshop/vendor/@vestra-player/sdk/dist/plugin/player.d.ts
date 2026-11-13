import type { SourceIntent, SourceOperation, SourceStamp, SourceState, SourceTicket } from "../source.ts";
import { Latest } from "./latest.ts";
import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import { resolvedStream } from "./stream.ts";
import type { StreamSource } from "./stream.ts";
import type { Conditional, Disposable, Player, PlayerState } from "./types.ts";
/**
 * The key of `plugin.player.state` in the runtime's field tracking
 * (`Runtime.depends`). Its fields are the state's own, plus `stamp` for
 * what `isCurrent` and `Media.current` compare.
 */
export declare const PLAYER = "player";
export declare const rawOf: (state: PlayerState) => SourceState | undefined;
export declare const sameStamp: (a: SourceStamp, b: SourceStamp) => boolean;
/**
 * One per plugin: the state store and the operation book client.
 *
 * The state copy (a mirror of `source.state`) follows the player's events
 * from `plugin.player.subscribe()` on (`subscribe`, first in `activate`
 * as the plugin writes it). Before that no event can arrive; `start` is
 * then called by the first use of the state (a read of
 * `plugin.player.state`, `onChange`, `isCurrent`, a media handle, a
 * scrub, a stream, a filter, a shader or view request, a capture) and
 * subscribes nothing until `subscribe()`, which subscribes it and, after
 * `read()`, reads the state once more (`#catchUp`).
 */
export declare class PlayerService implements StreamSource {
    #private;
    readonly runtime: Runtime;
    raw: SourceState;
    state: PlayerState;
    /**
     * The fields the last accepted state changed (`onChange` listeners are
     * told so a listener that read none of them needs no full pass).
     */
    change: ReadonlyMap<string, ReadonlySet<string>>;
    readonly output: Latest<{
        volume?: number;
        muted?: boolean;
    }>;
    constructor(runtime: Runtime);
    /**
     * `plugin.player.read()`: read the source state and the timeline now, and
     * from now on read the state again after the plugin's own actions
     * (`Connection.reads`). Idempotent: one read, however often it is called;
     * a read that failed is tried again by the next call.
     */
    read(): Promise<void>;
    /**
     * `plugin.player.subscribe()`: turn event delivery on and follow
     * `source.state` now. Idempotent.
     */
    subscribe(): void;
    /** Whether the mirror follows the player's events yet. */
    get started(): boolean;
    /**
     * Follow `source.state` from now on: the SDK's copy of the state (a
     * mirror; a plugin listens through `watch` and the bindings that read
     * `state`). Then read the state once more (after `read()` and
     * `subscribe()`): a change between that read and the subscription sent
     * no event to this plugin. Resolves when that read is in.
     */
    start(): Promise<void>;
    /** The state as the player has it now, for a call that needs a source. */
    current(): Promise<PlayerState>;
    /** Drop anything older than what was already delivered. */
    accept(raw: SourceState): void;
    watch(watcher: (raw: SourceState) => void): Disposable;
    refresh(): Promise<PlayerState>;
    /**
     * Submit an intent and wait for its definite outcome. A reply from the host
     * is final. Only a lost reply makes the outcome uncertain; then the same
     * operation id is queried (the pipe is ordered, so the host has already
     * admitted or rejected it). "pending" means accepted and still running.
     */
    submit(intent: SourceIntent, options?: {
        signal?: AbortSignal;
        accepted?: (ticket: SourceTicket) => void;
        /**
         * Send the submit another way under this service's operation id
         * (`media.sources.open`, which the player turns into this intent's
         * `source.submit`); followed with `source.operation` as any submit.
         */
        send?: (operationId: number) => Promise<SourceOperation>;
    }): Promise<SourceTicket>;
    expected(state: PlayerState | undefined): {
        expected?: SourceStamp;
    };
    control(intent: SourceIntent, options?: Conditional): Promise<void>;
    /** Close only if `generation` is still the player's source. */
    closeIfCurrent(generation: number): Promise<void>;
}
export declare const playerOf: (runtime: Runtime) => PlayerService;
/**
 * The player's source as a stream needs it (`stream.ts`, L1): a stream
 * follows the source, so its state copy starts following the events.
 */
export declare function streamSource(runtime: Runtime): StreamSource;
/** A file resolver's stream (`resolvers.ts`), over {@link streamSource}. */
export declare const resolvedPlayerStream: (scope: ScopeImpl, stream: Parameters<typeof resolvedStream>[2], signal: AbortSignal) => Promise<import("../source-resolvers.ts").ResolvedProducer>;
export declare function player(scope: ScopeImpl): Player;
