import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import { shorten } from "../copy.ts";
import activate from "../main.ts";
import { NO_VIDEO } from "../keyframes.ts";
import { FIRST_LINE_ONLY, INVALID, parseAddress } from "../open-url.ts";
import { parseStep, RANGE, stepText } from "../time-jump.ts";
const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
function withMedia(host = new TestHost(), source = "/movie.mp4") {
  host.openElsewhere(source);
  host.setSource({ duration: 100, position: 0 });
  return host;
}
async function until(predicate: () => boolean) {
  const deadline = Date.now() + 1500;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const intents = (host: TestHost) => host.sent("source.submit").map((p) => p.intent as Record<string, unknown>);
const seeks = (host: TestHost) =>
  intents(host)
    .filter((i) => i.kind === "control" && (i.control as { kind: string }).kind === "seek")
    .map((i) => (i.control as { seconds: number }).seconds);
const lastOsd = (host: TestHost) => host.osd.at(-1);
const state = (host: TestHost, id: string) => {
  const { enabled, checked, busy, reason } = host.command(id).state;
  return { enabled, checked, busy, reason };
};
/** A JSON file in the data directory once its debounced write satisfies `ready`. */
async function saved(file: string, ready: (value: Record<string, unknown>) => boolean = () => true) {
  let value: Record<string, unknown> | null = null;
  await until(() => {
    try {
      value = JSON.parse(readFileSync(file, "utf8"));
    } catch {
      return false;
    }
    return ready(value!);
  });
  return value!;
}
const URL_WINDOW = "playback-open-url";

// ------------------------------------------------------------------ A02

test("A02 addresses: supported schemes, local paths and file:// URLs; anything else is invalid", () => {
  for (const url of [
    "https://example.com/video.mp4",
    "http://h/a",
    "rtsp://cam.local:554/s",
    "rtmp://x/live",
    "rtmps://x/live",
    "mms://x/a",
    "ftp://x/a.mkv",
    "HTTPS://Example.com/a",
  ])
    assert.equal(parseAddress(url), url, url);
  assert.equal(parseAddress("  /Users/x/a b.mkv  "), "/Users/x/a b.mkv");
  assert.equal(parseAddress("file:///Users/x/%E7%94%B5%E5%BD%B1.mkv"), "/Users/x/电影.mkv");
  for (const bad of ["abc", "", "www.example.com/a.mp4", "javascript:alert(1)", "smb://x/a", "https://", "https://a b/c", "C:\\a.mp4"])
    assert.equal(parseAddress(bad), null, bad);
});

test("A02 Ctrl+U fills a clipboard address, selects it, and Enter opens it and closes the window", async () => {
  const clipboardHost = withMedia();
  clipboardHost.clipboard.text = "https://example.com/a.mp4\nsecond line";
  const { host, stop } = await start(clipboardHost);
  await host.press("Ctrl+U");
  await host.settle();
  const window = host.window(URL_WINDOW);
  assert.equal(window.file, "ui/open-url.slint");
  assert.equal(window.visible, true);
  assert.equal(window.props.value, "https://example.com/a.mp4");
  assert.equal(window.props["select-all"], true);
  assert.equal(window.props["paste-from-clipboard"], true);
  assert.equal(window.props.busy, false);
  window.emit("confirm", "https://example.com/a.mp4");
  await host.settle();
  // Unconditional: replaces the movie someone else opened.
  const open = intents(host).at(-1)!;
  assert.deepEqual([open.kind, open.source, open.expected], ["open", "https://example.com/a.mp4", undefined]);
  assert.equal(host.windows.has(URL_WINDOW), false);
  await stop();
});

test("A02 an invalid address keeps the window with a message; only the first line opens", async () => {
  const { host, stop } = await start();
  await host.command("open-url").run();
  await host.settle();
  const window = host.window(URL_WINDOW);
  assert.equal(window.props.value, ""); // empty clipboard, nothing typed yet
  assert.equal(window.props["select-all"], false);
  window.emit("confirm", "abc");
  await host.settle();
  assert.equal(window.props.message, INVALID);
  assert.equal(window.visible, true);
  window.emit("confirm", "   \n  ");
  await host.settle();
  assert.equal(window.props.message, INVALID);
  assert.equal(host.sent("source.submit").length, 0);
  window.emit("confirm", "\n  /a.mp4  \nhttps://x/b.mp4");
  await host.settle();
  assert.deepEqual([intents(host).at(-1)!.kind, intents(host).at(-1)!.source], ["open", "/a.mp4"]);
  assert.equal(host.windows.has(URL_WINDOW), false);
  assert.equal(lastOsd(host), FIRST_LINE_ONLY); // the window is gone, so the note is on the video
  await stop();
});

test("A02 while opening the button is busy and repeats are ignored; a refusal is shown in the window", async () => {
  const { host, stop } = await start();
  let release = () => {};
  host.handle("source.submit", () => new Promise((_, reject) => (release = () => reject(new HostError("busy", "播放器忙")))));
  await host.press("Ctrl+U");
  await host.settle();
  const window = host.window(URL_WINDOW);
  window.emit("confirm", "https://x/a.mp4");
  await until(() => window.props.busy === true);
  window.emit("confirm", "https://x/a.mp4");
  window.emit("confirm", "https://x/b.mp4");
  await new Promise((resolve) => setTimeout(resolve, 30)); // settle() would wait for the held open
  assert.equal(host.sent("source.submit").length, 1);
  release();
  await host.settle();
  assert.equal(window.props.busy, false);
  assert.match(String(window.props.message), /^打开失败：/);
  assert.equal(window.visible, true);
  await stop();
});

test("A02 without the clipboard option the last input of this run comes back; the option is saved", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-url-"));
  try {
    const firstHost = new TestHost();
    firstHost.clipboard.text = "https://clip/a.mp4";
    const first = await start(firstHost, data);
    await first.host.press("Ctrl+U");
    await first.host.settle();
    let window = first.host.window(URL_WINDOW);
    window.emit("paste-changed", false);
    window.emit("cancel", "rtsp://typed/stream");
    await first.host.settle();
    assert.equal(first.host.windows.has(URL_WINDOW), false);
    assert.deepEqual((await saved(join(data, "settings.json"))).openUrl, { pasteFromClipboard: false });
    await first.host.press("Ctrl+U");
    await first.host.settle();
    window = first.host.window(URL_WINDOW);
    assert.equal(window.props.value, "rtsp://typed/stream");
    assert.equal(window.props["select-all"], false);
    assert.equal(window.props["paste-from-clipboard"], false);
    await first.stop();

    // Next launch: the option is remembered, the typed text is not.
    const second = await start(new TestHost(), data);
    await second.host.press("Ctrl+U");
    await second.host.settle();
    assert.equal(second.host.window(URL_WINDOW).props.value, "");
    assert.equal(second.host.window(URL_WINDOW).props["paste-from-clipboard"], false);
    // A clipboard line that is not an address is never used.
    second.host.window(URL_WINDOW).emit("paste-changed", true);
    second.host.window(URL_WINDOW).emit("cancel", "");
    await second.host.settle();
    second.host.clipboard.text = "just some text";
    await second.host.press("Ctrl+U");
    await second.host.settle();
    assert.equal(second.host.window(URL_WINDOW).props.value, "");
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("A02 a clipboard the host cannot read fills nothing and shows nothing", async () => {
  const host = new TestHost();
  host.clipboard.text = "https://clip/a.mp4";
  host.handle("clipboard.readText", () => {
    throw new HostError("unavailable", "no display");
  });
  const { stop } = await start(host);
  await host.press("Ctrl+U");
  await host.settle();
  const window = host.window(URL_WINDOW);
  assert.equal(window.visible, true);
  assert.equal(window.props.value, "");
  assert.equal(window.props["select-all"], false);
  assert.deepEqual(host.osd, []);
  await stop();
});

test("settings: a bad field falls back to its default; a file that is not an object is set aside", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-settings-"));
  try {
    writeFileSync(join(data, "settings.json"), JSON.stringify({ preventSleep: false, hoverTime: "yes", openUrl: 1 }));
    const first = await start(new TestHost(), data);
    assert.equal(first.host.command("prevent-sleep").state.checked, false);
    assert.equal(first.host.command("hover-time").state.checked, false);
    assert.equal(first.host.mounted().props["hover-time"], false);
    await first.stop();

    writeFileSync(join(data, "settings.json"), "[1, 2]");
    const second = await start(new TestHost(), data);
    assert.equal(second.host.command("prevent-sleep").state.checked, true);
    assert.equal(existsSync(join(data, "settings.json")), false);
    assert.ok(readdirSync(data).some((name) => name.startsWith("settings.json.bad-")));
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ A10

test("A10 Ctrl+Y reloads the current source with its pause state and returns to the position", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 60, paused: true });
  await host.settle();
  assert.deepEqual(state(host, "reopen"), { enabled: true, checked: null, busy: false, reason: null });
  await host.press("Ctrl+Y");
  const open = intents(host).at(-1)!;
  assert.deepEqual([open.kind, open.source, open.startPaused], ["open", "/movie.mp4", true]);
  assert.equal(lastOsd(host), "重新打开: movie.mp4");
  host.setSource({ phase: "ready", duration: 120, position: 0 });
  await host.settle(20);
  assert.deepEqual(seeks(host), [60]);
  assert.equal(host.source.position, 60);
  await stop();
});

test("A10 no jump back when the position is 0 or past the reloaded length", async () => {
  for (const [position, duration] of [[0, 100], [90, 80]] as const) {
    const { host, stop } = await start(withMedia());
    host.setSource({ position });
    await host.settle();
    await host.press("Ctrl+Y");
    host.setSource({ phase: "ready", duration, position: 0 });
    await host.settle(20);
    assert.deepEqual(seeks(host), [], `${position}/${duration}`);
    await stop();
  }
});

test("A10 with nothing open, Ctrl+Y opens the last source from the start, also after a restart", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-last-"));
  try {
    const first = await start(withMedia(new TestHost(), "/Users/x/a.mp4"), data);
    const last = await saved(join(data, "last.json"));
    assert.equal(last.source, "/Users/x/a.mp4");
    assert.ok(!Number.isNaN(Date.parse(String(last.openedAt))));
    await first.host.press("F4");
    await first.host.settle();
    assert.equal(first.host.command("reopen").state.enabled, true);
    await first.host.press("Ctrl+Y");
    const open = intents(first.host).at(-1)!;
    assert.deepEqual([open.kind, open.source, open.startPaused], ["open", "/Users/x/a.mp4", false]);
    assert.equal(lastOsd(first.host), "重开最后文件: a.mp4");
    await first.stop();

    const second = await start(new TestHost(), data);
    assert.equal(second.host.command("reopen").state.enabled, true);
    await second.host.press("Ctrl+Y");
    assert.equal(intents(second.host).at(-1)!.source, "/Users/x/a.mp4");
    await second.stop();

    // A source opened by anyone becomes the last one; a broken last.json counts as none.
    writeFileSync(join(data, "last.json"), "{oops");
    const third = await start(new TestHost(), data);
    assert.deepEqual(state(third.host, "reopen"), { enabled: false, checked: null, busy: false, reason: "没有可重开的文件" });
    third.host.openElsewhere("https://example.com/v.m3u8");
    await third.host.settle();
    await saved(join(data, "last.json"), (value) => value.source === "https://example.com/v.m3u8");
    await third.host.press("F4");
    await third.host.settle();
    await third.host.press("Ctrl+Y");
    assert.equal(lastOsd(third.host), "重开最后文件: https://example.com/v.m3u8");
    await third.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("A10 a plugin stream is not remembered and cannot be reloaded; repeats while opening are ignored", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  host.setSource({ kind: "producer", source: "producer://1", phase: "ready" });
  await host.settle();
  assert.equal(existsSync(join(host.dataDirectory, "last.json")), false);
  assert.equal(host.command("reopen").state.enabled, false);

  host.openElsewhere("/b.mp4");
  await host.settle();
  let release = () => {};
  host.handle("source.submit", (p) => new Promise((resolve) => (release = () => resolve(p))));
  void host.command("reopen").run();
  await until(() => host.command("reopen").state.busy);
  assert.equal(host.command("reopen").state.enabled, false);
  await assert.rejects(host.press("Ctrl+Y")); // busy: the host does not run it
  release();
  host.reset("source.submit");
  await host.settle();
  assert.equal(host.command("reopen").state.busy, false);
  await stop();
});

// ------------------------------------------------------------------ B06

const TIME_JUMP = "playback-time-jump";

test("B06 step input: 0.1–3600 seconds with at most one decimal; titles drop trailing zeros", () => {
  for (const [text, value] of [["5", 5], [" 2.5 ", 2.5], ["0.1", 0.1], ["3600", 3600], ["10.0", 10]] as const)
    assert.equal(parseStep(text), value, text);
  for (const bad of ["", "abc", "0", "0.05", "1.25", "3600.1", "4000", "-1", "1e2", "5秒"]) assert.equal(parseStep(bad), null, bad);
  assert.equal(stepText(10), "10");
  assert.equal(stepText(2.5), "2.5");
});

test("B06 the window saves four steps: tier 1 goes to the core, tiers 2–4 retitle their commands", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-steps-"));
  try {
    const first = await start(withMedia(new TestHost(), "/movie.mp4"), data);
    const { host } = first;
    assert.deepEqual(host.sent("player.setStepSeek"), []); // the default 5 is the core's own
    // Works with nothing open, and shows the saved values.
    await host.press("F4");
    await host.command("time-jump").run();
    await host.settle();
    const window = host.window(TIME_JUMP);
    assert.equal(window.file, "ui/time-jump.slint");
    assert.equal(window.visible, true);
    assert.deepEqual([window.props.value1, window.props.value2, window.props.value3, window.props.value4], ["5", "30", "60", "300"]);

    // An invalid row is marked and nothing is saved or applied.
    window.emit("confirm", "10", "abc", "4000", "300");
    await host.settle();
    assert.deepEqual([window.props.invalid1, window.props.invalid2, window.props.invalid3, window.props.invalid4], [false, true, true, false]);
    assert.equal(window.props.message, RANGE);
    assert.equal(window.visible, true);
    assert.deepEqual(host.sent("player.setStepSeek"), []);
    // 默认值 restores only its own row.
    window.emit("reset", 1);
    await host.settle();
    assert.equal(window.props.value2, "30");
    assert.equal(window.props.invalid2, false);
    assert.equal(window.props.invalid3, true);

    window.emit("confirm", "10", "2.5", "90", "300");
    await host.settle();
    assert.equal(window.visible, false);
    assert.equal(host.stepSeek, 10);
    const titles = () =>
      ["backward-2", "forward-2", "backward-3", "forward-3", "backward-4", "forward-4"].map((id) => host.command(id).declaration.path.at(-1));
    assert.deepEqual(titles(), ["后退2.5秒", "前进2.5秒", "后退90秒", "前进90秒", "后退300秒", "前进300秒"]);
    assert.deepEqual(host.command("forward-3").declaration.bindings, ["Shift+ArrowRight"]);
    assert.equal(host.command("forward-3").declaration.order, 215);
    assert.deepEqual(host.conflicts, []);
    await saved(join(data, "settings.json"), (value) => JSON.stringify(value.seekSteps) === "[10,2.5,90,300]");

    host.openElsewhere("/movie.mp4");
    host.setSource({ duration: 100, position: 50 });
    await host.settle();
    await host.press("ArrowLeft"); // the core's tier 1 with the new step
    await host.settle(80);
    assert.equal(lastOsd(host), "后退 10秒: 00:40 / 01:40");
    await host.press("Shift+ArrowRight");
    await host.settle(80);
    assert.equal(host.source.position, 100);
    assert.equal(lastOsd(host), "前进 90秒: 01:40 / 01:40");
    await first.stop();

    // Next launch: tier 1 is given to the core again, the titles are restored.
    const second = await start(new TestHost(), data);
    await second.host.settle();
    assert.deepEqual(second.host.sent("player.setStepSeek").map((p) => p.seconds), [10]);
    assert.equal(second.host.command("backward-2").declaration.path.at(-1), "后退2.5秒");
    await second.stop();

    // A set with one value out of range falls back as a whole.
    writeFileSync(join(data, "settings.json"), JSON.stringify({ seekSteps: [10, 30, 60, 9999] }));
    const third = await start(new TestHost(), data);
    await third.host.settle();
    assert.deepEqual(third.host.sent("player.setStepSeek"), []);
    assert.equal(third.host.command("forward-4").declaration.path.at(-1), "前进300秒");
    await third.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("B06 cancel and Escape close the window without saving", async () => {
  const { host, stop } = await start();
  await host.command("time-jump").run();
  await host.settle();
  const window = host.window(TIME_JUMP);
  window.emit("cancel");
  await host.settle();
  assert.equal(window.visible, false);
  assert.deepEqual(host.sent("player.setStepSeek"), []);
  assert.equal(host.command("forward-2").declaration.path.at(-1), "前进30秒");
  await stop();
});

// ------------------------------------------------------------------ B08

const VIDEO = { codec: "h264", width: 1920, height: 1080, fps: 24 };

test("B08 keyframe steps need video, known once per source from stats(); a seekable media source only", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, video: VIDEO } as never;
  withMedia(host);
  const { stop } = await start(host);
  await host.settle();
  assert.deepEqual(state(host, "keyframe-forward"), { enabled: true, checked: null, busy: false, reason: null });
  const asked = host.sent("playback.stats").length;
  host.setSource({ position: 10, paused: true });
  host.setSource({ paused: false });
  await host.settle();
  assert.equal(host.sent("playback.stats").length, asked); // not asked again for the same source

  // An audio file: asked once for the new source, then disabled with a reason.
  host.stats = { ...host.stats, video: null } as never;
  host.openElsewhere("/song.mp3");
  host.setSource({ duration: 100, position: 0 });
  await host.settle();
  assert.equal(host.sent("playback.stats").length, asked + 1);
  assert.deepEqual(state(host, "keyframe-back"), { enabled: false, checked: null, busy: false, reason: NO_VIDEO });

  // A plugin stream or an unseekable source is not supported.
  host.stats = { ...host.stats, video: VIDEO } as never;
  host.setSource({ kind: "producer", source: "producer://1", phase: "ready" });
  await host.settle();
  assert.equal(state(host, "keyframe-forward").reason, "当前来源不支持此操作");
  host.openElsewhere("/live.ts");
  host.setSource({ duration: null, position: 0, capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle();
  assert.equal(state(host, "keyframe-forward").reason, "当前来源不支持此操作");
  await stop();
});

test("B08 Ctrl+Shift+←/→ step one keyframe with the time on the OSD; the ends say so", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, video: VIDEO } as never;
  withMedia(host);
  const keyframes = [0, 8.3, 16.7, 24];
  host.handle("player.step", (p) => {
    assert.equal(p.unit, "keyframe");
    const at = host.source.position ?? 0;
    const target =
      p.direction === "forward"
        ? (keyframes.find((k) => k > at + 0.001) ?? at)
        : ([...keyframes].reverse().find((k) => k < at - 0.001) ?? 0);
    host.setSource({ position: target });
    host.steps.push({ unit: String(p.unit), direction: String(p.direction) });
  });
  const { stop } = await start(host);
  host.setSource({ position: 3 });
  await host.settle();
  await host.press("Ctrl+Shift+ArrowRight");
  await host.settle();
  assert.equal(host.source.position, 8.3);
  assert.equal(lastOsd(host), "定位: 下一关键帧 00:08 / 01:40");
  // The condition is the state the plugin saw.
  assert.ok(host.sent("player.step")[0]!.expected);
  await host.press("Ctrl+Shift+ArrowRight");
  await host.press("Ctrl+Shift+ArrowRight");
  await host.settle();
  assert.equal(host.source.position, 24);
  await host.press("Ctrl+Shift+ArrowRight");
  await host.settle();
  assert.equal(lastOsd(host), "已是最后一个关键帧");
  await host.press("Ctrl+Shift+ArrowLeft");
  await host.settle();
  assert.equal(lastOsd(host), "定位: 上一关键帧 00:16 / 01:40");
  host.setSource({ position: 5 });
  await host.settle();
  await host.press("Ctrl+Shift+ArrowLeft");
  await host.settle();
  assert.equal(host.source.position, 0);
  assert.equal(lastOsd(host), "定位: 开始处");
  assert.deepEqual(
    host.steps.map((s) => s.direction),
    ["forward", "forward", "forward", "forward", "backward", "backward"],
  );
  await stop();
});

test("B08 repeats while a step is in flight are dropped; a refusal is shown, a stale one is not", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, video: VIDEO } as never;
  withMedia(host);
  let release = () => {};
  host.handle("player.step", () => new Promise<void>((resolve) => (release = resolve)));
  const { stop } = await start(host);
  await host.settle();
  void host.command("keyframe-forward").run();
  await until(() => host.command("keyframe-forward").state.busy);
  assert.equal(host.command("keyframe-back").state.enabled, false);
  await assert.rejects(host.press("Ctrl+Shift+ArrowRight")); // the held key's repeat: refused while busy
  release();
  await host.settle();
  assert.equal(host.sent("player.step").length, 1);
  assert.equal(host.command("keyframe-forward").state.busy, false);

  host.handle("player.step", () => {
    throw new HostError("unsupported", "this source cannot step");
  });
  await host.press("Ctrl+Shift+ArrowLeft");
  await host.settle();
  assert.equal(lastOsd(host), "来源操作失败：this source cannot step");
  const shown = host.osd.length;
  host.handle("player.step", () => {
    throw new HostError("stale", "stale");
  });
  await host.press("Ctrl+Shift+ArrowLeft");
  await host.settle();
  assert.equal(host.osd.length, shown);
  await stop();
});

// ------------------------------------------------------------------ B09

test("B09 Backspace returns to 0:00 keeping pause; after the end it replays; unseekable sources disable it", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 40, paused: true });
  await host.settle();
  await host.press("Backspace");
  await host.settle(20);
  assert.deepEqual(seeks(host), [0]);
  assert.equal(host.source.position, 0);
  assert.equal(host.source.paused, true);
  assert.equal(lastOsd(host), "定位: 开始处");

  host.setSource({ phase: "ended", terminalReason: "natural", position: 100 });
  await host.settle();
  assert.equal(host.command("jump-start").state.enabled, true);
  await host.press("Backspace");
  assert.equal(intents(host).at(-1)!.kind, "restart");
  assert.equal(host.source.phase, "ready");
  assert.equal(lastOsd(host), "定位: 开始处");

  host.setSource({ capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle();
  assert.deepEqual(state(host, "jump-start"), { enabled: false, checked: null, busy: false, reason: "当前来源不支持此操作" });
  await stop();
});

