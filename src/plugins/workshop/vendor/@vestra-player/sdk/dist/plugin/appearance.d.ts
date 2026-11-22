import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Appearance, LanguagePreference, ThemeId } from "./types.ts";
/** The palettes this SDK knows, the default first. */
export declare const THEMES: readonly ThemeId[];
/**
 * One per plugin: the value comes with the `window.state` read before
 * activation; `appearance.changed` keeps it current while the plugin
 * listens to that event (`player.on`), and this plugin's own `set` does.
 */
declare class AppearanceService {
    #private;
    theme: ThemeId;
    /** The saved interface language (the next start's). */
    language: LanguagePreference;
    constructor(runtime: Runtime);
    start(): void;
    /** An event's theme, or this plugin's own `set`: only a change counts. */
    accept(theme: ThemeId): void;
}
export declare const appearanceOf: (runtime: Runtime) => AppearanceService;
export declare function appearance(scope: ScopeImpl): Appearance;
export {};
