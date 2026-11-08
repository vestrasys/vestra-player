import type { Plugin, PlayerState, TimelineLayer } from "@vestra-player/sdk";
import { type Count, type SettingsStore } from "./settings.ts";
import { Failure, type Osd } from "./texts.ts";

/** Delays, in milliseconds; tests shorten them. */
export const loopTiming = {
  /** B23: nudges (held keys repeat) are committed to the player this long after the last one. */
  loopCommit: 100,
};

/** B23: the shortest span between A and B (seconds). */
export const MIN_SPAN = 0.2;
const EPS = 1e-6;
type Loop = { a: number; b: number; count: number | null };
const same = (x: Loop | null | undefined, y: Loop | null | undefined) =>
  (x ?? null) === (y ?? null) || (!!x && !!y && x.a === y.a && x.b === y.b && x.count === y.count);
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** Seconds to one decimal (the A and B points). */
export const round1 = (seconds: number) => Math.round(seconds * 10) / 10;
const two = (n: number) => String(n).padStart(2, "0");

/** The OSD's and the panel's point time: `m:ss.d`, `h:mm:ss.d` from one hour on. */
export function precise(seconds: number): string {
  const tenths = Math.max(0, Math.round(seconds * 10));
  const d = tenths % 10;
  const total = Math.floor(tenths / 10);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h ? `${h}:${two(m)}:${two(s)}.${d}` : `${m}:${two(s)}.${d}`;
}

/**
 * B23 current range: the span between the adjacent marks around `position`, the
 * start and end of the media counting as marks; `null` without marks.
 */
export function segmentAt(
  times: readonly number[],
  position: number,
  duration: number,
): { a: number; b: number } | null {
  if (!times.length) return null;
  const points = [...new Set([0, ...times.filter((t) => t > 0 && t < duration), duration])].sort((a, b) => a - b);
  for (let i = 0; i < points.length - 1; i++) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (position >= a && position < b) return { a, b };
  }
  return null;
}

/** Why the A-B commands are unavailable (shown as the commands' reason). */
export type LoopBlocked = "No media open" | "This source doesn't support A-B loop";

const hasMedia = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.source !== null;

/**
 * B23 A-B loop (moved here from the marks plugin; the Playback panel's A-B Loop
 * section and the `ab.*` commands). The plugin keeps A, B and the on/off
 * switch; the player runs the loop (`player.setLoop`) whenever both points are
 * set and the switch is on, and `setLoop(null)` otherwise. `state.loop` is the
 * truth for the switch: a loop set by another plugin shows as on, and the
 * switch turns it off. A new source clears A and B (the player clears the loop).
 *
 * Also here: the media's chapter starts (`player.chapters()`, once per source:
 * the panel's ticks and the current range), the pass the running loop is on
 * (counted from the jumps back the position shows), and a lone A as an `A`
 * marker in this plugin's seek-bar layer (`timeline.marks`).
 */
export class AbLoop {
  #a: number | null = null;
  #b: number | null = null;
  /** The switch as this plugin wants it (the player's `state.loop` may differ). */
  on = false;
  readonly #plugin: Plugin;
  readonly #settings: SettingsStore;
  readonly #osd: Osd;
  readonly #layer: TimelineLayer;
  /** What this plugin last got the player to run. */
  #committed: Loop | null = null;
  #pending = 0;
  #chain: Promise<void> = Promise.resolve();
  #timer: ReturnType<typeof setTimeout> | null = null;
  #seekOnStart = false;
  #sourceId: number | null = null;
  #lastLoop: Loop | null = null;
  #lastPosition: number | null = null;
  #pass = 1;
  /**
   * The last pass of a counted loop: the player clears `state.loop` at its last
   * jump back (mpv's count is the jumps back, so the range plays count + 1
   * times), so this keeps the range and the pass total until the position
   * leaves the range (passes B), the points change or the source changes.
   */
  #final: { a: number; b: number; total: number } | null = null;
  #chapters: number[] = [];
  #chaptersFor: number | null = null;
  /** What the seek-bar layer holds for this source (the player empties it on a new one). */
  #sent = "[]";

