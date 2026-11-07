# Snapshot

Saves and copies the current frame, captures bursts, exports video clips. Everything lives in one sidebar tab, **Snapshots & Clips**, beside Playlist and Chapters.

## Features

- **Single frames**: Save Current Frame saves straight into the snapshot folder, the frame shown at the key press, named after the title and the time of that frame; Save As… lets you pick the place; Copy puts it on the clipboard.
- **Source frame and rendered frame**: the rendered frame includes video effects and, optionally, the subtitles.
- **Formats**: PNG, JPEG (adjustable quality), BMP.
- **Burst**: 2–20 frames at a fixed interval, saved into a folder of their own.
- **Clips**: part of a local video. **Fast** copies the streams (lossless, about as fast as the disk, from the keyframe at or before the start, shown in the tab); **Precise** re-encodes from the exact frame as MP4 with a preset (Same as source, High quality, Small file) or Advanced settings, video effects and subtitles. A progress bar with the time left, the clip's range on the seek bar while the tab shows, and Show in Folder / Copy / Show in Tab in the "saved" message. The range can be typed (`1:02:03.450` or seconds), set with Now, copied from the A-B loop, or be the last 10/30/60/120 seconds (Last N s).
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
- A Fast cut starts on a keyframe (the tab shows which) and copies the video and one audio track, no subtitles; files whose codecs cannot be copied need Precise. Precise clips are H.264 + AAC in MP4.
- Use A-B range needs the A-B loop switched on.

## Translations

Texts are English in the source; `locales/` has Simplified Chinese, German, Spanish, French, Japanese and Korean (`vestra-plugin i18n extract`, `i18n check`).
