import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { cut, filePage, hwdec, pageText, sampleFormat, saveName, systemPage, VALUE_MAX } from "../properties.ts";
import type { Probe, Row } from "../properties.ts";
import { displayLine, systemVersion } from "../sysinfo.ts";

const directory = new URL("..", import.meta.url);
const stats = {
  backend: "libmpv · 软件渲染",
  session: 3,
  framesShown: 120,
  framesDropped: 1,
  video: { codec: "H.264 / AVC", width: 1920, height: 1080, fps: 23.976, decoder: "h264", pixelFormat: "yuv420p", colorspace: "bt.709", displayWidth: 1920, displayHeight: 800, displayFps: null, bitrate: 4_500_000, hwdec: "videotoolbox", transfer: "bt.1886" },
  audio: { codec: "AAC (Advanced Audio Coding)", sampleRate: 48000, channels: 2, decoder: "aac", format: "floatp", channelLayout: "stereo", bitrate: 128_000 },
  avSyncMs: -3.25,
  cacheSeconds: 12.4,
};
const probe: Probe = {
  format: { format_name: "matroska,webm", format_long_name: "Matroska / WebM", duration: "5400.5", bit_rate: "4700000", tags: { title: "示例影片", ARTIST: "某人", custom: "x" } },
  streams: [
    { index: 0, codec_type: "video", codec_name: "h264", codec_long_name: "H.264 / AVC", profile: "High", width: 1920, height: 1080, pix_fmt: "yuv420p", color_space: "bt709", avg_frame_rate: "24000/1001", disposition: { default: 1, forced: 0 } },
    { index: 1, codec_type: "audio", codec_name: "aac", sample_rate: "48000", channels: 6, channel_layout: "5.1(side)", sample_fmt: "fltp", tags: { language: "jpn", title: "日语" }, disposition: { default: 1 } },
    { index: 2, codec_type: "subtitle", codec_name: "ass", tags: { language: "chi" }, disposition: { forced: 1 } },
    { index: 3, codec_type: "attachment", tags: { filename: "font.ttf", mimetype: "font/ttf" } },
    { index: 4, codec_type: "video", codec_name: "mjpeg", width: 600, height: 600, disposition: { attached_pic: 1 } },
  ],
  chapters: [{}, {}, {}],
};

// The System page as the player reports it (`plugin.app.system()`): one macOS and one Windows machine.
const mac = {
  os: "macos" as const, name: "macOS", version: "15.2", build: "24C101", model: "Mac14,2", arch: "aarch64",
  displays: [{ name: "Color LCD", width: 2940, height: 1912, scale: 2, refreshRate: 60, primary: true }],
};
const windows = {
  os: "windows" as const, name: "Windows 11", version: "23H2", build: "22631.4460", model: "Surface Laptop 5", arch: "x86_64",
  displays: [
    { name: null, width: 2256, height: 1504, scale: 1.5, refreshRate: 59.94, primary: true },
    { name: "DELL U2723QE", width: 3840, height: 2160, scale: 1, refreshRate: null, primary: false },
  ],
};

const tick = () => new Promise((done) => setTimeout(done, 5));
async function until(predicate: () => boolean) {
  for (let i = 0; i < 600; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("timed out waiting for condition");
}
type Shown = { label: string; value: string; heading: boolean };
const rows = (host: TestHost) => host.window("properties").props.rows as Shown[];
const value = (host: TestHost, label: string) => rows(host).find((r) => r.label === label && !r.heading)?.value;
const headings = (host: TestHost) => rows(host).filter((r) => r.heading).map((r) => r.label);

async function start(source: string | null = "/videos/movie.mkv", dataDirectory?: string, app?: TestHost["app"], locale?: "zh-Hans") {
  const host = new TestHost();
  if (app) host.app = app;
  host.systemInfo = structuredClone(mac);
  host.stats = stats;
  if (source) {
    host.openElsewhere(source);
    host.setSource({ duration: 5400, position: 83 });
  }
  const plugin = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}), ...(locale ? { locale } : {}) });
  const stop = async () => {
    running.delete(stop);
    await plugin.stop();
  };
  running.add(stop);
  return { host, stop };
}
/** A failed test still stops its plugin (the 1-second refresh would keep the run alive). */
const running = new Set<() => Promise<void>>();
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

