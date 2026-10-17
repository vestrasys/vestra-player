// 实验插件工坊，目标 probe-0.2；不是最终插件 API。Pi 是本插件依赖，不属于播放器核心。
import { readFile } from "node:fs/promises";
import type { PluginContext } from "../../sdk/types.ts";
import { Workshop, parseRequest } from "./engine.ts";
import type { Request, Scenario } from "./model.ts";

export async function activate(ctx: PluginContext) {
  const workshop = await Workshop.create(ctx);
  let request: Request = {
    prompt: "生成一个带原生按钮和菜单入口的计数器插件",
    increment: 1,
    scenario: "repair",
  };
  const subscriptions: (() => void)[] = [];
  const action = (name: string, handler: (value: number) => Promise<void>) => {
    subscriptions.push(
      ctx.onAction(name, async (value) => {
        try {
          await handler(value);
        } catch (error) {
          await ctx.ui.patch({ status: String(error) });
        }
      }),
    );
  };
  action("generate", async (value) => {
    const increment = value === 0 ? request.increment : value;
    if (!Number.isInteger(increment) || increment < 1 || increment > 10)
      throw new Error("increment 必须是 1–10 整数");
    await workshop.start({ ...request, increment, scenario: "repair" });
  });
  for (const scenario of [
    "invalid",
    "activation-failure",
    "slow",
    "slint-failure",
  ] satisfies Scenario[]) {
    action(scenario, async () => {
      await workshop.start({ ...request, scenario });
    });
  }
  action("request-file", async () => {
    if (workshop.busy) throw new Error("生成期间不能修改需求");
    const path = await ctx.dialogs.openFile({
      title: "读取实验工坊需求 JSON（prompt / increment）",
      extensions: ["json"],
    });
    if (!path) return;
    const contents = await readFile(path, "utf8");
    if (contents.length > 8192) throw new Error("需求 JSON 超过实验大小限制");
    request = parseRequest(JSON.parse(contents));
    await ctx.ui.patch({
      request: request.prompt,
      status: `需求已读取，计数增量 ${request.increment}`,
    });
  });
  action("cancel", async () => {
    await workshop.cancel();
  });
  action("rollback", async () => {
    await workshop.rollback();
  });
  action("remove-generated", async () => {
    await workshop.remove();
  });
  await ctx.ui.patch({
    phase: "idle",
    status: "Pi Agent 已就绪 · 本地 mock · 无 API key / 无网络",
    request: request.prompt,
    busy: false,
  });
  return async () => {
    subscriptions.forEach((unsubscribe) => unsubscribe());
    await workshop.dispose();
  };
}
