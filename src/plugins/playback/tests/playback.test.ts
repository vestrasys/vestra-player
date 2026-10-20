import assert from "node:assert/strict";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";
import { parseTime, time } from "../format.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost()) => startPlugin(activate, { directory, host });
/** A ready 100-second source opened by someone else. */
function withMedia(host = new TestHost()) {
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 0 });
  return host;
}
/** Poll while a request is deliberately held (settle waits for idle). */
async function until(predicate: () => boolean) {
  const deadline = Date.now() + 1500;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const seeks = (host: TestHost) =>
  host
    .sent("source.submit")
    .map((p) => p.intent as { kind: string; control?: { kind: string; seconds?: number } })
    .filter((i) => i.kind === "control" && i.control?.kind === "seek")
    .map((i) => i.control!.seconds);

test("explicit time input rejects malformed and negative values", () => {
  assert.equal(parseTime("1:02:03.250"), 3723.25);
  assert.equal(parseTime("12:61"), null);
  assert.equal(parseTime("-1"), null);
  assert.equal(time(null), "--:--");
  assert.equal(time(3723), "1:02:03");
});

test("menu commands keep their paths, bindings and reasons; empty player disables source actions", async () => {
  const { host, stop } = await start();
  assert.deepEqual(
    [...host.commands].map(([id, { declaration: d }]) => [
      id,
      d.path.join("/"),
      d.title,
      "bindings" in d ? d.bindings : null,
      "order" in d ? d.order : null,
    ]),
    [
      ["open", "文件/打开媒体…", "打开媒体…", [], 202],
      ["close-source", "文件/关闭当前来源", "关闭当前来源", [], 201],
      ["toggle-pause", "播放/播放/暂停", "播放/暂停", ["Space"], 263],
      ["jump", "播放/跳转到…", "跳转到…", [], 264],
      ["backward-5", "播放/后退5秒", "后退5秒", ["ArrowLeft"], 262],
      ["forward-5", "播放/前进5秒", "前进5秒", ["ArrowRight"], 261],
      ["rate-05", "播放/速度/0.5倍", "0.5倍", [], 265],
      ["rate-10", "播放/速度/正常速度", "正常速度", [], 268],
      ["rate-15", "播放/速度/1.5倍", "1.5倍", [], 266],
      ["rate-20", "播放/速度/2倍", "2倍", [], 267],
      ["rate-custom", "播放/速度/自定义…", "自定义…", [], 269],
      ["mute", "音频/静音", "静音", [], 386],
      ["volume", "音频/音量…", "音量…", [], 387],
      ["bar-visible", "视图/显示播放栏", "显示播放栏", [], 502],
    ],
  );
  const reason = "当前来源不支持此操作";
  for (const id of ["close-source", "toggle-pause", "jump", "backward-5", "rate-05", "rate-custom"])
    assert.deepEqual([host.command(id).state.enabled, host.command(id).state.reason], [false, reason], id);
  assert.equal(host.command("rate-10").state.checked, true);
  for (const id of ["open", "mute", "volume", "bar-visible"]) assert.equal(host.command(id).state.enabled, true, id);
  assert.equal(host.command("bar-visible").state.checked, true);
  const bar = host.mounted();
  assert.equal(host.sent("ui.mount")[0]!.component, "PlaybackBar");
  assert.equal(host.sent("ui.mount")[0]!.global, "PlaybackBridge");
  assert.equal(bar.slot, "bottom");
  assert.equal(bar.props["source-label"], "未打开媒体");
  assert.equal(bar.props.status, "未打开媒体");
  assert.equal(bar.props["time-label"], "--:-- / --:--");
  assert.equal(bar.props["can-pause"], false);
  assert.equal(bar.props["can-open"], true);
  assert.equal(bar.props["can-retry"], false);
  // The bar's buttons bypass the host's enabled check, so the plugin checks it.
  bar.emit("action", "toggle-pause");
  await host.settle();
  assert.equal(host.sent("source.submit").length, 0);
  await stop();
});

test("open asks for a file, is busy while selecting, and cancelling changes nothing", async () => {
  const { host, stop } = await start();
  host.dialogAnswers.push(null);
  await host.command("open").run();
  assert.equal(host.sent("dialogs.open")[0]!.title, "打开媒体");
  assert.equal(host.sent("source.submit").length, 0);
  host.dialogAnswers.push(["/a.mp4"]);
  await host.command("open").run();
  const open = host.sent("source.submit")[0]!.intent as Record<string, unknown>;
  assert.deepEqual([open.kind, open.source, open.startPaused, open.expected], ["open", "/a.mp4", false, undefined]);
  assert.equal(host.mounted().props["source-label"], "/a.mp4");
  assert.equal(host.mounted().props.status, "正在打开");
  host.setSource({ phase: "ready", duration: 60, position: 0 });
  await host.settle();
  // Replacing a playing source says so; while the dialog is up the command is busy.
  let answer = (_: string[] | null) => {};
  host.handle("dialogs.open", () => new Promise((resolve) => (answer = (paths) => resolve(paths ? { cancelled: false, paths } : { cancelled: true, paths: [] }))));
  host.emit("commands.execute", { id: "open" });
  await until(() => host.command("open").state.busy);
  assert.equal(host.sent("dialogs.open").at(-1)!.title, "打开媒体（将替换当前来源）");
  // Unchanged from before the migration: the generic reason is also shown while selecting.
  assert.deepEqual(host.command("open").state, { enabled: false, checked: null, busy: true, reason: "当前来源不支持此操作" });
  assert.equal(host.mounted().props["can-open"], false);
  answer(null);
  await host.settle();
  assert.equal(host.command("open").state.enabled, true);
  await stop();
});

test("play/pause follows the player; ended offers replay; errors offer retry", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  assert.equal(bar.props["pause-label"], "暂停");
  assert.equal(bar.props.status, "播放中");
  assert.equal(host.command("toggle-pause").state.checked, false);
  bar.emit("action", "toggle-pause");
  await host.settle();
  assert.equal(host.source.paused, true);
  assert.equal(bar.props["pause-label"], "播放");
  assert.equal(bar.props.status, "已暂停");
  assert.equal(host.command("toggle-pause").state.checked, true);
  const pause = host.sent("source.submit").at(-1)!.intent as Record<string, unknown>;
  assert.ok(pause.expected, "pause is conditional on the state the plugin saw");

  host.setSource({ phase: "ended", terminalReason: "natural" });
  await host.settle();
  assert.equal(bar.props["pause-label"], "重播");
  assert.equal(bar.props.status, "已结束");
  assert.equal(host.command("toggle-pause").state.checked, null);
  await host.command("toggle-pause").run();
  assert.equal((host.sent("source.submit").at(-1)!.intent as { kind: string }).kind, "restart");
  assert.equal(host.source.phase, "ready");

  host.setSource({ phase: "error", terminalReason: { failed: "decoder missing" } });
  await host.settle();
  assert.equal(bar.props.status, "打开失败：decoder missing");
  assert.equal(bar.props["can-retry"], true);
  bar.emit("retry");
  await host.settle();
  const retry = host.sent("source.submit").at(-1)!.intent as Record<string, unknown>;
  assert.deepEqual([retry.kind, retry.source], ["open", "/movie.mp4"]);
  assert.equal(bar.props["can-retry"], false);
  await stop();
});

