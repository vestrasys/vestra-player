# Changelog

## 0.3.1

- Lazy activation (player 0.5.21, SDK 1.5.40). Who must change: nobody. What users see: the check marks under Settings ▸ Appearance and Settings ▸ Language are right at once when the palette or the language is switched elsewhere (the options window, another plugin), without the plugin starting. They are conditions the player evaluates, `when.theme(id)`, `when.language("system")` and `when.language(id)`, instead of functions of `plugin.appearance`; with the functions a stopped plugin had to be started to re-send them.
- Requires player 0.5.21 (`minPlayerVersion`).
