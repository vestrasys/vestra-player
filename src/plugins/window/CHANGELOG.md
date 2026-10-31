# Changelog

## 0.1.5

- Options ▸ Playback ▸ Window uses the Vestra widget library (`@vestra/widgets.slint`): the two rows of radios (window size when playback starts, always on top) are the library's `RadioGroup`, and the two checks (only once, hide the pointer while playing) are the library's `Check`. Who must change: nobody. What users see: the checks are the 13px Vestra box instead of the 18px Fluent one, and a checked radio has the accent ring (ui-spec ctx-radio) instead of the dim one. Each row of radios is one Tab stop and the arrow keys move the choice; Space flips a focused check. A click no longer takes the keyboard focus, so the player's shortcuts keep working. A long translation of a check's label wraps instead of being cut at one row. The same preferences, defaults and menu items as before.
- The local `Radio` and `Choices` components and the std-widgets `CheckBox` are gone.
- Requires player 0.6.0 (`minPlayerVersion`), which ships the library's form controls.
