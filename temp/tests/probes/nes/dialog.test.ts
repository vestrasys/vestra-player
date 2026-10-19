// 实验契约测试：只检查选择器元数据；执行/帧/音频由真实 ROM 集成测试另证。
import test from "node:test";
import assert from "node:assert/strict";
import { activate } from "../../../plugins/nes/index.ts";
import { createContext } from "../../../sdk/context.ts";
test("N2 ROM selector restricts extensions and cancellation creates no source", async () => {
  const calls: { method: string; params: Record<string, unknown> }[] = [];
  const dispatch = createContext(
    async <T>(method: string, params: Record<string, unknown>): Promise<T> => {
      calls.push({ method, params });
      return null as T;
    },
  );
  const dispose = await activate(dispatch.context);
  await dispatch.action("open-rom", 0);
  assert.deepEqual(
    calls.find((call) => call.method === "dialogs.openFile")?.params,
    { extensions: ["nes"], title: "打开合法 NES ROM" },
  );
  assert.equal(
    calls.some((call) => call.method === "sources.open"),
    false,
  );
  await dispose();
  dispatch.clear();
});