// ------------------------------------------------------------------ B29

const sleepRequests = (host: TestHost) => host.sent("power.preventSleep").map((p) => p.mode);

test("B29 the switch is passed to plugin.power once; playback changes do not repeat it", async () => {
  const host = withMedia();
  const { stop } = await start(host);
  await host.settle();
  assert.equal(host.power, "playing");
  // The host holds and drops the assertion itself on pause, end, close and a new source.
  await host.press("Space");
  await host.press("Space");
  host.setSource({ position: 10 });
  host.openElsewhere("/film.mkv");
  host.setSource({ phase: "ended", terminalReason: "natural" });
  await host.press("F4");
  await host.settle();
  assert.deepEqual(sleepRequests(host), ["playing"]);
  await stop();
});

test("B29 the switch turns it off and on and is saved; off at launch asks for nothing", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-sleep-"));
  try {
    const first = await start(new TestHost(), data);
    const { host } = first;
    host.setSource({ kind: "producer", source: "producer://1", phase: "ready" });
    await host.settle();
    await host.command("prevent-sleep").run();
    await host.settle();
    assert.equal(host.power, "off");
    assert.equal(host.command("prevent-sleep").state.checked, false);
    assert.equal(lastOsd(host), "播放时禁止屏保: 关");
    await saved(join(data, "settings.json"), (value) => value.preventSleep === false);
    host.setSource({ paused: true });
    host.setSource({ paused: false });
    await host.settle();
    assert.deepEqual(sleepRequests(host), ["playing", "off"]);
    await host.command("prevent-sleep").run();
    await host.settle();
    assert.equal(host.power, "playing");
    assert.equal(lastOsd(host), "播放时禁止屏保: 开");
    await host.command("prevent-sleep").run();
    await first.stop();

    const second = await start(withMedia(), data);
    await second.host.settle();
    assert.equal(second.host.command("prevent-sleep").state.checked, false);
    assert.deepEqual(sleepRequests(second.host), []);
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ I06

test("I06 the bar gets the duration and the hover option; the option is saved", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  assert.equal(bar.props.duration, 100);
  assert.equal(bar.props["hover-time"], false);
  await host.command("hover-time").run();
  await host.settle();
  assert.equal(bar.props["hover-time"], true);
  assert.equal(host.command("hover-time").state.checked, true);
  assert.equal(lastOsd(host), "进度条悬停显示时间: 开");
  await saved(join(host.dataDirectory, "settings.json"), (value) => value.hoverTime === true);
  host.openElsewhere("/live");
  await host.settle();
  assert.equal(bar.props.duration, 0);
  await stop();
});

// ------------------------------------------------------------------ I08

test("I08 copies the file name, folder and full path of a local file", async () => {
  const { host, stop } = await start(withMedia(new TestHost(), "/Users/x/电影/a b.mkv"));
  const copied: (string | null)[] = [];
  for (const id of ["copy-name", "copy-folder", "copy-path"]) {
    await host.command(id).run();
    copied.push(host.clipboard.text);
  }
  assert.deepEqual(copied, ["a b.mkv", "/Users/x/电影", "/Users/x/电影/a b.mkv"]);
  assert.equal(lastOsd(host), "已复制: /Users/x/电影/a b.mkv");
  host.handle("clipboard.writeText", () => {
    throw new HostError("failed", "系统拒绝写入");
  });
  await host.command("copy-name").run();
  assert.equal(lastOsd(host), "复制失败：系统拒绝写入");
  assert.equal(host.clipboard.text, "/Users/x/电影/a b.mkv");
  host.reset("clipboard.writeText");
  await host.press("F4");
  await host.settle();
  for (const id of ["copy-name", "copy-folder", "copy-path"])
    assert.deepEqual(state(host, id), { enabled: false, checked: null, busy: false, reason: "没有打开的文件" }, id);
  await stop();
});

test("I08 a network source only has its full address; long texts are shortened on the OSD", async () => {
  const url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL0123456789abcdefghijklmnopqrstuv";
  const { host, stop } = await start(withMedia(new TestHost(), url));
  for (const id of ["copy-name", "copy-folder"])
    assert.deepEqual(state(host, id), { enabled: false, checked: null, busy: false, reason: "网络来源没有文件名" }, id);
  assert.equal(host.command("copy-path").state.enabled, true);
  await host.command("copy-path").run();
  assert.equal(host.clipboard.text, url);
  assert.equal(lastOsd(host), `已复制: ${url.slice(0, 20)}…${url.slice(-37)}`);
  assert.equal(Array.from(shorten("电".repeat(61))).length, 58);
  assert.equal(shorten("a".repeat(60)), "a".repeat(60));
  await stop();
});
