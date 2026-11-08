import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";

import type { Locale } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { Tasks } from "../tasks.ts";
import type { Task } from "../tasks.ts";
import { frameName, REASON } from "../model.ts";
import { timelineWindow } from "../clip.ts";

/** The end of the small timeline's window around a range (the keyframe probe reads that far). */
const timelineEnd = (start: number, end: number) => timelineWindow(start, end, 600).to;
import { eventually, words } from "./fakes.ts";

/** A job as the test host records it. */
type MediaJob = NonNullable<ReturnType<TestHost["jobs"]["get"]>>;
type Row = { id: string; kind: string; name: string; state: string; reason: string; raw: string; retryable: boolean; cancellable: boolean; openable: boolean; copyable: boolean };

const directory = new URL("..", import.meta.url);
/** Most tests check what a Chinese user sees (the texts from before the English sources); English is tested on its own. */
const start = (host = new TestHost(), locale: Locale = "zh-Hans", dataDirectory?: string) => startPlugin(activate, { directory, host, locale, dataDirectory });
/** The tab's bridge values (Slint names). */
const tab = (host: TestHost) => host.mounted().props;
const rows = (host: TestHost) => tab(host).results as Row[];
/** A status line of the tab as shown. */
const statusOf = (host: TestHost, name: "clip-status" | "burst-status" | "results-status") => words(host).status(tab(host)[name] as never);
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
const TRACKS = [
  { trackId: 7, selected: true, external: false, streamIndex: 1, codec: "aac", title: null, language: "en" },
  { trackId: 8, selected: false, external: false, streamIndex: 2, codec: "opus", title: null, language: null },
];

test("Snapshot ▸ keeps five items; every other command is in the command palette with its keys; all ids stay", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    const menu = [...host.commands.values()].filter((c) => c.declaration.path?.length).map(({ declaration: d, display }) => [display.path.join(" > "), d.order, d.group, d.bindings.join(",")]);
    assert.deepEqual(menu, [
      // Menu sweep (B5): Snapshot ▸ is a top-level submenu in the domain group.
      ["截图 > 保存当前画面", 420, "domain", "Ctrl+E"],
      ["截图 > 复制当前画面", 422, "domain", "Ctrl+C"],
      ["截图 > 连拍…", 426, "domain", "Ctrl+G"],
      ["截图 > 导出片段…", 428, "domain", ""],
      ["截图 > 截取…", 440, "capture-tab", "Alt+S"],
    ]);
    const palette = [...host.commands.entries()].filter(([, c]) => !c.declaration.path?.length).map(([id, { declaration: d }]) => [id, d.bindings.join(","), d.home ?? ""]);
    assert.deepEqual(palette, [
      ["capture.clipExport", "Ctrl+Shift+E", "capture.panel"],
      ["capture.saveAs", "", "capture.panel"],
      ["capture.saveScreen", "Ctrl+Alt+E", "capture.panel"],
      ["capture.saveScreenAs", "", "capture.panel"],
      ["capture.copyScreen", "Ctrl+Alt+C", "capture.panel"],
      ["capture.saveVideo", "", "capture.panel"],
      ["capture.copyVideo", "", "capture.panel"],
      ["capture.tasks", "", ""],
      ["capture.format.png", "", "capture.settings"],
      ["capture.format.jpeg", "", "capture.settings"],
      ["capture.format.bmp", "", "capture.settings"],
      ["capture.screenSubtitles", "", "capture.panel"],
      ["capture.settings", "", ""],
      ["capture.folder", "", "capture.settings"],
      ["capture.openFolder", "", ""],
      ["capture.revealLast", "", "capture.tasks"],
      ["capture.copyLast", "", "capture.tasks"],
      ["capture.clipStart", "", "capture.panel"],
      ["capture.clipEnd", "", "capture.panel"],
      ["capture.last30", "Ctrl+Shift+R", "capture.panel"],
    ]);
    // Every id of capture 1.0.11 is still there, so keys the user changed keep working.
    for (const id of ["capture.save", "capture.saveAs", "capture.copy", "capture.saveScreen", "capture.saveScreenAs", "capture.copyScreen", "capture.burst", "capture.clip", "capture.tasks", "capture.format.png", "capture.format.jpeg", "capture.format.bmp", "capture.screenSubtitles", "capture.settings", "capture.folder", "capture.openFolder"])
      assert.ok(host.commands.has(id), id);
    assert.deepEqual(host.conflicts, []);
    assert.equal(host.sent("ui.open").length, 0, "no windows any more");
    assert.equal(host.mounted().slot, "sidebar");
    assert.equal(host.mounted().component, "CaptureTab");
    assert.equal(host.layout.sidebar?.panels?.[0]?.title, "截取", "a short title that fits the default sidebar");
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
      assert.equal(host.commands.size, 25);
      for (const [id, command] of host.commands) {
        assert.ok(command.declaration.category, `${id}: declares a category`);
        assert.ok(command.display.category?.trim(), `${id}: shows a category in ${locale}`);
        assert.ok((command.display.path.at(-1) ?? command.display.title ?? "").trim(), `${id}: has a title`);
        const { home } = command.declaration;
        if (home !== undefined) assert.ok(host.commands.has(home), `${id}: home ${home} is a command of this plugin`);
      }
      if (locale === "en") {
        assert.equal(host.command("capture.save").display.category, "Snapshot");
        for (const id of ["capture.clip", "capture.clipExport", "capture.tasks", "capture.last30", "capture.clipStart", "capture.clipEnd"]) assert.equal(host.command(id).display.category, "Export", id);
        for (const id of ["capture.panel", "capture.settings", "capture.folder", "capture.openFolder"]) assert.equal(host.command(id).display.category, "Snapshot", id);
      }
    } finally {
      await stop();
    }
  }
});

