// 实验单元测试：参数与媒体来源契约，最终 API 可能相差很大。
import test from "node:test";
import assert from "node:assert/strict";
import type { PlaybackState } from "../../../sdk/types.ts";
import { createContext } from "../../../sdk/context.ts";
import type { Request } from "../../../sdk/context.ts";
import { activate } from "../../../plugins/capture-export/index.ts";
import {
  captureSnapshot,
  finiteRange,
} from "../../../plugins/capture-export/snapshot.ts";

const state: PlaybackState = {
  path: "/sample.mp4",
  paused: false,
  position: 2,
  duration: 8,
  sessionId: "session-1",
  sourceKind: "file",
  gaussianEnabled: false,
  sigma: 2,
};

test("截图只接受文件来源及有效会话，不伪造外部来源 seek", () => {
  assert.deepEqual(captureSnapshot(state), {
    input: "/sample.mp4",
    sessionId: "session-1",
    position: 2,
    duration: 8,
  });
  assert.throws(() => captureSnapshot({ ...state, path: null }), /文件媒体/);
  assert.throws(
    () => captureSnapshot({ ...state, sourceKind: "external" }),
    /文件媒体/,
  );
  assert.throws(
    () => captureSnapshot({ ...state, sessionId: undefined }),
    /会话/,
  );
  assert.throws(
    () => captureSnapshot({ ...state, position: Infinity }),
    /位置/,
  );
});

test("有限连续截图及导出参数拒绝非有限、越界及小数数量", () => {
  assert.equal(finiteRange(100, 1, 100, "数量", true), 100);
  for (const value of [0, 101, NaN, Infinity, 2.5])
    assert.throws(() => finiteRange(value, 1, 100, "数量", true));
  assert.equal(finiteRange(0.1, 0.1, 600, "秒数"), 0.1);
});

test("缺少媒体任务能力时拒绝激活，不注册可误导的成功动作", async () => {
  const calls: string[] = [];
  const request: Request = async <T>(method: string): Promise<T> => {
    calls.push(method);
    if (method === "capabilities") return [] as T;
    throw new Error(`不应调用 ${method}`);
  };
  const bridge = createContext(request);
  await assert.rejects(activate(bridge.context), /media.tasks/);
  await bridge.action("capture-raw", 0);
  assert.deepEqual(calls, ["capabilities"]);
});
