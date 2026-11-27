# Playback Info

Information about the media playing, the player and the system.

## Features

- **Playback Info**: file, time, codecs, size, decoder (hardware or software), frame rate, the frames that really reach the screen (`Displayed: 30 fps (source 60)`), audio, dropped frames and more at the top left of the video, refreshed every second.
- **Brief Info**: the file name, time, current chapter, codecs, decoder, displayed frame rate and subtitle track for a few seconds.
- **Playback Properties**: three pages — Playback (decoder, hardware decoding, bitrates, A/V sync…), File (container, streams, chapters, metadata) and System (versions, system, displays, CPU, memory). Copy or save as text.
- **About Vestra Player**: the version and the copyright line (`© 2026 Ziyang Song`), component versions, how the video is rendered and why (`Rendering: Software — <the player's reason>`, `Rendering: GPU`; left out on players before 0.5.12, the reason before 0.5.13) and the loaded plugins.
- **Check for Updates**: the latest release of Vestra Player on GitHub, its date and release notes; downloads the installer for this system (from that release, never overwriting another file of the same name) into the Downloads folder, checks its size and SHA-256, and starts it when you choose Run Installer (never silently). An automatic check runs 30 seconds after start and at most once a day (an hour after a failed one), with one message per new version; turn it off in Options ▸ General ▸ Updates.

## Entries

- Command palette (Ctrl+Shift+P): Info: **Playback Info** (Tab); Brief Info with Shift+Tab.
- Command palette: Info: **Playback Properties…** (Ctrl+F1).
- Context menu **Tools ▸ About Vestra Player…** (F1), the last item of Tools ▸.
- Command palette: Help: **Check for Updates…** (also the Updates row of the About window); Options ▸ General ▸ **Updates**.

## Languages

English, Simplified Chinese (`locales/zh-Hans.po`). The copied or saved text is in the interface language too.

## Limits

- The File page needs ffprobe and reads local files only.
