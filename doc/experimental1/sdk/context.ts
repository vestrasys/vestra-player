// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
import type { PlaybackState, PluginContext, UiProperties } from './types.ts';

type ActionHandler = (value: number) => void | Promise<void>;
type PlaybackHandler = (state: PlaybackState) => void | Promise<void>;
export type Request = <T>(method: string, params: Record<string, unknown>) => Promise<T>;

export function createContext(request: Request) {
  const actions = new Map<string, Set<ActionHandler>>();
  const playback = new Set<PlaybackHandler>();
  const context: PluginContext = {
    onAction(name, handler) {
      const handlers = actions.get(name) ?? new Set<ActionHandler>();
      actions.set(name, handlers);
      handlers.add(handler);
      return () => { handlers.delete(handler); };
    },
    onPlayback(handler) { playback.add(handler); return () => { playback.delete(handler); }; },
    ui: { patch: (properties: UiProperties) => request<void>('ui.patch', { properties }) },
    dialogs: { openMedia: () => request<string | null>('dialogs.openMedia', {}) },
    player: {
      load: path => request<void>('player.load', { path }),
      togglePause: () => request<void>('player.togglePause', {}),
      seek: seconds => request<void>('player.seek', { seconds }),
      snapshot: () => request<PlaybackState>('player.snapshot', {}),
      filters: { setGaussian: options => request<void>('player.filters.setGaussian', options) },
    },
  };
  return {
    context,
    async action(name: string, value: number) {
      if (!Number.isFinite(value)) throw new Error('无效的界面事件参数');
      for (const handler of actions.get(name) ?? []) await handler(value);
    },
    async playback(state: PlaybackState) { for (const handler of playback) await handler(state); },
    clear() { actions.clear(); playback.clear(); },
  };
}
