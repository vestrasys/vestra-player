import type { BindingInfo, Plugin } from "@vestra-player/sdk";

/**
 * The bar elements whose tooltip names a key (G-07), and the player's own
 * actions whose keys the Playback panel shows (speed, frames, the first seek step).
 */
export type TipKey =
  | "play-pause"
  | "stop"
  | "previous"
  | "next"
  | "mute"
  | "open"
  | "fullscreen"
  | "rate-down"
  | "rate-up"
  | "rate-normal"
  | "frame-back"
  | "frame-forward"
  | "seek-backward"
  | "seek-forward";

/**
 * The commands' default keys as the menu labels them, used until (or when) the
 * binding table cannot be read.
 */
export const DEFAULT_KEYS: Record<TipKey, string> = {
  "play-pause": "Space",
  stop: "F4",
  previous: "PgUp",
  next: "PgDn",
  mute: "M",
  open: "Ctrl+O",
  // Player 0.6.0: Toggle Fullscreen is Escape only by default (Enter, Alt+Enter and F11 removed).
  fullscreen: "Escape",
  "rate-down": "X",
  "rate-up": "C",
  "rate-normal": "Z",
  "frame-back": "D",
  "frame-forward": "F",
  "seek-backward": "←",
  "seek-forward": "→",
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
 * Kept in `plugin.state`: a binding that reads a key follows a new table.
 */
export class Keys {
  readonly #st: { keys: Record<TipKey, string | null> };
  readonly #plugin: Plugin;

  constructor(plugin: Plugin) {
    this.#plugin = plugin;
    this.#st = plugin.state({ keys: { ...DEFAULT_KEYS } as Record<TipKey, string | null> });
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
    this.#st.keys = {
      "play-pause": host("play-pause"),
      stop: host("stop"),
      previous: host("previous"),
      next: host("next"),
      mute: host("mute"),
      open: find((info) => info.plugin === this.#plugin.id && info.command === "open"),
      fullscreen: host("fullscreen"),
      "rate-down": host("rate-down"),
      "rate-up": host("rate-up"),
      "rate-normal": host("rate-normal"),
      "frame-back": host("frame-back"),
      "frame-forward": host("frame-forward"),
      "seek-backward": host("seek-backward"),
      "seek-forward": host("seek-forward"),
    };
  }

  /** The key's menu text, "" when the command has no key. */
  key(key: TipKey) {
    return this.#st.keys[key] ?? "";
  }
}

/**
 * G-07: what the bar's tooltips show, as data: the keys ("" for none) and the
 * volume. `ui/bar.slint` words the tooltips.
 */
export function barTips(keys: Keys, state: { volume: number }) {
  return {
    keyPlayPause: keys.key("play-pause"),
    keyStop: keys.key("stop"),
    keyPrevious: keys.key("previous"),
    keyNext: keys.key("next"),
    keyOpen: keys.key("open"),
    keyMute: keys.key("mute"),
    keyFullscreen: keys.key("fullscreen"),
    volumePercent: Math.round(state.volume),
  };
}
