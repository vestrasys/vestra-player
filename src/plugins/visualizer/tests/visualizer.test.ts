import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { albumLine, CoverCache, fileName, folderCover, limits, parseProbe } from "../covers.ts";
import { imageInfo, imageProblem } from "../images.ts";
import { isUrl, localFile } from "../local-path.ts";
import { NOT_IMAGE, TOO_LARGE, UNREADABLE } from "../logo.ts";
import activate, { loadingName, PAGE_ID } from "../main.ts";
import { readSettings } from "../settings.ts";

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-visualizer-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));
let folders = 0;
/** A new folder with these files (name -> bytes). */
function folder(files: Record<string, Uint8Array | string>) {
  const path = join(scratch, `f${folders++}`);
  mkdirSync(path);
  for (const [name, bytes] of Object.entries(files)) writeFileSync(join(path, name), bytes);
  return path;
}

/** A PNG header of the given size (enough for the size checks; the host decodes, the tests do not). */
function png(width: number, height: number) {
  const bytes = Buffer.alloc(33);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(bytes, 0);
  bytes.writeUInt32BE(13, 8);
  bytes.write("IHDR", 12, "latin1");
  bytes.writeUInt32BE(width, 16);
  bytes.writeUInt32BE(height, 20);
  return bytes;
}
/** A JPEG header: SOI, an APP0 segment, SOF0 with the size, EOI. */
function jpeg(width: number, height: number) {
  const app0 = [0xff, 0xe0, 0x00, 0x10, ...Buffer.from("JFIF\0"), 1, 1, 0, 0, 1, 0, 1, 0, 0];
  const sof = [0xff, 0xc0, 0x00, 0x11, 8, height >> 8, height & 255, width >> 8, width & 255, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
  return Buffer.from([0xff, 0xd8, ...app0, ...sof, 0xff, 0xd9]);
}

const PROBE_TAGGED = JSON.stringify({
  streams: [
    { index: 0, codec_type: "audio", disposition: { attached_pic: 0 } },
    { index: 1, codec_type: "video", disposition: { attached_pic: 1 } },
  ],
  format: { tags: { title: "示例曲目", artist: "Vestra 测试", album: "封面专辑", date: "2024-03-01" } },
});
const PROBE_PLAIN = JSON.stringify({ streams: [{ index: 0, codec_type: "audio", disposition: { attached_pic: 0 } }], format: {} });

const VIDEO_STATS = {
  codec: "h264",
  width: 1920,
  height: 1080,
  fps: 24,
  decoder: null,
  pixelFormat: null,
  colorspace: null,
  displayWidth: null,
  displayHeight: null,
  displayFps: null,
  bitrate: null,
};

const running = new Set<() => Promise<void>>();
/** Started in English unless a locale is given. */
async function start(host = new TestHost(), dataDirectory?: string, locale: "en" | "zh-Hans" = "en") {
  const started = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  await host.settle();
  return { ...started, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

const props = (host: TestHost) => host.mounted().props;
/** Wait for ffprobe / ffmpeg answers and file writes. */
async function settle(host: TestHost) {
  for (let i = 0; i < 6; i++) await host.settle(20);
}

/**
 * Settings ▸ Appearance ▸ Choose Logo Image… with this dialog answer. `run()`
 * waits for host calls only; the plugin checks and copies the picture itself
 * (stat, read, write, rename), which can outlast it (it does on Windows). Every
 * picked file ends in one notice, so wait for it, then for the state update.
 */
async function chooseLogo(host: TestHost, answer: string[] | null) {
  const before = host.osd.length;
  host.dialogAnswers.push(answer);
  await host.command("choose-logo").run();
  for (let i = 0; i < 250 && answer && host.osd.length === before; i++) await host.settle(20);
  await host.settle();
}
/** Reset ▸ Idle Logo: the plugin deletes the copy itself, then shows its notice. */
async function resetLogo(host: TestHost) {
  const before = host.osd.length;
  await host.command("reset-logo").run();
  for (let i = 0; i < 250 && host.osd.length === before; i++) await host.settle(20);
  await host.settle();
}

// ------------------------------------------------------------------ pure parts

test("images: PNG and JPEG sizes from their headers; limits", () => {
  assert.deepEqual(imageInfo(png(640, 480)), { type: "png", width: 640, height: 480 });
  assert.deepEqual(imageInfo(jpeg(1000, 600)), { type: "jpeg", width: 1000, height: 600 });
  assert.equal(imageInfo(Buffer.from("GIF89a....")), null);
  assert.equal(imageProblem(png(4096, 4096)), null);
  assert.equal(imageProblem(png(5000, 100)), "size");
  assert.equal(imageProblem(jpeg(100, 4097)), "size");
  assert.equal(imageProblem(Buffer.from("not an image")), "type");
});

test("tags: any case, stream tags for Ogg, album · year; folder cover order; names", () => {
  const flac = parseProbe(JSON.stringify({ streams: [{ index: 0, codec_type: "audio" }], format: { tags: { TITLE: "T", ARTIST: "A", DATE: "2019-05-01" } } }));
  assert.deepEqual(flac, { tags: { title: "T", artist: "A", album: "", date: "2019-05-01" }, attached: null });
  assert.equal(albumLine(flac.tags), "2019");
  const opus = parseProbe(JSON.stringify({ streams: [{ index: 0, codec_type: "audio", tags: { title: "Opus" } }], format: {} }));
  assert.equal(opus.tags.title, "Opus");
  assert.equal(parseProbe(PROBE_TAGGED).attached, 1);
  assert.equal(albumLine(parseProbe(PROBE_TAGGED).tags), "封面专辑 · 2024");
  assert.equal(folderCover(["front.jpg", "Folder.PNG", "a.mp3"]), "Folder.PNG");
  assert.equal(folderCover(["cover.png", "COVER.JPG"]), "COVER.JPG");
  assert.equal(folderCover(["a.jpg"]), null);
  assert.equal(fileName("/Music/a b.flac"), "a b.flac");
  assert.equal(fileName("https://radio.example/live/%E9%9F%B3.mp3?x=1"), "音.mp3");
});

test("paths: names by the rules of each OS; links; file: URLs", () => {
  // POSIX (macOS): only `/` separates; a backslash is part of a name.
  assert.equal(fileName("/Music/a b.flac", "posix"), "a b.flac");
  assert.equal(fileName("/Music/a\\b.flac", "posix"), "a\\b.flac");
  assert.equal(fileName("file:///Music/%E9%9F%B3.mp3", "posix"), "音.mp3");
  assert.equal(localFile("file:///Music/a.mp3", "posix"), "/Music/a.mp3");
  assert.equal(localFile("/Music/a.mp3", "posix"), "/Music/a.mp3");
  assert.equal(localFile("https://radio.example/a.mp3", "posix"), null);
  assert.equal(isUrl("/Music/a.mp3", "posix"), false);
  // Windows: `\` and `/` both separate; drive, UNC and `C://` paths are paths, not links.
  assert.equal(fileName("C:\\Music\\a b.flac", "windows"), "a b.flac");
  assert.equal(fileName("C:/Music/a.flac", "windows"), "a.flac");
  assert.equal(fileName("\\\\nas\\share\\专辑\\b.mp3", "windows"), "b.mp3");
  assert.equal(isUrl("C://Music/a.mp3", "windows"), false);
  assert.equal(fileName("file:///C:/Music/%E9%9F%B3.mp3", "windows"), "音.mp3");
  assert.equal(localFile("file:///C:/Music/a.mp3", "windows"), "C:\\Music\\a.mp3");
  assert.equal(localFile("file://nas/share/a.mp3", "windows"), "\\\\nas\\share\\a.mp3");
  // No drive: not a local file on Windows; named like a link.
  assert.equal(localFile("file:///Music/a.mp3", "windows"), null);
  assert.equal(fileName("file:///Music/a.mp3", "windows"), "a.mp3");
  // Links are the same everywhere.
  for (const rules of ["posix", "windows"] as const) {
    assert.equal(fileName("https://radio.example/live/%E9%9F%B3.mp3?x=1", rules), "音.mp3");
    assert.equal(fileName("https://radio.example/", rules), "radio.example");
    assert.equal(isUrl("https://radio.example/", rules), true);
  }
  // The loading screen: a path's name only, a link's host and name.
  assert.equal(loadingName("C:\\Users\\me\\Music\\电影.mkv", "windows"), "电影.mkv");
  assert.equal(loadingName("\\\\nas\\share\\clip.mp4", "windows"), "clip.mp4");
  assert.equal(loadingName("C://media/clip.mp4", "windows"), "clip.mp4");
  assert.equal(loadingName("file:///C:/media/clip.mp4", "windows"), "clip.mp4");
  assert.equal(loadingName("file:///media/clip.mp4", "posix"), "clip.mp4");
  assert.equal(loadingName("/media/a\\b.mp4", "posix"), "a\\b.mp4");
  assert.equal(loadingName("https://cdn.example.com/films/clip.mp4", "windows"), "cdn.example.com/clip.mp4");
});

test("J11: a file: URL is read like its path (cover and tags)", async () => {
  const host = new TestHost();
  const dir = folder({ "u.flac": "x", "cover.jpg": jpeg(40, 40) });
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.openElsewhere(pathToFileURL(join(dir, "u.flac")).href);
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title, props(host)["has-cover"]], ["audio", "u.flac", true]);
  assert.equal(host.tools.calls[0]!.args.at(-1), realpathSync(join(dir, "u.flac")));
});

test("settings: defaults, field by field", () => {
  assert.deepEqual(readSettings(null), { audio: { on: true }, idle: { on: true, logo: null } });
  assert.deepEqual(readSettings({ audio: { on: false }, idle: { on: "x", logo: "../x.png" } }), {
    audio: { on: false },
    idle: { on: true, logo: null },
  });
  assert.equal(readSettings({ idle: { logo: "logo.jpg" } }).idle.logo, "logo.jpg");
});

// ------------------------------------------------------------------ plugin

test("activation: backdrop, settings page and menu entries, in English", async () => {
  const host = new TestHost();
  await start(host);
  assert.equal(host.mounted().slot, "backdrop");
  assert.deepEqual(host.backdrop, { owner: "vestra.visualizer", effective: true });
  assert.equal(props(host).mode, "idle");
  assert.equal(props(host)["custom-logo"], false);
  const page = host.settingsPage(PAGE_ID);
  assert.deepEqual([page.category, page.title, page.order], ["appearance", "Logo and Album Cover", 20]);
  // Menu sweep (B5): all four commands are in the command palette only (no menu path).
  for (const [id, c] of host.commands) assert.deepEqual([c.declaration.path, c.display.path], [[], []], `${id} has no menu path`);
  const menu = [...host.commands.values()].map((c) => [c.declaration.title, c.declaration.order, c.declaration.group]);
  assert.deepEqual(menu, [
    ["Show Album Cover for Audio Only", 530, "visual"],
    ["Show Logo When Idle", 845, "settings-logo"],
    ["Choose Logo Image…", 846, "settings-logo"],
    ["Idle Logo", 850, "reset-appearance"],
  ]);
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.display.title),
    ["Show Album Cover for Audio Only", "Show Logo When Idle", "Choose Logo Image…", "Idle Logo"],
  );
  // Command palette categories (player 0.7.6): "Category: Title" without the menu path.
  assert.deepEqual(
    [...host.commands.values()].map((c) => [c.display.category, c.declaration.home ?? null]),
    [
      ["Audio", null],
      ["Appearance", null],
      ["Appearance", null],
      ["Reset", null],
    ],
  );
  assert.equal(host.command("reset-logo").state.reason, "Using the default logo");
  assert.equal(host.command("audio-cover").state.checked, true);
  assert.equal(host.command("reset-logo").state.enabled, false);
  assert.equal(Object.values(host.command("audio-cover").declaration.bindings ?? {}).length, 0);
});

