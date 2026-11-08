import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { Text } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { parseTime, rateText, time } from "../format.ts";
import { eventually } from "./eventually.ts";

const directory = new URL("..", import.meta.url);
/** The player registers the basic actions itself (`host.coreCommands`). */
const newHost = () => new TestHost();
/** In Chinese: the texts the plugin showed before English became the source. */
const start = (host = newHost(), locale: "en" | "zh-Hans" = "zh-Hans") =>
  startPlugin(activate, { directory, host, locale });
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
/** 0.2.3: every seek as it went on the wire: the target, and `keyframes` when the plugin sent it. */
const wireSeeks = (host: TestHost) =>
  host
    .sent("source.submit")
    .map((p) => p.intent as { kind: string; control?: { kind: string; seconds?: number; keyframes?: boolean } })
    .filter((i) => i.kind === "control" && i.control?.kind === "seek")
    .map((i) => ({ seconds: i.control!.seconds, ...("keyframes" in i.control! ? { keyframes: i.control!.keyframes } : {}) }));
/** The plugin started with a saved `settings.json`. */
async function startWith(settings: Record<string, unknown>, host = withMedia()) {
  const data = mkdtempSync(join(tmpdir(), "playback-bar-"));
  writeFileSync(join(data, "settings.json"), JSON.stringify(settings));
  const started = await startPlugin(activate, { directory, host, locale: "zh-Hans", dataDirectory: data });
  return {
    host,
    stop: async () => {
      await started.stop();
      rmSync(data, { recursive: true, force: true });
    },
  };
}
const lastOsd = (host: TestHost) => host.osd.at(-1);
/** This plugin's own commands (the core's are in `host.coreCommands`), A-B Loop's `ab.*` aside (loop.test.ts). */
const own = (host: TestHost) => [...host.commands].filter(([id]) => !id.startsWith("ab."));

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
  // The menu as a Chinese user sees it: the texts before English became the source. Menu redesign
  // (step 7): the speed and seek commands are in the command palette only (no path), with their
  // keys, and name the Playback panel (`home: "panel"`) that Playback… (Alt+4) opens. Menu sweep
  // (B5): Reopen, Copy to Clipboard and the three Playback options are palette only too, and so
  // is Playback… (user ruling 2026-11-08: the player's Adjustments and Effects opens the column).
  assert.deepEqual(
    own(host).map(([id, { declaration: d, display }]) => [
      id,
      display.path.length ? display.path.join("/") : `(${display.title})`,
      display.path.length ? d.order : null,
      display.path.length ? d.group : null,
      d.radio,
      d.bindings,
      d.repeat,
      d.home ?? null,
    ]),
    [
      ["open", "打开文件…", 100, "open", false, ["Ctrl+O", "F3"], false, null],
      ["open-url", "打开/打开链接…", 110, "open", false, ["Ctrl+U"], false, null],
      ["reopen", "(重开当前/最后文件)", null, null, false, ["Ctrl+Y"], false, null],
      ["copy-name", "(文件名)", null, null, false, [], false, null],
      ["copy-folder", "(文件夹路径)", null, null, false, [], false, null],
      ["copy-path", "(完整路径)", null, null, false, [], false, null],
      ["panel", "(播放…)", null, null, false, ["Alt+4"], false, null],
      ["keyframe-back", "(上一关键帧)", null, null, false, ["Ctrl+Shift+ArrowLeft"], true, "panel"],
      ["keyframe-forward", "(下一关键帧)", null, null, false, ["Ctrl+Shift+ArrowRight"], true, "panel"],
      ["jump", "(跳转到…)", null, null, false, ["G"], false, "panel"],
      ["jump-start", "(跳到开头)", null, null, false, ["Backspace"], false, "panel"],
      ["time-jump", "(时间跨度设置…)", null, null, false, [], false, "panel"],
      ["rate-05", "(0.5倍)", null, null, true, [], false, "panel"],
      ["rate-15", "(1.5倍)", null, null, true, [], false, "panel"],
      ["rate-20", "(2倍)", null, null, true, [], false, "panel"],
      ["rate-custom", "(自定义…)", null, null, false, [], false, "panel"],
      ["prevent-sleep", "(播放时禁止屏保)", null, null, false, [], false, null],
      ["hover-time", "(进度条悬停显示时间)", null, null, false, [], false, null],
      ["hover-thumbnail", "(进度条悬停显示缩略图)", null, null, false, [], false, null],
      ["backward-2", "(后退30秒)", null, null, false, ["Ctrl+ArrowLeft"], false, "panel"],
      ["forward-2", "(前进30秒)", null, null, false, ["Ctrl+ArrowRight"], false, "panel"],
      ["backward-3", "(后退60秒)", null, null, false, ["Shift+ArrowLeft"], false, "panel"],
      ["forward-3", "(前进60秒)", null, null, false, ["Shift+ArrowRight"], false, "panel"],
      ["backward-4", "(后退300秒)", null, null, false, ["Ctrl+Alt+ArrowLeft"], false, "panel"],
      ["forward-4", "(前进300秒)", null, null, false, ["Ctrl+Alt+ArrowRight"], false, "panel"],
    ],
  );
  // The declared English paths and titles (a text value as its source and args).
  const english = (segment: string | Text) =>
    typeof segment === "string" ? segment : segment.$vestraText.replace("{}", String(Object.values(segment.args ?? {})[0]));
  assert.deepEqual(
    own(host).map(([id, { declaration: d }]) => [id, d.path.length ? d.path.map(english).join("/") : `(${english(d.title!)})`]),
    [
      ["open", "Open File…"],
      ["open-url", "@open/Open URL…"],
      ["reopen", "(Reopen Current/Last File)"],
      ["copy-name", "(File Name)"],
      ["copy-folder", "(Folder Path)"],
      ["copy-path", "(Full Path)"],
      ["panel", "(Playback…)"],
      ["keyframe-back", "(Previous Keyframe)"],
      ["keyframe-forward", "(Next Keyframe)"],
      ["jump", "(Jump to Time…)"],
      ["jump-start", "(Jump to Start)"],
      ["time-jump", "(Seek Steps…)"],
      ["rate-05", "(0.5×)"],
      ["rate-15", "(1.5×)"],
      ["rate-20", "(2×)"],
      ["rate-custom", "(Custom…)"],
      ["prevent-sleep", "(Disable Screen Saver While Playing)"],
      ["hover-time", "(Show Time on Seek Bar Hover)"],
      ["hover-thumbnail", "(Show Thumbnail on Seek Bar Hover)"],
      ["backward-2", "(Back 30 s)"],
      ["forward-2", "(Forward 30 s)"],
      ["backward-3", "(Back 60 s)"],
      ["forward-3", "(Forward 60 s)"],
      ["backward-4", "(Back 300 s)"],
      ["forward-4", "(Forward 300 s)"],
    ],
  );
  // No command clashes with the core's basic actions.
  assert.deepEqual(host.conflicts, []);
  const reasonOf = (id: string) => {
    const { enabled, reason } = host.command(id).state;
    return [enabled, reason === null ? null : host.translate(reason)];
  };
  const reason = "当前来源不支持此操作";
  for (const id of ["jump", "jump-start", "backward-2", "forward-4", "rate-05", "rate-custom", "keyframe-back"])
    assert.deepEqual(reasonOf(id), [false, reason], id);
  // First run: nothing played yet, so nothing to reopen.
  assert.deepEqual(reasonOf("reopen"), [false, "没有可重开的文件"]);
  for (const id of ["copy-name", "copy-folder", "copy-path"]) assert.deepEqual(reasonOf(id), [false, "没有打开的文件"], id);
  assert.equal(host.command("prevent-sleep").state.checked, true);
  // IA-PB-01: the hover time is off by default.
  assert.equal(host.command("hover-time").state.checked, false);
  for (const id of ["open", "open-url", "prevent-sleep", "hover-time", "time-jump"])
    assert.equal(host.command(id).state.enabled, true, id);

  const bar = host.mounted("bottom");
  assert.equal(host.sent("ui.mount")[0]!.component, "PlaybackBar");
  assert.equal(host.sent("ui.mount")[0]!.global, "PlaybackBridge");
  assert.equal(bar.slot, "bottom");
  // 0.2.14: the seek buttons' data went with them (the bar's arrows are previous/next).
  for (const removed of ["source-label", "status", "rate-label", "editor", "can-retry", "volume-label", "can-step", "key-back", "key-forward", "step-text"])
    assert.equal(removed in bar.props, false, removed);
  assert.equal(bar.props["elapsed-label"], "--:--");
  assert.equal(bar.props["total-label"], "--:--");
  for (const flag of ["can-pause", "can-stop", "can-previous", "can-next", "playing"])
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
  // 0.2.15: the command palette's category (Category: Title), in Chinese.
  assert.deepEqual(
    own(host).map(([id, command]) => [id, command.display.category]),
    [
      ["open", "文件"],
      ["open-url", "文件"],
      ["reopen", "文件"],
      ["copy-name", "复制到剪贴板"],
      ["copy-folder", "复制到剪贴板"],
      ["copy-path", "复制到剪贴板"],
      ["panel", "播放"],
      ["keyframe-back", "跳转"],
      ["keyframe-forward", "跳转"],
      ["jump", "跳转"],
      ["jump-start", "跳转"],
      ["time-jump", "跳转"],
      ["rate-05", "速度"],
      ["rate-15", "速度"],
      ["rate-20", "速度"],
      ["rate-custom", "速度"],
      ["prevent-sleep", "播放"],
      ["hover-time", "播放"],
      ["hover-thumbnail", "播放"],
      ["backward-2", "跳转"],
      ["forward-2", "跳转"],
      ["backward-3", "跳转"],
      ["forward-3", "跳转"],
      ["backward-4", "跳转"],
      ["forward-4", "跳转"],
    ],
  );
  assert.deepEqual(host.coreRuns.at(-1), "stop");
  await stop();
  // Stopped: the same keys still work.
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 0 });
  await host.press("Space");
  assert.equal(host.coreRuns.at(-1), "play-pause");
  assert.equal(lastOsd(host), "已暂停");
});

