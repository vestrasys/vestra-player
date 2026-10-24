import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { isDeepStrictEqual } from "node:util";
import type { Locale } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { clock, frameName, parseRange, REASON, sampleTimes, uniqueFile } from "../model.ts";
import { words } from "./fakes.ts";

/** A job as the test host records it. */
type MediaJob = NonNullable<ReturnType<TestHost["jobs"]["get"]>>;

const directory = new URL("..", import.meta.url);
/** Most tests check what a Chinese user sees (the texts from before the English sources); English is tested on its own. */
const start = (host = new TestHost(), locale: Locale = "zh-Hans", dataDirectory?: string) => startPlugin(activate, { directory, host, locale, dataDirectory });
type Data = Parameters<ReturnType<typeof words>["row"]>[0];
type Row = { id: string; title: string; detail: string; retryable: boolean; openable: boolean };
/** The task rows as the window shows them (title and detail in the test host's language). */
const rows = (host: TestHost): Row[] =>
  (host.window("tasks").props["task-rows"] as (Data & Row)[]).map((row) => ({ ...row, ...words(host).row(row) }));
/** A dialog's status line as shown. */
const statusOf = (host: TestHost, window: string) => words(host).status(host.window(window).props.status as never);
const tick = () => new Promise((done) => setTimeout(done, 5));
/**
 * Poll an observable result. `host.settle()` waits only for host calls, not for
 * the plugin's own file-system work or timers, so assert after those this way.
 */
async function until(predicate: () => boolean | Promise<boolean>) {
  for (let i = 0; i < 1200; i++) {
    if (await predicate()) return;
    await tick();
  }
  assert.fail("timed out waiting for condition");
}
/** The JSON in `path` once it equals `expected` (written by a debounced, asynchronous store). */
const jsonFile = (path: string, expected: unknown) =>
  until(() => readFile(path, "utf8").then((text) => isDeepStrictEqual(JSON.parse(text), expected), () => false));
/** A ready finite source with snapshot support. */
function ready(host: TestHost, path = "/contract/source-one.mp4") {
  host.openElsewhere(path);
  host.setSource({ position: 2, duration: 30 });
}
/** Jobs advance only when the test says so; cancellation ends on the next read. */
function manualJobs(host: TestHost) {
  host.handle("jobs.state", (p) => {
    const job = host.jobs.get(Number(p.jobId));
    if (!job) throw new HostError("missing", "job");
    if (job.status === "cancelling") Object.assign(job, { status: "cancelled", outcome: { cancelled: true } });
    return structuredClone(job);
  });
  return (id: number, patch: Partial<MediaJob>) => Object.assign(host.jobs.get(id)!, patch);
}
/** A temporary capture folder chosen through 截图文件夹…. */
async function withFolder(host: TestHost) {
  const folder = await mkdtemp(join(tmpdir(), "capture-folder-"));
  host.dialogAnswers.push([folder]);
  await host.command("capture.folder").run();
  await until(() => host.osd.at(-1) === `截图文件夹：${folder}`); // after the plugin's own stat of the folder
  return { folder, cleanup: () => rm(folder, { recursive: true, force: true }) };
}