  constructor(plugin: Plugin, settings: SettingsStore, osd: Osd) {
    this.#plugin = plugin;
    this.#settings = settings;
    this.#osd = osd;
    this.#layer = plugin.timeline.layer();
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
    this.#final = null;
    this.#publish();
  }
  /** The end point B (seconds), `null` when unset. */
  get b(): number | null {
    return this.#b;
  }
  set b(value: number | null) {
    if (value === this.#b) return;
    this.#b = value;
    this.#final = null;
    this.#publish();
  }

  get count(): Count {
    return this.#settings.value.abCount;
  }
  /**
   * The pass the running loop is on (1 at its start, +1 on each jump back to A).
   * A counted loop plays `count + 1` passes (mpv's `ab-loop-count` is the jumps back).
   */
  get pass(): number {
    return this.#pass;
  }
  /** The pass total while the last pass of a counted loop plays (the player's loop is gone then); else `null`. */
  get finalPass(): number | null {
    return this.#final?.total ?? null;
  }
  /** The media's chapter starts, in order (empty until read, and for a new source). */
  get chapters(): readonly number[] {
    return this.#chapters;
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

  /** Follow the player: forget A and B on a new source; notice a finished or foreign loop; count the passes. */
  observe(state: PlayerState) {
    const loop = state.loop ?? null;
    if (state.sourceId !== this.#sourceId) {
      this.#sourceId = state.sourceId;
      this.#chapters = [];
      this.#chaptersFor = null;
      this.#sent = "[]"; // the player emptied every layer for the new source
      this.a = this.b = null;
      this.on = false;
      this.#committed = null;
      this.#seekOnStart = false;
      if (this.#timer) clearTimeout(this.#timer);
      this.#timer = null;
      this.#lastLoop = loop;
      this.#lastPosition = state.position;
      this.#pass = 1;
      this.#final = null;
      this.#readChapters(state);
      return;
    }
    this.#readChapters(state);
    const before = this.#lastLoop;
    this.#lastLoop = loop;
    const previous = this.#lastPosition;
    this.#lastPosition = state.position;
    const passes = this.#pass;
    if (!same(before, loop)) this.#pass = 1;
    else if (
      // Not while this plugin starts a loop (its own seek to A).
      !this.#pending &&
      loop &&
      previous !== null &&
      state.position !== null &&
      state.position < previous - 0.2 &&
      previous >= loop.b - 1 &&
      state.position <= loop.a + 1
    )
      // The player jumped back from B to A.
      this.#pass++;
    if (this.#final && !this.#pending) {
      const final = this.#final;
      if (loop !== null) this.#final = null;
      else if (state.position !== null && (state.position >= final.b - EPS || state.position < final.a - 1)) {
        // The last pass is over (playback went past B, or a seek left the range).
        this.#final = null;
        this.#osd("A-B loop ended");
        this.#plugin.update();
      }
    }
    if (this.#pending || same(before, loop)) return;
    if (loop === null && this.#committed !== null) {
      // The count ran out (or someone else stopped it): keep A and B.
      const ran = this.#committed;
      this.#committed = null;
      this.on = false;
      const position = state.position;
      if (
        ran.count !== null &&
        passes >= ran.count &&
        position !== null &&
        position >= ran.a - EPS &&
        position < ran.b - EPS
      )
        // Cleared at the last jump back: the last pass plays now; "ended" once it passes B.
        this.#final = { a: ran.a, b: ran.b, total: ran.count + 1 };
      else this.#osd("A-B loop ended");
    } else if (loop !== null && !same(loop, this.#committed)) {
      // Another plugin's loop replaced ours.
      this.#committed = null;
      this.on = false;
    }
  }

  /** Chapters: read with `player.chapters()` once per source, when it first is ready. */
  #readChapters(state: PlayerState) {
    if (this.#chaptersFor === state.sourceId || !hasMedia(state)) return;
    this.#chaptersFor = state.sourceId;
    void this.#plugin.player.chapters().then(
      ({ sourceId, chapters }) => {
        if (sourceId !== this.#sourceId) return;
        this.#chapters = chapters
          .map((chapter) => chapter.start)
          .filter((start) => Number.isFinite(start) && start >= 0)
          .sort((x, y) => x - y);
        this.#plugin.update();
      },
      () => {
        // Refused (source gone meanwhile): try again on the next ready state.
        if (this.#chaptersFor === state.sourceId) this.#chaptersFor = null;
      },
    );
  }

  /**
   * The current range's marks: the chapter starts and the bookmarks any plugin
   * shows on the seek bar (`timeline.marks`, such as the marks plugin's).
   */
  times(): number[] {
    const bookmarks = this.#plugin.timeline.marks.filter((mark) => mark.kind === "bookmark").map((mark) => mark.time);
    return [...this.#chapters, ...bookmarks];
  }

  // --------------------------------------------------------------- commands

  setA() {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    this.#placeA(clamp(round1(state.position), 0, state.duration!));
  }

  setB() {
    const state = this.#state;
    if (this.blocked(state) || state.position === null) return;
    this.#placeB(Math.min(round1(state.position), state.duration!));
  }

  /**
   * A typed point (the panel's A or B field): placed as `[` / `]` would at that
   * time. `false` when refused (outside the media, or B not after A); the reason
   * is on the video.
   */
  type(point: "a" | "b", seconds: number): boolean {
    const state = this.#state;
    if (this.blocked(state)) return false;
    if (!Number.isFinite(seconds) || seconds < 0 || seconds > state.duration! + EPS) {
      new Failure("time-range").show(this.#osd);
      return false;
    }
    const at = Math.min(round1(seconds), state.duration!);
    return point === "a" ? this.#placeA(at) : this.#placeB(at);
  }

  #placeA(at: number): boolean {
    if (this.b !== null && this.b - at < MIN_SPAN - EPS) {
      this.b = null;
      this.#osd("Start point is after the end point; end point cleared");
    } else if (this.b === null) this.#osd("A-B start: {}", [precise(at)]);
    this.a = at;
    if (this.b !== null) this.#start();
    else this.on = false;
    this.#commitNow();
    return true;
  }

  #placeB(at: number): boolean {
    const a = this.a ?? 0;
    if (at - a < MIN_SPAN - EPS) {
      this.#osd("End point must be after the start point");
      return false;
    }
    this.a = a;
    this.b = at;
    this.#start();
    this.#commitNow();
    return true;
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

  /** `\` can run: A and B are set, or a loop runs (another plugin's too, so it can always stop one). */
  toggleable(): boolean {
    return (this.a !== null && this.b !== null) || !!this.#state.loop;
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
  loopSegment(times: readonly number[] = this.times()) {
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
    this.#settings.update((s) => {
      s.abCount = count;
    });
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

  /**
   * B22: while only A is set, an `A` marker on the seek bar (this plugin's
   * layer); with B too the loop span is the player's to draw. Written for the
   * source it belongs to (`ifCurrent`), and only when it changed.
   */
  #publish() {
    const state = this.#state;
    const a = this.#a;
    const show = a !== null && this.#b === null && state.duration !== null && a <= state.duration;
    const marks = show ? [{ time: a, kind: "marker" as const, title: "A" }] : [];
    const sent = JSON.stringify(marks);
    if (sent === this.#sent) return;
    this.#sent = sent;
    this.#layer.set(marks, { ifCurrent: state });
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
  /** Held nudges: commit once they pause for `loopTiming.loopCommit`. */
  #commitLater() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#chain = this.#chain.then(() => this.#flush());
    }, loopTiming.loopCommit);
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
      this.#final = null;
      this.#lastLoop = player.state.loop ?? null;
      this.#pass = 1;
      const position = player.state.position;
      if (want && seek && position !== null && (position < want.a || position >= want.b))
        await player.seek(want.a).catch(() => {});
      this.#lastPosition = player.state.position;
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
