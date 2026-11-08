import type { Json, LocalServices, Transport, Wire } from "../wire.ts";
import type { App, Disposable, Locale, StopReason, SystemInfo } from "./types.ts";
/**
 * Why something listens, for the player's log line that says why a plugin
 * keeps running (`events.subscribe { why }`, player 0.5.21): computed
 * only when the player is told.
 */
export type Why = () => string;
/**
 * Events the player wakes a stopped plugin for (player 0.5.21): a listener
 * of one does not keep the plugin running. They follow deliberate changes
 * (an install, a key edit, the palette, a resolver), not playback.
 */
export declare const WAKE_EVENTS: readonly string[];
export interface PluginManifest {
    id: string;
    capabilities: string[];
}
/** How the plugin is run: the player's runner, or `TestHost`. */
export interface RuntimeOptions {
    /**
     * A declaration the player refuses during activation fails activation
     * (`startPlugin`, so a plugin's tests show it) instead of being logged,
     * reported to `onError` and skipped (the player's runner).
     */
    strictDeclarations?: boolean;
}
/** A stop reason the runner passed on, if this SDK knows it. */
export declare function stopReason(value: unknown): StopReason | undefined;
type Listener = (data: unknown) => void;
/**
 * Per-plugin connection to the player. Scopes reach it through `Runtime`;
 * the flat RPC clients stay private here so protocol fields never reach
 * plugin code.
 */
export declare class Connection {
    #private;
    readonly host: Wire;
    readonly manifest: PluginManifest;
    readonly directory: string;
    /** Writable per-plugin data directory from the host, if it provided one. */
    readonly dataDirectory: string | undefined;
    /** Test doubles for local processes; the real system when absent. */
    readonly local: LocalServices | undefined;
    /** Registrations that activation must wait for before reporting ready. */
    readonly startup: Set<Promise<unknown>>;
    readonly views: Map<string, (callback: string, args: Json[]) => void>;
    readonly commands: Map<string, () => void>;
    readonly services: Map<string, unknown>;
    /**
     * Set when the player stops the plugin (`PluginInstance.dispose`), before
     * the root scope is disposed: `DisposeInfo` and `plugin.stopReason`.
     */
    stopping: {
        reason: StopReason | undefined;
    } | undefined;
    /** `plugin.app`, from the start-up handshake; `system()` asks the player. */
    readonly app: App;
    /** `plugin.i18n.locale`, from the start-up handshake; English when unknown. */
    readonly locale: Locale;
    /**
     * Protocol additions the player advertises at start-up (`features` in the
     * handshake's app data, player 0.6.0): `ui.patch` (row patches of
     * property writes). Not part of the plugin API; empty for older players.
     */
    readonly features: ReadonlySet<string>;
    constructor(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, app?: unknown, options?: RuntimeOptions);
    can(capability: string): boolean;
    /** Lazily created singletons shared by every scope of this plugin. */
    service<T>(name: string, create: () => T): T;
    /**
     * Registrations started while `activate` runs must succeed before the
     * plugin reports ready; later ones report failures to their scope.
     */
    starting<T>(work: Promise<T>, report: (error: unknown) => void): Promise<T>;
    /**
     * A declaration the player checks on its own (`what`: "command x"),
     * started like `starting`. In the player's runner a refusal of it during
     * activation (`invalid`, `conflict`, `limit`) does not fail the plugin:
     * it is logged, skipped and reported to the scope once activation
     * completes, so an `onError` added anywhere in `activate` hears it (SDK
     * 1.5.49; VS Code logs a bad contribution and activates the extension).
     * Under `strictDeclarations` (TestHost) it fails activation as before.
     */
    declaring<T>(work: Promise<T>, what: string, report: (error: unknown) => void): Promise<T>;
    activated(): Promise<void>;
    /**
     * Where the plugin's code called into the SDK: the first stack frame
     * outside the SDK, as a path relative to the plugin's directory and a
     * line (`main.ts:353`). Captured now, formatted when asked.
     */
    callSite(): () => string | undefined;
    /**
     * Host events with one `events.subscribe` per name, however many listeners.
     * Without the `events` capability the listener simply never fires.
     * `mirror`: the SDK's own copy of the state (`plugin.player.state` and
     * the like), not a listener of the plugin's (see {@link EventGroup}).
     */
    on(event: string, listener: Listener, 
    /** The capability the event needs; `null`: none. */
    needs?: string | null, mirror?: boolean): Disposable;
    /**
     * Count `event` as listened to while held: a watcher of a service's
     * state, a binding that reads it. Nothing when nothing subscribed to it
     * (no capability, or the service did not start).
     */
    hold(event: string, why?: Why): Disposable;
    /**
     * Why the plugin listens to each event it really listens to (mirrors
     * left out), as the player's log says it: for {@link TestHost} and
     * `vestra-plugin test`.
     */
    listening(): Map<string, string[]>;
    /** Raw transport events the host pushes without a subscription. */
    raw(event: string, listener: Listener): Disposable;
}
/** `plugin.app` from the start-up handshake (the runner's environment). */
export declare function appInfo(raw: unknown, system?: () => Promise<SystemInfo>): App;
export {};
