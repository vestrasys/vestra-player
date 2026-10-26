import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";
import type { PlaybackStats } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { MAX_MARKS, seekMarks } from "../bar.ts";
import activate from "../main.ts";
import { clock, ffmpegArgs, MAX_FAILURES, NEEDS_FFMPEG, SETTLE_MS, slotOf } from "../thumbs.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  // The Chinese texts are what the plugin showed before English became the source.
  startPlugin(activate, { directory, host, locale: "zh-Hans", ...(dataDirectory ? { dataDirectory } : {}) });
const VIDEO = {
  codec: "h264",
  width: 1920,
  height: 1080,
  fps: 24,
  decoder: "h264",
  pixelFormat: "yuv420p",
} as unknown as PlaybackStats["video"];

// A ceiling, not a delay: generous because the whole suite runs in parallel.
async function until(predicate: () => boolean, ms = 5000) {
  const deadline = Date.now() + ms;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** A real file to open (thumbnails need its realpath and mtime), removed after the test. */
function mediaFile(name = "movie.mkv") {
  const dir = mkdtempSync(join(tmpdir(), "vestra-playback-"));
  const file = join(dir, name);
  writeFileSync(file, "not really a video");
  return { file, done: () => rmSync(dir, { recursive: true, force: true }) };
}

function withMedia(host: TestHost, source: string, duration = 1000) {
  host.openElsewhere(source);
  host.setSource({ duration, position: 0 });
  return host;
}

/** ffmpeg that writes a small picture to its output path; `gate` holds a run until released. */
function fakeFfmpeg(host: TestHost, gate?: () => Promise<void>) {
  host.tools.handle("ffmpeg", async (args) => {
    await gate?.();
    writeFileSync(args.at(-1)!, Buffer.from([0xff, 0xd8, 0xff, 0xd9]));
    return {};
  });
}

/** Turn I07 on with a local video open: the bar shows the picture box. */
async function thumbnailsOn(host: TestHost) {
  await host.command("hover-thumbnail").run();
  await until(() => host.mounted().props["hover-thumbnail"] === true);
}

const ffmpegRuns = (host: TestHost) => host.tools.calls.filter((c) => c.tool === "ffmpeg");

/** The file name of the picture the bar shows (the folder uses the OS's separator). */
function shown(host: TestHost) {
  const path = host.imagePath(host.mounted().props.thumbnail);
  return path ? basename(path) : null;
}

/** Take over the settle timer: pending timers wait until the test fires them. */
function manualClock() {
  const saved = { ...clock };
  const pending = new Map<number, { run: () => void; ms: number }>();
  let next = 0;
  clock.setTimeout = (run, ms) => {
    pending.set(++next, { run, ms });
    return next;
  };
  clock.clearTimeout = (timer) => void pending.delete(timer as number);
  return {
    pending,
    /** Run every pending timer (the pointer has rested long enough). */
    fire() {
      const due = [...pending.values()];
      pending.clear();
      for (const timer of due) timer.run();
    },
    restore: () => Object.assign(clock, saved),
  };
}

// ------------------------------------------------------------------ B22

test("B22 the seek line gets everyone's marks as fractions with their time text; a new source clears them", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted();
  assert.deepEqual(bar.props.marks, []);
  host.setTimelineLayer("vestra.marks", [
    { time: 0, kind: "chapter", title: "开头" },
    { time: 83, kind: "bookmark", title: "精彩" },
    { time: 150, kind: "chapter" },
  ]);
  host.setTimelineLayer("community.other", [{ time: 100, kind: "marker", title: "A" }]);
  await until(() => (bar.props.marks as unknown[]).length === 4);
  assert.deepEqual(bar.props.marks, [
    { at: 0, kind: "chapter", title: "开头", clock: "00:00" },
    { at: 0.415, kind: "bookmark", title: "精彩", clock: "01:23" },
    { at: 0.5, kind: "marker", title: "A", clock: "01:40" },
    { at: 0.75, kind: "chapter", title: "", clock: "02:30" },
  ]);
  // The marks' publisher is disabled (its layer cleared): nothing of it stays.
  host.setTimelineLayer("vestra.marks", []);
  await until(() => (bar.props.marks as unknown[]).length === 1);
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => (bar.props.marks as unknown[]).length === 0);
  await stop();
});

