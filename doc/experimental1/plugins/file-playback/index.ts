// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
import type { PluginContext, PluginDisposer } from '../../sdk/types.ts';

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  const subscriptions = [
    ctx.onAction('open', async () => {
      const path = await ctx.dialogs.openMedia();
      if (path !== null) await ctx.player.load(path);
    }),
    ctx.onAction('toggle-pause', () => ctx.player.togglePause()),
    ctx.onAction('seek', seconds => ctx.player.seek(seconds)),
    ctx.onPlayback(state => ctx.ui.patch({
      status: state.error ?? state.path ?? '请选择 MP4 文件',
      'is-paused': state.paused,
      position: state.position,
      duration: state.duration,
    })),
  ];
  await ctx.ui.patch({ status: '请选择 MP4 文件', 'is-paused': true, position: 0, duration: 0 });
  return () => { for (const unsubscribe of subscriptions) unsubscribe(); };
}
