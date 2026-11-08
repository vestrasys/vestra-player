import type { HostAction, Panel, Plugin } from "@vestra-player/sdk";
import type { PanelActions } from "./commands.ts";
import { MAX_RATE, MIN_RATE, type Playback } from "./controller.ts";
import { parseTime, rateText } from "./format.ts";
import type { Keyframes } from "./keyframes.ts";
import { DEFAULT_STEPS, parseStep, type SeekSteps, type SettingsStore, stepText } from "./settings.ts";
import { dragRate, positionOf, TICKS, tickOf } from "./speed.ts";
import { Failure } from "./texts.ts";
import { Keys } from "./tips.ts";

export type PanelOptions = {
  playback: Playback;
  keyframes: Keyframes;
  settings: SettingsStore;
  /** Save the seek steps and "Jump to keyframes only" and apply them (the player's step, the commands' titles). */
  applySteps: (steps: SeekSteps, keyframes: boolean) => Promise<void>;
  jumpToStart: () => Promise<void>;
};

/**
 * The Playback panel in the adjust column (menu redesign §3.2, step 7): Speed,
 * Jump, Step and Seek steps. It replaces Playback ▸ Speed and Seek, the Jump to
 * Time and custom speed windows and the Seek Steps window. A typed value
 * applies to the current source on Enter; a refused one stays open in its field,
 * marked, with the reason on the video. Nothing here belongs to a source.
 */
export class PlaybackPanel implements PanelActions {
  readonly panel: Panel;
  /** The refused value: the seek step row (-1 none), the time, the speed. */
  #invalidStep = -1;
  #jumpInvalid = false;
  #rateInvalid = false;
  /** Counters the panel watches (ui/panel.slint): a bump focuses, opens or closes a field. */
  #editJump = 0;
  #editRate = 0;
  #revealSteps = 0;
  #done = 0;
  /**
   * The speed the ruler asked for and the player has not reached yet (a drag, a tick, a
   * double click), shown at once; `undefined` shows the player's. While a speed is being
   * applied the latest ask waits and replaces any older one (latest wins), so a drag never
   * queues one request per pointer move and never ends on a stale value.
   */
  #draft: number | undefined;
  #dragging = false;
  #pumping = false;
  /** The text fields that have the keyboard now (normally one; two for a moment while it moves). */
  readonly #focused = new Set<string>();
  readonly #plugin: Plugin;
  readonly #o: PanelOptions;

