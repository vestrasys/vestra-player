import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { Locale } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { clampNote, currentScale, fitted, mini, near, scaled } from "../geometry.ts";
import activate, { PAGE_ID } from "../main.ts";
import { normalize, timing } from "../settings.ts";
import { displaySize } from "../video-size.ts";

Object.assign(timing, { probeRetry: 10, probeAttempts: 5 });
const realLater = timing.later;

/**
 * A clock the test advances: the plugin's delays (`timing.later`) wait until
 * `fire()` instead of for real time, so a busy machine cannot run them early
 * or late. Restored after each test.
 */
function manualClock(host: TestHost) {
  const pending: { run: () => void; ms: number }[] = [];
  timing.later = (run, ms) => {
    const entry = { run, ms };
    pending.push(entry);
    return () => {
      const index = pending.indexOf(entry);
      if (index >= 0) pending.splice(index, 1);
    };
  };
  return {
    pending,
    /** Run every delay due now, then wait for what they asked the host. */
    async fire() {
      for (const entry of pending.splice(0)) entry.run();
      await host.settle();
    },
  };
}

/**
 * Hold the host's answer to `method` until `release()`. `reached` resolves
 * once the plugin's request is with the host: wait for that, not for time.
 */
function hold(host: TestHost, method: string, answer: (params: Record<string, unknown>) => unknown) {
  let release!: () => void;
  const released = new Promise<void>((resolve) => (release = resolve));
  let reach!: () => void;
  const reached = new Promise<void>((resolve) => (reach = resolve));
  host.handle(method, async (params) => {
    reach();
    await released;
    return answer(params);
  });
  return { reached, release };
}

