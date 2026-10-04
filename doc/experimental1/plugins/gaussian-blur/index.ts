// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
// TS 只发送滤镜配置，不读取帧；高斯算法由播放后端执行。
import type { PluginContext, PluginDisposer } from '../../sdk/types.ts';

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  let enabled = false;
  let sigma = 2;
  let sigmaDirty = false;
  async function apply() {
    try {
      await ctx.player.filters.setGaussian({ enabled, sigma });
    } catch (error) {
      const state = await ctx.player.snapshot();
      enabled = state.gaussianEnabled;
      sigma = state.sigma;
      sigmaDirty = false;
      await ctx.ui.patch({ enabled, sigma });
      throw error;
    }
    sigmaDirty = false;
    await ctx.ui.patch({ enabled, sigma, status: enabled ? '高斯模糊已启用' : '高斯模糊已关闭' });
  }
  const subscriptions = [
    ctx.onAction('enabled', async value => { enabled = value !== 0; await apply(); }),
    ctx.onAction('sigma', value => { sigma = Math.max(1, Math.min(12, value)); sigmaDirty = true; }),
    ctx.onAction('apply', apply),
    ctx.onPlayback(state => {
      enabled = state.gaussianEnabled;
      if (!sigmaDirty) sigma = state.sigma;
      return ctx.ui.patch({ enabled, sigma, status: enabled ? '高斯模糊已启用' : '高斯模糊已关闭' });
    }),
  ];
  await ctx.ui.patch({ enabled, sigma, status: '高斯模糊已关闭' });
  return async () => {
    for (const unsubscribe of subscriptions) unsubscribe();
    await ctx.player.filters.setGaussian({ enabled: false, sigma });
  };
}
