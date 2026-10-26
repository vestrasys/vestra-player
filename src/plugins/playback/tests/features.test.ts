import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { PlayerState } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { copyTarget, shorten } from "../copy.ts";
import activate from "../main.ts";
import { NO_VIDEO } from "../keyframes.ts";
import { HOST_RULES, isLocalPath, type PathRules } from "../local-path.ts";
import { FIRST_LINE_ONLY, parseAddress } from "../open-url.ts";
import { displayName } from "../reopen.ts";
import { parseStep, stepText } from "../time-jump.ts";
const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  // The Chinese texts are what the plugin showed before English became the source.
  startPlugin(activate, { directory, host, locale: "zh-Hans", ...(dataDirectory ? { dataDirectory } : {}) });
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
/** The command state, its reason as the menu shows it. */
const state = (host: TestHost, id: string) => {
  const { enabled, checked, busy, reason } = host.command(id).state;
  return { enabled, checked, busy, reason: reason === null ? null : host.translate(reason) };
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
const RULES: PathRules[] = ["posix", "windows"];
/** A local file as the player on this OS names it. */
const LOCAL = HOST_RULES === "windows" ? "C:\\a.mp4" : "/a.mp4";

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
    for (const rules of RULES) assert.equal(parseAddress(url, rules), url, `${rules} ${url}`);
  for (const rules of RULES)
    for (const bad of ["abc", "", "www.example.com/a.mp4", "javascript:alert(1)", "smb://x/a", "https://", "https://a b/c", "C:a.mp4"])
      assert.equal(parseAddress(bad, rules), null, `${rules} ${bad}`);
});

// What each local form means on each OS; both are checked on every machine.
const LOCAL_FORMS: [input: string, posix: string | null, windows: string | null][] = [
  ["  /Users/x/a b.mkv  ", "/Users/x/a b.mkv", null], // Windows: no drive, so it would be the player's current one
  ["\\a.mp4", null, null],
  ["C:\\a.mp4", null, "C:\\a.mp4"],
  ["  C:\\电影\\a b.mkv ", null, "C:\\电影\\a b.mkv"],
  ["C:/x/a.mp4", null, "C:\\x\\a.mp4"],
  ["\\\\server\\share\\a.mp4", null, "\\\\server\\share\\a.mp4"],
  ["//server/share/a.mp4", "//server/share/a.mp4", "\\\\server\\share\\a.mp4"],
  ["file:///Users/x/%E7%94%B5%E5%BD%B1.mkv", "/Users/x/电影.mkv", null], // Windows: no drive
  ["file:///C:/x/a%20b.mp4", "/C:/x/a b.mp4", "C:\\x\\a b.mp4"],
  ["file://localhost/C:/x/a.mp4", "/C:/x/a.mp4", "C:\\x\\a.mp4"],
  ["file://server/share/a.mp4", null, "\\\\server\\share\\a.mp4"], // macOS: a file URL names no other machine
];

test("A02 local paths and file:// URLs follow the player's OS: POSIX paths on macOS, drive and UNC paths on Windows", () => {
  for (const [input, posix, windows] of LOCAL_FORMS) {
    assert.equal(parseAddress(input, "posix"), posix, `posix ${input}`);
    assert.equal(parseAddress(input, "windows"), windows, `windows ${input}`);
    // Without rules the OS running the test decides.
    assert.equal(parseAddress(input), HOST_RULES === "windows" ? windows : posix, `here ${input}`);
  }
});

