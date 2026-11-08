import type { Conditional, Dialogs, Player, Scope, Scrub, TemplateArgs } from "@vestra-player/sdk";
import { rateText, time } from "./format.ts";
import { errorMessage, Failure, type Osd, type OsdText, sourceFailure } from "./texts.ts";

/** Source control failures keep the "Source operation failed: …" sentence the bar has always shown. */
async function source<T>(work: Promise<T>): Promise<T> {
  try {
    return await work;
  } catch (error) {
    throw sourceFailure(error);
  }
}

export const MIN_RATE = 0.25;
export const MAX_RATE = 4;

/**
 * The bar's local state and the playback actions the plugin still owns (the
 * basic ones are core actions run through `host.run`). Facts come from
 * `player.state`; this object only keeps busy flags, drafts and the drag gesture.
 * Feedback (results and failures) goes to the on-screen display.
 */
export class Playback {
  /**
   * What the bar, the panel and the commands show of this object, in
   * `plugin.state`: a write re-runs exactly the bindings that read the field.
   */
  readonly ui: {
    /** The Open dialog is up. */
    selecting: boolean;
    /** Jump to Start is replaying an ended source. */
    restartBusy: boolean;
    rateBusy: boolean;
    /** The bar's mute (a core action) is in flight: the volume slider waits. */
    muteBusy: boolean;
    /** Position shown while seeking. */
    draft: number | null;
    /** Volume shown while a slider value is being applied. */
    volumeDraft: number | null;
  };
  dragging = false;
  /** The newest requested seek target. */
  seekTarget: number | null = null;
  /** After a cancelled or invalidated drag, pointer moves are ignored until the next press. */
  blockedGesture = false;
  #volumeRequest = 0;
  #scrub: Scrub | undefined;
  /**
   * 0.2.3: the bar gesture seeks by keyframes ("Jump to keyframes only", user
   * ruling 2026-10-27). Decided at the press and kept for the press sample, every
   * move and the release: a gesture never mixes the two modes, and there is no
   * second, exact seek on release.
   */
  #gestureKeyframes = false;
  /** The newest gesture seek has landed (`true` from the host) while the pointer was still down. */
  #gestureLanded = false;
  readonly #player: Player;
  readonly #dialogs: Dialogs;
  readonly #osd: Osd;
  readonly #raw: (text: string) => void;

  /** `raw` shows a message that is not this plugin's text (the player's error messages). */
  constructor(scope: Scope, osd: Osd, raw: (text: string) => void) {
    this.#player = scope.player;
    this.#dialogs = scope.dialogs;
    this.ui = scope.state({ selecting: false, restartBusy: false, rateBusy: false, muteBusy: false, draft: null as number | null, volumeDraft: null as number | null });
    this.#osd = osd;
    this.#raw = raw;
  }

