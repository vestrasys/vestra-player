# CI (GitHub Actions)

For the design and trade-offs see [ci-research.md](ci-research.md) §10.1–10.5; this document only describes how it actually runs now. The check commands match [../../agentic-workflow.md](../../agentic-workflow.md) §6.1.

## Files

| File | Purpose |
| --- | --- |
| `.github/workflows/ci.yml` | Entry point: push to any branch other than main (tags excluded), manual trigger (`full`). Orchestration only. **No pull_request trigger**: a same-repository PR directly shows the checks of the push run (on the same head commit) |
| `.github/workflows/ci-macos.yml` | Entry point: the macOS build-test for PRs, run only on demand: a non-draft PR opened/reopened/ready_for_review, or the `ci:macos` label added (remove and add it again to rerun); not triggered when the PR does not change host paths |
| `.github/workflows/main.yml` | Entry point: push to main. The same plan as ci.yml; on host changes both Windows and macOS run (except for a system whose branch run already passed on the **same tree**, see below); docs-only changes run only plan and ci-ok; only this workflow (and nightly) writes the cargo cache |
| `.github/workflows/nightly.yml` | Daily at 01:17 UTC and manual: checks RustSec advisories every day; builds unsigned packages for both systems only if main had non-docs changes in the last 24 hours (using the cargo cache; packages are not published). Does not run build-test again when main.yml already passed on the tip (`main-green.sh`); otherwise runs it first |
| `.github/workflows/release.yml` | Tag `v*` and manual (`dry_run`, on by default): checks the tag against the version in `src/host/Cargo.toml`, the release notes and the dependency pins; skips static and build-test when main.yml already passed on this commit (or on an ancestor that differs from it only in docs), otherwise runs the same checks (without cache; `checks` forces them on a manual run); packaging never uses the cache and always builds from scratch; unsigned installers for both systems, `SHA256SUMS`; on a tag, creates a draft release in this repository. Signing is a TODO job (`environment: release`). See "Packaging" below and [doc/releases/README.md](../releases/README.md) |
| `.github/workflows/deps.yml` | Manual only: builds and pins the native dependencies (libmpv, FFmpeg) and publishes them to the pre-release `deps-<date>` in this repository. See "Packaging" below |
| `_static.yml` | Reusable: all static checks on ubuntu; the gate for Windows/macOS. The plugin tests also record the property values the plugins send to their Slint views (artifact `ui-record`, output `ui_record`; see "Recorded plugin props" below) |
| `_build-test.yml` | Reusable (`os`): clippy and test for the whole `src/host` workspace; Windows also runs rustdoc. `ui-record` also checks the recorded plugin props; `ui-only` runs only that check |
| `_package.yml` | Reusable (`os`, `artifact`): release build → `src/packaging/bundle-*.mjs` (including the license check) → upload installers → headless smoke test |
| `_audit.yml` | Reusable: `cargo deny check advisories` |
| `.github/actions/setup-rust` | Installs the toolchain from `rust-toolchain.toml`; `Swatinem/rust-cache`, saved only on main (and not by the `ui-only` job, which builds only part of the workspace) |
| `.github/actions/setup-node` | Node 26 + npm cache; `npm ci` in `src/` and in each plugin with a lockfile (workshop) |
| `.github/actions/setup-libmpv` | Uses the job token to download the dependency packages from this repository's `deps-*` release as listed in `src/packaging/deps.json`, verifies SHA-256, caches, extracts. Sets `MPV_LIBRARY_DIR` (read by rsmpv-sys's build.rs) and `VESTRA_DEPS_DIR` (read by the packaging scripts). On macOS also sets the rpath (test programs use it to find `@rpath/libmpv.2.dylib`); on Windows adds the DLL directory to PATH |
| `.github/scripts/plan.sh` | The plan of ci.yml and main.yml: decides what to run from the changed paths (see below) |
| `.github/scripts/ui-record-paths.mjs` | Makes the plugin paths in the recorded props relative to the checkout (ts job, before the upload) and absolute again in the checking job's checkout |
| `.github/scripts/main-green.sh` | release.yml, nightly.yml: whether main.yml has already passed on this commit. First waits for any unfinished main.yml run on this commit (release up to 30 minutes, nightly 20 minutes, both waiting on Linux), then looks at the most recent green main.yml: if it is this commit, or an ancestor with only `doc/` and `*.md` changes in between (a `*.md` under `src/plugins/` is not documentation: a built-in's README ships), the commit counts as checked |
| `rust-toolchain.toml`, `src/host/deny.toml`, `_typos.toml`, `.github/zizmor.yml` | Tool configuration |
| `.gitattributes` | LF for all checkouts (Windows converts to CRLF by default, which makes tests that compare source text fail) |
| `src/scripts/check-unsafe-policy.mjs` | The unsafe rule (see below) |

## Stages of ci.yml / main.yml

```
push(not main) / workflow_dispatch{full}     push(main)
│                                            │
plan [ubuntu]  changed paths → docs_only, host, lock, run_windows, run_macos
│
├─ static [ubuntu]  (skipped when docs_only)           ← gate
│    ├─ lint: typos · actionlint · zizmor · cargo fmt --check
│    │        cargo deny (licenses bans sources) · cargo shear · unsafe-policy
│    └─ ts:   npm run check · format:check · boundaries · portability --skip-host
│             licenses · test:sdk · plugins:build · plugin -- test <plugins> · sdk:sync-slint
│             (plugin tests with VESTRA_UI_RECORD → artifact ui-record)
│             git working tree unchanged
├─ audit [ubuntu]  (ci.yml only; lock: Cargo.lock / deny.toml changed)  non-blocking, failure only gives a warning
│
├─ windows [windows-2022, 2x]  needs static, if run_windows
│    clippy -D warnings · cargo test --workspace · recorded plugin props (if ui_record) · cargo doc -D warnings
├─ ui-props [windows-2022, 2x]  needs static, if ui_record and ui_props and not run_windows (only plugins changed)
│    recorded plugin props only (the host test program; no clippy, workspace tests, rustdoc or npm)
├─ macos   [macos-15, 10x]     needs static, if run_macos
│    clippy -D warnings · cargo test --workspace
│
└─ ci-ok  if: always(); fails if any needs is failure/cancelled (skipped counts as passed)

pull_request{opened, reopened, ready_for_review, labeled ci:macos}, non-draft, host paths changed
└─ ci-macos.yml: macos [macos-15]  clippy · cargo test (static and Windows are covered by the branch's push run)
```

**Where changes are counted from**: plan.sh uses the API (`actions: read`) to find the commit of this workflow's **most recent successful run** on this branch (it must be an ancestor of HEAD, not HEAD itself), and runs `git diff` from it to HEAD. When there is no such run: a branch counts from where it left main (`merge-base origin/main`), and main runs everything. So the changes of runs cancelled by cancel-in-progress, of queued main runs replaced by a newer push, and of failed runs are all checked again by the next run: a green ci-ok on the tip covers all changes since the previous green run.

- `host` = changes to `src/host/`, `src/runtime/`, `src/package*.json`, `rust-toolchain.toml`, `.gitattributes` or `.github/` (host tests start the Node plugin runner). The `paths` of ci-macos.yml match this. Changes only to `src/plugins/` do not count as host: plugin tests are in the static ts job.
- `run_windows` = `host` or `full`. `run_macos` = `full`, or `host` on main. Branch pushes do not run macOS: all branches are developed on a Mac, and `npm run ci:local` already runs macOS locally (including the GPU tests); Windows cannot run locally, so every host change on a branch runs it.
- Changes only to `doc/` or `*.md`: only plan and ci-ok run (main too). A `*.md` under `src/plugins/` does not count as docs (a built-in's README and CHANGELOG ship in its package): static runs, with that plugin's tests.
- `plugins` (which plugins' tests the ts job runs): `all` on a full run, or when any path outside `doc/`, `*.md`, `src/host/` and `src/plugins/<name>/` changed (the SDK version, `src/scripts/`, `.github/`, ...); otherwise the changed plugin directories (those with a `manifest.json`; a README or CHANGELOG counts), and `none` when there are none; a change to the host's Slint or value conversion also gives `all` (see "Recorded plugin props"). A plugin can only import its own files and the SDK (`npm run boundaries`), so changes to other plugins cannot affect it. When release calls `_static` it does not pass this, and the default is `all`. The tests of the 18 plugins take about 220 seconds in total; workshop alone takes 78 seconds.
- `ui_props` = some plugin's tests run (`plugins` is not `none`); on main it becomes false when a branch run on the same tree passed `windows / windows` or `ui-props / windows` (see below).
- **main reuses branch results**: when Windows / macOS needs to run on main, plan.sh first looks in the last 50 runs of ci.yml and ci-macos.yml for one whose `head_commit.tree_id` equals HEAD's tree and whose `windows / windows` (or `macos / macos`) job succeeded; if one exists, that system does not run again on main, and the log links to that run. When a PR that already contains the latest main is squash-merged, the main commit and the PR head have the same tree, so the inputs are identical. ci-macos.yml tests the PR's merge commit: when the head contains the base, its tree is the head's tree, and only in that case does main get the same tree as the head. **Rebase the PR onto the latest main before merging**, and main will mostly not compile again. static is not reused (Linux, cheap; the plugin test scope is computed from main's own previous green run).
- Each push has only one run (previously a branch with a PR ran once for the push and once for pull_request, and the PR run also ran macOS). The concurrency group of ci.yml is `ci-<ref>`; a new push cancels the old run.
- main.yml does not cancel a running main run (it has to write the cache).

**Branch protection needs only one required check: `ci-ok`.** Private repositories on the Free plan currently cannot enable branch protection (the API returns 403), so `ci-ok` is only for people to look at; the PR page shows the `ci-ok` of the push run.

## Mapping to §6.1

Every item of §6.1 is in CI, plus typos, cargo-deny, cargo-shear, unsafe-policy, actionlint and zizmor. Differences:

- In CI, `npm run portability` runs only the OS command scan (`--skip-host`). Its host part (`cargo check` for the Windows target on macOS) is replaced by the real compile, clippy and tests of the windows job.
- `cargo clippy` / `cargo test` use `--workspace`, so the three unsafe crates are also checked and tested.
- `cargo test` adds `--no-fail-fast`, so the remaining test programs still run after one fails.
- Tests that need a GPU (the two OpenGL tests in `unsafe/render`) are marked `#[ignore]`: the macOS VMs in CI cannot create an OpenGL 4.1 context. `ci:local` runs them on the local machine with `--include-ignored`.
- `cargo doc` runs only on Windows.
- `ci:local` runs the plugin tests with `VESTRA_UI_RECORD` and the host's ignored test `recorded_plugin_props_match_their_slint` with `--include-ignored`; CI does the same across two runners (see the next section).

## Recorded plugin props

The SDK's test host does not know the Slint declarations, so a value Slint refuses (a struct field the struct does not declare, a number where a string is declared) passes every plugin test and fails only in the player ("wrong type"; 0.5.0: the AI window did not open). The check needs the host (the Slint interpreter and the player's own value conversion), which builds only on Windows and macOS, while the plugin tests run on Linux:

1. ts job (`_static.yml`, Linux): `plugin -- test <plugins>` runs with `VESTRA_UI_RECORD`, so `src/scripts/ui-record.mjs` writes every property value sent to a window, panel or settings page as JSON lines. `ui-record-paths.mjs relative` makes the plugin paths relative to the checkout, and the directory is uploaded as the artifact `ui-record` (1 day; about 6 MB uncompressed for all 18 plugins, 3,900 values). Output `ui_record` is true when anything was recorded.
2. Windows (`_build-test.yml` with `ui-record`): after `cargo test --workspace` (the test program is already built), downloads the artifact, `ui-record-paths.mjs absolute`, then `cargo test --bin vestra-host -- --ignored --exact ui::ui_types::tests::recorded_plugin_props_match_their_slint`. It runs even when the tests failed.
3. When Windows does not run (only plugins changed), the separate `ui-props` job (`_build-test.yml` with `ui-only`) builds only the host's test program and runs that test. macOS is not used for this (10x, and it does not run on branch pushes).

**Scope**: the recording covers the plugins whose tests ran (`plugins`). A change to the host's Slint library (`src/host/ui/`, the `@vestra/*` imports), its value conversion (`src/host/src/ui/{view,ui_values,ui_types,ui_image,ui_markdown*,composition}.rs`, `src/host/src/i18n/`) or `src/host/Cargo.lock` (the Slint version) sets `plugins` to `all`, so every plugin is tested and recorded. Release passes `ui-record` to its Windows build-test (to macOS when only macOS runs); ci-macos.yml and nightly do not run the plugin tests and do not check.

## Unsafe rule

- `src/host/Cargo.toml` has `[workspace.lints.rust] unsafe_code = "forbid"`, and host uses `[lints] workspace = true`.
- The three crates in `src/host/unsafe/*` do not inherit it; each sets `unsafe_op_in_unsafe_fn = "deny"`, `clippy::undocumented_unsafe_blocks = "deny"` and `clippy::missing_safety_doc = "deny"`.
- `npm run unsafe-policy` (runs on any system, no compilation): checks the lint configuration of the workspace members; in all tracked `.rs` files (with comments and strings removed), `unsafe` or `allow(unsafe_code)` must not appear outside `src/host/unsafe/`, including platform code that is cfg'd out; every `unsafe {}` / `unsafe impl` in an unsafe crate must have a `// SAFETY:` above it. Adding an unsafe crate requires changing `UNSAFE_CRATES` in the script.

## Packaging

The scripts are all in `src/packaging/` (Node built-in modules, no npm dependencies). `_package.yml` calls them, and they also run locally. For the design see [packaging-proposal.md](../architecture/packaging-proposal.md), for the decisions see [2026-10-s7-packaging.md](rounds/2026-10-s7-packaging.md).

| File | Purpose |
| --- | --- |
| `node.json` | The bundled Node version and the SHA-256 of the archives for the two platforms, plus the URL and SHA-256 of the Node release keyring (`nodejs/release-keys` at a pinned commit) |
| `fetch-node.mjs` | Downloads Node, verifies the signature of `SHASUMS256.txt` with `gpgv`, checks that it matches `node.json`, then checks the archive; extracts only `node`/`node.exe` and `LICENSE`. `--pin <version>` upgrades Node: rewrites `node.json` after verifying the signature. The major version must equal the one in CI's `setup-node` (`npm run licenses` checks this) |
| `deps.json` | Pins for the dependency packages: `release` (for example `deps-20261022`), `assets` (file names and SHA-256), `upstream` (URLs and SHA-256 of the Windows upstream builds, read by `deps.yml`) |
| `collect-macos-deps.mjs`, `collect-windows-deps.mjs` | Used by `deps.yml`: collect the dependencies, change install names, write `components.json` (each library's version, license, source, files and license files; `min_macos` as well on macOS) |
| `bundle-macos.mjs` | Assembles `Vestra Player.app` (layout in the proposal §3.1): the main program, the dylib closure in `Frameworks/`, `MacOS/{node,ffmpeg,ffprobe}`, the icon, `Resources/{ui,runtime,plugins,licenses}`; changes the main program's reference to libmpv to `@rpath` and checks the closure; ad-hoc signs from the inside out (with `--identity`, uses the certificate, Hardened Runtime and `macos/*.entitlements`); license check; `hdiutil` creates a ULMO-format DMG; `--zip` also produces an `.app.zip` |
| `bundle-windows.mjs`, `windows/vestra.iss` | Stages the install directory (layout in the proposal §4.2), license check, portable zip, Inno Setup installer (per-user install, no administrator, AppId `com.vestraplayer.player`, keeps `%APPDATA%\Vestra` on uninstall by default) |
| `stage.mjs` | Shared by both systems: `ui`, `runtime` (`scripts/stage-runtime.mjs`), built-in plugins (`scripts/stage-plugins.mjs`), `licenses/` (`index.html` overall index, `LICENSE`, `rust-crates.html`, `third-party/…`) |
| `icon.mjs` | Generates icons from `src/assets/brand/logo.png`: on macOS an `.icns` with the logo on an 824 px dark rounded backplate, on Windows an `.ico` of the logo itself. Pure Node implementation, same output on every system |
| `about.toml`, `about.hbs` | cargo-about configuration and HTML template. `accepted` must equal `allow` in `src/host/deny.toml` |
| `third-party.toml` | Inventory of non-Cargo components: our own files (`[own]`), each component's files, licenses and sources; the libmpv/FFmpeg closure is expanded from the `components.json` in the package |
| `check-licenses.mjs` | `--static` (`npm run licenses`, the ts job of `_static`): about.toml matches deny.toml, repository files referenced in the inventory exist, plugins with npm dependencies have `licenses/`, plugins with `dependency-inventory.json` or `vendor/` have a matching `inventory`/`vendor` component in the inventory and every package in it has an allowed license expression, all license files exist, there are no unowned license files or vendor files, and the Node major version matches CI. For plugins that bundle dozens of npm packages, such as the workshop (Vestra Player AI), the inventory only says `inventory = "dependency-inventory.json"` and `vendor = "vendor"`; the plugin generates the package list itself (`vendor-pi.mjs`, `vendor-sdk.mjs`), so refreshing vendor does not require changing `third-party.toml`. `--bundle <root> --platform macos\|windows` (the last step of the packaging scripts): every file in the package belongs to us or to some component, every license file exists, every license expression is in `[policy].allowed`, plugin packages carry the licenses of the dependencies bundled into them (`inventory`/`vendor` components are checked again package by package inside the `.vxp`), and `node --version` equals the pinned version |
| `smoke.mjs` | Starts headless with `--headless --seconds 8` with an empty data directory and no node in PATH, and requires every built-in plugin in `builtin.json` (16 since 2026-10-27) to reach `runtime.ready` (panel plugins that report `mounts require a native shell` in headless mode also count) |
| `macos/node.entitlements`, `macos/host.entitlements` | For signing later (Node needs JIT) |

**Dependency packages (deps.yml)**

- macOS: on `macos-15`, `brew install mpv ffmpeg`, collect the whole dylib closure starting from `libmpv.2.dylib`, `ffmpeg` and `ffprobe`, change install names to `@rpath/<name>`, ad-hoc sign, copy each formula's license files into `licenses/<formula>/`, and pack each formula's source archive (`brew fetch -s`) into a separate `-sources.tar`.
- **The minimum OS version is 15.0, not the 14.0 of decision 3**: Homebrew no longer builds bottles for macOS 14 (45 formulae including mpv and ffmpeg only have bottles for sequoia and later), so on `macos-14` the whole dependency tree (including deno, python, shaderc) would have to be compiled from source, which is not feasible. `bundle-macos.mjs` uses the larger of `min_macos` from `components.json` and 14.0 as `LSMinimumSystemVersion`. 14.0 is only possible by compiling mpv/FFmpeg ourselves (option B in the proposal §6).
- Windows: the shinchiro mpv-dev and BtbN `win64-gpl-shared` (FFmpeg n9.0) pinned in `upstream` of `deps.json`; verifies SHA-256, takes `libmpv-2.dll`, the headers, `ffmpeg/{ffmpeg,ffprobe}.exe` and the DLLs, takes the license files from the matching mpv commit, and generates `mpv.lib` with MSVC's `dumpbin`/`lib`. Also ships the Vulkan loader `vulkan-1.dll` (Apache-2.0) from MSYS2 UCRT64: `libmpv-2.dll` imports it at load time, and Windows only has this file when a graphics driver with Vulkan is installed (the CI runners do not have it); without it the player does not start at all. It sits in the install directory root together with `libmpv-2.dll`.
- The result is published to this repository's pre-release `deps-<UTC date>` (`GITHUB_TOKEN`, `contents: write`), with `SHA256SUMS`, the two `components-*.json` files and the new `deps.json`. Only **after committing this `deps.json` as `src/packaging/deps.json`** do CI and packaging use the new dependencies.
- Rebuilding only one system: `-f platforms=windows` (or `macos`); the other system's assets are copied from the release pinned in the current `deps.json` (with SHA-256 verification), which saves the macOS runner.
- Upgrading: change `upstream` in `deps.json` (Windows), or simply rerun (macOS takes the Homebrew of that moment), and commit the new `deps.json`. Do not delete old `deps-*` releases: builds of old versions depend on them to be reproducible.

```
gh workflow run deps.yml --repo vestrasys/vestra-player --ref main
```

**Local packaging (macOS)**

```
cd src/host && MPV_LIBRARY_DIR=<deps>/lib cargo build --release   # or build with Homebrew's libmpv
cd .. && VESTRA_DEPS_DIR=<deps> npm run package:macos -- --out <output directory> --zip
node packaging/smoke.mjs --exe "<output directory>/Vestra Player.app/Contents/MacOS/vestra-host" \
  --plugins "<output directory>/Vestra Player.app/Contents/Resources/plugins"
```

`<deps>` can be downloaded with `gh release download <deps tag> -p vestra-deps-macos-arm64.tar.xz` and extracted, or produced locally with `node packaging/collect-macos-deps.mjs --out <deps>` (uses the local Homebrew; the minimum OS version follows the local machine, so it is only suitable for self-testing). Requires `brew install cargo-about` (or skip it with `--rust-licenses <html>`). On macOS the Windows package can be staged and zipped with `--no-installer`; the installer can only be built on Windows.

**Artifact names**: `VestraPlayer-<version>-macos-arm64.dmg`, `…-macos-arm64.app.zip` (needed later for signing on Linux), `…-windows-x64-setup.exe`, `…-windows-x64-portable.zip`, `SHA256SUMS`. Releases also carry the installers under fixed names for `https://github.com/vestrasys/vestra-player/releases/latest/download/<name>`: `VestraPlayer-windows-x64-setup.exe`, `VestraPlayer-macos-arm64.dmg` (`VestraPlayer-linux-x86_64.AppImage` reserved), made by `.github/scripts/stable-names.sh` in release.yml's `SHA256SUMS` and `draft-release` jobs and listed in `SHA256SUMS` ([doc/releases/README.md](../releases/README.md#stable-download-links)). The portable zip uses `7z -mx=5`: for the same 445 MB of libraries, `-mx=9` takes 8 times as long (89 seconds vs 11 seconds locally; `-mx=9` takes 215 seconds on the Windows runner), and the zip is only 1.5% smaller. The artifacts of release and dry runs are named `vestra-unsigned-<os>`, nightly ones `vestra-nightly-<date>-<short hash>-<os>`; all are kept for only 1 day (the artifact storage quota is only 500 MB; things to keep long-term go into release assets), and nightly does not publish a release.

**exe icon and version info**: on the MSVC target, `src/host/build.rs` uses `src/host/windows/resources.rs` to write the `.res` directly (the icon `src/host/windows/vestra.ico` plus VERSIONINFO) and hands it to link.exe for embedding, so rc.exe is not needed; the output is byte-for-byte identical to what `llvm-rc` produces for the same `.rc`. `vestra.ico` is generated with `node packaging/icon.mjs --ico host/windows/vestra.ico` and committed, and `_static` uses `--check` to confirm it matches the logo: it compares each entry's directory fields and decoded pixels, not bytes (the zlib output of PNG entries differs between x64 Linux and arm64 macOS); the tests are in `src/packaging/tests/` (`npm run test:packaging`).

## Running everything manually (including macOS)

```
gh workflow run ci.yml --repo vestrasys/vestra-player --ref <branch> -f full=true
```

Or on the Actions page choose CI → Run workflow → check full. A manual run with `full=false` counts as "all changes" and runs static + windows.

## Local equivalent commands

In `src/`:

```
npm run ci:local                  # static + clippy/test/doc for the local system
npm run ci:local -- --static      # static checks only
npm run ci:local -- --advisories  # also check RustSec advisories
```

Requires `brew install typos-cli actionlint zizmor cargo-deny cargo-shear`. cargo output goes to `$CARGO_TARGET_DIR`, or to `vestra-target/ci` in the system temp directory when it is not set. Locally, `portability` runs in full. Homebrew's Rust does not read `rust-toolchain.toml`, so the local version applies.

**Windows (first set up 2026-10-25)**: rustup (MSVC) reads `rust-toolchain.toml`; VS 2022 Build Tools with the C++ workload; Node 26; Git for Windows. The tools at the versions `_static.yml` / `_package.yml` install (typos 1.51.1, actionlint 1.7.12, zizmor 1.31.1, cargo-deny 0.20.2, cargo-shear 1.14.0, cargo-about 0.9.2) are the `x86_64-pc-windows-msvc` / `windows_amd64` assets of each project's GitHub release, checked against the release's asset digest. The deps archive is what `setup-libmpv` downloads (`gh release download <release> -p vestra-deps-windows-x64.zip`, SHA-256 from `deps.json`); set `MPV_LIBRARY_DIR` and `VESTRA_DEPS_DIR` to the extracted directory and put it on `PATH`, as CI does. CI never runs the static ts job on Windows, so `ci:local` on Windows is the only place the plugin tests run there; the ignored real-machine host test `software_rendering_decodes_in_hardware` needs `ffmpeg` on `PATH` (the deps' `ffmpeg\` directory). Packaging without Inno Setup: `bundle-windows.mjs --no-installer`; without 7-Zip the portable zip is written with Windows' own `tar.exe` (bsdtar). `fetch-node.mjs` needs `gpgv` from `C:\Program Files\Git\usr\bin`: append that directory to the end of `PATH`, never the front (its `link.exe` would shadow MSVC's).

## Cost

Billing multipliers: Linux 1, Windows 2, macOS 10; each job is rounded up to whole minutes and then multiplied (plan, ci-ok and lint count as 1 minute each; the macOS build-test takes 1.1–1.7 minutes of wall time, rounded up to 2 minutes, which is 20). The quota of a private repository is counted in these billed minutes; public repositories using standard runners are not billed.

**Cache**: only main (and nightly) writes; branches and PRs read main's cache (restore hits normally). On the evening of 2026-10-24 the repository cache was 7.2 GB / 10 GB in total, 67 entries: cargo `build-test` Windows 0.5–0.8 GB, macOS 0.77 GB; a few entries for an old Cargo.lock (about 3 GB) are unused and are deleted automatically after 7 days; when 10 GB is reached, eviction is by least recent use, so these old ones go first and the ones in use are not pushed out. Nightly packaging has a separate `package` cache (about 1 GB per system).

**Cold / warm cache measurements (job wall time, minutes)**

| job | Warm cache | Cold cache (release, no cache) | Billed |
| --- | --- | --- | --- |
| windows build-test | 3.8–4.8 (restore 45 s, npm 25 s, clippy 30–45 s, test 40–85 s) | 8.7 (clippy 3.8, test 3.8) | 8–10 / 18 |
| macos build-test | 1.1–1.2 (restore 19 s, clippy 10 s, test 23 s) | 1.7 (clippy 35 s, test 41 s) | 20 / 20 (occasionally 2.2 → 30) |
| ts (static) | 4.5–5.5, of which plugin tests 215–220 s; recording and upload add a few seconds | — | 5–6 |
| ui-props (Windows, only plugins changed) | not measured yet; estimated 3: restore 45 s, libmpv, compile `vestra-host` tests (locally 46 s on arm64), the check (locally 13 s) | — | about 6 |
| package windows | — | 21.0: `cargo build --release` 11.0, `cargo about` 1.3, 7z `-mx=9` 3.6, Inno Setup 1.9, smoke 1.1 | 42 |
| package macos | — | 4.4: release build 1.8, packaging 1.7, smoke 0.4 | 50 |

The macOS cache has almost no effect on billing (2 minutes cold or warm); the Windows cache saves about 8 each time.

**2026-10-24 13:00–20:30 (after ci/trim, 75 runs): 957 billed minutes in total**: Linux 417, Windows 190, macOS 350. Runs after 20:20 could not get a runner (quota used up).

| workflow / event | Billed minutes | Mainly |
| --- | --- | --- |
| main / push (30 runs) | 330 | macOS 120 (6 runs), ts 93, Windows 52, plan + ci-ok 47. From 17:27 to 18:39 main stayed red (ts failing), and each of the 9 pushes reran all plugin tests counted from the previous green run |
| CI / push (33 runs) | 325 | ts 148, Windows 78 (9 runs), plan + ci-ok 66, lint 31 |
| CI macOS / pull_request (8 runs) | 160 | 20 each time a PR that changes host is opened |
| release / push v0.4.2 | 142 | Packaging 92 (Windows 42, macOS 50), duplicate static + build-test 44 (main had already passed the same commit) |

Before that (2026-10-22 08:40 to 10-24 12:50, 136 runs) it was about 2470, of which docs-only pushes to main were about 690 and duplicate pull_request runs 310; both were removed in ci/trim.

**Changes in this round (estimated by replaying the runs of the 7.5 hours above)**

| Change | Savings |
| --- | --- |
| Plugin tests run only for changed plugins (`plugins`) | 26 of 47 ts jobs did not need a full run, about -77 in total (ts drops from 5–6 to 1–3) |
| main reuses Windows / macOS results that passed on the same tree on a branch | 2 times in this period (-36); when every PR is rebased before merging, -20 to -28 per PR that changes host |
| release skips static + build-test that main already passed | -44 each time (v0.4.2: 142 → about 98); waiting for main to finish costs at most a few Linux minutes |
| Portable zip `-mx=5` | about -6 per Windows packaging |
| Total | about 957 → 800 for these 7.5 hours (-16%) |
| nightly: no build-test when main already passed the tip; no packaging on docs-only days; packaging uses the cache | about 136 → about 60 per day with changes (Windows packaging with a warm cache estimated at 21 → 12 minutes, not measured) |

**Estimate per push (warm cache)**

| Scenario | Billed minutes |
| --- | --- |
| Branch push, docs only | 2 |
| Branch push, one plugin only | 10–12 (workshop 12): 4–6 plus `ui-props` about 6 (none when the plugin's tests record nothing) |
| Branch push, host changed (no plugin changes) | 13–15 (plan 1, lint 1, ts 2, Windows 8–10, ci-ok 1); a change to the host's Slint or value conversion tests all plugins: ts 5–6, Windows +about 1 for the check, 17–20 |
| Branch push, SDK version or `.github/` changed (all plugins + Windows, with the recorded props check) | 19–20 |
| PR opened (host changed) | + macOS 20 |
| main push, docs only / plugins only | 2 / 4–6 when the branch already checked the same tree, otherwise 10–12 |
| main push, host changed, PR rebased before merging, Windows and macOS both passed on the branch | 5 |
| main push, host changed, no reusable results | 33–35 |
| tag release (main already passed) | about 98 |
| nightly (with changes, main already passed) | about 60 |

**What we now learn a bit later**: macOS compile / test errors on a branch (outside a local `ci:local`) only show up in CI after the PR is opened / the `ci:macos` label is added, or after merging into main; docs-only main pushes do not run typos (branch pushes do); release checks use the result of main's cached run (the cargo cache holds only dependencies, not the workspace's own crates; the package itself still does not use the cache, is built from scratch and smoke-tested); to rerun without the cache, run it manually with `checks=true`.

**Options that need a decision (not done)**

At the intensity above (a dozen or so PRs and dozens of pushes a day), even with the 16% saved, a busy working day costs 700–800 billed minutes, and the monthly quota of a private repository (Free 2000, Team 3000) is used up in a few days. Only the last three options solve this at the root.

| Option | Savings | Cost |
| --- | --- | --- |
| PR macOS runs only when the `ci:macos` label is added (no longer on opened) | -20 per PR that changes host (-160 in this period) | macOS problems show up in CI only after merging into main (branches already run `ci:local` on a Mac) |
| macOS on main only in nightly | another -20 per host push to main | macOS problems at most 24 hours late |
| nightly packaging once a week (RustSec still daily) | about -60/day → about -1500 per month | packaging / install / smoke problems found at most a week late; a manual dry run is possible before a release |
| Remove release | -98 each time | Not recommended: releases are infrequent, and this is the only packaging built from scratch without a cache |
| Self-hosted macOS runner (the user's Mac) | all macOS (350 in this period, 37%) becomes 0 billed | The Mac must be online, otherwise jobs queue forever; code from anyone who can push a branch to the private repository (including agents) runs on this Mac: needs a separate macOS user / VM (tart) that does not touch the keychain or personal files; must be removed before the repository goes public (fork PRs in a public repository can run code on a self-hosted runner); GitHub once announced a per-minute platform fee for self-hosted runners in private repositories; check the billing page for the current state |
| Self-hosted Windows | all Windows (190, 20%) | Needs an x64 Windows machine (a mini PC or a cloud VM, about 15–30 USD per month): VS 2022 Build Tools (MSVC, Windows SDK), Git for Windows (bash), rustup, Node 26, 7-Zip, Inno Setup 6, the runner service, a Defender exclusion for the work directory; same security issues as above. A Windows ARM VM on Apple silicon has to emulate x64, not recommended |
| Make the repository public | all standard runners free (branch protection also becomes available, and `ci-ok` becomes a real required check) | Before going public: a full-history secret scan (gitleaks); confirm that the internal discussions and round docs in `doc/` can be public; the GPL FFmpeg in the `deps-*` releases needs the corresponding source (macOS already has `-sources.tar`, Windows is a link to the upstream build); add a `pull_request` trigger for fork PRs (read-only token, no cache writes; plan.sh's `actions: read` works); protection rules for the `release` environment; remove self-hosted runners |
| Pay as you go (set a spending limit) | — | About 5–6 USD for these 7.5 hours at the actual minutes (Linux 417, Windows 95, macOS 35 minutes, estimated at GitHub's current unit prices; check the billing page), about 100–170 USD per month |

Also worth trying (to be measured when CI has quota): on Windows, put `target` and cargo home on a Dev Drive (ReFS) with a Defender exclusion, which usually makes Windows compiles 20–30% faster; getting the warm-cache macOS job under 60 seconds (now 66–72 seconds) would drop billing from 20 to 10, but that is too close to the boundary to be reliable. sccache does not help much: rust-cache is already a full match, and the remaining time is mostly the tests themselves and the cache download.
