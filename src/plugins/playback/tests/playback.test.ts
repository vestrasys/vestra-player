import assert from "node:assert/strict";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { parseTime, rateText, time } from "../format.ts";

const directory = new URL("..", import.meta.url);
/** The player registers the basic actions itself (`host.coreCommands`). */
const newHost = () => new TestHost();
const start = (host = newHost()) => startPlugin(activate, { directory, host });
/** A ready 100-second source opened by someone else. */
function withMedia(host = newHost()) {
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
const lastOsd = (host: TestHost) => host.osd.at(-1);
/** This plugin's own commands (the core's are in `host.coreCommands`). */
const own = (host: TestHost) => [...host.commands];

test("time input and labels", () => {
  assert.equal(parseTime("1:02:03.250"), 3723.25);
  assert.equal(parseTime("12:61"), null);
  assert.equal(parseTime("-1"), null);
  assert.equal(time(null), "--:--");
  assert.equal(time(3723), "1:02:03");
  assert.equal(rateText(1.1000000001), "1.1");
  assert.equal(rateText(2), "2");
});

test("menu follows the allocation table: positions, groups, radio items, bindings, repeat", async () => {
  const { host, stop } = await start();
  assert.deepEqual(
    own(host).map(([id, { declaration: d }]) => [id, d.path.join("/"), d.order, d.group, d.radio, d.bindings, d.repeat]),
    [
      ["open", "打开文件…", 100, "open", false, ["Ctrl+O", "F3"], false],
      ["open-url", "打开/打开链接…", 110, "open", false, ["Ctrl+U"], false],
      ["reopen", "重开当前/最后文件", 121, "open", false, ["Ctrl+Y"], false],
      ["copy-name", "复制到剪贴板/文件名", 125, "open", false, [], false],
      ["copy-folder", "复制到剪贴板/文件夹路径", 126, "open", false, [], false],
      ["copy-path", "复制到剪贴板/完整路径", 127, "open", false, [], false],
      ["keyframe-back", "播放/跳转/上一关键帧", 206, "frame", false, ["Ctrl+Shift+ArrowLeft"], true],
      ["keyframe-forward", "播放/跳转/下一关键帧", 207, "frame", false, ["Ctrl+Shift+ArrowRight"], true],
      ["jump", "播放/跳转到…", 218, "domain", false, ["G"], false],
      ["jump-start", "播放/跳转/跳到开头", 219, "points", false, ["Backspace"], false],
      ["time-jump", "播放/跳转/时间跨度设置…", 223, "jump-settings", false, [], false],
      ["rate-05", "播放/速度/0.5倍", 223, "domain", true, [], false],
      ["rate-15", "播放/速度/1.5倍", 224, "domain", true, [], false],
      ["rate-20", "播放/速度/2倍", 225, "domain", true, [], false],
      ["rate-custom", "播放/速度/自定义…", 226, "domain", false, [], false],
      ["prevent-sleep", "播放/播放时禁止屏保", 240, "playback-options", false, [], false],
      ["hover-time", "播放/进度条悬停显示时间", 242, "playback-options", false, [], false],
      ["hover-thumbnail", "播放/进度条悬停显示缩略图", 243, "playback-options", false, [], false],
      ["backward-2", "播放/跳转/后退30秒", 212, "step", false, ["Ctrl+ArrowLeft"], false],
      ["forward-2", "播放/跳转/前进30秒", 213, "step", false, ["Ctrl+ArrowRight"], false],
      ["backward-3", "播放/跳转/后退60秒", 214, "step", false, ["Shift+ArrowLeft"], false],
      ["forward-3", "播放/跳转/前进60秒", 215, "step", false, ["Shift+ArrowRight"], false],
      ["backward-4", "播放/跳转/后退300秒", 216, "step", false, ["Ctrl+Alt+ArrowLeft"], false],
      ["forward-4", "播放/跳转/前进300秒", 217, "step", false, ["Ctrl+Alt+ArrowRight"], false],
    ],
  );
  // No command clashes with the core's basic actions.
  assert.deepEqual(host.conflicts, []);
  const reason = "当前来源不支持此操作";
  for (const id of ["jump", "jump-start", "backward-2", "forward-4", "rate-05", "rate-custom", "keyframe-back"])
    assert.deepEqual([host.command(id).state.enabled, host.command(id).state.reason], [false, reason], id);
  // First run: nothing played yet, so nothing to reopen.
  assert.deepEqual([host.command("reopen").state.enabled, host.command("reopen").state.reason], [false, "没有可重开的文件"]);
  for (const id of ["copy-name", "copy-folder", "copy-path"])
    assert.deepEqual([host.command(id).state.enabled, host.command(id).state.reason], [false, "没有打开的文件"], id);
  assert.equal(host.command("prevent-sleep").state.checked, true);
  // IA-PB-01: the hover time is off by default.
  assert.equal(host.command("hover-time").state.checked, false);
  for (const id of ["open", "open-url", "prevent-sleep", "hover-time", "time-jump"])
    assert.equal(host.command(id).state.enabled, true, id);

  const bar = host.mounted();
  assert.equal(host.sent("ui.mount")[0]!.component, "PlaybackBar");
  assert.equal(host.sent("ui.mount")[0]!.global, "PlaybackBridge");
  assert.equal(bar.slot, "bottom");
  for (const removed of ["source-label", "status", "rate-label", "editor", "can-retry", "volume-label"])
    assert.equal(removed in bar.props, false, removed);
  assert.equal(bar.props["elapsed-label"], "--:--");
  assert.equal(bar.props["total-label"], "--:--");
  for (const flag of ["can-pause", "can-stop", "can-step", "can-previous", "can-next", "playing", "has-sidebar"])
    assert.equal(bar.props[flag], false, flag);
  assert.equal(bar.props["can-open"], true);
  assert.equal(bar.props.duration, 0);
  assert.equal(bar.props["hover-time"], false);
  // A press on a button the player has just disabled is dropped without a message.
  bar.emit("action", "play-pause");
  bar.emit("action", "previous");
  bar.emit("action", "toggle-pause"); // not an action the bar knows
  await host.settle();
  assert.deepEqual(host.sent("host.run").map((p) => p.action), ["play-pause", "previous"]);
  assert.deepEqual(host.coreRuns, []);
  assert.deepEqual(host.osd, []);
  await stop();
});

test("the basic actions are the core's: the keys reach it with or without this plugin", async () => {
  const { host, stop } = await start(withMedia());
  for (const key of ["Space", "Space", "ArrowRight", "ArrowLeft", "ArrowUp", "ArrowDown", "M", "M", "C", "X", "Z", "D", "F"])
    await host.press(key);
  await host.settle();
  assert.deepEqual(host.coreRuns, [
    "play-pause",
    "play-pause",
    "seek-forward",
    "seek-backward",
    "volume-up",
    "volume-down",
    "mute",
    "mute",
    "rate-up",
    "rate-down",
    "rate-normal",
    "frame-back",
    "frame-forward",
  ]);
  await host.press("F4");
  assert.deepEqual(host.coreRuns.at(-1), "stop");
  await stop();
  // Stopped: the same keys still work.
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 0 });
  await host.press("Space");
  assert.equal(host.coreRuns.at(-1), "play-pause");
  assert.equal(lastOsd(host), "已暂停");
});

