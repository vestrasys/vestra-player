# Round 3b: UI composition (timeline mark layer, runtime images, backdrop layer, drag and drop, theme and appearance, settings pages, URL input widget) + plugin integration

Branch: `rebuild/features-r3b` (created by the coordinator from the commit after round 3a was merged into `main`). Basis: the 3b schedule in "Step 3: Rulings" of the [negotiation record](2026-10-features-negotiation.md) (below: "the rulings"), 3.2 item 5 (`ImageRef` release), 3.2 item 8, 3.3 "URL input dialog", the [feature requirements](../../product/feature-requirements.md) (below: "the requirements document"), [community workflow](../../../agentic-workflow.md) §3, §4, §5, and the items that [the round 3a task sheet](2026-10-features-r3a.md) (below: "r3a") §1 and §4.1 moved to this round.

## 1. Scope

Only the items the rulings mark as "3b"; do not start items from 4a onward early, and do not add interfaces outside the rulings. When a signature cannot be done, the maintainer records the reason and the adjusted signature in §6.2 of this document. Where the requirements document is broader than the rulings (Appendix A draft), the rulings always win; differences are in §4.3.

| Category | Item | New capability | Requirement |
| --- | --- | --- | --- |
| Timeline | R-M3 `plugin.timeline.marks` / `onChange` / `layer()` | `timeline.marks` | marks B22 (moved from r3a); playback drawing |
| Runtime images | R-U1 `plugin.ui.image(path): ImageRef`, `PropValue = Json \| ImageRef`; release rules of ruling 3.2 item 5 | — | playback I07 (waiting for this round since round 2); visualizer J11, H12 |
| Backdrop layer | R-U3 `panels.mount("backdrop", file)`, `panels.backdrop.state` / `onChange` | existing `ui.mount` | visualizer J11, H12 |
| Drag and drop | R-U2 `plugin.drops.provide`, `Panel.onDrop` (no `onDragOver`) | `ui.drop` | playlist A15 |
| Theme and appearance | R-H6 host can set Theme color tokens and push them down; R-S2 `plugin.appearance` | `ui.theme` (`set`) | preferences H10 |
| Settings pages | R-S1 `plugin.settings.page` / `show`; R-H7 options container window | `ui.settings` | preferences J04; page form of hotkeys J01, J02, plugins J07, visualizer H12 (moved to this round by r3a) |
| Host UI library | `@vestra/widgets.slint` adds `UrlInputRow` (ruling 3.3, no protocol) | — | playback A02, playlist "Add link…" |

Ruling 3.5's count for this round: 7 interfaces / modules (`ui.image`, `drops.provide`, `Panel.onDrop`, `panels.mount("backdrop")`, `settings.*`, `appearance`, `timeline.*`), 2 host behaviors (H6, H7), 4 new capability names (`ui.drop`, `timeline.marks`, `ui.settings`, `ui.theme`), plus 1 widgets component. The scope of this task sheet maps to it one to one, with nothing added or removed.

Not in this round: picture in picture (R-W5, R-H8, `window.state.pictureInPicture`), display layer (R-V9), frame doubling (R-V8), C-SIZE (R-H2) — 4a; C-GPU (R-H3), render options (R-V10), `transfer` (R-P8) — 4b. Rejected and not raised again: `onDragOver` (R-U2 rewritten), backdrop `mode` (R-U3 rewritten), the `themes` list and `corners` (R-S2 rewritten), panel `PopupWindow` overflow (R-U4), `commands.popup` (R-U5).

## 2. Two tracks and order

| Phase | Done by | Content | Done when |
| --- | --- | --- | --- |
| P0 | Coordinator | Create the branch; record the baseline (§6.1); confirm the r3a author deliveries are merged (the hotkeys and plugins plugins exist; C adds pages on top of them in this round) | Baseline written in §6.1 |
| P1 | Maintainer | All core items in §3 (all compatible extensions; H6 changing `Theme` to `in property` is compatible for readers, see §3.5) | All static checks pass; reference docs and TestHost in sync; coordinator review passed |
| P2 | Plugin authors | §4 assignments | `check/build/test` pass for each plugin |
| P3 | Coordinator | §5 acceptance, commit and push, merge into `main` | — |

P1 is delivered in two batches; authors start according to dependencies (§4.2):

| Batch | Content | Unlocks | Host Rust estimate |
| --- | --- | --- | --- |
| Batch 1: video area and data layer | §3.1 timeline mark layer, §3.2 runtime images, §3.3 backdrop layer, §3.7 `UrlInputRow` | marks B22, playback I07 and mark drawing, A02 widget migration; visualizer J11, H12; playlist "Add link…" | about 1 100 |
| Batch 2: input and UI composition | §3.4 drag and drop, §3.5 Theme push-down and appearance, §3.6 settings pages and options window | playlist A15; preferences J04, H10; settings pages of hotkeys, plugins, visualizer | about 1 900 |

Order inside batch 2: §3.5 before §3.6 (the options window is one more view that must receive the Theme, reusing the push-down path of §3.5); §3.4 is independent of both and can be interleaved.

## 3. Maintainer list (P1)

Each item must complete all of: host implementation (`src/host/`), SDK types and implementation (`src/sdk/`), TestHost simulation (`src/sdk/plugin/testing.ts`), reference docs (`doc/plugins/reference.md`), unit tests; new capability names are added to manifest validation (host `plugins.rs` and the build tool `scripts/plugin.mjs`). First-party Rust must not use `unsafe`. Signatures follow rulings 3.1 / 3.3; only implementation notes are written below.

### 3.0 Size budget

Host Rust (`src/host/src/*.rs`, including tests) grows by about 3 000 lines net. The baseline is 28 577 lines at the end of r3a (the P0 re-measurement wins).

| Item | Host change points | Host Rust estimate | SDK / runtime estimate |
| --- | --- | --- | --- |
| §3.1 timeline mark layer | New module `timeline.rs` (layers per owner, validation, merging, clearing on source change); cleanup on disable in `plugins.rs`; event `timeline.changed` | about 260 | about 120 |
| §3.2 runtime images | `ui_values.rs` (`PropValue` conversion, both write paths for panels and windows); new module `ui_image.rs` (path validation, PNG/JPEG header and size, decode cache, invalidation by scope, LRU) | about 500 | about 120 |
| §3.3 backdrop layer | `composition.rs::shell_source` adds a `backdrop` slot; video area layering in `host.slint`; visibility rules and `panels.backdrop` state event | about 300 | about 80 |
| §3.4 drag and drop | Collecting winit window events (`HoveredFile` / `DroppedFile`) in `main.rs`; drop point and modifier keys (`NSEvent` safe class methods); hit test against panel or video area; provider table by extension | about 430 | about 100 |
| §3.5 Theme and appearance | `theme.slint` color tokens become `in property`; two palette tables (Rust constants); push-down per instance to the main window shell, floating sidebar, tool windows, menu views, options window; `appearance` in `ui_state.rs`; `appearance.*` methods and event | about 600 | about 100 |
| §3.6 settings pages and options window | New module `settings_window.rs` (page registry, category tree, page composition source generation, switching, `show`, removal on disable, `settings.lastPage`); reuses compilation and callback forwarding from `composition.rs` | about 850 | about 180 |
| §3.7 `UrlInputRow` | Only changes `src/host/ui/vestra/widgets.slint` (about 60 lines of Slint, not counted as Rust) | 0 | — |
| Total | | **about 2 940** | **about 700** |