/** Wait (in event-loop turns, not time) until the player has received a state that passes `ready`. */
async function published(ready: () => boolean) {
  for (let turn = 0; turn < 1000 && !ready(); turn++) await new Promise((resolve) => setImmediate(resolve));
}

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-window-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const running = new Set<() => Promise<void>>();
/** Start in English (the source texts, and the player's English) unless a locale is given. */
async function start(host = new TestHost(), dataDirectory?: string, locale: Locale = "en") {
  const started = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
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
  timing.later = realLater;
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
  assert.equal(clampNote(asked, { ...asked, clamped: false }), null);
  assert.equal(clampNote(asked, { width: 1600, height: 900, clamped: true }), "shrunk");
  assert.equal(clampNote({ width: 160, height: 90 }, { width: 318, height: 120, clamped: true }), "enlarged");
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


test("settings: the spec defaults; a bad field falls back to its default, the others are kept", () => {
  assert.deepEqual(normalize({}), {
    onTop: "never",
    onTopChosen: false,
    hideCursorWhilePlaying: true,
    autoSize: 1,
    autoSizeOnce: false,
  });
  assert.deepEqual(normalize({ onTop: "sometimes", hideCursorWhilePlaying: false, autoSize: "fit", autoSizeOnce: true }), {
    onTop: "never",
    onTopChosen: false,
    hideCursorWhilePlaying: false,
    autoSize: "fit",
    autoSizeOnce: true,
  });
  assert.deepEqual(normalize({ onTop: "playing", hideCursorWhilePlaying: "yes", autoSize: 3, autoSizeOnce: 1 }), {
    onTop: "playing",
    onTopChosen: true,
    hideCursorWhilePlaying: true,
    autoSize: 1,
    autoSizeOnce: false,
  });
  assert.equal(normalize({ autoSize: "off" }).autoSize, "off");
  assert.equal(normalize({ autoSize: 0.5 }).autoSize, 0.5);
});

test("IA-WIN-03 (ruling 2026-11-15): the default is Never; only a mode the user chose is kept", () => {
  const onTop = (value: Record<string, unknown>) => {
    const { onTop, onTopChosen } = normalize(value);
    return [onTop, onTopChosen];
  };
  // No file, or a file without the field: the default.
  assert.deepEqual(onTop({}), ["never", false]);
  assert.deepEqual(onTop({ autoSize: 2 }), ["never", false]);
  // A file of 0.1.8 or older: the whole object was saved whenever any setting changed, so the old
  // default there is untouched and becomes the new one; any other mode was chosen.
  assert.deepEqual(onTop({ onTop: "fullscreen", hideCursorWhilePlaying: true, autoSize: 1, autoSizeOnce: true }), [
    "never",
    false,
  ]);
  for (const mode of ["never", "always", "playing"]) assert.deepEqual(onTop({ onTop: mode }), [mode, true], mode);
  // From 0.1.9 the file says whether the mode was chosen, for every mode.
  for (const mode of ["never", "always", "playing", "fullscreen"]) {
    assert.deepEqual(onTop({ onTop: mode, onTopChosen: true }), [mode, true], mode);
    assert.deepEqual(onTop({ onTop: mode, onTopChosen: false }), ["never", false], mode);
  }
  // A bad mode is never a choice, whatever the marker says.
  assert.deepEqual(onTop({ onTop: "sometimes", onTopChosen: true }), ["never", false]);
  assert.deepEqual(onTop({ onTop: 3 }), ["never", false]);
  // A bad marker: as an older file.
  assert.deepEqual(onTop({ onTop: "fullscreen", onTopChosen: "yes" }), ["never", false]);
  assert.deepEqual(onTop({ onTop: "always", onTopChosen: 1 }), ["always", true]);
});

// ------------------------------------------------------------------ menus

const rows = (host: TestHost) =>
  [...host.commands.entries()].map(([id, c]) => [
    id,
    c.display.title,
    c.declaration.order,
    c.declaration.group,
    c.declaration.bindings.join(","),
  ]);

test("menus: Picture-in-Picture first in Tools ▸, the rest palette only (menu sweep B5); titles, orders, groups and bindings; no conflicts", async () => {
  const { host } = await start();
  // User ruling 2026-11-27: Picture-in-Picture is the first item of Tools ▸
  // (order 300, before the player's Audio Track ▸ at 301; group "domain",
  // as the tracks and Snapshot ▸, so no separator).
  for (const [id, c] of host.commands) {
    const [declared, shown] = id === "pip" ? [["@tools", "Picture-in-Picture"], ["Tools", "Picture-in-Picture"]] : [[], []];
    assert.deepEqual([c.declaration.path, c.display.path], [declared, shown], `${id} menu path`);
  }
  assert.deepEqual(rows(host), [
    ["mini", "Mini Size", 710, "screen", "`"],
    ["pip", "Picture-in-Picture", 300, "domain", ""],
    ["size.0.5", "0.5×", 711, "screen-size", "1"],
    ["size.1", "1×", 712, "screen-size", "2"],
    ["size.1.5", "1.5×", 713, "screen-size", "3"],
    ["size.2", "2×", 714, "screen-size", "4"],
    ["fit", "Fit to Video Aspect Ratio", 715, "screen-size", "8"],
    ["auto.off", "Don't Resize", 716, "screen-auto", ""],
    ["auto.0.5", "0.5×", 717, "screen-auto", ""],
    ["auto.1", "1×", 718, "screen-auto", ""],
    ["auto.1.5", "1.5×", 719, "screen-auto", ""],
    ["auto.2", "2×", 720, "screen-auto", ""],
    ["auto.fit", "Fit to Video Aspect Ratio", 721, "screen-auto", ""],
    ["auto.once", "Only Once", 722, "screen-auto-once", ""],
    ["top.cycle", "Cycle Mode", 725, "screen-top", "Ctrl+T"],
    ["top.never", "Never", 726, "screen-top-mode", ""],
    ["top.always", "Always", 727, "screen-top-mode", ""],
    ["top.playing", "While Playing", 728, "screen-top-mode", ""],
    ["top.fullscreen", "While Playing in Fullscreen", 729, "screen-top-mode", ""],
    ["hide-cursor", "Hide Pointer While Playing", 241, "playback-options", ""],
  ]);
  for (const id of ["size.1", "auto.1", "top.never"]) assert.equal(host.command(id).declaration.radio, true);
  assert.equal(host.command("auto.once").declaration.radio, false);
  assert.deepEqual(host.conflicts, []);
  // First start: 1倍, 从不置顶 (IA-WIN-03, ruling 2026-11-15), 隐藏指针 checked.
  assert.deepEqual(
    ["auto.1", "auto.once", "top.never", "top.fullscreen", "hide-cursor", "mini", "pip"].map((id) => state(host, id).checked),
    [true, false, true, false, true, false, false],
  );
  // Only the cursor mode differs from the core's default; nothing floats in a window.
  assert.deepEqual(host.sent("window.setAlwaysOnTop"), []);
  assert.deepEqual(host.sent("window.setCursorAutoHide").map((p) => p.mode), ["playing"]);
  assert.equal(host.mainWindow.cursorAutoHide, "playing");
});

test("palette: every command has a category, the same one within a submenu; no home", async () => {
  const { host } = await start();
  const categories = [...host.commands.entries()].map(([id, c]) => [id, c.display.category]);
  for (const [id, category] of categories) assert.ok(typeof category === "string" && category.length > 0, `${id} has a category`);
  assert.deepEqual(categories, [
    ["mini", "Window"],
    ["pip", "Window"],
    ["size.0.5", "Window Size"],
    ["size.1", "Window Size"],
    ["size.1.5", "Window Size"],
    ["size.2", "Window Size"],
    ["fit", "Window Size"],
    ["auto.off", "Size on Open"],
    ["auto.0.5", "Size on Open"],
    ["auto.1", "Size on Open"],
    ["auto.1.5", "Size on Open"],
    ["auto.2", "Size on Open"],
    ["auto.fit", "Size on Open"],
    ["auto.once", "Size on Open"],
    ["top.cycle", "Always on Top"],
    ["top.never", "Always on Top"],
    ["top.always", "Always on Top"],
    ["top.playing", "Always on Top"],
    ["top.fullscreen", "Always on Top"],
    ["hide-cursor", "Window"],
  ]);
  for (const [id, c] of host.commands) assert.equal(c.declaration.home, undefined, id);
});

test("palette: commands that can be disabled say why (the palette hides them otherwise)", async () => {
  const { host } = await start();
  // No video: Mini Size, Picture-in-Picture, the sizes and Fit are disabled, each with a reason.
  for (const id of ["mini", "pip", "size.0.5", "size.1", "size.1.5", "size.2", "fit"]) {
    assert.equal(state(host, id).enabled, false, id);
    assert.equal(host.command(id).display.reason, "No video", id);
  }
});

test("menus: without system picture-in-picture there is no 画中画; ` is still 迷你尺寸", async () => {
  const { host } = await start(withoutPip());
  assert.equal(host.commands.has("pip"), false);
  const mini = host.command("mini");
  assert.deepEqual([mini.display.title, mini.declaration.order, mini.declaration.group, mini.declaration.bindings], ["Mini Size", 710, "screen", ["`"]]);
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
  host.setSource({ phase: "ended" });
  host.setSource({ phase: "loading", position: null }); // a restart: the same source through loading
  host.setSource({ phase: "ready", position: 0 });
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

test("G-02: not while maximized (MPC-HC, mpv); restored, the next file is sized; a size command still sizes it", async () => {
  const { host } = await start();
  host.setMainWindow({ maximized: true, videoArea: { width: 2560, height: 1340 } });
  await host.settle();
  await play(host, { width: 1280, height: 720 });
  await play(host, { width: 640, height: 480 });
  assert.deepEqual(host.sent("window.setVideoArea"), [], "a maximized window is left alone");
  assert.deepEqual(host.mainWindow.videoArea, { width: 2560, height: 1340 });

  // Fullscreen from maximized and back: still maximized, still left alone.
  host.setMainWindow({ fullscreen: true, videoArea: { width: 2560, height: 1440 } });
  host.setMainWindow({ fullscreen: false, videoArea: { width: 2560, height: 1340 } });
  await play(host, { width: 1920, height: 1080 });
  assert.deepEqual(host.sent("window.setVideoArea"), []);

  // Restored: the next file is sized again.
  host.setMainWindow({ maximized: false, videoArea: { width: 800, height: 450 } });
  await play(host, { width: 1280, height: 720 });
  assert.deepEqual(host.sent("window.setVideoArea"), [{ width: 1280, height: 720 }]);

  // An explicit size command while maximized is sent (on Windows the player restores the window first).
  host.setMainWindow({ maximized: true, videoArea: { width: 2560, height: 1340 } });
  await host.settle();
  await host.command("size.1").run();
  await host.settle();
  assert.deepEqual(host.sent("window.setVideoArea").at(-1), { width: 1280, height: 720 });
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
  assert.equal(lastOsd(host), "When playback starts: 2×");
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
  assert.equal(lastOsd(host), "When playback starts: Don't resize the window");
  const sent = host.sent("window.setVideoArea").length;
  await play(host, { width: 640, height: 360 });
  assert.equal(host.sent("window.setVideoArea").length, sent);
  await host.command("auto.1").run();
  await host.command("auto.once").run();
  assert.equal(lastOsd(host), "Resize only once when playback starts: On");
  assert.equal(state(host, "auto.once").checked, true);
  await stop();
  assert.deepEqual(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")), {
    onTop: "never",
    onTopChosen: false,
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
  assert.deepEqual([page.category, page.title, page.order, page.file], ["playback", "Window", 30, "ui/page.slint"]);
  assert.deepEqual(
    [page.props["auto-index"], page.props["auto-once"], page.props["top-index"], page.props["hide-cursor"]],
    [2, false, 0, true],
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

test("settings page: the radios and checks are the library's, bound two-way to the page's state", () => {
  const slint = readFileSync(new URL("../ui/page.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{ Check, RadioGroup \} from "@vestra\/widgets\.slint";/);
  assert.doesNotMatch(slint, /std-widgets|CheckBox|component Radio|component Choices/, "no std or hand-rolled controls left");
  // Each RadioGroup keeps the page's index (two-way) and reports a pick with the index.
  for (const [index, callback] of [["auto-index", "set-auto"], ["top-index", "set-top"]]) {
    assert.match(slint, new RegExp(`current-index <=> WindowPage\\.${index};\\s*selected\\(index\\) => \\{ WindowPage\\.${callback}\\(index\\); \\}`));
  }
  // Each Check flips itself (two-way) and reports the new state; no fixed height, so a long label wraps.
  for (const [prop, callback] of [["auto-once", "set-auto-once"], ["hide-cursor", "set-hide-cursor"]]) {
    assert.match(slint, new RegExp(`checked <=> WindowPage\\.${prop};\\s*toggled\\(on\\) => \\{ WindowPage\\.${callback}\\(on\\); \\}`));
  }
  assert.doesNotMatch(slint, /checkbox-row-height/);
});

// ------------------------------------------------------------------ H03

test("H03: disabled without video and in fullscreen; enabled once the video size is known", async () => {
  const { host } = await manual(withoutPip());
  assert.deepEqual(
    [state(host, "size.1").enabled, state(host, "size.1").reason, state(host, "fit").enabled, state(host, "mini").enabled],
    [false, "No video", false, false],
  );
  await play(host, null); // audio only
  assert.equal(state(host, "size.2").reason, "No video");
  await play(host);
  assert.equal(state(host, "size.2").enabled, true);
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.deepEqual([state(host, "size.2").enabled, state(host, "size.2").reason], [false, "Not available in fullscreen"]);
  assert.deepEqual([state(host, "mini").enabled, state(host, "fit").reason], [false, "Not available in fullscreen"]);
  host.setMainWindow({ fullscreen: false });
  await host.settle();
  host.setSource({ phase: "empty" });
  await host.settle();
  assert.equal(state(host, "size.2").reason, "No video");
});

test("H03: still enabled after the video ends and plays again (restart passes through loading, or a seek back)", async () => {
  const { host } = await manual(withoutPip());
  await play(host);
  assert.equal(state(host, "size.1").enabled, true);
  host.setSource({ phase: "ended", position: 60 });
  await host.settle();
  assert.equal(state(host, "size.1").enabled, true, "the last frame is still shown");
  // Space at EOF: the host restarts the same source (same id) through loading.
  host.setSource({ phase: "loading", position: null });
  await host.settle();
  assert.equal(state(host, "size.1").reason, "No video");
  host.setSource({ phase: "ready", position: 0 });
  await host.settle(30);
  assert.deepEqual([state(host, "size.1").enabled, state(host, "mini").enabled], [true, true]);
  // Ended again, then a seek back: the size is kept.
  host.setSource({ phase: "ended", position: 60 });
  await host.settle();
  host.setSource({ phase: "ready", position: 10 });
  await host.settle(30);
  assert.equal(state(host, "size.2").enabled, true);
});

test("H03: the size is read again while mpv has not reported it yet", async () => {
  const host = new TestHost();
  const clock = manualClock(host);
  await manual(host);
  await play(host, null);
  // The probe at `ready` found no video: one retry is waiting, after `probeRetry`.
  assert.deepEqual(
    clock.pending.map((entry) => entry.ms),
    [timing.probeRetry],
  );
  assert.equal(state(host, "size.1").reason, "No video");
  setVideo(host, { width: 1280, height: 720 });
  await clock.fire();
  assert.equal(state(host, "size.1").enabled, true);
  assert.equal(clock.pending.length, 0, "found: no more retries");
});

test("H03: the size is asked at most `probeAttempts` times", async () => {
  const host = new TestHost();
  const clock = manualClock(host);
  await manual(host);
  await play(host, null);
  let retries = 0;
  while (clock.pending.length > 0) {
    retries++;
    await clock.fire();
  }
  assert.equal(retries, timing.probeAttempts - 1);
  assert.equal(state(host, "size.1").reason, "No video");
});

test("H03: 1–4 set the video area to the display size times 0.5–2; the radio dot follows", async () => {
  const { host } = await manual();
  await play(host, { width: 720, height: 576, displayWidth: 1024, displayHeight: 576 });
  await host.press("2");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1024, height: 576 });
  assert.equal(lastOsd(host), "Window: 1×");
  assert.deepEqual(
    ["size.0.5", "size.1", "size.1.5", "size.2"].map((id) => state(host, id).checked),
    [false, true, false, false],
  );
  await host.press("3");
  await host.settle();
  assert.deepEqual(host.mainWindow.videoArea, { width: 1536, height: 864 });
  assert.equal(lastOsd(host), "Window: 1.5×");
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
  assert.equal(lastOsd(host), "Window: 0.5× (enlarged to the minimum size)");
  host.handle("window.setVideoArea", () => ({ width: 560, height: 315, clamped: true }));
  await host.press("4");
  await host.settle();
  assert.equal(lastOsd(host), "Window: 2× (shrunk to fit the screen)");
  host.handle("window.setVideoArea", () => {
    throw new HostError("unavailable", "Not available in fullscreen");
  });
  await host.press("2");
  await host.settle();
  assert.equal(lastOsd(host), "Cannot resize the window: Not available in fullscreen");
});

test("H03: busy while a resize is in flight", async () => {
  const { host } = await manual();
  await play(host);
  const { reached, release } = hold(host, "window.setVideoArea", (p) => ({
    width: Number(p.width),
    height: Number(p.height),
    clamped: false,
  }));
  const first = host.press("2");
  await reached; // settle() would wait for the held request
  await published(() => state(host, "size.2").busy);
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
  assert.equal(lastOsd(host), "Window: Fit to video aspect ratio");
  await host.press("8");
  await host.settle();
  assert.equal(host.sent("window.setVideoArea").length, 1);
  assert.equal(lastOsd(host), "The window already fits the video aspect ratio");
});

// ------------------------------------------------------------------ H05 / IA-WIN-03

test("IA-WIN-03 (ruling 2026-11-15): by default (从不) the player never floats, also playing in fullscreen", async () => {
  const { host } = await manual();
  await play(host);
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false, "playing in fullscreen");
  host.setMainWindow({ fullscreen: false });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  assert.deepEqual(host.sent("window.setAlwaysOnTop"), [], "nothing to send: the core's default is not on top");
});

test("IA-WIN-03: 全屏播放时置顶 (chosen) floats only while playing in fullscreen", async () => {
  const { host } = await manual();
  await host.command("top.fullscreen").run();
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

test("H05: modes by menu and Ctrl+T in `OnTopMode` order, OSD texts, playing mode follows pause, kept after a restart", async () => {
  const data = mkdtempSync(join(scratch, "top-"));
  const host = new TestHost();
  const { stop } = await start(host, data);
  await host.command("top.always").run();
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  assert.equal(lastOsd(host), "Always on top: Always");
  assert.deepEqual(
    ["top.never", "top.always", "top.playing", "top.fullscreen"].map((id) => state(host, id).checked),
    [false, true, false, false],
  );
  await host.press("Ctrl+T"); // always → playing, nothing playing
  await host.settle();
  assert.equal(lastOsd(host), "Always on top: While playing");
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
  assert.equal(lastOsd(again), "Always on top: While playing in fullscreen");
  await again.press("Ctrl+T"); // fullscreen → never
  await again.settle();
  assert.equal(lastOsd(again), "Always on top: Off");
  await again.press("Ctrl+T");
  await again.settle();
  assert.equal(again.mainWindow.onTop, true);
  assert.equal(lastOsd(again), "Always on top: Always");
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

test("IA-WIN-03: an existing user who never chose a mode gets Never; a chosen mode survives restarts", async () => {
  // A 0.1.8 file written when the user changed another setting: the old default was saved with it.
  const data = mkdtempSync(join(scratch, "top-old-"));
  const file = join(data, "settings.json");
  writeFileSync(file, JSON.stringify({ onTop: "fullscreen", hideCursorWhilePlaying: true, autoSize: 2, autoSizeOnce: false }));
  const host = new TestHost();
  const { stop } = await start(host, data);
  assert.equal(state(host, "top.never").checked, true);
  assert.equal(state(host, "auto.2").checked, true, "the other settings are kept");
  await play(host);
  host.setMainWindow({ fullscreen: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  // The user picks "While Playing in Fullscreen" on the settings page: kept as a choice.
  host.settingsPage(PAGE_ID).emit("set-top", 3);
  await host.settle();
  assert.equal(host.mainWindow.onTop, true);
  await stop();
  assert.deepEqual(
    (({ onTop, onTopChosen }) => [onTop, onTopChosen])(JSON.parse(readFileSync(file, "utf8"))),
    ["fullscreen", true],
  );
  const again = new TestHost();
  await start(again, data);
  assert.equal(again.command("top.fullscreen").state.checked, true, "an explicit choice of the old default stays");

  // A 0.1.8 file with a mode other than the old default: the user chose it.
  const other = mkdtempSync(join(scratch, "top-old-other-"));
  writeFileSync(join(other, "settings.json"), JSON.stringify({ onTop: "playing", hideCursorWhilePlaying: true }));
  const third = new TestHost();
  await start(third, other);
  assert.equal(third.command("top.playing").state.checked, true);
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
  assert.equal(lastOsd(host), "Hide pointer while playing: Off");
  assert.equal(state(host, "hide-cursor").checked, false);
  await stop();

  const again = new TestHost();
  await start(again, data);
  assert.deepEqual(again.sent("window.setCursorAutoHide"), []);
  assert.equal(again.mainWindow.cursorAutoHide, "fullscreen");
  await again.command("hide-cursor").run();
  await again.settle();
  assert.equal(again.mainWindow.cursorAutoHide, "playing");
  assert.equal(lastOsd(again), "Hide pointer while playing: On");
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
    assert.equal(lastOsd(host), "Mini size: On");
    assert.equal(state(host, "size.1").checked, false);
    assert.equal(host.mainWindow.pictureInPicture.active, false);
    await host.press("`");
    await host.settle();
    assert.deepEqual(host.mainWindow.videoArea, { width: 1280, height: 720 });
    assert.equal(state(host, "mini").checked, false);
    assert.equal(lastOsd(host), "Mini size: Off");
    assert.deepEqual(host.sent("window.setAlwaysOnTop"), [], "IA-WIN-02: Mini Size never floats the window");
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
  assert.deepEqual([state(host, "pip").enabled, state(host, "pip").reason], [false, "No video"]);
  await play(host, null);
  assert.equal(state(host, "pip").enabled, false);
  await play(host);
  assert.deepEqual([state(host, "pip").enabled, state(host, "pip").checked], [true, false]);
  const area = { ...host.mainWindow.videoArea };
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.mainWindow.pictureInPicture.active, true);
  assert.equal(state(host, "pip").checked, true);
  assert.equal(lastOsd(host), "Picture-in-picture: On");
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.mainWindow.pictureInPicture.active, false);
  assert.equal(state(host, "pip").checked, false);
  assert.equal(lastOsd(host), "Picture-in-picture: Off");
  assert.deepEqual(host.sent("window.setPictureInPicture").map((p) => p.on), [true, false]);
  assert.deepEqual(host.mainWindow.videoArea, area);
  assert.deepEqual(host.sent("window.setVideoArea"), []);
  assert.deepEqual(host.sent("window.setAlwaysOnTop"), []);
  assert.equal(host.osd.filter((text) => text.startsWith("Picture-in-picture")).length, 2);
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
  assert.equal(lastOsd(host), "Picture-in-picture: Off");

  await host.command("pip").run();
  await host.settle();
  // The new source has no video: the core closes the window.
  await play(host, null);
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle(20);
  assert.equal(lastOsd(host), "Picture-in-picture: Off (no video)");
  assert.equal(state(host, "pip").enabled, false);
});

test("H07: a refusal is shown and the check stays; busy is ignored", async () => {
  const { host } = await manual();
  await play(host);
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("failed", "refused by the system");
  });
  await host.command("pip").run();
  await host.settle();
  assert.equal(lastOsd(host), "Cannot enter picture-in-picture: refused by the system");
  assert.equal(state(host, "pip").checked, false);
  const shown = host.osd.length;
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("busy", "switching");
  });
  await host.command("pip").run();
  await host.settle();
  assert.equal(host.osd.length, shown);
});

test("H07: busy while the switch is in flight; a second run sends nothing", async () => {
  const { host } = await manual();
  await play(host);
  const { reached, release } = hold(host, "window.setPictureInPicture", (p) => {
    host.setMainWindow({ pictureInPicture: { supported: true, active: p.on === true } });
    return null;
  });
  const first = host.command("pip").run();
  await reached;
  await published(() => state(host, "pip").busy);
  assert.deepEqual([state(host, "pip").busy, state(host, "pip").reason], [true, "Switching picture-in-picture"]);
  assert.deepEqual([state(host, "mini").busy, state(host, "mini").reason], [true, "Switching picture-in-picture"]);
  await assert.rejects(host.command("pip").run());
  await assert.rejects(host.press("`"));
  release();
  await first;
  await host.settle();
  assert.equal(host.sent("window.setPictureInPicture").length, 1);
  assert.deepEqual([state(host, "pip").busy, state(host, "pip").checked, lastOsd(host)], [false, true, "Picture-in-picture: On"]);
  assert.equal(host.osd.filter((text) => text.startsWith("Picture-in-picture")).length, 1);
});

test("H07: 画中画 appears once the main window reports support, and goes away with it; ` stays 迷你尺寸", async () => {
  const { host } = await manual(withoutPip());
  assert.equal(host.commands.has("pip"), false);
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle();
  const pip = host.command("pip");
  assert.deepEqual(
    [pip.display.title, pip.declaration.path, pip.declaration.order, pip.declaration.group, pip.declaration.bindings],
    ["Picture-in-Picture", ["@tools", "Picture-in-Picture"], 300, "domain", []],
  );
  assert.equal(host.command("mini").display.title, "Mini Size");
  assert.deepEqual(host.conflicts, []);
  await play(host);
  await host.press("`");
  await host.settle();
  assert.deepEqual(host.sent("window.setPictureInPicture"), [], "` is Mini Size");
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
    throw new HostError("failed", "refused by the system");
  });
  await host.press("`");
  await host.settle();
  assert.deepEqual([state(host, "mini").checked, state(host, "pip").checked], [false, true]);
  assert.equal(lastOsd(host), "Cannot exit picture-in-picture: refused by the system");
});

// ------------------------------------------------------------------ standby (events and triggers, B3)

test("standby: nothing keeps the plugin running; woken mid-playback it neither sizes again nor loses on top", async () => {
  const { host } = await start();
  assert.deepEqual(host.keepsRunning, []);
  await host.command("top.playing").run();
  await play(host, { width: 1280, height: 720 });
  assert.deepEqual([host.mainWindow.videoArea, host.mainWindow.onTop], [{ width: 1280, height: 720 }, true]);
  host.setMainWindow({ videoArea: { width: 900, height: 506 } }); // the user's own size
  await host.settle();
  assert.deepEqual(host.keepsRunning, []);

  await host.park();
  assert.equal(host.parked, true);
  assert.equal(host.mainWindow.onTop, true, "the player keeps the setting through standby");
  // A wake event starts it again: the source already playing is not sized a second time.
  host.setMainWindow({ fullscreen: true });
  await host.settle(30);
  assert.equal(host.parked, false);
  host.setMainWindow({ fullscreen: false, videoArea: { width: 900, height: 506 } });
  await host.settle(30);
  assert.deepEqual(host.sent("window.setVideoArea"), [{ width: 1280, height: 720 }]);
  assert.equal(state(host, "size.1").enabled, true, "the size is known again after the wake");
  // The trigger's leave after the wake still lets go of on top.
  host.setSource({ paused: true });
  await host.settle();
  assert.equal(host.mainWindow.onTop, false);
  assert.deepEqual(host.sent("window.setAlwaysOnTop").map((p) => p.onTop), [true, false]);

  // Parked while paused, playing wakes it through the trigger.
  await host.park();
  host.setSource({ paused: false });
  await host.settle(30);
  assert.equal(host.mainWindow.onTop, true);
  // The next file is sized as before.
  await play(host, { width: 640, height: 480 });
  assert.deepEqual(host.mainWindow.videoArea, { width: 640, height: 480 });
});

test("video.reconfigured: a new shown size (a filter turned the picture) is the size the commands use", async () => {
  const { host } = await manual();
  await play(host, { width: 1920, height: 1080 });
  host.emit("video.reconfigured", {
    sourceId: host.source.stamp.generation,
    width: 1920,
    height: 1080,
    displayWidth: 1080,
    displayHeight: 1920,
    fps: 25,
    rotation: 90,
    hdr: false,
  });
  await host.settle();
  host.setMainWindow({ videoArea: { width: 540, height: 960 } });
  await host.settle();
  assert.equal(state(host, "size.0.5").checked, true);
});

// ------------------------------------------------------------------ zh-Hans

test("zh-Hans: the menus, the settings page and the OSD texts are the Chinese ones from before", async () => {
  const { host } = await start(new TestHost(), undefined, "zh-Hans");
  // Menu sweep (B5): palette only, so the title (and the category) is what a Chinese user reads.
  assert.deepEqual(
    [...host.commands.entries()].map(([id, c]) => [id, c.display.title]),
    [
      ["mini", "迷你尺寸"],
      ["pip", "画中画"],
      ["size.0.5", "0.5倍"],
      ["size.1", "1倍"],
      ["size.1.5", "1.5倍"],
      ["size.2", "2倍"],
      ["fit", "适应视频比例"],
      ["auto.off", "不调整"],
      ["auto.0.5", "0.5倍"],
      ["auto.1", "1倍"],
      ["auto.1.5", "1.5倍"],
      ["auto.2", "2倍"],
      ["auto.fit", "适应视频比例"],
      ["auto.once", "只调整一次"],
      ["top.cycle", "切换置顶方式"],
      ["top.never", "不置顶"],
      ["top.always", "总是置顶"],
      ["top.playing", "播放时置顶"],
      ["top.fullscreen", "全屏播放时置顶"],
      ["hide-cursor", "播放时隐藏鼠标指针"],
    ],
  );
  // The one menu item (user ruling 2026-11-27): 工具 ▸ 画中画.
  assert.deepEqual(host.command("pip").display.path, ["工具", "画中画"]);
  assert.equal(host.settingsPage(PAGE_ID).title, "窗口");
  // The palette categories: the page's and the submenu's words, and the two new ones.
  assert.deepEqual(
    ["mini", "size.1", "auto.once", "top.cycle", "hide-cursor"].map((id) => host.command(id).display.category),
    ["窗口", "窗口尺寸", "开始播放时的尺寸", "置顶", "窗口"],
  );

  // G-02, H05, B30 by menu.
  await host.command("auto.2").run();
  await host.command("auto.fit").run();
  await host.command("auto.off").run();
  await host.command("auto.once").run();
  await host.command("auto.once").run();
  await host.command("top.never").run();
  for (let i = 0; i < 4; i++) await host.press("Ctrl+T");
  await host.command("hide-cursor").run();
  await host.command("hide-cursor").run();
  await host.settle();
  assert.deepEqual(host.osd.splice(0), [
    "开始播放时: 2倍",
    "开始播放时: 适应视频比例",
    "开始播放时: 不调整窗口",
    "开始播放时只调整一次: 开",
    "开始播放时只调整一次: 关",
    "置顶: 关闭",
    "置顶: 总是",
    "置顶: 播放时",
    "置顶: 全屏播放时",
    "置顶: 关闭",
    "播放时隐藏鼠标: 关",
    "播放时隐藏鼠标: 开",
  ]);

  // H03 / H04 / mini.
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  await play(host, { width: 320, height: 180 });
  await host.press("1"); // 160×90 → the core's minimum
  await host.settle();
  host.handle("window.setVideoArea", () => ({ width: 560, height: 315, clamped: true }));
  await host.press("4");
  await host.settle();
  host.handle("window.setVideoArea", (p) => ({ width: Number(p.width), height: Number(p.height), clamped: false }));
  await host.press("2");
  await host.settle();
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  await host.press("8");
  await host.settle();
  host.setMainWindow({ videoArea: { width: 800, height: 450 } });
  await host.press("8"); // already fits
  await host.settle();
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  host.handle("window.setVideoArea", () => ({ width: 400, height: 225, clamped: true }));
  await host.press("8");
  await host.settle();
  host.setMainWindow({ videoArea: { width: 800, height: 800 } });
  host.handle("window.setVideoArea", () => ({ width: 900, height: 506, clamped: true }));
  await host.press("8");
  await host.settle();
  host.handle("window.setVideoArea", (p) => ({ width: Number(p.width), height: Number(p.height), clamped: false }));
  await host.press("`");
  await host.settle();
  await host.press("`");
  await host.settle();
  host.handle("window.setVideoArea", () => {
    throw new HostError("unavailable", "全屏时不可用");
  });
  await host.press("2");
  await host.settle();
  assert.deepEqual(host.osd.splice(0), [
    "窗口: 0.5倍（已按最小尺寸放大）",
    "窗口: 2倍（已按屏幕缩小）",
    "窗口: 1倍",
    "窗口: 适应视频比例",
    "窗口已适应视频比例",
    "窗口: 适应视频比例（已按屏幕缩小）",
    "窗口: 适应视频比例（已按最小尺寸放大）",
    "迷你尺寸: 开",
    "迷你尺寸: 关",
    "无法调整窗口：全屏时不可用",
  ]);

  // H07.
  await host.command("pip").run();
  await host.settle();
  await host.command("pip").run();
  await host.settle();
  await host.command("pip").run();
  await host.settle();
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle(20);
  await host.command("pip").run();
  await host.settle();
  await play(host, null);
  host.setMainWindow({ pictureInPicture: { supported: true, active: false } });
  await host.settle(20);
  await play(host);
  host.handle("window.setPictureInPicture", () => {
    throw new HostError("failed", "系统拒绝");
  });
  await host.command("pip").run();
  await host.settle();
  host.setMainWindow({ pictureInPicture: { supported: true, active: true } });
  await host.settle();
  await host.command("pip").run();
  await host.settle();
  assert.deepEqual(host.osd.splice(0), [
    "画中画: 开",
    "画中画: 关",
    "画中画: 开",
    "画中画: 关",
    "画中画: 开",
    "画中画: 关（没有视频）",
    "无法进入画中画：系统拒绝",
    "无法退出画中画：系统拒绝",
  ]);
});

test("zh-Hans: the reasons and the settings page texts are translated", async () => {
  const { host } = await start(new TestHost(), undefined, "zh-Hans");
  for (const text of [
    "No video",
    "Not available in fullscreen",
    "Resizing the window",
    "Switching picture-in-picture",
    "Window size when playback starts",
    "Off",
    "Fit aspect ratio",
    "Only once (only the first video of this run)",
    "Always on top",
    "Never",
    "Always",
    "While playing",
    "Playing in fullscreen",
    "Mouse pointer",
    "Hide the pointer while playing",
  ])
    assert.notEqual(host.translate(text), text, text);
  assert.deepEqual(
    ["No video", "Not available in fullscreen", "Resizing the window", "Switching picture-in-picture"].map((text) => host.translate(text)),
    ["没有视频", "全屏时不可用", "正在调整窗口", "正在切换画中画"],
  );
  assert.equal(
    host.translate("Each new file is resized once, when its video size is known, keeping the window's top left corner. Not in fullscreen, maximized, mini size or picture-in-picture; after you drag the window while it plays, that file is left alone."),
    "每个新文件拿到视频尺寸时调整一次，保持窗口左上角。全屏、最大化、迷你尺寸、画中画时不调整；播放中拖动过窗口后，这个文件不再调整。",
  );
});
