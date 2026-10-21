import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "../../../sdk/plugin/testing.ts";
import { currentChapter, segmentAt, stepPoint } from "../chapters.ts";
import { clock, fit, mediaKey, parseTime, precise } from "../format.ts";
import activate from "../main.ts";
import { bookmarkLines, chapterLines } from "../menu.ts";
import { LAYER_MAX, seekbarMarks } from "../seekbar.ts";
import { timing } from "../store.ts";

Object.assign(timing, { bookmarksWrite: 10, settingsWrite: 10, menuRebuild: 10, loopCommit: 30 });
const WAIT = 60; // longer than the shortened delays

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-marks-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));
let files = 0;
/** A real file (realpath and size work on it). */
function file(bytes = 10) {
  const path = join(scratch, `movie-${files++}.mkv`);
  writeFileSync(path, Buffer.alloc(bytes));
  return mediaKey(path);
}

const running = new Set<() => Promise<void>>();
async function start(host = new TestHost(), dataDirectory?: string) {
  const started = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  return { ...started, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

/** Another plugin opens `source`; it becomes ready with a duration. */
async function play(host: TestHost, source: string, duration = 600, position = 0) {
  host.openElsewhere(source);
  await host.settle();
  host.setSource({ duration, position });
  await host.settle(WAIT);
}
const at = async (host: TestHost, position: number) => {
  host.setSource({ position });
  await host.settle();
};
const menuLines = (host: TestHost, group: string) =>
  [...host.commands.values()]
    .filter((c) => c.declaration.group === group)
    .sort((a, b) => a.declaration.order - b.declaration.order)
    .map((c) => c.declaration.path.at(-1)!);
const idFor = (host: TestHost, title: string) =>
  [...host.commands.entries()].find(([, c]) => c.declaration.path.at(-1) === title)![0];
const seeks = (host: TestHost) =>
  host
    .sent("source.submit")
    .map((p) => p.intent as { kind: string; control?: { kind: string; seconds: number } })
    .filter((i) => i.kind === "control" && i.control!.kind === "seek")
    .map((i) => i.control!.seconds);
const loops = (host: TestHost) => host.sent("player.setLoop").map((p) => p.loop);
const press = async (host: TestHost, binding: string) => {
  await host.press(binding);
  await host.settle(WAIT);
};

// ------------------------------------------------------------------ helpers

test("formats: clock, m:ss.d, time input, menu byte limit", () => {
  assert.equal(clock(0), "00:00");
  assert.equal(clock(83.9), "01:23");
  assert.equal(clock(5025), "01:23:45");
  assert.equal(precise(70), "1:10.0");
  assert.equal(precise(62.34), "1:02.3");
  assert.equal(precise(3723.45), "1:02:03.5");
  assert.equal(precise(83.4, true), "0:01:23.4");
  assert.equal(parseTime("0:01:23.4"), 83.4);
  assert.equal(parseTime("1:23.45"), 83.5);
  assert.equal(parseTime("83.4"), 83.4);
  assert.equal(parseTime("1：02"), 62);
  for (const bad of ["", "abc", "1:60", "1.5:00", "1:2:3:4", "-3"]) assert.equal(parseTime(bad), null, bad);
  const long = fit("章".repeat(60), 80);
  assert.ok(Buffer.byteLength(long) <= 80);
  assert.ok(long.endsWith("…"));
  assert.equal(fit("  a\tb\n ", 80), "a b");
});

test("points: previous from position − 1, next from position + 0.5, current chapter, segment", () => {
  const times = [0, 10, 50];
  assert.equal(stepPoint(times, 52, -1), 50);
  assert.equal(stepPoint(times, 50.2, -1), 10);
  assert.equal(stepPoint(times, 0.5, -1), null);
  assert.equal(stepPoint(times, 10.2, 1), 50);
  assert.equal(stepPoint(times, 49.7, 1), null);
  const chapters = [
    { title: "a", start: 0 },
    { title: "b", start: 4 },
  ];
  assert.equal(currentChapter(chapters, 3.9), 0);
  assert.equal(currentChapter(chapters, 4), 1);
  assert.equal(currentChapter([], 4), -1);
  assert.deepEqual(segmentAt([4, 10], 5, 20), { a: 4, b: 10 });
  assert.deepEqual(segmentAt([4, 10], 2, 20), { a: 0, b: 4 });
  assert.deepEqual(segmentAt([4, 10], 12, 20), { a: 10, b: 20 });
  assert.equal(segmentAt([], 12, 20), null);
});

test("menu lines are numbered, unique and fit the 80-byte path segment", () => {
  assert.deepEqual(
    chapterLines([
      { title: "Opening", start: 0 },
      { title: null, start: 75 },
      { title: "  ", start: 3725 },
    ]),
    ["01  Opening    00:00", "02  章节 2    01:15", "03  章节 3    01:02:05"],
  );
  const lines = bookmarkLines([
    { title: "高潮", time: 83 },
    { title: "高潮", time: 83.4 },
    { title: "长".repeat(100), time: 5025 },
  ]);
  assert.deepEqual(lines.slice(0, 2), ["高潮    01:23", "高潮 (2)    01:23"]);
  assert.ok(lines.every((line) => Buffer.byteLength(line) <= 80 && line === line.trim()));
  assert.ok(lines[2]!.endsWith("…    01:23:45"));
});

// ------------------------------------------------------------------ B21 chapters

test("B21 chapters: listed once ready, current one checked, selecting jumps and names it", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "Opening", start: 0 },
    { title: null, start: 4 },
    { title: "Main", start: 60 },
  ];
  await start(host);
  assert.deepEqual(menuLines(host, "marks-chapters"), []);
  await play(host, file(), 600, 5);
  assert.deepEqual(menuLines(host, "marks-chapters"), ["01  Opening    00:00", "02  章节 2    00:04", "03  Main    01:00"]);
  const chapter = (i: number) => host.command(`chapter.${i}`);
  assert.equal(chapter(0).declaration.radio, true);
  assert.deepEqual([0, 1, 2].map((i) => chapter(i).state.checked), [false, true, false]);
  await chapter(2).run();
  await host.settle();
  assert.deepEqual(seeks(host), [60]);
  assert.equal(host.osd.at(-1), "章节定位：Main");
  assert.equal(chapter(2).state.checked, true);
  // Ctrl+PgUp / PgDn step between chapters only.
  await press(host, "Ctrl+PageUp");
  assert.deepEqual(seeks(host), [60, 4]);
  await press(host, "Ctrl+PageDown");
  assert.deepEqual(seeks(host), [60, 4, 60]);
  await press(host, "Ctrl+PageDown");
  assert.equal(host.osd.at(-1), "没有下一个章节");
  // Another file without chapters: the old ones are gone at once.
  host.chapters = [];
  host.openElsewhere(file());
  await host.settle(WAIT);
  assert.deepEqual(menuLines(host, "marks-chapters"), []);
});

