import type { ScopeImpl } from "./scope.ts";
import type { RenderOptions, RenderSettings, VideoView, ViewSettings } from "./types.ts";
export declare const VIEW_DEFAULTS: ViewSettings;
/** Why `values` would be refused (`null` when valid); the player's own rules. */
export declare function viewProblem(values: unknown): string | null;
export declare const RENDER_DEFAULTS: RenderSettings;
export declare const TONE_MAPPINGS: readonly ["auto", "off", "bt.2390", "hable", "mobius", "reinhard", "clip"];
export declare const MAX_SHADERS = 16;
/**
 * Why `values` would be refused (`null` when valid). The player also refuses
 * shader files that do not exist; that is checked there, not here.
 */
export declare function renderProblem(values: unknown): string | null;
export declare function videoView(scope: ScopeImpl): VideoView;
export declare function videoRender(scope: ScopeImpl): RenderOptions;
