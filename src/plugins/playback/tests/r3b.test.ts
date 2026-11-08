import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost, UI_VALUE_LIMITS } from "@vestra-player/sdk/testing";
import { jsonBytes, MARK_TIP_BYTES, MAX_MARKS, sampled, seekMarks, seekRanges, TICK_BYTES } from "../bar.ts";
import activate from "../main.ts";

// I07 (the seek-bar preview) is in preview.test.ts since 0.2.0.
const directory = new URL("..", import.meta.url);
const start = (host = new TestHost()) =>
  // The Chinese texts are what the plugin showed before English became the source.
  startPlugin(activate, { directory, host, locale: "zh-Hans" });

// A ceiling, not a delay: generous because the whole suite runs in parallel.
async function until(predicate: () => boolean, ms = 5000) {
  const deadline = Date.now() + ms;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
function withMedia(host: TestHost, source: string, duration = 1000) {
  host.openElsewhere(source);
  host.setSource({ duration, position: 0 });
  return host;
}

// ------------------------------------------------------------------ B22

test("B22 the seek line gets everyone's marks as fractions with their time text; a new source clears them", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted("bottom");
  assert.deepEqual(bar.props.marks, []);
  host.setTimelineLayer("vestra.marks", [
    { time: 0, kind: "chapter", title: "开头" },
    { time: 83, kind: "bookmark", title: "精彩" },
    { time: 150, kind: "chapter" },
  ]);
  host.setTimelineLayer("community.other", [{ time: 100, kind: "marker", title: "A" }]);
  await until(() => (bar.props.marks as unknown[]).length === 4);
  assert.deepEqual(bar.props.marks, [
    { at: 0, tip: "开头  00:00" },
    { at: 0.415, tip: "精彩  01:23" },
    { at: 0.5, tip: "A  01:40" },
    { at: 0.75, tip: "02:30" },
  ]);
  // No ticks until the bar reports its span; then one Path per kind, x the pixel column.
  assert.equal(bar.props["chapter-ticks"], "");
  assert.equal(bar.props["tick-span"], 0);
  bar.emit("spanChanged", 400);
  await until(() => bar.props["tick-span"] === 400);
  assert.equal(bar.props["chapter-ticks"], "M0 0V1M300 0V1");
  assert.equal(bar.props["bookmark-ticks"], "M166 0V1");
  assert.equal(bar.props["marker-ticks"], "M200 0V1");
  // A resize: the ticks follow the new span (whole physical px) once the reports pause.
  bar.emit("spanChanged", 250.5);
  await until(() => bar.props["tick-span"] === 250);
  assert.equal(bar.props["chapter-ticks"], "M0 0V1M187 0V1");
  assert.equal(bar.props["bookmark-ticks"], "M103 0V1");
  // The marks' publisher is disabled (its layer cleared): nothing of it stays.
  host.setTimelineLayer("vestra.marks", []);
  await until(() => (bar.props.marks as unknown[]).length === 1);
  // Properties are written one at a time, `marks` before the ticks: let the rest arrive.
  await host.settle();
  assert.equal(bar.props["chapter-ticks"], "");
  assert.equal(bar.props["bookmark-ticks"], "");
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => (bar.props.marks as unknown[]).length === 0);
  await host.settle(); // the ticks come after `marks`
  assert.equal(bar.props["marker-ticks"], "");
  await stop();
});

test("B22 span reports during a resize are debounced: the first at once, then the last after a pause", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted("bottom");
  host.setTimelineLayer("vestra.marks", [{ time: 100, kind: "bookmark", title: null }]);
  await until(() => (bar.props.marks as unknown[]).length === 1);
  bar.emit("spanChanged", 400);
  await until(() => bar.props["tick-span"] === 400);
  const sent = () => host.mounted("bottom").props["bookmark-ticks"];
  assert.equal(sent(), "M200 0V1");
  // A drag of the window edge: a report a frame.
  for (const px of [401, 405, 410, 420, 431]) {
    bar.emit("spanChanged", px);
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
  assert.equal(bar.props["tick-span"], 400, "nothing rebuilt while the reports keep coming");
  assert.equal(sent(), "M200 0V1");
  await until(() => bar.props["tick-span"] === 431);
  assert.equal(sent(), "M215 0V1");
  // Stopped with a report pending: nothing runs after.
  bar.emit("spanChanged", 500);
  await stop();
});

