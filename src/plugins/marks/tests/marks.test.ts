import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { Text } from "@vestra-player/sdk";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
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
async function start(host = new TestHost(), dataDirectory?: string, locale?: "zh-Hans") {
  const started = await startPlugin(activate, {
    directory,
    host,
    ...(dataDirectory ? { dataDirectory } : {}),
    ...(locale ? { locale } : {}),
  });
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
    .map((c) => c.display.path.at(-1)!);
const idFor = (host: TestHost, title: string) =>
  [...host.commands.entries()].find(([, c]) => c.display.path.at(-1) === title)![0];
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

test("menu lines are numbered, unique and fit the 80-byte path segment", async () => {
  const host = new TestHost();
  const { text } = (await start(host)).plugin.i18n;
  const shown = (lines: readonly Text[]) => lines.map((line) => host.translate(line));
  assert.deepEqual(
    shown(
      chapterLines(text, [
        { title: "Opening", start: 0 },
        { title: null, start: 75 },
        { title: "  ", start: 3725 },
      ]),
    ),
    ["01  Opening    00:00", "02  Chapter 2    01:15", "03  Chapter 3    01:02:05"],
  );
  const lines = shown(
    bookmarkLines(text, [
      { title: "高潮", number: 0, time: 83 },
      { title: "高潮", number: 0, time: 83.4 },
      { title: "长".repeat(100), number: 0, time: 5025 },
      { title: "", number: 2, time: 90 },
      { title: "", number: 2, time: 90.6 },
    ]),
  );
  assert.deepEqual(lines.slice(0, 2), ["高潮    01:23", "高潮 (2)    01:23"]);
  // Without a title: "Bookmark N"; a number used twice (one removed, one added) is told apart too.
  assert.deepEqual(lines.slice(3), ["Bookmark 2    01:30", "Bookmark 2 (2)    01:30"]);
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
  assert.deepEqual(menuLines(host, "marks-chapters"), ["01  Opening    00:00", "02  Chapter 2    00:04", "03  Main    01:00"]);
  const chapter = (i: number) => host.command(`chapter.${i}`);
  assert.equal(chapter(0).declaration.radio, true);
  assert.deepEqual([0, 1, 2].map((i) => chapter(i).state.checked), [false, true, false]);
  await chapter(2).run();
  await host.settle();
  assert.deepEqual(seeks(host), [60]);
  assert.equal(host.osd.at(-1), "Chapter: Main");
  assert.equal(chapter(2).state.checked, true);
  // Ctrl+PgUp / PgDn step between chapters only.
  await press(host, "Ctrl+PageUp");
  assert.deepEqual(seeks(host), [60, 4]);
  await press(host, "Ctrl+PageDown");
  assert.deepEqual(seeks(host), [60, 4, 60]);
  await press(host, "Ctrl+PageDown");
  assert.equal(host.osd.at(-1), "No next chapter");
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
  assert.equal(lines.at(-1), "More Chapters… (60 in all)");
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
  assert.equal(add.state.reason, "No media open");
  await play(host, file(), 600, 10);
  await press(host, "P");
  assert.equal(host.osd.at(-1), "Bookmark added: Bookmark 1 (00:10)");
  await at(host, 10.3);
  await press(host, "P");
  assert.equal(host.osd.at(-1), "A bookmark already exists here");
  await at(host, 50);
  await press(host, "P");
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["Bookmark 1    00:10", "Bookmark 2    00:50"]);
  await at(host, 52);
  await press(host, "Shift+PageUp");
  assert.deepEqual(seeks(host), [50]);
  assert.equal(host.osd.at(-1), "Bookmark: Bookmark 2");
  await press(host, "Shift+PageUp");
  assert.deepEqual(seeks(host), [50, 10]);
  await press(host, "Shift+PageUp");
  assert.equal(host.osd.at(-1), "No previous bookmark or chapter");
  await press(host, "Shift+PageDown");
  assert.deepEqual(seeks(host), [50, 10, 50]);
  // Selecting a bookmark in the menu jumps to it.
  await host.command(idFor(host, "Bookmark 1    00:10")).run();
  await host.settle();
  assert.deepEqual(seeks(host), [50, 10, 50, 10]);
});

