# Changelog

## 0.1.12

- Menu sweep (menu redesign §4): Settings ▸ Keyboard Shortcuts…, Mouse… and Ignore Default Shortcuts leave the right-click menu and are in the command palette only ("Settings: …"), with ids, orders and the checked state unchanged; the options window's Keys and Mouse pages are unchanged. Who must change: nobody.
- Texts follow the sweep: the Ignore Default Shortcuts question and the ignored row's tooltip point at the switch on the Keyboard Shortcuts page and the command palette; the description names the options window and the palette as the entry points. Translated in all six languages.

- Keyboard Shortcuts: the search box matches a command's menu path and category again, not only its shown name, so "Video" finds Video ▸ Snapshot ▸ Save Current Frame although the row reads "Snapshot: Save Current Frame". The display is unchanged.
- Keyboard Shortcuts: a command that works only while its plugin's panel has focus (`focus: "panel"`) says so in its row, "{} (in panel)" as the edit area below already does, so a panel command and the menu item it mirrors no longer show as an identical pair ("Playlist: Add Files…" and "Playlist: Add Files… (in panel)"). New text, translated in all six languages and marked `x-machine`. Export List… is unchanged: it has its own Scope column. Who must change: nobody.

## 0.1.11

- Keyboard Shortcuts and Mouse name a command like the player's command palette: **"Category: Title"** when it has a category (`BindingInfo.category`, player 0.7.6, SDK 1.6.4), so titles such as "2×" or "Off" stay clear once menu paths go away; a command without one is still named by its menu path (`Playback ▸ Seek ▸ Forward 5 s`), or its title outside the menu. The separator follows the interface language as the palette writes it (`速度：2×`, `Vitesse : 2×`); new text "{0}: {1}", translated in all six languages. The same name is used in the list, its search, sorting, the conflict and "already used by" messages, the OSD after a change and Export List…. Who must change: nobody.
- Every command of the plugin has a category for the command palette: **Settings** (Keyboard Shortcuts…, Mouse…, Ignore Default Shortcuts), translated like the player's Settings menu (machine, marked `x-machine`). Ignore Default Shortcuts names Keyboard Shortcuts… as its place (`home`): that page has the same switch. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.

## 0.1.10

- Ignore Default Shortcuts keeps a way out of fullscreen: Esc now belongs to Toggle Fullscreen (player 0.6.0), so Toggle Fullscreen is never ignored, like Quit and Leave Fullscreen. Enter Fullscreen has no default and is not exempt. The confirmation now names Toggle Fullscreen (Esc) instead of Leave Fullscreen; translations updated.
- A user who turned Ignore Default Shortcuts on with 0.1.9 or earlier had Toggle Fullscreen switched off too; at start (or the next change) its empty override is reset, so Esc works again, and it is no longer shown as Ignored. Other ignored commands are unchanged.
- README: the side buttons work anywhere in the window; Back (X1) is the next item, Forward (X2) the previous one (player 0.6.0).

## 0.1.9

- Keyboard Shortcuts…: the “Ignore default shortcuts” check box no longer shows checked while the confirmation is open; it is checked only after you confirm. If the question cannot be shown, the box returns to the real setting instead of staying checked.

## 0.1.8

- The Keyboard Shortcuts page's two filter drop-downs, its “Ignore default shortcuts” check box and the Mouse page's modifier radios are the player's shared controls (`@vestra/widgets.slint` Choice, Check, RadioGroup) instead of the plugin's own copies, so they look and behave like the same controls elsewhere in the player: the drop-downs and the check box can be reached with Tab (a focused control shows an accent ring), a drop-down opens with Space, Enter, F4 or Alt+↓ and its list follows the arrow keys, the modifier radios are one Tab stop that the arrow keys move through, and a chosen radio has the accent ring. Clicking any of them still leaves the keyboard to the player's shortcuts. Requires player 0.6.0.
- The source filter's words (All sources, Player, Plugins not loaded) and the “None” modifier are now translated by the plugin; the translations are unchanged.

## 0.1.7

- Settings ▸ Mouse… offers the `Cmd` modifier on macOS only (⌘). On Windows `Cmd` is the Windows logo key and on Linux the Super key: the system takes most of their combinations and the player's menus never show such bindings, so the modifier row there is None / Ctrl / Alt / Shift. A `Cmd` mouse binding that is already in the binding table (written on a Mac, or by an older version) is kept: while one exists the `Cmd` radio is shown, so the binding can be seen and changed; the radio goes once no such binding is left and another modifier is chosen.
- Keyboard Shortcuts…: on Windows and Linux a key pressed with the Windows (Super) key is not recorded (the capture keeps waiting) instead of becoming a `Ctrl` binding. On macOS ⌘ and ⌃ still record `Ctrl`, which answers to both.
- The system is read once at start (`plugin.app.system()`); if it cannot be read, keys behave as on Windows.