test("every command has a non-empty category (the command palette's Category: Title)", async () => {
  const { host, stop } = await start(withMedia(), "en");
  assert.equal(own(host).length, 25);
  const counts = new Map<string, number>();
  for (const [id, command] of own(host)) {
    const category = command.display.category;
    assert.ok(typeof category === "string" && category.trim() !== "", `${id} has no category`);
    // Declared as a text value; in English the shown category is its source text.
    assert.ok(command.declaration.category, `${id} declares no category`);
    counts.set(category, (counts.get(category) ?? 0) + 1);
  }
  // The seek steps (text-valued titles) are "Seek" too.
  assert.deepEqual(Object.fromEntries(counts), { File: 3, "Copy to Clipboard": 3, Seek: 11, Playback: 4, Speed: 4 });
  await stop();
});

test("in English the menus, reasons and messages are the source texts", async () => {
  const { host, stop } = await start(withMedia(), "en");
  assert.deepEqual(host.command("open").display.path, ["Open File…"]);
  assert.deepEqual(host.command("open-url").display.path, ["Open", "Open URL…"]);
  assert.deepEqual([host.command("backward-2").display.title, host.command("backward-2").display.category], ["Back 30 s", "Seek"]);
  assert.deepEqual([host.command("rate-05").display.title, host.command("rate-05").display.category], ["0.5×", "Speed"]);
  assert.deepEqual([host.command("panel").display.path, host.command("panel").display.title], [[], "Playback…"]);
  // Menu sweep (B5): Copy to Clipboard is in the command palette only.
  assert.deepEqual([host.command("copy-path").display.path, host.command("copy-path").display.title], [[], "Full Path"]);
  await host.command("rate-05").run();
  assert.equal(lastOsd(host), "Speed: 0.5x");
  await host.press("Ctrl+ArrowRight");
  await host.settle(80);
  assert.equal(lastOsd(host), "Forward 30 s: 00:30 / 01:40");
  await host.press("F4");
  await host.settle();
  assert.equal(host.translate(host.command("jump").state.reason!), "Not supported by the current source");
  await stop();
});

