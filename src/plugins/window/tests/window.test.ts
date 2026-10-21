import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { clampNote, currentScale, fitted, mini, near, scaled } from "../geometry.ts";
import activate, { PAGE_ID } from "../main.ts";
import { normalize, timing } from "../settings.ts";
import { displaySize } from "../video-size.ts";

Object.assign(timing, { probeRetry: 10, probeAttempts: 5 });

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-window-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const running = new Set<() => Promise<void>>();
async function start(host = new TestHost(), dataDirectory?: string) {
  const started = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  await host.settle();
  return { ...started, stop };
}
/** The H03 / H04 / 迷你尺寸 / 画中画 tests: G-02's adjustment when playback starts is switched off first. */
async function manual(host = new TestHost()) {
  const started = await start(host);
  await host.command("auto.off").run();
  host.osd.splice(0);
  return started;
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

type Video = { width: number; height: number; displayWidth?: number | null; displayHeight?: number | null };
function setVideo(host: TestHost, video: Video | null) {
  host.stats = {
    ...host.stats,
    video: video && {
      codec: "h264",
      fps: 25,
      decoder: "h264",
      pixelFormat: "yuv420p",
      colorspace: "bt.709",
      displayFps: null,
      bitrate: null,
      displayWidth: video.width,
      displayHeight: video.height,
      ...video,
    },
  };
}
/** Another plugin opens a video; the plugin reads its size. */
async function play(host: TestHost, video: Video | null = { width: 1920, height: 1080 }) {
  setVideo(host, video);
  host.openElsewhere("/movie.mkv");
  await host.settle(30);
}
/** A player without system picture-in-picture (Windows, Linux): no 画中画 item. */
function withoutPip() {
  const host = new TestHost();
  host.mainWindow.pictureInPicture.supported = false;
  return host;
}
const state = (host: TestHost, id: string) => host.command(id).state;
const lastOsd = (host: TestHost) => host.osd.at(-1);

// ------------------------------------------------------------------ geometry

test("geometry: scales, nearness, aspect fit, mini box, clamp notes, display size", () => {
  assert.deepEqual(scaled({ width: 1920, height: 1080 }, 0.5), { width: 960, height: 540 });
  assert.ok(near({ width: 100, height: 100 }, { width: 101, height: 99 }));
  assert.ok(!near({ width: 100, height: 100 }, { width: 102, height: 100 }));
  assert.equal(currentScale({ width: 1921, height: 1080 }, { width: 1280, height: 720 }), 1.5);
  assert.equal(currentScale({ width: 1000, height: 1080 }, { width: 1280, height: 720 }), null);
  assert.equal(currentScale({ width: 1280, height: 720 }, null), null);
  assert.deepEqual(fitted({ width: 800, height: 800 }, { width: 1920, height: 1080 }), { width: 800, height: 450 });
  assert.deepEqual(fitted({ width: 800, height: 800 }, { width: 2390, height: 1000 }), { width: 800, height: 335 });
  assert.deepEqual(mini({ width: 1920, height: 1080 }), { width: 400, height: 225 });
  assert.deepEqual(mini({ width: 640, height: 480 }), { width: 400, height: 300 });
  assert.deepEqual(mini({ width: 1080, height: 1920 }), { width: 169, height: 300 });
  const asked = { width: 3840, height: 2160 };
  assert.equal(clampNote(asked, { ...asked, clamped: false }), "");
  assert.equal(clampNote(asked, { width: 1600, height: 900, clamped: true }), "（已按屏幕缩小）");
  assert.equal(clampNote({ width: 160, height: 90 }, { width: 318, height: 120, clamped: true }), "（已按最小尺寸放大）");
  assert.deepEqual(displaySize({ width: 720, height: 576, displayWidth: 1024, displayHeight: 576 } as never), {
    width: 1024,
    height: 576,
  });
  assert.deepEqual(displaySize({ width: 256, height: 240, displayWidth: null, displayHeight: null } as never), {
    width: 256,
    height: 240,
  });
  assert.equal(displaySize(null), null);
  assert.equal(displaySize({ width: 0, height: 0, displayWidth: null, displayHeight: null } as never), null);
});


test("settings: reference player's defaults; a bad field falls back to its default, the others are kept", () => {
  assert.deepEqual(normalize({}), { onTop: "fullscreen", hideCursorWhilePlaying: true, autoSize: 1, autoSizeOnce: false });
  assert.deepEqual(normalize({ onTop: "sometimes", hideCursorWhilePlaying: false, autoSize: "fit", autoSizeOnce: true }), {
    onTop: "fullscreen",
    hideCursorWhilePlaying: false,
    autoSize: "fit",
    autoSizeOnce: true,
  });
  assert.deepEqual(normalize({ onTop: "playing", hideCursorWhilePlaying: "yes", autoSize: 3, autoSizeOnce: 1 }), {
    onTop: "playing",
    hideCursorWhilePlaying: true,
    autoSize: 1,
    autoSizeOnce: false,
  });
  assert.equal(normalize({ autoSize: "off" }).autoSize, "off");
  assert.equal(normalize({ autoSize: 0.5 }).autoSize, 0.5);
});

// ------------------------------------------------------------------ menus

const rows = (host: TestHost) =>
  [...host.commands.entries()].map(([id, c]) => [
    id,
    c.declaration.path?.join(" ▶ "),
    c.declaration.order,
    c.declaration.group,
    c.declaration.bindings.join(","),
  ]);

test("menus: paths, orders, groups and bindings as in the window chapter; no conflicts", async () => {
  const { host } = await start();
  assert.deepEqual(rows(host), [
    ["mini", "屏幕 ▶ 迷你尺寸", 710, "screen", "`"],
    ["pip", "屏幕 ▶ 画中画", 709, "screen", ""],
    ["size.0.5", "屏幕 ▶ 0.5倍", 711, "screen-size", "1"],
    ["size.1", "屏幕 ▶ 1倍", 712, "screen-size", "2"],
    ["size.1.5", "屏幕 ▶ 1.5倍", 713, "screen-size", "3"],
    ["size.2", "屏幕 ▶ 2倍", 714, "screen-size", "4"],
    ["fit", "屏幕 ▶ 适应视频比例", 715, "screen-size", "8"],
    ["auto.off", "屏幕 ▶ 开始播放时 ▶ 不调整", 716, "screen-auto", ""],
    ["auto.0.5", "屏幕 ▶ 开始播放时 ▶ 0.5倍", 717, "screen-auto", ""],
    ["auto.1", "屏幕 ▶ 开始播放时 ▶ 1倍", 718, "screen-auto", ""],
    ["auto.1.5", "屏幕 ▶ 开始播放时 ▶ 1.5倍", 719, "screen-auto", ""],
    ["auto.2", "屏幕 ▶ 开始播放时 ▶ 2倍", 720, "screen-auto", ""],
    ["auto.fit", "屏幕 ▶ 开始播放时 ▶ 适应视频比例", 721, "screen-auto", ""],
    ["auto.once", "屏幕 ▶ 开始播放时 ▶ 只调整一次", 722, "screen-auto-once", ""],
    ["top.cycle", "屏幕 ▶ 置顶 ▶ 切换置顶方式", 725, "screen-top", "Ctrl+T"],
    ["top.never", "屏幕 ▶ 置顶 ▶ 不置顶", 726, "screen-top-mode", ""],
    ["top.always", "屏幕 ▶ 置顶 ▶ 总是置顶", 727, "screen-top-mode", ""],
    ["top.playing", "屏幕 ▶ 置顶 ▶ 播放时置顶", 728, "screen-top-mode", ""],
    ["top.fullscreen", "屏幕 ▶ 置顶 ▶ 全屏播放时置顶", 729, "screen-top-mode", ""],
    ["hide-cursor", "播放 ▶ 播放时隐藏鼠标指针", 241, "playback-options", ""],
  ]);
  for (const id of ["size.1", "auto.1", "top.never"]) assert.equal(host.command(id).declaration.radio, true);
  assert.equal(host.command("auto.once").declaration.radio, false);
  assert.deepEqual(host.conflicts, []);
  // First start: 1倍, 全屏播放时置顶, 隐藏指针 checked.
  assert.deepEqual(
    ["auto.1", "auto.once", "top.fullscreen", "hide-cursor", "mini", "pip"].map((id) => state(host, id).checked),
    [true, false, true, true, false, false],
  );
  // Only the cursor mode differs from the core's default; nothing floats in a window.
  assert.deepEqual(host.sent("window.setAlwaysOnTop"), []);
  assert.deepEqual(host.sent("window.setCursorAutoHide").map((p) => p.mode), ["playing"]);
  assert.equal(host.mainWindow.cursorAutoHide, "playing");
});

test("menus: without system picture-in-picture there is no 画中画; ` is still 迷你尺寸", async () => {
  const { host } = await start(withoutPip());
  assert.equal(host.commands.has("pip"), false);
  const mini = host.command("mini").declaration;
  assert.deepEqual([mini.path?.join(" ▶ "), mini.order, mini.group, mini.bindings], ["屏幕 ▶ 迷你尺寸", 710, "screen", ["`"]]);
  assert.deepEqual(host.conflicts, []);
});

// ------------------------------------------------------------------ G-02

test("G-02: each new video is sized to 1× when its size is known; silent, clamped too; audio leaves the window", async () => {
  const { host } = await start();
  host.setMainWindow({ videoArea: { width: 800, height: 450 } });
  await play(host, { width: 1280, height: 720 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
  await play(host, { width: 640, height: 480 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 640, height: 480 });
  host.handle("window.setVideoArea", () => ({ width: 2240, height: 1260, clamped: true }));
  await play(host, { width: 3840, height: 2160 });
  assert.deepEqual(host.sent("window.setVideoArea").at(-1), { width: 3840, height: 2160 });
  const sent = host.sent("window.setVideoArea").length;
  await play(host, null); // mp3
  assert.equal(host.sent("window.setVideoArea").length, sent);
  assert.deepEqual(host.osd, [], "no OSD for the automatic adjustment");
  // A refusal is silent as well.
  host.handle("window.setVideoArea", () => {
    throw new HostError("unavailable", "headless");
  });
  await play(host, { width: 1280, height: 720 });
  assert.deepEqual(host.osd, []);
});

test("G-02: once per source; a replay, pause or seek does not size again; the same size sends nothing", async () => {
  const { host } = await start();
  await play(host, { width: 1280, height: 720 });
  assert.equal(host.sent("window.setVideoArea").length, 1);
  host.setMainWindow({ videoArea: { width: 900, height: 506 } }); // the user drags afterwards
  host.setSource({ paused: true });
  host.setSource({ paused: false, position: 30 });
  host.setSource({ phase: "ended" });
  await host.settle();
  host.setSource({ phase: "ready", position: 0 }); // replay: the same source
  await host.settle(30);
  assert.equal(host.sent("window.setVideoArea").length, 1);
  assert.deepEqual(host.mainWindow.videoArea, { width: 900, height: 506 });
  host.setMainWindow({ videoArea: { width: 1280, height: 720 } });
  await play(host, { width: 1280, height: 720 });
  assert.equal(host.sent("window.setVideoArea").length, 1, "already 1×");
});

test("G-02: not in fullscreen (the window keeps its size), with 迷你尺寸 or in 画中画", async () => {
  const { host } = await start();
  host.setMainWindow({ videoArea: { width: 800, height: 450 } });
  host.setMainWindow({ fullscreen: true, videoArea: { width: 2560, height: 1440 } });
  await play(host, { width: 1280, height: 720 });
  host.setMainWindow({ fullscreen: false, videoArea: { width: 800, height: 450 } });
  await host.settle();
  assert.deepEqual(host.sent("window.setVideoArea"), []);

  await host.press("`"); // 迷你尺寸: 800×450 → 400×225
  await host.settle();
  await play(host, { width: 640, height: 480 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 400, height: 225 });
  assert.equal(state(host, "mini").checked, true);

  await host.command("pip").run(); // leaves 迷你尺寸 first
  await host.settle();
  assert.equal(host.mainWindow.pictureInPicture.active, true);
  const sent = host.sent("window.setVideoArea").length;
  await play(host, { width: 1920, height: 1080 });
  assert.equal(host.sent("window.setVideoArea").length, sent);
});

test("G-02: not over the user's own size: dragging the window while it plays, before the size is known", async () => {
  const { host } = await start();
  // Dragged while playing, before the size is known.
  setVideo(host, null);
  host.openElsewhere("/b.mkv");
  await host.settle();
  host.setMainWindow({ videoArea: { width: 700, height: 400 } });
  setVideo(host, { width: 1920, height: 1080 });
  await host.settle(40);
  assert.deepEqual(host.sent("window.setVideoArea"), []);
  assert.deepEqual(host.mainWindow.videoArea, { width: 700, height: 400 });

  // The next source is sized again.
  await play(host, { width: 1024, height: 576 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 1024, height: 576 });
});

test("G-02: the six choices and 只调整一次 by menu; OSD; kept after a restart", async () => {
  const data = mkdtempSync(join(scratch, "auto-"));
  const host = new TestHost();
  const { stop } = await start(host, data);
  await host.command("auto.2").run();
  assert.equal(lastOsd(host), "开始播放时: 2倍");
  assert.deepEqual(
    ["auto.off", "auto.0.5", "auto.1", "auto.1.5", "auto.2", "auto.fit"].map((id) => state(host, id).checked),
    [false, false, false, false, true, false],
  );
  await play(host, { width: 640, height: 360 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
  await host.command("auto.0.5").run();
  await play(host, { width: 640, height: 360 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 320, height: 180 });
  await host.command("auto.fit").run();
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  await play(host, { width: 1920, height: 1080 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 800, height: 450 });
  await host.command("auto.off").run();
  assert.equal(lastOsd(host), "开始播放时: 不调整窗口");
  const sent = host.sent("window.setVideoArea").length;
  await play(host, { width: 640, height: 360 });
  assert.equal(host.sent("window.setVideoArea").length, sent);
  await host.command("auto.1").run();
  await host.command("auto.once").run();
  assert.equal(lastOsd(host), "开始播放时只调整一次: 开");
  assert.equal(state(host, "auto.once").checked, true);
  await stop();
  assert.deepEqual(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")), {
    onTop: "fullscreen",
    hideCursorWhilePlaying: true,
    autoSize: 1,
    autoSizeOnce: true,
  });

  // 只调整一次: only the run's first video source.
  const again = new TestHost();
  await start(again, data);
  assert.deepEqual([again.command("auto.1").state.checked, again.command("auto.once").state.checked], [true, true]);
  await play(again, null); // audio first: not a video source
  await play(again, { width: 1280, height: 720 });
  assert.deepEqual(again.mainWindow.videoArea, { width: 1280, height: 720 });
  await play(again, { width: 640, height: 480 });
  assert.deepEqual(again.mainWindow.videoArea, { width: 1280, height: 720 });
  assert.equal(again.sent("window.setVideoArea").length, 1);
});

test("G-02: a source already playing at activation is sized", async () => {
  const host = new TestHost();
  setVideo(host, { width: 1280, height: 720 });
  host.openElsewhere("/movie.mkv");
  await start(host);
  await host.settle(30);
  assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
});

// ------------------------------------------------------------------ settings page

test("settings page: 选项 ▶ 播放 ▶ 窗口 shows and sets the same preferences as the menu", async () => {
  const { host } = await start();
  const page = host.settingsPage(PAGE_ID);
  assert.deepEqual([page.category, page.title, page.order, page.file], ["playback", "窗口", 30, "ui/page.slint"]);
  assert.deepEqual(
    [page.props["auto-index"], page.props["auto-once"], page.props["top-index"], page.props["hide-cursor"]],
    [2, false, 3, true],
  );
  page.emit("set-auto", 5);
  page.emit("set-auto-once", true);
  page.emit("set-top", 1);
  page.emit("set-hide-cursor", false);
  await host.settle();
  assert.deepEqual(
    [state(host, "auto.fit").checked, state(host, "auto.once").checked, state(host, "top.always").checked, state(host, "hide-cursor").checked],
    [true, true, true, false],
  );
  assert.deepEqual([host.mainWindow.onTop, host.mainWindow.cursorAutoHide], [true, "fullscreen"]);
  assert.deepEqual(host.osd, [], "the page says nothing on the video");
  // A menu change shows on the page.
  await host.command("top.playing").run();
  await host.settle();
  assert.equal(page.props["top-index"], 2);
  page.emit("set-auto", 9); // out of range: ignored
  await host.settle();
  assert.equal(state(host, "auto.fit").checked, true);
});

// ------------------------------------------------------------------ H03

test("H03: disabled without video and in fullscreen; enabled once the video size is known", async () => {
  const { host } = await manual(withoutPip());
  assert.deepEqual(
    [state(host, "size.1").enabled, state(host, "size.1").reason, state(host, "fit").enabled, state(host, "mini").enabled],
    [false, "没有视频", false, false],
  );
  await play(host, null); // audio only
  assert.equal(state(host, "size.2").reason, "没有视频");
  await play(host);
  assert.equal(state(host, "size.2").enabled, true);
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.deepEqual([state(host, "size.2").enabled, state(host, "size.2").reason], [false, "全屏时不可用"]);
  assert.deepEqual([state(host, "mini").enabled, state(host, "fit").reason], [false, "全屏时不可用"]);
  host.setMainWindow({ fullscreen: false });
  await host.settle();
  host.setSource({ phase: "empty" });
  await host.settle();
  assert.equal(state(host, "size.2").reason, "没有视频");
});

test("H03: the size is read again while mpv has not reported it yet", async () => {
  const { host } = await manual();
  await play(host, null);
  setVideo(host, { width: 1280, height: 720 });
  await host.settle(40);
  assert.equal(state(host, "size.1").enabled, true);
});

test("H03: 1–4 set the video area to the display size times 0.5–2; the radio dot follows", async () => {
  const { host } = await manual();
  await play(host, { width: 720, height: 576, displayWidth: 1024, displayHeight: 576 });
  await host.press("2");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1024, height: 576 });
  assert.equal(lastOsd(host), "窗口: 1倍");
  assert.deepEqual(
    ["size.0.5", "size.1", "size.1.5", "size.2"].map((id) => state(host, id).checked),
    [false, true, false, false],
  );
  await host.press("3");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1536, height: 864 });
  assert.equal(lastOsd(host), "窗口: 1.5倍");
  assert.equal(state(host, "size.1.5").checked, true);
  // The user drags the window: no dot.
  host.setMainWindow({ videoArea: { width: 900, height: 600 } });
  await host.settle();
  assert.deepEqual(
    ["size.0.5", "size.1", "size.1.5", "size.2"].map((id) => state(host, id).checked),
    [false, false, false, false],
  );
});

