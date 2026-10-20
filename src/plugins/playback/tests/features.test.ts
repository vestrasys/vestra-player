import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import { shorten } from "../copy.ts";
import activate from "../main.ts";
import { FIRST_LINE_ONLY, INVALID, parseAddress } from "../open-url.ts";
import { fakeSystem } from "./fakes.ts";

const fakes = fakeSystem();
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
const reset = () => {
  fakes.children.length = 0;
  fakes.copied.length = 0;
  fakes.clipboard = "";
  fakes.failSpawn = fakes.failCopy = null;
};
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
  reset();
  fakes.clipboard = "https://example.com/a.mp4\nsecond line";
  const { host, stop } = await start(withMedia());
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
  reset();
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
  reset();
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
  reset();
  const data = mkdtempSync(join(tmpdir(), "playback-url-"));
  try {
    fakes.clipboard = "https://clip/a.mp4";
    const first = await start(new TestHost(), data);
    await first.host.press("Ctrl+U");
    await first.host.settle();
    let window = first.host.window(URL_WINDOW);
    window.emit("paste-changed", false);
    window.emit("cancel", "rtsp://typed/stream");
    await first.host.settle();
    assert.equal(first.host.windows.has(URL_WINDOW), false);
    assert.deepEqual(JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).openUrl, { pasteFromClipboard: false });
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
    fakes.clipboard = "just some text";
    await second.host.press("Ctrl+U");
    await second.host.settle();
    assert.equal(second.host.window(URL_WINDOW).props.value, "");
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ A10

