import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import test from "node:test";
import { isDeepStrictEqual } from "node:util";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { eventually, newFakes, stills, watchHost } from "./fakes.ts";

const fakes = newFakes();
const directory = new URL("..", import.meta.url);
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
const exists = (path: string) => stat(path).then(() => true, () => false);
const lastOsd = (host: TestHost) => host.osd.at(-1) ?? "";
/** Options ▸ Video ▸ Snapshot: the page and its bridge values. */
const page = (host: TestHost) => host.settingsPage("snapshot");
const props = (host: TestHost) => page(host).props;
/** The newest burst finished: its message. */
const burstDone = (host: TestHost) => lastOsd(host).startsWith("连拍已保存");

/** Jobs publish a real file at the requested destination (like the worker), so copies and the task list see it. */
function publishingJobs(host: TestHost) {
  host.handle("jobs.state", async (p) => {
    const job = host.jobs.get(Number(p.jobId))!;
    const current = structuredClone(job);
    if (job.status === "queued") job.status = "running";
    else if (job.status === "running") {
      const destination = String(host.sent("jobs.start")[job.jobId - 1]!.destination);
      await writeFile(destination, "png bytes");
      Object.assign(job, { status: "completed", publicationStarted: true, outcome: { destination } });
    } else if (job.status === "cancelling") Object.assign(job, { status: "cancelled", outcome: { cancelled: true } });
    return current;
  });
}

/** A started plugin with a ready local source, a temporary capture folder and data directory; texts as a Chinese user sees them. */
async function setup(path = "/contract/source-one.mp4") {
  const host = new TestHost();
  watchHost(host, fakes);
  host.openElsewhere(path);
  host.setSource({ position: 2, duration: 30 });
  publishingJobs(host);
  const data = await mkdtemp(join(tmpdir(), "capture-data-"));
  const folder = await mkdtemp(join(tmpdir(), "capture-folder-"));
  const plugin = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
  host.dialogAnswers.push([folder]);
  await host.command("capture.folder").run();
  await until(() => lastOsd(host) === `截图文件夹：${folder}`); // after the plugin's own stat of the folder
  return {
    host,
    folder,
    data,
    async stop() {
      await plugin.stop();
      await rm(folder, { recursive: true, force: true });
      await rm(data, { recursive: true, force: true });
    },
  };
}

test("F03: Ctrl+C copies the raw frame as PNG through a temporary file that is then removed", async () => {
  const { host, folder, stop } = await setup();
  try {
    await host.command("capture.format.jpeg").run(); // the clipboard ignores the format setting
    await host.press("Ctrl+C");
    await until(() => lastOsd(host) === "画面截图：复制到剪贴板");
    const capture = host.sent("snapshot.capture")[0]!;
    assert.equal(capture.mode, "raw");
    assert.deepEqual(capture.expected, host.source.stamp);
    const [copied] = fakes.copied;
    assert.equal(copied!.existed, true, "the PNG existed when it went to the clipboard");
    assert.deepEqual(host.clipboard.images, [copied!.path], "written through plugin.clipboard.writeImage");
    assert.equal(basename(dirname(copied!.path)), `vestra-capture-${process.pid}`);
    assert.match(copied!.path, /clip-1\.png$/);
    assert.equal(await exists(copied!.path), false, "temporary file removed");
    assert.deepEqual(host.exports.map((job) => job.kind), ["rawPng"], "always PNG");
    assert.equal(host.snapshots.size, 0, "snapshot released");
    assert.deepEqual(await readdir(folder), [], "nothing saved in the capture folder");
    assert.equal(host.sent("jobs.start").length, 1);
  } finally {
    await stop();
  }
  assert.equal(await exists(join(tmpdir(), `vestra-capture-${process.pid}`)), false, "temporary folder emptied on deactivation");
});

test("F03/F04: Ctrl+Alt+C copies the processed frame; a second press while one is in progress is refused", async () => {
  const { host, stop } = await setup();
  try {
    let release!: () => void;
    fakes.hold = new Promise((done) => (release = done));
    await host.press("Ctrl+Alt+C");
    await until(() => host.sent("jobs.start").length === 1);
    await host.press("Ctrl+C");
    await host.press("Ctrl+E");
    await host.settle();
    assert.equal(lastOsd(host), "上一张截图仍在处理");
    assert.equal(host.sent("snapshot.capture").length, 1, "not queued");
    release();
    await until(() => lastOsd(host) === "画面截图：复制到剪贴板");
    const capture = host.sent("snapshot.capture")[0]!;
    assert.equal(capture.mode, "processed");
    assert.equal(capture.includeSubtitles, true, "实际画面包含字幕 is on by default");
    fakes.hold = null;
    fakes.failCopy = true;
    await host.press("Ctrl+C");
    await until(() => lastOsd(host) === "无法写入剪贴板");
    assert.equal(host.snapshots.size, 0);
  } finally {
    await stop();
  }
});