test("H03: the stats are read again before resizing (a filter may have rotated the picture)", async () => {
  const { host } = await manual();
  await play(host, { width: 1920, height: 1080 });
  setVideo(host, { width: 1080, height: 1920 });
  await host.press("1");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 540, height: 960 });
});

test("H03: clamped sizes say why; refusals are shown", async () => {
  const { host } = await manual();
  await play(host, { width: 320, height: 180 });
  await host.press("1"); // 160×90 → the core's minimum
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 318, height: 120 });
  assert.equal(lastOsd(host), "窗口: 0.5倍（已按最小尺寸放大）");
  host.handle("window.setVideoArea", () => ({ width: 560, height: 315, clamped: true }));
  await host.press("4");
  await host.settle();
  assert.equal(lastOsd(host), "窗口: 2倍（已按屏幕缩小）");
  host.handle("window.setVideoArea", () => {
    throw new HostError("unavailable", "全屏时不可用");
  });
  await host.press("2");
  await host.settle();
  assert.equal(lastOsd(host), "无法调整窗口：全屏时不可用");
});

test("H03: busy while a resize is in flight", async () => {
  const { host } = await manual();
  await play(host);
  let release!: () => void;
  host.handle("window.setVideoArea", async (p) => {
    await new Promise<void>((resolve) => (release = resolve));
    return { width: Number(p.width), height: Number(p.height), clamped: false };
  });
  const first = host.press("2");
  await new Promise((resolve) => setTimeout(resolve, 20)); // settle() would wait for the held request
  assert.deepEqual([state(host, "size.2").busy, state(host, "fit").busy], [true, true]);
  await assert.rejects(host.press("4"));
  release();
  await first;
  await host.settle();
  assert.equal(state(host, "size.2").busy, false);
  assert.equal(host.sent("window.setVideoArea").length, 1);
});

