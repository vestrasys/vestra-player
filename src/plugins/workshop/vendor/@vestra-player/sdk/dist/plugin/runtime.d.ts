/**
 * Shared machinery behind every scope: the connection to the player
 * (`connection.ts`, L1) with the update passes (`passes.ts`, L2) on top.
 * `Runtime` is the connection plus the pass methods, delegated, so every
 * caller keeps `runtime.host`, `runtime.on`, `runtime.bind` and the rest.
 */
import { Connection } from "./connection.ts";
import type { PluginManifest, RuntimeOptions } from "./connection.ts";
import { Passes } from "./passes.ts";
import type { Change, Deps, PassCounts } from "./passes.ts";
import type { Transport } from "../wire.ts";
import type { Disposable } from "./types.ts";
export { WAKE_EVENTS } from "./connection.ts";
/** Per-plugin services: the connection, and the update passes over it. */
export declare class Runtime extends Connection {
    /** The update pass scheduler. */
    readonly passes: Passes;
    /** Pass counters, read by the SDK's own tests and benchmark ({@link PassCounts}). */
    readonly counts: PassCounts;
    constructor(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, app?: unknown, options?: RuntimeOptions);
    /** {@link Passes.bind} */
    bind(evaluate: () => void): Disposable;
    /** {@link Passes.reading} */
    reading(event: string): void;
    /** {@link Passes.depends} */
    depends(key: string, field?: string): void;
    /** {@link Passes.part} */
    part<T>(token: object, read: () => T): {
        value: T;
    } | undefined;
    /** {@link Passes.observe} */
    observe<T>(run: () => T): {
        value: T;
        deps: Deps;
    };
    /** {@link Passes.needsFull} */
    needsFull(deps: Deps, change: Change): boolean;
    /** {@link Passes.reader} */
    reader<T>(label: string, where: (() => string | undefined) | undefined, read: () => T, field?: boolean): T;
    /** {@link Passes.invalidate} */
    invalidate(): void;
    /** {@link Passes.changed} */
    changed(key: string, fields: Iterable<string>): void;
    /** {@link Passes.stop} */
    stop(): void;
}
