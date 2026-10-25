# Menus and UI ownership

> This table records the current implementation and corresponds to the top-level structure in [UI spec](ui-spec.md) §6.2.

The product's context menu tree is merged from the commands registered by each of the 19 plugins (since 2026-10-22 there is also one item from the built-in plugin store `store`, see the table below) (`youtube` and `bilibili` have moved to the plugin repository and are installed through the store; their two items only exist once they are installed), the host's 14 core actions, and the host's own "Fullscreen" and "Quit" (compiled on 2026-10-21 from what the plugin code on main fafa40d actually registers). Menus are organised by user domain, not by plugin. Before adding or moving an entry, check it against this table to make sure there is no duplicate entry and nothing is scattered into the wrong domain ([interaction requirements §1](interaction-requirements.md#1-menus-are-organised-by-user-domain)).

## Menu tree

2026-10-21, baseline main fafa40d. Compiled from the snapshot exported by `OUT=<file> node --test doc/process/rounds/menus-snapshot.test.ts` ([assignment table](../process/rounds/2026-10-plugin-ui.md) §3; at the time the script's default list covered all 19 plugins under `src/plugins`; after `youtube` and `bilibili` moved out, the default list is the other 17); the host's "Fullscreen", "Leave Fullscreen" and "Quit" are in `src/host/src/main.rs` (`register_host_commands`) and are not in the snapshot. Checked, radio and initially disabled states come from each command's `state` in the same test host.

The labels below are the English interface text (the English source text of each string); the Simplified Chinese interface shows the translations from the `zh-Hans.po` catalogs.

- Order is ascending by declared `order`; for equal `order`, by plugin id, then registration order. A submenu takes the smallest `order` among its items, and a group (`group`) takes the group of its first item.
- Exception: the positions of the 7 built-in top-level menus (`@open` … `@settings`) are fixed and do not follow the commands inside them: Open 110, Playback 200, Subtitles 300, Video 400, Audio 500, Screen 710, Settings 830 (`MENU_ANCHORS` in `src/host/src/commands.rs`). A plugin writing `["@video", …]`, the English key `["Video", …]` or the old Chinese title `["视频", …]` (Video) cannot move it. In the level directly below a built-in menu, items with `order` 0 (the plugin omitted `order`) come last ([Order and separators](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_key_bindings.html#order-and-separators)); built-in commands at that level all use an `order` of 100 or more, never 0. To change the top-level position of a built-in menu, change this table, not the commands' `order`.
- "Plugin" column: "core" is a host core action (`host/<id>`), "host" is the host's own command.
- Top-level separators are drawn as "——" rows (two adjacent items with different `group`). Inside a submenu a separator is also drawn between adjacent items with different `group`; the table does not draw each one.
- Shortcuts work in the global scope (when the video area, bottom bar or sidebar has focus; not inside tool windows). Shortcuts written with `Ctrl` can be triggered on macOS by both ⌃ and ⌘. Users can rebind them in "Settings ▸ Keyboard Shortcuts…", or turn off all default bindings with "Ignore Default Shortcuts".
- "Dynamic" items are generated at run time from the current media, records or devices; "disabled" items are always present but greyed out with a reason when their condition is not met.

| Menu path | Plugin | Shortcut | Notes |
| --- | --- | --- | --- |
| Open File… | playback | Ctrl+O, F3 | |
| Open ▸ Open URL… | playback | Ctrl+U | |
| Open ▸ Open Folder… | playlist | F2 | |
| Open ▸ YouTube Video… | youtube | | Official plugin from the plugin repository; appears after installation |
| Open ▸ Bilibili Video… | bilibili | | Official plugin from the plugin repository; appears after installation |
| Open ▸ Open Subtitle… | subtitles | Alt+O | |
| Open ▸ Load External Audio… | audio | | Disabled when no file is playing |
| Open ▸ Add Secondary Subtitle… | subtitles | | Disabled when there is no playable media |
| Open ▸ Recently Played ▸ (one item per recent entry) | history | | Dynamic; with no entries shows a greyed-out "(No Entries)" |
| Open ▸ Recently Played ▸ Remove Invalid Entries · Clear Entries | history | | |
| Open ▸ Recently Played ▸ Record Playback History | history | | Checked |
| Close | core | F4 | |
| Reopen Current/Last File | playback | Ctrl+Y | |
| Copy to Clipboard ▸ File Name · Folder Path · Full Path | playback | | |
| —— | | | |
| Playback ▸ Play/Pause | core | Space | |
| Playback ▸ Previous | core | PgUp, mouse side button X1 | |
| Playback ▸ Next | core | PgDn, mouse side button X2 | |
| Playback ▸ Seek ▸ Previous Frame · Next Frame | core | D · F | |
| Playback ▸ Seek ▸ Previous Keyframe · Next Keyframe | playback | Ctrl+Shift+← · Ctrl+Shift+→ | |
| Playback ▸ Seek ▸ Back 5 s · Forward 5 s | core | ← · → | |
| Playback ▸ Seek ▸ Back/Forward 30 s | playback | Ctrl+← · Ctrl+→ | On macOS ⌃←/⌃→ are taken by the system by default; use ⌘←/⌘→ (see Conflict check) |
| Playback ▸ Seek ▸ Back/Forward 60 s | playback | Shift+← · Shift+→ | |
| Playback ▸ Seek ▸ Back/Forward 300 s | playback | Ctrl+Alt+← · Ctrl+Alt+→ | |
| Playback ▸ Seek ▸ Previous Dialogue · Next Dialogue · Current Subtitle Start | subtitles | Home · End · Ctrl+Home | |
| Playback ▸ Seek ▸ Jump to Start | playback | Backspace | |
| Playback ▸ Seek ▸ Seek Steps… | playback | | |
| Playback ▸ Jump to Time… | playback | G | |
| Playback ▸ Speed ▸ Normal | core | Z | Radio |
| Playback ▸ Speed ▸ Slower 0.1 · Faster 0.1 | core | X · C | |
| Playback ▸ Speed ▸ 0.5× · 1.5× · 2× | playback | | Radio (same group as "Normal") |
| Playback ▸ Speed ▸ Custom… | playback | | |
| Playback ▸ Playlist ▸ Add Files… · Add Link… · Add Folder… | playlist | | |
| Playback ▸ Playlist ▸ Repeat ▸ Off · All · One | playlist | | Radio |
| Playback ▸ Playlist ▸ Shuffle | playlist | | Checked |
| Playback ▸ Playlist ▸ Open List File… · Save List… | playlist | | |
| Playback ▸ Playlist ▸ When Opening a File ▸ Open Only That File · Add Similar Files · Add All Files in the Folder | playlist | | Radio |
| Playback ▸ Playlist ▸ Fast Advance to Next · Remember List · Confirm Before Removing All | playlist | | Checked |
| Playback ▸ Disable Screen Saver While Playing | playback | | Checked |
| Playback ▸ Hide Pointer While Playing | window | | Checked, on by default (G-06) |
| Playback ▸ Show Time on Seek Bar Hover | playback | | Checked |
| Playback ▸ Show Thumbnail on Seek Bar Hover | playback | | Checked; disabled without ffmpeg |
| Playback ▸ Resume Playback ▸ Don't Resume · Resume Automatically · Ask Every Time | history | | Radio |
| Playback ▸ Resume Playback ▸ Also Remember Audio Files | history | | Checked |
| Playback ▸ Resume Playback ▸ Clear All Saved Positions | history | | |
| Playback ▸ Chapters and Bookmarks ▸ Chapter and Bookmark List… | marks | H | |
| Playback ▸ Chapters and Bookmarks ▸ Add Bookmark | marks | P | Disabled when no media is open |
| Playback ▸ Chapters and Bookmarks ▸ Previous Bookmark or Chapter · Next Bookmark or Chapter | marks | Shift+PgUp · Shift+PgDn | |
| Playback ▸ Chapters and Bookmarks ▸ Previous Chapter · Next Chapter | marks | Ctrl+PgUp · Ctrl+PgDn | |
| Playback ▸ Chapters and Bookmarks ▸ (one item per chapter) | marks | | Dynamic, radio (current chapter); at most 50 items, beyond that a final "More Chapters… (N in all)" |
| Playback ▸ Chapters and Bookmarks ▸ (one item per bookmark) | marks | | Dynamic; at most 50 items, beyond that a final "More Bookmarks… (N in all)" |
| Playback ▸ Chapters and Bookmarks ▸ Show Marks on Seek Bar | marks | | Checked; last in the submenu |
| Playback ▸ A-B Loop ▸ Set Start Point · Clear Start Point | marks | `[` · `{` | |
| Playback ▸ A-B Loop ▸ Start Point Back · Forward 0.1 s | marks | Ctrl+[ · Alt+[ | |
| Playback ▸ A-B Loop ▸ Set End Point · Clear End Point | marks | `]` · `}` | |
| Playback ▸ A-B Loop ▸ End Point Back · Forward 0.1 s | marks | Ctrl+] · Alt+] | |
| Playback ▸ A-B Loop ▸ Move Range Back · Forward 0.1 s | marks | Ctrl+Alt+[ · Ctrl+Alt+] | |
| Playback ▸ A-B Loop ▸ Loop Current Chapter or Bookmark Range | marks | Alt+\ | |
| Playback ▸ A-B Loop ▸ A-B Loop On/Off | marks | `\` | Checked |
| Playback ▸ A-B Loop ▸ Loop Count ▸ Endless · Once · 2 Times · 3 Times · 5 Times · 10 Times | marks | | Radio |
| Subtitles ▸ Show Subtitles | subtitles | Alt+H | Checked |
| Subtitles ▸ Subtitle Track ▸ Off · (actual track names) | subtitles | | Radio; track names are dynamic |
| Subtitles ▸ Subtitle Track ▸ Cycle | subtitles | Alt+L | |
| Subtitles ▸ Secondary Subtitle ▸ Off · (actual track names) | subtitles | | Radio; track names are dynamic |
| Subtitles ▸ Secondary Subtitle ▸ Position ▸ Top · Above Main Subtitle | subtitles | | Radio |
| Subtitles ▸ Subtitle Style ▸ Bold | subtitles | Alt+B | Checked |
| Subtitles ▸ Subtitle Style ▸ Size + · Size − | subtitles | Alt+PgUp · Alt+PgDn | |
| Subtitles ▸ Subtitle Style ▸ Move Up · Move Down · Reset Position | subtitles | Alt+↑ · Alt+↓ · Alt+Home | Alt+↑/↓ duplicate "Video ▸ Framing ▸ Pan ↑/↓" (see Conflict check) |
| Subtitles ▸ Subtitle Style ▸ Shadow · Opaque Box | subtitles | | Checked |
| Subtitles ▸ Original ASS/SSA Styles | subtitles | | Checked |
| Subtitles ▸ Sync ▸ Earlier 0.5 s · Later 0.5 s · Reset | subtitles | `.` · `,` · `/` | |
| Subtitles ▸ Sync ▸ Adjust Sync… | subtitles | | |
| Subtitles ▸ Online Subtitles ▸ Search and Download Subtitles… · Online Subtitle Settings… | subtitle-online | | |
| Subtitles ▸ Reload Subtitles | subtitles | Ctrl+Alt+Y | |
| Subtitles ▸ Refresh Subtitle Tracks · Subtitle Tuning… · Subtitle Settings… | subtitles | | |
| —— | | | |
| Video ▸ Pixel Shaders ▸ Use Shaders | video | Shift+P | Checked; disabled with software rendering ("Needs GPU rendering (currently software rendering)") |
| Video ▸ Pixel Shaders ▸ Shader List… · Open Shader Folder | video | | Also available with software rendering |
| Video ▸ Pixel Shaders ▸ Reload Shaders | video | | Disabled with software rendering |
| Video ▸ Pixel Shaders ▸ HDR Tone Mapping ▸ Auto · Off · BT.2390 · Hable · Mobius · Reinhard · Clip | video | | Radio; disabled with software rendering |
| Video ▸ Pixel Shaders ▸ HDR Tone Mapping ▸ Dynamic Peak Detection | video | | Checked; disabled with software rendering |
| Video ▸ Pixel Shaders ▸ HDR Settings… | video | | Disabled with software rendering |
| Video ▸ Deinterlace ▸ Cycle | video | Ctrl+I | |
| Video ▸ Deinterlace ▸ Auto · Always · Never | video | | Radio |
| Video ▸ Deinterlace ▸ Output ▸ By Frame · By Field (Double Rate) | video | | Radio |
| Video ▸ Aspect ▸ Keep Aspect Ratio · Ignore Aspect Ratio · Fill Window (Crop Excess) | video | | Radio |
| Video ▸ Aspect ▸ Cycle Aspect Ratio | video | Shift+F6 | |
| Video ▸ Aspect ▸ Default (Source) · 4:3 · 16:9 · 16:10 · 1.85:1 · 2.35:1 · Custom (x:y) | video | | Radio; "Custom" is disabled until a ratio has been entered, after which its title becomes e.g. "Custom (21:9)" |
| Video ▸ Aspect ▸ Custom Aspect Ratio… | video | | |
| Video ▸ Crop/Stretch ▸ None · Stretch to 4:3 / 16:9 / 16:10 / 1.85:1 / 2.35:1 · Crop to the same 5 ratios | video | | Radio |
| Video ▸ Crop/Stretch ▸ Reset Position · Move Left · Move Right · Move Up · Move Down | video | | |
| Video ▸ Framing ▸ Reset Framing | video | Numpad5, Alt+Backspace | |
| Video ▸ Framing ▸ Zoom In · Zoom Out | video | Numpad9, Alt+Plus · Numpad1, Alt+- | |
| Video ▸ Framing ▸ Pan → · ← · ↑ · ↓ | video | Ctrl+Numpad6, Alt+→ · Ctrl+Numpad4, Alt+← · Ctrl+Numpad8, Alt+↑ · Ctrl+Numpad2, Alt+↓ | Alt+↑/↓ duplicate subtitle Move Up/Move Down (see Conflict check) |
| Video ▸ Framing ▸ Center Frame | video | Ctrl+Numpad5 | |
| Video ▸ Image Processing ▸ Flip Vertically · Flip Horizontally | video | Ctrl+P · Ctrl+Z | Checked |
| Video ▸ Image Processing ▸ Gaussian Blur · Sharpen · Edge Sharpen | video | Ctrl+B · Ctrl+R · none | Checked |
| Video ▸ Image Processing ▸ Deblock · Deband · 3D Denoise · Gradual Denoise · Temporal Denoise | video | Ctrl+H · none · Ctrl+N · none · Ctrl+M | Checked |
| Video ▸ Image Processing ▸ Frame Doubling | video | | Checked |
| Video ▸ Image Processing ▸ Motion Interpolation | video | | Checked; disabled with software rendering |
| Video ▸ Rotation ▸ Cycle | video | Alt+K | |
| Video ▸ Rotation ▸ No Rotation · 90° · 180° · 270° | video | | Radio |
| Video ▸ Snapshot ▸ Save Current Frame · Save Current Frame As… · Copy Current Frame | capture | Ctrl+E · none · Ctrl+C | Disabled when there is no frame to capture |
| Video ▸ Snapshot ▸ Save Rendered Frame · Save Rendered Frame As… · Copy Rendered Frame | capture | Ctrl+Alt+E · none · Ctrl+Alt+C | Same as above |
| Video ▸ Snapshot ▸ Burst… | capture | Ctrl+G | Checked (while the burst window is open) |
| Video ▸ Snapshot ▸ Export Clip… · Export Tasks… | capture | | |
| Video ▸ Snapshot ▸ Capture as PNG · JPEG · BMP | capture | | Radio |
| Video ▸ Snapshot ▸ Include Subtitles in Rendered Frame | capture | | Checked |
| Video ▸ Snapshot ▸ Snapshot Settings… | capture | | |
| Video ▸ Snapshot ▸ Snapshot Folder… · Open Snapshot Folder | capture | | |
| Video ▸ Picture Adjustments… | video | | |
| Video ▸ Reset | video | Q | |
| Video ▸ Brightness −1% · +1% | video | W · E | |
| Video ▸ Contrast −1% · +1% | video | R · T | |
| Video ▸ Saturation −1% · +1% | video | Y · U | |
| Video ▸ Hue −1% · +1% | video | I · O | |
| —— | | | |
| Audio ▸ Volume + · Volume − | core | ↑, wheel up · ↓, wheel down | |
| Audio ▸ Mute | core | M | Checked |
| Audio ▸ Audio Workbench… | audio | | |
| Audio ▸ Audio Track ▸ (actual audio track names) | audio | | Radio; dynamic; with no audio tracks shows a greyed-out "No Audio Tracks" ("Loading…" while loading) |
| Audio ▸ Audio Track ▸ Cycle | audio | Alt+A | |
| Audio ▸ Audio Track ▸ Stereo · Left Channel · Right Channel · Mono Mix · Swap Left and Right | audio | | Radio |
| Audio ▸ Output Device ▸ (one item per output device) | audio | | Radio; dynamic; disconnected devices are greyed out ("Device not connected"); with no devices shows a greyed-out "No Output Devices" |
| Audio ▸ Equalizer | audio | Shift+E | Checked |
| Audio ▸ Preset ▸ Flat · Voice · Music · Cinema | audio | | Radio |
| Audio ▸ Audio Sync ▸ 50 ms Earlier · 50 ms Later · Reset | audio | `>` · `<` · `?` | |
| Audio ▸ Audio Sync ▸ Don't Keep · Keep for This Run · Always Keep | audio | | Radio |
| Audio ▸ Processing ▸ Normalize · Crystalizer | audio | Shift+N · Shift+C | Checked |
| Audio ▸ Reset | audio | | |
| Audio ▸ Show Album Cover for Audio Only | visualizer | | Checked |
| Emulator ▸ Open NES ROM… · Stop NES · NES Tools… | nes | | |
| —— | | | |
| Fullscreen | host | Enter, F11, double-click video | Checked; key `Toggle Fullscreen` (id `fullscreen`); in fullscreen Esc leaves fullscreen (command "Leave Fullscreen", key `Leave Fullscreen`, not in the menu) |
| Screen ▸ Picture-in-Picture | window | | Checked; appears only when the main window reports support for the system's picture-in-picture (macOS 14+) (id `pip`, IA-WIN-01); disabled without video (can still be left while in picture-in-picture); if mini size is active, the original size is restored first |
| Screen ▸ Mini Size | window | `` ` `` | Checked; all systems (id `mini`, IA-WIN-01); not on top (IA-WIN-02); disabled without video and in fullscreen; run in picture-in-picture, it leaves picture-in-picture first |
| Screen ▸ 0.5× · 1× · 1.5× · 2× | window | 1 · 2 · 3 · 4 | Radio; disabled without video |
| Screen ▸ Fit to Video Aspect Ratio | window | 8 | Disabled without video |
| Screen ▸ When Playback Starts ▸ Don't Resize · 0.5× · 1× · 1.5× · 2× · Fit to Video Aspect Ratio | window | | Radio, default 1× (G-02) |
| Screen ▸ When Playback Starts ▸ Only Once | window | | Checked, off by default (G-02) |
| Screen ▸ Always on Top ▸ Cycle Mode | window | Ctrl+T | Cycles Never → Always → While Playing → While Playing in Fullscreen |
| Screen ▸ Always on Top ▸ Never · Always · While Playing · While Playing in Fullscreen | window | | Radio, default "While Playing in Fullscreen" (IA-WIN-03) |
| —— | | | |
| Playlist | playlist | F6 | Checked |
| Playback Info | info | Tab | Checked |
| Playback Properties… | info | Ctrl+F1 | On macOS ⌃F1 is reserved by the system; use ⌘F1 (see Conflict check) |
| Vestra Player AI… | workshop | | |
| Plugin Store… | store | | `order` 822, `group: "windows"` (decided in step 2.2 on 2026-10-22); when plugins have updates, the title is "Plugin Store (N updates)…". The store's other entry points are not in the main menu: drag a `.vxp` onto the video area to install it; the "Options ▸ Plugins ▸ Plugin Store" settings page (third-party repositories, automatic checks) |
| —— | | | |
| Settings ▸ Options… | preferences | F5 | |
| Settings ▸ Keyboard Shortcuts… · Mouse… | hotkeys | | |
| Settings ▸ Ignore Default Shortcuts | hotkeys | | Checked |
| Settings ▸ Plugin Manager… | plugins | | |
| Settings ▸ Appearance ▸ Vestra Dark · Vestra Light | preferences | | Radio (palette); `order` 840, 841. Dark is the default; with Light the video area stays dark |
| Settings ▸ Appearance ▸ Show Logo When Idle | visualizer | | Checked |
| Settings ▸ Appearance ▸ Choose Logo Image… · Restore Default Logo | visualizer | | "Restore Default Logo" is disabled while the default logo is in use |
| —— | | | |
| About Vestra Player… | info | F1 | |
| —— | | | |
| Quit | host | Cmd+Q | key `Quit` |

Global keys not in the menu: Shift+Tab brief info (info), Esc "Leave Fullscreen" (host; only in fullscreen; in windowed mode the key goes to the focused control as usual).

### When the playlist has focus

The following keys only work while the playlist (the list or its search box) has focus (`focus: "panel"`), and take precedence over global commands on the same key; once focus leaves, the global meaning returns. They are not in the main context menu; they appear in the list's context menu, in the list's bottom-bar buttons, and in the pop-up menus of the "Add" and "Sort" buttons (panel context menus; before the 2026-10 visual identity round these two buttons were the text buttons ADD and SORT, now they are the icons ＋ and ⇅, with the same position and size).

| Command | Shortcut | Appears in | Notes |
| --- | --- | --- | --- |
| Play | Enter, P | List context menu | Shadows global Enter (fullscreen), P (add bookmark) |
| Add Files… | Ctrl+I | List context menu, Add | Shadows global Ctrl+I (cycle deinterlace) |
| Add Folder… | Ctrl+O | List context menu, Add | Shadows global Ctrl+O (open file) |
| Add Link… | Ctrl+U | List context menu, Add | Shadows global Ctrl+U (open URL) |
| Remove Selected | Delete, Backspace | List context menu | Shadows global Backspace (jump to start) |
| Remove Unselected | Ctrl+Delete | List context menu | |
| Remove Invalid Items | Ctrl+T | List context menu | Shadows global Ctrl+T (cycle always-on-top mode) |
| Remove All | Alt+Delete | List context menu | |
| Show in Finder | Ctrl+F | List context menu | The title is the same on every system (it also says "Finder" on Windows) |
| Select All | Ctrl+A | List context menu | |
| Search | Ctrl+S | List context menu | |
| Open List File… | F3 | List context menu | Shadows global F3 (open file) |
| Save List… | F2 | List context menu | Shadows global F2 (open folder) |
| Close List | F6 | List context menu | Same effect as global F6 (hide the list) |
| Move to Top · Move Up · Move Down · Move to Bottom | Ctrl+Home · Ctrl+↑ · Ctrl+↓ · Ctrl+End | List bottom-bar buttons | Ctrl+Home shadows the global binding (current subtitle start); on macOS ⌃↑/⌃↓ are taken by the system by default; use ⌘↑/⌘↓ |
| Ascending · Descending | Ctrl+1 · Ctrl+2 | Sort | Radio |
| By Title · File Name · Extension · Size · Duration · Date · Random Order · Folder | Ctrl+3 … Ctrl+9, Ctrl+0 | Sort | |
| Sort by Path · Reverse Current Order | | Sort | No shortcut |

## English menu keys

Menus merge by English key ([i18n §5](../architecture/i18n.md#5-declarations-and-menus)); the tables above show the English display text, and the Simplified Chinese interface shows its translations. How keys are written (`▸` separators, submenu merging, leaves not registered) and the list of submenu keys declared by the built-in plugins are on the SDK documentation site, [Reference: Commands](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#menu-paths) ([Submenu keys in use](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#submenu-keys-in-use), generated from the SDK's `BUILTIN_SUBMENUS`). When a built-in plugin adds or changes a submenu key, the SDK's `BUILTIN_SUBMENUS` changes with it (`src/tests/sdk/host-agreement.test.ts` starts all built-in plugins and compares; any mismatch fails).

## UI forms

| Plugin | UI |
| --- | --- |
| playback | Bottom bar (always present): source, seek bar (including the chapter/bookmark marks from marks), time, open/pause/speed/mute, volume; inline edit boxes (seek, speed, volume) |
| playlist | Sidebar (can be hidden): list and action buttons; "Add Link" tool window |
| video | Tool windows: picture adjustments (tabbed, including the HDR settings page), custom aspect ratio, shader list |
| audio | Tool window: audio workbench |
| subtitles | Tool window: subtitle workbench |
| capture | Tool window: snapshots and export |
| marks | Tool window: chapter/bookmark list |
| info | Read-only info overlay at the top left of the video (Tab, Shift+Tab); tool windows: playback properties, About Vestra Player |
| window | No windows of its own, only adjusts the main window (size, always on top, pointer, picture-in-picture); "Options ▸ Playback ▸ Window" settings page |
| visualizer | Video-area background layer (idle logo, album cover for audio only); settings page in "Options" |
| preferences | Tool window: Options (the player's settings window; this plugin provides the entry and the "Appearance" page) |
| hotkeys | Tool windows: keyboard shortcuts, mouse |
| plugins | Tool window: plugin manager |
| nes | Tool window: NES |
| youtube, bilibili | Tool windows: enter a link, play (YouTube also has download) |
| workshop | Tool window: Plugin Workshop; each plugin it generates and installs has its own window |
| store | Tool window: Plugin Store (list + details); "Options ▸ Plugins ▸ Plugin Store" settings page; the video area accepts dropped `.vxp` files (a drop provider for the `vxp` extension) |

Showing, hiding or closing a tool window does not affect effects the plugin has already applied; closing a tool does not undo processing and does not replace the bottom bar.

## Conflict check

On 2026-10-21 every default binding in the tables above (19 plugins, 14 core actions, the host's Fullscreen/Leave Fullscreen/Quit) was checked key by key per scope, treating `Cmd+X` and `Ctrl+X` as the same key on macOS (`CTRL_ACCEPTS_CMD`).

- **Global scope: one conflict.** `Alt+↑` and `Alt+↓` are default bindings of both "Video ▸ Framing ▸ Pan ↑/↓" (video, the alternate keys from round 4a) and "Subtitles ▸ Subtitle Style ▸ Move Up/Move Down" (subtitles). The host lets the plugin loaded first (the owner with the smaller generation) win; the other is ignored and `binding conflict` is printed; Ctrl+Numpad8/2 (framing) are not affected. The round 4a conflict check in the "Framing" section of [feature requirements](feature-requirements.md) only checked playback's arrow keys and missed subtitles. One side's default keys need to change (awaiting a ruling).
- The host's only ⌘ binding is Cmd+Q (quit); no command declares Ctrl+Q; no other Ctrl binding duplicates it.
- Standard window close keys (macOS ⌘W; Windows Ctrl+W, Ctrl+F4; Linux Ctrl+W; only in tool windows, floating panels and the options window, not in the binding table, see [Keys the system takes first](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#keys-the-system-takes-first)): checked on 2026-10-22, no default binding uses Ctrl+W, Cmd+W or Ctrl+F4 (F4 is "Stop", without a modifier, so no conflict).
- Panel focus scope (playlist): no conflicts.
- Cross-scope shadowing (by design, not a conflict): while the list has focus, Enter, P, Ctrl+I, Ctrl+O, Ctrl+U, Backspace, Ctrl+T, Ctrl+Home, F3, F2 and F6 have the list's own meaning; see the notes in the previous section.
- `>` `<` `?` are typed characters and are different key names from `.` `,` `/` (on a US keyboard the former need Shift), so they do not conflict; the same goes for `{` `}` and `[` `]`.
- Ctrl+I / Tab, Ctrl+H / Backspace, Ctrl+M / Enter and Ctrl+[ / Esc are the same control character in terminal terms; the host tells them apart by physical key (`bindings.rs`), so they do not conflict.

System-reserved keys (not duplicates within this table, but the system may take them first when pressed; the full list per system is on the SDK documentation site, [Keys the system takes first](https://vestrasys.github.io/vestra-player-sdk/documents/Commands,_menus_and_bindings.html#keys-the-system-takes-first); only the bindings in this table that are affected are listed below):

- macOS ⌃F1–⌃F8 are system keyboard-navigation keys. This table only uses **Ctrl+F1 (Playback Properties…)**: ⌃F1 is "Turn keyboard access on or off" by default, so only ⌘F1 triggers it reliably. ⌃F2–⌃F8 are unused.
- macOS ⌃← / ⌃→ (switch Spaces) and ⌃↑ / ⌃↓ (Mission Control, application windows) are taken by the system by default: this affects "Back/Forward 30 s" (Ctrl+←/→, global) and the list's "Move Up/Move Down" (Ctrl+↑/↓, panel). The ⌘ versions work.
- macOS ⌘H (hide) and ⌘M (minimise): if the system takes the ⌘ versions of Deblock (Ctrl+H) and Temporal Denoise (Ctrl+M) first, only ⌃H and ⌃M work (a trade-off accepted in round 4a).
- On macOS F11 is "Show Desktop" by default, and on Apple keyboards F1–F12 are media keys by default (fn needed): fullscreen also has Enter and double-click; the commands on F1–F6 need fn on such keyboards.
- Windows: some graphics drivers use Ctrl+Alt+arrow keys for screen rotation and may intercept "Back/Forward 300 s" (Ctrl+Alt+←/→). No other binding uses a Windows reserved combination (Alt+Tab, Alt+F4, Alt+Space, Ctrl+Esc, Win key combinations).
