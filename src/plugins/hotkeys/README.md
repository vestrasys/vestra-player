# Shortcut Settings

View and change the keyboard shortcuts and mouse actions of the player and of every plugin.

## Features

- **Keyboard Shortcuts**: lists every command with its keys; search by command text or key, filter by status (changed, conflicts, unbound) and source; add and remove keys of a command, restore its defaults or unbind it. A key already used by another command is reported and can be moved. The list can be exported as CSV or text.
- **Mouse**: assigns commands to left click, double-click, middle click, the side buttons and the wheel on the video (optionally with a modifier). The player's defaults (0.5.2): a left click plays or pauses, nothing is bound to a double click (bind Toggle Fullscreen to it to restore the old default), the side buttons go to the previous and next item, the wheel changes the volume. A left click waits for the double-click interval only while a double-click is bound and the click's command is not an on/off toggle.
- **Ignore Default Shortcuts**: switches off every default key at once (except Quit and Leave Fullscreen); turning it off restores them.
- Changes apply at once and are kept across restarts.

## Entry points

- Context menu Settings ▸ Keyboard Shortcuts… / Mouse…, or the Keys and Mouse category of the options window.
- Context menu Settings ▸ Ignore Default Shortcuts.

## Languages

English source texts; Simplified Chinese in `locales/zh-Hans.po`. The exported list is in the interface language too.

## Limitations

- Numeric keypad keys are recorded as the main keyboard's keys; ⌃P–⌃X cannot be recorded, use ⌘ instead (a `Ctrl+` binding answers to both ⌘ and ⌃).
- Keys the system takes (such as ⌘Tab, ⌘Space) cannot be recorded.
