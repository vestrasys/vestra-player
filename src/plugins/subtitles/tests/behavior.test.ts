// Orchestration against the SDK test host only. No native parsing or rendering assertions.
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { shownStatus } from "./status.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const embedded = { kind: "embedded" as const, id: 1 };
/**
 * Runs `action` and waits for the OSD line it ends with. The plugin reads and
 * parses subtitle files itself (not host calls), so `settle()` alone can
 * return before the result under load; a new OSD entry means the action ended.
 */
async function withOsd(host: TestHost, action: () => Promise<unknown>) {
  const before = host.osd.length;
  await action();
  for (let i = 0; i < 250 && host.osd.length === before; i++) await host.settle(20);
  await host.settle();
}

/** A ready source (generation 1) with one selected, stylable embedded track. */
function sourceHost() {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.subtitles.tracks.push({ target: embedded, title: "English", language: "en", styleSupported: true, styleReason: null });
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: embedded };
  return host;
}
async function open(host = sourceHost()) {
  const started = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.command("tune").run();
  return { ...started, window: host.window("subtitle-tune") };
}
async function openSettings(host: TestHost) {
  await host.command("settings").run();
  return host.window("subtitle-settings");
}
async function openSync(host: TestHost) {
  await host.command("sync-input").run();
  return host.window("subtitle-sync");
}
/** Simulate another source being opened: the subtitle state moves to generation 2. */
function switchSource(host: TestHost) {
  host.openElsewhere("/other.mp4");
  host.subtitles.selection.stamp = { generation: 2, revision: 1 };
}
async function srtFile(text: string | Buffer) {
  const folder = await mkdtemp(join(tmpdir(), "vestra-subtitles-"));
  const path = join(folder, "actual.srt");
  await writeFile(path, text);
  return { path, cleanup: () => rm(folder, { recursive: true, force: true }) };
}

