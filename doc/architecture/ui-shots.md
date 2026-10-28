# Offscreen screenshots of plugin interfaces (`ui:shots`)

Plugin authors (and agents) cannot see a GUI: code that compiles and passes its tests can still show oversized buttons, cut-off German labels or a self-drawn drop-down. This tool renders every window and settings page of a plugin to PNG, in the chosen languages, with the states the plugin's own tests produced, through the player's own Slint pipeline and without opening a window. Look at the images with the Read tool. Registered in the [UI polish round](../process/rounds/2026-10-ui-polish.md), Step 0.

## Use

In `src/` (after `npm ci`; Rust, the libmpv build environment and `CARGO_TARGET_DIR` as for `cargo test`):

```text
npm run ui:shots -- <plugin...> [--lang en,zh-Hans,de] [--out <dir>]
    [--state <text>] [--max <n>] [--theme vestra|vestra-light] [--scale <factor>] [--states <dir>]
```

- `<plugin>`: a directory under `src/plugins/` (`store`, `workshop`), or a path with a slash relative to `src/` (`examples/bookmark`). Several at once are fine.
- `--lang`: any of the player's languages (`en`, `zh-Hans`, `ja`, `ko`, `de`, `fr`, `es`); default `en,zh-Hans,de` (German has the longest strings).
- `--out`: default `<system temp>/vestra-ui-shots/<plugins>` (emptied first); a directory inside the repository is refused.
- `--state <text>`: only the states whose test name contains the text; `--max <n>`: at most n states per view.
- `--theme`: `vestra` (dark, default) or `vestra-light`. `--scale`: the scale factor (default 1: one PNG pixel per logical pixel).
- `--states <dir>`: reuse the state log of an earlier run (`<out>/states`) instead of running the tests again.

The output directory holds the PNG files and `index.md` / `index.json`: one entry per image with the plugin id, the view (`ai.slint`, `ui/settings.slint StorePage`), the variant, the language, the size, the state (`<test file> › <test name>`), the other states that gave the same picture, and problems (a value the player would refuse, an image it could not show). A first run builds the host's test program (a few minutes from scratch); after that, `store plugins workshop` in three languages (the tests, 127 states, 365 images) took about three minutes on the Windows dev machine.

For task briefs (round doc, Step 0): **before delivering, look at en and de with the screenshot tool and check against ui-spec.**

## What is shown

