/** Scopes own handles. Disposal runs newest first and never throws. */
import type { MotionEngine } from "./types.ts";
import type { Runtime } from "./runtime.ts";
import type { Disposable, DisposeInfo, Plugin, PluginError, Scope, StopReason, Trigger } from "./types.ts";
export declare class ScopeImpl implements Plugin {
    #private;
    readonly runtime: Runtime;
    constructor(runtime: Runtime, parent?: ScopeImpl);
    get id(): string;
    get dataDirectory(): string;
    get stopReason(): StopReason | undefined;
    get disposed(): boolean;
    /** The plugin itself, not a scope made with `scope()`. */
    get root(): boolean;
    /**
     * Start the SDK's copy that `event` keeps current (`plugin.commands.keys`,
     * `plugin.timeline`, `plugin.appearance`), for `player.on`; nothing for
     * an event without one.
     */
    followCopy(event: string): void;
    /** A fact event's data as `player.on`'s listener gets it ({@link FACTS}). */
    fact(event: string, data: unknown): unknown;
    readonly commands: import("./types.ts").Commands;
    readonly when: import("./types.ts").When;
    readonly windows: import("./types.ts").Windows;
    readonly panels: import("./types.ts").Panels;
    readonly dialogs: import("./types.ts").Dialogs;
    readonly player: import("./types.ts").Player;
    readonly video: {
        view: () => import("./types.ts").VideoView;
        render: () => import("./types.ts").RenderOptions;
        shaders: (id?: string) => import("./types.ts").VideoShaders;
        upscaling: import("./types.ts").VideoUpscaling;
        motion: (engine: MotionEngine) => import("./types.ts").VideoMotion;
        filter(id: string): import("./types.ts").VideoFilter;
    };
    readonly audio: {
        filter(id: string): import("./types.ts").AudioFilter;
    };
    readonly subtitles: import("./types.ts").Subtitles;
    readonly media: import("./types.ts").MediaCapture;
    readonly resolvers: import("./types.ts").Resolvers;
    readonly packages: import("./types.ts").Packages;
    readonly osd: import("./types.ts").Osd;
    readonly palette: import("./types.ts").Palette;
    readonly host: import("./types.ts").Host;
    readonly clipboard: import("./types.ts").Clipboard;
    readonly power: import("./types.ts").Power;
    readonly tools: import("./types.ts").Tools;
    readonly storage: import("./types.ts").Storage;
    readonly system: import("./types.ts").System;
    readonly secrets: import("./types.ts").Secrets;
    readonly mpv: import("./types.ts").Mpv;
    readonly rendering: import("./types.ts").Rendering;
    readonly window: import("./types.ts").MainWindow;
    readonly bindings: import("./types.ts").Bindings;
    readonly ui: import("./types.ts").Ui;
    readonly timeline: import("./types.ts").Timeline;
    readonly drops: import("./types.ts").Drops;
    readonly appearance: import("./types.ts").Appearance;
    readonly settings: import("./types.ts").Settings;
    readonly i18n: import("./types.ts").I18n;
    readonly wire: import("./types.ts").Wire;
    get app(): import("./types.ts").App;
    scope(): Scope;
    trigger(name?: string): Trigger;
    add<T extends Disposable>(item: T): T;
    /** Handles call this when disposed directly, so the scope forgets them. */
    forget(item: Disposable): void;
    onDispose(cleanup: ((info?: DisposeInfo) => void | Promise<void>) | (() => void | Promise<void>)): Disposable;
    onError(listener: (error: PluginError) => void): Disposable;
    report(error: unknown): void;
    /** Invoke plugin code on its behalf: report failures, then refresh bindings. */
    run(handler: (...args: never[]) => unknown, ...args: unknown[]): void;
    /**
     * Run a listener that pushes its own values (`player.on`) like `run`, except
     * that no full pass follows it: what it writes to `plugin.state` runs the
     * passes for its readers, and for anything else the plugin calls `update()`.
     * Its reads are not counted as a binding's.
     */
    runQuiet(handler: (...args: never[]) => unknown, ...args: unknown[]): void;
    update(): void;
    state<T extends object>(initial: T): T;
    dispose(): Promise<void>;
}