test("open asks for a file (Ctrl+O, F3), is busy while selecting, and cancelling changes nothing", async () => {
  const { host, stop } = await start();
  host.dialogAnswers.push(null);
  await host.press("Ctrl+O");
  assert.equal(host.sent("dialogs.open")[0]!.title, "Open Media");
  assert.equal(host.translate("Open Media"), "打开媒体");
  assert.equal(host.sent("source.submit").length, 0);
  host.dialogAnswers.push(["/a.mp4"]);
  await host.press("F3");
  const open = host.sent("source.submit")[0]!.intent as Record<string, unknown>;
  assert.deepEqual([open.kind, open.source, open.startPaused, open.expected], ["open", "/a.mp4", false, undefined]);
  host.setSource({ phase: "ready", duration: 60, position: 0 });
  await host.settle();
  assert.equal(host.mounted("bottom").props["total-label"], "01:00");
  // Replacing a playing source says so; while the dialog is up the command is busy.
  let answer = (_: string[] | null) => {};
  host.handle("dialogs.open", () => new Promise((resolve) => (answer = (paths) => resolve(paths ? { cancelled: false, paths } : { cancelled: true, paths: [] }))));
  host.mounted("bottom").emit("action", "open");
  await until(() => host.command("open").state.busy);
  assert.equal(host.sent("dialogs.open").at(-1)!.title, "Open Media (replaces the current source)");
  assert.equal(host.translate("Open Media (replaces the current source)"), "打开媒体（将替换当前来源）");
  assert.deepEqual(host.command("open").state, {
    enabled: false,
    checked: null,
    busy: true,
    reason: "Not supported by the current source",
  });
  assert.equal(host.translate("Not supported by the current source"), "当前来源不支持此操作");
  assert.equal(host.mounted("bottom").props["can-open"], false);
  answer(null);
  await host.settle();
  assert.equal(host.command("open").state.enabled, true);
  await stop();
});

test("play/pause follows the player with OSD; ended offers replay; a failed load is announced and reopened (Ctrl+Y)", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
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
  host.mounted("bottom").emit("action", "stop");
  await host.settle();
  assert.equal(host.source.phase, "empty");
  assert.deepEqual(host.coreRuns, ["stop"]);
  assert.equal(host.mounted("bottom").props["can-stop"], false);
  // disabled now: the key shows the reason on the video and runs nothing (SDK 1.5.12, as the player)
  const shown = host.osd.length;
  assert.equal(await host.press("F4"), "reason");
  assert.deepEqual(host.coreRuns, ["stop"]);
  assert.deepEqual(host.osd.slice(shown), ["当前来源不支持此操作"]);
  await stop();
});

test("a refused control is shown on the video", async () => {
  const { host, stop } = await start(withMedia());
  // A core action the player could not carry out, run from the bar.
  host.handle("host.run", () => {
    throw new HostError("backend", "stale");
  });
  host.mounted("bottom").emit("action", "play-pause");
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
  const bar = host.mounted("bottom");
  bar.emit("dragging", true);
  assert.deepEqual(host.mounted("bottom").locks, null); // sent asynchronously
  bar.emit("seek-changed", 0.1);
  bar.emit("seek-changed", 0.2);
  bar.emit("seek-changed", 0.3);
  bar.emit("seek-changed", 0.37);
  await host.settle(0);
  assert.equal(bar.props["elapsed-label"], "00:37");
  assert.deepEqual(host.mounted("bottom").locks, { focused: false, dragging: true, popupOpen: false });
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(seeks(host), [10, 37]);
  assert.equal(host.source.position, 37);
  assert.equal(bar.props["elapsed-label"], "00:37");
  assert.equal(bar.props["total-label"], "01:40");
  assert.equal(lastOsd(host), "跳转: 00:37 / 01:40");
  assert.deepEqual(host.mounted("bottom").locks, { focused: false, dragging: false, popupOpen: false });
  await stop();
});

test("cancel discards the unsent tail without undoing the accepted seek", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
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
    const bar = host.mounted("bottom");
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
    assert.deepEqual(host.mounted("bottom").locks, { focused: false, dragging: false, popupOpen: false });
    await stop();
  }
});

