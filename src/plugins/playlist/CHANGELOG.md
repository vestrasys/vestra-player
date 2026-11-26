# Changelog

## 0.2.12

- **Events and triggers (SDK 1.11, release B step B3)**: `plugin.player.subscribe()` and `plugin.player.onChange()` are gone. While the list shows, its rows follow the player with `plugin.player.stream()`, opened when the list appears and stopped when it is hidden (the sidebar closed, another tab in front, the fullscreen overlay collapsed); the stream's first state catches up on what changed meanwhile. While it is hidden the plugin hears `source.opening`, `source.ready` and `source.ended` instead: the bound row's status and length, an adopted row's natural end (which advances), a failure, a close (one `player.refresh()`, since the ended source's state does not say the player is empty), A13's following of sources other plugins open, and the thumbnails' hold while a source opens. The sequence provider, Play in New Window and the menus are unchanged; the plugin stays running for its sequence provider as before. Behaviour is unchanged. Who must change: nobody. Tests: the A13 cases, removing the playing row, and errors and stops also run with the list hidden (`PLAYLIST_TEST_HIDDEN=1` runs every case so).

## 0.2.11

- The hidden element that measures the hint in the Add Links window is placed (`x`, `y`), so its layout limits stay out of the window's. Nothing changes today; the same unplaced element made the Plugin Store's maximum height 0, so its size limits swapped (Store 0.3.3). Who must change: nobody.

## 0.2.10

- **Play in New Window** (user ruling 2026-11-21, player 0.10.3): a new item in the list's right-click menu, right after Play. The selected local files (the first 64, in list order) play in a new temporary window: a separate player with its own temporary profile, whose title says "(Temporary Window)". Disabled, with the reasons Show in Folder gives, when no selected row is a local file. The right-click menu is its only entry: the command (`playlist.play-new-window`) has `focus: "panel"` and no key, so the command palette does not list it. Not on macOS yet (left out of the menu there). It calls `plugin.app.openWindow` (SDK 1.10.1, built-in plugins only); a player before 0.10.3 refuses it, shown on the OSD. Translations in all six languages. Who must change: nobody.

## 0.2.9

- The description's last line, starting with `→ `, says where to open the plugin; Store 0.3.1 shows it on the plugin's card, also before install. Description text and its translations only. Who must change: nobody.

## 0.2.8

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.2.7

- The list command Show in Folder (`reveal`, Ctrl+F) is called "Show in Folder" on every system, as capture's snapshot notice says it, in all six languages (it was "Show in Finder", also on Windows). Id, key and behaviour are unchanged. Who must change: nobody.

## 0.2.6

