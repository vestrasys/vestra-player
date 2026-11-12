# S7 packaging and release: maintainer proposal (step 7.1)

> Historical design record (S7 packaging round, 2026-10-22). Current behaviour: [release steps](../releases/README.md), the scripts in `src/packaging/`; §10 (host implementation: install layout, mpv, Windows, logs) is still the current packaging reference. Not maintained apart from §10.

Date 2026-10-22, baseline main 9027212. This is research and a proposal only; the repository was not changed. All experiments were done in the scratchpad: release build, a hand-assembled `.app`, ad-hoc signing, headless launch, DMG creation, a trial run of cargo-about. The large files have been deleted.

Conclusions first:

- Packaging tool: **our own scripts** (Node `.mjs`, in `src/scripts/package/`). Neither cargo-bundle nor cargo-packager.
- macOS first version **arm64 only**, `LSMinimumSystemVersion 14.0`, DMG with the system's own `hdiutil`.
- Windows uses **Inno Setup**, per-user install without administrator rights, plus a portable zip.
- libmpv and Node are both **pinned versions, checked by SHA-256, and mirrored to our own release repository**. No ad-hoc `brew install` in CI.
- The host gets a new platform-layer function that gives the "install layout" in one place. Packaged runs use only the bundled Node and do not search PATH.

One license question must be decided by the user (§5.4): the program links GPL components, so publishing binaries means also providing the corresponding source, which conflicts with "keep the source repository private".

---

## 1. Current state: how the host finds each file, and what breaks inside a `.app`

| Item | Current code | After packaging |
| --- | --- | --- |
| Node | `Command::new("node")`, searches PATH. 4 places in total: `plugins.rs:141` starts plugins, `package_manager.rs:171` runs `node --check` when validating packages, and 2 more in tests (`plugins.rs:314` etc.). The error message is `start Node plugin runtime (Node >=22.18)` | **Broken**. When launched from Finder, PATH is only `/usr/bin:/bin:/usr/sbin:/sbin`. Measured: launching the `.app` with `env -i PATH=/usr/bin:/bin`, all 18 built-in plugins `failed to start` |
| runner | `plugins.rs:125`: `exe/../../Resources/runtime/runner.ts`, otherwise `CARGO_MANIFEST_DIR/../runtime/runner.ts` (a repository path fixed at compile time) | Works on macOS. Does not work on Windows, because `exe.parent().parent()` is the parent of the install directory |
| Built-in plugins | `package_manager.rs:1185`: `$VESTRA_BUILTIN_PLUGINS`, otherwise `exe/../../Resources/plugins`, which must contain `builtin.json` | Works on macOS (measured: all 18 were installed into the install store). On Windows, same as above: does not work |
| UI (`host.slint`, `@vestra`) | `ui.rs:12`: `exe/../../Resources/ui`, otherwise `CARGO_MANIFEST_DIR/ui` | Works on macOS. Does not work on Windows |
| Subtitle font Noto Sans CJK SC | `font_asset.rs:14` compiles it into the binary with `include_bytes!` (16 MB); at run time `font_carrier.rs` writes it to a temporary directory | Works. Only the OFL license (`src/assets/fonts/noto-cjk/LICENSE`) has to be shipped with it |
| libmpv | Linked at compile time by rsmpv-sys according to `MPV_LIBRARY_DIR`. The macOS binary records the absolute path `/opt/homebrew/opt/mpv/lib/libmpv.2.dylib`. On Windows `libmpv-2.dll` sits next to the exe | **Broken** on macOS if not handled: dyld fails right away on machines without brew. The 50 dylibs must be collected into `Frameworks/` and changed to `@rpath` (measured to work, see §3.3) |
| FFmpeg command line (offline export, embedded subtitle extraction) | `media_jobs.rs:604`: `$VESTRA_FFMPEG`, otherwise `platform::default_ffmpeg()`. On macOS this default is hard-coded to `/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg`; on Windows it searches PATH for `ffmpeg.exe`. ffprobe is the one in the same directory as ffmpeg (`media_worker.rs:548`) | **Works only on development machines**. On user machines snapshot export, clip export and embedded subtitles will all be `unavailable` |
| ffmpeg/ffprobe in plugins | SDK `plugin.tools.run` (`dist/plugin/local.js:102`): `VESTRA_FFMPEG`/`VESTRA_FFPROBE`, otherwise PATH | Same as above. Affects playlist durations, info, thumbnails, visualizer, capture; menus will show "Needs ffmpeg" |
| Child processes started by the workshop plugin | `engine.mjs:84` uses `spawn(process.execPath, …)` | No change needed; it automatically uses the bundled Node |
| Data directory | `platform::app_data_dir()`: `~/Library/Application Support/Vestra` on macOS, `%APPDATA%\Vestra` on Windows. Contains `packages/`, `plugin-data/`, `ui-state.json` | Not affected. Measured: the install store and plugin data were written here |
| Logs | Plugin stderr is inherited directly from the host's stderr | When launched by double-click in Finder, stderr is discarded. On Windows, once it is a GUI subsystem program, there is no stderr either |
| Command-line arguments | `main.rs:3998`: unknown arguments `bail!` immediately | No problem now. But once file associations exist, the file paths passed by the system would be rejected here (see §3.2). 2026-10-25: positional arguments are now files to open (`launch.rs`) |
| Windows console | No `#![windows_subsystem = "windows"]`, and child processes are started without `CREATE_NO_WINDOW` | Double-clicking the exe opens an extra console window, and every Node and ffmpeg child process opens one more |

Things that only matter during development: the three `CARGO_MANIFEST_DIR` fallbacks; the Homebrew path hard-coded in `default_ffmpeg`; direct `Command::new("node")` in tests; the `VESTRA_*` environment variables (these stay, for debugging).

**Experimental evidence**: assembled a `.app` from the release binary, the Homebrew libmpv closure, Node 24.22.1 and the output of `stage-runtime`/`stage-plugins`, with ad-hoc signing.

- Launched with `PATH=/usr/bin:/bin`: all 18 plugins fail.
- Launched with only `Contents/MacOS` (which holds node) added to PATH: 15 plugins reach `runtime.ready`. The other 3 report `mounts require a native shell`, which panel plugins always report in headless mode.
- `DYLD_PRINT_LIBRARIES` shows: 50 libraries loaded from `Contents/Frameworks`, none from `/opt/homebrew`.
- Conclusion: the package layout itself is fine; the only thing missing is where the host looks for Node.

## 2. Bundled Node

### 2.1 Which version (for the coordinator to decide)

nodejs.org `schedule.json` and `dist/index.json`, queried 2026-10-22:

| | Current patch version | Active LTS | Enters maintenance | End of support |
| --- | --- | --- | --- | --- |
| 24 Krypton | **24.22.1** (2026-10-20, security update) | 2025-10-28 | **2026-10-20 (already entered)** | 2028-04-30 |
| 26 | 26.12.1 (2026-10-21, security update) | **2026-10-28 (in 6 days)** | 2027-10-20 | 2029-04-30 |