test("open asks for a file (Ctrl+O, F3), is busy while selecting, and cancelling changes nothing", async () => {
  const { host, stop } = await start();
  host.dialogAnswers.push(null);
  await host.press("Ctrl+O");
  assert.equal(host.sent("dialogs.open")[0]!.title, "打开媒体");
  assert.equal(host.sent("source.submit").length, 0);
  host.dialogAnswers.push(["/a.mp4"]);
  await host.press("F3");
  const open = host.sent("source.submit")[0]!.intent as Record<string, unknown>;
  assert.deepEqual([open.kind, open.source, open.startPaused, open.expected], ["open", "/a.mp4", false, undefined]);
  host.setSource({ phase: "ready", duration: 60, position: 0 });
  await host.settle();
  assert.equal(host.mounted().props["total-label"], "01:00");
  // Replacing a playing source says so; while the dialog is up the command is busy.
  let answer = (_: string[] | null) => {};
  host.handle("dialogs.open", () => new Promise((resolve) => (answer = (paths) => resolve(paths ? { cancelled: false, paths } : { cancelled: true, paths: [] }))));
  host.mounted().emit("action", "open");
  await until(() => host.command("open").state.busy);
  assert.equal(host.sent("dialogs.open").at(-1)!.title, "打开媒体（将替换当前来源）");
  assert.deepEqual(host.command("open").state, { enabled: false, checked: null, busy: true, reason: "当前来源不支持此操作" });
  assert.equal(host.mounted().props["can-open"], false);
  answer(null);
  await host.settle();
  assert.equal(host.command("open").state.enabled, true);
  await stop();
});