test("a late failure from a cleared gesture does not report into the new one", async () => {
  const { host, stop } = await start(withMedia());
  const failures: (() => void)[] = [];
  host.handle("source.submit", () => new Promise((_, reject) => failures.push(() => reject(new HostError("backend", "old")))));
  const bar = host.mounted("bottom");
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

test("0.2.3 keyframes on (default): a click on the bar is one keyframe seek; the release on the same spot is not sent again", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.25);
  await host.settle(80); // the press seek lands while the pointer is still down
  assert.deepEqual(wireSeeks(host), [{ seconds: 25, keyframes: true }]);
  assert.equal(bar.props["elapsed-label"], "00:25", "the draft shows while pressed");
  bar.emit("seek-changed", 0.25); // the release sample: the pointer did not move
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(wireSeeks(host), [{ seconds: 25, keyframes: true }], "press and release: one seek, no exact seek after it");
  assert.deepEqual(host.seeks, [{ seconds: 25, keyframes: true }]);
  assert.equal(host.source.position, 25);
  assert.equal(lastOsd(host), "跳转: 00:25 / 01:40");
  assert.deepEqual(host.mounted("bottom").locks, { focused: false, dragging: false, popupOpen: false });
  // The draft is gone with the release: the player's position shows (a keyframe seek lands before the target).
  host.setSource({ position: 20 });
  await host.settle();
  assert.equal(bar.props["elapsed-label"], "00:20");
  await stop();
});

test("0.2.3 a drag keeps the mode decided at the press: moves and the release seek by keyframes, none exact", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.1);
  await host.settle(80);
  // The switch flips during the gesture (the Playback panel): the gesture keeps its mode.
  host.mounted("adjust").emit("setKeyframesOnly", false);
  await eventually(() => host.sent("player.setStepSeek").at(-1), { seconds: 5, keyframes: false });
  bar.emit("seek-changed", 0.3);
  await host.settle(80);
  bar.emit("seek-changed", 0.42); // the release sample, elsewhere
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(wireSeeks(host), [
    { seconds: 10, keyframes: true },
    { seconds: 30, keyframes: true },
    { seconds: 42, keyframes: true },
  ]);
  assert.equal(host.source.position, 42);
  assert.equal(bar.props["elapsed-label"], "00:42");
  // The next press reads the switch again: exact now, and still one seek for a click.
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.5);
  await host.settle(80);
  bar.emit("seek-changed", 0.5);
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(wireSeeks(host).slice(3), [{ seconds: 50 }]);
  assert.deepEqual(host.seeks.at(-1), { seconds: 50, keyframes: false });
  await stop();
});

test("0.2.3 a drag that returns to the press target: the release equals the last sent target and is dropped", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.25);
  await host.settle(80);
  bar.emit("seek-changed", 0.4);
  await host.settle(80);
  bar.emit("seek-changed", 0.25); // back to where the press was: a new target after 40, so it is sent
  await host.settle(80);
  assert.deepEqual(wireSeeks(host), [
    { seconds: 25, keyframes: true },
    { seconds: 40, keyframes: true },
    { seconds: 25, keyframes: true },
  ]);
  bar.emit("seek-changed", 0.25); // the release sample equals the last *sent* target
  bar.emit("dragging", false);
  await host.settle(80);
  assert.equal(wireSeeks(host).length, 3, "the release is not sent again");
  assert.equal(host.source.position, 25);
  assert.equal(lastOsd(host), "跳转: 00:25 / 01:40");
  host.setSource({ position: 20 });
  await host.settle();
  assert.equal(bar.props["elapsed-label"], "00:20", "no draft left behind");
  await stop();
});

test("0.2.3 keyframes off: the bar seeks exactly as before, without the flag on the wire", async () => {
  const { host, stop } = await startWith({ seekKeyframes: false });
  const bar = host.mounted("bottom");
  bar.emit("dragging", true);
  bar.emit("seek-changed", 0.1);
  await host.settle(80);
  bar.emit("seek-changed", 0.37);
  bar.emit("dragging", false);
  await host.settle(80);
  assert.deepEqual(wireSeeks(host), [{ seconds: 10 }, { seconds: 37 }]);
  assert.deepEqual(
    host.seeks.map((s) => s.keyframes),
    [false, false],
  );
  assert.equal(host.source.position, 37);
  await stop();
});

test("0.2.3 Jump to Time and Jump to Start stay exact with the switch on", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 30 });
  await host.settle();
  await host.press("G");
  host.mounted("adjust").emit("applyJump", "1:05");
  await eventually(() => wireSeeks(host), [{ seconds: 65 }]);
  await host.command("jump-start").run();
  await host.settle(80);
  assert.deepEqual(wireSeeks(host), [{ seconds: 65 }, { seconds: 0 }]);
  assert.deepEqual(
    host.seeks.map((s) => s.keyframes),
    [false, false],
  );
  await stop();
});

test("exact mode: relative seeks accumulate from the pending target, clamp to the source and show the target", async (t) => {
  // "Jump to keyframes only" off: tiers 2–4 are the plugin's own exact seeks, as before 0.2.1.
  const data = mkdtempSync(join(tmpdir(), "playback-exact-"));
  writeFileSync(join(data, "settings.json"), JSON.stringify({ seekKeyframes: false }));
  const { host, stop } = await startPlugin(activate, { directory, host: withMedia(), locale: "zh-Hans", dataDirectory: data });
  t.after(() => rmSync(data, { recursive: true, force: true }));
  assert.deepEqual(host.sent("player.setStepSeek"), [{ seconds: 5, keyframes: false }]);
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
  // ← / → are the core's first tier (0.2.14: the bar's buttons no longer seek).
  await host.press("ArrowLeft");
  await host.settle(80);
  assert.equal(host.source.position, 95);
  assert.equal(lastOsd(host), "后退 5秒: 01:35 / 01:40");
  // The bar no longer sends seeks; one that arrives anyway is ignored.
  host.mounted("bottom").emit("action", "seek-backward");
  await host.settle(80);
  assert.equal(host.source.position, 95);
  await host.press("ArrowRight");
  await host.settle(80);
  assert.deepEqual(host.coreRuns, ["seek-backward", "seek-forward"]);
  // Tiers 2–4 went through the plugin's own seeks, never the core's.
  assert.deepEqual(
    host.coreSeeks.map((s) => s.seconds),
    [5, 5],
  );
  await stop();
});

