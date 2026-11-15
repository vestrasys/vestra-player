import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { WireTimeline } from "../wire.ts";
import type { Timeline, TimelineMark, TimelineRange } from "./types.ts";
/**
 * The player's limits on one plugin's timeline layer
 * ({@link TimelineLayer}). A list over them fails with code `invalid`,
 * reported to `Scope.onError` (`set` and `setRanges` never throw). The
 * player's tests check that these agree with the player.
 *
 * @example Keep a long list of ranges inside the limit
 * The plugin publishes the first 1000 of its ranges; the player refuses a
 * longer list whole.
 * ```ts
 * import type { Plugin } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare const segments: { start: number; end: number }[];
 * // ---cut---
 * import { TIMELINE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * // A longer list would be refused whole.
 * plugin.timeline.layer().setRanges(
 *   segments
 *     .slice(0, TIMELINE_LIMITS.ranges)
 *     .map((s) => ({ ...s, style: "region" as const })),
 * );
 * ```
 *
 * @since 1.6.9
 *
 * @category Testing
 */
export declare const TIMELINE_LIMITS: Readonly<{
    /** Marks in one layer (`TimelineLayer.set`) (1000). */
    readonly marks: 1000;
    /** Ranges in one layer (`TimelineLayer.setRanges`) (1000; 100 before SDK 1.10.0). */
    readonly ranges: 1000;
    /** Characters of a mark's or a range's title, as shown (100). */
    readonly titleChars: 100;
}>;
/**
 * One per plugin: read by `plugin.player.read()`, then kept
 * current from `timeline.changed` (after `plugin.player.subscribe()`) once the plugin uses it
 * (or listens to that event with `player.on`).
 */
export declare class TimelineService {
    #private;
    marks: readonly TimelineMark[];
    ranges: readonly TimelineRange[];
    layer: boolean;
    constructor(runtime: Runtime);
    start(): void;
    accept(lists: WireTimeline): void;
    read(): Promise<void>;
}
export declare const timelineOf: (runtime: Runtime) => TimelineService;
export declare function timeline(scope: ScopeImpl): Timeline;