test("B21 menu: at most 50 chapters, then a line that opens the window", async () => {
  const host = new TestHost();
  host.chapters = Array.from({ length: 60 }, (_, i) => ({ title: `C${i}`, start: i * 10 }));
  await start(host);
  await play(host, file(), 700);
  const lines = menuLines(host, "marks-chapters");
  assert.equal(lines.length, 51);
  assert.equal(lines.at(-1), "更多章节…（共 60 项）");
  await host.command("chapter.more").run();
  await host.settle();
  assert.equal(host.window("marks").visible, true);
});

// ------------------------------------------------------------------ B21 bookmarks

test("B21 bookmarks: P adds, nearby refused, Shift+PgUp walks back through them", async () => {
  const host = new TestHost();
  await start(host);
  const add = host.command("marks.add");
  assert.equal(add.state.enabled, false);
  assert.equal(add.state.reason, "未打开媒体");
  await play(host, file(), 600, 10);
  await press(host, "P");
  assert.equal(host.osd.at(-1), "书签：书签 1 已添加（00:10）");
  await at(host, 10.3);
  await press(host, "P");
  assert.equal(host.osd.at(-1), "此处已有书签");
  await at(host, 50);
  await press(host, "P");
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["书签 1    00:10", "书签 2    00:50"]);
  await at(host, 52);
  await press(host, "Shift+PageUp");
  assert.deepEqual(seeks(host), [50]);
  assert.equal(host.osd.at(-1), "书签：定位到 书签 2");
  await press(host, "Shift+PageUp");
  assert.deepEqual(seeks(host), [50, 10]);
  await press(host, "Shift+PageUp");
  assert.equal(host.osd.at(-1), "没有上一个书签/章节");
  await press(host, "Shift+PageDown");
  assert.deepEqual(seeks(host), [50, 10, 50]);
  // Selecting a bookmark in the menu jumps to it.
  await host.command(idFor(host, "书签 1    00:10")).run();
  await host.settle();
  assert.deepEqual(seeks(host), [50, 10, 50, 10]);
});