// ------------------------------------------------------------------ H04

test("H04: 8 keeps the width and takes the video's aspect ratio; a second press only says so", async () => {
  const { host } = await manual();
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  await play(host, { width: 1920, height: 1080 });
  await host.press("8");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 800, height: 450 });
  assert.equal(lastOsd(host), "窗口: 适应视频比例");
  await host.press("8");
  await host.settle();
  assert.equal(host.sent("window.setVideoArea").length, 1);
  assert.equal(lastOsd(host), "窗口已适应视频比例");
});

// ------------------------------------------------------------------ H05 / IA-WIN-03

test("IA-WIN-03: 全屏播放时置顶 (the default) floats only while playing in fullscreen", async () => {
  const { host } = await manual();
  await play(host);
  assert.equal(host.mainWindow.onTop, false, "playing in a window");
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  host.setSource({ paused: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  host.setSource({ paused: false });
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  host.setMainWindow({ fullscreen: false });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  assert.deepEqual(host.sent("window.setAlwaysOnTop").map((p) => p.onTop), [true, false, true, false]);
});

test("H05: modes by menu and Ctrl+T in reference player's order, OSD texts, playing mode follows pause, kept after a restart", async () => {
  const data = mkdtempSync(join(scratch, "top-"));
  const host = new TestHost();
  const { stop } = await start(host, data);
  await host.command("top.always").run();
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  assert.equal(lastOsd(host), "置顶: 总是");
  assert.deepEqual(
    ["top.never", "top.always", "top.playing", "top.fullscreen"].map((id) => state(host, id).checked),
    [false, true, false, false],
  );
  await host.press("Ctrl+T"); // always → playing, nothing playing
  await host.settle();
  assert.equal(lastOsd(host), "置顶: 播放时");
  assert.equal(host.mainWindow.onTop, false);
  await play(host);
  assert.equal(host.mainWindow.onTop, true);
  host.setSource({ paused: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  host.setSource({ paused: false });
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  host.setSource({ phase: "ended" });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  await stop();
  assert.equal(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).onTop, "playing");

  const again = new TestHost();
  await start(again, data);
  assert.equal(again.command("top.playing").state.checked, true);
  await again.press("Ctrl+T"); // playing → fullscreen
  await again.settle();
  assert.equal(lastOsd(again), "置顶: 全屏播放时");
  await again.press("Ctrl+T"); // fullscreen → never
  await again.settle();
  assert.equal(lastOsd(again), "置顶: 关闭");
  await again.press("Ctrl+T");
  await again.settle();
  assert.equal(again.mainWindow.onTop, true);
  assert.equal(lastOsd(again), "置顶: 总是");
});

test("H05: only changes are sent; a refusing (headless) player does not break the menu", async () => {
  const host = new TestHost();
  host.handle("window.setAlwaysOnTop", () => {
    throw new HostError("unavailable", "headless");
  });
  await start(host);
  await host.command("top.always").run();
  await host.command("top.always").run();
  await host.settle();
  assert.equal(host.sent("window.setAlwaysOnTop").length, 1);
  assert.equal(state(host, "top.always").checked, true);
});

test("H05: a saved mode and cursor choice are kept; always on top is applied at activation", async () => {
  const data = mkdtempSync(join(scratch, "top-start-"));
  writeFileSync(join(data, "settings.json"), JSON.stringify({ onTop: "always", hideCursorWhilePlaying: false }));
  const host = new TestHost();
  await start(host, data);
  assert.equal(host.mainWindow.onTop, true);
  assert.equal(state(host, "hide-cursor").checked, false);
  assert.deepEqual(host.sent("window.setCursorAutoHide"), []);
  assert.equal(state(host, "auto.1").checked, true, "a field missing from an older file takes its default");
});

// ------------------------------------------------------------------ B30 / G-06

test("B30 / G-06: on by default; the toggle sets the core's cursor mode, says so, and is kept after a restart", async () => {
  const data = mkdtempSync(join(scratch, "cursor-"));
  const host = new TestHost();
  const { stop } = await start(host, data);
  assert.equal(state(host, "hide-cursor").checked, true);
  assert.equal(host.mainWindow.cursorAutoHide, "playing");
  await host.command("hide-cursor").run();
  await host.settle();
  assert.equal(host.mainWindow.cursorAutoHide, "fullscreen");
  assert.equal(lastOsd(host), "播放时隐藏鼠标: 关");
  assert.equal(state(host, "hide-cursor").checked, false);
  await stop();

  const again = new TestHost();
  await start(again, data);
  assert.deepEqual(again.sent("window.setCursorAutoHide"), []);
  assert.equal(again.mainWindow.cursorAutoHide, "fullscreen");
  await again.command("hide-cursor").run();
  await again.settle();
  assert.equal(again.mainWindow.cursorAutoHide, "playing");
  assert.equal(lastOsd(again), "播放时隐藏鼠标: 开");
});

// ------------------------------------------------------------------ 迷你尺寸 (IA-WIN-01, IA-WIN-02)

test("mini: ` shrinks to the mini box without floating, on every system; again restores the size", async () => {
  for (const host of [new TestHost(), withoutPip()]) {
    await manual(host);
    host.setMainWindow({ videoArea: { width: 1280, height: 720 } });
    await play(host, { width: 1920, height: 1080 });
    await host.press("`");
    await host.settle();
    assert.deepEqual(host.mainWindow.videoArea, { width: 400, height: 225 });
    assert.equal(state(host, "mini").checked, true);
    assert.equal(lastOsd(host), "迷你尺寸: 开");
    assert.equal(state(host, "size.1").checked, false);
    assert.equal(host.mainWindow.pictureInPicture.active, false);
    await host.press("`");
    await host.settle();
    assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
    assert.equal(state(host, "mini").checked, false);
    assert.equal(lastOsd(host), "迷你尺寸: 关");
    assert.deepEqual(host.sent("window.setAlwaysOnTop"), [], "IA-WIN-02: 迷你尺寸 never floats the window");
    assert.deepEqual(host.sent("window.setPictureInPicture"), []);
  }
});

test("mini: the on-top mode alone decides floating", async () => {
  const { host } = await manual();
  await play(host, { width: 1920, height: 1080 });
  await host.command("top.always").run();
  await host.press("`");
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  await host.press("`");
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  await host.command("top.never").run();
  await host.press("`");
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
});

test("mini: ends by itself when the user resizes or goes fullscreen; a size command ends it too", async () => {
  const { host } = await manual();
  await play(host, { width: 1920, height: 1080 });
  await host.press("`");
  await host.settle();
  host.setMainWindow({ videoArea: { width: 600, height: 340 } });
  await host.settle();
  assert.equal(state(host, "mini").checked, false);

  await host.press("`");
  await host.settle();
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.equal(state(host, "mini").checked, false);
  host.setMainWindow({ fullscreen: false });
  await host.settle();

  await host.press("`");
  await host.press("2");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1920, height: 1080 });
  assert.equal(state(host, "mini").checked, false);
});

test("mini: can be left after the source lost its video", async () => {
  const { host } = await manual();
  host.setMainWindow({ videoArea: { width: 800, height: 450 } });
  await play(host);
  await host.press("`");
  await play(host, null);
  assert.equal(state(host, "mini").enabled, true);
  await host.press("`");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 800, height: 450 });
});

