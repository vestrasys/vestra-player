import type { Json, LocalServices, Transport, Wire } from "../wire.ts";
import type { App, Disposable, Locale, PluginError, SystemInfo } from "./types.ts";
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
    constructor(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, app?: unknown);
    can(capability: string): boolean;
    /** Lazily created singletons shared by every scope of this plugin. */
    service<T>(name: string, create: () => T): T;
    /**
     * Registrations started while `activate` runs must succeed before the
     * plugin reports ready; later ones report failures to their scope.
     */
    starting<T>(work: Promise<T>, report: (error: unknown) => void): Promise<T>;
    activated(): Promise<void>;
    bind(evaluate: () => void): Disposable;
    /** Coalesce: one evaluation pass per microtask turn, however many changes. */
    invalidate(): void;
    /**
     * Host events with one `events.subscribe` per name, however many listeners.
     * Without the `events` capability the listener simply never fires.
     */
    on(event: string, listener: Listener, 
    /** The capability the event needs; `null`: none. */
    needs?: string | null): Disposable;
    /** Raw transport events the host pushes without a subscription. */
    raw(event: string, listener: Listener): Disposable;
}
/** `plugin.app` from the start-up handshake (the runner's environment). */
export declare function appInfo(raw: unknown, system?: () => Promise<SystemInfo>): App;
export {};