test("B21 bookmarks: a source that cannot seek cannot take one; no media disables stepping", async () => {
  const host = new TestHost();
  await start(host);
  assert.equal(host.command("marks.previous").state.reason, "未打开媒体");
  host.openElsewhere("https://live.example/stream.m3u8");
  await host.settle();
  host.setSource({ position: 3, capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle(WAIT);
  assert.equal(host.command("marks.add").state.enabled, false);
  assert.equal(host.command("marks.add").state.reason, "当前来源不能添加书签");
  assert.equal(host.command("marks.next").state.reason, "当前来源不能跳转");
});

test("B21 bookmarks are kept per file across restarts; a changed file is flagged", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const movie = file(100);
  {
    const host = new TestHost();
    const { stop } = await start(host, data);
    await play(host, movie, 600, 83.44);
    await press(host, "P");
    await stop();
  }
  const saved = JSON.parse(readFileSync(join(data, "bookmarks.json"), "utf8"));
  assert.equal(saved.version, 1);
  const entry = saved.media[movie];
  assert.equal(entry.size, 100);
  assert.deepEqual(
    entry.items.map((item: { time: number; title: string }) => [item.time, item.title]),
    [[83.4, "书签 1"]],
  );
  assert.ok(Date.parse(entry.items[0].created) > 0);
  {
    const host = new TestHost();
    await start(host, data);
    await play(host, movie);
    assert.deepEqual(menuLines(host, "marks-bookmarks"), ["书签 1    01:23"]);
    await host.command("marks.list").run();
    await host.settle();
    assert.equal(host.window("marks").props.note, "");
  }
  // The same path, now another size: the bookmarks stay, with a warning.
  writeFileSync(movie, Buffer.alloc(200));
  const host = new TestHost();
  await start(host, data);
  await play(host, movie);
  await host.command("marks.list").run();
  await host.settle();
  assert.equal(host.window("marks").props.note, "文件已改变，书签时间可能不准");
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["书签 1    01:23"]);
});

test("B21 bookmarks.json: bad items are dropped one by one", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const movie = file();
  writeFileSync(
    join(data, "bookmarks.json"),
    JSON.stringify({
      version: 1,
      media: {
        [movie]: {
          size: 10,
          modified: "2026-10-01T00:00:00Z",
          items: [{ time: 5, title: "好", created: "x" }, { time: -1, title: "bad", created: "x" }, { time: 7, title: "" }, null],
        },
      },
    }),
  );
  const host = new TestHost();
  await start(host, data);
  await play(host, movie);
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["好    00:05"]);
});

// ------------------------------------------------------------------ B21 window

