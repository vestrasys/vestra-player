import type { Json, LocalServices, Transport, Wire } from "../wire.ts";
import type { App, Disposable, Locale, PluginError, SystemInfo } from "./types.ts";
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
export declare const sleep: (ms: number) => Promise<void>;
export declare function toError(error: unknown): PluginError;
export declare const failure: (code: string, message?: string) => PluginError;
/** Host replies are definite; anything else (lost reply, client timeout) is not. */
export declare const uncertain: (error: unknown) => boolean;
/** Mark a promise as observed so an unused `ready` never crashes the runner. */
export declare function quiet<T>(promise: Promise<T>): Promise<T>;
/**
 * The fields of the SDK's state copies that a binding or listener read, by
 * state (`player`: `plugin.player.state`). An empty set: the state was
 * taken whole (no field read was seen), so any change of it counts.
 */
export type Deps = Map<string, Set<string>>;
/** A change of the SDK's state copies: the fields that changed, by state. */
export type Change = ReadonlyMap<string, ReadonlySet<string>>;
/**
 * The longest a binding waits for a full pass while precise passes skip
 * it: state a plugin changes outside code the SDK runs (its own timer,
 * without `update()`) still shows within about this long.
 */
export declare const SAFETY_PASS_MS = 1000;
/** How the plugin is run: the player's runner, or `TestHost`. */
export interface RuntimeOptions {
    /**
     * A declaration the player refuses during activation fails activation
     * (`startPlugin`, so a plugin's tests show it) instead of being logged,
     * reported to `onError` and skipped (the player's runner).
     */
    strictDeclarations?: boolean;
}
export declare const same: (a: Json | undefined, b: Json | undefined) => boolean;
/** One value in flight, only the newest waiting. Each push learns its fate. */
export declare class Latest<T> {
    #private;
    constructor(send: (value: T) => Promise<void>);
    push(value: T): Promise<boolean>;
    stop(): void;
}
type Listener = (data: unknown) => void;
/**
 * Per-plugin services. Scopes are thin views over this object; the flat RPC
 * clients stay private here so protocol fields never reach plugin code.
 */
export declare class Runtime {
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
    bind(evaluate: () => void): Disposable;
    /**
     * A service's state for `event` is read: a binding evaluated now depends
     * on it, so the event counts as listened to while the binding reads it.
     */
    reading(event: string): void;
    /**
     * A field of the SDK's copy of a state (`key`) is read; without `field`,
     * the state is taken (`plugin.player.state`). What is being evaluated
     * then depends on it: a precise pass after a change of that field runs
     * it again.
     */
    depends(key: string, field?: string): void;
    /**
     * Evaluate one part of the binding being evaluated (one `view.bind`
     * function), identified by `token`. A precise pass skips it when nothing
     * it read last time changed, and returns `undefined`; what it read then
     * still counts for the binding.
     */
    part<T>(token: object, read: () => T): {
        value: T;
    } | undefined;
    /**
     * Run plugin code (a state listener) and return the state copy fields it
     * read; they are not counted as a binding's.
     */
    observe<T>(run: () => T): {
        value: T;
        deps: Deps;
    };
    /**
     * Whether code that read `deps` while `change` was delivered needs a full
     * pass after it: it read a field that changed, or no field of that state
     * at all (so the SDK cannot tell what it depends on).
     */
    needsFull(deps: Deps, change: Change): boolean;
    /**
     * Evaluate `read` as `label`, added at `where`: the state it reads is
     * reported as read by it. `field`: a command's `enabled` or `checked`,
     * which a `plugin.when` condition can replace.
     */
    reader<T>(label: string, where: (() => string | undefined) | undefined, read: () => T, field?: boolean): T;
    /**
     * Where the plugin's code called into the SDK: the first stack frame
     * outside the SDK, as a path relative to the plugin's directory and a
     * line (`main.ts:353`). Captured now, formatted when asked.
     */
    callSite(): () => string | undefined;
    /**
     * A full pass: every binding is evaluated. Requested after plugin code
     * the SDK ran, by `plugin.update()`, and on any change other than a state
     * copy's fields. Coalesced: one pass per microtask turn, however many
     * requests.
     */
    invalidate(): void;
    /**
     * Fields of the SDK's copy of a state (`key`) changed: a precise pass
     * evaluates only the bindings (and view bind functions) that read one of
     * them, unless a full pass is due anyway.
     */
    changed(key: string, fields: Iterable<string>): void;
    /** The plugin was disposed: cancel the pending safety pass, arm no more. */
    stop(): void;
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
