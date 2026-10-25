# 2026-10 round: S7 packaging and release

Started 2026-10-22, after the baseline main 13e2cb5. User instruction: "after Dependabot is handled, go straight to S7" (处理完 Dependabot 直接做 S7). The previous round: [2026-10-slint-store.md](2026-10-slint-store.md).

**This document is the only progress source for this round.** After a context compaction, the coordinator reads this document first and then continues.

## Goals (ecosystem plan S7, item 12 of 1.6)

1. The player bundles Node 24 LTS and does not require users to install it; the SDK `engines` and type checks are adjusted to this version.
2. macOS: an `.app` (display name Vestra Player, the new logo icon, containing libmpv and its dependencies, the built-in plugins, Node and third-party licenses), then a `.dmg`.
3. Windows: an installer (with the same contents).
4. Third-party licenses: cargo-about for Rust dependencies; a manifest for non-Cargo dependencies such as libmpv/ffmpeg, Node and fonts, and CI checks that the manifest matches the actual files in the package.
5. Wired into `release.yml`: push a `v*` tag → full checks → packaging for each platform → a draft release at the public release location.
6. Signing (Apple Developer ID, Windows code signing): this round only leaves the hooks and adds no certificates; unsigned test builds come first.

## Coordinator decisions

**2026-10-22 (the user said "go straight to S7" (直接做 S7) and did not object to the two proposals in the previous message)**
1. Release location: create a new public repository `vestrasys/vestra-player-releases` that holds only installers and release notes (the player source repository stays private). Created when the release step needs it.
2. No signing yet; draft releases are unsigned first; the signing steps are TODO jobs under `environment: release`.

**2026-10-22, step 7.2 (the parts of the proposal the coordinator can decide)**
3. The first macOS version is arm64 only, minimum macOS 14.0 (picture-in-picture requires 14); dependencies are built on the `macos-14` runner. An Intel version is added later on demand.
4. Dependency source: a new `deps.yml` builds libmpv and other dependencies once, pins their hashes and mirrors them to the release repository; no fresh Homebrew install on every release, and Windows does not rely on old shinchiro builds that get deleted.
5. ffmpeg/ffprobe ship with the package on both platforms (export, embedded subtitles, durations and thumbnails all depend on them); the host sets `VESTRA_FFMPEG`/`VESTRA_FFPROBE`, the SDK does not change.
6. Windows: Inno Setup per-user install (no administrator) + a portable zip; uninstalling keeps user data by default; the exe uses the GUI subsystem, Node child processes open no console; the VC runtime is linked statically.
7. Turn off mpv's built-in Lua scripts and ytdl hook (network videos are resolved by plugins through yt-dlp; mpv does not need to call it itself).
8. Icon: first generated automatically from the logo on a rounded plate; replaced later when there is a design.
9. Release notes: `doc/releases/v<version>.md`; the release fails if the file is missing.
10. No file associations in the first version (the host cannot yet receive file paths from the system); scheduled separately.
11. Packaging uses our own scripts (not cargo-bundle/cargo-packager); the DMG uses the system `hdiutil`.
12. Add a LICENSE at the repository root (the full GPL-3.0 text); include the SDK's LICENSE when staging the runtime; add the jsnes (Apache-2.0) license to the nes plugin (handed to the nes author).

**2026-10-22, user rulings**
13. Node major version **26** (from 26 LTS; during `vestra/1` it only goes up, never down). The SDK `engines` follows (SDK 1.2.0).
14. GPL: the source repository will be open-sourced later; until then installers are not published externally, only internal test builds. Installers are published on **the `vestra-player` repository's own GitHub Releases**, and the website will point there too. **Overrides decision 1**: no new `vestra-player-releases`; the release uses this repository's `GITHUB_TOKEN` and needs no extra credentials. When the repository becomes public, the releases and the source become public at the same time, which satisfies the GPL. The dependency mirror (decision 4) also goes to this repository's Releases (fixed tags such as `deps-<date>`).
15. bundle id: `com.vestraplayer.player` (user ruling). A bundle id is only a naming convention: it does not require owning the domain and is not registered with Apple (no App Store); it conflicts only when two apps on the same Mac have the same full ID. vestrasys.com and vestra.com are registered by others; vestraplayer.com was not registered at the time (2026-10-22) and can be registered later for a website.

**2026-10-22, deviations in step 7.3**
16. Minimum macOS version changed to **15.0** (overrides 14.0 in decision 3): Homebrew no longer ships prebuilt packages for macOS 14, and supporting 14 would mean building mpv/FFmpeg ourselves. 15.0 (released 2024) is acceptable; building mpv ourselves is scheduled separately later. Dependencies are built on `macos-15`.
17. The packaging scripts are in `src/packaging/` (the proposal said `src/scripts/package/`).

**GitHub Actions quota (2026-10-22)**: this month's 2000 minutes are used up, mainly because the Windows tests of the old SDK CI ran for the full 6 hours each time (a public repository, so the cost was waived, but it used up the quota); the player itself cost about 3 USD. CI on private repositories is blocked until the user sets a budget or the quota resets on November 1. Suggested to the user: set an Actions budget of 20 USD and tick "stop usage when the limit is reached". Agents now verify locally first and use CI only when necessary.

**2026-10-24 Windows progress**: Windows build and tests pass (added `vulkan-1.dll`, Git Bash path escaping, the gpg path); the installer is built and installs silently, but the post-install smoke test has 0/18 plugins ready; under investigation.

## Process

Top-down (community-workflow §5): the maintainer researches and writes a proposal → the coordinator decides → the maintainer implements (host finds the bundled Node, resource paths, packaging scripts, licenses) → CI release rehearsal (dispatch dry_run) → the coordinator tests the packaged `.app` on a real machine (double-click to start, all plugins active, playback, store install) → merge. The Windows installer is built and smoke-tested on CI; real-machine acceptance comes with S6.

## Progress

- [x] 7.1 Maintainer research and proposal: [packaging-proposal.md](../../architecture/packaging-proposal.md) (trial .app 231 MB, DMG 76 MB; with node removed from PATH, all 18 plugins failed to start, confirming that Node must be bundled)
- [x] 7.2 Coordinator decisions (including user rulings 13–15)
- 7.3 in progress: host `s7/host` (0872b84, db6983d, all pass locally, Windows CI not run because of the quota); packaging `s7/packaging` (d3b9c73; deps.yml 39788694017 succeeded, release `deps-20261022`; built locally: .app 268 MB / DMG 78 MB, all dependencies loaded from inside the package)
- [ ] 7.3 Implementation
- [ ] 7.4 Release rehearsal (unsigned)
- [ ] 7.5 Real-machine test of the macOS package
- [ ] 7.6 Merge

## Record

- 2026-10-22: SDK Dependabot PR #1 (TypeScript 5.9 → 7.0) closed. Reason: TypeScript is a runtime dependency of the SDK, used by plugin authors for type checks; typedoc 0.28 supports only up to 6.0. PR #2 stops Dependabot from proposing TypeScript major versions. Upgrading to 7 is assessed later as a separate SDK version.
- 2026-10-22: disk cleanup: deleted `src/host/target` (7.7 GB, old portability output) and the old build directories from the Slint 1.16 era under `~/.cache/vestra-target`; free space went from 11 GB to 50 GB.
