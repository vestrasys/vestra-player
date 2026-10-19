import test from "node:test";
import assert from "node:assert/strict";
import { LatestProjection } from "../projection.ts";
test("blocked projection retains only active plus latest refresh", async () => {
  let release!: () => void;
  const blocked = new Promise<void>(resolve => { release = resolve; });
  let value = 0, active = 0, maximum = 0;
  const observed: number[] = [];
  const projection = new LatestProjection(async () => {
    active++; maximum = Math.max(maximum, active); const snapshot = value;
    if (!observed.length) await blocked;
    observed.push(snapshot); active--;
  });
  const settled = projection.request();
  for (value = 1; value <= 1000; value++) void projection.request();
  value = 1000; release(); await settled;
  assert.deepEqual(observed, [0, 1000]); assert.equal(maximum, 1);
  projection.stop(); await projection.request(); assert.equal(observed.length, 2);
});