test("B21 bookmarks: a source that cannot seek cannot take one; no media disables stepping", async () => {
  const host = new TestHost();
  await start(host);
  assert.equal(host.command("marks.previous").state.reason, "No media open");
  host.openElsewhere("https://live.example/stream.m3u8");
  await host.settle();
  host.setSource({ position: 3, capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle(WAIT);
  assert.equal(host.command("marks.add").state.enabled, false);
  assert.equal(host.command("marks.add").state.reason, "Can't add bookmarks to this source");
  assert.equal(host.command("marks.next").state.reason, "This source can't seek");
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
    entry.items.map((item: { time: number; title: string; number?: number }) => [item.time, item.title, item.number]),
    [[83.4, "", 1]],
  );
  assert.ok(Date.parse(entry.items[0].created) > 0);
  {
    const host = new TestHost();
    await start(host, data);
    await play(host, movie);
    assert.deepEqual(menuLines(host, "marks-bookmarks"), ["Bookmark 1    01:23"]);
    await host.command("marks.list").run();
    await host.settle();
    assert.equal(host.window("marks").props["file-changed"], false);
  }
  // The same path, now another size: the bookmarks stay, with a warning.
  writeFileSync(movie, Buffer.alloc(200));
  const host = new TestHost();
  await start(host, data);
  await play(host, movie);
  await host.command("marks.list").run();
  await host.settle();
  assert.equal(host.window("marks").props["file-changed"], true);
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["Bookmark 1    01:23"]);
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
          items: [
            { time: 5, title: "好", created: "x" },
            { time: 6, title: "", number: 3, created: "x" },
            { time: 8, title: "Bookmark 4", created: "x" },
            { time: -1, title: "bad", created: "x" },
            { time: 7, title: "" },
            { time: 9, title: "", created: "x" },
            { time: 10, title: "", number: 0, created: "x" },
            null,
          ],
        },
      },
    }),
  );
  const host = new TestHost();
  await start(host, data);
  await play(host, movie);
  // Untitled with its number; an old file's own "Bookmark 4" stays a title.
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["好    00:05", "Bookmark 3    00:06", "Bookmark 4    00:08"]);
});

// ------------------------------------------------------------------ B21 window

type WindowRow = { rowId: string; chapter: boolean; time: string; title: string; number: number; selected: boolean };
/** A row's name as the window shows it (MarkRow in ui/marks.slint), in English. */
const rowName = (row: WindowRow) => row.title || `${row.chapter ? "Chapter" : "Bookmark"} ${row.number}`;

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
  const rows = () => window.props.rows as WindowRow[];
  assert.deepEqual(
    rows().map((r) => [r.chapter, r.time, rowName(r)]),
    [
      [true, "0:00.0", "Opening"],
      [false, "0:20.0", "Bookmark 1"],
      [true, "0:40.0", "Main"],
    ],
  );
  const bookmark = rows()[1]!.rowId;
  window.emit("select", bookmark);
  await host.settle();
  assert.equal(window.props.editing, true);
  assert.deepEqual([rows()[1]!.title, rows()[1]!.number], ["", 1]);
  // No title yet: the input is empty, "Bookmark 1" its placeholder; left empty, it stays so.
  assert.deepEqual([window.props["edit-title"], window.props["edit-number"]], ["", 1]);
  assert.equal(window.props["edit-time"], "0:00:20.0");
  window.emit("commitTitle", " ");
  await host.settle();
  assert.equal(window.props.status, "");
  assert.equal(rows()[1]!.title, "");
  window.emit("commitTitle", "高潮");
  await host.settle(WAIT);
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["高潮    00:20"]);
  assert.equal(window.props["edit-number"], 0);
  window.emit("commitTitle", "   ");
  await host.settle();
  assert.equal(window.props.status, "empty-title");
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
  assert.equal(window.props.status, "bad-time");
  window.emit("commitTime", "11:00");
  await host.settle();
  assert.equal(window.props.status, "too-late");
  assert.equal(window.props["status-text"], "10:00.0");
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

