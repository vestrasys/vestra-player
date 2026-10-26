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
- [ ] Plugin Manager and Plugin Store: no rollback anywhere; built-ins can be neither uninstalled nor disabled; third-party plugins can be installed, uninstalled, disabled and enabled.
- [ ] The Quit menu item still shows ⌘Q on macOS (`MENU_SHOWS_CMD`).

### #41 Windows fixes batch
- [ ] `diag!` / runner: launching from a terminal and quitting works; the logs look as before.
- [ ] Startup: mount batching, the `settings-check` thread (settings pages compile off the UI thread) and the start queue. Every settings page opens and every panel mounts; no long stall at start (`VESTRA_FRAME_LOG=1`).
- [ ] Catalog quarantine: none expected. A normal profile starts with no `quarantine.json`.
- [ ] Vestra Player AI in the **installed** `.app` (a path with spaces and no `node_modules` above it): typecheck, check and pack of a generated plugin succeed, and the plugin card shows only after the agent's turn.
- [ ] Plugin Store offline (Wi-Fi off): the store opens; Installed and its details, uninstall, and install from a `.vxp` work; Available/Updates show a calm offline state.
- [ ] Playlist: saving a list with relative entries writes `/`.
