# Menus and UI ownership

> This table records the current implementation and corresponds to the top-level structure in [UI spec](ui-spec.md) §6.2. Menu redesign (user rulings 2026-11-07/08, design `menu-redesign` proposal v2 §4, mockup 04): the built-in task panels replaced the Video, Audio, Subtitles and Playback submenus, and the right-click menu keeps only what has no panel home. Everything else stays a command with its id, key and state, reachable from the **command palette** (Ctrl+Shift+P, the 🔍 button, or Command Palette… in the menu) as "Category: Title".

The product's context menu tree is merged from the commands registered by the 16 built-in plugins (`youtube` and `bilibili` live in the plugin repository and are installed through the store; their two items only exist once they are installed), the host's core actions, and the host's own commands (Adjustments and Effects, Sidebar, Toggle Fullscreen, Command Palette…, Quit). Menus are organised by user domain, not by plugin. Before adding or moving an entry, check it against this table so that nothing is duplicated and nothing is scattered into the wrong domain ([interaction requirements §1](interaction-requirements.md#1-menus-are-organised-by-user-domain)). A command that a panel or an Options page already covers does **not** get a menu entry: it is registered without `path` and lives in the palette.

## Menu tree

2026-11-08, baseline branch `b5-menu` (B5 of the menu redesign). Compiled from the snapshot exported by `OUT=<file> node --test tests/menus-snapshot.test.ts` (run in `src/`; it covers every plugin under `src/plugins`). Menu `order` values are listed per item below; the separators come from each item's group (open · domain · windows · screen · windows · settings-main · exit), not from fixed number ranges — the two column toggles (791 and 800, group `windows`) and Fullscreen (810, group `screen`) sit outside the ranges the proposal once gave. Checked, radio and initially disabled states come from each command's `state` in the same test host.

The labels below are the English interface text (the English source text of each string); the Simplified Chinese interface shows the translations from the `zh-Hans.po` catalogs.

- Order is ascending by declared `order`; for equal `order`, by plugin id, then registration order. A submenu takes the smallest `order` among its items, and a group (`group`) takes the group of its first item.
- The 8 built-in anchors (`@open`, `@playback`, `@subtitles`, `@video`, `@audio`, `@screen`, `@settings`, `@reset`; fixed positions Open 110, Playback 200, Subtitles 300, Video 400, Audio 500, Screen 710, Settings 830, Reset 840, `MENU_ANCHORS` in `src/host/src/commands.rs`) stay valid for third-party plugins and keep their positions; an anchor is not shown while no command is in it. Since B5 no built-in plugin puts a command into `@playback`, `@subtitles`, `@video`, `@audio`, `@screen`, `@settings` or `@reset`, so these submenus appear only when a third-party plugin fills them (for example Games ▸ VestraNES). The SDK's menu guidelines still allow them.
- "Plugin" column: "core" is a host core action (`host/<id>`), "host" is the host's own command.
- Top-level separators are drawn as "——" rows (two adjacent items with different `group`). Inside a submenu a separator is also drawn between adjacent items with different `group`.
- Shortcuts work in the global scope (when the video area, bottom bar, adjust column or sidebar has focus; not inside tool windows). Shortcuts written with `Ctrl` can be triggered on macOS by both ⌃ and ⌘. Users can rebind them in Keyboard Shortcuts… (palette, or the Options window), or turn off all default bindings with "Ignore Default Shortcuts".
- "Dynamic" items are generated at run time from the current media, records or devices; "disabled" items are always present but greyed out with a reason when their condition is not met.

