/**
 * The colours of the player's palettes, by `Theme` colour token (without the
 * `Theme.` prefix): `vestra`, the dark default, and `vestra-light` (player
 * 0.4.0). The player sets one of them on every view; a plugin reads the
 * colours through `Theme` in its Slint files and never needs these values.
 * They are here for the documentation (Reference: theme) and for tests that
 * check a design against both palettes.
 *
 * Current values: a later player may adjust them, a token keeps its
 * meaning. The `vestra` values are also the defaults of the SDK's copy of
 * the palette (`slint/palette.slint`); the player's tests check both
 * palettes against its own.
 *
 * @example
 * ```ts
 * import { THEME_PALETTES } from "@vestra-player/sdk/testing";
 *
 * const accent = THEME_PALETTES["vestra-light"].accent; // "#d4128a"
 * ```
 *
 * @since 1.5.14
 */
export declare const THEME_PALETTES: Readonly<Record<"vestra" | "vestra-light", Readonly<Record<string, string>>>>;
