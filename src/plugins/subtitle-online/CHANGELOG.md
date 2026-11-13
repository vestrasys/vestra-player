# Changelog

## 0.1.7

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.1.6

- Menu sweep (menu redesign §4): Search and Download Subtitles… moves from Subtitles ▸ Online Subtitles ▸ to Open ▸ (order 115, group `open`: after Load External Audio…, before the Recently Played separator); Online Subtitle Settings… leaves the menu and is in the command palette only ("Online Subtitles: Online Subtitle Settings…"). Ids unchanged, still no default keys. Who must change: nobody.

- Both commands have a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Online Subtitles** (Search and Download Subtitles…, Online Subtitle Settings…), the submenu's name with its translations. No `home`: each command opens its own window. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.
