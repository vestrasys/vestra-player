// B23 A-B Loop: moved here from the marks plugin (its tests, adapted) with the Playback panel's
// A-B Loop section (mockup 32).
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { loopTiming, precise, segmentAt } from "../loop.ts";
import activate from "../main.ts";
import { tickPath } from "../panel.ts";
import { eventually } from "./eventually.ts";

loopTiming.loopCommit = 30;
const WAIT = 60; // longer than the shortened delay

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-playback-loop-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const running = new Set<() => Promise<void>>();
async function start(host = new TestHost(), dataDirectory?: string, locale: "en" | "zh-Hans" = "en") {
  const started = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
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

let files = 0;
const file = () => `/movies/movie-${files++}.mkv`;
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
/** This plugin's seek-bar layer (the lone A marker). */
const published = (host: TestHost) => (host.timeline.layers.get(host.manifestId) ?? []).map((m) => [m.time, m.kind, m.title]);

// ------------------------------------------------------------------ helpers

test("formats and ranges: m:ss.d, the current range, the panel's chapter ticks", () => {
  assert.equal(precise(70), "1:10.0");
  assert.equal(precise(62.34), "1:02.3");
  assert.equal(precise(3723.45), "1:02:03.5");
  assert.deepEqual(segmentAt([4, 10], 5, 20), { a: 4, b: 10 });
  assert.deepEqual(segmentAt([4, 10], 2, 20), { a: 0, b: 4 });
  assert.deepEqual(segmentAt([4, 10], 12, 20), { a: 10, b: 20 });
  assert.equal(segmentAt([], 12, 20), null);
  // Ticks inside the media only (0 and the end are the track's ends), one per column.
  assert.equal(tickPath([0, 25, 50, 50.01, 100, 120], 100), "M250 0V1M500 0V1");
  assert.equal(tickPath([10], null), "");
});

// ------------------------------------------------------------------ commands (moved from marks)

test("commands: the 18 ab.* commands keep their ids, titles, keys, orders, groups and categories; the panel is their place", async () => {
  const host = new TestHost();
  await start(host);
  const rows = [...host.commands.entries()]
    .filter(([id]) => id.startsWith("ab."))
    .sort(([, x], [, y]) => x.declaration.order - y.declaration.order)
    .map(([id, c]) => [id, c.declaration.title, c.declaration.order, c.declaration.group, c.declaration.bindings.join(","), c.declaration.category]);
  assert.deepEqual(rows, [
    ["ab.set-a", "Set Start Point", 270, "ab-a", "[", "A-B Loop"],
    ["ab.clear-a", "Clear Start Point", 271, "ab-a", "{", "A-B Loop"],
    ["ab.a-back", "Start Point Back 0.1 s", 272, "ab-a", "Ctrl+[", "A-B Loop"],
    ["ab.a-forward", "Start Point Forward 0.1 s", 273, "ab-a", "Alt+[", "A-B Loop"],
    ["ab.set-b", "Set End Point", 274, "ab-b", "]", "A-B Loop"],
    ["ab.clear-b", "Clear End Point", 275, "ab-b", "}", "A-B Loop"],
    ["ab.b-back", "End Point Back 0.1 s", 276, "ab-b", "Ctrl+]", "A-B Loop"],
    ["ab.b-forward", "End Point Forward 0.1 s", 277, "ab-b", "Alt+]", "A-B Loop"],
    ["ab.back", "Move Range Back 0.1 s", 278, "ab-both", "Ctrl+Alt+[", "A-B Loop"],
    ["ab.forward", "Move Range Forward 0.1 s", 279, "ab-both", "Ctrl+Alt+]", "A-B Loop"],
    ["ab.segment", "Loop Current Chapter or Bookmark Range", 280, "ab-toggle", "Alt+\\", "A-B Loop"],
    ["ab.toggle", "A-B Loop On/Off", 281, "ab-toggle", "\\", "A-B Loop"],
    ["ab.count.endless", "Endless", 282, "ab-count", "", "Loop Count"],
    ["ab.count.1", "Once", 283, "ab-count", "", "Loop Count"],
    ["ab.count.2", "2 Times", 284, "ab-count", "", "Loop Count"],
    ["ab.count.3", "3 Times", 285, "ab-count", "", "Loop Count"],
    ["ab.count.5", "5 Times", 286, "ab-count", "", "Loop Count"],
    ["ab.count.10", "10 Times", 287, "ab-count", "", "Loop Count"],
  ]);
  for (const [id, c] of host.commands)
    if (id.startsWith("ab.")) {
      assert.deepEqual(c.declaration.path, [], `${id} is palette only`);
      assert.equal(c.declaration.home, "panel", id);
      assert.equal(c.declaration.radio, id.startsWith("ab.count."), id);
    }
  for (const id of ["ab.a-back", "ab.a-forward", "ab.b-back", "ab.b-forward", "ab.back", "ab.forward"])
    assert.equal(host.command(id).declaration.repeat, true, id);
  assert.deepEqual(host.conflicts, []);
});

test("zh-Hans: the A-B commands, categories, reasons and messages read as in the marks plugin", async () => {
  const host = new TestHost();
  await start(host, undefined, "zh-Hans");
  const shown = (id: string) => [host.command(id).display.category, host.command(id).display.title];
  assert.deepEqual(shown("ab.set-a"), ["A-B 区段循环", "设定起点"]);
  assert.deepEqual(shown("ab.segment"), ["A-B 区段循环", "循环当前章节/书签区段"]);
  assert.deepEqual(shown("ab.toggle"), ["A-B 区段循环", "区段循环 开/关"]);
  assert.deepEqual(shown("ab.count.2"), ["循环次数", "2 次"]);
  assert.equal(host.translate(host.command("ab.set-a").state.reason!), "未打开媒体");
  await play(host, file(), 600, 30);
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
});

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
  const loopCommit = loopTiming.loopCommit;
  loopTiming.loopCommit = 1000;
  try {
    for (let i = 0; i < 5; i++) await host.press("Ctrl+]");
    await host.settle();
    assert.equal(host.osd.at(-1), "A-B loop: 0:10.0 ~ 0:19.5");
    assert.equal(loops(host).length, before);
    await host.settle(loopTiming.loopCommit + WAIT);
    assert.deepEqual(loops(host).slice(before), [{ a: 10, b: 19.5 }]);
  } finally {
    loopTiming.loopCommit = loopCommit;
  }
  for (let i = 0; i < 3; i++) await host.press("Ctrl+Alt+[");
  await host.settle(WAIT);
  assert.deepEqual(loops(host).at(-1), { a: 9.7, b: 19.2 });
  await press(host, "Ctrl+[");
  await press(host, "Alt+[");
  await press(host, "Alt+[");
  assert.deepEqual(loops(host).at(-1), { a: 9.8, b: 19.2 });
});

