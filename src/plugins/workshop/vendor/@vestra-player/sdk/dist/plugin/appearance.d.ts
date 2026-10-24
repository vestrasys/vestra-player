import type { ScopeImpl } from "./scope.ts";
import type { Appearance, ThemeId } from "./types.ts";
/** The palettes this SDK knows, the default first. */
export declare const THEMES: readonly ThemeId[];
export declare function appearance(scope: ScopeImpl): Appearance;
