# SDK documentation plan

2026-10-22. Based on two research reports: [research-readme.md](research-readme.md) (READMEs of 12 repositories and 7 CONTRIBUTING files) and [research-api.md](research-api.md) (MATLAB, man-pages, MDN, Rust, Stripe, VS Code, Qt, OpenCV, Node.js, Diátaxis, TypeDoc).

## Current state

- The README is 73 lines of plain text: no logo, badges or screenshot; content for three audiences (plugin authors, advanced users, maintainers) is mixed on one page (maintenance content such as Pages and CI is in it too).
- CONTRIBUTING is really a maintainer's handbook; what external contributors need (reporting issues, PR rules, code of conduct, licensing) is missing.
- API comment coverage is sufficient (every export has one); **what is missing is depth and structure**: `types.ts` has 103 exported types with about 390 members; there are only 3 `@throws`, and 0 `@remarks`/`@see`/`@since`/`@category`; none of the 79 `@example`s has been type-checked, and many use undefined variables.
- Conceptual material has in fact been written: the Chinese `reference.md`, `tutorial.md` and `examples.md` in the player repository, about 2950 lines, but the SDK docs site cannot link to them.
- Factual drift: the error code table is missing `versionMismatch`, `host` and `ui-prepare`; a duplicate window id throws a plain Error without `code`; getting-started still says Node 22.18.
- The SDK repository has no `examples/`. The player repository has 4: bookmark, core-ui, markdown, panel-menu.

## Goals

1. A plugin author who opens the README knows within 30 seconds what this is, how to install it and how to run a first plugin.
2. Every API answers: what it is, when to use it, how to use it (an example that compiles), how it fails, and what permissions it needs.
3. All code in the docs compiles and runs, and CI keeps it from going stale.
4. The SDK docs become the single authoritative source for the plugin API; `reference.md` in the player becomes a redirect page.

## 1. README (one file for GitHub, npm and the TypeDoc home page)

60–90 lines, structure:

```
<centered logo>
Vestra Player SDK
One-line tagline
[npm version] [CI] [docs] [Node ≥26] [License]
Docs · Getting started · Examples · Changelog · 简体中文
<one screenshot: an example plugin running in the player>
Features (5 items)
Install (one line)
Quick start (≤10 lines of code + one run command)
Examples (table: name | what it shows | APIs used)
CLI (a small table of vestra-plugin subcommands)
Compatibility (SDK / player / Node matrix)
Contributing · License (one line each)
```

- Badges use shields.io; the CI badge points to `main.yml`. No download count or Discord for now.
- Logo: scale `src/assets/brand/logo.png` down to 256px, put it at `.github/assets/logo.png` in the SDK repository, and have the README use the raw absolute URL (npm only accepts absolute URLs, and supports neither `<picture>` nor `> [!NOTE]`).
- Move all maintenance content such as Pages, CI and the release process out of the README; move the long list of APIs added in 1.1 into the CHANGELOG.

## 2. CONTRIBUTING

Split into two files:
- `CONTRIBUTING.md` (50–70 lines, for external contributors): how to report issues; open an issue for discussion before changing the public API; development commands; PR rules; `slint/` is a read-only copy from the player; AI usage policy; code of conduct; Apache-2.0 licensing. Plus an issue template and a PR template.
- `docs/maintaining.md`: the existing content moves there as a whole (releases, Pages, CI, trusted publishing).

## 3. Examples (`examples/`)

All 4 examples in the player repository move to the SDK repository (deleted from the player repository; the plugin repository syncs later), and two new ones are added:

| Example | Shows | Status |
| --- | --- | --- |
| `hello` | Identical, word for word, to the README quick start: one menu command + an on-screen message | New |
| `bookmark` | Commands, a tool window, reading and writing playback state | Moved |
| `panel-menu` | Embedded panel, menus | Moved |
| `core-ui` | Core UI components, mounting, OSD | Moved |
| `markdown` | The Markdown display component, local and remote images | Moved |
| **`blur` (flagship example)** | **Real-time Gaussian blur**: menu toggle, parameter window (strength slider, step selector, live preview, reset), shortcut, settings page that remembers the parameters, on-screen message; uses the `gblur` node of `plugin.video.filter()`, keeping only the latest value during continuous dragging | New |