| Menu path | Plugin | Shortcut | Notes |
| --- | --- | --- | --- |
| Open File… | playback | Ctrl+O, F3 | `order` 100, group `open` |
| Open ▸ Open URL… | playback | Ctrl+U | 110 |
| Open ▸ Open Folder… | playlist | F2 | 110 |
| Open ▸ Open Subtitle… | subtitles | Alt+O | 113 |
| Open ▸ Load External Audio… | audio | | 114; disabled when no file is playing |
| Open ▸ Add Secondary Subtitle… | subtitles | | 114; disabled when there is no playable media |
| Open ▸ Search and Download Subtitles… | subtitle-online | | 115; moved here from Subtitles ▸ Online Subtitles ▸ (B5). Its settings item is palette-only |
| Open ▸ YouTube Video… · Bilibili Video… | youtube · bilibili | | Official plugins from the plugin repository; appear after installation (`@open`, unchanged) |
| Open ▸ —— Recently Played ▸ (one item per recent entry) | history | | 119, group `recent`; dynamic; with no entries shows a greyed-out "(No Entries)" |
| Open ▸ Recently Played ▸ Remove Invalid Entries · Clear Entries · Record Playback History | history | | Group `recent-manage`; Record Playback History is checked. They move to an Options ▸ Playback ▸ Resume page in proposal step 9 |
| Close | core | F4 | 120 |
| —— | | | |
| Play/Pause | core | Space, left click on the video | 201, group `domain`; top level since B5 (Previous/Next, the seek, frame and speed actions are palette-only; the Playback panel and the bottom bar have their controls) |
| Audio Track ▸ (actual audio track names) | audio | | 301, `domain`; radio; dynamic; with no audio tracks a greyed-out "No Audio Tracks" ("Loading…" while loading) |
| Subtitle Track ▸ Off · (actual track names) | subtitles | | 302, `domain`; radio; dynamic |
| Snapshot ▸ Save Current Frame · Copy Current Frame | capture | Ctrl+E · Ctrl+C | 420/422, `domain`; top level since B5 (was Video ▸ Snapshot ▸); disabled when there is no frame to capture |
| Snapshot ▸ Burst… | capture | Ctrl+G | 426; checked while the Capture tab's Settings page shows the Burst row |
| Snapshot ▸ Export Clip… | capture | | 428 |
| Snapshot ▸ —— Capture… | capture | Alt+S | 440, group `capture-tab`; opens the Capture sidebar tab (results, settings). Snapshot Settings… is palette-only |
| —— | | | |
| Adjustments and Effects | host | | 791, group `windows` (id `adjust`); checked; shows or hides the adjust column at its last tab (Picture, Audio, Subtitles, Playback); the same name as the bottom-bar sliders button. Replaced Picture… · Audio… · Subtitles… · Playback… (791–794) here (user ruling 2026-11-08); disabled while no panel is in the column |
| Sidebar | host | | 800 (id `sidebar`); checked; shows or hides the sidebar at its last tab; the same name as the bottom-bar sidebar button. Replaced Playlist (800) here (user ruling 2026-11-08); disabled while no panel is in the sidebar |
| Chapter and Bookmark List… | marks | H | 801; top level since B5 (was Playback ▸ Chapters and Bookmarks ▸). Becomes a Chapters sidebar tab in proposal step 8 |
| —— | | | |
| Fullscreen | host | Esc | 810, group `screen`; checked; key `Toggle Fullscreen` (id `fullscreen`); Esc is its only default (user ruling 2026-11-01). "Enter Fullscreen" and "Leave Fullscreen" are not in the menu and have no default key |
| —— | | | |
| Vestra Player AI… | workshop | | 820, `windows` |
| Plugin Store… | store | | 822, `windows`; when plugins have updates the title is "Plugin Store (N updates)…". Other entry points: drop a `.vxp` onto the video area; the "Options ▸ Plugins ▸ Plugin Store" page |
| —— | | | |
| Command Palette… | host | Ctrl+Shift+P | 829, group `settings-main` (id `palette`); also the 🔍 button at the right end of the bottom bar |
| Options… | preferences | F5 | 830, `settings-main`; top level since B5 (was Settings ▸ Options…). The Options window holds the pages that replaced Settings ▸: Appearance (theme, idle logo), Language, Keyboard Shortcuts, Mouse, Plugins, Updates |
| About Vestra Player… | info | F1 | 880, `settings-main`; Check for Updates… is palette-only (and a button in About) |
| —— | | | |
| Quit | host | Cmd+Q (shown on macOS; on Windows and Linux the menu shows no shortcut, the close button and Alt+F4 quit) | 990, group `exit`; key `Quit` |