test("B21 window: chapters read after the ready state show after settle, without a later state event", async () => {
  // A long menu rebuild: its timer must not be what refreshes the window.
  timing.menuRebuild = 5000;
  try {
    const host = new TestHost();
    host.chapters = [
      { title: "Opening", start: 0 },
      { title: "Main", start: 40 },
    ];
    await start(host);
    await press(host, "H");
    const window = host.window("marks");
    // The plugin asks for the chapters on this state; they resolve after its handler returned.
    host.openElsewhere(file());
    await host.settle(); // no wait for timers (the SDK's ~1 s safety pass)
    assert.deepEqual(
      (window.props.rows as WindowRow[]).map((r) => [r.chapter, r.time, rowName(r)]),
      [
        [true, "0:00.0", "Opening"],
        [true, "0:40.0", "Main"],
      ],
    );
  } finally {
    timing.menuRebuild = 10;
  }
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
  const rows = () => window.props.rows as WindowRow[];
  assert.deepEqual(
    rows().map((r) => [rowName(r), r.selected]),
    [
      ["Bookmark 1", false],
      ["Bookmark 2", false],
      ["Bookmark 3", true],
    ],
  );
  window.emit("select", rows()[1]!.rowId);
  window.emit("remove");
  await host.settle(WAIT);
  // The status line's argument: "Removed: {}".
  assert.equal(window.props["status-text"], "Bookmark 2");
  assert.deepEqual(
    rows().map((r) => [rowName(r), r.selected]),
    [
      ["Bookmark 1", false],
      ["Bookmark 3", true],
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
  assert.equal(window.props.status, "cleared");
  assert.equal(window.props["status-count"], 2);
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
  assert.equal(host.command("ab.set-a").state.reason, "No media open");
  await play(host, file(), 600, 10);
  await press(host, "[");
  assert.equal(host.osd.at(-1), "A-B start: 0:10.0");
  assert.deepEqual(loops(host), []);
  await at(host, 20.04);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "A-B loop: 0:10.0 ~ 0:20.0");
  // Set at the position B: it starts from A.
  assert.deepEqual(loops(host), [{ a: 10, b: 20 }]);
  assert.deepEqual(seeks(host), [10]);
  assert.equal(host.command("ab.toggle").state.checked, true);
  await press(host, "\\");
  assert.equal(host.osd.at(-1), "A-B loop off");
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
  assert.equal(host.osd.at(-1), "A-B loop: 0:00.0 ~ 0:05.0");
  await at(host, 4.9);
  await press(host, "[");
  assert.equal(host.osd.at(-1), "Start point is after the end point; end point cleared");
  assert.equal(host.command("ab.toggle").state.enabled, false);
  assert.deepEqual(loops(host).at(-1), null);
  await at(host, 5);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "End point must be after the start point");
  await at(host, 29.99);
  await press(host, "]");
  assert.deepEqual(loops(host).at(-1), { a: 4.9, b: 30 });
  // Nudges stay inside [0, B − 0.2] and [A + 0.2, duration].
  await press(host, "Alt+]");
  assert.equal(host.osd.at(-1), "A-B loop: 0:04.9 ~ 0:30.0");
  await press(host, "Ctrl+Alt+]");
  assert.equal(host.osd.at(-1), "Reached the end");
  await press(host, "{");
  assert.equal(host.osd.at(-1), "A-B loop off");
  assert.deepEqual(loops(host).at(-1), null);
  assert.equal(host.command("ab.a-back").state.reason, "Start point not set");
});

test("B23: held nudges are committed once, after they pause", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 10);
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  const before = loops(host).length;
  // Each press settles the test host (several timer rounds; one round is
  // ~16 ms on Windows), so the burst needs a pause longer than five presses.
  const loopCommit = timing.loopCommit;
  timing.loopCommit = 1000;
  try {
    for (let i = 0; i < 5; i++) await host.press("Ctrl+]");
    await host.settle();
    assert.equal(host.osd.at(-1), "A-B loop: 0:10.0 ~ 0:19.5");
    assert.equal(loops(host).length, before);
    await host.settle(timing.loopCommit + WAIT);
    assert.deepEqual(loops(host).slice(before), [{ a: 10, b: 19.5 }]);
  } finally {
    timing.loopCommit = loopCommit;
  }
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
  assert.equal(host.osd.at(-1), "Loop count: 2");
  await play(host, file(), 600, 10);
  await press(host, "[");
  await at(host, 20);
  await press(host, "]");
  assert.deepEqual(loops(host).at(-1), { a: 10, b: 20, count: 2 });
  // The player clears the loop after the last jump back.
  host.setSource({ loop: null });
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "A-B loop ended");
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
  assert.equal(host.osd.at(-1) === "A-B loop ended", false);
  await press(host, "\\");
  assert.deepEqual(loops(host).at(-1), null);
  assert.equal(host.source.loop, null);
});

