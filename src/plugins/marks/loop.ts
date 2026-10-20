import type { Plugin, PlayerState } from "../../sdk/plugin/index.ts";
import { precise, round1 } from "./format.ts";
import { segmentAt } from "./chapters.ts";
import { type Count, type Settings, timing } from "./store.ts";

/** B23: the shortest span between A and B (seconds). */
export const MIN_SPAN = 0.2;
const EPS = 1e-6;
type Loop = { a: number; b: number; count: number | null };
const same = (x: Loop | null | undefined, y: Loop | null | undefined) =>
  (x ?? null) === (y ?? null) || (!!x && !!y && x.a === y.a && x.b === y.b && x.count === y.count);
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** `A-B 循环：0:10.0 ~ 0:20.0` (PotPlayer S:1152/1153). */
export const loopText = (a: number, b: number) => `A-B 循环：${precise(a)} ~ ${precise(b)}`;

/**
 * B23 A-B 区段循环. The plugin keeps A, B and the on/off switch; the player
 * runs the loop (`player.setLoop`) whenever both points are set and the
 * switch is on, and `setLoop(null)` otherwise. `state.loop` is the truth for
 * the switch: a loop set by another plugin shows as on, and the switch turns
 * it off. A new source clears A and B (the player clears the loop).
 */
export class AbLoop {
  a: number | null = null;
  b: number | null = null;
  /** The switch as this plugin wants it (the player's `state.loop` may differ). */
  on = false;
  readonly #plugin: Plugin;
  readonly #settings: Settings;
  readonly #osd: (text: string) => void;
  /** What this plugin last got the player to run. */
  #committed: Loop | null = null;
  #pending = 0;
  #chain: Promise<void> = Promise.resolve();
  #timer: ReturnType<typeof setTimeout> | null = null;
  #seekOnStart = false;
  #sourceId: number | null = null;
  #lastLoop: Loop | null = null;

  constructor(plugin: Plugin, settings: Settings, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#settings = settings;
    this.#osd = osd;
    plugin.onDispose(() => {
      if (this.#timer) clearTimeout(this.#timer);
    });
  }

  get count(): Count {
    return this.#settings.abCount;
  }
  get #state() {
    return this.#plugin.player.state;
  }

  /** Why the A-B commands are unavailable now; `null` when they can run. */
  blocked(state: PlayerState = this.#state): string | null {
    if (state.kind !== "media" || state.source === null || state.phase === "empty") return "未打开媒体";
    if (state.phase !== "ready" || !state.can.seek || state.duration === null) return "当前来源不支持区段循环";
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
      this.#osd("区段循环结束");
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
      this.#osd("起点晚于止点，已清除止点");
    } else if (this.b === null) this.#osd(`A-B 起点：${precise(at)}`);
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
      this.#osd("止点必须晚于起点");
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
    this.#stop(this.on ? "解除区段循环" : "已解除起点");
  }
  clearB() {
    if (this.b === null) return;
    this.b = null;
    this.#stop(this.on ? "解除区段循环" : "已解除止点");
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
    if (this.a + delta < -EPS) return this.#osd("已到开头");
    if (this.b + delta > duration + EPS) return this.#osd("已到结尾");
    this.a = Math.max(0, round1(this.a + delta));
    this.b = Math.min(duration, round1(this.b + delta));
    this.#showPoints();
    this.#commitLater();
  }

  /** `\`: stop the running loop (whoever set it), or start ours when A and B are set. */
  toggle() {
    if (this.#state.loop) return this.#stop("解除区段循环");
    if (this.a === null || this.b === null) return this.#osd("请先设定起点和止点");
    this.#start();
    this.#osd(loopText(this.a, this.b));
    this.#commitNow();
  }

  /** Alt+\: loop the span between the marks around the position, and start now. */
  loopSegment(times: readonly number[]) {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    const span = segmentAt(times, state.position, state.duration!);
    if (!span) return this.#osd("没有章节或书签，无法确定区段");
    const a = round1(span.a);
    const b = Math.min(round1(span.b), state.duration!);
    if (b - a < MIN_SPAN - EPS) return this.#osd("区段太短，无法循环");
    this.a = a;
    this.b = b;
    this.#start();
    this.#commitNow();
  }

  setCount(count: Count) {
    this.#settings.setAbCount(count);
    this.#osd(`循环次数：${count === null ? "无限" : `${count} 次`}`);
    if (this.on) this.#commitNow();
  }

  // ------------------------------------------------------------- internals

  #start() {
    this.on = true;
    this.#seekOnStart = true;
    this.#osd(loopText(this.a!, this.b!));
  }
  #stop(text: string) {
    this.on = false;
    this.#osd(text);
    this.#commitNow();
  }
  #showPoints() {
    if (this.a !== null && this.b !== null) this.#osd(loopText(this.a, this.b));
    else if (this.a !== null) this.#osd(`A-B 起点：${precise(this.a)}`);
    else if (this.b !== null) this.#osd(`A-B 止点：${precise(this.b)}`);
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
      this.#osd(code === "unsupported" ? "当前来源不支持区段循环" : `区段循环失败：${(error as Error).message}`);
    } finally {
      this.#pending--;
      this.#plugin.update();
    }
  }
}
