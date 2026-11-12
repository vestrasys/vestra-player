# Release notes and release steps

This directory holds the release notes for each version: `v<version>.md`, for example `v0.3.0.md`. `release.yml` uses the file as-is as the body of the GitHub release; **if the file does not exist when the tag is pushed, the release fails** (a dry run only gives a warning). [unreleased.md](unreleased.md) collects user-facing notes of versions without a release of their own, as material for the next one.

Installers are published on this repository's own GitHub Releases (coordinator decision 14). Until the repository is public there are only internal test builds; every release is a draft. The release title is "Vestra Player <version>", signed or not; the release notes say whether a build is unsigned and how to install it (see below).

## Release steps

1. **Check that dependencies are pinned**: `release` in `src/packaging/deps.json` points to an existing `deps-*` release; to upgrade libmpv/FFmpeg, run `deps.yml` first and commit the new `deps.json` ([ci.md "Packaging"](../process/ci.md#packaging)). Upgrade the Node patch version with `node src/packaging/fetch-node.mjs --pin <version>` (the major version stays the same).
2. **Bump the versions** (see "Release version and host version" below):
   - always the **release version**, `version` in `src/release.json`;
   - the **host version**, `version` in `src/host/Cargo.toml` (`cargo build` updates `Cargo.lock`), when the host input hash changed since the last release (host code, `Cargo.lock`, the toolchain, the deps archive, …; not `src/runtime/`, which is not in the binary): set it to the new release version. A release that changes only built-in plugins, the runtime, docs or other non-host files leaves it alone, and CI then reuses the last release's host binary instead of compiling it. `node src/packaging/host.mjs check-bump` says whether it has to go up: it fails when a published release shipped this host version built from other inputs (release.yml runs it too and stops the release);
   - built-in plugins that changed bump their own versions too, otherwise they are not updated on users' machines (see the notes in `src/scripts/stage-plugins.mjs`).

   A commit that changes only `src/release.json` and `doc/` runs no CI checks (it counts as documentation; the release job checks it).
3. **Strict SDK agreement**: the SDK's product data (built-in command and submenu tables, the Slint library copy, the test host's Chinese texts, palette colours) only warns on main between SDK releases; a release ships an SDK that matches it ([agentic-workflow.md §1.6](../../agentic-workflow/agentic-workflow.md) "Release cadence"). On the commit to be tagged, in `src/`:
   ```
   VESTRA_SDK_STRICT=1 npm run test:sdk && npm run sdk:sync-slint -- --strict
   ```
   (or `npm run ci:local -- --strict-sdk`). If it fails, cut the SDK sync release first (`node scripts/sync-sdk-slint.mjs --write <sdk-repo>` for the Slint copy, the tables in the SDK repository), upgrade `@vestra-player/sdk` here and merge that before tagging. `release.yml`'s `sdk-agreement` job runs the same check and packaging waits for it.
4. **Write the release notes** `doc/releases/v<version>.md`: user-facing, in English (the release page is for all users); new features, fixes, known issues; for unsigned test builds include the installation steps (see below).
5. **Dry run**: after merging to main,
   ```
   gh workflow run release.yml --repo vestrasys/vestra-player --ref main -f dry_run=true
   ```
   Runs the full checks and builds the installers for both systems, `host.json` and `SHA256SUMS`, without signing and without creating a release. The package jobs say in their "Reuse a released host" step whether they took the host of a published release or build it; `-f fresh_host=true` forces a build. To check only one system, add `-f platforms=windows` (or `macos`); this skips build-test and packaging for the other system (macOS runners are billed at 10x). Tag builds always do both. The outputs are in the run's artifacts (`vestra-unsigned-macos`, `vestra-unsigned-windows`, `vestra-unsigned-SHA256SUMS`); download them with `gh run download <run id>` (kept for 1 day only). Try them on real machines using the checklist at the end of [packaging-proposal.md](../architecture/packaging-proposal.md).
6. **Tag**:
   ```
   git tag -a v<version> -m "Vestra Player <version>" && git push origin v<version>
   ```
   `release.yml` checks that the tag matches the release version in `src/release.json`, that the release notes exist and that the release version is not lower than the host version, runs the full checks (skipped when main already passed the commit) and packaging (with the host reused when its inputs did not change); the two TODO jobs `sign-*` must be approved in the `release` environment; finally it creates a **draft** release. The assets are `VestraPlayer-<version>-macos-arm64.dmg`, `…-macos-arm64.app.zip`, `…-windows-x64-setup.exe`, `…-windows-x64-portable.zip`, the fixed-name copies of the installers (see "Stable download links" below), `host.json` (the host binary inside, see below), `SHA256SUMS` and `latest.json` (see "latest.json" below).
7. **Check the draft**, and when it is correct publish it on GitHub (or `gh release edit v<version> --draft=false`).

If something fails, fix it and push a tag for a new patch version; do not move a tag that has already been pushed.

## Release version and host version

The player has two versions (user decision 2026-11-03), so that releases which only change built-in plugins do not spend CI time compiling an unchanged host:

| | Where | Bumped | Used for |
| --- | --- | --- | --- |
| Release version | `src/release.json` | every release | the tag `v<version>`, `v<version>.md`, installer and archive names, `latest.json`, the installer's version, macOS `CFBundleShortVersionString` and `CFBundleVersion`, the version the player shows (it reads `<resources>/release.json`) and the update check compares |
| Host version | `src/host/Cargo.toml` | when the host input hash changed (not for `src/runtime/`, which is not in the binary or the hash); then set to the release version | what the binary embeds (`vestra-host.exe`'s file version), `host.json` |

The host version is never higher than the release version (the release job and the packaging scripts check it). A release that reuses a host keeps the older host version: 0.7.2 may ship host 0.7.1.

A backport release from an older branch (a host version below one already released) fails `check-bump`'s "lower than" check by design: host versions only go up, so a fix for an older line gets a host version above every released one.

Whether a release can reuse a host is not decided by the versions but by the **host input hash** over everything the binary is built from ([ci.md "Reusing the host binary"](../process/ci.md#packaging)): if any host input changed, the package job compiles. The host version must then go up as well: one host version names one binary, because what the player keys by it (the GPU crash guard is keyed by host version and driver) would otherwise carry over from the old binary to the new one. `check-bump` enforces it, in release.yml's `verify` job and before any reuse. Every release carries `host.json`, listed in `SHA256SUMS`: per platform the target, the host input hash, the host version, the commit the binary was built from, its SHA-256 inside the archive, and `reusedFrom`, the release that first shipped it. Only published releases are reused; a draft never is.

```
node src/packaging/host.mjs versions   # release=… host=…; fails unless release ≥ host
node src/packaging/host.mjs check-bump # fails when the host version must go up (against published releases)
node src/packaging/host.mjs hash       # this checkout's host input hash (Windows target on Windows)
```

## Stable download links

The download page links straight to the installers through GitHub's redirect to the latest release (user ruling 2026-11-01):

```
https://github.com/vestrasys/vestra-player/releases/latest/download/<fixed name>
```

So every release also carries byte-identical copies of the installers under fixed names without the version:

| Fixed name | Copy of |
| --- | --- |
| `VestraPlayer-windows-x64-setup.exe` | `VestraPlayer-<version>-windows-x64-setup.exe` |
| `VestraPlayer-macos-arm64.dmg` | `VestraPlayer-<version>-macos-arm64.dmg` |
| `VestraPlayer-linux-x86_64.AppImage` | reserved for the Linux package (not built yet) |

The portable zip and the `.app.zip` keep only their versioned names. The names come from one place, `.github/scripts/stable-names.sh` (`VestraPlayer-<version>-<platform><kind>` → `VestraPlayer-<platform><kind>`, with `<platform>` as the bundle scripts pass it to `artifactBase`): a new platform or architecture (for example `macos-x64`) gets its fixed name by producing its versioned installer, and the download page needs the new link. `release.yml` runs it in the `SHA256SUMS` job, so the copies are listed in `SHA256SUMS` (same hash as the versioned file, so a file fetched by its fixed name checks with `sha256sum -c --ignore-missing SHA256SUMS`), and again in `draft-release` on the files it uploads (the signed ones once signing exists), which must match `SHA256SUMS`. A dry run does not upload the copies as artifacts (the 500 MB artifact quota); its `vestra-unsigned-SHA256SUMS` artifact and the job log list them.

`releases/latest` is the newest published release that is neither a draft nor a pre-release, so the links only work once a release is published that way (and, for visitors without access, once the repository is public); until then they return 404.

## latest.json

Every release also carries `latest.json`, a small description of that release (user ruling 2026-11-01; the same idea as Tauri's `latest.json` and electron-updater's `latest.yml`), at the stable URL

```
https://github.com/vestrasys/vestra-player/releases/latest/download/latest.json
```

Consumers: the player's update check (the Info plugin's Check for Updates, shipped with 0.6.3: compares `version` with its own, links `notes`, downloads `file` from that release's own URL `releases/download/v<version>/<file>` and checks its `size` and `sha256`), and the download page, which reads it at build time for the version, links, sizes and minimum systems.

```json
{
  "schema": 1,
  "version": "0.6.2",
  "published": "2026-11-01T12:34:56Z",
  "notes": "https://github.com/vestrasys/vestra-player/releases/tag/v0.6.2",
  "downloads": {
    "macos-arm64": {
      "file": "VestraPlayer-macos-arm64.dmg",
      "size": 123456789,
      "sha256": "…64 hex digits…",
      "minimumOs": "15.0"
    },
    "windows-x64": {
      "file": "VestraPlayer-windows-x64-setup.exe",
      "size": 123456789,
      "sha256": "…64 hex digits…",
      "minimumOs": "10.0"
    }
  }
}
```

| Field | Meaning |
| --- | --- |
| `schema` | Format version, `1`. A change that old readers would misread (a field renamed, removed or with a new meaning) raises it; new optional fields do not. Readers ignore fields they do not know and treat an unknown `schema` as "no update information". |
| `version` | The release's version, the tag without `v` (`src/release.json`). |
| `published` | UTC time (`YYYY-MM-DDThh:mm:ssZ`) the `draft-release` job wrote the file, that is, when the release was built; not when the draft was published. |
| `notes` | The release page, `https://github.com/<repo>/releases/tag/v<version>`. |
| `downloads` | One entry per installer, keyed by the platform as in the artifact names (`windows-x64`, `macos-arm64`; `linux-x86_64` once it exists). |
| `downloads.*.file` | The installer's fixed name (see "Stable download links"): fetch `releases/latest/download/<file>`. |
| `downloads.*.size`, `.sha256` | Byte size and SHA-256 (lowercase hex) of that file as uploaded. |
| `downloads.*.minimumOs` | Lowest OS version the installer runs on, present only where the packaging defines it: Windows from `MinVersion` in `src/packaging/windows/vestra.iss` (`10.0`, the Windows NT version, so Windows 10), macOS from `LSMinimumSystemVersion` of the build's `Info.plist` (14.0 or the dependency bundle's higher minimum, read from the release's `.app.zip`). |

`.github/scripts/latest-json.sh` writes and checks it. `release.yml` runs it in the `SHA256SUMS` job, after the fixed-name copies (so dry runs exercise it; it is not uploaded there), and in `draft-release`, after the copies are made again and `SHA256SUMS` is checked: it writes `latest.json` from the files that are uploaded, then checks that it parses, that `schema` is 1, `version` is the tag's, `notes` is this release's page, every listed file exists with that size and SHA-256 and the expected `minimumOs`, and that both `windows-x64` and `macos-arm64` are listed, before `gh release create` uploads it with the rest.

`latest.json` is not listed in `SHA256SUMS`: it is metadata about the files and carries their SHA-256 itself, and both come from the same release over the same HTTPS connection, so a checksum of it next to it proves nothing a reader cannot already check. It is also written after `SHA256SUMS`, which keeps `SHA256SUMS` the same in the `SHA256SUMS` job and in `draft-release`. Authenticity comes later from signing (the installers' signatures), not from either file.

A pre-release still gets a `latest.json` describing itself (reachable as `releases/download/v<version>/latest.json`), but `releases/latest` never points at a pre-release or a draft, so the update check and the download page keep seeing the newest full release.

## Installing an unsigned test build

- macOS (14 or later, arm64; the current dependency bundle requires 15 or later, see ci.md): open the DMG and drag Vestra Player into Applications. Gatekeeper blocks the first launch: go to System Settings → Privacy & Security and click "Open Anyway", or run `xattr -dr com.apple.quarantine "/Applications/Vestra Player.app"`. Files downloaded with `gh run download` / `gh release download` have no quarantine attribute and are not blocked.
- Windows 10/11 x64: run `…-setup.exe`; when SmartScreen warns, click "More info → Run anyway". It installs to `%LOCALAPPDATA%\Programs\Vestra Player` and needs no administrator rights; uninstalling keeps the plugins and settings in `%APPDATA%\Vestra` by default. You can also unzip the portable zip and run `vestra-host.exe` directly.

## Signing (later)

`sign-macos` and `sign-windows` in `release.yml` are placeholders for now. What is needed to add certificates is in §8 of the proposal; the bundle id is `com.vestraplayer.player` and cannot change after the first signed release.