test("J11: embedded cover and tags, then the cache answers without ffmpeg", async () => {
  const host = new TestHost();
  const dir = folder({ "song.mp3": "audio bytes" });
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_TAGGED }));
  host.tools.handle("ffmpeg", () => ({ stdout: jpeg(600, 600) }));
  const { plugin } = await start(host);
  host.openElsewhere(join(dir, "song.mp3"));
  await settle(host);
  assert.equal(props(host).mode, "audio");
  assert.equal(props(host).title, "示例曲目");
  assert.equal(props(host).artist, "Vestra 测试");
  assert.equal(props(host).album, "封面专辑 · 2024");
  assert.equal(props(host)["has-cover"], true);
  const cover = host.imagePath(props(host).cover)!;
  assert.ok(cover.startsWith(join(plugin.dataDirectory, "cache", "covers")));
  assert.ok(existsSync(cover));
  const [probe, extract] = host.tools.calls;
  assert.equal(probe!.tool, "ffprobe");
  assert.deepEqual(extract!.args.slice(extract!.args.indexOf("-map"), extract!.args.indexOf("-map") + 2), ["-map", "0:1"]);
  const tags = JSON.parse(readFileSync(join(plugin.dataDirectory, "cache", "covers", "tags.json"), "utf8"));
  assert.equal(Object.values<{ title: string }>(tags.entries)[0]!.title, "示例曲目");

  // Same file again: from the cache.
  host.tools.calls.length = 0;
  host.openElsewhere(join(dir, "song.mp3"));
  await settle(host);
  assert.equal(host.tools.calls.length, 0);
  assert.equal(props(host)["has-cover"], true);
  assert.equal(props(host).title, "示例曲目");
});

