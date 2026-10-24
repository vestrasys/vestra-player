import type { ThemeId } from "@vestra-player/sdk";

/** The palette a fresh player starts with, and what 恢复默认 returns to. */
export const DEFAULT_THEME: ThemeId = "vestra";

/** A colour as the page's Slint takes it (`rgb(r, g, b)`): the page has no colour literals. (Types, not interfaces: they are page property values, `Json`.) */
export type Rgb = { r: number; g: number; b: number };

/**
 * The colours one 120×40 preview block draws: the video area with a play
 * glyph (`text`; the video area is dark in every palette, so it is light in
 * every preview), the bar with a time (`dim`), the track (`track`, the
 * palette's `line`) and the knob (`accent`). The progress fill is four stops
 * `f0`–`f3` at 0, 45, 72 and 100 %, left to right.
 */
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
 * The player's palettes, in menu order. The preview colours copy the
 * player's own tables (bg-video, bg-bar, line, text-dim, accent and the
 * accent-fill stops; doc/product/ui-spec.md §2.1): `Theme` only ever holds
 * the current palette, so the preview of another one cannot come from it.
 */
export const THEMES: ThemeRow[] = [
  {
    id: "vestra",
    title: "Vestra 深色",
    note: "默认，取自标志的极光色谱",
    swatch: {
      video: rgb("#0a090e"),
      bar: rgb("#16141c"),
      track: rgb("#2b2734"),
      text: rgb("#f5f3fa"),
      dim: rgb("#8e88a0"),
      accent: rgb("#f72aa6"),
      f0: rgb("#fcc402"),
      f1: rgb("#fa0aa8"),
      f2: rgb("#7a0afa"),
      f3: rgb("#10a8fc"),
    },
  },
  {
    id: "vestra-light",
    title: "Vestra 浅色",
    note: "适合明亮环境；视频区保持深色",
    swatch: {
      video: rgb("#0e0d12"),
      bar: rgb("#f7f6fa"),
      track: rgb("#dcd8e4"),
      text: rgb("#f5f3fa"),
      dim: rgb("#6f6a7f"),
      accent: rgb("#d4128a"),
      f0: rgb("#f5a700"),
      f1: rgb("#e0089a"),
      f2: rgb("#6a08e8"),
      f3: rgb("#0a95e6"),
    },
  },
];

/** The menu and OSD name of a palette. */
export const titleOf = (theme: ThemeId) => THEMES.find((row) => row.id === theme)?.title ?? theme;

export const isTheme = (value: unknown): value is ThemeId => THEMES.some((row) => row.id === value);
