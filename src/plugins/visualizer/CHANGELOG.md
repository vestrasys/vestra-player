# Changelog

## 0.1.9

- Menu sweep (menu redesign §4): Audio ▸ Show Album Cover for Audio Only, Settings ▸ Appearance ▸ Show Logo When Idle / Choose Logo Image… and Reset ▸ Idle Logo leave the right-click menu and are in the command palette only ("Audio: …", "Appearance: …", "Reset: Idle Logo"), with ids and checked states unchanged; the settings page is unchanged. Who must change: nobody.
- The description names the Logo and Album Cover page and the command palette entries instead of the Audio ▸ and Settings ▸ paths, in all six languages.

- Every command has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Audio** (Show Album Cover for Audio Only), **Appearance** (Show Logo When Idle, Choose Logo Image…) and **Reset** (Idle Logo, so the palette reads "Reset: Idle Logo"). "Appearance" uses the submenu's translations; "Audio" and "Reset" are new in this catalog, translated like the player's Audio and Reset menus (machine, marked `x-machine`). No command names a `home`: no command opens the Logo and Album Cover settings page. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.