test("play/pause follows the player with OSD; ended offers replay; a failed load is announced and reopened (Ctrl+Y)", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  assert.equal(bar.props.playing, true);
  assert.equal(bar.props["can-pause"], true);
  assert.equal(bar.props["can-stop"], true);
  bar.emit("action", "play-pause");
  await host.settle();
  assert.equal(host.source.paused, true);
  assert.equal(bar.props.playing, false);
  assert.equal(lastOsd(host), "已暂停"); // the core's text, the same as Space
  assert.equal(host.coreAction("play-pause").state.checked, true);
  await host.press("Space");
  assert.equal(host.source.paused, false);
  assert.equal(lastOsd(host), "播放");

  host.setSource({ phase: "ended", terminalReason: "natural" });
  await host.settle();
  assert.equal(bar.props.playing, false);
  assert.equal(bar.props["can-pause"], true);
  bar.emit("action", "play-pause"); // the core replays after the end
  await host.settle();
  assert.equal(host.source.phase, "ready");
  assert.deepEqual(host.coreRuns, ["play-pause", "play-pause", "play-pause"]);

  host.setSource({ phase: "error", terminalReason: { failed: "decoder missing" } });
  await host.settle();
  assert.equal(lastOsd(host), "打开失败：decoder missing");
  const announced = host.osd.length;
  host.setSource({ position: 1 }); // more events in the same failed state are not announced again
  await host.settle();
  assert.equal(host.osd.length, announced);
  assert.equal(host.command("reopen").state.enabled, true);
  await host.press("Ctrl+Y");
  const retry = host.sent("source.submit").at(-1)!.intent as Record<string, unknown>;
  assert.deepEqual([retry.kind, retry.source], ["open", "/movie.mp4"]);
  assert.equal(lastOsd(host), "重新打开: movie.mp4");
  await stop();
});

test("stop closes the source (button and F4)", async () => {
  const { host, stop } = await start(withMedia());
  host.mounted().emit("action", "stop");
  await host.settle();
  assert.equal(host.source.phase, "empty");
  assert.deepEqual(host.coreRuns, ["stop"]);
  assert.equal(host.mounted().props["can-stop"], false);
  await assert.rejects(host.press("F4")); // disabled now
  await stop();
});

test("a refused control is shown on the video", async () => {
  const { host, stop } = await start(withMedia());
  // A core action the player could not carry out, run from the bar.
  host.handle("host.run", () => {
    throw new HostError("backend", "stale");
  });
  host.mounted().emit("action", "play-pause");
  await host.settle();
  assert.equal(lastOsd(host), "来源操作失败：stale");
  host.reset("host.run");
  // One of this plugin's own controls.
  host.handle("source.submit", () => {
    throw new HostError("stale", "stale");
  });
  await host.command("rate-15").run();
  await host.settle();
  assert.equal(lastOsd(host), "来源操作失败：stale");
  host.reset("source.submit");
  await host.command("rate-15").run();
  assert.equal(lastOsd(host), "播放速度: 1.5x");
  assert.equal(host.command("rate-15").state.checked, true);
  assert.equal(host.coreAction("rate-normal").state.checked, false);
  await stop();
});