test("English by default: menus, the tab's title, OSD and the clip's dialog are the English source texts", async () => {
  const host = new TestHost();
  ready(host);
  host.setSource({ audioTracks: TRACKS });
  const { stop } = await start(host, "en");
  try {
    // Menu sweep (B5): Snapshot ▸ is a top-level submenu in the domain group.
    assert.deepEqual([host.command("capture.save").declaration.path, host.command("capture.save").declaration.group], [["Snapshot", "Save Current Frame"], "domain"]);
    assert.deepEqual(host.command("capture.panel").display.path, ["Snapshot", "Capture…"]);
    assert.equal(host.command("capture.last30").display.title, "Export Last 30 Seconds");
    await host.command("capture.format.jpeg").run();
    await host.settle();
    assert.equal(host.osd.at(-1), "Snapshot format: JPEG");
    await eventually(() => tab(host)["audio-labels"], ["No Audio", "Current Track", "Track 7 · en", "Track 8"]);
    host.dialogAnswers.push(null);
    host.mounted().emit("export-as", "1", "4");
    await until(() => statusOf(host, "clip-status") === "No save location chosen.");
    assert.equal(host.sent("dialogs.open").at(-1)!.title, "Export Clip");
    assert.equal(host.sent("dialogs.open").at(-1)!.fileName, "source-one_00.01.000-00.04.000.mp4", "{name}_{start}-{end}");
    host.mounted().emit("export-clip", "5", "1");
    await until(() => statusOf(host, "clip-status") === REASON.endBeforeStart);
    await eventually(() => tab(host)["clip-status"], { kind: "error", reason: "endBeforeStart", raw: "", count: 0, name: "" }, "data, not words");
  } finally {
    await stop();
  }
});

