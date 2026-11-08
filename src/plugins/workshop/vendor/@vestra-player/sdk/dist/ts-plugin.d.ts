/**
 * The TypeScript language-service plugin `@vestra-player/sdk/ts-plugin`: the
 * capability checker of `vestra-plugin check` in the editor. A use of an API
 * member whose capability `manifest.json` does not declare is an error at the
 * member, with the quick fix "Add '<capability>' to manifest.json". A call
 * whose capability depends on a name the checker cannot read
 * (`plugin.wire.request(method)` with a computed method) is a warning
 * (`CAPABILITY_UNCHECKED`).
 *
 * A project enables it in its tsconfig.json:
 *
 *   { "compilerOptions": { "plugins": [{ "name": "@vestra-player/sdk/ts-plugin" }] } }
 *
 * The editor must run the project's own TypeScript (VS Code: "TypeScript:
 * Select TypeScript Version" → "Use Workspace Version"); `tsc` ignores
 * language-service plugins, which is why `vestra-plugin check` runs the same
 * checker. The manifest is the nearest `manifest.json` above the file; files
 * in its `tests/`, `node_modules/`, `dist/` and `vendor/` are not checked.
 *
 * @internal
 */
import type TS from "typescript";
/** @internal The plugin factory tsserver calls (`ts-plugin/index.js`). */
export default function init(modules: {
    typescript: typeof TS;
}): TS.server.PluginModule;
