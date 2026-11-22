# Changelog

## 0.3.7

- **Events and triggers (SDK 1.11, release B step B3)**: `plugin.player.subscribe()` is gone. The Appearance and Language pages follow a switch made elsewhere through a listener of `appearance.changed` (a wake event: it keeps nothing running), which keeps `plugin.appearance` current as the subscription did; the check marks stay player conditions. Behaviour is unchanged. Who must change: nobody. Tests: the pages follow a palette and language set elsewhere.

## 0.3.6

- The description's last line, starting with `→ `, says where to open the plugin; Store 0.3.1 shows it on the plugin's card, also before install. Description text and its translations only. Who must change: nobody.

## 0.3.5

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`), the call that replaces the one declaration that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.3.4

- Menu sweep (menu redesign §4): Options… (F5) moves from Settings ▸ to the top level (order 830, group `settings-main`, with About Vestra Player…); Settings ▸ Appearance ▸ and Settings ▸ Language ▸ leave the right-click menu and are in the command palette only ("Appearance: Vestra Dark", "Language: Follow System", …), with ids, radio and checked states unchanged. Who must change: nobody.
- The description names Options… (F5), its Appearance and Language pages and the command palette instead of the Settings ▸ paths, in all six languages.

- Translations only: the category text "Settings" added in 0.3.3 is now marked `x-machine` in de, es, fr, ja and ko. Who must change: nobody. No text or behaviour change.

## 0.3.3

- Command palette categories (player 0.7.6, SDK 1.6.4). Who must change: nobody. What users see: in the command palette (Ctrl+Shift+P) the commands read "Settings: Options…", "Appearance: Vestra Dark" and "Language: 日本語" instead of falling back to the menu path, translated like the menus. The palette and language radios name Options… as their home, since the options window has the Appearance and Language pages with the same choices. Menus, titles, ids and keys are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`).

## 0.3.2

- Library radio mark (player 0.6.0). Who must change: nobody. What users see: the radios on Options ▸ Appearance and Options ▸ General ▸ Language are the player's shared radio mark (`RadioMark` from `@vestra/widgets.slint`, ui-spec ctx-radio): 12px instead of 14px, and an unchecked ring turns to the accent colour while its row is hovered. The rows (previews, 24px language rows, hover plate) and what a click does are unchanged; the plugin's own `Radio` component is gone. Each row now tells screen readers it is a radio button, whether it is checked, and its name.
- Requires player 0.6.0 (`minPlayerVersion`).

## 0.3.1

- Lazy activation (player 0.5.21, SDK 1.5.40). Who must change: nobody. What users see: the check marks under Settings ▸ Appearance and Settings ▸ Language are right at once when the palette or the language is switched elsewhere (the options window, another plugin), without the plugin starting. They are conditions the player evaluates, `when.theme(id)`, `when.language("system")` and `when.language(id)`, instead of functions of `plugin.appearance`; with the functions a stopped plugin had to be started to re-send them.
- Requires player 0.5.21 (`minPlayerVersion`).
