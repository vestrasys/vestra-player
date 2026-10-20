import type { ChildProcess } from "node:child_process";
import type { Scope } from "../../sdk/plugin/index.ts";
import type { SettingsStore } from "./settings.ts";
import { system } from "./system.ts";

/**
 * B29 播放时禁止屏保: while playing, hold a `caffeinate` power assertion.
 * With video it keeps the display awake (`-d -i`); audio only keeps the system
 * awake (`-i`). `-w <pid>` ends it with this plugin process even if the
 * process is killed. At most one child at a time; it stops on pause, end,
 * close, error, a new source (re-decided), switching off, and deactivation.
 */
export class SleepGuard {
  #child: ChildProcess | null = null;
  /** The source the current child (or the decision in progress) is for. */
  #for: number | null = null;
  /** Bumped by every stop, so a decision still waiting for stats is dropped. */
  #generation = 0;
  readonly #scope: Scope;
  readonly #settings: SettingsStore;
  readonly #osd: (text: string) => void;

  constructor(scope: Scope, settings: SettingsStore, osd: (text: string) => void) {
    this.#scope = scope;
    this.#settings = settings;
    this.#osd = osd;
    scope.onDispose(() => this.stop());
  }

  get running() {
    return this.#child !== null;
  }

  #wanted() {
    const { phase, paused } = this.#scope.player.state;
    return this.#settings.value.preventSleep && phase === "ready" && !paused;
  }

  /** Follow the player: call after every state change and setting change. */
  sync() {
    if (!this.#wanted()) return this.stop();
    const { sourceId } = this.#scope.player.state;
    // Already held (or being decided, or failed) for this stretch of playback.
    if (this.#for === sourceId) return;
    this.stop();
    this.#for = sourceId;
    void this.#start(++this.#generation);
  }

  async #start(generation: number) {
    const { player } = this.#scope;
    let video = player.state.kind === "stream";
    if (!video) {
      try {
        video = (await player.stats()).video !== null;
      } catch {
        video = true; // unknown: keep the display awake rather than let it sleep mid-film
      }
    }
    if (generation !== this.#generation || !this.#wanted()) return;
    const args = [...(video ? ["-d"] : []), "-i", "-w", String(process.pid)];
    let child: ChildProcess;
    try {
      child = system.caffeinate(args);
    } catch (error) {
      this.#failed(error);
      return;
    }
    this.#child = child;
    child.once("error", (error) => {
      if (this.#child === child) {
        this.#child = null;
        this.#failed(error);
      }
    });
    // Ended from outside: not restarted until playback starts again.
    child.once("exit", () => {
      if (this.#child === child) this.#child = null;
    });
  }

  /** Shown once; the setting stays on and the next start of playback tries again. */
  #failed(error: unknown) {
    this.#osd(`无法阻止屏保：${error instanceof Error ? error.message : String(error)}`);
  }

  stop() {
    this.#generation++;
    this.#for = null;
    const child = this.#child;
    this.#child = null;
    if (child && child.exitCode === null && child.signalCode === null) child.kill();
  }
}