test("B21 window: rows by time, select and edit a bookmark, menu follows", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "Opening", start: 0 },
    { title: "Main", start: 40 },
  ];
  await start(host);
  await press(host, "H");
  const window = host.window("marks");
  assert.equal(window.props.media, false);
  assert.deepEqual(window.props.rows, []);
  await play(host, file(), 600, 20);
  await press(host, "P");
  const rows = () => window.props.rows as { rowId: string; chapter: boolean; time: string; title: string; selected: boolean }[];
  assert.deepEqual(
    rows().map((r) => [r.chapter, r.time, r.title]),
    [
      [true, "0:00.0", "Opening"],
      [false, "0:20.0", "书签 1"],
      [true, "0:40.0", "Main"],
    ],
  );
  const bookmark = rows()[1]!.rowId;
  window.emit("select", bookmark);
  await host.settle();
  assert.equal(window.props.editing, true);
  assert.equal(window.props["edit-title"], "书签 1");
  assert.equal(window.props["edit-time"], "0:00:20.0");
  window.emit("commitTitle", "高潮");
  await host.settle(WAIT);
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["高潮    00:20"]);
  window.emit("commitTitle", "   ");
  await host.settle();
  assert.equal(window.props.status, "标题不能为空");
  assert.equal(window.props["edit-title"], "高潮");
  window.emit("commitTime", "0:00:45.5");
  await host.settle(WAIT);
  assert.deepEqual(
    rows().map((r) => r.title),
    ["Opening", "Main", "高潮"],
  );
  assert.equal(rows()[2]!.selected, true);
  window.emit("commitTime", "1:2:3:4");
  await host.settle();
  assert.equal(window.props.status, "时间格式为 h:mm:ss.d");
  window.emit("commitTime", "11:00");
  await host.settle();
  assert.match(String(window.props.status), /^时间超出时长/);
  // Chapters are read only: selecting one shows no editor; double-click jumps.
  window.emit("select", rows()[0]!.rowId);
  await host.settle();
  assert.equal(window.props.editing, false);
  assert.equal(window.props["can-remove"], false);
  window.emit("activate", rows()[1]!.rowId);
  await host.settle();
  assert.deepEqual(seeks(host).at(-1), 40);
  // Search filters by title.
  window.emit("search", "高");
  await host.settle();
  assert.deepEqual(
    rows().map((r) => r.title),
    ["高潮"],
  );
  window.emit("search", "zz");
  await host.settle();
  assert.equal(window.props.filtering, true);
  assert.deepEqual(rows(), []);
});

test("B21 window: add selects the new bookmark, remove keeps the place, clear asks first", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  await press(host, "H");
  const window = host.window("marks");
  window.emit("add");
  await at(host, 30);
  window.emit("add");
  await at(host, 50);
  window.emit("add");
  await host.settle(WAIT);
  const rows = () => window.props.rows as { rowId: string; title: string; selected: boolean }[];
  assert.deepEqual(
    rows().map((r) => [r.title, r.selected]),
    [
      ["书签 1", false],
      ["书签 2", false],
      ["书签 3", true],
    ],
  );
  window.emit("select", rows()[1]!.rowId);
  window.emit("remove");
  await host.settle(WAIT);
  assert.deepEqual(
    rows().map((r) => [r.title, r.selected]),
    [
      ["书签 1", false],
      ["书签 3", true],
    ],
  );
  host.dialogAnswers.push({ confirmed: false, checked: false });
  window.emit("clear");
  await host.settle(WAIT);
  assert.equal(rows().length, 2);
  assert.equal(host.confirms.length, 1);
  assert.equal(host.confirms[0]!.destructive, true);
  host.dialogAnswers.push({ confirmed: true, checked: false });
  window.emit("clear");
  await host.settle(WAIT);
  assert.deepEqual(rows(), []);
  assert.equal(window.props.status, "已清空 2 个书签");
  assert.deepEqual(menuLines(host, "marks-bookmarks"), []);
  assert.equal(window.props["can-clear"], false);
});