test("B23: Alt+\\ loops the chapter/bookmark span around the position", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 100, 30);
  await press(host, "Alt+\\");
  assert.equal(host.osd.at(-1), "No chapters or bookmarks to define a range");
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
  assert.equal(host.osd.at(-1), "A-B loop: 0:25.0 ~ 1:00.0");
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
  assert.equal(host.osd.at(-1), "This source doesn't support A-B loop");
  assert.equal(host.command("ab.toggle").state.checked, false);
});

// ------------------------------------------------------------------ B22 seek-bar marks

const layer = (host: TestHost) => host.timeline.layers.get(host.manifestId) ?? [];
const published = (host: TestHost) => layer(host).map((m) => [m.time, m.kind, m.title]);

test("B22 marks: chapter names, cut titles, times inside the media, the first 1000 by time", async () => {
  const host = new TestHost();
  const { text } = (await start(host)).plugin.i18n;
  const marks = seekbarMarks(text, {
    chapters: [
      { title: null, start: 0 },
      { title: "长".repeat(150), start: 50 },
      { title: "Past the end", start: 700 },
    ],
    bookmarks: [
      { time: 50, title: "BM 1", number: 0 },
      { time: 60, title: "", number: 2 },
    ],
    a: 20,
    b: null,
    duration: 600,
  });
  assert.deepEqual(
    marks.map((m) => [m.time, m.kind, [...host.translate(m.title)].length]),
    [
      [0, "chapter", 9],
      [20, "marker", 1],
      [50, "chapter", 100],
      [50, "bookmark", 4],
      [60, "bookmark", 10],
    ],
  );
  assert.equal(host.translate(marks[0]!.title), "Chapter 1");
  assert.equal(host.translate(marks.at(-1)!.title), "Bookmark 2");
  // Only A set gives the marker; with B too the loop span is the player's to draw.
  assert.equal(seekbarMarks(text, { chapters: [], bookmarks: [], a: 20, b: 30, duration: null }).length, 0);
  const many = seekbarMarks(text, {
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
    [60, "chapter", "Chapter 2"],
  ]);
  const writes = host.sent("timeline.set");
  assert.equal(writes.at(-1)!.sourceId, host.source.stamp.generation); // ifCurrent
  await press(host, "P");
  assert.deepEqual(published(host), [
    [0, "chapter", "Opening"],
    [30, "bookmark", "Bookmark 1"],
    [60, "chapter", "Chapter 2"],
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
  assert.equal(host.osd.at(-1), "Bookmark and chapter marks on the seek bar: off");
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
  assert.equal(again.osd.at(-1), "Bookmark and chapter marks on the seek bar: on");
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

const fixedMenu = (host: TestHost, path: (c: ReturnType<TestHost["command"]>) => readonly unknown[]) =>
  [...host.commands.values()]
    .filter((c) => c.declaration.order < 10000)
    .sort((a, b) => a.declaration.order - b.declaration.order)
    .map((c) => [path(c).slice(1).join(" ▸ "), c.declaration.order, c.declaration.group, c.declaration.bindings.join(",")]);

test("menu: orders, groups and shortcuts as in the requirements; no binding conflicts", async () => {
  const host = new TestHost();
  await start(host);
  // English source text, in the player's Playback menu.
  assert.ok([...host.commands.values()].every((c) => c.declaration.path[0] === "@playback"));
  assert.deepEqual(fixedMenu(host, (c) => c.declaration.path), [
    ["Chapters and Bookmarks ▸ Chapter and Bookmark List…", 260, "marks-list", "H"],
    ["Chapters and Bookmarks ▸ Add Bookmark", 261, "marks-edit", "P"],
    ["Chapters and Bookmarks ▸ Previous Bookmark or Chapter", 262, "marks-edit", "Shift+PageUp"],
    ["Chapters and Bookmarks ▸ Next Bookmark or Chapter", 263, "marks-edit", "Shift+PageDown"],
    ["Chapters and Bookmarks ▸ Previous Chapter", 264, "marks-edit", "Ctrl+PageUp"],
    ["Chapters and Bookmarks ▸ Next Chapter", 265, "marks-edit", "Ctrl+PageDown"],
    ["A-B Loop ▸ Set Start Point", 270, "ab-a", "["],
    ["A-B Loop ▸ Clear Start Point", 271, "ab-a", "{"],
    ["A-B Loop ▸ Start Point Back 0.1 s", 272, "ab-a", "Ctrl+["],
    ["A-B Loop ▸ Start Point Forward 0.1 s", 273, "ab-a", "Alt+["],
    ["A-B Loop ▸ Set End Point", 274, "ab-b", "]"],
    ["A-B Loop ▸ Clear End Point", 275, "ab-b", "}"],
    ["A-B Loop ▸ End Point Back 0.1 s", 276, "ab-b", "Ctrl+]"],
    ["A-B Loop ▸ End Point Forward 0.1 s", 277, "ab-b", "Alt+]"],
    ["A-B Loop ▸ Move Range Back 0.1 s", 278, "ab-both", "Ctrl+Alt+["],
    ["A-B Loop ▸ Move Range Forward 0.1 s", 279, "ab-both", "Ctrl+Alt+]"],
    ["A-B Loop ▸ Loop Current Chapter or Bookmark Range", 280, "ab-toggle", "Alt+\\"],
    ["A-B Loop ▸ A-B Loop On/Off", 281, "ab-toggle", "\\"],
    ["A-B Loop ▸ Loop Count ▸ Endless", 282, "ab-count", ""],
    ["A-B Loop ▸ Loop Count ▸ Once", 283, "ab-count", ""],
    ["A-B Loop ▸ Loop Count ▸ 2 Times", 284, "ab-count", ""],
    ["A-B Loop ▸ Loop Count ▸ 3 Times", 285, "ab-count", ""],
    ["A-B Loop ▸ Loop Count ▸ 5 Times", 286, "ab-count", ""],
    ["A-B Loop ▸ Loop Count ▸ 10 Times", 287, "ab-count", ""],
  ]);
  assert.deepEqual(
    fixedMenu(host, (c) => c.display.path),
    fixedMenu(host, (c) => c.declaration.path),
  );
  const option = host.command("marks.seekbar").declaration;
  assert.deepEqual([option.path.slice(1).join(" ▸ "), option.order, option.group, option.bindings.join(",")], [
    "Chapters and Bookmarks ▸ Show Marks on Seek Bar",
    30000,
    "marks-option",
    "",
  ]);
  for (const id of ["ab.a-back", "ab.a-forward", "ab.b-back", "ab.b-forward", "ab.back", "ab.forward"])
    assert.equal(host.command(id).declaration.repeat, true, id);
  assert.deepEqual(host.conflicts, []);
});

/** A host playing a file with 51 chapters and 51 bookmarks: every kind of command, the "More…" lines included. */
async function withEveryCommand(locale?: "zh-Hans") {
  const data = mkdtempSync(join(scratch, "palette-"));
  const movie = file();
  const created = new Date().toISOString();
  const items = Array.from({ length: 51 }, (_, i) => ({ time: 5 + i * 10, title: `B${i}`, created }));
  const media = { [movie]: { size: 10, modified: created, items } };
  writeFileSync(join(data, "bookmarks.json"), JSON.stringify({ version: 1, media }));
  const host = new TestHost();
  host.chapters = Array.from({ length: 51 }, (_, i) => ({ title: `C${i}`, start: i * 10 }));
  await start(host, data, locale);
  await play(host, movie, 1000);
  return host;
}

test("palette: every command has a category (its submenu); listed items have the window as their home", async () => {
  const host = await withEveryCommand();
  const commands = [...host.commands.entries()];
  // 7 fixed + 50 chapters + More + 50 bookmarks + More; 12 A-B + 6 counts.
  assert.equal(commands.length, 7 + 51 + 51 + 18);
  const counts = new Map<unknown, number>();
  for (const [id, c] of commands) {
    const category = c.declaration.category;
    assert.ok(typeof category === "string" && category.length > 0, id);
    assert.equal(c.display.category, category, id);
    counts.set(category, (counts.get(category) ?? 0) + 1);
  }
  assert.deepEqual(Object.fromEntries(counts), { "Chapters and Bookmarks": 109, "A-B Loop": 12, "Loop Count": 6 });
  const homes = commands.filter(([, c]) => c.declaration.home !== undefined);
  assert.ok(homes.every(([, c]) => c.declaration.home === "marks.list"));
  assert.deepEqual(
    homes.map(([id]) => id).sort(),
    ["marks.add", ...Array.from({ length: 50 }, (_, i) => [`chapter.${i}`, `bookmark.${i}`]).flat()].sort(),
  );
});

test("palette: the categories read as the submenus in zh-Hans", async () => {
  const host = await withEveryCommand("zh-Hans");
  const shown = (id: string) => host.command(id).display.category;
  for (const id of ["marks.list", "marks.add", "marks.next", "chapter.0", "chapter.more", "bookmark.0", "bookmark.more", "marks.seekbar"])
    assert.equal(shown(id), "章节/书签", id);
  for (const id of ["ab.set-a", "ab.segment", "ab.toggle"]) assert.equal(shown(id), "A-B 区段循环", id);
  assert.equal(shown("ab.count.2"), "循环次数");
});

// ------------------------------------------------------------------ Simplified Chinese

test("zh-Hans: the menus read as before the migration", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  assert.ok([...host.commands.values()].every((c) => c.display.path[0] === "播放"));
  assert.deepEqual(fixedMenu(host, (c) => c.display.path), [
    ["章节/书签 ▸ 章节/书签列表…", 260, "marks-list", "H"],
    ["章节/书签 ▸ 添加书签", 261, "marks-edit", "P"],
    ["章节/书签 ▸ 上一书签/章节", 262, "marks-edit", "Shift+PageUp"],
    ["章节/书签 ▸ 下一书签/章节", 263, "marks-edit", "Shift+PageDown"],
    ["章节/书签 ▸ 上一章节", 264, "marks-edit", "Ctrl+PageUp"],
    ["章节/书签 ▸ 下一章节", 265, "marks-edit", "Ctrl+PageDown"],
    ["A-B 区段循环 ▸ 设定起点", 270, "ab-a", "["],
    ["A-B 区段循环 ▸ 解除起点", 271, "ab-a", "{"],
    ["A-B 区段循环 ▸ 起点后退 0.1 秒", 272, "ab-a", "Ctrl+["],
    ["A-B 区段循环 ▸ 起点前进 0.1 秒", 273, "ab-a", "Alt+["],
    ["A-B 区段循环 ▸ 设定止点", 274, "ab-b", "]"],
    ["A-B 区段循环 ▸ 解除止点", 275, "ab-b", "}"],
    ["A-B 区段循环 ▸ 止点后退 0.1 秒", 276, "ab-b", "Ctrl+]"],
    ["A-B 区段循环 ▸ 止点前进 0.1 秒", 277, "ab-b", "Alt+]"],
    ["A-B 区段循环 ▸ 区段整体后退 0.1 秒", 278, "ab-both", "Ctrl+Alt+["],
    ["A-B 区段循环 ▸ 区段整体前进 0.1 秒", 279, "ab-both", "Ctrl+Alt+]"],
    ["A-B 区段循环 ▸ 循环当前章节/书签区段", 280, "ab-toggle", "Alt+\\"],
    ["A-B 区段循环 ▸ 区段循环 开/关", 281, "ab-toggle", "\\"],
    ["A-B 区段循环 ▸ 循环次数 ▸ 无限", 282, "ab-count", ""],
    ["A-B 区段循环 ▸ 循环次数 ▸ 1 次", 283, "ab-count", ""],
    ["A-B 区段循环 ▸ 循环次数 ▸ 2 次", 284, "ab-count", ""],
    ["A-B 区段循环 ▸ 循环次数 ▸ 3 次", 285, "ab-count", ""],
    ["A-B 区段循环 ▸ 循环次数 ▸ 5 次", 286, "ab-count", ""],
    ["A-B 区段循环 ▸ 循环次数 ▸ 10 次", 287, "ab-count", ""],
  ]);
  assert.deepEqual(host.command("marks.seekbar").display.path.slice(1), ["章节/书签", "在进度条上显示标记"]);
  assert.equal(host.translate(host.command("marks.add").state.reason!), "未打开媒体");
  assert.equal(host.translate(host.command("ab.set-a").state.reason!), "未打开媒体");
});

test("zh-Hans: OSD texts and the clear dialog read as before the migration", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "Opening", start: 0 },
    { title: null, start: 4 },
  ];
  await start(host, undefined, "zh-Hans");
  await play(host, file(), 600, 10);
  await press(host, "P");
  assert.equal(host.osd.at(-1), "书签：书签 1 已添加（00:10）");
  await press(host, "P");
  assert.equal(host.osd.at(-1), "此处已有书签");
  await at(host, 12);
  await press(host, "Shift+PageUp");
  assert.equal(host.osd.at(-1), "书签：定位到 书签 1");
  await at(host, 10);
  await press(host, "Ctrl+PageUp");
  assert.equal(host.osd.at(-1), "章节定位：章节 2");
  await at(host, 4);
  await press(host, "Ctrl+PageUp");
  assert.equal(host.osd.at(-1), "章节定位：Opening");
  await at(host, 0.2);
  await press(host, "Ctrl+PageUp");
  assert.equal(host.osd.at(-1), "没有上一个章节");
  await at(host, 30);
  await press(host, "Shift+PageDown");
  assert.equal(host.osd.at(-1), "没有下一个书签/章节");
  await press(host, "[");
  assert.equal(host.osd.at(-1), "A-B 起点：0:30.0");
  await at(host, 40);
  await press(host, "]");
  assert.equal(host.osd.at(-1), "A-B 循环：0:30.0 ~ 0:40.0");
  await press(host, "\\");
  assert.equal(host.osd.at(-1), "解除区段循环");
  await press(host, "{");
  assert.equal(host.osd.at(-1), "已解除起点");
  await host.command("ab.count.3").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "循环次数：3 次");
  await host.command("ab.count.endless").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "循环次数：无限");
  await host.command("marks.seekbar").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "进度条上显示书签/章节标记：关");
  await press(host, "H");
  host.dialogAnswers.push({ confirmed: false, checked: false });
  host.window("marks").emit("clear");
  await host.settle(WAIT);
  const { message, detail, confirm } = host.confirmTexts.at(-1)!;
  assert.deepEqual([message, detail, confirm], ["清空当前文件的全部书签？", "将移除 1 个书签，不能撤销。", "清空书签"]);
});

