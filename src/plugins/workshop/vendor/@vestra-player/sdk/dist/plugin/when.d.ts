/**
 * `plugin.when`: conditions on the player's own state that the player evaluates
 * for a command's `enabled` and `checked`. The wire form is `commands.register`
 * / `commands.update` `when`.
 */
import type { Condition, LanguagePreference, PlayerState, ThemeId, When } from "./types.ts";
/** A condition in the wire form: an object of exactly one key. */
export type ConditionJson = {
    phase: PlayerState["phase"][];
} | {
    paused: boolean;
} | {
    kind: PlayerState["kind"];
} | {
    can: keyof PlayerState["can"];
} | {
    fullscreen: boolean;
} | {
    theme: ThemeId;
} | {
    language: LanguagePreference;
} | {
    all: ConditionJson[];
} | {
    any: ConditionJson[];
} | {
    not: ConditionJson;
};
/** What conditions read, in the SDK's names. */
export interface ConditionFacts {
    phase: PlayerState["phase"];
    paused: boolean;
    kind: PlayerState["kind"];
    can: Readonly<Record<string, boolean>>;
    fullscreen: boolean;
    theme: string;
    language: string;
}
/** Whether `value` is a condition made by `plugin.when`. */
export declare const isCondition: (value: unknown) => value is Condition;
/** A condition's wire form; throws for anything else. */
export declare function conditionJson(condition: Condition): ConditionJson;
/** The one `plugin.when` (conditions hold no plugin state). */
export declare const WHEN: When;
/** Evaluate a wire condition as the player does (for the test host). */
export declare function evaluate(json: ConditionJson, facts: ConditionFacts): boolean;
