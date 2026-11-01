import assert from "node:assert/strict";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";
import type { PlaybackStats } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { CACHE_VERSION, cacheKey, evict, readIndex } from "../preview-cache.ts";
import {
  CELL_BYTES,
  cellAt,
  cellCount,
  cellPlace,
  cellTime,
  coarseToFine,
  EXACT_BUDGET_S,
  exactCellCount,
  MAX_CELLS,
  nearestMade,
  PART_S,
  partEstimateMs,
  partition,
  placeCell,
  plan,
  sheetSize,
  SLOW_ALLOWANCE_MS,
  SLOW_FACTOR,
  stripCell,
  tooSlow,
} from "../sprite.ts";
import {
  BATCH,
  cellsArgs,
  clock,
  exactArgs,
  keyframesArgs,
  MAX_FAILURES,
  NEEDS_FFMPEG,
  parseKeyframes,
  PART_TIMEOUT_MS,
  PUBLISH_MS,
  SEEK_QUIET_MS,
  SHOW_MS,
  START_DELAY_MS,
  START_FALLBACK_MS,
  WORKERS,
} from "../thumbs.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  startPlugin(activate, { directory, host, locale: "en", ...(dataDirectory ? { dataDirectory } : {}) });
/** 4K60 in software: a 20 s file is 10 billion pixels, four times the exact budget, so these tests get keyframe sprites. */
const VIDEO = {
  codec: "hevc",
  width: 3840,
  height: 2160,
  fps: 60,
  decoder: "hevc",
  pixelFormat: "yuv420p10le",
  hwdec: "no",
} as unknown as PlaybackStats["video"];
/** 1080p30 in software: 20 s is 1.2 billion pixels (5 s of decoding): exact mode. */
const SMALL_VIDEO = {
  codec: "h264",
  width: 1920,
  height: 1080,
  fps: 30,
  decoder: "h264",
  pixelFormat: "yuv420p",
  hwdec: "no",
} as unknown as PlaybackStats["video"];
const facts = (video: PlaybackStats["video"], over: Partial<NonNullable<PlaybackStats["video"]>> = {}) => ({ ...video!, ...over });