test("a refused control shows the source failure and the next action clears it", async () => {
  const { host, stop } = await start(withMedia());
  host.handle("source.submit", () => {
    throw new HostError("stale", "stale");
  });
  await host.command("toggle-pause").run();
  assert.equal(host.mounted().props.status, "来源操作失败：stale");
  host.reset("source.submit");
  await host.command("rate-15").run();
  assert.equal(host.mounted().props.status, "播放中");
  assert.equal(host.mounted().props["rate-label"], "1.5倍");
  assert.equal(host.command("rate-15").state.checked, true);
  await stop();
});

test("a drag chains its own seeks: the release sample replaces intermediate points", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  bar.emit("dragging", true);
  assert.deepEqual(host.mounted().locks, null); // sent asynchronously
  bar.emit("seek-changed", 0.1);
  bar.emit("seek-changed", 0.2);
  bar.emit("seek-changed", 0.3);
  bar.emit("seek-changed", 0.37);
  await host.settle(0);
  assert.equal(bar.props.status, "跳转中");
  assert.deepEqual(host.mounted().locks, { focused: false, dragging: true, popupOpen: false });
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(seeks(host), [10, 37]);
  assert.equal(host.source.position, 37);
  assert.equal(bar.props.status, "播放中");
  assert.equal(bar.props["time-label"], "00:37 / 01:40");
  assert.deepEqual(host.mounted().locks, { focused: false, dragging: false, popupOpen: false });
  await stop();
});

test("cancel discards the unsent tail without undoing the accepted seek", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.1);
  bar.emit("seek-changed", 0.2);
  bar.emit("seek-changed", 0.3);
  bar.emit("seek-cancelled");
  await host.settle(80);
  assert.deepEqual(seeks(host), [10]);
  assert.equal(host.source.position, 10);
  assert.equal(bar.props.status, "播放中");
  // Moves after a cancel are ignored until the next press.
  bar.emit("seek-changed", 0.5);
  await host.settle(80);
  assert.deepEqual(seeks(host), [10]);
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.5);
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(seeks(host), [10, 50]);
  await stop();
});