test("B21 window: another source drops the selection and the typed text", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  await press(host, "P");
  await press(host, "H");
  const window = host.window("marks");
  window.emit("select", (window.props.rows as { rowId: string }[])[0]!.rowId);
  await host.settle();
  window.props["edit-title"] = "typed but not committed";
  await play(host, file());
  assert.equal(window.props.editing, false);
  assert.equal(window.props["edit-title"], "");
  assert.deepEqual(window.props.rows, []);
});

// ------------------------------------------------------------------ B23 A-B loop

test("B23: [ then ] loops, \\ stops and restarts; the core gets {a,b} → null → {a,b}", async () => {
  const host = new TestHost();
  await start(host);
  assert.equal(host.command("ab.set-a").state.reason, "未打开媒体");
  await play(host, file(), 600, 10);
  await press(host, "[");
  assert.equal(host.osd.at(-1), "A-B 起点：0:10.0");
  assert.deepEqual(loops(host), []);
  await at(host, 20.04);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "A-B 循环：0:10.0 ~ 0:20.0");
  // Set at the position B: it starts from A.
  assert.deepEqual(loops(host), [{ a: 10, b: 20 }]);
  assert.deepEqual(seeks(host), [10]);
  assert.equal(host.command("ab.toggle").state.checked, true);
  await press(host, "\\");
  assert.equal(host.osd.at(-1), "解除区段循环");
  assert.equal(host.command("ab.toggle").state.checked, false);
  await at(host, 15);
  await press(host, "\\");
  assert.deepEqual(loops(host), [{ a: 10, b: 20 }, null, { a: 10, b: 20 }]);
  assert.deepEqual(seeks(host), [10]); // inside [A, B): no jump
});

test("B23: point rules — A past B clears B, B before A refused, B without A starts at 0", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 30, 5);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "A-B 循环：0:00.0 ~ 0:05.0");
  await at(host, 4.9);
  await press(host, "[");
  assert.equal(host.osd.at(-1), "起点晚于止点，已清除止点");
  assert.equal(host.command("ab.toggle").state.enabled, false);
  assert.deepEqual(loops(host).at(-1), null);
  await at(host, 5);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "止点必须晚于起点");
  await at(host, 29.99);
  await press(host, "]");
  assert.deepEqual(loops(host).at(-1), { a: 4.9, b: 30 });
  // Nudges stay inside [0, B − 0.2] and [A + 0.2, duration].
  await press(host, "Alt+]");
  assert.equal(host.osd.at(-1), "A-B 循环：0:04.9 ~ 0:30.0");
  await press(host, "Ctrl+Alt+]");
  assert.equal(host.osd.at(-1), "已到结尾");
  await press(host, "{");
  assert.equal(host.osd.at(-1), "解除区段循环");
  assert.deepEqual(loops(host).at(-1), null);
  assert.equal(host.command("ab.a-back").state.reason, "未设定起点");
});

test("B23: held nudges are committed once, after they pause", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  const before = loops(host).length;
  for (let i = 0; i < 5; i++) await host.press("Ctrl+]");
  await host.settle();
  assert.equal(host.osd.at(-1), "A-B 循环：0:10.0 ~ 0:19.5");
  assert.equal(loops(host).length, before);
  await host.settle(WAIT);
  assert.deepEqual(loops(host).slice(before), [{ a: 10, b: 19.5 }]);
  for (let i = 0; i < 3; i++) await host.press("Ctrl+Alt+[");
  await host.settle(WAIT);
  assert.deepEqual(loops(host).at(-1), { a: 9.7, b: 19.2 });
  await press(host, "Ctrl+[");
  await press(host, "Alt+[");
  await press(host, "Alt+[");
  assert.deepEqual(loops(host).at(-1), { a: 9.8, b: 19.2 });
});

