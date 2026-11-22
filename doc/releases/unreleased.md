# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## Improvements

- **Motion smoothing on NVIDIA GPUs** (T8; with the store plugin NVIDIA Motion Smoothing). On a GeForce RTX 30 series or newer GPU with driver 573.38 or newer, the player can double a video's frame rate with in-between frames made from the GPU's optical flow hardware, as AMD Motion Smoothing does on Radeon GPUs: up to 4K at 30 fps with GPU rendering and 4K at 24 fps with CPU rendering. Nothing changes until the plugin turns it on; the converter's Vulkan device exists only while it runs. The player's video library is now built on FFmpeg 9.1.

- **Less memory on Windows.** The player now uses Windows' newer segment heap, which returns freed memory to the system sooner: about 20–45 MB less private memory with CPU rendering (idle 188 → 161 MB, 1080p 263 → 238 MB, 4K 406 → 387 MB, with the adjust panels 464 → 429 MB), about the same with GPU rendering. Frame rate, start-up time and CPU use are unchanged. Windows 10 before version 2004 ignores the setting.

## Built-in plugins (no host change)

### Fixed

- **The Plugin Store and Vestra Player AI windows resize properly again.** The Plugin Store can be made as tall as the screen, can no longer be squeezed below 480 pixels, opens 640 high again, and its cards no longer jump while the window is resized: the card at the top left stays in the top row when the number of columns changes. The Vestra Player AI window can be made wider than 720 pixels again, no longer gets narrower than that, and opens 900 wide (Store 0.3.3, Workshop 2.2.47). The same hidden-element pattern was made safe in the About, Properties, Marks and Add Links windows, which were not affected (Info 0.1.17, Marks 0.1.11, Playlist 0.2.11).
