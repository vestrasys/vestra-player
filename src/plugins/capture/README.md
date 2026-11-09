# Snapshot

Saves and copies the current frame and captures bursts. It has no panel: the right-click menu's Snapshot ▸, the command palette and the keys run it, and its settings are a page of the Options window, **Options ▸ Video ▸ Snapshot** (Snapshot ▸ Snapshot Settings…). Exporting video clips is a store plugin's job.

## Features

- **Save and Copy**, as mpv's `screenshot`: the frame shown at the key press, at the source's size, with the subtitles shown (subtitles the player cannot draw into a frame are left out, and the message says so), and with the video effects when the settings say so. Save goes straight into the snapshot folder, named after the title and the time of that frame; Copy puts it on the clipboard (always PNG).
- **Rendered frame**: the frame with the video effects and, unless switched off, the subtitles. **Video Frame Only**: the source frame alone.
- **Burst** (Ctrl+G): 2–20 frames at a fixed interval from the frame shown at the press, into a folder of their own.
- The "saved" message has buttons: Show in Folder, and Copy for a frame. Existing files are never overwritten.

## Settings (Options ▸ Video ▸ Snapshot)

The snapshot folder (or beside the video when possible), the image format (PNG, JPEG with its quality, BMP), whether Save and Copy Current Frame include the video effects, the burst's count, interval, effects and subtitles, and the file-name templates for frames (`{name}_{pos}`) and burst folders (`Capture-{date} {time}`), with `{name}`, `{pos}`, `{start}`, `{end}`, `{date}`, `{time}` and `{n}`.

## Entries

Right-click menu: Snapshot ▸ (a top-level submenu after Audio Track and Subtitle Track).

- Save Current Frame (Ctrl+E), Copy Current Frame (Ctrl+C), Burst (Ctrl+G)
- Snapshot Settings… opens the Options window at the Snapshot page

In the command palette (Ctrl+Shift+P): Save Rendered Frame (Ctrl+Alt+E), Copy Rendered Frame (Ctrl+Alt+C), Save / Copy Video Frame Only, the Save As… commands, Snapshot Folder…, Open Snapshot Folder, Show Last Capture in Folder, Copy Last Snapshot, Capture as PNG / JPEG / BMP, Include Subtitles in Rendered Frame.

## Limits

- Frames work wherever the player can capture one; a burst needs a known media duration.
- While subtitles are shown, Save and Copy Current Frame also include the video effects of plugins that have any: the player draws subtitles only into the frame with effects.
- Show Last Capture in Folder and Copy Last Snapshot act on the newest capture of this session.

## Translations

Texts are English in the source; `locales/` has Simplified Chinese, German, Spanish, French, Japanese and Korean (`vestra-plugin i18n extract`, `i18n check`).