Top level: 17 entries plus third-party anchors; 30 fixed items in all, the rest are dynamic lists. Items with the same `order` sort by plugin id, which is why Open ▸ Load External Audio… (audio, 114) comes before Add Secondary Subtitle… (subtitles, 114); mockup 04 drew them the other way round.

## Commands outside the menu

Every command below keeps its id, default key, category and checked/radio state. The palette shows it as "Category: Title" with its key and, where it has one, the place that now holds its control; `Alt+Enter` in the palette opens that place. Users' own key bindings keep working (no orphan bindings).

| Plugin | Outside the menu (left in B4 or B5, or never in it) | Where it lives now |
| --- | --- | --- |
| core (host) | Previous · Next (PgUp · PgDn, mouse X2 · X1); Seek ▸ Previous/Next Frame (D · F), Back/Forward 5 s (← · →); Speed ▸ Normal (Z), Slower/Faster 0.1 (X · C) | Bottom bar; Playback panel (speed ruler, frame steps); palette (category Playback / Seek / Speed) |
| playback | Reopen Current/Last File (Ctrl+Y); Copy to Clipboard ▸ File Name · Folder Path · Full Path; Seek ▸ Previous/Next Keyframe (Ctrl+Shift+←/→), Back/Forward 30 s (Ctrl+←/→), 60 s (Shift+←/→), 300 s (Ctrl+Alt+←/→), Jump to Start (Backspace), Seek Steps…; Jump to Time… (G); Speed ▸ 0.5× · 1.5× · 2× · Custom…; Disable Screen Saver While Playing; Show Time / Show Thumbnail on Seek Bar Hover | Playback panel (speed, jump, frame and keyframe steps, seek steps); the three toggles are palette-only until the Options ▸ Playback ▸ General page (proposal step 9) |
| video | Everything that was under Video ▸ (colour ±1 % keys W/E, R/T, Y/U, I/O; aspect, crop/stretch, framing, rotation, deinterlace, flip, image processing toggles and their keys; pixel shaders, HDR, GPU rendering) and Reset ▸ Picture (Q) · Framing (Numpad5, Alt+Backspace) · Crop Position; Reset: Picture Panel (the panel's Reset All, palette only) | Picture panel: Colour (sliders with ↺), Effects (toggle chips with strength, Frames Off / Doubling / Interpolate), Geometry (editor on a still: pan, zoom, crop with ratio chips; aspect; rotation; flip), Advanced (deinterlace, shaders, HDR, GPU rendering). Reset / Reset All at the panel bottom |
| audio | Everything that was under Audio ▸ except Audio Track ▸ (Cycle Tracks Alt+A, channel modes, Output Device ▸, Equalizer Shift+E, Preset ▸, Audio Sync ▸ `>` `<`, Processing ▸ Normalize Shift+N · Crystalizer Shift+C) and Reset ▸ Audio Sync (`?`) · Audio Processing | Audio panel (device, channels, equalizer with presets, sync with keep mode, processing) |
| subtitles | Everything that was under Subtitles ▸ except Subtitle Track ▸ (Show Subtitles Alt+H, Cycle Alt+L, Secondary Subtitle ▸, Subtitle Style ▸ Bold Alt+B · Size ± Alt+PgUp/PgDn · Move Up/Down · Shadow · Opaque Box, Original ASS/SSA Styles, Sync ▸ `.` `,` · Adjust Sync…, Reload Subtitles Ctrl+Alt+Y, Refresh Subtitle Tracks, Subtitle Tuning…, Subtitle Settings…), Previous/Next Dialogue (Home · End), Current Subtitle Start (Ctrl+Home), Remove External Subtitle, and Reset ▸ Subtitle Position (Alt+Home) · Subtitle Sync (`/`) | Subtitles panel (tracks, secondary, style, sync, files); the dialogue seeks stay palette-only |
| subtitle-online | Online Subtitle Settings… | Palette (an Options page later) |
| capture | Save Current Frame As…; Save/Copy Rendered Frame (Ctrl+Alt+E · Ctrl+Alt+C) and As…; Save/Copy Video Frame Only; Export Tasks…; Capture as PNG · JPEG · BMP; Include Subtitles in Rendered Frame; Snapshot Settings…; Snapshot Folder… · Open Snapshot Folder; Show Last Capture in Folder; Copy Last Snapshot; Set Clip Start · Set Clip End; Export Clip Range (Ctrl+Shift+E); Toggle range marks (Ctrl+Shift+R) | Capture sidebar tab (Snapshot: Save and Copy only; Clips; Results; Settings with format, effects, Burst, folder); keyboard commands stay in the palette |
| marks | Add Bookmark (P); Previous/Next Bookmark or Chapter (Shift+PgUp/PgDn); Previous/Next Chapter (Ctrl+PgUp/PgDn); the dynamic chapter and bookmark entries; Show Marks on Seek Bar | Chapter and Bookmark List… window (H) now; the Chapters sidebar tab in proposal step 8 |
| playback (A-B Loop, from marks) | A-B Loop ▸ (all: `[` `]` `{` `}`, Ctrl/Alt+`[`/`]`, Ctrl+Alt+`[`/`]`, Alt+\, `\`, Loop Count ▸); the 18 `ab.*` commands moved from `vestra.marks` to `vestra.playback` with the same ids and keys, and user overrides are migrated | Playback panel A-B Loop section (mini timeline, A and B rows with nudges and clear, Loop switch, Count, ↺ Chapter); palette (categories A-B Loop and Loop Count) |
| video · audio · subtitles · playback | Picture… (Alt+1) · Audio… (Alt+2) · Subtitles… (Alt+3) · Playback… (Alt+4), `video.picture` · `audio.workbench` · `tune` · `panel` (left 2026-11-08, user ruling) | Adjustments and Effects (menu and bottom-bar button) at the column's last tab, then the tab; the keys open their own tab |
| playlist | Playlist (F6), `playlist.visibility` (left 2026-11-08, user ruling) | Sidebar (menu and bottom-bar button) at the sidebar's last tab, then the Playlist tab; F6 opens the Playlist tab |
| playlist | Playback ▸ Playlist ▸ (Add Files… · Add Link… · Add Folder…, Repeat ▸ Off · All · One (+ Stop After Current, playlist 0.2.6, palette only), Shuffle, Open List File… · Save List…, When Opening a File ▸, Fast Advance to Next, Remember List, Confirm Before Removing All) | Playlist sidebar (Add ＋ button, list context menu); Repeat is the playlist bottom bar's Repeat button (design B2, 2026-11-09; Shuffle stays palette and settings page); the four settings are palette-only until an Options ▸ Playback ▸ Playlist page |
| history | Resume Playback ▸ Don't Resume · Resume Automatically · Ask Every Time, Also Remember Audio Files, Clear All Saved Positions | Palette until the Options ▸ Playback ▸ Resume page (proposal step 9) |
| window | Hide Pointer While Playing; Screen ▸ Picture-in-Picture, Mini Size (`` ` ``), 0.5× · 1× · 1.5× · 2× (1 · 2 · 3 · 4), Fit to Video Aspect Ratio (8), When Playback Starts ▸, Always on Top ▸ Cycle Mode (Ctrl+T) · modes | Palette (category Screen / Window); the "Options ▸ Playback ▸ Window" page holds the settings |
| visualizer | Show Album Cover for Audio Only; Show Logo When Idle; Choose Logo Image…; Reset ▸ Idle Logo | Options ▸ Appearance page; palette |
| hotkeys | Keyboard Shortcuts…; Mouse…; Ignore Default Shortcuts | Options window pages; palette |
| preferences | Appearance ▸ Vestra Dark · Vestra Light; Language ▸ (8) | Options ▸ Appearance and Language pages; palette |
| info | Playback Info (Tab); Brief Info (Shift+Tab, never had a menu entry); Playback Properties… (Ctrl+F1); Check for Updates… | Palette; the About window has the update check |

Reset commands (user ruling 2026-11-01, formerly the Reset ▸ submenu): each panel has ↺ on its controls and Reset / Reset All at the bottom; the reset command ids (`video.reset` Q, `video.frame.reset` Numpad5/Alt+Backspace, `video.reframe.reset`, `audio.sync.reset` `?`, `audio.reset`, `move-reset` Alt+Home, `sync-reset` `/`, `reset-logo`) and their keys are unchanged. The `@reset` anchor remains for third-party plugins.

Global keys not in the menu: Shift+Tab brief info (info). Esc is Toggle Fullscreen's: a menu, dialog, panel editor or text field that handles Esc keeps it; otherwise it enters or leaves fullscreen.

### When the playlist has focus

Focus rules (user ruling 2026-11-08): only a single click on a row (on release, so a drag does not take it; or a right press on a row) and the search box's Esc/↓ give the list the focus; it shows no focus indicator. A double-click or Enter on a row (play), Esc in the list, and a click on the video give the keys back to the video. The wheel, the bar's buttons, a drag, a click below the rows or on the bar's background leave the focus where it was.

While the list has focus it handles the standard list keys itself (in `sidebar.slint`, unmodified; Shift extends the selection), as desktop players and file lists do; these are the only keys it takes from the player:

| Key | In the list | Global meaning it covers while the list has focus |
| --- | --- | --- |
| ↑ · ↓ | Select the previous / next row | Volume + / Volume − |
| PgUp · PgDn | Move the selection a page up / down | Previous / Next |
| Home · End | Select the first / last row | Previous / Next Dialogue |
| Enter | Play the selected row, then give the keys back to the video | (none) |
| Esc | Give the keys back to the video | Toggle Fullscreen |

With Shift the navigation keys extend the selection (so Shift+PgUp/PgDn, Previous/Next Bookmark or Chapter, are the list's too); with Ctrl, Alt or ⌘ the list takes none of them.

Every other key goes on to the list's key bindings below and then to the player (Space, ←, →, the media keys and so on keep their global meaning).

The list's key bindings work only while the playlist (the list or its search box) has focus (`focus: "panel"`). None of them is a global key (coordinator ruling 2026-11-08: the list shortcuts that shadowed a global key were removed in playlist 0.2.6). The list commands are not in the main context menu; they appear in the list's context menu, in the list's bottom-bar buttons and in the pop-up menus of the "Add" (＋) and "Sort" (⇅) buttons. Those with a key are panel-scoped and left out of the palette; those without one are in the palette ("Playlist: …").

| Command | Shortcut | Appears in | Notes |
| --- | --- | --- | --- |
| Play | Enter | List context menu | Enter is handled by the list itself (above); the binding names the key in the menu |
| Add Files… | | List context menu, Add | No shortcut (Ctrl+I stays Deinterlace: Cycle) |
| Add Folder… | | List context menu, Add | No shortcut (Ctrl+O stays Open File…) |
| Add Link… | | List context menu, Add | No shortcut (Ctrl+U stays Open URL…) |
| Remove Selected | Delete | List context menu | Backspace stays Jump to Start |
| Remove Unselected | Ctrl+Delete | List context menu | |
| Remove Invalid Items | | List context menu | No shortcut (Ctrl+T stays Always on Top: Cycle Mode) |
| Remove All | Alt+Delete | List context menu | |
| Show in Finder | Ctrl+F | List context menu | The title is the same on every system (it also says "Finder" on Windows) |
| Select All | Ctrl+A | List context menu | |
| Search | Ctrl+S | List context menu | |
| Open List File… | | List context menu | No shortcut (F3 stays Open File…) |
| Save List… | | List context menu | No shortcut (F2 stays Open Folder…) |
| Close List | | List context menu | No shortcut; global F6 (Playlist) hides the list too |
| Playlist Settings… | | Palette only | No shortcut (F5 stays Options…); opens Options ▸ Playback ▸ Playlist |
| Move to Top · Move Up · Move Down · Move to Bottom | — · Ctrl+↑ · Ctrl+↓ · Ctrl+End | List context menu (a group after Remove All; `Panel.showMenu` is one level, so not a Move ▸ submenu) | The bottom bar has no move buttons (design B2, 2026-11-09); drag reorders too. Move to Top has no shortcut (Ctrl+Home stays Current Subtitle Start); on macOS ⌃↑/⌃↓ are taken by the system by default; use ⌘↑/⌘↓ |
| Ascending · Descending | Ctrl+1 · Ctrl+2 | Sort | Radio |
| By Title · File Name · Extension · Size · Duration · Date · Random Order · Folder | Ctrl+3 … Ctrl+9, Ctrl+0 | Sort | |
| Sort by Path · Reverse Current Order | | Sort | No shortcut |

## English menu keys

Menus merge by English key ([i18n §5](../architecture/i18n.md#5-declarations-and-menus)); the tables above show the English display text, and the Simplified Chinese interface shows its translations. How keys are written (`▸` separators, submenu merging, leaves not registered) and the list of submenu keys declared by the built-in plugins are on the SDK documentation site, [Reference: Commands](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#menu-paths) ([Submenu keys in use](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#submenu-keys-in-use), generated from the SDK's `BUILTIN_SUBMENUS`). When a built-in plugin adds, moves or removes a menu entry, the SDK's `BUILTIN_COMMANDS` / `BUILTIN_SUBMENUS` tables change with it in one batch (SDK 1.6.17 for B5; `src/tests/sdk/host-agreement.test.ts` starts all built-in plugins and compares; any mismatch fails).

## UI forms

Slots: `bottom` (the control bar), `adjust` (one column of task panels beside the video, Alt+1–4, Adjustments and Effects in the menu and on the bottom bar), `sidebar` (tabs, always rightmost, Sidebar in the menu and on the bottom bar), `backdrop` (video-area layers), plus tool windows (`ui.window`, owned by the main window since host 0.7.7: they stay above it, hide and restore with it, and have no taskbar button or minimize button). The bottom bar's right end is the host-owned button strip: `[plugin buttons…][»][🔍][Adjustments and Effects][Sidebar]`.

| Plugin | UI |
| --- | --- |
| playback | Bottom bar (always present): source, seek bar (including the chapter/bookmark marks from marks), time, open/pause/speed/mute, volume; inline edit boxes (seek, speed, volume). Adjust panel **Playback** (Alt+4): speed ruler 0.25–4 (1× centred, snapping, double-click 1×, click to type), A-B Loop (mini timeline, A and B rows, Loop switch, Count, ↺ Chapter), jump, frame and keyframe steps, seek steps |
| playlist | Sidebar tab **Playlist** (F6): list and action buttons; "Add Link" tool window |
| video | Adjust panel **Picture** (Alt+1): one flat panel — Rendering (GPU Rendering, AI Upscaling), Colour, Effects, Geometry, Advanced (collapsed row) — no jump tabs; the Geometry editor works on a bundled still (Big Buck Bunny frame, CC BY 3.0); tool windows: custom aspect ratio, shader list |
| audio | Adjust panel **Audio** (Alt+2): device, channels, equalizer, sync, processing |
| subtitles | Adjust panel **Subtitles** (Alt+3): tracks, secondary subtitle, style, sync, files |
| capture | Sidebar tab **Capture** (Alt+S): Snapshot, Clips, Results, Settings sub-pages; range marks and OSD actions on the seek bar |
| marks | Tool window: chapter/bookmark list (H); seek-bar marks. Planned: Chapters sidebar tab (proposal step 8) |
| info | Read-only info overlay at the top left of the video (Tab, Shift+Tab); tool windows: playback properties, About Vestra Player |
| window | No windows of its own, only adjusts the main window (size, always on top, pointer, picture-in-picture); "Options ▸ Playback ▸ Window" settings page |
| visualizer | Video-area background layer (idle logo, album cover for audio only); settings page in "Options" |
| preferences | Tool window: Options (the player's settings window; this plugin provides the entry and the "Appearance" and "Language" pages) |
| hotkeys | Tool windows: keyboard shortcuts (names commands as "Category: Title"), mouse |
| subtitle-online | Tool windows: search and download, settings |
| history | Recently Played ▸ submenu; resume prompts |
| youtube, bilibili | Tool windows: enter a link, play (YouTube also has download) |
| workshop | Tool window: Vestra Player AI (plugin workshop); each plugin it generates and installs has its own window |
| store | Tool window: Plugin Store (list + details; its Installed tab is where plugins are managed); "Options ▸ Plugins ▸ Plugin Store" settings page; the video area accepts dropped `.vxp` files |

Showing, hiding or closing a panel or tool window does not affect effects the plugin has already applied; closing a tool does not undo processing and does not replace the bottom bar. Both columns (adjust, sidebar) can be open at once; the window remembers size, position and maximized state (not fullscreen).

## Conflict check

On 2026-10-21 every default binding (19 plugins, 14 core actions, the host's Fullscreen/Leave Fullscreen/Quit) was checked key by key per scope, treating `Cmd+X` and `Ctrl+X` as the same key on macOS (`CTRL_ACCEPTS_CMD`). B5 (2026-11-08) moved and removed menu entries only; no default key changed, so the result stands. New since then: Ctrl+Shift+P (Command Palette…, checked free on 2026-11-07), Alt+1–4 (panels), Alt+S (Capture tab), Ctrl+Shift+E / Ctrl+Shift+R (capture range) — none duplicates an existing default.

- **Global scope: one conflict, resolved.** `Alt+↑` and `Alt+↓` are default bindings of both "Pan ↑/↓" (video) and "Subtitle Style ▸ Move Up/Move Down" (subtitles). User ruling 2026-10-27: the video pan keeps Alt+↑/↓; subtitle Move Up/Move Down have no default key, users bind them if they want. No conflict remains.
- The host's only ⌘ binding is Cmd+Q (quit); no command declares Ctrl+Q; no other Ctrl binding duplicates it.
- Standard window close keys (macOS ⌘W; Windows Ctrl+W, Ctrl+F4; Linux Ctrl+W; only in tool windows, floating panels and the options window, see [Keys the system takes first](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#keys-the-system-takes-first)): no default binding uses Ctrl+W, Cmd+W or Ctrl+F4 (F4 is "Close", without a modifier, so no conflict).
- Panel focus scope (playlist): no conflicts. The Picture panel's Geometry editor takes plain arrows, `+` `=` `-`, and Esc/Enter only in crop mode while it has focus; modified arrows (Ctrl, Alt, Shift) pass through to the global commands.
- Cross-scope shadowing (by design, not a conflict): while the list has focus (only after a single click on a row) it uses the standard list navigation keys ↑ ↓ PgUp PgDn Home End and Esc (which gives the keys back to the video); no other list shortcut shadows a global key (coordinator ruling 2026-11-08); see [When the playlist has focus](#when-the-playlist-has-focus).
- `>` `<` `?` are typed characters and are different key names from `.` `,` `/` (on a US keyboard the former need Shift), so they do not conflict; the same goes for `{` `}` and `[` `]`.
- Ctrl+I / Tab, Ctrl+H / Backspace, Ctrl+M / Enter and Ctrl+[ / Esc are the same control character in terminal terms; the host tells them apart by physical key (`bindings.rs`), so they do not conflict.

System-reserved keys (the system may take them first; the full list per system is on the SDK documentation site, [Keys the system takes first](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#keys-the-system-takes-first); only affected bindings are listed):

- macOS ⌃F1–⌃F8 are system keyboard-navigation keys. Only **Ctrl+F1 (Playback Properties…)** is used: ⌃F1 is "Turn keyboard access on or off" by default, so only ⌘F1 triggers it reliably.
- macOS ⌃← / ⌃→ (switch Spaces) and ⌃↑ / ⌃↓ (Mission Control, application windows) are taken by the system by default: this affects "Back/Forward 30 s" (Ctrl+←/→, global) and the list's "Move Up/Move Down" (Ctrl+↑/↓, panel). The ⌘ versions work.
- macOS ⌘H (hide) and ⌘M (minimise): if the system takes the ⌘ versions of Deblock (Ctrl+H) and Temporal Denoise (Ctrl+M) first, only ⌃H and ⌃M work (a trade-off accepted in round 4a).
- On macOS F11 is "Show Desktop" by default, and on Apple keyboards F1–F12 are media keys by default (fn needed): Toggle Fullscreen is Esc and F11 is unbound; the commands on F1–F6 need fn on such keyboards.
- Windows: some graphics drivers use Ctrl+Alt+arrow keys for screen rotation and may intercept "Back/Forward 300 s" (Ctrl+Alt+←/→). No other binding uses a Windows reserved combination (Alt+Tab, Alt+F4, Alt+Space, Ctrl+Esc, Win key combinations).
