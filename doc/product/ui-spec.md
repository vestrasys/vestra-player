# Interface specification

This document specifies the Vestra Player interface: sizes, colors, geometry and basic behavior. It is for API maintainers and plugin authors. Interaction details (states, timing, behavior in each mode) are in the [Interaction specification](interaction-spec.md); where the two conflict, the interaction specification wins. The complete menu tree is in [Menus and interface ownership](menus.md); the feature scope is in [Feature requirements](feature-requirements.md). Changes follow the [community workflow](../../agentic-workflow/agentic-workflow.md).

## 0. How to read this

- **IDs**: `UI-<area>-<number>`, for example `UI-BAR-03`. Code comments and other documents cite this document by ID and section number; IDs are never changed or reused. The owner is written as `host` (the Rust host and `host.slint`), `SDK` (the plugin API; core capabilities are collected in §10), or a plugin name.
- **Units**: Slint logical pixels. On macOS 1 logical pixel = 1 point; on Windows it is a 1x pixel at 96 DPI, scaled with the system scaling.
- **Colors**: given as theme token names (§2.1), with the value in the default palette `vestra` in parentheses. Plugin `.slint` files use only tokens and never write color values.
- **Style code**: written as CSS so it can be compared item by item with Slint properties (`height` ↔ `height`, `background` ↔ `background`, `gap` ↔ `spacing`, `padding` ↔ `padding`).
- **Title bar**: the main window uses the system title bar (macOS keeps the red, yellow and green buttons). The plan for a custom-drawn title bar is in interaction specification IA-CORE-07. For shortcuts, Ctrl also accepts Cmd on macOS.

## 1. Design decisions

| # | Question | Decision |
| --- | --- | --- |
| D1 | Visual identity | Direction A "V-Line × Aurora" (chosen by the user on 2026-10-24, [proposal](identity/proposal.md)): the palette takes the full spectrum of the logo (`src/assets/brand/logo.png`), magenta accent `#f72aa6`, cyan for "now playing", progress as an amber→magenta→purple→cyan gradient; dark `vestra` by default, light `vestra-light` optional (the video area stays dark); icons are V-Line round-capped strokes (§2.5); corner radii 4/8px (§2) |
| D2 | Tools for adjusting picture, audio and subtitles | Three separate small plugin windows, all following the panel page specification in §7.1; no container that gathers them into one window (C12) |
| D3 | Screenshots | "Save Current Frame" Ctrl+E saves directly to the snapshot folder and shows an on-screen message, with no confirmation; only "Save As…" opens a save dialog. Burst capture and Export Clip are separate dialogs |
| D4 | Menu entry point | No menu button inside the main window; the menu is opened only by right-click; no macOS top menu bar |
| D5 | Debug information | Not shown permanently. The "Playback Info" plugin toggles an information layer over the video with Tab, off by default (§9.2) |
| D6 | Playlist docking | Attached to the right side of the main window by default; can be detached into a floating window and attached back (C14) |

## 2. Global visual rules (provided by the host, used by all plugins)

### 2.1 Color table

The host has two palettes, both called Aurora and taken from the full spectrum of the logo: `vestra` (dark, default) and `vestra-light` (light, added on 2026-10-24). The host sends the colors to every view (main window shell and panels, floating sidebar, tool windows, options window); plugins read them through `Theme` in `@vestra/theme.slint`; the colors in `Theme` are read-only `out` properties. The user switches under "Settings ▸ Appearance ▸ Vestra Dark / Vestra Light" or on the "Appearance" page of the options window; all windows change at once, and the choice is saved across launches.

