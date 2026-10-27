# 2026-10 round: full UI test (Windows)

Started 2026-10-27 by the coordinator, after the shared runner (S2), session storage, deinterlace auto, keyframe seeks and SDK 1.5.26 had landed. Machine: Windows 11, Ryzen 9 7940HS, AMD 780M driving the 2560×1600@60 panel at scale 1.75; software rendering (Windows GPU rendering is not on main, user ruling). Builds: release main b76633c, then staged builds of each fix. Test profile: a separate `APPDATA` (`C:\vestra\ui-appdata`), not the user's.

**This document is the record of the test and of the fixes it produced.**

## 1. Shared runner vs own processes (`VESTRA_SHARED=0`)

User ruling (2026-10-27): the shared runner stays for its memory win; scheduling problems are fixed separately.

| Measure (17 built-ins, 4K60 H.264 playing, 20 s after start) | Shared | Own processes |
| --- | --- | --- |
| Node processes | 1 | 25 |
| Node working set | 91 MB | 1424 MB |
| Node private bytes | 98 MB | 1610 MB |
| Node CPU since start | 1.5 s | 12.6 s |
| Host working set | 485 MB | 486 MB |

UI work while 4K60 plays, maximized (`sweep2.ps1`, 7 s per phase, presented frames per second): pointer still on the video 60–61 in both; menu open and still 60–61 in both; sweeping the menu 56–61 (shared) and 59–61 (own); sweeping the seek bar 60–61 (shared) and one second at 31 (own, one occurrence, not reproduced). No scheduling problem found in shared mode.

Fault isolation: killing the shared runner during playback restarted all built-ins in about 1 s; the video kept playing; VestraTube (own process) was unaffected.

NES (homebrew ROM, 60 s, `VESTRA_FRAME_LOG`): after the fixes below, produced/presented 58–61 every second in both modes (mean 60.1 shared, 60.0 own).

## 2. Passed without changes

Single click play/pause (double click unbound); submenus open at once; Deinterlace defaults to Auto; Plugin Store gear menu items open their pages (the user's earlier report: fixed by then); Settings ▸ Mouse defaults; seek-bar thumbnails on hover; click-to-seek lands on the keyframe in keyframe mode while 60 fps holds; arrow keys step by keyframe (5 s step lands on the next keyframe of a 10 s GOP file); playlist row thumbnails toggle; fullscreen bar; VestraTube 0.4.1 install from the store, search, double-click on the title plays, infinite scroll.

Tooling note: synthetic arrow keys need their scan codes (`keybd_event(vk, scan, EXTENDED, …)`): without them winit reports no key.

## 3. Defects found and fixed

| Area | Defect | Fix | PR |
| --- | --- | --- | --- |
| Records | Idle stop recorded as dropped although the 08:59 ruling restored it (Chrome MV3) | Lazy-activation record corrected | #78 |
| Playback | Fullscreen button never implemented after the host half (#59) | Playback 0.2.4 | #80 |
| Plugin repo | Publish failed since VestraTube 0.4.0 (typos on a test video id and YouTube's content-length field name); store still offered 0.3.1 | `_typos.toml`; 0.4.1 released | plugins #10 |
| Host | NES (producer audio) never started on Windows: the 1024-frame buffer check fails on WASAPI (480–480) | Host 0.5.7: default buffer when 1024 is not offered; multichannel/mono layouts; chosen WASAPI device mapped | #82 |
| SDK | Stream producers ran at ~25 fps on Windows: one frame per pump and a 15.6 ms timer tick | SDK 1.5.27: refill the lead per pump | SDK #42 (1.5.27), player #90 |
| Host | Snapshots never available on Windows: the local-file lease route was Unix-only | Host 0.5.8: `vestra-lease://` stream (rsmpv stream-cb), identity-checked; the playing file stays renamable and deletable; network cache and sidecar cover art supplied by the host | #83 |
| Subtitles | No same-name subtitle auto-load (host sets `sub-auto=no`) | Subtitles 0.4.5: mpv `exact`-style auto-load, at most 2 tracks | #84 |
| Hotkeys | "Cmd" (Windows key) offered as a mouse modifier on Windows; Win+key recorded as Ctrl | Hotkeys 0.1.7 | #85 |
| Capture | File name 4–6 frames before the captured picture | Capture 1.0.4: capture at the press, name from the captured frame | #86 |
| Store | Detail pane showed a plugin not in the visible list after a tab switch | Store 0.2.2 | #87 |
| Video | `HostError: render options need GPU rendering` logged at every start under software rendering | Video 1.1.7 | #88 |
| VestraTube | Results list jumped to the top when the sidebar view was rebuilt (tab switch, shared-runner restart) | VestraTube 0.4.2 | plugins #11 |

Every fix went through the plugin's or maintainer's author, an independent review (fixes back to the author), the coordinator's GUI check on a staged build, then merge.

## 4. Coordinator decisions taken during the test (standard practice)

- Snapshots on Windows: no file locking (mpv opens with delete sharing); stream-cb so mpv reads the verified object, as `fd://` on Unix. Leased loads keep lavf (parity with macOS): mkv ordered chapters and editions are not followed for local files on either platform.
- Subtitle auto-load capped at 2 tracks per video (the host allows 4 live tracks per plugin per source; room for a manual load and a reload).
- Store: the tab stays after install/update/uninstall; the neighbour row is selected.
- Capture: the frame at the key press is captured (mpv, PotPlayer).

## 5. Open items and rulings

Installed player (`%LOCALAPPDATA%\Programs\Vestra Player`) updated to main 0e84da7 (host 0.5.8) at the end of the test; smoke-tested with the user's profile.

- Alt+↑/↓ were bound by both subtitles (Move Up/Down) and video (pan). User ruling 2026-10-27: the pan keeps them; subtitle Move Up/Down have no default key (subtitles 0.4.6, #89; product docs updated).
- SDK command table (`commands.js`, `llms-full.txt`) still lists Alt+ArrowUp/Down for the subtitles commands; update with the next SDK release.
- Host test `a_click_on_the_seek_bar_seeks_once` failed once (2 seeks) under the full parallel `ci:local` and passed 5/5 alone: make it independent of timing under load.
- Store offline test (owed by the coordinator since the earlier round).
- Host follow-ups from the snapshot reviews: network detection by handle (`FILE_REMOTE_DEVICE`, mpv's way); read mpv `image-exts` for cover art; cover listing off the backend thread for huge SMB folders.
- SDK follow-ups: TestHost lacks the host's subtitle limits (4/16/16) and reports; the host reports a hard limit as `busy`; a read-only GPU/software flag for plugins (pure addition).
- Audio plugin's same-name audio rule lacks the "closer video wins" rule the subtitles plugin has.
- Synthetic Alt+Shift switches the input language (it did, to German); avoid bare Alt+Shift sequences in GUI scripts. Restored to Chinese Pinyin after the test.

## 6. Mac test list (CI paused)

- `cargo test --features producer-audio real_output_device -- --ignored --nocapture` reports `requestedFrames: 1024` (CoreAudio unchanged).
- Snapshots on macOS unchanged: Save Current Frame on a local file; an mp3 with `cover.jpg` beside it shows the cover (now supplied by the host for leased loads).
- NES at 60 fps with SDK 1.5.27.
- Subtitle auto-load, store selection, capture naming, hotkeys (Cmd still offered on macOS) as on Windows.