test("ranges, burst bounds, frame names and unique naming", async () => {
  for (const [a, b] of [["", 1], [-1, 1], ["NaN", 1], [1, 1], [2, 1], [0, "Infinity"]]) assert.throws(() => parseRange(a, b));
  assert.deepEqual(parseRange("0.1", "2.5"), { start: 0.1, end: 2.5 });
  assert.deepEqual(sampleTimes(1, 5, 0.5, 10), [1, 1.5, 2, 2.5, 3]);
  for (const [a, b, c, d] of [[0, 1, 1, 10], [0, 21, 1, 100], [0, 5, 0.01, 10], [9, 2, 1, 10]]) assert.throws(() => sampleTimes(a!, b!, c!, d!));
  assert.equal(clock(62.5), "01:02.500");
  assert.equal(clock(3723.004), "1:02:03.004");
  assert.equal(frameName("/movies/My Film.mkv", 62.5, ".png"), "My Film_01.02.500.png");
  assert.equal(frameName(null, 0, ".png"), "capture_00.00.000.png");
  const folder = await mkdtemp(join(tmpdir(), "capture-unique-"));
  try {
    const file = join(folder, "capture.png");
    assert.equal(await uniqueFile(file), file);
    await writeFile(file, "existing");
    assert.equal(await uniqueFile(file), join(folder, "capture (1).png"));
    await writeFile(join(folder, "capture (1).png"), "existing second");
    assert.equal(await uniqueFile(file), join(folder, "capture (2).png"));
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test("menu per the allocation table; each dialog binds exactly its Slint callbacks", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const declared = [...host.commands.values()].map(({ declaration: d, display }) => [display.path.join(" > "), d.order, d.group, d.bindings.join(",")]);
  assert.deepEqual(declared, [
    ["视频 > 截图 > 保存当前画面", 420, "capture-source", "Ctrl+E"],
    ["视频 > 截图 > 当前画面另存为…", 421, "capture-source", ""],
    ["视频 > 截图 > 复制当前画面", 422, "capture-source", "Ctrl+C"],
    ["视频 > 截图 > 保存实际画面", 423, "capture-screen", "Ctrl+Alt+E"],
    ["视频 > 截图 > 实际画面另存为…", 424, "capture-screen", ""],
    ["视频 > 截图 > 复制实际画面", 425, "capture-screen", "Ctrl+Alt+C"],
    ["视频 > 截图 > 连拍…", 426, "capture-export", "Ctrl+G"],
    ["视频 > 截图 > 导出片段…", 428, "capture-export", ""],
    ["视频 > 截图 > 导出任务…", 429, "capture-export", ""],
    ["视频 > 截图 > 截取为 PNG", 430, "capture-format", ""],
    ["视频 > 截图 > 截取为 JPEG", 431, "capture-format", ""],
    ["视频 > 截图 > 截取为 BMP", 432, "capture-format", ""],
    ["视频 > 截图 > 实际画面包含字幕", 435, "capture-name", ""],
    ["视频 > 截图 > 截图设置…", 436, "capture-name", ""],
    ["视频 > 截图 > 截图文件夹…", 437, "capture-folder", ""],
    ["视频 > 截图 > 打开截图文件夹", 438, "capture-folder", ""],
  ]);
  assert.deepEqual(host.conflicts, []);
  await host.press("Ctrl+G");
  await host.command("capture.clip").run();
  await host.command("capture.tasks").run();
  await host.command("capture.settings").run();
  assert.deepEqual(host.sent("ui.open").map((o) => o.file), ["ui/burst.slint", "ui/clip.slint", "ui/tasks.slint", "ui/settings.slint"]);
  for (const name of ["burst", "clip", "tasks", "settings"]) {
    const slint = await readFile(new URL(`../ui/${name}.slint`, import.meta.url), "utf8");
    const callbacks = [...slint.matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
    assert.deepEqual([...host.window(name).callbacks].sort(), callbacks.sort(), name);
    assert.doesNotMatch(slint, /#[0-9a-f]{3,8}\b/i, `${name}: colours come from Theme`);
    assert.doesNotMatch(slint, /font-size:\s*\d/, `${name}: font sizes come from Theme`);
    assert.doesNotMatch(slint, /(text|title):\s*"[^"]*[a-z]{3,}[^"]*"/, `${name}: every label in @tr (format names like PNG are fine)`);
  }
  assert.equal(host.window("clip").props["start-time"], "2");
  assert.equal(host.window("clip").props["end-time"], "12");
  await stop();
});

test("English by default: menus, dialog titles, OSD and window texts are the English source texts", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host, "en");
  try {
    assert.deepEqual(host.command("capture.save").declaration.path, ["@video", "Snapshot", "Save Current Frame"]);
    assert.deepEqual(host.command("capture.saveScreenAs").display.path, ["Video", "Snapshot", "Save Rendered Frame As…"]);
    assert.deepEqual(host.command("capture.format.jpeg").display.path, ["Video", "Snapshot", "Capture as JPEG"]);
    await host.command("capture.format.jpeg").run();
    await host.settle();
    assert.equal(host.osd.at(-1), "Snapshot format: JPEG");
    await host.command("capture.clip").run();
    host.dialogAnswers.push(null);
    host.window("clip").emit("export-clip", "1", "4", false, false, 0);
    await until(() => statusOf(host, "clip") === "No save location chosen.");
    assert.equal(host.sent("dialogs.open").at(-1)!.title, "Export Clip");
    host.window("clip").emit("export-clip", "5", "1", false, false, 0);
    await until(() => statusOf(host, "clip") === REASON.range);
    assert.deepEqual(host.window("clip").props.status, { kind: "error", reason: "range", raw: "", count: 0, name: "" }, "data, not words");
  } finally {
    await stop();
  }
});

test("the reasons in model.ts and the windows' Words.reason in ui/parts.slint are the same texts; all are translated", async () => {
  const slint = await readFile(new URL("../ui/parts.slint", import.meta.url), "utf8");
  const said = Object.fromEntries([...slint.matchAll(/code == "(\w+)"\) \{ return @tr\("([^"]+)"\)/g)].map((m) => [m[1], m[2]]));
  assert.deepEqual(said, { ...REASON });
  const zh = await readFile(new URL("../locales/zh-Hans.po", import.meta.url), "utf8");
  assert.doesNotMatch(zh, /msgstr ""\n\n/, "every text translated");
  const { host, stop } = await start();
  const { reason, progress, status } = words(host);
  assert.equal(reason("space", ""), "磁盘空间不足");
  assert.equal(reason("", "a message of the player"), "a message of the player");
  assert.equal(progress({ status: "running", reason: "", raw: "", done: 3, total: 10 }), "导出中 · 3/10");
  assert.equal(progress({ status: "failed", reason: "exportFailed", raw: "", done: 0, total: 1 }), "失败：导出失败");
  assert.equal(status({ kind: "clip-failed", reason: "space", raw: "", count: 0, name: "" }), "导出失败：磁盘空间不足");
  assert.equal(status({ kind: "started", reason: "", raw: "", count: 4, name: "" }), "已开始连拍 4 张");
  await stop();
});

test("Ctrl+E saves the raw current frame into the capture folder with no dialog and an OSD", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    assert.equal(host.osd.at(-1), `截图文件夹：${folder}`);
    const dialogs = host.sent("dialogs.open").length;
    await host.press("Ctrl+E");
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
    assert.equal(host.sent("dialogs.open").length, dialogs, "no save panel, no confirmation");
    const capture = host.sent("snapshot.capture")[0]!;
    assert.equal(capture.mode, "raw");
    assert.deepEqual(capture.expected, host.source.stamp);
    const job = host.sent("jobs.start")[0]!;
    assert.equal(job.destination, join(folder, "source-one_00.02.000.png"));
    const outcome = host.jobs.get(1)!.outcome as { destination: string };
    assert.equal(host.osd.at(-1), `帧已保存为 ${outcome.destination}`, "the path the host confirmed");
  } finally {
    await stop();
    await cleanup();
  }
});

test("the chosen capture folder is kept in the data directory and restored on the next start", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-")); // outlives each test host, like the player's
  const host = new TestHost();
  ready(host);
  const first = await start(host, "zh-Hans", data);
  const { folder, cleanup } = await withFolder(host);
  const restart = new TestHost();
  ready(restart);
  try {
    // plugin.storage writes 300 ms after the last change
    await jsonFile(join(data, "settings.json"), { folder, format: "png", quality: 90, screenSubtitles: true, besideMedia: false });
    assert.deepEqual((await readdir(data)).sort(), ["settings.json"], "no temporary file left behind");
    await first.stop();
    const second = await startPlugin(activate, { directory, host: restart, dataDirectory: data });
    await restart.press("Ctrl+G");
    assert.equal(restart.window("burst").props.folder, folder);
    await second.stop();
    await rm(folder, { recursive: true, force: true }); // a removed folder falls back to the default
    const third = await startPlugin(activate, { directory, host: new TestHost(), dataDirectory: data });
    await third.host.command("capture.burst").run();
    assert.notEqual(third.host.window("burst").props.folder, folder);
    await third.stop();
  } finally {
    await cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

test("save commands are disabled without a capturable source", async () => {
  const { host, stop } = await start();
  assert.equal(host.command("capture.save").state.enabled, false);
  assert.equal(host.command("capture.save").state.reason, "No frame to capture");
  assert.equal(host.translate(host.command("capture.save").state.reason!), "没有可截取的画面");
  assert.equal(host.command("capture.saveAs").state.enabled, false);
  ready(host);
  await host.settle();
  assert.equal(host.command("capture.save").state.enabled, true);
  await stop();
});

test("另存为 asks for a file; a source change while it is open captures nothing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-as-"));
  try {
    host.handle("dialogs.open", () => {
      host.openElsewhere("/contract/other.mp4");
      return { cancelled: false, paths: [join(folder, "a.png")] };
    });
    await host.command("capture.saveAs").run();
    await host.settle();
    assert.equal(host.sent("snapshot.capture").length, 0);
    assert.equal(host.osd.at(-1), "截图失败：播放内容已变化，未开始。");
    host.reset("dialogs.open");
    host.setSource({ position: 5, duration: 30 });
    host.dialogAnswers.push([join(folder, "named")]);
    await host.command("capture.saveAs").run();
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
    assert.equal(host.sent("snapshot.capture")[0]!.time, 5, "the frame shown when the command ran");
    assert.equal(host.sent("jobs.start")[0]!.destination, join(folder, "named.png"));
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("clip: the snapshot is conditional on the seen source and honours the dialog options", async () => {
  const host = new TestHost();
  ready(host);
  host.setSource({ audioTracks: [{ trackId: 7, selected: true, external: false, streamIndex: 1, codec: "aac", title: null, language: "en" }] });
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-options-"));
  try {
    await host.command("capture.clip").run();
    const clip = host.window("clip");
    assert.deepEqual(clip.props["audio-labels"], ["无声音", "当前音轨", "音轨 7 · en"]);
    host.dialogAnswers.push([join(folder, "clip.mp4")]);
    clip.emit("export-clip", "1", "4", true, false, 2);
    // The status follows the job start (the name is checked on disk first); the job waits for setJob.
    await until(() => host.sent("jobs.start").length === 1 && statusOf(host, "clip") === "已开始导出，可在导出任务中查看。");
    const capture = host.sent("snapshot.capture")[0]!;
    assert.deepEqual(capture.expected, host.source.stamp);
    assert.equal(capture.mode, "processed");
    assert.equal(capture.time, 1);
    assert.equal(capture.includeSubtitles, false);
    assert.deepEqual(capture.audio, { kind: "track", trackId: 7 });
    assert.deepEqual({ kind: host.sent("jobs.start")[0]!.kind, end: host.sent("jobs.start")[0]!.end }, { kind: "processedMp4", end: 4 });
    setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: join(folder, "clip.mp4") } });
    await until(() => host.osd.at(-1) === `片段已导出为 ${join(folder, "clip.mp4")}` && statusOf(host, "clip") === "已导出 clip.mp4");
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("the clip guard is taken before the first await; a rejected source releases it", async () => {
  const { host, stop } = await start();
  await host.command("capture.clip").run();
  const clip = host.window("clip");
  const reads = host.sent("source.state").length;
  clip.emit("export-clip", "0", "1", false, false, 0);
  clip.emit("export-clip", "0", "1", false, false, 0);
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 1, "duplicate entry excluded before the source read");
  assert.equal(statusOf(host, "clip"), "当前画面无法导出，请先打开本地视频文件。");
  assert.equal(clip.props.busy, false);
  clip.emit("export-clip", "0", "1", false, false, 0);
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 2, "guard released after rejection");
  assert.equal(host.sent("dialogs.open").length, 0);
  await stop();
});

test("one running and four queued; the sixth is refused; queued input keeps its source; cancel", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const queued = () => host.osd.filter((line) => line === "截图已排队").length;
    for (let i = 1; i <= 5; i++) {
      await host.press("Ctrl+E");
      // Each press finishes (unique name, task accepted) before the next, or 仍在处理 would refuse it.
      await until(() => host.sent("snapshot.capture").length === i && (i === 1 ? host.sent("jobs.start").length === 1 : queued() === i - 1));
    }
    assert.equal(host.osd.at(-1), "截图已排队");
    assert.equal(host.sent("jobs.start").length, 1, "only one accepted task starts its worker");
    await host.press("Ctrl+E");
    await until(() => host.osd.at(-1) === "截图失败：已有一个导出在进行、四个在排队，请稍候。"); // after creating the folder
    assert.equal(host.sent("snapshot.capture").length, 5, "the sixth entry freezes nothing");
    await host.command("capture.tasks").run();
    host.window("tasks").emit("cancel-task", "2");
    await until(() => rows(host).some((row) => row.id === "2" && row.detail.endsWith("已取消")));
    assert.equal(host.sent("jobs.cancel").length, 0, "queued cancellation leaves the running worker alone");
    host.openElsewhere("/contract/source-two.mp4");
    setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: join(folder, "source-one_00.02.000.png") } });
    await until(() => host.sent("jobs.start").length === 2);
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 3, "skips the cancelled task and uses the third frozen input");
    assert.equal((host.snapshots.get(3) as { sourceStamp: { generation: number } }).sourceStamp.generation, 1, "queued input not refreshed");
    assert.equal(host.snapshots.has(1), false, "a completed task releases its input");
    await until(() => rows(host).find((row) => row.id === "1")!.detail.endsWith("已完成")); // the row update may trail the job start
    const first = rows(host).find((row) => row.id === "1")!;
    assert.deepEqual([first.title, first.detail], ["截图 · source-one_00.02.000.png", "00:02.000 · 已完成"]);
    assert.equal(rows(host).find((row) => row.id === "3")!.title, "截图 · source-one_00.02.000 (2).png", "queued frames never share a name");
    host.window("tasks").emit("cancel-task", "3");
    await until(() => rows(host).some((row) => row.id === "3" && row.detail.endsWith("已取消")));
    assert.equal(host.sent("jobs.cancel").length, 1, "the active job is cancelled");
    assert.equal(rows(host).find((row) => row.id === "3")!.retryable, true);
  } finally {
    await stop();
    await cleanup();
  }
});

