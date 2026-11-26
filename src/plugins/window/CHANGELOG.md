# Changelog

## 0.1.10

- Events and triggers (SDK 1.11, step B3): `plugin.player.subscribe()` and `player.onChange()` are gone, and so are the `window.onChange` watchers. Always on top is one trigger the player keeps (`when.playing`, `when.playing.and(when.fullscreen)`, or a condition that always holds; none for Never), and what was sent last is kept in session storage; whether a source shows is a trigger on `when.phase("ready", "ended")`; a new source, its end and its video size come from `source.opening`, `source.ready`, `source.ended` and `video.reconfigured` (with `player.stats()` kept as the fallback and before every resize); the window from `window.changed`. The last source seen is kept in session storage, so a plugin started again after standby does not resize the video that is already playing. Nothing it holds keeps it running any more, so the player may put it in standby while idle. Same commands, keys, settings page and OSD texts. Who must change: nobody.

## 0.1.9

- Always on top is "Never" by default (IA-WIN-03, user ruling 2026-11-15; mpv's `ontop=no`, VLC's "Never"), replacing "While Playing in Fullscreen" (PotPlayer's `OnTopMode=3`). Held on top in fullscreen, the player kept a plugin's Save or Open dialog and other applications behind it. The other modes, Ctrl+T's cycle and the OSD texts are unchanged. Who must change: nobody. What users see: a user who never chose a mode no longer floats in fullscreen; a chosen mode is kept. `settings.json` now records `onTopChosen` when the mode is chosen (menu, Ctrl+T, palette, settings page). A file written by 0.1.8 or older holds the whole settings object whenever any setting changed, so there "While Playing in Fullscreen" (the old default) is taken as never chosen and becomes "Never", and any other mode is kept as chosen; whoever wants the old behaviour picks "While Playing in Fullscreen" again once.

## 0.1.8

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.1.7

- Menu sweep (menu redesign §4): the whole Screen ▸ submenu (Picture-in-Picture, Mini Size, 0.5×–2×, Fit to Video Aspect Ratio, When Playback Starts ▸, Always on Top ▸) and Playback ▸ Hide Pointer While Playing leave the right-click menu and are in the command palette only, with their ids, keys, radio and checked states and the categories above; the settings page is unchanged. Who must change: nobody.

- Every command has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Window** (Mini Size, Picture-in-Picture, Hide Pointer While Playing), **Window Size** (0.5×–2×, Fit to Video Aspect Ratio), **Size on Open** (Screen ▸ When Playback Starts: Don't Resize, 0.5×–2×, Fit to Video Aspect Ratio, Only Once; "When Playback Starts: 1×" would not say that it is the window's size) and **Always on Top** (Cycle Mode and the four modes). "Window" and "Always on Top" use the settings page's and the submenu's translations; "Window Size" and "Size on Open" are new, machine translations in every language marked `x-machine` (also in zh-Hans), worded like the settings page's "Window size when playback starts". No `home`: the settings page (Options ▸ Playback ▸ Window) has no command that opens it. Every command that can be disabled already gives a reason. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.

## 0.1.6

- Window size when playback starts (G-02) leaves a maximized window alone, as MPC-HC's auto-zoom and mpv's autofit do: a new file no longer takes the window out of maximized and resizes it to the video. It reads `WindowState.maximized` (SDK 1.5.52, player 0.6.3). Who must change: nobody. What users see: with player 0.6.3 and later a maximized window stays maximized when a file starts; with older players, which do not report `maximized`, nothing changes (it reads `false`), so `minPlayerVersion` stays 0.6.0. The size commands (Screen ▸ 0.5×–2×, fit) still resize a maximized window: on Windows the player restores it first.
- The settings text under Options ▸ Playback ▸ Window says so ("Not in fullscreen, maximized, mini size or picture-in-picture"); the new wording is a machine translation in every language, marked `x-machine` (also in zh-Hans).

## 0.1.5

- Options ▸ Playback ▸ Window uses the Vestra widget library (`@vestra/widgets.slint`): the two rows of radios (window size when playback starts, always on top) are the library's `RadioGroup`, and the two checks (only once, hide the pointer while playing) are the library's `Check`. Who must change: nobody. What users see: the checks are the 13px Vestra box instead of the 18px Fluent one, and a checked radio has the accent ring (ctx-radio) instead of the dim one. Each row of radios is one Tab stop and the arrow keys move the choice; Space flips a focused check. A click no longer takes the keyboard focus, so the player's shortcuts keep working. A long translation of a check's label wraps instead of being cut at one row. The same preferences, defaults and menu items as before.
- The local `Radio` and `Choices` components and the std-widgets `CheckBox` are gone.
- Requires player 0.6.0 (`minPlayerVersion`), which ships the library's form controls.
