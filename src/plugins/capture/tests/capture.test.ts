import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";

import type { Locale } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { frameName, REASON } from "../model.ts";
import { eventually } from "./fakes.ts";

/** A job as the test host records it. */
type MediaJob = NonNullable<ReturnType<TestHost["jobs"]["get"]>>;

const directory = new URL("..", import.meta.url);
/** Most tests check what a Chinese user sees (the texts from before the English sources); English is tested on its own. */
const start = (host = new TestHost(), locale: Locale = "zh-Hans", dataDirectory?: string) => startPlugin(activate, { directory, host, locale, dataDirectory });
/** Options ▸ Video ▸ Snapshot: the page and its bridge values (Slint names). */
const page = (host: TestHost) => host.settingsPage("snapshot");
const props = (host: TestHost) => page(host).props;
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
/** The JSON in `path` once it matches `expected` (written by a debounced, asynchronous store). */
const jsonFile = (path: string, check: (value: Record<string, unknown>) => boolean) =>
  until(() => readFile(path, "utf8").then((text) => check(JSON.parse(text)), () => false));
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
  // The host records a job after its start request: wait for it, so a test never patches a job that is not there yet.
  return async (id: number, patch: Partial<MediaJob>) => {
    await until(() => !!host.jobs.get(id));
    Object.assign(host.jobs.get(id)!, patch);
  };
}
/** A temporary capture folder chosen through Snapshot Folder…. */
async function withFolder(host: TestHost) {
  const folder = await mkdtemp(join(tmpdir(), "capture-folder-"));
  host.dialogAnswers.push([folder]);
  await host.command("capture.folder").run();
  await until(() => host.osd.at(-1) === `截图文件夹：${folder}`); // after the plugin's own stat of the folder
  return { folder, cleanup: () => rm(folder, { recursive: true, force: true }) };
}

test("Snapshot ▸ has three items; every other command is in the command palette with its keys; no panel", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    const menu = [...host.commands.values()].filter((c) => c.declaration.path?.length).map(({ declaration: d, display }) => [display.path.join(" > "), d.order, d.group, d.bindings.join(",")]);
    assert.deepEqual(menu, [
      // User ruling 2026-11-11: Snapshot ▸ is in Tools ▸, in the domain group.
      ["工具 > 截图 > 保存当前画面", 420, "domain", "Ctrl+E"],
      ["工具 > 截图 > 复制当前画面", 422, "domain", "Ctrl+C"],
      ["工具 > 截图 > 连拍", 426, "domain", "Ctrl+G"],
    ]);
    const palette = [...host.commands.entries()].filter(([, c]) => !c.declaration.path?.length).map(([id, { declaration: d }]) => [id, d.bindings.join(","), d.home ?? ""]);
    assert.deepEqual(palette, [
      ["capture.settings", "", ""],
      ["capture.saveAs", "", ""],
      ["capture.saveScreen", "Ctrl+Alt+E", ""],
      ["capture.saveScreenAs", "", ""],
      ["capture.copyScreen", "Ctrl+Alt+C", ""],
      ["capture.saveVideo", "", ""],
      ["capture.copyVideo", "", ""],
      ["capture.format.png", "", "capture.settings"],
      ["capture.format.jpeg", "", "capture.settings"],
      ["capture.format.bmp", "", "capture.settings"],
      ["capture.screenSubtitles", "", ""],
      ["capture.folder", "", "capture.settings"],
      ["capture.openFolder", "", ""],
      ["capture.revealLast", "", ""],
      ["capture.copyLast", "", ""],
    ]);
    // Clip export is a store plugin's now: its commands and the Capture tab are gone.
    for (const id of ["capture.clip", "capture.clipExport", "capture.panel", "capture.tasks", "capture.clipStart", "capture.clipEnd", "capture.last30"]) assert.equal(host.commands.has(id), false, id);
    assert.deepEqual(host.conflicts, []);
    assert.equal(host.sent("ui.open").length, 0, "no windows");
    assert.equal(host.layout.sidebar?.panels?.length ?? 0, 0, "no sidebar tab");
    const settings = page(host);
    assert.deepEqual([settings.category, settings.title, settings.file, settings.component, settings.global], ["video", "截图", "ui/settings.slint", "SnapshotSettingsPage", "SnapshotPage"]);
  } finally {
    await stop();
  }
});