// A ceiling, not a delay: generous because the whole suite runs in parallel.
async function until(predicate: () => boolean | Promise<boolean>, ms = 5000) {
  const deadline = Date.now() + ms;
  while (!(await predicate())) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

/** A real file to open (the sprite is keyed on its realpath, size and mtime), removed after the test. */
function mediaFile(name = "movie.mkv") {
  const dir = mkdtempSync(join(tmpdir(), "vestra-preview-"));
  const file = join(dir, name);
  writeFileSync(file, "not really a video");
  return { file, done: () => rmSync(dir, { recursive: true, force: true }) };
}

/** A local video playing: `framesShown` is the player's counter, 10 so far. */
function playing(host: TestHost, source: string, duration = 20) {
  host.stats = { ...host.stats, video: VIDEO, framesShown: 10 };
  host.openElsewhere(source);
  host.setSource({ duration, position: 0 });
  return host;
}

const cellRuns = (host: TestHost) => host.tools.calls.filter((c) => c.tool === "ffmpeg" && c.args.includes("-skip_frame"));
const ffmpegRuns = (host: TestHost) => host.tools.calls.filter((c) => c.tool === "ffmpeg");
/** Exact mode's runs: one ffmpeg per part (`-frames:v`). */
const partRuns = (host: TestHost) => host.tools.calls.filter((c) => c.tool === "ffmpeg" && c.args.includes("-frames:v") && c.args.includes("-vf"));
const partOf = (args: string[]) => ({ n: Number(args[args.indexOf("-frames:v") + 1]), ss: args.includes("-ss") ? args[args.indexOf("-ss") + 1]! : null });
/** The seek times of one run (one per cell it makes). */
const seeksOf = (args: string[]) => args.flatMap((arg, i) => (arg === "-ss" ? [args[i + 1]!] : []));
const cellsMade = (host: TestHost) => cellRuns(host).reduce((n, c) => n + seeksOf(c.args).length, 0);
/** Sheet writes: one ffmpeg per changed sheet, raw pixels to `sheet-N.new.jpg`. */
const sheetWrites = (host: TestHost) => host.tools.calls.filter((c) => c.tool === "ffmpeg" && c.args.at(-1)!.endsWith(".new.jpg")).length;

/**
 * ffmpeg as the plugin uses it: a run of cells writes `CELL_BYTES` per input
 * on stdout; a sheet "JPEG" is a copy of its raw pixels, read back as they
 * are. `gate` holds each run of cells until released.
 */
function fakeFfmpeg(host: TestHost, gate?: () => Promise<void>) {
  host.tools.handle("ffmpeg", async (args) => {
    const input = args[args.indexOf("-i") + 1]!;
    if (args.includes("-skip_frame")) {
      await gate?.();
      return { stdout: new Uint8Array(CELL_BYTES * seeksOf(args).length).fill(1) };
    }
    if (args.at(-1) === "-") return { stdout: new Uint8Array(readFileSync(input)) };
    copyFileSync(input, args.at(-1)!);
    return {};
  });
}

/**
 * ffmpeg and ffprobe as exact mode uses them: ffprobe lists packets with
 * keyframes at `keyframes` seconds; a part run writes its `-frames:v` cells
 * (`short` fewer for the part seeking to `short.ss`), each byte the cell's
 * index in the part; `fail` makes every part run fail.
 */
function fakeExact(host: TestHost, keyframes: number[], options: { short?: { ss: string; by: number }; fail?: boolean } = {}) {
  host.tools.handle("ffprobe", () => ({
    stdout: [...keyframes.flatMap((k) => [`${k.toFixed(6)},K__`, `${(k + 0.033).toFixed(6)},___`]), "N/A,K__", ""].join("\n"),
  }));
  host.tools.handle("ffmpeg", async (args) => {
    const input = args[args.indexOf("-i") + 1]!;
    if (args.includes("-skip_frame")) return { stdout: new Uint8Array(CELL_BYTES * seeksOf(args).length).fill(1) };
    if (args.includes("-frames:v") && args.includes("-vf")) {
      if (options.fail) throw new HostError("failed", "Invalid data found when processing input");
      const { n, ss } = partOf(args);
      const cells = options.short && options.short.ss === ss ? n - options.short.by : n;
      const out = new Uint8Array(CELL_BYTES * cells);
      for (let slot = 0; slot < cells; slot++) out.fill(slot + 1, slot * CELL_BYTES, (slot + 1) * CELL_BYTES);
      return { stdout: out };
    }
    if (args.at(-1) === "-") return { stdout: new Uint8Array(readFileSync(input)) };
    copyFileSync(input, args.at(-1)!);
    return {};
  });
}

/** The plugin's timers on a clock the test moves; `advance` runs what falls due on the way. */
function fakeClock(host: TestHost) {
  const saved = { ...clock };
  let now = 1_000_000;
  let next = 0;
  const timers = new Map<number, { due: number; run: () => void }>();
  clock.now = () => now;
  clock.setTimeout = (run, ms) => {
    timers.set(++next, { due: now + ms, run });
    return next;
  };
  clock.clearTimeout = (timer) => void timers.delete(timer as number);
  return {
    async advance(ms: number) {
      const end = now + ms;
      for (let guard = 0; guard < 1000; guard++) {
        const [id, timer] = [...timers].sort((a, b) => a[1].due - b[1].due)[0] ?? [];
        if (id === undefined || timer!.due > end) break;
        timers.delete(id);
        now = Math.max(now, timer!.due);
        timer!.run();
        await host.settle();
      }
      now = end;
      await host.settle();
    },
    /** Move the clock without running timers (time passing inside one ffmpeg run). */
    tick: (ms: number) => void (now += ms),
    restore: () => Object.assign(clock, saved),
  };
}

/** Open, let the first frame show and the start delay pass: the sprite is being made. */
async function generating(host: TestHost, time: ReturnType<typeof fakeClock>) {
  await time.advance(300); // ready: the frame counter's baseline
  host.stats = { ...host.stats, framesShown: host.stats.framesShown + 1 };
  await time.advance(300); // the first frame is seen
  await time.advance(START_DELAY_MS);
}

// ------------------------------------------------------------------ plan

test("I07 sprite plan: up to 200 cells 2 s apart, seeking the middle of each span", () => {
  assert.equal(cellCount(null), 0);
  assert.equal(cellCount(0), 0);
  assert.equal(cellCount(1), 1);
  assert.equal(cellCount(20), 10);
  assert.equal(cellCount(399), 199);
  assert.equal(cellCount(7200), MAX_CELLS);
  assert.equal(cellTime(0, 10, 20), 1);
  assert.equal(cellTime(9, 10, 20), 19);
  assert.equal(cellTime(0, 1, 0.75), 0.25); // never past the last half second
  assert.equal(cellTime(199, 200, 7200), 7182);
  // The bar's lookup (bar.slint does the same).
  assert.equal(cellAt(0, 10), 0);
  assert.equal(cellAt(0.5, 10), 5);
  assert.equal(cellAt(1, 10), 9);
});

test("I07 sprite plan: cells are made coarse to fine, each exactly once", () => {
  assert.deepEqual(coarseToFine(1), [0]);
  assert.deepEqual(coarseToFine(10), [0, 8, 4, 2, 6, 1, 3, 5, 7, 9]);
  const order = coarseToFine(200);
  assert.deepEqual(order.slice(0, 4), [0, 128, 64, 192]);
  assert.deepEqual(
    [...order].sort((a, b) => a - b),
    Array.from({ length: 200 }, (_, i) => i),
  );
  // After the first 25 cells the largest gap on the line is 8 cells.
  const first = order.slice(0, 25).sort((a, b) => a - b);
  assert.ok(first.every((cell, i) => i === 0 || cell - first[i - 1]! <= 8));
});

test("I07 sprite plan: 10×10 sheets of 160×90, and the nearest made cell for the rest", () => {
  assert.deepEqual(cellPlace(0), { sheet: 0, x: 0, y: 0 });
  assert.deepEqual(cellPlace(23), { sheet: 0, x: 480, y: 180 });
  assert.deepEqual(cellPlace(123), { sheet: 1, x: 480, y: 180 });
  assert.deepEqual(sheetSize(0, 10), { width: 1600, height: 90 });
  assert.deepEqual(sheetSize(0, 200), { width: 1600, height: 900 });
  assert.deepEqual(sheetSize(1, 150), { width: 1600, height: 450 });
  const sheet = new Uint8Array(1600 * 900 * 3);
  placeCell(sheet, 1600, 23, new Uint8Array(CELL_BYTES).fill(7));
  const at = (x: number, y: number) => sheet[(y * 1600 + x) * 3];
  assert.deepEqual([at(480, 180), at(639, 269), at(479, 180), at(640, 180), at(480, 270)], [7, 7, 0, 0, 0]);
  assert.deepEqual(nearestMade([0, 0, 0]), [-1, -1, -1]);
  assert.deepEqual(nearestMade([0, 1, 0, 0, 1]), [1, 1, 1, 4, 4]);
  assert.deepEqual(nearestMade([1, 0, 1]), [0, 0, 2]); // a tie shows the earlier one
  // A run's strip: cells side by side, 160 px each.
  const strip = new Uint8Array(CELL_BYTES * 3);
  for (let line = 0; line < 90; line++) strip.fill(line, (line * 480 + 160) * 3, (line * 480 + 320) * 3);
  const middle = stripCell(strip, 1, 3);
  assert.deepEqual([middle[0], middle[159 * 3], middle[160 * 3 * 89]], [0, 0, 89]);
  assert.ok(stripCell(strip, 0, 3).every((v) => v === 0));
});

test("I07 a run makes up to five keyframes: per input -noaccurate_seek -ss -copyts before -i, one thread, a raw strip", () => {
  const args = cellsArgs("/a b/c.mkv", [12.5, 40, 1]);
  const inputs = args.flatMap((arg, i) => (arg === "-i" ? [i] : []));
  assert.equal(inputs.length, 3);
  for (const [n, i] of inputs.entries()) {
    // Each input: one decoder thread, keyframes only, the keyframe at or before the time.
    assert.deepEqual(args.slice(i - 8, i + 2), ["-threads", "1", "-skip_frame", "nokey", "-noaccurate_seek", "-ss", ["12.500", "40.000", "1.000"][n], "-copyts", "-i", "/a b/c.mkv"]);
  }
  assert.equal(args[args.indexOf("-filter_threads") + 1], "1");
  const graph = args[args.indexOf("-filter_complex") + 1]!;
  assert.match(graph, /^\[0:v:0\]trim=end_frame=1,setpts=0,scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:/);
  assert.match(graph, /\[c0\]\[c1\]\[c2\]hstack=inputs=3$/);
  assert.deepEqual(args.slice(-5), ["-frames:v", "1", "-f", "rawvideo", "-"]);
  assert.match(cellsArgs("/x.mkv", [3])[cellsArgs("/x.mkv", [3]).indexOf("-filter_complex") + 1]!, /\[c0\]null$/);
  assert.equal(BATCH, 5);
});

// ------------------------------------------------------------------ exact mode (0.2.2)

test("0.2.2 plan: exact within the decoding budget, keyframes beyond it (4K always), no facts → keyframes; a slow part ends the pass", () => {
  assert.equal(EXACT_BUDGET_S, 15);
  // 1080p30 software: 62 million pixels a second, 4 ns each → 60 s fits, 61 s does not.
  assert.deepEqual(plan(20, facts(SMALL_VIDEO)), { mode: "exact", count: 40 });
  assert.deepEqual(plan(60, facts(SMALL_VIDEO)), { mode: "exact", count: 120 });
  assert.deepEqual(plan(61, facts(SMALL_VIDEO)), { mode: "keyframe", count: 30 });
  // 1080p60: 30 s.
  assert.equal(plan(30, facts(SMALL_VIDEO, { fps: 60 })).mode, "exact");
  assert.equal(plan(31, facts(SMALL_VIDEO, { fps: 60 })).mode, "keyframe");
  // 4K60: 7.5 s, so any real file is keyframes; the player's hardware decoding changes nothing (its engine is not borrowed).
  assert.deepEqual(plan(20, facts(VIDEO)), { mode: "keyframe", count: 10 });
  assert.equal(plan(20, facts(VIDEO, { hwdec: "d3d11va-copy" })).mode, "keyframe");
  assert.equal(plan(7, facts(VIDEO, { hwdec: "d3d11va-copy" })).mode, "exact");
  // Without the facts (an older player, no video yet): the keyframe sprite.
  assert.deepEqual(plan(20, null), { mode: "keyframe", count: 10 });
  assert.equal(plan(20, facts(SMALL_VIDEO, { width: null })).mode, "keyframe");
  assert.equal(plan(20, facts(SMALL_VIDEO, { fps: 0 })).mode, "keyframe");
  // Cells: every half second, never finer than a frame, at most 200 (then duration/200 apart).
  assert.equal(exactCellCount(20, 30), 40);
  assert.equal(exactCellCount(100, 30), 200);
  assert.equal(exactCellCount(150, 30), 200);
  assert.equal(exactCellCount(0.3, 30), 1);
  assert.equal(exactCellCount(20, 1), 20, "a 1 fps slideshow: one cell per frame");
  assert.equal(exactCellCount(20, 0.4), 8);
  assert.deepEqual(plan(20, facts(SMALL_VIDEO, { fps: 1 })), { mode: "exact", count: 20 });
  // A part's estimate: its share of the video at one thread; twice that plus a second is too slow.
  assert.equal(SLOW_FACTOR, 2);
  assert.equal(SLOW_ALLOWANCE_MS, 1000);
  const estimate = partEstimateMs({ first: 0, n: 20, start: 0 }, 40, 20, facts(SMALL_VIDEO)); // 10 s of 1080p30
  assert.equal(Math.round(estimate), 2488);
  assert.equal(tooSlow(5000, estimate), false);
  assert.equal(tooSlow(6000, estimate), true);
  assert.equal(tooSlow(900, 0), false, "a tiny part is never judged by its estimate alone");
});

test("0.2.2 plan: parts begin at keyframes at least 8 s apart, coarse to fine; one part without keyframes", () => {
  assert.equal(PART_S, 8);
  // 31 s, 62 cells half a second apart, a keyframe every 10 s: the cell at 10.25 s is the first of the second part.
  assert.deepEqual(partition(62, 31, [0, 10, 20, 30]), [
    { first: 0, n: 20, start: 0 },
    { first: 40, n: 20, start: 20 },
    { first: 20, n: 20, start: 10 },
    { first: 60, n: 2, start: 30 },
  ]);
  // A keyframe every 2 s: grouped to 8 s.
  assert.deepEqual(
    partition(62, 31, Array.from({ length: 16 }, (_, i) => i * 2)).map((p) => p.start),
    [0, 16, 8, 24],
  );
  // A late keyframe: a short last part; a keyframe closer than 8 s to the previous bound opens no part.
  assert.deepEqual(partition(62, 31, [0, 5, 30.5]), [
    { first: 0, n: 61, start: 0 },
    { first: 61, n: 1, start: 30.5 },
  ]);
  assert.deepEqual(partition(62, 31, []), [{ first: 0, n: 62, start: 0 }]);
  // Unsorted input, a keyframe past the end (the container's duration): no empty part.
  assert.deepEqual(partition(40, 20, [10, 0, 20]), [
    { first: 0, n: 20, start: 0 },
    { first: 20, n: 20, start: 10 },
  ]);
});

test("0.2.2 a part run: software, one thread, the frame on screen at each cell time (fps grid, gaps repeat), -frames:v", () => {
  const sw = exactArgs("/a b/c.mkv", { first: 0, n: 20, start: 0 }, 40, 20);
  assert.deepEqual(sw.slice(0, 7), ["-nostdin", "-v", "error", "-threads", "1", "-i", "/a b/c.mkv"]);
  assert.ok(!sw.includes("-ss") && !sw.includes("-hwaccel"), "no seek for the first part; never the player's video engine");
  assert.equal(sw[sw.indexOf("-filter_threads") + 1], "1");
  // The grid: tick k at cell k's time (0.25 + k × 0.5 s); round=up keeps the last frame at or before each tick and
  // repeats it over a span without frames. Checked with ffmpeg n9 on 30 fps (0.233, 0.733, …), a 1 fps slideshow
  // (second k at cell k) and a clip without frames between 8 and 12 s (its 7.967 frame in the eight cells there).
  const graph = sw[sw.indexOf("-vf") + 1]!;
  assert.match(graph, /^setpts=PTS-0\.250000\/TB,fps=fps=1\/0\.500000:start_time=0:round=up,scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:/);
  assert.ok(!graph.includes("select"));
  assert.deepEqual(sw.slice(-7), ["-fps_mode", "passthrough", "-frames:v", "20", "-f", "rawvideo", "-"]);
  // The second part: from its keyframe (kept: a frame is not before its own time), cell times relative to the seek point.
  const second = exactArgs("/x.mp4", { first: 20, n: 20, start: 10 }, 40, 20);
  assert.deepEqual(second.slice(5, 9), ["-ss", "10.000", "-i", "/x.mp4"]);
  assert.match(second[second.indexOf("-vf") + 1]!, /^setpts=PTS-0\.250000\/TB,fps=fps=1\/0\.500000:/);
  // A slideshow: one cell per second, the grid at 0.5, 1.5, …
  assert.match(exactArgs("/s.mp4", { first: 0, n: 20, start: 0 }, 20, 20)[exactArgs("/s.mp4", { first: 0, n: 20, start: 0 }, 20, 20).indexOf("-vf") + 1]!, /^setpts=PTS-0\.500000\/TB,fps=fps=1\/1\.000000:/);
  assert.equal(PART_TIMEOUT_MS, 60_000);
  // ffprobe's keyframe list.
  assert.deepEqual(keyframesArgs("/x.mp4").slice(-5), ["-show_entries", "packet=pts_time,flags", "-of", "csv=p=0", "/x.mp4"]);
  assert.deepEqual(parseKeyframes("2.000000,K__\n0.000000,K__\n0.033333,___\nN/A,K__\n,\n"), [0, 2]);
});

test("0.2.2 exact mode: ffprobe first, then the parts two at a time; cells are the frames in order; its own cache folder", async () => {
  const media = mediaFile();
  const data = mkdtempSync(join(tmpdir(), "vestra-preview-exact-"));
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file); // 20 s of 1080p30: 40 exact cells
    host.stats = { ...host.stats, video: SMALL_VIDEO };
    fakeExact(host, [0, 10], { short: { ss: "10.000", by: 1 } });
    const { stop } = await start(host, data);
    const bar = host.mounted();
    await generating(host, time);
    await until(() => partRuns(host).length === 2);
    assert.deepEqual(host.tools.calls.slice(0, 3).map((c) => c.tool), ["ffprobe", "ffmpeg", "ffmpeg"]);
    assert.deepEqual(partRuns(host).map((c) => partOf(c.args)), [
      { n: 20, ss: null },
      { n: 20, ss: "10.000" },
    ]);
    assert.equal(cellRuns(host).length, 0, "no keyframe runs");
    bar.emit("pointing", true);
    await until(async () => {
      await time.advance(500);
      return bar.props["preview-count"] === 40 && (bar.props["preview-map"] as number[])[0] === 0;
    });
    const map = bar.props["preview-map"] as number[];
    assert.equal(map.length, 40);
    assert.deepEqual(map.slice(0, 20), Array.from({ length: 20 }, (_, i) => i));
    assert.equal(map[39], 38, "the stream ended before the last cell: its neighbour shows");
    // The sheet holds the frames in cell order: cell 25 is the second part's sixth frame.
    const sheet = host.imagePath(bar.props["preview-sheet-a"]);
    const pixels = readFileSync(sheet!);
    assert.equal(pixels[(0 * 1600 + 160 * 5) * 3], 6, "cell 5 of the first part");
    assert.equal(pixels[(180 * 1600 + 160 * 5) * 3], 6, "cell 25: the second part's sixth frame");
    const real = (await import("node:fs")).realpathSync(media.file);
    const { size, mtimeMs } = (await import("node:fs")).statSync(real);
    assert.deepEqual(readdirSync(join(data, "thumbs")), [cacheKey({ path: real, size, mtimeMs }, 40, "exact")]);
    assert.notEqual(cacheKey({ path: real, size, mtimeMs }, 40, "exact"), cacheKey({ path: real, size, mtimeMs }, 40));
    await stop();
  } finally {
    time.restore();
    media.done();
    rmSync(data, { recursive: true, force: true });
  }
});