test("B22 marks need a known duration, stay inside the line, and are capped at the player's array rows", () => {
  const marks = [{ time: 5, kind: "chapter" as const, title: null, plugin: "x" }];
  const none = { rows: [], chapterTicks: "", bookmarkTicks: "", markerTicks: "", tickSpan: 0 };
  assert.deepEqual(seekMarks(marks, null, 500), none);
  assert.deepEqual(seekMarks(marks, 0, 500), none);
  // Before the bar reports its span: the hover rows, no ticks.
  assert.deepEqual(seekMarks([{ ...marks[0]!, time: 12 }], 10), { ...none, rows: [{ at: 1, tip: "00:12" }] });
  assert.deepEqual(seekMarks([{ ...marks[0]!, time: 12 }], 10, 500), {
    ...none,
    rows: [{ at: 1, tip: "00:12" }],
    chapterTicks: "M500 0V1",
    tickSpan: 500,
  });
  // The cap fits in one property value (rows; 1 + 3 values a row). It is the
  // bar's own (13 halvings of the search); SDK 1.5.46 raised the host's rows.
  assert.ok(MAX_MARKS <= UI_VALUE_LIMITS.arrayRows);
  assert.ok(1 + 3 * MAX_MARKS <= UI_VALUE_LIMITS.values);
  const many = Array.from({ length: MAX_MARKS + 5 }, (_, i) => ({ ...marks[0]!, time: i }));
  const shown = seekMarks(many, 10_000, 100_000);
  assert.equal(shown.rows.length, MAX_MARKS);
  assert.equal(shown.chapterTicks.match(/M/g)?.length, MAX_MARKS);
  // Past the cap the marks are sampled across the whole duration, not cut at the end.
  assert.deepEqual(shown.rows.at(-1), { at: (MAX_MARKS + 4) / 10_000, tip: "1:08:20" });
});

type Kind = "chapter" | "bookmark" | "marker";
const timed = (count: number, kind: (i: number) => Kind) =>
  Array.from({ length: count }, (_, i) => ({ time: i, kind: kind(i), title: null, plugin: "x" }));

test("B22 more marks than the cap: picked evenly by index, first and last kept, in time order", () => {
  const all = timed(5000, () => "marker");
  assert.equal(sampled(all.slice(0, MAX_MARKS)).length, MAX_MARKS);
  const picked = sampled(all).map((mark) => mark.time);
  assert.equal(picked.length, MAX_MARKS);
  assert.equal(picked[0], 0);
  assert.equal(picked.at(-1), 4999);
  assert.ok(picked.every((value, i) => i === 0 || value > picked[i - 1]!), "sorted, no repeats");
  // Even: every quarter of the marks keeps about a quarter of the picks.
  for (let q = 0; q < 4; q++) {
    const inQuarter = picked.filter((v) => v >= q * 1250 && v < (q + 1) * 1250).length;
    assert.ok(Math.abs(inQuarter - MAX_MARKS / 4) <= 1, `quarter ${q}: ${inQuarter}`);
  }
});

test("B22 past the cap the user's bookmarks are kept first; the other kinds are sampled into the rest", () => {
  // 6000 marks, every 20th a bookmark (300), the rest chapters and markers.
  const all = timed(6000, (i) => (i % 20 === 0 ? "bookmark" : i % 2 ? "chapter" : "marker"));
  const picked = sampled(all);
  assert.equal(picked.length, MAX_MARKS);
  assert.equal(picked.filter((mark) => mark.kind === "bookmark").length, 300);
  assert.ok(picked.every((mark, i) => i === 0 || mark.time > picked[i - 1]!.time), "time order");
  assert.equal(picked.at(-1)!.time, 5999, "the others still reach the end");
  // More bookmarks than the cap: bookmarks only, sampled evenly.
  const bookmarks = sampled(timed(6000, (i) => (i % 5 ? "bookmark" : "chapter")));
  assert.equal(bookmarks.length, MAX_MARKS);
  assert.ok(bookmarks.every((mark) => mark.kind === "bookmark"));
  assert.equal(bookmarks[0]!.time, 1);
  assert.equal(bookmarks.at(-1)!.time, 5999);
});

