// 实验探针：此接口与最终成品 API 可能相差很大。
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";
import { audioFilter } from "./filter.ts";
import { neutral, preset, update, type AudioParameters } from "./parameters.ts";

const effectKey = "audio-chain";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  if (!(await ctx.capabilities()).includes("effects.audio"))
    throw new Error("宿主不支持音频处理能力 effects.audio");
  let parameters = neutral();
  let disposed = false;
  // Serialize changes even if a future runtime delivers UI and command events concurrently.
  let pending: Promise<void> = Promise.resolve();

  async function publish(status: string) {
    await ctx.ui.patch({
      bass: parameters.bass,
      mid: parameters.mid,
      treble: parameters.treble,
      normalize: parameters.normalize,
      voice: parameters.voice,
      "delay-ms": parameters.delayMs,
      status,
    });
  }

  function change(
    makeNext: (current: AudioParameters) => AudioParameters,
  ): Promise<void> {
    const operation = pending.then(async () => {
      if (disposed) return;
      const next = makeNext(parameters);
      try {
        const isNeutral =
          next.bass === 0 &&
          next.mid === 0 &&
          next.treble === 0 &&
          !next.normalize &&
          !next.voice &&
          next.delayMs === 0;
        if (isNeutral) await ctx.effects.remove(effectKey);
        else
          await ctx.effects.set({
            key: effectKey,
            kind: "audio",
            filter: audioFilter(next),
          });
        parameters = next;
        await publish("音频参数已应用；切换媒体继续生效");
      } catch (error) {
        await publish(
          `应用失败，保留之前参数：${error instanceof Error ? error.message : String(error)}`,
        );
        throw error;
      }
    });
    pending = operation.catch(() => undefined);
    return operation;
  }

  const subscriptions = [
    "bass",
    "mid",
    "treble",
    "delay-ms",
    "normalize",
    "voice",
  ].map((action) =>
    ctx.onAction(action, (value) =>
      change((current) => update(current, action, value)),
    ),
  );
  subscriptions.push(
    ctx.onAction("preset", (value) => change(() => preset(value))),
    ctx.onAction("reset", () => change(() => neutral())),
    ctx.onAction("preset-dialogue", () => change(() => preset(1))),
    ctx.onAction("preset-music", () => change(() => preset(2))),
    ctx.onAction("toggle-normalize", () =>
      change((current) => ({ ...current, normalize: !current.normalize })),
    ),
  );
  await publish("音频工作台就绪；未修改当前音频");
  return async () => {
    disposed = true;
    subscriptions.forEach((unsubscribe) => unsubscribe());
    await pending;
    await ctx.effects.remove(effectKey);
  };
}