test("previous/next are the core's: PageUp/PageDown and the bar's Previous/Next buttons", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  // no next item: 下一个 is disabled, so PageDown shows why and runs nothing (SDK 1.5.12, as the player)
  assert.equal(await host.press("PageDown"), "reason");
  assert.deepEqual(host.coreRuns, []);
  assert.deepEqual(host.osd, ["没有下一个"]);
  host.sequence = { canNext: true, canPrevious: true };
  host.setSource({ sequence: { next: true, previous: true } } as never);
  await host.settle();
  assert.equal(bar.props["can-next"], true);
  assert.equal(bar.props["can-previous"], true);
  await host.press("PageDown");
  await host.press("Mouse:X1"); // next from player 0.6.0 (SDK 1.5.47)
  bar.emit("action", "previous"); // a click on 上一个
  bar.emit("action", "next"); // a click on 下一个
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next", "next", "previous", "next"]);
  // No sequence any more: the core refuses, the bar says nothing.
  host.sequence = null;
  const shown = host.osd.length;
  bar.emit("action", "next");
  await host.settle();
  assert.equal(host.coreRuns.length, 4);
  assert.equal(host.osd.length, shown);
  await stop();
});

test("0.2.14 the Previous/Next buttons follow the playlist: off at its ends, each click one step", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  // The sequence provider (the playlist) says what is there; the bar shows it as the menu does.
  const at = async (previous: boolean, next: boolean) => {
    host.sequence = previous || next ? { canNext: next, canPrevious: previous } : null;
    host.setSource({ sequence: { next, previous } } as never);
    await host.settle();
    assert.equal(bar.props["can-previous"], previous);
    assert.equal(bar.props["can-next"], next);
    // The same as the menu items Previous / Next (the core actions).
    assert.equal(host.coreCommands.get("host/previous")?.state.enabled, previous);
    assert.equal(host.coreCommands.get("host/next")?.state.enabled, next);
  };
  // First item of three: Previous off, Next on.
  await at(false, true);
  bar.emit("action", "next");
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next"]);
  // Middle item: both on, each click is one step.
  await at(true, true);
  bar.emit("action", "previous");
  bar.emit("action", "next");
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next", "previous", "next"]);
  // Last item: Next off; a click that slips through runs nothing and shows nothing.
  await at(true, false);
  const shown = host.osd.length;
  bar.emit("action", "next");
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next", "previous", "next"]);
  assert.equal(host.osd.length, shown);
  bar.emit("action", "previous");
  await host.settle();
  assert.deepEqual(host.coreRuns, ["next", "previous", "next", "previous"]);
  // Repeat all wraps: both stay on at the ends (the playlist reports it).
  await at(true, true);
  // A single item, no repeat: both off.
  await at(false, false);
  bar.emit("action", "previous");
  await host.settle();
  assert.equal(host.coreRuns.length, 4);
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
  const bar = host.mounted("bottom");
  bar.emit("volume-changed", 80);
  bar.emit("volume-changed", 60);
  bar.emit("volume-released", 35);
  bar.emit("action", "mute"); // dropped: the volume has not landed yet
  // `can-mute` is written after `volume` (one property at a time): wait for both.
  await until(() => bar.props.volume === 35 && bar.props["can-mute"] === false);
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
  const bar = host.mounted("bottom");
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
  assert.equal(bar.props["volume-percent"], 150);
  assert.equal(host.translate("Volume: {}%", [150]), "音量: 150%");
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

test("Jump to Time (G) focuses the Playback panel's time field; a typed time applies to the current source", async () => {
  const { host, stop } = await start(withMedia());
  host.setSource({ position: 30 });
  await host.settle();
  const panel = host.mounted("adjust");
  assert.equal(panel.file, "ui/panel.slint");
  await eventually(() => host.layout.adjust?.preferred, false);
  await host.press("G");
  await eventually(() => panel.props["edit-jump"], 1);
  await eventually(() => [host.layout.adjust?.preferred, host.layout.adjust?.owner], [true, 1]);
  assert.deepEqual(
    ["Jump to Time", "Go to", "Go"].map((text) => host.translate(text)),
    ["跳转到", "跳转到", "定位"],
  );
  // A time past the end is refused: the field stays, marked, and the reason shows on the video.
  panel.emit("applyJump", "2:00");
  await eventually(() => panel.props["jump-invalid"], true);
  await eventually(() => lastOsd(host), "请输入来源时长范围内的HH:MM:SS.mmm");
  panel.emit("applyJump", "abc");
  await eventually(() => host.osd.filter((text) => text === "请输入来源时长范围内的HH:MM:SS.mmm").length, 2);
  assert.deepEqual(seeks(host), []);
  // A good one seeks exactly, says where, and closes the field (`done`).
  panel.emit("applyJump", "1:05");
  await eventually(() => host.source.position, 65);
  await eventually(() => lastOsd(host), "跳转: 01:05 / 01:40");
  await eventually(() => [panel.props["jump-invalid"], panel.props.done], [false, 1]);
  // A pause by someone else changes nothing: the jump applies to the current state.
  host.setSource({ paused: true, stamp: { ...host.source.stamp, intentVersion: host.source.stamp.intentVersion + 1 } });
  await host.settle();
  panel.emit("applyJump", "0:10");
  await eventually(() => host.source.position, 10);
  // G again keeps the column open and focuses the field again.
  await host.command("jump").run();
  await eventually(() => [panel.props["edit-jump"], host.layout.adjust?.preferred], [2, true]);
  // Another source drops a refused time's mark.
  panel.emit("applyJump", "9:00");
  await eventually(() => panel.props["jump-invalid"], true);
  host.openElsewhere("/next.mp4");
  host.setSource({ duration: 100, position: 0 });
  await eventually(() => panel.props["jump-invalid"], false);
  // Nothing to seek: the field is off and a stray Enter does nothing.
  await host.press("F4");
  await eventually(() => panel.props["can-jump"], false);
  const before = seeks(host).length;
  panel.emit("applyJump", "0:20");
  await host.settle(80);
  assert.equal(seeks(host).length, before);
  await stop();
});

test("the Playback panel's speed: the wheel is the player's X / C, a tick and a typed speed are conditional on the current state", async () => {
  const { host, stop } = await start(withMedia());
  const panel = host.mounted("adjust");
  await eventually(
    () => [panel.props["rate-text"], panel.props.tick, panel.props["rate-position"], panel.props["can-rate"]],
    ["1", 2, 0.5, true],
  );
  panel.emit("rateStep", 1);
  panel.emit("rateStep", -1);
  await eventually(() => host.coreRuns, ["rate-up", "rate-down"]);
  panel.emit("setTick", 3);
  await eventually(() => host.source.rate, 1.25);
  await eventually(() => [panel.props["rate-text"], panel.props.tick, panel.props["rate-position"]], ["1.25", 3, 0.625]);
  await eventually(() => lastOsd(host), "播放速度: 1.25x");
  panel.emit("setTick", 9); // not a tick: nothing
  panel.emit("setTick", 6);
  await eventually(() => [host.source.rate, panel.props.tick, panel.props["rate-position"]], [4, 6, 1]);
  // A double click on the ruler: 1×.
  panel.emit("speedNormal");
  await eventually(() => [host.source.rate, panel.props.tick], [1, 2]);
  // Custom… (palette only): the panel in front, the speed open for typing.
  await host.command("rate-custom").run();
  await eventually(() => [panel.props["edit-rate"], host.layout.adjust?.preferred], [1, true]);
  panel.emit("applyRate", "9");
  await eventually(() => panel.props["rate-invalid"], true);
  await eventually(() => lastOsd(host), "速度范围为0.25–4倍");
  panel.emit("applyRate", "");
  await eventually(() => host.osd.filter((text) => text === "速度范围为0.25–4倍").length, 2);
  panel.emit("applyRate", "1.3×");
  await eventually(() => host.source.rate, 1.3);
  await eventually(() => [panel.props["rate-text"], panel.props.tick, panel.props["rate-invalid"], panel.props.done], ["1.3", -1, false, 1]);
  assert.ok((host.sent("source.submit").at(-1)!.intent as Record<string, unknown>).expected);
  for (const id of ["rate-05", "rate-15", "rate-20"]) assert.equal(host.command(id).state.checked, false, id);
  await stop();
});

/** The speeds the plugin asked the player for, in order. */
const rates = (host: TestHost) =>
  host
    .sent("source.submit")
    .map((p) => p.intent as { kind: string; control?: { kind: string; rate?: number } })
    .filter((i) => i.kind === "control" && i.control?.kind === "rate")
    .map((i) => i.control!.rate);

test("the Playback panel's speed ruler: a drag snaps, steps by 0.05, shows at once and applies latest-wins", async () => {
  const { host, stop } = await start(withMedia(), "en");
  const panel = host.mounted("adjust");
  await eventually(() => panel.props["rate-text"], "1");
  // A burst of pointer moves: the first goes out, the rest collapse into the last one.
  const snap = 0.02;
  panel.emit("speedDrag", 0.57, snap); // 1.14 → 1.15
  panel.emit("speedDrag", 0.6, snap); // 1.2
  panel.emit("speedDrag", 0.74, snap); // near 1.5: snaps
  // Shown before the player answers, and the panel stays usable while it applies.
  await eventually(() => [panel.props["rate-text"], panel.props.tick, panel.props["can-rate"]], ["1.5", 4, true]);
  await eventually(() => host.source.rate, 1.5);
  await host.settle();
  const sent = rates(host);
  assert.equal(sent.at(-1), 1.5);
  assert.ok(sent.length <= 2, `latest wins: ${sent}`);
  // Between ticks: 0.05 steps; the release keeps what the player has.
  panel.emit("speedDrag", 0.94, snap);
  await eventually(() => host.source.rate, 3.05);
  panel.emit("speedRelease");
  await host.settle();
  assert.deepEqual([panel.props["rate-text"], panel.props.tick], ["3.05", -1]);
  assert.ok(Math.abs((panel.props["rate-position"] as number) - (0.875 + 0.125 * (1.05 / 2))) < 1e-9);
  // Past the ends: the player's limits.
  panel.emit("speedDrag", -0.3, snap);
  panel.emit("speedRelease");
  await eventually(() => [host.source.rate, panel.props["rate-text"]], [0.25, "0.25"]);
  // A speed someone else sets (X / C) moves the ruler.
  await host.press("C");
  await eventually(() => [panel.props["rate-text"], panel.props.tick], ["0.35", -1]);
  // A source that cannot change speed: the ruler is off and a drag does nothing.
  const before = rates(host).length;
  await host.press("F4");
  await eventually(() => panel.props["can-rate"], false);
  panel.emit("speedDrag", 0.9, snap);
  panel.emit("setTick", 0);
  await host.settle(80);
  assert.equal(rates(host).length, before);
  await stop();
});

test("the Playback panel's speed ruler: a drag whose release never arrives does not stick", async () => {
  const { host, stop } = await start(withMedia(), "en");
  const panel = host.mounted("adjust");
  await eventually(() => panel.props["rate-text"], "1");
  // Mid-drag the source stops (the playlist moves on, Alt+Tab): no `up`, no `speed-release`.
  panel.emit("speedDrag", 0.94, 0.02);
  await eventually(() => host.source.rate, 3.05);
  await host.press("F4");
  await eventually(() => panel.props["can-rate"], false);
  // The shown speed is the player's, not the drag's.
  await eventually(() => panel.props["rate-text"], rateText(host.source.rate));
  // The next source: the ruler follows the player again and takes a click.
  host.openElsewhere("/next.mp4");
  host.setSource({ duration: 100, position: 0, rate: 1 });
  await eventually(() => [panel.props["can-rate"], panel.props["rate-text"], panel.props.tick], [true, "1", 2]);
  panel.emit("setTick", 1);
  await eventually(() => [host.source.rate, panel.props["rate-text"]], [0.5, "0.5"]);
  // The pointer is cancelled (Slint's `cancel` ends a drag like a release): a late release is harmless.
  panel.emit("speedDrag", 0.6, 0.02);
  await eventually(() => host.source.rate, 1.2);
  panel.emit("speedRelease");
  await host.press("C");
  await eventually(() => [host.source.rate, panel.props["rate-text"]], [1.3, "1.3"]);
  await stop();
});

/** Hold the next speed request until `release` is called; it then lands like the test host's own. */
function holdRate(host: TestHost) {
  let release = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  host.handle("source.submit", async (p) => {
    const intent = p.intent as { kind: string; control?: { kind: string; rate?: number } };
    assert.equal(intent.control?.kind, "rate");
    await gate;
    host.reset("source.submit");
    const stamp = { ...host.source.stamp, intentVersion: host.source.stamp.intentVersion + 1 };
    host.setSource({ rate: intent.control!.rate!, stamp });
    return { status: "completed", ticket: { stamp, cycle: host.source.cycle, owner: 1 } };
  });
  return release;
}

test("the Playback panel's speed: a typed speed while a drag's request is still applying is not lost", async () => {
  const { host, stop } = await start(withMedia(), "en");
  const panel = host.mounted("adjust");
  await eventually(() => panel.props["rate-text"], "1");
  const release = holdRate(host);
  panel.emit("speedDrag", 0.6, 0.02); // 1.2, held
  panel.emit("speedRelease");
  await until(() => rates(host).length === 1);
  // The value is clicked and 1.3 typed within the request's time: it closes and is applied after.
  panel.emit("applyRate", "1.3");
  await eventually(() => [panel.props.done, panel.props["rate-text"], panel.props["rate-invalid"]], [1, "1.3", false]);
  release();
  await eventually(() => host.source.rate, 1.3);
  await host.settle();
  assert.deepEqual(rates(host), [1.2, 1.3]);
  assert.equal(panel.props["rate-text"], "1.3");
  // A refused typed speed is refused at once, busy or not.
  panel.emit("applyRate", "0.1");
  await eventually(() => panel.props["rate-invalid"], true);
  await stop();
});

test("the Playback panel's speed: a tick clicked while a palette speed is applying waits for it", async () => {
  const { host, stop } = await start(withMedia(), "en");
  const panel = host.mounted("adjust");
  await eventually(() => panel.props["rate-text"], "1");
  const release = holdRate(host);
  void host.command("rate-15").run(); // 1.5, held
  await until(() => rates(host).length === 1);
  panel.emit("setTick", 5); // 2: shown now, sent once the first request lands
  await eventually(() => [panel.props["rate-text"], panel.props.tick], ["2", 5]);
  // (`settle` waits for the held request: plain time instead.)
  await new Promise((resolve) => setTimeout(resolve, 60));
  assert.deepEqual(rates(host), [1.5]);
  release();
  await eventually(() => host.source.rate, 2);
  await host.settle();
  assert.deepEqual(rates(host), [1.5, 2]);
  assert.deepEqual([panel.props["rate-text"], panel.props.tick], ["2", 5]);
  await stop();
});

test("the Playback panel's steps: frames are the player's, keyframes and Jump to Start this plugin's", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, video: { codec: "h264", width: 1920, height: 1080, fps: 24 } } as never;
  withMedia(host);
  const { stop } = await start(host);
  host.setSource({ position: 12 });
  const panel = host.mounted("adjust");
  await eventually(() => [panel.props["can-frame"], panel.props["can-keyframe"], panel.props["can-start"]], [true, true, true]);
  panel.emit("frameStep", 1);
  panel.emit("frameStep", -1);
  await eventually(() => host.coreRuns, ["frame-forward", "frame-back"]);
  panel.emit("keyframeStep", 1);
  await eventually(() => host.sent("player.step").map((p) => [p.unit, p.direction]), [["keyframe", "forward"]]);
  // The test host does not move for a keyframe step: the plugin says so.
  await eventually(() => lastOsd(host), "已是最后一个关键帧");
  panel.emit("jumpStart");
  await eventually(() => host.source.position, 0);
  await eventually(() => lastOsd(host), "定位: 开始处");
  // An audio-only source: the keyframe buttons say why they are off.
  host.stats = { ...host.stats, video: null } as never;
  host.openElsewhere("/song.mp3");
  host.setSource({ duration: 100, position: 0 });
  await eventually(() => [panel.props["can-keyframe"], panel.props["keyframe-reason"]], [false, "video"]);
  await host.press("F4");
  await eventually(() => [panel.props["can-frame"], panel.props["can-start"], panel.props["keyframe-reason"]], [false, false, "source"]);
  await stop();
});