test("burst: unique folder under the capture folder, one snapshot, sample times t0 + i × spacing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    await host.press("Ctrl+G");
    const burst = host.window("burst");
    assert.equal(burst.props.folder, folder);
    assert.equal(burst.props["burst-count"], undefined, "the dialog keeps its own defaults");
    burst.emit("start-burst", "1", "3", "0.5", false, false, false);
    const progress = () => words(host).progress(burst.props.progress as never);
    await until(() => progress() === "已完成 · 3/3" && statusOf(host, "burst") === "已保存 3 张" && host.osd.at(-1)!.startsWith("连拍已保存"));
    assert.equal(host.sent("snapshot.capture").length, 1);
    const starts = host.sent("jobs.start");
    assert.deepEqual(starts.map((s) => s.sampleTime), [1, 1.5, 2]);
    const [child] = await readdir(folder);
    assert.match(child!, /^Capture-/);
    assert.deepEqual(starts.map((s) => s.destination), ["001-1.000s.png", "002-1.500s.png", "003-2.000s.png"].map((name) => join(folder, child!, name)));
    assert.equal(statusOf(host, "burst"), "已保存 3 张");
    assert.match(host.osd.at(-1)!, /^连拍已保存 3 张到 /);
    burst.emit("start-burst", "29", "3", "1", false, false, false);
    await until(() => statusOf(host, "burst") === "连拍超出媒体时长，请减少张数或间隔。"); // after creating the folder
    burst.emit("capture-frame");
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
  } finally {
    await stop();
    await cleanup();
  }
});