- **Windows** (`ui.open`): at the window's declared size, the preferred size Slint computes from the `.slint` (`preferred-width`/`-height`, or the layout's), the way winit sizes a new window. Variant `window`.
- **Settings pages** (`settings.register`): in the Options window the player builds for them (`settings_window::source`: the tree with the page's category and title, the page area 440 px wide, Close), at its preferred size 640×480 — variant `options`, what the user sees first. When the page is taller than the visible page area (439 px), also the whole page at 440 px wide on the window's background — variant `page`, which shows what is below the fold.
- **States**: each window or page the tests opened, with its property values at the moment it was closed or its test host ended (the last state of each view per test). States with the same values are rendered once and listed together; identical pictures of the same view are kept once.
- **Windows no test opened**: every `.slint` file of the plugin that exports a component inheriting `Window` and that no test opened is rendered with its default values (state "declared defaults").
- Not covered: panels (`ui.mount`; their size is the slot's), menus, dialogs and OSD messages (the player draws them, not the plugin), the title bar (native).

## How it works

1. **States from the plugin tests.** `scripts/ui-shots.mjs` runs `scripts/plugin.mjs test <plugins>` with `VESTRA_UI_STATES=<out>/states`. The preload `scripts/ui-record.mjs` (the same one that records values for `recorded_plugin_props_match_their_slint`, see [ci.md "Recorded plugin props"](../process/ci.md#recorded-plugin-props)) then logs every `ui.open` / `ui.set` / `ui.close`, `ui.mount` / `ui.setMount` and `settings.register` / `settings.set` / `settings.unregister` the plugins send to the SDK test host, in order and without removing repeats, with the test host, the view and the running test's name (a root `beforeEach` of `node:test`, only in test files). Failing tests do not stop the tool. The script replays the log per view into states.
2. **Rendering in the host.** The script runs the host's ignored test `ui::ui_shots::tests::render_plugin_screenshots` (`src/host/src/ui/ui_shots.rs`, test-only code) once per language with `VESTRA_UI_SHOTS=<job.json>`; the interface language is fixed per process (`i18n::init`). For each state it does what the player does:
   - `i18n::init(locale)`, the plugin's catalog loaded and registered (`i18n::load` / `register`, as `process/mod.rs`), so the file loader of `ui::view::compiler` rewrites the plugin's `@tr` (`slint_tr.rs`) and the player's own `ui/` strings come from its catalog;
   - fonts: `ui_fonts::install(locale)` on Slint's shared font collection (the CJK fallback chains, bundled Noto Sans CJK SC where the platform says so) before the first window, `theme::apply_fonts`, and `theme::apply` for a non-default palette;
   - compiled with `ui::definition_source` (`ui::compile`'s compile step: the palette export, `@vestra/*` library paths, fluent-dark) or `composition::build` (the Options window, the whole-page wrapper); each view is compiled once per language and instantiated per state;
   - values: text values rendered with the plugin's catalog (`i18n::text::Texts::resolve`), Markdown documents and runtime images (whole values and, from SDK 1.5.19, nested in rows, with their `maxSide`) converted as `prop_value` does (`ui_markdown::convert`, `ui_image::decode_within`), then `ui_values::own_default_models` and `ui_values::set` / `composition::set_alias_value`: a value the player would refuse is reported in the index, and the view is still drawn;
   - drawn by Slint's software renderer (`MinimalSoftwareWindow`, a platform of its own on that thread) into memory at the preferred size and saved as PNG.
3. **Index.** The script merges each language's results into `index.md` / `index.json`, and drops pictures identical to an earlier one of the same view.

The preferred size uses Slint's private `WindowInner` / `ItemTreeRc::layout_info` (as the winit backend does), like `ui::settle_layout`: recheck both on a Slint upgrade.

## Differences from the player

Compared with the coordinator's screenshots of the real player on Windows (plugin store, plugin manager, Vestra Player AI):

- **Renderer**: Slint's software renderer, not femtovg (OpenGL), which the player uses on most machines: text antialiasing and the edges of rounded rectangles differ by a pixel or so; layout, sizes, colours, fonts and fallback are the same.
- **Scale**: one pixel per logical pixel by default; the player draws at the display's scale factor (`--scale 1.5` or `2` gives the same layout at that density).
- **Size**: the declared size, which is what the player opens a plugin window at; a window the user resized is of course different.
- **No title bar** and no window shadow: they are the system's.
- **Options tree**: only the plugin's own pages; in the player it lists every plugin's pages.
- **Images**: a runtime image or Markdown image whose file no longer exists (recorded in a test's temporary data folder) is left empty and named as a problem in the index. The real player shows remote README images only after they are downloaded; the tool shows only local files.
- **Time**: no animation runs (a spinner or fade shows its first frame), and a field the view focuses at start (the store's search field) is drawn focused (cursor, accent underline) as in an active window.

Checked on 2026-10-26 against the coordinator's Windows screenshots: the Vestra Player AI window without a provider matches (the real one is 720×620 logical at scale 1.75, the same as the render; layout, banner wrapping, icon, button sizes and fonts agree); the plugin store and the Plugin Manager page match in controls, colours, fonts and layout. The store in that screenshot is about 900×480 logical, shorter than its declared 900×600 (probably resized by hand; the render shows the declared size), and the real Options tree lists every plugin's pages.
- **States are the tests'**: a state no test reaches is not shown; sample data such as plugin names and dates are the tests' fixtures.
