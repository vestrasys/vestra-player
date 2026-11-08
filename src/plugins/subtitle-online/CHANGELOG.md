# Changelog

## 0.1.6

- Menu sweep (menu redesign §4): Search and Download Subtitles… moves from Subtitles ▸ Online Subtitles ▸ to Open ▸ (order 115, group `open`: after Load External Audio…, before the Recently Played separator); Online Subtitle Settings… leaves the menu and is in the command palette only ("Online Subtitles: Online Subtitle Settings…"). Ids unchanged, still no default keys. Who must change: nobody.

- Both commands have a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Online Subtitles** (Search and Download Subtitles…, Online Subtitle Settings…), the submenu's name with its translations. No `home`: each command opens its own window. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.
