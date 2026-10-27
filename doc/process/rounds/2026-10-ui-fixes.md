# 2026-10 round: UI fixes (mouse defaults, VestraTube host defects, submenus, fullscreen action)

Started 2026-10-26. Host maintainer, worktree `C:\vestra\wt-uifix`, branch `ui-fixes` from main eae5f48. **This document is the only progress record for this round.** Status: implemented, uncommitted; needs SDK 1.5.21 (patch prepared, see "SDK") before it can merge, and the coordinator's GUI check.

Versions: host 0.5.2; built-ins hotkeys 0.1.6, info 0.1.7, playlist 0.1.10 (they have no CHANGELOG file; the changes are listed here). SDK 1.5.21.

## A. Default mouse bindings on the video (user ruling 2026-10-26, approved)

Ruling: a single left click on the video is Play/Pause; a double click does nothing by default.

- `main.rs` `register_host_commands`: Toggle Fullscreen keeps `Enter`, `Alt+Enter`, `F11`; `Mouse:LeftDoubleClick` removed.
- `core_actions.rs`: `play-pause` declares `Space`, `Mouse:LeftClick`.
- Timing (R-H11, `video_click.rs`): with no double-click binding the click fires on release, at the next host tick. Proved by `video_click::by_default_a_click_plays_or_pauses_on_release` (the real core-action declarations: `Mouse:LeftClick` resolves to `host/play-pause`, neither double-click half resolves, `immediate()` is true while playing and with nothing open, a 90 ms click fires at 96 ms). Play/Pause is checkable, so even when a user binds a double click again the click stays immediate (existing rule).
- A click whose command is disabled (nothing open, or a source that cannot pause) now does nothing: `click_tick` drops it instead of showing the reason on the OSD. A click is also the focus and window-drag gesture, so "Not supported by the current source" on every click of an empty player would be noise. Keys still show the reason.
- No click during picture-in-picture (`host.slint`): the main window shows a placeholder there and a double click brings the video back (IA-CORE-12); without this, that double click would also pause and resume.
- User overlays are untouched (a user who bound something else to `Mouse:LeftClick`, or bound a double click, keeps it). A plugin that declared `Mouse:LeftClick` as a default is now shadowed by the player's default, as for any key the player binds; none of the built-ins or the hosted plugins declares it.
- Clicks on overlays (checked in `host.slint` and `composition.rs`): the bottom bar (also in fullscreen, where it is drawn over the video), the docked and the fullscreen sidebar each sit in their own `TouchArea`, which takes the press; the menu layer covers the window and only dismisses; plugin dialogs and tool windows are other windows; the backdrop is below the video's `TouchArea`. None of them reaches `left-press`. **The OSD and the information blocks take no input by design**, so a click on them is a click on the video (as in other players); making them take presses would leave a dead zone over the picture that also blocks double click, the drag and the menu. The coordinator may rule otherwise.
- Hotkeys plugin 0.1.6: the Mouse page hint text ("A left click may wait for the double-click interval while a double-click is bound; …", six locales); the Keys page's "Shortcut updated" OSD and message list only keys, as its rows do (a mouse input belongs to the Mouse page; it showed `Space, Mouse:LeftClick, K`); tests follow the new defaults (they need SDK 1.5.21's `TestHost`); README states the defaults.

## B. Host defects found by the VestraTube author

### B1. `ui_values::reconcile` replaced list models

Findings:

- **Images in rows**: on main (host 0.5.1, #46) a model holding images already takes the in-place path (`has_documents` sees nested images, then `fits` / `merge`): selection changes, a new thumbnail and appended rows keep the model. The JSON fallback the author pointed at is reached only by values without images. VestraTube 0.4.0's observation (every click rebuilt the rows) is explained by the tooltip (B2), which ate the first press of a double click, together with the next point. Added a regression test of their exact case (rows with an image and a `selected` flag; one row selected: same model, one row rewritten).
- **Length changes replaced the model** (real defect): any plain-JSON model whose length changed was installed whole, so appending a page or removing a row recreated every row element (lost hover, a double click in progress, scroll). Now the shared rows update in place (`set_row_data` for changed rows only), the tail is removed or pushed. Rows written in must keep the declared row type, which only an existing row shows: pushed rows must `fit` the first current row; otherwise, and for rows written into an empty model, the value is installed whole as before (Slint then checks the type). Tests: `appended_and_removed_rows_keep_the_model_and_the_other_rows`, the updated `live_slint_model_keeps_identity_…`.

VestraTube can drop its fixed 200-row padding once it requires player 0.5.2.

### B2. `ToolTipArea` swallowed the first press

Root cause (Slint 1.18.1 `i-slint-core/window.rs` `process_mouse_input`): while any `PopupWindow` is open, a press outside it is routed to no item at all (the loop over active popups sets the target to none and stops unless the popup is a menu), whatever its `close-policy`; `close-on-click-outside` only adds the close. `no-auto-close` would not help. A normal popup also takes the keyboard focus when it opens (`take_focus_item`), so a hovered tooltip blurred a focused text field. Only popups of kind `ToolTip` are skipped by the routing and do not take focus, and only Slint's built-in `Tooltip` element (1.18, not experimental; lowered in `passes/lower_tooltips.rs` to an input-transparent popup) creates them.

Fix (`host/ui/vestra/widgets.slint`, shipped in the SDK as `slint/vestra/widgets.slint`): `ToolTipArea` draws Slint's `Tooltip` (text mode, `@markdown("\{root.text}")`, so the text is plain) instead of its own `PopupWindow`. Same API (`text`, `showing`); every press reaches the control under the pointer; no focus change. Differences: the delay is Slint's 500 ms (was 600 ms), the tip uses the std-widgets tooltip style (palette-aware), and a press on a child that takes the press itself (a button) no longer closes the tip; it closes when the pointer leaves. `showing` follows the delay with its own 500 ms timer (close, not exact). Bug fix, no API change: **needs an SDK patch release** (1.5.21).

Audit of the built-ins: no other hover-opened popup. The `PopupWindow`s in audio (preset and device lists), hotkeys, store (menu, dropdown) and video (dropdown) are click-opened lists, where a press outside closing the list is the expected behaviour.

### B3. Text selection colour in the dark palette

The two `TextInput`s that set a selection colour (playlist search, info properties) used `Theme.selected-bg`, close to the field background in the dark palette. Both now use std-widgets `Palette.selection-background` / `selection-foreground`, as the std `LineEdit` (store search) and VestraTube 0.4.1 do. There is no shared input component to fix centrally; no other built-in `TextInput` sets a selection colour.

## C. Submenus open at once on hover (user report, approved behaviour change)

Before: G-15 opened or switched a submenu after the pointer rested 400 ms on its row (`menu::HoverDelay`; Windows read `SPI_GETMENUSHOWDELAY`, 400 ms on this machine), committed by `menu_tick` in the 16 ms host tick.

Why the user saw 1–2 s: **not proven** (no GUI runs for this role). The state machine is correct (the old unit test and the headless pointer-walk test opened at 400–416 ms), and the tick is a 16 ms repeating timer, so the extra second must come from the tick running late, not from the logic. The likeliest cause is UI-thread load while a video plays: the hover event and the due check both wait for the host tick, which shares the UI thread with frame uploads and redraws (the Windows render round measured the tick plus upload passing 16.7 ms), and every command revision change during playback also rebuilds the whole menu projection in the tick (`refresh_menu`). The new design no longer waits 400 ms, but a hover still goes through the tick. GUI check: `VESTRA_DEBUG_MENU=1` logs `menu: hover … t=` and each open with the host clock; if a hover's open lags more than a few ticks, the tick is starved, which belongs to the render maintainer's UI-thread work.

New behaviour (`menu::HoverAim`, all platforms, no system delay):

- Hovering a row opens its submenu, or closes deeper levels for a command row, within the tick.
- Submenu aim: when the hover would replace an open submenu, the switch is held unless the step from the last tick is a clear move away (at least 4 px, `AIM_SLOP`, and outside the triangle), which switches at once (straight down a column). While held, the direction is the net displacement from where the hold began (the anchor) to the pointer: inside the triangle from the anchor to the submenu's near edge (8 px slack above and below) keeps holding; outside, once at least 4 px away, switches; nearer than 4 px is a rest or jitter and is not judged. The switch also happens 120 ms (`AIM_GRACE_MS`) after the pointer last moved while heading for the submenu; reaching the submenu cancels it. Unknown pointer (left the window): immediate.
- The pointer comes from each window's winit hook (`track_pointer`, logical px), the panels from the menu projection (`menu::panels`).
- Keyboard and clicks unchanged. `platform::menu_show_delay_ms` and `vestra_win32::menu_show_delay_ms` (one `unsafe` call) are removed.
- Tests: `menu::a_move_heads_for_a_submenu_inside_the_triangle_to_its_near_edge`, `menu::submenus_open_at_once_and_aiming_at_the_open_one_holds_a_switch`, and the headless view test `ui::menu_pointer_walk_…` (opens within one tick, straight down switches at once, a diagonal toward 跳转 ▸ across 速度微调 keeps it, held within the grace, switches after it).

## E. No `if` toggles under the pointer (render maintainer's request, coordinator's 4K measurements)

`menu-view.slint` `MenuItem` drew the lit row's 2 px accent bar with `if root.lit: Rectangle`, so every hover change destroyed one element and created another; in Slint 1.18.1 each `unregister_item_tree` rebuilds the whole window's AccessKit tree (`i-slint-backend-winit-1.18.1/accesskit.rs:205-233`), 120–160 ms on this machine at a 4K render target, which dropped video presents to 10–14 fps during a menu sweep (`C:\vestra\wt-prevstutter\doc\process\rounds\2026-10-ui-redraw-stutter.md`).

Changed to persistent elements hidden by `visible:`: the accent bar; the disabled-command explanation beside the current row (it followed the hovered row through an `if`; the indices are read unguarded, an index out of range gives the default level and row with an empty reason, and `visible` adds the range check); in `sidebar-tabs.slint` the tab hover plate (`!current && has-hover`) and the current-tab bar. Audit of the rest: the other `if`s in `MenuItem` (checked, radio, keys, submenu arrow), the separators, the scroll bars and the empty-level text depend on the model; `host.slint`'s `if`s (`menu-open`, `picture-in-picture`, the sidebar overlay) flip by click, key or window state; `widgets.slint`'s new `Tooltip` `if` flips on press and text only; the playback bar's hover tip is the playback author's (`if … seek-bar.pointing`, toggles on enter and leave, per the render doc). A comment rule at the top of `menu-view.slint` states it. The aim work (C) is unaffected: it reads the pointer and the panel geometry, not the elements.

## D. `host.run("fullscreen")` (request from the playback author)

### Request (§4.1)

```text
Requesting plugin / code location
  vestra.playback, bottom bar (playback/ui/bar.slint): a Fullscreen button right of
  Open, left of the time label (user request).
User action and expected result
  Click the button: windowed → fullscreen; click again in fullscreen → windowed,
  exactly like Enter / F11 (Toggle Fullscreen). The icon shows enter or leave.
Why the existing API cannot do it
  plugin.host.run accepts only core_actions::ACTIONS (playback actions); MainWindow
  has window.state (read) but no fullscreen setter; host/fullscreen is a host
  command a plugin cannot execute (commands run only their owner's).
Desired usage
  bar.on("fullscreen", () => plugin.host.run("fullscreen"));
  icon from plugin.window.state.fullscreen / onChange.
Who owns the state; failure, cancellation, missing capability
  The host owns the window. unavailable without a UI; same behaviour in PiP as the key.
Whether it breaks an existing contract
  Additive (a new HostAction id). Affects no plugin that does not use it.
Minimal acceptance scenario
  host.run("fullscreen") twice: windowed → fullscreen → windowed, window.state
  after each.
```

### Maintainer reply (§4.3)

Alternatives considered:

1. A key binding the plugin presses: plugins cannot inject input; rejected.
2. `window.setFullscreen(bool)` on `MainWindow` (a setter): a new member with its own semantics (and a race with the user's key); a toggle that runs the host's own command is smaller and is what the button means.
3. The plugin registers its own command bound to nothing and asks the user: not a button.

The window is host-only state (§4.3 item 4: a capability only the host has), and the button must behave as the existing command, so a core action id is the narrowest form.

### Ruling

Accepted as a core action: `HostAction` gains `"fullscreen"` (SDK 1.5.21, player 0.5.2), documented as a toggle. The user's clarification (2026-10-26): an explicit toggle, in when windowed, out when fullscreen; tests cover both directions.

Side change (review 2026-10-27, recorded): `host_command` was reordered so that `fullscreen` / `exit-fullscreen` go through `window_command`; as a result the host command `quit` now runs `quit_event_loop` (after flushing the UI state) even when there is no shell window (headless), where before the whole match was skipped without a shell. Quit from a key or the menu always has a shell, so no user-visible change; a headless run that triggers `quit` now quits instead of ignoring it.

Review findings fixed on 2026-10-27 (reviewer verdict "merge" with small fixes): the aim direction is now the net displacement from where the hold began (`HoverAim::anchor`), judged only past `AIM_SLOP` (4 px), so a slow diagonal with ticks that move only vertically and a 1 px jitter keep the submenu (unit tests `a_slow_diagonal_and_a_jitter_keep_the_open_submenu`); a changed row of another shape (`[{a:1}]` → `[{a:"x"},{a:2}]`) now falls back to a whole-model install before anything is written (test in `appended_and_removed_rows_…`); the `run_core_action` comment and the core-ui-design mouse-default notes were reworded.

Implementation: `core_actions::ACTIONS` gains `fullscreen` and `WINDOW_ACTIONS = ["fullscreen"]` marks it as the window's: it has no playback state or `declarations` entry (its command stays `register_host_commands`'), `core_action_states` skips it, and `run_core_action_with` runs it through `window_command`, the same function the key's `host_command("fullscreen")` uses (so PiP behaves as the key). No window: `unavailable` ("the player has no window"). The host agreement test checks `HostAction` = `ACTIONS`. SDK: `TestHost` toggles `host.mainWindow.fullscreen` and emits `window.state` (test: windowed → fullscreen → windowed, `onChange` seen `[true, false]`).

Icons (`host/ui/vestra/icons.slint`, synced to the SDK): `fullscreen` (four corners pointing out) and `exit-fullscreen` (pointing in), 14 × 12, 1.5 px V-Line strokes. The playback author implements the button after this branch lands; `src/plugins/playback` is untouched here.

## Spec changes for the interaction design lead (`doc/product/`, not edited here)

The coordinator applies these (exact text; current → requested). A ready patch of all of them: `C:\Users\123\AppData\Local\Temp\claude\C--PROJECT-vestra-player\44d1d55c-542f-4c25-879c-d7a87d0c3d88\scratchpad\uifix\product-docs.patch` (`git apply` from the repository root). Also add `fullscreen` and `exit-fullscreen` to `identity/icons-v-line.json` (commands as in `icons.slint`, viewbox 14 × 12, stroke 1.5).

ui-spec.md §4:

- CSS comment: `… | open */` → `… | open | fullscreen (enter or leave by state) */`.
- UI-BAR-05: "Six 41×40 icon buttons on the left of the button row: play/pause, stop (close the source), back, forward, open, fullscreen; right-click on back/forward = previous/next. Fullscreen (2026-10-26, user request) toggles the main window's fullscreen like Enter/F11 (`host.run("fullscreen")`, player 0.5.2): icon `fullscreen` when windowed and `exit-fullscreen` in fullscreen (`window.state`)". Owner: "playback (previous/next through C6; fullscreen through `host.run`)".
- UI-BAR-07: "… 11px, 10px after the last shown button (fullscreen, or open while fullscreen is hidden, UI-BAR-10); …".
- UI-BAR-10: "The time keeps priority over the fullscreen button: the fullscreen button is hidden first, when the bar is too narrow for six buttons, the time and the playlist toggle with 8px to spare, that is below 6 × 41 + 10 + time + 8 + 23 + 8 px. Measured with the 11px body font, `0:00:00 / 0:00:00` is about 85px, so the button shows from a bar width of 380px (compute it from the time's laid-out width, not the constant). Below a window width of 320px the right-hand buttons are hidden as well. …" (The 85 px is an estimate from the font size, digits about 6.2 px; the rule, not the constant, is what the plugin should implement.)

ui-spec.md §8 mouse:

- UI-MOUSE-01: "Double-click the video | Unbound by default (host 0.5.2, user ruling 2026-10-26; it was Toggle Fullscreen); the user can bind it under "Settings ▸ Mouse…" | host (C3)".
- UI-MOUSE-03: "Click the video | Play/Pause by default, on release (host 0.5.2); rebind under "Settings ▸ Mouse…" | host (C3)".

interaction-spec.md:

- G-15 title: "Submenus open on hover; the menu pops up on right-button release (P1)". Status: implemented, host 0.5.2 (user ruling), `menu::HoverAim`. Items 2, 3, 5 and the cross-platform note as in section C above (open at once; submenu aim with the triangle, 8 px slack, 120 ms grace; no system delay on any platform). Summary rows: line 32 ("a submenu opens at once on hover (a diagonal toward an open submenu keeps it)"), G-15 row, MN-05 ("None: at once on hover, with submenu aim (host 0.5.2)").
- IA-CORE-04 item 1: Toggle Fullscreen defaults `Enter`, `Alt+Enter`, `F11` (no `Mouse:LeftDoubleClick` since host 0.5.2); plugins toggle it with `host.run("fullscreen")`. F-12: "Enter, Alt+Enter, F11 (a double click only when the user binds it)".
- I-05: "Click on the video area | Nothing happens (Play/Pause is unavailable, and a click shows no message); a double click does nothing by default".
- M-01: "Left click | Play/Pause by default (host 0.5.2), on release; it waits for the double-click interval only while a double click is bound and the click's command is not an on/off toggle; a click whose command is disabled does nothing; works only inside the video area, not over the bottom bar, the sidebar or a menu, not during picture-in-picture | R-H11". M-02: "Left double-click | Unbound by default (host 0.5.2; it was Toggle Fullscreen); the left half and right half can be bound separately".

menus.md: Play/Pause row "Space, left click on the video"; Fullscreen row "Enter, Alt+Enter, F11"; the macOS note "fullscreen also has Enter;" (no double-click).

feature-requirements.md keeps its history; its acceptance lines that mention double-click fullscreen (backdrop sections) describe the defaults of their time.

`doc/architecture/core-ui-design.md` (maintainer's) is updated here: the mouse-default note and the `host/fullscreen` row.

## SDK 1.5.21 (patch, SDK repository is read-only for this role)

Patch: `C:\Users\123\AppData\Local\Temp\claude\C--PROJECT-vestra-player\44d1d55c-542f-4c25-879c-d7a87d0c3d88\scratchpad\uifix\sdk-1.5.21.patch` (from SDK main 103a7d2; `git apply` in the SDK repository). Contents: `HostAction` + `"fullscreen"` (types, `services.ts` client check, `TestHost`), `BUILTIN_COMMANDS` (play/pause `Space, Mouse:LeftClick`; fullscreen without the double click), `TestHost` play/pause default, the synced `slint/vestra/widgets.slint` (tooltip fix) and `icons.slint` (two icons), docs (commands, playback, theme-widgets example, generated tables), `llms*.txt`, CHANGELOG 1.5.21 (opens with who must change: nobody), version 1.5.21. Additive and a bug fix; no removal.

Run in a clone with the patch: `npm run check`, `format:check`, `lint`, `doc-lint`, `docs:check`, `examples` pass; `npm test` 189 pass, 0 fail (one earlier run had a timing failure in `round2 storage`, passed on rerun). `npm pack` works; `check-pack.mjs <tgz>` reports three missing files on Windows, the same on unpatched 103a7d2 (a Windows-only issue of the script, not of this patch).

Merge order: release SDK 1.5.21, then in `src/` `npm install @vestra-player/sdk@1.5.21` (package.json `^1.5.21`, lock), then in `src/plugins/workshop` `npm run vendor:sdk` (its vendored SDK test fails until then), then the §6.1 checks. Until 1.5.21 is installed, `test:sdk` (host agreement) and the hotkeys tests fail on this branch, by design.

## Checks (Windows, this worktree; SDK 1.5.21 from the patch copied over `src/node_modules/@vestra-player/sdk` for the npm checks)

- `cargo fmt --check`: pass. `cargo clippy --locked --all-targets -- -D warnings`: pass. `cargo test --locked --workspace`: 461 passed, 0 failed (6 ignored) (rerun 2026-10-27 after the review fixes and item E; `npm run check` and `typos` rerun too).
- `npm run check`, `format:check`, `boundaries`, `test:sdk`, `licenses`, `unsafe-policy`, `sdk:sync-slint` (against the patched SDK), `typos` (repository root): pass.
- `npm run plugins:build`: pass. `npm run plugin -- test --all`: everything passes except workshop's vendored-SDK test (expected until `vendor:sdk`, see above); marks and playback had timing failures in the parallel run and pass when run alone; hotkeys passes after its test updates. `npm run plugin -- check hotkeys info playlist`: pass (all locales complete).
- `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`: pass. `npm run portability`: clean. The host test `all_plugin_and_example_slint_files_compile` compiles every `ToolTipArea` user with the new `Tooltip`.
- Not run: `cargo deny`, `cargo shear`, `cargo doc`, actionlint/zizmor (no workflow or dependency change), `ui:shots`.
- Release build (`cargo build --release --locked`) staged at `C:\vestra\stage-uifix\VestraPlayer-0.5.2-windows-x64` (from the 0.5.1 stage): new `vestra-host.exe` and `resources/ui` from `src/host/ui` (both restaged 2026-10-27 after the review fixes and item E), the built hotkeys, info and playlist plugins (versions in `builtin.json` updated), and the patched SDK 1.5.21 `dist` in `resources/runtime`.

## GUI check list for the coordinator

1. Click on the playing video: pauses at once on release (no 0.5 s wait); click again: plays. Double click: pauses and resumes (two clicks), no fullscreen.
2. Click on an empty player: nothing, no OSD. Drag the video in a window: the window moves, no pause after release.
3. Click on the bottom bar (windowed and fullscreen), the sidebar, the fullscreen sidebar overlay, an open menu (closes it): no play/pause. Click on the OSD while it shows: play/pause (by design, see A).
4. Picture-in-picture: double click on the main window's placeholder brings the video back without a pause.
5. Settings ▸ Mouse…: Left click shows Play/Pause; Left double-click empty; bind double-click to Toggle Fullscreen: double click goes fullscreen, a single click still pauses at once.
6. Hover a toolbar/sidebar button until its tooltip shows (playlist bottom bar, VestraTube rows if loaded), then click it: the click works the first time; a double click on a playlist row plays.
7. Playlist search box and Playback Properties text in the dark palette: selected text is clearly highlighted.
8. Context menu: hover Playback, Video, Audio…: each submenu opens at once; move diagonally from Playback into its submenu across the rows below: it stays; rest on a sibling: switches after a moment; keyboard ← → ↑ ↓ unchanged. With a 4K video playing, check the same (and `VESTRA_DEBUG_MENU=1` timing if it lags).
9. Enter / F11 still toggle fullscreen; Esc leaves.
10. VestraTube (needs its 0.4.1): appending result pages keeps hover and scroll.