test("I05: top-level Playback Properties… (811, windows, Ctrl+F1) opens the window; a second Ctrl+F1 keeps it open", async () => {
  const { host, stop } = await start();
  assert.deepEqual(host.command("info.properties").declaration, {
    title: "Playback Properties…", path: ["Playback Properties…"], order: 811, group: "windows", radio: false, bindings: ["Ctrl+F1"], focus: null, repeat: false,
  });
  assert.deepEqual(host.conflicts, []);
  await host.press("Ctrl+F1");
  await host.settle();
  const window = host.window("properties");
  assert.equal(window.file, "ui/properties.slint");
  const slint = await readFile(new URL("../ui/properties.slint", import.meta.url), "utf8");
  const declared = [...slint.slice(slint.indexOf("export component")).matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
  assert.deepEqual([...window.callbacks].sort(), declared.sort(), "exactly the Slint callbacks");
  assert.doesNotMatch(slint, /#[0-9a-f]{3,8}\b/i, "colours come from Theme");
  assert.doesNotMatch(slint, /font-size:\s*\d/, "font sizes come from Theme");
  await host.press("Ctrl+F1");
  await host.settle();
  assert.equal(host.window("properties").visible, true, "brought to the front, not closed");
  window.emit("close-window");
  await host.settle();
  assert.throws(() => host.window("properties"), "Close closes the window");
  await stop();
});

test("I05 hwdec: the decoder in use, Not used for software, unknown when not reported", () => {
  assert.equal(hwdec("videotoolbox"), "videotoolbox");
  assert.equal(hwdec("d3d11va"), "d3d11va");
  assert.equal(hwdec("no"), "Not used");
  assert.equal(hwdec(null), null);
  assert.equal(hwdec(undefined), null);
});

test("I05 Playback: the extended stats, hardware decoding as reported, refreshed every second", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  assert.equal(host.window("properties").props["tab-index"], 0);
  assert.equal(host.window("properties").props["can-refresh"], false);
  assert.deepEqual(headings(host), ["Video", "Audio", "Other"]);
  assert.equal(value(host, "Decoder"), "h264");
  assert.equal(value(host, "Hardware decoding"), "videotoolbox");
  assert.equal(value(host, "Input format"), "H.264 / AVC · yuv420p · bt.709");
  assert.equal(value(host, "Size"), "1920×1080 → 1920×800");
  assert.equal(value(host, "Source frame rate"), "23.976 fps");
  assert.equal(value(host, "Display frame rate"), "-");
  assert.equal(value(host, "Video bitrate"), "4500 kbps");
  assert.equal(value(host, "Sample format"), "32-bit float (floatp)");
  assert.equal(value(host, "Channels"), "2 (stereo)");
  assert.equal(value(host, "A/V sync offset"), "-3.3 ms");
  assert.equal(value(host, "Cache"), "12.4 s");
  host.stats = { ...stats, framesShown: 500, video: { ...stats.video, hwdec: "no" } };
  await host.settle(1100);
  assert.equal(value(host, "Frames shown"), "500");
  assert.equal(value(host, "Hardware decoding"), "Not used", "software decoding after a fallback");
  await stop();
});

test("I05: without media Playback and File say No media open; System still shows", async () => {
  const { host, stop } = await start(null);
  await host.command("info.properties").run();
  await host.settle();
  assert.deepEqual(rows(host), [{ label: "", value: "No media open", heading: false }]);
  const window = host.window("properties");
  window.emit("tab-changed", 1);
  await host.settle();
  assert.deepEqual(rows(host), [{ label: "", value: "No media open", heading: false }]);
  assert.equal(host.tools.calls.length, 0);
  window.emit("tab-changed", 2);
  await until(() => value(host, "System version") !== undefined);
  assert.deepEqual(headings(host), ["Player", "System"]);
  await stop();
});

test("I05 File: ffprobe through plugin.tools; streams, chapters, tags and embedded resources", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const path = join(media, "movie.mkv");
  await writeFile(path, Buffer.alloc(2048));
  const { host, stop } = await start(path);
  host.tools.handle("ffprobe", () => ({ stdout: JSON.stringify(probe) }));
  try {
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => value(host, "Path") === path);
    assert.deepEqual(host.tools.calls, [{ tool: "ffprobe", args: ["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json", path] }]);
    assert.equal(host.window("properties").props["can-refresh"], true);
    assert.deepEqual(headings(host), ["Source", "Container", "Stream #0: Video", "Stream #1: Audio", "Stream #2: Subtitles", "Metadata", "Embedded resources"]);
    assert.equal(value(host, "File size"), "2.00 KB (2,048 bytes)");
    assert.equal(value(host, "Format"), "Matroska / WebM (matroska,webm)");
    assert.equal(value(host, "Duration"), "1:30:00");
    assert.equal(value(host, "Chapters"), "3");
    assert.equal(value(host, "Attachment #3"), "font.ttf · font/ttf");
    assert.equal(value(host, "Cover #4"), "mjpeg · 600×600");
    assert.equal(value(host, "Artist"), "某人");
    assert.match(value(host, "Modified")!, /^\d{1,2}\/\d{1,2}\/\d{4}, \d{2}:\d{2}:\d{2}$/, "dates in the interface language (English)");
    const text = pageText(rows(host) as Row[]);
    assert.match(text, /\n\nStream #1: Audio\nCodec: aac\nLanguage: jpn\nTitle: 日语\nFlags: Default\nSample rate: 48000 Hz\nChannels: 6 \(5\.1\(side\)\)\nSample format: 32-bit float \(fltp\)\n/);
    assert.match(text, /Stream #0: Video\n[^]*Frame rate: 23\.976 fps\n/);
    assert.match(text, /Stream #2: Subtitles\nCodec: ass\nLanguage: chi\nTitle: -\nFlags: Forced\n/);
    // Read once per source; Refresh reads again.
    host.window("properties").emit("tab-changed", 0);
    host.window("properties").emit("tab-changed", 1);
    await host.settle();
    assert.equal(host.tools.calls.length, 1);
    host.window("properties").emit("refresh");
    await until(() => host.tools.calls.length === 2);
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("I05 File: a missing ffprobe, a failure; a source change aborts the old read and reads the new source", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const [a, b] = [join(media, "a.mkv"), join(media, "b.mkv")];
  await writeFile(a, "a");
  await writeFile(b, "b");
  const { host, stop } = await start(a);
  try {
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => rows(host)[0]?.value === "Cannot read file information (ffprobe not found)");
    host.tools.handle("ffprobe", () => {
      throw new HostError("failed", "Invalid data found when processing input");
    });
    host.window("properties").emit("refresh");
    await until(() => rows(host)[0]?.value === "Cannot read file information: Invalid data found when processing input");
    let release!: () => void;
    const held = new Promise<void>((done) => (release = done));
    host.tools.handle("ffprobe", async (args) => {
      if (args.at(-1) === a) await held;
      return { stdout: JSON.stringify({ format: { format_name: args.at(-1) === a ? "old" : "new" } }) };
    });
    const before = host.tools.calls.length;
    host.window("properties").emit("refresh");
    await until(() => host.tools.calls.length === before + 1);
    assert.equal(rows(host)[0]?.value, "Reading…");
    host.openElsewhere(b);
    await until(() => value(host, "Format") === "new");
    release();
    await host.settle(50);
    assert.equal(value(host, "Format"), "new", "the aborted read of a never shows");
    assert.equal(value(host, "Path"), b);
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("I05 File: a network source shows what the player reports, without ffprobe", async () => {
  const { host, stop } = await start("https://cdn.test/live/stream.m3u8");
  host.tools.handle("ffprobe", () => ({ stdout: "{}" }));
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 1);
  await host.settle();
  assert.equal(rows(host)[0]!.value, "Network source: only what the player reports");
  assert.equal(value(host, "Address"), "https://cdn.test/live/stream.m3u8");
  assert.equal(value(host, "Video"), "H.264 / AVC · 1920×1080 · 23.976 fps");
  assert.equal(host.tools.calls.length, 0);
  await stop();
});

test("I05 System: system name, model and displays from plugin.app.system(); CPU and memory from os; versions from plugin.app", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 2);
  await until(() => value(host, "System version") === "macOS 15.2 (24C101)");
  assert.equal(value(host, "Vestra Player version"), "0.1.0 (build test)");
  assert.equal(value(host, "Plugin API version"), "vestra/1");
  assert.equal(value(host, "mpv version"), "mpv 0.41.0");
  assert.equal(value(host, "FFmpeg version"), "7.1");
  assert.equal(value(host, "Model"), "Mac14,2");
  assert.match(value(host, "CPU")!, /^.+ \(\d+ cores\)$/);
  assert.match(value(host, "Total memory")!, /^\d+\.\d GB$/);
  assert.match(value(host, "Free memory")!, /^\d+\.\d GB$/);
  assert.equal(value(host, "Display"), "Color LCD · 2940 x 1912 · 1470 x 956 @ 60 Hz");
  assert.equal(host.sent("app.system").length, 1, "read once per opening");
  assert.equal(host.tools.calls.length, 0, "no external programs");
  await stop();
});

test("I05 System: a Windows machine; several displays are numbered; a failed read leaves the system rows unknown", async () => {
  const { host, stop } = await start(null);
  host.systemInfo = structuredClone(windows);
  await host.command("info.properties").run();
  const window = host.window("properties");
  window.emit("tab-changed", 2);
  await until(() => value(host, "System version") === "Windows 11 23H2 (22631.4460)");
  assert.equal(value(host, "Model"), "Surface Laptop 5");
  assert.equal(value(host, "Display"), undefined);
  assert.equal(value(host, "Display 1"), "2256 x 1504 · 1504 x 1003 @ 59.94 Hz");
  assert.equal(value(host, "Display 2"), "DELL U2723QE · 3840 x 2160 · 3840 x 2160");
  host.handle("app.system", () => {
    throw new HostError("failed", "registry unreadable");
  });
  window.emit("refresh");
  await until(() => host.sent("app.system").length === 2 && value(host, "System version") === "-");
  assert.equal(value(host, "Model"), "-");
  assert.equal(value(host, "Display"), "-");
  assert.match(value(host, "CPU")!, /cores\)$/, "CPU still from os");
  await stop();
});

test("I05: Copy puts the current page's text on the clipboard; a refusal shows Copy failed", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  const window = host.window("properties");
  window.emit("copy-page");
  await until(() => host.osd.at(-1) === "Playback info copied");
  assert.equal(host.clipboard.text, pageText(rows(host) as Row[]));
  assert.match(host.clipboard.text!, /^Video\nDecoder: h264\nHardware decoding: videotoolbox\n/);
  assert.match(host.clipboard.text!, /\n\nAudio\n/);
  host.handle("clipboard.writeText", () => {
    throw new HostError("failed", "refused");
  });
  window.emit("copy-page");
  await until(() => window.props.status === "copy-failed");
  await stop();
});

test("I05: Save As… writes the page as <name> info.txt; cancelling writes nothing; a failed write shows Save failed", async () => {
  const folder = await mkdtemp(join(tmpdir(), "info-save-"));
  const { host, stop } = await start();
  try {
    await host.command("info.properties").run();
    await host.settle();
    const window = host.window("properties");
    host.dialogAnswers.push(null);
    window.emit("save-page");
    await host.settle();
    assert.equal(host.sent("dialogs.open").at(-1)!.fileName, "movie info.txt");
    assert.equal(host.sent("dialogs.open").at(-1)!.title, "Save As");
    const target = join(folder, "movie info.txt");
    host.dialogAnswers.push([target]);
    window.emit("save-page");
    await until(() => host.osd.at(-1) === "Saved to movie info.txt");
    assert.equal(await readFile(target, "utf8"), pageText(rows(host) as Row[]));
    assert.deepEqual(await readdir(folder), ["movie info.txt"], "no temporary file left");
    host.dialogAnswers.push([join(folder, "missing", "x.txt")]);
    window.emit("save-page");
    await until(() => window.props.status === "save-failed");
    assert.match(String(window.props["status-detail"]), /ENOENT/);
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("I05: the last page is remembered in info.json", async () => {
  const data = await mkdtemp(join(tmpdir(), "info-data-"));
  try {
    let { host, stop } = await start(null, data);
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 2);
    await host.settle();
    await stop();
    assert.deepEqual(JSON.parse(await readFile(join(data, "info.json"), "utf8")), { propertiesTab: 2 });
    ({ host, stop } = await start(null, data));
    await host.command("info.properties").run();
    await host.settle();
    assert.equal(host.window("properties").props["tab-index"], 2);
    await stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("I05 pure rules: values cut at 512, sample formats, save names, displays, empty groups left out", () => {
  assert.equal([...cut("a".repeat(600))].length, VALUE_MAX);
  assert.ok(cut("a".repeat(600)).endsWith("…"));
  assert.equal(sampleFormat("s16"), "16-bit (s16)");
  assert.equal(sampleFormat("dblp"), "64-bit float (dblp)");
  assert.equal(sampleFormat("weird"), "weird");
  assert.equal(saveName("/v/My Film.mkv"), "My Film info.txt");
  assert.equal(saveName("https://cdn.test/v/%E8%A7%86%E9%A2%91.mp4?x=1"), "视频 info.txt");
  assert.equal(saveName(null), "Vestra Player info.txt");
  assert.equal(systemVersion({ name: "macOS", version: "26.2", build: "25C56" }), "macOS 26.2 (25C56)");
  assert.equal(systemVersion({ name: "Linux", version: null, build: null }), "Linux");
  assert.equal(displayLine({ name: "Built-in Retina Display", width: 3024, height: 1964, scale: 2, refreshRate: 120, primary: true }), "Built-in Retina Display · 3024 x 1964 · 1512 x 982 @ 120 Hz");
  const page = filePage("/a.mkv", null, {});
  assert.deepEqual(page.filter((r) => r.heading).map((r) => r.label), ["Source", "Container"]);
  assert.equal(pageText([{ label: "A", value: "", heading: true }, { label: "k", value: "v", heading: false }, { label: "B", value: "", heading: true }, { label: "", value: "m", heading: false }]), "A\nk: v\n\nB\nm\n");
  const dated = filePage("/a.mkv", { size: 1, mtime: new Date(2026, 9, 25, 13, 5, 9) }, {}, "zh-Hans");
  assert.equal(dated.find((r) => r.label === "Modified")?.value, "2026/10/25 13:05:09", "dates in the interface language");
});

test("I05 System: versions the core does not know show Unknown; no build hash, extra libav libraries", () => {
  const page = systemPage({ version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "1.16.1" } }, null);
  const row = (label: string) => page.find((r) => r.label === label)?.value;
  assert.equal(row("Vestra Player version"), "0.2.0");
  assert.equal(row("mpv version"), "Unknown");
  assert.equal(row("FFmpeg version"), "Unknown");
  const more = systemPage(
    { version: "0.2.0", build: "abc1234", components: { mpv: "mpv v0.41.0", libav: { ffmpeg: "9.0.2", libavcodec: "62.11.100" }, slint: "1.16.1" } },
    null,
  );
  assert.equal(more.find((r) => r.label === "Vestra Player version")?.value, "0.2.0 (build abc1234)");
  assert.equal(more.find((r) => r.label === "FFmpeg version")?.value, "9.0.2 · libavcodec 62.11.100");
});

test("I05: closing the window with the title bar stops the 1-second polling (onClose); reopening starts it again", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle(1100);
  const polled = () => host.sent("playback.stats").length;
  assert.ok(polled() >= 2, "polling while open");
  host.window("properties").userClose();
  await host.settle();
  const after = polled();
  await host.settle(2200);
  assert.equal(polled(), after, "no stats reads after a title-bar close");
  await host.press("Ctrl+F1");
  await host.settle(1100);
  assert.equal(host.window("properties").visible, true);
  assert.ok(polled() >= after + 2, "polling again after reopening");
  await stop();
});

test("I05 in Simplified Chinese: the menu, the window's words and the messages of before", async () => {
  const folder = await mkdtemp(join(tmpdir(), "info-save-"));
  const { host, stop } = await start("/videos/movie.mkv", undefined, undefined, "zh-Hans");
  try {
    assert.deepEqual(host.command("info.properties").display.path, ["播放属性…"]);
    // What properties.slint shows through @tr.
    const words = ["Playback Properties", "Playback", "File", "System", "Refresh", "Copy", "Save As…", "Close", "Copy failed", "Save As"];
    assert.deepEqual(words.map((w) => host.translate(w)), ["播放属性", "播放信息", "文件信息", "系统信息", "刷新", "复制到剪贴板", "保存为…", "关闭", "复制失败", "保存为"]);
    assert.equal(host.translate("Save failed: {}", ["EACCES"]), "保存失败：EACCES");
    await host.command("info.properties").run();
    await host.settle();
    const window = host.window("properties");
    window.emit("copy-page");
    await until(() => host.osd.at(-1) === "已复制播放信息");
    host.dialogAnswers.push([join(folder, "movie info.txt")]);
    window.emit("save-page");
    await until(() => host.osd.at(-1) === "已保存到 movie info.txt");
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});