  constructor(plugin: Plugin, options: PanelOptions) {
    this.#plugin = plugin;
    this.#o = options;
    const { playback: p, keyframes, settings } = options;
    this.panel = plugin.panels.mount("adjust", "ui/panel.slint", { title: "Playback" });
    // The player's own actions (speed, frames, ← →) are in its binding table; this plugin's
    // commands come from `commands.keys`, read in `bind` so the caps follow every change.
    const keys = new Keys(plugin, () => plugin.update());
    void keys.refresh();
    const own = (id: string) => plugin.commands.keys(id)[0] ?? "";
    this.panel.bind(() => {
      // A source that cannot change speed ends any drag: nothing is pending, nothing to show.
      if (this.#draft !== undefined && !p.can("rate")) {
        this.#draft = undefined;
        this.#dragging = false;
      }
      const rate = this.#draft ?? p.state.rate;
      const keyframe = keyframes.state();
      return {
        keys: {
          slower: keys.key("rate-down"),
          faster: keys.key("rate-up"),
          normal: keys.key("rate-normal"),
          jump: own("jump"),
          start: own("jump-start"),
          frameback: keys.key("frame-back"),
          frameforward: keys.key("frame-forward"),
          keyback: own("keyframe-back"),
          keyforward: own("keyframe-forward"),
          back1: keys.key("seek-backward"),
          forward1: keys.key("seek-forward"),
          back2: own("backward-2"),
          forward2: own("forward-2"),
          back3: own("backward-3"),
          forward3: own("forward-3"),
          back4: own("backward-4"),
          forward4: own("forward-4"),
        },
        // Not greyed while a speed is being applied: the ruler keeps taking a drag (latest wins).
        canRate: p.can("rate"),
        rateText: rateText(rate),
        ratePosition: positionOf(rate),
        tick: tickOf(rate),
        canJump: p.canSeek(),
        canStart: p.canRestart() && !p.restartBusy,
        canFrame: p.canStep(),
        canKeyframe: keyframe.enabled,
        keyframeReason: keyframe.reason === "No video" ? "video" : keyframe.reason ? "source" : "",
        steps: settings.value.seekSteps.map(stepText),
        keyframesOnly: settings.value.seekKeyframes,
        invalidStep: this.#invalidStep,
        jumpInvalid: this.#jumpInvalid,
        rateInvalid: this.#rateInvalid,
        editJump: this.#editJump,
        editRate: this.#editRate,
        revealSteps: this.#revealSteps,
        done: this.#done,
      };
    });
    const panel = this.panel;
    panel.on("rateStep", (direction: number) => this.#host(direction < 0 ? "rate-down" : "rate-up"));
    panel.on("setTick", (index: number) => {
      const value = TICKS[index];
      if (value !== undefined) this.#ask(value);
    });
    panel.on("speedDrag", (position: number, snap: number) => {
      if (typeof position !== "number" || !p.can("rate")) return;
      this.#dragging = true;
      this.#ask(dragRate(position, typeof snap === "number" ? snap : 0));
    });
    panel.on("speedRelease", () => {
      this.#dragging = false;
      if (!this.#pumping && this.#draft !== undefined) {
        this.#draft = undefined;
        plugin.update();
      }
    });
    panel.on("speedNormal", () => this.#ask(1));
    panel.on("applyRate", (typed: string) => this.#applyRate(String(typed ?? "")));
    panel.on("applyJump", (typed: string) => this.#applyJump(String(typed ?? "")));
    panel.on("jumpStart", () => this.#guard(options.jumpToStart()));
    panel.on("frameStep", (direction: number) => this.#host(direction < 0 ? "frame-back" : "frame-forward"));
    panel.on("keyframeStep", (direction: number) =>
      this.#guard(keyframes.run(direction < 0 ? "backward" : "forward")),
    );
    panel.on("applyStep", (row: number, typed: string) => this.#applyStep(row, String(typed ?? "")));
    panel.on("resetStep", (row: number) => {
      const value = DEFAULT_STEPS[row];
      if (value !== undefined) return this.#applyStep(row, stepText(value));
    });
    panel.on("setKeyframesOnly", (on: boolean) => {
      if (typeof on !== "boolean" || on === settings.value.seekKeyframes) return;
      return this.#guard(options.applySteps(settings.value.seekSteps, on));
    });
    // Each text field reports its own focus: the panel holds the keyboard (keeps the fullscreen
    // column out) while any of them has it, whatever order the next field's focus and the last
    // one's blur arrive in. Leaving a field drops its mark.
    panel.on("focusChanged", (field: string, focused: boolean) => {
      const name = String(field);
      if (focused === true) this.#focused.add(name);
      else {
        this.#focused.delete(name);
        if (name === "jump") this.#jumpInvalid = false;
        else if (name === "rate") this.#rateInvalid = false;
        else if (name === `step${this.#invalidStep}`) this.#invalidStep = -1;
        plugin.update();
      }
      panel.setInteraction({ focused: this.#focused.size > 0 });
    });
  }

  /** The column is shown with this panel's tab in front. */
  shown() {
    return (this.panel.state?.preferred ?? false) && (this.panel.state?.active ?? false);
  }
  toggle() {
    return this.panel.toggle();
  }
  async focusJump() {
    await this.panel.show();
    this.#editJump++;
    this.#plugin.update();
  }
  async typeRate() {
    await this.panel.show();
    this.#editRate++;
    this.#plugin.update();
  }
  async revealSteps() {
    await this.panel.show();
    this.#revealSteps++;
    this.#plugin.update();
  }
  /** Another source: a refused time no longer applies to anything. */
  sourceChanged() {
    if (!this.#jumpInvalid) return;
    this.#jumpInvalid = false;
    this.#plugin.update();
  }

  /** A typed value was applied: the fields close and the time field empties. */
  #applied() {
    this.#invalidStep = -1;
    this.#jumpInvalid = this.#rateInvalid = false;
    this.#done++;
    this.#plugin.update();
  }
  /** A typed value was refused: its field stays open and marked; the reason shows on the video. */
  #refused(kind: "time-range" | "rate-range" | "step-range", mark: () => void) {
    mark();
    this.#o.playback.report(new Failure(kind));
  }

  /** A typed speed: checked against the player's limits here, then applied like the ruler's (latest wins). */
  #applyRate(typed: string) {
    const p = this.#o.playback;
    if (!p.can("rate")) return;
    const text = typed.trim();
    const value = Number(text.replace(/[x×]$/i, "").trim());
    if (!text || !Number.isFinite(value) || value < MIN_RATE || value > MAX_RATE)
      return this.#refused("rate-range", () => (this.#rateInvalid = true));
    this.#applied();
    this.#ask(value);
  }

  #applyJump(typed: string) {
    const p = this.#o.playback;
    if (!p.canSeek()) return;
    const seconds = parseTime(typed);
    if (seconds === null || seconds > p.state.duration!)
      return this.#refused("time-range", () => (this.#jumpInvalid = true));
    const target = p.seek(seconds);
    if (target !== null) p.notify("Seek: {} / {}", p.positionArgs(target));
    this.#applied();
  }

  async #applyStep(row: number, typed: string) {
    const { settings } = this.#o;
    if (!(Number.isInteger(row) && row >= 0 && row < 4)) return;
    const seconds = parseStep(typed);
    if (seconds === null) return this.#refused("step-range", () => (this.#invalidStep = row));
    const steps = [...settings.value.seekSteps] as SeekSteps;
    steps[row] = seconds;
    this.#applied();
    await this.#guard(this.#o.applySteps(steps, settings.value.seekKeyframes));
  }

  /** The ruler asks for a speed: shown at once, applied latest-wins. */
  #ask(rate: number) {
    if (!this.#o.playback.can("rate")) return;
    this.#draft = rate;
    this.#plugin.update();
    void this.#pump();
  }
  async #pump() {
    if (this.#pumping) return;
    this.#pumping = true;
    const p = this.#o.playback;
    try {
      let sent: number | undefined;
      while (this.#draft !== undefined && this.#draft !== sent) {
        // A speed set another way (a palette command) is still applying: wait for it rather
        // than drop the ask (`setRate` returns without sending while busy).
        while (p.rateBusy && p.can("rate")) await new Promise((resolve) => setTimeout(resolve, 10));
        if (!p.can("rate")) break;
        sent = this.#draft;
        if (Math.abs(sent - p.state.rate) < 1e-6) continue;
        try {
          await p.setRate(sent);
        } catch (error) {
          p.report(error);
          break;
        }
      }
    } finally {
      this.#pumping = false;
      if (!this.#dragging) this.#draft = undefined;
      this.#plugin.update();
    }
  }

  /** The player's own action (speed, frames), exactly as its key; one that is off right now does nothing. */
  async #host(action: HostAction) {
    try {
      await this.#plugin.host.run(action);
    } catch (error) {
      if ((error as { code?: string })?.code !== "unavailable") this.#o.playback.report(error);
    }
  }
  async #guard(work: Promise<unknown>) {
    try {
      await work;
    } catch (error) {
      this.#o.playback.report(error);
    }
    this.#plugin.update();
  }
}
