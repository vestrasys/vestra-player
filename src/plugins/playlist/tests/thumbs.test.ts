import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { settleFiles } from "./settle.ts";
import activate from "../main.ts";
import { THUMB_MARGIN } from "../scroll.ts";
import { HOLD_AFTER_OPEN_MS, THUMB_LIMIT, thumbArgs } from "../thumbs.ts";

type ViewRow = { id: string; label: string; duration: string; media: string; detail: string; thumb: Json };
/** A JPEG as far as the plugin cares (the test host does not decode pictures). */
const JPEG = Buffer.from([0xff, 0xd8, 0xff, 0xd9]);
const pause = (ms = 0) => new Promise((resolve) => setTimeout(resolve, ms));
/** 62 px rows: a 620 px list shows 10 rows, 11 counting the one cut at the bottom. */
const LIST_HEIGHT = 620;
const ON_SCREEN = 11;

/**
 * The playlist with `count` media files in one folder, ffprobe answering 100 s at
 * once, and an ffmpeg whose runs wait until the test releases them (`gate`) or
 * answer at once. Every panel write is recorded.
 */
async function start(
  // `root`: the media folder of an earlier start (its files are kept as they are, and removed by the caller).
  options: { count?: number; data?: string; names?: string[]; gate?: boolean; root?: string } = {},
) {
  const root = options.root ?? (await realpath(await mkdtemp(join(tmpdir(), "vestra-playlist-thumbs-"))));
  const names =
    options.names ?? Array.from({ length: options.count ?? 300 }, (_, i) => `Clip ${String(i).padStart(4, "0")}.mp4`);
  const files: string[] = [];
  for (const name of names) {
    const file = join(root, name);
    if (!options.root) await writeFile(file, "x");
    files.push(file);
  }
  const host = new TestHost();
  settleFiles(host);
  host.tools.handle("ffprobe", () => ({ stdout: JSON.stringify({ format: { duration: "100" } }) }));
  const gates: { file: string; release: (bytes?: Buffer | Error) => void }[] = [];
  let running = 0,
    most = 0;
  host.tools.handle("ffmpeg", async (args) => {
    const file = args[args.indexOf("-i") + 1]!;
    running++;
    most = Math.max(most, running);
    try {
      const answer = options.gate
        ? await new Promise<Buffer | Error | undefined>((resolve) => gates.push({ file, release: resolve }))
        : JPEG;
      if (answer instanceof Error) throw answer;
      return { stdout: answer ?? JPEG };
    } finally {
      running--;
    }
  });
  const writes: { property: string; value: Json }[] = [];
  const request = host.request.bind(host);
  host.request = (async (method: string, params: Record<string, Json> = {}) => {
    const result = await request(method, params);
    // The value the panel shows after the write (a row patch, SDK 1.5.44, carries operations).
    if (method === "ui.setMount") {
      const property = String(params.property);
      writes.push({ property, value: structuredClone("patch" in params ? (host.mounted().props[property] as Json) : (params.value as Json)) });
    }
    return result;
  }) as typeof host.request;
  const data = options.data ?? (await mkdtemp(join(tmpdir(), "vestra-playlist-thumbs-data-")));
  const { stop } = await startPlugin((plugin) => activate(plugin), {
    directory: new URL("..", import.meta.url),
    host,
    dataDirectory: data,
  });
  const panel = host.mounted();
  const app = {
    host,
    panel,
    root,
    files,
    data,
    gates,
    writes,
    most: () => most,
    /** ffmpeg runs so far: the file each one was for. */
    made: () => host.tools.calls.filter((call) => call.tool === "ffmpeg").map((call) => call.args[call.args.indexOf("-i") + 1]!),
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    /** Each row's picture (0.2.0: in the row). */
    thumbs: () => app.rows().map((row) => row.thumb),
    /** The file each row's picture shows (null: none, the row holds `null`). */
    pictures: () => app.thumbs().map((value) => {
      return value === null ? null : (host.imagePath(value) ?? null);
    }),
    until: async (check: () => boolean, what: string) => {
      for (const deadline = Date.now() + 15_000; !check(); await host.settle(5))
        if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
      await host.settle();
    },
    scroll: async (y: number, height = LIST_HEIGHT) => {
      panel.emit("list-area", y, height, 31);
      await host.settle();
    },
    async add() {
      host.dialogAnswers.push(files);
      await host.command("playlist.add").run();
      await app.until(() => app.rows().length === files.length, "the files to be added");
    },
    stop,
    async close() {
      await stop();
      if (!options.root) await rm(root, { recursive: true, force: true });
      if (!options.data) await rm(data, { recursive: true, force: true });
    },
  };
  return app;
}

