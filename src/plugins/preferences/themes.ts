import type { ThemeId } from "@vestra-player/sdk";

/** The palette a fresh player starts with, and what 恢复默认 returns to. */
export const DEFAULT_THEME: ThemeId = "vestra";

/** A colour as the page's Slint takes it (`rgb(r, g, b)`): the page has no colour literals. (Types, not interfaces: they are page property values, `Json`.) */
export type Rgb = { r: number; g: number; b: number };

/** The colours one 120×40 preview block draws. The progress fill is four stops `f0`–`f3`, left to right (one colour four times for a solid fill). */
export type Swatch = {
  video: Rgb;
  bar: Rgb;
  track: Rgb;
  text: Rgb;
  dim: Rgb;
  accent: Rgb;
  f0: Rgb;
  f1: Rgb;
  f2: Rgb;
  f3: Rgb;
};

export type ThemeRow = { id: ThemeId; title: string; note: string; swatch: Swatch };

const rgb = (hex: string): Rgb => {
  const value = Number.parseInt(hex.slice(1), 16);
  return { r: (value >> 16) & 0xff, g: (value >> 8) & 0xff, b: value & 0xff };
};

/**
 * Both palettes, in menu order. The preview colours copy the player's own
 * tables (bg-video, bg-bar, button-bg, text, text-dim, accent, accent-fill;
 * doc/product/ui-alignment.md §2.1): `Theme` only ever holds the current
 * palette, so the preview of the other one cannot come from it.
 */
export const THEMES: ThemeRow[] = [
  {
    id: "vestra",
    title: "Vestra",
    note: "品牌配色：琥珀 → 洋红 → 紫渐变（默认）",
    swatch: {
      video: rgb("#0b0a0f"),
      bar: rgb("#17141c"),
      track: rgb("#322c3b"),
      text: rgb("#f2eff7"),
      dim: rgb("#8a8496"),
      accent: rgb("#fa139c"),
      f0: rgb("#fdb506"),
      f1: rgb("#fc6f3b"),
      f2: rgb("#fa139c"),
      f3: rgb("#911efc"),
    },
  },
  {
    id: "classic",
    title: "经典",
    note: "经典配色：黄色强调，蓝色正在播放",
    swatch: {
      video: rgb("#101010"),
      bar: rgb("#1e1e1e"),
      track: rgb("#555555"),
      text: rgb("#f0f0f0"),
      dim: rgb("#777777"),
      accent: rgb("#fae100"),
      f0: rgb("#fae100"),
      f1: rgb("#fae100"),
      f2: rgb("#fae100"),
      f3: rgb("#fae100"),
    },
  },
];

/** The menu and OSD name of a palette. */
export const titleOf = (theme: ThemeId) => THEMES.find((row) => row.id === theme)?.title ?? theme;

export const isTheme = (value: unknown): value is ThemeId => THEMES.some((row) => row.id === value);
