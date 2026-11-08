import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { columns, cut, filePage, hwdec, pageText, sampleFormat, saveName, systemPage, VALUE_MAX, widestLabel } from "../properties.ts";
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

async function start(source: string | null = "/videos/movie.mkv", dataDirectory?: string, app?: TestHost["app"], locale: "en" | "zh-Hans" = "zh-Hans") {
  const host = new TestHost();
  if (app) host.app = app;
  host.systemInfo = structuredClone(mac);
  host.stats = stats;
  if (source) {
    host.openElsewhere(source);
    host.setSource({ duration: 5400, position: 83 });
  }
  const plugin = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await plugin.stop();
  };
  running.add(stop);
  const { i18n } = plugin.plugin;
  /** A row (or a pure function's value) as the player shows it. */
  const show = (shown: Row["value"] | null) => (shown === null ? null : i18n.resolve(typeof shown === "number" ? String(shown) : shown));
  return { host, i18n, show, stop };
}
/** A failed test still stops its plugin (the 1-second refresh would keep the run alive). */
const running = new Set<() => Promise<void>>();
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

test("I05: top-level Playback Properties… (811, windows, Ctrl+F1) opens the window; a second Ctrl+F1 keeps it open", async () => {
  const { host, stop } = await start();
  assert.deepEqual(host.command("info.properties").declaration, {
    title: "Playback Properties…", path: [], order: 811, group: "windows", radio: false, bindings: ["Ctrl+F1"], focus: null, repeat: false, category: { $vestraText: "Info" },
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

test("I05 hwdec: the decoder in use, 未使用 for software, unknown when not reported", async () => {
  const { i18n, show, stop } = await start(null);
  assert.equal(show(hwdec(i18n, "videotoolbox")), "videotoolbox");
  assert.equal(show(hwdec(i18n, "d3d11va")), "d3d11va");
  assert.equal(show(hwdec(i18n, "no")), "未使用");
  assert.equal(hwdec(i18n, null), null);
  assert.equal(hwdec(i18n, undefined), null);
  await stop();
});

test("I05 播放信息: the extended stats, hardware decoding as reported, refreshed every second", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  assert.equal(host.window("properties").props["tab-index"], 0);
  assert.equal(host.window("properties").props["can-refresh"], false);
  assert.deepEqual(headings(host), ["视频", "音频", "其它"]);
  assert.equal(value(host, "解码器"), "h264");
  assert.equal(value(host, "硬件解码"), "videotoolbox");
  assert.equal(value(host, "输入格式"), "H.264 / AVC · yuv420p · bt.709");
  assert.equal(value(host, "尺寸"), "1920×1080 → 1920×800");
  assert.equal(value(host, "源帧率"), "23.976 fps");
  assert.equal(value(host, "当前帧率"), "-");
  assert.equal(value(host, "视频码率"), "4500 kbps");
  assert.equal(value(host, "采样位数"), "32 位浮点（floatp）");
  assert.equal(value(host, "声道数"), "2（stereo）");
  assert.equal(value(host, "音画同步偏差"), "-3.3 ms");
  assert.equal(value(host, "缓冲时长"), "12.4 秒");
  host.stats = { ...stats, framesShown: 500, video: { ...stats.video, hwdec: "no" } };
  await host.settle(1100);
  assert.equal(value(host, "已显示帧"), "500");
  assert.equal(value(host, "硬件解码"), "未使用", "software decoding after a fallback");
  await stop();
});

test("I05 Playback in English", async () => {
  const { host, stop } = await start("/videos/movie.mkv", undefined, undefined, "en");
  await host.command("info.properties").run();
  await host.settle();
  assert.deepEqual(headings(host), ["Video", "Audio", "Other"]);
  assert.equal(value(host, "Hardware decoding"), "videotoolbox");
  assert.equal(value(host, "Bit depth"), "32-bit float (floatp)");
  assert.equal(value(host, "Channel count"), "2 (stereo)");
  assert.equal(value(host, "Cache"), "12.4 s");
  host.stats = { ...stats, video: { ...stats.video, hwdec: "no" } };
  await host.settle(1100);
  assert.equal(value(host, "Hardware decoding"), "Not used");
  await stop();
});

test("I05: the label column is sized by the page's widest label (label-sample), one per page", async () => {
  const { host, stop } = await start("/videos/movie.mkv", undefined, undefined, "en");
  await host.command("info.properties").run();
  await host.settle();
  const sample = () => host.window("properties").props["label-sample"];
  assert.equal(sample(), "Display frame rate");
  host.window("properties").emit("tab-changed", 2);
  await host.settle();
  assert.equal(sample(), "Vestra Player version");
  await stop();
  const zh = await start("/videos/movie.mkv");
  await zh.host.command("info.properties").run();
  await zh.host.settle();
  // In the player's language: the widest label of 播放信息 in Simplified Chinese.
  const shown = rows(zh.host).filter((r) => !r.heading && r.label !== "");
  const most = Math.max(...shown.map((r) => columns(r.label)));
  assert.equal(columns(zh.host.window("properties").props["label-sample"] as string), most);
  await zh.stop();
});

test("I05 widestLabel: wide characters count two columns; headings and messages are left out; none = empty", async () => {
  const { i18n, stop } = await start(null);
  assert.equal(columns("Size"), 4);
  assert.equal(columns("尺寸"), 4);
  assert.equal(columns("ｓｉｚｅ"), 8);
  assert.equal(columns("크기"), 4);
  const row = (label: string, heading = false): Row => ({ label, value: "v", heading });
  assert.equal(widestLabel(i18n, [row("A very long heading", true), row("Codec"), row("尺寸大小"), row("")]), "尺寸大小");
  assert.equal(widestLabel(i18n, [row("Heading", true), row("")]), "");
  assert.equal(widestLabel(i18n, []), "");
  await stop();
});

test("I05: lists are Slint ListViews; no hidden layout of every row; tabs are the library's", async () => {
  const slint = await readFile(new URL("../ui/properties.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{[^}]*\bListView\b[^}]*\} from "std-widgets\.slint"/);
  assert.match(slint, /import \{[^}]*\bTabs\b[^}]*\} from "@vestra\/widgets\.slint"/);
  assert.match(slint, /ListView \{[\s\S]*for row\[index\] in root\.rows:/);
  assert.doesNotMatch(slint, /ScrollView/);
  assert.doesNotMatch(slint, /component Tab\b/, "no local tab");
  assert.equal([...slint.matchAll(/for row\b/g)].length, 1, "the rows are instantiated by the list only");
  assert.match(slint, /label-measure := Text \{ text: root\.label-sample;/);
  const about = await readFile(new URL("../ui/about.slint", import.meta.url), "utf8");
  assert.match(about, /ListView \{[\s\S]*for plugin in root\.plugins: PluginLine/);
  assert.doesNotMatch(about, /ScrollView/);
});

test("I05: without media 播放信息 and 文件信息 say 未打开媒体; 系统信息 still shows", async () => {
  const { host, stop } = await start(null);
  await host.command("info.properties").run();
  await host.settle();
  assert.deepEqual(rows(host), [{ label: "", value: "未打开媒体", heading: false }]);
  const window = host.window("properties");
  window.emit("tab-changed", 1);
  await host.settle();
  assert.deepEqual(rows(host), [{ label: "", value: "未打开媒体", heading: false }]);
  assert.equal(host.tools.calls.length, 0);
  window.emit("tab-changed", 2);
  await until(() => value(host, "系统版本") !== undefined);
  assert.deepEqual(headings(host), ["播放器", "系统"]);
  await stop();
});

test("I05 文件信息: ffprobe through plugin.tools; streams, chapters, tags and embedded resources", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const path = join(media, "movie.mkv");
  await writeFile(path, Buffer.alloc(2048));
  const { host, i18n, stop } = await start(path);
  host.tools.handle("ffprobe", () => ({ stdout: JSON.stringify(probe) }));
  try {
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => value(host, "路径") === path);
    assert.deepEqual(host.tools.calls, [{ tool: "ffprobe", args: ["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json", path] }]);
    assert.equal(host.window("properties").props["can-refresh"], true);
    assert.deepEqual(headings(host), ["来源", "容器", "流 #0：视频", "流 #1：音频", "流 #2：字幕", "元数据", "内嵌资源"]);
    assert.equal(value(host, "文件大小"), "2.00 KB（2,048 字节）");
    assert.equal(value(host, "格式"), "Matroska / WebM（matroska,webm）");
    assert.equal(value(host, "总时长"), "1:30:00");
    assert.equal(value(host, "流数量"), "5");
    assert.equal(value(host, "章节数"), "3");
    assert.equal(value(host, "附件 #3"), "font.ttf · font/ttf");
    assert.equal(value(host, "封面 #4"), "mjpeg · 600×600");
    assert.equal(value(host, "艺术家"), "某人");
    assert.equal(value(host, "custom"), "x", "an unknown tag keeps its name");
    assert.match(value(host, "修改时间")!, /^\d{4}\/\d{1,2}\/\d{1,2} \d{2}:\d{2}:\d{2}$/, "dates in the interface language");
    const text = pageText(i18n, rows(host) as Row[]);
    assert.match(text, /\n\n流 #1：音频\n编码：aac\n语言：jpn\n标题：日语\n标记：默认\n采样率：48000 Hz\n声道：6（5\.1\(side\)）\n采样格式：32 位浮点（fltp）\n/);
    assert.match(text, /流 #0：视频\n[^]*帧率：23\.976 fps\n/);
    assert.match(text, /流 #2：字幕\n编码：ass\n语言：chi\n标题：-\n标记：强制\n/);
    // Read once per source; 刷新 reads again.
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

test("I05 File in English", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const path = join(media, "movie.mkv");
  await writeFile(path, Buffer.alloc(2048));
  const { host, i18n, stop } = await start(path, undefined, undefined, "en");
  host.tools.handle("ffprobe", () => ({ stdout: JSON.stringify({ ...probe, streams: [...probe.streams!, { index: 5, codec_type: "attachment", tags: {} }] }) }));
  try {
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => value(host, "Path") === path);
    assert.deepEqual(headings(host), ["Source", "Container", "Stream #0: Video", "Stream #1: Audio", "Stream #2: Subtitles", "Metadata", "Embedded resources"]);
    assert.equal(value(host, "File size"), "2.00 KB (2,048 bytes)");
    assert.equal(value(host, "Attachment #5"), "(no file name)");
    assert.match(value(host, "Modified")!, /^\d{1,2}\/\d{1,2}\/\d{4}, \d{2}:\d{2}:\d{2}$/, "dates in the interface language (English)");
    const text = pageText(i18n, rows(host) as Row[]);
    assert.match(text, /\n\nStream #1: Audio\nCodec: aac\nLanguage: jpn\nTitle: 日语\nFlags: Default\nSample rate: 48000 Hz\nChannels: 6 \(5\.1\(side\)\)\nSample format: 32-bit float \(fltp\)\n/);
    assert.match(text, /Stream #2: Subtitles\nCodec: ass\nLanguage: chi\nTitle: -\nFlags: Forced\n/);
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("I05 文件信息: a missing file, a missing ffprobe, a failure; a source change aborts the old read and reads the new source", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const [a, b] = [join(media, "a.mkv"), join(media, "b.mkv")];
  await writeFile(a, "a");
  await writeFile(b, "b");
  const { host, stop } = await start(a);
  try {
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => rows(host)[0]?.value === "无法读取文件信息（未找到 ffprobe）");
    host.tools.handle("ffprobe", () => {
      throw new HostError("failed", "Invalid data found when processing input");
    });
    host.window("properties").emit("refresh");
    await until(() => rows(host)[0]?.value === "无法读取文件信息：Invalid data found when processing input");
    host.tools.handle("ffprobe", () => ({ stdout: "not json" }));
    host.window("properties").emit("refresh");
    await until(() => rows(host)[0]?.value === "无法读取文件信息：ffprobe 输出无法解析");
    await rm(a);
    host.window("properties").emit("refresh");
    await until(() => rows(host)[0]?.value === "无法读取文件信息：文件不存在");
    await writeFile(a, "a");
    let release!: () => void;
    const held = new Promise<void>((done) => (release = done));
    host.tools.handle("ffprobe", async (args) => {
      if (args.at(-1) === a) await held;
      return { stdout: JSON.stringify({ format: { format_name: args.at(-1) === a ? "old" : "new" } }) };
    });
    const before = host.tools.calls.length;
    host.window("properties").emit("refresh");
    await until(() => host.tools.calls.length === before + 1);
    assert.equal(rows(host)[0]?.value, "正在读取…");
    host.openElsewhere(b);
    await until(() => value(host, "格式") === "new");
    release();
    await host.settle(50);
    assert.equal(value(host, "格式"), "new", "the aborted read of a never shows");
    assert.equal(value(host, "路径"), b);
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("I05 文件信息: a network source shows what the player reports, without ffprobe", async () => {
  const { host, stop } = await start("https://cdn.test/live/stream.m3u8");
  host.tools.handle("ffprobe", () => ({ stdout: "{}" }));
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 1);
  await host.settle();
  assert.equal(rows(host)[0]!.value, "网络来源：只显示播放器报告的信息");
  assert.equal(value(host, "地址"), "https://cdn.test/live/stream.m3u8");
  assert.equal(value(host, "视频"), "H.264 / AVC · 1920×1080 · 23.976 fps");
  assert.equal(value(host, "音频"), "AAC (Advanced Audio Coding) · 48000 Hz · 2 声道");
  assert.equal(value(host, "总时长"), "1:30:00");
  assert.deepEqual(headings(host), ["来源", "流"]);
  assert.equal(host.tools.calls.length, 0);
  await stop();
});

test("I05 系统信息: system name, model and displays from plugin.app.system(); CPU and memory from os; versions from plugin.app", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 2);
  await until(() => value(host, "系统版本") === "macOS 15.2（24C101）");
  assert.equal(value(host, "Vestra Player 版本"), "0.1.0（构建 test）");
  assert.equal(value(host, "插件 API 版本"), "vestra/1");
  assert.equal(value(host, "mpv 版本"), "mpv 0.41.0");
  assert.equal(value(host, "FFmpeg 版本"), "7.1");
  assert.equal(value(host, "机型"), "Mac14,2");
  assert.match(value(host, "CPU")!, /^.+（\d+ 核）$/);
  assert.match(value(host, "内存总量")!, /^\d+\.\d GB$/);
  assert.match(value(host, "可用内存")!, /^\d+\.\d GB$/);
  assert.equal(value(host, "显示器"), "Color LCD · 2940 x 1912 · 1470 x 956 @ 60 Hz");
  assert.equal(host.sent("app.system").length, 1, "read once per opening");
  assert.equal(host.tools.calls.length, 0, "no external programs");
  await stop();
});

test("I05 System in English", async () => {
  const { host, stop } = await start(null, undefined, undefined, "en");
  host.systemInfo = structuredClone(windows);
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 2);
  await until(() => value(host, "System version") === "Windows 11 23H2 (22631.4460)");
  assert.deepEqual(headings(host), ["Player", "System"]);
  assert.equal(value(host, "Vestra Player version"), "0.1.0 (build test)");
  assert.match(value(host, "CPU")!, /^.+ \(\d+ cores\)$/);
  assert.equal(value(host, "Display 2"), "DELL U2723QE · 3840 x 2160 · 3840 x 2160");
  await stop();
});

test("I05 系统信息: a Windows machine; several displays are numbered; a failed read leaves the system rows unknown", async () => {
  const { host, stop } = await start(null);
  host.systemInfo = structuredClone(windows);
  await host.command("info.properties").run();
  const window = host.window("properties");
  window.emit("tab-changed", 2);
  await until(() => value(host, "系统版本") === "Windows 11 23H2（22631.4460）");
  assert.equal(value(host, "机型"), "Surface Laptop 5");
  assert.equal(value(host, "显示器"), undefined);
  assert.equal(value(host, "显示器 1"), "2256 x 1504 · 1504 x 1003 @ 59.94 Hz");
  assert.equal(value(host, "显示器 2"), "DELL U2723QE · 3840 x 2160 · 3840 x 2160");
  host.handle("app.system", () => {
    throw new HostError("failed", "registry unreadable");
  });
  window.emit("refresh");
  await until(() => host.sent("app.system").length === 2 && value(host, "系统版本") === "-");
  assert.equal(value(host, "机型"), "-");
  assert.equal(value(host, "显示器"), "-");
  assert.match(value(host, "CPU")!, /核）$/, "CPU still from os");
  await stop();
});

test("I05: 复制到剪贴板 puts the current page's text on the clipboard; a refusal shows 复制失败", async () => {
  const { host, i18n, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  const window = host.window("properties");
  window.emit("copy-page");
  await until(() => host.osd.at(-1) === "已复制播放信息");
  assert.equal(host.clipboard.text, pageText(i18n, rows(host) as Row[]));
  assert.match(host.clipboard.text!, /^视频\n解码器：h264\n硬件解码：videotoolbox\n/);
  assert.match(host.clipboard.text!, /\n\n音频\n[^]*采样位数：32 位浮点（floatp）\n声道数：2（stereo）\n/);
  assert.match(host.clipboard.text!, /\n缓冲时长：12\.4 秒\n$/);
  host.handle("clipboard.writeText", () => {
    throw new HostError("failed", "refused");
  });
  window.emit("copy-page");
  await until(() => window.props.status === "copy-failed");
  await stop();
});

test("I05: 保存为… writes the page as <name> 信息.txt; cancelling writes nothing; a failed write shows 保存失败", async () => {
  const folder = await mkdtemp(join(tmpdir(), "info-save-"));
  const { host, i18n, stop } = await start();
  try {
    await host.command("info.properties").run();
    await host.settle();
    const window = host.window("properties");
    host.dialogAnswers.push(null);
    window.emit("save-page");
    await host.settle();
    assert.deepEqual(host.fileDialogs.at(-1), { kind: "saveFile", title: "保存为", fileName: "movie 信息.txt" });
    const target = join(folder, "movie 信息.txt");
    host.dialogAnswers.push([target]);
    window.emit("save-page");
    await until(() => host.osd.at(-1) === "已保存到 movie 信息.txt");
    assert.equal(await readFile(target, "utf8"), pageText(i18n, rows(host) as Row[]));
    assert.match(await readFile(target, "utf8"), /^视频\n解码器：h264\n/);
    assert.deepEqual(await readdir(folder), ["movie 信息.txt"], "no temporary file left");
    host.dialogAnswers.push([join(folder, "missing", "x.txt")]);
    window.emit("save-page");
    await until(() => window.props.status === "save-failed" && /ENOENT/.test(String(window.props["status-detail"])));
    assert.match(String(window.props["status-detail"]), /ENOENT/);
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("I05: Save As… in English proposes <name> info.txt", async () => {
  const { host, stop } = await start("/videos/movie.mkv", undefined, undefined, "en");
  await host.command("info.properties").run();
  await host.settle();
  host.dialogAnswers.push(null);
  host.window("properties").emit("save-page");
  await host.settle();
  assert.deepEqual(host.fileDialogs.at(-1), { kind: "saveFile", title: "Save As", fileName: "movie info.txt" });
  await stop();
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

test("I05 pure rules: values cut at 512, sample formats, save names, displays, empty groups left out", async () => {
  const { i18n, show, stop } = await start(null);
  assert.equal([...cut("a".repeat(600))].length, VALUE_MAX);
  assert.ok(cut("a".repeat(600)).endsWith("…"));
  assert.equal(show(sampleFormat(i18n, "s16")), "16 位（s16）");
  assert.equal(show(sampleFormat(i18n, "dblp")), "64 位浮点（dblp）");
  assert.equal(show(sampleFormat(i18n, "weird")), "weird");
  assert.equal(show(saveName(i18n, "/v/My Film.mkv")), "My Film 信息.txt");
  assert.equal(show(saveName(i18n, "https://cdn.test/v/%E8%A7%86%E9%A2%91.mp4?x=1")), "视频 信息.txt");
  assert.equal(show(saveName(i18n, null)), "Vestra Player 信息.txt");
  assert.equal(show(systemVersion(i18n, { name: "macOS", version: "26.2", build: "25C56" })), "macOS 26.2（25C56）");
  assert.equal(show(systemVersion(i18n, { name: "Linux", version: null, build: null })), "Linux");
  assert.equal(displayLine({ name: "Built-in Retina Display", width: 3024, height: 1964, scale: 2, refreshRate: 120, primary: true }), "Built-in Retina Display · 3024 x 1964 · 1512 x 982 @ 120 Hz");
  const page = filePage(i18n, "/a.mkv", null, {});
  assert.deepEqual(page.filter((r) => r.heading).map((r) => show(r.label)), ["来源", "容器"]);
  assert.equal(pageText(i18n, [{ label: "A", value: "", heading: true }, { label: "k", value: "v", heading: false }, { label: "B", value: "", heading: true }, { label: "", value: "m", heading: false }]), "A\nk：v\n\nB\nm\n");
  const dated = filePage(i18n, "/a.mkv", { size: 1, mtime: new Date(2026, 9, 25, 13, 5, 9) }, {}, "zh-Hans");
  assert.equal(dated.find((r) => show(r.label) === "修改时间")?.value, "2026/10/25 13:05:09", "dates in the interface language");
  const long = filePage(i18n, "/a.mkv", null, { format: { tags: Object.fromEntries(Array.from({ length: 200 }, (_, i) => [`k${i}`, "x".repeat(500)])) } });
  assert.equal(show(long.at(-1)!.value), "（其余内容过长，已省略）", "a page stays under the string limit");
  await stop();
});

test("I05 系统信息: versions the core does not know show 未知; no build hash, extra libav libraries", async () => {
  const { i18n, show, stop } = await start(null);
  const page = systemPage(i18n, { version: "0.2.0", build: null, components: { mpv: null, libav: {}, slint: "1.16.1" } }, null);
  const row = (label: string) => show(page.find((r) => show(r.label) === label)?.value ?? null);
  assert.equal(row("Vestra Player 版本"), "0.2.0");
  assert.equal(row("mpv 版本"), "未知");
  assert.equal(row("FFmpeg 版本"), "未知");
  assert.equal(row("状态"), "正在读取…");
  const more = systemPage(
    i18n,
    { version: "0.2.0", build: "abc1234", components: { mpv: "mpv v0.41.0", libav: { ffmpeg: "9.0.2", libavcodec: "62.11.100" }, slint: "1.16.1" } },
    null,
  );
  assert.equal(show(more.find((r) => show(r.label) === "Vestra Player 版本")!.value), "0.2.0（构建 abc1234）");
  assert.equal(show(more.find((r) => show(r.label) === "FFmpeg 版本")!.value), "9.0.2 · libavcodec 62.11.100");
  await stop();
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

test("lazy activation: the window reads the player's state and follows the source only while open; a source opened meanwhile is read when it opens", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const [a, b] = [join(media, "a.mkv"), join(media, "b.mkv")];
  await writeFile(a, "a");
  await writeFile(b, "b");
  const { host, stop } = await start(a);
  try {
    assert.deepEqual(host.keepsRunning, []);
    assert.equal(host.listening.has("source.state"), false);
    host.tools.handle("ffprobe", (args) => ({ stdout: JSON.stringify({ format: { format_name: args.at(-1) === a ? "first" : "second" } }) }));
    await host.command("info.properties").run();
    host.window("properties").emit("tab-changed", 1);
    await until(() => value(host, "格式") === "first");
    // While open: the source listener and the window's binding, both added when it opened.
    const why = host.listeningWhy.get("source.state") ?? [];
    assert.equal(why.length, 2, why.join("; "));
    assert.ok(why.some((line) => /^a listener added at dialog\.ts:\d+$/.test(line)), why.join("; "));
    assert.ok(why.some((line) => /^a view's bind reads plugin\.player\.state \(dialog\.ts:\d+\)$/.test(line)), why.join("; "));
    host.window("properties").userClose();
    await host.settle();
    assert.deepEqual(host.keepsRunning, []);
    assert.equal(host.listening.has("source.state"), false);
    const reads = host.tools.calls.length;
    host.openElsewhere(b);
    await host.settle(50);
    assert.equal(host.tools.calls.length, reads, "nothing is read while the window is closed");
    await host.press("Ctrl+F1");
    await until(() => value(host, "格式") === "second");
    assert.equal(value(host, "路径"), b);
    host.window("properties").emit("close-window");
    await host.settle();
    assert.equal(host.windows.has("properties"), false);
    assert.deepEqual(host.keepsRunning, [], "its Close button lets go as well");
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("I05 in Simplified Chinese: the menu, the window's words and the messages of before", async () => {
  const { host, stop } = await start();
  assert.deepEqual([host.command("info.properties").display.path, host.command("info.properties").display.title], [[], "播放属性…"]);
  // What properties.slint shows through @tr.
  const words = ["Playback Properties", "Refresh", "Copy", "Save As…", "Close", "Copy failed", "Save As"];
  assert.deepEqual(words.map((w) => host.translate(w)), ["播放属性", "刷新", "复制到剪贴板", "保存为…", "关闭", "复制失败", "保存为"]);
  assert.equal(host.translate("Save failed: {}", ["EACCES"]), "保存失败：EACCES");
  // The tabs have the context "tab": the System tab is 系统信息, the System group 系统.
  const po = await readFile(new URL("../locales/zh-Hans.po", import.meta.url), "utf8");
  for (const [tab, word] of [["Playback", "播放信息"], ["File", "文件信息"], ["System", "系统信息"]]) {
    assert.match(po, new RegExp(`msgctxt "tab"\\nmsgid "${tab}"\\nmsgstr "${word}"`));
  }
  assert.equal(host.translate("System"), "系统");
  await stop();
});

test("I05: two overlapping Ctrl+F1 start one 1-second poll; closing stops it", async () => {
  const { host, stop } = await start();
  let release!: () => void;
  const held = new Promise<void>((done) => (release = done));
  host.handle("playback.stats", async () => {
    await held; // the first read is slow: the second Ctrl+F1 arrives meanwhile
    return stats;
  });
  const first = host.command("info.properties").run();
  const second = host.command("info.properties").run();
  release();
  await Promise.all([first, second]);
  assert.equal(host.window("properties").visible, true);
  const polled = () => host.sent("playback.stats").length;
  const opened = polled();
  await host.settle(2200);
  assert.ok(polled() - opened <= 3, `one poll a second, not two (${polled() - opened} reads in 2.2 s)`);
  host.window("properties").emit("close-window");
  await host.settle();
  assert.equal(host.windows.has("properties"), false);
  const after = polled();
  await host.settle(2200);
  assert.equal(polled(), after, "no poll left running after closing");
  assert.deepEqual(host.keepsRunning, []);
  await stop();
});

test("I05: a window that cannot open neither polls nor follows the player", async () => {
  const { host, stop } = await start();
  host.handle("ui.open", () => {
    throw new HostError("ui", "injected native open rejection");
  });
  await host.command("info.properties").run(); // the failure goes to the plugin's error handler
  await host.settle();
  assert.equal(host.windows.has("properties"), false);
  assert.deepEqual(host.keepsRunning, []);
  assert.equal(host.listening.has("source.state"), false);
  const after = host.sent("playback.stats").length;
  await host.settle(2200);
  assert.equal(host.sent("playback.stats").length, after, "no stats reads after a failed open");
  await stop();
});
