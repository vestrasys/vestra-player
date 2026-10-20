import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import type { PlaybackStats, PlayerState } from "../../../sdk/plugin/index.ts";
import activate, { brief, BRIEF_COLUMNS, fileName, fit, shortCodec } from "../main.ts";

const directory = new URL("..", import.meta.url);
const stats = {
  backend: "libmpv · 软件渲染",
  session: 3,
  framesShown: 120,
  framesDropped: 1,
  video: { codec: "h264", width: 1920, height: 1080, fps: 24 },
  audio: { codec: "aac", sampleRate: 48000, channels: 2 },
};

test("top-level 播放信息 entry in the windows group, Tab toggles the overlay; off by default", async () => {
  const host = new TestHost();
  host.stats = stats;
  host.openElsewhere("/videos/movie.mp4");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host });
  const command = host.command("info.toggle");
  assert.deepEqual(command.declaration, {
    title: "播放信息",
    path: ["播放信息"],
    order: 810,
    group: "windows",
    radio: false,
    bindings: ["Tab"],
    focus: null,
    repeat: false,
  });
  assert.equal(command.state.checked, false);
  assert.equal(host.osdPanel, null);
  await host.press("Tab");
  await host.settle();
  assert.equal(command.state.checked, true);
  assert.equal(
    host.osdPanel,
    [
      "文件：movie.mp4",
      "时间：00:05 / 01:40",
      "视频：h264 1920×1080",
      "帧率：24.00 fps",
      "音频：aac 48000 Hz 2 声道",
      "后端：libmpv · 软件渲染（会话 3）",
      "已显示帧：120",
      "丢帧：1",
    ].join("\n"),
  );
  await host.press("Tab");
  await host.settle();
  assert.equal(host.osdPanel, null);
  assert.equal(command.state.checked, false);
  await stop();
});

test("refreshes once per second while shown, and stops when hidden", async () => {
  const host = new TestHost();
  host.stats = { ...stats, framesShown: 1 };
  host.openElsewhere("/movie.mp4");
  const { stop } = await startPlugin(activate, { directory, host });
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
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("info.toggle").run();
  await host.settle();
  assert.equal(
    host.osdPanel,
    [
      "文件：未打开媒体",
      "时间：--:-- / --:--",
      "视频：- -",
      "帧率：-",
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
  const { stop } = await startPlugin(activate, { directory, host });
  const command = host.command("info.brief");
  assert.deepEqual(command.declaration, {
    title: "简要信息",
    path: [],
    order: 0,
    group: "domain",
    radio: false,
    bindings: ["Shift+Tab"],
    focus: null,
    repeat: false,
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
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Tab");
  await host.settle();
  await host.press("Shift+Tab");
  await host.settle();
  const text = String(host.osdPanel);
  assert.ok(text.startsWith("movie.mp4\n01:23 / 45:00 · H.264 1920×1080 · AAC 2 声道\n\n文件：movie.mp4\n"), text);
  assert.equal(text.split("\n").length, 11);
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
  const { stop } = await startPlugin(activate, { directory, host });
  await host.press("Shift+Tab");
  await host.settle();
  assert.equal(host.osdPanel, "未打开媒体");
  await stop();
  const blank = { source: "/a.mp4", position: null, duration: null } as unknown as PlayerState;
  assert.equal(brief(blank, null), "a.mp4");
  const live = { source: "https://live.test/stream.m3u8", position: 83, duration: null } as unknown as PlayerState;
  assert.equal(
    brief(live, { ...stats, video: { codec: "h264", width: 0, height: 0, fps: 0 }, audio: { codec: null, sampleRate: null, channels: 2 } } as unknown as PlaybackStats),
    "stream.m3u8\n01:23 / 直播 · H.264 · 2 声道",
  );
});

test("I03: short codec names, and lines cut to the video width with an ellipsis", () => {
  assert.equal(shortCodec("H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10"), "H.264");
  assert.equal(shortCodec("H.265 / HEVC (High Efficiency Video Coding)"), "HEVC");
  assert.equal(shortCodec("Alliance for Open Media AV1"), "AV1");
  assert.equal(shortCodec("MPEG-4 part 2"), "MPEG-4");
  assert.equal(shortCodec("AAC (Advanced Audio Coding)"), "AAC");
  assert.equal(shortCodec("ATSC A/52B (AC-3, E-AC-3)"), "E-AC-3");
  assert.equal(shortCodec("MP3 (MPEG audio layer 3)"), "MP3");
  assert.equal(shortCodec("Some Very Long Codec Name / alias"), "Some Very L…");
  assert.equal(shortCodec(null), null);
  const state = { source: "/v/movie.mp4", position: 7, duration: 8 } as unknown as PlayerState;
  const long = { ...stats, video: { codec: "H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10", width: 640, height: 360, fps: 25 }, audio: { codec: "AAC (Advanced Audio Coding)", sampleRate: 48000, channels: 2 } };
  assert.equal(brief(state, long as unknown as PlaybackStats), "movie.mp4\n00:07 / 00:08 · H.264 640×360 · AAC 2 声道");
  assert.equal(fit("a".repeat(50)), `${"a".repeat(BRIEF_COLUMNS - 1)}…`);
  assert.equal(fit("视".repeat(30)), `${"视".repeat(21)}…`, "CJK characters take two columns");
  assert.equal(fit("a".repeat(BRIEF_COLUMNS)), "a".repeat(BRIEF_COLUMNS));
  const [name] = brief({ ...state, source: `/v/${"很长的文件名".repeat(10)}.mkv` } as unknown as PlayerState, null).split("\n");
  assert.equal(name, `${"很长的文件名".repeat(3)}很长的…`, "the file name line is cut too");
});
