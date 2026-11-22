# Changelog

## 0.1.9

- Events instead of `subscribe()` and `player.onChange` (removed in SDK 1.11.0): a new source (`source.opening`) aborts what ran for the old one and clears the results, and the automatic search runs at `source.ready`. The download window streams the state only while it is open. The plugin holds nothing otherwise and may be stopped while idle; a video that opens wakes it and is searched as when it was running. One edge: a source still loading when the plugin starts is now searched once it is ready (only the source already ready at start is left alone), since a plugin woken by an opening source starts the same way. Menus, windows and texts are unchanged. Who must change: nobody.

## 0.1.8

- The description's last line, starting with `→ `, says where to open the plugin; Store 0.3.1 shows it on the plugin's card, also before install. Description text and its translations only. Who must change: nobody.

## 0.1.7

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.1.6

- Menu sweep (menu redesign §4): Search and Download Subtitles… moves from Subtitles ▸ Online Subtitles ▸ to Open ▸ (order 115, group `open`: after Load External Audio…, before the Recently Played separator); Online Subtitle Settings… leaves the menu and is in the command palette only ("Online Subtitles: Online Subtitle Settings…"). Ids unchanged, still no default keys. Who must change: nobody.

- Both commands have a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Online Subtitles** (Search and Download Subtitles…, Online Subtitle Settings…), the submenu's name with its translations. No `home`: each command opens its own window. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.