// ------------------------------------------------------------------ H07 画中画 (IA-WIN-01)

test("H07: 屏幕 ▶ 画中画 opens and closes picture-in-picture; the check follows; no resize, no floating", async () => {
  const { host } = await manual();
  assert.deepEqual([state(host, "pip").enabled, state(host, "pip").reason], [false, "没有视频"]);
  await play(host, null);
  assert.equal(state(host, "pip").enabled, false);
  await play(host);
  assert.deepEqual([state(host, "pip").enabled, state(host, "pip").checked], [true, false]);
  const area = { ...host.mainWindow.videoArea };
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.mainWindow.pictureInPicture.active, true);
  assert.equal(state(host, "pip").checked, true);
  assert.equal(lastOsd(host), "画中画: 开");
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.mainWindow.pictureInPicture.active, false);
  assert.equal(state(host, "pip").checked, false);
  assert.equal(lastOsd(host), "画中画: 关");
  assert.deepEqual(host.sent("window.setPictureInPicture").map((p) => p.on), [true, false]);
  assert.deepEqual(host.mainWindow.videoArea, area);
  assert.deepEqual(host.sent("window.setVideoArea"), []);
  assert.deepEqual(host.sent("window.setAlwaysOnTop"), []);
  assert.equal(host.osd.filter((text) => text.startsWith("画中画")).length, 2);
});