  get state() {
    return this.#player.state;
  }
  can(kind: "pause" | "seek" | "rate" | "close") {
    const { can, phase } = this.state;
    return can[kind] && (kind === "close" ? phase !== "empty" : phase === "ready");
  }
  canSeek() {
    const { duration } = this.state;
    return this.can("seek") && duration !== null && duration > 0;
  }
  /** Relative seeks (tiers 2–4) need a known position. */
  canStep() {
    return this.can("seek") && this.state.position !== null;
  }
  /** Pause/resume when ready, or replay after the end. */
  canToggle() {
    return this.can("pause") || (this.state.phase === "ended" && this.state.can.restart);
  }
  /** Jump to Start: a seek to 0 while ready, a replay after the end. */
  canRestart() {
    const { can, phase } = this.state;
    return phase === "ready" ? can.seek : phase === "ended" && can.restart;
  }
  /** Someone else changed the source during the drag, so the gesture stopped. */
  get gestureLost() {
    return !!this.#scrub && !this.#scrub.active;
  }
  /** One line on the video; headless hosts refuse it, which is not a failure of the action. */
  notify(text: OsdText, args?: TemplateArgs) {
    this.#osd(text, args);
  }
  /** For failures of any action: shown on the video (the player's own messages as they are). */
  report(error: unknown) {
    if (error instanceof Failure) error.show(this.#osd);
    else this.#raw(errorMessage(error));
  }
  /** `["00:37", "01:40"]`: the position and the duration, for seek feedback ("… {} / {}"). */
  positionArgs(seconds: number): [string, string] {
    return [time(seconds), time(this.state.duration)];
  }

  clearSeek() {
    this.#scrub?.end();
    this.#scrub = undefined;
    this.ui.draft = null;
    this.seekTarget = null;
  }
  /**
   * Pointer pressed on the progress bar: a fresh gesture from the current
   * state. `keyframes` is the gesture's seek mode (the setting as it is now).
   */
  startDrag(keyframes = false) {
    this.blockedGesture = false;
    this.clearSeek();
    this.#gestureKeyframes = keyframes;
    this.#gestureLanded = false;
    this.#scrub = this.#player.scrub();
  }
  /**
   * The press, a move or the release of the bar gesture, in the mode decided at
   * the press. The same target again (a click, or a release where the pointer
   * stopped) is not sent twice: the first seek is already running or landed.
   */
  gestureSeek(seconds: number): number | null {
    const target = this.#clamp(seconds);
    if (target !== null && target === this.seekTarget && this.#scrub?.active) return target;
    return this.#seek(target, this.#gestureKeyframes);
  }
  /**
   * Pointer let go: the gesture is over. When its last seek landed while the
   * pointer was still down (a click: press and release on the same spot send
   * one seek), the host's position takes over from the draft now.
   */
  endDrag() {
    if (this.#gestureLanded && this.seekTarget !== null) {
      this.seekTarget = this.ui.draft = null;
      this.#gestureLanded = false;
    }
  }
  /** An exact seek within the current gesture (a new one when none is active); returns the clamped target. */
  seek(seconds: number): number | null {
    return this.#seek(this.#clamp(seconds), false);
  }
  #clamp(seconds: number): number | null {
    if (!this.can("seek")) return null;
    return Math.max(0, Math.min(this.state.duration ?? Infinity, seconds));
  }
  #seek(target: number | null, keyframes: boolean): number | null {
    if (target === null) return null;
    this.ui.draft = this.seekTarget = target;
    this.#gestureLanded = false;
    if (!this.#scrub?.active) this.#scrub = this.#player.scrub();
    const scrub = this.#scrub;
    // Player 0.5.3 / SDK 1.5.22: `keyframes` goes on the wire only when true (`absolute+keyframes`).
    source(scrub.seek(target, { keyframes })).then(
      (applied) => {
        if (!applied || scrub !== this.#scrub || this.seekTarget !== target) return;
        // While dragging the pointer owns the draft: the release (`endDrag`) or its own seek clears it.
        if (this.dragging) this.#gestureLanded = true;
        else {
          this.seekTarget = this.ui.draft = null;
        }
      },
      (error) => {
        if (scrub !== this.#scrub) return; // a cleared gesture never reports into a new one
        this.clearSeek();
        this.report(error);
      },
    );
    return target;
  }
  /** Relative seeks (tiers 2–4) accumulate from the newest pending target. */
  seekBy(delta: number) {
    const target = this.seek((this.seekTarget ?? this.state.position ?? 0) + delta);
    if (target !== null)
      this.notify(delta < 0 ? "Back {} s: {} / {}" : "Forward {} s: {} / {}", [
        String(Math.abs(delta)),
        ...this.positionArgs(target),
      ]);
  }

  async open() {
    if (this.ui.selecting) return;
    this.ui.selecting = true;
    try {
      const path = await this.#dialogs.openFile({
        title: this.state.phase === "empty" ? "Open Media" : "Open Media (replaces the current source)",
      });
      if (path) {
        this.clearSeek();
        await source(this.#player.open(path));
      }
    } finally {
      this.ui.selecting = false;
    }
  }
  /** B09: back to 0:00, keeping pause or play; after the end, replay like Space. */
  async jumpToStart() {
    if (!this.canRestart() || this.ui.restartBusy) return;
    if (this.state.phase === "ended") {
      this.clearSeek();
      this.ui.restartBusy = true;
      try {
        await source(this.#player.restart());
      } finally {
        this.ui.restartBusy = false;
      }
    } else if (this.seek(0) === null) return;
    this.notify("Jumped to the start");
  }
  async setRate(rate: number, options?: Conditional) {
    if (!this.can("rate") || this.ui.rateBusy) return;
    if (!Number.isFinite(rate) || rate < MIN_RATE || rate > MAX_RATE) throw new Failure("rate-range");
    this.clearSeek();
    this.ui.rateBusy = true;
    try {
      await source(this.#player.setRate(rate, options));
      this.notify("Speed: {}x", [rateText(rate)]);
    } finally {
      this.ui.rateBusy = false;
    }
  }
  /** Latest value wins; the target volume is applied before unmuting, never an old loud one. */
  setVolume(value: number, announce = false) {
    if (this.ui.muteBusy || !Number.isFinite(value)) return;
    const volume = (this.ui.volumeDraft = Math.max(0, Math.min(100, Math.round(value))));
    if (announce) this.notify("Volume: {}%", [volume]);
    const request = ++this.#volumeRequest;
    this.#player.setOutput({ volume, muted: false }).then(
      (applied) => {
        if (applied && request === this.#volumeRequest) {
          this.ui.volumeDraft = null;
        }
      },
      (error) => {
        this.ui.volumeDraft = null;
        this.report(error);
      },
    );
  }
}
