import type { BindingInfo, Plugin } from "@vestra-player/sdk";

/** The bar elements whose tooltip names a key (G-07). */
export type TipKey = "play-pause" | "stop" | "seek-backward" | "seek-forward" | "previous" | "next" | "mute" | "open" | "sidebar";

/**
 * The commands' default keys as the menu labels them, used until (or when) the
 * binding table cannot be read.
 */
export const DEFAULT_KEYS: Record<TipKey, string> = {
  "play-pause": "Space",
  stop: "F4",
  "seek-backward": "←",
  "seek-forward": "→",
  previous: "PgUp",
  next: "PgDn",
  mute: "M",
  open: "Ctrl+O",
  sidebar: "F6",
};

/** The host's menu text of a command's first active keyboard binding (`null`: none). */
export function firstKey(info: BindingInfo): string | null {
  const taken = new Set(info.shadowed.map((each) => each.input));
  const index = info.current.findIndex((input) => !input.startsWith("Mouse:") && !taken.has(input));
  return index < 0 ? null : (info.labels[index] ?? null);
}

/**
 * The bar elements' keys from the binding table (`plugin.bindings`), so a key
 * the user rebinds shows in the tooltip; the labels are the host's menu text.
 * The sidebar toggle has no command of its own: its key is that of the top-level
 * menu item 「播放列表」, whoever registers it (none: no key).
 */
export class Keys {
  #keys: Record<TipKey, string | null> = { ...DEFAULT_KEYS };
  readonly #plugin: Plugin;
  readonly #changed: () => void;

  constructor(plugin: Plugin, changed: () => void) {
    this.#plugin = plugin;
    this.#changed = changed;
    try {
      plugin.bindings.onChange(() => void this.refresh());
    } catch {
      // Without `bindings.manage` the default keys stay.
    }
  }

  /** Read the table; on failure the last known keys stay. */
  async refresh() {
    let table;
    try {
      table = await this.#plugin.bindings.list();
    } catch {
      return;
    }
    const find = (test: (info: BindingInfo) => boolean) => {
      const info = table.commands.find(test);
      return info ? firstKey(info) : null;
    };
    const host = (id: string) => find((info) => info.plugin === "host" && info.command === id);
    this.#keys = {
      "play-pause": host("play-pause"),
      stop: host("stop"),
      "seek-backward": host("seek-backward"),
      "seek-forward": host("seek-forward"),
      previous: host("previous"),
      next: host("next"),
      mute: host("mute"),
      open: find((info) => info.plugin === this.#plugin.id && info.command === "open"),
      sidebar: find((info) => info.path?.length === 1 && info.path[0] === "播放列表"),
    };
    this.#changed();
  }

  /** `title (key)`, or the title alone when the command has no key. */
  label(title: string, key: TipKey) {
    const text = this.#keys[key];
    return text ? `${title} (${text})` : title;
  }
}

/** Seconds as typed in 时间跨度设置 (5, 0.5, 300). */
const seconds = (value: number) => String(Math.round(value * 10) / 10);

/** G-07: the bar's tooltips from the current state, keys and first seek tier. */
export function barTips(
  keys: Keys,
  state: { playing: boolean; muted: boolean; volume: number; remaining: boolean; step: number; sidebarHidden: boolean },
) {
  return {
    tipPlayPause: keys.label(state.playing ? "暂停" : "播放", "play-pause"),
    tipStop: keys.label("停止", "stop"),
    tipBack: `${keys.label(`后退 ${seconds(state.step)} 秒`, "seek-backward")}\n右键：${keys.label("播放上一个", "previous")}`,
    tipForward: `${keys.label(`前进 ${seconds(state.step)} 秒`, "seek-forward")}\n右键：${keys.label("播放下一个", "next")}`,
    tipOpen: keys.label("打开文件", "open"),
    tipTime: state.remaining ? "点击切换已播放时间" : "点击切换剩余时间",
    tipMute: keys.label(state.muted ? "取消静音" : "静音", "mute"),
    tipVolume: `音量: ${Math.round(state.volume)}%`,
    tipSidebar: keys.label("播放列表", "sidebar") + (state.sidebarHidden ? "（窗口太窄，放不下播放列表）" : ""),
  };
}