test("the reasons in model.ts and Words.reason in ui/parts.slint are the same texts; all are translated", async () => {
  const slint = await readFile(new URL("../ui/parts.slint", import.meta.url), "utf8");
  const said = Object.fromEntries([...slint.matchAll(/code == "(\w+)"\) \{ return @tr\("([^"]+)"\)/g)].map((m) => [m[1], m[2]]));
  assert.deepEqual(said, { ...REASON });
  for (const locale of ["zh-Hans", "de", "es", "fr", "ja", "ko"]) {
    const po = await readFile(new URL(`../locales/${locale}.po`, import.meta.url), "utf8");
    assert.doesNotMatch(po.replace(/^msgid ""\nmsgstr ""\n/, ""), /msgstr ""\n\n/, `${locale}: every text translated`);
  }
  const { host, stop } = await start();
  const { reason, progress, status } = words(host);
  assert.equal(reason("space", ""), "磁盘空间不足");
  assert.equal(reason("", "a message of the player"), "a message of the player");
  assert.equal(progress({ status: "running", reason: "", raw: "", done: 3, total: 10 }), "导出中 · 3/10");
  assert.equal(status({ kind: "clip-failed", reason: "space", raw: "", count: 0, name: "" }), "导出失败：磁盘空间不足");
  assert.equal(status({ kind: "started", reason: "", raw: "", count: 4, name: "" }), "已开始连拍 4 张");
  await stop();
});

test("the tab's Slint: a ListView for Results, library widgets, key caps, Theme tokens, every label in @tr", async () => {
  const slint = await readFile(new URL("../ui/tab.slint", import.meta.url), "utf8");
  const parts = await readFile(new URL("../ui/parts.slint", import.meta.url), "utf8");
  assert.match(slint, /ListView \{\s*vertical-stretch: 1;\s*for row in CaptureBridge\.results: ResultItem/);
  // Real pages (user ruling 2026-11-07): four persistent pages, one shown, switched with `visible`.
  for (const page of [0, 1, 2, 3]) assert.equal(slint.split(`visible: CaptureBridge.page == ${page};`).length - 1, 1, `page ${page}`);
  assert.match(slint, /current-index <=> CaptureBridge\.page;/);
  // No wrapped text: a wrapped line is laid out one line tall and overlapped the rows below it (GUI check).
  assert.doesNotMatch(slint, /wrap:\s*word-wrap/);
  assert.doesNotMatch(slint, /jump-|viewport-y =/, "no anchors scrolled to");
  assert.doesNotMatch(slint, /ComboBox|CheckBox|StandardListView/);
  assert.match(slint, /import \{ Button, Check, Choice, KeyHint, RadioGroup, Slider, Tabs, ToolTipArea, ToolTipKey \} from "@vestra\/widgets\.slint";/);
  assert.match(slint, /KeyHint \{ text: root\.key; \}/);
  assert.match(slint, /tip-keys: \[\{ key: CaptureBridge\.keys\.save, text: "" \}\]/);
  for (const [name, text] of [["tab", slint], ["parts", parts]] as const) {
    assert.doesNotMatch(text, /#[0-9a-f]{3,8}\b/i, `${name}: colours come from Theme`);
    assert.doesNotMatch(text, /font-size:\s*\d/, `${name}: font sizes come from Theme`);
    assert.doesNotMatch(text, /(text|title|tip|label|detail|summary):\s*"[^"]*[a-z]{3,}[^"]*"/, `${name}: every label in @tr`);
  }
  // A `for` over bound data only inside the ListView (the variable chips iterate over literal lists).
  const loops = [...slint.matchAll(/for \w+ in ([\w.-]+|\[[^\]]*\])/g)].map((m) => m[1]);
  assert.deepEqual(loops.filter((what) => !what!.startsWith("[")), ["CaptureBridge.results"]);
  // The bridge's callbacks are the ones the plugin handles: each emitted callback reaches main.ts.
  const callbacks = [...slint.matchAll(/^\s+callback ([\w-]+)\(/gm)].map((m) => m[1]!);
  const main = await readFile(new URL("../main.ts", import.meta.url), "utf8");
  const camel = (name: string) => name.replace(/-(\w)/g, (_, c: string) => c.toUpperCase());
  const handled = new Set([...main.matchAll(/\b(?:on|onRow)\("(\w+)"/g)].map((m) => m[1]));
  const bridge = slint.slice(slint.indexOf("export global CaptureBridge"), slint.indexOf("// Glyphs of this tab"));
  const declared = [...bridge.matchAll(/callback ([\w-]+)\(/g)].map((m) => camel(m[1]!));
  assert.deepEqual(declared.filter((name) => !handled.has(name)), [], "every bridge callback has a handler");
  assert.deepEqual([...handled].filter((name) => !declared.includes(name!)), [], "every handler has a bridge callback");
  assert.ok(callbacks.length >= declared.length);
});

test("Ctrl+E saves the raw current frame into the capture folder with no dialog and an OSD; the result is listed", async () => {
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
    await until(() => rows(host).length === 1 && rows(host)[0]!.state !== "running");
    const [row] = rows(host);
    assert.deepEqual([row!.kind, row!.name, row!.id.startsWith("r"), row!.openable], ["image", basename(outcome.destination), true, true], "the file the host confirmed");
    await eventually(() => tab(host).unseen, 1, "the Results sub-tab counts it while another page shows");
    host.mounted().emit("set-page", 2);
    await host.settle();
    await eventually(() => tab(host).unseen, 0);
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
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
    // The tab's buttons do the same as the keys.
    host.reset("source.state");
    host.setSource({ position: 7.25 });
    host.mounted().emit("save");
    await until(() => host.sent("jobs.start").at(-1)!.destination === join(folder, "source-one_00.07.250.png"));
    assert.equal(host.sent("snapshot.capture").at(-1)!.time, 7.25);
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
    await host.press("Ctrl+G");
    const press = reads.length;
    host.mounted().emit("start-burst", "0", "3", "0.5", true, false, false);
    await until(() => host.sent("jobs.start").length === 4);
    const frozen = host.sent("snapshot.capture")[1]!;
    assert.equal(frozen.time, reads[press], "the frame at the press of Start, though the clock moved on");
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

test("the chosen capture folder and the tab's choices are kept in the data directory and restored on the next start", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-")); // outlives each test host, like the player's
  const host = new TestHost();
  ready(host);
  const first = await start(host, "zh-Hans", data);
  const { folder, cleanup } = await withFolder(host);
  try {
    host.mounted().emit("set-burst-open", true);
    host.mounted().emit("template-edited", "image", "{name}-{n}");
    host.mounted().emit("set-last-seconds", 2);
    await host.settle();
    await jsonFile(join(data, "settings.json"), (v) => v.folder === folder && v.burstOpen === true && v.lastSeconds === 60 && (v.templates as { image: string }).image === "{name}-{n}");
    await first.stop();
    const restart = new TestHost();
    ready(restart);
    const second = await startPlugin(activate, { directory, host: restart, dataDirectory: data });
    await restart.settle();
    await eventually(() => tab(restart).folder, folder);
    await eventually(() => tab(restart)["burst-open"], true);
    await eventually(() => tab(restart)["template-image"], "{name}-{n}");
    await eventually(() => tab(restart)["last-index"], 2);
    assert.equal(restart.command("capture.last30").display.title, "Export Last 60 Seconds");
    await second.stop();
    await rm(folder, { recursive: true, force: true }); // a removed folder falls back to the default
    const third = await startPlugin(activate, { directory, host: new TestHost(), dataDirectory: data });
    await third.host.settle();
    assert.notEqual(tab(third.host).folder, folder);
    await third.stop();
  } finally {
    await cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

const SNAPSHOTS = ["capture.save", "capture.saveAs", "capture.copy", "capture.saveScreen", "capture.saveScreenAs", "capture.copyScreen"];

test("lazy activation: the Snapshot items are player conditions; a background tab keeps nothing running and reads no player state", async () => {
  const host = new TestHost();
  host.mountOther("sidebar", "vestra.playlist", { title: "Playlist" }); // the playlist's tab is first and current, as in the player
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
    assert.equal(host.listening.has("source.state"), false, "the background tab does not follow playback");
    await eventually(() => tab(host).mode, undefined, "nothing of the player's state was sent");
    // The user shows the tab: it reads the player's state now, and follows it.
    host.selectTab("vestra.capture");
    await host.settle();
    await eventually(() => tab(host).mode, "local");
    await eventually(() => tab(host)["can-clip"], true);
    assert.equal(host.listening.has("source.state"), true);
    host.setSource({ phase: "loading" });
    await host.settle();
    await eventually(() => tab(host).mode, "none", "followed while it shows");
    host.selectTab("vestra.playlist");
    // The layout is read after the event; the binding then runs again without reading the player's state.
    await until(() => !host.listening.has("source.state"));
    assert.deepEqual(host.keepsRunning, []);
  } finally {
    await stop();
  }
});

test("tab states: no media, a plugin's stream, a network source, a local file", async () => {
  const host = new TestHost();
  const { stop } = await start(host, "en");
  try {
    await host.settle();
    await eventually(() => [tab(host).mode, tab(host)["can-capture"], tab(host)["can-clip"]], ["none", false, false]);
    host.openElsewhere("https://example.test/live.m3u8");
    host.setSource({ position: 2, duration: 30 });
    await host.settle();
    await eventually(() => [tab(host).mode, tab(host)["can-capture"], tab(host)["can-clip"], tab(host)["can-last"]], ["network", true, false, false]);
    host.setSource({ kind: "producer", phase: "ready" });
    await host.settle();
    await eventually(() => [tab(host).mode, tab(host)["can-capture"]], ["stream", false]);
    ready(host);
    host.setSource({ kind: "media" });
    await host.settle();
    await eventually(() => [tab(host).mode, tab(host)["can-capture"], tab(host)["can-clip"], tab(host)["can-last"]], ["local", true, true, true]);
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

test("clip, Precise: Export saves into the folder with the template's name and the tab's encoder settings (R6); progress (R4), the range written (R2), OSD buttons (R5)", async () => {
  const host = new TestHost();
  ready(host);
  host.setSource({ audioTracks: TRACKS });
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const panel = host.mounted();
    await eventually(() => tab(host)["audio-labels"], ["无声音", "当前音轨", "音轨 7 · en", "音轨 8"]);
    panel.emit("set-method", 1);
    panel.emit("set-preset", 2);
    panel.emit("set-audio", 2);
    panel.emit("set-processed", true);
    panel.emit("clip-edited", "0:01", "0:04.5");
    await host.settle();
    await eventually(() => tab(host)["clip-name"], "source-one_00.01.000-00.04.500.mp4", "the name the Export button gives");
    await eventually(() => tab(host)["field-error"], "");
    await eventually(() => tab(host).estimate && (tab(host).estimate as { length: string }).length, "3.5");
    await eventually(() => (tab(host).note as { method: string }).method, "precise");
    await eventually(() => tab(host)["preset-index"], 2, "Small file");
    panel.emit("export-clip", "0:01", "0:04.5");
    await until(() => host.sent("jobs.start").length === 1 && statusOf(host, "clip-status") === "已开始导出，可在结果中查看。");
    assert.equal(host.sent("dialogs.open").length, 1, "only the folder dialog: Export asks for nothing");
    const capture = host.sent("snapshot.capture")[0]!;
    assert.deepEqual(capture.expected, host.source.stamp);
    assert.deepEqual([capture.mode, capture.time, capture.includeSubtitles], ["processed", 1, false]);
    assert.deepEqual(capture.audio, { kind: "track", trackId: 7 });
    const job = host.sent("jobs.start")[0]!;
    assert.deepEqual([job.kind, job.end, job.destination], ["processedMp4", 4.5, join(folder, "source-one_00.01.000-00.04.500.mp4")]);
    // Small file; the test host's source plays, so one thread.
    assert.deepEqual(job.encoding, { crf: 26, speed: "balanced", maxHeight: 1080, audioBitrate: 128, threads: "auto" });
    // The progress card and the row while it runs.
    await until(() => (tab(host).card as { shown: boolean }).shown);
    await eventually(() => (tab(host).card as { name: string }).name, "source-one_00.01.000-00.04.500.mp4");
    await eventually(() => (tab(host).card as { percent: number }).percent, -1, "no percentage before the first progress");
    await setJob(1, { status: "running", progress: { done: 1.75, total: 3.5 } } as never);
    await until(() => (tab(host).card as { percent: number }).percent === 50 && rows(host)[0]?.state === "running" && (rows(host)[0] as unknown as { percent: number }).percent === 50);
    const destination = join(folder, "source-one_00.01.000-00.04.500.mp4");
    await writeFile(destination, "mp4 bytes");
    await setJob(1, { status: "completed", publicationStarted: true, progress: null, outcome: { destination, start: 1, end: 4.52 } } as never);
    await until(() => host.osd.at(-1) === `片段已导出为 ${destination}` && statusOf(host, "clip-status") === "已导出 source-one_00.01.000-00.04.500.mp4");
    await until(() => rows(host).length === 1 && rows(host)[0]!.state === "completed");
    await eventually(() => (tab(host).card as { shown: boolean }).shown, false);
    await eventually(() => [rows(host)[0]!.kind, (rows(host)[0] as unknown as { method: string; badge: string }).method, (rows(host)[0] as unknown as { badge: string }).badge], ["clip", "precise", "3.5"]);
    await eventually(() => (rows(host)[0] as unknown as { range: string }).range, "00:01.000–00:04.520", "R2: the range the file holds");
    // R5: the message's buttons, and what they do.
    assert.deepEqual(host.osdActions.at(-1), [{ title: "在文件夹中显示", command: "capture.revealLast" }, { title: "在页签中显示", command: "capture.tasks" }]);
    await host.pressOsdAction(0);
    await until(() => host.system.revealed.length === 1);
    assert.deepEqual(host.system.revealed, [destination]);
    host.mounted().emit("reveal-result", rows(host)[0]!.id);
    await until(() => host.system.revealed.length === 2);
  } finally {
    await stop();
    await cleanup();
  }
});

test("Export Clip… only opens the Clip section; Export Clip Range (Ctrl+Shift+E) exports, or opens it when there is no range", async () => {
  const host = new TestHost();
  ready(host);
  host.mountOther("sidebar", "vestra.playlist", { title: "Playlist" });
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { cleanup } = await withFolder(host);
  try {
    await host.settle();
    await eventually(() => (tab(host).keys as { clip: string }).clip, "⇧⌘E", "the Export button shows Export Clip Range's key");
    // No range yet: the key opens the Clip section with the position → +10 s, and exports nothing.
    await host.press("Ctrl+Shift+E");
    await host.settle();
    assert.equal(host.layout.sidebar?.owner, 1, "the tab is in front");
    await eventually(() => [tab(host)["clip-start"], tab(host)["clip-end"]], ["00:02.000", "00:12.000"]);
    await eventually(() => tab(host).page, 1, "the Clip page");
    assert.equal(host.sent("jobs.start").length, 0);
    // Export Clip… never exports, also while the tab shows.
    await host.command("capture.clip").run();
    await host.settle();
    assert.equal(host.sent("jobs.start").length, 0);
    await eventually(() => tab(host).page, 1);
    // With a range, the key exports it.
    await host.press("Ctrl+Shift+E");
    await until(() => host.sent("jobs.start").length === 1);
    assert.equal(host.sent("jobs.start")[0]!.end, 12);
    await setJob(1, { status: "cancelled", outcome: { cancelled: true } });
    // A range the user typed: Export Clip… keeps it; a bad one is said on screen by the key.
    host.mounted().emit("clip-edited", "0:05", "0:03");
    const sets = host.sent("ui.setMount").length;
    await host.command("capture.clip").run();
    await host.settle();
    await eventually(() => [tab(host)["clip-start"], tab(host)["clip-end"]], ["00:02.000", "00:12.000"], "nothing written over the typed fields");
    assert.ok(host.sent("ui.setMount").length > sets && host.sent("ui.setMount").slice(sets).every((p) => !/clip-?start/i.test(JSON.stringify(p))));
    await host.press("Ctrl+Shift+E");
    await until(() => host.osd.at(-1) === "导出失败：结束时间必须晚于开始时间。");
  } finally {
    await stop();
    await cleanup();
  }
});

test("field errors: a time that is none, an end before the start; Export is refused with the reason", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    const panel = host.mounted();
    panel.emit("clip-edited", "1:7x", "4");
    await host.settle();
    await eventually(() => [tab(host)["field-error"], tab(host)["error-field"]], ["time", "start"]);
    panel.emit("clip-edited", "0:05", "0:04");
    await host.settle();
    await eventually(() => [tab(host)["field-error"], tab(host)["error-field"]], ["endBeforeStart", "end"]);
    await eventually(() => tab(host)["clip-name"], "");
    panel.emit("export-clip", "0:05", "0:04");
    await until(() => statusOf(host, "clip-status") === "结束时间必须晚于开始时间。");
    assert.equal(host.sent("snapshot.capture").length, 0);
  } finally {
    await stop();
  }
});

test("Use A-B range copies the loop once; without a loop it says how to set one", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    await host.settle();
    await eventually(() => tab(host).ab, { shown: false, a: "", b: "" });
    host.mounted().emit("use-ab");
    await until(() => statusOf(host, "clip-status") === words(host).reason("noLoop", ""));
    host.setSource({ loop: { a: 3.5, b: 9.25, count: null } });
    await host.settle();
    await eventually(() => tab(host).ab, { shown: true, a: "00:03.500", b: "00:09.250" });
    host.mounted().emit("use-ab");
    await host.settle();
    await eventually(() => [tab(host)["clip-start"], tab(host)["clip-end"]], ["00:03.500", "00:09.250"]);
    host.setSource({ loop: { a: 1, b: 2, count: null } });
    await host.settle();
    await eventually(() => [tab(host)["clip-start"], tab(host)["clip-end"]], ["00:03.500", "00:09.250"], "a copy, not a link");
  } finally {
    await stop();
  }
});

test("Last N s (Ctrl+Shift+R): the seconds before the position into the folder; the length is adjustable and names the command", async () => {
  const host = new TestHost();
  ready(host);
  host.setSource({ position: 20 });
  const setJob = manualJobs(host);
  const { stop } = await start(host, "en");
  const folder = await mkdtemp(join(tmpdir(), "capture-last-"));
  try {
    host.dialogAnswers.push([folder]);
    await host.command("capture.folder").run();
    await until(() => host.osd.at(-1) === `Snapshot folder: ${folder}`);
    host.rebind("capture.last30", ["Ctrl+Alt+L"]);
    await assert.rejects(host.press("Ctrl+Shift+R"), /nothing is bound/, "the user's key replaced the default");
    await host.press("Ctrl+Alt+L");
    await until(() => host.sent("jobs.start").length === 1);
    assert.equal(host.sent("snapshot.capture")[0]!.time, 0, "max(0, 20 − 30)");
    assert.equal(host.sent("jobs.start")[0]!.end, 20);
    assert.equal(host.osd.at(-1), "Exporting the last 30 seconds");
    await eventually(() => [tab(host)["clip-start"], tab(host)["clip-end"]], ["00:00.000", "00:20.000"], "empty fields show what was exported");
    await setJob(1, { status: "cancelled", outcome: { cancelled: true } });
    // A range the user typed is kept; Last N s exports its own range all the same.
    host.mounted().emit("clip-edited", "0:03", "0:07");
    host.setSource({ position: 25 });
    host.mounted().emit("last");
    await until(() => host.sent("jobs.start").length === 2);
    assert.deepEqual([host.sent("snapshot.capture")[1]!.time, host.sent("jobs.start")[1]!.end], [0, 25]);
    await eventually(() => tab(host)["clip-end"], "00:20.000", "the typed fields were not written over (00:25.000 would be)");
    host.setSource({ position: 20 });
    await setJob(2, { status: "cancelled", outcome: { cancelled: true } });
    host.mounted().emit("clip-edited", "", "");
    host.mounted().emit("set-last-seconds", 0);
    await host.settle();
    assert.equal(host.command("capture.last30").display.title, "Export Last 10 Seconds");
    assert.deepEqual(host.command("capture.last30").declaration.bindings, ["Ctrl+Shift+R"]);
    await until(() => host.sent("source.state").length > 0);
    host.mounted().emit("last");
    await until(() => host.sent("jobs.start").length === 3);
    assert.equal(host.sent("snapshot.capture")[2]!.time, 10);
    // Network sources and too little played: refused with a reason.
    host.setSource({ position: 0.05 });
    host.mounted().emit("last");
    await until(() => host.osd.at(-1) === `Export failed: ${REASON.tooEarly}`);
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("the clip guard is taken before the first await; a rejected source releases it", async () => {
  const { host, stop } = await start();
  const panel = host.mounted();
  const reads = host.sent("source.state").length;
  panel.emit("export-clip", "0", "1");
  panel.emit("export-clip", "0", "1");
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 1, "duplicate entry excluded before the source read");
  assert.equal(statusOf(host, "clip-status"), "当前画面无法导出，请先打开本地视频文件。");
  await eventually(() => tab(host)["clip-busy"], false);
  panel.emit("export-clip", "0", "1");
  await host.settle();
  assert.equal(host.sent("source.state").length - reads, 2, "guard released after rejection");
  assert.equal(host.sent("dialogs.open").length, 0);
  await stop();
});

test("one running and four queued; the sixth is refused; queued input keeps its source; cancel from Results", async () => {
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
    await host.settle();
    await eventually(() => tab(host)["queue-full"], true);
    await eventually(() => (tab(host).card as { queued: number }).queued, 4);
    await host.press("Ctrl+E");
    await until(() => host.osd.at(-1) === "截图失败：已有一个导出在进行、四个在排队，请稍候。");
    assert.equal(host.sent("snapshot.capture").length, 5, "the sixth entry freezes nothing");
    host.mounted().emit("cancel-task", "t2");
    await until(() => rows(host).some((row) => row.id === "t2" && row.state === "cancelled"));
    assert.equal(host.sent("jobs.cancel").length, 0, "queued cancellation leaves the running worker alone");
    host.openElsewhere("/contract/source-two.mp4");
    await setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: join(folder, "source-one_00.02.000.png") } });
    await until(() => host.sent("jobs.start").length === 2);
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 3, "skips the cancelled task and uses the third frozen input");
    assert.equal((host.snapshots.get(3) as { sourceStamp: { generation: number } }).sourceStamp.generation, 1, "queued input not refreshed");
    assert.equal(host.snapshots.has(1), false, "a completed task releases its input");
    await until(() => rows(host).some((row) => row.id.startsWith("r") && row.name === "source-one_00.02.000.png"));
    await eventually(() => !!(rows(host).some((row) => row.name === "source-one_00.02.000 (2).png")), true, "queued frames never share a name");
    host.mounted().emit("cancel-task", "t3");
    await until(() => rows(host).some((row) => row.id === "t3" && row.state === "cancelled"));
    assert.equal(host.sent("jobs.cancel").length, 1, "the active job is cancelled");
    await eventually(() => rows(host).find((row) => row.id === "t3")!.retryable, true);
    host.mounted().emit("remove-result", "t3");
    await until(() => !rows(host).some((row) => row.id === "t3"));
  } finally {
    await stop();
    await cleanup();
  }
});

test("burst: the folder from the template under the capture folder, one snapshot, sample times t0 + i × spacing", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    await host.press("Ctrl+G");
    await host.settle();
    await eventually(() => tab(host)["burst-open"], true, "Ctrl+G opens the burst row");
    await eventually(() => tab(host)["burst-start"], "00:02.000", "from the position");
    await eventually(() => tab(host)["folder-shown"], folder);
    host.mounted().emit("start-burst", "1", "3", "0.5", false, false, false);
    await until(() => statusOf(host, "burst-status") === "已保存 3 张" && host.osd.at(-1)!.startsWith("连拍已保存"));
    assert.equal(host.sent("snapshot.capture").length, 1);
    const starts = host.sent("jobs.start");
    assert.deepEqual(starts.map((s) => s.sampleTime), [1, 1.5, 2]);
    const [child] = await readdir(folder);
    assert.match(child!, /^Capture-\d{4}-\d\d-\d\d \d\d\.\d\d\.\d\d$/, "Capture-{date} {time}");
    assert.deepEqual(starts.map((s) => s.destination), ["001-1.000s.png", "002-1.500s.png", "003-2.000s.png"].map((name) => join(folder, child!, name)));
    await until(() => rows(host)[0]?.kind === "burst" && rows(host)[0]!.state === "completed");
    host.mounted().emit("start-burst", "29", "3", "1", false, false, false);
    await until(() => statusOf(host, "burst-status") === "连拍超出媒体时长，请减少张数或间隔。");
    assert.equal((await readdir(folder)).length, 1, "the refused burst's folder is removed");
  } finally {
    await stop();
    await cleanup();
  }
});

test("a failed task keeps its input; retry from Results reuses it after the source changed", async () => {
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 1);
    await setJob(1, { status: "failed", outcome: { error: { code: "space", message: "disk full" } } });
    await until(() => host.osd.at(-1) === "截图失败：磁盘空间不足");
    await until(() => rows(host)[0]?.state === "failed");
    await eventually(() => [rows(host)[0]!.reason, rows(host)[0]!.retryable], ["space", true]);
    host.openElsewhere("/contract/source-two.mp4");
    host.mounted().emit("retry-task", "t1");
    await until(() => host.sent("jobs.start").length === 2 && statusOf(host, "results-status") === "已重新开始。");
    assert.equal(host.sent("snapshot.capture").length, 1, "no new snapshot");
    assert.equal(host.sent("jobs.start")[1]!.snapshotId, 1);
    assert.equal(host.sent("jobs.start")[1]!.destination, join(folder, "source-one_00.02.000 (1).png"));
    await until(() => !rows(host).some((row) => row.id === "t1"));
  } finally {
    await stop();
    await cleanup();
  }
});

