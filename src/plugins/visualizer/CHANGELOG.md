# Changelog

## 0.1.12

- SDK 1.11 (events and triggers, Release B): `player.subscribe()` and `player.onChange()` are gone. The scene follows `source.opening`, `source.ready` and `source.ended` instead of diffing the state stream, a source that closes after it ended is told by a trigger on `when.open` (its `leave`), and the backdrop coming back is told by `layout.changed` instead of `panels.backdrop.onChange` (which held `window.state`). No stream: the backdrop shows nothing that follows the position. Behaviour is unchanged; the mounted backdrop panel still keeps the plugin running. Who must change: nobody.

## 0.1.11

- The description's last line, starting with `→ `, says where to open the plugin; Store 0.3.1 shows it on the plugin's card, also before install. Description text and its translations only. Who must change: nobody.

## 0.1.10

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.1.9

- Menu sweep (menu redesign §4): Audio ▸ Show Album Cover for Audio Only, Settings ▸ Appearance ▸ Show Logo When Idle / Choose Logo Image… and Reset ▸ Idle Logo leave the right-click menu and are in the command palette only ("Audio: …", "Appearance: …", "Reset: Idle Logo"), with ids and checked states unchanged; the settings page is unchanged. Who must change: nobody.
- The description names the Logo and Album Cover page and the command palette entries instead of the Audio ▸ and Settings ▸ paths, in all six languages.

- Every command has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Audio** (Show Album Cover for Audio Only), **Appearance** (Show Logo When Idle, Choose Logo Image…) and **Reset** (Idle Logo, so the palette reads "Reset: Idle Logo"). "Appearance" uses the submenu's translations; "Audio" and "Reset" are new in this catalog, translated like the player's Audio and Reset menus (machine, marked `x-machine`). No command names a `home`: no command opens the Logo and Album Cover settings page. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.
- **Reset: Idle Logo** (`reset-logo`) is declared `destructive` (SDK 1.7.9): it deletes the copied logo file from the plugin's data folder (the original may be gone) and clears the setting, so when another built-in plugin runs it through `commands.run` (Ask Vestra), the player asks the user first. The other commands are not: Show Album Cover for Audio Only and Show Logo When Idle toggle one setting, and Choose Logo Image… opens a file dialog the user completes. The settings page's Clear button (cover cache, re-readable) is not a command. The palette runs Reset: Idle Logo as before. Who must change: nobody.
- Tests: `reset-logo` carries `destructive: true`, with the default and a custom logo, and no other command does.
