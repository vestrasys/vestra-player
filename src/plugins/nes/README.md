# NES Emulator

Play NES (Famicom) game ROMs in the player.

## Features

- Opens ROMs in iNES format (`.nes`); picture and sound play in the player like any other media.
- The small "NES Tools" window shows what is running and can open a ROM or stop it.

## Entries

- `.nes` files open like any other media: Open File (⌘O / Ctrl+O), a drop on the video, the playlist (including Previous / Next).
- Context menu "Emulator": Open NES ROM…, Stop NES, NES Tools…
- Closing the source (F4) stops the game as well. With this plugin disabled, `.nes` files open as ordinary files (and fail).

## Limits

- Needs player 0.4.3 or later.
- ROMs up to 16 MiB.
- No keyboard controls or save states yet.
- No game ROMs included.
- Compatibility is that of [JSNES](https://github.com/bfirsh/jsnes): NTSC only, a limited set of mappers.
