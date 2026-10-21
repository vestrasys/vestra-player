// Round 2: D18 formats, D09/D11 style toggles, D17 ASS original style, style.json, B11 dialogue jumps.
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const embedded = { kind: "embedded" as const, id: 1 };
const THREE = "1\n00:00:10,000 --> 00:00:12,000\nA\n\n2\n00:00:20,000 --> 00:00:22,000\nB\n\n3\n00:00:30,000 --> 00:00:32,000\nC\n";

function sourceHost(source = "/movie.mp4") {
  const host = new TestHost();
  host.openElsewhere(source);
  host.subtitles.tracks.push({ target: embedded, title: "English", language: "en", styleSupported: true, styleReason: null });
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: embedded };
  return host;
}
async function tempFiles(files: Record<string, string | Buffer>) {
  const folder = await mkdtemp(join(tmpdir(), "vestra-subtitles-r2-"));
  for (const [name, content] of Object.entries(files)) await writeFile(join(folder, name), content);
  return { path: (name: string) => join(folder, name), cleanup: () => rm(folder, { recursive: true, force: true }) };
}
async function loadFile(host: TestHost, path: string) {
  host.dialogAnswers.push([path]);
  await host.command("load").run();
  for (let i = 0; i < 50 && !host.osd.at(-1)?.startsWith("打开字幕") && host.sent("subtitles.commit").length === 0; i++)
    await host.settle(10);
  await host.settle();
}
const style = (host: TestHost) => host.subtitles.presentation.effective?.style as Record<string, unknown> | undefined;

test("D18: VTT, ASS and two-language SAMI files load with titles, formats and notes", async () => {
  const files = await tempFiles({
    "a.vtt": "WEBVTT\n\n00:01.000 --> 00:02.000\nHello\n",
    "b.ass": "[Script Info]\nScriptType: v4.00+\n\n[Fonts]\nfontname: x.ttf\nAAAA\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(1,1)}Hi\n",
    "c.smi": "<SAMI><HEAD><STYLE>.EN { Name: English; } .ZH { Name: 中文; }</STYLE></HEAD><BODY><SYNC Start=0><P Class=EN>Hi<SYNC Start=0><P Class=ZH>嗨<SYNC Start=900><P Class=EN>&nbsp;<P Class=ZH>&nbsp;</BODY></SAMI>",
  });
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host });
  try {
    await loadFile(host, files.path("a.vtt"));
    assert.equal(host.osd.at(-1), "打开字幕：a.vtt");
    await loadFile(host, files.path("b.ass"));
    assert.equal(host.osd.at(-1), "打开字幕：b.ass。已忽略字幕内嵌字体");
    await loadFile(host, files.path("c.smi"));
    assert.equal(host.osd.at(-1), "打开字幕：c.smi（2 条轨）");
    assert.deepEqual(host.subtitleLoads.map((l) => [l.format, l.title, l.select]), [
      ["srt", "a.vtt", "primary"],
      ["ass", "b.ass", "primary"],
      ["srt", "c.smi · English", "primary"],
      ["srt", "c.smi · 中文", "none"],
    ]);
    assert.equal(host.subtitleLoads[0]!.bytes.toString(), "1\n00:00:01,000 --> 00:00:02,000\nHello\n");
    assert.doesNotMatch(host.subtitleLoads[1]!.bytes.toString(), /\[Fonts\]/);
    assert.equal(host.command("track-4").state.checked, true, "the first SAMI language is selected");
    assert.equal(host.command("track-5").state.checked, false, "the second is only added");
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("D09/D11: Alt+B, 阴影 and 字幕边框不透明 toggle with OSD and check marks; the window sliders agree", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Alt+B");
  assert.equal(host.osd.at(-1), "粗体字幕");
  assert.equal(host.command("bold").state.checked, true);
  await host.command("shadow").run();
  assert.equal(host.osd.at(-1), "字幕阴影：开(2)");
  await host.command("box").run();
  assert.equal(host.osd.at(-1), "字幕边框不透明：开");
  await host.settle(170);
  assert.deepEqual([style(host)?.bold, style(host)?.shadow, style(host)?.box], [true, 2, 50]);
  assert.equal(host.command("shadow").state.checked, true);
  assert.equal(host.command("box").state.checked, true);
  await host.command("settings").run();
  const settings = host.window("subtitle-settings");
  assert.deepEqual([settings.props["bold"], settings.props["shadow-size"], settings.props["box-on"], settings.props["box-opacity"]], [true, 2, true, 50]);
  settings.emit("change-shadow", 3.6);
  settings.emit("change-box-opacity", 63);
  await host.settle(170);
  assert.deepEqual([style(host)?.shadow, style(host)?.box], [4, 65], "opacity snaps to 5 %");
  await host.command("shadow").run();
  await host.command("box").run();
  await host.press("Alt+B");
  assert.equal(host.osd.at(-1), "禁用粗体字幕");
  await host.settle(170);
  assert.deepEqual([style(host)?.bold, style(host)?.shadow, style(host)?.box], [false, 0, null]);
  await host.command("shadow").run();
  await host.command("box").run();
  await host.settle(170);
  assert.deepEqual([style(host)?.shadow, style(host)?.box], [4, 65], "turning back on restores the last values");
  host.subtitles.selection.target = null;
  await host.command("refresh").run();
  await host.settle();
  await host.press("Alt+B");
  assert.equal(host.osd.at(-1), "请先选择字幕轨。");
  await stop();
});

