import type { PlayerState, Scope } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { type Reason, sourceFailure, type State } from "./texts.ts";

export const UNSUPPORTED = "Not supported by the current source";
export const NO_VIDEO = "No video";

/**
 * B08 Previous Keyframe / Next Keyframe through `player.step("keyframe", …)`. Whether the
 * source has video is asked once per source when it first becomes ready
 * (`stats()`, ruling R-P1) and cached; until the answer arrives the commands
 * stay disabled. One step at a time: repeats while one is in flight are dropped,
 * so releasing a held key stops at once.
 */
export class Keyframes {
  busy = false;
  /** `stats().video !== null` for the source with this id. */
  #video: { sourceId: number; video: boolean } | null = null;
  #asking: number | null = null;
  readonly #scope: Scope;
  readonly #playback: Playback;
  readonly #update: () => void;

  constructor(scope: Scope, playback: Playback, update: () => void) {
    this.#scope = scope;
    this.#playback = playback;
    this.#update = update;
  }

  /** Call on every state change: asks `stats()` once per source on its first ready. */
  observe(state: PlayerState) {
    const { sourceId } = state;
    if (state.phase !== "ready" || state.kind !== "media" || sourceId === null) return;
    if (this.#video?.sourceId === sourceId || this.#asking === sourceId) return;
    this.#asking = sourceId;
    this.#scope.player.stats().then(
      (stats) => this.#answer(sourceId, stats.video !== null),
      () => this.#answer(sourceId, false), // unknown: no keyframe steps rather than a refusal each press
    );
  }

  #answer(sourceId: number, video: boolean) {
    if (this.#asking === sourceId) this.#asking = null;
    if (this.#scope.player.state.sourceId !== sourceId) return;
    this.#video = { sourceId, video };
    this.#update();
  }

  /** Whether the source with this id has video; `null` until `stats()` answered. */
  video(sourceId: number): boolean | null {
    return this.#video?.sourceId === sourceId ? this.#video.video : null;
  }

  state(): State {
    const s = this.#scope.player.state;
    let reason: Reason | null = null;
    if (s.kind !== "media" || s.phase !== "ready" || !s.can.seek || s.position === null) reason = UNSUPPORTED;
    else if (this.#video?.sourceId !== s.sourceId || !this.#video.video) reason = NO_VIDEO;
    return { enabled: reason === null && !this.busy, checked: null, busy: this.busy, reason };
  }

  async run(direction: "backward" | "forward") {
    if (!this.state().enabled) return;
    const { player } = this.#scope;
    const before = player.state;
    this.busy = true;
    try {
      await player.step("keyframe", direction, { ifCurrent: before });
    } catch (error) {
      // Someone else changed the source or the position first: their change stands.
      if ((error as { code?: string })?.code === "stale") return;
      throw sourceFailure(error);
    } finally {
      this.busy = false;
    }
    const after = player.state.position ?? 0;
    const p = this.#playback;
    if (direction === "backward" && after < 0.001) p.notify("Jumped to the start");
    else if (direction === "forward" && after <= (before.position ?? 0)) p.notify("Already at the last keyframe");
    else
      p.notify(direction === "forward" ? "Next keyframe: {} / {}" : "Previous keyframe: {} / {}", p.positionArgs(after));
  }
}
