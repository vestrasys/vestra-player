// 实验行为测试；模拟传输只验证插件策略，真实像素另由 integration 测试证明。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext } from "../../../sdk/context.ts";
import type { Request } from "../../../sdk/context.ts";
import type { UiProperties } from "../../../sdk/types.ts";
import { activate } from "../../../plugins/video-adjust/index.ts";

function harness(capabilities = ["effects.video"]) {
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  let properties: UiProperties = {};
  let failure = false;
  const request: Request = async <T>(
    method: string,
    params: Record<string, unknown>,
  ): Promise<T> => {
    calls.push({ method, params });
    if (method === "capabilities") return capabilities as T;
    if (method === "effects.set" && failure)
      throw new Error("测试后端拒绝此配置");
    if (method === "ui.patch")
      properties = { ...properties, ...(params.properties as UiProperties) };
    return undefined as T;
  };
  const bridge = createContext(request);
  return {
    bridge,
    calls,
    properties: () => properties,
    fail: (value: boolean) => {
      failure = value;
    },
  };
}

test("无视频处理能力时明确拒绝激活，不注册动作也不假装成功", async () => {
  const h = harness([]);
  await assert.rejects(activate(h.bridge.context), /缺少 effects.video/);
  assert.deepEqual(
    h.calls.map((call) => call.method),
    ["capabilities"],
  );
  await h.bridge.action("brightness-up", 0);
  assert.equal(h.calls.length, 1);
});

test("菜单与原生面板动作共享状态；预设一次提交，复位只撤销本资源", async () => {
  const h = harness();
  const dispose = await activate(h.bridge.context);
  await h.bridge.action("brightness-up", 0);
  assert.equal(h.properties().brightness, 0.05);
  await h.bridge.action("hue", 30);
  assert.equal(h.properties().brightness, 0.05);
  assert.equal(h.properties().hue, 30);
  const before = h.calls.filter((call) => call.method === "effects.set").length;
  await h.bridge.action("preset-cinema", 0);
  assert.equal(
    h.calls.filter((call) => call.method === "effects.set").length,
    before + 1,
  );
  assert.equal(h.properties().saturation, 0.85);
  await h.bridge.action("reset", 0);
  assert.equal(h.properties().brightness, 0);
  assert.equal(h.properties().hue, 0);
  assert.deepEqual(
    h.calls.findLast((call) => call.method === "effects.remove")?.params,
    { key: "adjustments" },
  );
  await dispose();
  const afterDispose = h.calls.length;
  await h.bridge.action("brightness-up", 0);
  assert.equal(h.calls.length, afterDispose, "停用后不存在残余动作订阅");
});

test("配置拒绝与范围错误不覆盖成功配置，失败后仍可继续操作", async () => {
  const h = harness();
  const dispose = await activate(h.bridge.context);
  await h.bridge.action("brightness", 0.25);
  h.fail(true);
  await assert.rejects(h.bridge.action("brightness", 0.5), /测试后端拒绝/);
  assert.equal(h.properties().brightness, 0.25);
  assert.match(String(h.properties().status), /应用失败/);
  const before = h.calls.length;
  await assert.rejects(h.bridge.action("saturation", -1), RangeError);
  assert.equal(h.calls.length, before, "非法值不能发送宿主请求");
  h.fail(false);
  await h.bridge.action("brightness-down", 0);
  assert.equal(
    h.properties().brightness,
    0.2,
    "使用上次成功配置而非失败值继续编辑",
  );
  await dispose();
});

test("manifest命令分布在多个入口，并与插件实际动作对应", async () => {
  const manifest: unknown = JSON.parse(
    await readFile(
      new URL("../../../plugins/video-adjust/manifest.json", import.meta.url),
      "utf8",
    ),
  );
  assert.ok(
    typeof manifest === "object" && manifest !== null && "commands" in manifest,
  );
  const commands = manifest.commands;
  assert.ok(Array.isArray(commands));
  const roots = new Set<string>();
  const h = harness();
  const dispose = await activate(h.bridge.context);
  for (const command of commands as unknown[]) {
    assert.ok(
      typeof command === "object" &&
        command !== null &&
        "id" in command &&
        "menu" in command,
    );
    assert.equal(typeof command.id, "string");
    assert.ok(
      Array.isArray(command.menu) && typeof command.menu[0] === "string",
    );
    roots.add(command.menu[0]);
    const before = h.calls.length;
    await h.bridge.action(String(command.id), 0);
    assert.ok(
      h.calls.length > before,
      `manifest命令 ${String(command.id)} 应有实际处理器`,
    );
  }
  assert.ok(roots.size >= 2);
  await dispose();
});