test("D17: 开启 ASS/SSA 原样式 switches the ASS override; the settings combo shares it", async () => {
  const host = sourceHost();
  host.subtitles.tracks[0]!.codec = "ass";
  host.subtitles.tracks[0]!.text = true;
  const { stop } = await startPlugin(activate, { directory, host });
  assert.equal(host.command("ass-original").state.checked, true, "on by default (keep)");
  await host.command("settings").run();
  const settings = host.window("subtitle-settings");
  assert.match(String(settings.props["font-note"]), /按字幕文件/);
  await host.command("ass-original").run();
  assert.equal(host.osd.at(-1), "ASS/SSA 动态字幕：关");
  await host.settle(170);
  assert.equal(style(host)?.assOverride, "force");
  assert.equal(host.command("ass-original").state.checked, false);
  assert.equal(settings.props["ass-index"], 1);
  assert.equal(settings.props["font-note"], "", "the plugin style applies to ASS again");
  settings.emit("change-ass-override", 0);
  await host.settle(170);
  assert.equal(style(host)?.assOverride, "keep");
  assert.equal(host.osd.at(-1), "ASS/SSA 动态字幕：开");
  await stop();
});

test("style.json: the style (not the sync) is restored when a stylable track is selected after a restart", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  try {
    let host = sourceHost();
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.press("Alt+B");
    await host.press("Alt+PageUp");
    await host.press(",");
    await host.settle(170);
    await started.stop();
    const saved = JSON.parse(await readFile(join(data, "style.json"), "utf8"));
    assert.equal(saved.style.bold, true);
    assert.equal(saved.style.fontSize, 38);
    assert.equal("offsetMs" in saved.style, false, "the sync is not kept");

    host = sourceHost();
    host.subtitles.selection.target = null; // nothing selected yet: nothing to restore onto
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle(170);
    assert.equal(host.sent("subtitles.style").length, 0);
    await host.command("track-1").run();
    await host.settle(170);
    assert.deepEqual([style(host)?.bold, style(host)?.fontSize, style(host)?.offsetMs], [true, 38, 0]);
    // 全部默认 forgets the remembered style.
    await host.command("settings").run();
    host.window("subtitle-settings").emit("reset-style");
    await host.settle();
    await started.stop();
    assert.equal(JSON.parse(await readFile(join(data, "style.json"), "utf8")).style, null);

    // A broken file is set aside and the defaults are used.
    await writeFile(join(data, "style.json"), "{ nope");
    host = sourceHost();
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle(170);
    assert.equal(host.sent("subtitles.style").length, 0);
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("D05: sync values up to ±600 000 ms are accepted", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("sync-input").run();
  const sync = host.window("subtitle-sync");
  sync.emit("apply", "-90000");
  await host.settle(170);
  assert.equal(style(host)?.offsetMs, -90000);
  assert.equal(host.osd.at(-1), "字幕同步: 超前 90 秒");
  await stop();
});

test("B11: End / Home / Ctrl+Home jump between the cues of a loaded file, shifted by the sync", async () => {
  const files = await tempFiles({ "three.srt": THREE });
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host });
  try {
    assert.equal(host.command("dialogue-next").state.enabled, false, "an embedded track without a stream number");
    assert.equal(host.command("dialogue-next").state.reason, "没有可用的文本字幕");
    await loadFile(host, files.path("three.srt"));
    host.setSource({ position: 25, duration: 100 });
    await host.settle();
    assert.equal(host.command("dialogue-next").state.enabled, true);
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "定位：下一对白位置 00:00:30.000");
    assert.equal(host.source.position, 29.95);
    await host.press("Home");
    await host.settle();
    assert.equal(host.osd.at(-1), "定位：上一对白位置 00:00:20.000");
    assert.equal(host.source.position, 19.95);
    await host.press("Home");
    await host.settle();
    assert.equal(host.source.position, 9.95, "from just before a start, the previous cue");
    await host.press("Home");
    await host.settle();
    assert.equal(host.osd.at(-1), "没有上一句对白");
    host.setSource({ position: 21 });
    await host.press("Ctrl+Home");
    await host.settle();
    assert.equal(host.osd.at(-1), "定位：当前对白起点 00:00:20.000");
    // A held key: each jump starts where the previous seek landed; one press may wait, more are dropped.
    host.setSource({ position: 0 });
    await Promise.all([host.press("End"), host.press("End"), host.press("End")]);
    await host.settle();
    assert.equal(host.source.position, 19.95);
    await host.press("End");
    await host.settle();
    assert.equal(host.source.position, 29.95);
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "没有下一句对白");
    // +1000 ms sync: cues show one second later.
    await host.press(",");
    await host.press(",");
    await host.settle(170);
    host.setSource({ position: 25 });
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "定位：下一对白位置 00:00:31.000");
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("B11: embedded text tracks are read once with ffmpeg -map 0:<streamIndex>; failures explain", async () => {
  const files = await tempFiles({ "movie.mkv": "fake media" });
  const host = sourceHost(files.path("movie.mkv"));
  Object.assign(host.subtitles.tracks[0]!, { codec: "ass", text: true, streamIndex: 3 });
  const { stop } = await startPlugin(activate, { directory, host });
  try {
    host.setSource({ position: 25, duration: 100 });
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "需要 ffmpeg 才能读取内嵌字幕");
    host.tools.handle("ffmpeg", () => ({ stdout: THREE }));
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "定位：下一对白位置 00:00:30.000");
    assert.deepEqual(host.tools.calls.at(-1)!.args, [
      "-v", "error", "-nostdin", "-i", files.path("movie.mkv"), "-map", "0:3", "-f", "srt", "pipe:1",
    ]);
    await host.press("Home");
    await host.settle();
    assert.equal(host.tools.calls.length, 1, "the cue table is cached per track");
    host.tools.handle("ffmpeg", () => {
      throw new HostError("timeout", "ffmpeg timed out");
    });
    switchTrack(host);
    await host.command("refresh").run();
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "读取内嵌字幕超时");
    host.tools.handle("ffmpeg", () => ({ stdout: "" }));
    await host.press("End");
    await host.settle();
    assert.equal(host.osd.at(-1), "没有可用的文本字幕");
    assert.equal(host.command("dialogue-next").state.enabled, false, "an empty table disables the commands");
  } finally {
    await stop();
    await files.cleanup();
  }
});
function switchTrack(host: TestHost) {
  host.subtitles.tracks.push({
    target: { kind: "embedded", id: 2 }, title: "中文", language: "zh", styleSupported: true, styleReason: null,
    codec: "subrip", text: true, streamIndex: 4,
  });
  host.subtitles.selection.target = { kind: "embedded", id: 2 };
}

test("B11: graphic tracks, network sources and unseekable media disable the jumps", async () => {
  const host = sourceHost("https://example.com/live.m3u8");
  Object.assign(host.subtitles.tracks[0]!, { codec: "subrip", text: true, streamIndex: 2 });
  const { stop } = await startPlugin(activate, { directory, host });
  assert.equal(host.command("dialogue-previous").state.reason, "没有可用的文本字幕", "network source");
  host.setSource({ capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle();
  await pause();
  assert.equal(host.command("dialogue-previous").state.reason, "当前媒体不能跳转");
  await stop();

  const local = sourceHost();
  Object.assign(local.subtitles.tracks[0]!, { codec: "hdmv_pgs_subtitle", text: false, streamIndex: 2 });
  const second = await startPlugin(activate, { directory, host: local });
  assert.equal(local.command("dialogue-current").state.reason, "没有可用的文本字幕", "graphic track");
  await second.stop();
});