test("thumbArgs: a keyframe at the time for video, the cover art for audio; small, one thread", () => {
  const video = thumbArgs("/m/a.mp4", 12.5);
  assert.deepEqual(video.slice(video.indexOf("-skip_frame"), video.indexOf("-i") + 2), [
    "-skip_frame",
    "nokey",
    "-noaccurate_seek",
    "-ss",
    "12.500",
    "-i",
    "/m/a.mp4",
  ]);
  assert.equal(video[video.indexOf("-map") + 1], "0:V:0", "not the cover art of a video file");
  assert.match(video[video.indexOf("-vf") + 1]!, /^scale=192:108:force_original_aspect_ratio=decrease/);
  assert.equal(video.filter((arg) => arg === "-threads").length, 2);
  assert.deepEqual(video.slice(-4), ["image2pipe", "-c:v", "mjpeg", "-"], "a JPEG on stdout");
  const audio = thumbArgs("/m/a.mp3", null);
  assert.ok(!audio.includes("-ss") && !audio.includes("-skip_frame"));
  assert.equal(audio[audio.indexOf("-map") + 1], "0:v:0");
  assert.ok(!thumbArgs("/m/a.mp4", 0).includes("-ss"));
});

test("thumbnails on by default: made only for the rows on screen and a few around, two at a time; a picture does not resend the rows", async () => {
  const app = await start({ gate: true });
  try {
    assert.equal(app.panel.props.thumbnails, true, "on by default");
    await app.add();
    // Before the list reports its size nothing is made.
    await app.host.settle(20);
    assert.equal(app.made().length, 0);
    await app.scroll(0);
    const near = ON_SCREEN + THUMB_MARGIN;
    await app.until(() => app.rows().slice(0, near).every((row) => row.duration === "01:40"), "the durations");
    await app.until(() => app.gates.length === THUMB_LIMIT, "two ffmpeg runs");
    assert.equal(app.made().length, THUMB_LIMIT, "no more than two at a time");
    assert.equal(app.rows()[0]!.media, "video");
    assert.equal(app.rows()[0]!.detail, basename(app.root));
    // At a tenth of the length the probe read.
    const first = app.host.tools.calls.find((call) => call.tool === "ffmpeg")!.args;
    assert.equal(first[first.indexOf("-ss") + 1], "10.000");

    const rowWrites = () => app.writes.filter((write) => write.property === "rows");
    const before = rowWrites().length;
    const shown = app.rows().map((row) => JSON.stringify(row));
    // Lookups finish in any order; whichever row runs first gets its picture.
    const released = app.gates.shift()!;
    released.release();
    const at = app.files.indexOf(released.file);
    await app.until(() => app.pictures()[at] !== null, "the first picture");
    assert.ok(rowWrites().length > before);
    // 0.2.0: the picture is in its row; the other rows are unchanged (on player 0.6.0 the
    // write is one `set` of that row).
    const changed = app.rows().flatMap((row, i) => (JSON.stringify(row) === shown[i] ? [] : [i]));
    assert.deepEqual(changed, [at], "a picture arriving changes its row only");
    assert.equal(app.pictures().filter(Boolean).length, 1, "the others keep the placeholder");

    // Release the rest: exactly the rows on screen and THUMB_MARGIN below get a picture.
    while (app.made().length < near || app.gates.length) {
      await app.until(() => app.gates.length > 0, "the next run");
      app.gates.shift()!.release();
      await app.host.settle(5);
    }
    await app.host.settle(50);
    assert.equal(app.made().length, near);
    assert.deepEqual(app.made().sort(), app.files.slice(0, near).sort());
    assert.ok(app.most() <= THUMB_LIMIT);
    assert.ok(app.pictures().slice(0, near).every((path) => path !== null));
    assert.ok(app.pictures().slice(near).every((path) => path === null));
    // Each picture is a file in the plugin's data directory.
    assert.equal((await readdir(join(app.data, "thumbs"))).filter((name) => name.endsWith(".jpg")).length, near);
  } finally {
    await app.close();
  }
});