test("an outside restart or open ends the gesture and drops its drafts", async () => {
  for (const outside of ["restart", "open"] as const) {
    const { host, stop } = await start(withMedia());
    const bar = host.mounted();
    bar.emit("dragging", true);
    bar.emit("seek-changed", 0.1);
    await host.settle(80);
    assert.deepEqual(seeks(host), [10]);
    // Someone else acts while the pointer is still down.
    if (outside === "open") {
      host.openElsewhere("/other.mp4");
      host.setSource({ duration: 100, position: 0 });
    } else host.setSource({ stamp: { ...host.source.stamp, intentVersion: host.source.stamp.intentVersion + 1 }, position: 0 });
    await host.settle();
    bar.emit("seek-changed", 0.6);
    await host.settle(80);
    assert.deepEqual(seeks(host), [10], outside);
    assert.equal(bar.props.status, "播放中", outside);
    assert.equal(bar.props["time-label"], "00:00 / 01:40", outside);
    assert.deepEqual(host.mounted().locks, { focused: false, dragging: false, popupOpen: false });
    await stop();
  }
});

test("a late failure from a cleared gesture does not report into the new one", async () => {
  const { host, stop } = await start(withMedia());
  const failures: (() => void)[] = [];
  host.handle("source.submit", () => new Promise((_, reject) => failures.push(() => reject(new HostError("backend", "old")))));
  const bar = host.mounted();
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.1);
  await until(() => failures.length === 1);
  bar.emit("dragging", true); // a new press clears the old gesture
  failures[0]!();
  await host.settle(60);
  assert.equal(bar.props.status, "播放中");
  assert.notEqual(bar.props.status, "来源操作失败：old");
  // A failure of the current gesture is shown.
  bar.emit("seek-changed", 0.2);
  await until(() => failures.length === 2);
  failures[1]!();
  await host.settle(60);
  assert.equal(bar.props.status, "来源操作失败：old");
  assert.equal(bar.props["time-label"], "00:00 / 01:40");
  await stop();
});

test("±5 seconds accumulate from the pending target and clamp to the source", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 3 });
  await host.settle();
  host.emit("commands.execute", { id: "forward-5" });
  host.emit("commands.execute", { id: "forward-5" });
  host.emit("commands.execute", { id: "forward-5" });
  await host.settle(80);
  assert.equal(host.source.position, 18);
  await host.command("backward-5").run();
  await host.settle(80);
  assert.equal(host.source.position, 13);
  host.setSource({ position: 2 });
  await host.settle();
  await host.command("backward-5").run();
  await host.settle(80);
  assert.equal(host.source.position, 0);
  await stop();
});

test("volume: latest value wins, volume before unmute, mute excluded while pending", async () => {
  const host = withMedia();
  const calls: [string, unknown][] = [];
  let release = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  host.handle("playback.set", async (p) => {
    calls.push([String(p.property), p.value]);
    if (p.property === "volume" && p.value === 80) await gate;
    host.source = { ...host.source, output: { ...host.source.output, [String(p.property)]: p.value } };
  });
  const { stop } = await start(host);
  const bar = host.mounted();
  bar.emit("volume-changed", 80);
  bar.emit("volume-changed", 60);
  bar.emit("volume-released", 35);
  bar.emit("action", "mute");
  await until(() => bar.props["volume-label"] === "35%");
  assert.equal(bar.props["can-mute"], false);
  assert.equal(bar.props["volume-label"], "35%");
  assert.equal(host.command("mute").state.busy, true);
  assert.deepEqual(calls, [["volume", 80]]);
  release();
  await host.settle();
  assert.deepEqual(calls, [["volume", 80], ["muted", false], ["volume", 35], ["muted", false]]);
  assert.equal(bar.props["can-mute"], true);
  assert.equal(bar.props.volume, 35);
  bar.emit("action", "mute");
  bar.emit("volume-changed", 90); // ignored: the mute is in flight
  await host.settle();
  assert.deepEqual(calls.at(-1), ["muted", true]);
  assert.equal(calls.some(([property, value]) => property === "volume" && value === 90), false);
  assert.equal(bar.props.muted, true);
  assert.equal(host.command("mute").state.checked, true);
  await stop();
});