test("0.2.2 exact mode: a part ended for a drag is made again whole; a pass that fails falls back to keyframes, remembered", async () => {
  const media = mediaFile();
  const data = mkdtempSync(join(tmpdir(), "vestra-preview-fallback-"));
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 31);
    host.stats = { ...host.stats, video: SMALL_VIDEO };
    // Part runs wait to be let go (`release`) or ended (`kill`: what the plugin's abort does to the real ffmpeg).
    const pending: { release: () => void; kill: () => void }[] = [];
    fakeExact(host, [0, 10, 20, 30]);
    const parts = host.tools.handlers.get("ffmpeg")!;
    host.tools.handle("ffmpeg", async (args) => {
      if (args.includes("-frames:v"))
        await new Promise<void>((resolve, reject) => pending.push({ release: resolve, kill: () => reject(new HostError("aborted", "aborted")) }));
      return parts(args);
    });
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await until(() => partRuns(host).length === WORKERS);
    // Coarse to fine over the parts: the first and the third (at 20 s) run first.
    assert.deepEqual(partRuns(host).map((c) => partOf(c.args).ss), [null, "20.000"]);
    bar.emit("dragging", true);
    await host.settle();
    for (const run of pending.splice(0)) run.kill();
    await time.advance(5000);
    assert.equal(partRuns(host).length, WORKERS, "nothing new while the pointer is down");
    bar.emit("dragging", false);
    await host.settle();
    await until(async () => {
      await time.advance(SEEK_QUIET_MS / 4);
      return partRuns(host).length === 2 * WORKERS;
    });
    // The ended parts come back whole (both at once, in either order).
    assert.deepEqual(
      partRuns(host)
        .slice(2)
        .map((c) => partOf(c.args))
        .sort((a, b) => (a.ss ?? "").localeCompare(b.ss ?? "")),
      [
        { n: 20, ss: null },
        { n: 20, ss: "20.000" },
      ],
    );
    // Let the rest run: four parts in all, each once more at most.
    await until(async () => {
      for (const run of pending.splice(0)) run.release();
      await time.advance(100);
      return partRuns(host).length === 6;
    });
    assert.deepEqual(partRuns(host).slice(4).map((c) => partOf(c.args)), [
      { n: 20, ss: "10.000" },
      { n: 2, ss: "30.000" },
    ]);
    assert.equal(cellRuns(host).length, 0);
    await stop();

    // A pass that cannot run: the keyframe sprite instead (its own folder, its own cell count).
    const again = new TestHost();
    playing(again, media.file);
    again.stats = { ...again.stats, video: SMALL_VIDEO };
    fakeExact(again, [0, 10], { fail: true });
    const second = await start(again, data);
    const bar2 = again.mounted();
    await generating(again, time);
    await until(async () => {
      await time.advance(50);
      return cellRuns(again).length > 0;
    });
    assert.equal(partRuns(again).length, 2, "both first parts failed");
    bar2.emit("pointing", true);
    await until(async () => {
      await time.advance(500);
      return bar2.props["preview-count"] === 10;
    });
    await until(async () => {
      await time.advance(500);
      return (bar2.props["preview-map"] as number[]).join() === "0,1,2,3,4,5,6,7,8,9";
    });
    assert.equal(cellsMade(again), 10);
    const real = (await import("node:fs")).realpathSync(media.file);
    const { size, mtimeMs } = (await import("node:fs")).statSync(real);
    // The keyframe sprite's last index write (complete) is not awaited by anyone: wait for it before stopping.
    const keyframeDir = join(data, "thumbs", cacheKey({ path: real, size, mtimeMs }, 10));
    await until(async () => (await readIndex(keyframeDir))?.complete === true);
    await second.stop();
    // The exact folder remembers the fallback; the next open of the file goes straight to keyframes.
    const exactDir = join(data, "thumbs", cacheKey({ path: real, size, mtimeMs }, 40, "exact"));
    // The fallback's index write is not awaited by anyone: wait for it to land.
    await until(async () => (await readIndex(exactDir))?.keyframesOnly === true);
    const third = new TestHost();
    playing(third, media.file);
    third.stats = { ...third.stats, video: SMALL_VIDEO };
    fakeExact(third, [0, 10]); // the pass would work now, but it is not tried
    const last = await start(third, data);
    await generating(third, time);
    third.mounted().emit("pointing", true);
    await until(async () => {
      await time.advance(500);
      return (third.mounted().props["preview-map"] as number[]).join() === "0,1,2,3,4,5,6,7,8,9";
    });
    assert.deepEqual(third.tools.calls.filter((c) => c.tool === "ffprobe").length, 0, "no keyframe list, no exact run");
    assert.equal(partRuns(third).length, 0);
    assert.equal(ffmpegRuns(third).length, 0, "the keyframe sprite of the second session is reused as it is");
    await last.stop();
  } finally {
    time.restore();
    media.done();
    rmSync(data, { recursive: true, force: true });
  }
});

