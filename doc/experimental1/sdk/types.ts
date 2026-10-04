// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
export type UiProperties = Record<string, string | number | boolean>;

export interface PlaybackState {
  path: string | null;
  paused: boolean;
  position: number;
  duration: number;
  error?: string;
  gaussianEnabled: boolean;
  sigma: number;
}

export interface PluginContext {
  onAction(name: string, handler: (value: number) => void | Promise<void>): () => void;
  onPlayback(handler: (state: PlaybackState) => void | Promise<void>): () => void;
  ui: { patch(properties: UiProperties): Promise<void> };
  dialogs: { openMedia(): Promise<string | null> };
  player: {
    load(path: string): Promise<void>;
    togglePause(): Promise<void>;
    seek(seconds: number): Promise<void>;
    snapshot(): Promise<PlaybackState>;
    filters: { setGaussian(options: { enabled: boolean; sigma: number }): Promise<void> };
  };
}

export interface PluginManifest {
  id: string;
  name: string;
  experimental: true;
  entry: string;
  // region 仅是此实验的挂载点约定，不是最终 placement API。
  ui: { file: string; component: string; global: string; region?: 'sidebar' };
}

export type PluginDisposer = () => void | Promise<void>;