test("every command has a category for the command palette, and every home is one of its commands", async () => {
  for (const locale of ["en", "zh-Hans"] as const) {
    const host = new TestHost();
    ready(host);
    const { stop } = await start(host, locale);
    try {
      assert.equal(host.commands.size, 18);
      for (const [id, command] of host.commands) {
        assert.ok(command.declaration.category, `${id}: declares a category`);
        assert.ok(command.display.category?.trim(), `${id}: shows a category in ${locale}`);
        assert.ok((command.display.path.at(-1) ?? command.display.title ?? "").trim(), `${id}: has a title`);
        const { home } = command.declaration;
        if (home !== undefined) assert.ok(host.commands.has(home), `${id}: home ${home} is a command of this plugin`);
        if (locale === "en") assert.equal(command.display.category, "Snapshot", id);
      }
    } finally {
      await stop();
    }
  }
});

test("English by default: menus, the settings page's title, OSD and the save panel are the English source texts", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host, "en");
  try {
    assert.deepEqual([host.command("capture.save").declaration.path, host.command("capture.save").declaration.group], [["@tools", "Snapshot", "Save Current Frame"], "domain"]);
    assert.deepEqual(host.command("capture.burst").display.path, ["Tools", "Snapshot", "Burst"]);
    assert.deepEqual([host.command("capture.settings").display.path, host.command("capture.settings").display.title], [[], "Snapshot Settings…"], "palette only");
    assert.equal(page(host).title, "Snapshot");
    await host.command("capture.format.jpeg").run();
    await host.settle();
    assert.equal(host.osd.at(-1), "Snapshot format: JPEG");
    host.dialogAnswers.push(null);
    await host.command("capture.saveAs").run();
    assert.equal(host.sent("dialogs.open").at(-1)!.title, "Save Current Frame As");
    assert.equal(host.sent("dialogs.open").at(-1)!.fileName, "source-one_00.02.000.jpg");
  } finally {
    await stop();
  }
});