test("thumbnails: scrolling away ends the runs and drops the waiting rows; the runs go to the new rows", async () => {
  const app = await start({ gate: true });
  try {
    await app.add();
    await app.scroll(0);
    await app.until(() => app.gates.length === THUMB_LIMIT, "two ffmpeg runs");
    const before = new Set(app.made());
    const started = before.size;
    // Far down: rows 200–210 on screen.
    await app.scroll(200 * 62);
    await app.until(() => app.made().length === started + THUMB_LIMIT, "runs for the new rows");
    const fresh = app.made().slice(started);
    assert.ok(
      fresh.every((file) => app.files.indexOf(file) >= 200 - THUMB_MARGIN && app.files.indexOf(file) < 200 + ON_SCREEN + THUMB_MARGIN),
      `${fresh.map((file) => basename(file)).join(", ")} are near the view`,
    );
    // The ended runs answer late: nothing is kept from them, and rows 2–13 were never run.
    for (const gate of app.gates.splice(0, started)) gate.release();
    await app.host.settle(20);
    assert.ok(!existsSync(join(app.data, "thumbs")) || (await readdir(join(app.data, "thumbs"))).every((name) => !name.endsWith(".jpg")));
    assert.ok(app.made().every((file) => before.has(file) || app.files.indexOf(file) >= 200 - THUMB_MARGIN));
  } finally {
    await app.close();
  }
});

test("thumbnails: cached on disk by file; a restart makes none again; a file without a picture is not retried", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-thumbs-data-"));
  const names = ["a.mp4", "b.mkv", "song.mp3", "plain.flac"];
  let app = await start({ data, names });
  try {
    app.host.tools.handle("ffmpeg", (args) => {
      if (args.at(-1) === "-" && args.includes(join(app.root, "plain.flac")))
        throw new HostError("failed", "Stream map '0:v:0' matches no streams.");
      return { stdout: JPEG };
    });
    await app.add();
    await app.scroll(0);
    await app.until(() => app.made().length === 4, "four ffmpeg runs");
    await app.until(() => app.pictures().filter(Boolean).length === 3, "three pictures");
    assert.deepEqual(
      app.rows().map((row) => row.media),
      ["video", "video", "audio", "audio"],
    );
    const audio = app.host.tools.calls.find((call) => call.args.includes(join(app.root, "song.mp3")))!.args;
    assert.ok(!audio.includes("-ss"), "the cover art: no seeking");
    assert.equal(app.pictures()[3], null, "no cover art: the placeholder");
    await app.stop();
    const index = JSON.parse(await readFile(join(data, "thumb-cache.json"), "utf8")) as [string, boolean][];
    assert.deepEqual(index.map(([, picture]) => picture).sort(), [false, true, true, true]);

    // Same files, same data directory: every row from the cache.
    const second = await start({ data, names, root: app.root });
    app = second;
    await second.add();
    await second.scroll(0);
    await second.until(() => second.pictures().filter(Boolean).length === 3, "the cached pictures");
    await second.host.settle(50);
    assert.equal(second.made().length, 0, "no ffmpeg run");
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
    await rm(app.root, { recursive: true, force: true });
  }
});

