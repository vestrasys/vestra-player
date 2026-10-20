import type { Conditional, Dialogs, Player, Scrub } from "../../sdk/plugin/index.ts";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Source control failures keep the "来源操作失败：" prefix the bar has always shown. */
async function source<T>(work: Promise<T>): Promise<T> {
  try {
    return await work;
  } catch (error) {
    throw new Error(`来源操作失败：${message(error)}`);
  }
}

/**
 * The bar's local state and every playback action. Facts come from
 * `player.state`; this object only keeps busy flags, drafts and the drag gesture.
 */
export class Playback {
  selecting = false;
  pauseBusy = false;
  rateBusy = false;
  /** A mute toggle is in flight. */
  outputBusy = false;
  /** Shown instead of the playback status until the next action clears it. */
  error = "";
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

  constructor(player: Player, dialogs: Dialogs, update: () => void) {
    this.#player = player;
    this.#dialogs = dialogs;
    this.#update = update;
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
  /** Pause/resume when ready, or replay after the end. */
  canToggle() {
    return this.can("pause") || (this.state.phase === "ended" && this.state.can.restart);
  }
  /** Someone else changed the source during the drag, so the gesture stopped. */
  get gestureLost() {
    return !!this.#scrub && !this.#scrub.active;
  }
  status() {
    const { phase, paused, error } = this.state;
    if (this.error) return this.error;
    if (phase === "loading") return "正在打开";
    if (phase === "error") return `打开失败：${error}`;
    if (phase === "ended") return "已结束";
    if (this.seekTarget !== null) return "跳转中";
    if (phase === "empty") return "未打开媒体";
    return paused ? "已暂停" : "播放中";
  }
  /** For failures outside a handler the SDK invoked (detached seeks and volume). */
  report(error: unknown) {
    this.error = message(error);
    this.#update();
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
  /** Seek within the current gesture (a new one when none is active). */
  seek(seconds: number) {
    if (!this.can("seek")) return;
    const target = Math.max(0, Math.min(this.state.duration ?? Infinity, seconds));
    this.draft = this.seekTarget = target;
    this.error = "";
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
  }
  /** Relative seeks accumulate from the newest pending target. */
  seekBy(delta: number) {
    this.seek((this.seekTarget ?? this.state.position ?? 0) + delta);
  }

  async open() {
    if (this.selecting) return;
    this.selecting = true;
    this.error = "";
    try {
      const path = await this.#dialogs.openFile({
        title: this.state.phase === "empty" ? "打开媒体" : "打开媒体（将替换当前来源）",
      });
      if (path) {
        this.clearSeek();
        await source(this.#player.open(path));
      }
    } finally {
      this.selecting = false;
    }
  }
  /** Reopen the failed source (the bar's "重试"). */
  async retry() {
    const { phase, source: path } = this.state;
    if (phase === "error" && path) await source(this.#player.open(path));
  }
  async closeSource() {
    this.clearSeek();
    await source(this.#player.close());
  }
  async togglePause() {
    if (this.pauseBusy || !this.canToggle()) return;
    this.clearSeek();
    this.pauseBusy = true;
    this.error = "";
    try {
      const { phase, paused } = this.state;
      await source(phase === "ended" ? this.#player.restart() : this.#player.setPaused(!paused));
    } finally {
      this.pauseBusy = false;
    }
  }
  async setRate(rate: number, options?: Conditional) {
    if (!this.can("rate") || this.rateBusy) return;
    if (!Number.isFinite(rate) || rate < 0.25 || rate > 4) throw new Error("速度范围为0.25–4倍");
    this.clearSeek();
    this.rateBusy = true;
    try {
      await source(this.#player.setRate(rate, options));
    } finally {
      this.rateBusy = false;
    }
  }
  async toggleMute() {
    if (this.outputBusy || this.volumeDraft !== null) return;
    this.outputBusy = true;
    try {
      await this.#player.setOutput({ muted: !this.state.muted });
    } finally {
      this.outputBusy = false;
    }
  }
  /** Latest value wins; the target volume is applied before unmuting, never an old loud one. */
  setVolume(value: number) {
    if (this.outputBusy || !Number.isFinite(value)) return;
    const volume = (this.volumeDraft = Math.max(0, Math.min(200, value)));
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
