# Audio

Adjusts the sound and picks the audio track and the output device.

## Features

- **Volume**: the player's volume and mute, as on the bottom bar.
- **Equalizer**: ten bands (31 Hz–16 kHz, ±6 dB) with an on/off switch and four presets: Flat, Voice, Music, Cinema.
- **Audio sync**: ±10 seconds in 50 ms steps, or typed; Don't Keep, Keep for This Run or Always Keep.
- **Processing**: Normalize (evens out the loudness), Crystalizer (brightens the treble, adjustable strength).
- **Channels**: Stereo, Left Channel, Right Channel, Mono Mix, Swap Left and Right.
- **Audio tracks**: pick or cycle through the tracks, load an external audio file; optionally load audio files with the same name as the video automatically (a file whose name matches another video of the folder more closely, such as `Movie.Extended.ac3` next to `Movie.Extended.mkv`, is left to that video).
- **Output device**: the sound device to play on; when it is unplugged the system default takes over, and the choice returns when it is plugged back in.

These settings are kept across files and restarts (the audio sync as its keep mode says).

## Entries

- **Audio panel**: the Audio tab of the adjust column beside the video. Audio… (Alt+2, also in the command palette; the right-click menu's Adjustments and Effects opens the column at its last tab) opens it, and closes it when it is already shown. Each control shows its key.
- Right-click menu: Tools ▸ Audio Track ▸ (the tracks), Open ▸ Load External Audio….
- Keys: Alt+A cycles the tracks, Shift+E equalizer, `>` / `<` audio 50 ms earlier / later, `?` audio sync back to 0, Shift+N Normalize, Shift+C Crystalizer.
- Command palette (Ctrl+Shift+P): every command by name, such as "Equalizer Preset: Music", "Audio Sync: Reset", "Audio Processing: Reset" (the equalizer, Normalize and Crystalizer back to neutral; sync and channels kept), "Output Device: …".

## Limits

- A loaded external track stays until the source changes.

## Translations

Texts are English in the source; `locales/*.po` have German, Spanish, French, Japanese, Korean and Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`).
