# Playback Info

Information about the media playing, the player and the system.

## Features

- **Playback Info**: file, time, codecs, size, decoder (hardware or software), frame rate, the frames that really reach the screen (`Displayed: 30 fps (source 60)`), audio, dropped frames and more at the top left of the video, refreshed every second.
- **Brief Info**: the file name, time, current chapter, codecs, decoder, displayed frame rate and subtitle track for a few seconds.
- **Playback Properties**: three pages — Playback (decoder, hardware decoding, bitrates, A/V sync…), File (container, streams, chapters, metadata) and System (versions, system, displays, CPU, memory). Copy or save as text.
- **About Vestra Player**: the version, component versions and the loaded plugins.

## Entries

- Context menu **Playback Info** (Tab); Brief Info with Shift+Tab.
- Context menu **Playback Properties…** (Ctrl+F1).
- Context menu **About Vestra Player…** (F1).

## Languages

English, Simplified Chinese (`locales/zh-Hans.po`). The copied or saved text is in the interface language too.

## Limits

- The File page needs ffprobe and reads local files only.