- Each example has its own README (with a screenshot) and tests; examples are not in the npm package; CI runs check/test/pack on each example to keep it from going stale.
- The example code is also the code source for the docs: the docs pull it in with TypeDoc's `{@includeCode examples/blur/main.ts#region}`, not by copy and paste.
- Later we add `vestra-plugin init <template>`, so the quick start can shrink to one command (not in this round).

## 4. API documentation (docs site)

### 4.1 Site structure (the four Diátaxis types written separately)

| Type | Content | Pages |
| --- | --- | --- |
| Tutorials | Write a first plugin from scratch; add a tool window; add a settings page; package and publish to the store | 4 |
| Concepts | Plugins and the process model; permissions (capabilities); commands and menus; tool windows, panels and settings pages ("what exactly is a window"); player state and events; the filter chain; storage, secrets and the data directory; packages and the store; errors and recovery | 9 |
| How-to guides | Short "how to do X" pages: sliders that do not stutter while dragging, remembering the window position, cross-platform paths, calling external programs, testing plugins... | about 20 |
| API reference | Generated by TypeDoc, grouped by `@category`: UI, playback, filters, commands, storage, network, package management, testing | — |

The material for the concept pages comes mainly from `reference.md` / `tutorial.md` / `examples.md` in the player repository (about 2950 lines).

### 4.2 Comment template for each symbol

Modeled on MATLAB (syntax / description / examples / parameters / tips / version history / see also), man-pages (ERRORS, NOTES) and Rust (`# Errors`, `# Examples`):

```
One-sentence summary (what it does).

@remarks
Background: what it is, when to use it, how it differs from similar APIs; lifecycle; platform differences.

@capability ui.window          ← custom tag: the required permission
@param  … value range, unit, default
@returns …
@throws {@link PluginError} `code: "invalid"` — under what conditions
@throws … (one per error code)
@example a complete snippet that compiles (or {@includeCode} from an example file)
@see related APIs, concept pages
@since 1.1.0
```

The research report §3.3 has already fully rewritten `Windows` / `Windows.create` / `Window` with this template as a sample.

### 4.3 Toolchain

- `typedoc.json`: `@category` grouping; docs and comments searchable; `typedoc-github-theme`; `typedoc-plugin-mdn-links` (links Node/Web types to MDN); `projectDocuments` includes the tutorials, concepts and how-to guides.
- Examples must compile, with three levels of checks:
  1. `examples/` is a real project, and CI runs `tsc` and the tests;
  2. inline code blocks in comments and Markdown are extracted and run through `tsc`;
  3. the examples are actually run once with the SDK's test host.
- Links: TypeDoc already validates links within the site; external links are scanned weekly with lychee.
- A `doc-lint` script: checks whether public methods have a summary, `@throws`, `@example` and `@category`; checks that the error codes in the docs match the error codes in the code.

### 4.4 Workload

- About 30 module entry types rewritten with the full template (background, lifecycle, examples).
- About 180 methods get error codes and permission tags, about 90 of which need examples that compile.
- About 60 data fields get units and value ranges.
- About 108 places get `@category`, and 6 cases of factual drift are fixed.
- Toolchain: 1–2 days.

## 5. Execution order

1. **Foundation**: README (with logo, badges, screenshot), the CONTRIBUTING split, moving the examples, `hello`, `blur`, the toolchain (theme, grouping, example compile checks, doc-lint). When done, release an SDK 1.2.x docs update.
2. **Sample**: first write the "UI" category (the concept page "Tool windows, panels and settings pages" + reference comments for `Windows`/`Window`/`Panels`/`Settings` + two how-to guides). **Ask the user to review the writing style and depth**, and roll out to the rest only after it is final.
3. **Roll-out**: proceed category by category with the remaining reference comments and concept pages (playback → filters → commands → storage → network → package management → testing), one agent per category, in parallel where possible.
4. **Wrap-up**: the 4 tutorials; `reference.md`, `tutorial.md` and `examples.md` in the player repository become redirect pages; fix the factual drift (a duplicate window id must carry `code`, complete the error code table; these touch SDK code and follow the API change process).