test("J11: folder cover copied when there is no ffmpeg; file name as title; placeholder without any", async () => {
  const host = new TestHost();
  const withCover = folder({ "a.flac": "x", "Cover.PNG": png(500, 500) });
  const without = folder({ "b.flac": "x" });
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.openElsewhere(join(withCover, "a.flac"));
  await settle(host);
  assert.equal(props(host).title, "a.flac");
  assert.equal(props(host).artist, "");
  assert.match(host.imagePath(props(host).cover)!, /\.png$/);
  assert.equal(props(host)["has-cover"], true);
  host.openElsewhere(join(without, "b.flac"));
  await settle(host);
  assert.equal(props(host).title, "b.flac");
  assert.equal(props(host)["has-cover"], false);
});

test("J11: a folder cover that changed is read again", async () => {
  const host = new TestHost();
  const dir = folder({ "a.flac": "x", "folder.jpg": jpeg(300, 300) });
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.openElsewhere(join(dir, "a.flac"));
  await settle(host);
  const first = readFileSync(host.imagePath(props(host).cover)!);
  writeFileSync(join(dir, "folder.jpg"), jpeg(320, 200));
  host.openElsewhere(join(dir, "a.flac"));
  await settle(host);
  const second = readFileSync(host.imagePath(props(host).cover)!);
  assert.notDeepEqual(first, second);
  assert.deepEqual(imageInfo(second), { type: "jpeg", width: 320, height: 200 });
});