The color values of both palettes are on the SDK documentation site, [Reference: Theme → Colours](https://vestrasys.github.io/vestra-player-sdk/documents/Theme.html#colours) (SDK `THEME_PALETTES`, checked against the host's `theme.rs` by host-agreement); the table below only records how the player interface uses each token.

| token | Use |
| --- | --- |
| `bg-bar` | Control bar, tab strip, window background |
| `bg-video` | Video area, idle screen (dark in both palettes) |
| `bg-list` | Playlist content area, floating panels, options tree |
| `bg-panel` | Tool pages, list bottom bar, menus, tooltips |
| `line` | 1px separators and outer borders, seek bar and slider tracks, menu separators, sidebar splitter |
| `hover-bg` | Hover background (the rounded plate of bottom-bar buttons, tabs); search box background |
| `press-bg` | Pressed background |
| `selected-bg` | Selected list row, current menu item, selected options tree item |
| `text` | Main text, icons in normal state, panel slider thumb |
| `text-dim` | Secondary text, small icons in normal state, total duration, shortcut text |
| `text-list` | List text, button text, menu text |
| `text-played` | Played items in the list |
| `accent` | Hovered icons, on state, checkboxes, bookmarks, current-item bar, slider outer ring |
| `accent-press` | Pressed accent |
| `accent-disabled` | Disabled accent |
| `icon-disabled` | Disabled icons, disabled sliders |
| `playing` | The list item that is playing and its left bar; A-B loop range and marks |
| `button-bg` | Small buttons and labels inside panels |
| `button-hover` |  |
| `button-press` |  |
| `button-disabled` |  |
| `danger` | Failure state, delete-type buttons |
| `accent-fill` | Progress fills (played part of the seek bar, volume, sliders), the short gradient line under tabs and in the OSD |

Contrast (WCAG): dark `text`/`bg-bar` 16.6:1, `accent`/`bg-bar` 5.1:1, `playing`/`bg-list` 7.7:1; light `text`/`bg-bar` 16.1:1, `accent`/`bg-bar` 4.6:1, `playing`/`bg-list` 5.0:1.

Fixed colors (independent of the palette):

- Things on the video area (on-screen messages, information layer, §9): white text `#ffffff` / `#f5f3fa`, translucent dark plate `#0b0a10c4`. The video area is dark in both palettes, so `Theme.text` is not used here (it is dark text in the light palette).
- Check marks on an accent background (menu checkboxes, checkboxes, task lists) are white `#ffffff`; the core of the bottom-bar slider thumb is white.

std-widgets controls (`LineEdit`, `ComboBox`, `CheckBox`, the scroll bars of `ScrollView` and `ListView`, etc.): the host compiles all Slint with the `fluent-dark` style. When the light palette is chosen, it sets the std-widgets `Palette.color-scheme` to `light` (`VestraPalette.color-scheme` in `ui/palette.slint`), and these controls become fluent light; switching back to dark restores them, with no recompilation. Remaining difference: fluent controls use their own neutral grays and blue accent (focus underline, checkboxes), not the Aurora colors, in both palettes.

### 2.2 Fonts

| Use | Specification |
| --- | --- |
| Default interface font | The system UI font of each platform (for Chinese: PingFang SC on macOS, Microsoft YaHei on Windows); the host does not set a font family; missing glyphs fall back per character in Slint (user ruling, 2026-10-22) |
| Monospace font (`Theme.font-mono`) | Set by the host per platform: Menlo on macOS, Consolas on Windows, DejaVu Sans Mono on Linux |
| Time, badges | 11px, tabular digits (`font-feature: tnum`) |
| Font size steps | Only these steps: 9 (`font-xs`, very small labels), 11 (`font-body`, body text, lists, time), 12 (`font-tab`, tabs, menus), 13 (`font-title`, window titles, tab titles) |

### 2.3 Control sizes

```css
button          { height: 21px; min-width: 66px; padding: 0 8px; }   /* normal dialog button; preferred width = max(min width, text width + padding) */
button.primary  { height: 24px; }                                    /* main action button at the bottom */
button.small    { height: 18px; min-width: 41px; }                   /* "Default" button inside panels */
input, select   { height: 21px; }
checkbox-row    { height: 18px; }
label           { height: 14px; }                                    /* labels in the left form column are right-aligned */
.form-row       { gap: 26px; }
.dialog         { padding: 14px; }
.control        { border-radius: 4px; }                              /* radius-control: buttons, menu items, tooltips, tab hover background, panel slider thumb */
.panel          { border-radius: 8px; }                              /* radius-panel: menus, overlays, hover plate of bottom-bar buttons, OSD plate */
```

The widget library `@vestra/widgets.slint` provides `Button`, `Slider`, `ToolTipArea` and others, sized by the tokens above; icons are in `@vestra/icons.slint` (§2.5).

**Key hints** (SDK 1.6.6, player 0.7.6; menu redesign §3.5). They always show the current binding: the user's keys, nothing when cleared, only the user's keys under Ignore Default Shortcuts, ⌃/⌘ on macOS.

- **KeyHint** draws one key cap.
  - Text: the label from `plugin.commands.keys(id)`, formatted as in the menus (`Ctrl+B`, `⌘B`), in `Theme.font-mono` at `Theme.font-xs`, colour `Theme.text-dim`.
  - Box: 1px `Theme.icon-disabled` border, radius 3px, `Theme.bg-list` fill, height 15px, width max(16px, text + 8px).
  - Empty text: nothing is drawn and no space is taken.
  - Placement: at the right end of the control's row, the minus key before the plus key (`W` `E`), 2px apart.
- **`ToolTipArea.keys` and `.detail`**: the tip reads `Title  [K1] −  [K2] +  · detail`.
  - Caps are drawn as KeyHint, with 10px between entries and 4px between a cap and its text; `detail` is in `Theme.text-dim`.
  - An entry with an empty key is left out.
  - Tip elements change only with the properties, never on hover (the AccessKit rule).

### 2.4 Theme provided by the host

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-THEME-01 | Provide an importable theme global `Theme` (all tokens in §2.1–2.3); plugin `.slint` files read values with `import { Theme } from "@vestra/theme.slint"` and do not write color values (except in the theme file) | host + SDK (C2) |
| UI-THEME-02 | All plugin windows follow the palette: the widget library is colored by `Theme`; std-widgets are fluent dark under the dark palette and are switched to fluent light by the host under the light palette (§2.1); the video area is dark in both palettes | host (C2) |
| UI-THEME-03 | The default font is the system UI font, CJK falls back per character, mixed Chinese and English text shows no tofu boxes | host |

### 2.5 Icons (V-Line)

All icons come from `@vestra/icons.slint` (host widget library; the SDK carries a read-only copy since 1.5): `Icon { name: "play"; color: Theme.text; }`. The source file is [icons-v-line.json](identity/icons-v-line.json).

- **Strokes**: stroke only, no fill; round caps (`round` cap) and round joins (`round` join); stroke width 1.5px, 1.25px for small icons up to 12px (sort arrows 1.1px).
- **V shape**: the V of the logo runs through the whole set: rewind and fast forward are « », move up and move down are ∧ ∨, move to top and move to bottom are ∧ ∨ with a bar, the submenu arrow, dropdown arrow, scroll arrows and sort arrows are all small Vs, and the third line of the playlist toggle ends with a small V.
- **Drawing box**: each icon has its own drawing box (`Icons.box-width` × `Icons.box-height`), which is its default size; it is exactly the same as the old icon it replaced, so button and row layouts do not change. Strokes stay at least half a stroke width inside the box edge and never leave the box.
- **Color**: the icon color is given by the caller (`color`), with the same rules as the old icons: normal `text` or `text-dim`, hover/on `accent`, pressed `accent-press`, disabled `icon-disabled`; check marks on an accent background are white.
- **No font characters as icons**: ✓ › ▲ ▼ ▾ × ▶ have all been replaced by icons (different systems draw them differently in their fonts).
- **List**: icon names and drawing boxes are on the SDK documentation site, [Reference: Theme → Icons](https://vestrasys.github.io/vestra-player-sdk/documents/Theme.html#icons).

## 3. Main window shell (host)

Layout: the attached panel on the right (playlist) takes the full window height, with a 3px draggable splitter between it and the video; the 62px (22 + 40) bottom control bar is **only under the video**; it does not extend under the playlist and does not cover the picture (except in fullscreen). The video is flush with the edges, with no gap around it.

```css
.window   { width: 780px; height: 671px; min-width: 320px; min-height: 182px; background: var(--bg-video); }
.video    { flex: 1; min-width: 318px; background: var(--bg-video); }
.bottom   { height: 62px; under: video-only; }
.splitter { width: 3px; background: var(--line); cursor: ew-resize; }
.sidebar  { width: 317px; min-width: 317px; full-height: true; }      /* default width 317, remembered after dragging */
.floating-sidebar { width: 300px; height: 408px; min-width: 300px; min-height: 250px; }
```

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-SHELL-01 | No controls above the video (no command buttons, no description text line) | host |
| UI-SHELL-02 | No permanent diagnostic text; playback information is shown on demand by the Playback Info plugin (§9.2) | host |
| UI-SHELL-03 | Video flush with the edges, background `bg-video` | host |
| UI-SHELL-04 | Default window 780×671 (content area), minimum 320×182 (for height changes see interaction specification IA-CORE-13) | host |
| UI-SHELL-05 | The sidebar takes the full height to the right of the video, and the bottom bar is only under the video: with the playlist open, the bottom edge of the list is aligned with the bottom edge of the window, and the left and right ends of the bottom bar are aligned with the video | host (composition) |
| UI-SHELL-06 | 3px draggable splitter (`line`) between sidebar and video; video at least 318px, sidebar at least 317px; the dragged width is saved across launches | host |
| UI-SHELL-07 | Default sidebar width 317px | host |
| UI-SHELL-08 | When the window is too narrow for video 318px + splitter + sidebar 317px, the user's display choice is kept: the video is first shrunk to 318px, and only if that is not enough the sidebar is hidden; the menu check stays at "shown" | host |
| UI-SHELL-09 | Fullscreen: the control bar covers the bottom of the video, shown only while the pointer is in its band at the bottom edge and hidden 500 ms after it leaves (FS-BAR-1); during playback the mouse pointer is hidden as well ("Playback ▸ Hide Pointer While Playing", on by default). How the playlist pops up in fullscreen is in interaction specification G-01 | host |

## 4. Bottom control bar (basic playback plugin playback)

Two rows, 62px high in total, all icon buttons, no text buttons.

```css
/* seek bar row */
.seek-row  { height: 22px; background: var(--bg-bar); border-top: 1px solid var(--line); /* only one line, on the top edge */
             padding: 0 10px 0 5px; }
.seekbar   { flex: 1; height: 10px; margin: 0 10px; }            /* 10px hit area */
.seekbar .track  { height: 2px; border-radius: 1px; background: var(--line); }
.seekbar .played { height: 2px; border-radius: 1px; background: var(--accent-fill); } /* icon-disabled when not seekable */
.seekbar .thumb  { width: 8px; height: 8px; border-radius: 50%; background: #ffffff; border: 2px solid var(--accent); } /* ring is accent-press when pressed */
.seekbar .mark   { width: 1px; height: 6px; }   /* chapters text-dim, bookmarks accent, marks playing; A-B loop is a 2px playing band with 1×8 ends */
.mute-btn  { width: 13px; height: 10px; color: var(--text-dim); margin-left: 10px; } /* hover accent, pressed accent-press */
.volume    { width: 70px; height: 10px; margin-left: 4px; }      /* 2px line, accent-fill fill, same thumb (white core + 2px accent ring) */

/* button row */
.button-row { height: 40px; background: var(--bg-bar); }
.big-btn    { width: 41px; height: 40px; color: var(--text); }     /* no separators between buttons; icon centered in the left 40px */
.big-btn::hover-plate { left: 4px; top: 4px; width: 32px; height: 32px; border-radius: 8px; }
            /* rounded plate hover-bg on hover, press-bg when pressed; disabled icon icon-disabled */
/* left→right: play/pause (one of the two by state) | stop | back (right-click: previous) | forward (right-click: next) | open | fullscreen (enter or leave by state) */
.time      { margin-left: 10px; font-size: 11px; }  /* "00:12" text + " / 15:00" text-dim; only the current time is clickable, it toggles elapsed/remaining */
/* right end: playlist toggle */
.icon-btn  { width: 23px; height: 22px; color: var(--text-dim); margin-right: 8px; } /* accent on hover or when on, pressed accent-press */
```

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-BAR-01 | Two rows: seek bar row 22px + button row 40px, background `bg-bar`, only one 1px `line` on the top edge (separating it from the video), total height fixed at 62px | playback |
| UI-BAR-02 | The bottom bar shows no source path and no status text. The source name is in the window title (C9); status goes into on-screen messages (C5) | playback + host |
| UI-BAR-03 | Seek bar as in the CSS above: a 2px round-capped line in a 10px hit area, unplayed `line`, played `accent-fill`; 8px round thumb, white core + 2px `accent` ring, ring `accent-press` when pressed | playback |
| UI-BAR-04 | Mute icon 13×10 + volume bar 70×10 at the right end of the seek bar row; the volume bar is fixed at 70px and does not stretch | playback |
| UI-BAR-05 | Six 41×40 icon buttons on the left of the button row: play/pause, stop (close the source), back, forward, open, fullscreen; right-click on back/forward = previous/next. Fullscreen (2026-10-26, user request) toggles the main window's fullscreen like its key, Esc (`host.run("fullscreen")`, player 0.5.2; Enter/F11 up to 0.5.x): icon `fullscreen` when windowed and `exit-fullscreen` in fullscreen (`window.state`) | playback (previous/next through C6; fullscreen through `host.run`) |
| UI-BAR-06 | Icons from `@vestra/icons.slint` (§2.5); normal `text`, disabled `icon-disabled` (the command strip at the right end is the exception: `accent`, see UI-BAR-09); hover/press lights a 32×32 rounded plate in the middle of the button; no separators between buttons | playback |
| UI-BAR-07 | The time is a text label `current / total`: current in `text`, the slash and total in `text-dim`, 11px, 10px after the last shown button (fullscreen, or open while fullscreen is hidden, UI-BAR-10); clicking the current time toggles between "elapsed / remaining" | playback |
| UI-BAR-08 | The bottom bar has no speed button and no inline edit field. Speed is set through the menu and Z/X/C; seeking to a time through the "Jump to Time…" dialog (G, §7.5); volume through the slider, ↑/↓, M | playback |
| UI-BAR-09 | The right end of the button row is the player's command strip (`BarButtons` from `@vestra/bar-buttons.slint`, player 0.5.10): 41×40 cells like the bar's own buttons, the plugins' contributed buttons (`commands.add({ button: { icon, order } })`, ascending `order` left to right, at most 1 per plugin and 8 in all) and, rightmost, the player's sidebar toggle (host command `host/sidebar`, "Toggle Sidebar", no default key). The strip's icons are all `accent` (`accent-press` while pressed, `icon-disabled` when disabled; a plugin's image icon is tinted the same), so the strip reads as one group (user request 2026-10-31). The toggle's on state is a 4px `accent` dot centred under its icon (cell y 29–33, like the macOS Dock's running indicator), consistent with the menu check and the sidebar state; the accessibility checked state is unchanged. The toggle is disabled with the reason "No panel is in the sidebar" when no panel is attached. Built-in buttons: Vestra Player AI (workshop, order 100) and Plugin Store (store, order 200), so the end reads [AI][Store][toggle] | the player draws the strip and runs the clicks; playback places it (C7) |
| UI-BAR-10 | The time keeps priority over every right-hand button. The strip gets the width the left buttons and the time leave: `room` = bar width − ((fullscreen shown ? 6 : 5) × 41 + 10 + time + 8) − 4, and shows as many whole 41px cells as fit. Cells go from the strip's left end inward: the lowest `order` first, the sidebar toggle last. The fullscreen button is hidden when the toggle's cell would no longer fit beside it, that is below 6 × 41 + 10 + time + 8 + 41 + 4 px. Measured with the 11px body font, `0:00:00 / 0:00:00` is about 85px: [AI][Store][toggle] all show from about 476px, fullscreen from about 394px, the toggle from about 353px (compute it from the time's laid-out width, not the constant). Below a bar width of 320px the strip gets no room. Information badges (HW/SW, codec, channels, etc.) are in interaction specification IA-PB-05 | playback (layout), player (strip) |
| UI-BAR-11 | When loading fails, the status goes into an on-screen message; retry is the menu command "Reopen Current/Last File" (Ctrl+Y) | playback |

## 5. Playlist (playlist plugin playlist + host)

### 5.1 Toggle and docking

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-PL-01 | The toggle command "Playlist" has a check mark, shortcut F6, and sits in the window entry group at the top level of the context menu (§6.2); when the list has focus, F6 also closes the list | playlist (menu position) + host (F6, C3) |
| UI-PL-02 | The bottom-bar toggle button (UI-BAR-09), the menu check and F6 / Close List are kept in the same state | playlist + playback (C7) |
| UI-PL-03 | Shown by default, attached on the right; visibility, docking state and width are saved across launches | host (persistence, C8) |
| UI-PL-04 | Can be detached into a floating window (default 300×408, minimum 300×250) and attached back to the main window (C14); the detach/attach button is at the right end of the tab strip | host (C14) + playlist (button) |

### 5.2 List appearance

```css
.pl-tabs   { height: 31px; background: var(--bg-bar); }          /* shown even when there is only the one "Playlist" tab */
.pl-tab    { width: 105px; height: 31px; font-size: 12px; color: var(--text); }  /* no color block; a 28×2 accent-fill round-capped short line centered under the text (1px above the bottom edge) */
.pl-close  { width: 14px; height: 13px; }                        /* right end of the tab strip */
.pl-list   { background: var(--bg-list); }                       /* no zebra stripes, no cards, rows without rounded corners */
.pl-item   { height: 20px; padding: 0 6px; font-size: 11px; color: var(--text-list); } /* single line: index. title … duration */
.pl-item.selected { background: var(--selected-bg); color: var(--text); }
.pl-item.current  { color: var(--playing); }                     /* now playing; 2px playing bar on the left (3px margin top and bottom, round caps) */
.pl-item.played   { color: var(--text-played); }                 /* played */
.pl-bottom { height: 35px; background: var(--bg-panel); border-top: 1px solid var(--line); padding-left: 6px; }
/* left→right: move to top, move up, move down, move to bottom (25px slot each, icon 14×12) | 6 | add | 4 | remove | 4 | sort | search box (24 high, hover-bg; collapses to a magnifier icon when narrow) | total duration 11px text-list */
.pl-btn    { height: 18px; width: 41px; border-radius: 4px; background: var(--button-bg); color: var(--text-list); }
           /* icon labels add / remove / sort (10×10) centered; tooltips "Add", "Delete", "Sort" */
```

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-PL-05 | A 31px tab strip at the top (a single "Playlist" tab, 105px wide), no large title; the detach button is at the right end of the tab strip, and there is no close button (user ruling 2026-11-07: next to the window's close button it closed the player when missed). When the sidebar holds panels from other plugins, the tab strip is drawn by the host instead (same style, one tab per panel, the playlist first, buttons act on the whole sidebar), and the playlist hides its own tab strip ([panel-slots.md](../architecture/panel-slots.md)) | playlist + host (tabs) |
| UI-PL-06 | List items are single lines: `index. title` left-aligned, duration right-aligned, background `bg-list`, no cards and no rounded corners, row height 20px | playlist |
| UI-PL-07 | State is shown by color: now playing `playing` (plus a 2px bar on the left), played `text-played`, selected `selected-bg` background; no status text line. Failures use an on-screen message + a small icon at the end of the row | playlist |
| UI-PL-08 | No button row above the list. Previous/next item belong to right-click on the bottom bar and PgUp/PgDn, playing is done by double-click/Enter, adding belongs to the popup menu of the bottom-bar "Add" button | playlist |
| UI-PL-09 | 35px button bar at the bottom: four icon buttons move to top/move up/move down/move to bottom (25px slots) + three 41×18 icon buttons add / remove / sort (+ − ⇅, with tooltips) + search box (about 24px high) + list total duration on the right | playlist |
| UI-PL-10 | No path or message text at the bottom; the full path is a mouse tooltip, messages use on-screen messages | playlist (tooltips use `ToolTipArea` from C2) |
| UI-PL-11 | Scroll bars follow the palette (std-widgets dark or light, §2.1) | playlist (C2) |

### 5.3 List interaction

| ID | Action | Requirement | Owner |
| --- | --- | --- | --- |
| UI-PL-12 | Double-click an item | Play | playlist |
| UI-PL-13 | Enter / P | Play the selection | playlist + host (keys while the list has focus, C3) |
| UI-PL-14 | Delete | Remove the selection | Same as above |
| UI-PL-15 | Ctrl+↑/↓, Ctrl+Home/End | Move up/move down/move to top/move to bottom | Same as above |
| UI-PL-16 | Ctrl+A, Shift/Ctrl+click | Multi-select; DEL and moves act on the selection | playlist |
| UI-PL-17 | Drag | Drag to reorder (off while a search filter is active) | playlist |
| UI-PL-18 | List context menu | This plugin's commands, in the same style as the main menu (list in the list context menu table of [menus.md](menus.md)) | playlist (C11) |
| UI-PL-19 | End of the list | By default playback stops at the end; under "Playback ▸ Playlist" there are loop (Off/All/One, radio) and shuffle (check), off by default | playlist |
| UI-PL-20 | When a new file is opened | The list is handled according to the "Playback ▸ Playlist ▸ When Opening a File" setting; defaults in interaction specification IA-PL-01 | playlist |

## 6. Context menu

### 6.1 Structure principles

- Frequently used direct commands (open file, close, fullscreen, window entries) are at the top level; domain features are in submenus.
- There are no "File" or "View" submenus: open-type commands are under "Open ▸", window toggles are directly at the top level.
- Commands are grouped by `group`, with a separator between groups; within a group they are sorted by `order` (C4).

### 6.2 Top-level structure

```
Open File…              Ctrl+O / F3      playback
Open ▸                                  playback, playlist, subtitles, audio, history, etc. (Open URL… Ctrl+U, Open Folder… F2, Open Subtitle… Alt+O, Recently Played ▸ …)
Close                   F4               core
Reopen Current/Last File Ctrl+Y          playback
Copy to Clipboard ▸                      playback
——
Playback ▸                              core, playback, playlist, subtitles, window (Play/Pause Space, Seek ▸, Jump to Time… G, Speed ▸, Previous PgUp, Next PgDn, Playlist ▸ …)
Subtitles ▸                             subtitles
——
Video ▸                                 video, capture
——
Audio ▸                                 core (volume ↑/↓, mute M), audio
Games ▸                                 (appears only when a plugin adds items, e.g. VestraNES)
——
Toggle Fullscreen       Enter            host
Screen ▸                                window
——
Playlist ✓              F6               playlist
Playback Info ✓         Tab              info (§9)
Playback Properties…    Ctrl+F1          info
Vestra Player AI…                        workshop
Plugin Store…                            store
——
Settings ▸                              preferences, hotkeys, plugins, visualizer (Options… F5, Keyboard Shortcuts…, Appearance ▸ …)
——
About Vestra Player…    F1               info
——
Quit                    Cmd+Q            host   (macOS; Windows/Linux show no shortcut)
```

The full contents of the submenus and the plugin that owns each item are in [menus.md](menus.md).

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-MENU-01 | Each plugin's command paths and shortcuts follow §6.2; when an entry is added or moved, [menus.md](menus.md) is updated at the same time | each plugin |
| UI-MENU-02 | The top level can hold direct commands (not only submenus), with separators and group order (C4) | host + SDK (C4) |
| UI-MENU-03 | No command to hide the bottom bar; in fullscreen the bottom bar hides automatically (UI-SHELL-09) | playback |
| UI-MENU-04 | "Subtitles ▸ Show Subtitles" is a check item, Alt+H | subtitles |
| UI-MENU-05 | Speed menu: "Normal" Z, "Slower 0.1" X, "Faster 0.1" C, plus a radio group of fixed rates (in the same group as "Normal") and "Custom…" | core + playback |
| UI-MENU-06 | Check items use a checkbox; mutually exclusive options (speed, loop mode, screenshot format, palette) use a radio dot | host (C4) |

### 6.3 Style

```css
.ctx-menu  { background: var(--bg-panel); border: 1px solid var(--line); border-radius: 8px; padding: 2px;
             box-shadow: 0 0 8px var(--menu-shadow); }   /* vestra #00000080, vestra-light #00000033 (theme.rs, MenuChrome.shadow) */
.ctx-item  { height: 24px; border-radius: 4px; padding: 0 8px 0 4px; font-size: 12px; color: var(--text-list); }
.ctx-item.lit      { background: var(--selected-bg); color: var(--text); }   /* hovered or keyboard-current item, at once, and on the levels above it the open items leading to it (one per level); 2×14 accent bar on the left edge (y 5) */
.ctx-item.open     { }                                                       /* its submenu is open: only the 2×14 accent bar while the cursor is on another item of the same level (0.6.0, Windows menus) */
.ctx-item:disabled { color: var(--text-dim); }
.ctx-shortcut      { font-size: 12px; color: var(--text-dim); text-align: right; }
.ctx-arrow         { width: 10px; }                                          /* submenu arrow: submenu icon 5×8, x 3 */
.ctx-sep   { height: 7px; } .ctx-sep::after { height: 1px; margin: 0 8px; background: var(--line); }
.ctx-check { width: 14px; height: 14px; border-radius: 4px; background: var(--accent); /* white menu-check icon */ }
.ctx-radio { width: 12px; height: 12px; border-radius: 50%; border: 1.5px solid var(--accent); /* 6px accent dot in the center */ }
.ctx-scroll-arrow  { height: 12px; }   /* when the menu does not fit, one strip at the top and one at the bottom, scroll-up / scroll-down icons 8×5 */
.ctx-tip   { max-width: 360px; padding: 5px 8px; background: var(--bg-panel); border: 1px solid var(--line); border-radius: 4px; font-size: 11px; }
```

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-MENU-07 | Menu colors, corner radii and shortcut text as in the CSS above. Shortcuts are written by system convention (`platform::KEY_LABELS`): on Windows and Linux `Ctrl+Shift+S`, `Alt+→`; on macOS symbols in the order ⌃⌥⇧⌘ without "+" (`⇧⌘S`, `⌥→`), a `Ctrl` binding is written as ⌘ (it responds to ⌘ on macOS), and Esc, Enter, Tab, Backspace, Del, PgUp, PgDn, Home, End are written as ⎋ ↩ ⇥ ⌫ ⌦ ⇞ ⇟ ↖ ↘. In this document and in menus.md all shortcuts use the Windows notation | host |
| UI-MENU-08 | Item height 24px, font size 12px; separator row 7px (1px line in the middle); the four constants row height, separator, padding and arrow are kept identical in `menu-view.slint` and `menu.rs` | host |
| UI-MENU-09 | A menu panel is as wide as its widest row measured with the UI font (title, shortcut text, check and arrow columns, paddings), between 168px and 420px; longer titles are elided. The constants in `ui/menu.rs` match the row layout in `menu-view.slint` (player 0.5.9) | host |

## 7. Tool windows (picture, audio, subtitle, capture and other plugins)

### 7.1 Panel page specification

Picture Adjustments, Audio Workbench and Subtitle Tuning use the same small-window specification. Changes take effect immediately, there is no "Apply" or "OK", and every item has its own "Default" reset:

```css
.tool        { width: 301px; background: var(--bg-bar); }          /* 8px left and right inside the content area, usable width 285 */
.cbox-tabs   { height: 31px; } .cbox-tab { width: 74px; font-size: 12px; color: var(--text-dim); }
             /* selected: text, a 28×2 accent-fill short line under the text; unselected hover: a radius-control rounded hover-bg plate inset by (3,4) */
.cbox-page   { background: var(--bg-panel); }
.row         { height: 24px; }                                       /* row pitch 24 */
.row label   { width: 65px; text-align: right; font-size: 11px; color: var(--text); }
.row slider  { width: 154px; height: 12px; }  /* 2px line, track line, fill accent-fill, thumb 14×12 text, corner radius radius-control */
.row default { width: 41px; height: 18px; }
.toggle      { width: 46px; height: 22px; }                          /* grouped buttons such as flip and rotate */
.choice      { width: 51px; height: 22px; } .choice.wide { width: 69px; }
.eq-slider   { width: 14px; height: 70px; gap: 23px; }               /* vertical equalizer sliders */
```

### 7.2 Window list

| Plugin | Window | Size |
| --- | --- | --- |
| video | Picture Adjustments (tabbed) | 301×245 |
| video | Custom Aspect Ratio | 240×98 |
| video | Shader List | 301×260 |
| audio | Audio Workbench (tabbed) | 301×260 |
| subtitles | Subtitle Tuning, Subtitle Settings | width 301 |
| subtitles | Adjust Sync | 350×121 |
| capture | Burst, export tasks | width 438 |
| capture | Export Clip | width 400 |
| capture | Snapshot Settings | width 360 |
| playback | Input boxes such as Jump to Time and Seek Steps | width 357 |
| playback, playlist, store | Link input | preferred width 397, minimum 260 |
| marks | Chapters and Bookmarks | 380×420, minimum 320×300 |
| info | Playback Properties | 460×420, minimum 400×320 |

### 7.3 Common requirements (all tool windows)

The rules for plugin authors are on the SDK documentation site, [Interface guidelines → Tool windows](https://vestrasys.github.io/vestra-player-sdk/documents/Interface_guidelines.html#tool-windows); this table is the acceptance list for the built-in plugins.

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-TOOL-01 | No large title inside the window (the window title bar already has the title) | all tool window plugins |
| UI-TOOL-02 | Theme styles (following the palette) and the §2.3 control sizes: buttons 21px, input boxes 21px, form row pitch 24–26px, padding 8–14px, body text 11px | all tool window plugins (C2) |
| UI-TOOL-03 | Sliders use the panel style of the widget library: 2px line, `accent-fill` fill, thumb 14×12 | all plugins that use sliders |
| UI-TOOL-04 | Changes take effect immediately + a "Default" button per item: no "Apply" or "OK" | video, audio, subtitles |
| UI-TOOL-05 | Status and error text is one sentence (source text in English, shown in the interface language); no debug information (task id, revision number, generation) | all plugins |

### 7.4 Window specifications (D2: separate windows, following §7.1)

```css
/* Picture Adjustments (video) */
.picture { width: 301px; height: 245px; }
.picture .row { label: 65px; slider: 154x12; default-btn: 41x18; gap: 6px; }  /* row width 272 */
/* tabbed; rows on the color page: brightness, contrast, saturation, hue, Gamma */

/* Audio Workbench (audio) */
.audio { width: 301px; height: 260px; }
.audio .tabs { tab: 74x31; }
.audio .eq   { vertical-sliders: 14x70; gap: 23px; }
.audio .toolbar { enable-check: 14px box + 11px text; preset: popup-menu; }

/* Subtitle Tuning (subtitles) */
.subtitle { width: 301px; }
/* track dropdown + load; font size − + default (51×22, 41×22); position; sync; color/outline/font are in "Subtitle Settings…" */
```

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-TOOL-06 | Picture Adjustments as specified above: row layout "label + slider + default button", no large title and no long status text; window no larger than 320×280 | video |
| UI-TOOL-07 | Audio Workbench as specified above: vertical equalizer, preset popup menu; window no larger than 320×280 | audio |
| UI-TOOL-08 | Subtitles are split into the small "Subtitle Tuning" window (same specification as above, no larger than 320×280) and "Subtitle Settings…" (font, color, outline, shadow, background box, margins, sync step, fallback encoding) | subtitles |
| UI-TOOL-09 | Shortcuts: picture Q (reset), W/E brightness, R/T contrast, Y/U saturation; equalizer Shift+E; subtitles Alt+H, `>` `<` `/` sync; the keys work and show an on-screen message | video, audio, subtitles (C3) |

### 7.5 Dialog-type windows

| ID | Dialog | Requirement | Owner |
| --- | --- | --- | --- |
| UI-DLG-01 | Jump to Time (G) | Separate small window (width 357) with a set of time inputs | playback |
| UI-DLG-02 | Open URL (Ctrl+U) | All link input windows use `UrlInputRow` from the widget library: input box + [Open] [Cancel], Return confirms, Esc cancels; no history list. Shared by Open URL, the list's "Add Link…", the Plugin Store and link-type plugins (YouTube, Bilibili) | playback, playlist, store, link-type plugins |
| UI-DLG-03 | Screenshot | No window: Ctrl+E saves directly to the snapshot folder (PNG by default), Ctrl+C copies to the clipboard, "Save As…" opens a save dialog; an on-screen message on success; the save folder is chosen in Snapshot Settings | capture |
| UI-DLG-04 | Burst | Separate dialog (width 438): storage, format/size, count and range groups, bottom buttons 24px high | capture |
| UI-DLG-05 | Export Clip | Separate dialog (width 400), grouped by the §2.3 specification | capture |
| UI-DLG-06 | Plugin Workshop | Only the §7.3 common requirements | workshop |

## 8. Keyboard and mouse

### 8.1 Shortcuts

Default bindings (the complete list and conflict check are in [menus.md](menus.md)):

| Function | Default keys | Owner |
| --- | --- | --- |
| Play/pause | Space | core |
| Back/forward 5 seconds | ← / → | core |
| Back/forward 30, 60, 300 seconds | Ctrl / Shift / Ctrl+Alt + ←/→ | playback |
| Previous/next | PgUp / PgDn | core (C6) |
| Volume +/− | ↑ / ↓ | core |
| Mute | M | core |
| Speed normal/−0.1/+0.1 | Z / X / C | core |
| Jump to Time | G | playback |
| Open file / link / subtitles | Ctrl+O (F3) / Ctrl+U / Alt+O | playback, subtitles |
| Close | F4 | core |
| Fullscreen | Esc toggles (user ruling 2026-11-01; Enter, Alt+Enter, F11 up to 0.5.x); Enter / Leave Fullscreen have no default key | host |
| Playlist | F6 | playlist |
| Playback Info | Tab; Brief Info Shift+Tab | info |
| Screenshot save/copy | Ctrl+E / Ctrl+C | capture |
| Picture color | Q W E R T Y U | video |
| Equalizer | Shift+E | audio |
| Subtitles show/sync | Alt+H / > < / | subtitles |

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-KEY-01 | The host routes any key (including modifiers, function keys, letters, arrow keys) to commands through the binding table (C3); when the same input is declared by two commands, a conflict is reported | host + SDK (C3) |
| UI-KEY-02 | Keys pressed while the playlist has focus (Enter, Delete, Ctrl+↑/↓, etc.) act only on the list | host + SDK (focus scope in C3) |

### 8.2 Mouse

| ID | Action | Default | Owner |
| --- | --- | --- | --- |
| UI-MOUSE-01 | Double-click the video | Unbound by default (host 0.5.2, user ruling 2026-10-26; it was Toggle Fullscreen); the user can bind it under "Settings ▸ Mouse…" | host (C3) |
| UI-MOUSE-02 | Wheel | Volume +/− | core (C3 binding table) |
| UI-MOUSE-03 | Click the video | Play/Pause by default, on release (host 0.5.2); rebind under "Settings ▸ Mouse…" | host (C3) |
| UI-MOUSE-04 | Right-click | Menu | host |
| UI-MOUSE-05 | Drag the video in windowed mode | Move the window | host |

Side buttons X1 (back) / X2 (forward) default to next / previous (user setting, 2026-11-01) and work anywhere in the window (browser precedent, host 0.6.0).

## 9. On-screen messages and playback information

### 9.1 On-screen messages (OSD)

Seeking, volume, mute, speed, pause, color, subtitle sync, screenshots and similar actions show a one-line message in the top-left corner of the video, for example "Volume: 80%", "Speed: 1.5x", "Paused".

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-OSD-01 | The host provides `plugin.osd.show(text)`: top-left corner, 10px margin; translucent dark rounded plate `#0b0a10c4`, white text `#ffffff` semibold (600), a 2px `accent-fill` gradient line along the bottom edge (inset by one corner radius on each side); the font size is 3% of the video area height, rounded and clamped to 12–48px (interaction specification G-10), and the plate padding and corner radius scale with the font size (at 18px: 12 left/right, 5 top/bottom, radius 8); shown for 2 seconds, then a 300ms fade-out; a new message replaces the old one | host + SDK (C5) |
| UI-OSD-02 | Each plugin gives action feedback through on-screen messages: playback (seek, volume, mute, speed, pause), video (color), subtitles (show, sync), capture (save), playlist (switching, end of list, failure) | each plugin |

### 9.2 Playback Info plugin (info, D5)

| ID | Requirement | Owner |
| --- | --- | --- |
| UI-INFO-01 | "Playback Info" plugin: "Playback Info ✓" at the top level of the context menu, toggled with Tab, off by default; Shift+Tab for brief info | info (C10) |
| UI-INFO-02 | The overlay is in the top-left corner of the video, 4px below the on-screen message plate (10px left margin); each block has the same translucent rounded plate as on-screen messages (`#0b0a10c4`, corner radius scaled with the font size), text `#f5f3fa`, font size scaled with on-screen messages (11px when on-screen messages are 16px, clamped to 9–28px), refreshed every second; content is one item per line with Chinese labels: file name, time / duration, video codec and resolution, frame rate, audio codec / sample rate / channels, backend, frames displayed, dropped frames, session and frame counters | info |

Playback Properties (Ctrl+F1) is a separate window, split into video information (codec, size, frame rate) and audio information (codec, sample rate, bit depth, channels).

## 10. Core capabilities (host + SDK)

This section lists the core capabilities the interface depends on. Principle: the core only provides capabilities that several plugins share or that plugins cannot provide themselves; each one has a minimal interface; old interfaces replaced by a new mechanism are deleted, with no compatibility layer. The interfaces are in [Core UI design](../architecture/core-ui-design.md) and the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html).

### 10.1 Capability list

| ID | Capability | Requirement | Items covered |
| --- | --- | --- | --- |
| C1 | Main window shell | Video flush with the edges, `bg-video`; the sidebar takes the full height to the right of the video, 3px draggable splitter (video ≥318, sidebar ≥317, default 317); bottom bar only under the video; default window 780×671, minimum 320×182; in fullscreen the bottom bar overlays the video, shown only on hover over its band (FS-BAR-1); the pointer hides after 3 seconds idle; in a narrow window the video is shrunk first, then the sidebar is hidden | UI-SHELL-01–09 |
| C2 | Theme | Importable Slint theme (§2.1 both palettes, §2.2 font sizes, §2.3 control sizes, §2.5 icons) + controls that follow the palette (buttons, sliders, input boxes, checkboxes, dropdowns, scroll bars, tooltips); the default font is the system UI font | UI-THEME-01–03, UI-TOOL-02/03, UI-PL-10/11 |
| C3 | Input binding table | Commands declare `bindings`. Bindable inputs: all keys × modifiers (Ctrl/Shift/Alt/Cmd); on the video area, mouse left click/double-click, double-click on the left half/right half, right double-click, middle button, side buttons X1/X2 (window-level, host 0.6.0), wheel up/down/left/right, long press, all with optional modifiers. Scope: global, or while a given panel/window has focus. The host keeps the plugin defaults + a user override layer (persisted); when the same input is declared by two commands, a conflict is reported, with no silent override. Right-click is reserved for the menu | UI-KEY-01–02, UI-MOUSE-01–05, §8.1 |
| C4 | Menu structure | Commands declare `path` (any depth, including direct commands at the top level), `group` (recommended skeleton: open / domain / picture / window entries / quit) and `order`; sorted by `order` within a group, separators between groups; `radio` groups; shortcut text generated from the binding table; style as in §6.3 | UI-MENU-02/06–08, §6.2 |
| C5 | On-screen messages | `plugin.osd.show(text)`, specified in UI-OSD-01 | UI-OSD-01–02, UI-BAR-02/11, UI-PL-07/10 |
| C6 | Play sequence | One plugin registers as the sequence provider: `plugin.player.provideSequence({ next, previous, canNext, canPrevious })` (only one at a time); any plugin calls `player.next()` / `player.previous()` and reads `player.state.can.next/previous`; with no provider these are unavailable | UI-BAR-05, PgUp/PgDn, side buttons |
| C7 | Shared sidebar state | Any plugin can read and toggle whether the sidebar is shown (`plugin.panels.sidebar`) without knowing which plugin mounted it | UI-BAR-09, UI-PL-02 |
| C8 | Interface state persistence | The host saves across launches: window size and position, sidebar visibility, docking state and width, the user binding override layer | UI-PL-03, UI-SHELL-06, C3 |
| C9 | Window title | The host shows the current source name in the window title | UI-BAR-02 |
| C10 | Video overlay and playback statistics | A plugin shows a read-only information panel over the video (follows the video area, takes no input); `player.stats`: video codec, resolution, frame rate, audio codec / sample rate / channels, backend, frames displayed, dropped frames | UI-INFO-01–02 |
| C11 | Context menu inside panels | Right-clicking in a plugin's own panel / window pops up a set of that plugin's commands, in the same style as the main menu | UI-PL-18 |

### 10.2 Core items decided by §1

| ID | Capability | Decision | Conclusion |
| --- | --- | --- | --- |
| C12 | Control panel container (small tabbed window, plugins mount pages) | D2 | Won't do |
| C13 | macOS top menu bar mirroring the command tree | D4 | Won't do |
| C14 | Detach sidebar panels into a floating window and attach them back to the main window (attached by default; floating window default 300×408, minimum 300×250; state and position remembered) | D6 | Do |

### 10.3 Implementation record

C1–C11 and C14 were implemented on 2026-10-20.

## 11. Items owned by each plugin

Check this table when a plugin changes its interface. When the core capabilities are not enough, file a request with the API maintainer under §4 of the [community workflow](../../agentic-workflow/agentic-workflow.md); the maintainer first discusses with the author whether existing interfaces or an in-plugin implementation can solve it, and changes the core only when it is really necessary. Acceptance is by the GUI test in §6.2 of the workflow.

| Plugin | Items |
| --- | --- |
| all plugins | UI-MENU-01 (menu position as in §6.2), §8.1 shortcuts, UI-OSD-02 (on-screen messages) |
| playback | UI-BAR-01–11, UI-MENU-03/05, UI-DLG-01/02 |
| playlist | UI-PL-01/02/04–20, UI-DLG-02 |
| video | UI-TOOL-01/03–06/09 |
| audio | UI-TOOL-01/03–05/07/09 |
| subtitles | UI-TOOL-01/03–05/08/09, UI-MENU-04 |
| capture | UI-TOOL-01/05, UI-DLG-03–05 |
| store, link-type plugins | UI-TOOL-01/05, UI-DLG-02 |
| workshop | UI-TOOL-01/05, UI-DLG-06 |
| info | UI-INFO-01–02 |

## 12. Open items

(None.) All values in this document are settled. Open interaction items are in the appendix of the [Interaction specification](interaction-spec.md).
