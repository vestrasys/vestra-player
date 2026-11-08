import type { HostAction, Panel, Plugin } from "@vestra-player/sdk";
import type { PanelActions } from "./commands.ts";
import { MAX_RATE, MIN_RATE, type Playback } from "./controller.ts";
import { parseTime, rateText } from "./format.ts";
import type { Keyframes } from "./keyframes.ts";
import { type AbLoop, precise } from "./loop.ts";
import { COUNTS, DEFAULT_STEPS, parseStep, type SeekSteps, type SettingsStore, stepText } from "./settings.ts";
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
  /** B23 A-B Loop: the section after Speed. */
  loop: AbLoop;
};

/** The A-B Loop section's mini timeline: chapter ticks as one Path in a 1000 × 1 view box. */
export function tickPath(starts: readonly number[], duration: number | null): string {
  if (!duration || duration <= 0) return "";
  const columns = new Set<number>();
  for (const start of starts) if (start > 0 && start < duration) columns.add(Math.round((start / duration) * 1000));
  return [...columns].map((x) => `M${x} 0V1`).join("");
}

/** The first values of the panel's own state (`PlaybackPanel.#st`). */
const FIRST = {
  invalidStep: -1,
  jumpInvalid: false,
  rateInvalid: false,
  abInvalid: null as "a" | "b" | null,
  editJump: 0,
  editRate: 0,
  revealSteps: 0,
  done: 0,
  draft: undefined as number | undefined,
};

/**
 * The Playback panel in the adjust column (menu redesign §3.2, step 7): Speed,
 * A-B Loop (mockup 32: moved here from the marks plugin), Jump, Step and Seek steps. It replaces Playback ▸ Speed and Seek, the Jump to
 * Time and custom speed windows and the Seek Steps window. A typed value
 * applies to the current source on Enter; a refused one stays open in its field,
 * marked, with the reason on the video. Nothing here belongs to a source.
 */
export class PlaybackPanel implements PanelActions {
  readonly panel: Panel;
  /**
   * What the panel shows besides the player's state, in `plugin.state` (a write
   * re-runs the panel's binding):
   * - `invalidStep`, `jumpInvalid`, `rateInvalid`: the refused value: the seek step row (-1 none), the time, the speed;
   * - `abInvalid`: a typed A or B point was refused;
   * - `editJump`, `editRate`, `revealSteps`, `done`: counters the panel watches (ui/panel.slint): a bump focuses, opens or closes a field;
   * - `draft`: the speed the ruler asked for and the player has not reached yet (a drag, a tick, a
   *   double click), shown at once; `undefined` shows the player's. While a speed is being
   *   applied the latest ask waits and replaces any older one (latest wins), so a drag never
   *   queues one request per pointer move and never ends on a stale value.
   */
  readonly #st: typeof FIRST;
  #dragging = false;
  #pumping = false;
  /** The text fields that have the keyboard now (normally one; two for a moment while it moves). */
  readonly #focused = new Set<string>();
  readonly #plugin: Plugin;
  readonly #o: PanelOptions;