test("J11: video sources, URLs and the switch", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.stats = { ...host.stats, video: VIDEO_STATS };
  host.openElsewhere(join(folder({ "v.mkv": "x" }), "v.mkv"));
  await settle(host);
  assert.equal(props(host).mode, "none");
  assert.equal(host.tools.calls.length, 0, "no ffprobe for a video");

  host.stats = { ...host.stats, video: null };
  host.openElsewhere("https://radio.example/stream/live.mp3");
  await settle(host);
  assert.equal(props(host).mode, "audio");
  assert.equal(props(host).title, "live.mp3");
  assert.equal(host.tools.calls.length, 0, "no tools for a URL");

  await host.command("audio-cover").run();
  await host.settle();
  assert.equal(props(host).mode, "none");
  assert.equal(host.osd.at(-1), "Album cover for audio only: Off");
  assert.equal(host.command("audio-cover").state.checked, false);
  // Switched off: a new audio file is not probed.
  host.openElsewhere(join(folder({ "c.mp3": "x" }), "c.mp3"));
  await settle(host);
  assert.equal(host.tools.calls.length, 0);
  // On again: read now.
  await host.command("audio-cover").run();
  await settle(host);
  assert.equal(props(host).mode, "audio");
  assert.equal(host.tools.calls.length, 1);
});

test("J11: a new source aborts the running ffprobe; its late answer is dropped", async () => {
  const host = new TestHost();
  let release!: () => void;
  host.tools.handle("ffprobe", (args) =>
    args.at(-1)!.endsWith("slow.mp3")
      ? new Promise((resolve) => {
          release = () => resolve({ stdout: PROBE_TAGGED });
        })
      : { stdout: PROBE_PLAIN },
  );
  host.tools.handle("ffmpeg", () => ({ stdout: jpeg(10, 10) }));
  await start(host);
  host.openElsewhere(join(folder({ "slow.mp3": "x" }), "slow.mp3"));
  await host.settle(20);
  host.openElsewhere(join(folder({ "fast.mp3": "x" }), "fast.mp3"));
  await settle(host);
  release();
  await settle(host);
  assert.equal(props(host).title, "fast.mp3");
  assert.equal(props(host)["has-cover"], false);
});