test("H07: available in fullscreen (the core leaves it first)", async () => {
  const { host } = await manual();
  await play(host);
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.equal(state(host, "pip").enabled, true);
  await host.command("pip").run();
  await host.settle();
  assert.deepEqual([host.mainWindow.fullscreen, host.mainWindow.pictureInPicture.active], [false, true]);
});

test("H07: closed from the small window or by the core: the check clears and it is said once", async () => {
  const { host } = await manual();
  await play(host);
  await host.command("pip").run();
  await host.settle();
  // 返回 / close button in the small window.
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle(20);
  assert.equal(state(host, "pip").checked, false);
  assert.equal(lastOsd(host), "画中画: 关");

  await host.command("pip").run();
  await host.settle();
  // The new source has no video: the core closes the window.
  await play(host, null);
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle(20);
  assert.equal(lastOsd(host), "画中画: 关（没有视频）");
  assert.equal(state(host, "pip").enabled, false);
});

test("H07: a refusal is shown and the check stays; busy is ignored", async () => {
  const { host } = await manual();
  await play(host);
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("failed", "系统拒绝");
  });
  await host.command("pip").run();
  await host.settle();
  assert.equal(lastOsd(host), "无法进入画中画：系统拒绝");
  assert.equal(state(host, "pip").checked, false);
  const shown = host.osd.length;
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("busy", "正在切换");
  });
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.osd.length, shown);
});

