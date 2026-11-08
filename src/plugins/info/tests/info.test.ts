import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import type { Locale, PlaybackStats, PlayerState, TextArg } from "@vestra-player/sdk";
import activate, { brief, briefColumns, currentChapter, decoding, displayed, fileName, shortCodec, subtitleName } from "../main.ts";

const directory = new URL("..", import.meta.url);
const zh = { locale: "zh-Hans" } as const;
/** The plugin's `i18n` in `locale`, for the pure functions (they return text values). */
async function words(locale: "en" | "zh-Hans" = "zh-Hans") {
  const { plugin, stop } = await startPlugin(activate, { directory, locale });
  const show = (value: TextArg | null) => (value === null ? null : plugin.i18n.resolve(typeof value === "number" ? String(value) : value));
  return { i18n: plugin.i18n, show, stop };
}
const stats = {
  backend: "libmpv · 软件渲染",
  session: 3,
  framesShown: 120,
  framesDropped: 1,
  video: { codec: "h264", width: 1920, height: 1080, fps: 24 },
  audio: { codec: "aac", sampleRate: 48000, channels: 2 },
};

test("Playback Info in the command palette only (menu sweep B5), Tab toggles the overlay; off by default", async () => {
  const host = new TestHost();
  host.stats = stats;
  host.openElsewhere("/videos/movie.mp4");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host });
  const command = host.command("info.toggle");
  assert.deepEqual(command.declaration, {
    title: "Playback Info",
    path: [],
    order: 810,
    group: "windows",
    radio: false,
    bindings: ["Tab"],
    focus: null,
    repeat: false,
    category: { $vestraText: "Info" },
  });
  assert.equal(command.state.checked, false);
  assert.equal(host.osdPanel, null);
  await host.press("Tab");
  await host.settle();
  assert.equal(command.state.checked, true);
  assert.equal(
    host.osdPanel,
    [
      "File: movie.mp4",
      "Time: 00:05 / 01:40",
      "Video: h264 1920×1080",
      "Decoder: -",
      "Frame rate: 24.00 fps",
      "Displayed: -",
      "Audio: aac 48000 Hz 2 channels",
      "Backend: libmpv · 软件渲染 (session 3)",
      "Frames shown: 120",
      "Dropped frames: 1",
    ].join("\n"),
  );
  await host.press("Tab");
  await host.settle();
  assert.equal(host.osdPanel, null);
  assert.equal(command.state.checked, false);
  await stop();
});

test("the overlay in Simplified Chinese is word for word that of before", async () => {
  const host = new TestHost();
  host.stats = stats;
  host.openElsewhere("/videos/movie.mp4");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Tab");
  await host.settle();
  assert.equal(
    host.osdPanel,
    [
      "文件：movie.mp4",
      "时间：00:05 / 01:40",
      "视频：h264 1920×1080",
      "解码：-",
      "帧率：24.00 fps",
      "实际显示：-",
      "音频：aac 48000 Hz 2 声道",
      "后端：libmpv · 软件渲染（会话 3）",
      "已显示帧：120",
      "丢帧：1",
    ].join("\n"),
  );
  await stop();
});

test("Simplified Chinese shows the menus of before", async () => {
  const { host, stop } = await startPlugin(activate, { directory, locale: "zh-Hans" });
  assert.deepEqual([host.command("info.toggle").display.path, host.command("info.toggle").display.title], [[], "播放信息"]);
  assert.equal(host.command("info.brief").display.title, "简要信息");
  assert.deepEqual([host.command("info.properties").display.path, host.command("info.properties").display.title], [[], "播放属性…"]);
  assert.deepEqual(host.command("info.about").display.path, ["关于 Vestra Player…"]);
  await stop();
});

test("every text of the template is translated into Simplified Chinese", async () => {
  const { readFile } = await import("node:fs/promises");
  const read = (file: string) => readFile(new URL(`../locales/${file}`, import.meta.url), "utf8");
  const ids = [...(await read("messages.pot")).matchAll(/^msgid "(.+)"$/gm)].map((m) => m[1]);
  assert.ok(ids.includes("Playback Properties") && ids.includes("Copy Info"));
  const zh = await read("zh-Hans.po");
  assert.doesNotMatch(zh, /msgstr ""\n\n/);
  assert.match(zh, /msgid "Copy"\nmsgstr "复制到剪贴板"/);
});

