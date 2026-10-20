import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { MediaJob } from "../../../sdk/media.ts";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";
import { parseRange, sampleTimes, uniqueFile } from "../model.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost()) => startPlugin(activate, { directory, host });
type Row = { id: string; summary: string; retained: boolean; openable: boolean };
const rows = (host: TestHost) => host.window("capture").props["task-rows"] as Row[];
const props = (host: TestHost) => host.window("capture").props;
const tick = () => new Promise((done) => setTimeout(done, 5));
async function until(predicate: () => boolean) {
  for (let i = 0; i < 1200; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("timed out waiting for condition");
}
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
/** Confirm every final confirmation as soon as it is shown. */
function autoConfirm(host: TestHost) {
  const timer = setInterval(() => {
    if (host.windows.get("capture")?.props.confirming === true) host.window("capture").emit("confirm-start");
  }, 2);
  return () => clearInterval(timer);
}

test("ranges, burst bounds and unique naming", async () => {
  for (const [a, b] of [["", 1], [-1, 1], ["NaN", 1], [1, 1], [2, 1], [0, "Infinity"]]) assert.throws(() => parseRange(a, b));
  assert.deepEqual(parseRange("0.1", "2.5"), { start: 0.1, end: 2.5 });
  assert.deepEqual(sampleTimes(1, 5, 0.5, 10), [1, 1.5, 2, 2.5, 3]);
  for (const [a, b, c, d] of [[0, 1, 1, 10], [0, 21, 1, 100], [0, 5, 0.01, 10], [9, 2, 1, 10]]) assert.throws(() => sampleTimes(a!, b!, c!, d!));
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

test("four menu entrances share one window; every Slint callback is bound before it opens", async () => {
  const { host, stop } = await start();
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.declaration.path),
    ["保存当前画面…", "连拍…", "导出片段…", "查看导出任务…"].map((title) => ["视频", "截图与导出", title]),
  );
  for (const command of host.commands.values()) assert.equal(command.declaration.title, command.declaration.path.at(-1));
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d }) => ("order" in d ? d.order : null)),
    [321, 324, 322, 323],
  );
  await host.command("capture.clip").run();
  await host.command("capture.tasks").run();
  assert.equal(host.sent("ui.open").length, 1);
  assert.equal(props(host).section, "Export tasks");
  const slint = await readFile(new URL("../capture.slint", import.meta.url), "utf8");
  const declared = [...slint.matchAll(/callback\s+([\w-]+)\s*\(/g)].map((m) => m[1]!);
  assert.deepEqual([...host.window("capture").callbacks].sort(), declared.sort());
  await stop();
});

test("the form guard is taken before the first await; a rejected source releases it", async () => {
  const { host, stop } = await start();
  await host.command("capture.still").run();
  const reads = host.sent("source.state").length;
  host.window("capture").emit("export-image", "0", false, false, false);
  host.window("capture").emit("export-image", "0", false, false, false);
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 1, "duplicate entry excluded before the source read");
  assert.match(String(props(host).status), /^Export not accepted: This source cannot be captured/);
  assert.equal(props(host)["form-busy"], false);
  host.window("capture").emit("export-image", "0", false, false, false);
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 2, "guard released after rejection");
  assert.equal(host.sent("dialogs.open").length, 0);
  await stop();
});

test("a source change while choosing the destination fails as stale and captures nothing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  await host.command("capture.still").run();
  host.handle("dialogs.open", () => {
    host.openElsewhere("/contract/other.mp4");
    return { cancelled: false, paths: [join(tmpdir(), "capture-stale.png")] };
  });
  host.window("capture").emit("export-image", "0", false, false, true);
  await host.settle();
  assert.match(String(props(host).status), /^Export not accepted: Capture cancelled or source intent changed/);
  assert.equal(host.sent("snapshot.capture").length, 0);
  assert.equal(props(host).confirming, false);
  await stop();
});

