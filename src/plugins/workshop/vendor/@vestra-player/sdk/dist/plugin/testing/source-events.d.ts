/**
 * The discrete source events of the test host: `source.opening`,
 * `source.ready` and `source.ended`, derived from two consecutive source
 * states by the player's rules (player `src/host/src/source_events.rs`;
 * the cases of its tests are repeated in test/source-events.test.ts).
 *
 * A playthrough is one load of a source (`stamp.generation`, `cycle`); it
 * is in playback while `loading` or `ready`. One that leaves playback gets
 * `ended` (its last state: the state that says it ended, or the last one
 * before a replacement or a close); a new one gets `opening`, then
 * `ready`. A new cycle of the same source that is not loading is a seek
 * away from the end: it plays on, nothing opens. The test host's restart
 * goes to `ready` at once, so it says it is a new load (`reload`).
 *
 * Internal to the SDK; not exported from `@vestra-player/sdk/testing`.
 */
import type { SourceState } from "../../source.ts";
export type SourceEventName = "source.opening" | "source.ready" | "source.ended";
/** The events `next` adds to `previous`, in order. */
export declare function sourceEvents(previous: SourceState, next: SourceState, reload?: boolean): [SourceEventName, unknown][];
/**
 * `source.ended` with `reason: "quit"` for a source in playback (the
 * player quits; only a running plugin gets it).
 */
export declare function quitEvent(state: SourceState): {
    state: SourceState;
    reason: string;
} | null;
