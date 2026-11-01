# Changelog

## 2.2.21

- The vendored SDK is 1.5.48 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The plugins the AI writes are now tested against a TestHost that refuses command declarations as the player does (menu texts within their UTF-8 byte limits, no control characters or surrounding whitespace, no path that collides with another command, the player's own included), so a menu the player would refuse fails the AI's test run instead of the player. The reference covers the new `COMMAND_LIMITS` fields. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.20

- The vendored SDK is 1.5.47 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now give player 0.6.0's default keys: Toggle Fullscreen is Escape only, the new Enter Fullscreen and Leave Fullscreen have none, and Next / Previous are the mouse's X1 / X2 side buttons. Encoding frozen list rows is cached. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.19

- The vendored SDK is 1.5.46 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.18

- The vendored SDK is 1.5.45 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.17

- The vendored SDK is 1.5.44 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.16

- The vendored SDK is 1.5.41 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.15

- The vendored SDK is 1.5.40 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.14

- The vendored SDK is 1.5.39 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the shader channel `plugin.video.shaders(id?)` (`VideoShaders`, several plugins' mpv user shaders at once, GPU rendering only), the `convolution` and `lut3d` video nodes (`ConvolutionPlane`, `.cube` files from the plugin's package or data directory), `VIDEO_PROCESSING_LIMITS`, and TestHost's `videoShaders`, `fileRoots` and `failVideoShaders`; all of it needs player 0.5.20. The reference also no longer says only macOS renders on the GPU. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.13

- The vendored SDK is 1.5.38 (`npm run vendor:sdk`), the version the player now uses. Who must change: nobody. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover the player's lazy activation: the _Idle plugins are stopped_ section (what stays while a plugin is stopped, what keeps it running, and keeping state that must outlive a stop in `plugin.storage.session`, as in Chrome Manifest V3), mirror subscriptions (`events.subscribe { event, mirror: true }`, player 0.5.18), `PluginInfo.process` (`"running"` / `"idle"`) and `TestHost.listening`. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.12

- The vendored SDK is 1.5.37 (`npm run vendor:sdk`), the version the player now uses. The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover `plugin.video.upscaling` (the GPU's AI video upscaler, NVIDIA RTX Video Super Resolution on Windows; capability `video.upscaling`, player 0.5.16), its availability reasons and `TestHost.upscaling`. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.11

- The vendored SDK is 1.5.36 (`npm run vendor:sdk`), the version the player now uses. `BUILTIN_COMMANDS` and the reference the AI reads (`llms.txt` / `llms-full.txt`) list the player's `Video ▸ GPU Rendering (Experimental)` item (`video.gpu-rendering`), so the AI sees that menu item and keeps it in mind when it picks menu places and default keys. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.10

- The vendored SDK is 1.5.35 (`npm run vendor:sdk`). The reference the AI reads (`llms.txt` / `llms-full.txt`) and the declarations its type check and tests use now cover `plugin.rendering` (the GPU or software rendering choice, capability `rendering.manage`, player 0.5.13), `SystemInfo.renderingReason`, `TestHost.rendering` and the how-to guide *Offer a GPU rendering switch*. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.9

- The vendored SDK is 1.5.34 (`npm run vendor:sdk`), the version the player now uses. The reference the AI reads (`llms.txt` / `llms-full.txt`) carries every declaration's examples and remarks, the corrected facts and the new how-to guides of SDK 1.5.30–1.5.34 (playback control, data storage, loading subtitles), and `SUBTITLE_LIMITS`; the type check and tests the AI runs use the 1.5.34 declarations and TestHost. No change to the plugin's own behaviour; `minPlayerVersion` stays 0.5.10.

## 2.2.8

- "Vestra Player AI…" has a button on the bottom bar (`button: { icon: "ai", order: 100 }`, SDK 1.5.29): at the bar's right end, left of the Plugin Store's (order 200) and the sidebar toggle; in a narrow window it hides before them. A click opens the AI window like the menu item; the tooltip is "Vestra Player AI" with its key. Needs player 0.5.10 (`minPlayerVersion`).
- `check` has the player compile the plugin before any install: it packs into a scratch folder and calls `packages.validateArchive` (manifest, the entry's JavaScript, every `.slint` file), the same checks an install runs. A refusal fails the check with the player's own diagnostics in `problems`, one entry per line the player wrote (for Slint: `file:line:column: error|warning: message`, the file inside the package; for JavaScript: `dist/main.js:line` and node's lines) and tells the AI to fix each error at that place and check again; a scratch package the check could not remove is swept when the chat's agent next starts; a pass reads "Check passed (manifest, strict type check, build, the player's Slint compile)". Before, Slint errors such as "'changed' is not a callback in SearchField" first showed at `install_plugin`, after the AI considered the plugin done. The system prompt says that typecheck and test do not compile Slint and check does; where to look for a component (the .slint file and its imports, `sdk/slint/` for `@vestra` components, Slint's documentation for std-widgets); that warning lines can be ignored; and that a JavaScript error names the built `dist/main.js`, to be fixed in the .ts source. Stopping the chat while the player validates ends the check at once ("Check cancelled"); the player's late reply is dropped. `install_plugin` still validates (last line of defence); its failure now also tells the AI to fix and run check again.
- The vendored SDK is 1.5.29 (`npm run vendor:sdk`): the reference the AI reads covers bottom-bar buttons (`CommandOptions.button`, `BarButtons` in `@vestra/bar-buttons.slint`) and TestHost's `barButtons`.

## 2.2.7

- New plugin projects list the SDK's TypeScript plugin in `tsconfig.json` (`"plugins": [{ "name": "@vestra-player/sdk/ts-plugin" }]`, SDK 1.5.26): in an editor, an API call whose capability `manifest.json` does not declare is marked, with a quick fix that adds it. The AI's own type check is unaffected: `tsc` does not load language-service plugins.
- "Export Source .zip" adds `.vscode/settings.json` (`typescript.tsdk` → `node_modules/typescript/lib`, with the prompt to use it): VS Code loads a project's plugins only with the project's own TypeScript, which `npm install` brings in as a dependency of `@vestra-player/sdk`.
- `typecheck` and `check` report undeclared capabilities in their own list (`undeclared`: file, line, capability) with a one-line summary, instead of a bare "Check failed"; the system prompt tells the AI to add the capability to `manifest.json` when the call is needed, or drop the call. This takes effect once the vendored SDK is 1.5.26 or later (`npm run vendor:sdk`); with 1.5.25 nothing changes.