Division of work follows the usual process: one agent per block, the SDK maintainer reviews API facts, and the coordinator reviews the writing style, runs CI and takes real-device screenshots (the screenshots for the README and the examples need the player running on a real machine).

## 6. Decisions needed from the user

1. **Documentation language**. The two research reports disagree:
   - Report one: README mainly in English, with a full Chinese version alongside, because npm, the API docs and the CLI are all in English;
   - Report two: the docs site and comments mainly in Chinese (the product is Chinese-first, and the conceptual material is in Chinese now), with an English introduction kept in the README.
   - The coordinator recommends: **README in English + Chinese; docs site (tutorials, concepts, guides) in Chinese; API comments in English**. Comments show up in every plugin author's editor, and English reaches the most people; the concept pages have Chinese source material and the highest translation cost, so Chinese first. An English docs site can be considered later.
2. **Sample category**: is it OK to write "UI" first?
3. **Flagship example**: is the Gaussian blur (menu + parameter window + live effect) enough, or do we want another one?

## 7. User rulings (2026-10-22)

1. **Everything in English**: the README (no separate Chinese version), the docs site (tutorials, concepts, how-to guides), API comments, example walkthroughs.
2. The coordinator decides the sample based on the research: write the "UI" category first.
3. Add a second flagship example, with a use case different from blur, of some complexity, covering video and audio streams: decided as **`signal-generator`**: the plugin generates its own picture and sound (`player.openStream`): test patterns, an oscilloscope that shows its own waveform, a timecode overlay; for sound, selectable waveform, frequency and level; a control window adjusts picture and sound in real time; `plugin.audio.filter` demonstrates audio processing.
4. Examples are explained step by step, line by line where needed (the research agrees: OpenCV and MATLAB examples are all explained step by step, with the code pulled in from real example files).

Execution: the foundation (maintainer: README, CONTRIBUTING, toolchain, moving the 4 examples + hello) and the two flagship examples (one agent each) in parallel; then the "UI" sample; then roll out by category.

## 8. API problems found by the example authors (pending for the maintainer)

From blur (examples/blur fdf9a93):
1. **Menu paths are matched exactly by title, and the player's built-in menus have Chinese titles**: an English plugin that writes `["Video", "Blur"]` gets an extra top-level menu and does not attach under "视频" (Video). Built-in menus need to be located by a language-independent anchor (such as `menu: "video"` or reserved ids); the API change goes through a proposal.
2. `gblur.steps` in `VideoNode` is fixed at 3 (consistent across the SDK types, validation and the test host), so the "steps 1–3" in plan §3 does not hold; blur switches to a preset dropdown. The maintainer decides whether to open up steps.
3. The filter docs leave out the `events` capability: without `events` the SDK does not see source changes, and values set before the media is ready stay `waiting` forever. The descriptions of `VideoFilter` and `Scope.video` need to be completed.
4. `Theme` has no error or danger color, so failure states can only use `accent`.
5. `vestra-plugin check` inherits the root tsconfig (`rootDir: src`) in the SDK repository, so all examples report TS6059; handed to the foundation agent to fix in one place.

