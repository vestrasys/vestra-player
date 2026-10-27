# Handoff: continue Vestra Player on Windows

Written 2026-10-25 by the coordinator session that ran on macOS. A new Claude Code session on the user's Windows machine continues from here. **Read this file fully, then `agentic-workflow.md`, before doing anything.** The user will not write a separate prompt; this file is the brief.

## 1. Who you are and how to work

You are the **coordinator** of a simulated plugin community (`agentic-workflow.md`). That document is binding. In short:

- **Delegate.** All investigation, design and implementation go to sub-agents in their roles: API maintainer (host, `src/runtime`, SDK repo, `src/scripts`, `src/tests`, `doc/architecture`), plugin authors (one plugin directory each), infrastructure maintainer (`.github/`, `src/packaging/`, `doc/process/ci.md`), researcher (read-only), independent verifier (before merges). Your own context is for dispatching, recording, reviewing, merging and real GUI tests. Fix only trivial 1–2 line issues yourself.
- **Only you** run `git commit`, `push`, merge PRs, tag releases and drive the GUI. Sub-agents never commit, push, open PRs or start player windows.
- **API changes start bottom-up:** a plugin author writes a §4.1 request (evidence, proposed API) into the current round doc under `doc/process/rounds/`; the maintainer replies there (§4.3, conservative, alternatives first). The round doc is the blackboard and the single source of progress.
- **Each sub-agent works in its own git worktree** and its own `CARGO_TARGET_DIR` (delete it afterwards; disk is limited). Brief them with the files they own, the exact acceptance checks, and "no commit/push; report briefly in Chinese".
- **Verify before merging:** `cd src && npm run ci:local` must pass (21/21). Use an independent verifier agent for larger batches.
- **CI costs money** (private repo; macOS minutes bill 10×, Windows 2×). Batch doc pushes; don't re-run both push and PR runs; PRs are the normal path into `main`.

### Rules the user has given (carried over from the macOS session's memory, which you cannot see)