test("G-16b: loading shows the loading screen with the source name; ready judges again; empty is idle", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.openElsewhere(join(folder({ "a.mp3": "x" }), "a.mp3"));
  await settle(host);
  assert.equal(props(host).mode, "audio");
  host.openElsewhere(join(folder({ "b.mp3": "x" }), "b.mp3"), "loading");
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).loading], ["loading", "b.mp3"]);
  host.setSource({ phase: "ready" });
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["audio", "b.mp3"]);
  // A video: the backdrop is not shown once it plays; the plugin draws nothing.
  host.stats = { ...host.stats, video: VIDEO_STATS };
  host.openElsewhere("https://cdn.example.com/films/clip.mp4", "loading");
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).loading], ["loading", "cdn.example.com/clip.mp4"]);
  host.setSource({ phase: "ready" });
  await settle(host);
  assert.equal(props(host).mode, "none");
  // The same source loads again (reload): judged again once ready.
  host.setSource({ phase: "loading" });
  await settle(host);
  assert.equal(props(host).mode, "loading");
  host.setSource({ phase: "ready" });
  await settle(host);
  assert.equal(props(host).mode, "none");
  host.setSource({ phase: "empty", source: null });
  await settle(host);
  assert.equal(props(host).mode, "idle");
  // The loading screen does not depend on the idle switch.
  await host.command("idle-logo").run();
  await host.settle();
  assert.equal(props(host).mode, "none");
  assert.equal(host.osd.at(-1), "Logo when idle: Off");
  host.openElsewhere(join(folder({ "c.mp3": "x" }), "c.mp3"), "loading");
  await settle(host);
  assert.equal(props(host).mode, "loading");
});

test("G-16b: an error is the idle screen, never the card of the source before", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  host.openElsewhere(join(folder({ "a.mp3": "x" }), "a.mp3"));
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["audio", "a.mp3"]);
  host.openElsewhere("https://broken.example/x.mp3", "loading");
  await settle(host);
  assert.equal(props(host).mode, "loading");
  host.setSource({ phase: "error" });
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["idle", ""]);
  // Failing without a loading phase first: the same.
  host.openElsewhere(join(folder({ "b.mp3": "x" }), "b.mp3"));
  await settle(host);
  assert.equal(props(host).mode, "audio");
  host.openElsewhere("https://broken.example/y.mp3", "error");
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["idle", ""]);
  // Idle logo switched off: nothing.
  await host.command("idle-logo").run();
  await host.settle();
  host.openElsewhere("https://broken.example/z.mp3", "loading");
  await settle(host);
  host.setSource({ phase: "error" });
  await settle(host);
  assert.equal(props(host).mode, "none");
});

test("generated streams count as video: no card while one runs, none left after a failing open", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_PLAIN }));
  await start(host);
  // A file resolver's stream: the path that was opened, `kind` stream, no mpv video track.
  assert.equal(host.stats.video, null);
  host.openElsewhere(join(folder({ "test.nes": "rom" }), "test.nes"), "loading");
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).loading], ["loading", "test.nes"]);
  host.setSource({ kind: "producer", phase: "ready" });
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["none", ""]);
  assert.equal(host.tools.calls.length, 0, "no ffprobe for a stream");
  // The backdrop comes back while the stream runs: still no card.
  host.setBackdropEffective(true);
  await settle(host);
  assert.equal(props(host).mode, "none");

  // Another file whose resolution fails.
  host.openElsewhere(join(folder({ "bad.nes": "x" }), "bad.nes"), "loading");
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).loading], ["loading", "bad.nes"]);
  host.setSource({ kind: "producer", phase: "error" });
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["idle", ""]);

  // A plain audio file after it still gets its card.
  host.openElsewhere(join(folder({ "song.mp3": "x" }), "song.mp3"));
  await settle(host);
  assert.deepEqual([props(host).mode, props(host).title], ["audio", "song.mp3"]);
});