test("B23: loop count is sent and remembered; when it runs out the switch turns off", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const host = new TestHost();
  await start(host, data);
  assert.equal(host.command("ab.count.endless").state.checked, true);
  await host.command("ab.count.2").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "循环次数：2 次");
  await play(host, file(), 600, 10);
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  assert.deepEqual(loops(host).at(-1), { a: 10, b: 20, count: 2 });
  // The player clears the loop after the last jump back.
  host.setSource({ loop: null });
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "区段循环结束");
  assert.equal(host.command("ab.toggle").state.checked, false);
  assert.equal(host.command("ab.toggle").state.enabled, true); // A and B are kept
  assert.deepEqual(JSON.parse(readFileSync(join(data, "marks.json"), "utf8")), { version: 1, abCount: 2, showOnSeekbar: true });
  // Changing the count while looping re-sends the loop.
  await press(host, "\\");
  await host.command("ab.count.endless").run();
  await host.settle(WAIT);
  assert.deepEqual(loops(host).at(-1), { a: 10, b: 20 });
});

test("B23: a new source clears A and B; another plugin's loop shows and \\ stops it", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  await play(host, file());
  assert.equal(host.source.loop, null);
  assert.equal(host.command("ab.toggle").state.checked, false);
  assert.equal(host.command("ab.toggle").state.enabled, false);
  assert.equal(host.command("ab.clear-a").state.enabled, false);
  // Someone else loops 1–2.
  host.setSource({ loop: { a: 1, b: 2, count: null } });
  await host.settle(WAIT);
  assert.equal(host.command("ab.toggle").state.checked, true);
  assert.equal(host.osd.at(-1) === "区段循环结束", false);
  await press(host, "\\");
  assert.deepEqual(loops(host).at(-1), null);
  assert.equal(host.source.loop, null);
});

test("B23: Alt+\\ loops the chapter/bookmark span around the position", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 100, 30);
  await press(host, "Alt+\\");
  assert.equal(host.osd.at(-1), "没有章节或书签，无法确定区段");
  host.chapters = [
    { title: "a", start: 0 },
    { title: "b", start: 25 },
  ];
  await play(host, file(), 100, 30);
  await at(host, 60);
  await press(host, "P");
  await at(host, 40);
  await press(host, "Alt+\\");
  assert.deepEqual(loops(host).at(-1), { a: 25, b: 60 });
  assert.equal(host.osd.at(-1), "A-B 循环：0:25.0 ~ 1:00.0");
  await at(host, 70);
  await press(host, "Alt+\\");
  assert.deepEqual(loops(host).at(-1), { a: 60, b: 100 });
});

test("B23: a refused loop turns the switch off and says why", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  host.handle("player.setLoop", () => {
    throw new HostError("unsupported", "the source cannot seek");
  });
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "当前来源不支持区段循环");
  assert.equal(host.command("ab.toggle").state.checked, false);
});

// ------------------------------------------------------------------ B22 seek-bar marks

const layer = (host: TestHost) => host.timeline.layers.get(host.manifestId) ?? [];
const published = (host: TestHost) => layer(host).map((m) => [m.time, m.kind, m.title]);

test("B22 marks: chapter names, cut titles, times inside the media, the first 1000 by time", () => {
  const marks = seekbarMarks({
    chapters: [
      { title: null, start: 0 },
      { title: "长".repeat(150), start: 50 },
      { title: "Past the end", start: 700 },
    ],
    bookmarks: [{ time: 50, title: "书签 1" }],
    a: 20,
    b: null,
    duration: 600,
  });
  assert.deepEqual(
    marks.map((m) => [m.time, m.kind, [...m.title].length]),
    [
      [0, "chapter", 4],
      [20, "marker", 1],
      [50, "chapter", 100],
      [50, "bookmark", 4],
    ],
  );
  assert.equal(marks[0]!.title, "章节 1");
  // Only A set gives the marker; with B too the loop span is the player's to draw.
  assert.equal(seekbarMarks({ chapters: [], bookmarks: [], a: 20, b: 30, duration: null }).length, 0);
  const many = seekbarMarks({
    chapters: Array.from({ length: 1200 }, (_, i) => ({ title: `C${i}`, start: 1200 - i })),
    bookmarks: [],
    a: null,
    b: null,
    duration: null,
  });
  assert.equal(many.length, LAYER_MAX);
  assert.deepEqual([many[0]!.time, many.at(-1)!.time], [1, 1000]);
});

