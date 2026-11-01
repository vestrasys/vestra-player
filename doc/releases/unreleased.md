# Notes for the next release

User-facing notes of host versions built since 0.5.0 that had no release of their own, moved here from the [architecture overview](../architecture/README.md) on 2026-11-01. They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

- **0.5.11**: Installing or uninstalling a plugin with a panel no longer makes the main window flicker or move.
- **0.5.12**: `plugin.app.system()` reports `rendering` (`"gpu"` or `"software"`, SDK 1.5.34); an audio file in a huge network folder no longer holds up the player while its cover art is looked for.
- **0.5.13**: Plugins can read why the player renders on the GPU or in software, choose software or GPU rendering for the next start and restart the player; the player keeps GPU rendering off on graphics drivers known to misbehave and after a start that crashed in GPU rendering.
- **0.5.15**: The player's background threads no longer wake every few milliseconds when there is nothing to do; after moving the window to another display the video follows that display's refresh at once; the frame log reports presented frames under the software window renderer.
- **0.5.19**: On Windows the player draws its windows with Direct3D 12 also when it renders video in software (smoother, vsynced presentation; OpenGL or software windows remain the fallback); in full screen, interface updates no longer add presents between video frames; the experimental GPU rendering no longer waits for each frame on the CPU and recovers from a stalled GPU.
- **0.5.23**: On a laptop with two GPUs, GPU rendering uses the high-performance GPU unless you chose a GPU for Vestra in Windows Settings > System > Display > Graphics.
