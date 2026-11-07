# Changelog

## 1.0.10

- Command palette (player 0.7.6, SDK 1.6.4): every command declares a `category`, so the palette (Ctrl+Shift+P) names it on its own once the menus change. "Snapshot" (the submenu's own text and translations) for the six Save / Save As / Copy frame commands, Burst…, Capture as PNG / JPEG / BMP and Include Subtitles in Rendered Frame; "Video" (translated in every locale) for Export Clip…, Export Tasks…, Snapshot Settings…, Snapshot Folder… and Open Snapshot Folder, whose titles already name their subject ("Snapshot: Snapshot Settings…" would say it twice, and a clip is no snapshot). Capture as PNG / JPEG / BMP, Include Subtitles in Rendered Frame and Snapshot Folder… set `home` to Snapshot Settings…, the window that holds their controls. Who must change: nobody. What users see: palette rows such as "Snapshot: Save Current Frame" and "Video: Export Clip…", the options with "Snapshot Settings…" as their place. Titles, ids, menu paths, keys and order are unchanged; the frame commands keep their "No frame to capture" reason, so they stay listed (greyed) while disabled.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse `category` and `home`.

## 1.0.9

- Code only: the window parameter of the open/bind/release helpers is named `toolWindow` (was `window`). Who must change: nobody. No behaviour change.

## 1.0.8

- Export Tasks: the list no longer lags while a video plays. With SDK 1.5.45's precise invalidation, state changed outside SDK-invoked code is drawn only at the next update (up to about a second later while playing), and three task-row changes happened after an `await`: the history trim (beyond 20 tasks, each further old task is removed after the previous one's snapshot is released), a task's first job state once the export call returns (`Preparing` becoming `Exporting`), and a burst frame's output once its job finishes (`Exporting · 3/10` counting up). All three now call `plugin.update()`. Who must change: nobody. What users see: the rows change at once.

## 1.0.7

- The Burst, Export Clip and Snapshot Settings dialogs use the player library's `Choice` and `Check` (`@vestra/widgets.slint`) instead of std-widgets' `ComboBox` (3) and `CheckBox` (7), so every drop-down and check box in the player has one look. What users see: Vestra-style drop-downs (21px, the list below with at most 10 rows, arrow keys / Home / End change the choice, Space / Enter / F4 open it) and check boxes (13px accent box, a label that wraps instead of widening the dialog); a control reached with Tab draws a 1px accent focus ring, and clicking one no longer takes the keyboard focus from the player. The Format choice reports a pick only when it changes. Who must change: nobody; the dialogs' properties and callbacks (`format-index`, `audio-index`, `choose-format(int)`, `set-beside-media(bool)`, `set-screen-subtitles(bool)`) are unchanged.
- Requires player 0.6.0 (`minPlayerVersion`), whose widget library ships `Choice` and `Check`.

## 1.0.6

- Export Tasks: the task list is Slint's `ListView` (std-widgets) instead of a `ScrollView` holding a `VerticalLayout` with a `for` over the rows, so only the rows in view are instantiated, as in the other lists of the player. Who must change: nobody. What users see: nothing new; the rows (two lines, Cancel / Open / Retry, 40 px each), the empty-list note and the scrolling are the same.
- The dialogs keep the std `ComboBox` and `CheckBox`: moving them to the library `Choice` / `Check` is optional in the widgets migration and would change their look.

## 1.0.5

- Lazy activation (player 0.5.21, SDK 1.5.40): the plugin no longer keeps running while idle. Who must change: nobody. What users see: nothing new; the Snapshot items are greyed with "No frame to capture" (translated as before) until a frame can be captured, and the Burst and Export Clip dialogs show what they did. Under `VESTRA_LAZY=1` the player may now stop the plugin while nothing uses it and start it again on the next capture.
- Save / Save As / Copy Current Frame and Save / Save As / Copy Rendered Frame (and Ctrl+E, Ctrl+C, Ctrl+Alt+E, Ctrl+Alt+C) are enabled by a condition the player evaluates, `when.can("snapshot").and(when.phase("ready", "ended"), when.stream.not())`, the same rule as `usable` in model.ts, with the fixed reason "No frame to capture" the player shows only while the condition is false. Before, their `enabled` and `reason` were functions of `plugin.player.state`, which kept the plugin listening to every playback update.
- The Burst and Export Clip dialogs read the player's state (the folder shown, whether a frame can be captured or exported, the audio tracks) only while open: those values are bound in a child scope made when the dialog opens and disposed when the user closes it (or when it fails to open), and read afresh at the next opening. Before, the closed dialogs' bindings read `plugin.player.state` from activation on. Their other values (busy, progress, status, format) stay bound as before.
- Requires player 0.5.21 (`minPlayerVersion`), whose menus evaluate `plugin.when` conditions.

## 1.0.4

- Save Current Frame, Save Rendered Frame, Copy Current Frame, Copy Rendered Frame and a "current frame" Burst capture the frame shown when the key (or Start) was pressed, like mpv's `screenshot` and PotPlayer. Before, the player froze its current frame only after the snapshot folder was checked and the state read again, up to about 100 ms (several frames) later while playing. The position read at the press is now sent as the capture time, as Save As already did; this costs nothing, since every snapshot is decoded by the export worker anyway. Paused, the frame is the same as before.
- The saved file is named by the time the player reports for the captured frame, which is also the frame the export writes, like mpv's `screenshot-template` `%P`. Before, Save Current / Rendered Frame named the file by the position read at the press while the picture was the later frame (`movie_00.04.400.png` holding the frame at 00:00:04.467). Bursts name each file by its own sample time, as before; Copy has no name; Export Clip uses the times in its dialog.
