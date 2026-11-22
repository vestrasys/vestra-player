import type { Condition, PlayerState, Plugin, Trigger } from "@vestra-player/sdk";
import type { OnTop as Mode, Preferences } from "./settings.ts";

export const playing = (state: PlayerState) => state.phase === "ready" && !state.paused;

/** `storage.session` key: the on-top value this plugin last sent in this player run. */
const APPLIED = "onTop.applied";

/**
 * H05 / IA-WIN-03: the window floats when the mode says so (by default never): always, while
 * playing, or while playing in fullscreen. Mini Size
 * does not float (IA-WIN-02). The player keeps the mode's condition (one trigger) and calls
 * `enter` / `leave` when it flips, also while this plugin is in standby (it is started for it);
 * the on-top setting itself stays with the player through standby. Only changes are sent, in
 * order, so another plugin's last write is not overridden for nothing; what was sent last is kept
 * in session storage, so a plugin started again after standby still knows it. The core drops this
 * plugin's request when it is deactivated.
 */
export class OnTop {
  readonly #plugin: Plugin;
  readonly #preferences: Preferences;
  readonly #trigger: Trigger;
  #applied = false;
  #queue: Promise<void>;

  /** Called in the synchronous part of `activate()`: it declares the trigger. */
  constructor(plugin: Plugin, preferences: Preferences) {
    this.#plugin = plugin;
    this.#preferences = preferences;
    this.#trigger = plugin
      .trigger()
      .enter(() => this.#set(true))
      .leave(() => this.#set(false));
    // Every decision waits for the value this run sent last (the core's default, off, if none).
    this.#queue = plugin.storage.session.get<unknown>(APPLIED).then(
      (value) => {
        if (typeof value === "boolean") this.#applied = value;
      },
      () => {},
    );
    // The same condition again changes nothing (no second `enter`); "never" has none.
    const condition = this.#condition();
    if (condition) this.#trigger.firesOn(condition);
  }

  /** The mode's condition; `null` for "never". "always" is a condition that always holds. */
  #condition(): Condition | null {
    const { when } = this.#plugin;
    switch (this.#preferences.value.onTop) {
      case "always":
        return when.open.or(when.open.not());
      case "playing":
        return when.playing;
      case "fullscreen":
        return when.playing.and(when.fullscreen);
      default:
        return null;
    }
  }

  /**
   * The mode changed: hand the player its condition, and send what the new mode wants now (a
   * condition that holds at once is also entered, which then sends nothing more).
   */
  async changed() {
    const condition = this.#condition();
    if (condition) this.#trigger.firesOn(condition);
    else this.#trigger.off();
    await this.#set(await this.#wanted(this.#preferences.value.onTop));
  }

  /** What `mode` wants now: the state read once, the window as `window.changed` keeps it. */
  async #wanted(mode: Mode) {
    if (mode === "always") return true;
    if (mode === "never") return false;
    const state = await this.#plugin.player.refresh().catch(() => this.#plugin.player.state);
    return playing(state) && (mode === "playing" || this.#plugin.window.state.fullscreen);
  }

  #set(on: boolean) {
    this.#queue = this.#queue
      .then(async () => {
        if (on === this.#applied) return;
        this.#applied = on;
        void this.#plugin.storage.session.set(APPLIED, on).catch(() => {});
        // Headless players refuse it (`unavailable`); there is nothing to float then.
        await this.#plugin.window.setAlwaysOnTop(on);
      })
      .catch(() => {});
    return this.#queue;
  }
}
