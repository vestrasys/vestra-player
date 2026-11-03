/**
 * Builds the `plugin` object. Plugins import the types from the package entry
 * (`@vestra-player/sdk`, src/index.ts); `createPlugin` is for the player's
 * runner (`@vestra-player/sdk/runtime`) and the test host only.
 *
 * @internal
 */
import type { Transport } from "../wire.ts";
import type { PluginManifest, RuntimeOptions } from "./runtime.ts";
import type { Plugin } from "./types.ts";
export type * from "./types.ts";
export type { PluginManifest, RuntimeOptions };
export interface PluginInstance {
    plugin: Plugin;
    /**
     * Wait for registrations started during activation; rejects if one failed.
     * A command declaration the player refused is not a failure (it is
     * logged, skipped and reported to `onError`) unless
     * `RuntimeOptions.strictDeclarations` is set.
     */
    activated(): Promise<void>;
    /**
     * Stop the plugin: dispose the root scope, then stop the runtime.
     * `info.reason` (SDK 1.5.54) is why: the runner passes `runtime.stop`'s
     * `data.reason` (player 0.6.4), or `"host-lost"` when its input closed
     * without a `runtime.stop`. It reaches `plugin.stopReason` and every
     * `onDispose` cleanup; a reason the SDK does not know reads as
     * `undefined`. A second call runs no cleanup again and keeps the first reason.
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