test("refreshes once per second while shown, and stops when hidden", async () => {
  const host = new TestHost();
  host.stats = { ...stats, framesShown: 1 };
  host.openElsewhere("/movie.mp4");
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.command("info.toggle").run();
  await host.settle();
  assert.match(String(host.osdPanel), /已显示帧：1\n/);
  host.stats = { ...stats, framesShown: 25 };
  await host.settle(1100);
  assert.match(String(host.osdPanel), /已显示帧：25\n/);
  await host.command("info.toggle").run();
  await host.settle();
  host.stats = { ...stats, framesShown: 50 };
  await host.settle(1100);
  assert.equal(host.osdPanel, null);
  await stop();
});

test("without media every fact is a dash", async () => {
  const host = new TestHost();
  host.stats = { backend: "libmpv · 软件渲染", session: 0, framesShown: 0, framesDropped: null, video: null, audio: null };
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.command("info.toggle").run();
  await host.settle();
  assert.equal(
    host.osdPanel,
    [
      "文件：未打开媒体",
      "时间：--:-- / --:--",
      "视频：- -",
      "解码：-",
      "帧率：-",
      "实际显示：-",
      "音频：- - -",
      "后端：libmpv · 软件渲染（会话 0）",
      "已显示帧：0",
      "丢帧：-",
    ].join("\n"),
  );
  await stop();
});

test("file names come from paths and URLs", () => {
  assert.equal(fileName("C:\\media\\a b.mkv"), "a b.mkv");
  assert.equal(fileName("https://cdn.test/v/%E8%A7%86%E9%A2%91.mp4?x=1"), "视频.mp4");
  assert.equal(fileName("https://www.youtube.com/watch?v=abc"), "watch");
  assert.equal(fileName("https://youtu.be/"), "youtu.be");
});

const playing = (host: TestHost) => {
  host.stats = stats;
  host.openElsewhere("/videos/movie.mp4");
  host.setSource({ duration: 2700, position: 83 });
};

test("I03: Shift+Tab shows two short lines for three seconds; no menu entry", async () => {
  const host = new TestHost();
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  const command = host.command("info.brief");
  assert.deepEqual(command.declaration, {
    title: "Brief Info",
    path: [],
    order: 0,
    group: "domain",
    radio: false,
    bindings: ["Shift+Tab"],
    focus: null,
    repeat: false,
    category: { $vestraText: "Info" },
  });
  assert.deepEqual(host.conflicts, []);
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "movie.mp4\n01:23 / 45:00 · H.264 1920×1080 · AAC 2 声道");
  host.setSource({ position: 85 });
  await host.settle(1100);
  assert.equal(host.osdPanel, "movie.mp4\n01:25 / 45:00 · H.264 1920×1080 · AAC 2 声道", "time refreshed every second");
  await host.settle(2000);
  assert.equal(host.osdPanel, null);
  await stop();
});

test("I03: the short information in English", async () => {
  const host = new TestHost();
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "movie.mp4\n01:23 / 45:00 · H.264 1920×1080 · AAC 2 ch");
  await stop();
});

test("I03: a second press restarts the three seconds", async () => {
  const host = new TestHost();
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Shift+Tab");
  await host.settle(2000);
  await host.press("Shift+Tab");
  await host.settle(1500); // 3.5 s after the first press
  assert.match(String(host.osdPanel), /^movie\.mp4\n/);
  await host.settle(1700);
  assert.equal(host.osdPanel, null);
  await stop();
});

test("I03: with the full overlay open both blocks show, short first; expiry leaves the full one", async () => {
  const host = new TestHost();
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Tab");
  await host.settle();
  await host.press("Shift+Tab");
  await host.settle();
  const text = String(host.osdPanel);
  assert.ok(text.startsWith("movie.mp4\n01:23 / 45:00 · H.264 1920×1080 · AAC 2 声道\n\n文件：movie.mp4\n"), text);
  assert.equal(text.split("\n").length, 13);
  await host.settle(3100);
  assert.match(String(host.osdPanel), /^文件：movie\.mp4\n/);
  assert.equal(host.command("info.toggle").state.checked, true);
  await host.press("Shift+Tab");
  await host.settle();
  await host.press("Tab"); // closing the full overlay keeps the short one
  await host.settle();
  assert.match(String(host.osdPanel), /^movie\.mp4\n01:/);
  await stop();
});