test("B22: chapters ∪ bookmarks on the seek bar, for the source they belong to", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "Opening", start: 0 },
    { title: null, start: 60 },
  ];
  await start(host);
  assert.deepEqual(published(host), []);
  await play(host, file(), 600, 30);
  assert.deepEqual(published(host), [
    [0, "chapter", "Opening"],
    [60, "chapter", "章节 2"],
  ]);
  const writes = host.sent("timeline.set");
  assert.equal(writes.at(-1)!.sourceId, host.source.stamp.generation); // ifCurrent
  await press(host, "P");
  assert.deepEqual(published(host), [
    [0, "chapter", "Opening"],
    [30, "bookmark", "书签 1"],
    [60, "chapter", "章节 2"],
  ]);
  // Only A: an `A` marker until B is set.
  await at(host, 40);
  await press(host, "[");
  assert.deepEqual(published(host).at(2), [40, "marker", "A"]);
  await at(host, 50);
  await press(host, "]");
  assert.equal(published(host).some(([, kind]) => kind === "marker"), false);
  // A file without chapters or bookmarks: nothing left over, nothing re-sent.
  host.chapters = [];
  const before = host.sent("timeline.set").length;
  await play(host, file());
  assert.deepEqual(published(host), []);
  assert.equal(host.sent("timeline.set").length, before);
  // Unchanged updates (the position moving) send nothing.
  host.chapters = [{ title: "x", start: 5 }];
  await play(host, file());
  const count = host.sent("timeline.set").length;
  await at(host, 100);
  await at(host, 101);
  assert.equal(host.sent("timeline.set").length, count);
});

test("B22: the switch empties the layer, is remembered and brings the marks back", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const host = new TestHost();
  host.chapters = [{ title: "Opening", start: 0 }];
  const media = file();
  const first = await start(host, data);
  const option = () => host.command("marks.seekbar");
  assert.equal(option().state.checked, true);
  await play(host, media, 600, 10);
  assert.equal(published(host).length, 1);
  await option().run();
  await host.settle(WAIT);
  assert.deepEqual(published(host), []);
  assert.equal(option().state.checked, false);
  assert.equal(host.osd.at(-1), "进度条上显示书签/章节标记：关");
  assert.equal(JSON.parse(readFileSync(join(data, "marks.json"), "utf8")).showOnSeekbar, false);
  await first.stop();

  const again = new TestHost();
  again.chapters = [{ title: "Opening", start: 0 }];
  await start(again, data);
  assert.equal(again.command("marks.seekbar").state.checked, false);
  await play(again, media, 600, 10);
  assert.deepEqual(published(again), []);
  await again.command("marks.seekbar").run();
  await again.settle(WAIT);
  assert.equal(again.osd.at(-1), "进度条上显示书签/章节标记：开");
  assert.deepEqual(published(again), [[0, "chapter", "Opening"]]);
});

test("B22: stopping the plugin clears its layer; other plugins' layers stay", async () => {
  const host = new TestHost();
  host.chapters = [{ title: "Opening", start: 0 }];
  const started = await start(host);
  await play(host, file(), 600, 10);
  assert.equal(published(host).length, 1);
  // Another plugin's layer stays when this one goes.
  host.setTimelineLayer("vestra.other", [{ time: 5, kind: "marker" }]);
  await started.stop();
  assert.deepEqual(published(host), []);
  assert.deepEqual(host.timeline.marks.map((m) => m.plugin), ["vestra.other"]);
});