test("0.2.2 exact mode: a part slower than twice its estimate ends the pass; the file gets keyframe thumbnails", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file); // 20 s of 1080p30: two parts of 10 s, 2.5 s estimated each
    host.stats = { ...host.stats, video: SMALL_VIDEO };
    fakeExact(host, [0, 10]);
    const parts = host.tools.handlers.get("ffmpeg")!;
    host.tools.handle("ffmpeg", async (args) => {
      if (args.includes("-frames:v") && !args.includes("-ss")) time.tick(7000); // the first part took 7 s: over 2 × 2.5 s + 1 s
      return parts(args);
    });
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await until(async () => {
      await time.advance(50);
      return cellRuns(host).length > 0;
    });
    assert.equal(partRuns(host).length, 2, "the two parts had started; nothing exact after the slow one");
    bar.emit("pointing", true);
    await until(async () => {
      await time.advance(500);
      return bar.props["preview-count"] === 10 && (bar.props["preview-map"] as number[]).join() === "0,1,2,3,4,5,6,7,8,9";
    });
    assert.equal(partRuns(host).length, 2);
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

// ------------------------------------------------------------------ cache

test("I07 cache: keyed on the real path, size, modification time, cell count and (0.2.2) exact or not", () => {
  const file = { path: "/m/a.mkv", size: 100, mtimeMs: 5 };
  const key = cacheKey(file, 200);
  assert.match(key, /^[0-9a-f]{20}$/);
  assert.equal(cacheKey({ ...file }, 200), key);
  assert.equal(cacheKey(file, 200, "keyframe"), key, "0.2.0 keyframe sprites keep their folders");
  for (const other of [
    cacheKey({ ...file, path: "/m/b.mkv" }, 200),
    cacheKey({ ...file, size: 101 }, 200),
    cacheKey({ ...file, mtimeMs: 6 }, 200),
    cacheKey(file, 199),
    cacheKey(file, 200, "exact"),
  ])
    assert.notEqual(other, key);
});

