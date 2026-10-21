// Round 3a: A08 secondary subtitle (menu, 添加次字幕…, position, the 字幕微调 row, mutual exclusion).
import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const ZH = { kind: "embedded" as const, id: 1 };
const EN = { kind: "embedded" as const, id: 2 };
const PGS = { kind: "embedded" as const, id: 3 };
const text = (codec: string) => ({ styleSupported: true, styleReason: null, codec, text: true, streamIndex: null });

/** A ready source with Chinese (selected), English and a PGS track. */
function dualHost() {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.subtitles.tracks.push(
    { target: ZH, title: "中文", language: "zh", ...text("subrip") },
    { target: EN, title: "English", language: "en", ...text("ass") },
    { target: PGS, title: "PGS", language: null, styleSupported: false, styleReason: "图形字幕", codec: "hdmv_pgs_subtitle", text: false, streamIndex: null },
  );
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: ZH };
  return host;
}
const items = (host: TestHost, path: string) =>
  [...host.commands.entries()]
    .filter(([, { declaration: d }]) => d.path.slice(0, -1).join(" > ") === path)
    .map(([id, { declaration: d, state }]) => [id, d.path.at(-1), d.order, d.group, d.radio, state.enabled, state.checked, state.reason] as const);
const secondary = (host: TestHost) => host.subtitles.selection.secondary ?? null;
const styleSent = (host: TestHost) => host.sent("subtitles.style").map((p) => (p.style as Record<string, unknown> | null)?.secondaryPosition);
async function srt(name: string, body: string) {
  const folder = await mkdtemp(join(tmpdir(), "vestra-subtitles-r3a-"));
  const path = join(folder, name);
  await writeFile(path, body);
  return { path, cleanup: () => rm(folder, { recursive: true, force: true }) };
}

test("A08: 次字幕 ▶ lists the tracks; the primary and bitmap tracks are disabled with the reason", async () => {
  const host = dualHost();
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  assert.deepEqual(items(host, "字幕 > 次字幕"), [
    ["second-off", "不显示", 312, "subtitle-second", true, true, true, null],
    ["second-1", "中文 · zh", 313, "subtitle-second", true, false, false, "已作为主字幕"],
    ["second-2", "English · en", 314, "subtitle-second", true, true, false, null],
    ["second-3", "PGS", 315, "subtitle-second", true, false, false, "次字幕仅支持文本字幕"],
  ]);
  assert.deepEqual(items(host, "字幕 > 次字幕 > 位置").map((i) => i.slice(1, 7)), [
    ["顶部", 321, "subtitle-second-pos", true, true, true],
    ["主字幕上方", 321, "subtitle-second-pos", true, true, false],
  ]);
  await host.command("second-2").run();
  await host.settle();
  assert.deepEqual(secondary(host), EN);
  assert.deepEqual(host.subtitles.selection.target, ZH, "the primary stays");
  assert.equal(host.sent("subtitles.select").at(-1)!.role, "secondary");
  assert.equal(host.osd.at(-1), "次字幕：English · en");
  assert.equal(host.command("second-2").state.checked, true);
  assert.equal(host.command("second-off").state.checked, false);
  // Alt+H is the primary only.
  await host.press("Alt+H");
  await host.settle();
  assert.equal(host.subtitles.selection.target, null);
  assert.deepEqual(secondary(host), EN);
  await host.press("Alt+H");
  await host.settle();
  assert.deepEqual(host.subtitles.selection.target, ZH, "brings back the last primary, never the secondary");
  await host.command("second-off").run();
  await host.settle();
  assert.equal(secondary(host), null);
  assert.equal(host.osd.at(-1), "次字幕：关闭");
  // A new source clears the secondary in the player; the menu follows.
  await host.command("second-2").run();
  await host.settle();
  host.openElsewhere("/other.mkv");
  host.subtitles.selection = { stamp: { generation: 2, revision: 1 }, target: null, secondary: null };
  await host.command("refresh").run();
  await host.settle();
  assert.equal(host.command("second-off").state.checked, true);
  await stop();
});

test("A08: choosing the secondary as the primary turns the secondary off; 依次切换 skips it", async () => {
  const host = dualHost();
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  await host.command("second-2").run();
  await host.settle();
  await host.command("track-2").run(); // 选择字幕轨 ▶ English
  await host.settle();
  assert.deepEqual(host.subtitles.selection.target, EN);
  assert.equal(secondary(host), null);
  assert.equal(host.osd.at(-1), "字幕：English · en。次字幕已关闭（已改为主字幕）");
  assert.deepEqual(items(host, "字幕 > 次字幕").map((i) => [i[1], i[5], i[7]]), [
    ["不显示", true, null],
    ["中文 · zh", true, null],
    ["English · en", false, "已作为主字幕"],
    ["PGS", false, "次字幕仅支持文本字幕"],
  ]);
  // Primary 中文, secondary English: Alt+L goes 中文 → PGS → 关闭 → 中文, never English.
  await host.command("track-1").run();
  await host.command("second-2").run();
  await host.settle();
  const seen: unknown[] = [];
  for (let i = 0; i < 3; i++) {
    await host.press("Alt+L");
    for (let n = 0; n < 20 && host.sent("subtitles.select").length === 0; n++) await pause(5);
    await host.settle();
    seen.push(host.subtitles.selection.target);
  }
  assert.deepEqual(seen, [PGS, null, ZH]);
  assert.deepEqual(secondary(host), EN, "cycling keeps the secondary");
  await stop();
});

