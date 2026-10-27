# Portability round (top-down)

Branch: `rebuild/portability` (from `main` ca548d5). Basis: [community workflow](../../../agentic-workflow.md) §1.5 (cross-platform rule), §5 (top-down).

## 1. Goals

Make the plugin API independent of the operating system, gather the host's OS-specific code into platform modules with a Windows implementation that at least compiles and behaves reasonably, and stop plugins from calling macOS commands directly.

Not in this round: GPU rendering, picture-in-picture (prototypes in `rebuild/prototypes`, waiting for a later user ruling).

## 2. Known issues (checked by the coordinator 2026-10-21)

**Host**:
- `power.rs` starts `caffeinate` unconditionally.
- `clipboard.rs` and `confirm.rs` return "unsupported" on systems other than macOS.
- `platform_window.rs` and drag and drop (`drops.rs`) only have AppKit implementations.
- The handling of ⌘/⌃ and Option in `bindings.rs` has never been tested on Windows.
- Host dependencies (`objc2`, etc.) are already split by `cfg`, but have never been compiled on Windows.

**SDK and plugin runner**: `system.reveal`, `openPath`, `openUrl` hard-code `/usr/bin/open` (`sdk/plugin/local.ts`).

**Plugins**:
- capture: `sips` (JPEG/BMP conversion)
- playlist: `plutil` (`.webloc`)
- info: `sw_vers`, `sysctl`, `system_profiler` (system information)
- subtitle-online: `security` (keychain)

**Docs**: the requirements and reference documents contain many macOS-specific descriptions, such as Finder, keychain, the ⌘ key.

## 3. Phases

| Phase | Owner | Content | Done when |
| --- | --- | --- | --- |
| P0 | Maintainer | Temporary cross-platform check script `src/scripts/check-portability.mjs` (deleted once a proper CI exists):<br>· host `cargo check --target x86_64-pc-windows-msvc`, printing install hints when the target or toolchain is missing, and reporting which dependencies block;<br>· scan `src/plugins`, `src/sdk`, `src/runtime` for system command paths and macOS-specific calls | The script runs and lists all issues |
| P1 | Maintainer | Host platform modules: `src/host/src/platform/{mod,macos,windows,other}.rs`, covering sleep prevention, clipboard, confirmation dialogs, window control, drag and drop, open/reveal paths, open URLs.<br>Windows implementations use safe wrappers, e.g. the safe interfaces of the `windows` crate, `arboard`, `rfd`; where that cannot be done safely, create a separate subcrate under the rule "`unsafe` only lives in `src/host/unsafe/`" and explain it in the task sheet.<br>The SDK's `system.*` is implemented by the host instead, removing `/usr/bin/open`.<br>New core capabilities replace the system commands in plugins (decided conservatively per §4.3, with a reason for each):<br>· image format conversion, or `Snapshot.export` writing JPEG/BMP directly<br>· system information (`plugin.app` extension)<br>· secret storage (`plugin.secrets`)<br>· `.webloc` reading: suggest the plugin parses the plist itself in pure JS, not in core<br>The reference document states platform differences and when `unsupported` applies | All checks pass on macOS; the Windows target compiles; the portability check shows no host issues |
| P2 | Authors | Migrate plugins to the API changes, remove direct system command calls, use the new capabilities or cross-platform implementations | Each plugin's check, build and test pass; the portability scan reports zero |
| P3 | Coordinator | Real-machine regression on macOS: clipboard, confirmation dialogs, always on top, drag and drop, snapshot formats, system information, keychain and secret storage; then merge | — |

Running on a real Windows machine is not possible this round; we only guarantee that it compiles and that unit tests cover the platform-independent logic. Real-machine verification comes later.

## 4. Records

### 4.1 P0 delivery (maintainer, 2026-10-21)

`src/scripts/check-portability.mjs` (temporary, deleted once a proper CI exists), `npm run portability` under `src/` (`-- --skip-host` / `-- --skip-scan` runs only half). Exit code 0 means the host passed completely and the scan found no system commands.

