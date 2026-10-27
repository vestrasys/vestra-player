# This round: plugin interaction redesign (step two, bottom-up)

Branch: `rebuild/plugin-ui` (from `rebuild/core-ui` 8c1a53e). The process follows [community workflow](../../../agentic-workflow.md) §2, §4 (bottom-up). Before starting, each author reads: this document, community workflow §1, §2, §4, §8, [UI rework document](../../product/ui-spec.md) §2, §6, §8, §9 and the sections relevant to their own plugin (§11 lists the rework items per plugin), the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html), and [core UI design](../../architecture/core-ui-design.md) §2–§12.

## 1. Goals

Each plugin reworks its interaction and appearance according to rework document §11. The core step one is done, and the shared capabilities needed should in principle all be there: the binding table (`bindings`, `focus`, `repeat`), free-form menus (`path` optional, `order`, `group`, `radio`), the theme and widget library (Button, Slider, ToolTipArea from `@vestra/theme.slint` and `@vestra/widgets.slint`), on-screen messages `plugin.osd.show`, the playback sequence `player.next/previous`, panel state and detaching `plugin.panels.*`, and the panel context menu `panel.showMenu`.

## 2. Rules

- Each author only writes in their own plugin directory. Do not change the core, do not change sibling plugins.
- Build to the target values in the rework document; for items marked "unknown" in the document, use the provisional values it gives.
- When something cannot be done with the existing interfaces, first look for a way inside the plugin; if it still cannot be done, write a request in the "API requests" section of the report following community workflow §4.1 (with a minimal repro and the desired usage), and **skip that item and continue with the others**. The coordinator forwards it to the API maintainer, who is strict and may answer with a way to do it with existing interfaces instead of changing the core.
- Menu positions and shortcuts follow the allocation table in §3 strictly, to avoid conflicts between plugins; for items not in the table, explain in the report and take an unused range from the table.
- No external image assets; icons are drawn with Slint `Path`.
- Colors, font sizes and widget sizes come from `Theme`; no hard-coded colors (special colors the theme does not cover can be explained in the report).
- Tests: if behavior covered by existing tests changes because of the redesign, update the tests; add tests for new interactions.
- Done when: `npm run plugin -- check|build|test <name>` all pass; the report lists the rework item numbers completed, items not completed and why, and API requests.

## 3. Menu and shortcut allocation table (rework document §6.2, §8.1)

`group` and the top-level `order` skeleton: `open` 100–199, `domain` 200–699, `screen` 700–799 (host fullscreen 700), `windows` 800–899, `exit` 900–999 (host quit 990). A submenu's position is the smallest `order` under it.

| Menu path | Plugin | order | group | Binding |
| --- | --- | --- | --- | --- |
| Open File… | playback | 100 | open | Ctrl+O, F3 |
| Open ▶ YouTube Video… | youtube | 111 | open | |
| Open ▶ Bilibili Video… | bilibili | 112 | open | |
| Open ▶ Open Subtitles… | subtitles | 113 | open | Alt+O |
| Close | playback | 120 | open | F4 |
| Playback ▶ Play/Pause | playback | 201 | domain | Space |
| Playback ▶ Previous / Next (`player.previous/next`) | playback | 202 / 203 | domain | PageUp, Mouse:X1 / PageDown, Mouse:X2 |
| Playback ▶ Seek ▶ Back 5 s / Forward 5 s | playback | 210 / 211 | domain | ArrowLeft / ArrowRight (`repeat`) |
| Playback ▶ Seek ▶ Back/Forward 30 s, 60 s, 300 s | playback | 212–217 | domain | Ctrl / Shift / Ctrl+Alt + ←/→ |
| Playback ▶ Go To… | playback | 218 | domain | G |
| Playback ▶ Speed ▶ Normal / Slower 0.1 / Faster 0.1 | playback | 220–222 | domain | Z / X / C |
| Playback ▶ Speed ▶ 0.5 / 1.5 / 2× (`radio`), Custom… | playback | 223–226 | domain | |
| Playback ▶ Playlist ▶ Add Files… / Add Link… / Loop ▶ (radio) / Shuffle | playlist | 230–249 | domain | |
| Subtitles ▶ Show Subtitles (checkbox) | subtitles | 301 | domain | Alt+H |
| Subtitles ▶ Subtitle Track ▶ / Sync Earlier / Later / Reset / Refresh / Subtitle Settings… | subtitles | 302–319 | domain | `>` / `<` / `/` |
| Video ▶ Picture Adjustments… / Reset / Brightness ±1 / Contrast / Saturation / Gaussian Blur | video | 401–419 | domain | Q, W/E, R/T, Y/U, Gaussian Ctrl+B |
| Video ▶ Screenshot ▶ Save Current Frame / Save Current Frame As… / Burst… / Export Clip… / Export Jobs… / Screenshot Folder… | capture | 420–439 | domain | Save Ctrl+E, Burst Ctrl+G |
| Audio ▶ Volume + / Volume − / Mute (checkbox) | playback | 501–503 | domain | ArrowUp, Mouse:WheelUp / ArrowDown, Mouse:WheelDown (`repeat`) / M |
| Audio ▶ Audio Workbench… / Equalizer (checkbox) / Presets ▶ (radio) / Reset | audio | 510–529 | domain | Equalizer Shift+E |
| Emulator ▶ … | nes | 601–619 | domain | |
| Fullscreen | host | 700 | screen | Enter, F11, double-click |
| Playlist (checkbox) | playlist | 800 | windows | F6 |
| Playback Info (checkbox) | info | 810 | windows | Tab |
| Plugin Workshop… | workshop | 820 | windows | |
| Quit | host | 990 | exit | Cmd+Q |