test("G-16b: loading names: file name, link host and last segment, cut with an ellipsis", () => {
  assert.equal(loadingName("/media/电影.mkv"), "电影.mkv");
  assert.equal(loadingName("https://www.example.com/watch/v/%E8%A7%86%E9%A2%91.mp4?x=1"), "www.example.com/视频.mp4");
  assert.equal(loadingName("https://live.example.com/"), "live.example.com");
  assert.equal(loadingName(null), "");
  const long = loadingName(`/m/${"很长".repeat(40)}.mkv`);
  assert.equal([...long].length, 60);
  assert.ok(long.endsWith("…"));
});

test("H12: custom logo is checked, copied, shown, kept over restarts and reset", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const host = new TestHost();
  const first = await start(host, data);
  const pictures = folder({ "mine.png": png(800, 400), "huge.png": png(5000, 5000), "fake.jpg": "text", "a.gif": "GIF89a" });

  await chooseLogo(host, [join(pictures, "huge.png")]);
  assert.equal(host.osd.at(-1), TOO_LARGE);
  await chooseLogo(host, [join(pictures, "fake.jpg")]);
  assert.equal(host.osd.at(-1), NOT_IMAGE);
  await chooseLogo(host, [join(pictures, "a.gif")]);
  assert.equal(host.osd.at(-1), NOT_IMAGE);
  await chooseLogo(host, null);
  assert.equal(props(host)["custom-logo"], false);

  await chooseLogo(host, [join(pictures, "mine.png")]);
  await host.settle();
  assert.equal(props(host)["custom-logo"], true);
  assert.equal(host.imagePath(props(host).logo), join(data, "logo.png"));
  assert.equal(host.imagePath(host.settingsPage(PAGE_ID).props.logo), join(data, "logo.png"));
  assert.equal(host.command("reset-logo").state.enabled, true);
  unlinkSync(join(pictures, "mine.png"));
  await first.stop();

  // Restart: the copy is still there.
  const again = new TestHost();
  await start(again, data);
  assert.equal(props(again)["custom-logo"], true);
  await resetLogo(again);
  assert.equal(props(again)["custom-logo"], false);
  assert.equal(existsSync(join(data, "logo.png")), false);
  assert.equal(again.osd.at(-1), "Default logo restored");
});

test("H12: an unreadable custom logo falls back once", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  writeFileSync(join(data, "visualizer.json"), JSON.stringify({ audio: { on: true }, idle: { on: true, logo: "logo.jpg" } }));
  writeFileSync(join(data, "logo.jpg"), "broken");
  const host = new TestHost();
  const { stop } = await start(host, data);
  assert.equal(props(host)["custom-logo"], false);
  assert.deepEqual(host.osd, [UNREADABLE]);
  await stop(); // writes the settings now
  assert.equal(JSON.parse(readFileSync(join(data, "visualizer.json"), "utf8")).idle.logo, null);
});

test("settings page: switches, cache line and clearing", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_TAGGED }));
  host.tools.handle("ffmpeg", () => ({ stdout: jpeg(100, 100) }));
  await start(host);
  const page = host.settingsPage(PAGE_ID);
  assert.deepEqual([page.props["cache-available"], page.props["cache-entries"]], [true, 0]);
  page.emit("setAudio", false);
  await host.settle();
  assert.equal(host.command("audio-cover").state.checked, false);
  assert.equal(page.props["audio-on"], false);
  page.emit("setAudio", true);
  page.emit("setIdle", false);
  await host.settle();
  assert.equal(host.command("idle-logo").state.checked, false);
  host.openElsewhere(join(folder({ "s.mp3": "x" }), "s.mp3"));
  await settle(host);
  assert.equal(page.props["cache-entries"], 1);
  assert.match(String(page.props["cache-size"]), /^\d+(\.\d)? [KM]B$/);
  assert.equal(page.props["can-clear"], true);
  page.emit("clearCache");
  await settle(host);
  assert.equal(page.props["cache-entries"], 0);
  assert.equal(host.osd.at(-1), "Cover cache cleared");
});

