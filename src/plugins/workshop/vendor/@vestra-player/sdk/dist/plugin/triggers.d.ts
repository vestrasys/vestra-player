import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Trigger } from "./types.ts";
/**
 * The player's limits on triggers (`plugin.trigger`): how many a plugin
 * declares and the player keeps conditions for, and how long a name is.
 * A 65th trigger throws `limit`; the player refuses a 65th kept condition
 * with `limit` too. A position list holds at most `TIMELINE_LIMITS.ranges`
 * ranges.
 *
 * @example Stay within the limit
 * A plugin that makes one trigger per item of a list keeps the list
 * within the number of triggers the player allows.
 * ```ts
 * import { TRIGGER_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const names = ["sleep", "wake"].slice(0, TRIGGER_LIMITS.triggers);
 * ```
 *
 * @since 1.10.0
 *
 * @category Testing
 */
export declare const TRIGGER_LIMITS: Readonly<{
    /** Triggers a plugin declares, and conditions the player keeps for it. */
    triggers: 64;
    /** Characters of a trigger's name. */
    nameChars: 64;
}>;
/** One plugin's triggers (a runtime service). */
export declare class TriggerRegistry {
    #private;
    readonly runtime: Runtime;
    constructor(runtime: Runtime);
    /** `activate()` begins: triggers may be declared until `end`. */
    begin(): void;
    /**
     * The synchronous part of `activate()` is over: the player learns what
     * was declared (only when something was), and what came early is
     * delivered.
     */
    end(): void;
    /**
     * `plugin.trigger(name?)` of `scope`. Only the plugin itself declares
     * triggers, not a sub-scope (`plugin.scope()`): a trigger's condition is
     * the plugin's, kept by the player beyond any one run, while a sub-scope
     * is disposed within a run, and a kept condition whose functions went
     * with it would go on waking the plugin for nothing.
     */
    declare(scope: ScopeImpl, name?: string): Trigger;
    /** Nobody handles `trigger`'s notifications (T14): the player drops its condition. */
    unhandled(trigger: number | string): void;
}
/** The triggers of `runtime`'s plugin. */
export declare const triggersOf: (runtime: Runtime) => TriggerRegistry;