test("every reason and every text is translated in all six languages", async () => {
  for (const locale of ["zh-Hans", "de", "es", "fr", "ja", "ko"]) {
    const po = await readFile(new URL(`../locales/${locale}.po`, import.meta.url), "utf8");
    assert.doesNotMatch(po.replace(/^msgid ""\nmsgstr ""\n/, ""), /msgstr ""\n\n/, `${locale}: every text translated`);
    assert.doesNotMatch(po, /^#~ /m, `${locale}: no obsolete texts`);
    for (const text of Object.values(REASON)) assert.ok(po.includes(`msgid "${text}"`), `${locale}: ${text}`);
  }
  const { host, stop } = await start();
  assert.equal(host.translate(REASON.space), "磁盘空间不足");
  await stop();
});

test("the settings page's Slint: library widgets, Theme tokens, every label in @tr, every callback handled", async () => {
  const slint = await readFile(new URL("../ui/settings.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{ Button, Check, Choice, Slider \} from "@vestra\/widgets\.slint";/);
  assert.doesNotMatch(slint, /ComboBox|CheckBox|StandardListView/);
  assert.doesNotMatch(slint, /#[0-9a-f]{3,8}\b/i, "colours come from Theme");
  assert.doesNotMatch(slint, /font-size:\s*\d/, "font sizes come from Theme");
  assert.doesNotMatch(slint, /(text|title|tip|label|detail|summary):\s*"[^"]*[a-z]{3,}[^"]*"/, "every label in @tr");
  // A `for` only over literal lists (the label column's measure).
  const loops = [...slint.matchAll(/for \w+ in ([\w.-]+|\[[^\]]*\])/g)].map((m) => m[1]);
  assert.deepEqual(loops.filter((what) => !what!.startsWith("[")), []);
  // The bridge's callbacks are the ones the plugin handles.
  const main = await readFile(new URL("../main.ts", import.meta.url), "utf8");
  const camel = (name: string) => name.replace(/-(\w)/g, (_, c: string) => c.toUpperCase());
  const handled = new Set([...main.matchAll(/\bon\("(\w+)"/g)].map((m) => m[1]));
  const bridge = slint.slice(slint.indexOf("export global SnapshotPage"), slint.indexOf("component Section"));
  const declared = [...bridge.matchAll(/callback ([\w-]+)\(/g)].map((m) => camel(m[1]!));
  assert.deepEqual(declared.filter((name) => !handled.has(name)), [], "every bridge callback has a handler");
  assert.deepEqual([...handled].filter((name) => !declared.includes(name!)), [], "every handler has a bridge callback");
});

test("Ctrl+E saves the current frame into the capture folder with no dialog and an OSD with buttons", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
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
    assert.deepEqual(host.osdActions.at(-1)?.map((action) => action.command), ["capture.revealLast", "capture.copyLast"]);
  } finally {
    await stop();
    await cleanup();
  }
});

/**
 * A playing source: every read of the state returns the position at that
 * moment, and the clock then moves on four frames at 60 fps, so the source
 * has moved on by the time the plugin asks for the snapshot. Returns the
 * positions read, oldest first.
 */
function playing(host: TestHost) {
  const reads: number[] = [];
  host.handle("source.state", () => {
    const now = structuredClone(host.source);
    reads.push(now.position ?? 0);
    // After the reply, so the read returns this moment and not the next.
    setImmediate(() => host.setSource({ position: (now.position ?? 0) + 4 / 60 }));
    return now;
  });
  return reads;
}

test("while playing, Save Current / Rendered Frame capture the frame at the press and name it by the time the host reports", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const reads = playing(host);
    const saved = async (keys: string, mode: string) => {
      const before = { jobs: host.sent("jobs.start").length, reads: reads.length };
      await host.press(keys);
      await until(() => host.sent("jobs.start").length === before.jobs + 1);
      const capture = host.sent("snapshot.capture").at(-1)!;
      assert.equal(capture.mode, mode);
      assert.equal(capture.time, reads[before.reads], "the position read at the press, though the clock moved on during the folder check");
      return host.sent("jobs.start").at(-1)!;
    };
    const raw = await saved("Ctrl+E", "raw");
    assert.equal(raw.destination, join(folder, "source-one_00.02.000.png"));
    assert.equal(raw.sampleTime, 2, "the export takes the frame the name states");
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
    const screen = await saved("Ctrl+Alt+E", "processed");
    assert.ok(Number(screen.sampleTime) > 2);
    assert.equal(screen.destination, join(folder, frameName("/contract/source-one.mp4", Number(screen.sampleTime), ".png")));
  } finally {
    await stop();
    await cleanup();
  }
});

test("while playing, Save As offers the name of the frame it captures; a burst of the current frame names each file by its own time", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const reads = playing(host);
    let offered = "";
    host.handle("dialogs.open", (p) => {
      offered = String(p.fileName);
      return { cancelled: false, paths: [join(folder, offered)] };
    });
    await host.command("capture.saveAs").run();
    await until(() => host.sent("jobs.start").length === 1);
    const time = Number(host.sent("snapshot.capture")[0]!.time);
    assert.equal(offered, frameName("/contract/source-one.mp4", time, ".png"), "the offered name and the frozen frame agree");
    host.reset("dialogs.open");
    page(host).emit("burst-edited", "3", "0.5");
    const press = reads.length;
    await host.press("Ctrl+G");
    await until(() => host.sent("jobs.start").length === 4);
    const frozen = host.sent("snapshot.capture")[1]!;
    assert.equal(frozen.time, reads[press], "the frame at the press of Ctrl+G, though the clock moved on");
    const starts = host.sent("jobs.start").slice(1);
    const first = Number(starts[0]!.sampleTime);
    assert.deepEqual(
      starts.map((s) => basename(String(s.destination))),
      [0, 1, 2].map((i) => `00${i + 1}-${(first + i * 0.5).toFixed(3)}s.png`),
      "each file is named by its own sample time",
    );
  } finally {
    await stop();
    await cleanup();
  }
});

test("the settings page's choices are kept in the data directory and restored on the next start", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-")); // outlives each test host, like the player's
  const host = new TestHost();
  ready(host);
  const first = await start(host, "zh-Hans", data);
  const { folder, cleanup } = await withFolder(host);
  try {
    const settings = page(host);
    settings.emit("template-edited", "image", "{name}-{n}");
    settings.emit("burst-edited", "5", "0.2");
    settings.emit("set-burst-processed", true);
    settings.emit("set-snapshot-effects", true);
    await host.settle();
    await jsonFile(join(data, "settings.json"), (v) =>
      v.folder === folder && (v.templates as { image: string }).image === "{name}-{n}" && (v.burst as { count: number }).count === 5 && v.snapshotEffects === true);
    await first.stop();
    const restart = new TestHost();
    ready(restart);
    const second = await startPlugin(activate, { directory, host: restart, dataDirectory: data });
    await restart.settle();
    const again = () => restart.settingsPage("snapshot").props;
    await eventually(() => again().folder, folder);
    await eventually(() => [again()["template-image"], again()["burst-count"], again()["burst-step"], again()["burst-processed"], again()["snapshot-effects"]], ["{name}-{n}", "5", "0.2", true, true]);
    await second.stop();
    await rm(folder, { recursive: true, force: true }); // a removed folder falls back to the default
    const third = await startPlugin(activate, { directory, host: new TestHost(), dataDirectory: data });
    await third.host.settle();
    assert.notEqual(third.host.settingsPage("snapshot").props.folder, folder);
    await third.stop();
  } finally {
    await cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

test("the burst fields keep what the user types; only valid values are used and kept", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-"));
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host, "zh-Hans", data);
  try {
    const settings = page(host);
    settings.emit("burst-edited", "1", "1");
    await host.settle();
    await eventually(() => props(host)["burst-count"], "1", "the field keeps the text on the way to 15");
    settings.emit("burst-edited", "15", "");
    await host.settle();
    await eventually(() => [props(host)["burst-count"], props(host)["burst-step"]], ["15", ""]);
    await jsonFile(join(data, "settings.json"), (v) => (v.burst as { count: number; step: number }).count === 15 && (v.burst as { step: number }).step === 1);
  } finally {
    await stop();
    await rm(data, { recursive: true, force: true });
  }
});