test("the editor acts only on the source seen when it opened", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  host.setSource({ position: 30 });
  await host.settle();
  await host.command("jump").run();
  assert.equal(bar.props.editor, "jump");
  assert.equal(bar.props["editor-value"], "00:30");
  assert.deepEqual(bar.locks, { focused: true, dragging: false, popupOpen: true });
  bar.props["editor-value"] = "2:00"; // the user types
  bar.emit("confirm-editor", "2:00");
  await host.settle();
  assert.equal(bar.props.status, "请输入来源时长范围内的HH:MM:SS.mmm");
  assert.equal(bar.props.editor, "jump");
  assert.equal(bar.props["editor-value"], "2:00");
  bar.emit("confirm-editor", "1:05");
  await host.settle(80);
  assert.equal(host.source.position, 65);
  assert.equal(bar.props.editor, "");
  assert.deepEqual(bar.locks, { focused: false, dragging: false, popupOpen: false });

  // Custom rate: validated, then conditional on the state seen at open.
  bar.emit("action", "rate-custom");
  await host.settle();
  assert.equal(bar.props["editor-value"], "1");
  bar.emit("confirm-editor", "9");
  await host.settle();
  assert.equal(bar.props.status, "速度范围为0.25–4倍");
  bar.emit("confirm-editor", "1.25");
  await host.settle();
  assert.equal(host.source.rate, 1.25);
  const rate = host.sent("source.submit").at(-1)!.intent as Record<string, unknown>;
  assert.ok(rate.expected);

  // Volume editor.
  await host.command("volume").run();
  assert.equal(bar.props["editor-value"], "100");
  bar.emit("confirm-editor", "201");
  await host.settle();
  assert.equal(bar.props.status, "音量范围为0–200%");
  bar.emit("confirm-editor", "40");
  await host.settle();
  assert.equal(host.source.output.volume, 40);
  assert.equal(bar.props.editor, "");

  // Someone else pauses after the editor opened: the editor closes and a late confirm is refused.
  await host.command("jump").run();
  const before = host.sent("source.submit").length;
  host.setSource({ paused: true, stamp: { ...host.source.stamp, intentVersion: host.source.stamp.intentVersion + 1 } });
  await host.settle();
  assert.equal(bar.props.editor, "");
  bar.emit("confirm-editor", "0:10");
  await host.settle();
  assert.equal(bar.props.status, "来源已变化，请重新打开编辑器");
  assert.equal(host.sent("source.submit").length, before);

  // Cancel closes the editor and releases the popup lock.
  await host.command("rate-custom").run();
  bar.emit("cancel-editor");
  await host.settle();
  assert.equal(bar.props.editor, "");
  assert.deepEqual(bar.locks, { focused: false, dragging: false, popupOpen: false });
  await stop();
});

test("another source closes the editor, clears errors and resets the locks", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  await host.command("volume").run();
  bar.emit("confirm-editor", "x");
  await host.settle();
  assert.equal(bar.props.status, "音量范围为0–200%");
  host.openElsewhere("/next.mp4");
  await host.settle();
  assert.equal(bar.props.editor, "");
  assert.equal(bar.props.status, "播放中");
  assert.deepEqual(bar.locks, { focused: false, dragging: false, popupOpen: false });
  await stop();
});

test("focus locks follow the bar; the view command toggles the bar preference", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  bar.emit("interaction", true);
  await host.settle();
  assert.deepEqual(bar.locks, { focused: true, dragging: false, popupOpen: false });
  await host.command("bar-visible").run();
  assert.equal(host.layout.bottom?.preferred, false);
  assert.equal(host.command("bar-visible").state.checked, false);
  await host.command("bar-visible").run();
  assert.equal(host.layout.bottom?.preferred, true);
  assert.equal(host.command("bar-visible").state.checked, true);
  await host.command("close-source").run();
  assert.equal(host.source.phase, "empty");
  assert.equal((host.sent("source.submit").at(-1)!.intent as Record<string, unknown>).kind, "close");
  assert.equal(bar.props.status, "未打开媒体");
  await stop();
});

test("text typed into the editor survives other bar callbacks", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  host.setSource({ position: 30 });
  await host.settle();
  await host.command("jump").run();
  assert.equal(bar.props["editor-value"], "00:30");
  bar.props["editor-value"] = "1:05"; // the user types
  bar.emit("interaction", false); // focus leaves the field
  bar.emit("volume-changed", 80);
  await host.settle(80);
  assert.equal(bar.props["editor-value"], "1:05");
  bar.emit("cancel-editor");
  await host.settle();
  await host.command("jump").run(); // reopening writes the current position again
  assert.equal(bar.props["editor-value"], "00:30");
  await stop();
});