test("I03: without media only 未打开媒体; unknown items are left out; live streams", async () => {
  const host = new TestHost();
  host.stats = { backend: "libmpv · 软件渲染", session: 0, framesShown: 0, framesDropped: null, video: null, audio: null };
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "未打开媒体");
  await stop();
  const { i18n, show, stop: done } = await words();
  const blank = { source: "/a.mp4", position: null, duration: null } as unknown as PlayerState;
  assert.equal(show(brief(i18n, blank, null)), "a.mp4");
  const live = { source: "https://live.test/stream.m3u8", position: 83, duration: null } as unknown as PlayerState;
  const liveStats = { ...stats, video: { codec: "h264", width: 0, height: 0, fps: 0 }, audio: { codec: null, sampleRate: null, channels: 2 } } as unknown as PlaybackStats;
  assert.equal(show(brief(i18n, live, liveStats)), "stream.m3u8\n01:23 / 直播 · H.264 · 2 声道");
  await done();
  const en = await words("en");
  assert.equal(en.show(brief(en.i18n, live, liveStats)), "stream.m3u8\n01:23 / Live · H.264 · 2 ch");
  await en.stop();
});

test("I03: short codec names", async () => {
  assert.equal(shortCodec("H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10"), "H.264");
  assert.equal(shortCodec("H.265 / HEVC (High Efficiency Video Coding)"), "HEVC");
  assert.equal(shortCodec("Alliance for Open Media AV1"), "AV1");
  assert.equal(shortCodec("MPEG-4 part 2"), "MPEG-4");
  assert.equal(shortCodec("AAC (Advanced Audio Coding)"), "AAC");
  assert.equal(shortCodec("ATSC A/52B (AC-3, E-AC-3)"), "E-AC-3");
  assert.equal(shortCodec("MP3 (MPEG audio layer 3)"), "MP3");
  assert.equal(shortCodec("Some Very Long Codec Name / alias"), "Some Very L…");
  assert.equal(shortCodec(null), null);
  const { i18n, show, stop } = await words();
  const state = { source: "/v/movie.mp4", position: 7, duration: 8 } as unknown as PlayerState;
  const long = { ...stats, video: { codec: "H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10", width: 640, height: 360, fps: 25 }, audio: { codec: "AAC (Advanced Audio Coding)", sampleRate: 48000, channels: 2 } };
  assert.equal(show(brief(i18n, state, long as unknown as PlaybackStats)), "movie.mp4\n00:07 / 00:08 · H.264 640×360 · AAC 2 声道");
  await stop();
});

test("I03: the player cuts each line to the video width after translating it (maxColumns)", async () => {
  const host = new TestHost();
  playing(host);
  host.openElsewhere(`/v/${"很长的文件名".repeat(10)}.mkv`);
  host.setSource({ duration: 2700, position: 83 });
  host.subtitles.tracks.push({ target: { kind: "embedded", id: 1 }, title: "很长的字幕轨名".repeat(5), language: "chi", styleSupported: true, styleReason: null });
  host.subtitles.selection.target = { kind: "embedded", id: 1 };
  host.setMainWindow({ videoArea: { width: 318, height: 179 } });
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Shift+Tab");
  await host.settle();
  const [name, , subtitle] = String(host.osdPanel).split("\n");
  assert.equal(name, `${"很长的文件名".repeat(3)}很长的…`, "CJK characters take two columns");
  assert.equal(subtitle, `${"很长的字幕轨名".repeat(3)}…`);
  await stop();
});