test("H07: busy while the switch is in flight; a second run sends nothing", async () => {
  const { host } = await manual();
  await play(host);
  let release!: () => void;
  host.handle("window.setPictureInPicture", async (p) => {
    await new Promise<void>((resolve) => (release = resolve));
    host.setMainWindow({ pictureInPicture: { supported: true, active: p.on === true } });
  });
  const first = host.command("pip").run();
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual([state(host, "pip").busy, state(host, "pip").reason], [true, "正在切换画中画"]);
  assert.deepEqual([state(host, "mini").busy, state(host, "mini").reason], [true, "正在切换画中画"]);
  await assert.rejects(host.command("pip").run());
  await assert.rejects(host.press("`"));
  release();
  await first;
  await host.settle();
  assert.equal(host.sent("window.setPictureInPicture").length, 1);
  assert.deepEqual([state(host, "pip").busy, state(host, "pip").checked, lastOsd(host)], [false, true, "画中画: 开"]);
  assert.equal(host.osd.filter((text) => text.startsWith("画中画")).length, 1);
});

test("H07: 画中画 appears once the main window reports support, and goes away with it; ` stays 迷你尺寸", async () => {
  const { host } = await manual(withoutPip());
  assert.equal(host.commands.has("pip"), false);
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle();
  const pip = host.command("pip").declaration;
  assert.deepEqual([pip.path?.join(" ▶ "), pip.order, pip.group, pip.bindings], ["屏幕 ▶ 画中画", 709, "screen", []]);
  assert.equal(host.command("mini").declaration.path?.join(" ▶ "), "屏幕 ▶ 迷你尺寸");
  assert.deepEqual(host.conflicts, []);
  await play(host);
  await host.press("`");
  await host.settle();
  assert.deepEqual(host.sent("window.setPictureInPicture"), [], "` is 迷你尺寸");
  assert.equal(state(host, "mini").checked, true);
  // The window goes away again (headless): no 画中画.
  host.setMainWindow({ pictureInPicture: { supported: false, active: false } });
  await host.settle();
  assert.equal(host.commands.has("pip"), false);
  assert.equal(state(host, "mini").checked, true);
});