test("a failed task keeps its input; retry reuses it after the source changed", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 1);
    setJob(1, { status: "failed", outcome: { error: { code: "space", message: "disk full" } } });
    await until(() => host.osd.at(-1) === "截图失败：磁盘空间不足");
    await host.command("capture.tasks").run();
    assert.equal(rows(host)[0]!.detail, "00:02.000 · 失败：磁盘空间不足");
    assert.equal(rows(host)[0]!.retryable, true);
    host.openElsewhere("/contract/source-two.mp4");
    host.window("tasks").emit("retry-task", "1");
    // The new task starts its job before retry() returns; the status follows.
    await until(() => host.sent("jobs.start").length === 2 && rows(host).length === 2 && statusOf(host, "tasks") === "已重新开始。");
    assert.equal(host.sent("snapshot.capture").length, 1, "no new snapshot");
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 1);
    assert.equal(host.sent("jobs.start")[1]!.destination, join(folder, "source-one_00.02.000 (1).png"));
  } finally {
    await stop();
    await cleanup();
  }
});

test("a dialog that cannot open is reported in a visible error window", async () => {
  const host = new TestHost();
  host.handle("ui.open", (p) => {
    if (p.id === "burst") throw new HostError("ui", "injected native open rejection");
    host.reset("ui.open");
    return host.request("ui.open", p);
  });
  const { stop } = await start(host);
  await host.command("capture.burst").run();
  await host.settle();
  const error = host.window("error");
  assert.equal(error.file, "ui/error.slint");
  assert.deepEqual(error.callbacks, []);
  // Its one text is in the Slint file.
  assert.equal(error.props.message, undefined);
  const text = "The snapshot window could not open. Close this notice and try again.";
  assert.match(await readFile(new URL("../ui/error.slint", import.meta.url), "utf8"), new RegExp(`@tr\\("${text}"\\)`));
  assert.equal(host.translate(text), "截图窗口无法打开，请关闭此提示后重试。");
  // A later successful open closes the error window.
  await host.command("capture.burst").run();
  await host.settle();
  assert.equal(host.windows.has("error"), false);
  assert.equal(host.window("burst").visible, true);
  await stop();
});

test("times the user changed survive other callbacks", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    await host.command("capture.clip").run();
    const clip = host.window("clip");
    clip.props["start-time"] = "5"; // the user types
    clip.emit("mark-end");
    await host.settle();
    assert.equal(clip.props["start-time"], "5");
    assert.equal(clip.props["end-time"], "2");
    await host.command("capture.clip").run(); // reopening an open dialog keeps what the user typed
    assert.equal(clip.props["start-time"], "5");
  } finally {
    await stop();
  }
});

test("Burst… is checked while its dialog is open; a title-bar close clears it (onClose)", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    const burst = host.command("capture.burst");
    assert.equal(burst.state.checked, false);
    await host.press("Ctrl+G");
    await host.settle();
    assert.equal(burst.state.checked, true);
    host.window("burst").userClose();
    await host.settle();
    assert.equal(host.windows.has("burst"), false);
    assert.equal(burst.state.checked, false);
    await burst.run(); // opens again, checked again
    await host.settle();
    assert.equal(host.window("burst").visible, true);
    assert.equal(burst.state.checked, true);
  } finally {
    await stop();
  }
});
