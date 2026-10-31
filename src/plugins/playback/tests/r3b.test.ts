import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost, UI_VALUE_LIMITS } from "@vestra-player/sdk/testing";
import { MARK_TIP_BYTES, MARK_UNITS, MAX_MARKS, seekMarks } from "../bar.ts";
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
  const bar = host.mounted();
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
  // One Path per kind draws the ticks (x of 100000 across the line).
  assert.equal(bar.props["chapter-ticks"], "M0 0V1M75000 0V1");
  assert.equal(bar.props["bookmark-ticks"], "M41500 0V1");
  assert.equal(bar.props["marker-ticks"], "M50000 0V1");
  // The marks' publisher is disabled (its layer cleared): nothing of it stays.
  host.setTimelineLayer("vestra.marks", []);
  await until(() => (bar.props.marks as unknown[]).length === 1);
  assert.equal(bar.props["chapter-ticks"], "");
  assert.equal(bar.props["bookmark-ticks"], "");
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => (bar.props.marks as unknown[]).length === 0);
  assert.equal(bar.props["marker-ticks"], "");
  await stop();
});

test("B22 marks need a known duration, stay inside the line, and are capped at the player's array rows", () => {
  const marks = [{ time: 5, kind: "chapter" as const, title: null, plugin: "x" }];
  const none = { rows: [], chapterTicks: "", bookmarkTicks: "", markerTicks: "" };
  assert.deepEqual(seekMarks(marks, null), none);
  assert.deepEqual(seekMarks(marks, 0), none);
  assert.deepEqual(seekMarks([{ ...marks[0]!, time: 12 }], 10), {
    ...none,
    rows: [{ at: 1, tip: "00:12" }],
    chapterTicks: `M${MARK_UNITS} 0V1`,
  });
  // The cap is what one property value may hold (rows; 1 + 3 values a row).
  assert.equal(MAX_MARKS, UI_VALUE_LIMITS.arrayRows);
  assert.ok(1 + 3 * MAX_MARKS <= UI_VALUE_LIMITS.values);
  const many = Array.from({ length: MAX_MARKS + 5 }, (_, i) => ({ ...marks[0]!, time: i }));
  const shown = seekMarks(many, 10_000);
  assert.equal(shown.rows.length, MAX_MARKS);
  assert.equal(shown.chapterTicks.match(/M/g)?.length, MAX_MARKS);
});

test("B22 a kind's ticks on the same x are drawn once; the hover rows keep every mark", () => {
  const mark = (time: number, kind: "chapter" | "bookmark" | "marker") => ({ time, kind, title: null, plugin: "x" });
  // 1e-6 of the duration apart: the same x of 100000.
  const shown = seekMarks([mark(1, "chapter"), mark(1.000001, "chapter"), mark(1.000002, "bookmark"), mark(2, "chapter")], 2);
  assert.equal(shown.rows.length, 4);
  assert.equal(shown.chapterTicks, `M50000 0V1M${MARK_UNITS} 0V1`);
  assert.equal(shown.bookmarkTicks, "M50000 0V1");
  assert.equal(shown.markerTicks, "");
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
  const bar = host.mounted();
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
  const bar = host.mounted();
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
  assert.match(slint, new RegExp(`viewbox-width: ${MARK_UNITS};`));
  assert.match(slint, /left <= 3px/);
  assert.match(slint, /PlaybackBridge\.marks\[seek-bar\.near-mark\]\.tip/);
  // The lower-bound search closes a range of MAX_MARKS rows: floor(log2 n) + 1 halvings.
  assert.equal((slint.match(/= root\.narrow\(/g) ?? []).length, Math.floor(Math.log2(MAX_MARKS)) + 1);
  for (const file of ["bar.slint", "open-url.slint", "prompt.slint", "time-jump.slint"])
    assert.doesNotMatch(readFileSync(new URL(`../ui/${file}`, import.meta.url), "utf8"), /#[0-9a-fA-F]{6}/, file);
});

// ------------------------------------------------------------------ A02

test("A02 the window uses the shared UrlInputRow instead of its own input row", () => {
  const slint = readFileSync(new URL("../ui/open-url.slint", import.meta.url), "utf8");
  assert.match(slint, /import \{ UrlInputRow \} from "@vestra\/widgets\.slint"/);
  assert.match(slint, /enabled: !root\.busy/);
  assert.match(slint, /confirm-text: root\.busy \? @tr\("Opening…"\) : @tr\("Open"\)/);
  assert.doesNotMatch(slint, /TextEdit|Button/);
});