test("B22 a kind's ticks in the same pixel column are drawn once; the hover rows keep every mark", () => {
  const mark = (time: number, kind: "chapter" | "bookmark" | "marker") => ({ time, kind, title: null, plugin: "x" });
  // 0.3px apart on a 600px line: the same column.
  const shown = seekMarks([mark(1, "chapter"), mark(1.001, "chapter"), mark(1.002, "bookmark"), mark(2, "chapter")], 2, 600);
  assert.equal(shown.rows.length, 4);
  assert.equal(shown.chapterTicks, "M300 0V1M600 0V1");
  assert.equal(shown.bookmarkTicks, "M300 0V1");
  assert.equal(shown.markerTicks, "");
});

/**
 * Where Slint 1.18.1 draws a MarkTicks tick, in the window's physical px, at
 * scale factor `s`. The element's geometry is `bar.slint`'s MarkTicks,
 * written out (1phx = 1/s px); `Path::fitted_path_events` stretches the
 * viewbox into the element less the stroke width and offsets it by half the
 * stroke; the software renderer (`draw_path`, `convert_path_data_to_zeno`)
 * maps a point to (element's window position + offset + point) × s,
 * unrounded. `origin` is the seek bar's `absolute-position`, `inset` where
 * the line's box starts in it (0.2.10: `lead` 10px, `line-top` 6px; before:
 * none, the seek bar was the line's box).
 */
function drawnTick(
  x: number,
  s: number,
  origin: { x: number; y: number },
  columns: number,
  tickSpan: number,
  inset = { x: 10, y: 6 },
) {
  const phx = 1 / s;
  const rows = Math.round(6 / phx);
  const element = {
    x: Math.round((origin.x + inset.x + 4) / phx) * phx - origin.x,
    y: Math.round((origin.y + inset.y + 2) / phx) * phx - 0.5 * phx - origin.y,
    width: (columns + 1) * phx,
    height: (rows + 1) * phx,
  };
  const stroke = phx;
  const left = (origin.x + element.x) * s;
  const centre = (origin.x + element.x + stroke / 2 + (x * (element.width - stroke)) / tickSpan) * s;
  const top = (origin.y + element.y + stroke / 2) * s;
  const bottom = (origin.y + element.y + stroke / 2 + (element.height - stroke)) * s;
  return { left, centre, top, bottom, rows };
}

const near = (value: number, target: number) => Math.abs(value - target) < 1e-6;