test("menu per the allocation table; windows bind exactly their Slint callbacks", async () => {
  const { host, window, stop } = await open();
  const declared = [...host.commands.values()]
    .map(({ declaration: d, display }) => [display.path.join(" > "), d.order, d.group, d.bindings.join(",")] as const)
    .sort((a, b) => a[1] - b[1]);
  assert.deepEqual(declared, [
    ["打开 > 打开字幕…", 113, "open", "Alt+O"],
    ["打开 > 添加次字幕…", 114, "open", ""],
    ["播放 > 跳转 > 上一句对白", 217, "jump-subtitle", "Home"],
    ["播放 > 跳转 > 下一句对白", 217, "jump-subtitle", "End"],
    ["播放 > 跳转 > 当前字幕起点", 217, "jump-subtitle", "Ctrl+Home"],
    ["字幕 > 显示字幕", 301, "domain", "Alt+H"],
    ["字幕 > 选择字幕轨 > 不显示", 302, "subtitle-track", ""],
    ["字幕 > 选择字幕轨 > English · en", 303, "subtitle-track", ""],
    ["字幕 > 选择字幕轨 > 依次切换", 311, "subtitle-cycle", "Alt+L"],
    ["字幕 > 次字幕 > 不显示", 312, "subtitle-second", ""],
    ["字幕 > 次字幕 > 位置 > 顶部", 321, "subtitle-second-pos", ""],
    ["字幕 > 次字幕 > 位置 > 主字幕上方", 321, "subtitle-second-pos", ""],
    ["字幕 > 字幕样式 > 粗体", 324, "style-font", "Alt+B"],
    ["字幕 > 字幕样式 > 字号 +", 325, "style-font", "Alt+PageUp"],
    ["字幕 > 字幕样式 > 字号 −", 326, "style-font", "Alt+PageDown"],
    ["字幕 > 字幕样式 > 上移", 330, "style-move", ""],
    ["复位 > 字幕位置", 330, "reset-subtitles", "Alt+Home"],
    ["字幕 > 字幕样式 > 下移", 331, "style-move", ""],
    ["复位 > 字幕同步", 331, "reset-subtitles", "/"],
    ["字幕 > 字幕样式 > 阴影", 335, "style-box", ""],
    ["字幕 > 字幕样式 > 字幕边框不透明", 336, "style-box", ""],
    ["字幕 > 开启 ASS/SSA 原样式", 337, "subtitle-render", ""],
    ["字幕 > 同步 > 超前 0.5 秒", 339, "subtitle-sync", "."],
    ["字幕 > 同步 > 滞后 0.5 秒", 340, "subtitle-sync", ","],
    ["字幕 > 同步 > 手动调整同步…", 342, "subtitle-sync-input", ""],
    ["字幕 > 重载字幕", 370, "subtitle-tools", "Ctrl+Alt+Y"],
    ["字幕 > 刷新字幕轨", 371, "subtitle-tools", ""],
    ["字幕 > 字幕微调…", 372, "subtitle-tools", ""],
    ["字幕 > 字幕设置…", 373, "subtitle-tools", ""],
  ]);
  assert.deepEqual(host.conflicts, []);
  const settings = await openSettings(host);
  const sync = await openSync(host);
  assert.deepEqual(host.sent("ui.open").map((o) => o.file), ["ui/tune.slint", "ui/settings.slint", "ui/sync.slint"]);
  for (const [name, view] of [["tune", window], ["settings", settings], ["sync", sync]] as const) {
    const slint = await readFile(new URL(`../ui/${name}.slint`, import.meta.url), "utf8");
    const callbacks = [...slint.matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
    assert.deepEqual([...view.callbacks].sort(), callbacks.sort(), name);
  }
  assert.deepEqual(window.props["track-labels"], ["关闭字幕", "English · en"]);
  assert.equal(window.props["selected-track"], 1);
  assert.equal(window.props["can-style"], true);
  assert.equal(window.props["sync-seconds"], "0");
  assert.equal(host.command("show").state.checked, true);
  assert.equal(host.command("track-1").state.checked, true);
  assert.equal(host.commands.has("track-2"), false, "one item per real track");
  await stop();
});

test("window text is short Chinese with no debug strings", async () => {
  const { host, window, stop } = await open();
  const settings = await openSettings(host);
  const sync = await openSync(host);
  for (const view of [window, settings, sync])
    for (const [name, value] of Object.entries(view.props))
      if (typeof value === "string" && name !== "track-labels")
        assert.doesNotMatch(value, /generation|revision|applied|waiting|uncertain|ready|媒体版本|\n/, name);
  for (const file of ["tune", "settings", "sync"]) {
    const slint = await readFile(new URL(`../ui/${file}.slint`, import.meta.url), "utf8");
    assert.doesNotMatch(slint, /#[0-9a-f]{3,8}\b/i, `${file}: colours come from Theme`);
    assert.doesNotMatch(slint, /font-size:\s*\d/, `${file}: font sizes come from Theme`);
  }
  await stop();
});

test("style values stay within the host's accepted ranges", async () => {
  const { host, window, stop } = await open();
  const settings = await openSettings(host);
  for (let i = 0; i < 30; i++) window.emit("size-step", 1);
  settings.emit("change-outline", 8);
  settings.emit("change-margin", 0);
  await host.settle(170);
  const style = host.sent("subtitles.style").at(-1)!.style as Record<string, number>;
  assert.deepEqual([style.fontSize, style.outline, style.marginPercent], [72, 4, 2]);
  await stop();
});

test("a slider burst is merged into one style write with the latest values", async () => {
  const { host, window, stop } = await open();
  window.emit("sync-step", 1);
  window.emit("sync-step", 1);
  window.emit("size-step", 1);
  window.emit("position-step", 1);
  await host.settle();
  assert.equal(window.props["sync-seconds"], "+1"); // shown before the host confirms
  assert.equal(window.props["subtitle-size"], 38);
  await host.settle(170);
  const styles = host.sent("subtitles.style");
  assert.equal(styles.length, 1);
  assert.deepEqual(styles[0]!.style, {
    fontId: "sans", fontSize: 38, color: "white", outline: 2, marginPercent: 6, offsetMs: 1000,
    bold: false, shadow: 0, box: null, assOverride: "keep", secondaryPosition: "top",
  });
  assert.equal(shownStatus(window.props), "字幕样式已更新");
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 1000);
  await stop();
});

test("a style edit only applies to the selection it began on", async () => {
  const { host, window, stop } = await open();
  window.emit("size-step", 1);
  host.subtitles.selection.stamp.revision++; // another plugin changes the selection meanwhile
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective, null);
  assert.equal(shownStatus(window.props), "字幕轨已变化，请重新调整。");
  await stop();
});

