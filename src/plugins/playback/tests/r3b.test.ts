import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { MAX_MARKS, seekMarks } from "../bar.ts";
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
    { at: 0, kind: "chapter", title: "开头", clock: "00:00" },
    { at: 0.415, kind: "bookmark", title: "精彩", clock: "01:23" },
    { at: 0.5, kind: "marker", title: "A", clock: "01:40" },
    { at: 0.75, kind: "chapter", title: "", clock: "02:30" },
  ]);
  // The marks' publisher is disabled (its layer cleared): nothing of it stays.
  host.setTimelineLayer("vestra.marks", []);
  await until(() => (bar.props.marks as unknown[]).length === 1);
  host.openElsewhere("/other.mkv");
  host.setSource({ duration: 50 });
  await until(() => (bar.props.marks as unknown[]).length === 0);
  await stop();
});

test("B22 marks need a known duration, stay inside the line, and are capped for the bar's search", () => {
  const marks = [{ time: 5, kind: "chapter" as const, title: null, plugin: "x" }];
  assert.deepEqual(seekMarks(marks, null), []);
  assert.deepEqual(seekMarks(marks, 0), []);
  assert.deepEqual(seekMarks([{ ...marks[0]!, time: 12 }], 10), [
    { at: 1, kind: "chapter", title: "", clock: "00:12" },
  ]);
  const many = Array.from({ length: MAX_MARKS + 5 }, (_, i) => ({ ...marks[0]!, time: i }));
  assert.equal(seekMarks(many, 10_000).length, MAX_MARKS);
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

test("B22 the bar draws marks with Theme colours and finds the hovered one within 3px", () => {
  const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
  assert.match(slint, /mark\.kind == "bookmark" \? Theme\.accent : mark\.kind == "marker" \? Theme\.playing : Theme\.text-dim/);
  assert.match(slint, /left <= 3px/);
  assert.match(slint, /title \+ "  " \+/);
  // 13 halvings of the lower-bound search cover MAX_MARKS.
  assert.equal(2 ** (slint.match(/= root\.narrow\(/g) ?? []).length, MAX_MARKS);
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

