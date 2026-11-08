import type { HostAction, Panel, Plugin } from "@vestra-player/sdk";
import type { PanelActions } from "./commands.ts";
import type { Playback } from "./controller.ts";
import { parseTime, rateText } from "./format.ts";
import type { Keyframes } from "./keyframes.ts";
import { DEFAULT_STEPS, parseStep, type SeekSteps, type SettingsStore, stepText } from "./settings.ts";
import { Failure } from "./texts.ts";
import { Keys } from "./tips.ts";

/** The speed buttons under the speed (ui/panel.slint, in this order). */
export const PRESETS = [0.5, 1, 1.25, 1.5, 2] as const;

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
      const { rate } = p.state;
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
        canRate: p.can("rate") && !p.rateBusy,
        rateText: rateText(rate),
        preset: PRESETS.findIndex((value) => Math.abs(value - rate) < 1e-6),
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
    panel.on("setPreset", (index: number) => {
      const value = PRESETS[index];
      if (value !== undefined) return this.#guard(p.setRate(value));
    });
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

  async #applyRate(typed: string) {
    const p = this.#o.playback;
    if (!p.can("rate") || p.rateBusy) return;
    const value = Number(typed.trim().replace(/[x×]$/i, "").trim());
    try {
      await p.setRate(typed.trim() ? value : Number.NaN);
    } catch (error) {
      if (error instanceof Failure && error.kind === "rate-range")
        return this.#refused("rate-range", () => (this.#rateInvalid = true));
      return p.report(error);
    }
    this.#applied();
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
