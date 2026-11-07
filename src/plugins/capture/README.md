# Snapshot

Saves and copies the current frame, captures bursts, exports video clips. Everything lives in one sidebar tab, **Snapshots & Clips**, beside Playlist and Chapters.

## Features

- **Single frames**: Save Current Frame saves straight into the snapshot folder, the frame shown at the key press, named after the title and the time of that frame; Save As… lets you pick the place; Copy puts it on the clipboard.
- **Source frame and rendered frame**: the rendered frame includes video effects and, optionally, the subtitles.
- **Formats**: PNG, JPEG (adjustable quality), BMP.
- **Burst**: 2–20 frames at a fixed interval, saved into a folder of their own.
- **Clips**: part of a local video as MP4, with a choice of audio track, video effects and subtitles; the range can be typed (`1:02:03.450` or seconds), set with Now, copied from the A-B loop, or be the last 10/30/60/120 seconds (Last N s).
- **Results**: running and queued exports with Cancel, failed ones with Retry, and the last twenty finished captures (kept across restarts) with Open, Copy and Show in Folder.
- **File names**: templates with `{name}`, `{pos}`, `{start}`, `{end}`, `{date}`, `{time}`, `{n}`; the defaults are the names the plugin always used (a clip also states its end).
- The snapshot folder can be chosen, or captures go beside the video when possible. Existing files are never overwritten.

## Entries

Right-click menu: Video ▸ Snapshot.

- Save Current Frame (Ctrl+E), Copy Current Frame (Ctrl+C)
- Burst… (Ctrl+G) and Export Clip… open the tab at their place
- Snapshots & Clips… (Alt+S) shows or hides the tab

In the command palette (Ctrl+Shift+P): Save Rendered Frame (Ctrl+Alt+E), Copy Rendered Frame (Ctrl+Alt+C), the Save As… commands, Export Clip Range (Ctrl+Shift+E, the Export button), Export Last N Seconds (Ctrl+Shift+R), Set Clip Start / End, Export Tasks…, Snapshot Settings…, Snapshot Folder…, Open Snapshot Folder, Capture as PNG / JPEG / BMP, Include Subtitles in Rendered Frame.

## Limits

- Clips need a local video with a known duration; frames work wherever the player can capture one.
- Clips are re-encoded (H.264 + AAC in MP4). Fast stream-copy cuts, quality presets, a percentage while exporting, the clip's range on the seek bar and buttons in the "saved" message come with a later player (host requests R1–R6 in the design).

## Translations

Texts are English in the source; `locales/` has Simplified Chinese, German, Spanish, French, Japanese and Korean (`vestra-plugin i18n extract`, `i18n check`).