const SNAPSHOTS = ["capture.save", "capture.saveAs", "capture.copy", "capture.burst", "capture.saveScreen", "capture.saveScreenAs", "capture.copyScreen"];

test("lazy activation: the Snapshot items are player conditions; nothing keeps the plugin running and it reads no player state", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  try {
    await host.settle();
    assert.deepEqual(host.keepsRunning, []);
    assert.equal(host.listening.has("source.state"), false);
    for (const id of SNAPSHOTS)
      assert.deepEqual(host.command(id).when, { enabled: { all: [{ can: "snapshot" }, { phase: ["ready", "ended"] }, { not: { kind: "stream" } }] } }, id);
    assert.deepEqual(SNAPSHOTS.map((id) => host.command(id).state.enabled), SNAPSHOTS.map(() => false));
    assert.equal(host.translate(host.command("capture.save").state.reason!), "没有可截取的画面");
    assert.equal(await host.press("Ctrl+E"), "reason");
    ready(host);
    await host.settle();
    assert.deepEqual(SNAPSHOTS.map((id) => host.command(id).state.enabled), SNAPSHOTS.map(() => true));
    assert.deepEqual(host.keepsRunning, []);
    assert.equal(host.listening.has("source.state"), false, "the plugin does not follow playback");
  } finally {
    await stop();
  }
});

test("Save As asks for a file; a source change while it is open captures nothing", async () => {
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

test("one running and four queued; the sixth is refused; queued input keeps its source and never shares a name; a failure is said", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const queued = () => host.osd.filter((line) => line === "截图已排队").length;
    for (let i = 1; i <= 5; i++) {
      await host.press("Ctrl+E");
      await until(() => host.sent("snapshot.capture").length === i && (i === 1 ? host.sent("jobs.start").length === 1 : queued() === i - 1));
    }
    assert.equal(host.sent("jobs.start").length, 1, "only one accepted task starts its worker");
    await host.press("Ctrl+E");
    await until(() => host.osd.at(-1) === "截图失败：还有五项截图正在保存，请稍候。");
    assert.equal(host.sent("snapshot.capture").length, 5, "the sixth entry freezes nothing");
    host.openElsewhere("/contract/source-two.mp4");
    await setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: join(folder, "source-one_00.02.000.png") } });
    await until(() => host.sent("jobs.start").length === 2);
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 2, "the second frozen input");
    assert.equal((host.snapshots.get(2) as { sourceStamp: { generation: number } }).sourceStamp.generation, 1, "queued input not refreshed");
    assert.equal(host.snapshots.has(1), false, "a completed task releases its input");
    assert.equal(host.sent("jobs.start")[1]!.destination, join(folder, "source-one_00.02.000 (1).png"), "queued frames never share a name");
    await setJob(2, { status: "failed", outcome: { error: { code: "space", message: "disk full" } } });
    await until(() => host.osd.at(-1) === "截图失败：磁盘空间不足");
    await until(() => host.sent("jobs.start").length === 3);
    assert.equal(host.snapshots.has(2), false, "a failed task releases its input");
  } finally {
    await stop();
    await cleanup();
  }
});

