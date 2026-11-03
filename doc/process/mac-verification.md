# macOS verification list

Since 2026-10-26 the GitHub Actions budget has been used up, so CI does not run. PRs are merged after full local verification on Windows (`npm run ci:local` equivalent, `npm run plugin -- test --all`, GUI checks, and an independent review for large batches). macOS is verified by the user on their Mac before the budget is raised again. **This list is what has to be checked there.** Whoever merges a PR adds its macOS-relevant items. Tick an item off when it has been verified on a Mac.

## Automated checks (run once, on the latest main)

- [ ] `cd src && npm ci && npm run ci:local`: all steps, including `cargo test --include-ignored` (GPU/IOSurface tests) and `npm run plugin -- test --all`, which on Windows now passes with 0 failures in all 18 plugins.

## Per PR

### #36 Windows frame pacing (shared parts)
- [ ] `render_target::BufferRing` (4 reused frame buffers) is shared code and live on macOS. Under software rendering (`VESTRA_SOFTWARE_RENDER=1`), 4K60 still plays at 60/60 with `VESTRA_FRAME_LOG=1`; resize and source changes show no stale or torn frames.
- [ ] GPU rendering (default): playback is unchanged. Vblank pacing and the reused texture are Windows-only, so macOS keeps its display link.

### #39 / #40 Plugin management (built-ins in place, no rollback)
- [ ] The `.app` bundle carries the built-ins unpacked under `Contents/Resources/plugins/<id>/` with `builtin.json` schema 2. Signing (ad-hoc, and with an identity if available) and notarization accept the loose files.
- [ ] First start with an empty `~/Library/Application Support/Vestra`: nothing is written under `packages/packages/` for built-ins, and the UI is responsive.
- [ ] Upgrade from a 0.5.0 profile: the old built-in copies are removed off the UI thread, and settings, secrets and third-party plugins are kept.
- [ ] Plugin Store (the only plugin management UI since 2026-10-26; "Settings ▸ Plugin Manager…" is gone): no rollback anywhere; built-ins can be neither uninstalled nor disabled; third-party plugins can be installed, uninstalled, disabled and enabled.
- [ ] The Quit menu item still shows ⌘Q on macOS (`MENU_SHOWS_CMD`).

### #41 Windows fixes batch
- [ ] `diag!` / runner: launching from a terminal and quitting works; the logs look as before.
- [ ] Startup: mount batching, the `settings-check` thread (settings pages compile off the UI thread) and the start queue. Every settings page opens and every panel mounts; no long stall at start (`VESTRA_FRAME_LOG=1`).
- [ ] Catalog quarantine: none expected. A normal profile starts with no `quarantine.json`.
- [ ] Vestra Player AI in the **installed** `.app` (a path with spaces and no `node_modules` above it): typecheck, check and pack of a generated plugin succeed, and the plugin card shows only after the agent's turn.
- [ ] Plugin Store offline (Wi-Fi off): the store opens; Installed and its details, uninstall, and install from a `.vxp` work; Available/Updates show a calm offline state.
- [ ] Playlist: saving a list with relative entries writes `/`.

### Host 0.5.4 / SDK 1.5.24: deinterlace `when: "auto"` (DEINT-AUTO-1)
Default settings, GPU rendering; once `vestra.video` 1.1.6 uses the node (until then, with a test chain of only `{ kind: "deinterlace", when: "auto", output: "field" }`). Interlaced clip (25i, top field first): `ffmpeg -f lavfi -i "testsrc2=size=1920x1080:rate=50,interlace=scan=tff" -t 20 -c:v libx264 -flags +ildct+ilme -x264-params tff=1 -pix_fmt yuv420p interlaced-tff.mp4`.
- [ ] Progressive file: `stats().video.hwdec` (`hwdec-current`) is `videotoolbox`, not `videotoolbox-copy`; `deinterlace-active` false.
- [ ] Interlaced clip: no combing on motion, double rate (50 fps for 25i), `hwdec-current` still `videotoolbox`; the log has `HW-downloading from videotoolbox` and no `no deinterlace filter available` / `creating deinterlacer failed`.
- [ ] Mixed source (progressive then interlaced in one file). Encode the halves separately and join them without re-encoding, so only the second half is flagged: `ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=25 -t 5 -c:v libx264 -pix_fmt yuv420p p.ts`, `ffmpeg -f lavfi -i "testsrc2=size=1920x1080:rate=50,interlace=scan=tff" -t 5 -c:v libx264 -pix_fmt yuv420p -flags +ildct+ilme -x264-params tff=1 i.ts`, then `ffmpeg -i "concat:p.ts|i.ts" -c copy mixed.ts`. Expected: zero-copy (no download) for the progressive part; from the first interlaced frame on every frame is downloaded (mpv keeps its deinterlacer once inserted), with no stall, error or `hwdec` change at the switch; the next file starts zero-copy again.
- [ ] Paused on an interlaced frame, turning Auto off (or stopping the video plugin) shows the combed frame at once, without resuming.

### Carried over from the round records (2026-10-20 to 11-02)
- [ ] ⌥ + named keys (for example ⌥⌦) on a real keyboard trigger their bindings, and the panel context menu still works afterwards; ⌥E/N/U/I/\` dead keys also work in the floating sidebar window.
- [ ] Full UI test items first checked on Windows: CoreAudio with `requestedFrames: 1024`; snapshots and `cover.jpg` on leased loads; VestraNES at 60 fps; subtitle auto-load (at most 2 tracks); store keeps its tab and selects the neighbour row after install/uninstall; capture file naming; hotkeys still offers ⌘ combinations; bottom-bar buttons and their narrowing order; submenu sweep highlighting.
- [ ] `cargo test -- --include-ignored a_click_on_the_seek_bar_seeks_once`, and a click, drag and keyframe drag on the seek bar in the GUI.
- [ ] Playlist row thumbnails: light theme, audio covers, link rows, a 2000-row list.
- [ ] The `.app` first start and `codesign --verify --deep` after the plugin-management changes; the host compiles on macOS after the source split.