test("a failing OSD never fails the action", async () => {
  const host = withMedia();
  host.handle("osd.show", () => {
    throw new HostError("unavailable", "headless");
  });
  const { stop } = await start(host);
  await host.command("rate-15").run();
  await host.settle();
  assert.equal(host.source.rate, 1.5);
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
  assert.equal(bar.props["elapsed-label"], "00:37");
  assert.deepEqual(host.mounted().locks, { focused: false, dragging: true, popupOpen: false });
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(seeks(host), [10, 37]);
  assert.equal(host.source.position, 37);
  assert.equal(bar.props["elapsed-label"], "00:37");
  assert.equal(bar.props["total-label"], "01:40");
  assert.equal(lastOsd(host), "跳转: 00:37 / 01:40");
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
    assert.equal(bar.props["elapsed-label"], "00:00", outside);
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
  assert.equal(host.osd.includes("来源操作失败：old"), false);
  // A failure of the current gesture is shown.
  bar.emit("seek-changed", 0.2);
  await until(() => failures.length === 2);
  failures[1]!();
  await host.settle(60);
  assert.equal(lastOsd(host), "来源操作失败：old");
  assert.equal(bar.props["elapsed-label"], "00:00");
  await stop();
});

test("relative seeks accumulate from the pending target, clamp to the source and show the target", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 3 });
  await host.settle();
  host.emit("commands.execute", { id: "forward-2" });
  host.emit("commands.execute", { id: "forward-2" });
  await host.settle(80);
  assert.equal(host.source.position, 63);
  assert.equal(lastOsd(host), "前进 30秒: 01:03 / 01:40");
  await host.press("Ctrl+ArrowLeft");
  await host.settle(80);
  assert.equal(host.source.position, 33);
  assert.equal(lastOsd(host), "后退 30秒: 00:33 / 01:40");
  await host.press("Shift+ArrowLeft");
  await host.settle(80);
  assert.equal(host.source.position, 0);
  assert.equal(lastOsd(host), "后退 60秒: 00:00 / 01:40");
  await host.press("Ctrl+Alt+ArrowRight");
  await host.settle(80);
  assert.equal(host.source.position, 100);
  // ← / → and the bar's back/forward buttons are the core's first tier.
  host.mounted().emit("action", "seek-backward");
  await host.settle(80);
  assert.equal(host.source.position, 95);
  assert.equal(lastOsd(host), "后退 5秒: 01:35 / 01:40");
  host.mounted().emit("action", "seek-forward");
  await host.settle(80);
  assert.deepEqual(host.coreRuns, ["seek-backward", "seek-forward"]);
  await stop();
});

test("previous/next are the core's: PageUp/PageDown and right-click on back/forward", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  await assert.rejects(host.press("PageDown"));
  host.sequence = { canNext: true, canPrevious: true };
  host.setSource({ sequence: { next: true, previous: true } } as never);
  await host.settle();
  assert.equal(bar.props["can-next"], true);
  assert.equal(bar.props["can-previous"], true);
  await host.press("PageDown");
  await host.press("Mouse:X1");
  bar.emit("action", "previous"); // right-click on 后退
  bar.emit("action", "next"); // right-click on 前进
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next", "previous", "previous", "next"]);
  // No sequence any more: the core refuses, the bar says nothing.
  host.sequence = null;
  const shown = host.osd.length;
  bar.emit("action", "next");
  await host.settle();
  assert.equal(host.coreRuns.length, 4);
  assert.equal(host.osd.length, shown);
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
  bar.emit("action", "mute"); // dropped: the volume has not landed yet
  await until(() => bar.props.volume === 35);
  assert.equal(bar.props["can-mute"], false);
  assert.deepEqual(calls, [["volume", 80]]);
  assert.equal(lastOsd(host), "音量: 35%");
  release();
  await host.settle();
  assert.deepEqual(calls, [["volume", 80], ["muted", false], ["volume", 35], ["muted", false]]);
  assert.deepEqual(host.coreRuns, []);
  assert.equal(bar.props["can-mute"], true);
  assert.equal(bar.props.volume, 35);

  // The mute button is the core's mute; the slider waits for it.
  let unmute = () => {};
  host.handle("host.run", (p) => {
    assert.equal(p.action, "mute");
    return new Promise<void>((resolve) => (unmute = () => {
      host.setSource({ output: { ...host.source.output, muted: true } });
      resolve();
    }));
  });
  bar.emit("action", "mute");
  await until(() => bar.props["can-volume"] === false);
  bar.emit("volume-changed", 90); // ignored: the mute is in flight
  unmute();
  await host.settle();
  assert.equal(calls.some(([property, value]) => property === "volume" && value === 90), false);
  assert.equal(bar.props.muted, true);
  assert.equal(bar.props["can-volume"], true);
  host.reset("host.run");
  bar.emit("action", "mute");
  await host.settle();
  assert.equal(bar.props.muted, false);
  assert.equal(lastOsd(host), "静音: 关");
  await stop();
});

