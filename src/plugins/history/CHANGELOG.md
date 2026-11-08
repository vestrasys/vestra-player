# Changelog

## 0.1.8

- Menu sweep (menu redesign §4): Playback ▸ Resume Playback ▸ (all five items) leaves the right-click menu and is in the command palette only ("Resume Playback: …"), with ids, radio and checked states unchanged; Open ▸ Recently Played ▸ stays and is now the last line of Open ▸ (order 119, after a separator; it sat between Load External Audio… and Add Secondary Subtitle… at 114). Who must change: nobody.
- The description places Resume Playback in the command palette, in all six languages.

- Every command has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Recently Played** (the records, "(No Entries)", Remove Invalid Entries, Clear Entries, Record Playback History) and **Resume Playback** (Don't Resume, Resume Automatically, Ask Every Time, Also Remember Audio Files, Clear All Saved Positions). Both are the submenus' own labels, so they read the same as the menu in every language of the plugin. No command names a `home`: the plugin has no window that holds these controls.
- Remove Invalid Entries and Clear Entries, disabled while there are no records, now say why ("No recently played items"), so the palette still lists them instead of hiding them.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Tests: every command has a non-empty category, equal to its submenu's label as shown, empty and with records, in English and Chinese; the two reasons are shown while disabled and gone once there are records.

## 0.1.7

- Code only: the resume window handle is named `#resumeWindow` (was `#window`). Who must change: nobody. No behaviour change.

## 0.1.6

- The plugin no longer fails to start ("invalid command: path segment requires 1..=80 bytes, no controls or surrounding whitespace") when a recently played file has a long non-Latin name, so Open ▸ Recently Played and the resume points come back for those users. The player limits a menu line to 80 UTF-8 bytes, while the plugin shortened a name to 80 characters: a Chinese or Japanese name (3 bytes a character) with the point in brackets went far past it, and the refusal of that one line failed the whole activation. Each line is now shortened in the middle by bytes, leaving room for the number, the point, the longest translation of its brackets (`{}（{}）`, 6 bytes) and a duplicate's ` (n)`, which is now given whenever a line would read like another line or a fixed item of the submenu as the player keys them (in English, with the point: `X (1:02:03)` and `X` at 1:02:03; a file named `Clear Entries`); only whole characters are cut (no half CJK character, split emoji or detached accent). Controls in a name become spaces and surrounding whitespace is trimmed, both of which the player also refuses; a name of whitespace only shows its path. A line that still would not pass is left out and logged instead of failing the plugin.
- Tests: long Chinese names with a point in Chinese, an emoji name, edge spaces, controls and ten duplicates past the tenth record, in Chinese and English, each line checked against the player's rule as declared and as shown (the SDK's test host does not check the 80 bytes); every locale's `{} ({})` fits the reserved room.
- Nobody needs to change anything.

## 0.1.5

- Stopping the plugin (deactivation, quitting the player) no longer fails with "command recent.item.0 is already declared" when a rebuild of Open ▸ Recently Played was due at that moment (a record or a resume point had changed within the last 0.5 s). The rebuild ran while the old submenu was still being removed and declared its commands again; the submenu now lives in a scope of its own that is released after the pending rebuild is cancelled, and a rebuild that was already under way stops there. The last position is still written on stop as before.
- Tests: the store files are read once written (polled) instead of right after a fixed wait, so a loaded machine does not read them before the write ends.
