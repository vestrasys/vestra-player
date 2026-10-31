import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { WindowFacts } from "../wire.ts";
import type { MainWindow, WindowState } from "./types.ts";
/**
 * One per plugin: read before activation, then kept current from
 * `window.state` events (with `events`) and after this plugin's own changes.
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
    /** The file resolvers' extensions, as the reply carried them (`plugin.resolvers.fileTypes`). */
    fileTypes: string[] | undefined;
    /** The saved interface language, as the reply carried it (`plugin.appearance.language`). */
    language: string | undefined;
    accept(facts: WindowFacts): void;
    read(): Promise<void>;
    watch(listener: (state: WindowState) => void): {
        dispose: () => Promise<void>;
    };
}
export declare const windowOf: (runtime: Runtime) => WindowService;
export declare function mainWindow(scope: ScopeImpl): MainWindow;
