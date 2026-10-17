// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  // 操作错误属于插件状态；普通 playback tick 不能立刻用旧媒体路径覆盖它。
  let operationError: string | undefined;
  let lastPath: string | null = null;
  async function showError(error: unknown) {
    operationError = `操作失败：${String(error)}`;
    await ctx.ui.patch({ status: operationError });
  }
  async function perform(operation: () => Promise<unknown>) {
    try {
      await operation();
      operationError = undefined;
      const state = await ctx.player.snapshot();
      await ctx.ui.patch({
        status: state.error ?? state.path ?? "请选择 MP4 文件",
      });
    } catch (error) {
      await showError(error);
    }
  }
  const subscriptions = [
    ctx.onAction("open", async () => {
      try {
        const path = await ctx.dialogs.openMedia();
        if (path !== null) await perform(() => ctx.player.load(path));
      } catch (error) {
        await showError(error);
      }
    }),
    ctx.onAction("toggle-pause", () => perform(() => ctx.player.togglePause())),
    ctx.onAction("seek", (seconds) => perform(() => ctx.player.seek(seconds))),
    ctx.onPlayback((state) => {
      if (state.path !== lastPath) operationError = undefined;
      lastPath = state.path;
      return ctx.ui.patch({
        status:
          state.error ?? operationError ?? state.path ?? "请选择 MP4 文件",
        "is-paused": state.paused,
        position: state.position,
        duration: state.duration ?? 0,
      });
    }),
  ];
  await ctx.ui.patch({
    status: "请选择 MP4 文件",
    "is-paused": true,
    position: 0,
    duration: 0,
  });
  return () => {
    for (const unsubscribe of subscriptions) unsubscribe();
  };
}