Removed (already decided in the rework document): "View → Show Playback Bar" (UI-MENU-03). "File" and "View" are no longer top-level submenus.

Inside the playlist (`focus: "panel"`; `path` can be omitted so the item only goes into the list context menu): Enter / P play, Delete remove selected, Ctrl+↑/↓/Home/End move, Ctrl+A select all.

## 4. Assignment

| Batch | Author | Plugins | Rework items (rework document §11) |
| --- | --- | --- | --- |
| 1 | A | playback | UI-BAR-01–11, UI-MENU-01/03/05, UI-DLG-01, the menus and keys in §3 that belong to playback, UI-OSD-02 |
| 1 | B | playlist | UI-PL-01/02/04–20, UI-MENU-01, UI-OSD-02 |
| 1 | C | video, audio | UI-TOOL-01/03–07/09, UI-MENU-01, UI-OSD-02 |
| 1 | D | subtitles, capture | UI-TOOL-01/03–05/08/09, UI-MENU-01/04, UI-DLG-03–05, UI-OSD-02 |
| 2 | E | youtube, bilibili, nes, workshop, info | UI-TOOL-01/05, UI-DLG-02/06, UI-MENU-01, UI-INFO appearance |

## 5. Acceptance (coordinator)

After all plugins are done: static checks, all plugin tests, menu snapshot (compared with §3), full GUI test (community workflow §6.2 plus the "acceptance" column of each rework document item), then commit and push.

## 6. Record

### Author deliveries (2026-10-20)

| Author | Plugins | Result | Tests |
| --- | --- | --- | --- |
| A | playback | two-row 62px bottom bar, icon buttons, 2px progress bar, 70px volume, time toggle, sidebar toggle, go-to dialog, all keys and on-screen messages | 21 |
| B | playlist | F6 checkbox toggle, detach button, tab strip, 20px single rows, color states, 35px bottom bar, double-click/Enter/Del/Ctrl+arrow keys, multi-select, drag, list context menu, loop/shuffle; durations via ffprobe | 35 |
| C | video, audio | 301-wide control-panel windows, Q/W/E/R/T/Y/U, Ctrl+B, Shift+E, on-screen messages; Gaussian strength cap corrected to 10 (existing bug) | 10, 15 |
| D | subtitles, capture | two windows for subtitle fine-tuning/settings, menu with real track names, Alt+H, `> < /`; Ctrl+E saves directly, Save As, burst dialog, screenshot folder persisted | 19, 13 |
| E | youtube, bilibili, nes, workshop, info | unified 420×122 enter-link dialog, one-line Chinese status, menu positions, Chinese field names in the info overlay | 14, 15, 11, 17, 4 |

### API requests and maintainer rulings