test("B23: loop count is sent and remembered (settings.json); when it runs out the switch turns off", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const host = new TestHost();
  const first = await start(host, data);
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
  // Changing the count while looping re-sends the loop.
  await press(host, "\\");
  await host.command("ab.count.5").run();
  await host.settle(WAIT);
  assert.deepEqual(loops(host).at(-1), { a: 10, b: 20, count: 5 });
  await first.stop();
  await eventually(() => JSON.parse(readFileSync(join(data, "settings.json"), "utf8")).abCount, 5);
  // Kept for the next start; a bad saved value falls back to Endless.
  const again = new TestHost();
  await start(again, data);
  assert.equal(again.command("ab.count.5").state.checked, true);
  const bad = mkdtempSync(join(scratch, "data-"));
  writeFileSync(join(bad, "settings.json"), JSON.stringify({ abCount: 4 }));
  const third = new TestHost();
  await start(third, bad);
  assert.equal(third.command("ab.count.endless").state.checked, true);
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

test("B23: Alt+\\ loops the span between the chapters and the seek bar's bookmarks around the position", async () => {
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
  // A bookmark the marks plugin shows on the seek bar (any plugin's bookmark marks count).
  host.setTimelineLayer("vestra.marks", [
    { time: 25, kind: "chapter", title: "b" },
    { time: 60, kind: "bookmark", title: "Bookmark 1" },
  ]);
  await at(host, 40);
  await press(host, "Alt+\\");
  assert.deepEqual(loops(host).at(-1), { a: 25, b: 60 });
  assert.equal(host.osd.at(-1), "A-B loop: 0:25.0 ~ 1:00.0");
  await at(host, 70);
  await press(host, "Alt+\\");
  assert.deepEqual(loops(host).at(-1), { a: 60, b: 100 });
  // Without the bookmark layer (the marks plugin off): the chapters alone.
  host.setTimelineLayer("vestra.marks", []);
  await host.settle();
  await press(host, "Alt+\\");
  assert.deepEqual(loops(host).at(-1), { a: 25, b: 100 });
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

test("B22/B23: a lone A is an `A` marker in this plugin's seek-bar layer, for its source", async () => {
  const host = new TestHost();
  await start(host);
  await play(host, file(), 600, 30);
  assert.deepEqual(published(host), []);
  await at(host, 40);
  await press(host, "[");
  assert.deepEqual(published(host), [[40, "marker", "A"]]);
  assert.equal(host.sent("timeline.set").at(-1)!.sourceId, host.source.stamp.generation); // ifCurrent
  await press(host, "Alt+[");
  assert.deepEqual(published(host), [[40.1, "marker", "A"]]);
  // With B the loop span is the player's to draw.
  await at(host, 50);
  await press(host, "]");
  assert.deepEqual(published(host), []);
  await press(host, "}");
  assert.deepEqual(published(host), [[40.1, "marker", "A"]]);
  // Another source: nothing left over, nothing re-sent.
  const before = host.sent("timeline.set").length;
  await play(host, file());
  assert.deepEqual(published(host), []);
  assert.equal(host.sent("timeline.set").length, before);
});

// ------------------------------------------------------------------ the Playback panel's A-B Loop section

test("panel: idle, set A and B, nudge, clear, the switch, the count, the chapter range", async () => {
  const host = new TestHost();
  host.chapters = [
    { title: "a", start: 0 },
    { title: "b", start: 150 },
    { title: "c", start: 450 },
  ];
  await start(host);
  const panel = host.mounted("adjust");
  const ab = () => ({
    usable: panel.props["ab-usable"],
    reason: panel.props["ab-reason"],
    a: panel.props["ab-a"],
    b: panel.props["ab-b"],
    on: panel.props["ab-on"],
    canLoop: panel.props["ab-can-loop"],
    count: panel.props["ab-count"],
    status: panel.props["ab-status"],
  });
  // Nothing open: greyed with the reason; the keys are the commands' own (modifiers as the host labels them).
  await eventually(ab, { usable: false, reason: "media", a: "", b: "", on: false, canLoop: false, count: 0, status: "" });
  const keys = panel.props.keys as Record<string, string>;
  assert.deepEqual([keys.seta, keys.cleara, keys.setb, keys.clearb, keys.abtoggle], ["[", "{", "]", "}", "\\"]);
  for (const key of [keys.aback, keys.aforward, keys.bback, keys.bforward, keys.absegment]) assert.ok(key!.length > 1, key);
  host.rebind("ab.set-a", []);
  await eventually(() => (panel.props.keys as Record<string, string>).seta, "");
  host.rebind("ab.set-a", ["["]);

  // Open: the mini timeline has the chapter ticks and the position; A is not set yet.
  await play(host, file(), 600, 243);
  await eventually(ab, { usable: true, reason: "", a: "", b: "", on: false, canLoop: false, count: 0, status: "4:03.0" });
  await eventually(() => panel.props["ab-ticks"], "M250 0V1M750 0V1");
  assert.equal(panel.props["ab-at"], 243 / 600);
  assert.equal(panel.props["ab-span"], false);

  // "Set A here" (= [), then the time with its nudges.
  panel.emit("abSet", "a");
  await eventually(() => panel.props["ab-a"], "4:03.0");
  assert.equal(host.osd.at(-1), "A-B start: 4:03.0");
  assert.equal(panel.props["ab-a-at"], 243 / 600);
  assert.equal(panel.props["ab-status"], "");
  panel.emit("abNudge", "a", -1);
  panel.emit("abNudge", "a", -1);
  await eventually(() => panel.props["ab-a"], "4:02.8");
  assert.deepEqual(published(host), [[242.8, "marker", "A"]]);

  // "Set B here" (= ]): the loop starts at once and jumps to A; the span is coloured.
  await at(host, 289);
  panel.emit("abSet", "b");
  await eventually(() => [panel.props["ab-b"], panel.props["ab-on"], panel.props["ab-can-loop"]], ["4:49.0", true, true]);
  assert.deepEqual(loops(host).at(-1), { a: 242.8, b: 289 });
  assert.equal(seeks(host).at(-1), 242.8);
  assert.deepEqual([panel.props["ab-span"], panel.props["ab-span-a"], panel.props["ab-span-b"]], [true, 242.8 / 600, 289 / 600]);
  assert.equal(panel.props["ab-status"], "∞");

  // The count: 5 times re-sends the loop; the pass follows the jumps back.
  panel.emit("abCountChosen", 4);
  await eventually(() => panel.props["ab-count"], 4);
  assert.deepEqual(loops(host).at(-1), { a: 242.8, b: 289, count: 5 });
  assert.equal(panel.props["ab-status"], "1 / 5");
  await at(host, 260);
  await at(host, 288.8);
  await at(host, 242.9); // the player jumped back to A
  await eventually(() => panel.props["ab-status"], "2 / 5");
  assert.equal(host.command("ab.count.5").state.checked, true);

  // The switch: off keeps A and B (the span grey), on again.
  panel.emit("abToggle");
  await eventually(() => panel.props["ab-on"], false);
  assert.deepEqual(loops(host).at(-1), null);
  assert.deepEqual([panel.props["ab-span"], panel.props["ab-status"]], [true, ""]);
  panel.emit("abToggle");
  await eventually(() => panel.props["ab-on"], true);

  // A typed time: applied (the field closes), or refused (marked; the reason on the video).
  const done = panel.props.done as number;
  panel.emit("abType", "b", "5:00");
  await eventually(() => panel.props["ab-b"], "5:00.0");
  assert.equal(panel.props.done, done + 1);
  assert.deepEqual(loops(host).at(-1), { a: 242.8, b: 300, count: 5 });
  panel.emit("abType", "b", "4:00");
  await eventually(() => panel.props["ab-invalid"], "b");
  assert.equal(host.osd.at(-1), "End point must be after the start point");
  assert.equal(panel.props["ab-b"], "5:00.0");
  panel.emit("focusChanged", "b", false);
  await eventually(() => panel.props["ab-invalid"], "");
  panel.emit("abType", "a", "20:00");
  await eventually(() => panel.props["ab-invalid"], "a");
  assert.equal(host.osd.at(-1), "Enter a time within the source's duration as HH:MM:SS.mmm");
  panel.emit("abType", "a", "nonsense");
  assert.equal(panel.props["ab-a"], "4:02.8");

  // × clears a point: the loop goes off.
  panel.emit("abClear", "b");
  await eventually(() => [panel.props["ab-b"], panel.props["ab-on"], panel.props["ab-can-loop"]], ["", false, false]);
  assert.equal(host.osd.at(-1), "A-B loop off");

  // ↺ Chapter (= Alt+\): the chapter around the position, started at once.
  await at(host, 200);
  panel.emit("abSegment");
  await eventually(() => [panel.props["ab-a"], panel.props["ab-b"], panel.props["ab-on"]], ["2:30.0", "7:30.0", true]);
  assert.deepEqual(loops(host).at(-1), { a: 150, b: 450, count: 5 });
  assert.deepEqual(host.conflicts, []);
});

test("panel: a source that cannot loop greys the section with its reason", async () => {
  const host = new TestHost();
  await start(host);
  const panel = host.mounted("adjust");
  host.openElsewhere(file());
  host.setSource({ duration: 100, position: 5, capabilities: { ...host.source.capabilities, seek: false } });
  await host.settle(WAIT);
  await eventually(() => [panel.props["ab-usable"], panel.props["ab-reason"]], [false, "source"]);
  panel.emit("abSet", "a");
  await host.settle(WAIT);
  assert.equal(panel.props["ab-a"], "");
  assert.deepEqual(loops(host), []);
});

test("panel: a point's field that goes with a new source leaves no keyboard hold behind", async () => {
  const host = new TestHost();
  await start(host);
  const panel = host.mounted("adjust");
  await play(host, file(), 600, 30);
  panel.emit("abSet", "a");
  await eventually(() => panel.props["ab-a"], "0:30.0");
  const focused = () => (panel.locks as { focused?: boolean } | null)?.focused;
  panel.emit("focusChanged", "a", true);
  await eventually(focused, true);
  // Another file: A is gone, and so is its field (no blur comes from it).
  await play(host, file());
  await eventually(() => [panel.props["ab-a"], focused()], ["", false]);
});
