import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

/**
 * 0.2.10 (user request): the seek line is easier to hit. The pointer area
 * (hover, time tip and preview, press and drag) is the whole 22px seek row,
 * as PotPlayer's, not the 10px around the 2px line; the drawing is unchanged.
 * 0.2.11 (user request): the pointer area also reaches 4px above the bar's
 * top edge, over the bottom of the video; not below the seek row.
 * The test host does not run Slint: the geometry is `bar.slint`'s, written
 * out below and checked against the source; the plugin side is driven with
 * the callbacks the band sends at its edges.
 */
const directory = new URL("..", import.meta.url);
const slint = readFileSync(new URL("../ui/bar.slint", import.meta.url), "utf8");
const seekBar = slint.slice(slint.indexOf("component SeekBar"), slint.indexOf("component MuteButton"));
const row = slint.slice(slint.indexOf("seek-row := Rectangle"), slint.indexOf("\n    row := Rectangle {"));

/** `bar.slint` in numbers for a bar `width` px wide (the seek row is y 0…22). */
function layout(width: number) {
  const lead = 10;
  const trail = 12;
  const lineTop = 6;
  const band = { x: 5, y: 0, width: Math.max(0, width - 110), height: 22 };
  const reach = 4;
  const span = Math.max(1, band.width - lead - trail - 8);
  return {
    band,
    /** The pointer area (the TouchArea): the band and `reach` above it, in the seek row's y. */
    touch: { x: band.x, y: band.y - reach, width: band.width, height: band.height + reach },
    // The line's box, which the drawing keeps (before 0.2.10 it was the seek bar itself).
    box: { x: band.x + lead, y: band.y + lineTop, width: band.width - lead - trail, height: 10 },
    line: { y: band.y + lineTop + 4, height: 2 },
    knob: (ratio: number) => ({ x: band.x + lead + ratio * span, y: band.y + lineTop + 1 }),
    mute: width - 97,
    /** The fraction a pointer at the band's own x gives (`fraction`). */
    fraction: (x: number) => Math.min(1, Math.max(0, (x - lead - 4) / span)),
    /** The tip's bottom edge (its y + height). */
    tipBottom: band.y + lineTop + 4 - 4,
  };
}

test("0.2.11 the band is the whole seek row and 4px above the bar's top edge, short of the mute button", () => {
  for (const width of [320, 640, 1280, 3840]) {
    const { band, touch, box, line, knob, mute, tipBottom } = layout(width);
    // The seek bar (what the drawing is placed in) is the seek row, rows 0…21.
    assert.equal(band.y, 0);
    assert.equal(band.y + band.height, 22);
    // The pointer area: rows -4…21, from 4px over the video (0.2.10: the bar's top
    // edge) down to the button row (y 22), exclusive: not into the buttons.
    assert.equal(touch.y, -4);
    assert.equal(touch.y + touch.height, 22);
    assert.equal(touch.x, band.x);
    assert.equal(touch.width, band.width);
    // Left to x 5, right to 8px before the mute button.
    assert.equal(band.x, 5);
    assert.equal(mute - (band.x + band.width), 8);
    // The look is unchanged: the line's box at 15px…width - 117px, y 6…16 (0.2.9: the seek bar
    // at x 15, y 6, width - 132 wide, 10 tall), the line at y 10, the knob at y 7, the tip 4px above the line.
    assert.deepEqual(box, { x: 15, y: 6, width: width - 132, height: 10 });
    assert.deepEqual(line, { y: 10, height: 2 });
    assert.equal(knob(0).x, 15);
    assert.equal(knob(0).y, 7);
    assert.equal(knob(1).x, 15 + (width - 132 - 8));
    assert.equal(tipBottom, 6);
  }
});

test("0.2.10 a pointer anywhere in the band maps to the line, clamped to its ends outside it", () => {
  const { band, fraction } = layout(1000);
  // The band's left edge (10px left of the line's box) and the line's own start: 0.
  assert.equal(fraction(0), 0);
  assert.equal(fraction(14), 0);
  // The band's right edge (12px right of the line's box): the end.
  assert.equal(fraction(band.width), 1);
  assert.equal(fraction(band.width - 12 - 4), 1);
  // The middle of the line is the middle, at any height in the band (x alone counts).
  assert.equal(fraction(14 + (band.width - 30) / 2), 0.5);
});