test("I08 and A10 name a local file by the OS's path rules; a link is not a file", () => {
  const playing = (source: string) => ({ source, kind: "media", phase: "playing" }) as unknown as PlayerState;
  const copies = (source: string, rules: PathRules) =>
    (["name", "folder", "path"] as const).map((kind) => {
      const target = copyTarget(playing(source), kind, rules);
      return "text" in target ? target.text : null;
    });
  assert.deepEqual(copies("/Users/x/a b.mkv", "posix"), ["a b.mkv", "/Users/x", "/Users/x/a b.mkv"]);
  assert.deepEqual(copies("C:\\电影\\a b.mkv", "windows"), ["a b.mkv", "C:\\电影", "C:\\电影\\a b.mkv"]);
  assert.deepEqual(copies("\\\\nas\\media\\a.mkv", "windows"), ["a.mkv", "\\\\nas\\media\\", "\\\\nas\\media\\a.mkv"]);
  assert.deepEqual(copies("C:\\电影\\a b.mkv", "posix"), [null, null, "C:\\电影\\a b.mkv"]);
  for (const rules of RULES) assert.deepEqual(copies("https://h/a.mp4", rules), [null, null, "https://h/a.mp4"]);
  assert.equal(displayName("C:\\电影\\a b.mkv", "windows"), "a b.mkv");
  assert.equal(displayName("/Users/x/a b.mkv", "posix"), "a b.mkv");
  assert.equal(displayName("https://h/a.mp4", "windows"), "https://h/a.mp4");
  for (const rules of RULES) assert.equal(isLocalPath("https://h/a.mp4", rules), false);
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
  assert.equal(window.props.notice, "invalid");
  assert.equal(window.visible, true);
  window.emit("confirm", "   \n  ");
  await host.settle();
  assert.equal(window.props.notice, "invalid");
  assert.equal(host.sent("source.submit").length, 0);
  window.emit("confirm", `\n  ${LOCAL}  \nhttps://x/b.mp4`);
  await host.settle();
  assert.deepEqual([intents(host).at(-1)!.kind, intents(host).at(-1)!.source], ["open", LOCAL]);
  assert.equal(host.windows.has(URL_WINDOW), false);
  // The window is gone, so the note is on the video.
  assert.equal(host.sent("osd.show").at(-1)!.text, FIRST_LINE_ONLY);
  assert.equal(lastOsd(host), "只打开第一行；多个链接请用播放列表的「添加链接…」");
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
  assert.deepEqual([window.props.notice, window.props.detail], ["failed", "播放器忙"]);
  assert.equal(host.translate("Open failed: {}", ["播放器忙"]), "打开失败：播放器忙");
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
  // busy, so disabled: the key passes on (SDK 1.5.12, as the player) and nothing runs again
  const shownBusy = host.osd.length;
  const submitted = host.sent("source.submit").length;
  assert.equal(await host.press("Ctrl+Y"), "passed");
  assert.equal(host.osd.length, shownBusy);
  assert.equal(host.sent("source.submit").length, submitted);
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
    // Every start gives the core the pair: step 5 and "Jump to keyframes only" on by default.
    await until(() => host.sent("player.setStepSeek").length === 1);
    assert.deepEqual(host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: true }]);
    assert.equal(host.stepSeekKeyframes, true);
    // Works with nothing open, and shows the saved values.
    await host.press("F4");
    await host.command("time-jump").run();
    await host.settle();
    const window = host.window(TIME_JUMP);
    const values = () => [window.props.value1, window.props.value2, window.props.value3, window.props.value4, window.props.keyframes];
    await until(() => window.visible && values().every((value) => value !== undefined));
    assert.equal(window.file, "ui/time-jump.slint");
    assert.deepEqual(values(), ["5", "30", "60", "300", true]);
    assert.equal(host.translate("Jump to keyframes only"), "仅跳转到关键帧");

    // An invalid row is marked and nothing is saved or applied, the checkbox included.
    window.emit("confirm", "10", "abc", "4000", "300", false);
    const marks = () => [window.props.invalid1, window.props.invalid2, window.props.invalid3, window.props.invalid4, window.props["out-of-range"]];
    await until(() => window.props["out-of-range"] === true && window.props.invalid3 === true);
    assert.deepEqual(marks(), [false, true, true, false, true]);
    assert.equal(host.translate("Range 0.1–3600 s, at most one decimal place"), "范围 0.1–3600 秒，最多一位小数");
    assert.equal(window.visible, true);
    assert.deepEqual(host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: true }]);
    assert.equal(host.stepSeekKeyframes, true);
    // Default restores only its own row.
    window.emit("reset", 1);
    await host.settle();
    assert.equal(window.props.value2, "30");
    assert.equal(window.props.invalid2, false);
    assert.equal(window.props.invalid3, true);

    // OK with the box cleared: the pair goes to the core and is saved with the steps.
    window.emit("confirm", "10", "2.5", "90", "300", false);
    await until(() => !window.visible && host.sent("player.setStepSeek").length === 2);
    assert.deepEqual(host.sent("player.setStepSeek").at(-1), { seconds: 10, keyframes: false });
    assert.equal(host.stepSeek, 10);
    assert.equal(host.stepSeekKeyframes, false);
    const titles = () =>
      ["backward-2", "forward-2", "backward-3", "forward-3", "backward-4", "forward-4"].map((id) => host.command(id).display.path.at(-1));
    // Any seconds are translated: the title is a text value with the seconds as data.
    assert.deepEqual(titles(), ["后退2.5秒", "前进2.5秒", "后退90秒", "前进90秒", "后退300秒", "前进300秒"]);
    assert.deepEqual(host.command("forward-3").declaration.bindings, ["Shift+ArrowRight"]);
    assert.equal(host.command("forward-3").declaration.order, 215);
    assert.deepEqual(host.conflicts, []);
    await saved(join(data, "settings.json"), (value) => JSON.stringify(value.seekSteps) === "[10,2.5,90,300]" && value.seekKeyframes === false);

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

    // Next launch: the pair is given to the core again, the titles and the checkbox are restored.
    const second = await start(new TestHost(), data);
    await until(() => second.host.sent("player.setStepSeek").length === 1);
    assert.deepEqual(second.host.sent("player.setStepSeek"), [{ seconds: 10, keyframes: false }]);
    assert.equal(second.host.command("backward-2").display.path.at(-1), "后退2.5秒");
    await second.host.command("time-jump").run();
    const reopened = second.host.window(TIME_JUMP);
    await until(() => reopened.visible && reopened.props.keyframes !== undefined && reopened.props.value1 !== undefined);
    assert.deepEqual([reopened.props.value1, reopened.props.keyframes], ["10", false]);
    await second.stop();

    // A set with one value out of range falls back as a whole; a checkbox value that is not a boolean, to on.
    writeFileSync(join(data, "settings.json"), JSON.stringify({ seekSteps: [10, 30, 60, 9999], seekKeyframes: "no" }));
    const third = await start(new TestHost(), data);
    await until(() => third.host.sent("player.setStepSeek").length === 1);
    assert.deepEqual(third.host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: true }]);
    assert.equal(third.host.command("forward-4").display.path.at(-1), "前进300秒");
    await third.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