test("when the tab cannot be shown, its commands say so on screen; the keys keep working", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { cleanup } = await withFolder(host);
  try {
    host.handle("layout.set", () => {
      throw new HostError("unavailable", "no interface");
    });
    await host.command("capture.tasks").run();
    await until(() => host.osd.at(-1) === "无法打开“截取”页签。");
    await host.press("Ctrl+E");
    await until(() => host.osd.at(-1)?.startsWith("帧已保存为") ?? false);
  } finally {
    await stop();
    await cleanup();
  }
});

test("typed times survive other actions; Now writes only its own field", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  try {
    host.mounted().emit("clip-edited", "5", "9");
    host.mounted().emit("clip-now", "end");
    await host.settle();
    await eventually(() => tab(host)["clip-end"], "00:02.000");
    await eventually(() => tab(host)["clip-start"], "5", "what the user typed");
    await host.command("capture.clipStart").run();
    await host.settle();
    await eventually(() => tab(host)["clip-start"], "00:02.000");
    assert.equal(host.osd.at(-1), "片段起点：00:02.000");
  } finally {
    await stop();
  }
});

test("the typed fields survive standby (session storage)", async () => {
  const host = new TestHost();
  ready(host);
  const first = await start(host);
  host.mounted().emit("clip-edited", "0:07", "0:11");
  host.mounted().emit("set-audio", 0);
  await until(() => !!host.sessionStorage.get("form"));
  const kept = host.sessionStorage.get("form");
  assert.deepEqual([(kept as { start: string }).start, (kept as { audioIndex: number }).audioIndex], ["0:07", 0]);
  await first.stop();
  const again = new TestHost();
  ready(again);
  const second = await startPlugin(activate, { directory, host: again, locale: "zh-Hans", sessionStorage: new Map([["form", kept!]]) });
  // The fields come back with `set`, the audio choice with the binding: wait for all three.
  await until(() => tab(again)["clip-start"] === "0:07" && tab(again)["clip-end"] === "0:11" && tab(again)["audio-index"] === 0);
  await second.stop();
});

