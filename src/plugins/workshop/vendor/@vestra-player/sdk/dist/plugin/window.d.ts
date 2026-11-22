import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { WindowFacts } from "../wire.ts";
import type { MainWindow, WindowState } from "./types.ts";
/**
 * One per plugin: read before activation, then kept current from
 * `window.state` events while something holds them (`onChange`, a
 * `window.state` listener), from `window.changed`, and after this
 * plugin's own changes.
 */
export declare class WindowService {
    #private;
    state: WindowState;
    constructor(runtime: Runtime);
    init(): Promise<void>;
    /**
     * Follow `window.state` events once the plugin uses `plugin.window` (the
     * shell sends one per layout change, so plugins that never look at the
     * window do not receive them).
     */
    start(): void;
    /** The palette, as the `window.state` reply carried it (`plugin.appearance`). */
    appearance: string | undefined;
    /** The saved interface language, as the reply carried it (`plugin.appearance.language`). */
    language: string | undefined;
    accept(facts: WindowFacts): void;
    read(): Promise<void>;
    watch(listener: (state: WindowState) => void): {
        dispose: () => Promise<void>;
    };
}
/**
 * `window.changed`'s data as the listener gets it (`ScopeImpl.fact`,
 * SDK 1.11.0): the copy takes the facts first, so `plugin.window.state`
 * holds them when the listener runs.
 */
export declare function windowChanged(runtime: Runtime, data: unknown): WindowState;
export declare const windowOf: (runtime: Runtime) => WindowService;
export declare function mainWindow(scope: ScopeImpl): MainWindow;