- **Host**: `cargo check --locked --target x86_64-pc-windows-msvc --all-targets` (`--target-dir host/target/portability`, does not affect normal builds). With rustup, a missing target is added automatically with `rustup target add`; this machine has Homebrew Rust (no rustup), so the script uses the toolchain's own rust-src instead: `RUSTC_BOOTSTRAP=1 … -Zbuild-std=std,panic_abort`, check only, no linking. When neither is available it prints install hints, and the result is "NOT CHECKED", not a pass. When a dependency's build script or C code cannot be cross-checked, it lists the dependency name and reason, then checks what can be checked with `--no-default-features`; the result is marked "PARTIAL" and exits non-zero. It also lists the host's Windows warnings (by file) and `cfg(unix)` / `cfg(target_os)` / `std::os::` outside the platform modules (hints, not counted as failures).
- **Scan**: parses `.ts/.mjs/.js` etc. in `src/plugins`, `src/sdk`, `src/runtime` with TypeScript (skipping `tests`, `dist`, `vendor`, `node_modules`, `*.test.*`), and reports system program paths in string literals (`/usr/bin/`, `/usr/sbin/`, `/bin/`, `/opt/homebrew`, `/Library/`, etc.), `spawn`/`execFile`/`exec` calls whose first argument is a macOS-specific command name (`open`, `sips`, `plutil`, `security`…), and `process.platform` branches (hints, not counted as failures). Command names in comments do not count.

Initial results (`rebuild/portability` 2729deb):