test("thumbnails: the bottom bar's toggle turns them off (no work, compact rows) and back; remembered across a restart", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-thumbs-data-"));
  let app = await start({ data, gate: true });
  try {
    await app.add();
    await app.scroll(10 * 62);
    await app.until(() => app.gates.length === THUMB_LIMIT, "two ffmpeg runs");
    const made = app.made().length;
    const asked = app.panel.props["scroll-request"] as number;
    app.panel.emit("toggle-thumbnails");
    await app.host.settle();
    assert.equal(app.panel.props.thumbnails, false);
    assert.ok(app.pictures().every((path) => path === null), "no pictures sent: every row shows the placeholder");
    assert.equal(app.rows()[0]!.media, "");
    assert.equal(app.rows()[0]!.detail, "");
    // The row at the top of the view stays there: row 10 of 20 px rows.
    assert.ok((app.panel.props["scroll-request"] as number) > asked);
    assert.equal(app.panel.props["scroll-to-y"], 10 * 20);
    await app.scroll(10 * 20, LIST_HEIGHT);
    await app.scroll(150 * 20, LIST_HEIGHT);
    for (const gate of app.gates.splice(0)) gate.release();
    await app.host.settle(50);
    assert.equal(app.made().length, made, "off: no ffmpeg run while scrolling");

    await app.stop();
    const saved = JSON.parse(await readFile(join(data, "playlists.json"), "utf8")) as { settings: { thumbnails: boolean } };
    assert.equal(saved.settings.thumbnails, false);
    const second = await start({ data, gate: true, root: app.root });
    app = second;
    assert.equal(second.panel.props.thumbnails, false, "still off after a restart");
    await second.add();
    await second.scroll(0);
    await second.host.settle(50);
    assert.equal(second.made().length, 0);
    second.panel.emit("toggle-thumbnails");
    await second.host.settle();
    assert.equal(second.panel.props.thumbnails, true);
    await second.until(() => second.gates.length === THUMB_LIMIT, "runs again once on");
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
    await rm(app.root, { recursive: true, force: true });
  }
});

test("thumbnails: ffmpeg waits while a source opens and for its first seconds", async () => {
  const app = await start({ names: ["a.mp4", "b.mp4", "c.mp4", "d.mp4", "e.mp4", "f.mp4"] });
  try {
    await app.add();
    app.panel.emit("play-row", app.rows()[0]!.id);
    await app.host.settle();
    app.host.setSource({ phase: "loading" });
    await app.host.settle();
    await app.scroll(0);
    await app.host.settle(50);
    assert.equal(app.made().length, 0, "nothing while the source loads");
    app.host.setSource({ phase: "ready", duration: 100, position: 0 });
    await app.host.settle(50);
    assert.equal(app.made().length, 0, "nor in its first seconds");
    await pause(HOLD_AFTER_OPEN_MS);
    await app.until(() => app.made().length === 6, "the runs after the hold");
    assert.ok(app.pictures().slice(0, 6).every(Boolean));
  } finally {
    await app.close();
  }
});

test("thumbnails: a link row shows the link placeholder and its host; nothing is run for it", async () => {
  const app = await start({ names: [] });
  try {
    // Links come in through the link window; a list file is the simplest route here.
    const list = join(app.root, "list.m3u8");
    await writeFile(list, "#EXTM3U\nhttps://example.com/watch?v=1\n");
    app.host.dialogAnswers.push([list]);
    await app.host.command("playlist.open-list").run();
    await app.until(() => app.rows().length === 1, "the link row");
    await app.scroll(0);
    await app.host.settle(50);
    assert.equal(app.rows()[0]!.media, "link");
    assert.equal(app.rows()[0]!.detail, "example.com");
    assert.equal(app.made().length, 0);
    assert.deepEqual(app.pictures(), [null]);
  } finally {
    await app.close();
  }
});