From signal-generator (examples/signal-generator 27c37ac):
6. **The audio of a generated stream does not go through the player's audio filters**: the host refuses to set a processing chain on a stream source, and the SDK retries every 250 ms and keeps reporting `waiting`; this is undocumented, and TestHost behaves differently from the player (it applies the processing chain to streams). Either route stream audio through the processing chain, or document the limit and give a final state instead of polling.
7. **`openStream` actually also needs `playback.control`** (selecting the source goes through `source.submit`); getting-started, the type comments and reference §2/§5.2 only mention `producer.av`.
8. Undocumented limits: a single frame is at most 4 MiB (1920×1080 RGBA is rejected); sample values must be finite and within ±1, otherwise the stream ends; each frame is copied twice when sent.
9. `Stream.onStop` does not fire for the plugin's own `stop()` (the behavior is correct, but undocumented).
10. While a long-running `run` waits for `openStream`, window-bound properties do not refresh, and `plugin.update()` is needed to show "Starting…"; worth writing up as a how-to guide.
11. npm 11 does not run `prepare` for `file:../..` dependencies, so `dist/` has to be built at the root before installing the examples (already added to the check script).

## 9. Progress (2026-10-22)

- Foundation, 7 examples, two flagship examples: SDK PR #4 merged; SDK 1.3.0 released (npm has provenance, trusted publishing works).
- API problems found by the examples (§8): handled by the maintainer (anchors such as menu `@video`, `Theme.danger`, clearing with an empty chain, a final state for filters on generated streams, the `conflict` code, docs completed); player 0.3.0 merged (after main d31e1f7).
- **Video filters never took effect with GPU rendering** (hardware-decoded frames are in video memory, and mpv silently drops software filters): fixed by switching to copy-back hardware decoding when a filter exists; confirmed on a real machine that blur takes effect and turning it off gives no error.
- Real-device screenshots (packaged 0.3.0, GPU rendering): SDK PR #5. The core-ui screenshot does not show what it demonstrates; to be replaced.
- Next step: the "UI" category sample (the concept page "Tool windows, panels and settings pages" + reference comments for `Windows`/`Window`/`Panels`/`Settings` + two how-to guides), for user review.

## 10. UI sample (SDK PR #7, 2026-10-22, pending user review)

The concept page `docs/concepts/ui.md`, two how-to guides, and 24 UI symbols rewritten with the template; doc-lint is enforced for the UI category and passes (284/284), and all 38 `@example`s compile.

Code problems found by the sample (for the maintainer, next round):
1. Several failures throw a plain Error without `code` (calling `on()` after the window is open, a repeated `panels.mount`, a repeated `osd.panel()`, a panel/settings file whose exports do not match); when a `.slint` cannot be read, Node's ENOENT is thrown.
2. Similar failures use different error codes: a duplicate settings page id is `invalid`, a duplicate window id is `conflict`; an occupied bottom bar/sidebar is `ui-prepare`, the background layer is `conflict`; a missing `.slint` is `host` instead of `missing`.
3. Writing a property that Slint has not declared: each UI reports a different code; one such property on a settings page makes the whole options window impossible to open for all plugins.
4. Some errors never reach the plugin: the SDK ignores the host's `ui.error` and `ui-value` errors; bad images in the first batch of values for panels and settings pages only go to the log.
5. There is no API for tool window position and size, and the player does not remember them either.

Template changes suggested by the sample (to adopt before rolling out to the other categories): write `@throws` as "`code` (throws/rejects) condition"; `@remarks` is not required for methods, but required for modules and handle types; at the start of each category, mark it full or data in `doc-standard.mjs`; facts follow the player code; add a Slint compile check to CI; the category of `MainWindow` and `Drops` is undecided.

## 11. Examples made atomic (2026-10-24, user feedback)

User: the examples are too hard to read (blur built a small architecture, classes, JSON persistence). Standard: each example covers only one thing, with a flat structure (one `main.ts`, plain functions); extra features go in "Try next" at the end of the walkthrough; when only docs change or the change is very small, the SDK bumps only the patch version.
- 8 examples rewritten this way: hello, blur (one slider), signal-generator (color bars + sine), bookmark, panel-menu, menus, theme-widgets (split from core-ui), markdown. Merge branch `examples/atomic` (SDK repository), originally PRs #11 and #12.
- Found on a real machine: when a plugin adds a command with an anchor such as `@video` without `order`, the whole built-in menu is pulled to the top of the right-click menu → host fix in progress (`fix/menu-anchor-order`).
- Screenshots: the user asked to retake them all after the new theme (A + light) is merged.