test("Playback… (Alt+4) toggles the adjust column at the Playback panel; the keys show beside the controls", async () => {
  const { host, stop } = await start(withMedia(), "en");
  const panel = host.mounted("adjust");
  assert.equal(panel.slot, "adjust");
  assert.equal(host.layout.adjust?.panels?.[0]?.title, "Playback");
  assert.deepEqual(
    host.sent("ui.mount").map((m) => [m.slot, m.component, m.global]),
    [
      ["bottom", "PlaybackBar", "PlaybackBridge"],
      ["adjust", "PlaybackPanel", "PlaybackPanelBridge"],
    ],
  );
  const command = host.command("panel");
  assert.deepEqual(
    [command.display.path, command.display.title, command.declaration.bindings, command.display.category],
    [[], "Playback…", ["Alt+4"], "Playback"],
  );
  await host.press("Alt+4");
  await eventually(() => [host.layout.adjust?.preferred, command.state.checked], [true, true]);
  await host.press("Alt+4");
  await eventually(() => [host.layout.adjust?.preferred, command.state.checked], [false, false]);
  // Another plugin's tab in front: Alt+4 brings Playback forward instead of closing the column.
  host.mountOther("adjust", "other.wheels", { title: "Wheels" });
  host.selectTab("other.wheels");
  host.layout.adjust!.preferred = true;
  host.emit("window.state", {});
  await host.press("Alt+4");
  await eventually(() => [host.layout.adjust?.preferred, host.layout.adjust?.owner], [true, 1]);
  // The key caps: this plugin's commands and the player's own actions, as the binding table labels
  // them (the test host writes the arrow as "ArrowLeft"; the player's menus write "←").
  await eventually(() => {
    const keys = panel.props.keys as Record<string, string>;
    return [keys.slower, keys.faster, keys.normal, keys.frameback, keys.frameforward, keys.back1, keys.jump, keys.start, keys.back2 !== ""];
  }, ["X", "C", "Z", "D", "F", "ArrowLeft", "G", "Backspace", true]);
  // A key the user changes shows at once; a cleared one shows no cap.
  host.rebind("jump", ["Shift+G"]);
  host.rebind("jump-start", []);
  await eventually(() => {
    const keys = panel.props.keys as Record<string, string>;
    return [keys.jump !== "G" && keys.jump !== "", keys.start];
  }, [true, ""]);
  // A text field with the keyboard keeps the fullscreen column out.
  const focused = () => (panel.locks as { focused?: boolean } | null)?.focused;
  panel.emit("focusChanged", "jump", true);
  await eventually(focused, true);
  panel.emit("focusChanged", "jump", false);
  await eventually(focused, false);
  // Focus moving from field to field: the next field's focus may arrive before the last one's
  // blur. The panel keeps the keyboard until no field has it.
  panel.emit("focusChanged", "rate", true);
  panel.emit("focusChanged", "step2", true);
  panel.emit("focusChanged", "rate", false);
  await host.settle();
  assert.equal(focused(), true);
  panel.emit("focusChanged", "step2", false);
  panel.emit("focusChanged", "step2", false); // a repeated blur changes nothing
  await eventually(focused, false);
  // A blur only drops its own field's mark.
  panel.emit("applyJump", "9:99");
  panel.emit("applyRate", "9");
  await eventually(() => [panel.props["jump-invalid"], panel.props["rate-invalid"]], [true, true]);
  panel.emit("focusChanged", "rate", false);
  await eventually(() => [panel.props["jump-invalid"], panel.props["rate-invalid"]], [true, false]);
  await stop();
});