test("Burst… is checked while the tab shows with its burst row open; Alt+S toggles the tab", async () => {
  const host = new TestHost();
  ready(host);
  host.mountOther("sidebar", "vestra.playlist", { title: "Playlist" });
  const { stop } = await start(host);
  try {
    const burst = host.command("capture.burst");
    assert.equal(burst.state.checked, false);
    await host.press("Ctrl+G");
    await host.settle();
    assert.equal(burst.state.checked, true);
    host.selectTab("vestra.playlist");
    await host.settle();
    assert.equal(burst.state.checked, false);
    await host.press("Alt+S");
    await host.settle();
    assert.equal(host.layout.sidebar?.owner, 1);
    assert.equal(host.command("capture.panel").state.checked, true);
    await host.press("Alt+S");
    await host.settle();
    assert.equal(host.layout.sidebar?.preferred, false, "pressed again with the tab in front: the sidebar hides");
  } finally {
    await stop();
  }
});

test("key caps follow the user's keys (commands.keys)", async () => {
  const host = new TestHost();
  host.systemInfo.os = "windows";
  ready(host);
  const { stop } = await start(host);
  try {
    await host.settle();
    await eventually(() => tab(host).keys, { save: "Ctrl+E", copy: "Ctrl+C", savescreen: "Ctrl+Alt+E", copyscreen: "Ctrl+Alt+C", burst: "Ctrl+G", clip: "Ctrl+Shift+E", panel: "Alt+S", last: "Ctrl+Shift+R" });
    host.rebind("capture.save", ["Ctrl+Shift+S"]);
    host.rebind("capture.copy", []);
    await until(() => (tab(host).keys as { save: string }).save === "Ctrl+Shift+S");
    await eventually(() => (tab(host).keys as { copy: string }).copy, "", "a cleared key shows no cap");
  } finally {
    await stop();
  }
});

