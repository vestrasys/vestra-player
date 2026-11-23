# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## Improvements

- **Motion smoothing on NVIDIA GPUs** (T8; with the store plugin NVIDIA Motion Smoothing). On a GeForce RTX 30 series or newer GPU with driver 573.38 or newer, the player can double a video's frame rate with in-between frames made from the GPU's optical flow hardware, as AMD Motion Smoothing does on Radeon GPUs: up to 4K at 30 fps; the plugin says when frames are being dropped. Nothing changes until the plugin turns it on; the converter's Vulkan device exists only while it runs. The player's video library is now built on FFmpeg 9.1.