test("the time label toggles between elapsed and remaining", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
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

test("0.2.5 the bar mounts beside the player's button strip and leaves the sidebar toggle to it", async () => {
  const { host, stop } = await start(withMedia());
  try {
    const bar = host.mounted("bottom");
    assert.equal(bar.slot, "bottom");
    // SDK 1.6.8: the player places the strip; the mounted file reports what
    // its row needs (`content-min-width`) and places no deprecated BarButtons.
    const slint = await readFile(new URL(`../${bar.file}`, import.meta.url), "utf8");
    assert.ok(slint.includes('import { BarButtonsHost } from "@vestra/bar-buttons.slint";'));
    assert.match(slint, /out property <length> content-min-width: /);
    assert.doesNotMatch(slint, /\bBarButtons\s*\{/);
    // Playback contributes no button of its own: the strip holds other
    // plugins' buttons and the player's Toggle Sidebar (host/sidebar).
    assert.deepEqual(host.barButtons, []);
    // No sidebar state of its own any more: the player's toggle shows it.
    for (const removed of ["has-sidebar", "sidebar-on", "sidebar-hidden", "key-sidebar", "tip-sidebar"])
      assert.equal(removed in bar.props, false, removed);
    const mount = host.sent("ui.mount")[0]!;
    if (Array.isArray(mount.callbacks)) assert.equal(mount.callbacks.includes("toggle-sidebar"), false);
    // The sidebar is left alone: nothing the bar does sets it.
    host.layout.sidebar = { owner: 2, preferred: true, effective: true, reason: null, detached: false };
    host.emit("window.state", {});
    await host.settle();
    assert.deepEqual(host.sent("layout.set"), []);
  } finally {
    await stop();
  }
});

test("dragging the volume keeps the bar shown until release", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted("bottom");
  bar.emit("volume-changed", 50);
  await host.settle();
  assert.deepEqual(bar.locks, { focused: false, dragging: true, popupOpen: false });
  bar.emit("volume-released", 40);
  await host.settle();
  assert.deepEqual(bar.locks, { focused: false, dragging: false, popupOpen: false });
  assert.equal(host.source.output.volume, 40);
  await stop();
});