test("F03: copy commands are disabled without a capturable source", async () => {
  const { host, stop } = await startPlugin(activate, { directory, host: new TestHost(), locale: "zh-Hans" });
  for (const id of ["capture.copy", "capture.copyScreen", "capture.saveScreen", "capture.saveScreenAs"]) {
    assert.equal(host.command(id).state.enabled, false, id);
    assert.equal(host.translate(host.command(id).state.reason!), "没有可截取的画面");
  }
  assert.equal(host.command("capture.openFolder").state.enabled, true, "F09 works without media");
  await stop();
});

test("F04: 保存实际画面 includes effects and, unless unticked, subtitles; 保存当前画面 stays raw", async () => {
  const { host, folder, stop } = await setup();
  try {
    await host.press("Ctrl+Alt+E");
    await until(() => lastOsd(host).startsWith("帧已保存为"));
    let capture = host.sent("snapshot.capture").at(-1)!;
    assert.deepEqual([capture.mode, capture.includeSubtitles], ["processed", true]);
    assert.equal(host.sent("jobs.start").at(-1)!.destination, join(folder, "source-one_00.02.000.png"), "same name and folder as a source frame");
    const toggle = host.command("capture.screenSubtitles");
    assert.equal(toggle.state.checked, true);
    await toggle.run();
    await host.settle();
    assert.equal(toggle.state.checked, false);
    await host.press("Ctrl+Alt+E");
    await until(() => host.sent("snapshot.capture").length === 2);
    capture = host.sent("snapshot.capture").at(-1)!;
    assert.deepEqual([capture.mode, capture.includeSubtitles], ["processed", false]);
    await until(() => host.sent("jobs.start").length === 2 && lastOsd(host).includes("(1).png"));
    await host.press("Ctrl+E");
    await until(() => host.sent("snapshot.capture").length === 3);
    assert.equal(host.sent("snapshot.capture").at(-1)!.mode, "raw");
    await until(() => host.sent("jobs.start").length === 3);
    host.dialogAnswers.push([join(folder, "screen")]);
    await host.command("capture.saveScreenAs").run();
    await until(() => host.sent("snapshot.capture").length === 4);
    assert.equal(host.sent("snapshot.capture").at(-1)!.mode, "processed");
    const panel = host.sent("dialogs.open").at(-1)!;
    assert.equal(panel.title, "Save Rendered Frame As");
    assert.equal(host.translate(String(panel.title)), "实际画面另存为");
  } finally {
    await stop();
  }
});

test("F07: JPEG is exported by the player with the chosen quality; quality and format persist; BMP too", async () => {
  const { host, folder, data, stop } = await setup();
  try {
    const jpeg = host.command("capture.format.jpeg");
    await jpeg.run();
    await host.settle();
    assert.equal(lastOsd(host), "截图格式：JPEG");
    assert.equal(jpeg.state.checked, true);
    assert.equal(host.command("capture.format.png").state.checked, false);
    const settings = page(host);
    await eventually(() => props(host)["format-index"], 1);
    settings.emit("set-quality", 80);
    await host.settle();
    await eventually(() => props(host).quality, 80);
    await host.press("Ctrl+E");
    await until(() => lastOsd(host).startsWith("帧已保存为"));
    const target = join(folder, "source-one_00.02.000.jpg");
    const [first] = host.exports;
    assert.deepEqual([first!.kind, first!.destination, first!.quality, first!.sampleTime], ["rawJpeg", target, 80, 2]);
    assert.equal(lastOsd(host), `帧已保存为 ${target}`);
    assert.deepEqual(await readdir(folder), ["source-one_00.02.000.jpg"], "only the final file, no temporary PNG");
    // plugin.storage writes 300 ms after the last change
    const expected = { folder, format: "jpeg", quality: 80, screenSubtitles: true, besideMedia: false };
    const picked = (value: Record<string, unknown>) => Object.fromEntries(Object.keys(expected).map((key) => [key, value[key]]));
    await until(() => readFile(join(data, "settings.json"), "utf8").then((text) => isDeepStrictEqual(picked(JSON.parse(text)), expected), () => false));
    settings.emit("choose-format", 2);
    await host.settle();
    assert.equal(host.command("capture.format.bmp").state.checked, true);
    await host.press("Ctrl+E");
    await until(() => host.exports.length === 2 && lastOsd(host).endsWith(".bmp"));
    const second = host.exports[1]!;
    assert.deepEqual([second.kind, second.destination, "quality" in second], ["rawBmp", join(folder, "source-one_00.02.000.bmp"), false]);
    // Restart: the format and quality come back.
    const restart = new TestHost();
    const again = await startPlugin(activate, { directory, host: restart, dataDirectory: data, locale: "zh-Hans" });
    assert.equal(restart.command("capture.format.bmp").state.checked, true);
    await restart.settle();
    await eventually(() => restart.settingsPage("snapshot").props.quality, 80);
    await again.stop();
  } finally {
    await stop();
  }
});

