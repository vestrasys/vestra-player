import assert from "node:assert/strict";
import test from "node:test";
import { MAX_RATE, MIN_RATE } from "../controller.ts";
import { clampRate, DRAG_STEP, dragRate, positionOf, rateAt, TICK_POSITIONS, TICKS, tickOf } from "../speed.ts";

const near = (actual: number, expected: number, message?: string) =>
  assert.ok(Math.abs(actual - expected) < 1e-9, `${message ?? ""} ${actual} ≠ ${expected}`);

test("speed ruler: the ticks are the presets, 1× in the centre, each half evenly spaced", () => {
  assert.deepEqual([...TICKS], [0.25, 0.5, 1, 1.25, 1.5, 2, 4]);
  assert.equal(TICKS[0], MIN_RATE);
  assert.equal(TICKS.at(-1), MAX_RATE);
  for (const [index, tick] of TICKS.entries()) {
    near(positionOf(tick), TICK_POSITIONS[index]!, `tick ${tick}`);
    near(rateAt(TICK_POSITIONS[index]!), tick, `position of ${tick}`);
    assert.equal(tickOf(tick), index);
  }
  near(positionOf(1), 0.5);
  // The slow half: two equal segments; the fast half: four.
  near(positionOf(0.5) - positionOf(0.25), positionOf(1) - positionOf(0.5));
  for (let index = 3; index < TICKS.length; index++)
    near(TICK_POSITIONS[index]! - TICK_POSITIONS[index - 1]!, 0.125, `segment ${index}`);
  assert.equal(tickOf(1.3), -1);
});

test("speed ruler: linear between ticks, and position ↔ speed round-trips", () => {
  near(rateAt(0.125), 0.375);
  near(rateAt(0.5625), 1.125);
  near(rateAt(0.9375), 3);
  for (let rate = MIN_RATE; rate <= MAX_RATE + 1e-9; rate += 0.05) near(rateAt(positionOf(rate)), rate, `rate ${rate}`);
  for (let position = 0; position <= 1 + 1e-9; position += 1 / 64)
    near(positionOf(rateAt(position)), position, `position ${position}`);
  // Positions only grow with the speed.
  let last = -1;
  for (let rate = MIN_RATE; rate <= MAX_RATE + 1e-9; rate += 0.01) {
    assert.ok(positionOf(rate) > last, `monotonic at ${rate}`);
    last = positionOf(rate);
  }
});

test("speed ruler: clamping to the player's limits", () => {
  assert.equal(clampRate(0.1), MIN_RATE);
  assert.equal(clampRate(9), MAX_RATE);
  assert.equal(positionOf(0.1), 0);
  assert.equal(positionOf(16), 1);
  near(rateAt(-0.5), MIN_RATE);
  near(rateAt(1.5), MAX_RATE);
  assert.equal(dragRate(-1, 0), MIN_RATE);
  assert.equal(dragRate(2, 0), MAX_RATE);
  assert.equal(dragRate(Number.NaN, 0), MIN_RATE);
});

test("speed ruler: a drag snaps near a tick and moves in 0.05 steps between ticks", () => {
  const snap = 0.02;
  // Within the snap distance of a tick: the tick itself.
  assert.equal(dragRate(0.5 + 0.015, snap), 1);
  assert.equal(dragRate(0.625 - 0.019, snap), 1.25);
  assert.equal(dragRate(0.875 + 0.01, snap), 2);
  assert.equal(dragRate(0.01, snap), 0.25);
  // Beyond it: the speed there, rounded to 0.05.
  assert.equal(dragRate(0.57, snap), 1.15); // 1.14
  assert.equal(dragRate(0.9375, snap), 3);
  assert.equal(dragRate(0.94, snap), 3.05);
  assert.equal(dragRate(0.35, snap), 0.7);
  for (let position = 0; position <= 1; position += 1 / 200) {
    const rate = dragRate(position, snap);
    near(Math.round(rate / DRAG_STEP) * DRAG_STEP, rate, `step at ${position}`);
    assert.ok(rate >= MIN_RATE && rate <= MAX_RATE);
    // Never a long float: rateText shows it as typed.
    assert.equal(rate, Number(rate.toFixed(2)));
  }
  // No snap distance: exactly on a tick still lands on it.
  assert.equal(dragRate(0.75, 0), 1.5);
});