test("B22 at 100/125/150/175/200% each tick fills one whole physical pixel column and whole rows", () => {
  const mark = (time: number) => ({ time, kind: "marker" as const, title: null, plugin: "x" });
  // A duration of 1024 s: every `at` is exact, so no column depends on a rounding of the last bit.
  const times = [...Array.from({ length: 103 }, (_, i) => i * 10 - 3).slice(1), 1024];
  // The line's box sits at 15px, 6px in the bar; the bar where the window puts it (fractional too).
  const lines = [{ x: 15, y: 6 }, { x: 15, y: 433.6 }, { x: 15.4, y: 6.2 }, { x: 15.8, y: 917.75 }];
  for (const s of [1, 1.25, 1.5, 1.75, 2])
    for (const line of lines)
      for (const span of [100, 467, 512.75, 1203.5]) {
        // 0.2.10: the seek bar is the pointer band, 10px left of and 6px above the line's box.
        const origin = { x: line.x - 10, y: line.y - 6 };
        const where = `scale ${s}, origin ${origin.x},${origin.y}, span ${span}px`;
        // What the bar reports: the span in whole physical px.
        const columns = Math.floor(span * s);
        const shown = seekMarks(times.map(mark), 1024, columns);
        assert.equal(shown.tickSpan, columns);
        const xs = [...shown.markerTicks.matchAll(/M(\d+) 0V1/g)].map((m) => Number(m[1]));
        const wanted = new Set(times.map((t) => Math.floor(Math.min(1, t / 1024) * columns)));
        assert.equal(xs.length, wanted.size, `${where}: one tick per physical column`);
        for (const x of xs) {
          assert.ok(Number.isInteger(x) && wanted.has(x), `${where}: x ${x}`);
          const tick = drawnTick(x, s, origin, columns, shown.tickSpan);
          // The line's left edge is on a physical pixel; the 1 physical px stroke is
          // centred on edge + x + 0.5: exactly column edge + x, no antialiased halves.
          assert.ok(near(tick.left, Math.round(tick.left)), `${where}: left ${tick.left}`);
          assert.ok(near(tick.centre, Math.round(tick.left) + x + 0.5), `${where}: x ${x} centre ${tick.centre}`);
          // Butt caps: from a pixel row down over whole rows, 6 logical px rounded.
          assert.ok(near(tick.top, Math.round(tick.top)), `${where}: top ${tick.top}`);
          assert.ok(near(tick.bottom - tick.top, tick.rows), `${where}: rows`);
          // Within a physical px of where the logical layout puts the line (4px…4px + span).
          assert.ok(Math.abs(tick.left - (line.x + 4) * s) <= 0.5 + 1e-9, `${where}: left at the inset`);
          assert.ok(tick.centre <= (line.x + 4 + span) * s + 1, `${where}: inside the line`);
          // 0.2.10 draws every tick exactly where 0.2.9 did (its seek bar was the line's box).
          const before = drawnTick(x, s, line, columns, shown.tickSpan, { x: 0, y: 0 });
          for (const key of ["left", "centre", "top", "bottom"] as const)
            assert.ok(near(tick[key], before[key]), `${where}: ${key} ${tick[key]} was ${before[key]}`);
        }
      }
});

test("B22 a kind's tick string stays under its cap however wide the line", () => {
  const many = Array.from({ length: MAX_MARKS }, (_, i) => ({ time: i, kind: "chapter" as const, title: null, plugin: "x" }));
  // Absurdly wide, so 4096 ticks of up to 18 bytes would pass the cap.
  const shown = seekMarks(many, MAX_MARKS, 1e12);
  assert.ok(shown.chapterTicks.length > TICK_BYTES - 20, "the cap is reached");
  assert.ok(shown.chapterTicks.length <= TICK_BYTES);
  assert.ok(TICK_BYTES <= UI_VALUE_LIMITS.stringBytes);
  assert.equal(shown.rows.length, MAX_MARKS);
});

test("B22 the tip budget counts JSON escapes: a control character as 6 bytes (\\u00XX)", () => {
  assert.equal(jsonBytes("ab"), 6);
  assert.equal(jsonBytes("长"), 3);
  assert.equal(jsonBytes("😀"), 6);
  assert.equal(jsonBytes('"\\'), 6);
  assert.equal(jsonBytes("a\u0001"), 3 + 6);
  assert.equal(jsonBytes("\ud800"), 6);
  for (const text of ["a\u0001\u001f", "\ud800x", "x\udc00", '"\\\n长😀'])
    assert.ok(Buffer.byteLength(JSON.stringify(text)) - 2 <= jsonBytes(text), JSON.stringify(text));
  // Titles of control characters take twice the budget of plain ones.
  const title = "\u0001".repeat(100);
  const many = Array.from({ length: MAX_MARKS }, (_, i) => ({ time: i, kind: "marker" as const, title, plugin: "x" }));
  const titled = seekMarks(many, MAX_MARKS).rows.filter((row) => (row as { tip: string }).tip.startsWith(title)).length;
  assert.equal(titled, Math.floor(MARK_TIP_BYTES / (6 * 100 + 3 * (2 + 5))));
});

test("B22 the hover titles stay within their text budget; past it a tip is the time alone", () => {
  const title = "长".repeat(100);
  const many = Array.from({ length: MAX_MARKS }, (_, i) => ({ time: i, kind: "marker" as const, title, plugin: "x" }));
  const { rows } = seekMarks(many, MAX_MARKS);
  const tips = rows.map((row) => (row as { tip: string }).tip);
  const titled = tips.filter((tip) => tip.startsWith(title)).length;
  assert.equal(titled, Math.floor(MARK_TIP_BYTES / (3 * (100 + 2 + 5))));
  assert.match(tips[titled - 1]!, /^长+ {2}\d\d:\d\d$/);
  assert.match(tips[titled]!, /^\d\d:\d\d$/);
  assert.equal(tips.at(-1), "1:08:15");
  assert.ok(tips.reduce((sum, tip) => sum + Buffer.byteLength(tip), 0) <= MARK_TIP_BYTES);
});