test("B22 marks need a known duration, stay inside the line, and are capped for the bar's search", () => {
  const marks = [{ time: 5, kind: "chapter" as const, title: null, plugin: "x" }];
  assert.deepEqual(seekMarks(marks, null), []);
  assert.deepEqual(seekMarks(marks, 0), []);
  assert.deepEqual(seekMarks([{ ...marks[0]!, time: 12 }], 10), [
    { at: 1, kind: "chapter", title: "", clock: "00:12" },
  ]);
  const many = Array.from({ length: MAX_MARKS + 5 }, (_, i) => ({ ...marks[0]!, time: i }));
  assert.equal(seekMarks(many, 10_000).length, MAX_MARKS);
});

test("B22 the A-B loop is drawn whoever set it, and goes with the source", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted();
  assert.equal(bar.props["has-loop"], false);
  host.setSource({ loop: { a: 20, b: 50, count: null } });
  await until(() => bar.props["has-loop"] === true);
  assert.equal(bar.props["loop-a"], 0.1);
  assert.equal(bar.props["loop-b"], 0.25);
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => bar.props["has-loop"] === false);
  await stop();
});

test("B22 the bar draws marks with Theme colours and finds the hovered one within 3px", () => {
  const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
  assert.match(slint, /mark\.kind == "bookmark" \? Theme\.accent : mark\.kind == "marker" \? Theme\.playing : Theme\.text-dim/);
  assert.match(slint, /left <= 3px/);
  assert.match(slint, /title \+ "  " \+/);
  // 13 halvings of the lower-bound search cover MAX_MARKS.
  assert.equal(2 ** (slint.match(/= root\.narrow\(/g) ?? []).length, MAX_MARKS);
  for (const file of ["bar.slint", "open-url.slint", "prompt.slint", "time-jump.slint"])
    assert.doesNotMatch(readFileSync(new URL(`../ui/${file}`, import.meta.url), "utf8"), /#[0-9a-fA-F]{6}/, file);
});

// ------------------------------------------------------------------ I07

test("I07 slots are 0.5% of the duration (at least 2 s); ffmpeg seeks before -i and makes one 160×90 frame", () => {
  assert.deepEqual(slotOf(0, 100), { index: 0, seconds: 0 });
  assert.deepEqual(slotOf(3.9, 100), { index: 1, seconds: 2 }); // 0.5 s < 2 s
  assert.deepEqual(slotOf(3000, 6000), { index: 100, seconds: 3000 }); // 30 s slots
  assert.equal(slotOf(100, 100).seconds, 99.5); // never past the end
  const args = ffmpegArgs("/a b/c.mkv", 12.5, "/d/1.jpg");
  assert.ok(args.indexOf("-ss") < args.indexOf("-i"));
  assert.equal(args[args.indexOf("-i") + 1], "/a b/c.mkv");
  assert.deepEqual(args.slice(args.indexOf("-frames:v"), args.indexOf("-frames:v") + 6), [
    "-frames:v",
    "1",
    "-vf",
    "scale=160:90:force_original_aspect_ratio=decrease",
    "-q:v",
    "5",
  ]);
  assert.equal(args.at(-1), "/d/1.jpg");
});

test("I07 without ffmpeg the option is greyed with 「需要 ffmpeg」; it is off by default and saved", async () => {
  const { host, stop } = await start();
  await until(() => host.command("hover-thumbnail").state.enabled === false);
  assert.deepEqual(
    (({ enabled, checked, reason }) => ({ enabled, checked, reason }))(host.command("hover-thumbnail").state),
    { enabled: false, checked: false, reason: NEEDS_FFMPEG },
  );
  await stop();

  const data = mkdtempSync(join(tmpdir(), "vestra-playback-data-"));
  try {
    const first = new TestHost();
    fakeFfmpeg(first);
    const a = await start(first, data);
    await until(() => a.host.command("hover-thumbnail").state.enabled);
    await a.host.command("hover-thumbnail").run();
    assert.equal(a.host.osd.at(-1), "进度条悬停显示缩略图: 开");
    assert.equal(a.host.command("hover-thumbnail").state.checked, true);
    await until(() => {
      try {
        return JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).hoverThumbnail === true;
      } catch {
        return false;
      }
    });
    await a.stop();
    const second = new TestHost();
    fakeFfmpeg(second);
    const b = await start(second, data);
    assert.equal(b.host.command("hover-thumbnail").state.checked, true);
    await b.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("I07 a resting pointer makes a picture once; the same slot then shows at once from the cache", async () => {
  const media = mediaFile();
  const timers = manualClock();
  try {
    const host = withMedia(new TestHost(), media.file);
    host.stats = { ...host.stats, video: VIDEO };
    fakeFfmpeg(host);
    const { stop } = await start(host);
    const bar = host.mounted();
    assert.equal(bar.props["hover-thumbnail"], false); // off by default
    await thumbnailsOn(host);
    assert.equal(bar.props["has-thumbnail"], false);
    // Callbacks reach the plugin asynchronously and `settle` does not wait for them:
    // wait for what each one does instead.
    bar.emit("hover-changed", 0.5);
    await until(() => timers.pending.size === 1);
    // Still settling: one 80 ms timer, no ffmpeg yet.
    assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [SETTLE_MS]);
    assert.equal(ffmpegRuns(host).length, 0);
    timers.fire();
    await until(() => bar.props["has-thumbnail"] === true);
    const runs = ffmpegRuns(host);
    assert.equal(runs.length, 1);
    assert.equal(runs[0]!.args[runs[0]!.args.indexOf("-ss") + 1], "500.000");
    assert.equal(runs[0]!.args[runs[0]!.args.indexOf("-i") + 1], media.file);
    const path = host.imagePath(bar.props.thumbnail);
    assert.ok(path?.startsWith(join(host.dataDirectory, "thumbs")), path ?? "no picture");
    assert.equal(basename(path!), "100.jpg");
    assert.ok(existsSync(path!));
    // Elsewhere, then back into the same 5 s slot: no second run, shown without waiting.
    bar.emit("hover-changed", 0.1);
    await until(() => timers.pending.size === 1);
    timers.fire();
    await until(() => ffmpegRuns(host).length === 2);
    await until(() => shown(host) === "20.jpg");
    bar.emit("hover-changed", 0.502);
    await until(() => host.imagePath(bar.props.thumbnail) === path);
    assert.equal(timers.pending.size, 0); // a cache hit does not wait
    assert.equal(ffmpegRuns(host).length, 2);
    bar.emit("hover-ended");
    await stop();
  } finally {
    timers.restore();
    media.done();
  }
});

test("I07 one ffmpeg at a time: a target passed meanwhile is only cached, the newest runs next", async () => {
  const media = mediaFile();
  try {
    const host = withMedia(new TestHost(), media.file);
    host.stats = { ...host.stats, video: VIDEO };
    const releases: (() => void)[] = [];
    fakeFfmpeg(host, () => new Promise<void>((resolve) => releases.push(resolve)));
    const { stop } = await start(host);
    const bar = host.mounted();
    await thumbnailsOn(host);
    bar.emit("hover-changed", 0.2);
    await until(() => ffmpegRuns(host).length === 1);
    bar.emit("hover-changed", 0.4);
    await sleep(120);
    bar.emit("hover-changed", 0.6);
    await sleep(120);
    assert.equal(ffmpegRuns(host).length, 1); // 0.4 and 0.6 wait
    releases.shift()!();
    await until(() => ffmpegRuns(host).length === 2);
    // 0.2 was cached, not shown (the pointer is at 0.6), and 0.4 was skipped.
    assert.equal(bar.props["has-thumbnail"], false);
    const second = ffmpegRuns(host)[1]!.args;
    assert.equal(second[second.indexOf("-ss") + 1], "600.000");
    releases.shift()!();
    await until(() => shown(host) === "120.jpg");
    bar.emit("hover-changed", 0.2);
    await until(() => shown(host) === "40.jpg");
    assert.equal(ffmpegRuns(host).length, 2);
    await stop();
  } finally {
    media.done();
  }
});

test("I07 only local files with video: network sources and audio show the time only", async () => {
  const media = mediaFile("song.mp3");
  try {
    const host = withMedia(new TestHost(), "https://example.com/a.mp4");
    host.stats = { ...host.stats, video: VIDEO };
    fakeFfmpeg(host);
    const { stop } = await start(host);
    const bar = host.mounted();
    await host.command("hover-thumbnail").run();
    await host.settle();
    assert.equal(bar.props["hover-thumbnail"], false);
    bar.emit("hover-changed", 0.5);
    await sleep(120);
    assert.equal(ffmpegRuns(host).length, 0);
    host.stats = { ...host.stats, video: null };
    host.openElsewhere(media.file);
    host.setSource({ duration: 100 });
    await sleep(50);
    assert.equal(bar.props["hover-thumbnail"], false);
    await stop();
  } finally {
    media.done();
  }
});

test("I07 failures show nothing; after five in a row the source is not tried again", async () => {
  const media = mediaFile();
  try {
    const host = withMedia(new TestHost(), media.file);
    host.stats = { ...host.stats, video: VIDEO };
    host.tools.handle("ffmpeg", () => {
      throw new HostError("failed", "Invalid data found when processing input");
    });
    const { stop } = await start(host);
    const bar = host.mounted();
    await thumbnailsOn(host);
    for (let i = 0; i < MAX_FAILURES; i++) {
      bar.emit("hover-changed", i / 10);
      await until(() => ffmpegRuns(host).length === i + 1);
      await sleep(10);
    }
    await until(() => bar.props["hover-thumbnail"] === false);
    assert.equal(host.osd.at(-1), "进度条悬停显示缩略图: 开"); // no message per failure
    bar.emit("hover-changed", 0.9);
    await sleep(120);
    assert.equal(ffmpegRuns(host).length, MAX_FAILURES);
    await stop();
  } finally {
    media.done();
  }
});

test("I07 a new source removes the old source's pictures; a start empties thumbs/", async () => {
  const media = mediaFile();
  const data = mkdtempSync(join(tmpdir(), "vestra-playback-data-"));
  try {
    mkdirSync(join(data, "thumbs", "stale"), { recursive: true });
    writeFileSync(join(data, "thumbs", "stale", "1.jpg"), "x");
    const host = withMedia(new TestHost(), media.file);
    host.stats = { ...host.stats, video: VIDEO };
    fakeFfmpeg(host);
    const { stop } = await start(host, data);
    assert.equal(existsSync(join(data, "thumbs", "stale")), false);
    const bar = host.mounted();
    await thumbnailsOn(host);
    bar.emit("hover-changed", 0.3);
    await until(() => bar.props["has-thumbnail"] === true);
    const [dir] = readdirSync(join(data, "thumbs"));
    assert.ok(dir);
    host.openElsewhere("/other.mkv");
    await until(() => !existsSync(join(data, "thumbs", dir!)));
    assert.equal(bar.props["has-thumbnail"], false);
    await stop();
  } finally {
    media.done();
    rmSync(data, { recursive: true, force: true });
  }
});

test("I07 ffmpeg gone after the start: the run fails as unavailable and the option greys out", async () => {
  const media = mediaFile();
  try {
    const host = withMedia(new TestHost(), media.file);
    host.stats = { ...host.stats, video: VIDEO };
    host.tools.handle("ffmpeg", () => {
      throw new HostError("unavailable", "ffmpeg was not found");
    });
    const { stop } = await start(host);
    await thumbnailsOn(host);
    host.mounted().emit("hover-changed", 0.5);
    await until(() => host.command("hover-thumbnail").state.reason === NEEDS_FFMPEG);
    assert.equal(host.mounted().props["hover-thumbnail"], false);
    await stop();
  } finally {
    media.done();
  }
});

// ------------------------------------------------------------------ A02

test("A02 the window uses the shared UrlInputRow instead of its own input row", () => {
  const slint = readFileSync(new URL("../ui/open-url.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{ UrlInputRow \} from "@vestra\/widgets\.slint"/);
  assert.match(slint, /enabled: !root\.busy/);
  assert.match(slint, /confirm-text: root\.busy \? @tr\("Opening…"\) : @tr\("Open"\)/);
  assert.doesNotMatch(slint, /TextEdit|Button/);
});

test("I07 ffmpeg installed while the player runs: the next source finds it", async () => {
  const { host, stop } = await start();
  await until(() => host.command("hover-thumbnail").state.reason === NEEDS_FFMPEG);
  fakeFfmpeg(host);
  host.openElsewhere("/movie.mkv");
  await until(() => host.command("hover-thumbnail").state.enabled);
  await stop();
});