test("IA-INFO-01: the chapter after the time, the subtitle track on a third line", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "片头", start: 0 },
    { title: null, start: 60 },
    { title: "第二章 标题", start: 600 },
  ];
  host.subtitles.tracks.push({ target: { kind: "embedded", id: 1 }, title: "简体中文", language: "chi", styleSupported: true, styleReason: null });
  host.subtitles.selection.target = { kind: "embedded", id: 1 };
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "movie.mp4\n01:23 / 45:00 · 第 2 章 · H.264 1920×1080 · AAC 2 声道\n简体中文");
  // The next chapter, and the subtitle turned off by another plugin: read again within a second.
  host.setSource({ position: 754 });
  host.subtitles.selection.target = null;
  host.subtitles.selection.stamp = { ...host.subtitles.selection.stamp, revision: host.subtitles.selection.stamp.revision + 1 };
  await host.settle(1100);
  assert.equal(host.osdPanel, "movie.mp4\n12:34 / 45:00 · 第二章 标题 · H.264 1920×1080 · AAC 2 声道");
  // The narrowest window: the line is cut to the video area.
  host.setMainWindow({ videoArea: { width: 318, height: 179 } });
  await host.settle(1100);
  assert.equal(host.osdPanel, "movie.mp4\n12:34 / 45:00 · 第二章 标题 · H.264 1920×10…");
  await stop();
});

test("IA-INFO-01: the chapter list is read once per source", async () => {
  const host = new TestHost();
  host.chapters = [{ title: "A", start: 0 }];
  playing(host);
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Shift+Tab");
  await host.settle(2100);
  assert.equal(host.sent("player.chapters").length, 1);
  assert.match(String(host.osdPanel), /· A ·/);
  host.chapters = [];
  playing(host); // a new source without chapters
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.sent("player.chapters").length, 2);
  assert.equal(host.osdPanel, "movie.mp4\n01:23 / 45:00 · H.264 1920×1080 · AAC 2 声道");
  await stop();
});

test("IA-INFO-01: chapter and subtitle names", async () => {
  const { i18n, show, stop } = await words();
  const list = [
    { title: "  ", start: 0 },
    { title: "Two", start: 10 },
  ];
  assert.equal(show(currentChapter(i18n, list, 5)), "第 1 章");
  assert.equal(show(currentChapter(i18n, list, 10)), "Two");
  assert.equal(currentChapter(i18n, [{ title: "late", start: 30 }], 5), null);
  assert.equal(currentChapter(i18n, list, null), null);
  assert.equal(currentChapter(i18n, [], 5), null);
  assert.equal(show(subtitleName(i18n, { title: "English SDH", language: "en" })), "English SDH");
  assert.equal(show(subtitleName(i18n, { title: "", language: "en" })), "en");
  assert.equal(show(subtitleName(i18n, { title: "", language: null })), "字幕");
  assert.equal(subtitleName(i18n, null), null);
  assert.deepEqual([0, 318, 780, 1920, 99_999].map(briefColumns), [44, 44, 115, 290, 1000]);
  const state = { source: "/a.mkv", position: 3, duration: 8 } as unknown as PlayerState;
  assert.equal(show(brief(i18n, state, null, { chapter: "Intro", subtitle: "English" })), "a.mkv\n00:03 / 00:08 · Intro\nEnglish");
  await stop();
});

/** A 4K60 source decoded with d3d11va-copy while the window shows 29.5 new frames a second. */
const halved = {
  ...stats,
  presentedFps: 29.46,
  video: { codec: "av1", width: 3840, height: 2160, fps: 60, hwdec: "d3d11va-copy", decoding: "hardware" as const },
};

test("the overlay shows the decoder and the frames that reached the screen", async () => {
  const host = new TestHost();
  host.stats = halved;
  host.openElsewhere("/videos/movie.mkv");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Tab");
  await host.settle();
  const lines = String(host.osdPanel).split("\n");
  assert.equal(lines[3], "Decoder: hardware (d3d11va-copy)");
  assert.equal(lines[4], "Frame rate: 60.00 fps");
  assert.equal(lines[5], "Displayed: 29.5 fps (source 60)");
  host.stats = { ...halved, presentedFps: 60, video: { ...halved.video, hwdec: "no", decoding: "software" } };
  await host.settle(1100);
  const again = String(host.osdPanel).split("\n");
  assert.deepEqual([again[3], again[5]], ["Decoder: software", "Displayed: 60 fps (source 60)"]);
  await stop();
});