- The ecosystem plan says Node 24. But CI (`setup-node` with `node-version: "26"`) and the local machine (v26.12.0) both run 26. In other words, **all tests have actually passed on 26, and 24 has never been tested once**.
- My recommendation is to switch to **Node 26 LTS**, for three reasons:
  - development, CI and the final product use the same version;
  - one more year of support;
  - during `vestra/1` the Node major version only goes up, never down; starting from 24 would force an upgrade before 2028-04.
- If 24 is kept: CI's `setup-node` must be changed to 24 as well, so that what is tested is what ships.
- Whichever is chosen, the rule is the same: CI, packaging and SDK types use the same major version. Patch versions are updated with player minor releases.
- Below, `<NODE>` stands for the chosen version; the examples use 24.22.1.

### 2.2 Where to download from and how to verify

- The source is the official `https://nodejs.org/dist/v<NODE>/`:
  - `node-v<NODE>-darwin-arm64.tar.xz`, 27.5 MB for 24.22.1, 30.7 MB for 26.12.1;
  - `node-v<NODE>-darwin-x64.tar.xz`, 29.3 MB;
  - `node-v<NODE>-win-x64.zip`, 37.7 MB;
  - there is also `win-arm64`, left for later.
- Verification has two layers:
  1. New `src/packaging/node.json` with `{ version, sha256: { "darwin-arm64": …, "win-x64": … } }`. After downloading, CI only compares against this SHA-256, the same as for mpv-dev in `setup-libmpv`.
  2. Upgrading Node uses the script `src/scripts/package/update-node.mjs <version>`: it downloads `SHASUMS256.txt` and `SHASUMS256.txt.sig`, verifies them with `gpgv --keyring pubring.kbx` (the keyring is `gpg-only-active-keys/pubring.kbx` from a pinned commit of the `nodejs/release-keys` repository), and on success writes the hashes into `node.json`. CI packaging repeats this step too; it is cheap, and serves as a second safeguard.
  - Measured: the 24.22.1 signature gives `Good signature from "Antoine du Hamel"`, and the SHA-256 of `darwin-arm64.tar.xz` matches SHASUMS.
- The package contains **only the `node` executable and its `LICENSE`**. No npm, npx, corepack, `include/` (64 MB) or `share/`.
- Size: the arm64 `node` is 122 MB unpacked (already stripped officially), depends only on system frameworks and `libc++`, `minos 13.5`.

### 2.3 Architectures and location

- macOS first version **arm64 only**.
  - x64 later as a separate DMG, not universal: universal means `lipo`-ing every dylib and Node into both architectures, nearly doubling the size, and Homebrew can only install one architecture on a runner.
  - Node's macOS x64 drops to Tier 2 from 26 on, supported until early 2028. Whether to ship x64 is for the coordinator to decide.
- Windows first version x64 only; arm64 later (shinchiro has an aarch64 mpv-dev, and GitHub has a `windows-11-arm` runner).
- File locations:
  - macOS: `Vestra Player.app/Contents/MacOS/node`. Apple's rule is that executable helpers go in `MacOS/` or `Helpers/`; putting them in `Resources/` causes problems with signing and notarization;
  - Windows: `<install directory>\node\node.exe`.

### 2.4 How the host resolves it: the platform layer

New `platform::install_layout(exe: &Path) -> Option<InstallLayout>`. One implementation per system, all pure functions, testable on any machine:

```text
InstallLayout { resources, node, ffmpeg: Option, ffprobe: Option }
macOS:   exe = …/X.app/Contents/MacOS/vestra-host, and Contents/Info.plist exists
         → resources = Contents/Resources, node = Contents/MacOS/node, ffmpeg/ffprobe = Contents/MacOS/ff*
Windows: the exe's directory contains resources\runtime\runner.ts
         → resources = <dir>\resources, node = <dir>\node\node.exe, ffmpeg = <dir>\ffmpeg\ff*.exe
Linux:   to be decided later (AppImage or /usr/lib/vestra)
```

Shared code gets a `runtime_paths.rs`, resolved once at startup into `RuntimePaths { node, runner, ui, builtin_plugins, ffmpeg, ffprobe, packaged: bool }`.

- **Packaged run** (`install_layout` returns a result):
  - Node is taken from `$VESTRA_NODE` (explicit, for debugging), then the bundled node. **PATH is not searched**; otherwise any Node version on the user's machine could end up being used, and the `vestra/1` promise about the Node version would not hold.
  - If the bundled node does not exist, a clear error is reported at startup (with a confirmation dialog when there is a UI, see `platform::confirm`). Do not wait for the 18 plugins to fail one by one.
- **Development run**: Node is taken from `$VESTRA_NODE`, then `node` on PATH, as today. The runner and UI use `CARGO_MANIFEST_DIR`.
- The 4 `Command::new("node")` calls, `ui_dir()`, `builtin directory()` and `default_ffmpeg()` all switch to `RuntimePaths`.
- When starting plugin processes in a packaged run:
  - remove `NODE_OPTIONS` and `NODE_PATH`, so things like `--require` from the user's environment do not leak into plugin processes. `NODE_EXTRA_CA_CERTS` is kept; corporate proxies need it;
  - if the user has not set `VESTRA_FFMPEG`/`VESTRA_FFPROBE`, set them to the two bundled binaries. The SDK already reads these two variables, so **the SDK does not change**.

### 2.5 Node launch arguments

No arguments.

- Neither 24.22.1 nor 26 prints an ExperimentalWarning any more when type-stripping `runner.ts` (measured: no warning in the stderr of 24.22.1), so `--disable-warning` is not needed.
- npm and corepack are not in the package, which amounts to disabling them.
- No `--no-addons`: plugins are trusted full Node processes, as already decided in ecosystem plan 1.6 item 4.

### 2.6 SDK

- SDK 1.1.1 currently says `engines.node: ">=22.18"` and `dependencies["@types/node"]: ">=22.18"`. The latter resolves to the newest `@types/node` (25, 26), so plugin authors get API types newer than the player's actual Node, which is a real trap.
- Change to: `engines.node: ">=<MAJOR>"`; `@types/node: "^<MAJOR>.0.0"`, that is, the types of the first bundled major version. The major version only goes up, so this lower bound always holds.
- On the player side: the devDependency `@types/node ^25` in `src/package.json` changes to the same major version. The Node ≥ 22.18 in the documentation (`reference.md:8`, `tutorial.md:317`, `store-design.md:203`, architecture README §2) changes to "the player bundles Node <MAJOR>".
- **An SDK release is needed, but it does not block packaging**. Suggest releasing 1.2.0: tightening a dependency range counts as a minor version and does not change `apiVersion`. Release it together with S7.

## 3. macOS

### 3.1 Package layout

