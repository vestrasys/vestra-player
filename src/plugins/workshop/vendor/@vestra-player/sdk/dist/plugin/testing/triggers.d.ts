import type { Json } from "../../wire.ts";
import { matchedRange } from "../when.ts";
import type { ConditionFacts, ConditionJson } from "../when.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/**
 * One kept trigger condition, as `TestHost.triggers` lists it.
 *
 * @since 1.10.0
 * @category Testing
 */
export interface KeptTrigger {
    /** The trigger: its name, or its place among the unnamed ones. */
    readonly trigger: number | string;
    /** The condition in the wire form. */
    readonly when: Json;
    /** Its sequence number. */
    readonly seq: number;
    /** It holds now ("entered"). */
    readonly entered: boolean;
}
interface Kept {
    trigger: number | string;
    when: ConditionJson;
    seq: number;
    /** The source its position list belongs to. */
    bound: number | null;
    /** Entered, with the range it is in. */
    entered: {
        range: number | null;
    } | null;
    /** The run that gave it. */
    run: number;
}
/** What the player keeps for the plugin's triggers. */
declare class TriggerState {
    readonly kept: Map<string, Kept>;
    seq: number;
    /** The run (process generation): a park, a wake and an upgrade start one. */
    run: number;
    /** How many unnamed triggers this version declared. */
    unnamed: number | null;
    /** The virtual wall clock (Unix ms). */
    now: number;
    parked: boolean;
    /** The wake events the plugin listened to when it was parked. */
    wakes: Set<string>;
    /** What waits for the next start: (merge key, event, data). */
    backlog: [string, string, unknown][];
    waking: Promise<void> | undefined;
    /** The backlog is delivered now (at the woken plugin's ready). */
    flushing: boolean;
    /** Starts the plugin again (`startPlugin`); `entry`: other code (an upgrade). */
    start: ((entry?: (plugin: never) => unknown) => Promise<void>) | undefined;
    /** Stops the running plugin with a reason. */
    stop: ((reason: string) => Promise<void>) | undefined;
    /** Evaluating: no evaluation inside another. */
    busy: boolean;
}
export declare function triggerState(host: TestHost): TriggerState;
/** The facts the player's conditions read, from the host. */
export declare function conditionFacts(host: TestHost, state?: TriggerState): ConditionFacts;
/** Evaluates `only` (or every kept condition) and sends what flipped (T1, T10). */
export declare function evaluateTriggers(host: TestHost, inner: Internals, only?: Kept): void;
/**
 * `emit` of `event`: the trigger conditions are evaluated first for the
 * player's own state changes; a wake event for a parked plugin that listened
 * to it is held and wakes it (`true`: not delivered now).
 */
export declare function hearEvent(host: TestHost, inner: Internals, event: string, data: unknown): boolean;
/** `TestHost.triggers`. */
export declare function keptTriggers(host: TestHost): KeptTrigger[];
/** `TestHost.advance`: the clock and, while playing, the position move by `ms`. */
export declare function advance(host: TestHost, inner: Internals, ms: number): Promise<void>;
/** `TestHost.park`: the player puts the plugin in standby. */
export declare function park(host: TestHost, _inner: Internals): Promise<void>;
/** `TestHost.upgrade`: another version replaces the plugin (§4.2). */
export declare function upgrade(host: TestHost, _inner: Internals, entry?: (plugin: never) => unknown): Promise<void>;
/** The default handlers of this area's wire methods, by method name. */
export declare function triggersHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
/** `matchedRange` for tests of the host's position lists. */
export { matchedRange };
