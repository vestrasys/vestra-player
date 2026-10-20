import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";
import { cut, filePage, pageText, sampleFormat, saveName, VALUE_MAX } from "../properties.ts";
import type { Probe, Row } from "../properties.ts";
import { parseDisplays, sysinfo } from "../sysinfo.ts";

const directory = new URL("..", import.meta.url);
const stats = {
  backend: "libmpv · 软件渲染",
  session: 3,
  framesShown: 120,
  framesDropped: 1,
  video: { codec: "H.264 / AVC", width: 1920, height: 1080, fps: 23.976, decoder: "h264", pixelFormat: "yuv420p", colorspace: "bt.709", displayWidth: 1920, displayHeight: 800, displayFps: null, bitrate: 4_500_000 },
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

// 系统信息 commands answered in memory.
const commands: string[] = [];
sysinfo.run = async (program, args) => {
  commands.push([program, ...args].join(" "));
  if (program.endsWith("sw_vers")) return args[0] === "-productVersion" ? "15.2\n" : "24C101\n";
  if (args.includes("hw.model")) return "Mac14,2\n";
  if (args.includes("machdep.cpu.brand_string")) return "Apple M2\n";
  return JSON.stringify({ SPDisplaysDataType: [{ spdisplays_ndrvs: [{ _name: "Color LCD", _spdisplays_pixels: "2940 x 1912", _spdisplays_resolution: "1470 x 956 @ 60.00Hz" }] }] });
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

async function start(source: string | null = "/videos/movie.mkv", dataDirectory?: string) {
  const host = new TestHost();
  host.stats = stats;
  if (source) {
    host.openElsewhere(source);
    host.setSource({ duration: 5400, position: 83 });
  }
  const plugin = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
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

test("I05: top-level 播放属性… (811, windows, Ctrl+F1) opens the window; a second Ctrl+F1 keeps it open", async () => {
  const { host, stop } = await start();
  assert.deepEqual(host.command("info.properties").declaration, {
    title: "播放属性…", path: ["播放属性…"], order: 811, group: "windows", radio: false, bindings: ["Ctrl+F1"], focus: null, repeat: false,
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
  assert.throws(() => host.window("properties"), "关闭 closes the window");
  await stop();
});

test("I05 播放信息: the extended stats, hardware decoding fixed to 未使用, refreshed every second", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  assert.equal(host.window("properties").props["tab-index"], 0);
  assert.equal(host.window("properties").props["can-refresh"], false);
  assert.deepEqual(headings(host), ["视频", "音频", "其它"]);
  assert.equal(value(host, "解码器"), "h264");
  assert.equal(value(host, "硬件解码"), "未使用");
  assert.equal(value(host, "输入格式"), "H.264 / AVC · yuv420p · bt.709");
  assert.equal(value(host, "尺寸"), "1920×1080 → 1920×800");
  assert.equal(value(host, "源帧率"), "23.976 fps");
  assert.equal(value(host, "当前帧率"), "-");
  assert.equal(value(host, "视频码率"), "4500 kbps");
  assert.equal(value(host, "采样位数"), "32 位浮点（floatp）");
  assert.equal(value(host, "声道数"), "2（stereo）");
  assert.equal(value(host, "音画同步偏差"), "-3.3 ms");
  assert.equal(value(host, "缓冲时长"), "12.4 秒");
  host.stats = { ...stats, framesShown: 500 };
  await host.settle(1100);
  assert.equal(value(host, "已显示帧"), "500");
  await stop();
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
  await until(() => value(host, "macOS 版本") !== undefined);
  assert.deepEqual(headings(host), ["播放器", "系统"]);
  await stop();
});

test("I05 文件信息: ffprobe through plugin.tools; streams, chapters, tags and embedded resources", async () => {
  const media = await mkdtemp(join(tmpdir(), "info-media-"));
  const path = join(media, "movie.mkv");
  await writeFile(path, Buffer.alloc(2048));
  const { host, stop } = await start(path);
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
    assert.equal(value(host, "章节数"), "3");
    assert.equal(value(host, "附件 #3"), "font.ttf · font/ttf");
    assert.equal(value(host, "封面 #4"), "mjpeg · 600×600");
    assert.equal(value(host, "艺术家"), "某人");
    const text = pageText(rows(host) as Row[]);
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

test("I05 文件信息: a missing ffprobe, a failure; a source change aborts the old read and reads the new source", async () => {
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
  assert.equal(host.tools.calls.length, 0);
  await stop();
});

test("I05 系统信息: sw_vers, sysctl and system_profiler; Vestra, mpv and FFmpeg versions 未知 until plugin.app", async () => {
  commands.length = 0;
  const { host, stop } = await start();
  await host.command("info.properties").run();
  host.window("properties").emit("tab-changed", 2);
  await until(() => value(host, "macOS 版本") === "15.2（24C101）");
  assert.equal(value(host, "Vestra 版本"), "未知");
  assert.equal(value(host, "插件 API 版本"), "vestra.experimental/0.1");
  assert.equal(value(host, "mpv 版本"), "未知");
  assert.equal(value(host, "FFmpeg 版本"), "未知");
  assert.equal(value(host, "机型"), "Mac14,2");
  assert.match(value(host, "CPU")!, /^Apple M2（\d+ 核）$/);
  assert.match(value(host, "内存总量")!, /^\d+\.\d GB$/);
  assert.equal(value(host, "显示器"), "Color LCD · 2940 x 1912 · 1470 x 956 @ 60.00Hz");
  assert.deepEqual(commands.sort(), [
    "/usr/bin/sw_vers -buildVersion",
    "/usr/bin/sw_vers -productVersion",
    "/usr/sbin/sysctl -n hw.model",
    "/usr/sbin/sysctl -n machdep.cpu.brand_string",
    "/usr/sbin/system_profiler SPDisplaysDataType -json",
  ]);
  await stop();
});

test("I05: 复制到剪贴板 puts the current page's text on the clipboard; a refusal shows 复制失败", async () => {
  const { host, stop } = await start();
  await host.command("info.properties").run();
  await host.settle();
  const window = host.window("properties");
  window.emit("copy-page");
  await until(() => host.osd.at(-1) === "已复制播放信息");
  assert.equal(host.clipboard.text, pageText(rows(host) as Row[]));
  assert.match(host.clipboard.text!, /^视频\n解码器：h264\n硬件解码：未使用\n/);
  assert.match(host.clipboard.text!, /\n\n音频\n/);
  host.handle("clipboard.writeText", () => {
    throw new HostError("failed", "refused");
  });
  window.emit("copy-page");
  await until(() => window.props.status === "复制失败");
  await stop();
});

test("I05: 保存为… writes the page as <name> 信息.txt; cancelling writes nothing", async () => {
  const folder = await mkdtemp(join(tmpdir(), "info-save-"));
  const { host, stop } = await start();
  try {
    await host.command("info.properties").run();
    await host.settle();
    const window = host.window("properties");
    host.dialogAnswers.push(null);
    window.emit("save-page");
    await host.settle();
    assert.equal(host.sent("dialogs.open").at(-1)!.fileName, "movie 信息.txt");
    const target = join(folder, "movie 信息.txt");
    host.dialogAnswers.push([target]);
    window.emit("save-page");
    await until(() => host.osd.at(-1) === "已保存到 movie 信息.txt");
    assert.equal(await readFile(target, "utf8"), pageText(rows(host) as Row[]));
    assert.deepEqual(await readdir(folder), ["movie 信息.txt"], "no temporary file left");
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
  assert.equal(sampleFormat("s16"), "16 位（s16）");
  assert.equal(sampleFormat("dblp"), "64 位浮点（dblp）");
  assert.equal(sampleFormat("weird"), "weird");
  assert.equal(saveName("/v/My Film.mkv"), "My Film 信息.txt");
  assert.equal(saveName("https://cdn.test/v/%E8%A7%86%E9%A2%91.mp4?x=1"), "视频 信息.txt");
  assert.equal(saveName(null), "Vestra 信息.txt");
  assert.deepEqual(parseDisplays("not json"), []);
  const page = filePage("/a.mkv", null, {});
  assert.deepEqual(page.filter((r) => r.heading).map((r) => r.label), ["来源", "容器"]);
  assert.equal(pageText([{ label: "A", value: "", heading: true }, { label: "k", value: "v", heading: false }, { label: "B", value: "", heading: true }, { label: "", value: "m", heading: false }]), "A\nk：v\n\nB\nm\n");
});