```text
Vestra Player.app/Contents/
  Info.plist
  MacOS/vestra-host          main program (CFBundleExecutable)
  MacOS/node                 Node
  MacOS/ffmpeg, MacOS/ffprobe  share the same libav* as libmpv (about 0.7 MB extra)
  Frameworks/*.dylib         libmpv closure, all install names are @rpath/<name>
  Resources/AppIcon.icns
  Resources/ui/              src/host/ui
  Resources/runtime/         output of stage-runtime.mjs (plus a copy of the SDK's LICENSE)
  Resources/plugins/         output of stage-plugins.mjs (builtin.json and one unpacked directory per built-in; .vxp files until 2026-10-26)
  Resources/licenses/        see §5
```

Measured sizes:

| Part | Size |
| --- | --- |
| Main program | 47 MB (41.5 MB after strip; suggest adding `[profile.release] strip = "symbols"`) |
| Frameworks | 61 MB |
| Node | 122 MB |
| plugins | 4.4 MB |
| Whole `.app` | 231 MB |
| DMG (ULMO/lzma) | **76 MB** (UDZO is 107 MB) |

### 3.2 Info.plist (proposed)

| Key | Value |
| --- | --- |
| `CFBundleIdentifier` | **Set to `com.vestraplayer.player`** (final decision by the coordinator on 2026-10-22, replacing the proposal below and the old value of decision 15; the host code does not use the bundle id: the data directory is `Vestra`, the keychain service name is `vestra.plugin.<id>`). Original proposal: `com.vestrasys.player`. Need to confirm whether the domain vestrasys.com is ours; if not, use `io.github.vestrasys.player`. **It cannot be changed once a signed release is out** (keychain, TCC permissions and preferences all hang off it) |
| `CFBundleName`, `CFBundleDisplayName` | `Vestra Player` |
| `CFBundleExecutable` | `vestra-host` |
| `CFBundleIconFile` | `AppIcon` |
| `CFBundlePackageType` | `APPL` |
| `CFBundleShortVersionString` | The version from `src/host/Cargo.toml` (since 0.7.1 the player also has a release version, `src/release.json`, README §8; `Info.plist` is written at packaging time, not built into the binary, so it can carry the release version, as Finder shows this key; that is the packaging side's call) |
| `CFBundleVersion` | The same value |
| `LSMinimumSystemVersion` | `14.0` |
| `LSApplicationCategoryType` | `public.app-category.video` |
| `NSHighResolutionCapable` | `true` |
| `NSSupportsAutomaticGraphicsSwitching` | `true`. We use OpenGL; without this key, dual-GPU machines are forced onto the discrete GPU |
| `NSHumanReadableCopyright` | `© 2026 Vestra Player contributors · GPL-3.0-or-later` |

- **The minimum system version is the highest among the dependencies**:
  - Node requires 13.5;
  - Rust's default deployment target is 11.0;
  - picture-in-picture is checked at run time (`unsafe/pip` calls `isOperatingSystemAtLeastVersion`, and the menu appears only when supported), so it does not affect the minimum;
  - **the real constraint is the Homebrew bottles**: the `minos` of the local bottles is 26.0. A bottle's minimum version is the system it was built on; a `macos-15` runner installs 15.0.
  - So the libmpv dependency bundle must be produced on a `macos-14` runner, see §6. The minimum is then 14.0, matching the picture-in-picture requirement.
- **Document types**: the first version **declares none**, and Windows gets no file associations either. The reason is that the host cannot receive them yet:
  - opening files on macOS goes through an Apple Event (`application:openURLs:`), which winit/Slint does not forward; it has to be received in the platform layer;
  - the command line `bail!`s on unknown arguments.
  - Drag and drop already works. Once this is wired up, add `CFBundleDocumentTypes` (`LSHandlerRank = Alternate`, UTIs of common video and audio) as a separate item.
  - **Wired up on 2026-10-25**:
    - positional command-line arguments are files to open (`launch.rs`; `file://` is converted to a path, everything after `--` is a file), opened the same way as dropping the same files on the video area (`open_files.rs` → `drops.drop`); at startup they are delivered after the installed plugins have activated;
    - on macOS the platform layer receives the open-documents Apple Event (`unsafe/open`, using `NSAppleEventManager`, without touching winit's delegate); `Info.plist` declares `public.movie` and `public.audio`, and types the system does not declare (mkv, mka, ogv, etc.) are added with `UTImportedTypeDeclarations`, role Viewer, `LSHandlerRank = Alternate`, so we do not take over the default;
    - single instance (`platform/instance.rs`, the same code on every system): the first process locks `instance.lock` in the data directory and listens on a loopback port; later processes hand over their files and exit, and the running window comes to the front (on Windows `AllowSetForegroundWindow` first). Runs with `--plugin` or `--headless` do not take part unless `VESTRA_INSTANCE_DIR` is set;
    - the Windows installer always registers the ProgID, `OpenWithProgids` and `Applications\vestra-host.exe\SupportedTypes` (Vestra Player appears in "Open with"); the optional task "associate" (unchecked by default) also writes `Capabilities` and `RegisteredApplications`, and the user chooses in "Settings › Default apps". The list is in `packaging/file-types.mjs`, which generates `windows/file-types.iss`.
- **Icon**: the source is `src/assets/brand/logo.png` (1024×1024, with transparency), a full-bleed V without a plate.
  - Generating `.icns` with `sips` and `iconutil` has been measured to work (806 KB).
  - But since macOS 11 the icon guideline is an 824 px rounded-square plate plus content; macOS 26 puts icons that do not follow the guideline into a gray plate.
  - Option A (recommended): the packaging script composes a dark rounded plate with the V scaled to about 70% in the middle.
  - Option B: use it as is for now and wait for a design.
  - For the Windows `.ico` the V as is works fine.

### 3.3 libmpv dependency closure

- Measured: the closure of Homebrew mpv 0.41.0_10 has **50 dylibs, 64 MB**, including ffmpeg 9.0.2, libass, libplacebo, luajit, mujs, x264, x265, svt-av1, shaderc, vulkan-loader, openssl@4, etc.
- Approach (the experimental script is about 40 lines and worked on the first try):
  1. Starting from `vestra-host`, walk breadth-first with `otool -L` and copy everything except `/usr/lib` and `/System` into `Frameworks/`; `@rpath`/`@loader_path` are resolved relative to the directory where the referencing file originally was.
  2. Run `install_name_tool -id @rpath/<name>` on each dylib and `-change <old> @rpath/<name>` for each reference, and add the rpath `@loader_path`. The main program gets the rpath `@executable_path/../Frameworks`.
  3. `install_name_tool` invalidates the existing signatures, and unsigned code cannot run on arm64, so the last step **must be re-signing**: without a certificate use ad-hoc (`codesign -s -`), from the inside out: dylibs, ffmpeg/ffprobe, node (with entitlements), the main program, and finally the whole `.app`. Measured: `codesign --verify --deep --strict` passes.
- This step is done once, in the "generate dependency bundle" workflow (§6). Packaging receives a directory that already has `@rpath` applied and only needs to copy and sign.
- **No dylibbundler**: it adds a brew dependency, and it does not handle the ffmpeg/ffprobe we also want to collect.

### 3.4 Hardened Runtime and entitlements (prepared for later signing; the files are written now)

- `src/packaging/macos/node.entitlements`: `com.apple.security.cs.allow-jit`, `com.apple.security.cs.allow-unsigned-executable-memory`. V8 needs these two. The other three that the official node carries are not wanted: `disable-executable-page-protection`, `disable-library-validation` (only needed for native addons), `get-task-allow` (not accepted by notarization). Coordinator decision 2026-11-13: native Node addons (`.node`) are not supported on signed macOS builds (library validation stays on); store plugins cannot ship them anyway (the plugin repository's `check-package.mjs` rejects native code).
- `src/packaging/macos/host.entitlements`: empty for now. All dylibs are signed with our own Team ID, so `disable-library-validation` is not needed.
  - Risk: libmpv includes LuaJIT. The host has not turned off mpv's built-in Lua scripts (ytdl_hook, stats, console, etc., loaded by default); under Hardened Runtime LuaJIT cannot get MAP_JIT and may fall back to interpretation or may fail.
  - Suggest the host explicitly sets `ytdl=no`, `load-stats-overlay=no`, `load-console=no`, `load-auto-profiles=no`, `load-select=no`, `load-positioning=no`, `load-commands=no`, `load-scripts=no`. Network sources are resolved by plugins; the host does not need mpv to call yt-dlp itself. This needs the coordinator's confirmation: it changes behavior, since previously mpv might call yt-dlp itself when opening a URL.
  - If measurements show JIT is still needed, add `allow-jit` to the host.
- Unsigned test builds: from macOS 15 on, an unsigned app downloaded from a browser can no longer be opened with right-click "Open"; the user has to go to "System Settings → Privacy & Security → Open Anyway", or run `xattr -dr com.apple.quarantine`. Files downloaded with `gh run download` carry no quarantine attribute.

### 3.5 DMG

- `hdiutil create -volname "Vestra Player" -srcfolder <directory: the .app plus a symlink to /Applications> -format ULMO`, with no extra dependencies. ULMO is supported since 10.15; measured 76 MB.
- No background image and no window layout: that needs AppleScript controlling Finder, which is unreliable in CI.
- After signing, the DMG itself also needs `codesign`, notarization and stapling.

### 3.6 Tool choice

| Tool | Conclusion |
| --- | --- |
| **Our own scripts** (`src/scripts/package/macos.mjs`, using `install_name_tool`/`codesign`/`hdiutil`/`iconutil`, all part of the system) | **Recommended**. Reasons: the locations of the libmpv closure, Node and ffmpeg must be fully under our control; `stage-runtime`/`stage-plugins` are already mjs and can be reused directly; Windows uses the same staging logic; no new Rust build dependency, and nothing to pass through cargo-deny |
| cargo-bundle | Not recommended: not actively maintained, does not handle the dylib closure, and does not support extra executables such as Node |
| cargo-packager | Not recommended: it can produce `.app`/DMG/NSIS/WiX, but we would still have to compute the closure ourselves and feed it in; DMG depends on create-dmg plus AppleScript; and there would be an extra adaptation layer between its conventions and our layout |

## 4. Windows

### 4.1 Installer technology

| | Inno Setup | NSIS | WiX (MSI) | MSIX |
| --- | --- | --- | --- | --- |
| Per-user install without administrator | `PrivilegesRequired=lowest`, installs to `%LOCALAPPDATA%\Programs\Vestra Player`, simple | Possible, written by hand | Per-user MSI is awkward (`ALLUSERS=2`/`MSIINSTALLPERUSER`) | Per-user by nature |
| Installable without signing | Yes (with a SmartScreen warning) | Yes | Yes | **No**: even sideloading needs a trusted signature |
| Signing | `SignTool=` directive, signs both the installer and the uninstaller, can use Azure Trusted Signing | Possible | Possible | Required |
| Upgrades, later auto-update | Silent in-place install over the same AppId (`/VERYSILENT`); the restart manager closes the running program | Possible | Major upgrade rules are complex | Store or App Installer |
| Data cleanup on uninstall | `[UninstallDelete]` plus a `[Code]` prompt | By hand | Custom action | Virtualized AppData is deleted automatically (together with the user's data) |
| File associations | `[Registry]` writes HKCU\Software\Classes | By hand | Yes | Declared in the manifest |
| Available in CI | The windows-2022 image has **InnoSetup 6.7.1** preinstalled | 3.10 preinstalled | 3.14 preinstalled | SDK available |
| Other | Script language is Pascal | Low-level scripting | Suited to enterprise mass deployment by group policy | File system and registry virtualization affects child processes and the data directory |

**Inno Setup is recommended**, with its configuration in `src/packaging/windows/vestra.iss`.

- `PrivilegesRequired=lowest`, `PrivilegesRequiredOverridesAllowed=dialog` (those who need it can choose to install for all users).
- `ArchitecturesAllowed=x64compatible`, `MinVersion=10.0`.
- Create a "Vestra Player" shortcut in the Start menu.
- On uninstall ask "Also delete plugins and settings?", default no. The scope is `%APPDATA%\Vestra` and the entries in Credential Manager; the latter's names are recorded by the host's `secret_index`, and the uninstaller cannot remove them completely, which can be left for now.
- Also produce a **portable zip**, at almost no cost; the first version has this first.

### 4.2 Contents and runtime libraries

```text
<install directory>\vestra-host.exe   with VERSIONINFO (FileDescription = Vestra Player) and .ico
<install directory>\libmpv-2.dll      shinchiro build: compiled with MinGW, FFmpeg statically linked inside the DLL
<install directory>\node\node.exe     official build, static CRT
<install directory>\ffmpeg\ffmpeg.exe, ffprobe.exe   see below
<install directory>\resources\{ui,runtime,plugins}
<install directory>\licenses\…
```

- **VC runtime**:
  - `libmpv-2.dll` imports only system DLLs and `api-ms-win-crt-*`, that is, the UCRT, which ships with the system since Windows 10 (checked with strings);
  - node.exe uses the static CRT;
  - but Rust's MSVC target links `VCRUNTIME140.dll` dynamically by default, and Windows 10/11 is not guaranteed to have that file.
  - Suggest adding `-C target-feature=+crt-static` for `x86_64-pc-windows-msvc` in `src/host/.cargo/config.toml`, the same for all builds, so no vc_redist has to be shipped.
- **Host changes**:
  - add `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` for release builds;
  - when starting child processes such as Node and ffmpeg, call the platform function `platform::hide_console(&mut Command)`: on Windows it uses std's `CommandExt::creation_flags(CREATE_NO_WINDOW)`, with no unsafe; on other systems it does nothing;
  - stderr is redirected to a log file (§9).
- **ffmpeg/ffprobe**: shinchiro's ffmpeg package contains only `ffmpeg.exe` (103 MB), **no ffprobe**.
  - We could use BtbN FFmpeg-Builds `win64-gpl-shared` instead (with ffmpeg, ffprobe and av*.dll, about 35 MB compressed), also pinned and mirrored.
  - Or the first Windows version ships without it, and the related menus show "Needs ffmpeg".
  - For the coordinator to decide. I suggest shipping it: on macOS it adds almost no size, and Windows then has the same features as macOS.
- `libmpv-2.dll` is 121 MB because shinchiro's dev package is compiled with `-Ddebug=true`. `llvm-strip` can be tried later; not handled in the first version.
- Estimated installer size 90–120 MB compressed.

### 4.3 CI smoke test (real-machine Windows acceptance comes with S6)

1. `setup.exe /VERYSILENT /CURRENTUSER /DIR=<temporary directory>`;
2. run `vestra-host.exe --headless --silent --seconds 6` with node removed from PATH; all 18 plugins must reach ready or report a known headless error;
3. uninstall silently and confirm the install directory has been removed.

## 5. Third-party licenses

### 5.1 Rust: cargo-about

Tried cargo-about 0.9.2 in the scratchpad:

- Both `--target aarch64-apple-darwin` and `--target x86_64-pc-windows-msvc` exit 0 with `--fail`, with no unrecognized licenses.
- Slint resolves to `GPL-3.0-only`; the host's own crate is `GPL-3.0-or-later`.

The configuration goes in `src/host/about.toml`:

```toml
accepted = [ …exactly the same as deny.toml [licenses].allow… ]
targets = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
ignore-build-dependencies = true
ignore-dev-dependencies = true
```

- Note: `filter-noassertion = true` is a configuration error in 0.9; do not add it.
- The agreement between `accepted` and deny.toml is checked by a small script (part of `check-licenses.mjs`). If either list changes without the other, CI fails.
- Output: the packaging job of each platform generates `licenses/rust-crates.html` with `--target <this platform>` (template `src/host/about.hbs`, based on cargo-about's official HTML example).
- cargo-about is installed in CI with `taiki-e/install-action`, pinned by SHA.

### 5.2 Components not managed by Cargo: inventory

New `src/packaging/third-party.toml`, one entry per component: `name`, `version`, `spdx`, `files` (globs of paths in the package), `license_files` (paths in the package), `source` (source URL and SHA-256).

| Component | License | Evidence and notes |
| --- | --- | --- |
| Vestra Player itself | GPL-3.0-or-later | **The repository has no LICENSE file**; a full GPL-3.0 text must be added and placed at `licenses/LICENSE` in the package |
| libmpv (Homebrew on macOS, shinchiro on Windows) | GPL-2.0-or-later, because the build enables the GPL options; the combination falls under GPLv3 | shinchiro's `mpv.cmake` does not turn GPL off |
| FFmpeg 9.0.2 (Homebrew) / shinchiro's FFmpeg | **GPL-3.0-or-later**: `--enable-gpl --enable-version3`, **no `--enable-nonfree`, no fdk-aac** | `ffmpeg -buildconf`; shinchiro's `packages/ffmpeg.cmake` |
| x264, x265, rubberband | GPL-2.0-or-later | In the Homebrew closure; statically linked in shinchiro |
| lame, libbluray, etc. | LGPL-2.1-or-later | |
| OpenSSL 4, shaderc, vulkan-loader, etc. | Apache-2.0 | Compatible with GPLv3 |
| libass, harfbuzz, freetype, fribidi, dav1d, zimg, etc. | ISC, MIT, FTL/GPL-2.0, LGPL, BSD | All must be listed |
| Node.js | MIT. Its LICENSE file already contains the licenses of its bundled dependencies (V8, OpenSSL, ICU…) | Put into the package as is |
| Noto Sans CJK SC | OFL-1.1 | `src/assets/fonts/noto-cjk/LICENSE` |
| `@vestra-player/sdk` (in `Resources/runtime`) | Apache-2.0 | `stage-runtime.mjs` currently **does not copy the LICENSE**; must be added |
| Dependencies bundled by the workshop plugin | MIT, Apache-2.0 | Already has `licenses/` and `dependency-inventory.json`, put into `dist/licenses` of the `.vxp` |
| jsnes 2.1.0 bundled by the nes plugin | **Apache-2.0** | **`dist/main.js` has no license text and there is no `licenses/`**, which violates Apache-2.0 §4. The plugin author adds `licenses/jsnes/LICENSE` (and points `vestra.assets` at it); this is a plugin change |

The Homebrew part of the inventory is generated by the dependency bundle workflow (§6) from `brew info --json=v2` and written as `deps.json`, containing the formula, version, `license` field and source URL. The shinchiro part is written by hand from its `packages/*.cmake`, listing only the GPL and LGPL components.

### 5.3 Where they go in the package, and how CI checks them

- Location: macOS `Contents/Resources/licenses/`, Windows `<install directory>\licenses\`:
  - `index.html`: overall index;
  - `LICENSE`: the full GPL text of Vestra;
  - `rust-crates.html`;
  - `third-party/<component>/…`.
- Later, add a "Third-party licenses" link to the "About" window of the info plugin, opened with `open_path`.
- `src/scripts/package/check-licenses.mjs` checks the staged package:
  1. every file in the package must be covered by "our own files" or by the `files` of some inventory entry; no unknown files;
  2. every `license_files` of every inventory entry is really in the package;
  3. versions match: `node --version` equals `node.json`, and the dylibs and dlls match `deps.json`;
  4. for each `.vxp`: if its `package.json` has `dependencies`, its `dist` must contain a `licenses/` directory. By this rule the current nes fails;
  5. the license lists of about.toml and deny.toml match.
- The release, nightly and PR packaging jobs all run this check.

### 5.4 GPL risk (**for the user to decide**)

- No components incompatible with GPLv3 were found: no nonfree, no fdk-aac; OpenSSL is Apache-2.0 since 3.
- **But what is published is the binary of a GPL combined work**. Slint is used under the GPL-3.0-only option; FFmpeg/mpv use GPL builds, and the host depends on GPL parts of them, for example the `hqdn3d` filter, which exists only in GPL builds. Under GPLv3 §6, recipients of the binary must be able to get the **complete corresponding source, including the host source**. This conflicts with "keep the player source repository private". Options:
  - **(a) Recommended**: attach `vestra-player-<version>-src.tar.gz` (a `git archive` of the tag) to every release in the public release repository, plus the third-party source bundle (source tarballs of the Homebrew formulas, and the commits of mpv, FFmpeg and each library matching the shinchiro build). The source repository itself can stay private, but the source is in effect public to binary recipients.
  - (b) Switch to Slint's commercial or royalty-free license, and replace mpv and FFmpeg with LGPL builds. The cost is losing GPL filters such as `hqdn3d` and x264 encoding (used in `media_worker.rs:776`), and it contradicts the GPL-3.0-or-later already declared in Cargo. Not recommended.
- I am not a lawyer; the user makes the final decision.

## 6. Reproducible inputs

- Current situation:
  - macOS runs `brew install mpv` in CI, getting that day's bottle each time, so the version is not pinned; the minimum system version follows the runner; only arm64 can be installed;
  - Windows pins the shinchiro 20261021 build, but **shinchiro keeps only the latest 30 releases** (the earliest now is 20260919), so this pinned URL will 404 in about a month.
- Options:

| Option | Assessment |
| --- | --- |
| A. `brew install` in CI every time | Not reproducible, minimum system version follows the runner; ruled out |
| B. Build mpv, FFmpeg and dozens of dependencies ourselves | Full control, but high maintenance and builds take hours; consider later |
| C. Use someone else's builds: media-kit/libmpv-darwin-build | Downloaded and checked v0.7.3: mpv **0.36.0** (2023), FFmpeg 6, **LGPL build, no hqdn3d**, no libplacebo. Too old and not enough filters; ruled out |
| C'. MPVKit (xcframework, FFmpeg 9.0.2, has a GPL edition) | Aimed at iOS/SwiftPM; linking it into the host needs conversion. Fallback |
| C''. IINA's dependencies | Downloaded from IINA's own server with no version promise; ruled out |
| **D. Generate once, pin, mirror** | **Recommended** |

How option D works:

- New workflow `deps.yml`, manual trigger only.
- The macOS job runs on the **`macos-14`** runner:
  1. `brew install mpv ffmpeg`;
  2. collect the closure (libmpv, ffmpeg, ffprobe) as in §3.3 and change it to `@rpath`;
  3. generate `deps.json` (formula, version, license, source URL and sha256);
  4. get each formula's source with `brew fetch -s` and make a source bundle.
- Windows job: re-host shinchiro's mpv-dev and BtbN's ffmpeg unchanged.
- The artifacts are uploaded to a prerelease `deps-<date>` in `vestrasys/vestra-player-releases`.
- `src/packaging/deps.json` in the player repository records the URLs and SHA-256. `setup-libmpv` and the packaging jobs download only from there and verify.
- Upgrading means re-running `deps.yml` and committing the new hashes. Every build uses exactly the same bytes, and the GPL sources come along.
- Risk: GitHub's `macos-14` image will be retired at some point, around 2027. Then either switch to `macos-15` (the minimum becomes 15) or move to option B.

## 7. Integration into release.yml

```text
tag v* / dispatch(dry_run)
verify     version matches the tag; doc/releases/v<version>.md exists (source of the release notes); node.json, deps.json exist
static → windows / macos (the existing build-test, no cache)
package-macos   [macos-15]  download and verify node and deps → cargo build --release (MPV_LIBRARY_DIR = deps directory)
                → cargo about → package/macos.mjs (stage, assemble .app, ad-hoc sign, check-licenses)
                → smoke test (env -i, no node in PATH, --headless --seconds 6, check all plugins ready)
                → hdiutil ULMO → artifact vestra-unsigned-macos-arm64
package-windows [windows-2022]  same flow → iscc → setup.exe, portable.zip → silent install, smoke test, uninstall
sign-macos      TODO (environment: release)
sign-windows    TODO (environment: release)
source          git archive source bundle, plus links to or copies of the deps source bundles (§5.4)
publish         needs all of the above; environment: release; runs only on a tag and not in dry_run
                → get a token with the GitHub App → on vestra-player-releases gh release create v<version> --draft
                  --target main --notes-file doc/releases/v<version>.md, upload assets and SHA256SUMS
```

- **Cross-repository writes with minimal permissions**: **a GitHub App** is recommended.
  - The App is installed only on `vestra-player-releases`, with permission `contents: write`;
  - the App ID and private key live only in the `release` environment (tag deployments only, manual approval required);
  - the job uses `actions/create-github-app-token` (pinned SHA) to get a token valid for 1 hour and only for this one repository;
  - it does not depend on any person's account and never needs renewing.
  - Alternative: a fine-grained PAT, authorized only for Contents read/write on this one repository, valid for at most 1 year, also in the `release` environment. Simpler, but tied to a personal account and needs periodic renewal.
- The existing `draft-release` job uses `github.token` and creates the release in the private source repository; this must change.
- Asset names:

```text
VestraPlayer-<version>-macos-arm64.dmg
VestraPlayer-<version>-windows-x64-setup.exe
VestraPlayer-<version>-windows-x64-portable.zip
VestraPlayer-<version>-source.tar.gz            (GPL, §5.4)
VestraPlayer-<version>-third-party-sources.txt  (or the source bundle itself)
VestraPlayer-macos-arm64.dmg                    (fixed-name copies for releases/latest/download/,
VestraPlayer-windows-x64-setup.exe               see doc/releases/README.md "Stable download links")
SHA256SUMS
```

  Unsigned draft releases say "Unsigned test build" in the title.
- **nightly**: keeps using `_package.yml`, now producing a DMG and setup.exe, kept as Actions artifacts for 7 days and **not published to the public repository**. Named `vestra-nightly-<date>-<short hash>-<platform>`. The macOS packaging job is billed at 10×, so it runs only when main has new commits; the existing logic is unchanged.
- `_package.yml` gets an input `kind: release|nightly`; release does not use the cache. The smoke test and the license check run in both cases.

## 8. What to reserve for signing later

- **Reserved slots**:
  - `sign-macos`: run rcodesign on ubuntu (ci-research §10.2):
    - `rcodesign sign --code-signature-flags runtime`, from the inside out, with the entitlements of §3.4 and a timestamp;
    - `rcodesign notary-submit --staple`, for both the `.app` and the `.dmg`;
    - signing happens on Linux, so the packaging job must upload the "unwrapped .app" (zip), and the DMG is regenerated after signing. A replacement for `hdiutil` on Linux is possible too, but going back to macOS after signing to create the DMG is more reliable; decide when we get there.
  - `sign-windows`: Azure Trusted Signing, GitHub OIDC (`id-token: write`) plus `azure/trusted-signing-action`. First sign `vestra-host.exe`, `node.exe` (re-signing the officially signed node.exe is optional), `libmpv-2.dll`, ff*.exe, then compile the installer, finally sign setup.exe and the uninstaller (Inno's `SignTool=`), then `signtool verify /pa`.
  - Both signing jobs only read the packaging output, do not execute repository code, and the secrets live only in the `release` environment.
- **What the user needs to prepare**:
  1. Apple Developer Program membership (USD 99/year; an organization account needs a D-U-N-S number);
  2. a Developer ID Application certificate (.p12 and password); a Developer ID Installer certificate is optional, needed only for .pkg;
  3. an App Store Connect API Key (Issuer ID, Key ID, .p8), for notarization;
  4. the Team ID;
  5. an Azure subscription with Trusted Signing enabled (about USD 9.99/month), with identity verification completed (individual or organization; organizations must have existed for some time);
  6. a Code Signing Account and Certificate Profile on Azure;
  7. an App registration configured with GitHub OIDC federated credentials (tenant ID, client ID, subscription ID);
  8. the bundle id decided (§3.2); it cannot be changed after signing.

## 9. Implementation plan

| Step | Content | Owner |
| --- | --- | --- |
| 7.3a | Host: `platform::install_layout` and `runtime_paths.rs` (§2.4), replacing the 4 node lookups and the runner, ui, builtins and ffmpeg lookups; in packaged runs set `VESTRA_FFMPEG`/`VESTRA_FFPROBE` and remove `NODE_OPTIONS`/`NODE_PATH`; Windows: `windows_subsystem`, `hide_console`, `+crt-static`, icon and VERSIONINFO (add `embed-resource` in build.rs, must pass cargo-deny); logs written to `~/Library/Logs/Vestra/host.log` or `%LOCALAPPDATA%\Vestra\logs`, plugin stderr included, rotated by size; turn off mpv's built-in Lua scripts (if approved); `[profile.release] strip`. Unit tests: build both layouts in temporary directories and test the resolution results and the fallback order | maintainer |
| 7.3b | `src/packaging/`: `node.json`, `deps.json`, `third-party.toml`, Info.plist template, entitlements, `vestra.iss`, icon source; `src/scripts/package/{common,macos,windows,update-node,check-licenses}.mjs`; `stage-runtime` adds the SDK's LICENSE; add Vestra's LICENSE file | maintainer |
| 7.3c | `deps.yml`, first generation and mirroring of the dependencies (the release repository and GitHub App must exist first) | maintainer writes it; coordinator creates the repository and App and triggers the run |
| 7.3d | `_package.yml`, `release.yml`, `nightly.yml` and `setup-libmpv` download from the deps mirror; cargo-about; smoke tests; update `doc/process/ci.md` | maintainer |
| 7.3e | The nes plugin adds the jsnes license | nes plugin author (**the only plugin change**) |
| 7.3f | SDK 1.2.0: `engines` and `@types/node`; the player upgrades the dependency and aligns `@types/node`; CI setup-node aligned to the chosen major version; Node version in the documentation | maintainer changes, coordinator tags |
| 7.4 | Dry-run `release.yml` with dispatch, then push a test tag and look at the draft release | coordinator |
| 7.5 | Real-machine test (see below) | coordinator |

**Risks**

1. The minimum system version of the Homebrew bottles depends on the `macos-14` runner, which will be retired (§6).
2. The LuaJIT and V8 entitlements under Hardened Runtime can only really be verified once signing is in place; unsigned test builds cannot reveal problems.
3. No real Windows machine. `CREATE_NO_WINDOW`, the GUI subsystem and logging can only be verified by the CI smoke test and S6.
4. The obligation to provide GPL corresponding source (§5.4).
5. Package size: DMG 76 MB, Windows estimated 90–120 MB.
6. Built-in plugins are upgraded on user machines only when their version number increases (see the notes in `stage-plugins.mjs`); before each release, check that every changed plugin has bumped its version.
7. The dependency bundle contains some unused libraries (encoders such as x265 and svt-av1, shaderc, vulkan) that only take space; trimming them requires building ourselves.
8. App Translocation: when run directly from the DMG or the downloads folder, the whole bundle is moved to a random read-only path. We never write into the bundle and find resources relative to the exe, so this does not affect us.

**Coordinator real-machine test checklist (macOS `.app`, DMG from dry_run or nightly)**

1. Drag from the DMG into "Applications". If downloaded from a browser, first acknowledge the Gatekeeper prompt, then click "Open Anyway" in "Privacy & Security". Before **launching by double-click in Finder**, confirm that `which node` finds nothing, or temporarily rename Homebrew's node, so that there really is no node in PATH.
2. The Dock icon and the menu bar name both read Vestra Player; the version, build, mpv, ffmpeg and Slint versions in the "About" window are correct. Whether the host should also add the Node version to `components` is for the coordinator to decide; I suggest adding it.
3. All 18 built-in plugins are active on the "Plugins" page.
4. Local video: playback, seeking, volume, fullscreen, sidebar and bottom bar, picture-in-picture (macOS 14 and later).
5. Subtitles: external SRT/ASS and embedded subtitles. Embedded subtitles depend on the bundled ffmpeg. Chinese subtitles are shown with the Noto font.
6. Playlist durations (depend on ffprobe), snapshot export, clip export (depend on ffmpeg and x264).
7. NES: load a ROM; picture, sound and keyboard control work.
8. Store: install `zysilm.youtube` or `zysilm.bilibili` (third-party plugins hosted in the official repository) from the official repository; signature verification passes and the plugin works after installation; uninstall.
9. Workshop: just open it (it starts `process.execPath`, that is, the bundled Node).
10. Quit and relaunch: plugin state, sidebar and window position, and playback history are all still there. The data is in `~/Library/Application Support/Vestra/{packages,plugin-data,ui-state.json}`, and **no file inside the bundle has been changed** (`codesign --verify --deep --strict` still passes).
11. Check the plugin processes with `ps` and confirm they are `Vestra Player.app/Contents/MacOS/node`, not Homebrew's node.
12. The log file is created in the agreed location.
13. Delete the `.app` and install again; the data is still there.

---

## Items for the coordinator (or the user) to decide

1. **Node major version**: 26 LTS (recommended: same as development and CI, supported until 2029-04) or keep 24 (entered maintenance on 2026-10-20, ends 2028-04; if 24 is chosen, CI must be changed to test 24).
2. **GPL corresponding source** (§5.4, user decides): attach the source bundle and third-party sources to every release (recommended), or take another route.
3. **bundle id**: `com.vestrasys.player` (domain to be confirmed) or `io.github.vestrasys.player`.
4. **macOS architectures and minimum version**: first version arm64 only, minimum 14.0 (recommended); whether to ship Intel x64 later.
5. **Dependency source**: the "deps.yml generates once, mirrors to the release repository, pins hashes" approach (recommended), with the macOS dependencies generated on `macos-14`.
6. **Bundling ffmpeg/ffprobe**: macOS ships ffmpeg and ffprobe from Homebrew ffmpeg (recommended); Windows ships BtbN gpl-shared (about 35 MB extra), or the first version ships without.
7. **Windows installer**: Inno Setup per-user install plus a portable zip (recommended); user data is kept by default on uninstall.
8. **Credentials for the release repository**: GitHub App (recommended) or fine-grained PAT; the coordinator creates the `vestra-player-releases` repository and the App.
9. **Turn off mpv's built-in Lua scripts and ytdl** (§3.4): changes behavior, but avoids the Hardened Runtime problem and unexpected yt-dlp calls.
10. **Icon**: automatically composed rounded plate (recommended), or wait for a design.
11. **Source of release notes**: `doc/releases/v<version>.md` (recommended); the release fails if this file is missing.
12. **Document types and file associations**: not in the first version (recommended), scheduled separately later as a host feature. Done on 2026-10-25, see "Document types" in §3.2.
13. **SDK 1.2.0** (`engines`, `@types/node`): released together with S7.

---

## 10. Host implementation (7.3a, 2026-10-22)

Implemented as in §2.4, with two differences from the text above: `InstallLayout` gives each directory directly (not just `resources`), and ffmpeg/ffprobe are not `Option` (decision 5: bundled on both platforms).

### 10.1 Install layout (the packaging scripts place files according to this table)

`platform::install_layout(system, exe, is_file)` (`src/host/src/platform/layout.rs`), a pure function, with unit tests for all three layouts on any machine.

| | macOS `.app` | Windows install directory (same for installer and portable zip) | Development build |
| --- | --- | --- | --- |
| Detection | exe is `<X>.app/Contents/MacOS/<exe>`, and `Contents/Info.plist` exists | the exe's directory contains `resources\runtime\runner.js` (`runner.ts` until 2026-10-27) | neither of the above |
| UI | `Contents/Resources/ui/` (with `host.slint`) | `resources\ui\` | `src/host/ui` |
| Runtime | `Contents/Resources/runtime/runner.js` | `resources\runtime\runner.js` | `src/runtime/runner.ts` (the packaged runners are type-stripped `.js` since 2026-10-27, `stage-runtime.mjs`) |
| Built-in plugins | `Contents/Resources/plugins/builtin.json` | `resources\plugins\builtin.json` | none (`VESTRA_BUILTIN_PLUGINS`) |
| Node | `Contents/MacOS/node` | `node\node.exe` | `VESTRA_NODE`, otherwise `node` on PATH |
| FFmpeg | `Contents/MacOS/ffmpeg`, `Contents/MacOS/ffprobe` | `ffmpeg\ffmpeg.exe`, `ffmpeg\ffprobe.exe` | `VESTRA_FFMPEG`/`VESTRA_FFPROBE`, otherwise PATH |
| Licenses | `Contents/Resources/licenses/` | `licenses\` (not under `resources`) | none |

`runtime_paths.rs` resolves once, on first use. In packaged runs:

- Environment variables still take precedence (for debugging): `VESTRA_NODE`, `VESTRA_BUILTIN_PLUGINS`, `VESTRA_FFMPEG`, `VESTRA_FFPROBE`; an empty string counts as unset.
- **PATH is not searched for Node**. If any of Node, the runner, `ui/host.slint` or `plugins/builtin.json` is missing, startup reports an error and exits (with an error dialog listing the missing files when there is a UI), instead of letting the 18 plugins fail one by one.
- Plugin processes: `NODE_OPTIONS` and `NODE_PATH` are removed, `NODE_EXTRA_CA_CERTS` is kept; when the bundled ffmpeg/ffprobe exist they are set as `VESTRA_FFMPEG`/`VESTRA_FFPROBE`. No Node arguments (§2.5).
- The Node version in "About" is the info plugin process's own `process.versions.node`; plugin processes are started by the Node above, so it is the bundled one, and nothing needs to change.
- Development builds: the host's own ffmpeg lookup now searches PATH (previously the Homebrew `ffmpeg-full` was hard-coded; Homebrew's plain `ffmpeg` 9.0.2 also has x264 and hqdn3d). The plugin process environment is unchanged.

### 10.2 mpv (decision 7)

Before initialization, `backend.rs` sets `osc`, `ytdl`, `load-scripts`, `load-stats-overlay`, `load-console`, `load-auto-profiles`, `load-select`, `load-positioning`, `load-commands` and `load-context-menu` all to `no`; an option that an older libmpv does not have is skipped (it then does not have that script either). Neither the host nor the plugins use these scripts (no `script-binding`, `script-message` or ytdl-related code). A test confirms that `script-binding stats/…` fails once they are turned off (and succeeds when they are not).

### 10.3 Windows

- Release builds use the GUI subsystem (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`); debug builds keep the console.
- Child processes (plugin Node processes, ffmpeg/ffprobe run by the host itself) go through `platform::hide_console`: when the host's stderr is not a console (GUI subsystem, or redirected), `CREATE_NO_WINDOW` is added. The child gets a hidden console, and the processes it starts in turn (ffmpeg run by plugins, the workshop's Node) inherit this hidden console and do not pop up windows either. `plugin.tools.run` in SDK 1.2.0 additionally sets `windowsHide`.
- Static CRT linking: `src/host/.cargo/config.toml` adds `-C target-feature=+crt-static` for `cfg(all(target_os = "windows", target_env = "msvc"))`, so no VC++ runtime is needed. `libmpv-2.dll` (MinGW) uses the system UCRT, and neither side frees the other's memory (rsmpv uses `mpv_free`), so there is no conflict. The environment variable `RUSTFLAGS` overrides this setting, so neither CI nor packaging may set it; `platform/windows.rs` has a test that checks the build really uses the static CRT.
- The exe icon and VERSIONINFO are embedded: `build.rs` (`windows_resources`) writes a `.res` from `windows/vestra.ico` (generated by `src/packaging/icon.mjs`) and the crate version with `windows/resources.rs`, and link.exe embeds it; no `rc.exe` is needed.
- `[profile.release] strip = "symbols"`.

### 10.4 Where the logs are

- Host and plugin logs are both stderr (plugin processes inherit the host's stderr).
- macOS: when launched from Finder, stderr is discarded. To see the logs, run `"/Applications/Vestra Player.app/Contents/MacOS/vestra-host"` directly in a terminal.
- Windows release: the GUI subsystem has no console. To see the logs, run `vestra-host.exe 2> host.log` in cmd (a GUI program can use the redirected handle too, and plugin output goes into this file as well). Debug builds print directly to the console.
- The log file mentioned in §9 7.3a (`~/Library/Logs/Vestra`, `%LOCALAPPDATA%\Vestra\logs`, rotated by size) was first left out: redirecting the host's own stderr to a file needs `dup2`/`SetStdHandle` (unsafe), or replacing every `eprintln!` with a logging library. (2026-10-26: every host diagnostic now goes through `diag!` (`src/host/src/diag.rs`), which drops a failed write instead of panicking: a release host whose stderr is a pipe its launcher closed aborted with 0xc0000409 on the first `eprintln!`. `clippy.toml` disallows the std print macros in the host, and the plugin runner ignores a broken stderr too.) Since 0.6.3 it exists as one more writer beside `diag!`, in other folders than §9 named: see the next item.
- Since 0.6.3 a release host keeps a small rolling log of selected lines (start and exit, render mode and GPU, crash guard findings, plugin shader changes, restarts, `host.error` codes, a memory line every 60 s): `<app data>/logs/host.log` (Windows `%APPDATA%\Vestra\logs\host.log`, macOS `~/Library/Application Support/Vestra/logs/host.log`, Linux `~/.local/share/vestra/logs/host.log` or under `$XDG_DATA_HOME`), 512 KB then rotated to `host.log.1`; `VESTRA_LOG_FILE` overrides the path. It is beside `restart.log` (the same `logs` folder of the application data directory) on purpose, rather than `%LOCALAPPDATA%` or `~/Library/Logs`: the player's logs stay in one folder, under the directory that already holds its state (`render-state.json`, `render.json`). It is written by `logged!` beside `diag!`, not by redirecting stderr, so plugin output and other diagnostics stay on stderr only. Details: [README §4.3](README.md#43-render-mode-driver-denylist-and-crash-guard-0513).