test("A10 Ctrl+Y reloads the current source with its pause state and returns to the position", async () => {
  reset();
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
  reset();
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
  reset();
  const data = mkdtempSync(join(tmpdir(), "playback-last-"));
  try {
    const first = await start(withMedia(new TestHost(), "/Users/x/a.mp4"), data);
    const saved = JSON.parse(readFileSync(join(data, "last.json"), "utf8"));
    assert.equal(saved.source, "/Users/x/a.mp4");
    assert.ok(!Number.isNaN(Date.parse(saved.openedAt)));
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
    assert.equal(JSON.parse(readFileSync(join(data, "last.json"), "utf8")).source, "https://example.com/v.m3u8");
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
  reset();
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

// ------------------------------------------------------------------ B09

test("B09 Backspace returns to 0:00 keeping pause; after the end it replays; unseekable sources disable it", async () => {
  reset();
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

test("B29 caffeinate runs while playing: -i for audio, -d -i with video, -w the plugin process", async () => {
  reset();
  const host = withMedia();
  const { stop } = await start(host);
  await host.settle();
  assert.equal(fakes.running.length, 1);
  assert.deepEqual(fakes.running[0]!.args, ["-i", "-w", String(process.pid)]);

  // Pause ends it, play starts a new one; one child at a time.
  await host.press("Space");
  await host.settle();
  assert.equal(fakes.running.length, 0);
  await host.press("Space");
  await host.settle();
  assert.equal(fakes.running.length, 1);
  assert.equal(fakes.children.length, 2);
  host.setSource({ position: 10 }); // position updates leave it alone
  await host.settle();
  assert.equal(fakes.children.length, 2);

  // A new source decides again: this one has video.
  host.stats = { ...host.stats, video: { codec: "h264", width: 1920, height: 1080, fps: 24 } } as never;
  host.openElsewhere("/film.mkv");
  await host.settle();
  assert.equal(fakes.running.length, 1);
  assert.deepEqual(fakes.running[0]!.args, ["-d", "-i", "-w", String(process.pid)]);
  assert.equal(fakes.children.length, 3);

  host.setSource({ phase: "ended", terminalReason: "natural" });
  await host.settle();
  assert.equal(fakes.running.length, 0);
  await host.press("Backspace"); // replay
  await host.settle();
  assert.equal(fakes.running.length, 1);
  await host.press("F4");
  await host.settle();
  assert.equal(fakes.running.length, 0);
  host.openElsewhere("/a.mp4");
  await host.settle();
  host.setSource({ phase: "error", terminalReason: { failed: "x" } });
  await host.settle();
  assert.equal(fakes.running.length, 0);

  // Deactivation ends it.
  host.openElsewhere("/a.mp4");
  await host.settle();
  assert.equal(fakes.running.length, 1);
  await stop();
  assert.equal(fakes.running.length, 0);
});

test("B29 a plugin stream keeps the display awake; the switch stops and restarts it and is saved", async () => {
  reset();
  const host = new TestHost();
  const { stop } = await start(host);
  host.setSource({ kind: "producer", source: "producer://1", phase: "ready" });
  await host.settle();
  assert.deepEqual(fakes.running[0]!.args, ["-d", "-i", "-w", String(process.pid)]);
  await host.command("prevent-sleep").run();
  await host.settle();
  assert.equal(fakes.running.length, 0);
  assert.equal(host.command("prevent-sleep").state.checked, false);
  assert.equal(lastOsd(host), "播放时禁止屏保: 关");
  assert.equal(JSON.parse(readFileSync(join(host.dataDirectory, "settings.json"), "utf8")).preventSleep, false);
  host.setSource({ paused: true });
  host.setSource({ paused: false });
  await host.settle();
  assert.equal(fakes.running.length, 0);
  await host.command("prevent-sleep").run();
  await host.settle();
  assert.equal(fakes.running.length, 1);
  assert.equal(lastOsd(host), "播放时禁止屏保: 开");
  await stop();
});

test("B29 a failed start is shown once, the switch stays on, and the next start of playback tries again", async () => {
  reset();
  fakes.failSpawn = Object.assign(new Error("spawn /usr/bin/caffeinate ENOENT"), { code: "ENOENT" });
  const host = withMedia();
  const { stop } = await start(host);
  await host.settle();
  assert.equal(fakes.running.length, 0);
  assert.equal(host.osd.filter((t) => t.startsWith("无法阻止屏保：")).length, 1);
  assert.equal(lastOsd(host), "无法阻止屏保：spawn /usr/bin/caffeinate ENOENT");
  assert.equal(host.command("prevent-sleep").state.checked, true);
  host.setSource({ position: 5 });
  await host.settle();
  assert.equal(fakes.children.length, 1); // not retried within the same playback
  await host.press("Space");
  await host.press("Space");
  await host.settle();
  assert.equal(fakes.running.length, 1);
  await stop();
});

// ------------------------------------------------------------------ I06

test("I06 the bar gets the duration and the hover option; the option is saved", async () => {
  reset();
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  assert.equal(bar.props.duration, 100);
  assert.equal(bar.props["hover-time"], true);
  await host.command("hover-time").run();
  await host.settle();
  assert.equal(bar.props["hover-time"], false);
  assert.equal(host.command("hover-time").state.checked, false);
  assert.equal(lastOsd(host), "进度条悬停显示时间: 关");
  assert.equal(JSON.parse(readFileSync(join(host.dataDirectory, "settings.json"), "utf8")).hoverTime, false);
  host.openElsewhere("/live");
  await host.settle();
  assert.equal(bar.props.duration, 0);
  await stop();
});

// ------------------------------------------------------------------ I08

test("I08 copies the file name, folder and full path of a local file", async () => {
  reset();
  const { host, stop } = await start(withMedia(new TestHost(), "/Users/x/电影/a b.mkv"));
  await host.command("copy-name").run();
  await host.command("copy-folder").run();
  await host.command("copy-path").run();
  assert.deepEqual(fakes.copied, ["a b.mkv", "/Users/x/电影", "/Users/x/电影/a b.mkv"]);
  assert.equal(lastOsd(host), "已复制: /Users/x/电影/a b.mkv");
  fakes.failCopy = new Error("超时");
  await host.command("copy-name").run();
  assert.equal(lastOsd(host), "复制失败：超时");
  await host.press("F4");
  await host.settle();
  for (const id of ["copy-name", "copy-folder", "copy-path"])
    assert.deepEqual(state(host, id), { enabled: false, checked: null, busy: false, reason: "没有打开的文件" }, id);
  await stop();
});

test("I08 a network source only has its full address; long texts are shortened on the OSD", async () => {
  reset();
  const url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL0123456789abcdefghijklmnopqrstuv";
  const { host, stop } = await start(withMedia(new TestHost(), url));
  for (const id of ["copy-name", "copy-folder"])
    assert.deepEqual(state(host, id), { enabled: false, checked: null, busy: false, reason: "网络来源没有文件名" }, id);
  assert.equal(host.command("copy-path").state.enabled, true);
  await host.command("copy-path").run();
  assert.deepEqual(fakes.copied, [url]);
  assert.equal(lastOsd(host), `已复制: ${url.slice(0, 20)}…${url.slice(-37)}`);
  assert.equal(Array.from(shorten("电".repeat(61))).length, 58);
  assert.equal(shorten("a".repeat(60)), "a".repeat(60));
  await stop();
});