test("G-13b the volume slider runs 0–100 (volume before unmute); a louder volume shows at the right end", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ output: { volume: 100, muted: true } });
  await host.settle();
  const bar = host.mounted();
  bar.emit("volume-released", 250);
  await host.settle();
  assert.deepEqual(host.source.output, { volume: 100, muted: false });
  assert.equal(lastOsd(host), "音量: 100%");
  bar.emit("volume-released", 50);
  await host.settle();
  assert.equal(lastOsd(host), "音量: 50%");
  bar.emit("volume-released", -3);
  await host.settle();
  assert.equal(host.source.output.volume, 0);
  // Set above 100 by someone else: the slider sits at its right end, the tooltip tells the truth.
  host.setSource({ output: { volume: 150, muted: false } });
  await host.settle();
  assert.equal(bar.props.volume, 100);
  assert.equal(bar.props["tip-volume"], "音量: 150%");
  await stop();
});

test("speed: the fixed rates are radio items next to the core's 正常, Z/X/C are the core's", async () => {
  const { host, stop } = await start(withMedia());
  await host.command("rate-05").run();
  assert.equal(host.source.rate, 0.5);
  assert.equal(lastOsd(host), "播放速度: 0.5x");
  assert.equal(host.command("rate-05").state.checked, true);
  assert.equal(host.coreAction("rate-normal").state.checked, false);
  await host.press("C");
  assert.equal(host.source.rate, 0.6);
  assert.equal(host.command("rate-05").state.checked, false);
  await host.command("rate-20").run();
  assert.equal(host.command("rate-20").state.checked, true);
  await host.press("Z");
  assert.equal(host.source.rate, 1);
  assert.equal(host.coreAction("rate-normal").state.checked, true);
  for (const id of ["rate-05", "rate-15", "rate-20"]) assert.equal(host.command(id).state.checked, false, id);
  assert.deepEqual(host.coreRuns, ["rate-up", "rate-normal"]);
  await stop();
});

test("跳转到 (G) is a separate window bound to the source it opened for", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 30 });
  await host.settle();
  await host.press("G");
  await host.settle();
  const window = host.window("playback-jump");
  assert.equal(window.file, "ui/prompt.slint");
  assert.equal(window.visible, true);
  assert.equal(window.props["window-title"], "跳转到");
  assert.equal(window.props["group-title"], "时间定位(时:分:秒.毫秒)");
  assert.equal(window.props["action-label"], "定位");
  assert.equal(window.props.value, "00:30");
  assert.equal(host.mounted().props["can-seek"], true); // the bar stays as it was: no editor row
  window.emit("confirm", "2:00");
  await host.settle();
  assert.equal(window.props.message, "请输入来源时长范围内的HH:MM:SS.mmm");
  assert.equal(window.visible, true);
  window.emit("confirm", "1:05");
  await host.settle(80);
  assert.equal(host.source.position, 65);
  assert.equal(lastOsd(host), "跳转: 01:05 / 01:40");
  assert.equal(window.visible, false);

  // Reopening writes the current position again and clears the message.
  await host.command("jump").run();
  await host.settle();
  assert.equal(window.visible, true);
  assert.equal(window.props.value, "01:05");
  assert.equal(window.props.message, "");
  // A pause by someone else does not invalidate it; the jump applies to the current state.
  host.setSource({ paused: true, stamp: { ...host.source.stamp, intentVersion: host.source.stamp.intentVersion + 1 } });
  await host.settle();
  window.emit("confirm", "0:10");
  await host.settle(80);
  assert.equal(host.source.position, 10);

  // Another source hides it, and a late confirm is refused without seeking.
  await host.command("jump").run();
  host.openElsewhere("/next.mp4");
  host.setSource({ duration: 100, position: 0 });
  await host.settle();
  assert.equal(window.visible, false);
  const before = seeks(host).length;
  window.emit("confirm", "0:20");
  await host.settle(80);
  assert.equal(window.props.message, "来源已变化，请重新打开");
  assert.equal(seeks(host).length, before);

  // Cancel (Escape) hides it.
  await host.command("jump").run();
  window.emit("cancel");
  await host.settle();
  assert.equal(window.visible, false);
  await stop();
});