  constructor(plugin: Plugin, options: PanelOptions) {
    this.#plugin = plugin;
    this.#o = options;
    this.#st = plugin.state(FIRST);
    const { playback: p, keyframes, settings, loop } = options;
    this.panel = plugin.panels.mount("adjust", "ui/panel.slint", { title: "Playback" });
    // The player's own actions (speed, frames, ← →) are in its binding table; this plugin's
    // commands come from `commands.keys`, read in `bind` so the caps follow every change.
    const keys = new Keys(plugin);
    void keys.refresh();
    const own = (id: string) => plugin.commands.keys(id)[0] ?? "";
    this.panel.bind(() => {
      const rate = this.#st.draft ?? p.state.rate;
      const keyframe = keyframes.state();
      const state = p.state;
      const duration = state.duration && state.duration > 0 ? state.duration : null;
      const at = (seconds: number | null | undefined) =>
        duration === null || seconds === null || seconds === undefined ? 0 : Math.min(1, Math.max(0, seconds / duration));
      // The span: the running loop (whoever set it), else this plugin's A and B (grey).
      const running = state.loop ?? null;
      const span = running ?? (loop.a !== null && loop.b !== null ? { a: loop.a, b: loop.b } : null);
      const abBlocked = loop.blocked();
      const abUsable = abBlocked === null;
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
          seta: own("ab.set-a"),
          cleara: own("ab.clear-a"),
          aback: own("ab.a-back"),
          aforward: own("ab.a-forward"),
          setb: own("ab.set-b"),
          clearb: own("ab.clear-b"),
          bback: own("ab.b-back"),
          bforward: own("ab.b-forward"),
          abtoggle: own("ab.toggle"),
          absegment: own("ab.segment"),
        },
        // Not greyed while a speed is being applied: the ruler keeps taking a drag (latest wins).
        canRate: p.can("rate"),
        rateText: rateText(rate),
        ratePosition: positionOf(rate),
        tick: tickOf(rate),
        canJump: p.canSeek(),
        canStart: p.canRestart() && !p.ui.restartBusy,
        canFrame: p.canStep(),
        canKeyframe: keyframe.enabled,
        keyframeReason: keyframe.reason === "No video" ? "video" : keyframe.reason ? "source" : "",
        steps: settings.value.seekSteps.map(stepText),
        keyframesOnly: settings.value.seekKeyframes,
        invalidStep: this.#st.invalidStep,
        jumpInvalid: this.#st.jumpInvalid,
        rateInvalid: this.#st.rateInvalid,
        editJump: this.#st.editJump,
        editRate: this.#st.editRate,
        revealSteps: this.#st.revealSteps,
        // B23 A-B Loop: the points ("" unset), the switch (any running loop), the count, the timeline.
        abUsable,
        abReason: abBlocked === "No media open" ? "media" : abBlocked ? "source" : "",
        abA: loop.a === null ? "" : precise(loop.a),
        abB: loop.b === null ? "" : precise(loop.b),
        abOn: Boolean(running),
        abCanLoop: abUsable && loop.toggleable(),
        abCount: Math.max(0, COUNTS.indexOf(loop.count)),
        // The pass being played of all passes: mpv's count is the jumps back, so count + 1 passes;
        // the last one plays after the player dropped the loop (`finalPass`).
        abStatus: running
          ? running.count === null
            ? "∞"
            : `${Math.min(loop.pass, running.count + 1)} / ${running.count + 1}`
          : loop.finalPass !== null
            ? `${loop.finalPass} / ${loop.finalPass}`
            : loop.a === null && abUsable && state.position !== null
            ? precise(state.position)
            : "",
        abAt: at(state.position),
        abAAt: at(loop.a),
        abBAt: at(loop.b),
        abSpan: span !== null && duration !== null,
        abSpanA: at(span?.a),
        abSpanB: at(span?.b),
        abTicks: tickPath(loop.chapters, duration),
        abInvalid: this.#st.abInvalid ?? "",
        done: this.#st.done,
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
      if (!this.#pumping) this.#st.draft = undefined;
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
    // B23 A-B Loop: "a" or "b" for the point.
    const point = (name: unknown) => (name === "a" || name === "b" ? name : null);
    panel.on("abSet", (name: string) => {
      const which = point(name);
      if (which === "a") loop.setA();
      else if (which === "b") loop.setB();
    });
    panel.on("abClear", (name: string) => {
      const which = point(name);
      if (which === "a") loop.clearA();
      else if (which === "b") loop.clearB();
    });
    panel.on("abNudge", (name: string, direction: number) => {
      const which = point(name);
      if (!which || (direction !== 1 && direction !== -1)) return;
      if (which === "a") loop.nudgeA(direction * 0.1);
      else loop.nudgeB(direction * 0.1);
    });
    panel.on("abType", (name: string, typed: string) => {
      const which = point(name);
      if (!which || loop.blocked()) return;
      // Not a time: refused like one outside the media (the reason on the video).
      const seconds = parseTime(String(typed ?? "")) ?? Number.NaN;
      if (loop.type(which, seconds)) this.#applied();
      else this.#st.abInvalid = which;
    });
    panel.on("abToggle", () => {
      if (loop.toggleable()) loop.toggle();
    });
    panel.on("abCountChosen", (index: number) => {
      const count = COUNTS[index];
      if (count !== undefined && count !== loop.count) loop.setCount(count);
    });
    panel.on("abSegment", () => loop.loopSegment());
    // Each text field reports its own focus: the panel holds the keyboard (keeps the fullscreen
    // column out) while any of them has it, whatever order the next field's focus and the last
    // one's blur arrive in. Leaving a field drops its mark.
    panel.on("focusChanged", (field: string, focused: boolean) => {
      const name = String(field);
      if (focused === true) this.#focused.add(name);
      else {
        this.#focused.delete(name);
        if (name === "jump") this.#st.jumpInvalid = false;
        else if (name === "rate") this.#st.rateInvalid = false;
        else if (name === `step${this.#st.invalidStep}`) this.#st.invalidStep = -1;
        else if (name === this.#st.abInvalid) this.#st.abInvalid = null;
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
    this.#st.editJump++;
  }
  async typeRate() {
    await this.panel.show();
    this.#st.editRate++;
  }
  async revealSteps() {
    await this.panel.show();
    this.#st.revealSteps++;
  }
  /**
   * Another source: a refused time no longer applies to anything. The A and B
   * fields go with their points (unset now), so they report no blur: their focus
   * marks go here.
   */
  sourceChanged() {
    const a = this.#focused.delete("a");
    const b = this.#focused.delete("b");
    if (a || b) this.panel.setInteraction({ focused: this.#focused.size > 0 });
    this.#st.jumpInvalid = false;
    this.#st.abInvalid = null;
  }
  /** Every state change: a source that cannot change speed ends any drag; nothing is pending, nothing to show. */
  observe() {
    if (this.#st.draft === undefined || this.#o.playback.can("rate")) return;
    this.#st.draft = undefined;
    this.#dragging = false;
  }

  /** A typed value was applied: the fields close and the time field empties. */
  #applied() {
    this.#st.invalidStep = -1;
    this.#st.abInvalid = null;
    this.#st.jumpInvalid = this.#st.rateInvalid = false;
    this.#st.done++;
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
      return this.#refused("rate-range", () => (this.#st.rateInvalid = true));
    this.#applied();
    this.#ask(value);
  }

  #applyJump(typed: string) {
    const p = this.#o.playback;
    if (!p.canSeek()) return;
    const seconds = parseTime(typed);
    if (seconds === null || seconds > p.state.duration!)
      return this.#refused("time-range", () => (this.#st.jumpInvalid = true));
    const target = p.seek(seconds);
    if (target !== null) p.notify("Seek: {} / {}", p.positionArgs(target));
    this.#applied();
  }

  async #applyStep(row: number, typed: string) {
    const { settings } = this.#o;
    if (!(Number.isInteger(row) && row >= 0 && row < 4)) return;
    const seconds = parseStep(typed);
    if (seconds === null) return this.#refused("step-range", () => (this.#st.invalidStep = row));
    const steps = [...settings.value.seekSteps] as SeekSteps;
    steps[row] = seconds;
    this.#applied();
    await this.#guard(this.#o.applySteps(steps, settings.value.seekKeyframes));
  }

  /** The ruler asks for a speed: shown at once, applied latest-wins. */
  #ask(rate: number) {
    if (!this.#o.playback.can("rate")) return;
    this.#st.draft = rate;
    void this.#pump();
  }
  async #pump() {
    if (this.#pumping) return;
    this.#pumping = true;
    const p = this.#o.playback;
    try {
      let sent: number | undefined;
      while (this.#st.draft !== undefined && this.#st.draft !== sent) {
        // A speed set another way (a palette command) is still applying: wait for it rather
        // than drop the ask (`setRate` returns without sending while busy).
        while (p.ui.rateBusy && p.can("rate")) await new Promise((resolve) => setTimeout(resolve, 10));
        if (!p.can("rate")) break;
        sent = this.#st.draft;
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
      if (!this.#dragging) this.#st.draft = undefined;
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
  }
}
