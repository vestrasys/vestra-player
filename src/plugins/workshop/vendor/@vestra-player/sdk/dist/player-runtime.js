/**
 * `@vestra-player/sdk/runtime`: for the player only (its plugin runner and its
 * own tests). Not part of the plugin API and not covered by semver: it can
 * change in any release. Plugins use `@vestra-player/sdk` (types) and
 * `@vestra-player/sdk/testing`.
 *
 * @internal
 * @packageDocumentation
 */
export { createPlugin } from "./plugin/index.js";
export { HostError } from "./wire.js";
export { API_VERSION, CAPABILITIES } from "./capabilities.js";
export { COMMAND_LIMITS, MENU_IDS } from "./commands.js";
export { MANIFEST_FIELDS, MANIFEST_LIMITS } from "./manifest.js";
export { MARKDOWN_LIMITS } from "./plugin/markdown.js";
