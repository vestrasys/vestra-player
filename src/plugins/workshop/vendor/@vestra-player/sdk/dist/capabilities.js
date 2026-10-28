import { capabilityInfo } from "./capability-rules.js";
/**
 * Every manifest capability the player knows (`vestra/1`), in the player's
 * order, with the members it opens, the capabilities they also need and the
 * store's wording. Built from the player's own capability table (its
 * `src/host/capabilities.json`, copied into the SDK) and the requests each
 * member sends, the same rule `vestra-plugin check` and the SDK's
 * TypeScript plugin apply; the player's tests check that the copy is
 * current.
 *
 * A member not listed under any capability needs none (`plugin.tools`,
 * `plugin.storage`, `plugin.system`, `plugin.app`, `plugin.ui`,
 * `plugin.i18n`, reading `plugin.window.state`, `plugin.appearance.theme`
 * and `plugin.packages.list`).
 *
 * @example Check in a test that the manifest declares every required capability
 * The test fails, naming the missing capability, when the manifest declares
 * a capability without one its members also need. It reads the plugin's own
 * `manifest.json` and looks each declared name up in `CAPABILITY_INFO`,
 * whose `requires` lists the capabilities that must be declared with it;
 * at run time the player would refuse such a call with code `capability`.
 * ```ts
 * import assert from "node:assert/strict";
 * import { readFile } from "node:fs/promises";
 * import { CAPABILITY_INFO } from "@vestra-player/sdk/testing";
 *
 * const manifest: { capabilities: string[] } = JSON.parse(
 *   await readFile(new URL("../manifest.json", import.meta.url), "utf8"),
 * );
 * for (const capability of manifest.capabilities)
 *   // An unknown name has no entry: `vestra-plugin check` reports it.
 *   for (const needed of CAPABILITY_INFO[capability]?.requires ?? [])
 *     assert.ok(manifest.capabilities.includes(needed), needed);
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export const CAPABILITY_INFO = Object.freeze(Object.fromEntries(Object.entries(capabilityInfo()).map(([name, info]) => [
    name,
    Object.freeze({
        apis: Object.freeze(info.apis),
        requires: Object.freeze(info.requires),
        label: info.label,
        risky: info.risky,
    }),
])));
/**
 * Manifest capabilities the player knows (`vestra/1`): the keys of
 * {@link CAPABILITY_INFO}, in the player's order. The same list as the
 * player's `host/capabilities.json`; the player's tests check that they agree.
 */
export const CAPABILITIES = Object.freeze(Object.keys(CAPABILITY_INFO));
/** The plugin API version this SDK implements (`manifest.apiVersion`). */
export const API_VERSION = "vestra/1";
/**
 * The longest message a plugin may send the player, in UTF-8 bytes of one
 * protocol line with its newline (1 MiB). The player stops a plugin that
 * writes a longer one, so the plugin's side refuses such a request before
 * sending it: it rejects with code `limit`. Property values meet
 * {@link UI_VALUE_LIMITS} before this.
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export const MESSAGE_LIMIT = 1024 * 1024;
