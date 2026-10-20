// Trial start: one submission per click, a lost first reply recovers the original
// trial by its request id, and stopping targets that trial. Scripted host answers.
import assert from "node:assert/strict";
import test from "node:test";
import { startWorkshop } from "./fixture.mjs";

/** A trial whose first reply the test resolves or loses by hand. */
function heldTrial() {
  const held = { count: 0, requestId: undefined, lose: undefined };
  held.trial = ({ requestId }) => {
    held.count++;
    held.requestId = requestId;
    return new Promise((_resolve, reject) => (held.lose = () => reject(new Error("first trial reply lost"))));
  };
  return held;
}

test("a second click while the trial starts submits nothing", async () => {
  const held = heldTrial();
  const w = await startWorkshop({ trial: held.trial, trialStateByRequest: () => ({ trialId: 42, status: "running" }) });
  await w.generate();
  w.window.emit("start-trial");
  await w.until(() => held.count === 1 && w.props.busy === true, "trial submission");
  assert.equal(w.props["can-trial"], false);
  w.window.emit("start-trial");
  await w.until(() => /候选尚未就绪/.test(w.status()), "second click refused");
  assert.equal(held.count, 1);
  held.lose();
  await w.until(() => w.props.busy === false, "recovery");
  assert.equal(held.count, 1);
  await w.stop();
});

test("a lost first reply recovers the original trial, which stop then cleans up", async () => {
  const held = heldTrial();
  const queries = [];
  const stopped = [];
  const w = await startWorkshop({
    trial: held.trial,
    trialStateByRequest: ({ requestId }) => {
      queries.push(requestId);
      return { trialId: 42, status: "running" };
    },
    stopTrial: ({ trialId }) => {
      stopped.push(trialId);
      return { trialId, status: "stopped", cleanupConfirmed: true };
    },
  });
  await w.generate();
  w.window.emit("start-trial");
  await w.until(() => held.count === 1 && w.props.busy === true, "trial submission");
  held.lose();
  await w.until(() => w.props.busy === false, "recovery");
  assert.ok(held.requestId);
  assert.deepEqual(queries, [held.requestId]);
  assert.equal(w.status(), "试载中：请在计数器窗口测试「增加」「归零」。");
  assert.deepEqual([w.props["can-stop"], w.props["can-recover"], w.props["can-trial"]], [true, false, false]);
  await w.send("stop-trial");
  assert.deepEqual(stopped, [42]);
  assert.equal(w.status(), "试载已停止并清理。");
  assert.equal(w.props["can-trial"], true);
  assert.equal(held.count, 1);
  await w.stop();
});

test("trial starting phases are waited out; cancel-task stops a running trial", async () => {
  let polls = 0;
  const w = await startWorkshop({
    trial: () => ({ trialId: 5, status: "preparing" }),
    trialState: ({ trialId }) => ({ trialId, status: ++polls < 2 ? "activating" : "running" }),
  });
  await w.generate();
  await w.send("start-trial");
  await w.until(() => w.props.busy === false, "trial start");
  assert.equal(w.status(), "试载中：请在计数器窗口测试「增加」「归零」。");
  await w.send("cancel-task");
  assert.deepEqual(w.host.sent("packages.stopTrial"), [{ trialId: 5 }]);
  assert.equal(w.status(), "试载已停止并清理。");
  await w.stop();
});

test("when even the recovery query is lost the trial is not resubmitted automatically", async () => {
  const held = heldTrial();
  const w = await startWorkshop({
    trial: held.trial,
    trialStateByRequest: () => {
      throw new Error("query reply lost");
    },
  });
  await w.generate();
  w.window.emit("start-trial");
  await w.until(() => held.count === 1 && w.props.busy === true, "trial submission");
  held.lose();
  await w.until(() => w.props.busy === false, "failure");
  assert.match(w.status(), /^错误：.*query reply lost/);
  assert.equal(held.count, 1);
  assert.equal(w.props["can-stop"], false);
  assert.equal(w.props["can-recover"], false);
  await w.stop();
});
