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
 * B06 Seek Steps…: four rows of seconds. OK saves and applies them at once
 * (`apply`); an invalid row is marked and keeps the window open, with the
 * allowed range below (`out-of-range`). The window does not belong to a
 * source: it works with nothing open.
 */
export class TimeJump {
  readonly window: Window;
  invalid = [false, false, false, false];
  outOfRange = false;
  readonly #settings: SettingsStore;

  constructor(scope: Scope, settings: SettingsStore, apply: (steps: SeekSteps) => Promise<void>) {
    this.#settings = settings;
    this.window = scope.windows.create("ui/time-jump.slint", { id: "playback-time-jump" });
    this.window.bind(() => ({
      outOfRange: this.outOfRange,
      invalid1: this.invalid[0],
      invalid2: this.invalid[1],
      invalid3: this.invalid[2],
      invalid4: this.invalid[3],
    }));
    this.window.on("cancel", () => this.window.hide());
    // Default restores only its own row.
    this.window.on("reset", (row: number) => {
      if (!(row >= 0 && row < 4)) return;
      this.window.set({ [`value${row + 1}`]: stepText(DEFAULT_STEPS[row]!) });
      this.invalid[row] = false;
      if (!this.invalid.includes(true)) this.outOfRange = false;
    });
    this.window.on("confirm", async (...rows: string[]) => {
      const steps = rows.slice(0, 4).map((row) => parseStep(String(row ?? "")));
      this.invalid = steps.map((step) => step === null);
      if (steps.length !== 4 || this.invalid.includes(true)) {
        this.outOfRange = true;
        return;
      }
      this.outOfRange = false;
      const next = steps as SeekSteps;
      this.#settings.update((s) => {
        s.seekSteps = next;
      });
      await this.window.hide();
      await apply(next);
    });
  }

  async show() {
    if (this.window.opened) return this.window.show();
    const steps = this.#settings.value.seekSteps;
    this.invalid = [false, false, false, false];
    this.outOfRange = false;
    this.window.set({
      value1: stepText(steps[0]),
      value2: stepText(steps[1]),
      value3: stepText(steps[2]),
      value4: stepText(steps[3]),
    });
    await this.window.show();
  }
}
