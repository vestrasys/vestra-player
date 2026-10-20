import assert from "node:assert/strict";
import test from "node:test";
import { bands, clamp, matches, neutral, nodes, parse, presetOf, presets, serialize, syncText } from "../model.ts";

test("reset clears the owned chain and preset gains stay inside the ±6 dB EQ range", () => {
  assert.deepEqual(nodes(neutral()), []);
  for (const preset of presets) {
    assert.equal(preset.gains.length, bands.length);
    for (const gain of preset.gains) assert.ok(Math.abs(gain) <= 6);
  }
  assert.throws(() => clamp(Number.NaN, -6, 6));
  assert.throws(() => clamp("4", -6, 6));
  assert.equal(clamp(50, -6, 6), 6);
  assert.ok(matches(neutral(), 0));
  assert.ok(!matches(neutral(), 1));
});

test("normalization and 晶化 use typed processing nodes without device volume", () => {
  assert.deepEqual(nodes({ ...neutral(), normalize: true, clarity: { on: true, db: 3 } }), [
    { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 },
    { kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 },
  ]);
  assert.ok(nodes({ ...neutral(), gains: [6, 6, 6, 6, 6, 6] }).every((node) => node.kind === "equalizer"));
  assert.deepEqual(nodes({ ...neutral(), clarity: { on: false, db: 5 } }), [], "晶化 off keeps its strength but makes no node");
});

test("a bypassed equalizer drops only the bands; presetOf names the matching preset", () => {
  const value = { ...neutral(), equalizer: false, gains: [3, 0, 0, 0, 0, 0], clarity: { on: true, db: 1 } };
  assert.deepEqual(nodes(value), [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 }]);
  assert.equal(presetOf(neutral()), 0);
  assert.equal(presetOf({ ...neutral(), gains: [...presets[3].gains] }), 3);
  assert.equal(presetOf(value), -1);
});

test("settings parse leniently; a legacy numeric clarity migrates to the 晶化 switch", () => {
  assert.deepEqual(parse(null), { settings: neutral(), syncKeep: "none", tab: 0, extra: {} });
  assert.deepEqual(parse({ clarity: 2.5 }).settings.clarity, { on: true, db: 2.5 });
  assert.deepEqual(parse({ clarity: 0 }).settings.clarity, { on: false, db: 3 });
  assert.deepEqual(parse({ clarity: { on: true, db: 9 } }).settings.clarity, { on: true, db: 6 });
  assert.deepEqual(parse({ gains: [1, "x", 9, 0, 0, 0] }).settings.gains, [1, 0, 6, 0, 0, 0]);
  assert.deepEqual(parse({ gains: [1, 2] }).settings.gains, [0, 0, 0, 0, 0, 0]);
  assert.equal(parse({ syncKeep: "session", syncMs: 500 }).settings.delayMs, 0, "only 始终保持 restores the delay");
  assert.equal(parse({ syncKeep: "always", syncMs: 5003 }).settings.delayMs, 2000);
  assert.equal(parse({ syncKeep: "bogus" }).syncKeep, "none");
  assert.equal(parse({ tab: 7 }).tab, 0);
  const stored = parse({ version: 1, channels: { mode: "left" }, syncKeep: "always", syncMs: 120 });
  assert.deepEqual(stored.extra, { channels: { mode: "left" } }, "later fields are kept");
  assert.deepEqual(serialize(stored), {
    channels: { mode: "left" },
    version: 1,
    equalizer: true,
    gains: [0, 0, 0, 0, 0, 0],
    normalize: false,
    clarity: { on: false, db: 3 },
    syncKeep: "always",
    syncMs: 120,
    tab: 0,
  });
  assert.equal("syncMs" in serialize({ ...stored, syncKeep: "session" }), false);
  assert.equal(syncText(-50), "音频同步: -0.05秒");
  assert.equal(syncText(300), "音频同步: +0.3秒");
  assert.equal(syncText(0), "音频同步: 0秒");
});