| Request | From | Ruling |
| --- | --- | --- |
| R1 read duration without opening the file | playlist | Rejected; ffprobe inside the plugin (local files, concurrency 2, 10 second timeout, abortable, cached) |
| R2 duplication in the widget library (icon buttons, slider callbacks) | coordinator observation | Button changed to Path icons (P1b design flaw, fixed as a bug); Slider gets no callback, the bottom-bar-specific component stays in the plugin |
| R3 change menu titles at runtime | subtitles | Only make `Command.dispose()` really delete (`commands.unregister`); dynamic lists are rebuilt with a child scope |
| R4 plugin data directory | capture | `plugin.dataDirectory` accepted; trial uses a temporary directory |

### Coordinator review

- `npm run check`, `format:check`, `boundaries` 0; `test:sdk` 54/54; `plugins:build` 0; all plugin and example tests pass (youtube normal over 3 consecutive runs); `cargo test` 158 pass.
- The menu snapshot matches the §3 allocation table; no global binding conflicts (F6 has both the global toggle and "Close List" inside list focus, with different scopes).
- `doc/product/menus.md` updated to the new menu tree.

### GUI test (2026-10-20, coordinator)

Real host, 11 plugins installed as managed, CGEvent mouse and keyboard + screenshots.

| Area | Result |
| --- | --- |
| Bottom bar | Pass: two rows about 60px, icon buttons, 2px yellow-line progress bar, 70px volume; the bottom bar button opens a file; right-click forward = next (through the core playback sequence) |
| On-screen messages | Pass: ↑ shows "Volume: 105%", E shows "Brightness +1% (+5%)" |
| Menus | Pass after fixes: the five groups open / domain / fullscreen / window entries / quit, with correct shortcut text, checkboxes and radio items; the subtitle track menu appears dynamically on load |
| Playlist | Pass after fixes: ADD pop-up menu, 20px single rows, durations (ffprobe) and total, full path on hover, double-click plays, now playing in blue, F6 toggle in sync with the bottom bar icon |
| Picture adjustments | Pass: 301-wide panel-style window, "Default" reset works |
| Audio workbench | Pass: vertical equalizer, preset pop-up menu, choosing "Vocals" updates sliders and values, applied |
| Subtitles | Pass: Open Subtitles → shown; menu "Show Subtitles" checkbox with Alt+H, Subtitle Track lists the real tracks |
| Screenshot | Pass after fixes: ⌘E (Ctrl+E) saves a PNG straight to the Pictures folder |
| Playback info | Pass: Tab shows the info overlay with Chinese field names |
| NES, YouTube, Plugin Workshop | Pass: compact windows; YouTube public video plays; workshop generate → install → counter |

Bugs found and fixed during the GUI test:

1. **Pressing ⌘ quits the program (core, severe)**: when a modifier key alone is pressed, Slint sends internal codes such as `\u{11}`, and `bindings.rs` parsed it as the control character of Ctrl+letter, giving Q, which hit the host ⌘Q binding. Any ⌘ combination (including ⌘E screenshot) triggered quit first. Fix: ignore `\u{10}`–`\u{18}` (Slint modifier key codes) first, with a unit test added. Step one P5 only tested ⌘Q and did not cover other ⌘ combinations, so it slipped through.
2. **Menu domain group cut up by separators**: video and audio used custom `group` names, adding extra separators between "Video", "Audio" and the other domains. Changed to `domain`, tests updated.
3. **Playlist row content centered**: `ToolTipArea` had no width or height, shrank to the content width and was centered; it now fills the row's width and height.
4. **Plugin Workshop button row stretched to about 100px**: the widgets `Button` has a minimum and preferred height but no maximum, so it stretches when the layout has spare room; the workshop layout now prevents the row from stretching vertically. Whether `Button` should by default not exceed its own height is recorded as an observation for the API maintainer.

Review: `npm run check`, `format:check`, `boundaries` 0; `test:sdk` 54/54; `plugins:build` 0; all plugin and example tests pass; `cargo fmt --check`, `clippy -D warnings` pass; `cargo test` 159 pass.

Known remaining (not in this round): subtitle track names show the host's temporary file name; the test launcher's commands put the "Plugins" menu first (affects only the test environment).