test("IA-WIN-01: 画中画 and 迷你尺寸 exclude each other: each leaves the other first", async () => {
  const { host } = await manual();
  host.setMainWindow({ videoArea: { width: 1280, height: 720 } });
  await play(host, { width: 1920, height: 1080 });
  await host.press("`");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 400, height: 225 });
  // 画中画 while 迷你尺寸 is on: the size before it comes back first.
  await host.command("pip").run();
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
  assert.deepEqual([state(host, "mini").checked, state(host, "pip").checked], [false, true]);
  // ` in 画中画: 画中画 ends, then 迷你尺寸.
  await host.press("`");
  await host.settle();
  assert.deepEqual(host.sent("window.setPictureInPicture").map((p) => p.on), [true, false]);
  assert.deepEqual([state(host, "mini").checked, state(host, "pip").checked], [true, false]);
  assert.deepEqual(host.mainWindow.videoArea, { width: 400, height: 225 });
  // 画中画 cannot be left: 迷你尺寸 does not start.
  await host.press("`");
  await host.command("pip").run();
  await host.settle();
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("failed", "系统拒绝");
  });
  await host.press("`");
  await host.settle();
  assert.deepEqual([state(host, "mini").checked, state(host, "pip").checked], [false, true]);
  assert.equal(lastOsd(host), "无法退出画中画：系统拒绝");
});