test("file picker cancellation never begins an upload", async () => {
  const { host, window, stop } = await open();
  host.dialogAnswers.push(null);
  window.emit("load-file");
  await host.settle();
  assert.equal(host.sent("subtitles.upload.begin").length, 0);
  assert.match(shownStatus(window.props), /未选择字幕文件/);
  await stop();
});

test("a source change while the picker is open fences the file before it is read", async () => {
  const { host, window, stop } = await open();
  let answer!: (value: unknown) => void;
  host.handle("dialogs.open", () => new Promise((resolve) => (answer = resolve)));
  window.emit("load-file");
  await pause(); // the picker request stays in flight, so settle() would wait for it
  assert.equal(host.command("load").state.busy, true);
  assert.equal(host.command("load").state.enabled, false);
  switchSource(host);
  await pause();
  answer({ cancelled: false, paths: ["/must-not-be-read.srt"] });
  await host.settle();
  assert.equal(host.sent("subtitles.upload.begin").length, 0);
  assert.match(shownStatus(window.props), /媒体已切换/);
  assert.equal(host.command("load").state.busy, false);
  await stop();
});

test("a track list from the old source cannot select the matching embedded id in the new one", async () => {
  const { host, window, stop } = await open();
  host.subtitles.source = { generation: 2, phase: "ready" }; // changed without an event yet
  host.subtitles.selection.stamp = { generation: 2, revision: 1 };
  window.emit("select-track", 1);
  await host.settle();
  assert.equal(host.sent("subtitles.select").length, 0);
  await stop();
});

test("source replacement and disposal cancel pending drag writes", async () => {
  const { host, window, stop } = await open();
  window.emit("sync-step", 1);
  switchSource(host);
  await host.settle(140);
  assert.equal(host.sent("subtitles.style").length, 0);
  assert.match(shownStatus(window.props), /媒体已切换/);
  window.emit("size-step", -1);
  await host.settle();
  await stop();
  await pause(140);
  assert.equal(host.sent("subtitles.style").length, 0);
});

