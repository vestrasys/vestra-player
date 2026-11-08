/**
 * The colours of the player's palettes, by `Theme` colour token (without the
 * `Theme.` prefix): `vestra`, the dark default, and `vestra-light`. The player
 * sets one of them on every view; a plugin reads the colours through `Theme` in
 * its Slint files and never needs these values. They are here for the
 * documentation (Reference: theme) and for tests that check a design against
 * both palettes.
 *
 * Current values: a later player may adjust them, a token keeps its
 * meaning. The `vestra` values are also the defaults of the SDK's copy of
 * the palette (`slint/palette.slint`); the player's tests check both
 * palettes against its own.
 *
 * @example Check in a test that a colour stays readable in both themes
 * The test fails if the plugin's own highlight colour gets too close to
 * the panel background of either palette. Plugins draw with `Theme` tokens
 * in Slint and the player switches palettes; a colour the plugin picks
 * itself does not switch, so a test compares it with each palette's values
 * (hex strings, `#rrggbb`).
 * ```ts
 * import assert from "node:assert/strict";
 * import { THEME_PALETTES } from "@vestra-player/sdk/testing";
 *
 * const highlight = "#e07000"; // the plugin's own orange
 * const luminance = (hex: string) => {
 *   const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
 *   return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!; // rough, without gamma
 * };
 * for (const [name, palette] of Object.entries(THEME_PALETTES)) {
 *   const gap = Math.abs(luminance(highlight) - luminance(palette["bg-panel"]!));
 *   assert.ok(gap > 0.3, `${name}: highlight too close to bg-panel`);
 * }
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export declare const THEME_PALETTES: Readonly<Record<"vestra" | "vestra-light", Readonly<Record<string, string>>>>;