test("a source change during confirmation cancels the start; Back and cancel-form start nothing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  await host.command("capture.still").run();
  const window = host.window("capture");
  const png = join(tmpdir(), "capture-confirm.png");
  host.dialogAnswers.push([png], [png], [png], null);
  window.emit("export-image", "0", false, false, true);
  await until(() => props(host).confirming === true);
  assert.match(String(props(host).confirmation), /Source: \/contract\/source-one\.mp4/);
  window.emit("reject-start");
  await host.settle();
  assert.equal(props(host).status, "Start cancelled.");
  window.emit("export-image", "0", false, false, true);
  await until(() => props(host).confirming === true);
  window.emit("cancel-form");
  await host.settle();
  assert.equal(props(host).status, "Start cancelled.");
  window.emit("export-image", "0", false, false, true);
  await until(() => props(host).confirming === true);
  host.openElsewhere("/contract/other.mp4");
  await host.settle();
  assert.equal(props(host).confirming, false);
  window.emit("export-image", "0", false, false, true);
  await host.settle();
  assert.equal(props(host).status, "Save cancelled. No new export was accepted.");
  assert.equal(host.sent("snapshot.capture").length, 0);
  await stop();
});

test("the snapshot is conditional on the confirmed source and honours the form options", async () => {
  const host = new TestHost();
  ready(host);
  host.setSource({ audioTracks: [{ trackId: 7, selected: true, external: false, streamIndex: 1, codec: "aac", title: null, language: "en" }] });
  const { stop } = await start(host);
  const done = autoConfirm(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-options-"));
  try {
    await host.command("capture.clip").run();
    assert.equal(props(host).tracks, "7: en (selected)");
    assert.equal(props(host)["start-time"], "2");
    assert.equal(props(host)["end-time"], "12");
    host.dialogAnswers.push([join(folder, "clip.mp4")]);
    host.window("capture").emit("export-clip", "1", "4", true, "selected", false);
    await until(() => host.sent("jobs.start").length === 1);
    const capture = host.sent("snapshot.capture")[0]!;
    assert.deepEqual(capture.expected, host.source.stamp);
    assert.equal(capture.mode, "processed");
    assert.equal(capture.time, 1);
    assert.equal(capture.includeSubtitles, false);
    assert.deepEqual(capture.audio, { kind: "track", trackId: 7 });
    assert.deepEqual({ kind: host.sent("jobs.start")[0]!.kind, end: host.sent("jobs.start")[0]!.end }, { kind: "processedMp4", end: 4 });
  } finally {
    done();
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("one running and four queued; the sixth is refused; queued input keeps its source; cancel", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const done = autoConfirm(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-queue-"));
  try {
    await host.command("capture.still").run();
    for (let i = 1; i <= 5; i++) {
      host.dialogAnswers.push([join(folder, "new-export.png")]);
      host.window("capture").emit("export-image", "0", false, false, false);
      await until(() => host.sent("snapshot.capture").length === i && props(host)["form-busy"] === false && rows(host).length === i);
    }
    assert.equal(host.sent("jobs.start").length, 1, "only one accepted task starts its worker");
    assert.match(String(props(host)["queue-count"]), /^5\/5 accepted/);
    host.window("capture").emit("export-image", "0", false, false, false);
    await until(() => String(props(host).status).includes("four queued"));
    assert.equal(host.sent("snapshot.capture").length, 5, "the sixth entry freezes nothing");
    host.window("capture").emit("cancel-task", "2");
    await until(() => rows(host).some((row) => row.id === "2" && row.summary.includes("cancelled")));
    assert.equal(host.sent("jobs.cancel").length, 0, "queued cancellation leaves the running worker alone");
    host.openElsewhere("/contract/source-two.mp4");
    setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: join(folder, "new-export.png") } });
    await until(() => host.sent("jobs.start").length === 2);
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 3, "skips the cancelled task and uses the third frozen input");
    assert.equal((host.snapshots.get(3) as { sourceStamp: { generation: number } }).sourceStamp.generation, 1, "queued input not refreshed");
    assert.match(rows(host).find((row) => row.id === "1")!.summary, /\/contract\/source-one\.mp4/);
    assert.equal(host.snapshots.has(1), false, "a completed task releases its input");
    host.window("capture").emit("cancel-task", "3");
    await until(() => rows(host).some((row) => row.id === "3" && row.summary.includes("cancelled")));
    assert.equal(host.sent("jobs.cancel").length, 1, "the active job is cancelled");
    assert.equal(rows(host).find((row) => row.id === "3")!.retained, true, "cancelled input is retained for retry");
  } finally {
    done();
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("burst: unique folder, one snapshot, sample times t0 + i × spacing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const done = autoConfirm(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-burst-"));
  try {
    await host.command("capture.burst").run();
    host.dialogAnswers.push([folder]);
    host.window("capture").emit("export-burst", "1", "3", "0.5", false, false, false);
    await until(() => rows(host)[0]?.summary.includes("· raw · completed\n") ?? false);
    assert.equal(host.sent("snapshot.capture").length, 1);
    const starts = host.sent("jobs.start");
    assert.deepEqual(starts.map((s) => s.sampleTime), [1, 1.5, 2]);
    const [child] = await readdir(folder);
    assert.match(child!, /^Capture-/);
    assert.deepEqual(starts.map((s) => s.destination), ["001-1.000s.png", "002-1.500s.png", "003-2.000s.png"].map((name) => join(folder, child!, name)));
    host.dialogAnswers.push([folder]);
    host.window("capture").emit("export-burst", "29", "3", "1", false, false, false);
    await host.settle();
    assert.match(String(props(host).status), /Burst exceeds source duration/);
  } finally {
    done();
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("a failed task keeps its input; retained retry reuses it after the source changed", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const done = autoConfirm(host);
  const folder = await mkdtemp(join(tmpdir(), "capture-retry-"));
  try {
    await host.command("capture.still").run();
    host.dialogAnswers.push([join(folder, "a.png")]);
    host.window("capture").emit("export-image", "5", false, false, false);
    await until(() => host.sent("jobs.start").length === 1);
    setJob(1, { status: "failed", outcome: { error: { code: "space", message: "disk full" } } });
    await until(() => rows(host)[0]?.summary.includes("· raw · failed\n") ?? false);
    assert.match(rows(host)[0]!.summary, /space: disk full/);
    assert.equal(rows(host)[0]!.retained, true);
    host.openElsewhere("/contract/source-two.mp4");
    host.dialogAnswers.push([join(folder, "b.png")]);
    host.window("capture").props.section = "Export tasks"; // the user is on the tasks tab
    host.window("capture").emit("retry-retained", "1");
    await until(() => host.sent("jobs.start").length === 2 && rows(host).length === 2);
    assert.equal(host.sent("snapshot.capture").length, 1, "no new snapshot");
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 1);
    assert.equal(props(host).section, "Export tasks");
    assert.match(rows(host)[0]!.summary, /^#2 · \/contract\/source-one\.mp4/);
    assert.equal(rows(host)[1]!.retained, false, "the old task gave its input away");
  } finally {
    done();
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("a window that cannot open is reported in a visible error window", async () => {
  const host = new TestHost();
  host.handle("ui.open", (p) => {
    if (p.id === "capture") throw new HostError("ui", "injected native open rejection");
    host.reset("ui.open");
    return host.request("ui.open", p);
  });
  const { stop } = await start(host);
  await host.command("capture.still").run();
  await host.settle();
  const error = host.window("capture-error");
  assert.equal(error.file, "capture-error.slint");
  assert.deepEqual(error.callbacks, []);
  assert.match(String(error.props.message), /injected native open rejection/);
  // A later successful open closes the error window.
  await host.command("capture.still").run();
  await host.settle();
  assert.equal(host.windows.has("capture-error"), false);
  assert.equal(host.window("capture").visible, true);
  await stop();
});

test("the tab and times the user changed survive other callbacks", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    await host.command("capture.clip").run();
    assert.equal(props(host).section, "Video clip");
    assert.equal(props(host)["start-time"], "2");
    props(host)["start-time"] = "5"; // the user types
    host.window("capture").emit("mark-end");
    await host.settle();
    assert.equal(props(host)["start-time"], "5");
    assert.equal(props(host)["end-time"], "2");
    props(host).section = "Burst"; // the user switches tab
    host.window("capture").emit("mark-start");
    await host.settle();
    assert.equal(props(host).section, "Burst");
    assert.equal(props(host)["start-time"], "2");
    await host.command("capture.tasks").run(); // a menu entrance still selects its tab
    assert.equal(props(host).section, "Export tasks");
  } finally {
    await stop();
  }
});
