import { mkdir, realpath, rename, rm, stat, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { ImageRef, PlayerState, Plugin } from "@vestra-player/sdk";
import { isLocalPath } from "./local-path.ts";
import { cacheKey, type CacheIndex, CACHE_VERSION, evict, readIndex, writeIndex } from "./preview-cache.ts";
import type { SettingsStore } from "./settings.ts";
import {
  CELL_BYTES,
  CELL_HEIGHT,
  CELL_WIDTH,
  cellCount,
  cellTime,
  coarseToFine,
  exactCellTime,
  nearestMade,
  type Part,
  partEstimateMs,
  partition,
  type Plan,
  placeCell,
  plan,
  sheetCount,
  sheetSize,
  stripCell,
  tooSlow,
  type VideoFacts,
} from "./sprite.ts";
import type { State } from "./texts.ts";

export const NEEDS_FFMPEG = "Needs ffmpeg";
/** ffmpeg processes at most (cells and sheet writes together). */
export const WORKERS = 2;
/** Failures in a row after which a source is not tried again. */
export const MAX_FAILURES = 5;
/** How often the plugin looks for the source's first shown frame. */
export const FRAME_POLL_MS = 250;
/** Generation starts this long after the source's first frame was shown. */
export const START_DELAY_MS = 1000;
/** Ready but no new frame (opened paused): start this long after ready anyway. */
export const START_FALLBACK_MS = 3000;
/** Sheets are written again at most this often while cells come in. */
export const PUBLISH_MS = 1000;
/**
 * 0.2.3: while the pointer is on the seek line, changed sheets are handed to
 * the bar at most this often (`PUBLISH_MS` still rules the disk writes).
 *
 * The trade-off: the player decodes every sheet it is handed on its UI thread
 * (11.5 ms p50 for a 1600×900 JPEG, `2026-10-ui-redraw-stutter.md`) and
 * uploads it as a texture (about 4 ms), so a sheet a second during the 15 s or
 * so of generation stalled the UI thread, and the picture under the pointer,
 * for up to 15–25 ms once a second while the user hovered the bar. At 2.5 s
 * the thumbnails under a resting pointer fill in a little later (the cell
 * under the pointer is already made once its sheet is handed over; the
 * nearest made one shows until then), and a sheet nobody changed keeps its
 * reference, so a run that touched only one of the two sheets costs one
 * decode, not two. Nothing is held back while nobody points: the bar sends no
 * sheets then, and the pointer's arrival sends the newest at once.
 */
export const SHOW_MS = 2500;
/** After a seek, or the end of a drag, generation waits this long. */
export const SEEK_QUIET_MS = 1000;
/** A jump of the position by more than this (seconds) counts as a seek. */
export const SEEK_JUMP_S = 1.5;
export const CELL_TIMEOUT_MS = 10_000;
/**
 * An exact part decodes 8 s of video or more: within the budget a whole pass
 * is about 15 s, so one part on a slow machine stays well under this.
 */
export const PART_TIMEOUT_MS = 60_000;
/** The keyframe list of an exact pass (ffprobe): about 20 bytes per packet. */
export const KEYFRAMES_MAX_OUTPUT = 8 * 1024 * 1024;
/** The timers' clock; tests replace it to run time themselves. */
export const clock = {
  now: () => Date.now(),
  setTimeout: (run: () => void, ms: number): unknown => setTimeout(run, ms),
  clearTimeout: (timer: unknown) => clearTimeout(timer as ReturnType<typeof setTimeout>),
};

/** How a sprite's `index.json` is written; tests hold it to play a slow disk. */
export const indexFile = { write: writeIndex };
/**
 * 0.2.12, best effort: on stop the plugin waits this long at most (real time)
 * for its pending disk work (index and sheet writes, clean-ups); a cache
 * clean-up waits as long for an ended source's index writes. Short: the
 * player gives a stopping plugin 300 ms in the shared runner (more counts as
 * leaked). Folders still being written are kept by the clean-up in any case.
 */
export const SETTLE_MS = 250;
/** The wait in use (`SETTLE_MS`); tests lengthen it to hold a clean-up back. */
export const settle = { ms: SETTLE_MS };

/** Cells one ffmpeg run makes at most: fewer process starts (each costs ~0.2 s, and virus scanners react to it). */
export const BATCH = 5;

/**
 * Cells at `seconds` (one input each), side by side as one strip of raw rgb24
 * on stdout, each padded to 160×90 (letterboxed); no file per cell.
 *
 * - Keyframes only (`-skip_frame nokey`); `-ss` before `-i` with
 *   `-noaccurate_seek` takes the keyframe at or before the time instead of
 *   decoding on to it (without it ffmpeg waits for the next keyframe, which
 *   near the end may not exist).
 * - `-copyts`: without it a cell after the file's last keyframe comes out
 *   empty (ffmpeg n9, measured); `setpts=0` then lines the inputs up for
 *   `hstack` without decoding more frames.
 * - One thread per decoder and for the filters: a keyframe gains nothing from
 *   more, and the player keeps the other cores.
 *
 * Measured on 4K HEVC: each strip slot is byte-identical to a run of its own,
 * at ~14 % less CPU per cell and 5× fewer process starts.
 */
export function cellsArgs(source: string, seconds: readonly number[]): string[] {
  const args = ["-nostdin", "-v", "error", "-filter_threads", "1"];
  const cell = `scale=${CELL_WIDTH}:${CELL_HEIGHT}:force_original_aspect_ratio=decrease,pad=${CELL_WIDTH}:${CELL_HEIGHT}:(ow-iw)/2:(oh-ih)/2,format=rgb24`;
  let graph = "";
  let labels = "";
  seconds.forEach((time, i) => {
    args.push("-threads", "1", "-skip_frame", "nokey", "-noaccurate_seek", "-ss", time.toFixed(3), "-copyts", "-i", source);
    graph += `[${i}:v:0]trim=end_frame=1,setpts=0,${cell}[c${i}];`;
    labels += `[c${i}]`;
  });
  graph += seconds.length > 1 ? `${labels}hstack=inputs=${seconds.length}` : `${labels}null`;
  return [...args, "-filter_complex", graph, "-frames:v", "1", "-f", "rawvideo", "-"];
}

/**
 * Exact mode (0.2.2): one sequential decode of the cells `first` … `first + n - 1`
 * (of `count` over `duration` seconds), starting at the keyframe at `start`
 * seconds, as `n` frames of raw rgb24 on stdout in cell order.
 *
 * - Each cell is the frame on screen at its time: the `fps` filter resamples
 *   the stream to one frame per span on a grid shifted (`setpts`) so that
 *   tick k is cell k's time, `round=up` placing every frame on the tick at or
 *   after it, so each tick keeps the last frame at or before it, and a span
 *   without a frame (a slideshow, a still stretch of a screen recording)
 *   repeats the previous frame instead of shifting every later cell by one.
 *   `-frames:v` ends the run after the last cell (the rest of the file is not
 *   read). Verified frame by frame on 30 fps, 1 fps and a 4 s frameless
 *   stretch (round notes, 2026-10-27).
 * - `-ss` at the keyframe: the demuxer starts there and the default accurate
 *   seek keeps the keyframe itself (a frame is never before its own time), so
 *   a cell right after a keyframe followed by a still stretch shows the
 *   keyframe. Timestamps are then relative to the seek point.
 * - Software decoding, one thread, like the keyframe runs: two parts run at a
 *   time. No `-hwaccel`: the video engine belongs to the player
 *   (`DECODE_NS_PER_PIXEL`).
 */
export function exactArgs(source: string, part: Part, count: number, duration: number): string[] {
  const span = duration / count;
  const start = part.start;
  const first = exactCellTime(part.first, count, duration) - start;
  const args = ["-nostdin", "-v", "error", "-threads", "1"];
  if (start > 0) args.push("-ss", start.toFixed(3));
  args.push("-i", source, "-an", "-sn", "-dn", "-filter_threads", "1");
  const cell = `scale=${CELL_WIDTH}:${CELL_HEIGHT}:force_original_aspect_ratio=decrease,pad=${CELL_WIDTH}:${CELL_HEIGHT}:(ow-iw)/2:(oh-ih)/2,format=rgb24`;
  args.push("-vf", `setpts=PTS-${first.toFixed(6)}/TB,fps=fps=1/${span.toFixed(6)}:start_time=0:round=up,${cell}`);
  return [...args, "-fps_mode", "passthrough", "-frames:v", String(part.n), "-f", "rawvideo", "-"];
}

/** ffprobe: every video packet's time and flags (`K` first for keyframes), one per line. */
export const keyframesArgs = (source: string) => [
  "-v",
  "error",
  "-select_streams",
  "v:0",
  "-show_entries",
  "packet=pts_time,flags",
  "-of",
  "csv=p=0",
  source,
];

/** The keyframe times in `keyframesArgs` output (packets without a time are skipped). */
export function parseKeyframes(output: string): number[] {
  const times: number[] = [];
  for (const line of output.split("\n")) {
    const [time, flags] = line.trim().split(",");
    if (flags?.startsWith("K") && time !== undefined && time !== "N/A" && Number.isFinite(Number(time))) times.push(Number(time));
  }
  return times.sort((a, b) => a - b);
}

/** A sheet's raw pixels (`raw`) written as `out` (JPEG). */
export const sheetArgs = (raw: string, width: number, height: number, out: string) => [
  "-nostdin",
  "-v",
  "error",
  "-threads",
  "1",
  "-f",
  "rawvideo",
  "-pix_fmt",
  "rgb24",
  "-s",
  `${width}x${height}`,
  "-i",
  raw,
  "-frames:v",
  "1",
  "-q:v",
  "4",
  "-y",
  out,
];

/** A cached sheet read back as raw pixels (to go on with a sprite left half made). */
export const readSheetArgs = (file: string) => [
  "-nostdin",
  "-v",
  "error",
  "-threads",
  "1",
  "-i",
  file,
  "-f",
  "rawvideo",
  "-pix_fmt",
  "rgb24",
  "-",
];

/**
 * What the bar draws from: `count` cells of 160×90 in 10×10 sheets, and per
 * cell the made cell to show (-1: none yet). Local files get it from the
 * sprite made here; a network source could later fill the same shape from the
 * site's own preview sheets (Bilibili `videoshot`, YouTube storyboards), which
 * needs a host API first (seek-thumbnails research §3.7).
 */
export type Preview = { count: number; map: number[]; sheets: (ImageRef | null)[] };

type Job = {
  sourceId: number;
  path: string;
  duration: number;
  /** Cells (known once the plan is made at the start; 0 before). */
  count: number;
  /** How the sprite is made: decided at the start from the video facts. */
  plan: Plan | null;
  /** `stats().video` seen while waiting for the first frame. */
  video: VideoFacts | null;
  /** Exact mode failed for this source (or was never possible): keyframes it is. */
  keyframesOnly: boolean;
  /** Exact mode: the parts of the pass (cells in coarse-to-fine part order fill `queue`). */
  parts: Part[];
  abort: AbortController;
  /** waiting: for the first frame; running: making cells; done: nothing left (or failed). */
  stage: "waiting" | "starting" | "running" | "done";
  /** framesShown when the source was first seen ready; when it was. */
  frames: number | null;
  readyAt: number | null;
  /** When the first new frame was seen. */
  shownAt: number | null;
  dir: string | null;
  key: string | null;
  sheets: Uint8Array[];
  made: Uint8Array;
  failed: Set<number>;
  queue: number[];
  /** Cells being made, and the run making them. */
  running: Map<number, AbortController>;
  failures: number;
  /** Cells of a failed batch: each runs alone next. */
  alone: Set<number>;
  /** Sheets with cells not yet written to disk. */
  dirty: Set<number>;
  writing: boolean;
  /** When the last sheet write started (throttle). */
  wroteAt: number;
  /** Sheets written to disk since the bar last got them (`SHOW_MS` throttle while pointing). */
  unshown: Set<number>;
  /** The index writes, one after another in call order (the newest lands last). */
  indexed: Promise<void>;
};

/** The part a cell belongs to (every queued cell has one; a cell without is decoded from the start on its own). */
const partOf = (job: Job, cell: number): Part =>
  job.parts.find((p) => cell >= p.first && cell < p.first + p.n) ?? { first: cell, n: 1, start: 0 };

/**
 * I07 Show Thumbnail on Seek Bar Hover (0.2.0: also while dragging): a
 * sprite of each local video, made in the background once the source's first
 * frame is on screen, and kept on disk across sessions.
 *
 * - Never before the first frame, never competing with an open: a new source
 *   ends the old one's ffmpeg at once.
 * - At most `WORKERS` ffmpeg processes, one thread each; cells coarse to fine.
 * - Paused while the user drags the seek line, and for `SEEK_QUIET_MS` after a
 *   seek or a drag: running cells are ended and done again later.
 * - Sheets reach the disk every `PUBLISH_MS` while cells come in, and the bar
 *   at most every `SHOW_MS` while the pointer is on the seek line (0.2.3).
 * - 0.2.2: files cheap enough to decode whole (`plan`) get exact cells from
 *   one sequential pass in keyframe-aligned parts, software decoding; the
 *   others keyframe cells. An exact pass that fails, or whose part takes over
 *   twice its estimate (`tooSlow`), falls back to the keyframe sprite.
 */
export class Thumbnails {
  /** ffmpeg was found (`null` until the first answer). */
  available: boolean | null = null;
  /** The sprite the bar shows, `null` when the source has none (yet). */
  preview: Preview | null = null;
  #root: string | null = null;
  #seen: number | null = null;
  #job: Job | null = null;
  #timer: unknown = null;
  #due = 0;
  #slots = WORKERS;
  /** Work waiting for a slot, first come first served. */
  #waiting: (() => void)[] = [];
  #held = false;
  #quietUntil = 0;
  /** The pointer is on the seek line (hover or drag): the bar is sending the sheets. */
  #pointing = false;
  /** When the bar last got a changed sheet. */
  #shownAt = 0;
  #last: { position: number; at: number; paused: boolean; rate: number } | null = null;
  /** Disk work nobody awaits: ended sources' index writes and file removals, cache clean-ups. */
  readonly #pending = new Set<Promise<unknown>>();
  /** Ended jobs whose index writes have not landed: a clean-up keeps their folders. */
  readonly #ending = new Set<Job>();
  readonly #scope: Plugin;
  readonly #settings: SettingsStore;
  readonly #video: (sourceId: number) => boolean | null;

  /** `video`: whether the source has video (`stats().video`), `null` while unknown. */
  constructor(scope: Plugin, settings: SettingsStore, video: (sourceId: number) => boolean | null) {
    this.#scope = scope;
    this.#settings = settings;
    this.#video = video;
    try {
      this.#root = join(scope.dataDirectory, "thumbs");
    } catch {
      this.#root = null; // no data directory: nowhere to keep pictures
    }
    scope.onDispose(() => this.#end());
  }

  /** Look for ffmpeg (at start, and on each new source while it is missing). */
  async probe() {
    const found = this.#root !== null && (await this.#scope.tools.available("ffmpeg").catch(() => false));
    this.available = found;
    this.#scope.update();
    this.#schedule(0);
  }

  command(): State {
    const missing = this.available === false;
    return {
      enabled: !missing,
      checked: this.#settings.value.hoverThumbnail,
      busy: false,
      reason: missing ? NEEDS_FFMPEG : null,
    };
  }

  /** The bar draws the picture box: the option is on and this source can have a sprite. */
  active(state: PlayerState = this.#scope.player.state): boolean {
    const job = this.#job;
    return (
      this.#settings.value.hoverThumbnail &&
      this.available === true &&
      job !== null &&
      job.sourceId === state.sourceId &&
      job.failures < MAX_FAILURES &&
      state.phase === "ready" &&
      this.#video(state.sourceId) === true
    );
  }

  /** Call on every state change: a new source ends the old one's work. */
  observe(state: PlayerState) {
    this.#seeks(state);
    if (this.#job?.sourceId === state.sourceId) {
      // Phase or video facts may have changed; a pending look (the frame poll) stays.
      if (this.#timer === null) this.#schedule(0);
      return;
    }
    if (this.#seen !== state.sourceId) {
      this.#seen = state.sourceId;
      void this.#end();
      if (this.available === false && this.#root !== null) void this.probe(); // ffmpeg may be installed by now
    }
    // A local file with a known length (it may be known only once ready).
    const { kind, source, duration } = state;
    if (kind === "media" && source !== null && isLocalPath(source) && cellCount(duration) > 0 && this.#root !== null) {
      this.#job = this.#fresh(state.sourceId, source, duration!);
      this.#seeks(state); // the position a later jump is measured from
      this.#schedule(0);
    }
  }

  /**
   * The pointer came onto the seek line (`true`) or left it. While it is there
   * a changed sheet waits for `SHOW_MS` since the last one; when it leaves, a
   * waiting sheet goes out at once (the bar sends no sheets to decode then).
   */
  pointing(on: boolean) {
    if (on === this.#pointing) return;
    this.#pointing = on;
    const job = this.#job;
    if (!on && job && job.unshown.size > 0) this.#show(job);
  }

  /** The seek line is being dragged (`true`) or was let go. */
  hold(held: boolean) {
    if (held === this.#held) return;
    this.#held = held;
    if (held) {
      const job = this.#job;
      // Running cells give the CPU back at once and are made again later.
      if (job) for (const run of job.running.values()) run.abort();
    } else {
      this.#quiet();
    }
  }

  // ---------------------------------------------------------------- timing

  /** A position jump that playback does not explain is a seek: wait a moment. */
  #seeks(state: PlayerState) {
    const now = clock.now();
    const last = this.#last;
    if (state.phase !== "ready" || state.position === null || this.#job?.sourceId !== state.sourceId) {
      this.#last = null;
      return;
    }
    if (last) {
      const expected = last.paused ? last.position : last.position + ((now - last.at) / 1000) * last.rate;
      if (Math.abs(state.position - expected) > SEEK_JUMP_S) this.#quiet();
    }
    this.#last = { position: state.position, at: now, paused: state.paused, rate: state.rate };
  }

  #quiet() {
    this.#quietUntil = Math.max(this.#quietUntil, clock.now() + SEEK_QUIET_MS);
    this.#schedule(SEEK_QUIET_MS);
  }

  /** Look again in `ms` (a pending look that comes sooner stays). */
  #schedule(ms: number) {
    const due = clock.now() + ms;
    if (this.#timer !== null) {
      if (this.#due <= due) return;
      clock.clearTimeout(this.#timer);
    }
    this.#due = due;
    this.#timer = clock.setTimeout(() => {
      this.#timer = null;
      void this.#step();
    }, ms);
  }

  #clearTimer() {
    if (this.#timer !== null) clock.clearTimeout(this.#timer);
    this.#timer = null;
  }

  /** Move the current source's job on: wait for its first frame, start, keep the workers busy. */
  async #step() {
    const job = this.#job;
    if (!job || job.abort.signal.aborted) return;
    if (job.unshown.size > 0) this.#publish(job); // a sheet held back by the throttle, due now
    const state = this.#scope.player.state;
    if (job.stage === "waiting") return this.#watch(job, state);
    if (job.stage === "running") this.#pump(job);
  }

  /** Before generation: the source must be ready, with video, and its first frame shown. */
  async #watch(job: Job, state: PlayerState) {
    if (state.sourceId !== job.sourceId || state.phase !== "ready" || this.#video(job.sourceId) !== true) {
      if (state.phase === "loading" || this.#video(job.sourceId) === null) this.#schedule(FRAME_POLL_MS);
      return;
    }
    const now = clock.now();
    if (job.shownAt === null) {
      const stats = await this.#scope.player.stats().catch(() => null);
      if (this.#job !== job || job.abort.signal.aborted) return;
      const frames = stats?.framesShown ?? null;
      if (stats?.video) job.video = stats.video;
      if (job.frames === null) {
        // The counter is the player's own (all sources): a frame drawn after
        // this one is surely this source's.
        job.frames = frames;
        job.readyAt = now;
      } else if (frames !== null && frames > job.frames) {
        job.shownAt = now;
      }
      if (job.shownAt === null && now - job.readyAt! >= START_FALLBACK_MS) job.shownAt = job.readyAt! + START_FALLBACK_MS - START_DELAY_MS;
      if (job.shownAt === null) return this.#schedule(FRAME_POLL_MS);
    }
    const wait = job.shownAt + START_DELAY_MS - now;
    if (wait > 0) return this.#schedule(wait);
    if (!this.#settings.value.hoverThumbnail || this.available !== true) return this.#schedule(1000); // the option may come on
    job.stage = "starting";
    await this.#start(job);
  }

  // ---------------------------------------------------------------- work

  #fresh(sourceId: number, path: string, duration: number): Job {
    return {
      sourceId,
      path,
      duration,
      count: 0,
      plan: null,
      video: null,
      keyframesOnly: false,
      parts: [],
      abort: new AbortController(),
      stage: "waiting",
      frames: null,
      readyAt: null,
      shownAt: null,
      dir: null,
      key: null,
      sheets: [],
      made: new Uint8Array(0),
      failed: new Set(),
      queue: [],
      running: new Map(),
      failures: 0,
      alone: new Set(),
      dirty: new Set(),
      writing: false,
      wroteAt: 0,
      unshown: new Set(),
      indexed: Promise.resolve(),
    };
  }

  /** The sprite folder: reuse what an earlier session made, or begin a new one. */
  async #start(job: Job) {
    try {
      const real = await realpath(job.path);
      const { size, mtimeMs } = await stat(real);
      let made = plan(job.duration, job.keyframesOnly ? null : job.video);
      if (made.mode === "exact") {
        // An earlier session found the pass failing or too slow for this file: keyframes, no new try.
        const earlier = await readIndex(join(this.#root!, cacheKey({ path: real, size, mtimeMs }, made.count, "exact")));
        if (earlier?.keyframesOnly) made = plan(job.duration, null);
      }
      job.plan = made;
      job.count = made.count;
      job.made = new Uint8Array(made.count);
      const key = cacheKey({ path: real, size, mtimeMs }, job.count, made.mode);
      const dir = join(this.#root!, key);
      job.key = key; // before the folder exists: a clean-up running meanwhile keeps it
      await mkdir(dir, { recursive: true });
      if (this.#job !== job || job.abort.signal.aborted) return;
      job.dir = dir;
      const index = await readIndex(dir);
      const sheets = sheetCount(job.count);
      job.sheets = Array.from({ length: sheets }, (_, s) => {
        const { width, height } = sheetSize(s, job.count);
        return new Uint8Array(width * height * 3);
      });
      if (index && index.count === job.count) {
        for (const i of index.made) job.made[i] = 1;
        for (const i of index.failed) job.failed.add(i);
        if (!index.complete) await this.#readBack(job);
      }
      if (this.#job !== job || job.abort.signal.aborted) return;
      const left = (i: number) => !job.made[i] && !job.failed.has(i);
      if (made.mode === "exact" && job.made.some((m) => !m)) {
        // Parts begin at keyframes; cells go in part order (parts coarse to fine).
        job.parts = partition(job.count, job.duration, await this.#keyframes(job));
        if (this.#job !== job || job.abort.signal.aborted) return;
        job.queue = job.parts.flatMap((part) => Array.from({ length: part.n }, (_, i) => part.first + i)).filter(left);
      } else {
        job.queue = coarseToFine(job.count).filter(left);
      }
      job.stage = "running";
      if (job.made.some(Boolean)) this.#show(job);
      void this.#saveIndex(job);
      this.#evict(key);
      if (job.queue.length === 0) return this.#finish(job);
      this.#pump(job);
    } catch {
      if (this.#job === job) job.stage = "done"; // the file went away: no pictures
    }
  }

  /** The file's keyframe times (ffprobe, in a slot); none when it cannot say: one part then. */
  async #keyframes(job: Job): Promise<number[]> {
    try {
      const { stdout } = await this.#take(() =>
        this.#scope.tools.run("ffprobe", keyframesArgs(job.path), {
          timeoutMs: CELL_TIMEOUT_MS,
          maxOutput: KEYFRAMES_MAX_OUTPUT,
          signal: job.abort.signal,
        }),
      );
      return parseKeyframes(new TextDecoder().decode(stdout));
    } catch {
      return [];
    }
  }

  /** Cells made in an earlier session come back from the cached sheets (one ffmpeg each). */
  async #readBack(job: Job) {
    for (let s = 0; s < job.sheets.length; s++) {
      const file = join(job.dir!, `sheet-${s}.jpg`);
      const ok = await this.#take(async () => {
        const { stdout } = await this.#scope.tools.run("ffmpeg", readSheetArgs(file), {
          timeoutMs: CELL_TIMEOUT_MS,
          maxOutput: job.sheets[s]!.length,
          signal: job.abort.signal,
        });
        if (stdout.length !== job.sheets[s]!.length) throw new Error("sheet size changed");
        job.sheets[s]!.set(stdout);
      }).then(
        () => true,
        () => false,
      );
      if (ok || job.abort.signal.aborted) continue;
      // Unreadable: its cells are made again.
      for (let i = s * 100; i < Math.min(job.count, (s + 1) * 100); i++) job.made[i] = 0;
    }
  }

  /** Run `work` in one of the `WORKERS` slots (taken at once when one is free). */
  async #take<T>(work: () => Promise<T>): Promise<T> {
    if (this.#slots > 0) this.#slots--;
    else await new Promise<void>((resolve) => this.#waiting.push(resolve)); // handed over by a release
    try {
      return await work();
    } finally {
      const next = this.#waiting.shift();
      if (next) next();
      else this.#slots++;
      // Also wakes a newer source's job, waiting for a slot an ended run held.
      if (this.#job) this.#schedule(0);
    }
  }

  #paused() {
    return this.#held || clock.now() < this.#quietUntil;
  }

  /** Start cells (and sheet writes) while slots are free and nothing holds the work. */
  #pump(job: Job) {
    if (this.#job !== job || job.stage !== "running" || job.abort.signal.aborted) return;
    if (this.#paused()) {
      if (!this.#held) this.#schedule(Math.max(1, this.#quietUntil - clock.now()));
      return;
    }
    if (!this.#settings.value.hoverThumbnail) return this.#schedule(1000);
    // Sheets go to disk when due (the first cells at once), taking a slot like a cell.
    const writeDue = job.dirty.size > 0 && !job.writing && clock.now() - job.wroteAt >= PUBLISH_MS;
    if (writeDue && this.#slots > 0) void this.#write(job);
    // Not due yet: look again when it is. (Due without a free slot: the next release looks.)
    else if (!writeDue && job.dirty.size > 0 && !job.writing) this.#schedule(PUBLISH_MS - (clock.now() - job.wroteAt));
    while (this.#slots > 0 && job.queue.length > 0) void this.#cells(job, this.#batch(job));
    if (job.queue.length === 0 && job.running.size === 0 && job.dirty.size === 0 && !job.writing) this.#finish(job);
  }

  /**
   * The next cells for one run. Keyframe mode: up to `BATCH` in queue order; a
   * cell of a failed batch runs alone. Exact mode: the rest of the part the
   * next cell belongs to.
   */
  #batch(job: Job): number[] {
    if (job.plan?.mode === "exact") {
      const part = partOf(job, job.queue[0]!);
      const cells: number[] = [];
      while (job.queue.length > 0 && job.queue[0]! >= part.first && job.queue[0]! < part.first + part.n) cells.push(job.queue.shift()!);
      return cells;
    }
    if (job.alone.has(job.queue[0]!)) return [job.queue.shift()!];
    const cells: number[] = [];
    while (cells.length < BATCH && job.queue.length > 0 && !job.alone.has(job.queue[0]!)) cells.push(job.queue.shift()!);
    return cells;
  }

  async #cells(job: Job, cells: number[]) {
    const run = new AbortController();
    const end = () => run.abort();
    job.abort.signal.addEventListener("abort", end, { once: true });
    for (const index of cells) job.running.set(index, run);
    const exact = job.plan?.mode === "exact";
    try {
      if (exact) {
        // The part's cells from the first left to the last (made ones in between are made again).
        const part = partOf(job, cells[0]!);
        const first = Math.min(...cells);
        const n = Math.max(...cells) - first + 1;
        let began = 0;
        const { stdout } = await this.#take(() => {
          began = clock.now();
          return this.#scope.tools.run("ffmpeg", exactArgs(job.path, { first, n, start: part.start }, job.count, job.duration), {
            timeoutMs: PART_TIMEOUT_MS,
            maxOutput: CELL_BYTES * n,
            signal: run.signal,
          });
        });
        // Slower than the estimate allows: this machine (or this moment) cannot afford the pass.
        if (tooSlow(clock.now() - began, partEstimateMs({ first, n, start: part.start }, job.count, job.duration, job.video!))) throw new Error("too slow");
        const got = Math.floor(stdout.length / CELL_BYTES);
        if (got === 0) throw new Error("no picture");
        for (let slot = 0; slot < n; slot++) {
          const index = first + slot;
          const s = Math.floor(index / 100);
          if (slot < got) {
            placeCell(job.sheets[s]!, sheetSize(s, job.count).width, index, stdout.subarray(slot * CELL_BYTES, (slot + 1) * CELL_BYTES));
            job.made[index] = 1;
            job.dirty.add(s);
          } else job.failed.add(index); // the stream ended before the cell's time: its neighbour shows
        }
        job.failures = 0;
        return;
      }
      const seconds = cells.map((index) => cellTime(index, job.count, job.duration));
      const { stdout } = await this.#take(() =>
        this.#scope.tools.run("ffmpeg", cellsArgs(job.path, seconds), {
          timeoutMs: CELL_TIMEOUT_MS,
          maxOutput: CELL_BYTES * cells.length,
          signal: run.signal,
        }),
      );
      if (stdout.length !== CELL_BYTES * cells.length) throw new Error("no picture");
      cells.forEach((index, slot) => {
        const s = Math.floor(index / 100);
        placeCell(job.sheets[s]!, sheetSize(s, job.count).width, index, stripCell(stdout, slot, cells.length));
        job.made[index] = 1;
        job.dirty.add(s);
      });
      job.failures = 0;
    } catch (error) {
      if (job.abort.signal.aborted) return;
      if (run.signal.aborted) {
        job.queue.unshift(...cells); // ended for a drag: made again later
        return;
      }
      if ((error as { code?: string })?.code === "unavailable") {
        this.available = false; // ffmpeg went away: the menu item turns grey
        job.queue.length = 0;
        job.stage = "done";
        this.#scope.update();
        return;
      }
      if (exact) {
        // The pass cannot run (a file ffmpeg cannot read) or is too slow here: keyframes instead.
        this.#fallback(job);
        return;
      }
      if (cells.length > 1) {
        // One bad position fails the whole run: each is tried again on its own.
        for (const index of cells) job.alone.add(index);
        job.queue.unshift(...cells);
        return;
      }
      // A failed cell shows its neighbour, without a message.
      job.failed.add(cells[0]!);
      job.failures++;
      if (job.failures >= MAX_FAILURES) {
        job.queue.length = 0;
        this.#scope.update();
      }
    } finally {
      job.abort.signal.removeEventListener("abort", end);
      for (const index of cells) job.running.delete(index);
    }
  }

  /**
   * An exact pass that failed: the source gets the keyframe sprite instead, as
   * a fresh job (the old one's other runs end; their late answers are ignored).
   * The exact folder's index remembers it (`keyframesOnly`), so the next open
   * of the file goes straight to keyframes; the folder ages out with the others.
   */
  #fallback(job: Job) {
    if (this.#job !== job) return;
    job.abort.abort();
    const remembered = this.#saveIndex(job, true);
    this.#hold(job); // a stop meanwhile waits for it too; clean-ups keep the folder
    const next = this.#fresh(job.sourceId, job.path, job.duration);
    next.video = job.video;
    next.keyframesOnly = true;
    next.stage = "starting";
    this.#job = next;
    this.preview = null;
    this.#scope.update();
    // After the exact folder's index is on disk: the new job's start evicts
    // folders without a valid index, and that one is not kept.
    void remembered.then(() => {
      if (this.#job === next) void this.#start(next);
    });
  }

  /** Write the changed sheets (raw → JPEG by ffmpeg, then renamed into place) and show them. */
  async #write(job: Job) {
    job.writing = true;
    job.wroteAt = clock.now();
    const sheets = [...job.dirty];
    job.dirty.clear();
    try {
      // One slot for all of them, taken now (the pump saw it free). Tracked: a stop waits for it.
      await this.#tracked(this.#take(async () => {
        for (const s of sheets) {
          const { width, height } = sheetSize(s, job.count);
          const raw = join(job.dir!, `sheet-${s}.rgb`);
          const fresh = join(job.dir!, `sheet-${s}.new.jpg`);
          await writeFile(raw, job.sheets[s]!);
          try {
            await this.#scope.tools.run("ffmpeg", sheetArgs(raw, width, height, fresh), {
              timeoutMs: CELL_TIMEOUT_MS,
              maxOutput: 64 * 1024,
              signal: job.abort.signal,
            });
            await rename(fresh, join(job.dir!, `sheet-${s}.jpg`));
          } finally {
            await unlink(raw).catch(() => {});
          }
        }
      }));
      if (this.#job !== job || job.abort.signal.aborted) return;
      for (const s of sheets) job.unshown.add(s);
      this.#publish(job);
      // Not awaited: while `writing` is set no sheet write starts, so a slow index
      // write would hand a landed run's free slot to the next cells and keep the
      // changed sheets off the disk (and the bar) until another run ends.
      void this.#saveIndex(job);
    } catch {
      if (job.abort.signal.aborted) return;
      for (const s of sheets) job.dirty.add(s); // tried again at the next write
    } finally {
      job.writing = false;
      if (this.#job === job) this.#schedule(0);
    }
  }

  /**
   * Hand the written sheets to the bar: now, or, while the pointer is on the
   * seek line and the bar got a sheet less than `SHOW_MS` ago, when that time
   * is up (a timer; `pointing(false)` also ends the wait).
   */
  #publish(job: Job) {
    const wait = this.#shownAt + SHOW_MS - clock.now();
    if (this.#pointing && this.preview !== null && wait > 0) return this.#schedule(wait);
    this.#show(job);
  }

  /**
   * Hand the sheets on disk to the bar. A changed sheet gets a new reference
   * (the player reads the file again); an unchanged one keeps its reference,
   * so the player does not decode it again.
   */
  #show(job: Job) {
    const previous = this.preview?.count === job.count ? this.preview.sheets : [];
    const sheets = Array.from({ length: sheetCount(job.count) }, (_, s) => {
      const first = s * 100;
      const any = job.made.subarray(first, Math.min(job.count, first + 100)).some(Boolean);
      if (!any) return null;
      const kept = previous[s];
      return kept && !job.unshown.has(s) ? kept : this.#scope.ui.image(join(job.dir!, `sheet-${s}.jpg`));
    });
    job.unshown.clear();
    this.#shownAt = clock.now();
    this.preview = { count: job.count, map: nearestMade(job.made), sheets };
    this.#scope.update();
  }

  /** Write the job's index as it is now, after the job's earlier index writes (so the newest lands last). */
  #saveIndex(job: Job, keyframesOnly = false): Promise<void> {
    const dir = job.dir;
    if (!dir) return job.indexed;
    const made: number[] = [];
    job.made.forEach((m, i) => void (m && made.push(i)));
    const index: CacheIndex = {
      version: CACHE_VERSION,
      count: job.count,
      made,
      failed: [...job.failed].sort((a, b) => a - b),
      complete: made.length + job.failed.size >= job.count,
      usedAt: Date.now(),
      ...(keyframesOnly ? { keyframesOnly } : {}),
    };
    job.indexed = job.indexed.then(() => indexFile.write(dir, index).catch(() => {}));
    return job.indexed;
  }

  #finish(job: Job) {
    if (job.stage === "done") return;
    job.stage = "done";
    job.sheets = []; // the pixels live on in the sheet files
    void this.#saveIndex(job);
    this.#evict(job.key);
  }

  /** Keep `promise` in `#pending` until it settles. */
  #track(promise: Promise<unknown>) {
    const tracked = promise.catch(() => {});
    this.#pending.add(tracked);
    void tracked.then(() => this.#pending.delete(tracked));
  }

  /** `#track`, handing `promise` back (its outcome unchanged). */
  #tracked<T>(promise: Promise<T>): Promise<T> {
    this.#track(promise);
    return promise;
  }

  /** A job that no longer runs but has index writes on the way: tracked, and its folder kept until they land. */
  #hold(job: Job) {
    this.#ending.add(job);
    this.#track(job.indexed.finally(() => this.#ending.delete(job)));
  }

  /** `#pending` as it is now has settled, or `SETTLE_MS` (real time) passed. */
  #settled(): Promise<void> {
    if (this.#pending.size === 0) return Promise.resolve();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const limit = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, settle.ms);
      timer.unref?.();
    });
    return Promise.race([Promise.all([...this.#pending]).then(() => {}), limit]).finally(() => clearTimeout(timer));
  }

  /**
   * Clean the cache once ended sources' index writes landed (at most
   * `SETTLE_MS`). Kept: `key`, and, as it is when each folder is looked at,
   * the current job's folder (it may have begun meanwhile, without an index
   * yet) and those of ended jobs whose index writes are still on the way.
   */
  #evict(key: string | null) {
    const root = this.#root!;
    const keep = {
      has: (name: string) => name === key || this.#job?.key === name || [...this.#ending].some((job) => job.key === name),
    };
    this.#track(this.#settled().then(() => evict(root, keep)));
  }

  /**
   * The source changed or the plugin stops: end its ffmpeg and forget its
   * sprite (the files stay). Resolves once its index and sheet writes and the
   * pending clean-ups are done, or after `SETTLE_MS`; stop awaits it (0.2.12),
   * so as a rule nothing touches the data directory after the plugin stopped
   * (best effort: a disk slower than that still lands later).
   */
  #end(): Promise<void> {
    const job = this.#job;
    this.#job = null;
    this.#last = null;
    this.preview = null;
    this.#clearTimer();
    if (job) {
      job.abort.abort();
      this.#hold(job);
      if (job.dir) {
        for (let s = 0; s < sheetCount(job.count); s++) this.#track(rm(join(job.dir, `sheet-${s}.new.jpg`), { force: true }));
      }
    }
    return this.#settled();
  }
}