- **Language:** reply to the user in **Chinese**. All documentation (repo docs, round docs, SDK docs, READMEs, PR bodies, commit messages) is **English**.
- **Cross-platform:** the product ships on macOS and Windows. The plugin API stays OS-neutral; OS-specific code lives only in host platform modules (`src/host/src/platform/`, unsafe code only in `src/host/unsafe/*` crates registered in the unsafe policy). Plugins never call OS commands.
- **SDK docs live in one place:** https://vestrasys.github.io/vestra-player-sdk/ (repo `vestrasys/vestra-player-sdk`, public). Nothing a plugin author needs may live only in this private repo or in code. Before restructuring docs, write a detailed plan and get the user's approval.
- **SDK versions:** patch bumps by default (1.5.x), even for additive changes. Releases need the user's npm approval in the Actions run (tag `vX.Y.Z` → Release workflow waits for approval).
- **Examples** in the SDK are atomic: one feature, one `main.ts`, plain functions.
- **UI fixes:** never redesign. `doc/product/ui-spec.md` and the existing screens are the standard; fix only what is clearly broken or inconsistent. Do not name other software as a reference (sole exception: the plugin center list rows may follow the VS Code extensions view).
- **Release testing depth:** 0.5.1 is local only (no CI release); test only the changed areas. **0.6.0** (next published release) needs one complete UI pass of every window, page, menu and flow (including Vestra Player AI, the plugin store and every settings page) in one language, plus a light spot check of the other languages. 0.5.0 shipped with Vestra Player AI broken because a window was not opened during the check.
- **Plugin classification:** only built-ins are official (`vestra.` ids are reserved). Plugins in `vestrasys/vestra-player-plugins` are third-party (`zysilm.youtube`, `zysilm.bilibili`; `zysilm` is the user's GitHub id).
- **Safety:** ask before any force push. Never print or commit signing keys or API tokens. Don't type the user's passwords. Before every synthetic click or key in a GUI test, verify the player is the frontmost window.
- **Releases are manual:** tag → the Release workflow builds and drafts → you check the draft → publish (as pre-release while unsigned).

## 2. State at handoff

- `main` (8472d10 at handoff) = 0.5.0 (published pre-release https://github.com/vestrasys/vestra-player/releases/tag/v0.5.0) plus the 0.5.1 work merged since (PRs #27–#33): Vestra Player AI and subtitle settings windows open again; plugin props are type-checked against their Slint in CI; volume and mute are remembered; the playlist centres the current item when reopened; the fullscreen bottom bar shows only on hover (FS-BAR-1); the info overlay shows the decoder and the frame rate that reaches the screen, and `VESTRA_FRAME_LOG=1` is much richer; plugin-author docs live only on the SDK site (with agreement tests); all repo docs are in English. The host version is still 0.5.0; 0.5.1 is not released (user ruling: no CI release for 0.5.1; 0.6.0 is the next published release).
- SDK: 1.5.15 on npm; the player depends on `^1.5.15`.
- No open PRs at handoff. Check `git log` and `gh pr list` when you start.
- Rounds and their docs: `doc/process/rounds/` (most recent: `2026-10-i18n.md`, `2026-10-ui-polish.md`, `2026-10-windows-render.md`).

## 3. Your tasks, in order

### Task 1 — Windows plays 4K60 at ~30 fps and UI animations stutter (0.5.1)

What we know (details and code references: `doc/process/rounds/2026-10-windows-render.md`, sections 1 and Appendix A):

- User's machine: 60 Hz monitor; mpv plays the same files smoothly. Vestra on Windows: hardware decoding is active (Task Manager → GPU → Video Decode), but video shows ~30 fps, and **the playlist panel animation also stutters**, so the whole UI seems to run at ~30 Hz.
- `VESTRA_FRAME_LOG=1` logs from the user (4K60 AV1): mpv **produced ≈60 fps** in every run; **shown ≈27–28** (default), ≈35–37 (`SLINT_BACKEND=winit-software`), ≈30–35 (`SLINT_BACKEND=winit-femtovg`). So decode and mpv rendering are fine; frames are lost on the UI thread or in presentation.
- **Rejected hypotheses:** Windows 15.6 ms timer granularity (winit 0.30 uses high-resolution waitable timers; new frames wake the UI thread via posted messages; values of 33–37 fps are impossible under a 31.25 ms cap) and a 30 Hz monitor (the user confirmed 60 Hz).
- On macOS (M2) the same software path holds 4K60 at 60/60 but costs ~320% CPU; GPU rendering is 60/60 at low CPU.
- `VESTRA_FRAME_LOG=1` now also prints presented frames, redraws/s, display refresh rate, host ticks and max/mean tick/render/set times on the UI thread; the info overlay (Tab) shows the decoder and "Displayed: N fps (source M)".

What to do:

1. Build and run from source on this machine (see section 4). Reproduce with a 4K60 clip (the user has a 4K60 AV1 screen recording; you can also generate HEVC/H.264 4K60 test clips with the bundled `ffmpeg`).
2. Measure with the extended frame log in windowed and fullscreen modes, with both Slint backends. Find where the UI thread spends its time per frame (texture upload, `set`, host tick, Slint render, present/vsync wait) and why redraws happen at ~30 Hz. Profile if needed (Windows Performance Recorder / ETW, or `cargo flamegraph`).
3. Fix the root cause in the host (Windows-specific parts in `platform/windows.rs` or an unsafe crate). Acceptance: on the 60 Hz monitor, 4K60 plays with presented ≈ 60 fps windowed and fullscreen, UI animations are smooth, CPU usage is reported. Also check 1080p60 and 24 fps content.
4. Have the user confirm on their screen; then merge.

### Task 2 — Windows GPU rendering (0.6.0, after Task 1)

The research recommends: mpv renders to a D3D11 texture through the render API that the shipped libmpv already has (render API 2.6, `render.h:481`; the header `render_d3d11.h` must be vendored), shared with Slint's femtovg OpenGL renderer via `WGL_NV_DX_interop2`, mirroring the macOS IOSurface design (`src/host/unsafe/gpu-render`, `platform/macos.rs`). Zero-copy d3d11va decoding, same HDR handling as macOS, shaders and peak detection work, software fallback kept. Estimated 9–13 agent-days. Risks: hybrid-GPU laptops (OpenGL and D3D on different adapters), shader compile errors on Windows. Steps and acceptance (4K60 AV1/HEVC/H.264 ≥ 59.5 fps presented, CPU ≤ 25%, zero-copy confirmed) are in the round doc §3. Start with the standalone prototype gate: confirm the shipped libmpv really renders through the D3D11 API.

Decide with the user whether Task 2 is still top priority once Task 1's result and CPU numbers are known.

Related finding (macOS, 2026-10-25, info overlay): a plain 4K60 HEVC file decodes as `videotoolbox-copy` (copy-back), not zero-copy, under GPU rendering. The frame log at open shows the host always installs an `@vestra-orientation` lavfi filter (`[null,sidedata=mode=delete:type=DISPLAYMATRIX]`, `source_geometry.rs`), and any lavfi video filter forces copy-back (`processing.rs` `frames_in_memory`). Zero-copy on either platform needs this handled without a software filter when there is no rotation (or with mpv's own `video-rotate`). Investigate as part of Task 2.

### Task 3 — Windows real-machine checks for things only tested on macOS

- Chinese/Japanese/Korean text without the Chinese language pack (font fallback chain; `VESTRA_DEBUG_FONTS=1` prints the chains).
- Open With / double-click / single instance / the installer's optional associations (`src/packaging/windows/vestra.iss`).
- FS-BAR-1 fullscreen bar, volume memory, playlist centring (verified on macOS only).

### Later (parked; ask the user before starting)

- **Plugin center + UI polish round:** `doc/process/rounds/2026-10-ui-polish.md` (store absorbs the plugin manager: tabs, VS Code–style row buttons, logos, gear menu; then fix clearly broken UI only).
- **Video processing API direction:** core video processing as mpv GPU-side parameters + user GLSL shaders (always zero-copy, drop lavfi video filters); requires Windows GPU rendering first. A future "native video processor" capability for things shaders cannot host (DLSS 5 can be injected on a final frame; the leaked runtime is not shippable). Needs a researcher report first.
- **API category completion (WP7):** most SDK API symbols are still in the TypeDoc "Other" group; complete `@category` per module in patch releases.
- **Smaller open items:** OSD text longer than 200 characters after translation is rejected instead of truncated; plain strings in command paths can collide with catalog msgids (document `text("{}")` or add a data marker); marks search does not find untitled bookmarks by their localized name; numbers use "." in every language; the picture window's status message is ellipsized; `⌥⌦` in the playlist and real-mouse double-click on playlist rows still need a real-input check; the SDK storage test 'debounced atomic save' flaked once on Windows.

## 4. Setting up on Windows

- Clone `vestrasys/vestra-player` (private; `gh auth login` first) and, when needed, `vestrasys/vestra-player-sdk` and `vestrasys/vestra-player-plugins`.
- Toolchain: Rust (MSVC), Node 26, the native dependencies as CI fetches them (`src/packaging/collect-windows-deps.mjs`, pins in `src/packaging/deps.json`; `.github/actions/setup-libmpv`). `doc/process/ci.md` describes every step CI runs on Windows; mirror it locally.
- Build and test: `cd src && npm ci && npm run ci:local`; run the host from source per `doc/architecture/README.md`; package with `src/packaging/bundle-windows.mjs` when you need an installer-like layout.
- GUI testing on Windows: the macOS session used small Swift tools for synthetic input and `screencapture`; on Windows write equivalents (PowerShell with `SendInput` / `System.Windows.Forms`, or a tiny Rust/C# helper) and keep them out of the repo. Always check the foreground window before sending input.
- Diagnostics: `VESTRA_FRAME_LOG=1`, `VESTRA_DEBUG_FONTS=1`, `VESTRA_HWDEC=<mode>`, `SLINT_BACKEND=winit-software|winit-femtovg`. The release build is a GUI-subsystem exe, so redirect stderr (`Start-Process -RedirectStandardError`). The player is single-instance: close every Vestra window before a measured launch.

## 5. When you finish a task

Record the outcome in the round doc, merge through a PR, and tell the user in Chinese what changed, what was verified on this machine, and what still needs their eyes. Delete this file (or move its still-open parts into a round doc) once all its tasks are done.
