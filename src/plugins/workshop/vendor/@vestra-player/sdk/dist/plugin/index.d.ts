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
    dispose(): Promise<void>;
}
export declare function createPlugin(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, 
/** `plugin.app` (the runner passes the player's `VESTRA_APP_INFO`). */
app?: unknown, 
/** `TestHost` sets `strictDeclarations`; the player's runner does not. */
options?: RuntimeOptions): Promise<PluginInstance>;
