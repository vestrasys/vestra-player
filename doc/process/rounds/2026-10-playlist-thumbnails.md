# 2026-10: Playlist row thumbnails (playlist 0.1.11)

User request (2026-10-26): the playlist shows a video thumbnail on each row by default, like the VestraTube sidebar; it must be lazy the way durations are, and must not slow the playlist, opening a video or playback. The on/off switch is a **button in the playlist's bottom row** (Add and Remove narrowed to make room), default on, remembered, with an icon that shows the current mode. No settings-page entry.

## Design (playlist author)

- Thumbnails come from the plugin's own `ffmpeg` run (`plugin.tools.run`, no new capability): for a video, the keyframe at or before 10 % of the duration, decoded alone, embedded cover art excluded, scaled to fit 192×108, JPEG from stdout, written to a temp file and renamed; for audio, the embedded cover, else a note icon; for links, a globe icon and the site's domain. Cache: `<dataDirectory>/thumbs/<sha1(realpath+size+mtime)>.jpg` with an LRU index (5000 entries); unreadable files are remembered as "no picture" (no retry storm), timeouts only for the session, no ffmpeg at all disables the feature.
- Laziness, as `probe.ts`: only visible rows ± 3; cache lookups 4 at a time, then at most 2 ffmpeg processes (1 thread each); work for rows scrolled away is dropped (queued) or ended (running); nothing runs while the list is hidden or unmeasured; all ffmpeg work pauses while a source is loading and for 3 s after it is ready.
- Rows keep plain values; images live in a separate `[image] thumbs` list with one ImageRef per picture, so a thumbnail's arrival re-sends only `thumbs`. On main 0.5.1 image rows already reconcile in place when the length is unchanged; the split is kept because it is cheaper and does not depend on the host's merge path.
- Rows are 62 px with thumbnails (96×54 picture, duration badge, two-line title, folder), 20 px without; `minPlayerVersion` 0.5.1 (images in lists).

## Measurements (author, headless, machine loaded)

One file: 1080p 207–395 ms, 4K60 H.264 422 ms, 4K HEVC 10-bit 443 ms, AV1 228 ms, long-GOP clips 132–290 ms (one keyframe, so GOP length does not matter); 2–3 KB per JPEG. Adding 2000 real videos: list shown in 124–171 ms with thumbnails on or off (no change); first screen of 17 thumbnails complete in 2.7–3.8 s; scrolling to row 1500 ran 20 more ffmpeg runs (37 in total).

## Checks

Author: `plugin -- test playlist` 119/119, `check`/`build`, `npm run check`, `format:check`, `boundaries`, `typos` all pass. Coordinator: host `ui::tests::all_plugin_and_example_slint_files_compile` and `ui_types::tests::recorded_plugin_props_match_their_slint` pass against this worktree (`VESTRA_UI_RECORD` from the author's recording).

## Acceptance (coordinator, Windows, 2026-10-27, staged on host 0.5.1 with store 0.2.1)

- Opening a 10-min 1080p clip: its row shows the picture, the `10:00` badge, the title and the folder name within seconds; no ffmpeg process was left running afterwards.
- Add Files… with seven clips (1080p, 4K H.264/HEVC 10-bit/AV1, long GOP): all rows got pictures and badges within ~12 s; the AV1 "screen" clip shows its dark first keyframe, as expected.
- A bogus entry (a mistyped path) shows the placeholder and `--:--` and is not retried; Remove (−) deletes it.
- The bottom-row button: tooltip "Hide Thumbnails" / "Show Thumbnails", the icon follows the mode, the list switches between 62 px and 20 px rows; the choice survives a restart (compact rows and the compact icon after restarting with thumbnails off; on again after toggling).
- Not checked: 2000-file lists in the GUI (author's headless numbers stand), audio covers, link rows, narrow sidebar, light theme, drop insertion with 62 px rows, Mac.
- Note for the staging step: `resources/plugins/builtin.json` must list the new plugin version, or the host refuses the built-in ("its manifest is vestra.playlist 0.1.11, not 0.1.9"); write it without a BOM.
- Known host defect seen here (fixed on the `ui-fixes` branch): the first click on the toggle after its tooltip appeared was swallowed by the old `ToolTipArea`.
