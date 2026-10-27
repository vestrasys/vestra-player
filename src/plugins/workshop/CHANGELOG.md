# Changelog

## 2.2.7

- New plugin projects list the SDK's TypeScript plugin in `tsconfig.json` (`"plugins": [{ "name": "@vestra-player/sdk/ts-plugin" }]`, SDK 1.5.26): in an editor, an API call whose capability `manifest.json` does not declare is marked, with a quick fix that adds it. The AI's own type check is unaffected: `tsc` does not load language-service plugins.
- "Export Source .zip" adds `.vscode/settings.json` (`typescript.tsdk` → `node_modules/typescript/lib`, with the prompt to use it): VS Code loads a project's plugins only with the project's own TypeScript, which `npm install` brings in as a dependency of `@vestra-player/sdk`.
- `typecheck` and `check` report undeclared capabilities in their own list (`undeclared`: file, line, capability) with a one-line summary, instead of a bare "Check failed"; the system prompt tells the AI to add the capability to `manifest.json` when the call is needed, or drop the call. This takes effect once the vendored SDK is 1.5.26 or later (`npm run vendor:sdk`); with 1.5.25 nothing changes.