test("custom speed uses its own small window, validated, conditional on the current state", async () => {
  const { host, stop } = await start(withMedia());
  await host.command("rate-custom").run();
  await host.settle();
  const window = host.window("playback-rate");
  assert.equal(window.props["window-title"], "自定义速度");
  assert.equal(window.props.value, "1");
  window.emit("confirm", "9");
  await host.settle();
  assert.equal(window.props.message, "速度范围为0.25–4倍");
  window.emit("confirm", "");
  await host.settle();
  assert.equal(window.props.message, "速度范围为0.25–4倍");
  window.emit("confirm", "1.25");
  await host.settle();
  assert.equal(host.source.rate, 1.25);
  assert.equal(lastOsd(host), "播放速度: 1.25x");
  assert.ok((host.sent("source.submit").at(-1)!.intent as Record<string, unknown>).expected);
  assert.equal(window.visible, false);
  for (const id of ["rate-05", "rate-15", "rate-20"]) assert.equal(host.command(id).state.checked, false, id);
  assert.equal(host.coreAction("rate-normal").state.checked, false);
  await stop();
});

test("the time label toggles between elapsed and remaining", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  host.setSource({ position: 12 });
  await host.settle();
  assert.equal(bar.props["elapsed-label"], "00:12");
  bar.emit("toggle-time");
  await host.settle();
  assert.equal(bar.props["elapsed-label"], "-01:28");
  assert.equal(bar.props["total-label"], "01:40");
  bar.emit("toggle-time");
  await host.settle();
  assert.equal(bar.props["elapsed-label"], "00:12");
  await stop();
});

test("the playlist button reads and toggles the sidebar position, whoever mounted it", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  assert.equal(bar.props["has-sidebar"], false);
  host.layout.sidebar = { owner: 2, preferred: true, effective: true, reason: null, detached: false };
  host.emit("window.state", {});
  await host.settle();
  assert.equal(bar.props["has-sidebar"], true);
  assert.equal(bar.props["sidebar-on"], true);
  bar.emit("toggle-sidebar");
  await host.settle();
  assert.deepEqual(
    host.sent("layout.set").map((p) => [p.slot, p.visible]),
    [["sidebar", false]],
  );
  assert.equal(bar.props["sidebar-on"], false);
  // A change made elsewhere (the playlist's own F6) is followed.
  host.layout.sidebar = { ...host.layout.sidebar, preferred: true, effective: true };
  host.emit("window.state", {});
  await host.settle();
  assert.equal(bar.props["sidebar-on"], true);
  await stop();
});

test("dragging the volume keeps the bar shown until release", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  bar.emit("volume-changed", 50);
  await host.settle();
  assert.deepEqual(bar.locks, { focused: false, dragging: true, popupOpen: false });
  bar.emit("volume-released", 40);
  await host.settle();
  assert.deepEqual(bar.locks, { focused: false, dragging: false, popupOpen: false });
  assert.equal(host.source.output.volume, 40);
  await stop();
});