test("results persist across restarts (finished only); a moved file says so; Clear Finished empties the list", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-"));
  const host = new TestHost();
  ready(host);
  const setJob = manualJobs(host);
  const first = await start(host, "zh-Hans", data);
  const { cleanup } = await withFolder(host);
  try {
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 1);
    const destination = String(host.sent("jobs.start")[0]!.destination);
    await writeFile(destination, "png");
    await setJob(1, { status: "completed", publicationStarted: true, outcome: { destination } });
    await until(() => rows(host).length === 1 && rows(host)[0]!.state === "completed");
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 2);
    await setJob(2, { status: "failed", outcome: { error: { code: "space", message: "full" } } });
    await until(() => rows(host).length === 2);
    await jsonFile(join(data, "results.json"), (v) => (v.list as unknown[]).length === 1);
    await first.stop();
    const again = new TestHost();
    const second = await startPlugin(activate, { directory, host: again, dataDirectory: data, locale: "zh-Hans" });
    await until(() => rows(again).length === 1);
    await eventually(() => [rows(again)[0]!.name, rows(again)[0]!.state], [basename(destination), "completed"], "the failed task stayed in its session");
    await second.stop();
    await rm(destination);
    const third = new TestHost();
    const after = await startPlugin(activate, { directory, host: third, dataDirectory: data, locale: "zh-Hans" });
    await until(() => rows(third)[0]?.state === "gone");
    await eventually(() => rows(third)[0]!.openable, false);
    third.mounted().emit("clear-finished");
    await until(() => rows(third).length === 0);
    await after.stop();
  } finally {
    await cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

test("file-name templates: edits rename the next capture; a chip adds a variable; Default name restores it", async () => {
  const host = new TestHost();
  ready(host);
  const { stop } = await start(host);
  const { folder, cleanup } = await withFolder(host);
  try {
    const panel = host.mounted();
    panel.emit("template-edited", "image", "shot {name}");
    panel.emit("insert-variable", "image", "{pos}");
    await host.settle();
    await eventually(() => tab(host)["template-image"], "shot {name}{pos}");
    await eventually(() => (tab(host).previews as { image: string }).image, "shot source-one05.09.412.png", "the preview names the playing file");
    await host.press("Ctrl+E");
    await until(() => host.sent("jobs.start").length === 1);
    assert.equal(host.sent("jobs.start")[0]!.destination, join(folder, "shot source-one00.02.000.png"));
    panel.emit("reset-template", "image");
    await host.settle();
    await eventually(() => tab(host)["template-image"], "{name}_{pos}");
  } finally {
    await stop();
    await cleanup();
  }
});

/** A fake ffprobe: the video's codec and start time, and key packets every 2 s (plus one at 241 s that is no keyframe). */
function probing(host: TestHost, offset = 0) {
  host.tools.handle("ffprobe", (args) =>
    args.some((arg) => arg.startsWith("stream=codec_name"))
      ? { stdout: `codec_name=h264\nstart_time=${offset.toFixed(6)}\n` }
      : { stdout: [236, 238, 240, 244].map((t) => `${(t + offset).toFixed(6)},K__`).concat([`${(241 + offset).toFixed(6)},___`]).join("\n") },
  );
}

test("Fast (R1): the default; keyframes from ffprobe on the player's clock; a stream copy from the keyframe; the range on the seek bar (R3) and in the row (R2)", async () => {
  const host = new TestHost();
  ready(host, "/contract/movie.mp4");
  host.setSource({ position: 250, duration: 600, audioTracks: TRACKS });
  // A file whose timestamps start at 1.4 s (MPEG-TS, say): ffprobe reports 1.4 s later than the player.
  probing(host, 1.4);
  const setJob = manualJobs(host);
  const { stop } = await start(host, "en");
  const folder = await mkdtemp(join(tmpdir(), "capture-fast-"));
  try {
    host.dialogAnswers.push([folder]);
    await host.command("capture.folder").run();
    await until(() => host.osd.at(-1) === `Snapshot folder: ${folder}`);
    const panel = host.mounted();
    await host.settle();
    await eventually(() => tab(host)["method-index"], 0, "Fast is the default");
    await eventually(() => tab(host).support, { fast: true, encode: true, progress: true });
    panel.emit("clip-edited", "4:02.3", "4:19");
    // The codec probe and the keyframe probe answer separately: wait for the note and the name they give.
    await until(() => (tab(host).note as { snapped: boolean; container: string }).snapped && (tab(host).note as { container: string }).container === "mp4" && tab(host)["clip-name"] === "movie_04.00.000-04.19.000.mp4");
    await eventually(() => tab(host).note, { method: "fast", start: "04:00.000", end: "04:19.000", early: "2.3", snapped: true, container: "mp4" });
    const line = tab(host).timeline as { key: number; gota: number; wanta: number; ticks: string };
    assert.ok(line.key >= 0 && line.gota < line.wanta && line.ticks.includes("M"));
    const probe = host.tools.calls.find((call) => call.args.includes("-read_intervals"))!;
    const window = probe.args[probe.args.indexOf("-read_intervals") + 1]!.split("%").map(Number);
    assert.ok(Math.abs(window[1]! - (timelineEnd(242.3, 259) + 1.4)) < 0.01, "the window in the file's timestamps");
    // R3: what the file will hold, on the seek bar while the tab shows.
    await until(() => (host.timeline.rangeLayers.get("vestra.capture") ?? []).length === 1);
    assert.deepEqual(host.timeline.rangeLayers.get("vestra.capture"), [{ start: 240, end: 259, style: "selection", title: "Clip" }]);
    panel.emit("export-clip", "4:02.3", "4:19");
    await until(() => host.sent("jobs.start").length === 1);
    const capture = host.sent("snapshot.capture")[0]!;
    assert.deepEqual([capture.mode, capture.time], ["raw", 242.3], "a copy needs a raw snapshot");
    const job = host.sent("jobs.start")[0]!;
    assert.deepEqual([job.kind, job.end, job.container], ["rawCopy", 259, "auto"]);
    assert.equal(job.encoding, undefined);
    // The player wrote MKV and reports where the file really starts.
    const written = join(folder, "movie_04.00.000-04.19.000.mkv");
    await writeFile(written, "mkv bytes");
    await setJob(1, { status: "completed", publicationStarted: true, outcome: { destination: written, start: 239.96, end: 259.02 } } as never);
    await until(() => rows(host)[0]?.state === "completed");
    const [row] = rows(host) as unknown as { name: string; method: string; range: string; badge: string }[];
    assert.deepEqual([row!.name, row!.method, row!.range, row!.badge], ["movie_04.00.000-04.19.000.mkv", "fast", "03:59.960–04:19.020", "19.1"]);
    assert.equal(host.osd.at(-1), `Clip exported as ${written}`);
    // Precise: the exact frame; the seek bar follows.
    panel.emit("set-method", 1);
    await host.settle();
    await eventually(() => [(tab(host).note as { method: string; start: string }).method, (tab(host).note as { start: string }).start], ["precise", "04:02.300"]);
    await eventually(() => tab(host)["clip-name"], "movie_04.02.300-04.19.000.mp4");
    await until(() => host.timeline.rangeLayers.get("vestra.capture")?.[0]?.start === 242.3);
    // In the background the range goes.
    host.mountOther("sidebar", "vestra.playlist", { title: "Playlist" });
    host.selectTab("vestra.playlist");
    await until(() => !(host.timeline.rangeLayers.get("vestra.capture") ?? []).length);
    await host.settle();
    assert.deepEqual(host.keepsRunning, [], "an empty layer in the background keeps nothing running");
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("a stream copy the player cannot make: Fast cut is not possible, Use Precise; an unknown source bitrate asks for another quality", async () => {
  const host = new TestHost();
  ready(host, "/contract/movie.mp4");
  host.setSource({ position: 5, duration: 600 });
  probing(host);
  const { stop } = await start(host, "en");
  const folder = await mkdtemp(join(tmpdir(), "capture-fail-"));
  try {
    host.dialogAnswers.push([folder]);
    await host.command("capture.folder").run();
    await until(() => host.osd.at(-1) === `Snapshot folder: ${folder}`);
    host.exportFailure = { code: "unsupported", message: "codec vc1 cannot be copied" };
    host.mounted().emit("export-clip", "0:01", "0:04");
    await until(() => host.osd.at(-1) === `Export failed: ${REASON.fastImpossible}`);
    // The tab's values follow the OSD in their own update: wait for them.
    await until(() => (tab(host)["clip-status"] as { reason: string }).reason === "fastImpossible" && rows(host)[0]?.reason === "fastImpossible");
    // Same as source on a file whose bitrate the player cannot tell.
    host.mounted().emit("set-method", 1);
    host.mounted().emit("set-preset", 0);
    host.mounted().emit("export-clip", "0:01", "0:04");
    await until(() => host.osd.at(-1) === `Export failed: ${REASON.noSourceBitrate}`);
    const job = host.sent("jobs.start").at(-1)!;
    assert.deepEqual([job.kind, (job.encoding as { videoBitrate?: string }).videoBitrate, "crf" in (job.encoding as object)], ["rawMp4", "source", false]);
  } finally {
    host.exportFailure = null;
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});

test("SDK 1.5.45: task rows changed after an await call plugin.update() (trimming history, a job's first state)", async () => {
  const seen: { length: number; status: string }[] = [];
  const scope = { update: () => seen.push({ length: tasks.list.length, status: tasks.list.at(-1)!.status }) } as never;
  const tasks: Tasks = new Tasks(scope, () => {});
  const spec = { kind: "image", start: 0, count: 1, step: 0, processed: false, subtitles: false, track: "none", current: false } as const;
  const later = <T>(value: T) => new Promise<T>((done) => setTimeout(() => done(value), 5));
  for (let i = 0; i < 21; i++) {
    const snapshot = { dispose: () => later(undefined) } as never;
    tasks.list.push({ id: `old${i}`, spec, sourceId: 1, source: "a", snapshot, destination: "x.png", status: "failed", cancel: false, jobs: [], outputs: [], progress: null } as Task);
  }
  let exported!: (job: unknown) => void;
  const job = { state: { status: "running" }, done: new Promise(() => {}), onChange: () => ({ dispose: async () => {} }), cancel: async () => {} };
  const snapshot = { export: () => new Promise((done) => (exported = done)), dispose: async () => {} } as never;
  tasks.add({ spec, sourceId: 1, source: "a", snapshot, destination: "new.png" });
  await until(() => seen.at(-1)?.length === 20);
  assert.equal(tasks.list.length, 20);
  exported(job);
  await until(() => seen.at(-1)?.status === "running");
  assert.equal(tasks.list.at(-1)!.status, "running");
});

test("SDK 1.5.45: each finished frame of a burst calls plugin.update(), so the row's n/total is current", async () => {
  const done: number[] = [];
  const scope = { update: () => done.push(tasks.list.at(-1)?.outputs.length ?? 0) } as never;
  const tasks: Tasks = new Tasks(scope, () => {});
  const spec = { kind: "burst", start: 0, count: 3, step: 1, processed: false, subtitles: false, track: "none", current: false } as const;
  const job = (state: unknown, finished: Promise<unknown>) => ({ state, done: finished, onChange: () => ({ dispose: async () => {} }), cancel: async () => {} });
  let exports = 0;
  const snapshot = {
    export: async ({ destination }: { destination: string }) =>
      exports++ === 0
        ? job({ status: "running" }, new Promise((ok) => setTimeout(() => ok({ status: "completed", outcome: { destination } }), 5)))
        : new Promise(() => {}),
    dispose: async () => {},
  } as never;
  tasks.add({ spec, sourceId: 1, source: "a", snapshot, destination: "burst" });
  await until(() => done.at(-1) === 1);
  assert.equal(tasks.list.at(-1)!.outputs.length, 1);
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
    assert.deepEqual(await saved(() => host.mounted().emit("save") as unknown as Promise<void>), ["processed", true], "the tab's Save is the same");
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
    host.mounted().emit("set-snapshot-effects", true);
    await host.settle();
    assert.deepEqual(await saved(() => host.press("Ctrl+E")), ["processed", false], "the effects without subtitles");
    await eventually(() => tab(host)["snapshot-effects"], true);
  } finally {
    await stop();
    await cleanup();
  }
});

test("real pages: the last sub-tab is remembered; commands select their page; Burst lives in Settings", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-"));
  const host = new TestHost();
  ready(host);
  const first = await start(host, "en", data);
  try {
    await host.settle();
    await eventually(() => tab(host).page, 0, "Snapshot first");
    await host.command("capture.tasks").run();
    await host.settle();
    await eventually(() => tab(host).page, 2, "Export Tasks… shows Results");
    await host.press("Ctrl+G");
    await host.settle();
    await eventually(() => [tab(host).page, tab(host)["burst-open"]], [3, true], "Burst… shows Settings with its row open");
    assert.equal(host.command("capture.burst").state.checked, true);
    host.mounted().emit("set-page", 1);
    await host.settle();
    assert.equal(host.command("capture.burst").state.checked, false);
    await jsonFile(join(data, "settings.json"), (v) => v.page === 1);
    await first.stop();
    const again = new TestHost();
    ready(again);
    const second = await startPlugin(activate, { directory, host: again, dataDirectory: data, locale: "en" });
    await again.settle();
    await eventually(() => tab(again).page, 1, "the Clip page again after a restart");
    await again.command("capture.settings").run();
    await again.settle();
    await eventually(() => tab(again).page, 3);
    await second.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});