test("F07: a failed JPEG export fails the task like a PNG one; nothing is left behind", async () => {
  const { host, folder, stop } = await setup();
  try {
    await host.command("capture.format.jpeg").run();
    host.handle("jobs.state", (p) => {
      const job = host.jobs.get(Number(p.jobId))!;
      const current = structuredClone(job);
      if (job.status === "queued") job.status = "running";
      else if (job.status === "running")
        Object.assign(job, { status: "failed", outcome: { error: { code: "failed", message: "JPEG encoding failed" } } });
      return current;
    });
    await host.press("Ctrl+E");
    await until(() => lastOsd(host) === "截图失败：无法写入图片文件");
    assert.equal(stills(host)[0]!.kind, "rawJpeg");
    assert.deepEqual(await readdir(folder), []);
  } finally {
    await stop();
  }
});

test("F07: 另存为 uses the typed extension once; otherwise the current format's extension", async () => {
  const { host, folder, stop } = await setup();
  try {
    await host.command("capture.format.jpeg").run();
    host.dialogAnswers.push([join(folder, "a.png")]);
    await host.command("capture.saveAs").run();
    assert.equal(host.sent("dialogs.open").at(-1)!.fileName, "source-one_00.02.000.jpg", "default name carries the format");
    await until(() => lastOsd(host).startsWith("帧已保存为"));
    assert.equal(host.sent("jobs.start").at(-1)!.destination, join(folder, "a.png"));
    assert.equal(host.exports.at(-1)!.kind, "rawPng");
    assert.equal(stills(host).length, 0);
    assert.equal(host.command("capture.format.jpeg").state.checked, true, "the setting is unchanged");
    host.dialogAnswers.push([join(folder, "b")]);
    await host.command("capture.saveAs").run();
    await until(() => stills(host).length === 1);
    assert.deepEqual([stills(host)[0]!.kind, stills(host)[0]!.destination], ["rawJpeg", join(folder, "b.jpg")]);
    host.dialogAnswers.push([join(folder, "c.BMP")]);
    await host.command("capture.saveAs").run();
    await until(() => stills(host).length === 2);
    assert.deepEqual([stills(host)[1]!.kind, stills(host)[1]!.destination], ["rawBmp", join(folder, "c.BMP")]);
  } finally {
    await stop();
  }
});

test("F07: bursts use the shared format; the settings page's format list changes it", async () => {
  const { host, folder, stop } = await setup();
  try {
    const settings = page(host);
    await eventually(() => props(host)["format-index"], 0);
    settings.emit("choose-format", 1);
    settings.emit("burst-edited", "3", "0.5");
    await host.settle();
    assert.equal(host.command("capture.format.jpeg").state.checked, true);
    await host.press("Ctrl+G");
    await until(() => burstDone(host));
    const [child] = await readdir(folder);
    assert.deepEqual((await readdir(join(folder, child!))).sort(), ["001-2.000s.jpg", "002-2.500s.jpg", "003-3.000s.jpg"]);
    assert.equal(host.exports.length, 3);
    assert.ok(host.exports.every((job) => job.kind === "rawJpeg" && job.quality === 90));
  } finally {
    await stop();
  }
});