// ------------------------------------------------------------------ menus

test("menu: orders, groups and shortcuts as in the requirements; no binding conflicts", async () => {
  const host = new TestHost();
  await start(host);
  const table = [...host.commands.values()]
    .filter((c) => c.declaration.order < 10000)
    .sort((a, b) => a.declaration.order - b.declaration.order)
    .map((c) => [c.declaration.path.slice(1).join(" ▶ "), c.declaration.order, c.declaration.group, c.declaration.bindings.join(",")]);
  assert.deepEqual(table, [
    ["章节/书签 ▶ 章节/书签列表…", 260, "marks-list", "H"],
    ["章节/书签 ▶ 添加书签", 261, "marks-edit", "P"],
    ["章节/书签 ▶ 上一书签/章节", 262, "marks-edit", "Shift+PageUp"],
    ["章节/书签 ▶ 下一书签/章节", 263, "marks-edit", "Shift+PageDown"],
    ["章节/书签 ▶ 上一章节", 264, "marks-edit", "Ctrl+PageUp"],
    ["章节/书签 ▶ 下一章节", 265, "marks-edit", "Ctrl+PageDown"],
    ["A-B 区段循环 ▶ 设定起点", 270, "ab-a", "["],
    ["A-B 区段循环 ▶ 解除起点", 271, "ab-a", "{"],
    ["A-B 区段循环 ▶ 起点后退 0.1 秒", 272, "ab-a", "Ctrl+["],
    ["A-B 区段循环 ▶ 起点前进 0.1 秒", 273, "ab-a", "Alt+["],
    ["A-B 区段循环 ▶ 设定止点", 274, "ab-b", "]"],
    ["A-B 区段循环 ▶ 解除止点", 275, "ab-b", "}"],
    ["A-B 区段循环 ▶ 止点后退 0.1 秒", 276, "ab-b", "Ctrl+]"],
    ["A-B 区段循环 ▶ 止点前进 0.1 秒", 277, "ab-b", "Alt+]"],
    ["A-B 区段循环 ▶ 区段整体后退 0.1 秒", 278, "ab-both", "Ctrl+Alt+["],
    ["A-B 区段循环 ▶ 区段整体前进 0.1 秒", 279, "ab-both", "Ctrl+Alt+]"],
    ["A-B 区段循环 ▶ 循环当前章节/书签区段", 280, "ab-toggle", "Alt+\\"],
    ["A-B 区段循环 ▶ 区段循环 开/关", 281, "ab-toggle", "\\"],
    ["A-B 区段循环 ▶ 循环次数 ▶ 无限", 282, "ab-count", ""],
    ["A-B 区段循环 ▶ 循环次数 ▶ 1 次", 283, "ab-count", ""],
    ["A-B 区段循环 ▶ 循环次数 ▶ 2 次", 284, "ab-count", ""],
    ["A-B 区段循环 ▶ 循环次数 ▶ 3 次", 285, "ab-count", ""],
    ["A-B 区段循环 ▶ 循环次数 ▶ 5 次", 286, "ab-count", ""],
    ["A-B 区段循环 ▶ 循环次数 ▶ 10 次", 287, "ab-count", ""],
  ]);
  const option = host.command("marks.seekbar").declaration;
  assert.deepEqual([option.path.slice(1).join(" ▶ "), option.order, option.group, option.bindings.join(",")], [
    "章节/书签 ▶ 在进度条上显示标记",
    30000,
    "marks-option",
    "",
  ]);
  for (const id of ["ab.a-back", "ab.a-forward", "ab.b-back", "ab.b-forward", "ab.back", "ab.forward"])
    assert.equal(host.command(id).declaration.repeat, true, id);
  assert.deepEqual(host.conflicts, []);
});
