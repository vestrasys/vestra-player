/**
 * `plugin.when`: conditions on the player's own state that the player
 * evaluates: a command's `enabled` and `checked`, and since 1.10.0 a
 * trigger's condition (`plugin.trigger().firesOn`). The wire form is
 * `commands.register` / `commands.update` `when` and `triggers.set` `when`.
 */
import type { Condition, LanguagePreference, PlayerState, ThemeId, When } from "./types.ts";
/** A numeric atom's interval: `min ≤ value < max`, at least one bound. */
export type IntervalJson = {
    min?: number;
    max?: number;
};
/** A position list in the wire form: `[start, end?]` pairs, and a lead. */
export type PositionJson = {
    ranges: ([number] | [number, number | null])[];
    lead?: number;
};
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
    duration: IntervalJson;
} | {
    volume: IntervalJson;
} | {
    rate: IntervalJson;
} | {
    audioTrack: number;
} | {
    subtitleTrack: string;
} | {
    audioDevice: string;
} | {
    muted: boolean;
} | {
    maximized: boolean;
} | {
    pip: boolean;
} | {
    looping: boolean;
} | {
    time: IntervalJson;
} | {
    position: PositionJson;
} | {
    all: ConditionJson[];
} | {
    any: ConditionJson[];
} | {
    not: ConditionJson;
};
/**
 * What conditions read, in the SDK's names. The fields added in 1.10.0 are
 * optional: missing, a fact reads as unknown (false).
 */
export interface ConditionFacts {
    phase: PlayerState["phase"];
    paused: boolean;
    kind: PlayerState["kind"];
    can: Readonly<Record<string, boolean>>;
    fullscreen: boolean;
    theme: string;
    language: string;
    /** Seconds; `null` while unknown. */
    duration?: number | null;
    volume?: number;
    rate?: number;
    muted?: boolean;
    audioTrack?: number | null;
    subtitleTrack?: string | null;
    audioDevice?: string | null;
    looping?: boolean;
    maximized?: boolean;
    pip?: boolean;
    /** The current source's id. */
    sourceId?: number;
    /** The position; `null` without one. */
    position?: number | null;
    /** The wall clock, Unix milliseconds. */
    time?: number;
    /** The source a position list was bound to (T12). */
    bound?: number | null;
}
/** Whether `value` is a condition made by `plugin.when`. */
export declare const isCondition: (value: unknown) => value is Condition;
/** A condition's wire form; throws for anything else. */
export declare function conditionJson(condition: Condition): ConditionJson;
/** Whether a wire condition reads the position or the clock (a trigger's only). */
export declare function readsPositionOrTime(json: ConditionJson): boolean;
/** How many position lists a wire condition has (at most one is allowed). */
export declare function positionLists(json: ConditionJson): number;
/** The one `plugin.when` (conditions hold no plugin state). */
export declare const WHEN: When;
/** One evaluation: whether it holds and, when its position list does, the range. */
export interface Truth {
    holds: boolean;
    /** The index of the range the position is in, when its position list holds. */
    range: number | null;
}
/**
 * The range of `list` the position `at` is in (`start − lead ≤ at < end`):
 * the earliest start when several are, as the player does.
 */
export declare function matchedRange(list: PositionJson, at: number): number | null;
/** Evaluate a wire condition as the player does, with its range (the test host). */
export declare function evaluateTruth(json: ConditionJson, facts: ConditionFacts): Truth;
/** Evaluate a wire condition as the player does (for the test host). */
export declare function evaluate(json: ConditionJson, facts: ConditionFacts): boolean;
