/**
 * Builds the `plugin` object. Plugins import the types from the package entry
 * (`@vestra-player/sdk`, src/index.ts); `createPlugin` is for the player's
 * runner (`@vestra-player/sdk/runtime`) and the test host only.
 *
 * @internal
 */
import type { Transport } from "../wire.ts";
import type { PluginManifest } from "./runtime.ts";
import type { Plugin } from "./types.ts";
export type * from "./types.ts";
export type { PluginManifest };
export interface PluginInstance {
    plugin: Plugin;
    /** Wait for registrations started during activation; rejects if one failed. */
    activated(): Promise<void>;
    dispose(): Promise<void>;
}
export declare function createPlugin(transport: Transport, manifest: PluginManifest, directory: string, dataDirectory?: string, 
/** `plugin.app` (the runner passes the player's `VESTRA_APP_INFO`). */
app?: unknown): Promise<PluginInstance>;