- The commands that take rows out of the list declare `destructive` (SDK 1.7.9): Remove Selected (`remove`), Remove Unselected (`remove-unselected`), Remove Invalid Items (`remove-invalid`), Remove All (`clear`). When another built-in plugin runs one of them through `commands.run` (Ask Vestra), the player asks the user first; a menu pick, a key or the command palette runs them as before. Open List File… is not marked: it asks "Replace…?" itself, so the user is not asked twice. Who must change: nobody.
- Repeat button and Stop After Current (design B2, 2026-11-09): the bottom bar loses its four move buttons (Move to Top / Up / Down / to Bottom) and gains a Repeat button after the thumbnail toggle; a click cycles Off → All → One → Stop After Current → Off. The button keeps the bar's colour in every mode; its icon shows the mode (Off: the loop with a slash; All: the loop; One: the loop with "1"; Stop After Current: an arrow into a stop block) and its tooltip names it ("Repeat: Off (play the list once)", "Repeat: All (loop the list)", "Repeat: One (loop this video)", "Repeat: Stop After Current"). The search box gets the room back (at the narrowest sidebar, 317px, it is no longer folded). Who must change: nobody.
- New repeat mode **Stop After Current** (`repeat-once`, palette "Repeat: Stop After Current", fourth item of the settings page's Repeat list, notice "Repeat: Stop After Current"): the current item plays once and playback stops at its end; the list does not advance and the current row stays, as at the end of the list. It is saved like the other modes (`repeat: "once"`); shuffle has no effect in it, and manual Next/Previous still work. Who must change: nobody.
- The list's right-click menu has the four moves as a group after the Remove items (Move to Top, Move Up Ctrl+↑, Move Down Ctrl+↓, Move to Bottom Ctrl+End): Move to Top had no other place than its bar button. `Panel.showMenu` is one level, so they are a group, not a "Move ▸" submenu. The bridge loses `can-top`, `can-up`, `can-down`, `can-bottom` and the `place` callback, and gains `repeat` and `cycle-repeat`. Who must change: nobody.
- No focus ring (user ruling 2026-11-08): the list no longer draws the 1px accent ring while it has the keys; which clicks and keys focus it, and what it does with the keys, are unchanged. Who must change: nobody.
- Menu sweep (menu redesign §4): the whole Playback ▸ Playlist ▸ submenu (Add Files/Link/Folder…, Repeat ▸, Shuffle, Open List File…, Save List…, When Opening a File ▸, Fast Advance to Next, Remember List, Confirm Before Removing All) leaves the right-click menu and is in the command palette only ("Playlist: …", "Repeat: …", "When Opening a File: …"), with ids, keys, radio and checked states unchanged; Playlist (F6) and Open ▸ Open Folder… stay. Who must change: nobody.
- The description names the command palette instead of Playback ▸ Playlist, in all six languages.
- List focus (user ruling 2026-11-08): only a click on a row (on release, so a drag does not take it; or a right press on a row) and the search box's Esc/↓ focus the list; the bar's buttons, a disabled button, the bar's background, a click below the rows, the wheel and a drag leave the focus where it was. While focused the list handles ↑ ↓ PgUp PgDn (new: a page of rows) Home End itself; every other key goes on to the list's key bindings and then to the player. Who must change: nobody.
- Keys back to the video (`VestraFocus.release()`, SDK 1.7.0): playing an item from the list (a double-click, whose second release no longer focuses the list, or Enter, which the list now handles itself through the new `play-selected` callback) and Esc in the list give the keyboard back to the video, so the arrows and Space control the player again. While the list has focus it uses the navigation keys ↑ ↓ PgUp PgDn Home End and Esc, and no other list shortcut shadows a player key (coordinator ruling 2026-11-08): Play loses P (Add Bookmark), Add Files… Ctrl+I (Deinterlace: Cycle), Add Folder… Ctrl+O and Open List File… F3 (Open File…), Add Link… Ctrl+U (Open URL…), Remove Selected Backspace (Jump to Start), Remove Invalid Items Ctrl+T (Always on Top: Cycle Mode), Move to Top Ctrl+Home (Current Subtitle Start), Save List… F2 (Open Folder…), Close List F6 (Playlist, which hides it too) and Playlist Settings… F5 (Options…); they stay in the list menu and the command palette, and the Move to Top tooltip drops its key. Who must change: users of the removed list keys (bind them in Keyboard Shortcuts).

Who must change: nobody. What users see: in the command palette (Ctrl+Shift+P) playlist commands name what they are about, for example "Repeat: All" or "When Opening a File: Add Similar Files", and say where their control lives (the playlist, or Playlist Settings); lists of keys name the list commands the same way ("Sort: By Size"). Needs player 0.7.6.

- Every command declares a `category` (SDK 1.6.4): Playlist, Repeat, When Opening a File or Sort, the labels of the submenus and popups the commands sit in, with their existing translations.
- Commands whose control is in the playlist panel (the list commands, Add, Open and Save List) declare `home` "playlist.visibility"; the options that are also on the settings page (Repeat, Shuffle, When Opening a File, Fast Advance to Next, Remember List, Confirm Before Removing All) declare `home` "playlist.settings".
- **Playlist** (`playlist.visibility`, F6) leaves the right-click menu and is in the command palette only (user ruling 2026-11-08): the right-click menu's player items **Adjustments and Effects** and **Sidebar** (named like the bottom-bar buttons) open the adjust column and the sidebar at their last tab instead. Title, category, id, key and checked state are unchanged. The description says so. Who must change: nobody.

## 0.2.5

Who must change: nobody. What users see: the list's header has no close button (✕) any more; right below the window's own close button, a missed click closed the player. Hide the sidebar with F6, the bottom bar's sidebar button or Close List in the list menu. Needs player 0.6.0.

- The header keeps only the detach/attach button, at its right end; the `hide` callback and its "Close list" text are gone.

## 0.2.4

- Code only: the Add Link window handle is named `linkWindow` (was `window`). Who must change: nobody. No behaviour change.

## 0.2.3

Who must change: nobody. What users see: with long lists a click shows a little sooner (20,000 rows: about 40 to 21 ms in the plugin with SDK 1.5.47). Needs player 0.6.0.

- Each row the list is sent is frozen and stays the same object while it does not change, so SDK 1.5.47 reuses its encoding instead of encoding all rows on every update.
- The comparison of a row with the one sent last covers every field of the row; the type check fails if a new field is left out.

## 0.2.2

Who must change: nobody. What users see: turning thumbnails on or off now really keeps the row at the top of the list (0.2.1 could still jump far away). Needs player 0.6.0.

- The list keeps its top row itself when the row height changes; the sidebar no longer sets the scroll position then (that ran before the list's layout, which still used the old height).
- The scroll the list reports after the toggle is no longer taken for the user's (it may differ from the expected one by less than a row), so following the playing row does not pause for 2 seconds.

## 0.2.1

Who must change: nobody. What users see: turning thumbnails on or off keeps the row at the top of the list where it was (before, the list could jump far away); long lists react faster to clicks. Needs player 0.6.0.

- The thumbnail toggle no longer asks the list to scroll: the list keeps its top row itself when the row height changes. The folder and media kind are sent in both row styles, so the toggle changes only the pictures near the view.
- An unchanged row is the same object from one update to the next, the search numbering is rebuilt only when the order changes, and the selection-dependent command states (move, remove unselected, show in folder) are computed once per change instead of on every update.
- The list tells screen readers how many rows it has; a drag keeps its drop position right when rows are added or removed meanwhile.

## 0.2.0

Who must change: nobody. What users see: nothing new; long lists scroll and update more smoothly. Needs player 0.6.0 (row patches, and the higher row limits of large lists P3 for long lists).

- The list is a `ListView` over every shown row (large lists P3) instead of a hand-made window of 200 rows in a scroll view: only the rows in view have elements, and the plugin no longer moves a window as the list scrolls. Each row carries a stable `id`, so on player 0.6.0 a change is sent as row operations (a move, an insertion, a removal, one changed row) instead of rewriting rows; row numbers come from the row's place in the list (and are sent with the rows only while a search filters), so moving a row changes no other row.
- Pressing, dragging (with edge scrolling), releasing and double-clicking are handled once for the whole list, which maps the pointer to a row by the row height and the scroll: a row scrolled out of view during a drag no longer takes the drag with it. One insertion marker is drawn for the list. Rows tell screen readers their place and whether they are selected.
- Each row holds its thumbnail (none away from the view: an empty image, player 0.6.0), so a picture moves with its row; the separate list of pictures and the placeholder picture file are gone.
- The list is saved only when something saved changed, not checked every 200 ms while the list updates.

## 0.1.12

- The drag insertion marker, the playing row's bar, the failed mark, the thumbnail placeholder and duration badge, "No matches", the folded search icon and the search box's hint are now always present and shown or hidden, instead of being created and removed as the pointer moves, the list scrolls or a row changes. Screen readers no longer see the list's accessibility tree rebuilt during a drag. Nothing looks different.

## 0.1.11

- Each row shows a thumbnail by default, like the VestraTube list: a 96×54 picture with the duration on its bottom right, the title on up to two lines and the file's folder (a link's host) below it. Video files show a keyframe from a tenth of the way in, audio files their embedded cover art; links, audio without cover art and files the player cannot read show a placeholder.
- The new button at the right of the bottom bar turns thumbnails off (the compact single-line rows of before) and on; its icon shows the current mode, its tooltip what a click does. The choice is remembered. Add and Delete are narrower to make room.
- Thumbnails are made like the durations: only for the rows on screen and three above and below, at most two ffmpeg processes with one thread each, and never while a file opens or in its first 3 seconds. Rows scrolled away stop their work. Pictures are kept in the plugin's data folder (`thumbs/`, at most 5000, least recently used removed first), so a row shown again, also after a restart, needs no ffmpeg; a file ffmpeg cannot read is not tried again.
- Needs player 0.5.1 (pictures in a list).

## 0.1.10

- The search box's selected text uses the standard selection colours (as the std-widgets LineEdit), legible in both palettes.

## 0.1.9

Earlier versions have no changelog; see the repository history.