test("cache: least recently used dropped beyond the size limit; orphans removed on load", async () => {
  const data = mkdtempSync(join(scratch, "cache-"));
  const host = new TestHost();
  host.tools.handle("ffprobe", () => ({ stdout: PROBE_TAGGED }));
  host.tools.handle("ffmpeg", () => ({ stdout: jpeg(100, 100) }));
  const { plugin } = await start(host, data);
  const cache = new CoverCache(data, plugin.tools);
  cache.load();
  const size = jpeg(100, 100).length;
  const saved = limits.cacheBytes;
  limits.cacheBytes = size * 2;
  try {
    for (const name of ["1.mp3", "2.mp3", "3.mp3"]) {
      await cache.lookup(join(folder({ [name]: name }), name), new AbortController().signal);
      await new Promise((r) => setTimeout(r, 5));
    }
  } finally {
    limits.cacheBytes = saved;
  }
  await cache.flush();
  assert.equal(cache.usage.entries, 2);
  writeFileSync(join(cache.directory, "stray.jpg"), "x");
  const reloaded = new CoverCache(data, plugin.tools);
  reloaded.load();
  assert.equal(reloaded.usage.entries, 2);
  assert.equal(existsSync(join(cache.directory, "stray.jpg")), false);
});

test("a failing ffprobe is not cached; the folder cover still shows", async () => {
  const host = new TestHost();
  host.tools.handle("ffprobe", () => {
    throw new HostError("failed", "Invalid data found when processing input");
  });
  await start(host);
  const dir = folder({ "x.mp3": "x", "front.jpg": jpeg(50, 50) });
  host.openElsewhere(join(dir, "x.mp3"));
  await settle(host);
  assert.equal(props(host)["has-cover"], true);
  host.openElsewhere(join(dir, "x.mp3"));
  await settle(host);
  assert.equal(host.tools.calls.filter((c) => c.tool === "ffprobe").length, 2);
});

test("Simplified Chinese: menus, settings page and messages from locales/zh-Hans.po", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  writeFileSync(join(data, "visualizer.json"), JSON.stringify({ audio: { on: true }, idle: { on: true, logo: "logo.jpg" } }));
  writeFileSync(join(data, "logo.jpg"), "broken");
  const host = new TestHost();
  const { plugin } = await start(host, data, "zh-Hans");
  assert.equal(plugin.i18n.locale, "zh-Hans");
  assert.deepEqual(host.osd, ["徽标图片无法读取，已使用默认徽标"]);
  assert.equal(host.settingsPage(PAGE_ID).title, "徽标与专辑封面");
  // Menu sweep (B5): palette only, so the title and the category are what a Chinese user reads.
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.display.title),
    ["纯音频时显示专辑封面", "空闲时显示徽标", "选择徽标图片…", "空闲徽标"],
  );
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.display.category),
    ["声音", "外观", "外观", "复位"],
  );
  // The player translates the reason with the plugin's catalog when it shows it.
  assert.equal(host.translate(host.command("reset-logo").state.reason!), "正在使用默认徽标");
  await host.command("audio-cover").run();
  await host.command("idle-logo").run();
  await host.settle();
  assert.deepEqual(host.osd.slice(1), ["纯音频时显示专辑封面：关", "空闲时显示徽标：关"]);
  const pictures = folder({ "huge.png": png(5000, 5000), "a.gif": "GIF89a", "ok.png": png(10, 10) });
  await chooseLogo(host, [join(pictures, "huge.png")]);
  await chooseLogo(host, [join(pictures, "a.gif")]);
  await chooseLogo(host, [join(pictures, "ok.png")]);
  await resetLogo(host);
  assert.deepEqual(host.osd.slice(3), ["图片过大（最大 4096×4096、16 MB）", "只能使用 PNG 或 JPEG 图片", "已使用自定义徽标", "已恢复默认徽标"]);
  // The window texts are @tr in ui/*.slint; the same catalog translates them.
  assert.equal(host.translate("Opening: {}", ["a.mp3"]), "正在打开：a.mp3");
  assert.equal(host.translate("Cover cache: empty"), "封面缓存：空");
});

test("palette: every command has a category in every language (player 0.7.6)", async () => {
  for (const locale of ["en", "zh-Hans", "ja", "ko", "de", "fr", "es"] as const) {
    const { host, stop } = await startPlugin(activate, { directory, host: new TestHost(), locale });
    assert.equal(host.commands.size, 4);
    for (const [id, { display }] of host.commands) {
      const category = display.category;
      assert.ok(typeof category === "string" && category.trim() !== "", `${locale}: ${id} has no category`);
      // The CJK catalogs translate every category (the European ones may keep "Audio").
      if (locale === "zh-Hans" || locale === "ja" || locale === "ko") assert.match(category, /[^\x20-\x7e]/, `${locale}: ${id} is not translated`);
    }
    await stop();
  }
});
