# Snapshot

Saves and copies the current frame, captures bursts, exports video clips. Everything lives in one sidebar tab, **Capture**, beside Playlist and Chapters, with four pages: Snapshot, Clip, Results and Settings. The tab remembers the page shown last.

## Features

- **Snapshot page**: Save and Copy, as mpv's `screenshot`: the frame shown at the key press, at the source's size, with the subtitles shown (subtitles the player cannot draw into a frame are left out), and with the video effects when Settings says so. Save goes straight into the snapshot folder, named after the title and the time of that frame; Copy puts it on the clipboard.
- **Clip page**: **Fast** copies the streams (lossless, about as fast as the disk, from the keyframe at or before the start: the small timeline marks it with its time) or **Precise** re-encodes from the exact frame; the ⓘ beside them explains both. Start and End (typed as `1:02:03.450` or seconds, or set with Now), Use A-B range, Last N s, Export and Export As…. **Advanced** (closed until opened) holds the audio track, the quality (Same as source, High quality, Small file, or resolution, frame rate, quality number, speed and audio bitrate), video effects and burned-in subtitles. While a clip exports, its progress (percentage, time left) shows at the top of the page; the clip's range shows on the seek bar while the tab is in front.
- **Results page**: running and queued exports with Cancel, failed ones with Retry, and the last twenty finished captures (kept across restarts) with Open, Copy and Show in Folder. What finishes while another page is in front is counted on the Results sub-tab.
- **Settings page**: the snapshot folder (or beside the video when possible), the image format (PNG, JPEG with its quality, BMP), whether snapshots include the video effects, **Burst** (2–20 frames at a fixed interval, into a folder of their own), the file-name templates (`{name}`, `{pos}`, `{start}`, `{end}`, `{date}`, `{time}`, `{n}`; the defaults are the names the plugin always used, a clip also states its end) and the Last N s length.
- The "saved" message has buttons: Show in Folder, and Copy (a frame) or Show in Tab (a clip or burst). Existing files are never overwritten.

## Entries

Right-click menu: Video ▸ Snapshot.

- Save Current Frame (Ctrl+E), Copy Current Frame (Ctrl+C)
- Burst… (Ctrl+G) and Export Clip… open the tab at their page
- Capture… (Alt+S) shows or hides the tab

In the command palette (Ctrl+Shift+P): Save Rendered Frame (Ctrl+Alt+E), Copy Rendered Frame (Ctrl+Alt+C), Save / Copy Video Frame Only (the source frame without subtitles), the Save As… commands, Export Clip Range (Ctrl+Shift+E, the Export button), Export Last N Seconds (Ctrl+Shift+R), Set Clip Start / End, Export Tasks…, Snapshot Settings…, Snapshot Folder…, Open Snapshot Folder, Show Last Capture in Folder, Copy Last Snapshot, Capture as PNG / JPEG / BMP, Include Subtitles in Rendered Frame.

## Limits

- Clips need a local video with a known duration; frames work wherever the player can capture one.
- While subtitles are shown, Save and Copy Current Frame (Ctrl+E, Ctrl+C) also include the video effects of plugins that have any: the player draws subtitles only into the frame with effects (a host request to separate them is to follow). Subtitles it cannot draw into a frame (only plain text can be) are left out, and the message says so.
- A Fast cut starts on a keyframe and copies the video and one audio track, no subtitles; files whose codecs cannot be copied need Precise. Precise clips are H.264 + AAC in MP4, encoded on the cores the player can spare (at most half, at below-normal priority).
- Use A-B range needs the A-B loop switched on.

## Translations

Texts are English in the source; `locales/` has Simplified Chinese, German, Spanish, French, Japanese and Korean (`vestra-plugin i18n extract`, `i18n check`).
