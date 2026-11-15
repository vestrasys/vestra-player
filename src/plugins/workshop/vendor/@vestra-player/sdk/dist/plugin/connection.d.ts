import { RequestQueue } from "../request-queue.ts";
import type { Json, LocalServices, Transport, Wire } from "../wire.ts";
import type { App, Disposable, Locale, StopReason, SystemInfo } from "./types.ts";
/**
 * Why something listens, for the player's log line that says why a plugin keeps
 * running (`events.subscribe { why }`): computed only when the player is told.
 */
export type Why = () => string;
/**
 * Events the player wakes a stopped plugin for: a listener of one does not keep
 * the plugin running. They follow deliberate changes (an install, a key edit,
 * the palette, a resolver), not playback, or report one thing that happened
 * to a source (it opened, became ready, ended), never its stream of states.
 */
export declare const WAKE_EVENTS: readonly string[];
/**
 * What the runtime reads of the plugin's manifest: its id. Nothing in the
 * manifest switches behaviour (1.9.0): event delivery and the start-up
 * reads are the plugin's explicit calls (`plugin.player.subscribe()`,
 * `plugin.player.read()`).
 */
export interface PluginManifest {
    id: string;
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
 * How primitives tell the framework above them about plugin-visible state
 * they changed or read, without importing it (refactor plan §2.1: L1 never
 * imports the passes). `Runtime` installs the update passes here; on a
 * bare connection every call does nothing.
 */
export interface StateSignals {
    /** `key`'s `field` (without one: all of it) is read by what is evaluated now. */
    read(key: string, field?: string): void;
    /** Fields of `key` changed: what read them is evaluated again. */
    changed(key: string, fields: Iterable<string>): void;
    /** Something the plugin sees changed that no key describes: everything is evaluated again. */
    invalidate(): void;
}
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
    /**
     * Every request of this plugin goes through it: at most
     * `REQUEST_LIMITS` in flight, the rest wait in order (`request-queue.ts`).
     */
    readonly requests: RequestQueue;
    /**
     * The player's state is read again after the plugin's own actions:
     * `plugin.player.read()` was called.
     */
    reads: boolean;
    readonly views: Map<string, (callback: string, args: Json[]) => void>;
    readonly commands: Map<string, () => void>;
    /**
     * Command unregistrations in flight, by id: a command registered again
     * with the id (`commands.add` or `plugin.wire.commands.register`) waits
     * for its removal, or the player would refuse it with `conflict`.
     */
    readonly removals: Map<string, Promise<void>>;
    readonly services: Map<string, unknown>;
    /**
     * How to read a state copy again, by the event that keeps it current: a
     * mirror registers itself (`source.state` → the player's), an action that
     * changed that state elsewhere asks for it (`reread`).
     */
    readonly rereads: Map<string, () => Promise<unknown>>;
    /**
     * Where a state copy (mirror) that threw on an event is reported: the
     * plugin's error handlers once the root scope exists (`onError`, as for
     * a listener), the log before.
     */
    reportMirror: (error: unknown) => void;
    /** The framework's ears ({@link StateSignals}); silent until `Runtime` sets them. */
    signals: StateSignals;
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
     * handshake's app data): `ui.patch` (row patches of property writes). Not
     * part of the plugin API.
     */
    readonly features: ReadonlySet<string>;
    constructor(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, app?: unknown, options?: RuntimeOptions);
    /** Event delivery is on (`plugin.player.subscribe()` was called). */
    get delivering(): boolean;
    /**
     * Turn event delivery on (`plugin.player.subscribe()`): every later
     * listener is subscribed, and so is what was added before and heard
     * nothing. Idempotent.
     */
    deliver(): void;
    /** Read the state copy kept by `event` again, if one exists; never rejects. */
    reread(event: string): Promise<void>;
    /** Lazily created singletons shared by every scope of this plugin. */
    service<T>(name: string, create: () => T): T;
    /**
     * Registrations started while `activate` runs must succeed before the
     * plugin reports ready; later ones report failures to their scope.
     */
    starting<T>(work: Promise<T>, report: (error: unknown) => void): Promise<T>;
    /**
     * A declaration the player checks on its own (`what`: "command x"), started
     * like `starting`. In the player's runner a refusal of it during activation
     * (`invalid`, `conflict`, `limit`) does not fail the plugin: it is logged,
     * skipped and reported to the scope once activation completes, so an
     * `onError` added anywhere in `activate` hears it (VS Code logs a bad
     * contribution and activates the extension). Under `strictDeclarations`
     * (TestHost) it fails activation as before.
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
     * An event that needs delivery (`"events"`) is subscribed only once
     * `plugin.player.subscribe()` was called; until then its listener simply
     * never fires.
     * `mirror`: the SDK's own copy of the state (`plugin.player.state` and
     * the like), not a listener of the plugin's (see {@link EventGroup}).
     */
    on(event: string, listener: Listener, 
    /** `"events"`: it arrives after `plugin.player.subscribe()`; `null`: always. */
    needs?: "events" | null, mirror?: boolean): Disposable;
    /**
     * Count `event` as listened to while held: a watcher of a service's
     * state, a binding that reads it. Nothing when nothing subscribed to it
     * (no `plugin.player.subscribe()`, or the service did not start).
     */
    hold(event: string, why?: Why): Disposable;
    /**
     * Hear `event` while followed, whether or not event delivery is on, and
     * count it as listened to (`plugin.player.stream`, SDK 1.10.0): the group
     * the caller started (a state copy's) is subscribed now if it waited for
     * `plugin.player.subscribe()`, and only it. When the last follower ends,
     * such a group waits again as before: its subscription at the player
     * ends, and the copy stays as it was. Nothing when nothing started the
     * group.
     */
    follow(event: string, why?: Why): Disposable;
    /**
     * Why the plugin listens to each event it really listens to (mirrors
     * left out), as the player's log says it: for {@link TestHost} and
     * `vestra-plugin test`.
     */
    listening(): Map<string, string[]>;
    /**
     * One request by its method name, as given (`plugin.wire.request`): nothing
     * added, nothing checked here; the player checks the request. Every other
     * part of the SDK uses the typed clients (`host`); `scripts/check-layers.mjs`
     * keeps it so. It waits in the request queue like every other.
     */
    request<T>(method: string, params?: Json): Promise<T>;
    /** Raw transport events the host pushes without a subscription. */
    raw(event: string, listener: Listener): Disposable;
}
/** `plugin.app` from the start-up handshake (the runner's environment). */
export declare function appInfo(raw: unknown, system?: () => Promise<SystemInfo>): App;
export {};
