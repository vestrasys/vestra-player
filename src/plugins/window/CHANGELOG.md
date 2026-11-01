# Changelog

## 0.1.6

- Window size when playback starts (G-02) leaves a maximized window alone, as MPC-HC's auto-zoom and mpv's autofit do: a new file no longer takes the window out of maximized and resizes it to the video. It reads `WindowState.maximized` (SDK 1.5.52, player 0.6.3). Who must change: nobody. What users see: with player 0.6.3 and later a maximized window stays maximized when a file starts; with older players, which do not report `maximized`, nothing changes (it reads `false`), so `minPlayerVersion` stays 0.6.0. The size commands (Screen ▸ 0.5×–2×, fit) still resize a maximized window: on Windows the player restores it first.
- The settings text under Options ▸ Playback ▸ Window says so ("Not in fullscreen, maximized, mini size or picture-in-picture"); the new wording is a machine translation in every language, marked `x-machine` (also in zh-Hans).

## 0.1.5

- Options ▸ Playback ▸ Window uses the Vestra widget library (`@vestra/widgets.slint`): the two rows of radios (window size when playback starts, always on top) are the library's `RadioGroup`, and the two checks (only once, hide the pointer while playing) are the library's `Check`. Who must change: nobody. What users see: the checks are the 13px Vestra box instead of the 18px Fluent one, and a checked radio has the accent ring (ui-spec ctx-radio) instead of the dim one. Each row of radios is one Tab stop and the arrow keys move the choice; Space flips a focused check. A click no longer takes the keyboard focus, so the player's shortcuts keep working. A long translation of a check's label wraps instead of being cut at one row. The same preferences, defaults and menu items as before.
- The local `Radio` and `Choices` components and the std-widgets `CheckBox` are gone.
- Requires player 0.6.0 (`minPlayerVersion`), which ships the library's form controls.
