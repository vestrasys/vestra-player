# Snapshot

Saves and copies the current frame, captures bursts, exports video clips.

## Features

- **Single frames**: Save Current Frame saves straight into the snapshot folder, the frame shown at the key press, named after the title and the time of that frame; Save As… lets you pick the place; Copy puts it on the clipboard.
- **Source frame and rendered frame**: the rendered frame includes video effects and, optionally, the subtitles.
- **Formats**: PNG, JPEG (adjustable quality), BMP.
- **Burst**: 2–20 frames at a fixed interval, saved into a folder of their own.
- **Export Clip**: part of a video as MP4, with a choice of audio track, video effects and subtitles.
- **Export Tasks**: view, cancel and retry exports, open their output.
- The snapshot folder can be chosen, or captures go beside the video when possible. Existing files are never overwritten.

## Entries

Right-click menu: Video ▸ Snapshot.

- Save Current Frame (Ctrl+E), Copy Current Frame (Ctrl+C)
- Save Rendered Frame (Ctrl+Alt+E), Copy Rendered Frame (Ctrl+Alt+C)
- Burst… (Ctrl+G), Export Clip…, Export Tasks…
- Snapshot Settings…, Snapshot Folder…, Open Snapshot Folder

## Limits

- Local videos with a known duration only.
- Exports show no percentage, only the stage and the number of frames saved.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