test("F09: 打开截图文件夹 reveals the folder, recreating it; the media folder when 储存到播放文件夹 is on", async () => {
  const media = await mkdtemp(join(tmpdir(), "capture-media-"));
  const { host, folder, stop } = await setup(join(media, "movie.mp4"));
  try {
    // 打开截图文件夹 checks and creates the folder on disk before system.openPath: wait for the open.
    await host.command("capture.openFolder").run();
    await until(() => fakes.opened.length === 1);
    assert.deepEqual(fakes.opened, [folder]);
    await rm(folder, { recursive: true });
    await host.command("capture.openFolder").run();
    await until(() => fakes.opened.length === 2);
    assert.equal(await exists(folder), true, "recreated");
    await host.command("capture.settings").run();
    assert.deepEqual(host.settings.shows, ["snapshot"], "Snapshot Settings… opens the Options window at the page");
    page(host).emit("set-beside-media", true);
    await host.settle();
    await host.command("capture.openFolder").run();
    await until(() => fakes.opened.length === 3);
    assert.equal(fakes.opened.at(-1), media);
    await host.press("Ctrl+E");
    await until(() => lastOsd(host).startsWith("帧已保存为"));
    assert.equal(host.sent("jobs.start").at(-1)!.destination, join(media, "movie_00.02.000.png"), "saved beside the media");
    host.openElsewhere("https://example.test/live.m3u8");
    await host.settle();
    await host.command("capture.openFolder").run();
    await until(() => fakes.opened.length === 4);
    assert.equal(fakes.opened.at(-1), folder, "a URL source uses the capture folder");
    fakes.failOpen = true;
    await host.command("capture.openFolder").run();
    await until(() => lastOsd(host) === "无法打开文件夹。");
  } finally {
    await stop();
    await rm(media, { recursive: true, force: true });
  }
});

test("F09: the saved message's buttons act on the newest capture: Show in Folder, Copy; a moved file says so", async () => {
  const { host, folder, stop } = await setup();
  try {
    await host.command("capture.revealLast").run();
    await until(() => lastOsd(host) === host.translate("No snapshot saved yet."));
    await host.press("Ctrl+E");
    await until(() => lastOsd(host).startsWith("帧已保存为"));
    const frame = join(folder, "source-one_00.02.000.png");
    assert.deepEqual(host.osdActions.at(-1)?.map((action) => action.command), ["capture.revealLast", "capture.copyLast"]);
    await host.pressOsdAction(0);
    await until(() => host.system.revealed.length === 1);
    assert.equal(host.system.revealed[0], frame);
    await host.pressOsdAction(1);
    await until(() => lastOsd(host) === "画面截图：复制到剪贴板");
    assert.equal(fakes.copied.at(-1)!.path, frame);
    page(host).emit("burst-edited", "2", "0.5");
    await host.press("Ctrl+G");
    await until(() => burstDone(host));
    assert.deepEqual(host.osdActions.at(-1)?.map((action) => action.command), ["capture.revealLast"], "a burst's message: Show in Folder only");
    const [burstFolder] = (await readdir(folder)).filter((name) => name.startsWith("Capture-"));
    await host.command("capture.revealLast").run();
    await until(() => host.system.revealed.length === 2);
    assert.equal(host.system.revealed[1], join(folder, burstFolder!, "001-2.000s.png"), "a burst shows its first frame");
    await rm(join(folder, burstFolder!), { recursive: true });
    await host.command("capture.revealLast").run();
    await until(() => lastOsd(host) === host.translate("The file or folder was moved or deleted."));
    assert.equal(host.system.revealed.length, 2, "nothing shown for a moved file");
  } finally {
    await stop();
  }
});

test("settings: a damaged settings.json is set aside as .bad-<time>; a partly invalid one falls back field by field", async () => {
  const data = await mkdtemp(join(tmpdir(), "capture-data-"));
  const folder = await mkdtemp(join(tmpdir(), "capture-folder-"));
  try {
    await writeFile(join(data, "settings.json"), "{ broken");
    let started = await startPlugin(activate, { directory, host: new TestHost(), dataDirectory: data });
    assert.equal(started.host.command("capture.format.png").state.checked, true);
    await started.stop();
    assert.ok((await readdir(data)).some((name) => /^settings\.json\.bad-\d+$/.test(name)));
    await writeFile(join(data, "settings.json"), JSON.stringify({ folder, format: "gif", quality: 250, screenSubtitles: "yes", besideMedia: true }));
    started = await startPlugin(activate, { directory, host: new TestHost(), dataDirectory: data });
    const { host } = started;
    assert.equal(host.command("capture.format.png").state.checked, true, "unknown format → PNG");
    assert.equal(host.command("capture.screenSubtitles").state.checked, true, "not a boolean → default");
    await host.settle();
    const values = props(host);
    assert.deepEqual([values.folder, values.quality, values["beside-media"]], [folder, 100, true]);
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
    await rm(folder, { recursive: true, force: true });
  }
});