test("B06 cancel and Escape close the window without saving, the checkbox included", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-steps-cancel-"));
  try {
    const { host, stop } = await start(new TestHost(), data);
    await until(() => host.sent("player.setStepSeek").length === 1);
    await host.command("time-jump").run();
    const window = host.window(TIME_JUMP);
    await until(() => window.visible && window.props.keyframes === true);
    // The view's own edits (a cleared box, another step), then Cancel.
    window.props.keyframes = false;
    window.props.value2 = "45";
    assert.deepEqual([window.props.keyframes, window.props.value2], [false, "45"]);
    window.emit("cancel");
    await until(() => !window.visible);
    await host.settle(400); // past storage.json's debounce: nothing may be written
    assert.deepEqual(host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: true }]);
    assert.equal(host.stepSeekKeyframes, true);
    assert.equal(host.command("forward-2").display.path.at(-1), "前进30秒");
    assert.equal(existsSync(join(data, "settings.json")), false);
    // Opening again shows the saved values, not the cancelled ones.
    await host.command("time-jump").run();
    await until(() => window.visible && window.props.keyframes === true && window.props.value2 === "30");
    await stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

/** Two keyframes 10 s apart, a 100-second source at 12 s, the plugin started with `settings`. */
async function keyframeSource(settings?: Record<string, unknown>) {
  const data = mkdtempSync(join(tmpdir(), "playback-steps-kf-"));
  if (settings) writeFileSync(join(data, "settings.json"), JSON.stringify(settings));
  const host = withMedia();
  host.keyframes = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
  const started = await start(host, data);
  await until(() => host.sent("player.setStepSeek").length === 1);
  host.setSource({ position: 12 });
  await host.settle();
  return {
    host,
    stop: async () => {
      await started.stop();
      rmSync(data, { recursive: true, force: true });
    },
  };
}
/** The plugin's own on-screen texts (templates as sent, before translation). */
const ownOsd = (host: TestHost) => host.sent("osd.show").map((p) => p.text);

