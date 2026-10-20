// Orchestration against the SDK test host only. No native parsing or rendering assertions.
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const embedded = { kind: "embedded" as const, id: 1 };

/** A ready source (generation 1) with one selected, stylable embedded track. */
function sourceHost() {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.subtitles.tracks.push({ target: embedded, title: "English", language: "en", styleSupported: true, styleReason: null });
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: embedded };
  return host;
}
async function open(host = sourceHost()) {
  const started = await startPlugin(activate, { directory, host });
  await host.command("workbench").run();
  return { ...started, window: host.window("subtitle-workbench") };
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

test("independent window with the Slint callbacks only and complete Chinese menu paths", async () => {
  const { host, window, stop } = await open();
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.declaration.path),
    [["字幕", "字幕工作台…"], ["字幕", "加载外部 SRT…"], ["字幕", "关闭字幕"], ["字幕", "刷新字幕轨"]],
  );
  const opened = host.sent("ui.open");
  assert.equal(opened.length, 1);
  assert.equal(opened[0]!.file, "ui/workbench.slint");
  assert.deepEqual([...window.callbacks].sort(), [
    "cancel", "change-color", "change-delay", "change-font", "change-margin", "change-outline",
    "change-size", "load-file", "refresh", "remove-selected", "reset-style", "select-track",
  ]);
  assert.equal(window.props["status-text"], "字幕工作台已就绪。");
  assert.deepEqual(window.props["track-labels"], ["关闭字幕", "English · en"]);
  assert.equal(window.props["selected-track"], 1);
  assert.equal(window.props["can-style"], true);
  assert.equal(window.props["source-text"], "媒体版本 1 · ready · 1 条字幕轨");
  await stop();
});

test("a slider burst is merged into one style write with the latest values", async () => {
  const { host, window, stop } = await open();
  window.emit("change-delay", 200);
  window.emit("change-delay", 650);
  window.emit("change-size", 52);
  await host.settle();
  assert.equal(window.props["delay-ms"], 650); // shown before the host confirms
  await host.settle(170);
  const styles = host.sent("subtitles.style");
  assert.equal(styles.length, 1);
  assert.deepEqual(styles[0]!.style, { fontId: "sans", fontSize: 52, color: "white", outline: 2, marginPercent: 5, offsetMs: 650 });
  assert.equal(window.props["status-text"], "字幕样式：applied。");
  assert.equal(host.subtitles.presentation.effective?.style.offsetMs, 650);
  await stop();
});

test("a style edit only applies to the selection it began on", async () => {
  const { host, window, stop } = await open();
  window.emit("change-size", 60);
  host.subtitles.selection.stamp.revision++; // another plugin changes the selection meanwhile
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective, null);
  assert.equal(window.props["status-text"], "字幕轨已变化，请在当前轨道重新调整。");
  await stop();
});

test("file picker cancellation never begins an upload", async () => {
  const { host, window, stop } = await open();
  host.dialogAnswers.push(null);
  window.emit("load-file");
  await host.settle();
  assert.equal(host.sent("subtitles.upload.begin").length, 0);
  assert.match(String(window.props["status-text"]), /已取消选择/);
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
  assert.match(String(window.props["status-text"]), /媒体已切换/);
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
  window.emit("change-delay", 900);
  switchSource(host);
  await host.settle(140);
  assert.equal(host.sent("subtitles.style").length, 0);
  assert.match(String(window.props["status-text"]), /媒体已切换/);
  window.emit("change-size", 40);
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
    await host.command("load").run();
    await host.settle();
    const appended = host.sent("subtitles.upload.append").map((c) => Buffer.from(String(c.base64), "base64"));
    assert.deepEqual(Buffer.concat(appended), bytes);
    assert.equal(host.sent("subtitles.upload.begin")[0]!.generation, 1);
    const commits = host.sent("subtitles.commit");
    assert.equal(commits.length, 2); // the lost one, then the real host call it wrapped
    assert.deepEqual(commits[0]!.expected, { generation: 1, revision: 1 });
    assert.equal(host.sent("subtitles.operation")[0]!.operationId, commits[0]!.operationId);
    assert.equal(host.sent("subtitles.upload.discard").length, 0);
    assert.equal(window.props["status-text"], `SRT 已加载（${bytes.length} 字节），轨道：applied。`);
    assert.equal(window.props["selected-track"], 2);
    assert.equal(window.props["can-remove"], true);

    window.emit("remove-selected");
    await host.settle();
    assert.equal(host.subtitles.tracks.length, 1);
    assert.equal(window.props["status-text"], "外部字幕已移除。");
    assert.equal(window.props["can-remove"], false);
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("non-SRT and invalid UTF-8 files are refused before any upload", async () => {
  const bad = await srtFile(Buffer.from([0x31, 0x0a, 0xff, 0xfe]));
  const { host, window, stop } = await open();
  try {
    host.dialogAnswers.push(["/tmp/movie.ass"]);
    window.emit("load-file");
    await host.settle();
    assert.equal(window.props["status-text"], "请选择 UTF-8 .srt 字幕文件。");
    host.dialogAnswers.push([bad.path]);
    window.emit("load-file");
    await host.settle();
    assert.match(String(window.props["status-text"]), /encoded data was not valid/);
    assert.equal(host.sent("subtitles.upload.begin").length, 0);
  } finally {
    await stop();
    await bad.cleanup();
  }
});

test("a host refusal of the upload (e.g. over its 2 MiB limit) is shown as the host's message", async () => {
  const file = await srtFile(Buffer.alloc(2 * 1024 * 1024 + 1, 0x41));
  const { host, window, stop } = await open();
  host.handle("subtitles.upload.begin", (p) => {
    if (Number(p.expectedBytes) > 2 * 1024 * 1024)
      throw new HostError("invalid", "invalid upload ID, byte count or chunk bounds");
    return {};
  });
  try {
    host.dialogAnswers.push([file.path]);
    window.emit("load-file");
    await host.settle();
    assert.equal(window.props["status-text"], "invalid upload ID, byte count or chunk bounds");
    assert.equal(host.sent("subtitles.commit").length, 0);
  } finally {
    await stop();
    await file.cleanup();
  }
});

test("disable, refresh and reset style act on the current selection", async () => {
  const { host, window, stop } = await open();
  window.emit("change-color", 1);
  await host.settle(170);
  assert.equal(host.subtitles.presentation.effective?.style.color, "yellow");
  window.emit("reset-style");
  await host.settle();
  assert.equal(host.subtitles.presentation.effective, null);
  assert.equal(window.props["status-text"], "本插件的样式与延迟已撤销。");
  await host.command("disable").run();
  assert.equal(host.subtitles.selection.target, null);
  assert.equal(window.props["status-text"], "字幕已关闭。");
  assert.equal(window.props["selected-track"], 0);
  await host.command("refresh").run();
  assert.equal(window.props["status-text"], "字幕状态已刷新。");
  await stop();
});

test("cancelling during the upload stops it, discards and never commits", async () => {
  const file = await srtFile(Buffer.alloc(100 * 1024, 0x41)); // four 32 KiB chunks
  const { host, window, stop } = await open();
  host.handle("subtitles.upload.append", (params) => {
    host.reset("subtitles.upload.append");
    window.emit("cancel"); // the user cancels while the first chunk is in flight
    return host.request("subtitles.upload.append", params);
  });
  try {
    host.dialogAnswers.push([file.path]);
    window.emit("load-file");
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
  await stop();
});
