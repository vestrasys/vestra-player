# Current status

Updated: 2026-10-20.

## Interface rework progress

The [interface rework](product/ui-spec.md) has two steps:

- **Step one (core, done 2026-10-20, branch `rebuild/core-ui`)**: input binding table, new menu model, main window shell, theme and widget library, on-screen messages and info layer, play sequence, panel state and detaching, persistent interface state, window title, panel context menus; new playback info plugin (Tab). The old APIs were removed; all plugins were moved to the new style with unchanged interaction. Record: [round record](process/rounds/2026-10-core-ui.md).
- **Step two (interaction redesign of each plugin, done 2026-10-20, branch `rebuild/plugin-ui`)**: the 10 plugins and the playback info plugin were redesigned according to the rework documents; at plugin request the core added icon buttons, command unregistration and plugin data directories. Record: [round record](process/rounds/2026-10-plugin-ui.md).

## Plugins

All 10 product plugins use the plugin API (SDK v2, see [doc/plugins/](plugins/README.md)). On 2026-10-20 all plugins were loaded in one player process and given a full GUI test with a real mouse and keyboard; all 12 items passed. This was tested by the developer; there is no conclusion from an independent acceptance tester yet.

| Plugin | Directory | Tested | Not tested / known gaps |
| --- | --- | --- | --- |
| Basic Playback | `src/plugins/playback` | Open, pause/play, drag to seek, speed, mute, volume | Cmd/Ctrl+O shortcut not implemented (the video area only routes Space/←/→) |
| Playlist | `src/plugins/playlist` | Add, play a selected item, previous/next item, end-of-list message, message when switching to a source outside the list | Network items not retested separately; after an automatic advance is judged stale, that row's state stays at "Opening" |
| Video Adjustments | `src/plugins/video` | Brightness/contrast/saturation/gamma/Gaussian blur apply to the picture, reset, adjusting does not interrupt playback | — |
| Audio Workbench | `src/plugins/audio` | Equalizer, presets, reset, playback not interrupted | The "Loudness normalization" check box has low contrast on the dark background |
| Subtitles | `src/plugins/subtitles` | Loading external SRT, mixed Chinese and English display, track selection, style and undo | The track name shows the host's temporary name `vestra-subtitle-*.vtt` instead of the original file name; the plugin allows 8 MiB SRT, the host's upload limit is 2 MiB |
| Capture and Export | `src/plugins/capture` | Save the current frame; the PNG matches the source frame pixel for pixel | Burst capture and MP4 not retested; no numeric progress; the burst page's "Sample start" keeps the time from when the window opened |
| NES | `src/plugins/nes` | Opening a ROM shows picture and sound; the screen clears after stop | Keyboard input and save/load states deferred by scope |
| Plugin Workshop (experimental) | `src/plugins/workshop` | Generate → host validation → install → the generated counter works (local mock model, no paid calls) | Upgrade and rollback not tested in the GUI |
| YouTube | plugin repository `plugins/youtube` | Public video playback | Download not retested in the GUI |
| Bilibili | plugin repository `plugins/bilibili` | Public video playback | — |

There is also the example plugin `src/examples/bookmark` (used by the [tutorial](plugins/tutorial.md)), tested in the player.

## Intermittent problems (not reliably reproduced)

Each appeared once in the GUI smoke test after the host cleanup on 2026-10-20; retrying with the same steps on the host before and after the cleanup did not reproduce them:

- After playing NES and stopping, then opening ordinary media, the subtitle workbench kept showing "loading · 0 subtitle tracks" for that source; refreshing did not help and no subtitles could be loaded; opening another media file afterwards made it normal again.
- After applying picture brightness, loading an external SRT and then seeking, the brightness effect disappeared (the video adjustments window still showed the original value).

## Pending API requests

Handled through the bottom-up flow of the [community workflow](process/community-workflow.md).

| Item | Source | Suggested priority |
| --- | --- | --- |
| Forward all root component callbacks while a window is open, removing the "callbacks must be bound before the first `show()`" restriction | Maintainer assessment | Medium |
| Host broadcasts subtitle state events, removing the subtitle plugin's 1.5 s polling | Maintainer assessment | Medium |
| Unify the subtitle upload limit (plugin 8 MiB / host 2 MiB) | Subtitles plugin | Medium |
| Build tool reports an error for imports of npm dependencies that are not bundled (NES once failed to start after install because of this) | GUI test | Medium |
| Automated integration tests against a real host with plugins as fixtures (currently only unit tests and manual GUI tests) | Maintainer assessment | Medium |
| `commands.unregister` (remove a single command at run time) | Migration | Low, wait until a plugin needs it |
| Numeric progress for export tasks, encoder precheck | Capture plugin | When needed |
| Open media shortcut Cmd/Ctrl+O | Basic Playback | When needed |
| Plugin disable/management API (plugin manager); there is currently no public way to disable a plugin, so the GUI test cannot test "disabling only undoes its own effects" | GUI test | When needed |

## Check commands

In `src/`: `npm run check`, `npm run format:check`, `npm run boundaries`, `npm run test:sdk`, `npm run plugins:build`, `npm run plugin -- test <plugin>`.
In `src/host/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