test("B06 keyframes on: tiers 2–4 are the core's step seek with their step; it lands and shows the landed time, the plugin no text", async () => {
  const { host, stop } = await keyframeSource();
  assert.equal(host.stepSeekKeyframes, true);
  const shown = ownOsd(host).length;
  const press = async (key: string, count: number) => {
    await host.press(key);
    await until(() => host.coreSeeks.length === count && host.osd.length > 0);
    return host.coreSeeks.at(-1);
  };
  // From 12: forward 30 is target 42, the first keyframe at or after it is 50.
  assert.deepEqual(await press("Ctrl+ArrowRight", 1), { action: "seek-forward", seconds: 30, keyframes: true, landed: 50 });
  await until(() => host.source.position === 50 && lastOsd(host) === "前进 30秒: 00:50 / 01:40");
  // From 50: back 60 is clamped to 0.
  assert.deepEqual(await press("Shift+ArrowLeft", 2), { action: "seek-backward", seconds: 60, keyframes: true, landed: 0 });
  await until(() => host.source.position === 0 && lastOsd(host) === "后退 60秒: 00:00 / 01:40");
  host.setSource({ position: 12 });
  await host.settle();
  // Forward 300 runs to the end; back 30 from 12 to 0.
  assert.deepEqual(await press("Ctrl+Alt+ArrowRight", 3), { action: "seek-forward", seconds: 300, keyframes: true, landed: 100 });
  host.setSource({ position: 47 });
  await host.settle();
  // Back 30 from 47 is target 17, the last keyframe at or before it is 10.
  assert.deepEqual(await press("Ctrl+ArrowLeft", 4), { action: "seek-backward", seconds: 30, keyframes: true, landed: 10 });
  await until(() => host.source.position === 10 && lastOsd(host) === "后退 30秒: 00:10 / 01:40");
  // Each press was one host.run with the tier's step; ← / → stays the core's own binding.
  assert.deepEqual(
    host.sent("host.run").map((p) => [p.action, p.seconds]),
    [
      ["seek-forward", 30],
      ["seek-backward", 60],
      ["seek-forward", 300],
      ["seek-backward", 30],
    ],
  );
  await host.press("ArrowRight");
  await until(() => host.coreSeeks.length === 5);
  assert.deepEqual(host.coreSeeks.at(-1), { action: "seek-forward", seconds: 5, keyframes: true, landed: 20 });
  // No seek text of the plugin's own in keyframe mode: the core showed every one.
  await host.settle(80);
  assert.deepEqual(ownOsd(host).slice(shown), []);
  await stop();
});

test("B06 keyframes off: tiers 2–4 keep the plugin's exact seeks and its own text; the setting is read at each press", async () => {
  const { host, stop } = await keyframeSource({ seekKeyframes: false });
  assert.deepEqual(host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: false }]);
  await host.press("Ctrl+ArrowRight");
  await until(() => host.source.position === 42 && lastOsd(host) === "前进 30秒: 00:42 / 01:40");
  assert.deepEqual(host.sent("host.run"), []);
  assert.deepEqual(host.coreSeeks, []);
  assert.equal(ownOsd(host).at(-1), "Forward {} s: {} / {}");
  // ← / → is exact too: the core had the pair with keyframes off.
  await host.press("ArrowLeft");
  await until(() => host.coreSeeks.length === 1);
  assert.deepEqual(host.coreSeeks[0], { action: "seek-backward", seconds: 5, keyframes: false, landed: 37 });

  // Turned on in the window: the next press of a tier goes to the core.
  await host.command("time-jump").run();
  const window = host.window(TIME_JUMP);
  await until(() => window.visible && window.props.keyframes === false && window.props.value1 === "5");
  window.emit("confirm", "5", "30", "60", "300", true);
  await until(() => !window.visible && host.sent("player.setStepSeek").length === 2);
  assert.deepEqual(host.sent("player.setStepSeek").at(-1), { seconds: 5, keyframes: true });
  host.setSource({ position: 12 });
  await host.settle();
  const shown = ownOsd(host).length;
  await host.press("Ctrl+ArrowRight");
  await until(() => host.coreSeeks.length === 2 && host.source.position === 50);
  assert.deepEqual(host.coreSeeks[1], { action: "seek-forward", seconds: 30, keyframes: true, landed: 50 });
  await host.settle(80);
  assert.deepEqual(ownOsd(host).slice(shown), []);
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
  assert.deepEqual(state(host, "keyframe-back"), { enabled: false, checked: null, busy: false, reason: "没有视频" });
  assert.equal(host.command("keyframe-back").state.reason, NO_VIDEO);

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
  // the held key's repeat: the busy command is disabled, so the key passes on and no second step starts
  const shownBusy = host.osd.length;
  assert.equal(await host.press("Ctrl+Shift+ArrowRight"), "passed");
  assert.equal(host.osd.length, shownBusy);
  assert.equal(host.sent("player.step").length, 1);
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
  const [folder, file] =
    HOST_RULES === "windows" ? ["C:\\Users\\x\\电影", "C:\\Users\\x\\电影\\a b.mkv"] : ["/Users/x/电影", "/Users/x/电影/a b.mkv"];
  const { host, stop } = await start(withMedia(new TestHost(), file));
  const copied: (string | null)[] = [];
  for (const id of ["copy-name", "copy-folder", "copy-path"]) {
    await host.command(id).run();
    copied.push(host.clipboard.text);
  }
  assert.deepEqual(copied, ["a b.mkv", folder, file]);
  assert.equal(lastOsd(host), `已复制: ${file}`);
  host.handle("clipboard.writeText", () => {
    throw new HostError("failed", "系统拒绝写入");
  });
  await host.command("copy-name").run();
  assert.equal(lastOsd(host), "复制失败：系统拒绝写入");
  assert.equal(host.clipboard.text, file);
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
