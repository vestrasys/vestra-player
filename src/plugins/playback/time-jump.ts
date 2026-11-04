import type { Scope, Window } from "@vestra-player/sdk";
import { DEFAULT_STEPS, type SeekSteps, type SettingsStore, validStep } from "./settings.ts";

/** `2.5`, `10`: no trailing zeros. */
export const stepText = (seconds: number) => String(Math.round(seconds * 10) / 10);

/** A row's text as seconds, or `null` when empty, not a number, out of range or finer than 0.1. */
export function parseStep(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+(\.\d)?$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return validStep(value) ? value : null;
}

/**
 * B06 Seek Steps…: four rows of seconds and "Jump to keyframes only". OK saves
 * and applies them at once (`apply`); Cancel saves nothing, and the next opening
 * shows the saved values. An invalid row is marked and keeps the window open, with the
 * allowed range below (`out-of-range`). The window does not belong to a
 * source: it works with nothing open.
 */
export class TimeJump {
  readonly timeJumpWindow: Window;
  invalid = [false, false, false, false];
  outOfRange = false;
  readonly #settings: SettingsStore;

  constructor(scope: Scope, settings: SettingsStore, apply: (steps: SeekSteps) => Promise<void>) {
    this.#settings = settings;
    this.timeJumpWindow = scope.windows.create("ui/time-jump.slint", { id: "playback-time-jump" });
    this.timeJumpWindow.bind(() => ({
      outOfRange: this.outOfRange,
      invalid1: this.invalid[0],
      invalid2: this.invalid[1],
      invalid3: this.invalid[2],
      invalid4: this.invalid[3],
    }));
    this.timeJumpWindow.on("cancel", () => this.#hide());
    // Default restores only its own row.
    this.timeJumpWindow.on("reset", (row: number) => {
      if (!(row >= 0 && row < 4)) return;
      this.timeJumpWindow.set({ [`value${row + 1}`]: stepText(DEFAULT_STEPS[row]!) });
      this.invalid[row] = false;
      if (!this.invalid.includes(true)) this.outOfRange = false;
    });
    this.timeJumpWindow.on("confirm", async (...args: unknown[]) => {
      const steps = args.slice(0, 4).map((row) => parseStep(String(row ?? "")));
      this.invalid = steps.map((step) => step === null);
      if (steps.length !== 4 || this.invalid.includes(true)) {
        this.outOfRange = true;
        return;
      }
      this.outOfRange = false;
      const next = steps as SeekSteps;
      // "Jump to keyframes only": anything but a boolean keeps the saved choice.
      const keyframes = typeof args[4] === "boolean" ? args[4] : this.#settings.value.seekKeyframes;
      this.#settings.update((s) => {
        s.seekSteps = next;
        s.seekKeyframes = keyframes;
      });
      await this.#hide();
      await apply(next);
    });
  }

  /** On screen: a hidden window stays opened, so `Window.opened` alone does not tell. */
  #visible = false;
  #hide() {
    this.#visible = false;
    return this.timeJumpWindow.hide();
  }

  /** Brings an open window to the front as it is; otherwise shows the saved values (a cancelled edit is gone). */
  async show() {
    if (this.#visible && this.timeJumpWindow.opened) return this.timeJumpWindow.show();
    this.#visible = true;
    const steps = this.#settings.value.seekSteps;
    this.invalid = [false, false, false, false];
    this.outOfRange = false;
    this.timeJumpWindow.set({
      value1: stepText(steps[0]),
      value2: stepText(steps[1]),
      value3: stepText(steps[2]),
      value4: stepText(steps[3]),
      keyframes: this.#settings.value.seekKeyframes,
    });
    await this.timeJumpWindow.show();
  }
}
