# Interaction specification

This document is the Vestra interaction specification: mouse, keyboard, menu, window and notification behaviour in each window state, written as requirements. Product-level constraints (menus organised by domain, UI ownership, playback continuity) are in [Interaction requirements](interaction-requirements.md); which features exist is in [Feature requirements](feature-requirements.md); UI sizes and colours are in [UI specification](ui-spec.md); the menu tree is in [Menus](menus.md); the plugin interface is in the [Plugin API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html).

The layout follows the feature requirements: the core (host) first, then one chapter per plugin. Each item states the requirement, any necessary implementation constraints, whether the core needs a new API, cross-platform notes and acceptance items. Appendix A gives the specification cell by cell as "state × interaction element", Appendix B is the complete rule for the playlist's "similar files" check, and Appendix C lists the implementation round and status of each item.

## 0. Scope and conventions

### 0.1 What this document covers

- It describes **behaviour**: which input in which state produces which result, including defaults, thresholds, delays and persistence. Sizes, colours and icons belong to the UI specification; this document mentions them only where they affect interaction.
- The specification has been checked against the code. The values in the items (thresholds, delays, animation durations) are Vestra's values; to change one later, change this document and update the code to match.
- Interfaces marked [New core API] are request drafts that have not been accepted yet. Following the [community workflow](../../agentic-workflow.md) §4.1 they are submitted separately and go through the negotiation process of §4.3; once the maintainer accepts them they are written into the reference documentation.

### 0.2 Settled interaction decisions

The decisions below replace earlier wording in the feature requirements and the UI specification; the corresponding original text has been rewritten or given a "superseded" note.