test("A08: 添加次字幕… loads a file as the secondary and leaves the primary; it needs media", async () => {
  const file = await srt("english.srt", "1\n00:00:01,000 --> 00:00:02,000\nHello\n");
  const empty = new TestHost();
  let started = await startPlugin(activate, { directory, host: empty });
  assert.equal(empty.command("load-secondary").state.enabled, false);
  assert.equal(empty.command("load-secondary").state.reason, "没有可播放的媒体");
  assert.deepEqual(items(empty, "字幕 > 次字幕").map((i) => i[1]), ["不显示"], "no text track: only 不显示");
  await started.stop();
  const host = dualHost();
  started = await startPlugin(activate, { directory, host });
  try {
    assert.deepEqual(host.command("load-secondary").declaration.path, ["打开", "添加次字幕…"]);
    assert.equal(host.command("load-secondary").declaration.order, 114);
    host.dialogAnswers.push([file.path]);
    await host.command("load-secondary").run();
    await host.settle();
    assert.deepEqual(host.subtitleLoads.map((l) => [l.format, l.title, l.select]), [["srt", "english.srt", "secondary"]]);
    assert.deepEqual(host.subtitles.selection.target, ZH, "the primary is unchanged");
    assert.equal(secondary(host)?.kind, "external");
    assert.equal(host.osd.at(-1), "次字幕：english.srt");
    assert.equal(host.command("second-4").state.checked, true, "the new track is listed and checked");
  } finally {
    await started.stop();
    await file.cleanup();
  }
});

test("A08: 位置 ▶ 主字幕上方 is applied as a style field, saved, and re-applied for a later secondary", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  try {
    let host = dualHost();
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.command("second-position-aboveMain").run();
    await host.settle(170);
    assert.equal(host.osd.at(-1), "次字幕位置：主字幕上方");
    assert.deepEqual(styleSent(host), ["aboveMain"]);
    assert.equal(host.command("second-position-aboveMain").state.checked, true);
    assert.equal(host.command("second-position-top").state.checked, false);
    // Other style edits keep the position.
    await host.press("Alt+B");
    await host.settle(170);
    assert.deepEqual(styleSent(host), ["aboveMain", "aboveMain"]);
    await started.stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.secondaryPosition, "aboveMain");

    // Next start: nothing is sent until a secondary is shown, then the position once.
    await rm(join(data, "style.json"), { force: true });
    host = dualHost();
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle(170);
    assert.deepEqual(styleSent(host), []);
    await host.command("second-2").run();
    await host.settle(170);
    assert.deepEqual(styleSent(host), ["aboveMain"]);
    assert.equal(host.subtitles.presentation.effective?.style.secondaryPosition, "aboveMain");
    assert.equal((await readdir(data)).includes("style.json"), false, "the position alone is not a remembered style");
    await host.command("second-position-top").run();
    await host.settle(170);
    assert.deepEqual(styleSent(host), ["aboveMain", "top"]);
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("A08: the 字幕微调 次字幕 row selects, refuses the primary with a reason, and turns off", async () => {
  const host = dualHost();
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("tune").run();
  const window = host.window("subtitle-tune");
  assert.deepEqual(window.props["secondary-labels"], ["关闭次字幕", "中文 · zh", "English · en", "PGS"]);
  assert.equal(window.props["selected-secondary"], 0);
  window.emit("select-secondary", 2);
  await host.settle();
  assert.deepEqual(secondary(host), EN);
  assert.equal(window.props["selected-secondary"], 2);
  assert.equal(window.props["status-text"], "次字幕：English · en");
  window.emit("select-secondary", 1);
  await host.settle();
  assert.equal(window.props["status-text"], "无法设为次字幕：已作为主字幕");
  window.emit("select-secondary", 3);
  await host.settle();
  assert.equal(window.props["status-text"], "无法设为次字幕：次字幕仅支持文本字幕");
  assert.deepEqual(secondary(host), EN, "refusals change nothing");
  window.emit("select-secondary", 0);
  await host.settle();
  assert.equal(secondary(host), null);
  assert.equal(window.props["selected-secondary"], 0);
  await stop();
});

test("A08: an automatic reload of the secondary keeps it the secondary and the primary as it is", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-data-"));
  const file = await srt("english.srt", "1\n00:00:01,000 --> 00:00:02,000\nHello\n");
  const host = dualHost();
  const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data });
  try {
    await host.command("settings").run();
    host.window("subtitle-settings").emit("change-auto-reload", true);
    host.dialogAnswers.push([file.path]);
    await host.command("load-secondary").run();
    await host.settle();
    const first = structuredClone(secondary(host));
    await pause(200);
    await writeFile(file.path, "1\n00:00:01,000 --> 00:00:02,000\nHello again\n");
    for (let i = 0; i < 60 && host.osd.at(-1) !== "字幕已自动重载"; i++) await pause(100);
    await host.settle();
    assert.equal(host.osd.at(-1), "字幕已自动重载");
    assert.deepEqual(host.subtitleLoads.map((l) => l.select), ["secondary", "secondary"]);
    assert.equal(secondary(host)?.kind, "external");
    assert.notDeepEqual(secondary(host), first, "the new upload is the secondary");
    assert.deepEqual(host.subtitles.selection.target, ZH);
    assert.equal(host.subtitles.tracks.filter((t) => t.target.kind === "external").length, 1, "the old track is removed");
  } finally {
    await stop();
    await file.cleanup();
    await rm(data, { recursive: true, force: true });
  }
});
