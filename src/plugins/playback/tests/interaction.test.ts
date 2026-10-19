import assert from "node:assert/strict";
import test from "node:test";
import { LatestValue, SeekFence, parseTime, time } from "../interaction.ts";
test("seek stamp advances via own receipts and rejects outside restart/open", () => {
  const fence = new SeekFence({ generation: 4, intentVersion: 10 });
  assert.deepEqual(fence.begin(), { generation: 4, intentVersion: 10 });
  assert.equal(fence.observe({ generation: 4, intentVersion: 11 }), true);
  assert.equal(fence.complete({ generation: 4, intentVersion: 11 }), true);
  assert.deepEqual(fence.begin(), { generation: 4, intentVersion: 11 });
  assert.equal(fence.complete({ generation: 4, intentVersion: 12 }), true);
  for (const outside of [{ generation: 4, intentVersion: 14 }, { generation: 5, intentVersion: 13 }]) {
    const old = new SeekFence({ generation: 4, intentVersion: 12 }); old.begin();
    assert.equal(old.observe(outside), false);
    assert.equal(old.complete({ generation: 4, intentVersion: 13 }), false);
    assert.equal(old.begin(), null);
  }
});
test("cancel discards the unsent tail without cancelling the accepted call", async () => {
  let release = () => {};
  const accepted = new Promise<void>(resolve => { release = resolve; });
  const calls: number[] = [], failures: unknown[] = [];
  const queue = new LatestValue<number>(async value => { calls.push(value); await accepted; }, e => failures.push(e));
  queue.push(10); queue.push(20); queue.push(30); queue.clear(); release();
  await new Promise(resolve => setTimeout(resolve, 60));
  assert.deepEqual(calls, [10]); assert.deepEqual(failures, []);
});
test("release sample replaces intermediate points and is eventually submitted", async () => {
  let release = () => {}, finish = () => {};
  const accepted = new Promise<void>(resolve => { release = resolve; });
  const finished = new Promise<void>(resolve => { finish = resolve; });
  const calls: number[] = [];
  const queue = new LatestValue<number>(async value => { calls.push(value); if (value === 10) await accepted; if (value === 37) finish(); }, e => { throw e; });
  queue.push(10); queue.push(20); queue.push(30); queue.push(37); release();
  await finished; assert.deepEqual(calls, [10, 37]);
});
test("late failed operation from a cleared gesture cannot report into the new gesture", async () => {
  let fail: (error: Error) => void = () => {};
  const pending = new Promise<void>((_, reject) => { fail = reject; });
  const failures: unknown[] = [];
  const queue = new LatestValue<number>(async () => pending, e => failures.push(e));
  queue.push(10); queue.clear(); fail(new Error("old"));
  await new Promise(resolve => setTimeout(resolve, 60)); assert.deepEqual(failures, []);
});
test("explicit time input rejects malformed and negative values", () => {
  assert.equal(parseTime("1:02:03.250"), 3723.25); assert.equal(parseTime("12:61"), null); assert.equal(parseTime("-1"), null);
  assert.equal(time(null), "--:--"); assert.equal(time(3723), "1:02:03");
});
