# Changelog

## 0.3.2

- Library radio mark (player 0.6.0). Who must change: nobody. What users see: the radios on Options ▸ Appearance and Options ▸ General ▸ Language are the player's shared radio mark (`RadioMark` from `@vestra/widgets.slint`, ui-spec ctx-radio): 12px instead of 14px, and an unchecked ring turns to the accent colour while its row is hovered. The rows (previews, 24px language rows, hover plate) and what a click does are unchanged; the plugin's own `Radio` component is gone. Each row now tells screen readers it is a radio button, whether it is checked, and its name.
- Requires player 0.6.0 (`minPlayerVersion`).

## 0.3.1

- Lazy activation (player 0.5.21, SDK 1.5.40). Who must change: nobody. What users see: the check marks under Settings ▸ Appearance and Settings ▸ Language are right at once when the palette or the language is switched elsewhere (the options window, another plugin), without the plugin starting. They are conditions the player evaluates, `when.theme(id)`, `when.language("system")` and `when.language(id)`, instead of functions of `plugin.appearance`; with the functions a stopped plugin had to be started to re-send them.
- Requires player 0.5.21 (`minPlayerVersion`).
