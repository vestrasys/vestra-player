# Audio Workbench

Adjusts the sound and picks the audio track and the output device.

## Features

- **Equalizer**: six bands (60 Hz–10 kHz, ±6 dB) with an on/off switch and four presets: Flat, Voice, Music, Cinema.
- **Audio sync**: ±10 seconds in 50 ms steps; Don't Keep, Keep for This Run or Always Keep.
- **Processing**: Normalize (evens out the loudness), Crystalizer (brightens the treble, adjustable strength).
- **Channels**: Stereo, Left Channel, Right Channel, Mono Mix, Swap Left and Right.
- **Audio tracks**: pick or cycle through the tracks, load an external audio file; optionally load audio files with the same name as the video automatically.
- **Output device**: the sound device to play on; when it is unplugged the system default takes over, and the choice returns when it is plugged back in.

These settings are kept across files and restarts (the audio sync as its keep mode says).

## Entries

- Right-click menu Audio: Audio Workbench…, Audio Track, Output Device, Equalizer (Shift+E), Preset, Audio Sync (`>` earlier, `<` later, `?` reset), Processing (Normalize Shift+N, Crystalizer Shift+C), Reset.
- Audio ▸ Audio Track ▸ Cycle Tracks: Alt+A.
- Open ▸ Load External Audio….
- The Audio Workbench… window has three tabs: Equalizer, Effects, Output.

## Limits

- Volume and mute are the player's, not this plugin's.
- A loaded external track stays until the source changes.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