test("real file bytes are uploaded; a lost commit reply is resolved by the SDK and never discarded", async () => {
  const bytes = Buffer.from("1\n00:00:00,000 --> 00:00:01,000\nHello 你好\n");
  const file = await srtFile(bytes);
  const { host, window, stop } = await open();
  host.dialogAnswers.push([file.path]);
  host.handle("subtitles.commit", async (params) => {
    host.reset("subtitles.commit");
    const state = await host.request("subtitles.commit", params);
    host.handle("subtitles.operation", () => ({ status: "completed", result: state }));
    throw new Error("reply lost");
  });
  try {
    await withOsd(host, () => host.command("load").run());
    const appended = host.sent("subtitles.upload.append").map((c) => Buffer.from(String(c.base64), "base64"));
    assert.deepEqual(Buffer.concat(appended), bytes);
    assert.equal(host.sent("subtitles.upload.begin")[0]!.generation, 1);
    const commits = host.sent("subtitles.commit");
    assert.equal(commits.length, 2); // the lost one, then the real host call it wrapped
    assert.deepEqual(commits[0]!.expected, { generation: 1, revision: 1 });
    assert.equal(host.sent("subtitles.operation")[0]!.operationId, commits[0]!.operationId);
    assert.equal(host.sent("subtitles.upload.discard").length, 0);
    assert.equal(shownStatus(window.props), "打开字幕：actual.srt");
    assert.equal(host.osd.at(-1), "打开字幕：actual.srt", "the menu entry reports on screen");
    assert.deepEqual(host.subtitleLoads.map((l) => [l.format, l.title, l.select]), [["srt", "actual.srt", "primary"]]);
    assert.equal(window.props["selected-track"], 2);
    assert.equal(window.props["can-remove"], true);

    window.emit("remove-selected");
    await host.settle();
    assert.equal(host.subtitles.tracks.length, 1);
    assert.equal(shownStatus(window.props), "已移除外部字幕。");
    assert.equal(window.props["can-remove"], false);
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("unreadable, unknown, graphic and oversized files are refused before any upload", async () => {
  const unknown = await srtFile(Buffer.from([0x31, 0x0a, 0xff, 0xfe]));
  const sup = await srtFile(Buffer.from("PG\u0000\u0000\u0000\u0000"));
  const large = await srtFile(Buffer.from(`1\n00:00:00,000 --> 00:00:01,000\n${"A".repeat(2 * 1024 * 1024)}\n`));
  const { host, window, stop } = await open();
  try {
    for (const [path, text] of [
      ["/tmp/missing.ass", "无法读取字幕文件。"],
      [unknown.path, "无法识别的字幕格式"],
      [sup.path, "暂不支持图形字幕文件"],
      [large.path, "字幕文件超过 2 MiB"],
    ] as const) {
      host.dialogAnswers.push([path]);
      window.emit("load-file");
      for (let i = 0; i < 100 && shownStatus(window.props) !== text; i++) await host.settle(20);
      assert.equal(shownStatus(window.props), text);
    }
    assert.equal(host.sent("subtitles.upload.begin").length, 0);
  } finally {
    await stop();
    for (const file of [unknown, sup, large]) await file.cleanup();
  }
});

test("a host refusal of the upload is shown as one Chinese sentence", async () => {
  const file = await srtFile("1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const { host, window, stop } = await open();
  host.handle("subtitles.upload.begin", () => {
    throw new HostError("invalid", "invalid upload ID, byte count or chunk bounds");
  });
  try {
    host.dialogAnswers.push([file.path]);
    window.emit("load-file");
    const refused = "播放器拒绝了该字幕（文件过大或内容无效）。";
    // The plugin reads the file itself before the upload: poll for the result, `settle()` only waits for host calls.
    for (let i = 0; i < 100 && shownStatus(window.props) !== refused; i++) await host.settle(20);
    assert.equal(shownStatus(window.props), refused);
    assert.equal(host.sent("subtitles.commit").length, 0);
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("settings, per-field defaults, reset style, hide and refresh act on the current selection", async () => {
  const { host, window, stop } = await open();
  const settings = await openSettings(host);
  settings.emit("change-color", 1);
  settings.emit("change-outline", 3.4);
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.color, "yellow");
  assert.equal(host.subtitles.presentation.effective?.style.outline, 3);
  settings.emit("slider-released");
  await host.settle();
  assert.equal(settings.props["outline-size"], 3, "a released slider snaps to the applied value");
  settings.emit("reset-field", "color");
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.color, "white");
  assert.equal(host.subtitles.presentation.effective?.style.outline, 3);
  settings.emit("reset-style");
  await host.settle();
  assert.equal(host.subtitles.presentation.effective, null);
  assert.equal(shownStatus(settings.props), "已恢复默认样式。");
  await host.command("track-off").run();
  await host.settle();
  assert.equal(host.subtitles.selection.target, null);
  assert.equal(shownStatus(window.props), "隐藏字幕");
  assert.equal(window.props["selected-track"], 0);
  assert.equal(window.props["can-style"], false);
  assert.equal(window.props["style-reason"], "请先选择字幕轨。");
  await host.command("refresh").run();
  await host.settle();
  assert.equal(shownStatus(window.props), "字幕轨已刷新。");
  await stop();
});

test("Alt+H hides and brings back the last shown track, with OSD and the check mark", async () => {
  const host = sourceHost();
  host.subtitles.tracks.push({ target: { kind: "embedded", id: 2 }, title: "中文", language: "zh", styleSupported: true, styleReason: null });
  host.subtitles.selection.target = { kind: "embedded", id: 2 };
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  assert.equal(host.command("show").state.checked, true);
  assert.equal(host.command("track-2").state.checked, true);
  await host.press("Alt+H");
  await host.settle();
  assert.equal(host.subtitles.selection.target, null);
  assert.equal(host.command("show").state.checked, false);
  assert.equal(host.command("track-off").state.checked, true);
  assert.equal(host.osd.at(-1), "隐藏字幕");
  await host.press("Alt+H");
  await host.settle();
  assert.deepEqual(host.subtitles.selection.target, { kind: "embedded", id: 2 }, "the last shown track, not the first");
  assert.equal(host.command("show").state.checked, true);
  assert.equal(host.osd.at(-1), "显示字幕");
  await host.command("track-1").run();
  await host.settle();
  assert.deepEqual(host.subtitles.selection.target, embedded);
  assert.equal(host.osd.at(-1), "字幕：English · en");
  await stop();
});

test("the track menu follows the real track list: names, loads and source changes", async () => {
  const bytes = Buffer.from("1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const file = await srtFile(bytes);
  const host = sourceHost();
  host.subtitles.tracks.push({ target: { kind: "embedded", id: 2 }, title: "English", language: "en", styleSupported: true, styleReason: null });
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  const titles = () =>
    [...host.commands.values()].filter((c) => c.declaration.group === "subtitle-track").sort((a, b) => a.declaration.order - b.declaration.order).map((c) => c.display.path.at(-1));
  try {
    assert.deepEqual(titles(), ["不显示", "English · en", "English · en (2)"], "equal names stay distinct paths");
    host.dialogAnswers.push([file.path]);
    await withOsd(host, () => host.command("load").run());
    assert.equal(titles().length, 4);
    assert.equal(host.command("track-3").state.checked, true, "the loaded track is selected");
    switchSource(host);
    host.subtitles.tracks.length = 0;
    host.subtitles.selection.target = null;
    await host.command("refresh").run();
    await host.settle();
    assert.deepEqual(titles(), ["不显示"], "items of the old source are removed, not disabled");
    assert.equal(host.command("track-off").state.checked, true);
  } finally {
    await stop();
    await file.cleanup();
  }
});

test(". , / shift the subtitle sync by half a second with OSD; > and < are left to the audio sync", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.press(".");
  assert.equal(host.osd.at(-1), "字幕同步: 超前 0.5 秒");
  await host.press(".");
  assert.equal(host.osd.at(-1), "字幕同步: 超前 1 秒");
  await host.press(",");
  await host.press(",");
  await host.press(",");
  assert.equal(host.osd.at(-1), "字幕同步: 滞后 0.5 秒");
  await host.settle(170);
  assert.equal(host.sent("subtitles.style").length, 1, "key repeats merge into one write");
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 500);
  await host.press("/");
  assert.equal(host.osd.at(-1), "字幕同步: 0 秒");
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 0);
  for (const key of [">", "<", "?"]) await assert.rejects(host.press(key), /nothing is bound/, key);
  await stop();
});

test("sync on a track without style support explains why on screen and writes nothing", async () => {
  const host = sourceHost();
  host.subtitles.tracks[0]!.styleSupported = false;
  host.subtitles.tracks[0]!.styleReason = "图形字幕不支持调整。";
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.press(".");
  await host.settle(170);
  assert.equal(host.osd.at(-1), "图形字幕不支持调整。");
  assert.equal(host.sent("subtitles.style").length, 0);
  host.subtitles.selection.target = null;
  await host.command("refresh").run();
  await host.settle();
  await host.press(",");
  assert.equal(host.osd.at(-1), "请先选择字幕轨。");
  await stop();
});

test("cancelling during the upload stops it, discards and never commits", async () => {
  // An SRT of about 100 KiB: four 32 KiB chunks.
  const file = await srtFile(`1\n00:00:00,000 --> 00:00:01,000\n${"A".repeat(100 * 1024)}\n`);
  const { host, window, stop } = await open();
  host.handle("subtitles.upload.append", (params) => {
    host.reset("subtitles.upload.append");
    window.emit("cancel"); // the user cancels while the first chunk is in flight
    return host.request("subtitles.upload.append", params);
  });
  try {
    host.dialogAnswers.push([file.path]);
    window.emit("load-file");
    // The plugin reads the file before the upload: wait for the discard, `settle()` only waits for host calls.
    for (let i = 0; i < 250 && host.sent("subtitles.upload.discard").length === 0; i++) await host.settle(20);
    await host.settle();
    assert.equal(host.sent("subtitles.upload.append").length, 2); // the intercepted call and its forward
    assert.equal(host.sent("subtitles.upload.discard").length, 1);
    assert.equal(host.sent("subtitles.commit").length, 0);
    assert.equal(host.subtitles.tracks.length, 1);
    assert.equal(window.props["track-labels"]?.toString(), "关闭字幕,English · en");
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("an untitled embedded track is labelled by its track number", async () => {
  const host = sourceHost();
  host.subtitles.tracks.push({ target: { kind: "embedded", id: 3 }, title: "", language: null, styleSupported: false, styleReason: null });
  const { window, stop } = await open(host);
  assert.deepEqual(window.props["track-labels"], ["关闭字幕", "English · en", "内嵌 3"]);
  assert.equal(host.command("track-2").display.path.at(-1), "内嵌 3");
  await stop();
});

/** Three stylable embedded tracks, the first selected. */
function threeTracks() {
  const host = sourceHost();
  host.subtitles.tracks.push(
    { target: { kind: "embedded", id: 2 }, title: "中文", language: "zh", styleSupported: true, styleReason: null },
    { target: { kind: "embedded", id: 3 }, title: "日本語", language: "ja", styleSupported: true, styleReason: null },
  );
  return host;
}
const selectedId = (host: TestHost) => (host.subtitles.selection.target as { id: number } | null)?.id ?? null;

test("D02: Alt+L cycles 1 → 2 → 3 → 关闭 → 1 with OSD; a burst ends on the last press", async () => {
  const host = threeTracks();
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  const seen: (number | null)[] = [];
  for (let i = 0; i < 4; i++) {
    await host.press("Alt+L");
    await host.settle();
    seen.push(selectedId(host));
  }
  assert.deepEqual(seen, [2, 3, null, 1]);
  assert.equal(host.osd.at(-1), "字幕：English · en（1/3）");
  assert.ok(host.osd.includes("字幕：关闭"));
  assert.ok(host.osd.includes("字幕：中文 · zh（2/3）"));
  // Five quick presses count from each other, not from the host's (older) selection.
  const before = host.osd.length;
  await Promise.all([1, 2, 3, 4, 5].map(() => host.press("Alt+L")));
  await host.settle();
  assert.equal(selectedId(host), 2, "1 + 5 steps: 2, 3, 关闭, 1, 2");
  assert.deepEqual(host.osd.slice(before), ["字幕：中文 · zh（2/3）"], "only the final result is shown");
  await stop();
});

test("D02: one track toggles with 关闭; no track disables the command", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.press("Alt+L");
  await host.settle();
  assert.equal(selectedId(host), null);
  await host.press("Alt+L");
  await host.settle();
  assert.equal(selectedId(host), 1);
  host.subtitles.tracks.length = 0;
  host.subtitles.selection.target = null;
  await host.command("refresh").run();
  await host.settle();
  assert.equal(host.command("track-cycle").state.enabled, false);
  assert.equal(host.translate(host.command("track-cycle").state.reason!), "没有字幕轨");
  await stop();
});

test("D06/D07: size and position keys share the tune window's edits and report on screen", async () => {
  const { host, window, stop } = await open();
  await host.press("Alt+PageUp");
  assert.equal(host.osd.at(-1), "字幕尺寸：增加(38)");
  assert.equal(window.props["subtitle-size"], 38);
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.fontSize, 38);
  await host.press("Alt+PageDown");
  await host.press("Alt+PageDown");
  assert.equal(host.osd.at(-1), "字幕尺寸：减小(34)");
  for (let i = 0; i < 25; i++) await host.press("Alt+PageUp");
  assert.equal(host.osd.at(-1), "字幕尺寸：增加(72)", "the limit keeps the value and still reports it");
  // Move Up / Down have no default key (Alt+↑/↓ pan the video): run the commands.
  for (let i = 0; i < 3; i++) await host.command("move-up").run();
  assert.equal(host.osd.at(-1), "字幕位置：上移(8%)");
  await host.command("move-down").run();
  assert.equal(host.osd.at(-1), "字幕位置：下移(7%)");
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.marginPercent, 7);
  await host.press("Alt+Home");
  assert.equal(host.osd.at(-1), "字幕位置：默认(5%)");
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.marginPercent, 5);
  assert.equal(host.subtitles.presentation.effective?.style.fontSize, 72);
  await host.command("track-off").run();
  await host.settle();
  await host.press("Alt+PageUp");
  assert.equal(host.osd.at(-1), "请先选择字幕轨。");
  await stop();
});

test("D05: typed sync values apply in whole ms; bad input reverts with a reason", async () => {
  const { host, stop } = await open();
  const sync = await openSync(host);
  assert.equal(sync.props["value-text"], "0");
  assert.equal(sync.props["can-style"], true);
  sync.emit("apply", "1250");
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 1250);
  assert.equal(host.osd.at(-1), "字幕同步: 滞后 1.25 秒");
  assert.equal(sync.props["value-text"], "1250");
  sync.emit("apply", "−300"); // U+2212 minus
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, -300);
  sync.emit("apply", "abc");
  await host.settle();
  assert.equal(shownStatus(sync.props), "请输入整数毫秒，例如 1250 或 -500。");
  assert.equal(sync.props["value-text"], "-300", "the field goes back to the value in effect");
  sync.emit("apply", "99999999");
  await host.settle();
  assert.equal(shownStatus(sync.props), "同步值须在 −600000 至 600000 毫秒之间");
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, -300);
  sync.emit("nudge", "-300", 1);
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 200);
  assert.equal(sync.props["value-text"], "200");
  await host.press("/");
  await host.settle(170);
  assert.equal(sync.props["value-text"], "0", "the field follows key edits");
  await host.command("track-off").run();
  await host.settle();
  assert.equal(sync.props["can-style"], false);
  assert.equal(sync.props["style-reason"], "请先选择字幕轨。");
  await stop();
});

