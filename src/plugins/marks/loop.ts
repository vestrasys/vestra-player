import type { Plugin, PlayerState } from "@vestra-player/sdk";
import { precise, round1 } from "./format.ts";
import { segmentAt } from "./chapters.ts";
import type { Osd } from "./osd.ts";
import { type Count, type Settings, timing } from "./store.ts";

/** B23: the shortest span between A and B (seconds). */
export const MIN_SPAN = 0.2;
const EPS = 1e-6;
type Loop = { a: number; b: number; count: number | null };
const same = (x: Loop | null | undefined, y: Loop | null | undefined) =>
  (x ?? null) === (y ?? null) || (!!x && !!y && x.a === y.a && x.b === y.b && x.count === y.count);
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** Why the A-B commands are unavailable (shown as the commands' reason). */
export type LoopBlocked = "No media open" | "This source doesn't support A-B loop";

/**
 * B23 A-B loop. The plugin keeps A, B and the on/off switch; the player
 * runs the loop (`player.setLoop`) whenever both points are set and the
 * switch is on, and `setLoop(null)` otherwise. `state.loop` is the truth for
 * the switch: a loop set by another plugin shows as on, and the switch turns
 * it off. A new source clears A and B (the player clears the loop).
 */
export class AbLoop {
  #a: number | null = null;
  #b: number | null = null;
  readonly #pointListeners: (() => void)[] = [];
  /** The switch as this plugin wants it (the player's `state.loop` may differ). */
  on = false;
  readonly #plugin: Plugin;
  readonly #settings: Settings;
  readonly #osd: Osd;
  /** What this plugin last got the player to run. */
  #committed: Loop | null = null;
  #pending = 0;
  #chain: Promise<void> = Promise.resolve();
  #timer: ReturnType<typeof setTimeout> | null = null;
  #seekOnStart = false;
  #sourceId: number | null = null;
  #lastLoop: Loop | null = null;

  constructor(plugin: Plugin, settings: Settings, osd: Osd) {
    this.#plugin = plugin;
    this.#settings = settings;
    this.#osd = osd;
    plugin.onDispose(() => {
      if (this.#timer) clearTimeout(this.#timer);
    });
  }

  /** The start point A (seconds), `null` when unset. */
  get a(): number | null {
    return this.#a;
  }
  set a(value: number | null) {
    if (value === this.#a) return;
    this.#a = value;
    for (const listener of this.#pointListeners) listener();
  }
  /** The end point B (seconds), `null` when unset. */
  get b(): number | null {
    return this.#b;
  }
  set b(value: number | null) {
    if (value === this.#b) return;
    this.#b = value;
    for (const listener of this.#pointListeners) listener();
  }
  /** A or B moved (B22 publishes a lone A as a seek-bar marker). */
  onPoints(listener: () => void) {
    this.#pointListeners.push(listener);
  }

  get count(): Count {
    return this.#settings.abCount;
  }
  get #state() {
    return this.#plugin.player.state;
  }

  /** Why the A-B commands are unavailable now; `null` when they can run. */
  blocked(state: PlayerState = this.#state): LoopBlocked | null {
    if (state.kind !== "media" || state.source === null || state.phase === "empty") return "No media open";
    if (state.phase !== "ready" || !state.can.seek || state.duration === null)
      return "This source doesn't support A-B loop";
    return null;
  }

  /** Follow the player: forget A and B on a new source; notice a finished or foreign loop. */
  observe(state: PlayerState) {
    const loop = state.loop ?? null;
    if (state.sourceId !== this.#sourceId) {
      this.#sourceId = state.sourceId;
      this.a = this.b = null;
      this.on = false;
      this.#committed = null;
      this.#seekOnStart = false;
      if (this.#timer) clearTimeout(this.#timer);
      this.#timer = null;
      this.#lastLoop = loop;
      return;
    }
    const before = this.#lastLoop;
    this.#lastLoop = loop;
    if (this.#pending || same(before, loop)) return;
    if (loop === null && this.#committed !== null) {
      // The count ran out (or someone else stopped it): keep A and B.
      this.#committed = null;
      this.on = false;
      this.#osd("A-B loop ended");
    } else if (loop !== null && !same(loop, this.#committed)) {
      // Another plugin's loop replaced ours.
      this.#committed = null;
      this.on = false;
    }
  }

  // --------------------------------------------------------------- commands

  setA() {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    const at = clamp(round1(state.position), 0, state.duration!);
    if (this.b !== null && this.b - at < MIN_SPAN - EPS) {
      this.b = null;
      this.#osd("Start point is after the end point; end point cleared");
    } else if (this.b === null) this.#osd("A-B start: {}", [precise(at)]);
    this.a = at;
    if (this.b !== null) this.#start();
    else this.on = false;
    this.#commitNow();
  }

  setB() {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    const at = Math.min(round1(state.position), state.duration!);
    const a = this.a ?? 0;
    if (at - a < MIN_SPAN - EPS) {
      this.#osd("End point must be after the start point");
      return;
    }
    this.a = a;
    this.b = at;
    this.#start();
    this.#commitNow();
  }

  clearA() {
    if (this.a === null) return;
    this.a = null;
    this.#stop(this.on ? "A-B loop off" : "Start point cleared");
  }
  clearB() {
    if (this.b === null) return;
    this.b = null;
    this.#stop(this.on ? "A-B loop off" : "End point cleared");
  }

  nudgeA(delta: number) {
    const duration = this.#state.duration;
    if (this.a === null || duration === null) return;
    const high = this.b !== null ? this.b - MIN_SPAN : duration;
    this.a = clamp(round1(this.a + delta), 0, round1(high));
    this.#showPoints();
    this.#commitLater();
  }
  nudgeB(delta: number) {
    const duration = this.#state.duration;
    if (this.b === null || duration === null) return;
    this.b = clamp(round1(this.b + delta), round1((this.a ?? 0) + MIN_SPAN), duration);
    this.#showPoints();
    this.#commitLater();
  }
  /** Move the whole span, its length unchanged; stops at either end. */
  shift(delta: number) {
    const duration = this.#state.duration;
    if (this.a === null || this.b === null || duration === null) return;
    if (this.a + delta < -EPS) return this.#osd("Reached the beginning");
    if (this.b + delta > duration + EPS) return this.#osd("Reached the end");
    this.a = Math.max(0, round1(this.a + delta));
    this.b = Math.min(duration, round1(this.b + delta));
    this.#showPoints();
    this.#commitLater();
  }

  /** `\`: stop the running loop (whoever set it), or start ours when A and B are set. */
  toggle() {
    if (this.#state.loop) return this.#stop("A-B loop off");
    if (this.a === null || this.b === null) return this.#osd("Set the start and end points first");
    this.#start();
    this.#showLoop(this.a, this.b);
    this.#commitNow();
  }

  /** Alt+\: loop the span between the marks around the position, and start now. */
  loopSegment(times: readonly number[]) {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    const span = segmentAt(times, state.position, state.duration!);
    if (!span) return this.#osd("No chapters or bookmarks to define a range");
    const a = round1(span.a);
    const b = Math.min(round1(span.b), state.duration!);
    if (b - a < MIN_SPAN - EPS) return this.#osd("Range too short to loop");
    this.a = a;
    this.b = b;
    this.#start();
    this.#commitNow();
  }

  setCount(count: Count) {
    this.#settings.setAbCount(count);
    if (count === null) this.#osd("Loop count: endless");
    else this.#osd("Loop count: {}", [count]);
    if (this.on) this.#commitNow();
  }

  // ------------------------------------------------------------- internals

  #start() {
    this.on = true;
    this.#seekOnStart = true;
    this.#showLoop(this.a!, this.b!);
  }
  /** `A-B loop: 0:10.0 ~ 0:20.0`. */
  #showLoop(a: number, b: number) {
    this.#osd("A-B loop: {} ~ {}", [precise(a), precise(b)]);
  }
  #stop(text: "A-B loop off" | "Start point cleared" | "End point cleared") {
    this.on = false;
    this.#osd(text);
    this.#commitNow();
  }
  #showPoints() {
    if (this.a !== null && this.b !== null) this.#showLoop(this.a, this.b);
    else if (this.a !== null) this.#osd("A-B start: {}", [precise(this.a)]);
    else if (this.b !== null) this.#osd("A-B end: {}", [precise(this.b)]);
  }

  #wanted(): Loop | null {
    if (!this.on || this.a === null || this.b === null) return null;
    return { a: this.a, b: this.b, count: this.count };
  }
  #commitNow() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = null;
    this.#chain = this.#chain.then(() => this.#flush());
  }
  /** Held nudges: commit once they pause for `timing.loopCommit`. */
  #commitLater() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#chain = this.#chain.then(() => this.#flush());
    }, timing.loopCommit);
  }

  async #flush() {
    const want = this.#wanted();
    const { player } = this.#plugin;
    const current = player.state.loop ?? null;
    const seek = this.#seekOnStart;
    this.#seekOnStart = false;
    if (same(want, current) && same(want, this.#committed)) return;
    if (want === null && current === null) {
      this.#committed = null;
      return;
    }
    const sourceId = player.state.sourceId;
    this.#pending++;
    try {
      const request = want && { a: want.a, b: want.b, ...(want.count !== null ? { count: want.count } : {}) };
      try {
        await player.setLoop(request);
      } catch (error) {
        // A user seek between reading the state and the request: once more on the new state.
        if ((error as { code?: string }).code !== "stale") throw error;
        await player.setLoop(request);
      }
      if (player.state.sourceId !== sourceId) return;
      this.#committed = want;
      this.#lastLoop = player.state.loop ?? null;
      const position = player.state.position;
      if (want && seek && position !== null && (position < want.a || position >= want.b))
        await player.seek(want.a).catch(() => {});
    } catch (error) {
      if (player.state.sourceId !== sourceId) return;
      if (want) this.on = false;
      const code = (error as { code?: string }).code;
      if (code === "unsupported") this.#osd("This source doesn't support A-B loop");
      else this.#osd("A-B loop failed: {}", [(error as Error).message]);
    } finally {
      this.#pending--;
      this.#plugin.update();
    }
  }
}
