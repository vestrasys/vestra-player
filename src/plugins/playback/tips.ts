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
 * The top-level menu item whose key the sidebar toggle shows, as the binding
 * table names it (the menu text in the interface language): the playlist's
 * English title and its Chinese translation. The table has no
 * language-independent path yet (round doc, 第 2 轮请求).
 */
// i18n-ignore: other plugins' menu titles, compared with, never shown
const SIDEBAR_ITEM: ReadonlySet<string> = new Set(["Playlist", "播放列表"]);

/**
 * The bar elements' keys from the binding table (`plugin.bindings`), so a key
 * the user rebinds shows in the tooltip; the labels are the host's menu text.
 * The sidebar toggle has no command of its own: its key is that of the top-level
 * menu item Playlist ({@link SIDEBAR_ITEM}), whoever registers it (none: no key).
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
      sidebar: find((info) => info.path?.length === 1 && SIDEBAR_ITEM.has(info.path[0]!)),
    };
    this.#changed();
  }

  /** The key's menu text, "" when the command has no key. */
  key(key: TipKey) {
    return this.#keys[key] ?? "";
  }
}

/** Seconds as typed in Seek Steps (5, 0.5, 300). */
const seconds = (value: number) => String(Math.round(value * 10) / 10);

/**
 * G-07: what the bar's tooltips show, as data: the keys ("" for none), the
 * first seek tier and the volume. `ui/bar.slint` words the tooltips.
 */
export function barTips(keys: Keys, state: { volume: number; step: number }) {
  return {
    keyPlayPause: keys.key("play-pause"),
    keyStop: keys.key("stop"),
    keyBack: keys.key("seek-backward"),
    keyForward: keys.key("seek-forward"),
    keyPrevious: keys.key("previous"),
    keyNext: keys.key("next"),
    keyOpen: keys.key("open"),
    keyMute: keys.key("mute"),
    keySidebar: keys.key("sidebar"),
    stepText: seconds(state.step),
    volumePercent: Math.round(state.volume),
  };
}