Host:
- **The cross-check itself runs through**: the build scripts of rsmpv (libmpv), cpal, rfd and slint do not need the target platform's C libraries, and `cargo check` does not link, so no dependency blocks. Actually linking a Windows executable also needs the MSVC linker and a Windows libmpv; not done this round (end of §3).
- The only error: the test at `src/ui_image.rs:289` calls `std::os::unix::fs::symlink` unconditionally (only appears with `--all-targets`).
- 52 warnings on Windows, all code only used on Unix: `media_worker.rs` 17, `native_subtitle.rs` 10, `offline_descriptor.rs` 7, `producer.rs` 7, `producer_audio.rs` 6, `subtitle_backend.rs` 3, `offline_audio.rs` 1, `source_backend.rs` 1. The reason is that these features are `cfg(not(unix))` "unavailable" stubs on Windows: snapshot PNG export (`media_worker::encode`, which connects the lease file to ffmpeg's standard input via a Unix file descriptor), generated stream transport (`producer.rs`, Unix domain sockets), `fd://` playback of local media (`source_backend.rs`).
- 49 places of OS-specific code outside the platform modules: `clipboard.rs`, `confirm.rs`, `platform_window.rs` (`cfg(target_os = "macos")` and "unsupported" stubs), `main.rs:3434` (double-click interval), `media_worker.rs`, `producer.rs`, `source_backend.rs`, `source_lease.rs`, `package_stage.rs`, `font_carrier.rs`, `ui_image.rs` (`cfg(unix)`, `std::os::unix`).

Scan (10 commands, 3 platform branches):

| Location | Content |
| --- | --- |
| `sdk/plugin/local.ts:118`, `:125` | `/usr/bin/open` (`system.reveal` / `openPath` / `openUrl`) |
| `plugins/capture/sips.ts:13` | `/usr/bin/sips` (JPEG/BMP conversion) |
| `plugins/info/sysinfo.ts:24–28` | `/usr/bin/sw_vers` ×2, `/usr/sbin/sysctl` ×2, `/usr/sbin/system_profiler` (system information) |
| `plugins/playlist/drop.ts:51` | `/usr/bin/plutil` (binary `.webloc`) |
| `plugins/subtitle-online/secrets.ts:9` | `/usr/bin/security` (keychain) |
| `plugins/subtitle-online/secrets.ts:28` | Platform branch `process.platform === "darwin"` |
| `plugins/youtube/service.ts:38`, `:41` | Platform branch `process.platform !== 'win32'` (process group; already has a Windows branch, can stay) |

The SDK has one more problem the scan cannot find: `locate` in `local.ts` builds `ffmpeg` from `PATH`, but on Windows the executable is called `ffmpeg.exe`, so it is not found (fixed together in P1).

### 4.2 P1 delivery (maintainer, 2026-10-21)

**Host platform modules** `src/host/src/platform/`:

- `mod.rs`: shared types (window ids, screen geometry, `OsFacts`, `Display`), choosing the implementation by `cfg(target_os)`, the list of functions every implementation must provide (the table in the module docs), and the keyring secret storage shared by macOS and Windows.
- `macos.rs`: the AppKit window code from the former `platform_window.rs` (moved as-is), the NSPasteboard and NSAlert code from the former `clipboard.rs` / `confirm.rs`, `caffeinate` (former `power.rs`), `/usr/bin/open` (former SDK `local.ts`, the 10-second timeout and the error semantics of the last stderr line unchanged), keychain, NSProcessInfo + `sysctl -n hw.model`, NSScreen displays, data directory, Hiragino font, default FFmpeg path, double-click interval, Slint's ⌘/⌃ swap.
- `windows.rs`: windows via winit (Slint's backend): after every `show()` the host calls `platform::shown(window)` to register it (winit has no "all windows of this application"), always on top is `WindowLevel::AlwaysOnTop` (windows opened afterwards are also on top), `resize_content` / `screen_for` use winit's inner/outer sizes and monitors; clipboard via arboard; confirmation dialogs via rfd message boxes; `system.*` via opener; secrets via keyring (Credential Manager); system information from the registry (windows-registry); data directory `%APPDATA%\Vestra`; font Microsoft YaHei; FFmpeg found as `ffmpeg.exe` on `PATH`.
- `other.rs`: Linux etc., everything `unsupported` (window observation is `Unavailable`), data directory `$XDG_DATA_HOME/vestra`.
- The former `cfg(target_os)` in shared code (`clipboard.rs`, `confirm.rs`, double-click interval and modifiers in `main.rs`, five places in `bindings.rs`), the hard-coded `~/Library/Application Support` (`plugins.rs`, `package_manager.rs`, `ui_state.rs`), the Hiragino path (`ui.rs`) and `/opt/homebrew/...ffmpeg` (`media_jobs.rs`) now all call the platform module. `platform_window.rs` is deleted.
- `power.rs` keeps only the OS-independent merging logic (`Hold { display }`); the startup assertion is provided by `platform::sleep_launcher()`.

**`unsafe` subcrate** `src/host/unsafe/win32` (`vestra-win32`, compiled only on Windows): `GetCursorPos` (pointer position during drag and drop; winit 0.30 does not report the drag position), `GetKeyState` (modifiers held during drag and drop), `GetDoubleClickTime`, `SetThreadExecutionState` (sleep prevention). Reason: these four calls have no mature safe wrapper crate (`keepawake` would pull in a whole dependency for one call, `device_query` needs an extra permission model); each is a single by-value call or one local output parameter, every `unsafe` has a SAFETY comment, and the public API is safe. All other Windows features use mature safe crates: arboard 3.7, keyring 3.6 (`windows-native`; macOS uses `apple-native`), opener 0.9 (`reveal`), windows-registry 0.6, plus the existing rfd and winit. The main host crate is still `forbid(unsafe_code)`; `grep -rnw unsafe src/host/src` is empty.

**New requests** (`src/host/src/system.rs`; argument checks live here, system calls in the platform module; potentially blocking calls run on a worker thread, results come back as `Event::Reply`): `system.reveal` / `openPath` / `openUrl` (no capability needed), `secrets.get` / `set` / `delete` (new capability `secrets`, service name is the plugin manifest id), `app.system` (no capability needed). Snapshot export gets new job kinds `rawJpeg` / `processedJpeg` (`quality` 1–100, default 90), `rawBmp` / `processedBmp`: FFmpeg still writes PNG, which after validation is converted to JPEG or 24-bit BMP by the `image` crate and then published without overwriting (`media_worker::convert_still`; `image` gains the `bmp` feature, no new crate).

**SDK**: `system.*` now sends requests (paths are still made absolute and checked for existence in the plugin process); `plugin.secrets`; `plugin.app` has type `App` (`AppInfo` + `system()`, `system` is non-enumerable, so old `AppInfo` literals and `deepEqual` are not affected); `Snapshot.export` supports `jpeg` (`quality`) and `bmp` (new type `ExportOptions`); `local.ts` is the SDK's only platform file, `/usr/bin/open` removed, `tools` finds executables on Windows by `PATHEXT` (`executableNames`). `LocalServices.open` / `openUrl` become optional and are marked deprecated (see the §4.4 compatibility period). `TestHost`: `system.*` requests go through `host.local.open` / `openUrl` (old fakes still intercept them), new `host.secrets`, `host.systemInfo`, `host.exports`. The build tool and the host's capability table add `secrets`. workshop's `vendor/sdk` has been re-synced.

**Docs**: reference §1 preface (OS and the `unsupported` principle), §2 capability table, §3.4 Windows data directory, §3.5 `app.system()`, §4.1 Ctrl/Cmd on Windows, §6.6 always on top and visible area, §6.7 drag-and-drop modifiers, §7 Windows confirmation dialogs, §10 JPEG/BMP and export unavailable on Windows, §13 `unsupported`, §14 TestHost, platform differences in each §16 section, new §16.6 secrets, new §16.7 platform differences summary, §17 "Portability round P1" change notes.

**P0 script additions**: Windows warnings are deduplicated by location (previously the binary and the tests each reported once); hard-coded system paths outside the platform modules (`"/usr/`, `"/opt/`, `"/tmp`, etc., skipping comments) are also listed as hints.

**Portability check status** (`npm run portability`): host `OK: the host compiles for Windows` (no blocking dependencies); the scan has 8 commands left (all in plugins, handled in P2) and 4 `process.platform` (the two in youtube can stay, the one in subtitle-online is removed in P2, `sdk/plugin/local.ts` is the SDK platform file itself). The exit code is 1 until P2 is done; `-- --skip-scan` looks only at the host.

**Windows parts of the host still not done** (not in this round's P1 list; listed for the coordinator to decide when to do them):

| Location | Current state | Effect |
| --- | --- | --- |
| `media_worker.rs` `encode` (`cfg(unix)`) | Connects the held source file to FFmpeg as `/dev/fd/0`; on Windows an "unavailable" stub | On Windows snapshot and clip export end with `failed` (already stated in reference §10) |
| `producer.rs` (Unix domain sockets, `/tmp`) | Returns `unsupported` on Windows | On Windows `player.openStream` (generated streams such as nes, visualizer) is unavailable |
| `source_backend.rs` (`fd://`) | Opens by plain path on Windows | A local file replaced during playback is no longer protected by the lease |
| `source_lease.rs`, `package_stage.rs` (`dev`/`ino`) | On Windows only compares length and modification time / always equal | Weaker file identity check |
| `font_carrier.rs` (permission bits) | Skipped on Windows | None |
| rfd confirmation dialog button text | `common-controls-v6` not enabled (needs an application manifest, otherwise startup fails) | Windows shows the system's "确定" "取消" (OK, Cancel) |
| Runner path `Resources/runtime/runner.ts` | macOS app bundle layout | The layout must be decided when packaging for Windows |
| `other.rs` | Linux cross-check blocked by pkg-config in `yeslogic-fontconfig-sys`, not checked | None (no Linux this round) |

**Checks** (macOS, 2026-10-21, all actually run):

- `src/`: `npm run check`, `format:check`, `boundaries` pass; `test:sdk` 97 pass (5 new in `tests/sdk/portability.test.ts`); `plugins:build` passes; `npm run plugin -- test` passes for all 19 plugins (workshop 17 pass after re-syncing `vendor/sdk`).
- `src/host/`: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` (also with `--features pip`) pass; `cargo test --locked` 234 pass (new tests for platform, `system.rs`, JPEG/BMP conversion, job kinds, etc.).
- `npm run portability -- --skip-scan`: `OK: the host compiles for Windows`, exit code 0; `cargo clippy` was also run for the Windows target the same way, with no warnings in the platform modules and `vestra-win32`. The full `npm run portability` exits 1 because of the 8 commands left in plugins (P2).
- `grep -rnw unsafe src/host/src`: empty.
- Not run: running and linking on a real Windows machine (no Windows environment and MSVC linker); macOS GUI real-machine regression (coordinator P3, §4.5); Linux cross-check (blocked by fontconfig's pkg-config).

### 4.3 Capability request rulings (§4.3 process; this round is top-down, the requests come from the §3 task sheet, authors did not object)

| Request | Way without changing core, and its cost | Ruling | Reason |
| --- | --- | --- | --- |
| capture: JPEG/BMP snapshots (replacing `sips`) | Plugin encodes in pure JS (e.g. `jpeg-js` + a hand-written BMP): needs bundled npm dependencies, must first decode the PNG in JS, a 4K frame takes several hundred milliseconds to a second or two and uses plugin process memory; or call a different command on each OS | **Accepted after rewrite**: no general "image conversion" API; only add `jpeg` (`quality`) and `bmp` to `Snapshot.export` | Export formats are already the core's job (offline worker, validation, publish without overwriting); the host already links `image`'s JPEG codec and only adds the `bmp` feature; conversion finishes before publishing, so the plugin no longer needs a hidden temporary PNG and failure fallback. A general conversion would have only one user, so it does not go into core |
| info: system information (replacing `sw_vers`, `sysctl`, `system_profiler`) | Node `os` gives the CPU model (`cpus()[0].model`), core count and memory, but not the OS version (`os.release()` is the Darwin kernel version), build number or model, and nothing at all about displays; reading `/System/Library/CoreServices/SystemVersion.plist` is again a macOS-specific path | **Accepted after rewrite**: `plugin.app.system()` gives `os`, `name`, `version`, `build`, `model`, `arch`, `displays`; CPU and memory **refused**, use Node `os` | OS version and model need system interfaces (NSProcessInfo, registry), and displays are window-system information only the host has; diagnostics and issue-report plugins will all use it, so it is a general basic capability. Node already provides CPU and memory cross-platform; not duplicated |
| subtitle-online: secret storage (replacing `security`) | Plugin calls commands per OS (macOS `security`; on Windows only PowerShell or `cmdkey`, and the latter cannot read passwords back); or only store a 0600 file (Windows has no equivalent permission bits) | **Accepted**: `plugin.secrets` (new capability `secrets`) | Node has no system key store interface; any plugin with an account (online subtitles, bilibili login, etc.) needs it, and each plugin writing its own would be reinventing the wheel with inconsistent behavior. Isolated per plugin id; the macOS service name matches subtitle-online's existing keychain items, so secrets users already saved are not lost |
| playlist: reading binary `.webloc` (replacing `plutil`) | Plugin parses the bplist in pure JS: a `.webloc` holds only one `URL` string; parsing the bplist00 offset table and ASCII/UTF-16 string objects is about 60 lines, no dependencies | **Refused**, with an alternative (see §4.4 playlist) | Only one plugin needs it, the format is simple and stable, pure JS works cross-platform; not a core responsibility |
| SDK `system.*` (replacing `/usr/bin/open`) | Stay in the SDK and call `open` / `explorer` / `xdg-open` per OS: Windows command-line quoting rules break paths containing spaces or `&` | **Accepted** (part of the P1 task sheet) | Opening files and URLs should use the system shell interfaces (`ShellExecute`, `SHOpenFolderAndSelectItems`), placed in the host platform module with the other system code; the SDK no longer has OS branches |

### 4.4 Migration list for authors (P2)

After migrating, each plugin runs `npm run plugin -- check|build|test <name>`, and `npm run portability -- --skip-host` under `src/` reports zero.

- **capture** (`sips.ts`, `tasks.ts`, `tests/fakes.ts`, `README.md`)
  - Delete `sips.ts`. Export JPEG/BMP directly: `task.snapshot.export({ format: "jpeg", destination, sampleTime, quality: spec.quality ?? 90 })`, `{ format: "bmp", destination, sampleTime }`.
  - Delete `convert()` in `tasks.ts`, the hidden `.<name>.vestra-tmp.png` staging (`staging`) and the "无法转换为 JPEG，已保存为 PNG" ("Could not convert to JPEG, saved as PNG") fallback: the player only publishes the target file after a successful conversion; on failure the task is `failed`, handled the same as a PNG failure.
  - `watchHost` in `tests/fakes.ts` should no longer replace `host.local.open` (removed after the compatibility period); inject failures with `host.handle("system.openPath", …)` / `host.handle("system.reveal", …)` instead; change `fakes.converted` to assert `kind` (`rawJpeg`, `rawBmp`) and `quality` in `host.exports`.
  - Remove the description of `sips` from the README.
- **info** (`sysinfo.ts`, `properties.ts`, `dialog.ts`, tests, `README.md`)
  - Delete the three commands in `sysinfo.ts`. `const system = await plugin.app.system()`; the OS version line is made of `system.name`, `system.version`, `system.build` (e.g. "macOS 26.2（25C56）", Windows "Windows 11 23H2（22631.4460）"), model is `system.model`; CPU uses `os.cpus()[0]?.model` plus the core count, memory still uses `os`.
  - The display line uses `system.displays`: `name · width x height · (width/scale) x (height/scale) @ refreshRate Hz`; delete `parseDisplays` (the JSON from `system_profiler`).
  - Rename `SystemFacts.macos` to an OS-independent field (e.g. `os`), and change the label "macOS 版本" ("macOS version") to "系统版本" ("OS version"). Tests use `host.systemInfo` presets instead (can add a set with `os: "windows"`).
- **playlist** (`drop.ts`)
  - Delete `execFile("/usr/bin/plutil", …)`. Parse binary `.webloc` (starting with `bplist00`) in pure JS: read the 32-byte trailer at the end of the file (offset table entry size, object reference size, object count, top-level object, offset table position), find the top-level dictionary (`0xD?`) through the offset table, look among its keys for the ASCII string (`0x5?`) `URL`, and take the corresponding value string (`0x5?` ASCII or `0x6?` UTF-16BE; when the length is `0x?F` an integer object follows). The existing regex for XML `.webloc` is unchanged. Add a binary `.webloc` test sample (can be generated once on macOS with `plutil -convert binary1` and committed as test data).
- **subtitle-online** (`secrets.ts`, `main.ts`, `manifest.json`, tests)
  - Add the capability `"secrets"` to the manifest.
  - Change `Secrets` to use `plugin.secrets.get/set/delete` (`delete` for an empty value); delete `SECURITY`, `execFile`, the `process.platform === "darwin"` branch and `VESTRA_SUBTITLE_ONLINE_KEYCHAIN` (tests use `host.secrets`).
  - When `plugin.secrets` fails with `unsupported` / `unavailable`, keep the existing fallback: `secrets.json` (0600) in the data directory, or remembered only for the current run.
  - `location` text: based on `(await plugin.app.system()).os`, show "macOS 钥匙串" ("macOS Keychain") / "Windows 凭据管理器" ("Windows Credential Manager") / "仅本次运行" ("this run only"), etc. The service name is the manifest id `vestra.subtitle-online` and the account name is the secret name, the same as the keychain items written today, so secrets users already saved can be read directly (macOS asks for authorization once the first time the player reads them).
- **youtube**: the two `process.platform !== 'win32'` in `service.ts` are the cross-platform way to handle process groups; keep them, not an issue.
- **playback**: change the README description "with video `-d -i`, audio only `-i`", "caffeinate" to "the player holds a system assertion (reference §16.2)". No code change.
- **Other plugins**: usage of `system.reveal` / `openPath` / `openUrl`, `clipboard`, `dialogs.confirm`, `power`, `drops` is unchanged; no change needed.

**Compatibility period, removed by the maintainer after all of P2 is done**: `LocalServices.open` / `openUrl` (`sdk/wire.ts`) and the part of `TestHost` that forwards `system.*` to `host.local.open` / `openUrl` (`sdk/plugin/testing.ts`). Currently only capture's `tests/fakes.ts` uses them.

### 4.5 macOS regression points for the coordinator (P3)

The macOS code was moved into `platform/macos.rs` as-is, but the call paths changed; the following need real-machine confirmation:

1. **Clipboard**: after capture "Copy to clipboard" (PNG, JPEG), paste in Preview; copy text in info.
2. **Confirmation dialogs**: playlist "Clear" (with a "don't ask again" checkbox), plugins "Uninstall" (with "also delete data") — the sheet is attached to the tool window that started it, the checkbox result is correct; Esc cancels.
3. **Always on top**: after "Always on top" in the window plugin, the main window, floating sidebar and tool windows opened afterwards are all above other applications; restored after turning it off.
4. **Drag and drop**: drag media and subtitles into the video area (dispatched by extension); drag onto the sidebar playlist panel (`onDrop` coordinates land on the correct row); `modifiers` correct when dropping with ⌥/⇧/⌘ held.
5. **Video area size**: `setVideoArea` (the window plugin's 1x / 2x) keeps the top-left corner and stays within the visible area.
6. **Sleep prevention**: while playing video, `pmset -g assertions` shows `caffeinate` (`-d -i -w <pid>`); audio only has just `-i`; gone after pausing.
7. **Files and links**: capture "Open snapshot folder", playlist "Show in Finder" (several selected files in the same folder highlighted in one window), subtitle-online opening a URL — now executed by the player via `/usr/bin/open`; a non-existent path still reports `missing`.
8. **Double click and keys**: the single-click delay of `Mouse:LeftClick` still follows the system double-click interval; `Ctrl+…` bindings also trigger with ⌘; ⌘Q quits.
9. **Fonts and data directory**: Chinese in plugin windows is still Hiragino Sans GB; plugin data, the install library and UI state under `~/Library/Application Support/Vestra/` are read as before (path unchanged).
10. **After P2**: capture JPEG/BMP snapshots (BMP pixel-identical to PNG, JPEG quality setting takes effect, no overwrite when the target exists); the info system information page (version, build, model, displays); subtitle-online secrets (already saved secrets can be read, keychain authorization prompt the first time; after saving or clearing, the corresponding change is visible in "Keychain Access"). Development builds prompt for authorization again after every rebuild when reading the keychain; this is expected.

### 4.6 P4 delivery (maintainer, 2026-10-21)

Removed the §4.4 compatibility-period interfaces: `LocalServices.open` / `openUrl` (`sdk/wire.ts`, workshop `vendor/sdk` re-synced) and the part of `TestHost` that forwarded `system.*` through `host.local` (now recorded directly in `host.system`, failures injected with `host.handle`); grep confirms plugins and tests no longer reference them, reference §14/§17 updated; `src/sdk` no longer contains `/usr/bin/open`, `npm run portability` (including the host) is clean, and all src and host checks pass.

### 4.7 Coordinator acceptance (2026-10-21)

**All checks pass**: 19 plugin tests, test:sdk 97, cargo test 234. `npm run portability` including the Windows cross-compile check is clean.

**macOS real-machine regression passed**:
- System information page: OS version, model, CPU, memory and displays all correct.
- Snapshots: the core exports JPEG (40 KB) and BMP directly, with no leftover temporary files.
- "Open snapshot folder": opens in Finder through the host's `system.openPath`.

**Not verified this round**: running on a real Windows machine. There are also several known Windows gaps: snapshot export, generated streams, confirmation dialog button text; see §4.2.
