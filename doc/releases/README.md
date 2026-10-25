# Release notes and release steps

This directory holds the release notes for each version: `v<version>.md`, for example `v0.3.0.md`. `release.yml` uses the file as-is as the body of the GitHub release; **if the file does not exist when the tag is pushed, the release fails** (a dry run only gives a warning).

Installers are published on this repository's own GitHub Releases (coordinator decision 14). Until the repository is public there are only internal test builds; every release is a draft and its title says "unsigned test build".

## Release steps

1. **Check that dependencies are pinned**: `release` in `src/packaging/deps.json` points to an existing `deps-*` release; to upgrade libmpv/FFmpeg, run `deps.yml` first and commit the new `deps.json` ([ci.md "Packaging"](../process/ci.md#packaging)). Upgrade the Node patch version with `node src/packaging/fetch-node.mjs --pin <version>` (the major version stays the same).
2. **Bump the version**: `version` in `src/host/Cargo.toml` (`cargo build` updates `Cargo.lock`). Built-in plugins that changed must bump their own versions too, otherwise they are not updated on users' machines (see the notes in `src/scripts/stage-plugins.mjs`).
3. **Write the release notes** `doc/releases/v<version>.md`: user-facing, in English (the release page is for all users); new features, fixes, known issues; for unsigned test builds include the installation steps (see below).
4. **Dry run**: after merging to main,
   ```
   gh workflow run release.yml --repo vestrasys/vestra-player --ref main -f dry_run=true
   ```
   Runs the full checks and builds the installers for both systems and `SHA256SUMS`, without signing and without creating a release. To check only one system, add `-f platforms=windows` (or `macos`); this skips build-test and packaging for the other system (macOS runners are billed at 10x). Tag builds always do both. The outputs are in the run's artifacts (`vestra-unsigned-macos`, `vestra-unsigned-windows`, `vestra-unsigned-SHA256SUMS`); download them with `gh run download <run id>` (kept for 1 day only). Try them on real machines using the checklist at the end of [packaging-proposal.md](../architecture/packaging-proposal.md).
5. **Tag**:
   ```
   git tag -a v<version> -m "Vestra Player <version>" && git push origin v<version>
   ```
   `release.yml` checks that the tag matches the version and that the release notes exist, runs the full checks and packaging; the two TODO jobs `sign-*` must be approved in the `release` environment; finally it creates a **draft** release. The assets are `VestraPlayer-<version>-macos-arm64.dmg`, `…-macos-arm64.app.zip`, `…-windows-x64-setup.exe`, `…-windows-x64-portable.zip` and `SHA256SUMS`.
6. **Check the draft**, and when it is correct publish it on GitHub (or `gh release edit v<version> --draft=false`).

If something fails, fix it and push a tag for a new patch version; do not move a tag that has already been pushed.

## Installing an unsigned test build

- macOS (14 or later, arm64; the current dependency bundle requires 15 or later, see ci.md): open the DMG and drag Vestra Player into Applications. Gatekeeper blocks the first launch: go to System Settings → Privacy & Security and click "Open Anyway", or run `xattr -dr com.apple.quarantine "/Applications/Vestra Player.app"`. Files downloaded with `gh run download` / `gh release download` have no quarantine attribute and are not blocked.
- Windows 10/11 x64: run `…-setup.exe`; when SmartScreen warns, click "More info → Run anyway". It installs to `%LOCALAPPDATA%\Programs\Vestra Player` and needs no administrator rights; uninstalling keeps the plugins and settings in `%APPDATA%\Vestra` by default. You can also unzip the portable zip and run `vestra-host.exe` directly.

## Signing (later)

`sign-macos` and `sign-windows` in `release.yml` are placeholders for now. What is needed to add certificates is in §8 of the proposal; the bundle id is `com.vestraplayer.player` and cannot change after the first signed release.
