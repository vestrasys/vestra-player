import type { SourceIntent, SourceStamp, SourceState, SourceTicket } from "../source.ts";
import { Latest } from "./latest.ts";
import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Conditional, Disposable, Player, PlayerState } from "./types.ts";
/**
 * The key of `plugin.player.state` in the runtime's field tracking
 * (`Runtime.depends`). Its fields are the state's own, plus `stamp` for
 * what `isCurrent` and `Media.current` compare.
 */
export declare const PLAYER = "player";
export declare const rawOf: (state: PlayerState) => SourceState | undefined;
/** A step-seek length the player accepts: 0.1–3600 seconds in tenths. */
export declare const validStep: (seconds: unknown) => seconds is number;
export declare const sameStamp: (a: SourceStamp, b: SourceStamp) => boolean;
/** One per plugin: the state store and the operation book client. */
export declare class PlayerService {
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
export declare function player(scope: ScopeImpl): Player;