test("B22 4096 marks with 100-character titles reach the bar within the player's limits", async () => {
  // Five publishers of 1000 marks (a layer's most): more than the bar shows.
  // Before 0.2.6 the bar's `marks` (4 fields a row) was refused from about 3277 marks.
  const host = withMedia(new TestHost(), "/movie.mkv", 10_000);
  const { stop } = await start(host);
  const bar = host.mounted("bottom");
  const title = `"长"${"标".repeat(96)}\\`;
  for (let layer = 0; layer < 5; layer++)
    host.setTimelineLayer(
      `p${layer}.marks`,
      Array.from({ length: 1000 }, (_, i) => ({
        time: layer * 1000 + i + 0.5,
        kind: (["chapter", "bookmark", "marker"] as const)[i % 3]!,
        title,
      })),
    );
  await until(() => (bar.props.marks as unknown[]).length === MAX_MARKS);
  for (const kind of ["chapter", "bookmark", "marker"]) {
    const ticks = bar.props[`${kind}-ticks`] as string;
    assert.ok(Buffer.byteLength(ticks) <= UI_VALUE_LIMITS.stringBytes, kind);
  }
  // The bar keeps updating (a refused write would leave the old values).
  host.setSource({ position: 125 });
  await until(() => bar.props["elapsed-label"] === "02:05");
  await stop();
});

test("B22 the A-B loop is drawn whoever set it, and goes with the source", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted("bottom");
  assert.equal(bar.props["has-loop"], false);
  host.setSource({ loop: { a: 20, b: 50, count: null } });
  await until(() => bar.props["has-loop"] === true);
  assert.equal(bar.props["loop-a"], 0.1);
  assert.equal(bar.props["loop-b"], 0.25);
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => bar.props["has-loop"] === false);
  await stop();
});