test("I07 cache: over the limit the least recently used go first; old or broken folders always", async () => {
  const root = mkdtempSync(join(tmpdir(), "vestra-preview-cache-"));
  try {
    const entry = (name: string, usedAt: number, version = CACHE_VERSION) => {
      mkdirSync(join(root, name));
      writeFileSync(join(root, name, "sheet-0.jpg"), Buffer.alloc(1000));
      const index = { version, count: 10, made: [0], failed: [], complete: false, usedAt };
      writeFileSync(join(root, name, "index.json"), JSON.stringify(index));
    };
    entry("current", 1); // the oldest, but in use
    entry("old", 2);
    entry("recent", 3);
    entry("v1", 4, 1);
    mkdirSync(join(root, "0123456789abcdef")); // 0.1.x: one JPEG per slot, no index
    writeFileSync(join(root, "0123456789abcdef", "12.jpg"), "x");
    assert.equal(await readIndex(join(root, "v1")), null);
    const removed = await evict(root, new Set(["current"]), 2500);
    assert.deepEqual(removed.sort(), ["0123456789abcdef", "old", "v1"]);
    assert.deepEqual(readdirSync(root).sort(), ["current", "recent"]);
    // Under the limit nothing more goes.
    assert.deepEqual(await evict(root, new Set(), 1_000_000), []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ when it runs

test("I07 nothing is made before the source's first frame; then after a short delay, two at a time", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 400); // 200 cells, 2 s apart
    host.setSource({ phase: "loading" });
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    await time.advance(5000);
    assert.equal(ffmpegRuns(host).length, 0, "not while loading");
    host.setSource({ phase: "ready" });
    await time.advance(START_FALLBACK_MS - 500);
    assert.equal(ffmpegRuns(host).length, 0, "ready, but no frame of this source drawn yet");
    host.stats = { ...host.stats, framesShown: 11 }; // the first frame
    await time.advance(300); // a poll sees it
    await time.advance(START_DELAY_MS - 350);
    assert.equal(ffmpegRuns(host).length, 0, "still in the start delay");
    await time.advance(400);
    await until(() => cellRuns(host).length === WORKERS);
    await time.advance(5000);
    assert.equal(cellRuns(host).length, WORKERS, "at most two ffmpeg at a time");
    // Coarse to fine, five per run: cells 0, 128, 64, 192, 32, then 96, 160, 16, 48, 80.
    assert.deepEqual(
      cellRuns(host).map((c) => seeksOf(c.args)),
      [
        ["1.000", "257.000", "129.000", "385.000", "65.000"],
        ["193.000", "321.000", "33.000", "97.000", "161.000"],
      ],
    );
    releases.shift()!();
    await until(async () => {
      await time.advance(10); // the freed slot's next look is a timer
      return cellRuns(host).length === WORKERS + 1;
    });
    for (const release of releases.splice(0)) release();
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 opened paused (no new frame): generation starts a while after ready anyway", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file);
    host.setSource({ paused: true });
    fakeFfmpeg(host);
    const { stop } = await start(host);
    await time.advance(START_FALLBACK_MS - 300);
    assert.equal(ffmpegRuns(host).length, 0);
    await time.advance(START_DELAY_MS + 600);
    await until(() => cellRuns(host).length > 0);
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 dragging the seek line pauses generation; it goes on a moment after the drag (and after a seek)", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 400); // 200 cells: plenty left after the first runs
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await until(() => cellRuns(host).length === WORKERS);
    bar.emit("dragging", true);
    await host.settle();
    for (const release of releases.splice(0)) release();
    await time.advance(5000);
    assert.equal(cellRuns(host).length, WORKERS, "nothing new while the pointer is down");
    bar.emit("dragging", false);
    await host.settle();
    await time.advance(SEEK_QUIET_MS - 100);
    assert.equal(cellRuns(host).length, WORKERS, "the drag's last seek gets a moment");
    await time.advance(200);
    await until(() => cellRuns(host).length > WORKERS);
    const before = cellRuns(host).length;
    // A seek from elsewhere (a key, another plugin): the position jumps.
    host.setSource({ position: 15 });
    await host.settle();
    for (const release of releases.splice(0)) release();
    await time.advance(SEEK_QUIET_MS - 100);
    assert.equal(cellRuns(host).length, before);
    await time.advance(200);
    await until(() => cellRuns(host).length > before);
    for (const release of releases.splice(0)) release();
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

// ------------------------------------------------------------------ the bar

test("I07 the bar gets the sheets and the cell map; the next session reuses them without ffmpeg", async () => {
  const media = mediaFile();
  const data = mkdtempSync(join(tmpdir(), "vestra-preview-data-"));
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file);
    fakeFfmpeg(host);
    const first = await start(host, data);
    const bar = host.mounted();
    assert.equal(bar.props["hover-thumbnail"], true, "on by default for a local video");
    assert.equal(bar.props["preview-count"], 0);
    await generating(host, time);
    await until(async () => {
      await time.advance(500);
      return existsSync(join(data, "thumbs")) && readdirSync(join(data, "thumbs")).length === 1 && cellsMade(host) === 10;
    });
    await time.advance(3000); // the last sheet write
    // Nobody points at the seek line: the player decodes no sheet.
    assert.equal(bar.props["preview-count"], 0);
    assert.equal(bar.props["preview-sheet-a"], undefined);
    bar.emit("pointing", true);
    await until(() => (bar.props["preview-map"] as number[]).join() === "0,1,2,3,4,5,6,7,8,9");
    await until(() => host.imagePath(bar.props["preview-sheet-a"]) !== null);
    assert.equal(bar.props["preview-count"], 10);
    assert.equal(cellsMade(host), 10);
    assert.equal(cellRuns(host).length, 2, "five cells per ffmpeg run");
    const sheet = host.imagePath(bar.props["preview-sheet-a"]);
    assert.equal(basename(sheet ?? ""), "sheet-0.jpg");
    assert.ok(sheet!.startsWith(join(data, "thumbs")));
    assert.equal(readFileSync(sheet!).length, 1600 * 90 * 3);
    await until(() => readdirSync(join(data, "thumbs")).length === 1);
    const dir = join(data, "thumbs", readdirSync(join(data, "thumbs"))[0]!);
    await until(() => existsSync(join(dir, "index.json")) && JSON.parse(readFileSync(join(dir, "index.json"), "utf8")).complete);
    assert.deepEqual(readdirSync(dir).sort(), ["index.json", "sheet-0.jpg"]);
    await first.stop();

    const again = new TestHost();
    playing(again, media.file);
    fakeFfmpeg(again);
    const second = await start(again, data);
    await generating(again, time);
    again.mounted().emit("pointing", true);
    await until(() => (again.mounted().props["preview-map"] as number[]).join() === "0,1,2,3,4,5,6,7,8,9");
    assert.equal(ffmpegRuns(again).length, 0, "a finished sprite is shown as it is");
    await second.stop();
  } finally {
    time.restore();
    media.done();
    rmSync(data, { recursive: true, force: true });
  }
});

test("0.2.3 while the pointer is on the bar a changed sheet goes out at most every SHOW_MS; at once when nobody points", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 400); // 200 cells on two sheets, five per run
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await until(() => releases.length === WORKERS);
    const sheet = () => JSON.stringify(bar.props["preview-sheet-a"] ?? null);
    /** Cells made, as the bar sees them (the map names each made cell at least once). */
    const made = () => new Set((bar.props["preview-map"] as number[] | undefined) ?? []).size;
    // Let one run land: its slot is free for the sheet write, then the next run starts.
    const land = async () => {
      const wrote = sheetWrites(host);
      releases.shift()!();
      await until(async () => {
        await time.advance(0);
        return releases.length === WORKERS;
      });
      // The write took the freed slot before the next run did, and is done (its hand-over decided).
      assert.ok(sheetWrites(host) > wrote, "the landed cells are written before the next run starts");
    };
    bar.emit("pointing", true);
    await host.settle();
    // The first cells are written and handed over at once.
    await land();
    assert.equal(made(), 5);
    const first = sheet();
    assert.notEqual(first, "null");
    // A second later more cells land and the sheet is written (PUBLISH_MS), but the
    // bar keeps the one it has until SHOW_MS after the last hand-over.
    await time.advance(PUBLISH_MS);
    await land();
    assert.equal(made(), 5, "held back while pointing");
    assert.equal(sheet(), first);
    await time.advance(SHOW_MS - PUBLISH_MS - 100);
    assert.equal(made(), 5, "still held back");
    await time.advance(100);
    assert.equal(made(), 10, "handed over when due");
    assert.notEqual(sheet(), first);
    const second = sheet();
    // Nobody points: the bar sends no sheets, so a written sheet is adopted at
    // once, and the pointer's return shows the newest without waiting.
    bar.emit("pointing", false);
    await host.settle();
    assert.equal(bar.props["preview-count"], 0);
    await time.advance(PUBLISH_MS);
    await land();
    bar.emit("pointing", true);
    await host.settle();
    assert.equal(made(), 15);
    assert.notEqual(sheet(), second);
    const third = sheet();
    // Held back again; the pointer leaving ends the wait at once.
    await time.advance(PUBLISH_MS);
    await land();
    assert.equal(made(), 15, "held back");
    assert.equal(sheet(), third);
    bar.emit("pointing", false);
    await host.settle();
    bar.emit("pointing", true);
    await host.settle();
    assert.equal(made(), 20);
    assert.notEqual(sheet(), third);
    for (const release of releases.splice(0)) release();
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("0.2.3 a sheet without new cells keeps its image reference across a hand-over: only the changed sheet is decoded again", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 400); // 200 cells on two sheets, five per run
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await until(() => releases.length === WORKERS);
    const ref = (sheet: "a" | "b") => JSON.stringify(bar.props[`preview-sheet-${sheet}`] ?? null);
    // Runs land in queue order; after each the slot is free for the write (PUBLISH_MS has passed) and the next run starts.
    const land = async () => {
      const wrote = sheetWrites(host);
      releases.shift()!();
      await until(async () => {
        await time.advance(0);
        return releases.length === WORKERS;
      });
      // The write took the freed slot before the next run did, and is done (its hand-over decided).
      assert.ok(sheetWrites(host) > wrote, "the landed cells are written before the next run starts");
    };
    // Coarse to fine, five per run: runs 0–3 touch both sheets, run 4 only sheet B
    // (cells 120–184), run 5 only sheet A (cells 4–36).
    for (let run = 0; run < 4; run++) {
      await time.advance(PUBLISH_MS);
      await land();
    }
    bar.emit("pointing", true);
    await host.settle();
    const [a0, b0] = [ref("a"), ref("b")];
    assert.notEqual(a0, "null");
    assert.notEqual(b0, "null");
    await time.advance(SHOW_MS); // not held back by the throttle
    await land(); // run 4: sheet B only
    assert.equal(ref("a"), a0, "sheet A unchanged: the same reference, nothing to decode again");
    assert.notEqual(ref("b"), b0, "sheet B changed: a new reference");
    const b1 = ref("b");
    await time.advance(SHOW_MS);
    await land(); // run 5: sheet A only
    assert.notEqual(ref("a"), a0);
    assert.equal(ref("b"), b1, "sheet B unchanged this time");
    for (const release of releases.splice(0)) release();
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 a new source ends the old one's work at once; its own sprite starts from its first frame", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file, 400);
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    await generating(host, time);
    await until(() => cellRuns(host).length === WORKERS);
    const other = mediaFile("other.mkv");
    try {
      host.openElsewhere(other.file, "loading");
      await host.settle();
      for (const release of releases.splice(0)) release();
      await time.advance(5000);
      assert.equal(cellRuns(host).length, WORKERS, "nothing for the old source, nothing before the new one shows");
      assert.equal(host.mounted().props["preview-count"], 0);
      host.setSource({ phase: "ready", duration: 20 });
      await generating(host, time);
      await until(() => cellRuns(host).length > WORKERS);
      assert.equal(cellRuns(host).at(-1)!.args[cellRuns(host).at(-1)!.args.indexOf("-i") + 1], other.file);
      for (const release of releases.splice(0)) release();
      await stop();
    } finally {
      other.done();
    }
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 only local files with video: network sources and audio show the time only", async () => {
  const media = mediaFile("song.mp3");
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, "https://example.com/a.mp4");
    fakeFfmpeg(host);
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    assert.equal(bar.props["hover-thumbnail"], false);
    host.stats = { ...host.stats, video: null };
    host.openElsewhere(media.file);
    host.setSource({ duration: 100 });
    await generating(host, time);
    assert.equal(bar.props["hover-thumbnail"], false);
    assert.equal(ffmpegRuns(host).length, 0);
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 failed cells show their neighbours; after five in a row the source is given up", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file);
    host.tools.handle("ffmpeg", () => {
      throw new HostError("failed", "Invalid data found when processing input");
    });
    const { stop } = await start(host);
    const bar = host.mounted();
    await generating(host, time);
    await time.advance(1000);
    await until(() => bar.props["hover-thumbnail"] === false);
    // The failed runs of five are tried cell by cell; the other worker's cell
    // may already be running when the fifth single cell fails.
    const runs = cellRuns(host).length;
    const singles = cellRuns(host).filter((c) => seeksOf(c.args).length === 1).length;
    assert.ok(singles >= MAX_FAILURES && singles < MAX_FAILURES + WORKERS, String(singles));
    await time.advance(10_000);
    assert.equal(cellRuns(host).length, runs);
    assert.equal(host.osd.length, 0, "no message per failure");
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

test("I07 one bad position fails only its own cell: the rest of its run is made, it shows a neighbour", async () => {
  const media = mediaFile();
  const data = mkdtempSync(join(tmpdir(), "vestra-preview-data-"));
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file); // 10 cells; cell 4 seeks to 9 s
    fakeFfmpeg(host);
    const real = host.tools.handlers.get("ffmpeg")!;
    host.tools.handle("ffmpeg", (args) => {
      if (seeksOf(args).includes("9.000")) throw new HostError("failed", "Invalid data found when processing input");
      return real(args);
    });
    const { stop } = await start(host, data);
    const bar = host.mounted();
    bar.emit("pointing", true);
    await generating(host, time);
    await until(async () => {
      await time.advance(500);
      return (bar.props["preview-map"] as number[]).join() === "0,1,2,3,3,5,6,7,8,9";
    });
    assert.equal(bar.props["hover-thumbnail"], true);
    const dir = join(data, "thumbs", readdirSync(join(data, "thumbs"))[0]!);
    await until(() => {
      const index = JSON.parse(readFileSync(join(dir, "index.json"), "utf8"));
      return index.complete && index.failed.join() === "4" && index.made.length === 9;
    });
    await stop();
  } finally {
    time.restore();
    media.done();
    rmSync(data, { recursive: true, force: true });
  }
});

