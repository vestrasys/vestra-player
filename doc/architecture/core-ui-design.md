# Core UI and input capability design (C1–C11, C14)

> Historical design record (core UI round C1–C11, C14, 2026-10-20). Current behaviour: [architecture overview §4](README.md#4-slint-composition-and-input), [Windows, panels, settings pages, dialogs and the OSD](https://vestrasys.github.io/vestra-player-sdk/documents/Windows,_panels,_settings_pages,_dialogs_and_the_OSD.html), [Commands, menus and key bindings](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_key_bindings.html). Not maintained.

Date: 2026-10-20. Stage: P1a design; the P2 review has passed (its rulings are merged into this document); P1b implements this document.
Basis: the core UI round's task; [UI spec](../product/ui-spec.md) §2, §3, §6, §8, §9, §10.
Current code: `src/host/src/{main,commands}.rs`, `src/host/src/host/`, `src/host/src/ui/{menu,composition,window_state,view}.rs`, `src/host/src/process/mod.rs`, `src/host/ui/*.slint`, `src/sdk/plugin/*.ts`, `src/runtime/runner.ts`.

How to read: each item first gives the public interface, then how the host does it, lifecycle and errors, a usage example, the old things it replaces that are deleted in P4, and how existing plugins migrate. §14 explains why the menu tree does not change after migration, §15 is the rewrite reference for migrators to copy, §16 is the complete deletion list, and §18 holds the questions the coordinator has to decide.

## 0. General rules

- **New public surface at a glance** (only what plugins can see):

| Location | Added / changed | Item |
| --- | --- | --- |
| `CommandOptions` | Adds `order`, `group`, `radio`, `bindings`, `focus`, `repeat`; removes `shortcut`; `path` becomes 1–8 segments with any content, and can be omitted (not in the main menu) | C3, C4 |
| `plugin.panels` | Adds `panels.bottom`, `panels.sidebar` (slot state, any plugin can read and toggle); the `mount` options gain `title`; `Panel` drops `visibility`/`onVisibility`/`setVisible`/`toggle` and gains `showMenu` | C7, C11, C14 |
| `plugin.player` | Adds `provideSequence`, `next`, `previous`, `stats`; `PlayerState.can` gains `next`, `previous` | C6, C10 |
| `plugin.osd` (new module) | `show(text)`, `panel()` | C5, C10 |
| manifest capabilities | Adds `ui.osd` (`plugin.osd.*`). The other new interfaces use existing capabilities | C5, C10 |
| Slint | `import { Theme } from "@vestra/theme.slint"`, `import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint"`; all plugin Slint is compiled with the `fluent-dark` style by default | C2 |

- C1, C8, C9 have no plugin interface; they are host behaviour only.
- wire methods only serve the SDK ([architecture overview](README.md) §3). New: `layout.state`, `layout.set`, `ui.menu`, `osd.show`, `osd.panel`, `playback.stats`, `sequence.set`, `sequence.step`; new event pushed from the host to plugins: `sequence.step` (sent only to the sequence provider, no subscription needed). Removed: `ui.mountState`, `ui.showMount`. Changed: the `declaration` of `commands.register`, the parameters of `ui.mount`, `source.state` (the data gains a `sequence` field).
- During migration (P1b–P3, ended in P4, see §19) old and new fields **coexist**: the host accepts both `declaration.shortcut` and `bindings` (`shortcut` counts as one more binding), and the SDK keeps the old visibility methods of `Panel` (`ui.showMount`/`ui.mountState` still work). For commands that use **none of the new fields** the SDK still sends the old shape `{ title, path, shortcut }`, so tests of unmigrated plugins are not affected; once any new field is used (migrated plugins all write `order`), it sends the full shape. P4 deletes all of this at once (§16); from then on the SDK always sends the full shape.

## 1. C1 Main window shell

### 1.1 Host behaviour (no plugin interface)

`host.slint` changes to the structure below (values from UI spec §3):

```text
HostWindow (default 780×671, minimum 320×182, background Theme.bg-video #101010, default font see C2)
└─ keys := FocusScope              // entry point for global bindings (C3), wraps all content
   └─ HorizontalLayout { spacing: 0 }
      ├─ VerticalLayout (video column, horizontal-stretch: 1)
      │  ├─ canvas := FocusScope + video Image + mouse TouchArea (C3) + on-screen message layer (C5, C10)
      │  └─ bottom bar slot (normal mode: below the video; fullscreen: overlaid on the bottom of the video)
      ├─ divider 3px #000 (when the sidebar is attached and shown)
      └─ sidebar slot (full height, width = sidebar-width)
   └─ menu layer (the existing MenuView, styled per C4)
```

- Remove the Commands button row, the explanatory text, the diagnostic line, and the 8px outer margin (UI-SHELL-01–03). The video goes edge to edge.
- The sidebar takes the full height to the right of the video; the bottom bar is only below the video column (UI-SHELL-05).
- **Divider** (UI-SHELL-06/07): while dragging, Slint changes `sidebar-width` internally, range `[317, available width − 3 − 318]`; on release it calls `sidebar-resized(width)`, and the host records the width and persists it (C8). Default 317.
- **Not enough space** (UI-SHELL-08): the rules of the host's `window_state::Policy` become:
  - sidebar width = `min(user width, content width − 3 − 318)`, and not less than 317;
  - hidden only when `content width < 318 + 3 + 317 = 638`, with `reason: "space"`; `preferred` does not change (the menu check mark still says "shown");
  - there is no height condition any more (the sidebar is full height); delete the 640×360, 240, 280 constants.
- **Fullscreen** (UI-SHELL-09): the bottom bar is overlaid on the bottom of the video, shown only while the pointer is in its band (the bar's laid-out height `bottom-band-height` at the bottom edge, left of a popped-out sidebar) and hidden 500 ms after it leaves (FS-BAR-1, `Policy::bottom_band`); activity never reveals it. If playing (`!paused`) and `Policy::idle` (3 s without motion or keys, independent of the bar), the video area has `mouse-cursor: none`. In fullscreen the sidebar collapses into an overlay at the right edge and pops out when the pointer moves to the right edge (interaction alignment G-01; `reason: "fullscreen"` means collapsed); it does not hide while the pointer rests on the bottom bar or the popped-out layer (G-04).
- The logic that automatically enlarges the window when it is too narrow to show the sidebar (`sidebar_resize_requested`) is deleted and replaced by the shrinking rule above (see §18 Q7).
- The window's initial size and position are restored from persistence (C8); without a record, 780×671 and the system default position.

### 1.2 Replaced and deleted (P4)

In `host.slint`: the Commands button row, the explanatory text, the `diagnostic` property and `Text`, `padding: 8px`, the 1100×720 / 720×480 sizes, the window title constant; in `main.rs`: the two places that write `diagnostic` (frame refresh, "Preparing plugin UI…") and `sidebar_resize*`; in `window_state.rs`: `CANVAS_MIN_*`, `SIDEBAR_MIN_WIDTH = 240`, `SIDEBAR_TARGET_WIDTH = 280` and the "hide directly when there is not enough space" check.

Migration: plugins need no changes.

## 2. C2 Theme

### 2.1 Slint interface

```slint
import { Theme } from "@vestra/theme.slint";      // tokens, all constant out properties
import { ToolTipArea } from "@vestra/widgets.slint"; // hover tooltip (std-widgets has none)
```

`@vestra/theme.slint` (shipped with the host, path `src/host/ui/vestra/theme.slint`):

```slint
export global Theme {
    // §2.1 colours
    out property <color> bg-bar: #1e1e1e;      out property <color> bg-video: #101010;
    out property <color> bg-list: #3b3b3b;     out property <color> bg-panel: #333333;
    out property <color> line: #000000;        out property <color> hover-bg: #272727;
    out property <color> press-bg: #181818;    out property <color> text: #f0f0f0;
    out property <color> text-dim: #777777;    out property <color> text-list: #aeaeae;
    out property <color> text-played: #888888; out property <color> accent: #fae100;
    out property <color> accent-press: #c8b400; out property <color> accent-disabled: #7d7000;
    out property <color> icon-disabled: #3b3b3b; out property <color> playing: #2282f2;
    out property <color> button-bg: #555555;   out property <color> button-hover: #5c5c5c;
    out property <color> button-press: #444444; out property <color> button-disabled: #2a2a2a;
    // §2.2 fonts and font sizes (only these steps)
    out property <string> font-mono: "Menlo";  // the default font is not here: each window uses the system default font (§2.2)
    out property <length> font-xs: 9px;  out property <length> font-body: 11px;
    out property <length> font-tab: 12px; out property <length> font-title: 13px;
    // §2.3 control sizes
    out property <length> button-height: 21px;  out property <length> button-min-width: 66px;
    out property <length> button-padding: 8px;  out property <length> button-primary-height: 24px;
    out property <length> button-small-height: 18px; out property <length> button-small-min-width: 41px;
    out property <length> input-height: 21px;   out property <length> checkbox-row-height: 18px;
    out property <length> label-height: 14px;   out property <length> form-row-gap: 26px;
    out property <length> dialog-padding: 14px;
}
```

`@vestra/widgets.slint` provides three components this round (P2 ruling Q13: all tool windows and the bottom bar in step two use them; no other controls are made, use the `fluent-dark` std-widgets):

```slint
export component ToolTipArea {           // wraps any content; after hovering 600 ms shows text next to the pointer
    in property <string> text;
    @children
}
export component Button {                // dialog button (UI spec §2.3)
    in property <string> text;
    in property <string> icon;           // Path data; empty text and non-empty icon = icon button (step 2 R2)
    in property <length> icon-width: 14px;   // also the viewbox size
    in property <length> icon-height: 12px;
    in property <bool> stroked;          // draw a 1.5px line instead of filling
    in property <bool> enabled: true;
    in property <bool> small;            // false: 21px high, minimum 66px wide; true: 18px high, minimum 41px wide
    in property <bool> checked;          // on state of an icon button (icon becomes Theme.accent)
    callback clicked();
}
export component Slider {                // 2px line slider
    in-out property <float> value;
    in property <float> minimum: 0;
    in property <float> maximum: 1;
    in property <bool> enabled: true;
    in property <bool> panel;            // false: control bar style, 8px dot thumb, fill Theme.accent;
                                         // true: panel style, 14×12 thumb, fill Theme.playing, track Theme.bg-bar
    callback changed(float);             // called continuously while dragging
    callback released(float);            // on release
}
```

Styles: ToolTipArea has background `Theme.bg-panel`, a 1px `Theme.line` border, `Theme.font-body`, `Theme.text`, is shown with a `PopupWindow`, and disappears when the pointer leaves or a button is pressed. A Button text button has background `Theme.button-bg` (hover `button-hover`, pressed `button-press`, disabled `button-disabled`) and text `Theme.text-list` 11px; an icon button has no background, its Path is `Theme.text-dim` normally, `Theme.accent` on hover and when on, `Theme.accent-press` when pressed, `Theme.icon-disabled` when disabled. The Slider thumb is `Theme.text` normally; in control bar style it is `Theme.accent` on hover and `Theme.accent-press` when pressed; the unfilled part in control bar style is `Theme.line`.

### 2.2 How the host does it

- **Import mechanism**: `Compiler::set_library_paths` in slint-interpreter 1.16 supports imports of the form `@library/file` (`find_file_in_library_path` in `i-slint-compiler` `typeloader.rs`). The host maps the library name `vestra` to the shipped directory (during development `src/host/ui/vestra/`, when packaged `Resources/ui/vestra/`, the same way `host.slint` is found today). No virtual files or rewriting of plugin sources is needed.
- The host currently compiles Slint in three places: `ui.rs` (tool windows), `composition.rs` (main window and panels), `package_manager.rs` (package validation). They are merged into one `slint_compiler()`: set the library path, `set_style("fluent-dark")`. As a result, **std-widgets controls in plugins that do not change their code become dark automatically** (Button, Slider, LineEdit, CheckBox, ComboBox, ScrollView); this is an expected change. Checked: the `.slint` files of the existing light tool windows (video, subtitles, capture, nes, youtube, bilibili, workshop, playlist/link, bookmark) do not hard-code text colours, so no unreadable text appears in dark mode; the audio, playback, and playlist sidebars already use their own dark styling.
- `Theme` is read-only constants; the same file imported by several plugins and shared in the main window does not cause interference (what the comment in `composition.rs` forbids is sharing **stateful** globals).
- **Default font**: delete the forced `Arial Unicode MS` and the code that imports it by file. P1b found that Slint 1.16 lays out text with parley and falls back per character through fontique (on macOS via CoreText's system fallback chain, `i-slint-core` `textlayout/sharedparley.rs`), so by default **no font is specified**: system UI font + system CJK fallback. A fallback option is implemented but off by default: with the environment variable `VESTRA_FONT_FALLBACK=hiragino`, the system's `/System/Library/Fonts/Hiragino Sans GB.ttc` is imported and set as the default font of the main window, floating windows, and tool windows (tool windows keep "inject only if the plugin did not write `default-font-family`"). **P5 testing (2026-10-20): without a specified font, all Chinese text in plugin windows and panels showed as blank, so Hiragino was enabled by default instead** (`VESTRA_FONT_FALLBACK=system` turns it off); the info layer originally used `font-mono` (Menlo), which also lacks Chinese, and was changed to the default font (§18 Q12). **Current state (2026-10-22, user ruling)**: Slint 1.18.1 already falls back per character; the forced Hiragino and `VESTRA_FONT_FALLBACK` have been deleted, and windows do not specify a font (system default + fallback); the info layer does not need to go back to `font-mono`. For how to fill in a missing Chinese font on a platform, see [architecture overview §4](README.md) "Fonts".
- The host's own UI (`host.slint`, `menu-view.slint`) also uses the `Theme` tokens; the menu-specific colours (§6.3) stay inside `menu-view.slint` and are not in `Theme`.
- Editor support: the SDK documentation site [Concepts: UI](https://vestrasys.github.io/vestra-player-sdk/documents/Windows,_panels,_settings_pages,_dialogs_and_the_OSD.html#rules-for-plugin-slint-files) describes the `libraryPaths` setting of the Slint language server (`"vestra": "<repo>/src/host/ui/vestra"`).

### 2.3 Errors

Importing a token or component that does not exist is a Slint compile error, reported as today: tool window `show()` fails, panel `ready` fails (`ui-prepare`), package validation fails.

### 2.4 Example

```slint
import { Button } from "std-widgets.slint";
import { Theme } from "@vestra/theme.slint";
import { ToolTipArea } from "@vestra/widgets.slint";

export component Picture inherits Window {
    width: 301px; background: Theme.bg-panel;
    in property <string> path;
    callback reset();
    VerticalLayout {
        padding: Theme.dialog-padding; spacing: 8px;
        ToolTipArea { text: root.path;
            Text { text: "Brightness"; color: Theme.text; font-size: Theme.font-body; } }
        Button { text: "Default"; height: Theme.button-small-height; clicked => { root.reset(); } }
    }
}
```

### 2.5 Replaced and deleted, migration

- Delete: the `Arial Unicode.ttf` import and `"Arial Unicode MS"` in `ui.rs` and `composition.rs`; `default-font-family: "Arial Unicode MS"` in `host.slint`; rewrite the sentence "plugin UI uses the host's built-in CJK font by default" in architecture overview §4.
- Migration: plugins do **not** need to change their `.slint` this round (behaviour is unchanged; dark is an expected change on the host side). Switching to `Theme` tokens and removing hard-coded colours is step two (UI-THEME-01 is accepted in step two).

## 3. C3 Input binding table

### 3.1 SDK interface

```ts
interface CommandOptions {
  // …other fields see C4
  /** Default bindings, e.g. ["Space"], ["Ctrl+O", "F3"], ["Mouse:WheelUp"]. */
  bindings?: string[];
  /** "panel": these bindings only apply while this plugin's panel (attached or floating) has focus. Global by default. */
  focus?: "panel";
  /** true: when held down, fires repeatedly with the system auto-repeat (arrow-key seeking, volume). Default false. */
  repeat?: boolean;
}
```

The `Command` handle is unchanged. The user overlay has no SDK interface (no rebinding UI is made).

### 3.2 Binding syntax

A binding = some modifiers + one input, joined with `+`, case-sensitive, fixed spelling:

```text
binding   = { modifier "+" } input
modifier  = "Ctrl" | "Alt" | "Shift" | "Cmd"          (written in the fixed order Ctrl, Alt, Shift, Cmd)
input     = key | "Mouse:" mouse
key       = "A".."Z" | "0".."9" | "F1".."F24"
          | "Space" | "Enter" | "Tab" | "Escape" | "Backspace" | "Delete" | "Insert"
          | "Home" | "End" | "PageUp" | "PageDown"
          | "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
          | "Plus" | any other single printable symbol (> < / , . ; ' [ ] \ - = ` etc.)
mouse     = "LeftDoubleClick" [ ":LeftHalf" | ":RightHalf" ]
          | "MiddleClick" | "X1" | "X2"
          | "WheelUp" | "WheelDown" | "WheelLeft" | "WheelRight"
```

Rules:

- **Letters** match by key name, regardless of Caps Lock: `E` = press E; `Shift+E` = Shift plus E.
- **Symbol keys** match the character actually typed, and cannot also be written with `Shift`: subtitle sync's `>` is written `">"`, not `"Shift+."`. `+` is written `Plus`. The same for digits: `"Shift+1"` is invalid; write `"!"`.
- **Ctrl and Cmd on macOS**: on macOS `Ctrl+X` fires on both ⌃X and ⌘X (UI spec §0); `Cmd+X` only accepts ⌘X. For one event, an exact match is looked up first (⌘X looks for `Cmd+X` first); if none is found, it is looked up as `Ctrl+X`. On other platforms `Cmd` is the Win/Super key. (On macOS Slint reports ⌘ as `control` and ⌃ as `meta`; the host swaps them back in Rust.)
- **Mouse** bindings only apply in the video area (except `X1`/`X2`, see below) and can have modifiers. `LeftDoubleClick:LeftHalf`/`:RightHalf` distinguish by whether the double-click is on the left or right half of the video; when bindings with and without a zone exist at the same time, the one with the zone wins, and this does not count as a conflict. The wheel counts steps by accumulated distance (the threshold is set in P1b by testing with trackpad and mouse), firing once per step. `X1`/`X2` are the mouse side buttons (winit `Back`/`Forward`); since host 0.6.0 they are window-level, as in browsers: the window's winit hook resolves them wherever the pointer is (video, bottom bar, sidebar, a floating panel), not over an open menu (`input.rs` `observe_side_button`).
- The right button is reserved for the menu and cannot be bound. Left single click, long press, and right double-click are not done this round (§18 Q3). Default mouse bindings, as history: until host 0.5.1 the host bound `Mouse:LeftDoubleClick` to fullscreen and nothing to a single click; from 0.5.2 (user ruling 2026-10-26) `Mouse:LeftClick` (added in round 3a) is Play/Pause and nothing is bound to a double click.
- Default bindings **fire only on press and ignore the auto-repeat of a held key** (same as Space/←/→ today); when a command declares `repeat: true`, the system auto-repeat of keys also fires (§18 Q6). Mouse events have no repeat.
- While the menu is open or a system dialog is open, all bindings are paused (same as today).
- A bound `Tab` is caught by the host and is no longer used for focus switching inside the main window.

When the format is invalid (unknown key name, wrong modifier order, symbol key with Shift, duplicates), `commands.add` throws immediately with an error of `code: "invalid"` (the SDK checks first, the host checks again on registration, and the host wins).

### 3.3 Scope and focus

| Scope | When it applies | Implementation |
| --- | --- | --- |
| Global (default) | The main window or a floating panel window has focus, and the key was not consumed by the focused control (typing in an input box, arrow keys that a list handles itself, etc.) | The outermost `keys := FocusScope` in `host.slint` receives keys that bubble up. In Slint 1.16 keys go first to the focused control, and those not accepted are passed up level by level to the parent FocusScopes (`i-slint-core` `window.rs` `process_key_input`), so space and arrow keys in an input box do not fire bindings |
| `focus: "panel"` | While focus is inside that plugin's panel, looked up **before** global | The host wraps a FocusScope around the panel (`focus-on-click` is true by default, so clicking empty space in the panel gives it focus); keys not accepted inside the panel are first looked up here in the "panel P" scope, and if none is found they bubble up to global |

- Tool windows (plugin `windows.create`) do not fire any bindings, same as today; tool-window-scoped bindings are not done this round (§18 Q5).
- Compared with today, global bindings also apply when focus is on empty space in the sidebar or bottom bar (today only when the video has focus). All places where shortcuts worked before still work.

### 3.4 Host data structures

New file `src/host/src/bindings.rs` (pure data, no UI):

```rust
pub struct Mods { ctrl: bool, alt: bool, shift: bool, cmd: bool }
pub enum Input { Key { key: Key, mods: Mods }, Mouse { event: MouseEvent, zone: Option<Zone>, mods: Mods } }
pub enum Scope { Global, Panel(OwnerGeneration) }
pub struct Claim { owner: OwnerGeneration /* 0 = host */, command: String, user: bool }
pub struct Table {
    defaults: BTreeMap<(OwnerGeneration, String), (Vec<Input>, Scope)>, // from command declarations
    overlay: Overlay,               // user overlay: "<manifest id>/<command id>" → binding list ([] = unbind)
    active: BTreeMap<(Input, Scope), Claim>,   // recomputed from the two above
    conflicts: Vec<Conflict>,                  // declarations that lost
}
impl Table {
    fn parse(text: &str) -> Result<Input, String>;
    fn resolve(&self, input: &Input, focused_panel: Option<OwnerGeneration>) -> Option<&Claim>;
    fn label(&self, owner: OwnerGeneration, command: &str) -> Option<String>; // text shown in the menu
}
```

- The table is **derived data**: it always equals a recompute from the declarations and the overlay. Since the many-plugin performance work (2026-11-02) it is kept up to date one command at a time: a registration, removal (plugin deactivated) or overlay change takes out and puts back only the claims of the commands it touches, sorted per input, and recomputes the winner, fallbacks and conflicts of those inputs only (a whole recompute per registration was quadratic: 4.3 s for 6000 commands). A unit test checks it against the whole recompute after every kind of change.
- The host's own commands (owner 0) are also in the table:

| Command | Default bindings | Menu | State |
| --- | --- | --- | --- |
| `host/fullscreen` Toggle Fullscreen | `Escape` (user ruling 2026-11-01, IA-CORE-04; `Enter`, `Alt+Enter`, `F11` up to 0.5.x; `Mouse:LeftDoubleClick` until host 0.5.1) | top level, `group: "screen"`, `order: 810` (700 until the slim menu of 2026-11-08: now alone between two separators, after the column toggles and the chapter list and before Vestra Player AI…), checked = whether fullscreen | always available |
| `host/adjust` Adjustments and Effects | none (the panels' own commands have Alt+1…4) | top level since user ruling 2026-11-08, `group: "windows"`, `order: 791` (where Picture…, Audio…, Subtitles…, Playback… were; they are palette-only now); also the sliders button of the bottom-bar strip, same title; opens the adjust column at its last tab, checked = whether the column is wanted shown (host 0.7.6 to then: outside the menu, titled Adjust) | available while a panel is in the adjust column |
| `host/sidebar` Sidebar | none (F6 is the playlist's) | top level since user ruling 2026-11-08, `group: "windows"`, `order: 800` (where Playlist was; `playlist.visibility` is palette-only now); also the rightmost button of the bottom-bar strip, same title; opens the sidebar at its last tab, checked = whether the sidebar is wanted shown (player 0.5.10 to then: outside the menu, titled Toggle Sidebar) | available while a panel is in the sidebar |
| `host/palette` Command Palette… | `Ctrl+Shift+P` (⌘⇧P on macOS too) | top level since the slim menu (2026-11-08), `group: "settings-main"`, `order: 829`, directly above Options… (outside the menu from host 0.7.4 until then) | always available |
| `host/enter-fullscreen` Enter Fullscreen | none | not in the menu | only available in windowed mode |
| `host/exit-fullscreen` Leave Fullscreen | none (`Escape` up to 0.5.x) | not in the menu | only available in fullscreen |
| `host/quit` Quit | `Cmd+Q` (the menu shows it on macOS only: on Windows and Linux `Cmd` is the Windows / Super key, whose combinations the system takes, so menus show no `Cmd` binding, `platform::MENU_SHOWS_CMD`; there the close button and Alt+F4 quit) | top level, `group: "exit"`, `order: 990` | always available |

  Whether "Fullscreen" and "Quit" appear in the menu this round, see §18 Q2.
- **Main window event flow**: the Slint callbacks `key-input(scope, text, ctrl, alt, shift, meta, repeat) -> bool` and `mouse-input(kind, zone, ctrl, alt, shift, meta) -> bool` query a read-only copy of the table **synchronously** on the UI thread (`Rc<RefCell<Table>>`, replaced as a whole on recompute); if bound, they return true (`accept`) and send `Event::Binding(owner, id)`; the main loop then checks enabled/busy with the existing `resolve_execution` and sends `commands.execute`. Bound but the command is disabled: the key is caught and not executed (same as Space today).
- **Emulator input capture** (`producer.declareInput`, not public in the SDK): the hard-coded `F11 | Escape` pass-through in the winit hook becomes "pass through keys bound to host commands"; other captured keys still go to the producer first.

### 3.5 Conflict rules

- Conflict = two or more commands declare the same `(input, scope)`. Registration is not made to fail (one plugin choosing a key should not make another plugin fail to activate); instead **exactly one is kept, deterministically**, and the rest are recorded in `conflicts`:
  1. User overlay bindings win over any default binding (an overridden default binding is not a conflict, just replaced);
  2. The host's own commands win over plugins;
  3. Among plugins, the one loaded first (smaller owner generation) wins. Generations are assigned in `--plugin` order / package store order, so the same set of plugins gives the same result on every start; newly installed or trial-loaded plugins have larger generations and do not take keys from existing plugins.
- Reporting: after each change, each new conflict writes one host log line (`binding conflict: Space → vestra.playback/toggle-pause; ignored: x.y/z`); the test host provides `host.conflicts` for plugin tests to assert on. There is no conflict query interface for plugins (no users). Alternative rules see §18 Q4.
- When the winner is deactivated, its inputs are recomputed and the bindings that lost take effect automatically.
- Different scopes do not conflict: a panel-scoped `Enter` and the host's global `Enter` coexist; while the panel has focus, the panel wins.
- Plugins being trial-loaded: their bindings are always ignored (same as clearing `shortcut` today).

### 3.6 Persistence

The user overlay is stored in the `bindings` field of the C8 state file; keys are `"<manifest id>/<command id>"` (host commands are `"host/<id>"`), values are arrays of binding strings, `[]` means unbound. This round it is only read, not written (there is no rebinding UI); when other state is saved it is kept as is. Invalid entries are ignored and logged.

### 3.7 Example

```ts
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  plugin.commands.add({
    id: "toggle", path: ["Playback", "Play/Pause"], order: 263,
    bindings: ["Space"],
    run: () => player.setPaused(!player.state.paused),
  });
  plugin.commands.add({
    id: "volume-up", path: ["Audio", "Volume +"],
    bindings: ["ArrowUp", "Mouse:WheelUp"],
    run: () => player.setOutput({ volume: Math.min(200, player.state.volume + 5) }),
  });
  plugin.commands.add({
    id: "remove", path: ["Playlist", "Remove Selected"], focus: "panel",
    bindings: ["Delete"],     // only while this plugin's panel has focus
    run: () => removeSelected(),
  });
}
```

### 3.8 Replaced and deleted, migration

- Delete: `CommandOptions.shortcut`, `CommandDeclaration.shortcut` (two places in the SDK), `Declaration.shortcut` in `commands.rs`, `canvas_shortcut()`, `resolve_shortcut()`, the "main-canvas shortcut already registered" conflict; the hard-coded F11, Escape, Space, ←, → and `window-input("shortcut:…")` in the canvas `key-pressed` of `host.slint`; `Host::canvas_shortcut` in `main.rs`, the `shortcut:` / `toggleFullscreen` / `escape` branches of `WindowInput`; the code that clears `shortcut` on trial load; `"F11" | "Escape"` in the producer hook.
- Migration (behaviour unchanged): `shortcut: "Space"` → `bindings: ["Space"]`; `"ArrowLeft"`, `"ArrowRight"` keep the same names. Among existing plugins only playback has these three.

## 4. C4 Menu structure

### 4.1 SDK interface

```ts
interface CommandOptions {
  id: string;
  /** Menu path, 1–8 segments; the last segment is the menu item. 1 segment = top-level direct command. Omitted = not in the main menu. */
  path?: string[];
  title?: string;               // defaults to the last segment of path; required when path is omitted
  /** Order among siblings, smaller first; default 0. Integer, range ±1 000 000. */
  order?: number;
  /** Sibling group name; when two adjacent items are in different groups, a separator is drawn between them. Default "domain". */
  group?: string;
  /** checked is drawn as a radio dot instead of ✓. The plugin maintains mutual exclusion of checked itself. */
  radio?: boolean;
  bindings?: string[]; focus?: "panel";   // C3
  enabled?; checked?; busy?; reason?; run();  // unchanged
}
```

### 4.2 Ordering and grouping rules

1. At each menu level, every node has a key `(order, group)`:
   - menu item: its own `order`, `group`;
   - submenu (a path segment): the `order` and `group` of the **first-ordered command** under it (recursively). Submenus need no separate declaration.
2. Siblings are sorted by `order` ascending; with equal `order`, first by plugin manifest id, then by registration order within the same plugin. The result does not depend on when registrations arrive (the existing promise is unchanged). **No more sorting by title alphabetically.**
3. `group` only decides separators: after sorting, a line is drawn between two adjacent items whose `group` differs. `group` does not take part in sorting, so for a group to stay together, its `order` values must also be contiguous.
4. Path rules: 1–8 segments, each 1–80 bytes, no control characters and no leading or trailing whitespace; the same path cannot be registered twice, and cannot be both a menu item and a submenu (`conflict`, same as today). The top level is no longer restricted to domain names. Commands that omit `path` (P2 ruling Q9) are not in the main menu, `title` is required; they take part in the binding table as usual and can be put into a panel context menu (C11); in step two the playlist's list menu (select all, sort) and Del/Enter inside the list use this.
5. Recommended skeleton (a host documentation convention, not enforced by the host):

| Group `group` | Top-level `order` range | Content |
| --- | --- | --- |
| `open` | 100–199 | Open File, Open ▸, Close |
| `domain` | 200–699 | Domain submenus such as Playback, Subtitles, Video, Audio, Emulator |
| `screen` | 700–799 | Fullscreen (host 700) |
| `windows` | 800–899 | Window entries such as Playlist, Playback Info, Plugin Workshop |
| `exit` | 900–999 | Quit (host 990) |

   The top-level numbering table is in [doc/product/menus.md](../product/menus.md) (maintained by the interaction owner); the numbering inside a submenu is decided by the plugin that owns that submenu.
6. Trial-loaded plugins: the **last segment** of the menu item gets the prefix `Trial <owner> · ` (today it is the 2nd segment; the result is the same while all existing commands have 2 segments), and bindings are ignored.

### 4.3 Host data structures

`commands.rs`:

```rust
pub struct Declaration {
    pub title: String,
    pub path: Vec<String>,          // 0..=8; empty = not in the main menu
    pub order: i32,                 // default 0
    pub group: String,              // default "domain"
    pub radio: bool,
    pub bindings: Vec<String>,      // raw text; parsing and activation in bindings.rs
    pub focus: Option<Focus>,       // Some(Panel)
    pub repeat: bool,
}
pub struct Registry { commands: BTreeMap<CommandKey, CommandSnapshot>, sequence: BTreeMap<CommandKey, u64>, revision: u64 }
```

- `Registry::new` no longer takes a domain list; `snapshot()` builds the tree per §4.2. `MenuNode::Group` gains `order`, `group` (taken from the first child), `MenuNode::Command` is unchanged; the menu projection (`menu.rs`) inserts separator rows where the group changes.
- The output of `snapshot()` goes to the menu projection; it is built once per registry revision and shared (`Arc<MenuSnapshot>`) until the next change; `bindings::Table::label()` provides the shortcut text on the right of each row.

### 4.4 Menu style (host, §6.3)

`menu-view.slint`: background `#444444`, 1px `#000000` border, no rounded corners; text `#bbbbbb` 12px; hover background `#181818`, text `#fae100`; disabled `#777777`; separator 1px `#27282f` (row height 7px, the geometry in `menu.rs` updated to match); check box 14×14, corner radius 3, background `#34353f`, white check; radio 12×12 outer ring `#34353f`, centre dot `#2282f2`; shortcut text right-aligned `#777777`. Item height 24px.

The shortcut text is the command's first active **keyboard** binding (mouse bindings are not shown), displayed as: modifiers `Ctrl+`, `Alt+`, `Shift+`, `Cmd+`; `ArrowLeft/Right/Up/Down` → `← → ↑ ↓`; `Escape` → `Esc`; `PageUp/PageDown` → `PgUp/PgDn`; `Delete` → `Del`; everything else as is (§18 Q11).

### 4.5 Errors

| Case | Result |
| --- | --- |
| Invalid number of path segments, text, or `order`; invalid `group` (same rules as path segments) | Registration in `commands.add` fails with `invalid` (during activation = activation fails) |
| Duplicate path, leaf conflicts with submenu | `conflict` (unchanged) |
| More than 64 per plugin, more than 512 in total | `limit` (unchanged) |

### 4.6 Example

```ts
// top-level direct command, groups, radio group
plugin.commands.add({ id: "open", path: ["Open File…"], group: "open", order: 100,
  bindings: ["Ctrl+O", "F3"], run: () => openFile() });
for (const [i, rate] of [0.5, 1, 1.5, 2].entries())
  plugin.commands.add({
    id: `rate-${rate}`, path: ["Playback", "Speed", `${rate}×`], order: 230 + i, radio: true,
    checked: () => player.state.rate === rate,
    run: () => player.setRate(rate),
  });
plugin.commands.add({ id: "list", path: ["Playlist"], group: "windows", order: 800,
  checked: () => plugin.panels.sidebar.state?.preferred ?? false,
  bindings: ["F6"], run: () => plugin.panels.sidebar.toggle() });
```

### 4.7 Replaced and deleted, migration

- Delete: `Registry::new(allowed_domains)` and `domains`/`domain_order`, the 8 domain names in `main.rs`, the "2..=6 segments" and "domain is not allowed" checks, the domain ranking + alphabetical `path[1..]` sort in `snapshot()`, the code that rewrites `path[1]` on trial load, and the sentence "top-level grouping convention" in reference doc §4.
- Migration: add `order` to every command, with values from the table in §14; `path` and `title` are unchanged; do not write `group` (all default `"domain"`, no separators, same as today).

## 5. C5 On-screen messages

### 5.1 Interface

```ts
interface Osd {
  /** Shows a one-line message at the top left of the video, fading out after 2 seconds; a new message replaces the old one immediately. */
  show(text: string): Promise<void>;
  panel(): OsdPanel;   // C10
}
interface Scope { readonly osd: Osd }
```

Needs the new capability `ui.osd`.

### 5.2 Host behaviour

- Top left of the video area, left/top margin 10px, bold 16px, white text with a 1px black outline (Slint `Text` `stroke`/`stroke-width`; from the 2026-10 visual identity round, a semi-transparent dark rounded backing plate instead, see UI spec UI-OSD-01); after 2000 ms it fades out over 300 ms; a new message from any plugin replaces the current one (only one line). Shown in both fullscreen and normal mode.
- Text 1–200 characters; line breaks are replaced with spaces.
- A message does not belong to any resource: when a plugin is deactivated, a message already shown disappears on time as usual (its action buttons go at once, see below).
- wire: `osd.show { text }`; the host replies after showing it.
- **Actions** (player 0.7.6, SDK 1.6.9; capture tab proposal R5): `osd.show { text, args?, actions? }` with at most 2 (`OSD_ACTIONS`) `{ title, command }`. A title is translated like the text and is 1–40 characters (`OSD_ACTION_CHARS`); a command is the local id of one of the plugin's own registered commands, checked when shown (otherwise `invalid`). The buttons follow the text on the same line (`host.slint` `OsdAction`, two fixed slots `osd-action-1/2`: an empty title draws nothing and takes no space, so no element is created or destroyed while the pointer moves). A message with actions stays `OSD_ACTION_MS` (4000 ms); the window tick (`osd_hold`) keeps it while `osd-hover` is true and the pointer is not idle (`policy.idle`), and hides it 2000 ms after the pointer leaves or rests, and at the latest `OSD_HOLD_MS` (30 s) after it showed: Slint updates `has-hover` only on pointer events, so a message that moved under a still pointer (a fullscreen toggle by key, the sliding title strip) would otherwise read as hovered for ever. Once hidden, the button titles are cleared after `OSD_CLEAR_MS` (600 ms: the 300 ms fade and 300 ms more), so no hidden, disabled button stays in the accessibility tree. Only a message with actions takes the pointer (its `TouchArea` is disabled otherwise, so a press on a plain message still reaches the video). A press sends `Event::OsdAction`: the message hides and the host runs the command through `execute_command`, as a menu does (a disabled command does nothing). Any newer message, the host's own included, replaces the buttons. When the plugin stops, its buttons go and the text stays for its time; while they show, `ui.osd` records the plugin as live ("an OSD message with actions"), so it is not parked meanwhile.
- **Layout, rebuilds and diagnosis** (0.7.6): the text takes at most the video width less the margins, padding and buttons, and ends in "…" beyond it, so the buttons always show whole (`osd-text-room`). The host keeps what the OSD shows (`OsdShown`: text, button titles, visible) and writes it again into a shell rebuilt for a mount change, which starts with the OSD hidden; the message keeps its buttons and its remaining time (`osd_until`), and the plugins' information blocks are written again too. `VESTRA_DEBUG_OSD=1` writes a stderr line for every message shown or hidden: the plugin that showed it (or the player), its number of buttons, the text, and why it went (`time`, `pointer idle`, `hold cap`, `action pressed`, `replaced`, `its plugin stopped: buttons only`), and one when a rebuild kept it.

### 5.3 Errors

`invalid` (empty or too long), `unavailable` (headless mode), `capability`.

### 5.4 Example

```ts
plugin.commands.add({
  id: "volume-up", path: ["Audio", "Volume +"], bindings: ["ArrowUp"],
  run: async () => {
    const volume = Math.min(200, plugin.player.state.volume + 5);
    await plugin.player.setOutput({ volume });
    await plugin.osd.show(`Volume: ${volume}%`);
  },
});
```

### 5.5 Replaced and deleted, migration

No old interface. Plugins do not use it this round (the on-screen message texts are step two, UI-OSD-02).

## 6. C6 Playback sequence

### 6.1 Interface

```ts
interface SequenceProvider {
  next(): unknown;                 // called by the SDK in the provider's scope
  previous(): unknown;
  canNext: Bindable<boolean>;      // re-evaluated on every update round, sent only on change
  canPrevious: Bindable<boolean>;
}
interface Player {
  /** Registers as the playback sequence provider; only one at a time. dispose revokes it. */
  provideSequence(provider: SequenceProvider): Disposable;
  next(): Promise<void>;           // handed to the provider; does not wait for the provider to finish
  previous(): Promise<void>;
}
interface PlayerState { readonly can: { /* existing fields */ next: boolean; previous: boolean } }
```

Capabilities: `provideSequence`, `next`, `previous` need `playback.control`; `can.next/previous` are read with `player.state` (`playback.read`; live updates need `events`).

### 6.2 Host behaviour

- The host keeps `sequence: Option<(owner, can_next, can_previous)>`. wire: `sequence.set { canNext, canPrevious }` (the first call registers; another provider already exists → `conflict`), `sequence.set null` revokes; `sequence.step { direction: "next" | "previous" }`.
- `sequence.step`: no provider, or the matching flag is false → `unavailable`; otherwise it pushes the event `sequence.step { direction }` to the provider and replies to the caller immediately. The provider's SDK calls `next()` / `previous()` in the scope; errors go to the provider's own `onError`.
- The flags are merged into the source state: the host adds `sequence: { next, previous }` to the data of every `source.state` event and `source.state` reply; when the flags change, the host resends `source.state` once with the current source state (`eventSequence` unchanged). The SDK already accepts states with "the same sequence number but different content" (`player.ts` `accept`), so no new event is needed; the source stamp is unchanged, and `ifCurrent` conditions are not affected.
- When the provider is deactivated, calls `dispose`, or the plugin exits, the host revokes the provider; the flags become false and the state is resent.

### 6.3 Errors

`conflict` (another provider already exists; during activation = activation fails), `unavailable` (no provider, or cannot go forward/back), `capability`.

### 6.4 Example

```ts
// playlist: register as provider
plugin.player.provideSequence({
  canNext: () => !!queue.neighbor(1) && !busy(),
  canPrevious: () => !!queue.neighbor(-1) && !busy(),
  next: () => action("next"),
  previous: () => action("previous"),
});

// any plugin: call
plugin.commands.add({
  id: "next", path: ["Playback", "Next"], bindings: ["PageDown", "Mouse:X2"],
  enabled: () => plugin.player.state.can.next,
  run: () => plugin.player.next(),
});
```

### 6.5 Replaced and deleted, migration

No old interface (previously plugins had no way to call each other). In the reference migration, playlist registers as the provider (no visible change to users; the menu items "Previous Item/Next Item" are still its own commands). Other plugins do not use it this round.

## 7. C7 Panel slot state (sidebar and bottom bar)

### 7.1 Interface

```ts
interface Panels {
  mount(slot: "bottom" | "sidebar", file: string,
        options?: { component?: string; global?: string; title?: string }): Panel;
  /** State of the bottom bar slot; any plugin can read and toggle it, without knowing who mounted it. */
  readonly bottom: PanelSlot;
  readonly sidebar: SidebarSlot;
}
interface PanelSlot {
  readonly state: SlotState | null;          // null: no panel in this slot
  onChange(listener: (state: SlotState | null) => void): Disposable;
  setVisible(visible: boolean): Promise<void>;  // sets the user preference
  toggle(): Promise<void>;                      // flips the preference based on the host's current value
}
interface SidebarSlot extends PanelSlot {
  setDetached(detached: boolean): Promise<void>;  // C14
}
interface SlotState {
  preferred: boolean;      // the user's choice (menu check mark)
  effective: boolean;      // whether it is shown now (attached in the main window or in a floating window)
  reason: "userHidden" | "space" | "fullscreen" | "idle" | null;
  detached: boolean;       // whether the sidebar is a floating window; always false for the bottom bar
}
interface Panel extends View, Disposable {
  readonly ready: Promise<void>;
  setInteraction(locks: Partial<{ focused; dragging; popupOpen: boolean }>): void; // unchanged
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>; // C11
}
```

Capabilities: `panels.bottom/sidebar` need `ui.mount`; live updates of `state` need `events` (without it, it is only refreshed after your own `setVisible`/`toggle`/`setDetached`).

### 7.2 Host behaviour

- wire: `layout.state` → `{ bottom: SlotState | null, sidebar: SlotState | null }`, replying `pending` while the layout is not confirmed (same rule as `ui.mountState` today; the SDK keeps the last value); `layout.set { slot, visible?, detached? }`. The caller no longer has to be the one that mounted it.
- After receiving a `window.state` event the SDK reads `layout.state` again (the logic of today's `PanelHandle.#read` moves to module level, one per plugin), and only calls `onChange` and triggers an update round when the value changes.
- The preference is persisted by the host (C8; the sidebar is recorded by the manifest id of the plugin that mounted that slot; the bottom bar is not persisted, same as today).

### 7.3 Errors

`missing` (no panel in this slot), `invalid` (setting `detached` on the bottom bar), `unavailable` (headless mode), `capability`.

### 7.4 Example

```ts
// playback's bottom bar button toggles the playlist sidebar (how step two UI-BAR-09 writes it)
const sidebar = plugin.panels.sidebar;
bar.bind(() => ({
  listShown: sidebar.state?.preferred ?? false,
  listAvailable: sidebar.state !== null,
}));
bar.on("toggleList", () => sidebar.toggle());
```

### 7.5 Replaced and deleted, migration

- Delete: `Panel.visibility`, `onVisibility`, `setVisible`, `toggle`, the type `PanelVisibility` (renamed `SlotState`, with `detached` added); wire `ui.mountState`, `ui.showMount` (including the `scope: "presentation"` branch the SDK never calls, and `Policy::reveal_sidebar_in_fullscreen`, `sidebar_override`); the `MountVisibility` type; the matching fake implementations in the test host.
- Migration (§15): `panel.toggle()` → `plugin.panels.<slot>.toggle()`; `panel.visibility?.x` → `plugin.panels.<slot>.state?.x`; `panel.setVisible(v)` → `plugin.panels.<slot>.setVisible(v)`; `onVisibility` → `onChange`. Only playback (bottom bar) and playlist (sidebar) use them.

## 8. C8 UI state persistence

### 8.1 Host behaviour (no plugin interface)

File: `$VESTRA_UI_STATE`, when unset `~/Library/Application Support/Vestra/ui-state.json` (the same directory as the package store). Headless mode (`--headless`) neither reads nor writes it.

```json
{
  "version": 1,
  "window": { "x": 102, "y": 143, "width": 780, "height": 671 },
  "sidebar": { "width": 317 },
  "panels": {
    "vestra.playlist": { "visible": true, "detached": false,
                         "floating": { "x": 438, "y": 100, "width": 300, "height": 408 } }
  },
  "bindings": { "vestra.playback/toggle-pause": ["Space"] }
}
```

| Field | When read | When written |
| --- | --- | --- |
| `window` | Before the main window is created (logical pixels; before restoring, checks whether it is within the visible area of some screen, and if not uses the default position) | Size or position changes in normal mode (fullscreen is not recorded) |
| `sidebar.width` | When the main window is built | Divider released |
| `panels.<id>` | When that plugin mounts the sidebar, as the initial `preferred`, `detached`, floating window position | Preference, detached state, floating window position changes |
| `bindings` | Overlay loaded at startup (C3) | Not written this round, kept as is |

- Writing: changes are merged and written once 1 second after a change, and once more before exit; written to a temporary file first and then renamed, to avoid corrupting it.
- Read failure (not JSON, unknown `version`): log it, rename the original file to `ui-state.json.bad`, start with defaults. An invalid single field only ignores that field.
- Module: new file `src/host/src/ui/ui_state.rs` (`load() -> UiState`, `UiState::changed()`, `save_if_due(now)`); `save_if_due` is called in the 16 ms timer of the host main loop.

### 8.2 Replaced and deleted, migration

No old interface. Plugins need no changes. Note: the sidebar's visibility changes from "shown on every start" to "restore the last state"; this is the expected change of C8 (UI-PL-03).

## 9. C9 Window title

Host behaviour (no plugin interface): without a source the title is `Vestra Player`; with a source it is `<name> - Vestra Player`. Name: for a local file, the file name; for a network source, the `title` given by the resolver, or the URL if there is none; for a generated stream (emulator), `Vestra Player`. Updated in `shell.window()`'s title when the main loop receives a `source.state` change (`HostWindow` gains `in property <string> window-title`).

Delete: the old fixed development title in `host.slint`. Migration: none.

## 10. C10 Video info layer and playback statistics

### 10.1 Interface

```ts
interface OsdPanel extends Disposable {
  /** Shows a block of read-only multi-line text at the top left of the video; null hides it. The last write wins. */
  set(text: string | null): void;
}
interface Osd { panel(): OsdPanel }        // at most one per plugin; the second call throws

interface Player { stats(): Promise<PlaybackStats> }
interface PlaybackStats {
  backend: string;                 // e.g. "libmpv · software rendering"
  session: number;                 // the session from the old diagnostic line
  framesShown: number;             // number of frames the host has shown (the embedded frame from the old diagnostic line)
  framesDropped: number | null;
  video: { codec: string | null; width: number | null; height: number | null; fps: number | null } | null;
  audio: { codec: string | null; sampleRate: number | null; channels: number | null } | null;
}
```

Capabilities: `osd.panel` needs `ui.osd`; `stats` needs `playback.read`.

### 10.2 Host behaviour

- Info layer: top left of the video area, below the one-line on-screen message (y = 36px, x = 10px), semi-transparent black background `#000000b0`, padding 6px, `Theme.font-mono` 11px, `Theme.text` (from the 2026-10 visual identity round, the same rounded backing plate as the OSD with fixed light text, see UI spec UI-INFO-02); does not take input (no TouchArea), follows the video area, also shown in fullscreen. When several plugins each have a block, they are stacked top to bottom by owner generation.
- Text at most 32 lines, 4096 bytes; the SDK sends with a "last write wins" pump (one request in flight at a time), so refreshing once per second produces only one request. wire: `osd.panel { text: string | null }`.
- When the plugin is deactivated or the handle is `dispose`d, the host removes the block.
- `playback.stats`: forwarded to the backend worker thread, which reads `video-codec`, `video-params/w,h`, `estimated-vf-fps` (or `container-fps` if missing), `audio-codec-name`, `audio-params/samplerate`, `audio-params/channel-count`, `frame-drop-count` from libmpv, plus the host's own session and shown frame number. Without a source, `video` and `audio` are `null`.

### 10.3 Errors

Second call of `osd.panel`: the SDK throws synchronously (like "a plugin can only mount one panel"). `set` failures (`invalid` too long, `unavailable` no UI) go to the scope's `onError`. `stats`: `capability`, `backend` (worker thread stopped).

### 10.4 Playback info plugin (new `src/plugins/info/`)

manifest capabilities: `commands`, `playback.read`, `events`, `ui.osd`. One command:

```ts
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();
  let shown = false, timer: ReturnType<typeof setInterval> | undefined;
  const render = async () => {
    const s = await plugin.player.stats(), p = plugin.player.state;
    if (!shown) return;               // turned off while waiting
    panel.set([p.source ?? "No media open", `${clock(p.position)} / ${clock(p.duration)}`,
      `Video ${s.video?.codec ?? "-"} ${s.video?.width ?? "-"}×${s.video?.height ?? "-"} ${s.video?.fps ?? "-"} fps`,
      `Audio ${s.audio?.codec ?? "-"} ${s.audio?.sampleRate ?? "-"} Hz ${s.audio?.channels ?? "-"} channels`,
      `${s.backend} · session ${s.session} · frames ${s.framesShown} · dropped ${s.framesDropped ?? "-"}`].join("\n"));
  };
  plugin.commands.add({
    id: "info.toggle", path: ["View", "Playback Info"], order: 503, bindings: ["Tab"],
    checked: () => shown,
    run: () => {
      shown = !shown; clearInterval(timer);
      if (shown) { void render(); timer = setInterval(() => void render(), 1000); } else panel.set(null);
    },
  });
  plugin.onDispose(() => clearInterval(timer));
}
```

(`clock` is a time formatting function inside the plugin. Menu position see §18 Q10.)

### 10.5 Replaced and deleted, migration

Delete the host diagnostic text (see the C1 deletions). Existing plugins need no changes.

## 11. C11 Panel context menu

### 11.1 Interface

```ts
interface Panel {
  /**
   * Pops up a set of this plugin's commands (null = separator) in the window this panel is in, styled like the main menu.
   * at is in window coordinates (logical pixels); in Slint use self.absolute-position + the pointer position.
   */
  showMenu(items: (Command | null)[], at: { x: number; y: number }): Promise<void>;
}
```

### 11.2 Host behaviour

- wire: `ui.menu { items: (string | null)[], x, y }`, where `items` are this plugin's command ids. The host builds a **single-level** menu from the titles, states, check/radio marks in the registry and the shortcut text from the binding table, reusing `MenuView` (bindings paused while the menu is open, Esc closes, clicking outside closes). Choosing an item = the same execution path as the main menu (`resolve_execution` → `commands.execute`).
- It pops up in the window the panel is currently in: when attached, in the main window's menu layer; when detached, in the floating window's own menu layer (the window source generated by C14 contains the same `MenuView`). Only one menu at a time; a new one replaces the old one (including the main context menu).
- Submenus and context menus in tool windows are not done this round (§18 Q8).

### 11.3 Errors

`missing` (some id is not a command of this plugin, or has been released), `unavailable` (the panel is not currently shown), `invalid` (empty list, more than 32 items, non-finite coordinates).

### 11.4 Example

```ts
// sidebar.slint: TouchArea { pointer-event(e) => { if e.button == PointerEventButton.right && e.kind == PointerEventKind.down {
//   Bridge.context-menu(self.absolute-position.x / 1px + self.mouse-x / 1px, self.absolute-position.y / 1px + self.mouse-y / 1px); } } }
const play = plugin.commands.add({ id: "play", path: ["Playlist", "Play"], run: playSelected });
const remove = plugin.commands.add({ id: "remove", path: ["Playlist", "Remove Selected"], run: removeSelected });
const list = plugin.panels.mount("sidebar", "ui/sidebar.slint", { title: "Playlist" });
list.on("contextMenu", (x: number, y: number) => list.showMenu([play, null, remove], { x, y }));
```

### 11.5 Replaced and deleted, migration

No old interface. Plugins do not use it this round (the playlist context menu is step two, UI-PL-18).

## 12. C14 Panel detaching

### 12.1 Interface

`plugin.panels.sidebar.setDetached(detached)` and `SlotState.detached` (see C7); the `title` option of `panels.mount` is used as the floating window title (empty by default, the host shows `Vestra Player`). The plugin's panel handle, properties, and callbacks are **completely unchanged**.

### 12.2 Host behaviour: the same panel content

- The main window's source is unchanged (the panel component is still imported into the main window, and the `global` bridge object is in the main window instance). When detaching, the host **compiles a separate floating window** for the panel, with source generated by the host:

```slint
import { <component> as Owner7View, <global> as Owner7Bridge } from "<panel file>";
import { MenuView, MenuLevel } from "<host menu-view.slint>";
import { Theme } from "@vestra/theme.slint";
export { Owner7Bridge }
export component FloatingPanel inherits Window {
    title: "<title>"; min-width: 300px; min-height: 250px;
    background: Theme.bg-list;  // no default-font-family (system default font)
    // key-input / mouse-input / menu layer: the same host properties and callbacks as HostWindow
    keys := FocusScope { /* global + panel-scoped bindings */  Owner7View { width: parent.width; height: parent.height; } }
}
```

- A Slint global exists once per window instance, so the host writes the same properties to both: when the floating window is created it is initialised from `Mount.properties` (all set properties the host already keeps for rebuilding the main window); after that every `ui.setMount` writes both the main window and the floating window. Callbacks from both are forwarded as `$mount` to the same plugin, so the plugin does not notice the window changed. When attaching, the floating window is closed; the properties in the main window are already up to date, so the main window does not need to be rebuilt.
- Only in-out properties that the user changed in Slint but the plugin did not write back (e.g. scroll position, uncommitted text in an input box) are not kept across detach/attach — the same as today when another plugin's mount causes the main window to be rebuilt. Plugin state (list rows, selection) lives in the plugin and is not lost.
- Floating window: default position (438, 100), default size 300×408, minimum 300×250; position and size changes are recorded in C8. Clicking the window's close button = `setVisible(false)`, and it stays detached; when shown again it is still a floating window.
- While detached, `effective` only follows `preferred` (not affected by main window space or fullscreen); the sidebar slot in the main window is left empty and the video fills it.
- At startup, if persistence records it as detached and shown, the floating window is opened directly after the panel is mounted.
- Plugin deactivated: both instances are revoked together. When the main window is rebuilt (another plugin mounts), the floating window is not affected; the host repoints its callbacks to the new main window generation.

### 12.3 Errors

`missing` (no sidebar panel), `unavailable` (headless mode), `ui-prepare` (the floating window fails to compile; the state stays attached).

### 12.4 Example

```ts
// demo in src/examples: two commands trigger detach and attach (the button is added by playlist in step two)
const sidebar = plugin.panels.sidebar;
plugin.commands.add({ id: "detach", path: ["View", "Detach Sidebar"], order: 510,
  enabled: () => sidebar.state !== null && !sidebar.state.detached,
  run: () => sidebar.setDetached(true) });
plugin.commands.add({ id: "attach", path: ["View", "Attach Sidebar"], order: 511,
  enabled: () => sidebar.state?.detached ?? false,
  run: () => sidebar.setDetached(false) });
```

### 12.5 Replaced and deleted, migration

No old interface. In the reference migration, playlist's `mount` adds `title: "Playlist"` (no visible change while attached).

## 13. Lifecycle summary

| Resource | Owner | On plugin deactivation / dispose |
| --- | --- | --- |
| Command bindings | With the command | The owner's claims taken out after the host's `remove_owner`; when the SDK releases a command, the command becomes disabled (the binding still holds the key but does not execute, same as today) |
| Sidebar preference, detached state | Host (persisted by manifest id) | Panel unmounted; the state stays in the file for the next mount |
| Floating window | Host | Closed |
| Sequence provider | Provider plugin | Revoked; `can.next/previous` become false |
| Info layer | Plugin | Removed |
| On-screen message | None | Disappears on time |
| Panel context menu | Host | Closed if it belongs to that plugin |

## 14. Why the menu tree does not change after migration

The current order (`commands.rs` `snapshot()`): the top level follows the host domain table (File, Playback, Video, Audio, Subtitles, View, Emulator, Plugins); below that, the UTF-8 byte lexicographic order of `path[1..]`, with submenus and menu items interleaved and no separators. The new model only looks at `order`, and a submenu takes the `order` of its first child, so giving each command an `order` **equal to its current display position** reproduces the tree item by item:

- One range per old domain: File 200, Playback 260, Video 320, Audio 380, Subtitles 440, View 500, Emulator 560, Plugins 620 (all inside the recommended skeleton's `domain` range 200–699, so the redesign in step two does not need to move ranges);
- Within a range, numbered from +1 in the current display order (pre-order with all submenus expanded);
- No `group`: all default `"domain"`, so no separators appear;
- Top level: File(201) < Playback(261) < Video(321) < Audio(381) < Subtitles(441) < View(501) < Emulator(561) < Plugins(621), the same as the domain table order; the relative order inside each submenu is the same as today, and cross-plugin interleaving (audio and playback under Audio) is also guaranteed by the numbers.

| Plugin | Command id | Path (unchanged) | `order` | `bindings` |
| --- | --- | --- | --- | --- |
| playback | `close-source` | File / Close Current Source | 201 | |
| playback | `open` | File / Open Media… | 202 | |
| playlist | `playlist.previous` | File / Playlist / Previous Item | 203 | |
| playlist | `playlist.move-up` | File / Playlist / Move Up | 204 | |
| playlist | `playlist.next` | File / Playlist / Next Item | 205 | |
| playlist | `playlist.move-down` | File / Playlist / Move Down | 206 | |
| playlist | `playlist.play` | File / Playlist / Play Selected Item | 207 | |
| playlist | `playlist.add` | File / Playlist / Add Files… | 208 | |
| playlist | `playlist.link` | File / Playlist / Add Link… | 209 | |
| playlist | `playlist.clear` | File / Playlist / Clear List | 210 | |
| playlist | `playlist.remove` | File / Playlist / Remove Selected Item | 211 | |
| bilibili | `bilibili.open` | File / Network Media / Bilibili Video… | 212 | |
| youtube | `youtube.open` | File / Network Media / YouTube Video… | 213 | |
| playback | `forward-5` | Playback / Forward 5 s | 261 | `["ArrowRight"]` |
| playback | `backward-5` | Playback / Back 5 s | 262 | `["ArrowLeft"]` |
| playback | `toggle-pause` | Playback / Play/Pause | 263 | `["Space"]` |
| playback | `jump` | Playback / Jump To… | 264 | |
| playback | `rate-05` | Playback / Speed / 0.5× | 265 | |
| playback | `rate-15` | Playback / Speed / 1.5× | 266 | |
| playback | `rate-20` | Playback / Speed / 2× | 267 | |
| playback | `rate-10` | Playback / Speed / Normal Speed | 268 | |
| playback | `rate-custom` | Playback / Speed / Custom… | 269 | |
| capture | `capture.still` | Video / Capture and Export / Save Current Frame… | 321 | |
| capture | `capture.clip` | Video / Capture and Export / Export Clip… | 322 | |
| capture | `capture.tasks` | Video / Capture and Export / View Export Tasks… | 323 | |
| capture | `capture.burst` | Video / Capture and Export / Burst Capture… | 324 | |
| video | `video.picture` | Video / Picture Adjustment… | 325 | |
| video | `video.gaussian` | Video / Gaussian Blur… | 326 | |
| audio | `audio.preset.1` | Audio / Equalizer Presets / Vocal | 381 | |
| audio | `audio.preset.0` | Audio / Equalizer Presets / Flat | 382 | |
| audio | `audio.preset.3` | Audio / Equalizer Presets / Cinema | 383 | |
| audio | `audio.preset.2` | Audio / Equalizer Presets / Music | 384 | |
| audio | `audio.reset` | Audio / Reset Audio Processing | 385 | |
| playback | `mute` | Audio / Mute | 386 | |
| playback | `volume` | Audio / Volume… | 387 | |
| audio | `audio.workbench` | Audio / Audio Workbench… | 388 | |
| subtitles | `disable` | Subtitles / Turn Off Subtitles | 441 | |
| subtitles | `refresh` | Subtitles / Refresh Subtitle Tracks | 442 | |
| subtitles | `load` | Subtitles / Load External SRT… | 443 | |
| subtitles | `workbench` | Subtitles / Subtitle Workbench… | 444 | |
| playlist | `playlist.visibility` | View / Show Playlist | 501 | |
| playback | `bar-visible` | View / Show Playback Bar | 502 | |
| nes | `nes.window` | Emulator / NES Tools | 561 | |
| nes | `nes.stop` | Emulator / Stop NES | 562 | |
| nes | `nes.open` | Emulator / Open NES ROM… | 563 | |
| workshop | `workshop.open` | Plugins / Plugin Workshop… | 621 | |

New items (not part of the migration, expected additions this round): the playback info plugin's "View / Playback Info" 503; the host's "Fullscreen" 700 and "Quit" 990 (§18 Q2); shortcut text appears on the right of each main menu row (C4).

**Verification** (added by the maintainer in P1b, reviewed by the coordinator):

1. `commands.rs` unit test: register the 46 commands from the table above (in shuffled registration order), and assert that the pre-order of the expanded menu tree is exactly the same as the result of today's `snapshot()` (domain table + byte order).
2. Snapshot script `menus-snapshot.test.ts`: `shortcut` changes to reading `declaration.bindings` (compared with `null` ↔ `[]`, `"Space"` ↔ `["Space"]`), and also outputs `order`; `menus-after.json` is compared with `menus-before.json` on path, title, and shortcut, and `order` is then compared with the table above. The existing snapshot does not contain order, so test 1 is the main evidence that the order is unchanged.

## 15. Migration rewrites (for migrators to copy)

### 15.1 Commands: `shortcut` → `bindings`, add `order`

```ts
// old
plugin.commands.add({ id: "toggle-pause", path: ["Playback", "Play/Pause"], shortcut: "Space", run });
// new
plugin.commands.add({ id: "toggle-pause", path: ["Playback", "Play/Pause"], order: 263, bindings: ["Space"], run });
```

Commands without `shortcut` only add `order` (table in §14). Plugins that register from a table (playback `commands.ts`, playlist `main.ts`, audio presets) add an `order` column to the table.

### 15.2 Panel visibility → slot state

```ts
// old (playback)
entry("bar-visible", ["View", "Show Playback Bar"], () => bar.toggle(), () => ready(true, bar.visibility?.preferred ?? true))
// new
const bottom = plugin.panels.bottom;
entry("bar-visible", ["View", "Show Playback Bar"], () => bottom.toggle(), () => ready(true, bottom.state?.preferred ?? true))

// old (playlist)
checked: () => (name === "visibility" ? (list.visibility?.preferred ?? true) : null),
reason: () => name === "visibility" && list.visibility?.reason === "space" ? "Not enough space; enlarge the window to show it" : …,
case "visibility": await list.toggle(); break;
list.on("hide", () => list.setVisible(false));
// new
const sidebar = plugin.panels.sidebar;
checked: () => (name === "visibility" ? (sidebar.state?.preferred ?? true) : null),
reason: () => name === "visibility" && sidebar.state?.reason === "space" ? "Not enough space; enlarge the window to show it" : …,
case "visibility": await sidebar.toggle(); break;
list.on("hide", () => sidebar.setVisible(false));
```

`setInteraction` is unchanged.

### 15.3 Tests

```ts
// old
assert.deepEqual(host.command("workshop.open").declaration, { title: "Plugin Workshop…", path: ["Plugins", "Plugin Workshop…"], shortcut: null });
// new (order is written, so the test host receives the full shape; unwritten fields have default values)
assert.deepEqual(host.command("workshop.open").declaration, {
  title: "Plugin Workshop…", path: ["Plugins", "Plugin Workshop…"], order: 621, group: "domain",
  radio: false, bindings: [], focus: null, repeat: false });
```

New in the test host: `host.press("Space", { panel? })` (executes a command through the binding table, rejected in the same way when disabled; `panel: true` simulates the panel having focus), `host.conflicts`, `host.layout` (state of both slots; after changing it, `host.emit("window.state", {})` simulates a change from elsewhere), `host.osd` (messages shown, newest last), `host.osdPanel`, `host.sequence` (flags of the current provider) and `host.step("next")`, `host.stats` (the return value of `player.stats()`), `host.menus` (records of `showMenu`). `host.mounted().visibility` is deleted in P4; after migration read `host.layout` instead.

### 15.4 Per-plugin list (P3)

| Plugin | What to change |
| --- | --- |
| playback (maintainer reference migration) | Add `order` to 14 commands; 3 × `shortcut` → `bindings`; `bar.toggle()`/`bar.visibility` → `plugin.panels.bottom` |
| playlist (maintainer reference migration) | Add `order` to 10 commands; sidebar visibility → `plugin.panels.sidebar`; `mount` adds `title: "Playlist"`; register `provideSequence` (reusing the existing next/previous logic) |
| video, audio, subtitles, capture, nes, youtube, bilibili | Add `order` to every command; change the `declaration` assertions in tests to the new shape |
| workshop | `order: 621`; test assertions; copy `vendor/sdk` again from the new SDK (`tests/vendor-sdk.test.mjs` requires byte-for-byte equality, the command is at the top of that test file; from P1b this test fails until the copy is made) |
| examples/bookmark (maintainer) | "Playback / Bookmarks…" is an example, not in the snapshot; add `order: 270` |

## 16. Deletion list (P4; after it is all done these no longer appear in the code)

Host (`src/host/`):

- `commands.rs`: `Declaration.shortcut`; `canvas_shortcut()`; `Registry::resolve_shortcut()`; the domain parameter of `Registry::new` and `domains`, `domain_order`; the "2..=6 segments", "menu domain is not allowed", "main-canvas shortcut already registered" checks; the domain ranking and alphabetical `path[1..]` sort in `snapshot()`; rewrite the matching old tests.
- `main.rs`: the 8 domain names in Host initialisation; `Host::canvas_shortcut`; the `shortcut:`, `toggleFullscreen`, `escape` branches of `Event::WindowInput`; the routes and capability table entries of `ui.mountState`, `ui.showMount`; the `scope: "presentation"` branch; rewriting `path[1]` and clearing `shortcut` on trial load; the two writes of `diagnostic`; `sidebar_resize_requested`, `sidebar_resize` and their calculation (if §18 Q7 passes); `"F11" | "Escape"` in the producer hook.
- `window_state.rs`: `reveal_sidebar_in_fullscreen`, `sidebar_override`; `CANVAS_MIN_WIDTH = 640`, `CANVAS_MIN_HEIGHT = 360`, `SIDEBAR_MIN_WIDTH = 240`, `SIDEBAR_TARGET_WIDTH = 280` and the "hide directly when there is not enough space" rule (replaced by 318/3/317).
- `ui.rs`, `composition.rs`: the `Arial Unicode.ttf` import, `"Arial Unicode MS"`; the three separate `Compiler::default()` calls (merged into one).
- `host.slint`: the Commands button row and explanatory text; the `diagnostic` property and text line; `padding: 8px`; `default-font-family: "Arial Unicode MS"`; the hard-coded F11/Escape/Space/←/→ in the canvas `key-pressed`; 1100×720, 720×480; the fixed title; hard-coded colours such as `#111318`.
- `menu-view.slint`: the old colour scheme, rounded corners, text ✓.

SDK and runner (`src/sdk/`):

- `plugin/types.ts`: `CommandOptions.shortcut`; `Panel.visibility`, `onVisibility`, `setVisible`, `toggle`; `PanelVisibility`.
- `plugin/ui.ts`: the visibility reading and toggling code of `PanelHandle` (moved to module-level slot state).
- `commands.ts`: `CommandDeclaration.shortcut`.
- `wire.ts`: `ui.showMount`, `ui.mountState`, `MountVisibility`.
- `plugin/testing.ts`: the `ui.mountState` fake, `FakeCommand.declaration.shortcut`, the panel visibility fake.

Docs: the `shortcut` description and "top-level grouping convention" in `doc/plugins/reference.md` §4, panel visibility in §6.3; the `commands` and `panels` rows of the table in `doc/architecture/sdk-design.md` §2; the font sentence in `doc/architecture/README.md` §4.

Plugins: every `shortcut:`, `.visibility`, `.onVisibility`, `panel.setVisible`, `panel.toggle()`; the old copy of workshop's `vendor/sdk`.

## 17. Error code summary (new uses)

| code | Appears in |
| --- | --- |
| `invalid` | Binding syntax, invalid path/`order`/`group` (`commands.add`); message/info layer text empty or too long; `showMenu` parameters; setting `detached` on the bottom bar |
| `conflict` | Another sequence provider already exists; duplicate path (unchanged). **Binding conflicts do not raise errors**, see §3.5 |
| `unavailable` | `next`/`previous` without a provider or unable to go forward/back; messages, info layer, slot state, detaching in headless mode; `showMenu` while the panel is not shown |
| `missing` | No panel in the slot; a `showMenu` command does not belong to this plugin or has been released |
| `pending` | `layout.state` while the layout is not confirmed (handled inside the SDK, plugins do not see it) |
| `ui-prepare` | Floating window fails to compile |
| `capability` | Calling `plugin.osd.*` without declaring `ui.osd`; otherwise the existing rules |

## 18. P2 rulings (2026-10-20)

The coordinator's review passed. The table below is the final decision (the original questions and options are in the P1a version in git history).

| # | Question | Ruling |
| --- | --- | --- |
| Q1 | Floating window default size | 300×408 (minimum 300×250) |
| Q2 | Host "Fullscreen" and "Quit" in the top-level menu | Yes (`screen` group 700, `exit` group 990); P1b confirms ⌘Q does not duplicate the macOS default app menu |
| Q3 | Range of mouse events | No left single click, long press, or right double-click; `Mouse:LeftDoubleClick` is the default binding for host fullscreen (until 0.5.1; from 0.5.2 a left click is Play/Pause and a double click is unbound, user ruling 2026-10-26) |
| Q4 | Binding conflicts between plugins | First loaded wins (§3.5) |
| Q5 | Focus scopes | Only `panel` |
| Q6 | Repeat when held | Commands can declare `repeat: true`, default false (§3.1, §3.2) |
| Q7 | Automatic window enlargement | Deleted |
| Q8 | Panel context menu | Single level, panels only |
| Q9 | Omitting `path` | Allowed; not in the main menu, `title` required (§4.1, §4.2) |
| Q10 | Menu position of playback info | "View → Playback Info" |
| Q11 | Shortcut text format | `Ctrl+O`, arrows |
| Q12 | Default font | In the tested order; when it can only be imported by file, use the system's Hiragino Sans GB |
| Q13 | Theme widget library | `@vestra/widgets.slint` provides `Slider`, `Button`, `ToolTipArea` (§2.1); no other controls |
| Q14 | Floating panel in fullscreen | Known limitation |

## 19. P1b implementation record (2026-10-20)

Differences from the text above and facts found during implementation, to be verified in P5 as needed.

- **Code locations**: new host files `bindings.rs` (binding table), `ui_state.rs` (persistence), `ui/vestra/theme.slint`, `ui/vestra/widgets.slint`; `commands.rs` rewritten for the new menu model (including the order test for the 46 commands of §14); `menu.rs` gains separators, radio items, the shortcut column and single-level panel menus; `host.slint` rewritten as the shell; `composition.rs` generates the sidebar divider, the key forwarding layer and the floating window source. New SDK files `sdk/plugin/bindings.ts` (syntax check with the same rules as the host), `sdk/plugin/osd.ts`. New plugin `src/plugins/info/`; demos `src/examples/core-ui/` (menu groups and radio items, `repeat` bindings, commands not in the menu, on-screen messages, `next/previous` calls, sidebar toggle and detach, a theme widget window) and `src/examples/panel-menu/` (panel context menu, panel-focus keys, sequence provider, floating window title).
- **⌘Q**: on macOS the winit backend of Slint 1.16 builds the app menu by default (muda, including the system "Quit ⌘Q", `i-slint-backend-winit` `muda.rs` `create_default_app_menu`); ⌘Q is handled first by the system menu and does not reach the window, so it does not fire twice together with the host's `Cmd+Q` binding; on macOS the host binding only provides the shortcut text in the menu. When quitting through the system menu the host does not go through its own quit path, so UI state can lose at most the changes from the last 1 second that were not yet written (the context menu's "Quit" writes first). Needs P5 testing.
- **Theme tokens**: `Theme.font-family` is not provided (each window uses the system default font, §2.2); `font-mono` is supplied by the host per system (macOS `Menlo`, Windows `Consolas`, Linux `DejaVu Sans Mono`, 2026-10-22).
- **wire methods**: `playback.stats` reads libmpv on the backend thread; `layout.set` runs one layout pass before replying, so an immediately following `layout.state` sees the result. Host error codes of `ui.menu` follow §11.3.
- **Wheel step**: one step per 40 logical pixels (`WHEEL_STEP`); needs adjusting after testing with trackpad and mouse.
- **Deletions done early** (because the shell was replaced, the old code had no callers): the hard-coded Space/←/→/F11/Esc routing in `host.slint`, `Host::canvas_shortcut`, the `shortcut:`/`toggleFullscreen`/`escape` branches of `WindowInput`, `Registry::resolve_shortcut`, the domain list, `diagnostic` and the state snapshot the backend kept for it, `sidebar_resize*` (Q7), the Arial Unicode font. The other items in §16 (the `shortcut` field, the old panel visibility interface and wire, `reveal_sidebar_in_fullscreen`, old test host fields) are left for P4.
- **Menu order during migration**: commands of unmigrated plugins have `order` 0, so until P3 is done they appear before migrated commands in the GUI; the per-plugin menu snapshots are not affected.

### P4 deletions (2026-10-20)

The list in §16 is fully done, and the coexistence code of the migration period was deleted with it:

- SDK: `CommandOptions.shortcut`; `LegacyCommandDeclaration` and the old declaration shape in `sdk/commands.ts` (the SDK now always sends the full declaration, unwritten fields have default values); `Panel.visibility`, `onVisibility`, `setVisible`, `toggle` and the type `PanelVisibility`, and the code in `PanelHandle` that read `ui.mountState` and subscribed to `window.state` (after a panel is mounted it now refreshes the state of `panels.<slot>`); wire `ui.showMount`, `ui.mountState`, `MountVisibility`. Test host: the `ui.showMount`, `ui.mountState` fakes, `visibility` of the fake panel object, the compatibility branch that looks up bindings by `shortcut`; `FakeCommand.declaration` only has the full shape.
- Host: `commands::Declaration.shortcut` and `all_bindings()`; the routes and capability table entries of `ui.mountState`, `ui.showMount` (including `scope: "presentation"`); clearing `shortcut` on trial load; `reveal_sidebar_in_fullscreen`, `sidebar_override` in `window_state` (in fullscreen an attached sidebar is always hidden; floating windows are not affected). The shortcut text field in menu rows is renamed `keys` (to distinguish it from the deleted `shortcut` field; meaning unchanged).
- Tests: SDK tests and playback tests read the full declaration and `host.layout`; the fullscreen override test of `window_state` is replaced by "fullscreen hides the attached sidebar, keeps the preference and the floating window".
- Docs: `doc/plugins/reference.md` drops two migration-period notes; the deleted interfaces no longer appear in `doc/plugins/`. Workshop's `vendor/sdk` was resynced with the command at the top of its test (authorised by the coordinator).


## 20. Interface changes for step two (2026-10-20, on request)

| Request | Decision | Change |
| --- | --- | --- |
| R1 Playlist: read durations without opening files | Rejected | The plugin calls ffprobe itself (`VESTRA_FFPROBE` → same directory as `VESTRA_FFMPEG` → PATH), with limited concurrency, timeout, cancellable, local files only; on failure it shows `--:--` |
| R2 Controls: Path icons, slider callbacks | Button accepted, Slider rejected | `Button.icon` changes from `image` to Path data, adding `icon-width`, `icon-height` (default 14×12, also the viewbox) and `stroked`; the original `image` icon had no users. The bottom-bar-specific buttons and the progress bar stay inside the playback plugin |
| R3 Change title / hide menu items | Only "release means delete" | wire `commands.unregister`; `Command.dispose()` removes it from the menu and the binding table, and the id can be reused (the SDK orders a re-registration with the same id after the deletion). Variable lists are released with a child scope and rebuilt; no mutable `title` |
| R4 Per-plugin data directory | Accepted | `plugin.dataDirectory`: `plugin-data/<manifest id>/`, created by the host before starting the plugin and passed to the runner through `VESTRA_PLUGIN_DATA_DIR`; trial loads use a separate id, deleted on stop; reading without a directory throws `unavailable`; the test host provides a temporary directory, and `startPlugin({ dataDirectory })` can reuse a directory |
