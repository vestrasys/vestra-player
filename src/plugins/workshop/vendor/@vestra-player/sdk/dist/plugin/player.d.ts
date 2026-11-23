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
    /**
     * Register the SDK's copy of the state (a mirror of `source.state`; a
     * plugin listens through `watch`, `follow` and `player.on`). It hears
     * the events while something holds the event (`Connection.hold`), which
     * also reads the state once more after `read()` when the subscription
     * starts. Resolves at once.
     */
    start(): Promise<void>;
    /**
     * The state as the player has it now, for a call that acts on the
     * current state (a capture): the copy as it is while it follows the
     * player's events, else read first ({@link fresh}).
     */
    current(): Promise<PlayerState>;
    /**
     * Wait until the copy is the player's current state, for a call that
     * acts on it (a control, a step, a capture, a scrub). While something
     * holds `source.state` the copy follows the player's events: nothing to
     * wait for. Otherwise it is the newest snapshot the plugin has, which an
     * admitted intent since (a pause, a seek, a new file, by anyone) made
     * stale: it is read first. Calls in a burst share one read ({@link inTurn}
     * keeps their order). A read that fails leaves the copy as it was; the
     * call goes on with it.
     */
    fresh(): Promise<void> | undefined;
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
    /**
     * A fact event's fields of the state: taken into the copy
     * when `belongs` holds for it, as `accept` takes a state (a precise
     * pass for the fields that changed); its event sequence is kept, so the
     * next `source.state` is accepted as before.
     */
    patch(belongs: (raw: SourceState) => boolean, fields: Partial<SourceState>): void;
    watch(watcher: (raw: SourceState) => void, always?: boolean): Disposable;
    /**
     * `watch` for the SDK's own handles that send their values again when the
     * source changes (the video view, a shader channel, a processing node):
     * the copy hears `source.state` while the handle lives, but the plugin is
     * not counted as listening to it (`Connection.track`). The handle's own
     * resource is what keeps the plugin running.
     */
    track(watcher: (raw: SourceState) => void): Disposable;
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
    /**
     * Send a call in its turn among the plugin's calls on the source: at
     * once (in the same tick) when no read is on its way, else right after
     * that read, with the calls that waited for it in the order they were
     * made. `read`: the call acts on the current state, so a read starts
     * first when the copy does not follow the player ({@link fresh}).
     */
    inTurn<T>(send: () => Promise<T>, read?: boolean): Promise<T>;
    /**
     * `send` with the stamp a call sends, in its turn ({@link inTurn}):
     * `ifCurrent`'s when given (as it is), else the current state's, read
     * first when the copy does not follow the player.
     */
    withStamp<T>(ifCurrent: PlayerState | undefined, send: (stamp: {
        expected?: SourceStamp;
    }) => Promise<T>): Promise<T>;
    control(intent: SourceIntent, options?: Conditional): Promise<void>;
    /** Close only if `generation` is still the player's source. */
    closeIfCurrent(generation: number): Promise<void>;
}
/**
 * `audio.tracks.changed`'s data as the listener gets it (`ScopeImpl.fact`,
 * SDK 1.11.0): the tracks as {@link PlayerState.audioTracks} lists them.
 * The copy takes them first, when they are of its source.
 */
export declare function audioTracksChanged(runtime: Runtime, data: unknown): {
    sourceId: number;
    audioTracks: AudioTrack[];
};
/**
 * `audio.devices.changed`'s data as the listener gets it:
 * as the player sent it; the copy takes `requested` and `active` into
 * {@link PlayerState.audioOutput} first.
 */
export declare function audioDevicesChanged(runtime: Runtime, data: unknown): unknown;
export declare const playerOf: (runtime: Runtime) => PlayerService;
/**
 * The player's source as a stream needs it (`stream.ts`, L1): a stream
 * follows the source, so its state copy starts following the events.
 */
export declare function streamSource(runtime: Runtime): StreamSource;
/** A file resolver's stream (`resolvers.ts`), over {@link streamSource}. */
export declare const resolvedPlayerStream: (scope: ScopeImpl, stream: Parameters<typeof resolvedStream>[2], signal: AbortSignal) => Promise<import("../source-resolvers.ts").ResolvedProducer>;
export declare function player(scope: ScopeImpl): Player;
