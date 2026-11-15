/**
 * Builds the `plugin` object. Plugins import the types from the package entry
 * (`@vestra-player/sdk`, src/index.ts); `createPlugin` is for the player's
 * runner (`@vestra-player/sdk/runtime`) and the test host only.
 *
 * @internal
 */
import type { Transport } from "../wire.ts";
import type { PluginManifest, RuntimeOptions } from "./connection.ts";
import type { Plugin } from "./types.ts";
export type * from "./types.ts";
export type { PluginManifest, RuntimeOptions };
export interface PluginInstance {
    plugin: Plugin;
    /**
     * Call the plugin's `activate(plugin)` as `entry`: `plugin.trigger()`
     * works only during its synchronous part, after which the player is told
     * which triggers it declared (1.10.0). Returns what `entry` returned, as a
     * promise. The runner and the test host call it instead of `entry`.
     */
    activate(entry: (plugin: Plugin) => unknown): Promise<unknown>;
    /**
     * Wait for registrations started during activation; rejects if one failed.
     * A command declaration the player refused is not a failure (it is
     * logged, skipped and reported to `onError`) unless
     * `RuntimeOptions.strictDeclarations` is set.
     */
    activated(): Promise<void>;
    /**
     * Stop the plugin: dispose the root scope, then stop the runtime.
     * `info.reason` is why: the runner passes `runtime.stop`'s `data.reason`, or
     * `"host-lost"` when its input closed without a `runtime.stop`. It reaches
     * `plugin.stopReason` and every `onDispose` cleanup; a reason the SDK does
     * not know reads as `undefined`. A second call runs no cleanup again and
     * keeps the first reason.
     */
    dispose(info?: {
        reason?: unknown;
    }): Promise<void>;
}
export declare function createPlugin(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, 
/** `plugin.app` (the runner passes the player's `VESTRA_APP_INFO`). */
app?: unknown, 
/** `TestHost` sets `strictDeclarations`; the player's runner does not. */
options?: RuntimeOptions): Promise<PluginInstance>;