test("zh-Hans: untitled chapters and bookmarks read as before the migration; bookmarks.json saves no words for them", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const movie = file();
  const host = new TestHost();
  host.chapters = [
    { title: "Opening", start: 0 },
    ...Array.from({ length: 60 }, (_, i) => ({ title: null, start: 5 + i })),
  ];
  await start(host, data, "zh-Hans");
  await play(host, movie, 600, 70);
  await press(host, "P");
  const chapters = menuLines(host, "marks-chapters");
  assert.deepEqual(chapters.slice(0, 2), ["01  Opening    00:00", "02  章节 2    00:05"]);
  assert.equal(chapters.at(-1), "更多章节…（共 61 项）");
  assert.deepEqual(menuLines(host, "marks-bookmarks"), ["书签 1    01:10"]);
  assert.equal(idFor(host, "书签 1    01:10"), "bookmark.0");
  assert.deepEqual(published(host).at(-1), [70, "bookmark", "书签 1"]);
  assert.deepEqual(published(host)[1], [5, "chapter", "章节 2"]);
  const saved = JSON.parse(readFileSync(join(data, "bookmarks.json"), "utf8"));
  assert.deepEqual(
    saved.media[movie].items.map((item: { title: string; number?: number }) => [item.title, item.number]),
    [["", 1]],
  );
  await press(host, "H");
  const window = host.window("marks");
  // The search finds untitled rows by the name the window shows: its
  // `@tr("Bookmark {}", "{}")` / `@tr("Chapter {}", "{}")` come with the text.
  const search = async (text: string) => {
    window.emit("search", text, host.translate("Chapter {}", ["{}"]), host.translate("Bookmark {}", ["{}"]));
    await host.settle();
    return (window.props.rows as WindowRow[]).map((r) => [r.chapter, r.number]);
  };
  assert.deepEqual(await search("书签"), [[false, 1]]);
  assert.deepEqual(await search("章节 6"), [
    [true, 6],
    [true, 60],
    [true, 61],
  ]);
  assert.deepEqual(await search("Bookmark"), []);
  assert.equal((await search("")).length, 62);
  window.emit("select", (window.props.rows as WindowRow[]).find((r) => !r.chapter)!.rowId);
  window.emit("remove");
  await host.settle(WAIT);
  assert.equal(host.translate("Removed: {}", [window.props["status-text"] as string]), "已移除：书签 1");
});
