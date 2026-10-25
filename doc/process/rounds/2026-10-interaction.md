# Interaction alignment round

Branch: `rebuild/interaction` (from `main`). Basis: [interaction spec](../../product/interaction-spec.md) (below, "the rework document"); its fine-grained audit entries are authoritative. Process: first change the core top-down per community workflow §5, then bottom-up per §4 (§4.3 conservative negotiation); cross-platform work follows §1.5.

## 1. Scope of this round (rework document appendix C, rounds 1 and 2)

- **Line A, maintainer**: all core entries of appendix C round 1: G-04, G-01, G-03 step one, G-15, IA-CORE-01, G-05, G-10, G-13a, G-16a, IA-CORE-03, IA-CORE-12, IA-CORE-04 item 1. Also update reference §5.5, §6.3, §6.5, and give plugin migration notices in §4.
- **Line B, authors**: the entries of appendix C round 2 that do not depend on the core, started in parallel with line A:
  - playback: G-07, G-14, IA-PB-01
  - playlist: G-08, G-11, G-12, IA-PL-02
  - window: G-02, G-06, IA-WIN-01, IA-WIN-02, IA-WIN-03
  - info: IA-INFO-01

  Entries that depend on line A wait until line A delivers: G-13b, G-18, G-16b.
- **User rulings (2026-10-21)**:
  - IA-PL-01 goes ahead, but the "files of the same kind" rule must first be confirmed by reverse engineering; after that the playlist author implements it;
  - IA-CORE-07 becomes a custom-drawn title bar, with no main menu button, in round 3;
  - IA-CORE-09 control panel is cancelled.

## 2. Rules

- Authors: only change their own plugin directory and the sections for their plugin in the requirements document / rework document; request core changes per §4.1.
- Maintainer: only changes the core, the SDK and the reference docs; `unsafe` only under `src/host/unsafe/`.
- Nobody commits or operates the GUI. GUI testing is done by the coordinator.

## 3. Record

(Filled in as each stage completes)

### 3.1 Line A delivery (maintainer, 2026-10-21)

**Result**: all core entries of appendix C round 1 implemented, no new plugin API. Three commits (on top of branch `rebuild/interaction`, not pushed):

| Commit | Entries |
| --- | --- |
| Interaction core: bottom bar hover lock and fullscreen sidebar overlay | G-04, G-01 |
| Interaction core: context menu scrolling, submenu delay, right-button release, Esc | G-03 step one, G-15, IA-CORE-01; also fixes `ToolTipArea` (author report) |
| Interaction core: video area, OSD, volume, loading backdrop, window size, PiP, Alt+Enter | G-05, G-10, G-13a, G-16a, IA-CORE-03, IA-CORE-12, IA-CORE-04 item 1 |