test("D05: the 同步单位 changes the step and the menu titles, and is saved", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  try {
    let host = sourceHost();
    let started = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
    const settings = await openSettings(host);
    // The step names are in the Slint list, in the order of SYNC_STEPS.
    const slint = await readFile(new URL("../ui/settings.slint", import.meta.url), "utf8");
    const steps = [...slint.matchAll(/@tr\("\{\} s", "([\d.]+)"\)/g)].map((m) => host.translate("{} s", [m[1]!]));
    assert.deepEqual(steps, ["0.05 秒", "0.1 秒", "0.25 秒", "0.5 秒", "1 秒", "5 秒"]);
    assert.equal(settings.props["sync-step-index"], 3);
    settings.emit("change-sync-step", 1);
    await host.settle();
    const title = (id: string) => host.command(id).display.path.at(-1);
    assert.equal(title("sync-earlier"), "超前 0.1 秒");
    assert.equal(title("sync-later"), "滞后 0.1 秒");
    assert.equal(host.command("sync-earlier").declaration.order, 339);
    await host.press(".");
    assert.equal(host.osd.at(-1), "字幕同步: 超前 0.1 秒");
    await host.settle(170);
    assert.equal(host.subtitles.presentation.effective?.style.offsetMs, -100);
    assert.deepEqual(host.conflicts, []);
    await started.stop();
    assert.equal(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).syncStepMs, 100);
    host = sourceHost();
    started = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
    assert.equal(host.command("sync-later").display.path.at(-1), "滞后 0.1 秒");
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

/** A storage file once its debounced write landed. */
async function savedJson(path: string) {
  for (let i = 0; i < 50; i++) {
    try {
      return JSON.parse(await readFile(path, "utf8"));
    } catch {
      await pause(20);
    }
  }
  throw new Error(`${path} was not written`);
}
/** Load `path` through 打开字幕… and return the new track's id. */
async function loadFile(host: TestHost, path: string) {
  host.dialogAnswers.push([path]);
  await withOsd(host, () => host.command("load").run());
}
const externalCount = (host: TestHost) => host.subtitles.tracks.filter((t) => t.target.kind === "external").length;

test("A09: Ctrl+Alt+Y re-reads a changed file, replaces the track, keeps the sync; unchanged files are not re-sent", async () => {
  const file = await srtFile("1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const { host, window, stop } = await open();
  try {
    await loadFile(host, file.path);
    const firstTarget = structuredClone(host.subtitles.selection.target);
    await host.press(",");
    await host.settle(170);
    await withOsd(host, () => host.press("Ctrl+Alt+Y"));
    assert.equal(host.osd.at(-1), "字幕未改变");
    assert.equal(host.sent("subtitles.upload.begin").length, 1);
    await writeFile(file.path, "1\n00:00:00,000 --> 00:00:01,000\nHello again\n");
    await withOsd(host, () => host.press("Ctrl+Alt+Y"));
    assert.equal(host.osd.at(-1), "已重载字幕 actual.srt");
    assert.equal(host.sent("subtitles.upload.begin").length, 2);
    assert.equal(externalCount(host), 1, "the old track is removed");
    assert.notDeepEqual(host.subtitles.selection.target, firstTarget, "the new track is selected");
    assert.equal(host.subtitles.selection.target?.kind, "external");
    assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 500, "sync is unchanged");
    assert.equal(window.props["can-remove"], true);
    await rm(file.path);
    await withOsd(host, () => host.press("Ctrl+Alt+Y"));
    assert.equal(host.osd.at(-1), "无法重载：找不到字幕文件 actual.srt");
    assert.equal(externalCount(host), 1, "the track stays");
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("A09: an embedded track needs no reload; nothing selected says so", async () => {
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.press("Ctrl+Alt+Y");
  await host.settle();
  assert.equal(host.osd.at(-1), "内嵌字幕无需重载");
  await host.command("track-off").run();
  await host.settle();
  await host.press("Ctrl+Alt+Y");
  await host.settle();
  assert.equal(host.osd.at(-1), "没有选中的字幕。");
  assert.equal(host.sent("subtitles.upload.begin").length, 0);
  await stop();
});

test("A09: with 自动重载 on, rewriting a loaded file reloads it after the changes stop", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  const file = await srtFile("1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
  try {
    const settings = await openSettings(host);
    settings.emit("change-auto-reload", true);
    await host.settle();
    assert.equal(settings.props["auto-reload"], true);
    assert.equal((await savedJson(join(data, "settings.json"))).autoReload, true);
    await loadFile(host, file.path);
    await pause(200); // let the folder watch start before the edit
    await writeFile(file.path, "1\n00:00:00,000 --> 00:00:01,000\nChanged\n");
    for (let i = 0; i < 60 && host.osd.at(-1) !== "字幕已自动重载"; i++) await pause(100);
    await host.settle();
    assert.equal(host.osd.at(-1), "字幕已自动重载");
    assert.equal(host.sent("subtitles.upload.begin").length, 2);
    assert.equal(externalCount(host), 1);
    assert.equal(host.subtitles.selection.target?.kind, "external");
  } finally {
    await stop();
    await file.cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

test("A09: an automatic reload of a track that is not shown keeps the current selection", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  const file = await srtFile("1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const host = sourceHost();
  const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
  try {
    const settings = await openSettings(host);
    settings.emit("change-auto-reload", true);
    await loadFile(host, file.path);
    await host.command("track-1").run(); // back to the embedded track
    await host.settle();
    await pause(200);
    await writeFile(file.path, "1\n00:00:00,000 --> 00:00:01,000\nChanged\n");
    for (let i = 0; i < 60 && host.osd.at(-1) !== "字幕已自动重载"; i++) await pause(100);
    await host.settle();
    assert.equal(host.osd.at(-1), "字幕已自动重载");
    assert.deepEqual(host.subtitles.selection.target, embedded);
    assert.equal(externalCount(host), 1);
  } finally {
    await stop();
    await file.cleanup();
    await rm(data, { recursive: true, force: true });
  }
});