test("Shift+Tab adds a line with the decoder and the displayed frame rate", async () => {
  const host = new TestHost();
  host.stats = halved;
  host.openElsewhere("/videos/movie.mkv");
  host.setSource({ duration: 2700, position: 83 });
  const { stop } = await startPlugin(activate, { directory, host, ...zh });
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "movie.mkv\n01:23 / 45:00 · AV1 3840×2160 · AAC 2 声道\n解码：硬件（d3d11va-copy） · 实际显示：29.5 fps（源 60）");
  await stop();
});

test("decoder and displayed texts: unknown parts are left out", async () => {
  const { i18n, show, stop } = await words("en");
  const video = halved.video as PlaybackStats["video"];
  assert.equal(show(decoding(i18n, video)), "hardware (d3d11va-copy)");
  assert.equal(show(decoding(i18n, { ...video!, hwdec: null })), "hardware");
  assert.equal(show(decoding(i18n, { ...video!, hwdec: "no", decoding: "software" })), "software");
  assert.equal(decoding(i18n, { ...video!, decoding: null }), null);
  assert.equal(decoding(i18n, null), null);
  const full = halved as unknown as PlaybackStats;
  assert.equal(show(displayed(i18n, full)), "29.5 fps (source 60)");
  assert.equal(show(displayed(i18n, { ...full, video: { ...video!, fps: 23.976 } })), "29.5 fps (source 23.98)");
  assert.equal(show(displayed(i18n, { ...full, video: { ...video!, fps: null } })), "29.5 fps");
  assert.equal(displayed(i18n, { ...full, presentedFps: null }), null, "older players, the first second");
  assert.equal(displayed(i18n, { ...full, video: null }), null, "no video");
  const state = { source: "/a.mkv", position: 3, duration: 8 } as unknown as PlayerState;
  assert.equal(show(brief(i18n, state, { ...full, presentedFps: null })), "a.mkv\n00:03 / 00:08 · AV1 3840×2160 · AAC 2 ch\nDecoder: hardware (d3d11va-copy)");
  await stop();
});

test("the new texts are machine translated in every language and marked for review", async () => {
  const { readFile } = await import("node:fs/promises");
  const ids = ["software", "hardware ({})", "hardware", "{} fps", "{} fps (source {})", "Decoder: {}", "Displayed: {}"];
  for (const lang of ["zh-Hans", "ja", "ko", "de", "fr", "es"]) {
    const po = await readFile(new URL(`../locales/${lang}.po`, import.meta.url), "utf8");
    for (const id of ids) {
      const escaped = id.replace(/[(){}]/g, "\\$&");
      assert.match(po, new RegExp(`#, x-machine\nmsgid "${escaped}"\nmsgstr "[^"]+"`), `${lang}: ${id}`);
    }
  }
});

test("palette: every command has a category, translated in every language (player 0.7.6)", async () => {
  const expected: Record<Locale, { info: string; help: string }> = {
    en: { info: "Info", help: "Help" },
    "zh-Hans": { info: "信息", help: "帮助" },
    ja: { info: "情報", help: "ヘルプ" },
    ko: { info: "정보", help: "도움말" },
    de: { info: "Info", help: "Hilfe" },
    fr: { info: "Infos", help: "Aide" },
    es: { info: "Información", help: "Ayuda" },
  };
  for (const [locale, words] of Object.entries(expected) as [Locale, { info: string; help: string }][]) {
    const { host, stop } = await startPlugin(activate, { directory, locale });
    const shown = Object.fromEntries([...host.commands.entries()].map(([id, { display }]) => [id, display.category]));
    for (const [id, category] of Object.entries(shown)) assert.ok(typeof category === "string" && category.trim() !== "", `${locale}: ${id} has no category`);
    assert.deepEqual(shown, {
      "info.toggle": words.info,
      "info.brief": words.info,
      "info.properties": words.info,
      "info.about": words.help,
      "info.update": words.help,
    });
    assert.equal(host.command("info.update").declaration.home, "info.about");
    await stop();
  }
});
