import type { Conditional, Dialogs, Player, Scrub, TemplateArgs } from "@vestra-player/sdk";
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
  selecting = false;
  /** Jump to Start is replaying an ended source. */
  restartBusy = false;
  rateBusy = false;
  /** The bar's mute (a core action) is in flight: the volume slider waits. */
  muteBusy = false;
  dragging = false;
  /** Position shown while seeking, and the newest requested target. */
  draft: number | null = null;
  seekTarget: number | null = null;
  /** After a cancelled or invalidated drag, pointer moves are ignored until the next press. */
  blockedGesture = false;
  volumeDraft: number | null = null;
  #volumeRequest = 0;
  #scrub: Scrub | undefined;
  readonly #player: Player;
  readonly #dialogs: Dialogs;
  readonly #update: () => void;
  readonly #osd: Osd;
  readonly #raw: (text: string) => void;

  /** `raw` shows a message that is not this plugin's text (the player's error messages). */
  constructor(player: Player, dialogs: Dialogs, update: () => void, osd: Osd, raw: (text: string) => void) {
    this.#player = player;
    this.#dialogs = dialogs;
    this.#update = update;
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
    this.#update();
  }
  /** `["00:37", "01:40"]`: the position and the duration, for seek feedback ("… {} / {}"). */
  positionArgs(seconds: number): [string, string] {
    return [time(seconds), time(this.state.duration)];
  }

  clearSeek() {
    this.#scrub?.end();
    this.#scrub = undefined;
    this.draft = null;
    this.seekTarget = null;
  }
  /** Pointer pressed on the progress bar: a fresh gesture from the current state. */
  startDrag() {
    this.blockedGesture = false;
    this.clearSeek();
    this.#scrub = this.#player.scrub();
  }
  /** Seek within the current gesture (a new one when none is active); returns the clamped target. */
  seek(seconds: number): number | null {
    if (!this.can("seek")) return null;
    const target = Math.max(0, Math.min(this.state.duration ?? Infinity, seconds));
    this.draft = this.seekTarget = target;
    if (!this.#scrub?.active) this.#scrub = this.#player.scrub();
    const scrub = this.#scrub;
    source(scrub.seek(target)).then(
      (applied) => {
        if (!applied || scrub !== this.#scrub) return;
        // While dragging the pointer owns the draft; the release sample clears it.
        if (!this.dragging && this.seekTarget === target) {
          this.seekTarget = this.draft = null;
          this.#update();
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
    if (this.selecting) return;
    this.selecting = true;
    try {
      const path = await this.#dialogs.openFile({
        title: this.state.phase === "empty" ? "Open Media" : "Open Media (replaces the current source)",
      });
      if (path) {
        this.clearSeek();
        await source(this.#player.open(path));
      }
    } finally {
      this.selecting = false;
    }
  }
  /** B09: back to 0:00, keeping pause or play; after the end, replay like Space. */
  async jumpToStart() {
    if (!this.canRestart() || this.restartBusy) return;
    if (this.state.phase === "ended") {
      this.clearSeek();
      this.restartBusy = true;
      try {
        await source(this.#player.restart());
      } finally {
        this.restartBusy = false;
      }
    } else if (this.seek(0) === null) return;
    this.notify("Jumped to the start");
  }
  async setRate(rate: number, options?: Conditional) {
    if (!this.can("rate") || this.rateBusy) return;
    if (!Number.isFinite(rate) || rate < MIN_RATE || rate > MAX_RATE) throw new Failure("rate-range");
    this.clearSeek();
    this.rateBusy = true;
    try {
      await source(this.#player.setRate(rate, options));
      this.notify("Speed: {}x", [rateText(rate)]);
    } finally {
      this.rateBusy = false;
    }
  }
  /** Latest value wins; the target volume is applied before unmuting, never an old loud one. */
  setVolume(value: number, announce = false) {
    if (this.muteBusy || !Number.isFinite(value)) return;
    const volume = (this.volumeDraft = Math.max(0, Math.min(100, Math.round(value))));
    if (announce) this.notify("Volume: {}%", [volume]);
    const request = ++this.#volumeRequest;
    this.#player.setOutput({ volume, muted: false }).then(
      (applied) => {
        if (applied && request === this.#volumeRequest) {
          this.volumeDraft = null;
          this.#update();
        }
      },
      (error) => {
        this.volumeDraft = null;
        this.report(error);
      },
    );
  }
}
