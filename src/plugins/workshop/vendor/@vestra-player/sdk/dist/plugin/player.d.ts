import type { SourceIntent, SourceStamp, SourceState, SourceTicket } from "../source.ts";
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
 * from the plugin's start when the manifest declares `events`
 * (`createPlugin` subscribes before the start-up read, as before 1.7.0, so
 * a read in any later handler is current). Without `events` no event can
 * arrive; `start` is then called by the first use of the state (a read of
 * `plugin.player.state`, `onChange`, `isCurrent`, a media handle, a
 * scrub, a stream, a filter, a shader or view request, a capture) and
 * subscribes nothing. A start on demand with `events` and `playback.read`
 * would read the state again; `createPlugin` makes it unreachable today.
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
    init(): Promise<void>;
    /** Whether the mirror follows the player's events yet. */
    get started(): boolean;
    /**
     * Follow `source.state` from now on: the SDK's copy of the state (a
     * mirror; a plugin listens through `watch` and the bindings that read
     * `state`). Then read the state once more (with `playback.read` and
     * `events`): a change between the start-up read and the subscription
     * sent no event to this plugin. Resolves when that read is in.
     * `reread: false`: subscribed before the start-up read
     * (`createPlugin`), which then is that read.
     */
    start(reread?: boolean): Promise<void>;
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
