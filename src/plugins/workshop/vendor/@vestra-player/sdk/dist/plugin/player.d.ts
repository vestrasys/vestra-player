import type { SourceIntent, SourceOperation, SourceStamp, SourceState, SourceTicket } from "../source.ts";
import { Latest } from "./latest.ts";
import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import { resolvedStream } from "./stream.ts";
import type { StreamSource } from "./stream.ts";
import type { AudioTrack, Conditional, Disposable, Player, PlayerState } from "./types.ts";
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
 * The state copy (a mirror of `source.state`) is started by the first use
 * of the state (a read of `plugin.player.state`, `isCurrent`, a media
 * handle, a scrub, a stream, a filter, a shader or view request, a
 * capture). It follows the player's events only while something holds
 * `source.state` (a stream, a `player.on` listener of it, a media
 * handle's `ready` or `onEnd`, a scrub; `Connection.hold`); otherwise it
 * is the newest snapshot the plugin has: its start-up read, its own calls
 * and the state a source event carries.
 */
export declare class PlayerService implements StreamSource {
    #private;
    readonly runtime: Runtime;
    raw: SourceState;
    state: PlayerState;
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
    /** Whether the mirror is registered yet. */
    get started(): boolean;
    /**
     * Register the SDK's copy of the state (a mirror of `source.state`; a
     * plugin listens through `watch`, `follow` and `player.on`). It hears
     * the events while something holds the event (`Connection.hold`), which
     * also reads the state once more after `read()` when the subscription
     * starts. Resolves at once.
     */
    start(): Promise<void>;
    /** The state as the player has it now, for a call that needs a source. */
    current(): Promise<PlayerState>;
    /** Drop anything older than what was already delivered. */
    accept(raw: SourceState): void;
    /**
     * A source event's data (`source.opening`, `source.ready`,
     * `source.ended`) as the listener gets it: `state` as a
     * {@link PlayerState}. The state of an opening or a ready is the
     * player's newest, so `plugin.player.state` takes it first (when it is
     * newer than what the plugin has): `isCurrent(state)` and `ifCurrent`
     * then hold after a wake too, without a read. An ended source's state
     * is not the player's current one and is not taken.
     */
    sourceEvent(event: string, data: unknown): unknown;
    /**
     * `watcher` hears each accepted state while the handle lives; it holds
     * `source.state` (`Connection.hold`). `always`: the state is read once
     * more when the subscription starts, also without `read()`.
     */
    watch(watcher: (raw: SourceState) => void, always?: boolean): Disposable;
    /**
     * `watch`, for `plugin.player.stream`: the copy hears `source.state` while
     * the watcher is open, and the plugin counts as listening to it
     * (`Connection.follow`; the stream reads the state itself).
     */
    follow(watcher: (raw: SourceState) => void, why: () => string): Disposable;
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
/**
 * `audio.tracks.changed`'s data as the listener gets it (`ScopeImpl.fact`,
 * SDK 1.11.0): the tracks as {@link PlayerState.audioTracks} lists them.
 */
export declare function audioTracksChanged(data: unknown): {
    sourceId: number;
    audioTracks: AudioTrack[];
};
export declare const playerOf: (runtime: Runtime) => PlayerService;
/**
 * The player's source as a stream needs it (`stream.ts`, L1): a stream
 * follows the source, so its state copy starts following the events.
 */
export declare function streamSource(runtime: Runtime): StreamSource;
/** A file resolver's stream (`resolvers.ts`), over {@link streamSource}. */
export declare const resolvedPlayerStream: (scope: ScopeImpl, stream: Parameters<typeof resolvedStream>[2], signal: AbortSignal) => Promise<import("../source-resolvers.ts").ResolvedProducer>;
export declare function player(scope: ScopeImpl): Player;