test("burst: the folder from the template under the capture folder, one snapshot, sample times t0 + i × interval; past the end it is refused", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    page(host).emit("burst-edited", "3", "0.5");
    await host.press("Ctrl+G");
    await until(() => host.osd.at(-1)?.startsWith("连拍已保存") ?? false);
    assert.equal(host.sent("snapshot.capture").length, 1);
    const starts = host.sent("jobs.start");
    assert.deepEqual(starts.map((s) => s.sampleTime), [2, 2.5, 3]);
    const [child] = await readdir(folder);
    assert.match(child!, /^Capture-\d{4}-\d\d-\d\d \d\d\.\d\d\.\d\d$/, "Capture-{date} {time}");
    assert.deepEqual(starts.map((s) => s.destination), ["001-2.000s.png", "002-2.500s.png", "003-3.000s.png"].map((name) => join(folder, child!, name)));
    host.setSource({ position: 29 });
    await host.press("Ctrl+G");
    await until(() => host.osd.at(-1) === "连拍失败：连拍超出媒体时长，请减少张数或间隔。");
    assert.equal((await readdir(folder)).length, 1, "the refused burst's folder is removed");
  } finally {
    await stop();
    await cleanup();
  }
});

test("file-name templates: edits rename the next capture; a chip adds a variable; Default name restores it", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const settings = page(host);
    settings.emit("template-edited", "image", "shot {name}");
    settings.emit("insert-variable", "image", "{pos}");
    await host.settle();
    await eventually(() => props(host)["template-image"], "shot {name}{pos}");
    await eventually(() => (props(host).previews as { image: string }).image, "shot video05.09.412.png", "the preview names a sample file");
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 1);
    assert.equal(host.sent("jobs.start")[0]!.destination, join(folder, "shot source-one00.02.000.png"));
    settings.emit("template-edited", "image", "");
    await host.settle();
    await eventually(() => props(host)["template-image"], "", "an empty template stays empty in the field while typing");
    settings.emit("reset-template", "image");
    await host.settle();
    await eventually(() => props(host)["template-image"], "{name}_{pos}");
    settings.emit("template-edited", "clip", "{name}");
    await host.settle();
    assert.deepEqual(Object.keys(props(host).previews as object), ["image", "burst"], "no clip template");
  } finally {
    await stop();
    await cleanup();
  }
});

test("Save and Copy take the frame as mpv does: the subtitles shown, the effects when Settings says so; Video Frame Only is the source frame", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { cleanup } = await withFolder(host);
  const saved = async (run: () => Promise<unknown>) => {
    const before = host.sent("jobs.start").length;
    await run();
    await until(() => host.sent("jobs.start").length === before + 1);
    const capture = host.sent("snapshot.capture").at(-1)!;
    return [capture.mode, capture.mode === "processed" ? capture.includeSubtitles !== false : false];
  };
  try {
    assert.deepEqual(await saved(() => host.press("Ctrl+E")), ["raw", false], "no subtitles shown: the source frame");
    host.subtitles.tracks.push({ target: { kind: "embedded", id: 1 }, title: "English", language: "eng", styleSupported: true, styleReason: null } as never);
    host.subtitles.selection.target = { kind: "embedded", id: 1 } as never;
    assert.deepEqual(await saved(() => host.press("Ctrl+E")), ["processed", true], "the subtitles shown are in the frame");
    assert.deepEqual(await saved(() => host.command("capture.saveVideo").run()), ["raw", false], "Save Video Frame Only");
    // Subtitles the player cannot draw into a frame (only plain text can be): the frame without them, not a failure.
    host.handle("snapshot.capture", async (p) => {
      if (p.mode === "processed") throw new HostError("unsupported", "styled subtitles cannot be flattened");
      // The retry without subtitles: the test host's own answer.
      host.reset("snapshot.capture");
      return await host.request("snapshot.capture", p);
    });
    assert.deepEqual(await saved(() => host.press("Ctrl+E")), ["raw", false]);
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") === true && host.osd.at(-1)!.endsWith("（不含字幕：字幕无法画入帧中）"));
    // Copy says so too.
    host.handle("snapshot.capture", async (p) => {
      if (p.mode === "processed") throw new HostError("unsupported", "styled subtitles cannot be flattened");
      host.reset("snapshot.capture");
      return await host.request("snapshot.capture", p);
    });
    host.handle("clipboard.writeImage", () => {}); // the test host's exports write no real PNG
    await host.press("Ctrl+C");
    await until(() => host.osd.at(-1) === "已复制画面（不含字幕：字幕无法画入帧中）");
    host.reset("snapshot.capture");
    // Settings: Include video effects, no subtitles shown.
    host.subtitles.selection.target = null;
    page(host).emit("set-snapshot-effects", true);
    await host.settle();
    assert.deepEqual(await saved(() => host.press("Ctrl+E")), ["processed", false], "the effects without subtitles");
    await eventually(() => props(host)["snapshot-effects"], true);
  } finally {
    await stop();
    await cleanup();
  }
});