test("B22 the bar draws marks as one Path per kind in Theme colours and finds the hovered one within 3px", () => {
  const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
  // No element per mark: three Paths take the plugin's commands.
  assert.doesNotMatch(slint, /for \w+ in PlaybackBridge\.marks/);
  assert.match(slint, /commands: PlaybackBridge\.chapter-ticks;\s+stroke: Theme\.text-dim;/);
  assert.match(slint, /commands: PlaybackBridge\.marker-ticks;\s+stroke: Theme\.playing;/);
  assert.match(slint, /commands: PlaybackBridge\.bookmark-ticks;\s+stroke: Theme\.accent;/);
  // The geometry `drawnTick` models, in physical px: snapped origin, scale 1phx a unit, 1phx stroke.
  const geometry = [
    "private property <int> rows: Math.round(6px / 1phx);",
    "x: Math.round((self.origin-x + self.inset-x + 4px) / 1phx) * 1phx - self.origin-x;",
    "y: Math.round((self.origin-y + self.inset-y + 2px) / 1phx) * 1phx - 0.5phx - self.origin-y;",
    "width: (self.columns + 1) * 1phx;",
    "height: (self.rows + 1) * 1phx;",
  ];
  const ticks = slint.slice(slint.indexOf("component MarkTicks"), slint.indexOf("component SeekBar"));
  for (const line of geometry) assert.ok(ticks.includes(line), line);
  assert.match(ticks, /viewbox-width: PlaybackBridge\.tick-span > 0 \? PlaybackBridge\.tick-span : max\(1, self\.columns\);\s+viewbox-height: 1;\s+fit: fill;\s+stroke-width: 1phx;/);
  // The bar reports its span in whole physical px at start, on a change, and when ticks come for another span.
  assert.ok(slint.includes("private property <int> columns: Math.floor(root.span / 1phx);"));
  assert.ok(slint.includes("init => { PlaybackBridge.span-changed(root.columns); }"));
  assert.ok(slint.includes("changed columns => { PlaybackBridge.span-changed(self.columns); }"));
  assert.match(slint, /changed stale-ticks => /);
  for (const kind of ["chapter", "marker", "bookmark"])
    assert.match(
      slint,
      new RegExp(
        `origin-x: root\\.absolute-position\\.x;\\s+origin-y: root\\.absolute-position\\.y;\\s+inset-x: root\\.lead;\\s+inset-y: root\\.line-top;\\s+columns: root\\.columns;\\s+commands: PlaybackBridge\\.${kind}-ticks;`,
      ),
    );
  assert.match(slint, /left <= 3px/);
  assert.match(slint, /PlaybackBridge\.marks\[seek-bar\.near-mark\]\.tip/);
  // The lower-bound search closes a range of MAX_MARKS rows: floor(log2 n) + 1 halvings.
  assert.equal((slint.match(/= root\.narrow\(/g) ?? []).length, Math.floor(Math.log2(MAX_MARKS)) + 1);
  for (const file of ["bar.slint", "open-url.slint", "panel.slint"])
    assert.doesNotMatch(readFileSync(new URL(`../ui/${file}`, import.meta.url), "utf8"), /#[0-9a-fA-F]{6}/, file);
});

// ------------------------------------------------------------------ timeline ranges (0.2.17)

const range = (start: number, end: number, style: "selection" | "region", title: string | null = null) => ({
  start,
  end,
  style,
  title,
  plugin: "x",
});

test("Ranges: one filled rectangle per run of physical columns, in the ticks' view box", () => {
  // 1000 s over 400 columns: 2.5 s a column.
  const shown = seekRanges([range(100, 200, "selection"), range(500, 750, "region")], 1000, 400);
  assert.equal(shown.selectionRanges, "M40 0H80V1H40Z");
  assert.equal(shown.regionRanges, "M200 0H300V1H200Z");
  // Partial columns are covered (floor at the start, ceil at the end); a tiny range still gets a column.
  assert.equal(seekRanges([range(101, 199, "selection")], 1000, 400).selectionRanges, "M40 0H80V1H40Z");
  assert.equal(seekRanges([range(500, 500.1, "region")], 1000, 400).regionRanges, "M200 0H201V1H200Z");
  // A range at the very end stays inside the line; one past the duration is clamped.
  assert.equal(seekRanges([range(999.9, 1000, "selection")], 1000, 400).selectionRanges, "M399 0H400V1H399Z");
  assert.equal(seekRanges([range(900, 1200, "selection")], 1000, 400).selectionRanges, "M360 0H400V1H360Z");
});

test("Ranges: both styles have their own Path; overlapping or touching ranges of a style are one rectangle", () => {
  const shown = seekRanges(
    [
      range(300, 400, "selection"),
      range(0, 100, "region"),
      range(50, 150, "region"),
      range(150, 200, "region"),
      range(600, 700, "region"),
      range(320, 330, "selection"),
    ],
    1000,
    100,
  );
  // Out of start order on input, still merged: 0–15 and 15–20 touch, 60–70 stands alone.
  assert.equal(shown.regionRanges, "M0 0H20V1H0ZM60 0H70V1H60Z");
  assert.equal(shown.selectionRanges, "M30 0H40V1H30Z");
  // A style the bar does not know is not drawn.
  const unknown = {
    ...range(0, 500, "region"),
    style: "other",
  } as unknown as ReturnType<typeof range>;
  assert.deepEqual(seekRanges([unknown], 1000, 100), {
    selectionRanges: "",
    regionRanges: "",
  });
});

test("Ranges: none drawn without ranges, a known duration or the bar's span", () => {
  const none = { selectionRanges: "", regionRanges: "" };
  assert.deepEqual(seekRanges([], 1000, 400), none);
  assert.deepEqual(seekRanges([range(0, 10, "selection")], null, 400), none);
  assert.deepEqual(seekRanges([range(0, 10, "selection")], 0, 400), none);
  assert.deepEqual(seekRanges([range(0, 10, "selection")], 1000, 0), none);
});

test("Ranges: the bar follows everyone's ranges, and a new source clears them", async () => {
  const host = withMedia(new TestHost(), "/movie.mkv", 200);
  const { stop } = await start(host);
  const bar = host.mounted("bottom");
  assert.equal(bar.props["selection-ranges"], "");
  assert.equal(bar.props["region-ranges"], "");
  bar.emit("spanChanged", 400);
  await until(() => bar.props["tick-span"] === 400);
  // No marks at all: ranges alone are drawn.
  host.setTimelineRanges("vestra.capture", [{ start: 50, end: 100, style: "selection", title: "Clip" }]);
  host.setTimelineRanges("community.other", [{ start: 0, end: 20, style: "region" }]);
  await until(() => bar.props["selection-ranges"] === "M100 0H200V1H100Z");
  await until(() => bar.props["region-ranges"] === "M0 0H40V1H0Z");
  // The publisher changes its ranges, then clears them.
  host.setTimelineRanges("vestra.capture", [{ start: 150, end: 200, style: "selection" }]);
  await until(() => bar.props["selection-ranges"] === "M300 0H400V1H300Z");
  host.setTimelineRanges("vestra.capture", []);
  await until(() => bar.props["selection-ranges"] === "");
  assert.equal(bar.props["region-ranges"], "M0 0H40V1H0Z");
  // A resize rebuilds them for the new span.
  bar.emit("spanChanged", 200);
  await until(() => bar.props["region-ranges"] === "M0 0H20V1H0Z");
  // Another source clears every layer.
  host.setTimelineRanges("vestra.capture", [{ start: 50, end: 100, style: "selection" }]);
  await until(() => bar.props["selection-ranges"] !== "");
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => bar.props["selection-ranges"] === "" && bar.props["region-ranges"] === "");
  await stop();
});

test("Ranges: the bar draws them as one Path per style in Theme colours, between the line and the ticks", () => {
  const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
  assert.doesNotMatch(slint, /for \w+ in PlaybackBridge\.\w+-ranges/);
  assert.match(slint, /commands: PlaybackBridge\.region-ranges;\s+fill: Theme\.text-dim\.with-alpha\(0\.\d+\);/);
  assert.match(slint, /commands: PlaybackBridge\.selection-ranges;\s+fill: Theme\.accent\.with-alpha\(0\.\d+\);/);
  const seekBar = slint.slice(slint.indexOf("component SeekBar"));
  const at = (text: string) => seekBar.indexOf(text);
  assert.ok(
    at("background: PlaybackBridge.can-seek ? Theme.accent-fill") < at("commands: PlaybackBridge.region-ranges"),
  );
  assert.ok(at("commands: PlaybackBridge.region-ranges") < at("commands: PlaybackBridge.selection-ranges"));
  assert.ok(at("commands: PlaybackBridge.selection-ranges") < at("commands: PlaybackBridge.chapter-ticks"));
  // No stroke, so the view box fills the element: `columns` physical px over `columns` units, on whole pixels.
  const band = slint.slice(slint.indexOf("component RangeBand"), slint.indexOf("component SeekBar"));
  assert.doesNotMatch(band, /stroke(-width)?:/);
  for (const line of [
    "x: Math.round((self.origin-x + self.inset-x + 4px) / 1phx) * 1phx - self.origin-x;",
    "y: Math.round((self.origin-y + self.inset-y + 2px) / 1phx) * 1phx - self.origin-y;",
    "width: max(1, self.columns) * 1phx;",
    "height: self.rows * 1phx;",
  ])
    assert.ok(band.includes(line), line);
  // A range alone (no marks) also asks for the span again when it was built for another one.
  assert.match(slint, /stale-ticks: \(PlaybackBridge\.marks\.length > 0 \|\| PlaybackBridge\.selection-ranges != ""/);
});

// ------------------------------------------------------------------ A02

test("A02 the window uses the shared UrlInputRow instead of its own input row", () => {
  const slint = readFileSync(new URL("../ui/open-url.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{ UrlInputRow \} from "@vestra\/widgets\.slint"/);
  assert.match(slint, /enabled: !root\.busy/);
  assert.match(slint, /confirm-text: root\.busy \? @tr\("Opening…"\) : @tr\("Open"\)/);
  assert.doesNotMatch(slint, /TextEdit|Button/);
});