| Decision | Item |
| --- | --- |
| In fullscreen an attached playlist does not disappear; when the pointer moves to the right edge of the screen it pops out as an overlay; `SlotState.reason: "fullscreen"` means "the fullscreen overlay is currently retracted" | G-01 |
| Opening a local file first clears the list, then adds files according to the "When Opening a File" mode (default "Add Similar Files"); "Append when opening or dropping; don't clear the list" is an option, off by default | IA-PL-01 |
| Dropping on the video area: clear the list first, then add and play; dropping on the list: append to the end, "insert at the drop position" is an option, off by default | G-11, G-12 |
| When playback starts, resize the window to 1× the video size (default) | G-02 |
| Hide the pointer while playing is on by default | G-06 |
| Four always-on-top modes, default "While Playing in Fullscreen" | IA-WIN-03 |
| `` ` `` is "Mini Size" on every system; mini size is not on top; picture-in-picture is a separate command with no default key | IA-WIN-01, IA-WIN-02 |
| Seek bar hover time is off by default | IA-PB-01 |
| The volume limit in the UI and for keys is 100; `player.setOutput` still accepts 0–200 | G-13a, G-13b |
| The background layer takes effect while loading and shows the loading screen | G-16a, G-16b |
| OSD font size follows the video area height | G-10 |
| The menu pops up when the right button is released; a submenu opens at once on hover (a diagonal toward an open submenu keeps it); Esc closes one level at a time | G-15, IA-CORE-01 |
| The video area has no minimum height | IA-CORE-13 |
| Custom-drawn title bar with no main menu button; the menu is still opened with the right button | IA-CORE-07 |
| No F7 control panel: picture, audio and subtitles stay three separate tool windows | IA-CORE-09 (cancelled) |

### 0.3 Not planned

- **Control panel (F7)**: not planned. Picture Adjustments, Audio Workbench and the subtitle workbench are each a separate tool window. The items that depend on the control panel (IA-CORE-09, IA-PB-08, IA-VID-01, IA-AUD-01, IA-SUB-01, IA-MK-01, and the control panel switch of IA-PB-06) are cancelled together; their numbers are kept and not reused.
- **Main menu button**: no main menu button on the title bar; the menu opens only from the right button.
- **360°, 3D, browser (chat) buttons**: Vestra has none of these features, so the bottom bar has no buttons for them. Building these features later belongs in the feature requirements.
- **Audio-only layout**: audio-only playback keeps the video layout, and the background layer shows the album cover (Appendix A G-09).
- **Drop target hints during a drag**: when dragging files in, no "play / add to list" style drop zones are shown (R-08).

### 0.4 Scope

**In scope**: interaction of the main window, floating windows, menus, the bottom bar, the playlist, the OSD and the info layer in windowed mode, fullscreen, maximised, mini size, picture-in-picture, idle, loading and audio-only states.

**Out of scope**: appearance (colours, corner radii, icon glyphs; see the UI specification); new features (live streams, chat, etc.).

### 0.5 Relation to other documents

- **Feature requirements** ([feature-requirements.md](feature-requirements.md)) cover "which features exist"; this document covers "interaction details". If the two conflict, this document wins and the feature requirements are corrected.
- **UI specification** ([ui-spec.md](ui-spec.md)) covers sizes, colours and layout; if it conflicts with this document, this document wins for interaction.
- **Menus** ([menus.md](menus.md)): after menu items added or moved by this document are implemented, menus.md and the conflict check are updated to match.
- **Plugin API reference** ([SDK documentation site](https://vestrasys.github.io/vestra-player-sdk/)): the "Core API requests in this chapter" at the end of each chapter are request drafts; after the maintainer decides, the maintainer updates the reference documentation. Semantic changes that alter only behaviour and not signatures are also written into the change notes of the reference documentation.

### 0.6 Numbering, priority, status and conventions

- **Numbering**: `G-xx` are the eighteen high-priority items; those that span the core and a plugin are split into `G-xxa` (core) and `G-xxb` (plugin). The rest are numbered per chapter as `IA-<chapter>-nn`: `CORE` (core), `PB` (playback), `PL` (playlist), `WIN` (window), `INFO`, `VIS` (visualizer), `HK` (hotkeys), `VID` (video), `AUD` (audio), `SUB` (subtitles), `MK` (marks). The per-cell numbers of Appendix A (`W-xx`, `F-xx`, `M-xx`, etc.) and the items in the main text refer to each other. Numbers are not reordered or reused; code and other documents refer to items by number.
- **Priority**: **P1** affects daily use; **P2** inconsistent state or commonly used options; **P3** completeness and rarely used items.
- **Status**: each item has one. **Implemented**: the code matches this document. **Partially implemented**: states which parts are implemented. **Not implemented**: this document is the target, scheduled in rounds 3 and 4 of Appendix C. **Cancelled**: the number is kept and the item will not be implemented.
- **Cross-platform**: per workflow §1.5, system-specific code lives only in the platform modules under `src/host/src/platform/`, and the plugin API exposes no system-specific concepts. Each item states the macOS and Windows approach; other systems go through `platform/other.rs`, and what cannot be done returns a uniform `unsupported`.

---

## Core (host)

**Location**: `src/host/` (`src/host/src/*.rs`, `src/host/ui/host.slint`, `src/host/ui/menu-view.slint`, platform modules `src/host/src/platform/`)

### G-01 In fullscreen the attached playlist pops out from the right edge (P1)

**Status**: implemented (window state in `window_state.rs`, overlay in `composition.rs`). The switch "pop out in fullscreen even when closed in windowed mode" belongs to IA-CORE-06 and is not implemented; the behaviour is fixed at the default (off).

**Requirements**:

1. Applies when: fullscreen, and the sidebar is mounted and attached (`detached` is false). A floating sidebar stays a separate window and is not affected.
2. Two states: **retracted** (not shown) and **popped out** (overlaid on the right side of the video, without squeezing the video). Entering fullscreen starts retracted.
3. Pop-out condition: the user preference is shown (`preferred`), and the pointer enters within **4 px** of the right edge of the screen. When the preference is hidden, the right edge does not trigger.
4. Pop-out geometry: full height from the top of the screen to the bottom; the width is the sidebar width the user dragged in windowed mode, and at most "screen width − 318". While popped out, the bottom bar width immediately narrows to "screen width − pop-out width", with the right end of the bottom bar aligned to the left edge of the pop-out.
5. Animation: slides out to the left in 150 ms when popping out, slides back to the right in 150 ms when retracting.
6. Retract conditions (any one):
   - 500 ms after the pointer leaves the pop-out (x less than the left edge of the pop-out, including resting on the bottom bar);
   - the pop-out has no lock (item 7) and there is no activity for 3000 ms (the host's activity idle timer, shared with the pointer auto-hide; the fullscreen bar no longer uses it, FS-BAR-1);
   - the user presses F6 or clicks the list button on the bottom bar, changing the preference to hidden;
   - fullscreen is exited.
7. Locks: while the panel reports `focused` (for example the search box has focus), `dragging` (drag reordering) or `popupOpen` (list context menu) as true through `Panel.setInteraction`, it does not retract; the same lock rules as the bottom bar. The host's own hover lock (G-04) also applies to the pop-out.
8. Pressing F6 in fullscreen: when the preference changes from hidden to shown it pops out immediately and stays until the pointer enters and then leaves, or 3000 ms without activity; when it changes from shown to hidden it retracts immediately.
9. Focus: popping out does not take keyboard focus; after the user clicks the pop-out, focus goes into the panel; when retracting, if focus is in the panel it returns to the video area.
10. Pointer: the pointer is not hidden while it is over the pop-out.
11. `SlotState` semantics: `effective` means "visible right now", `true` when popped out and `false` when retracted; `reason: "fullscreen"` means "the fullscreen overlay is currently retracted". Whether it is in overlay form is decided by the plugin with `window.state.fullscreen && !state.detached`.
12. The pop-out is a separate instance of the panel component in fullscreen, rebuilt when entering / exiting fullscreen; local state in the component (such as the scroll position) is not kept across fullscreen, while properties the plugin writes on the global are not affected. In fullscreen, drops onto the pop-out go to the sidebar panel.

**Cross-platform**: pure host logic. macOS fullscreen is a separate space, and moving the pointer to the top of the screen reveals the system menu bar, which does not conflict with the right edge. On Windows with multiple monitors, the right edge is computed for the screen the main window is on (pointer coordinates are window coordinates anyway).

**Acceptance**:

1. With the list open in windowed mode, enter fullscreen: the list is not shown, and the video fills the screen.
2. Move the pointer to the far right of the screen: the list slides out from the right, covering the right side of the video, and the video does not shrink; the right end of the bottom bar aligns with the left edge of the list.
3. Move the pointer back to the middle of the video: the list slides back after about half a second.
4. With the list popped out, click the search box, type text and move the pointer away: the list does not retract; press Esc to clear the search, so focus returns to the list, then move away: the list retracts.
5. With the list closed in windowed mode (F6), enter fullscreen and move the pointer to the right edge: it does not pop out. Press F6 in fullscreen: it pops out immediately; press F6 again: it retracts immediately.
6. The lit / dim state of the list button on the bottom bar matches the preference in each step of item 5 (G-18).
7. Enter fullscreen while the list is floating: the floating window stays as it is, and the right edge does not trigger.

### G-04 In fullscreen, the bottom bar does not hide while the pointer rests on it (P1)

**Status**: implemented. The switch in item 4 belongs to IA-CORE-06 and is not implemented; the behaviour is fixed at the default (on).

**Requirements**:

1. While the pointer is inside the bottom bar rectangle, the bottom bar counts as locked and does not hide.
2. Since FS-BAR-1 the fullscreen bar shows only while the pointer is in its band and hides 500 ms after the pointer leaves it (see the FS-BAR-1 requirement below); moving over the bar still counts as activity for the pointer auto-hide.
3. While the pointer is on the bottom bar (or in its fullscreen band) the pointer is not hidden (pointer hiding applies only to the video area).
4. The host settings page (IA-CORE-06) provides the switch "Don't auto-hide the pointer over the control bar", on by default.

**Implementation constraints**: the bottom bar and the fullscreen sidebar pop-out are wrapped in a `TouchArea` that only detects hover (as the parent of the content, not consuming the children's events), outputting `bottom-hover` / `sidebar-hover`; the video area outputs `video-hover`. The hover lock is the host's own lock, and a plugin's `setInteraction` cannot clear it.

**Cross-platform**: pure host logic.

**Acceptance**:

1. Play in fullscreen, move the pointer onto the bottom bar and keep it still for 10 seconds: the bottom bar does not disappear, and the pointer does not disappear.
2. Move the pointer left and right on the bottom bar for 10 seconds: the bottom bar does not disappear.
3. Move the pointer from the bar to the middle of the video and rest it there: the bar disappears after 500 ms, the pointer after 3 seconds (FS-BAR-1).
4. With the host settings switch off (IA-CORE-06, not implemented): the bar still shows only while the pointer is in its band (FS-BAR-1); the switch affects only the pointer auto-hide over the bar.

#### FS-BAR-1 The fullscreen bar shows only on hover over its band (2026-10-25)

**Status**: implemented (round record `doc/process/rounds/2026-10-ui-polish.md`).

**Requirement**: in fullscreen the bottom bar is hidden and shows only while the pointer is in its band: the bar's own rectangle, from the bottom edge up by the bar's laid-out height (logical px, any monitor size and scale), across the full width minus a popped-out sidebar (a floating sidebar takes nothing). Entering the band shows it at once; it hides 500 ms after the pointer leaves (the G-01 `LEAVE_MS`), and coming back within that time keeps it. Nothing else reveals it: pointer motion elsewhere, keys and bound commands (seek, volume, pause, speed), the wheel, clicks, menus, dialogs, plugin activity and entering fullscreen. A menu or dialog opened while the bar shows holds it until closed; one opened while it is hidden does not reveal it. A bar lock (`setInteraction`, e.g. a seek or volume drag) holds it until released. The OSD is unchanged. The pointer auto-hide is independent of the bar (3 s without motion or keys, no menu or dialog, not over the bar or in its band, no bar lock). Windowed mode is unchanged.

**Acceptance**: fullscreen on two monitor sizes, with the sidebar popped out and floating: the bar appears only with the pointer at the bottom band; Ctrl+→ shows "Forward 30 s: ..." on the OSD with no bar; a seek drag that leaves the band keeps the bar until release.

### G-03 The context menu is fully usable in a small window and can extend beyond the main window (P1)

**Status**: partially implemented. Step one (scrolling inside the window) is implemented; step two (a menu overlay window, so the menu can extend beyond the main window) is not implemented.

**Requirements**:

1. Each menu level is positioned within the **available area**: in step one, the inside of the main window; in step two, the work area of the display the main window is on (excluding the menu bar and Dock on macOS, and the taskbar on Windows), not limited by the size of the main window.
2. The top level pops up at the pointer; if there is no room on the right it opens to the left, if there is no room below it opens upwards (against the pointer), and if it still does not fit it sticks to the edge of the available area.
3. A submenu opens to the right of its row, overlapping the parent by 2 px; if there is no room on the right it flips to the left of the parent; if there is no room below it moves up as a whole until it touches the bottom edge of the available area. A submenu is positioned at its parent row's position after scrolling.
4. When a level is taller than the available area, its height equals the height of the available area and a 12 px scroll arrow strip (▲ / ▼) is shown at each end; while the pointer rests on an arrow strip it scrolls one row every 50 ms, the wheel scrolls by pixel offset, and the keyboard ↑↓ scroll automatically when moving out of the visible area. The host keeps the scroll position of each level, so opening a submenu does not lose the parent's scroll position.
5. Closing: the menu closes on a click anywhere outside it, on switching to another application, and on Esc (IA-CORE-01). When a click outside the menu closes it, that click is consumed by the menu and not passed to the video area.
6. The floating list window and panel menus (`Panel.showMenu`) use the same rules.

**Implementation constraints (step two)**: when opening a menu, the host creates a borderless, transparent, always-on-top top-level window that does not show in the taskbar, covering the whole work area of the display the pointer is on, and draws `MenuView` in this window; positioning in `menu.rs` is computed from the work area rectangle; "close the menu when the main window loses focus" becomes "close when the application is deactivated (neither the overlay nor the main window is the active window)". Step one's in-window scrolling is kept for systems where a window cannot be created.

**Core API**: none new.

**Cross-platform** (the platform module gets a function like `menu_overlay(attrs)`):

- macOS: the overlay window's level is set to the pop-up menu level (`NSPopUpMenuWindowLevel`), and its collection behaviour adds `fullScreenAuxiliary` and `moveToActiveSpace`, so it can show inside a fullscreen space as well.
- Windows: winit `with_decorations(false)`, `with_skip_taskbar(true)`, `with_window_level(AlwaysOnTop)`, `with_transparent(true)`; the work area comes from `MonitorInfo.rcWork`.
- The work area query goes into the return value of `platform::screen_for` (`ScreenFacts` gains a work area rectangle).
- Other systems: step one only.

**Acceptance**:

1. Drag the main window to 320×182 and right-click: the top-level menu is as tall as the window with ▲▼ strips at top and bottom; resting on a strip scrolls, the wheel scrolls, and the ↓ key scrolls automatically at the bottom; every item can be clicked (step one). Step two: the menu shows completely outside the window.
2. Right-click with the main window against the right edge of the screen: the menu opens to the left; a submenu flips to the left at the right edge of the screen (step two).
3. Right-click with the main window against the bottom edge of the screen: the menu opens upwards and is not covered by the Dock / taskbar (step two).
4. On a 768-high screen, open a submenu with more items than the screen height (a test plugin registers 40 items): up and down arrows appear and it can scroll (step two).
5. Right-click in the floating list window (300×250): the list menu can scroll and every item can be clicked.
6. Right-click in fullscreen (macOS fullscreen space): the menu shows normally inside the fullscreen space.
7. With the menu open, click the video outside the menu: the menu closes and the video does not respond to this click; switch away with ⌘Tab / Alt+Tab: the menu closes.

### G-15 Submenus open on hover; the menu pops up on right-button release (P1)

**Status**: implemented. Host 0.5.2 (2026-10-26, user ruling): submenus open and switch at once, with no delay on any platform; the aim rule is `menu::HoverAim` (pure data, unit tested), fed with the pointer the window's winit hook records. Before, a submenu opened after a 400 ms hover (Windows: `SPI_GETMENUSHOWDELAY`), which users saw as 1–2 s.

**Requirements**:

1. **Pop up on right-button release**: pressing the right button on the video area only records the position; on release the main menu pops up at **the release position**. Pressing, moving out of the video area (dragging onto the bottom bar or outside the window) and then releasing: no menu. No menu while the right button is held down.
2. **Open at once**: the pointer entering a row with a submenu opens it within the host tick (16 ms); entering a command row closes deeper levels at once. No system delay setting is read.
3. **Submenu aim**: while a submenu is open and the pointer moves toward it, it stays open when the pointer crosses sibling rows on the way: the move counts as toward it when the new pointer position is inside the triangle from the previous position to the submenu's near edge (extended 8px above and below). The switch to the sibling under the pointer happens as soon as a move leaves that triangle, or once the pointer has not moved toward the submenu for 120 ms (`AIM_GRACE_MS`); entering the submenu cancels it. Without a known pointer position (it left the window) the switch is immediate.
4. Clicking a submenu row opens it immediately; keyboard → / Enter opens it immediately; when keyboard ↑↓ moves to a command row, deeper levels close immediately.
5. The same on every platform; the system's submenu delay (`SPI_GETMENUSHOWDELAY`) is no longer read.
6. Panel menus (`Panel.showMenu`) are single-level and have no delay. Panels drawn by plugins are advised to call `showMenu` on right-button release as well, consistent with the main menu.

**Core API**: none new.

**Cross-platform**: see item 5. macOS system menus also open at once and keep a submenu during a diagonal move; Vestra now behaves the same on both systems.

**Acceptance**:

1. Press the right button and hold: no menu appears; release: the menu appears at the release point. Press the right button on the video, drag to the bottom bar and release: no menu.
2. Rest the pointer on "Video ▸" for 0.2 seconds and move away: the submenu does not open; rest for 0.5 seconds: it opens.
3. With "Video ▸" open, move the pointer diagonally from the "Video" row across the "Audio" row into the Video submenu: the submenu is not switched.
4. With "Video ▸" open, rest the pointer on the "Audio" row for about 0.4 seconds: the Video submenu closes and the Audio submenu opens.
5. Click the "Video ▸" row: it opens immediately.
6. On Windows, change the system menu delay to 100 ms and restart the player: submenus open after about 0.1 seconds.

### IA-CORE-01 Esc in a menu closes one level at a time (P2)

**Status**: implemented.

**Requirements**: while a submenu is open, Esc is the same as ←: it closes the deepest level and the keyboard cursor returns to the parent's open row; when only the top level is open, Esc closes the whole menu. For a submenu opened with the mouse (the keyboard cursor is still on the parent), Esc also closes the deepest level first.

**Cross-platform**: no difference.

**Acceptance**: open the two-level submenu "Video ▸ Aspect ▸" and press Esc three times: "Aspect", "Video" and the whole menu close in turn.

### IA-CORE-02 Menu mnemonics (P3)

**Status**: not implemented.

**Requirements**: with a menu open, pressing a letter or digit key jumps to the next item in the current level whose mnemonic is that character; if only one item in the current level has that character, it is executed (command) or opened (submenu) directly. The mnemonic is shown underlined in parentheses after the title, for example "Open File(O)".

**Implementation constraints**: the host adds a `mnemonic` field to the row data in `menu.rs`, and the keyboard handling in `menu-view.slint` matches on it.

**Core API** [New core API]:

```ts
interface CommandOptions {
  // …existing fields
  mnemonic?: string;   // a single A–Z or 0–9, case-insensitive; shown in the menu as "(X)" after the title
}
```

Without an API change: the host takes the first Latin letter of the title automatically. Cost: most Chinese titles have no Latin letter and get no mnemonic. So the request is still made, at the lowest priority, and can start with host commands and top-level and second-level menus only.

**Cross-platform**: done on both systems (macOS system menus have no mnemonics; this is Vestra's own menu).

**Acceptance**: declare `mnemonic: "O"` for "Open File…", open the menu and press O: Open File is executed.

### G-05 Dragging on the picture moves the window (P1)

**Status**: implemented. The switch in item 5 belongs to IA-CORE-06 and is not implemented; the behaviour is fixed at the default (off).

**Requirements**:

1. In windowed mode, pressing the left button on the video area and moving more than 4 px (the same tolerance as the click check) starts a window drag, and the system takes over the move.
2. Starting a drag cancels the pending click (`Mouse:LeftClick` does not fire), and this press does not take part in the double-click check either: if the press right after the drag would form a double-click with the drag's press, that double-click is ignored.
3. No dragging in fullscreen, while a menu is open, or in picture-in-picture; mini size can be dragged as usual.
4. A drag with a modifier (Alt / Ctrl / Shift / ⌘) does not move the window; it is left for IA-CORE-17.
5. The host settings page (IA-CORE-06) provides "Move the window only by the title bar", off by default; when on, dragging on the video area does not move the window.

**Implementation constraints**: the host calls winit's `drag_window()`.

**Cross-platform**: `drag_window()` is implemented on both macOS (`performWindowDragWithEvent`) and Windows (`WM_NCLBUTTONDOWN` + `HTCAPTION`); on Windows such a drag triggers the system's window snapping. On other systems where winit does not support it, the host ignores the error.

**Acceptance**:

1. In windowed mode, hold the left button on the video and drag: the window follows, and stays at the new position after release.
2. Bind `Mouse:LeftClick` to "Play/Pause", then drag the window: it does not pause after release.
3. Drag 2 px and release: handled as a click (if bound).
4. Drag in fullscreen: the window does not move; click once right after the drag: it does not enter fullscreen.
5. On Windows, drag the window to the left edge of the screen: the system snaps it to half the screen.
6. Turn on "Move the window only by the title bar": dragging on the video area does not move the window (with IA-CORE-06).

### G-10 OSD and info block font size scales with the video area (P1)

**Status**: implemented. The switch in item 4 belongs to IA-CORE-06 and is not implemented; the behaviour is fixed at automatic.

**Requirements**:

1. OSD font size = `clamp(round(video area height × 3%), 12px, 48px)`, bold. Examples: default window (video area about 609 high) 18 px; fullscreen at 1080 high 32 px; 12 px when the video area is very short.
2. White text (semi-bold) on a translucent dark rounded plate; the plate's padding and corner radius scale with the font size (at 18 px: 12 left and right, 5 top and bottom, radius 8), with a 2 px gradient line along the bottom edge (UI specification UI-OSD-01); left margin 10 px. Before the 2026-10 visual identity round it was a black outer outline with no plate.
3. Info block font size = `clamp(11px × font size / 16px, 9px, 28px)`, padding in the same proportion; the info block is 4 px below the OSD plate and moves with the font size.
4. The host settings page (IA-CORE-06) provides an "automatic font size" switch, on by default; when off, a fixed font size is used (adjustable, default 16).

**Cross-platform**: computed in logical pixels, consistent between Windows high DPI and macOS Retina.

**Acceptance**: press ↑ in the default window: the OSD is about 18 px; press ↑ in fullscreen (1080p screen): about 32 px; drag to 320×182: about 12 px and not beyond the video area; press Tab: the info block is below the OSD plate and its font size changes in proportion.

### G-13a The volume keys stop at 100 (P1)

**Status**: implemented.

**Requirements**:

1. `volume-up` (↑, wheel up): when the current volume is below 100, add 5, with the result at most 100; when the current volume is already ≥ 100 (another plugin set it above 100 with `setOutput`), it does not increase further, and the OSD shows "Volume: N%" as usual.
2. `volume-down`: subtract 5, not below 0 (150 → 145).
3. The range of `player.setOutput` is still 0–200, reserved for a later "volume boost" option.

**Cross-platform**: no difference.

**Acceptance**: at 95% volume press ↑ twice: 100%, 100%; scrolling the wheel up also stops at 100%.

### G-16a The background layer takes effect while loading (P1)

**Status**: implemented.

**Requirements**:

1. When the source phase is `loading`, the background layer's `effective` is `true`: the video area no longer shows the last frame of the previous source.
2. On `error`, the previous value is kept.
3. What the background layer draws is decided by the background layer plugin (G-16b); without a background layer plugin, the video area is the solid colour `Theme.bg-video` while loading.
4. It takes effect immediately, with no delay (when a local file opens quickly, the loading screen flashes briefly).
5. Picture-in-picture uses a different rule: while loading it keeps the previous value, and when the source changes the small window does not close because of "no video".

**Cross-platform**: no difference.

**Acceptance**: while playing video A, open network link B: before B shows a picture, the video area no longer stays on A's last frame and shows the loading screen (with visualizer) or a solid colour (without); change the source in picture-in-picture: the small window stays.

### IA-CORE-03 Window size memory distinguishes "normal size" from "maximised" (P2)

**Status**: implemented.

**Requirements**:

1. The window position and size are saved only when the window is neither fullscreen nor maximised.
2. On startup the window always opens at normal size; maximised is not restored.
3. Restoring from maximised returns to the normal size from before maximising (system behaviour).
4. After closing a file the window keeps its current size.

**Cross-platform**: Slint's `is_maximized` corresponds to "zoom" on macOS and to `IsZoomed` on Windows, both provided by winit.

**Acceptance**: maximise the window (on macOS double-click the title bar or Option-click the green button), quit and restart: the window has the size and position from before maximising.

### IA-CORE-04 Fullscreen entries: Esc, Enter / Leave Fullscreen, fullscreen+, fullscreen on another display (P2 / P3)

**Status**: partially implemented. Item 1 is implemented; items 2 and 3 are not implemented.

**Requirements**:

1. (P2) The only default binding of the host's "Toggle Fullscreen" is `Escape` (user ruling 2026-11-01): Esc toggles, so in windowed mode an Esc that no menu, dialog or text field handles enters fullscreen. Up to 0.5.x the defaults were `Enter`, `Alt+Enter`, `F11` and Esc was "Leave Fullscreen"'s, doing nothing in windowed mode; until host 0.5.1 a double click toggled too (user ruling 2026-10-26: a left click on the video is Play/Pause, and the user can bind the double click under Settings ▸ Mouse…). "Enter Fullscreen" (`host/enter-fullscreen`, enabled only in windowed mode) and "Leave Fullscreen" (`host/exit-fullscreen`, enabled only in fullscreen) are separate host commands with no default key and no menu row, bindable in Keyboard Shortcuts. The shortcut text of the menu's "Toggle Fullscreen" row shows its first key, `Esc`. Plugins toggle it with `host.run("fullscreen")` (player 0.5.2), the same as the key.
2. (P3) Host command "fullscreen+" Ctrl+Enter: enters fullscreen and at the same time stretches the picture to fill the screen (aspect ratio not kept); pressing it again during stretched fullscreen exits fullscreen. Menu position after "Toggle Fullscreen", group `screen`, order 701.
3. (P3) Host command "fullscreen on another display" Ctrl+Shift+Enter: moves the main window to the next display and then enters fullscreen; disabled when there is only one display, with the reason "only one display".

**Implementation constraints**: stretching uses mpv's `keepaspect=no` (restored when fullscreen ends); changing displays uses winit's `available_monitors()` and `set_outer_position`.

**Cross-platform**: on macOS ⌥↩ and ⌃↩ are not taken by the system; Ctrl bindings also fire with ⌘ on macOS. macOS fullscreen creates a new space, so changing displays must first exit fullscreen, move the window, then enter again.

**Acceptance**: Esc enters fullscreen in windowed mode and leaves it in fullscreen; Esc in an open menu, a dialog or a text field only closes or cancels it; Ctrl+Enter makes the picture fill the screen with no black bars; with two displays, Ctrl+Shift+Enter goes fullscreen on the other screen.

### IA-CORE-05 Windowed-mode "auto-hide the control bar" switch (P3)

**Status**: not implemented.

**Requirements**: host command "Screen ▸ Auto-hide the control bar in windowed mode" (checkable, off by default; order 720, group `screen-options`). When on, windowed mode behaves like fullscreen: the bottom bar overlays the bottom of the video, hides after 3000 ms idle, appears on activity, and the G-04 hover lock applies as well. Only the bottom bar is hidden, not the title bar. The host saves the choice in the UI state file.

**Implementation constraints**: `window_state::Policy` gains `auto_hide_windowed`, and the overlay and idle checks become `fullscreen || auto_hide_windowed`; the bottom bar position and height conditions in composition are replaced the same way.

**Cross-platform**: no difference.

**Acceptance**: when checked, play in windowed mode and keep the pointer still for 3 seconds: the bottom bar disappears and the video fills the window; move the pointer: the bottom bar appears overlaid on the bottom of the video.

### IA-CORE-06 Host settings page (P2)

**Status**: not implemented.

**Requirements**: at the very front of the "General" category of the Options window, add the host's own "Interface" page (lowest order), containing:

| Option | Default | Item |
| --- | --- | --- |
| Move the window only by the title bar | Off | G-05 |
| In fullscreen, pop out the attached panel even when it is closed in windowed mode | Off | G-01 |
| Don't auto-hide the pointer over the control bar | On | G-04 |
| Show OSD | On | O-07 |
| OSD position: horizontal (left / centre / right), vertical (top / centre / bottom), horizontal margin, vertical margin | Left, top, 10, 10 | O-02 |
| OSD automatic font size / fixed font size | Automatic; 16 when fixed | G-10 |

When "Show OSD" is off, `osd.show` resolves as usual but nothing is drawn. The choices are saved in the UI state file.

**Implementation constraints**: the host page is implemented with the host's own Slint components and sits in the Options window in the same place as plugin pages, without going through the plugin API; the page set in `settings_window.rs` gains a "host page" kind. The alternative, opening a set of host option interfaces to the preferences plugin, would add interfaces that belong only to the host, and is not adopted.

**Core API**: none new. **Cross-platform**: no difference.

**Acceptance**: the first page of "General" in the Options window is "Interface"; toggling each option changes the corresponding behaviour, and the choices persist after a restart.

### IA-CORE-07 Custom-drawn title bar (P3)

**Status**: not implemented.

**Requirements**:

1. The main window does not use the system title bar; the host draws a 31 px title bar at the top of the content. The parts of the title bar outside the buttons can be dragged to move the window, and a double-click maximises / restores (Windows) or zooms (macOS) following system convention.
2. No main menu button; the menu entry is still the right button on the video area. Right-clicking the title bar does not pop up a menu.
3. File type: the short name of the current source's container format (such as `MKV`, `MP4`; network links show `URL`), hidden when the window is narrower than 300; when there is a file type, a vertical separator sits between it and the title.
4. Title: the current source name; when too long it scrolls left at 1 px / 50 ms, pauses for 1 second at the end and starts again from the beginning.
5. Buttons on the right:
   - **Windows**: from right to left, close, fullscreen, maximise / restore (one of the two), minimise, always-on-top state, with 10 between buttons.
   - **macOS**: the system red, yellow and green buttons stay at the left end (system controls, responsible for fullscreen, the tiling menu, zoom and other system behaviour), followed by the file type and the title; on the right only the always-on-top state and fullscreen buttons are drawn.
6. Always-on-top state button: the icon shows one of 4 kinds depending on the current always-on-top mode; a click fires the binding input `Title:OnTop` (IA-WIN-07). When no plugin declares that binding, the button shows as disabled.
7. The title bar is not shown in fullscreen: in a macOS fullscreen space the system shows the menu bar when the pointer moves to the top; on Windows it is hidden in fullscreen.
8. The minimum window height increases by 31 px accordingly (IA-CORE-13).
9. The title bar colour does not change when the window loses focus.
10. The colours of the four icon states (normal, hover, pressed, disabled) follow the current theme of the UI specification.

**Implementation constraints**:

- A host-provided `TitleBar` component is inserted at the top of the root layout in `host.slint`; the host writes the title and file type properties. Pressing the `TouchArea` on the empty part of the title bar calls `drag_window()` (as in G-05).
- Windows: after `with_decorations(false)` the host handles border resizing itself (winit `drag_resize_window(direction)`, with a 4 px resize hot zone on each of the four edges), minimise / maximise (`set_minimized`, `set_maximized`), and the Windows 11 snap layout menu when hovering the maximise button (the platform module must make `WM_NCHITTEST` return `HTMAXBUTTON`; if that cannot be done, no snap layout menu is offered and everything else works as usual).
- macOS: the system title bar is made transparent and the content extends under the title bar (`titlebarAppearsTransparent`, `NSFullSizeContentViewWindowMask`, `titleVisibility = hidden`); the system buttons are still drawn by the system. Done only in `platform/macos.rs`.
- Other systems keep the system title bar, no custom title bar is drawn, and `Title:OnTop` never fires.

**Core API** [New core API]:

```ts
// Binding input (one more item in the "input" syntax of reference §4.1)
"Title:OnTop"          // click on the always-on-top state button of the custom title bar; cannot take modifiers

// Main window (reference §6.6)
setAlwaysOnTop(onTop: boolean, options?: { mode?: "never" | "always" | "playing" | "fullscreen" }): Promise<void>;
// mode is used only for the title bar icon; when omitted, the icon shows the two kinds "always / never" according to onTop
```

Without an API change: the always-on-top button shows only on / off, and a click pops up the "Screen ▸ Always on Top" submenu. Cost: the icon does not show the four modes, and a click takes one more step. Another option is for the host to take over the always-on-top modes itself, but "While Playing / While Playing in Fullscreen" is the window plugin's business, and moving it into the core violates "the core does not carry plugin business". So the request is made: `Title:OnTop` is only one more input name in the binding table, following the existing conflict and rebinding rules; `mode` is an optional parameter and does not affect existing calls.

**Acceptance**:

1. Windows: title bar 31 px; from the left, file type and title; from the right, close, fullscreen, maximise, minimise, always-on-top state, spacing 10; the four edges can be dragged to resize; dragging the title bar to the top of the screen maximises.
2. macOS: the red, yellow and green buttons are at the left end, followed by the file type and title; on the right are the always-on-top state and fullscreen; the empty part of the title bar can be dragged; a double-click zooms according to the system setting.
3. Open a file with a very long name: the title scrolls slowly to the left.
4. Drag the window below 299 wide: the file type is hidden.
5. Click the always-on-top state button: the always-on-top mode cycles in the window plugin's order, and the icon changes with it (IA-WIN-07).

### IA-CORE-08 Completing binding inputs: mouse double-click, push-and-hold, side-button wheel, and Scroll Lock (P3)

**Status**: not implemented. Existing inputs are in reference §4.1: `Mouse:LeftClick`, `Mouse:LeftDoubleClick[:LeftHalf|:RightHalf]`, `Mouse:MiddleClick`, `Mouse:X1`, `Mouse:X2`, the four wheel directions; the right button is reserved for the menu.

**Requirements**:

1. New input names: `Mouse:RightDoubleClick`, `Mouse:MiddleDoubleClick`, `Mouse:X1DoubleClick`, `Mouse:X2DoubleClick`, `Mouse:LeftPush`, `Mouse:RightPush`, `Mouse:MiddlePush`, `Mouse:X1Push`, `Mouse:X2Push`, `Mouse:X1+WheelUp` / `WheelDown`, `Mouse:X2+WheelUp` / `WheelDown`; key name `ScrollLock`. All can take modifiers. This only adds the capability, with no default bindings.
2. "Push": held for 500 ms after pressing without release, with the pointer moving no more than 4 px.
3. The double-click check follows the rules of `Mouse:LeftClick`: only when some double-click input has an active binding is the click of the corresponding button delayed until the double-click interval ends; without a binding the behaviour is unchanged.
4. Right button: when `Mouse:RightDoubleClick` has a binding, the menu pops up only after the double-click interval ends following the right-button release; a second press counts as a double-click and does not pop up the menu. Without a binding it pops up immediately on release per G-15. When `Mouse:RightPush` has a binding, it fires as soon as the hold reaches the threshold, and that release does not pop up the menu.
5. When `X1` / `X2` have corresponding "side button + wheel" bindings, the wheel while the side button is held goes to those bindings, and releasing the side button no longer fires the `X1` / `X2` click.

**Implementation constraints**: input parsing and matching in `bindings.rs`; the video area in `host.slint` handles press / release separately for each button; the host's pending click is generalised to per-button pending clicks and hold timers. The mouse page of the hotkeys plugin lists them too (IA-HK-01).

**Core API** [New core API]: the binding syntax gains the input names above (reference §4.1); no method signature changes. Only the host receives mouse events on the video area, so plugins have no alternative.

**Cross-platform**: Scroll Lock: Windows keyboards have it; Mac keyboards usually do not (F14 on the Apple extended keyboard is reported by winit as `F14`), so on macOS `ScrollLock` usually never fires, and the related commands are run from the menu (IA-INFO-02). Side buttons are reported by winit as `Back` / `Forward` on both systems.

**Acceptance**: in the shortcut settings bind "middle double-click" to "Toggle Fullscreen": a middle double-click toggles fullscreen, and a middle click (if bound) fires only after the double-click interval; bind "right push" to "Play/Pause": holding the right button for half a second pauses, and the release does not pop up the menu; without these bindings the right button behaves as in G-15.

### IA-CORE-09 Control panel (cancelled)

**Status**: cancelled (§0.3). Picture Adjustments, Audio Workbench and the subtitle workbench stay three separate tool windows; no F7 control panel is provided, and no `plugin.controlPanel` interface is added.

### IA-CORE-10 Title row and snapping for floating windows (P3)

**Status**: not implemented. Current state: the floating list window uses the system title bar with only a close button; it does not snap.

**Requirements**:

1. The top of a floating window is a 34 px title row drawn by the host; on the right, in order: an opacity slider (20%–100%), an attach button (same as `setDetached(false)`), a pin (keeps only this floating window on top), and close (same as `setVisible(false)`). The detach / attach button on the panel's own tab strip stays.
2. Snapping: while dragging a floating window or the main window, when a window edge comes within 10 px of the edge of the screen work area or of another player window's edge, it snaps into alignment.
3. Floating windows do not follow when the main window moves.

**Implementation constraints**: floating windows change to no system title bar (Windows) / a transparent title bar (macOS), and the title row is generated by the host. Snapping is computed when the host handles window move events (winit `Moved`) and applied with `set_outer_position`; on macOS changing the position during a system drag may be overridden by the system, and if that cannot be done it falls back to "snap on release".

**Core API**: none new. The pin applies only to the floating window and does not go through `setAlwaysOnTop`.

**Cross-platform**: the built-in snapping on Windows only covers screen edges, so snapping between windows is computed by the host; macOS does not snap, so it is also computed by the host. Window level and opacity go in the platform modules.

**Acceptance**: detach the list: the top of the floating window has opacity, attach, pin and close; dragging the opacity slider makes the window transparent; click the pin, then click another application: the floating window stays on top while the main window is not on top; drag the floating window near the right edge of the main window: it snaps flush.

### IA-CORE-11 Minimising minimises all windows together (P3)

**Status**: not implemented.

**Requirements**: when the main window is minimised, the floating sidebar and tool windows are hidden with it; when the main window is restored they come back with it (only those visible before minimising).

**Cross-platform**: Windows uses the owner window relationship (owned window), so minimising the main window automatically hides the owned windows; macOS child windows (`addChildWindow`) would make them move with the main window, which is not suitable, so the host minimises them one by one. Both in the platform modules.

**Acceptance**: detach the list, open Picture Adjustments, minimise the main window: all three windows disappear; restore: all three come back.

### IA-CORE-12 Double-clicking the video area during picture-in-picture (P2)

**Status**: implemented.

**Requirements**: during picture-in-picture, the video area of the main window shows the grey text "Playing in picture-in-picture"; a double-click on the video area is the same as the small window's "return": it closes the small window and the picture returns to the main window, without pausing or switching to fullscreen.

**Cross-platform**: picture-in-picture currently exists only on macOS (reference §6.6); other systems never enter this state.

**Acceptance**: on macOS enter picture-in-picture and double-click the video area of the main window: the small window closes, the picture returns to the main window, it does not enter fullscreen and does not pause.

### IA-CORE-13 Minimum window height (P3)

**Status**: not implemented. Current state: the main window minimum is 320×182 (bottom bar 62 plus a video area of at least 120).

**Requirements**: the main window minimum width is 320; the video area has no minimum height, and the minimum window height = title bar (31 once IA-CORE-07 lands, 0 before) + bottom bar 62. The video height lower bound of `setVideoArea` changes to 1. When the video area height is 0, render output pauses (no zero-size texture is created), and playback continues.

**Core API**: none new (the lower bound change of `setVideoArea` is written into reference §6.6).

**Cross-platform**: Windows has no extra minimum height for borderless windows; macOS with the system title bar approach has its own system minimum height, and the system value wins.

**Acceptance**: the window can be dragged down to just the height of the bottom bar (plus the title bar); playback continues, and the picture returns after enlarging.

### IA-CORE-14 Embedded closed caption tracks (P3)

**Status**: not implemented.

**Requirements**: the host turns on mpv's `sub-create-cc-track=yes` for every source, so CEA-608/708 closed captions in the video stream appear as an ordinary embedded subtitle track in `subtitles.state.tracks` (`codec` is `eia_608` or `eia_708`), not selected by default.

**Core API**: none new (`SubtitleTrack` already has a `codec` field). **Cross-platform**: an mpv option, no difference.

**Acceptance**: open a TS file with CEA-608 captions: the subtitle menu has one more closed caption track, not shown by default.

### IA-CORE-15 Main window interface extensions: maximise, position, aspect ratio lock, active state (P3)

**Status**: not implemented. Plugin side in IA-WIN-05, IA-WIN-06, IA-PB-07.

**Core API** [New core API]:

```ts
interface WindowState {
  // …existing fields
  readonly maximized: boolean;
  readonly active: boolean;      // the main window (or any window of this application) is the active window
  readonly minimized: boolean;
  readonly frame: { x: number; y: number; width: number; height: number }; // screen coordinates, logical points
}
setMaximized(mode: "off" | "maximized" | "cover"): Promise<void>;  // cover = maximise covering the taskbar / Dock
moveBy(dx: number, dy: number): Promise<{ x: number; y: number }>; // logical points; the result is clamped to the visible screen area
setAspectLock(ratio: number | null): Promise<void>;                 // keep the video area aspect ratio while the user drags to resize; null turns it off
```

Without an API change: "window +/−" can simply scale with the existing `setVideoArea` (IA-WIN-05 does exactly that). "Maximise", "move window", "keep ratio" and "pause on focus loss / minimise" all need the host's window state or window operations, and plugins have no alternative. Only part of it can be accepted: the maintainer is advised to accept the read-only `active` and `minimized` first and schedule the rest as needed.

**Cross-platform**: `cover` on Windows is an ordinary window at fullscreen size (without switching to fullscreen mode), and on macOS an ordinary window filling the whole screen (including the menu bar area); `maximized` corresponds to "zoom" on macOS. `frame` uses global coordinates with multiple monitors. In the platform modules.

**Acceptance**: see the plugin items.

### IA-CORE-16 Download rate in playback statistics (P3)

**Status**: not implemented.

**Core API** [New core API]:

```ts
interface PlaybackStats {
  // …existing fields
  cacheSpeed: number | null;   // bytes/second, from mpv cache-speed; null for local files
}
```

Plugins cannot read mpv properties, so there is no alternative; this is one more field in the existing "mpv property as is" category of `stats()`, a compatible extension.

**Cross-platform**: an mpv property, no difference.

**Acceptance**: while playing a network link, `stats().cacheSpeed` is a positive number; for a local file it is `null`.

### IA-CORE-17 Modifier drags on the video area (P3)

**Status**: not implemented. Plugin side in IA-VID-02, IA-SUB-02.

**Requirements**: a left-button drag with a modifier (Alt / Ctrl / Shift) on the video area goes to the plugin that registered that modifier; when no plugin has registered it, nothing happens (the window does not move, G-05 item 4).

**Core API** [New core API]:

```ts
// Needs the new capability input.videoDrag
plugin.input.onVideoDrag(
  modifiers: "alt" | "ctrl" | "shift",
  listener: (e: { phase: "start" | "move" | "end"; dx: number; dy: number; videoArea: { width: number; height: number } }) => unknown,
): Disposable;   // dx/dy is the offset since start (logical points); for the same modifier the first registration wins
```

Without an API change: make "Alt+drag" a discrete binding input (for example firing a command every 10 px of movement). Cost: commands take no parameters, so the drag can only move discretely in fixed steps, with poor feel and precision, and up, down, left and right need four commands. Both users are off by default; the maintainer is advised to schedule this last and, if necessary, downgrade directly to discrete bindings.

**Cross-platform**: on macOS Ctrl+left button is the same as the right button in the system, so `ctrl` uses ⌘ on macOS (consistent with the binding table's "Ctrl also accepts ⌘").

**Acceptance**: see IA-VID-02, IA-SUB-02.

### Core API requests in this chapter

Changes only to host behaviour or semantics, with no API change: G-01 (`SlotState.reason: "fullscreen"` semantics), G-04, G-03, G-15, IA-CORE-01, G-05, G-10 (reference §6.5), G-13a (§5.5), G-16a (§6.3), IA-CORE-03, IA-CORE-04, IA-CORE-05, IA-CORE-06, IA-CORE-10, IA-CORE-11, IA-CORE-12, IA-CORE-13 (§6.6 lower bound), IA-CORE-14.

[New core API] request drafts (none submitted yet):

| Signature | Reason (workflow §4.3 step 4) | Alternatives considered first | Source |
| --- | --- | --- | --- |
| `CommandOptions.mnemonic?: string` | The host draws the menu and handles its keys | The host takes a letter from the title automatically (none for Chinese titles) | IA-CORE-02 |
| Binding input `Title:OnTop` | The title bar is host UI | A click pops up the "Always on Top" submenu | IA-CORE-07 |
| `window.setAlwaysOnTop(onTop, options?: { mode? })` | The title bar icon must show 4 always-on-top modes | Show only on / off | IA-CORE-07 |
| Binding inputs `Mouse:RightDoubleClick`, `Mouse:MiddleDoubleClick`, `Mouse:X1DoubleClick`, `Mouse:X2DoubleClick`, `Mouse:LeftPush`, `Mouse:RightPush`, `Mouse:MiddlePush`, `Mouse:X1Push`, `Mouse:X2Push`, `Mouse:X1+WheelUp/Down`, `Mouse:X2+WheelUp/Down`, `ScrollLock` | Only the host receives mouse events on the video area | None | IA-CORE-08 |
| `WindowState.maximized`, `.active`, `.minimized`, `.frame` | Host window state | None | IA-CORE-15 |
| `window.setMaximized(mode)` | Only the host can perform window operations | None | IA-CORE-15 |
| `window.moveBy(dx, dy)` | Same as above | None | IA-CORE-15 |
| `window.setAspectLock(ratio \| null)` | Resizing by user drag is handled by the system and the host | None | IA-CORE-15 |
| `PlaybackStats.cacheSpeed` | mpv property | None | IA-CORE-16 |
| `plugin.input.onVideoDrag(modifiers, listener)`; capability `input.videoDrag` | Only the host receives drags on the video area | Discrete bindings (poor feel), possible downgrade | IA-CORE-17 |

---

## Playback control (`playback`)

**Location**: `src/plugins/playback/` (bottom bar `ui/bar.slint`, `bar.ts`, settings `settings.ts`)

### G-07 Hover tooltips on the bottom bar buttons (P1)

**Status**: implemented (`tips.ts`, `bar.ts`, `ui/bar.slint`).

**Requirements**: every button and clickable label has a hover tooltip (`ToolTipArea` from `@vestra/widgets`, appearing after 600 ms and shown again on every hover), with the text including the currently active shortcut:

| Element | Tooltip text |
| --- | --- |
| Play / pause | "Play (Space)" / "Pause (Space)" |
| Stop | "Stop (F4)" |
| Back | "Back 5 seconds (←)\nRight-click: Play previous (PgUp)" |
| Forward | "Forward 5 seconds (→)\nRight-click: Play next (PgDn)" |
| Open | "Open file (Ctrl+O)" |
| Current time | "Click to show the remaining time" / "Click to show the elapsed time" |
| Mute | "Mute (M)" / "Unmute (M)" |
| Volume bar | "Volume: N%" (the actual value, can exceed 100) |
| Playlist toggle | "Playlist (F6)" |

1. The number of seconds for back / forward follows the first tier setting of B06.
2. Key names come from the `labels` of `plugin.bindings.list()` (the manifest has `bindings.manage`, read only): the command's first keyboard binding not taken by another command; reread on `bindings.onChange`; the default key is used when nothing can be read. After the user rebinds, the tooltip changes with it.
3. The playlist toggle has no command of its own and takes the keys of the top-level menu item "Playlist"; when no plugin registers it, the tooltip shows only "Playlist".
4. Whether the shortcut text shows ⌘ or Ctrl on macOS follows the menu's rules (labels generated by the host); the plugin does not assemble it itself.

**Acceptance**: rest the pointer on each button for 0.6 seconds: the tooltips in the table above appear; move away and hover the same button again: the tooltip appears again; after changing play / pause to K in the shortcut settings, the tooltip becomes "Play (K)".

### G-13b The volume slider maps to 0–100 (P1)

**Status**: implemented.

**Requirements**: the slider range is 0–100, and dragging to the far right is 100%; at the default 100% the slider is at the far right end. When the current volume is > 100 (set by another plugin), the slider stays at the far right, the hover tooltip shows the actual value ("Volume: 150%"), and after a drag the dragged value is set (≤ 100). No notification while dragging; on release the OSD shows "Volume: N%" once.

**Acceptance**: on first launch (volume 100%) the volume slider is at the far right; drag it to the middle and release: OSD "Volume: 50%".

### G-14 The time display toggle is saved, and the click area is only the current time (P1)

**Status**: implemented (settings key `remainingTime`).

**Requirements**:

1. Clicking the current time toggles between "elapsed / remaining"; the result is saved as `remainingTime: boolean` (default `false`) in playback's `settings.json` and persists after a restart. Remaining time shows as `-MM:SS / total`.
2. Only the current time text is clickable, and the pointer shows a hand over it; the slash and the total duration are not clickable.
3. Time format: `MM:SS` under 1 hour, otherwise `H:MM:SS`; `--:-- / --:--` when idle.

**Acceptance**: click the current time to switch to remaining time and restart the player: remaining time is still shown; click the total duration: nothing happens; the pointer is a hand over the current time.

### G-18 State of the bottom bar sidebar toggle when the sidebar cannot be seen (P2)

**Status**: implemented. Since player 0.5.10 the toggle is the player's (`host/sidebar` in the bar's command strip, UI-BAR-09) and the player computes this state (the strip's `dimmed` cell); before that, the playback bridge property `sidebar-hidden`.

**Requirements**: the button has three appearances:

| State | Condition | Appearance |
| --- | --- | --- |
| Off | `preferred === false` | Accent icon, no dot (player 0.5.22; before: normal colour) |
| On | `preferred && effective`, or `reason === "fullscreen"` | Accent icon with the dot under it (0.5.22; before: accent icon) |
| On but not visible | `preferred && !effective && reason === "space"` | Icon and dot at 50% opacity; tooltip "Toggle Sidebar (the window is too narrow for the sidebar)", with the key when the user bound one |

In fullscreen, `reason === "fullscreen"` means the overlay is retracted (G-01) and becomes visible when the pointer moves to the right edge, so it shows as "On". When the window is narrower than 638 (video 318 + divider 3 + sidebar 317), the sidebar is hidden for lack of space and the preference is unchanged.

**Acceptance**: with the list open, drag the window narrower to 600: the list disappears, the bottom bar button (icon and dot) becomes half-lit, and the hover tooltip explains why; drag it wider: the list comes back and the button is fully lit.

### IA-PB-01 Seek bar hover time is off by default (P2)

**Status**: implemented.

**Requirements**: the seek bar hover time tooltip (`hoverTime`) defaults to `false` and can be turned on with the menu "Playback ▸ Show Time on Seek Bar Hover". Users who have already saved settings keep their value. The "title  time" tooltip when hovering near a bookmark / chapter (within 3 px) is not affected by this switch.

**Acceptance**: delete playback's `settings.json`, start, and rest the pointer on the seek bar: no time tooltip; the menu "Playback ▸ Show Time on Seek Bar Hover" is unchecked.

### IA-PB-02 Hiding rules for the right-hand buttons in a narrow window (P3)

**Status**: superseded in part by the command strip (player 0.5.10, UI-BAR-10): its cells hide from the left end inward, the sidebar toggle last, and never overlap the time.

**Requirements**: each right-hand button has its own minimum left edge position (list toggle 320, attached panel toggle 320, stream selection 350, CC 410, see IA-PB-06); a button is hidden when its left edge x is less than that value, so as the window narrows the right-hand buttons disappear from left to right; in addition, a button is also hidden when there is less than 8 px between its left edge and the right edge of the time text (including badges, IA-PB-05), so they never overlap.

**Acceptance**: play a video longer than 1 hour and drag the window from 800 down to 320: the right-hand buttons disappear from left to right and never overlap the time text at any width.

### IA-PB-03 A disabled slider is shown when seeking is not possible (P3)

**Status**: not implemented. Current state: when seeking is not possible the whole slider is hidden.

**Requirements**: when seeking is not possible (idle, live stream) the slider is still shown at the current position (at the far left when idle), coloured `Theme.accent-disabled`, and does not respond to the pointer; the played part uses `icon-disabled`.

**Acceptance**: when idle, the left end of the bottom bar seek bar has a dark slider, and clicking it does nothing.

### IA-PB-04 Shapes of the A-B and bookmark marks on the seek bar (P3)

**Status**: not implemented. Current state: marks are drawn as 1×6 vertical lines, and the A-B range as a colour band with 1×8 vertical lines at both ends.

**Requirements**: points A and B are drawn as 6×10 triangles (A pointing right, B pointing left), bookmarks as 6×10 flags, and chapters stay vertical lines; the colour is the theme's text colour with a black outline. The hover hit range is unchanged (3 px).

**Acceptance**: set an A-B loop and add a bookmark: two triangles and one flag appear on the seek bar.

### IA-PB-05 Info badges (P3)

**Status**: not implemented.

**Requirements**:

1. After the time text, badges are shown in order, each only when the main window is at least the threshold width:

   | Badge | Content | Main window width ≥ |
   | --- | --- | --- |
   | Decoding | `HW` / `SW` (HW when `stats().video.hwdec` is not `"no"`) | 460 |
   | HDR | Shows `HDR` when `video.transfer` is `pq` or `hlg` | 700 |
   | Video codec | Upper-case short name of `video.codec` (such as `H264`, `HEVC`) | 630 |
   | Audio codec | Upper-case short name of `audio.codec` | 670 |
   | Channels | `audio.channels` plus "ch" (such as `2ch`, `6ch`) | 690 |
   | Download rate | `stats().cacheSpeed` (IA-CORE-16), such as `1.2 MB/s`; not shown for local files | 740 |

2. Style: height 14, left and right padding 5, spacing 5 between badges, 11 px text, secondary text colour; on hover the background brightens and the text is the accent colour.
3. A badge is not shown when its data is missing. Refreshed once per second, polling only while there is a source.
4. Badges have no click action (switching HW / SW changes rendering options and belongs to the video plugin; the channel menu belongs to the audio plugin); they only show a hover state.

**Implementation constraints**: `bar.ts` calls `player.stats()` on a timer; the main window width is computed from `window.state.videoArea` and the sidebar width and passed into `bar.slint`.

**Core API**: the download rate badge depends on IA-CORE-16; nothing else new.

**Cross-platform**: Windows currently uses software rendering, and `hwdec` uses copy-back mode (`auto-copy-safe`, such as `d3d11va-copy`), so the badge shows `SW`, reflecting the facts.

**Acceptance**: at a window width of 800, play an HEVC 5.1-channel HDR source: `HW` `HDR` `HEVC` `EAC3` `6ch` are shown in order; drag narrower to 650: only `HW` `HEVC` remain; drag to 450: all disappear.

### IA-PB-06 Completing the right-hand bottom bar buttons: CC, stream selection, attached panel (P3)

**Status**: not implemented. Current state (player 0.5.10): the right end is the player's command strip (UI-BAR-09): contributed buttons ([AI][Store]) and the sidebar toggle (41×40, 4 px from the right). The control panel toggle is cancelled together with IA-CORE-09. When this item is taken up, CC and stream selection become commands with `button` (no separate 23×22 buttons), so they follow the strip's order and narrowing rules.

**Requirements**: from left to right on the right side: CC, stream selection, playlist toggle, attached panel on / off; 8 between the 23×22 buttons.

1. **CC**: shown when the current source's subtitle tracks include a closed caption track (`codec === "eia_608"` or `"eia_708"`, IA-CORE-14); a click selects / deselects that track (`plugin.subtitles.select(track)` / `select(null)`); pressed state while that track is selected. The "last shown track" remembered by subtitles' "Show Subtitles" (Alt+H) may be out of sync with the CC button's choice; Alt+H restores the track it remembers itself.
2. **Stream selection**: shown when the current source has 2 or more audio tracks or any subtitle track; a click pops up a single-level menu above the button: the audio track list (radio), a separator, and the subtitle track list (radio, including "subtitles off"). Implemented with commands without `path` registered by this plugin plus `Panel.showMenu`: audio tracks use `player.selectAudioTrack`, subtitles use `subtitles.select`; rebuilt when the tracks change.
3. **Playlist toggle**: appearance per G-18.
4. **Attached panel on / off** (41×40, flush right, with a 1 px separator on its left): same as `sidebar.toggle()`; shows the "close" icon when the sidebar is attached and visible, otherwise the "open" icon.
5. Hiding of each button in a narrow window follows IA-PB-02.

**Implementation constraints**: the manifest adds the `subtitles.read` and `subtitles.control` capabilities.

**Core API**: depends on IA-CORE-14 (closed caption tracks); the rest uses the existing API.

**Acceptance**: play an MKV with two audio tracks and subtitles: the stream selection button appears, a click pops up the two groups of audio and subtitles, and choosing the second audio track switches the sound; play a TS with CEA-608: the CC button appears, a click shows the closed captions and the button turns pressed; the attached panel button has the same effect as F6.

### IA-PB-07 Pause on focus loss, pause on minimise (P3)

**Status**: not implemented.

**Requirements**: playback's settings page (the "Playback" category) gets two checkboxes, both off by default. When on: pause when none of this application's windows is the active window; pause when minimised. Regaining focus / restoring does **not** resume automatically. It is recorded only when playing and paused by this rule, to avoid resuming by mistake.

**Core API**: depends on `WindowState.active` and `minimized` from IA-CORE-15.

**Cross-platform**: "active" on both systems means "any window of this application is the foreground window".

**Acceptance**: turn on "pause on focus loss", and during playback click another application: it pauses; switch back: it stays paused.

### IA-PB-08 Control panel "Playback" page (cancelled)

**Status**: cancelled (together with IA-CORE-09).

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| `PlaybackStats.cacheSpeed` (IA-CORE-16) | IA-PB-05 |
| `WindowState.active`, `WindowState.minimized` (IA-CORE-15) | IA-PB-07 |

The other items use only the existing API.

---

## Playlist (`playlist`)

**Location**: `src/plugins/playlist/`

### G-08 The selected item and the playing item scroll into view automatically; edge auto-scroll while dragging (P1)

**Status**: implemented (`scroll.ts`, `main.ts`, `ui/sidebar.slint`).

**Requirements**:

1. After the keyboard moves the selection (↑↓, Home / End, Shift extend, ↓ from the search box back to the list, Ctrl+↑ / ↓ / Home / End move), the row with the keyboard cursor is fully visible: if it is above the view, scroll until it is the first row; if below, until it is the last row (minimal scrolling).
2. When the playing item changes (auto-advance, next, double-click to play), if it is not in view, scroll so it is centred; if it is in view, do not move. When the list has a search filter and that item is filtered out, do not scroll.
3. During drag reordering, when the pointer enters within 20 px of the top / bottom edge of the list area, scroll in that direction at one row every 50 ms, faster the closer to the edge (at most three rows every 50 ms). The dragged row is recorded when the drag starts (row elements may be reused while the list scrolls).
4. Within 2 seconds after the user scrolls by themselves (wheel or dragging the scroll bar), changes of the playing item do not trigger scrolling.
5. PgUp / PgDn with focus in the list are still the global "previous / next", not list paging.

**Acceptance**: with a 200-item list, select item 1 and press ↓ until item 40: the view keeps following; press End: it scrolls to the bottom; play item 1 and press PgDn several times up to item 60: item 60 scrolls to the middle; drag item 5 to the bottom edge of the list and hold: the list scrolls down.

### G-11 Dropping on the picture: clear the list first, then add and play (P1)

**Status**: implemented.

**Requirements**:

1. Dropping on the video area (no modifier): expand the dropped content (rules for folders and list files in feature requirements A15); when at least one item can be added, clear the list first, then add in the dropped order and play the first item. When no item can be added, the list is unchanged.
2. When only **one** local media file is dropped, it is expanded according to the "When Opening a File" mode and the similar files rule (IA-PL-01), and the dropped file is played; multiple files, folders and list files are not expanded.
3. No confirmation before clearing ("Confirm Before Removing All" only applies to the "Remove All" command); no "list cleared" notification when clearing.
4. When the setting "Append when opening or dropping; don't clear the list" (IA-PL-02, off by default) is on: append to the end and play the first new item.
5. Dropping with Option held (Alt on Windows): append only; no clearing, no playing, no expanding.
6. While the player is opening another source (`busy`): do not clear, only append, OSD "opening, added to the list".
7. Dropping subtitles or other files that playlist does not claim: playlist is not involved and the list is unchanged.

**Cross-platform**: Option / Alt is already unified by `DropModifiers.alt`.

**Acceptance**: with 5 items in the list, drop 2 mkv files on the video area: only these 2 items remain in the list and the first plays; turn on the "append" setting and drop again: the list becomes 7 items; drop with Option / Alt held: appended and not played.

### G-12 Dropping on the list: append to the end by default (P1)

**Status**: implemented (settings key `dropAtPointer`).

**Requirements**:

1. Dropping on the list (attached or floating): append to the end, no playing, no clearing.
2. When the setting "Insert at the drop position when dropping on the list" (IA-PL-02, off by default) is on: insert before the row at the drop point (a 2 px insertion line shows the position).
3. Drag reordering inside the list is not affected.

**Acceptance**: with 5 items in the list, drop 2 files on row 2: they are appended as items 6 and 7; turn on "insert at the drop position" and drop on row 2 again: they are inserted before the original row 2.

### IA-PL-01 Opening a local file clears the list and adds similar files (P1)

**Status**: implemented (`siblings` / `similarStep` / `isSimilar` in `media.ts`, `follow.ts`, `main.ts`, `store.ts`).

**Requirements**:

1. "Playback ▸ Playlist ▸ When Opening a File ▸", one of three: Open Only That File / Add Similar Files (default) / Add All Files in the Folder.
2. When another plugin opens a **local file** (trigger condition as in feature requirements A13 item 1): clear the list first, then add according to the mode, and the opened file becomes the current row (takeover). In "Open Only That File" mode the cleared list has only this item, which it takes over.
3. When the setting "Append when opening or dropping; don't clear the list" (IA-PL-02, off by default, shared with G-11) is on: back to the "append without duplicates" rule; in "Open Only That File" the list is not touched.
4. "Open ▸ Open Folder…" follows the same setting: by default clear first, then add and play the first item; do not clear while another source is opening.
5. Opening network links or generated streams leaves the list unchanged.
6. List files (`.m3u` etc.) opened with "Open List File…" replace the list, unchanged.
7. No confirmation before clearing. When "Remember List" is on, the new list after clearing is saved as usual.
8. When the plugin starts (or is re-enabled), if the file being played is already in the restored list, it is only taken over, not cleared.
9. When more than 5000 similar files match, 5000 are taken centred on the opened file, OSD "Added N files from the same folder, limit of 5000 reached" (N does not include the opened file itself).
10. The settings file carries the version number `settings.version = 2`; when an older file below 2 is read, `openMode` and `appendOnOpen` are migrated once to the current defaults (similar files, no append), and the other settings are kept. The old field `clearOnOpenFolder` is ignored when read.

#### Similar files rule

Applies only when opening a **single** local file (opening several files at once does not expand; for drops see G-11 item 2). The complete algorithm and constants are in Appendix B; `media.ts` implements Appendix B, and the constants and integer division must not be changed.

1. Input: the opened file path P; mode `similar | folder` ("Open Only That File" does not call this rule).
2. Split: `ext` is the part after the last `.`, converted to lower case; `stem` is the file name without the extension, keeping the original case.
3. Grouping: if `ext` is in the audio extension set, the audio group; otherwise, if it is in the video extension set, the video group; if neither, return only `[P]`. The sets follow feature requirements Appendix B.5 (`VIDEO`, `AUDIO`), matched exactly on the whole extension.
4. Precondition for similar mode: `stem` contains at least one space, `-`, `_` or ASCII `0-9`, and its length is ≥ 2; otherwise return `[P]`. Full-width digits and Chinese numerals do not count as digits.
5. Enumerate the folder containing P, not recursively, files only (symbolic links to files count too), not directories. Check each entry:
   - skip names starting with `.` (macOS `._*` metadata files);
   - skip P itself;
   - the candidate must be in the same group as P: audio only matches audio, video only matches video;
   - "all" mode: same group means included. "similar" mode: included only when `isSimilar(a, b, audio)` is true, where `a = asciiLower(stem) + ".tmp"` and `b = asciiLower(candidate stem) + ".tmp"`. `asciiLower` only converts A–Z to lower case; `.tmp` must be appended, and all length thresholds count it.
6. Add P itself to the result and sort by comparing file names with `Intl.Collator("zh", {numeric: true, sensitivity: "base"})` (natural order, case-insensitive).
7. There is no count limit; truncation happens only when more than 5000 match (item 9).
8. The opened file becomes the current row; files sorted before it are in the list too.

**Test cases** (17 groups, in `tests/similar.test.ts`; "Opened" is the current row; the list is sorted per item 6; the default mode is similar)

| # | Opened | Folder contents | Expected list (in order) | Must not be added | Step passed |
| --- | --- | --- | --- | --- | --- |
| 1 | `Show.S01E02.1080p.mkv` | `Show.S01E01.1080p.mkv` `Show.S01E02.1080p.mkv` `Show.S01E10.1080p.mkv` `Show.S02E01.1080p.mkv` `Show.S01E03.720p.mp4` `Show.S01E01.1080p.srt` `Other.Movie.2019.mkv` | `Show.S01E01.1080p.mkv` `Show.S01E02.1080p.mkv` `Show.S01E03.720p.mp4` `Show.S01E10.1080p.mkv` `Show.S02E01.1080p.mkv` | `.srt` (not media), `Other.Movie.2019.mkv` | E01, E03: S1; E10, S02E01: S3 equal |
| 2 | `第02集.mp4` | `第01集.mp4` `第02集.mp4` `第10集.mp4` `第01集.mkv` `花絮.mp4` | `第01集.mkv` `第01集.mp4` `第02集.mp4` `第10集.mp4` | `花絮.mp4` | S3 equal; mkv and mp4 are both in the video group |
| 3 | `[字幕组] 进击的巨人 - 02 [1080p].mkv` | `[字幕组] 进击的巨人 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 02 [1080p].mkv` `[字幕组] 进击的巨人 - 13 [1080p].mkv` `[字幕组] 鬼灭之刃 - 01 [1080p].mkv` `[其他组] 进击的巨人 - 03 [720p].mkv` | `[字幕组] 鬼灭之刃 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 01 [1080p].mkv` `[字幕组] 进击的巨人 - 02 [1080p].mkv` `[字幕组] 进击的巨人 - 13 [1080p].mkv` | `[其他组] 进击的巨人 - 03 [720p].mkv` | 01: S1; 13: S3 equal; 鬼灭之刃: S3 prefix |
| 4 | `A02.mkv` | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` `song.mp3` | `A01.mkv` `A02.mkv` `A03.mkv` | `B.mkv`, `song.mp3` | S3 equal |
| 5 | `A02.mkv` (**All** mode) | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` `song.mp3` `notes.txt` | `A01.mkv` `A02.mkv` `A03.mkv` `B.mkv` | `song.mp3` (different group), `notes.txt` | Group only |
| 6 | `movie.mkv` | `movie.mkv` `movie2.mkv` `trailer.mkv` | `movie.mkv` | `movie2.mkv`, `trailer.mkv` | Precondition not met |
| 7 | `ep1.mp4` | `ep1.mp4` `ep2.mp4` `EP3.MP4` `ep4.avi` `ep10.mp4` `ep5.mp3` | `ep1.mp4` `ep2.mp4` `EP3.MP4` `ep4.avi` `ep10.mp4` | `ep5.mp3` | S3 equal (case-insensitive) |
| 8 | `Ep 001.mkv` | `Ep 001.mkv` `Ep 002.mkv` `Ep 010.mkv` `Ep 100.mkv` `Ep 1000.mkv` | All, in the original order | None | 002: S1; the rest: S3 equal (different digit counts) |
| 9 | `01 - Intro.mp3` | `01 - Intro.mp3` `02 - Song Two.mp3` `03 - Another.flac` `10 - Outro.mp3` `Interview.mp3` `cover.jpg` | `01 - Intro.mp3` `02 - Song Two.mp3` `03 - Another.flac` `10 - Outro.mp3` | `Interview.mp3`, `cover.jpg` | S4 audio |
| 10 | `Vlog_2023-05-01.mp4` | `Vlog_2023-05-01.mp4` `Vlog_2023-05-02.mp4` `Vlog_2024-12-31.mp4` `Holiday_2023-05-01.mp4` | `Vlog_2023-05-01.mp4` `Vlog_2023-05-02.mp4` `Vlog_2024-12-31.mp4` | `Holiday_2023-05-01.mp4` | S1 |
| 11 | `Lecture 1.mp4` | `Lecture 1.mp4` `Lecture 2.mp4` `lecture 3.mp4` `Lecture 12.mp4` `Lab 1.mp4` | `Lecture 1.mp4` `Lecture 2.mp4` `lecture 3.mp4` `Lecture 12.mp4` | `Lab 1.mp4` | S1 |
| 12 | `Friends.S01E01.mkv` | `Friends.S01E01.mkv` `Friends.S01E02.mkv` `Friends.Special.mkv` `Friends.S01E01.ass` `Friends.S01E03.zip` | `Friends.S01E01.mkv` `Friends.S01E02.mkv` `Friends.Special.mkv` | `.ass`, `.zip` | S1 (`Special` also counts as similar) |
| 13 | `西游记01.rmvb` | `西游记01.rmvb` `西游记02.rmvb` `西游记25.rmvb` `红楼梦01.rmvb` | `西游记01.rmvb` `西游记02.rmvb` `西游记25.rmvb` | `红楼梦01.rmvb` | S3 equal |
| 14 | `abc-01.mkv` | `abc-01.mkv` `abc-02.mkv` `abd-01.mkv` `xyz-01.mkv` | `abc-01.mkv` `abc-02.mkv` `abd-01.mkv` | `xyz-01.mkv` | S1 |
| 15 | `Show.S01E01.mkv` | `Show.S01E01.mkv` `Show.S01E02.mkv` `Shower.Cam.mkv` `Showtime.S01E01.mkv` | `Show.S01E01.mkv` `Show.S01E02.mkv` `Shower.Cam.mkv` | `Showtime.S01E01.mkv` | E02: S1; Shower.Cam: S4 mask |
| 16 | `第０２集.mp4` (full-width digits) | `第０１集.mp4` `第０２集.mp4` `第１０集.mp4` | `第０２集.mp4` | The other two | Precondition not met |
| 17 | `ab1.mkv` | `ab1.mkv` `ab2.mkv` `ac1.mkv` `b1.mkv` | `ab1.mkv` `ab2.mkv` | `ac1.mkv`, `b1.mkv` | S3 equal (short names must be identical) |

The unit tests compare both the result of `siblings` and, one by one, the step returned by `similarStep` (`S1` / `S2` / `S3 equal` / `S3 prefix` / `S4 audio` / `S4 mask`) against "Step passed" in the table above; they also test the precondition, the `.tmp` length boundary (group 17: `ab1.tmp` has 7 characters in total, 7/2 = 3 < 5, so it does not enter S1; `ac1` is also judged not similar in S3 and S4 for insufficient length), "digit against non-digit" at position n in the mask being judged not similar, skipping names starting with `.` and folders, and truncation at 5000.

**Acceptance**:

1. The list has 3 items and the folder contains `A01 A02 A03 B`: open `A02` with the bottom bar "Open file": the list becomes `A01 A02 A03`, `A02` is the current row, and `A03` plays automatically when it ends.
2. After changing the mode to "Open Only That File", open `B` with the bottom bar: only `B` remains in the list, as the current row.
3. Turn on the "append" setting and open `A01` again: appended without duplicates, existing rows kept.
4. Open a network link: the list is unchanged.
5. "Open Folder…" and choose a folder: the list is replaced with the contents of that folder and the first item plays.

### IA-PL-02 Playlist settings page and F5 with focus in the list (P2)

**Status**: implemented (`ui/settings.slint`; the manifest has `ui.settings`, page id `playlist`, category "Playback", order 30).

**Requirements**:

1. The "Options ▸ Playback ▸ Playlist" page:
   - When Opening a File: Open Only That File / Add Similar Files (default) / Add All Files in the Folder (drop-down);
   - Append when opening or dropping; don't clear the list (off by default; covers opening files, opening folders and dropping on the picture);
   - Insert at the drop position when dropping on the list (off by default);
   - Repeat (drop-down), Shuffle, Fast Advance to Next, Remember List, Confirm Before Removing All (checkboxes).
   The page and the existing checkable items in the menu stay in sync both ways.
2. With focus in the list, F5 opens this page (the list command "Playlist Settings…", not in the menu, `focus: "panel"`); without focus in the list, F5 is still the global "Options…".
3. After manual play (double-click, Enter, P), focus stays in the list.

**Cross-platform**: on Apple keyboards F5 needs fn.

**Acceptance**: with focus in the list press F5: the Options window opens on the "Playlist" page; change "When Opening a File" on the page, and the menu radio dot follows.

### IA-PL-03 Focus frame for the keyboard cursor (P3)

**Status**: not implemented. Current state: selected rows have a background colour; the keyboard cursor and the selection are combined.

**Requirements**: when the list has focus, the row with the keyboard cursor gets a 1 px dashed frame (the theme's secondary text colour), regardless of whether it is selected; when the list loses focus it is not drawn.

**Acceptance**: select row 3, then Ctrl+click to deselect it: row 3 still has the dashed frame; press ↓: the frame moves to row 4.

### IA-PL-04 Icon column in list rows (P3)

**Status**: not implemented. Current state: "number. title", with the duration right-aligned.

**Requirements**: after the number and before the title, draw a 12×12 type icon of three kinds: video, audio and network link (video / audio by extension, links by `kind === "url"`). When the title comes from the file name, the extension is kept.

**Acceptance**: mix video, audio and links in the list: the three icons differ.

### Core API requests in this chapter

None. This chapter uses only the existing API.

---

## Window (`window`)

**Location**: `src/plugins/window/`

### G-02 When playback starts, resize the window to 1× the video size (P1)

**Status**: implemented (`window/settings.ts`, `main.ts`; settings keys `autoSize`, `autoSizeOnce`).

**Requirements**:

1. Setting "Window size when playback starts": Don't Resize / 0.5× / 1× (default) / 1.5× / 2× / Fit to Video Aspect Ratio. The menu "Screen ▸ When Playback Starts ▸" has six radio items (order 716–721, group `screen-auto`); the settings page "Options ▸ Playback ▸ Window" (order 30) offers the same.
2. Setting "Only Once" (off by default): the first source with video in this run uses up the one time (even if it was not resized because of fullscreen or similar). Menu "Screen ▸ When Playback Starts ▸ Only Once" (checkable, order 722, group `screen-auto-once`).
3. Trigger: each new source (`sourceId` changes) is resized once, when it first gets a video size (read `stats()` after `ready`, retrying every 500 ms at most 10 times). Replaying, pausing or seeking within the same source does not trigger. A source already playing when the plugin activates is treated as a new source.
4. No resizing when: fullscreen; mini size in effect; in picture-in-picture; audio only or no video; a plugin-generated stream with no size.
5. The size rules are the same as for the manual multipliers (feature requirements H03): the target is the video area, the core adds the bottom bar and the attached sidebar, keeps the top-left corner, shrinks to the screen when it does not fit and enlarges to the minimum size when too small.
6. Automatic resizing shows no OSD; nothing is shown when it is shrunk to the screen either. Changing the setting from the menu shows "When playback starts: 1×", "When playback starts: Don't resize the window", "Resize only once when playback starts: On / Off"; changing it on the settings page shows nothing; changing the setting does not resize the current window immediately, it takes effect with the next source.
7. If, after the source is ready and before the size is known, the user changes the window manually (in windowed mode the video area changes by more than 1 point; fullscreen toggling and picture-in-picture do not count), this source is no longer resized automatically.

**Cross-platform**: on Windows `setVideoArea` computes the visible area from the whole monitor (it may overlap the taskbar, reference §6.6); this is a known difference in the core.

**Acceptance**:

1. With default settings open a 1280×720 source (screen 2560×1440): the video area becomes 1280×720, and the top-left corner of the window does not move.
2. Then open a 640×480 source: the video area becomes 640×480.
3. Open a 3840×2160 source: it shrinks to fit the screen, with no notification.
4. Open an mp3: the window is unchanged.
5. In fullscreen, switch to the next video through the list: no resizing; after exiting fullscreen the window has its size from before fullscreen.
6. Set "Don't Resize": the window is unchanged when opening files; set "Only Once": only the first video resizes it.

### G-06 Hiding the pointer while playing is on by default (P1)

**Status**: implemented (settings key `hideCursorWhilePlaying` defaults to `true`).

**Requirements**: on by default (menu "Playback ▸ Hide Pointer While Playing", settings page "Options ▸ Playback ▸ Window"). Hide conditions (reference §6.6 `setCursorAutoHide("playing")`): playing, the pointer is over the video area, 3 seconds without activity, no menu or dialog, and the main window is the active window. Not hidden while paused; not hidden while the pointer is over the bottom bar or the pop-out. Users who have already saved settings keep their value.

**Acceptance**: delete window's `settings.json`, start, play in windowed mode and keep the pointer still over the video for 3 seconds: it disappears; the menu "Playback ▸ Hide Pointer While Playing" is checked.

### IA-WIN-01 `` ` `` is "Mini Size", picture-in-picture is a separate command (P2)

**Status**: implemented.

**Requirements**:

1. "Screen ▸ Mini Size" (checkable, `` ` ``, id `mini`, order 710, group `screen`) exists on every system: the video area shrinks to the largest same-ratio size that fits into 400×300, keeping the top-left corner, and pressing it again restores. Disabled without video and in fullscreen. Always-on-top rules in IA-WIN-02.
2. "Screen ▸ Picture-in-Picture" (checkable, id `pip`, order 709, group `screen`, **no default key**) is registered only while `pictureInPicture.supported` is true and unregistered when it becomes false; disabled without video (it can still exit when already in picture-in-picture).
3. The two are mutually exclusive: when entering picture-in-picture while mini size is in effect, the original size is restored first; running mini size during picture-in-picture first exits picture-in-picture, and if exiting fails (notification "Cannot exit picture-in-picture: <reason>"), mini size is not entered. While either switch is in progress, each marks the other as busy.
4. The user's rebinding of `` ` `` applies to mini size.

**Cross-platform**: mini size uses only `setVideoArea`, the same on both systems; picture-in-picture exists only on macOS.

**Acceptance**: on macOS press `` ` ``: the window shrinks to mini size (without entering picture-in-picture); enter picture-in-picture from "Screen ▸ Picture-in-Picture"; on Windows the menu has no "Picture-in-Picture", and `` ` `` behaves the same as on macOS.

### IA-WIN-02 Mini size is not on top (P2)

**Status**: implemented.

**Requirements**: mini size only changes the window size; whether it is on top is decided entirely by the always-on-top mode (IA-WIN-03). Entering / exiting mini size does not call `setAlwaysOnTop`.

**Acceptance**: with the always-on-top mode "Never", press `` ` ``: the window becomes small, and after clicking Finder / File Explorer the player is covered.

### IA-WIN-03 Four always-on-top modes, default "While Playing in Fullscreen" (P2)

**Status**: implemented (uses only the existing `setAlwaysOnTop(boolean)`).

**Requirements**:

1. "Screen ▸ Always on Top ▸" has four modes (radio): Never, Always, While Playing, While Playing in Fullscreen (order 729). The default is "While Playing in Fullscreen". The settings page "Options ▸ Playback ▸ Window" also has these four radio options.
2. "While Playing in Fullscreen": on top when `window.state.fullscreen && phase === "ready" && !paused`, otherwise not; recomputed when fullscreen toggles. `setAlwaysOnTop` is called only when the value changes.
3. Ctrl+T cycles: Never → Always → While Playing → While Playing in Fullscreen → Never.
4. OSD: "Always on top: While playing in fullscreen".
5. Users who have already saved settings keep their value.
6. Modeless tool windows are not put on top on their own; they go on top together when the whole is on top.

**Cross-platform**: macOS fullscreen is a separate space where the main window is in front anyway, so "While Playing in Fullscreen" has no visible effect on macOS; it is still offered and is the default, to keep the settings consistent on both systems. On Windows, fullscreen is an always-on-top borderless window, so other windows that pop up do not cover the player.

**Acceptance**: on first launch the radio dot of the menu "Screen ▸ Always on Top" is on "While Playing in Fullscreen"; on Windows, while playing in fullscreen, open a program that brings itself to the foreground: the player stays on top; not on top in windowed mode. Ctrl+T four times returns to the original mode.

### IA-WIN-04 Lower and upper bounds when resizing to the video size (P3)

**Status**: not implemented. Current state: only two rules, shrinking to the available screen area and enlarging to the minimum size (reference §6.6).

**Requirements**: the "Window" settings page gets two groups of options (both disabled by default): "at least N% of the desktop" and "at most N% of the desktop" (N from 10 to 100). When enabled, G-02's automatic resizing and the manual multipliers clamp the video area size by these two percentages before calling `setVideoArea` (computed from the visible screen area, keeping the aspect ratio). The desktop size comes from the display information of `plugin.app.system()`.

**Acceptance**: enable "at most 50% of the desktop", then on a 2560×1440 screen press 4 (2×) while playing 1080p: the video area does not exceed 1280×720.

### IA-WIN-05 More window size presets: maximise, window +/−, window move (P3)

**Status**: not implemented. Current state: 0.5 / 1 / 1.5 / 2× and fit to aspect ratio.

**Requirements**:

1. "Screen ▸ Enlarge Window / Shrink Window": the video area grows / shrinks by 10%, using the existing `setVideoArea`.
2. "Screen ▸ Maximise / Maximise+" (radio, can be deselected): Maximise does not cover the taskbar / Dock, Maximise+ covers it; uses `setMaximized("maximized" | "cover")`.
3. "Screen ▸ Move Window ▸ Up / Down / Left / Right": moves 10 points each time, Ctrl+Alt+numpad 8 / 2 / 4 / 6; uses `moveBy`.
4. No "custom size".
5. All disabled in fullscreen, with the reason "Not available in fullscreen" (the same as the existing multiplier commands).

**Core API**: depends on `setMaximized`, `moveBy` and `WindowState.maximized` from IA-CORE-15 (item 1 does not).

**Cross-platform**: "Maximise+" covers the menu bar and Dock area on macOS and the taskbar on Windows.

**Acceptance**: press "Enlarge Window": the video area grows by 10%; Ctrl+Alt+Num6: the window moves right; after "Maximise+" the window covers the taskbar / Dock.

### IA-WIN-06 Keep the ratio when resizing the window (P3)

**Status**: not implemented.

**Requirements**: "Screen ▸ Keep Ratio When Resizing the Window" (checkable, off by default); when on and the user drags the window border, the video area keeps the display aspect ratio of the current video; no lock without video. When the source changes, the lock is reset to the new video.

**Core API**: depends on `setAspectLock` from IA-CORE-15.

**Cross-platform**: macOS uses `NSWindow.contentAspectRatio` (the host subtracts the fixed sizes of the bottom bar and sidebar), Windows corrects the rectangle in `WM_SIZING`; both in the platform modules.

**Acceptance**: when checked, play a 16:9 video and drag the bottom-right corner of the window: the video area never has black bars.

### IA-WIN-07 Title bar always-on-top button (P3)

**Status**: not implemented (with IA-CORE-07).

**Requirements**: the default bindings of the "cycle always-on-top mode" command (Ctrl+T) add `Title:OnTop`; every time the always-on-top mode changes, call `setAlwaysOnTop(wanted, { mode })` so the title bar shows one of the 4 icons (even when whether it is actually on top has not changed, for example "While Playing" during a pause).

**Core API**: depends on `Title:OnTop` and the `mode` option of `setAlwaysOnTop` from IA-CORE-07.

**Acceptance**: click the title bar always-on-top icon four times: the OSD shows the four modes in turn, and the icon changes with them.

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| `window.setMaximized(mode)`, `window.moveBy(dx, dy)`, `WindowState.maximized` (IA-CORE-15) | IA-WIN-05 |
| `window.setAspectLock(ratio \| null)` (IA-CORE-15) | IA-WIN-06 |
| Binding input `Title:OnTop`; `setAlwaysOnTop(onTop, { mode })` (IA-CORE-07) | IA-WIN-07 |

G-02, G-06 and IA-WIN-01–04 use only the existing API.

---

## Playback info (`info`)

**Location**: `src/plugins/info/`

### IA-INFO-01 Brief info includes the subtitle name and chapter (P2)

**Status**: implemented (`info/main.ts`; the manifest has `subtitles.read`).

**Requirements**: Shift+Tab shows the brief info for 3 seconds:

1. First line: the file name.
2. Second line: `elapsed / total · current chapter · video codec size · audio codec channels`. The chapter comes from `player.chapters()` (the core's chapter table, read once per source): the last chapter whose start time is not later than the current position; "Chapter N" when it has no title; omitted when there are no chapters.
3. Third line: the name of the current subtitle track (title; if empty, the language; if that is empty too, "Subtitles"); this line is not shown when no subtitle is selected. Subtitle selection has no push event, so while shown, `subtitles.refresh()` runs once per second together with `stats()`.
4. Each line is truncated to the video area width: the number of columns is estimated as `(video area width − 32) / 6.5`, at least 44 columns.

**Acceptance**: play an MKV with chapters and subtitles and press Shift+Tab: the file name is shown; `12:34 / 45:00 · Chapter 2 title · H.264 1920×1080 · AAC 2 channels`; the subtitle name.

### IA-INFO-02 Misc info (Scroll Lock) and "show only in fullscreen" (P3)

**Status**: not implemented.

**Requirements**:

1. Command "Misc Info" (checkable, bound to `ScrollLock`, menu position per the info group in menus.md): when on, one line is shown permanently in the video area: `12:34 / 45:00 (27%) · file name · chapter title`.
2. The playback number (which item in the list) is not shown: the play sequence is provided by playlist, and the info plugin cannot read it. Doing it later needs the core to provide the sequence position in `PlayerState`, with a separate request at that time.
3. Setting "show only in fullscreen" (off by default, on the info settings page).
4. Display position: uses the info plugin's existing info block (`osd.panel()`); when it shares the block with the full Tab info, it goes at the top. Refreshed every second.

**Core API**: the shortcut depends on the `ScrollLock` key name from IA-CORE-08; the menu entry does not.

**Cross-platform**: Mac keyboards usually have no Scroll Lock; on macOS it is toggled from the menu.

**Acceptance**: on Windows press Scroll Lock: the misc info line appears in the video area, press again and it disappears; with "show only in fullscreen" on it is not shown in windowed mode and is shown in fullscreen.

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| Binding key name `ScrollLock` (IA-CORE-08) | IA-INFO-02 |

---

## Album cover and idle screen (`visualizer`)

**Location**: `src/plugins/visualizer/`

### G-16b Show the loading screen while loading (P1)

**Status**: implemented (`mode: "loading"` in `visualizer/main.ts`, `LoadingScene` in `ui/backdrop.slint`).

**Requirements**:

1. When `player.state.phase === "loading"` and the background layer is in effect, draw the loading screen: a 74×74 spinner in the centre of the video area (centre moved up 30 px), 12 dots, advancing one step every 50 ms, the current step brightest and the following ones progressively dimmer; below it (moved down 30 px) one line of bold 15 px text "Opening: <source name>". Both dots and text use `Theme.text-dim`.
2. Source name: the file name for local files; "host/last path segment" for links, only the host when there is no path; truncated with "…" beyond 60 characters, and elided further by width in Slint.
3. The loading screen shows immediately, without fading in; the logo and cover still fade out in 200 ms. Reloading the same source (`sourceId` unchanged) also shows the loading screen.
4. After entering `ready`, the existing rules apply (picture → background layer not in effect; audio only → cover). On entering `error`, it returns to what the plugin showed before loading started (logo, cover or solid colour), and the error notification (OSD) explains.
5. The loading screen is mutually exclusive with the idle logo and the cover, and is not affected by the two switches "Show Logo When Idle" and "Show Album Cover for Audio Only".

**Core API**: none new; depends on G-16a (`effective` is `true` during `loading`).

**Acceptance**: open a network link: before the picture appears, the centre of the video area has a spinner and "Opening: …"; it disappears once the picture appears.

### Core API requests in this chapter

None (depends on the behaviour of G-16a).

---

## Shortcut settings (`hotkeys`)

**Location**: `src/plugins/hotkeys/`

### IA-HK-01 The mouse page lists the new mouse inputs (P3)

**Status**: not implemented (with IA-CORE-08). Current state: `keys.ts` lists 11 kinds of mouse input.

**Requirements**: the mouse page adds: right double-click, middle double-click, back button double-click, forward button double-click, left push, right push, middle push, back button push, forward button push, wheel up / down while holding the back button, wheel up / down while holding the forward button. The descriptions of right double-click and right push note "when bound, the context menu pops up after release and waiting for the double-click interval" (IA-CORE-08 item 4).

**Core API**: depends on IA-CORE-08.

**Acceptance**: on the mouse page "middle double-click" can be selected and bound to "Toggle Fullscreen", and the binding takes effect.

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| New mouse binding inputs (IA-CORE-08) | IA-HK-01 |

---

## Picture adjustments (`video`)

**Location**: `src/plugins/video/`

### IA-VID-01 Control panel "Video" page (cancelled)

**Status**: cancelled (together with IA-CORE-09). Picture Adjustments stays a separate tool window.

### IA-VID-02 Alt / Ctrl + drag adjusts the picture (P3, off by default)

**Status**: not implemented.

**Requirements**: setting "Adjust the picture with Alt / Ctrl + drag" (off by default). When on: Alt + drag pans the picture (the panning of feature requirements E13), Ctrl (⌘ on macOS) + drag zooms the picture (drag up to zoom in). Shift + drag is not bound.

**Core API**: depends on IA-CORE-17 (`plugin.input.onVideoDrag`); if the maintainer rejects it, downgrade to discrete binding commands (one step every 10 px).

**Acceptance**: with the setting on, hold Alt and drag on the video: the picture pans along; with the setting off, the same operation: no panning (moving the window by dragging without a modifier is not affected).

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| `plugin.input.onVideoDrag` (IA-CORE-17) | IA-VID-02 |

---

## Audio workbench (`audio`)

**Location**: `src/plugins/audio/`

### IA-AUD-01 Control panel "Audio" page (cancelled)

**Status**: cancelled (together with IA-CORE-09). Audio Workbench stays a separate tool window.

---

## Subtitles (`subtitles`)

**Location**: `src/plugins/subtitles/`

### IA-SUB-01 Control panel "Subtitles" page (cancelled)

**Status**: cancelled (together with IA-CORE-09). The subtitle workbench stays a separate tool window.

### IA-SUB-02 Ctrl + drag moves the subtitles (P3, off by default)

**Status**: not implemented.

**Requirements**: setting "Move the subtitles with Ctrl + drag" (off by default). When on, Ctrl (⌘ on macOS) + vertical drag changes the vertical position of the subtitles (`marginPercent`), and on release the OSD shows "Subtitle position: N%". Subtitles use only Ctrl and picture panning uses only Alt, so they do not conflict when both are on (for the same modifier the first registration wins, IA-CORE-17).

**Core API**: depends on IA-CORE-17; the downgrade is the same as IA-VID-02.

**Acceptance**: with the setting on, Ctrl / ⌘ + drag up: the subtitles move up.

### Core API requests in this chapter

| Signature | Source |
| --- | --- |
| `plugin.input.onVideoDrag` (IA-CORE-17) | IA-SUB-02 |

---

## Chapters, bookmarks and ranges (`marks`)

**Location**: `src/plugins/marks/`

### IA-MK-01 A-B loop block on the control panel "Playback" page (cancelled)

**Status**: cancelled (together with IA-CORE-09). A-B loops use the commands and menus of feature requirements B23.

---

## Appendix A: state × interaction element, cell by cell

Vestra's specification written cell by cell as "state × interaction element". One value per cell; where the main text has an item, the "Item" column gives its number, and the main text wins. Cells marked "not implemented" describe the target; the current state is in the "Status" of the corresponding item in the main text.

### A.1 High-priority items (G-01–G-18)

| No. | State | Element | Specification | Item |
| --- | --- | --- | --- | --- |
| G-01 | Fullscreen | Attached playlist | Pops out as an overlay when the pointer moves within 4 px of the right edge of the screen, retracts 500 ms after leaving or after 3 seconds idle; F6 pops out / retracts immediately | G-01 |
| G-02 | Windowed × opening a file | Window size | Each new source resizes the window to 1× the video size; options Don't Resize / 0.5 / 1.5 / 2× / fit to aspect ratio, Only Once | G-02 |
| G-03 | All, especially small windows | Context menu | Scrolls when it does not fit, and every item can be clicked; target: the menu can extend beyond the main window and is positioned by the screen work area (not implemented) | G-03 |
| G-04 | Fullscreen | Bottom bar auto-hide | Does not hide while the pointer is on the bottom bar | G-04 |
| G-05 | Windowed | Dragging on the picture | Moves the window | G-05 |
| G-06 | Windowed × playing | Mouse pointer | Hidden while playing, on by default | G-06 |
| G-07 | All | Bottom bar buttons | All have hover tooltips with the current shortcut; back / forward state the right-click function | G-07 |
| G-08 | List | Scrolling | Scrolls into view automatically on keyboard moves and when the playing item changes; edge auto-scroll while dragging | G-08 |
| G-09 | Audio only | Main window layout | Keeps the video layout, the background layer shows the album cover; window size and bottom bar unchanged; no audio-specific layout, no separate audio / video window sizes | §0.3 |
| G-10 | All | OSD font size | Follows the video area height: `clamp(round(height × 3%), 12px, 48px)` | G-10 |
| G-11 | Windowed | Dropping on the picture | Clear the list first, then add and play | G-11 |
| G-12 | Windowed | Dropping on the list | Append to the end | G-12 |
| G-13 | All | Volume range | UI and keys 0–100; `setOutput` 0–200 | G-13a, G-13b |
| G-14 | Windowed | Time display toggle | Only the current time is clickable; the choice is saved | G-14 |
| G-15 | All | Context menu | Pops up on right-button release; submenus open at once on hover, with submenu aim | G-15 |
| G-16 | Loading | Video area | Shows the loading screen (spinner and "Opening: …") | G-16a, G-16b |
| G-17 | Windowed | Floating playlist | Host title row (opacity, attach, pin, close), 10 px snapping (not implemented) | IA-CORE-10 |
| G-18 | Windowed | Narrow window and sidebar | Below a width of 638 the sidebar is hidden, the preference is unchanged, and the bottom bar list button is half-lit | G-18 |

### A.2 Window state × UI element

"Layout" means whether it takes window space (squeezing the video) or overlays the video.

**Windowed mode**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| W-01 | Bottom bar layout | Below the video, taking height, not covering the picture | |
| W-02 | Bottom bar width | Only below the video, not extending under the playlist | |
| W-03 | Bottom bar auto-hide | Not hidden by default; optional "auto-hide the control bar in windowed mode" (not implemented) | IA-CORE-05 |
| W-04 | Playlist display | Shown and attached on the right by default, full height, with a 3 px divider from the video | |
| W-05 | Window change when opening the playlist | The window width is unchanged and the video is squeezed | |
| W-06 | Title bar | Currently the system title bar showing the source name; target: a 31 px custom title bar (not implemented) | IA-CORE-07 |
| W-07 | Appearance when the window loses focus | Title bar and bottom bar colours unchanged | IA-CORE-07 |
| W-08 | OSD | Top-left of the video area, margin 10, white text on a translucent dark rounded plate with a gradient line along the bottom; font size follows the video area | G-10 |
| W-09 | Pointer | Hidden while playing after the pointer rests 3 seconds over the video area (on by default) | G-06 |
| W-10 | Minimum size | Currently 320×182; target width 320, with the height determined only by the title bar and bottom bar (not implemented) | IA-CORE-13 |
| W-11 | Size and position on first launch | 780×671, position decided by the system | |
| W-12 | Restoring the last window on startup | Restores the normal size and position; maximised is not restored | IA-CORE-03 |
| W-13 | Snapping to screen edges | Snaps within 10 px while dragging the window (not implemented); Windows system snapping works as usual | IA-CORE-10 |
| W-14 | Window size after closing a file | Keeps the current size | |

**Fullscreen**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| F-01 | Bottom bar layout | Overlays the bottom of the video; the video is not scaled | |
| F-02 | Revealing the bottom bar | Only the pointer in the bar's band (its laid-out height at the bottom edge, left of a popped-out sidebar); keys, motion elsewhere, wheel, clicks, menus, dialogs and entering fullscreen do not reveal it | FS-BAR-1 |
| F-03 | Bottom bar hide delay | 500 ms after the pointer leaves the band (a bar lock or a menu or dialog opened while shown holds it) | FS-BAR-1 |
| F-04 | Show and hide animation | Appears immediately, disappears immediately | |
| F-05 | Pointer resting on the bottom bar | The bottom bar does not hide | G-04 |
| F-06 | Bottom bar while paused | The same: shows only while the pointer is in the band | FS-BAR-1 |
| F-07 | Attached playlist | Pops out from the right edge | G-01 |
| F-08 | Floating playlist | Stays a separate window; the right edge does not trigger | G-01 |
| F-09 | Pointer | Hidden after 3 seconds idle while playing; not hidden while paused; not hidden over the bottom bar or the pop-out (nor in the bar's band); independent of the bar | G-06, FS-BAR-1 |
| F-10 | Title bar | Not shown (on macOS the system shows the menu bar when the pointer moves to the top) | IA-CORE-07 |
| F-11 | OSD and info layer | Same as windowed mode; misc info can be set to "show only in fullscreen" (not implemented) | IA-INFO-02 |
| F-12 | Entering fullscreen | Esc toggles (user ruling 2026-11-01; Enter, Alt+Enter and F11 up to 0.5.x; a double click only when the user binds it); Enter / Leave Fullscreen without a default key; Ctrl+Enter stretched fullscreen, Ctrl+Shift+Enter fullscreen on another display (not implemented) | IA-CORE-04 |
| F-13 | Always on top | Default "While Playing in Fullscreen" | IA-WIN-03 |
| F-14 | Window size commands | Disabled, with the reason "Not available in fullscreen" | |
| F-15 | Other displays | System behaviour, not filled with black | |

**Maximised**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| X-01 | Layout | Same as a normal window | |
| X-02 | Kinds of maximise | The system's maximise / zoom; additional "Maximise" and "Maximise+" commands (not implemented) | IA-WIN-05 |
| X-03 | Memory | The window size is not recorded while maximised | IA-CORE-03 |

**Mini size, always on top, picture-in-picture**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| P-01 | `` ` `` key | Mini size (all systems) | IA-WIN-01 |
| P-02 | Always on top in mini size | Not on top; depends only on the always-on-top mode | IA-WIN-02 |
| P-03 | UI in mini size | Bottom bar and sidebar as usual; below 638 wide the sidebar is hidden for lack of space | G-18 |
| P-04 | Main window during picture-in-picture | The video area shows "Playing in picture-in-picture"; the bottom bar is usable | |
| P-05 | Double-click on the video area during picture-in-picture | Returns it to the main window, without pausing or switching to fullscreen | IA-CORE-12 |
| P-06 | OSD and info layer during picture-in-picture | Drawn only in the main window, not visible in the small window | |
| P-07 | Always-on-top modes | Four: Never, Always, While Playing, While Playing in Fullscreen; Ctrl+T cycles; default While Playing in Fullscreen | IA-WIN-03 |
| P-08 | Window size presets | 0.5 / 1 / 1.5 / 2×, fit to aspect ratio, mini size; window +/−, window move, maximise (not implemented) | IA-WIN-05 |

**Idle (no media open)**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| I-01 | Video area | The background layer shows the logo (visualizer), background colour `Theme.bg-video` | |
| I-02 | Bottom bar | Shown; play, stop, back and forward disabled, open available | |
| I-03 | Seek bar slider | Currently hidden; target: a disabled-colour slider at the far left (not implemented) | IA-PB-03 |
| I-04 | Time text | `--:-- / --:--` | G-14 |
| I-05 | Click on the video area | Nothing happens (Play/Pause is unavailable, and a click shows no message); a double click does nothing by default | |
| I-06 | Context menu | The full menu; unavailable items are greyed out and show the reason on hover | |
| I-07 | Loading | Loading screen | G-16a, G-16b |

**Audio only**

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| A-01 | Layout | Keeps the video layout; the background layer shows the cover | G-09 |
| A-02 | Window size | Shares one set with video | G-09 |
| A-03 | Pointer, bottom bar | Same as with video (pointer hiding depends only on whether it is playing) | |
| A-04 | Window size commands | Disabled, with the reason "No video" | |

### A.3 Mouse

| # | Location | Action | Specification | Item |
| --- | --- | --- | --- | --- |
| M-01 | Picture | Left click | Play/Pause by default (host 0.5.2), on release; it waits for the double-click interval only while a double click is bound and the click's command is not an on/off toggle; a click whose command is disabled does nothing; works only inside the video area, not over the bottom bar, the sidebar or a menu, not during picture-in-picture | R-H11 |
| M-02 | Picture | Left double-click | Unbound by default (host 0.5.2; it was Toggle Fullscreen); the left half and right half can be bound separately | |
| M-03 | Picture | Right click | Pops up the main menu on release | G-15 |
| M-04 | Picture | Right double-click | Bindable (not implemented) | IA-CORE-08 |
| M-05 | Picture | Middle click, double-click | Click bindable, not bound by default; double-click bindable (not implemented) | IA-CORE-08 |
| M-06 | Window | Side buttons X1 (back), X2 (forward) | Click defaults to next / previous (user setting, 2026-11-01), anywhere in the window (browser precedent, host 0.6.0); double-click bindable (not implemented) | IA-CORE-08 |
| M-07 | Picture | Push (left / right / middle / X1 / X2) | Bindable, 500 ms (not implemented) | IA-CORE-08 |
| M-08 | Picture | Wheel up, down | Volume ±5, limit 100 | G-13a |
| M-09 | Picture | Wheel left, right | Bindable, not bound by default | |
| M-10 | Picture | Wheel while holding a side button | Bindable (not implemented) | IA-CORE-08 |
| M-11 | Picture | Drag | No modifier: moves the window; with a modifier: goes to the registered plugin (not implemented) | G-05, IA-CORE-17 |
| M-12 | Bottom bar | Wheel | No effect | |
| M-13 | Seek bar | Click | Jumps to the click position on press | |
| M-14 | Seek bar | Drag | Seeks in real time without pausing; on release OSD "Seek: …" | |
| M-15 | Seek bar | Hover | Time tooltip off by default, can be turned on in the menu; thumbnail off by default | IA-PB-01 |
| M-16 | Seek bar | Hover near a bookmark or chapter | Within 3 px shows "title  time" | |
| M-17 | Seek bar | Wheel | No effect | |
| M-18 | Volume bar | Click, drag | 70×10, 0–100 | G-13b |
| M-19 | Volume bar | Wheel | No effect | |
| M-20 | Mute button | Click | Toggles mute | |
| M-21 | Back, forward buttons | Left button, right button | Left button ±5 seconds; right button on release previous / next | G-07 |
| M-22 | Time label | Click | Only the current time is clickable; toggles elapsed / remaining and saves | G-14 |
| M-23 | Playlist | Click, Ctrl / Shift+click | Single selection; Ctrl toggles selection, Shift selects a range; pressing on an already selected item can drag several items | |
| M-24 | Playlist | Double-click | Play | |
| M-25 | Playlist | Right button | Pops up the list menu on release; right-clicking an unselected item selects it first | |
| M-26 | Playlist | Wheel | Scrolls the list | |
| M-27 | Playlist | Drag reordering | Supported; unavailable while a search filter is active; edge auto-scroll | G-08 |
| M-28 | Bottom bar, sidebar | Right-click on empty space | Bottom bar: no effect; sidebar: the list menu | |
| M-29 | All buttons | Hover tooltip | The buttons of the bottom bar and of the list bottom bar all have one | G-07 |
| M-30 | Hover tooltip | Delay | 600 ms | |
| M-31 | Window edge | Hover | In fullscreen the right edge pops out the attached panel; no edge behaviour in windowed mode | G-01 |

### A.4 Keyboard focus

| # | Scenario | Specification | Item |
| --- | --- | --- | --- |
| K-01 | Video area has focus | The global binding table is in effect; the video area gets focus on startup | |
| K-02 | Playlist has focus | Bindings with `focus: "panel"` take precedence (Enter / P play, Del, Ctrl+↑↓ / Home / End, F6 close, F5 list settings, etc., see "when the playlist has focus" in menus.md); keys that match nothing go to the global table | |
| K-03 | ↑↓ / Home / End with focus in the list | Move the selection, Shift extends the selection; the view follows | G-08 |
| K-04 | ←→ with focus in the list | Go to the global table, ±5 second seek | |
| K-05 | F5 with focus in the list | Opens "Playlist Settings" | IA-PL-02 |
| K-06 | Focus after manual play | Stays in the list | IA-PL-02 |
| K-07 | Bottom bar has focus | The bottom bar has a `FocusScope`; the buttons themselves do not take focus | |
| K-08 | Tool windows (Picture Adjustments, Audio Workbench, etc.) | Do not fire any binding | |
| K-09 | Floating playlist window | Both global and list bindings are in effect | |
| K-10 | While a menu is open | All keys stay in the menu; bindings are paused | |
| K-11 | Menu keyboard operation | ↑↓←→, Enter, Space, Esc; mnemonics (not implemented) | IA-CORE-02 |
| K-12 | Esc: fullscreen | Toggles fullscreen (enters it in windowed mode, user ruling 2026-11-01) unless a menu, dialog or text field handles it | IA-CORE-04 |
| K-13 | Esc: windowed mode | No effect | |
| K-14 | Esc: menu | Closes one level at a time | IA-CORE-01 |
| K-15 | Esc: tool windows and dialogs | Closes the window in small dialogs (seek, open URL, aspect ratio, confirmation); not handled in large tool windows (Picture Adjustments, audio, subtitles, snapshot) | |
| K-16 | Esc: list search box | Clears the search and returns focus to the list | |
| K-17 | Disabling the default shortcuts | "Ignore Default Shortcuts" (hotkeys) | |

### A.5 Bottom bar and seek bar

| # | State | Element | Specification | Item |
| --- | --- | --- | --- | --- |
| B-01 | All | Total height | 62 px (seek bar row 22 + button row 40) | |
| B-02 | All | Left-hand buttons | 41×40: play / pause, stop, back, forward, open | |
| B-03 | All | Play / pause | One button that changes its icon with the state | |
| B-04 | All | Stop | Closes playback | |
| B-05 | All | Right-hand buttons | Currently the playlist toggle; target: CC, stream selection, playlist toggle, attached panel toggle (not implemented) | IA-PB-06 |
| B-06 | All | Info badges | HW/SW, HDR, video codec, audio codec, channels, download rate, shown by window width (not implemented) | IA-PB-05 |
| B-07 | Narrow window | Right-hand buttons hide in turn | Currently the list toggle hides when the bottom bar is narrower than 320; target: hide by each button's threshold and the right edge of the time text (not implemented) | IA-PB-02 |
| B-08 | All | Time text | The current time in the primary text colour, "/ total" in the secondary text colour | |
| B-09 | All | Time format | `MM:SS` under 1 hour, otherwise `H:MM:SS` | G-14 |
| B-10 | All | Remaining time | `-MM:SS / total` | G-14 |
| B-11 | All | Time corrected for playback speed | Not offered | |
| B-12 | All | Seek bar | 10 px click area, 2 px line, 8 px round slider; colour changes on hover and press | |
| B-13 | All | Seek bar position | `x: 15px`, width = bottom bar width − 132 | |
| B-14 | When seeking is not possible | Seek bar | The played part uses the disabled colour; slider see I-03 | IA-PB-03 |
| B-15 | All | A-B loop, bookmark, chapter marks | Currently vertical lines and a colour band; target: A / B triangles, bookmark flags, chapter vertical lines (not implemented) | IA-PB-04 |
| B-16 | All | Seek bar hover tooltip | Follows the pointer, 4 px above the seek bar, keeping 2 px on each side inside the bottom bar | IA-PB-01 |
| B-17 | All | Mute and volume | 13×10 + 70×10, at the far right of the seek bar row; 0–100 | G-13b |
| B-18 | All | OSD while adjusting volume | No notification while dragging; "Volume: N%" once on release | G-13b |
| B-19 | Fullscreen | Bottom bar layout | The same two rows as windowed mode, overlaid on the picture, across the screen width (narrower when the sidebar pops out) | G-01 |
| B-20 | Fullscreen | Bottom bar background | Opaque, `Theme.bg-bar` | |
| B-21 | All | Command to hide the bottom bar | None | |
| B-22 | All | Playlist toggle state | Off / on / on but not visible (half-lit) | G-18 |

### A.6 Playlist

| # | State | Element | Specification | Item |
| --- | --- | --- | --- | --- |
| L-01 | Windowed | Attached / detached | One button at the right end of the tab strip toggles attached and detached | |
| L-02 | Floating | Default rectangle and minimum size | (438,100) 300×408, minimum 300×250 | |
| L-03 | Floating | Title bar | Currently the system title bar with only close; target: a host title row (not implemented) | IA-CORE-10 |
| L-04 | Floating | Snapping and following the main window | 10 px snapping (not implemented); does not follow when the main window moves | IA-CORE-10 |
| L-05 | Fullscreen | Attached list | Pops out from the right edge | G-01 |
| L-06 | Windowed | Width memory | Remembers the dragged width, default 317 | |
| L-07 | Windowed | Divider | 3 px; video at least 318, panel at least 317 | |
| L-08 | Windowed | Auto-hide | Hidden for lack of space only when the window is narrower than 638 | G-18 |
| L-09 | Windowed | Relation to the window size | Squeezes the video; the window width is unchanged | |
| L-10 | All | Double-click, Enter, P | Play | |
| L-11 | All | Selection and keyboard cursor | Selected rows have a background colour; keyboard cursor dashed frame (not implemented) | IA-PL-03 |
| L-12 | All | Playing / played | Playing uses `Theme.playing`, played uses `text-played` | |
| L-13 | All | Columns | "number. title", duration right-aligned; type icon (not implemented) | IA-PL-04 |
| L-14 | All | Row height and font | 20 px, 12 px text | |
| L-15 | All | Hover tooltip | Full path, shown after 600 ms | |
| L-16 | All | Drag reordering within the list | Supported, 2 px insertion line | |
| L-17 | All | Dragging into the list from outside | Appends to the end | G-12 |
| L-18 | All | Dragging onto the picture from outside | Clears the list, then adds and plays | G-11 |
| L-19 | All | Scrolling after keyboard moves | Kept visible | G-08 |
| L-20 | When switching files | Playing item | Scrolled to the centre when not in view | G-08 |
| L-21 | All | List bottom bar | 35 px: four move buttons, add / remove / sort, search box, total duration; when narrow the search box collapses to a magnifier | |
| L-22 | All | Ways to close | F6, the bottom bar button, the close button of the floating window, "Close" in the list menu, the × on the tab strip when attached | |
| L-23 | All | Panel locks | The panel is locked while the search box has focus and during drag reordering (in fullscreen the pop-out does not retract); focus on the list itself does not lock | G-01 |

### A.7 OSD and info layer

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| O-01 | Position | Top-left of the video area, margin 10 | |
| O-02 | Adjustable position | Horizontal / vertical alignment and margins can be set (not implemented) | IA-CORE-06 |
| O-03 | Font size | Follows the video area height | G-10 |
| O-04 | Colour and plate | White text, translucent dark rounded plate `#0b0a10c4`, 2 px `accent-fill` along the bottom; the same in both colour schemes | G-10, UI-OSD-01 |
| O-05 | Display duration, fade-out | Shown for 2000 ms, then fades out in 300 ms | |
| O-06 | Stacking | A new notification replaces the old one | |
| O-07 | Master switch | "Show OSD", on by default (not implemented) | IA-CORE-06 |
| O-08 | Misc info | One line of info toggled by Scroll Lock (not implemented) | IA-INFO-02 |
| O-09 | Brief info (Shift+Tab) | File name; time, chapter, codecs; subtitle name; 3 seconds | IA-INFO-01 |
| O-10 | Full info (Tab) | Info block, translucent black background, refreshed every second | |
| O-11 | Stacking of OSD and info block | The info block sits right below the OSD line | G-10 |
| O-12 | Fullscreen and window | The same; misc info can be set to fullscreen only (not implemented) | IA-INFO-02 |
| O-13 | Whether snapshots include the OSD | Not included; the capture plugin has the option "Include Subtitles in Rendered Frame" | |
| O-14 | OSD while a menu is open | Shown and faded out as usual | |

### A.8 Menus

| # | State | Element | Specification | Item |
| --- | --- | --- | --- | --- |
| MN-01 | All | Pop-up position | At the pointer (the release position) | G-15 |
| MN-02 | All | Pop-up timing | Right-button release | G-15 |
| MN-03 | All | Can it extend beyond the window | Currently confined to the window and scrollable; target: can extend beyond the window (not implemented) | G-03 |
| MN-04 | All | Submenu opening direction | To the right; flips to the left at the edge of the available area | G-03 |
| MN-05 | All | Submenu open delay | None: at once on hover, with submenu aim (host 0.5.2) | G-15 |
| MN-06 | All | Disabled items | Dim text; the reason is shown beside it on hover | |
| MN-07 | All | Check and radio | 14×14 checkbox, 12×12 radio dot | |
| MN-08 | All | Row height | 24 px | |
| MN-09 | All | Separator | 1 px, taking 7 px of height | |
| MN-10 | All | Shortcut text | Generated from the binding table, right-aligned | |
| MN-11 | Fullscreen | Availability | The full menu | |
| MN-12 | Picture-in-picture | Availability | Pops up only in the main window; the picture-in-picture small window has no menu | |
| MN-13 | Idle | Availability | Items that depend on media are greyed out and explain why | |
| MN-14 | Mini size, small window | Availability | Every item can be reached (scrolling) | G-03 |
| MN-15 | All | Simplified menu | There is only one menu | |
| MN-16 | All | Playback while a menu is open | Not paused ([Interaction requirements](interaction-requirements.md) §3) | |
| MN-17 | All | Bottom bar while a menu is open | Not hidden | |
| MN-18 | Focus lost | Menu | Closes when the main window loses focus; once the overlay window lands, closes when the application is deactivated | G-03 |
| MN-19 | Bottom bar, list | Right button | No menu on the bottom bar; the list menu on the list | |

### A.9 Window size and layout

| # | Element | Specification | Item |
| --- | --- | --- | --- |
| S-01 | Main window minimum size | See W-10 | IA-CORE-13 |
| S-02 | Video area minimum size | At least 318 wide when the sidebar is open; the height lower bound is currently 120, target: removed (not implemented) | IA-CORE-13 |
| S-03 | Window size when playback starts | 1× | G-02 |
| S-04 | Bounds when resizing to the video size | Shrink to the available screen area, enlarge to the minimum size; user-settable desktop percentage bounds (not implemented) | IA-WIN-04 |
| S-05 | Reference point when resizing | Keeps the top-left corner | |
| S-06 | Centring when playback starts | Not centred | |
| S-07 | When the sidebar opens | Squeezes the video; the window width is unchanged | |
| S-08 | When resizing by a multiplier | The video area is computed by the multiplier, then the width of the sidebar and divider is added | |
| S-09 | Sidebar and bottom bar when toggling fullscreen | The bottom bar becomes an overlay; the attached sidebar becomes a right-edge pop-out | G-01 |
| S-10 | After exiting fullscreen | Back to the original window; the sidebar is shown again | |
| S-11 | Keeping the aspect ratio when resizing the window | Optional, off by default (not implemented) | IA-WIN-06 |
| S-12 | Window too narrow for the sidebar | Below a width of 638 the sidebar is hidden, the preference is unchanged | G-18 |

### A.10 Other state-related rules

| # | Rule | Specification | Item |
| --- | --- | --- | --- |
| R-01 | Pause on focus loss | Optional, off by default (not implemented) | IA-PB-07 |
| R-02 | Pause on minimise | Optional, off by default (not implemented) | IA-PB-07 |
| R-03 | Pause on screen lock | Does not pause | |
| R-04 | Prevent the screen saver while playing | On (playback's `preventSleep: true`) | |
| R-05 | Modeless tool windows on top | Not on top on their own; on top together when the whole is on top | IA-WIN-03 |
| R-06 | Minimise all windows together | On (not implemented) | IA-CORE-11 |
| R-07 | Not taking focus when launched with a file | Not offered | |
| R-08 | Drop target hints when dragging files in | Not shown | §0.3 |
| R-09 | When playback fails | The playlist plugin shows a notification | |
| R-10 | End of list playback | No repeat and no shuffle by default; stops when finished | |

## Appendix B: Similar files check (S1–S4)

The `isSimilar(a, b, audio)` of item 5 of the IA-PL-01 "Similar files rule". `a` and `b` are the comparison keys of the two files: `asciiLower(stem) + ".tmp"`. Implemented in `src/plugins/playlist/media.ts` (`similarStep` returns the step passed, `isSimilar` is whether it is non-empty); the constants and integer division are part of the specification and must not be adjusted.

**Notation**

- `T1`: 12 separator characters: space `_` `-` `.` `(` `)` `{` `}` `[` `]` `'` `"`.
- Digits: only ASCII `0-9`; full-width digits, Chinese numerals and characters with code points ≥ 0x80 are not digits.
- `half(s) = ⌊len(s) / 2⌋`, `third(s) = ⌊len(s) / 3⌋`.
- `stripT1(s)`: removes `T1` characters (the content inside brackets is kept; only the bracket characters themselves are removed).
- `mask(s)`: `T1` characters become `?`, digits become `*`, other characters are unchanged, and the length is unchanged.
- `digitDiff(x, y, n)`: looks at the first `n` characters; if at any position either side is not a digit (or is out of range), the result is 100; otherwise it is the number of positions where the two differ.
- `loosePrefix(x, y)`: for `a = stripT1(x)`, `b = stripT1(y)`, compare position by position over the first `⌊min(len) × 2 / 3⌋` positions: when equal, count it; when different, if both characters are < 0x80 and at least one is a digit, stop comparing, otherwise skip. True when the count is ≥ `⌊max(len) / 2⌋`.
- `maskEq(x, y, n, prefix)`: when `prefix` is false, different lengths mean false. Compare position by position within the shorter length: when equal, if `prefix` is true and the position is ≥ `n`, true immediately; when different, if either side has `*` at that position, false, otherwise the result is "position ≥ `n`". True when all compared positions are equal.

**Steps** (checked in order; the first that passes means similar)

1. **S1**: if `min(half(a), half(b)) ≥ 5`: `loosePrefix(a, b)` is true, or `digitDiff(a, b, min(third(a), third(b))) ≤ 2`.
2. **S2**: `a2 = stripT1(a)`, `b2 = stripT1(b)`; if `min(half(a2), half(b2)) ≥ 5` and `digitDiff(a2, b2, min(third(a2), third(b2))) ≤ 2`.
3. **S3**: `a3`, `b3` are `a2`, `b2` with digits also removed. `a3 == b3` is "S3 equal"; otherwise, if `min(half(a3), half(b3)) ≥ 5` and `maskEq(a3, b3, min(third(a3), third(b3), 5), true)`, it is "S3 prefix".
4. **S4**: `am = mask(a)`, `bm = mask(b)`. Audio group: if `maskEq(am, bm, 2, false)` or `maskEq(am, bm, 3, true)` is true, it is "S4 audio". Otherwise, if `min(half(am), half(bm)) < 5`, not similar; if `maskEq(am, bm, min(third(am), third(bm), 5), true)` is true, it is "S4 mask".
5. None passes: not similar.

The "Step passed" column of the 17 IA-PL-01 cases is labelled with the step names above.

## Appendix C: Implementation rounds and status

Principle: the core before plugins; within one round, core behaviour changes (no API change) come before changes that need API negotiation; a plugin item starts after the core items it depends on pass acceptance, and plugin items that do not depend on the core can run in parallel with the core rounds (workflow §3).

### Round 1: core behaviour (done, 2026-10-21)

G-04, G-01, G-03 step one (scrolling the menu inside the window), G-15, IA-CORE-01, G-05, G-10, G-13a, G-16a, IA-CORE-03, IA-CORE-12, IA-CORE-04 item 1. The semantic notes in reference §5.5, §6.3 and §6.5 have been updated to match.

### Round 2: plugins (done, 2026-10-21)

G-07, G-13b, G-14, G-18, IA-PB-01; G-08, G-11, G-12, IA-PL-01, IA-PL-02; G-02, G-06, IA-WIN-01, IA-WIN-02, IA-WIN-03; IA-INFO-01; G-16b.

### Round 3: core, second batch (not started)

| Item | Needs API negotiation |
| --- | --- |
| G-03 step two (menu overlay window) | No |
| IA-CORE-06 host settings page (including the switches of G-01, G-04, G-05, G-10) | No |
| IA-CORE-05 auto-hide the control bar | No |
| IA-CORE-13 minimum height | No |
| IA-CORE-14 closed caption tracks | No |
| IA-CORE-10 floating window title row and snapping | No |
| IA-CORE-11 minimise together | No |
| IA-CORE-04 items 2 and 3 | No |
| IA-CORE-07 custom-drawn title bar | Yes (`Title:OnTop`, the `mode` of `setAlwaysOnTop`) |
| IA-CORE-08 completing binding inputs | Yes |
| IA-CORE-15 main window interface extensions | Yes (accepting `active` and `minimized` first is advised) |
| IA-CORE-16 `cacheSpeed` | Yes (compatible extension) |
| IA-CORE-02 mnemonics | Yes (lowest) |
| IA-CORE-17 drags on the video area | Yes (lowest, can be downgraded) |

Items that need negotiation are submitted one by one per workflow §4.3; during negotiation, the items that need none are done first.

### Round 4: plugins, second batch (not started)

IA-PB-02, IA-PB-03, IA-PB-04, IA-PB-05, IA-PB-06, IA-PB-07, IA-PL-03, IA-PL-04, IA-WIN-04, IA-WIN-05, IA-WIN-06, IA-WIN-07, IA-INFO-02, IA-HK-01, IA-VID-02, IA-SUB-02. Of these, IA-PB-02, IA-PB-03, IA-PB-04, IA-PL-03, IA-PL-04 and IA-WIN-04 do not depend on the core and can be done earlier.

### Cancelled

IA-CORE-09, IA-PB-08, IA-VID-01, IA-AUD-01, IA-SUB-01, IA-MK-01 (control panel, §0.3).

### At the end of each round

- The coordinator runs real GUI tests per the "Acceptance" of each item (workflow §6.2), tested on macOS; Windows gets only build and portability checks (workflow §6.1), and when a Windows machine is available the Windows behaviour in each item's "Cross-platform" is tested as well.
- The interaction design owner rewrites the superseded items in the feature requirements and the UI specification to match, and updates menus.md.
