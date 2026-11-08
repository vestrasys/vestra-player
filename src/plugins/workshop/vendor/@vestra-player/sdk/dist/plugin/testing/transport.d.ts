import { HostError } from "../../wire.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
export declare const tick: () => Promise<void>;
/**
 * Refusals for a position or resolver another plugin holds: not activation
 * failures (`startPlugin`).
 */
export declare const takenErrors: WeakSet<Error>;
export declare const taken: (message: string) => HostError;
/** Lines `vestra-plugin test` printed already (one per plugin and reasons). */
export declare const told: Set<string>;
/**
 * Under `vestra-plugin test` (`VESTRA_PLUGIN_RESIDENCY=1`): say once per
 * test file why the plugin would keep running after it starts.
 */
export declare function tellResidency(host: TestHost): void;
/** Implements `TestHost.on`. */
export declare function on(_host: TestHost, inner: Internals, event: string, listener: (data: unknown) => void): () => void;
/** Implements `TestHost.emit`. */
export declare function emit(host: TestHost, inner: Internals, event: string, data: unknown): void;
/** Implements `TestHost.settle`. */
export declare function settle(_host: TestHost, inner: Internals, ms?: number): Promise<void>;
/** Implements `TestHost.close`. */
export declare function close(host: TestHost, inner: Internals): void;
/** Implements `TestHost.keepsRunning`. */
export declare function keepsRunning(host: TestHost, _inner: Internals): string[];
/** The default handlers of this area's wire methods, by method name. */
export declare function transportHandlers(host: TestHost, _inner: Internals): Record<string, Handler>;