test("I07 ffmpeg gone after the start: the run fails as unavailable and the option greys out", async () => {
  const media = mediaFile();
  const host = new TestHost();
  const time = fakeClock(host);
  try {
    playing(host, media.file);
    host.tools.handle("ffmpeg", () => {
      throw new HostError("unavailable", "ffmpeg was not found");
    });
    const { stop } = await start(host);
    await generating(host, time);
    await until(() => host.command("hover-thumbnail").state.reason === NEEDS_FFMPEG);
    await until(() => host.mounted().props["hover-thumbnail"] === false);
    const runs = cellRuns(host).length; // the two workers' first cells
    assert.ok(runs <= WORKERS, String(runs));
    await time.advance(10_000);
    assert.equal(cellRuns(host).length, runs);
    await stop();
  } finally {
    time.restore();
    media.done();
  }
});

// ------------------------------------------------------------------ the option

test("I07 on by default (0.2.0); an older settings file's off was the old default; a choice now is kept", async () => {
  const fresh = await start();
  await until(() => fresh.host.command("hover-thumbnail").state.reason === NEEDS_FFMPEG);
  assert.deepEqual(
    (({ enabled, checked }) => ({ enabled, checked }))(fresh.host.command("hover-thumbnail").state),
    { enabled: false, checked: true },
    "on, but greyed without ffmpeg",
  );
  await fresh.stop();

  const data = mkdtempSync(join(tmpdir(), "vestra-preview-data-"));
  const launch = async () => {
    const host = new TestHost();
    fakeFfmpeg(host);
    const app = await start(host, data);
    await until(() => app.host.command("hover-thumbnail").state.enabled);
    return app;
  };
  const saved = () => JSON.parse(readFileSync(join(data, "settings.json"), "utf8"));
  try {
    const a = await launch();
    assert.equal(a.host.command("hover-thumbnail").state.checked, true, "…and on where ffmpeg is");
    await a.stop();
    // 0.1.x saved every field on any change: `false` there was only the default.
    writeFileSync(join(data, "settings.json"), JSON.stringify({ hoverThumbnail: false, hoverTime: true }));
    const b = await launch();
    assert.equal(b.host.command("hover-thumbnail").state.checked, true);
    await b.host.command("hover-thumbnail").run();
    assert.equal(b.host.osd.at(-1), "Show thumbnail on seek bar hover: Off");
    await until(() => {
      try {
        return saved().hoverThumbnail === false && saved().format === 2;
      } catch {
        return false;
      }
    });
    await b.stop();
    const c = await launch();
    assert.equal(c.host.command("hover-thumbnail").state.checked, false, "turned off in 0.2.0: stays off");
    await c.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("I07 ffmpeg installed while the player runs: the next source finds it", async () => {
  const { host, stop } = await start();
  await until(() => host.command("hover-thumbnail").state.reason === NEEDS_FFMPEG);
  fakeFfmpeg(host);
  host.openElsewhere("/movie.mkv");
  await until(() => host.command("hover-thumbnail").state.enabled);
  await stop();
});

test("I07 bar.slint crops the pointer's cell from the sheets itself: no round trip on hover or drag", () => {
  const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
  assert.doesNotMatch(slint, /hover-changed|hover-ended/);
  assert.match(slint, /PlaybackBridge\.preview-map\[clamp\(Math\.floor\(seek-bar\.pointer-fraction \* PlaybackBridge\.preview-count\)/);
  // 0.2.3: the box and the two sheet images are permanent elements (`visible:`),
  // one per sheet: a recreated Image uploads its 1600×900 texture again, and
  // nothing under the pointer may toggle `if` on hover.
  assert.match(slint, /visible: preview\.cell >= 0 && preview\.cell < 100;\s+width: 160px;\s+height: 90px;\s+source: PlaybackBridge\.preview-sheet-a;/);
  assert.match(slint, /visible: preview\.cell >= 100;\s+width: 160px;\s+height: 90px;\s+source: PlaybackBridge\.preview-sheet-b;/);
  assert.match(slint, /tip := Rectangle \{\s+visible: \(PlaybackBridge\.hover-time \|\| PlaybackBridge\.hover-thumbnail \|\| seek-bar\.near-mark >= 0\)[^\n]* && seek-bar\.pointing;/);
  assert.match(slint, /preview := Rectangle \{\s+visible: tip\.picture;/);
  assert.doesNotMatch(slint, /if [^\n]*(pointing|preview\.cell|hover-thumbnail)[^\n]*: (Rectangle|Image)/, "no `if` toggled by the pointer");
  for (const clip of ["source-clip-x: Math.mod(preview.slot, 10) * 160;", "source-clip-y: Math.floor(preview.slot / 10) * 90;"])
    assert.ok(slint.includes(clip), clip);
  // The tip shows while pressed too, so the picture follows a drag; the plugin
  // hears only when the pointer comes and goes (it sends the sheets meanwhile).
  assert.match(slint, /out property <bool> pointing: touch\.has-hover \|\| touch\.pressed;/);
  assert.match(slint, /changed pointing => \{ PlaybackBridge\.pointing\(self\.pointing\); \}/);
});
