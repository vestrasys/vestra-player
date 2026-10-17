// 实验测试：模拟传输检查状态策略，真实音频另由 integration 测试验证。
import { test } from "node:test";
import assert from "node:assert/strict";
import { createContext, type Request } from "../../../sdk/context.ts";
import type { UiProperties } from "../../../sdk/types.ts";
import { activate } from "../../../plugins/audio-workbench/index.ts";

function harness(capable = true) {
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let properties: UiProperties = {};
  let reject = false;
  const request: Request = async <T>(
    method: string,
    params: Record<string, unknown>,
  ): Promise<T> => {
    calls.push({ method, params });
    if (method === "capabilities")
      return (capable ? ["effects.audio"] : []) as T;
    if (method === "effects.set" && reject) throw new Error("后端拒绝配置");
    if (method === "ui.patch")
      properties = { ...properties, ...(params.properties as UiProperties) };
    return undefined as T;
  };
  return {
    bridge: createContext(request),
    calls,
    properties: () => properties,
    reject: (value: boolean) => {
      reject = value;
    },
  };
}

test("菜单和滑块共享参数，拒绝后恢复，队列保留最后成功值，复位/停用只撤销自身", async () => {
  const h = harness();
  const dispose = await activate(h.bridge.context);
  await h.bridge.action("preset-dialogue", 0);
  assert.equal(h.properties().bass, -3);
  assert.equal(h.properties().voice, true);
  await h.bridge.action("toggle-normalize", 0);
  assert.equal(h.properties().normalize, false);
  h.reject(true);
  await assert.rejects(h.bridge.action("bass", 10), /后端拒绝/);
  assert.equal(h.properties().bass, -3);
  h.reject(false);
  await Promise.all([h.bridge.action("bass", 5), h.bridge.action("mid", 2)]);
  assert.equal(h.properties().bass, 5);
  assert.equal(h.properties().mid, 2);
  await h.bridge.action("reset", 0);
  assert.deepEqual(
    h.calls.findLast((call) => call.method === "effects.remove")?.params,
    { key: "audio-chain" },
  );
  assert.equal(h.properties().bass, 0);
  await dispose();
  const count = h.calls.length;
  await h.bridge.action("bass", 5);
  assert.equal(h.calls.length, count, "停用清除所有动作订阅");
});

test("能力缺失明确失败，不注册残余动作也不提交效果", async () => {
  const h = harness(false);
  await assert.rejects(activate(h.bridge.context), /effects.audio/);
  await h.bridge.action("bass", 1);
  assert.equal(
    h.calls.some((call) => call.method === "effects.set"),
    false,
  );
});