When over budget (workflow §4.2, ruling 3.5 "split when over"):

- If after batch 1 the actual net growth already exceeds 1 400 lines, or the maintainer's re-estimate of batch 2 puts the whole round above 3 200 lines, **split §3.6 (R-S1 + R-H7) out into round 3c**; this round keeps §3.4 and §3.5. After the split: preferences J04 ships in the requirements document's "interim" form (option C: "Options…" opens the plugin's own tool window containing only the appearance page); the settings pages of hotkeys, plugins and visualizer are postponed. §3.5 is not split: H10 depends only on it, and it is a prerequisite of §3.6.
- The split decision, measured line counts and reasons go in §6.2, and the §4.1 assignment table is updated accordingly. Do not cut tests to reduce line counts.

### 3.1 Timeline mark layer (R-M3, capability `timeline.marks`)

```ts
type MarkKind = "chapter" | "bookmark" | "marker";
interface TimelineMark { time: number; kind: MarkKind; title: string | null; plugin: string /* publisher manifest id */ }
plugin.timeline.marks: readonly TimelineMark[];                       // synchronous read-only; all layers merged, ascending by time; needs playback.read
plugin.timeline.onChange(listener: (marks: readonly TimelineMark[]) => void): Disposable; // needs events
plugin.timeline.layer(): TimelineLayer;                               // one layer per plugin, second call throws; needs timeline.marks
interface TimelineLayer extends Disposable {
  set(marks: { time: number; kind: MarkKind; title?: string }[], opts?: { ifCurrent?: PlayerState }): void; // last write wins; [] clears
}
```

- Limits (requirements document B22): ≤ 1000 marks per layer, `title` ≤ 100 characters; `time` in `[0, duration]` (only ≥ 0 required when the duration is unknown); violations are `invalid`. `set` returns `void`, failures go to `onError` (same as `osd.panel().set`); when `ifCurrent` does not match, the write is dropped without an error.
- On a source change (new `sourceId`) the core clears **all** layers; when a plugin is disabled or calls `dispose`, its layer is cleared. The core does not interpret `kind`; it is only for readers to choose colors.
- Changes are pushed with the new event `timeline.changed` (full merged list, pushed only on change, at most once every 100 ms). A reader without `events` reads `marks` only once at startup.
- TestHost: `host.timeline` (layers recorded per plugin id, merged result); `host.setTimelineLayer(pluginId, marks)` simulates another plugin publishing; cleared when a new source is opened.

### 3.2 Runtime images (R-U1, ruling 3.2 item 5)

```ts
plugin.ui.image(path: string): ImageRef;   // returns an opaque reference synchronously; no capability needed
type PropValue = Json | ImageRef;          // property value of View.set / bind; only as the top-level value of a property
```

- Path: absolute, under this plugin's `dataDirectory` **or this plugin's package directory**; rejected if it escapes after normalization or escapes through a symbolic link.
- Validation happens when the host **first uses** the reference: PNG/JPEG (by file header, not extension), file ≤ 16 MB, ≤ 4096×4096; failure is a property write error passed to `onError` (`invalid` or `missing`).
- Cache: decode results are cached by (plugin, normalized path, file size, modification time); decode memory limit is 64 MB per plugin, and LRU evicts only entries **not currently referenced by any property**; an evicted reference stays valid and is decoded again on next use. The host does not watch files: when the content changes, the plugin calls `ui.image` again to get a new reference.
- Lifetime: an `ImageRef` belongs to the scope that created it; after the scope is released, writing the reference fails with `invalid`; an image already shown in a property stays until it is overwritten or the view closes.
- The wire form is decided by the maintainer (the reference docs state it is an internal detail). The host re-validates every use against "this plugin + path rules", so a plugin that bypasses the SDK and forges a wire value gains no extra permissions.
- Change points: `ui_values.rs` stops rejecting images, `Value::Image` is only produced from `ImageRef`; both write paths, panels (`composition.rs::set_json_global`) and tool windows, must support it; `slint::Image` is built from the decoded RGBA.
- TestHost: `plugin.ui.image` returns comparable references (`host.images` records path and owning scope); view property assertions can read the path behind a reference; writing after the scope is released gives `invalid`.

### 3.3 Backdrop layer (R-U3, existing capability `ui.mount`)

```ts
plugin.panels.mount("backdrop", file): Panel;            // only one backdrop at a time, the second gets conflict
plugin.panels.backdrop: {
  readonly state: { effective: boolean } | null;         // null: no backdrop; effective: whether it is shown now
  onChange(listener: (state: { effective: boolean } | null) => void): Disposable;
};
```

- The core is only responsible for "show the backdrop when there is no video frame": `effective = true` when the source phase is `empty`, or `ready` with no video output (`stats().video === null`); `loading` and `error` keep the previous state, no flicker; `false` when there is a video frame. No `mode`: the plugin uses `player.state.phase` and `stats().video` to decide what to draw itself (ruling R-U3).
- After a source is closed, the video area no longer keeps the last frame (observation left over from round 1): on `empty` the host clears the video image; without a backdrop it is the solid color `Theme.bg-video`.
- Layering: drawn at the video image position, filling the video area; **below** the host input layer (TouchArea), and **below** OSD messages and info blocks; receives no pointer or keyboard input (double-click fullscreen, context menu, wheel and drag and drop are still handled by the host video area).
- Reuses panel mount composition (`composition.rs` adds `Slot::Backdrop`, rebuilds the shell on first mount, same as the bottom bar and sidebar); component and global inference rules are the same as `panels.mount`; unmounted when the plugin is disabled.
- Known behavior (ruling 3.2 item 8): for audio files with cover art, mpv treats the cover as a video track, so `effective = false` and what is shown is the cover rendered by mpv. The reference docs state this; it is not a defect.
- TestHost: `host.backdrop` (mounter, `effective`); either `host.setBackdropEffective(bool)` or derivation from `host.source.phase` / `host.stats.video`, pick one and write it in the reference docs.

### 3.4 Drag and drop (R-U2, capability `ui.drop`)

```ts
interface DropModifiers { alt: boolean; shift: boolean; cmd: boolean }
plugin.drops.provide(handler: { extensions: string[]; drop(e: { paths: string[]; modifiers: DropModifiers }): unknown }): Disposable;
Panel.onDrop(listener: (e: { paths: string[]; at: { x: number; y: number }; modifiers: DropModifiers }) => unknown): Disposable;
```

- `extensions` are lowercase without a dot; on release over the video area the core groups `paths` by extension (keeping Finder order) and hands each group to the matching provider in one call. When several providers claim the same extension, **the first registered wins** and a log entry is written (same rule as binding conflicts); for extensions without a provider the core does nothing.
- Folders: the rulings do not say. The maintainer sets a rule that adds no fields and writes it in the reference docs (suggestion: directories match the reserved name `"folder"`, consistent with requirements document A15), recorded in §6.2.
- Release over a panel (attached bottom bar, sidebar, and the floating sidebar window): the whole group goes to that panel's `onDrop`; if the panel has no `onDrop` listener, dispatch follows the video area rule. The coordinate system of `at` is stated in the reference docs (the author response was designed for **panel coordinates**; playlist converts to rows using the scroll position it reports back itself). Release over a tool window: ignored.
- No position during the drag (no `onDragOver`); playback is not paused or changed during drag and drop.
- Implementation notes: winit 0.30.13 only has `HoveredFile(PathBuf)` / `DroppedFile(PathBuf)` / `HoveredFileCancelled`, one event per path, with no position or modifier keys. The host collects consecutive `DroppedFile` events in the same event loop iteration into one group; on release it takes the position and modifiers from `NSEvent::mouseLocation` and `NSEvent::modifierFlags` (`objc2-app-kit` safe class methods), converts to window coordinates and hit-tests the panel or video area. If grouping or position proves unreliable in testing, record it in §6.2 and give a fallback (for example, hit-test only by the window of the last `HoveredFile`).
- TestHost: `host.drop(paths, { target: "video" | "panel", at?, modifiers? })` dispatches by the same rules; `host.dropProviders` lists registrations.

### 3.5 Theme push-down and appearance (R-H6, R-S2, capability `ui.theme`)

(At the time `ThemeId` had two values: `vestra` and a second palette; the second palette was removed in 2026-10, and below it is called `classic`.)

```ts
plugin.appearance: {
  readonly theme: "vestra" | "classic";              // synchronous read-only, no capability needed
  onChange(listener: (theme: "vestra" | "classic") => void): Disposable;
  set(theme: "vestra" | "classic"): Promise<void>;   // needs ui.theme; other values invalid
};
```

- `theme.slint`: color tokens and `accent-fill` change from `out property` to `in property` (default values are the current Vestra values, so the look is unchanged when nothing is pushed); `radius-*`, font sizes and control sizes stay constants. Plugin Slint only reads `Theme`, existing code needs no change; plugins cannot write `Theme` (Slint does not allow assigning a global's `in` property inside `.slint`, and the SDK's `set` / `bind` only reach the plugin's own global) — add a compile test asserting this.
- Palette tables live in the host (Rust constants, each set lists all color tokens): `vestra` = current values; `classic` = the second set of values in §2.1 of the UI spec at the time (yellow accent color etc., removed together with that palette), with a solid color for `accent-fill`.
- Push-down: each view instance (main window shell including bottom bar and sidebar, floating sidebar, tool windows, menu views, backdrop layer, options window) calls `set_global_property("Theme", …)` when created and when the palette changes; a failure in one view does not affect other views and is retried on the next refresh; switching does not interrupt a slider being dragged or an open menu.
- The core persists the choice: `appearance.theme` in `ui-state.json` (debounced save); the palette stays when preferences is disabled. Changes are pushed with the new event `appearance.changed`, with the same push rules as `window.onChange` (followed when the plugin has `events`).
- The OSD message "Palette: …" is shown by the calling plugin itself, not by the core.
- TestHost: `host.appearance` (current value), `host.setAppearance(theme)` simulates another plugin switching.

### 3.6 Settings pages and options window (R-S1, R-H7, capability `ui.settings`)

```ts
type SettingsCategory = "general" | "playback" | "subtitles" | "video" | "audio" | "input" | "appearance" | "plugins" | "other";
plugin.settings.page(file: string, options: { id: string; category: SettingsCategory; title: string; order?: number }):
  View & Disposable & { readonly ready: Promise<void> };
plugin.settings.show(pageId?: string): Promise<void>;   // can only name this plugin's pages; omitted = the last page
```

- Page file rules and component and global inference are the same as `panels.mount`; **no** `component` / `global` options and no `visible` (ruling R-S1). `id` is unique within the plugin (`missing` / `invalid` rules go in the reference docs); `title` 1–24 characters; ≤ 8 pages per plugin. Callbacks have no "bind before show" restriction.
- The container window (H7) is created and owned by the core, titled "Options": 640×480, minimum 600×420, resizable; a category tree 160 wide on the left (nine categories in fixed order: General / Playback / Subtitles / Video / Audio / Keyboard and Mouse / Appearance / Plugins / Other; empty categories are not shown; within a category sorted by `order`, then by plugin id); a page area of fixed width 440 on the right, scrolling vertically when too tall; only "Close" at the bottom. Geometry and colors follow "UI" in requirements document J04.
- Behavior: calling `show()` again brings the window to the front and switches page; `show(another plugin's page)` is `invalid`; `settings.lastPage` is stored in `ui-state.json`, and the first page is shown if that page no longer exists; when the plugin owning a page is disabled the page is removed (if it is showing, switch to the first page of the same category; if the category becomes empty, switch to the first page of "General"; if everything is empty, show "Nothing to set"), and it comes back with registration when re-enabled; ⌘W closes; shortcuts do not fire inside the window (same as tool windows); opening does not affect playback.
- Composition: reuses compilation and callback forwarding from `composition.rs`; each page has one owner global (`global_alias`), and property writes and callbacks are connected only to the plugin that registered it. When the page set changes the window is rebuilt, and page state is restored by each plugin's `bind`.
- TestHost: `host.settings` (registered pages, record of `show` calls, current page); pages are `View`s whose properties can be asserted and whose callbacks can be triggered.

### 3.7 URL input widget `UrlInputRow` (ruling 3.3, no protocol)

- Add `export component UrlInputRow` in `src/host/ui/vestra/widgets.slint`: input field + "OK" and "Cancel", following UI-DLG-02 of the [UI spec](../../product/ui-spec.md) (no history list). Suggested interface: `in-out property <string> text`, `in property <string> placeholder`, `in property <string> confirm-text` (default "Open"), `in property <bool> enabled`, `callback accepted(string)` (Return or clicking OK), `callback cancelled()` (clicking Cancel or Esc).
- Uses only `Theme` tokens and contains no business logic (does not validate URLs; plugins validate).
- Add it to the widget list in reference docs §6.4; give a usage example of ≤ 15 lines.

### 3.8 Documentation

- Reference docs: all interfaces above; add `ui.drop`, `timeline.marks`, `ui.settings`, `ui.theme` to the §2 capability table, and `"backdrop"` to the `ui.mount` row; §6.1 `PropValue` and `ImageRef` (release rules); §6.3 backdrop layer and `Panel.onDrop`; §6.4 `Theme` now pushed down by the host (read-only for plugins), `UrlInputRow`; new subsections: timeline mark layer, drag and drop, appearance, settings pages; §14 new TestHost items; §17 change notes for this round (`Theme` changing from constants to host push-down is a default behavior change; state that plugins need no changes).
- Examples: one short example each (≤ 30 lines) for showing an image with `ui.image`, `timeline.layer().set`, `settings.page` + `show`, and `drops.provide`.

## 4. Plugin integration assignments (P2)

Author rules are the same as round 2 ([2026-10-features-r2.md](2026-10-features-r2.md) §4) and r3a §4: write only in your own plugin directory; implement according to the matching section of the requirements document; when a core capability is missing, write a request per workflow §4.1 and keep working on the parts that do not depend on it; home-grown implementations of what the core now provides in this round must be deleted and replaced by the core capability, and the report lists the deleted files and line counts. Downgraded items are implemented per the rulings, and the matching section of the requirements document is updated at the same time (§4.3, reviewed by the coordinator when merging).

### 4.1 Assignment table

| Author | Plugin | New features (id → dependency) | Migrate to core capability | Downgrades and docs | Effort |
| --- | --- | --- | --- | --- | --- |
| A | playback | I07 seek bar hover thumbnail (`ui.image` + `tools.run("ffmpeg")`, cached in `dataDirectory/thumbs/`, tooltip drawn over the video area with negative `y` per R-U4); B22 drawing side: draw `timeline.marks` on the progress line (chapters `Theme.text-dim`, bookmarks `Theme.accent`) and the `state.loop` range (`Theme.playing`); hovering within ±3px of a mark shows the tooltip "Title  01:23", merged with I06 | A02 "Open link…" window switches to `UrlInputRow` (the hand-drawn input row in `ui/open-url.slint` is deleted) | None | Medium-high (batch 1) |
| B | marks, playlist | marks: B22 publishing side (`timeline.layer()`; "Show marks on seek bar" toggle, `showOnSeekbar` in `marks.json`; chapters ∪ bookmarks, with `ifCurrent`, truncated above 1000). playlist: A15 drag and drop (`drops.provide` registers media extensions, list files, folders; video area adds and plays the first item, Option only adds; `Panel.onDrop` inserts before the row at the drop point) | playlist "Add link…" (`ui/link.slint`) switches to `UrlInputRow`; the 2 hard-coded colors in `ui/sidebar.slint` switch to `Theme` tokens (H10 acceptance requires no hard-coded colors in plugin Slint) | A15: rewrite the requirements document per §4.3 | Medium (marks batch 1; playlist drag and drop batch 2) |
| C | hotkeys, plugins | hotkeys: settings page form of J01, J02 (`settings.page`, category `input`, ids `hotkeys.keyboard` / `hotkeys.mouse`; "Keyboard shortcuts…" and "Mouse…" become `settings.show(…)`; J01 relaid out for 440 width: command 220, shortcut 140, status 60). plugins: settings page form of J07 (category `plugins`, "Manage plugins…" becomes `settings.show`) | The existing tool windows of both plugins: the page and window content are the same; the author decides whether to keep the window or only the page, and writes it in the README | If §3.6 is split into 3c (§3.0), this whole row is postponed | Small-medium (batch 2) |
| D | visualizer (new) | J11 album cover for audio-only files (`panels.mount("backdrop")` + `ui.image`; cover extracted with `tools.run("ffmpeg"/"ffprobe")`, or `cover.jpg` etc. in the same folder; tags via ffprobe; cache in `dataDirectory/cache/covers/`); H12 idle logo (built-in logo with `@image-url`; custom logo `dialogs.openFile` → copy to `dataDirectory/logo.<ext>` → `ui.image`); "Logo and album cover" settings page (`settings.page`, category `appearance`) | — | No `mode`: decide with `phase` and `stats().video` (§4.3); README records the known behavior of mpv treating the cover as a video track | High (batch 1; settings page batch 2) |
| E | preferences (new) | J04 preferences: "Settings ▶ Options…" (F5, order 830) → `settings.show()`; this plugin's "Appearance" page (`settings.page`, category `appearance`). H10 palette: "Settings ▶ Appearance ▶ Vestra / Classic" (radio, 840, 841) + palette radio buttons and a 120×40 preview block on the appearance page (drawn in Slint by the plugin) + "Restore defaults"; `appearance.set` / `onChange`; OSD "Palette: …" on switch | — | Drop the corner radius option and the `themes` list (§4.3); if §3.6 is split into 3c, J04 ships in the requirements document's "interim" form (its own tool window, containing only the appearance page) | Medium (batch 2) |

Plugin items not in this round: window H07 (4a); video E11, E13, E16–E18 (4a / 4b). Turning subtitles "Subtitle settings…" into a settings page is in the requirements document's "could do later" and is not required this round. The URL input dialogs of youtube and bilibili (same spec, UI-DLG-02) may switch to `UrlInputRow` along the way (not required; say so in the report). Other plugins have no items this round; if H10 acceptance finds hard-coded colors in another plugin's Slint, it is returned to that plugin's author as a defect.

### 4.2 Start conditions

- After batch 1 (§3.1–§3.3, §3.7) passes review: A, B (first marks B22 and playlist's `UrlInputRow` and color fix) and D start.
- After batch 2 (§3.4–§3.6) passes review: B continues with playlist A15; as slots free up, start E (preferences; J04 acceptance depends on it) and then C (hotkeys and plugins pages); D adds the settings page.
- Precondition for C: the r3a author deliveries for hotkeys and plugins are merged (confirmed in P0).
- Authors can do the parts that do not wait for dependencies first: UI, menus, settings storage, tests under TestHost (for example, E first does the menu radio items and the appearance page layout, D first does cover extraction and caching).
- No more than 3 authors run at the same time (4 including the coordinator). Suggested order: A, B, D → (after A finishes) E → (next free slot) C.

### 4.3 Rewriting the requirements document per the rulings (each author edits their own sections)

| Section | Current text in requirements document | Change to, per the rulings |
| --- | --- | --- |
| playlist A15 | `kinds`; `onDragOver` draws an insertion line; `conflict` for the same extension; with no handler the core shows the OSD "Cannot open the dropped files" | `extensions`; no insertion line, insert before the row at the drop point, append when dropped on empty space; first registered wins; with no handler the core does nothing (acceptance item becomes "list unchanged, no message") |
| visualizer J11, H12 | `backdrop.state.mode: "idle" \| "audio"` | Only `effective`; the plugin distinguishes with `phase` and `stats().video` |
| preferences J04 | `component?`, `global?` options; `SettingsPage.visible` | Removed; inference rules same as `panels.mount` |
| preferences H10 | `themes`, `ThemeInfo`, `corners`; corner radius option; `corners` in `ui-state.json` | Two fixed ids; corner radius follows the palette and cannot be changed separately; only `appearance.theme` is stored |
| playback I07 | Path only under `dataDirectory` | `dataDirectory` or this plugin's package directory (thumbnails still go in `dataDirectory`, behavior unchanged) |
| hotkeys J01 | Page 600×460 | Settings page 440 wide (command 220, shortcut 140, status 60); if the tool window form is kept it keeps the original size |

## 5. Acceptance

### 5.1 Static checks and tests

All commands of workflow §6.1; no plugin's test count decreases; new plugins (visualizer, preferences) pass `check/build/test`; the menu snapshot only gains the items added this round (`doc/process/rounds/menus-snapshot.test.ts`); grepping plugin `.slint` files (except theme files) for hard-coded colors `#[0-9a-fA-F]{6}` returns 0 results.

### 5.2 Real GUI tests (coordinator)

All required scenarios of workflow §6.2, plus this round:

| Area | Scenario | Evidence |
| --- | --- | --- |
| Timeline marks | mkv with 5 chapters + 2 bookmarks: 7 vertical lines on the seek bar, bookmarks in the accent color, hovering a bookmark shows title and time; turning off "Show marks on seek bar" removes them within 1 second, turning it back on restores them; switching to a file without chapters removes old marks immediately; after setting an A-B loop, the range and the A and B lines are visible; after disabling marks there are no marks, the A-B range is still there | Screenshot |
| Runtime images / I07 | Turn on hover thumbnails, move over a 1080p local mkv: the thumbnail appears within 200 ms and matches the frame at that time; hovering the same position again shows it immediately; a YouTube source shows only the time; with ffmpeg moved away the menu item is grayed out; moving back and forth quickly for 10 seconds shows at most 1 ffmpeg in `ps`; host memory does not keep rising during long hovering (Activity Monitor before/after) | Screenshot, `ps`, memory readings |
| Backdrop / J11 | MP3 with embedded cover shows the cover and title / artist / album; a FLAC folder without embedded cover but with `cover.jpg` shows it; without a cover, a translucent logo and the file name; double-click still goes fullscreen, right-click still opens the menu, wheel still changes volume; switching from audio to video shows the video within 1 second; with the toggle off it is a solid color; after disabling visualizer it is a solid color and playback is not interrupted | Screenshot |
| Backdrop / H12 | Starting without arguments shows the logo and "Vestra"; it disappears after opening a file and reappears after closing with F4 (no last frame left behind); with the window dragged to minimum the logo is 64px and does not overflow, 256px in fullscreen; a custom PNG replaces it immediately, it is still there after deleting the original file and restarting, "Restore default logo" goes back to the brand logo; double-click while idle toggles fullscreen | Screenshot |
| Drag and drop / A15 | Drag 1 mkv from Finder to the video area: added and played; drag a folder: expanded per A03 and the first item played; drag 3 files holding Option: 3 rows added, not played; drag 2 files between rows 3 and 4 of the list: inserted before the row at the drop point, not played; drag `.m3u8`: parsed and the first item played; drag `.srt` with no plugin registered: nothing happens, list unchanged; with the sidebar floating, dropping on the floating window behaves the same as attached; with the sidebar hidden for lack of space, dropping on the video area still works | Physical drag and drop recording, screenshot |
| Theme / H10 | Within 1 second of switching to "Classic": the bottom bar seek bar turns yellow, the selected menu item changes color, the open audio workbench window and floating sidebar change color together; switching while dragging the volume slider does not interrupt the slider; kept after restart; palette unchanged after disabling preferences; after switching back to Vestra it matches the screenshot from before this round | Screenshot |
| Settings pages / J04 | F5 opens "Options"; the category tree has at least "Appearance" (preferences, visualizer), "Keyboard and Mouse" (hotkeys), "Plugins" (plugins); clicking switches the page on the right, playback unaffected; after disabling hotkeys that category disappears within 1 second and comes back after re-enabling; switching the palette on the appearance page changes the options window itself and the main window immediately; closing and reopening shows the last page; ⌘W closes; pressing Space in the window does not trigger play/pause; "Keyboard shortcuts…" opens the matching page directly | Screenshot |
| URL input widget | playback "Open link…" and playlist "Add link…" look the same (same widget); Return = OK, Esc = Cancel; both work the same as before the change | Screenshot |

If §3.6 is split into 3c, the "Settings pages" row of this table changes to the interim form: F5 opens preferences' own window containing the appearance page.

Defect ownership is the same as workflow §6.2.

## 6. Records

### 6.1 Baseline

(Filled in P0: static checks, test counts per plugin; host Rust `src/host/src/*.rs` line count — 28 577 when this task sheet was written; SDK and runner line counts)

### 6.2 Maintainer deliveries and signature adjustments

(Filled at each P1 batch delivery: implementation list, signature adjustments and implementation trade-offs, checks, measured host Rust net growth, GUI test points for the coordinator. Must record at least: the folder matching rule for drag and drop, the coordinate system of `Panel.onDrop`, measured conclusions on drag and drop grouping and drop point, the wire form of `ImageRef`, whether the §3.0 split was triggered)

#### P1 batch 1 (2026-10-21, maintainer): timeline mark layer, runtime images, backdrop layer, `UrlInputRow`

Based on `main` `b16f4cf` (after the round 3a merge, including the key fix).

**Implementation list** (each item: host + SDK types and wrappers + TestHost + reference docs + unit tests)

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §3.1 | `plugin.timeline` (R-M3, capability `timeline.marks`) | New module `timeline.rs`: layers per owner, validation (≤ 1000, title ≤ 100 characters, `[0, duration]`, three `kind`s), stable merge by time; `timeline.marks` (`playback.read`), `timeline.set` (`timeline.marks`); compares the source generation when broadcasting `source.state` and clears all layers if it changed; `disable` clears that plugin's layer; new event `timeline.changed` (pushed only when the revision changed and ≥ 100 ms since the last push) | `sdk/plugin/timeline.ts`: read once before activation when the plugin has `playback.read`, follows events after the first read of `marks` / `onChange`; `layer()` throws on the second call, `set` uses `Latest` (last write wins), failures go to `onError`, `dispose` writes `[]`; `host.timeline`, `host.setTimelineLayer`; reference §5.6 |
| §3.2 | `plugin.ui.image`, `PropValue` (R-U1, ruling 3.2 item 5) | New module `ui_image.rs`: path (absolute, inside the package directory or data directory after normalization, including symbolic links), ≤ 16 MB, PNG/JPEG file header, the `image` crate reads the size first (≤ 4096×4096) then decodes to RGBA; cache key (plugin, normalized path, size, modification time), 64 MB per plugin, LRU evicts only entries no property is showing (keys being shown are recorded per "plugin + view + property"); `Host::prop_value` is the single entry point of all write paths: `ui.open` initial values, `ui.set`, `ui.setMount` (main window and floating sidebar), replay on shell rebuild and when the floating window opens; `ui_values::reconcile` always reinstalls images | `ImageHandle` (`ui.ts`); `ViewBase` converts to the wire form before sending, `invalid` when the scope is already released (stale images in initial values are omitted and reported); `Scope.ui`; `host.images`, `host.imagePath`; reference §6.1 |
| §3.3 | `panels.mount("backdrop")`, `panels.backdrop` (R-U3) | `composition::Slot::Backdrop`, new marker `VESTRA-MOUNT-BACKDROP` in `host.slint` after the video image and before the `mouse` TouchArea (a composition test asserts the order); new property `no-video` (video image `visible: !no-video`, backdrop `visible: no-video`); `composition::no_video` state rule; new internal field `videoOutput` in `source.state` (mpv has a selected video track, including cover art); clears the video image on `empty`; a second backdrop gets `conflict` (including queued mounts); the backdrop is not part of window policy (no preferences, no auto-hide, `setInteraction` has no effect, `showMenu` is `unavailable`) | `layout.state` and `window.state` gain `backdrop: { owner, effective }`; `LayoutService` carries it; `host.backdrop`, `host.setBackdropEffective(bool)` (of the two options, explicit setting was chosen); reference §6.3 |
| §3.7 | `UrlInputRow` | `ui/vestra/widgets.slint` (about 55 lines of Slint, placed at the end of the file: Slint requires `Button` to be declared first); used by the library compile test in `ui.rs` | Reference §6.4; example §29 |
| — | §4.3 request: confirmation dialog attached to the caller's window | See "Request ruling" below: `confirm.rs` now attaches to `NSApplication.keyWindow` (safe method, when visible and without an attached sheet), otherwise the main window | Reference §7, §17 |
| §3.8 | Documentation | — | Reference §2, §3.3, §5.6 (new), §6.1, §6.3, §6.4, §7, §14, §17 batch 1; examples §27–§29 |

**Signature adjustments and implementation trade-offs** (all signatures match the rulings)

1. **Wire form of `ImageRef`**: `{ "$vestraImage": "<path>", "ref": <sequence number> }`, produced by the SDK's `ImageHandle.toJSON()`. `ref` makes every `ui.image` call a new value: otherwise two calls for the same path have identical JSON, and the "send only on change" behavior of `bind` would swallow the "file content changed, get a new reference" step. The host only accepts objects with exactly these two keys and re-checks the path on every use, so a forged wire value gains no extra permissions. The `ImageRef` type carries `path` (read-only, for tests and logs) and a brand field `__vestraImage`.
2. **Scope release** is decided by the SDK (the host cannot see SDK scopes): `ImageHandle` remembers the scope that created it; if that scope is already released at write time it is `invalid` and no request is sent. So TestHost's `host.images` only records images that were written (path and sequence number), not scopes; "`host.images` records path and owning scope" in task sheet §3.2 is narrowed accordingly.
3. **`ifCurrent` in `timeline.layer().set` compares only the source (`sourceId`)**, not the whole `PlayerState` stamp: under the rules of other controls, a user pausing briefly while chapters are being read would silently drop the marks with no retry, which makes no sense for a mark layer (marks belong to the source, not to the pause state). On the wire it is `timeline.set { marks, sourceId? }`, returning `{ applied }` (not exposed by the SDK).
4. `timeline.marks` is read once before activation (only for plugins with `playback.read`), in parallel with `player.state` and `window.state`, so that `activate` reads the real value synchronously; reference §17 already notes that protocol count tests will see it.
5. "No video output" for the backdrop is decided by mpv's track-list (a selected video track, **including** cover art), consistent with `stats().video !== null`, satisfying ruling 3.2 item 8 (audio with cover art gives `effective = false`); generated streams (NES) count as having a picture once ready. `ended` follows the same rule as `ready` (with keep-open the last frame is still there). At startup (no source yet) it is `true`.
6. Clearing the video image after closing a source happens only when the phase becomes `empty`; when audio-only is ready the image is not cleared but hidden through `no-video`, so switching from video to audio does not leave the previous frame either.
7. Each plugin can still mount only one panel (existing host and SDK limit; a sentence was added to reference §6.3). visualizer only mounts the backdrop and is not affected; if a plugin later needs both a bottom bar and a backdrop, revisit then.
8. Image decoding runs synchronously on the UI thread (about 0.1 s for a 4096² JPEG); small images such as thumbnails (I07) are not noticeable. A cache hit does not read file contents, only one `stat`.
9. New dependency: `image = "=0.25.10"` (`default-features = false`, `png`, `jpeg`); Slint already depends on the same version, so `Cargo.lock` gains only one dependency edge and no new crate.

**Request ruling: attach `dialogs.confirm` to the calling tool window** (forwarded by the coordinator, §4.3; requesters plugins, hotkeys, later preferences)

- Request: the confirmation dialog is always attached as a sheet to the main window; when called from a tool window the main window is brought to the front and covers the tool window; they want an option `{ window?: Window }`. Plugins currently work around it by calling `show()` on their own window after the confirmation.
- Maintainer analysis: this is a defect in the existing implementation (the parent of a confirmation dialog should be the window the user is working in), fixed as a defect per §4.2 item 1, with no new parameter. The tool window that calls the confirmation is the app's active window (key window) at the moment the user clicks the button, so the host now attaches to `NSApplication.keyWindow` (when visible and without an attached sheet); with no active window it attaches to the main window, and failing that it is app-modal. This avoids the host mapping the SDK's `Window` handle to a native window (tool windows currently have no native window association; doing that would need a window snapshot before and after `ui.open`, about 40 lines, plus one more parameter to maintain).
- Cost: when a plugin shows a confirmation in the background (in a timer, while the user is in another window), it attaches to the user's current window instead of the plugin's window — still a reasonable place for the user; no scenario currently needs an explicit parent window.
- Conclusion: **accepted after rewriting** (no `window` option, the host behavior is corrected). Signature unchanged; reference §7 and §17 updated. The "call `show()` on itself after confirming" workarounds in plugins and hotkeys can be removed (authors handle it along the way this round, not required).

**Checks** (actually run in the worktree)

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 84/84 (new `tests/sdk/round3b.test.ts` with 3 cases: timeline, runtime images, backdrop); plugin tests (`plugin -- test --all`) audio 35, bilibili 15, capture 25, history 22, hotkeys 17, info 26, marks 20, nes 11, playback 45, playlist 62, plugins 10, subtitle-online 13, subtitles 52, video 22, window 17, workshop 17, youtube 14 all pass (workshop `vendor/sdk` resynced, same commit).
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 202/202 (new: mark layer merge and push 1, mark validation 1, image reference form 1, image path/format/size 1, image cache and eviction 1, backdrop state rule 1; the composition compile test gains a backdrop fixture and layering assertion, the library compile test gains `UrlInputRow`).
- Not run: GUI (not operated, as instructed).
- Host Rust: `src/host/src/*.rs` 28 771 (`main` `b16f4cf`) → 29 597 (**+826**, including tests; `timeline.rs` 241, `ui_image.rs` 345). Below the 1 400 line threshold of §3.0; batch 2 re-estimate about 1 700 lines, whole round about 2 500, **the §3.6 split is not triggered** (to be reconfirmed with measurements at batch 2 delivery). SDK and runner (`src/sdk/plugin/*.ts` + `src/runtime/`) +about 470 (including TestHost +140), `src/sdk/wire.ts` +28.

**GUI test points for the coordinator** (needs a plugin calling these interfaces: author B's marks, D's visualizer, A's playback, or a temporary probe)

1. Timeline: after a probe publishes 3 marks, `timeline.marks` contains them immediately; after opening another file it is empty (`timeline.changed` pushes an empty list); after disabling the publisher its marks disappear and other plugins' marks remain.
2. Runtime images: both PNG and JPEG in the data directory display (once in a tool window and once in the backdrop); renaming `.png` to `.jpg` still displays (judged by content); files outside the data directory, symbolic links pointing outside, a 5000×5000 image and a GIF all fail into `onError` with the property unchanged; after overwriting a file with the same name, calling `ui.image` again shows the new content; images survive switching the sidebar between floating and attached, and bottom bar rebuilds.
3. Backdrop: starting without arguments shows the backdrop in the video area; it disappears after opening a video and the video is visible; after closing with F4 the backdrop reappears with **no last frame left behind**; with no backdrop plugin it is a solid color after F4; MP3 with embedded cover: backdrop not shown (mpv draws the cover); FLAC without cover: backdrop shown; while the backdrop is shown, double-click still goes fullscreen, right-click still opens the menu, wheel still changes volume; switching directly from video to audio without cover does not flash the last frame. A second plugin mounting a backdrop gets `conflict`.
4. `UrlInputRow`: Return = OK, Shift+Return inserts a newline, Esc = Cancel; with `enabled: false` OK is grayed out and Return does nothing; colors follow `Theme`.
5. Confirmation dialog: when triggered from a tool window (plugin manager "Uninstall", keyboard settings "Restore defaults"), the sheet attaches to that tool window and the main window is not brought to the front; when triggered from a main window menu it still attaches to the main window.

#### P1 batch 2 (2026-10-21, maintainer): drag and drop, Theme push-down and appearance, settings pages and options window

Based on `rebuild/features-r3b` (batch 1 `82862f4`). **The §3.0 split was not triggered**: §3.6 is completed in this batch (measurements under "Checks").

**Implementation list**

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §3.5 | Theme push-down (R-H6) | New file `ui/palette.slint` (host-only global `VestraPalette`, colors and `accent-fill` are `in` properties, default Vestra); colors in `ui/vestra/theme.slint` become `out` properties bound to it; new module `theme.rs`: two palette tables (21 color tokens; `classic`'s `accent-fill` is a solid color), `apply` uses `set_global_property("VestraPalette", …)`; pushed per instance to the shell (`host.slint` exports the palette), floating sidebar, tool windows (`ui.rs` appends `export { VestraPalette } from <absolute path>` to the end of the source and pushes before showing), and options window; on failure `theme_pending` is set and everything is resent on the next 16 ms tick | Reference §6.4, §6.8, §17 |
| §3.5 | `plugin.appearance` (R-S2, capability `ui.theme`) | `appearance.set` (`invalid` validation); `appearance.theme` in `ui-state.json` (debounced save, a corrupt value is discarded on read); broadcasts `appearance.changed` on change; the `window.state` reply carries `appearance` | `sdk/plugin/appearance.ts`: the initial value comes with the `window.state` read before activation (no new request), follows events after the first read of `theme` / `onChange`; `host.appearance`, `host.setAppearance` |
| §3.6 | `plugin.settings` (R-S1, capability `ui.settings`) and options window (R-H7) | New module `settings_window.rs`: page table (validates id, category, title 1–24, `order`, ≤ 8 per plugin, separate compile check at registration), tree with nine categories in fixed order, window source generation (per page a `Page<n>View` and `Settings<n>Bridge` alias, instantiated with `if current == k`, a 440 wide `ScrollView` on the right), property writes reuse `prop_value` (images supported) and `reconcile`, callbacks go through `ui.callback` (view `$settings:<id>`) only to the registering plugin; `show` opens or brings to front with winit `focus_window`; `settings.lastPage`; disable / `dispose` removes the page and rebuilds on the next tick, falling back in the order "same category → General → first page → empty"; ⌘W, the close button and the title bar close all hide and discard the instance | `sdk/plugin/settings.ts` (a `ViewBase` subclass, component and global inference reuse `exported`); `host.settings`, `host.settingsPage`; reference §6.9; example §30 |
| §3.4 | `plugin.drops.provide`, `Panel.onDrop` (R-U2, capability `ui.drop`) | New module `drops.rs`: provider table (first registered wins, conflicts logged), grouping by extension, `folder`, a group ends after 30 ms with no new path; winit `DroppedFile` is turned into events in the window hooks of the shell and the floating sidebar; on release `platform_window::pointer_in` (`NSWindow::mouseLocationOutsideOfEventStream` + `contentRectForFrameRect`) and `modifier_keys` (`NSEvent::modifierFlags` class method), all objc2 safe methods; `drops::hit` decides bottom bar / sidebar by video column width and height and converts to panel coordinates; the floating sidebar is associated with its native window when opened | `sdk/plugin/drops.ts` (provider ids generated by the SDK; panel listeners are counted, the first `onDrop` sends `drops.listen(true)`, the last one revokes it); `host.drop`, `host.dropProviders`, `host.panelDrops`; reference §6.7; example §31 |
| §3.8 | Documentation | — | Reference §2, §3.3, §6.3, §6.4, §6.7–§6.9 (new), §14, §17 batch 2; examples §30, §31 |

**Signature adjustments and implementation trade-offs** (all public signatures match the rulings)

1. **"Plugins cannot write Theme" uses a different mechanism**: task sheet §3.5 assumed "Slint does not allow assigning a global's `in` property inside `.slint`", but testing shows Slint 1.16 allows it (`Theme.accent = #ff0000` compiles with no diagnostic). Instead: the writable colors live in the host-only `VestraPalette` (`ui/palette.slint`, not in the `@vestra` library directory), and `Theme`'s colors stay `out` properties bound to it. Plugins still `import { Theme }`, and assigning `Theme.x` fails to compile (asserted by the compile test in `theme.rs`). In theory a plugin could import the palette by absolute path and change colors itself; that counts as a deliberate bypass (plugins are trusted, see the start of the reference).
2. In the palette table, `selected-bg` for `classic`, which had no matching entry, is `#444444` (button pressed gray); `button-hover` / `button-press` / `button-disabled` use the values from the comments in §2.1 of the UI spec at the time. Vestra's `accent-fill` gradient is read from the first instance and cached (the public Slint API cannot construct gradients), and pushed when switching back to Vestra.
3. The initial value of `appearance.theme` comes back with the existing `window.state` read before activation, with no extra protocol request; the event `appearance.changed` is added per the task sheet and needs `events`.
4. **Folder rule for drag and drop**: directories (by `is_dir` at release) match the reserved name `"folder"` and are not expanded; files without an extension match no provider.
5. **Coordinate system of `Panel.onDrop`**: panel coordinates = logical points relative to the top-left of the panel root component (bottom bar: `y − video column height`; sidebar: `x − video column width − 3`; floating sidebar: window content coordinates).
6. **Drag and drop grouping and drop point**: for one drop of several files, winit 0.30 sends `DroppedFile` one by one in the same batch of events; the host ends a group after "30 ms with no new path"; the position is not taken from winit (there is no `CursorMoved` during the drag) but from AppKit at the end (`mouseLocationOutsideOfEventStream`, when the pointer is at the release point). **None of this has been tested in the GUI** (GUI not operated, as instructed); it is listed in the GUI points. Fallback if testing shows it is unreliable: hit-test by window (dropped on the floating sidebar window = panel, main window = video area rule). In fullscreen the bottom bar floats over the video; this batch always treats it as the video area (recorded in reference §6.7).
7. When a panel has no `onDrop` listener, dispatch follows the video area rule (ruling text); listening is per plugin (each plugin has only one panel).
8. `settings.show(pageId)`: any `pageId` that is not a page registered by this plugin is `invalid` (the ruling only mentions "another plugin's page"; plugins cannot see other plugins' ids, so a non-existent id of this plugin is treated the same). Duplicate ids, more than 8 pages, and invalid titles or categories are also `invalid`.
9. The options window is rebuilt whenever the page set changes (all pages are compiled into one instance; switching pages only changes `current`, no rebuild); a rebuild keeps window position and size, properties are replayed from the last written values, images come from the cache. Each page is compiled once separately at registration; a broken page fails its own `ready` (`ui-prepare`) and does not take down the window.
10. Closing the options window discards the instance (rebuilt on the next `show`); page state is restored from the plugin's `bind` and the properties recorded by the player.

**Checks** (actually run in the worktree)

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 87/87 (new `tests/sdk/round3b-input.test.ts` with 3 cases: drag and drop, appearance, settings pages); plugin tests (after merging author batch 1 `eda77b8`, `plugin -- test --all`) audio 35, bilibili 15, capture 25, history 22, hotkeys 17, info 26, marks 24, nes 11, playback 59, playlist 63, plugins 10, subtitle-online 13, subtitles 52, video 22, window 17, workshop 17, youtube 14 all pass (workshop `vendor/sdk` resynced, same commit). On the first full run playback's "I07 a resting pointer makes a picture once…" failed once with a timeout (a timing case under parallel load; 3 consecutive separate runs and a second full run all passed); passed to author A to watch.
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 208/208 (new in batch 2: palette tables match the palette file 1, plugins cannot assign Theme and the export compiles 1, settings tree ordering and fallback 1, options window source compiles (with pages / without pages) 1, drag and drop grouping and provider order 1, drop grouping and panel hit test 1).
- Not run: GUI.
- Host Rust: `src/host/src/*.rs` 29 597 → 30 966 (batch 2 **+1 369**, `settings_window.rs` 633, `drops.rs` 344, `theme.rs` 234); round total 28 771 → 30 966 = **+2 195** (budget about 3 000, split threshold 3 200). Also Slint: `ui/palette.slint` adds 27 lines. SDK and runner this batch +about 590 (`drops.ts` 120, `appearance.ts` 71, `settings.ts` 102, TestHost about +150, `types.ts` about +110).

**GUI test points for the coordinator**

1. Palette: within 1 second after a probe (or author E's preferences) calls `appearance.set("classic")`: the bottom bar seek bar becomes solid yellow, and the selected menu item, open tool windows (such as the audio workbench), floating sidebar and options window change color together; switching while dragging the volume slider does not interrupt the slider; still classic after restart; unchanged after disabling the caller; switching back to Vestra matches the screenshot from before the switch (gradient restored).
2. Settings pages: after two probes each register one page (different categories), F5/`show()`: "Options" is 640×480, the tree follows the nine-category order, empty categories are not shown; clicking switches pages; pressing Space in the window does not pause; ⌘W closes; reopening shows the last page (also after restart); after disabling one of the plugins its page disappears within 1 second (if it was showing, switch to the first page of the same category / General / "Nothing to set"); comes back after enabling; text fields and sliders on the page work, and callbacks reach the registering plugin; the right side scrolls when the page is too tall.
3. Drag and drop (physical drag and drop, **must be tested**): drag 1 mkv from Finder to the video area → the provider receives 1 path; drag 3 files at once → one call, order matches Finder; drag a folder → the `folder` provider receives the directory path; release while holding Option → `modifiers.alt: true` (same for Shift and ⌘); drag between rows 3 and 4 of the attached sidebar → the `at.y` received by `onDrop` matches the row position in the panel (compare against `mouse-y` in the panel); drag to the floating sidebar window → also goes through `onDrop`, coordinates are window content coordinates; drag to the bottom bar → received if the bottom bar plugin listens, otherwise dispatched by extension; drag `.srt` with no provider → nothing happens; drag to a tool window → nothing happens; playback does not pause during drag and drop. If one drop is split into several groups or the coordinates are off by one title bar height, fall back per item 6 and tell the maintainer.

### 6.3 Author deliveries

(Filled in by each author on completion)

### 6.4 Acceptance results

(Filled in P3)

### Coordinator acceptance (2026-10-21, accepted together with 4a)

**Passed real GUI testing**:
- Timeline: chapter marks, bookmark marks and the A-B range show on the seek bar; hovering shows thumbnails.
- Drag and drop: dropping on the video area appends and plays; dropping on a list row inserts before that row without interrupting current playback.
- Palette: the Vestra and classic palettes switch instantly in all windows; "Restore defaults" works.
- Options window: category tree, appearance page, keyboard shortcuts page.
- visualizer: shows the logo when idle; an MP3 with embedded cover shows the plugin-drawn cover and three tag lines, and mpv no longer shows the cover itself.

**Fixed on the spot**:
- Command limits: changed from 64 per plugin to 160, and from 512 total to 1024; TestHost enforces the same limits (8512d2d).
- The options window button row was stretched vertically (e3e4901).
- visualizer tag line height was too small, so Chinese text did not show.
- playback I07 test was flaky; changed to an injectable timer.
