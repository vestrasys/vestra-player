// 实验插件：接口、处理策略与最终产品可能存在很大差异。
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";
import { change, filter, isNeutral, preset } from "./settings.ts";
import type { AdjustmentName, Adjustments, PresetName } from "./settings.ts";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  if (!(await ctx.capabilities()).includes("effects.video")) {
    throw new Error("当前宿主缺少 effects.video 能力，无法应用画面调节");
  }
  let settings = preset("neutral");
  const subscriptions: (() => void)[] = [];

  async function publish(status: string): Promise<void> {
    await ctx.ui.patch({ ...settings, status });
  }

  async function apply(next: Adjustments, status: string): Promise<void> {
    // 只有宿主接受参数后才提交本地状态。失败时不冒充已生效。
    try {
      if (isNeutral(next)) await ctx.effects.remove("adjustments");
      else
        await ctx.effects.set({
          key: "adjustments",
          kind: "video",
          filter: filter(next),
        });
    } catch (error) {
      await publish(`应用失败：${String(error)}`);
      throw error;
    }
    settings = next;
    await publish(status);
  }

  function on(name: string, handler: (value: number) => Promise<void>): void {
    subscriptions.push(ctx.onAction(name, handler));
  }

  for (const name of [
    "brightness",
    "contrast",
    "saturation",
    "hue",
  ] as const satisfies readonly AdjustmentName[]) {
    on(name, (value) => apply(change(settings, name, value), "画面参数已应用"));
  }
  for (const name of [
    "bright",
    "cinema",
    "neutral",
  ] as const satisfies readonly PresetName[]) {
    on(`preset-${name}`, () =>
      apply(
        preset(name),
        name === "neutral" ? "画面已复位" : `已应用 ${name} 预设`,
      ),
    );
  }
  on("brightness-up", () =>
    apply(
      change(
        settings,
        "brightness",
        Math.min(1, Math.round((settings.brightness + 0.05) * 100) / 100),
      ),
      "亮度已增加",
    ),
  );
  on("brightness-down", () =>
    apply(
      change(
        settings,
        "brightness",
        Math.max(-1, Math.round((settings.brightness - 0.05) * 100) / 100),
      ),
      "亮度已降低",
    ),
  );
  on("reset", () => apply(preset("neutral"), "画面已复位"));
  await publish("画面调节就绪");

  return async () => {
    for (const unsubscribe of subscriptions) unsubscribe();
    await ctx.effects.remove("adjustments");
  };
}