**Implementation notes and deviations** (everything else follows each entry's "target behavior" and "implementation notes" in the rework document):

- G-04 / G-01: the bottom bar and the fullscreen sidebar pop-out are wrapped in a `TouchArea` that only does hover detection (as the parent of the content; Slint's hover filter runs before the children and does not swallow their events), outputting `bottom-hover` / `sidebar-hover`; the video area outputs `video-hover`. The hover lock lives in `Policy.hover`, and a plugin's `setInteraction` cannot overwrite it. `Policy` adds `set_hover`, `pointer`, `tick`, and `set_preferred` gains `now`. The pop-out is a separate panel component instance in fullscreen (rebuilt on entering / leaving fullscreen, consistent with the earlier behavior of destroying the sidebar in fullscreen; local state in the component such as scroll position is not kept across fullscreen, while plugin properties on the global are unaffected). While the pop-out is shown the bottom bar narrows immediately (no animation with the 150ms slide). In fullscreen, drops onto the pop-out go to the sidebar panel (`drops.panel`). A pointer resting on the bottom bar also counts as "left of the pop-out" and starts the 500ms leave timer.
- The settings page toggles in G-04 item 4, G-05 item 5 and G-10 item 4 belong to IA-CORE-06 (round 3) and are not done this round; the behavior is fixed at the defaults.
- G-03 step one: the wheel scrolls by the pixel delta Slint reports, not strictly "by line" (line by line would be too fast on a trackpad); arrow strips are 12px, one line every 50ms. The scroll offset is kept by the host (`menu_scroll`), so opening a submenu does not lose the parent's scroll position; submenus are positioned at the parent row's scrolled position.
- G-15: the delay state machine is `menu::HoverDelay` (pure data, unit tested); the host ticks every 16ms and commits paths that are due; the `menu-hover` callback gains a `bool` (immediate for click / keyboard). Platform function `platform::menu_show_delay_ms()`: Windows reads `SPI_GETMENUSHOWDELAY` (one new call in `unsafe/win32`), macOS and other systems 400ms.
- G-05: "no modifier keys" includes ⌘ (Slint's meta). After a drag the system may swallow the release event; if the next press would form a double-click with the drag's press, that double-click is ignored (`drag-swallow`).
- G-13a: the code in the rework document's "implementation notes" would turn 150% into 100% directly on ↓; this round implements "target behavior" item 2, "minus 5 (unchanged)": `volume-up` stops at 100 / stays unchanged when already above 100, and `volume-down` still subtracts 5 with a floor of 0 (150 → 145).
- G-16a: picture-in-picture uses a separate "keep the previous value while loading" check (`composition::video_absent`); otherwise the small window would close on a source change because there is "no video" while loading.
- `ToolTipArea` (reported by an author per §4.1, verified as true): after showing, `self.running = false` removed the binding of `running`, so each tooltip appeared only once. Changed to a private flag `done`, reset when the pointer leaves, which also closes the tooltip; added a read-only `showing`; added a headless test with real pointer events.

**Checks** (run for every commit; results of the third commit): under `src/`, `npm run check`, `format:check`, `boundaries`, `test:sdk`, `plugins:build` pass; `npm run plugin -- test` passes for all 19 plugins (playback's `r3b.test.ts` "I07 a resting pointer makes a picture once" failed once in a full run and passed 4 times when rerun alone; judged a timing-related intermittent failure unrelated to this round, recorded for follow-up) (the workshop's `vendor/sdk` was resynced; this round only changed `testing.ts`, and the copy has no differences); `npm run portability` passes (full check of the Windows target); under `src/host/`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` (262 tests) pass; `grep -rnw unsafe src/host/src` is empty. The GUI was not operated.

New tests: `window_state` (the bottom bar hover lock is not overwritten by plugins; right-edge pop-out, collapse after leaving for 500ms, cancel on return; collapse after 3 seconds idle, no collapse when locked, no pop-out when the preference is hidden or floating; F6 in fullscreen pops out / collapses immediately); `menu` (every item of 25 can be scrolled to in a 182-high window; offset clamping; panel menu scrolling; submenus positioned at the scrolled parent row; delayed open, cancel on leave, diagonal crossing does not switch, resting switches, command row closes children); `ui` (menu Esc closes level by level, `ToolTipArea` shows repeatedly, OSD font size follows the video area); `pip` (double-click to put back does not pause); `core_actions` (volume cap); `composition` (loading backdrop layer, hover and pop-out generation); SDK (`TestHost` fullscreen sidebar, volume cap, loading backdrop layer).

### 3.2 Plugin migration notices (semantic changes, signatures unchanged; see reference §17 "Interaction alignment round line A")

| Plugin | Change | What to change |
| --- | --- | --- |
| playback | In fullscreen the sidebar's `reason: "fullscreen"` now means "overlay collapsed", and `effective: true` while popped out | G-18: show the list button as "on" when `reason === "fullscreen"`; only `reason === "space"` is half-lit |
| playback | `volume-up` caps at 100 | G-13b: volume slider becomes 0–100, displayed value is `min(volume, 100)` |
| playback | A pointer resting on the bottom bar is locked by the host | No change; no need to call `setInteraction` yourself for hover |
| playback | `ToolTipArea` shows on every hover | G-07 can use it directly |
| visualizer | The backdrop layer is `effective: true` while `loading` | G-16b: draw a loading screen when `phase === "loading"`; otherwise the idle logo or the previous cover shows while loading (the comment at `main.ts:109` is out of date) |
| playlist | In fullscreen the sidebar is a pop-out at the right edge; the `setInteraction` lock locks the pop-out in fullscreen | Currently `focused: focus.list \|\| focus.search`: the pop-out also does not collapse when the list itself has focus. Rework document G-01 acceptance item 4 requires "after focus returns to the list and the pointer moves away: the list collapses", so `focused` should only be true when the search box has focus. Suggest calling `showMenu` on right-button **release** (consistent with the main menu); when the menu is too tall the host scrolls it automatically, no change needed |
| All | "Fullscreen" gains `Alt+Enter` by default; left-dragging the video area moves the window | Plugins that bind `Alt+Enter` themselves are overridden by the host (none at present); `Mouse:LeftClick` no longer fires during a drag |

### 3.3 GUI test points for the coordinator (each maps to the rework document's acceptance)

Preconditions: all plugins loaded; macOS. Windows only got a compile check.

- **G-04**: 1 play fullscreen, pointer resting on the bottom bar for 10 seconds → neither the bar nor the pointer disappears; 2 move left and right on the bottom bar for 10 seconds → does not disappear; 3 move to the middle of the video and stay still → bar and pointer disappear after 3 seconds. (The settings toggle in item 4 is not in this round.) Also check: bottom bar tooltips, buttons and progress bar dragging work as usual (there is now an extra hover-detection layer outside).
- **G-01**: 1 in window mode with the list open, enter fullscreen → the list is not shown, video fills the screen; 2 move the pointer to the far right of the screen → the list slides in from the right, covering the video without shrinking it, and the bottom bar's right end aligns with the list's left edge; 3 move the pointer back to the middle → slides back after about half a second; 4 while popped out, click the search box, type, move away → does not collapse; clear the search with Esc and move away → collapses (**depends on playlist giving `focused` only to the search box**, see §3.2; otherwise it does not collapse while the list has focus); 5 in window mode close the list with F6, enter fullscreen, pointer to the right edge → no pop-out; F6 in fullscreen → pops out immediately, F6 again → collapses immediately; 6 the lit/dim state of the bottom bar list button at each step of item 5 (**depends on playback G-18**); 7 enter fullscreen while the list is floating → the floating window is unchanged and the right edge does not trigger. Also check: dragging files into the list in the pop-out; the context menu in the pop-out; after collapsing, keys go back to the video (Space pauses).
- **G-03 step one**: 1 shrink the main window to 320×182 and right-click → the top-level menu is as tall as the window, with ▲▼ strips at top and bottom; resting on a strip scrolls, the wheel scrolls, ↓ to the bottom scrolls automatically, and every item can be clicked; 5 right-click in the floating list window (300×250) → the list menu scrolls and every item can be clicked. (Items 2–4, 6 and 7 belong to step two, "menu overlay window", in round 3.)
- **G-15**: 1 press and hold the right button → no menu; release → it appears at the release point; press the right button on the video, drag to the bottom bar / outside the window and release → no menu; 2 rest on "Video ▶" for 0.2 seconds and move away → does not open; rest 0.5 seconds → opens; 3 after it opens, cross diagonally from the "Video" row over "Audio" into the submenu → does not switch; 4 rest on "Audio" for about 0.4 seconds → switches; 5 click "Video ▶" → opens immediately. (Item 6, the Windows system delay, waits for a Windows machine.)
- **IA-CORE-01**: open "Video ▶ Aspect Ratio ▶" and press Esc three times → closes "Aspect Ratio", "Video", then the whole menu in turn; when opened with the mouse (keyboard cursor still at the top level) Esc likewise closes the deepest level first.
- **G-05**: 1 in window mode, hold the left button on the video and drag → the window follows; 2 bind `Mouse:LeftClick` to play/pause and drag → does not pause after release; 3 drag 2px and release → counts as a click; 4 drag in fullscreen → nothing moves; click once right after dragging → does not enter fullscreen. (Item 5, Windows snapping, and item 6, the settings toggle, are not tested this round.)
- **G-10**: in the default window press ↑ → OSD about 18px; 1080p fullscreen press ↑ → about 32px; shrink to 320×182 → about 12px and stays inside the video area; Tab (info block) → sits right below the OSD line, with font size scaled the same way.
- **G-13a**: at 95% volume press ↑ twice → 100%, 100%; wheel up likewise stops at 100%. (The bottom bar slider 0–100 is playback's G-13b.)
- **G-16a**: while playing video A, open network link B → before B shows a picture, the video area no longer stays on A's last frame (until visualizer does G-16b it shows its idle screen or a solid color); change source in picture-in-picture → the small window stays.
- **IA-CORE-03**: maximize (double-click the title bar or ⌥-click the green button), quit, restart → the window has the size and position from before maximizing.
- **IA-CORE-12**: enter picture-in-picture, double-click the main window's video area → the small window closes, the picture returns to the main window, without entering fullscreen or pausing.
- **IA-CORE-04 item 1**: ⌥↩ enters fullscreen (the shortcut text on the menu's "Fullscreen" row still shows the first key, `Enter`).
- **ToolTipArea**: hover any bottom bar button → the tooltip appears; move away and hover the same button again → it appears again.

## 4. Coordinator acceptance (2026-10-21)

**All checks pass**: 19 plugin tests, cargo test 263, portability result clean.

**Real device (macOS, release) passed**:
- G-02: the window size adjusts when playback starts, limited by the screen size.
- G-04: in fullscreen, with the pointer resting on the bottom bar, the bar does not hide.
- G-01: in fullscreen, F6 pops out the sidebar overlay, it collapses after inactivity, and pops out again when the pointer moves to the right edge; while popped out the bottom bar starts at the left edge and ends at the sidebar's left edge.
- G-05: dragging the picture moves the window.
- G-15: the menu pops up on right-button release.

**Fixed on the spot**:
- Bottom bar centering offset while the sidebar is popped out (b95f337).
- ToolTipArea could only show once.
- playback's I07 test was unstable.

**After this round**: appendix C rounds 3 and 4 (IA-CORE-07 custom-drawn title bar, settings page toggles, etc.), scheduled per user rulings.