test("0.2.11 bar.slint: the band geometry, a TouchArea that fills it and reaches 4px above, and the drawing moved into it", () => {
  // The band in the seek row.
  assert.match(row, /seek-bar := SeekBar \{\s+x: 5px;\s+y: 0;\s+height: 22px;\s+width: max\(0px, root\.width - 110px\);\s+\}/);
  assert.match(row, /MuteButton \{\s+x: root\.width - 97px;/);
  assert.match(seekBar, /in property <length> lead: 10px;\s+in property <length> trail: 12px;\s+in property <length> line-top: 6px;/);
  assert.ok(seekBar.includes("in property <length> reach: 4px;"));
  assert.ok(seekBar.includes("private property <length> span: max(1px, root.width - root.lead - root.trail - 8px);"));
  assert.ok(seekBar.includes("height: 22px;"));
  assert.ok(seekBar.includes("return clamp((x - root.lead - 4px) / root.span, 0, 1);"));
  // The TouchArea is the seek bar's width and height plus `reach` above it: nothing
  // sticks out sideways or below. (The player does not clip the bottom bar, and the
  // bar lies in front of the video's TouchArea: host composition.rs and host.slint.)
  assert.match(
    seekBar,
    /touch := TouchArea \{\s+x: 0;\s+y: -root\.reach;\s+width: 100%;\s+height: root\.height \+ root\.reach;\s+enabled: PlaybackBridge\.can-seek;/,
  );
  assert.equal((seekBar.match(/TouchArea/g) ?? []).length, 1);
  // Hover (tip, preview) and drag still come from that TouchArea.
  assert.ok(seekBar.includes("out property <bool> pointing: touch.has-hover || touch.pressed;"));
  assert.ok(seekBar.includes("moved => { if self.pressed { PlaybackBridge.seek-changed(root.fraction(self.mouse-x)); } }"));
  assert.ok(seekBar.includes("if event.kind == PointerEventKind.cancel {"));
  // Every drawn element sits in the line's box: offsets from `lead` and `line-top`.
  for (const line of [
    "x: root.lead;\n        y: root.line-top + 4px;\n        width: root.width - root.lead - root.trail;",
    "x: root.lead;\n        y: root.line-top + 4px;\n        width: 4px + root.ratio * root.span;",
    "x: root.lead + 4px + PlaybackBridge.loop-a * root.span;\n        y: root.line-top + 6px;",
    "x: root.lead + Math.floor((4px + PlaybackBridge.loop-a * root.span) / 1px) * 1px;\n        y: root.line-top + 1px;",
    "x: root.lead + Math.floor((4px + PlaybackBridge.loop-b * root.span) / 1px) * 1px;\n        y: root.line-top + 1px;",
    "x: root.lead + root.ratio * root.span;\n        y: root.line-top + 1px;",
  ])
    assert.ok(seekBar.includes(line), line);
  // No y or x left from the old 10px seek bar (y 1px, 4px, 6px without the offset).
  assert.doesNotMatch(seekBar, /^\s+y: (1|4|6)px;/m);
  // The tip keeps its place: 4px above the line.
  assert.ok(slint.includes("y: seek-bar.y + seek-bar.line-top + 4px - 4px - self.height;"));
  assert.ok(slint.includes("x: clamp(seek-bar.x + seek-bar.pointer-x - self.width / 2,"));
});

/** A ready 100-second source opened by someone else. */
function withMedia(host = new TestHost()) {
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 40 });
  return host;
}
const seeks = (host: TestHost) =>
  host
    .sent("source.submit")
    .map((p) => p.intent as { kind: string; control?: { kind: string; seconds?: number } })
    .filter((i) => i.kind === "control" && i.control?.kind === "seek")
    .map((i) => i.control!.seconds);

test("0.2.10 hover and presses at the band's edges reach the plugin: preview on, seeks to the ends", async () => {
  const host = withMedia();
  const { stop } = await startPlugin(activate, { directory, host, locale: "en" });
  try {
    const bar = host.mounted();
    const { band, fraction } = layout(1000);
    // Hover at the band's top-left corner: the bar reports pointing (the tip and the
    // preview show from it); no seek.
    bar.emit("pointing", true);
    await host.settle(0);
    assert.deepEqual(seeks(host), []);
    // A press at the band's left edge, above the line: the start.
    bar.emit("dragging", true);
    bar.emit("seekChanged", fraction(0));
    bar.emit("seekChanged", fraction(0));
    bar.emit("dragging", false);
    await host.settle(80);
    assert.deepEqual(seeks(host), [0]);
    // A press at the band's right edge, below the line: the end.
    bar.emit("dragging", true);
    bar.emit("seekChanged", fraction(band.width));
    bar.emit("seekChanged", fraction(band.width));
    bar.emit("dragging", false);
    await host.settle(80);
    assert.deepEqual(seeks(host).at(-1), 100);
    bar.emit("pointing", false);
    await host.settle(0);
  } finally {
    await stop();
  }
});
