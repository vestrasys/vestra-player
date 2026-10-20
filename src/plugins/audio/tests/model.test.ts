import assert from "node:assert/strict";
import test from "node:test";
import { bands, clamp, matches, neutral, nodes, presets } from "../model.ts";

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

test("normalization and clarity use typed processing nodes without device volume", () => {
  assert.deepEqual(nodes({ ...neutral(), normalize: true, clarity: 3 }), [
    { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 },
    { kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 },
  ]);
  assert.ok(nodes({ ...neutral(), gains: [6, 6, 6, 6, 6, 6] }).every((node) => node.kind === "equalizer"));
});
