# Round 3a: input, window, plugin management, audio output, chapters and loops, secondary subtitles, confirm dialog + plugin integration

Branch: `rebuild/features-r3a` (created by the coordinator from the commit after round 2 was merged into `main`). Based on: the 3a schedule and the 3.3 shared capabilities in "Step 3: Rulings" of the [negotiation record](2026-10-features-negotiation.md) (below: "the rulings"), the [feature requirements](../../product/feature-requirements.md) (below: "the requirements document"), the [community workflow](../community-workflow.md) §3, §4, §5, and the items carried over from [round 2 task sheet](2026-10-features-r2.md) §6.2.

## 1. Scope

Only the items marked "3a" in the rulings, plus the items carried over from round 2 into this round. Do not start items from 3b or later early, and do not add interfaces outside the rulings. When a signature cannot be met, the maintainer records the reason and the adjusted signature in §6.2 of this document.

| Category | Items | Requirements |
| --- | --- | --- |
| Window | R-W1 `window.state` / `onChange`, R-W2 `setVideoArea`, R-W3 `setAlwaysOnTop`, R-W4 `setCursorAutoHide`; capability `window.control` | window H03–H05, B30 |
| App info | R-W6 `plugin.app` | info I09 (carried over from round 2), the mpv / FFmpeg version lines of I05 |
| Plugin management | R-G1 `packages.list` / `onChange`, R-G2 `setEnabled`, R-G3 `uninstall`; R-H10 persistent disable | plugins J07; info I09 |
| Bindings | R-K1 `plugin.bindings.*`; R-K3 `Mouse:LeftClick` + R-H11 single-click detection; capability `bindings.manage` | hotkeys J01–J03 |
| Audio output | R-A2 `audioDevices`, R-A3 `setAudioDevice`, R-A4 `PlayerState.audioOutput`, R-H9 generated streams follow the device | audio G16 (carried over from round 2) |
| Chapters and loops | R-M1 `player.chapters`, R-M2 `setLoop` / `PlayerState.loop`, R-H13 changing the source clears the loop | marks B21, B23 |
| Secondary subtitles | R-T3 `select(…, { role })`, `SubtitlesState.secondary`, `load({ select: "secondary" })`, the remaining R-T7 item `secondaryPosition` | subtitles A08 (carried over from round 2) |
| Shared capability (host) | `plugin.dialogs.confirm` (rulings 3.3) | playlist (replaces `confirm.ts`), plugins J07, hotkeys |

Already done in round 2 and not listed in this round: `Window.onClose` (round 2 fix commit).

Not in this round: picture-in-picture (R-W5, R-H8, 4a), the `pictureInPicture` field (4a), the timeline marker layer (R-M3, 3b), settings page / appearance / runtime images / drag and drop / background layer (3b), `UrlInputRow` (3b), C-SIZE, C-GPU.

## 2. Two tracks and order

| Phase | Done by | Content | Done when |
| --- | --- | --- | --- |
| P0 | Coordinator | Create the branch; record the baseline (§6.1) | Baseline written into §6 |
| P1 | Maintainer | All core items in §3 (all are compatible extensions, no switch needed) | All static checks pass; reference docs and TestHost in sync; coordinator review passed |
| P2 | Plugin authors | §4 assignments | `check/build/test` pass for each plugin |
| P3 | Coordinator | §5 acceptance, commit and push, merge into `main` | — |

P1 can be delivered in two batches (first window, app info, confirm dialog, chapters and loops; then bindings, plugin management, audio output, secondary subtitles). Authors start according to their dependencies (§4.2).

## 3. Maintainer checklist (P1)

Each item must include all of: host implementation (`src/host/`), SDK types and implementation (`src/sdk/`), TestHost simulation (`src/sdk/plugin/testing.ts`), reference docs (`doc/plugins/reference.md`), unit tests; new capability names are added to manifest validation in both places (host `plugins.rs` and the build tool `scripts/plugin.mjs`). First-party Rust must not use `unsafe`. Signatures follow rulings 3.1 / 3.3; below are only the implementation notes. Size budget: about 3 000 net new lines of host Rust (including tests); if exceeded, explain in §6.2 and split it out.

### 3.1 Window (R-W1–R-W4, capability `window.control`)

```ts
plugin.window.state: { fullscreen: boolean; videoArea: { width: number; height: number } };   // synchronous, read-only
plugin.window.onChange(listener: (state) => void): Disposable;
plugin.window.setVideoArea(size: { width: number; height: number }): Promise<{ width; height; clamped: boolean }>;
plugin.window.setAlwaysOnTop(onTop: boolean): Promise<void>;
plugin.window.setCursorAutoHide(mode: "fullscreen" | "playing"): Promise<void>;
```

- `state`: comes from the existing layout snapshot in `window_state.rs` (the video area is in logical points and excludes the bottom bar, sidebar and divider); pushed with the existing `window.state` event, only on change. Reading needs no capability; writing needs `window.control`.
- `setVideoArea`: the core adds the bars according to the layout rules and clamps to the visible screen area and the minimum size; `unavailable` in fullscreen; returns the actual video area and `clamped`.
- `setAlwaysOnTop`: the main window, the floating sidebar and tool windows opened later are all set to the floating level (`NSWindow` level, safe `objc2-app-kit` methods); with several plugins the last write wins; restored when that plugin is disabled.
- `setCursorAutoHide`: in the existing rule `cursor-hidden = fullscreen && idle && playing`, the `"playing"` mode relaxes `fullscreen` to "any window mode"; the idle timer and occlusion rules are unchanged; restored to `"fullscreen"` on disable.
- TestHost: `host.mainWindow: { fullscreen, videoArea, onTop, cursorAutoHide }`, and `host.setMainWindow(patch)` simulates the user changing the window (the name does not clash with `host.window(id)` for tool windows).

### 3.2 App info (R-W6)

```ts
plugin.app: { readonly version: string; build: string | null;
              components: { mpv: string | null; libav: Record<string, string>; slint: string } };
```

- Synchronous and read-only, no capability needed. The host passes it with the handshake when it starts a plugin (environment variable or `createPlugin` parameter, no RPC): `version` = `CARGO_PKG_VERSION`; `build` = the short git hash at build time (`null` when the build script cannot get it); `mpv` = mpv `mpv-version`; `libav` = `ffmpeg-version` and the versions of each `libav*` library (as far as mpv reports them); `slint` = the linked Slint version.
- TestHost: `host.app` can be preset.

### 3.3 Plugin management (R-G1–R-G3, R-H10)

```ts
interface PluginInfo { id: string; version: string | null; origin: "installed" | "development" | "trial";
  state: "starting" | "running" | "disabled" | "failed" | "stopped"; error: string | null;
  capabilities: string[]; directory: string; previousVersion: string | null }
packages.list(): Promise<PluginInfo[]>;                          // no capability needed
packages.onChange(listener: (plugins: PluginInfo[]) => void): Disposable;   // no capability needed
packages.setEnabled(id: string, enabled: boolean): Promise<void>;           // plugins.manage
packages.uninstall(id: string, options?: { removeData?: boolean }): Promise<PackageChange>; // plugins.manage
```

- `list` merges the running table, the install store (current / previous in `package_store.rs`) and the disabled set; the data directory rule is written into the reference docs (`<data root>/<id>`), not given as a field.
- `setEnabled`: disable = the existing `disable(owner)`; enable = reload the package; disabling itself is `invalid`; trial loads are `unsupported`; development loads only apply for the current run; the disabled set for installed plugins is written into the install store catalog (same transaction boundary as the install records) and skipped at startup.
- `uninstall`: installed plugins only; disable first, then write a removal record in the store (log-style commit + the existing `gc`); with `removeData` the core deletes `plugin-data/<id>/`; the plugin's bindings in the overlay stay as orphans. Reuses the existing `PackageChange` handle semantics.
- `onChange`: pushes the new list on load, disable, failure, install, uninstall (new event `packages.changed`).
- TestHost: `host.packages` (list can be preset), records `setEnabled` / `uninstall` calls.

### 3.4 Bindings (R-K1, R-K3, R-H11, capability `bindings.manage`)

```ts
interface BindingRef { plugin: string /* manifest id; "host" for the host */; command: string }
interface BindingInfo extends BindingRef {
  title: string; path: string[] | null; scope: "global" | "panel";
  defaults: string[]; current: string[]; overridden: boolean;
  shadowed: { input: string; by: BindingRef }[]; labels: string[];
}
plugin.bindings.list(): Promise<{ commands: BindingInfo[]; orphans: { ref: BindingRef; inputs: string[] }[] }>;
plugin.bindings.set(ref: BindingRef, inputs: string[]): Promise<void>;  // [] = unbind; validation failure: invalid; command missing and not an orphan: missing
plugin.bindings.reset(ref?: BindingRef): Promise<void>;                 // omitted = clear the overlay
plugin.bindings.onChange(listener: () => void): Disposable;
```

- No `ignoreDefaults` (rulings rejected R-K2; hotkeys implements it with K1).
- `list` is assembled from `bindings::Table`; `labels` uses the same function as the text on the right side of menus; `shadowed` comes from the conflict computation in `rebuild()`. `set` / `reset` write `Table.overlay`, then `rebuild()`, and save through the `ui_state.rs` debounce. Changes (declarations, unregistrations, overlay) push the new event `bindings.changed` (only to plugins that declare `bindings.manage`).
- `Mouse:LeftClick`: a new input name (syntax kept in sync in the host and the SDK). Only when it has an active binding is a left-button press in the video area judged with a delay: it counts as a single click only if no second press happens within the system double-click interval with movement ≤ 4 px; with no binding the current immediate focus behavior stays, and double-click detection is unchanged.
- TestHost: `host.bindings` (overlay and list simulation); `host.press("Mouse:LeftClick")` works.

### 3.5 Audio output (R-A2–R-A4, R-H9)

```ts
player.audioDevices(): Promise<{ id: string; name: string }[]>;   // playback.read
player.setAudioDevice(id: string | null): Promise<void>;          // playback.control; null = system default
PlayerState.audioOutput: { requested: string | null; active: string | null };
```

- mpv path: `audio-device-list` / `audio-device`; `active` is the device actually in use (falls back to the default when the device is unplugged, switches back to `requested` when it is plugged in again); remembered for the current run, not saved across launches (the audio plugin saves it and sets it on activation).
- Generated stream path (`producer_audio.rs`, cpal): open the device by the same `id`, fall back to the default on disconnect, switch back on recovery. First confirm that cpal devices and mpv CoreAudio devices map one to one (by name or UID); if not, record it in §6.2 and let generated streams follow only the system default (stated in the reference docs).
- No `devicesRevision`: newly plugged-in devices that are not selected are discovered by plugin polling (rulings).
- TestHost: `host.audioDevices` can be preset; `setAudioDevice` updates `audioOutput`.

### 3.6 Chapters and loops (R-M1, R-M2, R-H13)

```ts
player.chapters(): Promise<{ sourceId: number; chapters: { title: string | null; start: number }[] }>;  // playback.read
player.setLoop(loop: { a: number; b: number; count?: number } | null, opts?: Conditional): Promise<void>; // playback.control
PlayerState.loop: { a: number; b: number; count: number | null } | null;
```

- `chapters`: mpv `chapter-list`, read on demand, not part of `source.state`.
- `setLoop`: mpv `ab-loop-a` / `ab-loop-b` / `ab-loop-count` (`count` omitted = infinite); `0 ≤ a < b ≤ duration`, otherwise `invalid`; sources that cannot seek are `unsupported`; conditional commit as with other controls; no remaining count.
- When the source changes (new `sourceId`) the core clears `ab-loop-*` (H13).
- TestHost: `host.chapters` can be preset; `setLoop` writes `source.loop`.

### 3.7 Secondary subtitles (R-T3, remaining R-T7 item)

```ts
subtitles.select(track: SubtitleTrack | null, opts?: { role?: "primary" | "secondary"; ifCurrent? }): Promise<SubtitlesState>;
SubtitlesState.secondary: SubtitleTrack | null;
subtitles.load(bytes, { select?: "primary" | "secondary" | "none", … });
SubtitleStyle.secondaryPosition?: "top" | "aboveMain";
```

- mpv `secondary-sid`; `secondaryPosition` → `secondary-sub-pos` (`top` = at the top, `aboveMain` = above the primary subtitle).
- Primary and secondary cannot be the same track, and image tracks cannot be secondary (`invalid`); a source change clears the secondary; `trackConfirmation` is shared.
- `subtitle_state.rs` gets one more selection slot; removing a resource also clears the secondary selection.
- TestHost: `host.subtitles.selection` gets `secondary`.

### 3.8 Confirm dialog (rulings 3.3, capability `ui.dialog`)

```ts
plugin.dialogs.confirm(options: { message: string; detail?: string; confirm: string; cancel?: string;
  destructive?: boolean; checkbox?: string }): Promise<{ confirmed: boolean; checked: boolean }>;
```

- `NSAlert` (safe `objc2-app-kit` methods: `addButtonWithTitle`, `setShowsSuppressionButton` as the checkbox, `setHasDestructiveAction` on the button); a sheet on the main window, or a modal window when there is no main window; shares the "one dialog at a time" busy rule with file dialogs; `unavailable` in headless mode.
- Text lengths: `message` 1–200, `detail` ≤ 2000, buttons and checkbox 1–40 characters.
- TestHost: the `host.dialogAnswers` queue is also used for `confirm` (push `{ confirmed, checked }`).

### 3.9 Documentation

- Reference docs: all the interfaces above; add `window.control` and `bindings.manage` to the §2 capability table; add `Mouse:LeftClick` to §4.1; state the data directory rule in §3.4 (used by J07 "Open data directory"); this round's change notes in §17.
- Examples: one short example (≤ 30 lines) each for `window` state and `setVideoArea`, `bindings.list/set`, and `dialogs.confirm`.

## 4. Plugin integration assignments (P2)

Author rules are the same as in round 2 ([2026-10-features-r2.md](2026-10-features-r2.md) §4): write only in your own plugin directory; implement according to the matching section of the requirements document; when a core capability is missing, write a request per workflow §4.1 and keep working on the parts that do not depend on it. Home-made implementations in plugins that this round's core now provides (such as playlist's `confirm.ts`) must be deleted and replaced with the core capability; the report lists the deleted files and line counts.

### 4.1 Assignment table

| Author | Plugin | New features (ID → dependency) | Migrate to core capability | Fallback and docs |
| --- | --- | --- | --- | --- |
| A | window (new) | H03 size multiples (`setVideoArea` + `stats().video.displayWidth/Height`), H04 fit to video aspect ratio, H05 always on top (`setAlwaysOnTop`; for "in fullscreen" see that section of the requirements document), B30 hide pointer while playing (`setCursorAutoHide`) | — | H07 picture-in-picture moves to 4a: this round may offer an optional "mini size" form (W2 + W3); the author decides and writes it into the requirements document |
| B | marks (new) | B21 chapter/bookmark menu and jumping (`player.chapters`; bookmarks use `storage.json`), B23 A-B loop (`setLoop` / `state.loop`) | — | B22 progress bar markers move to 3b (timeline marker layer); the bookmark list window is done this round |
| C | hotkeys (new), plugins (new) | hotkeys: J01 shortcut settings, J02 mouse actions (including `Mouse:LeftClick`), J03 ignore default shortcuts (per the alternative in ruling R-K2, recorded with `bindings.set` and this plugin's `storage.json`). plugins: J07 list, disable, enable, uninstall, roll back (existing `rollback`), open data directory (`system.openPath`) | Uninstall and disable confirmations use `dialogs.confirm` | J04 preferences mounting moves to 3b: this round both plugins use their own tool windows |
| D | audio, subtitles | audio: G16 output device (`audioDevices` / `setAudioDevice` / `state.audioOutput`; the device choice is saved with `storage.json` and restored on activation; 5-second polling to discover new devices). subtitles: A08 secondary subtitles (`select({ role: "secondary" })`, `load({ select: "secondary" })`, `secondaryPosition`) | — | G16: if the host records that generated streams cannot follow the device (§3.5), the menu description says so |
| E | info, playlist | info: I09 About (`plugin.app`, `packages.list`); the mpv / FFmpeg version lines of I05 switch to `plugin.app.components`. playlist: confirmations for clear, remove, etc. switch to `dialogs.confirm` | playlist `confirm.ts` → `dialogs.confirm` (delete that file); if info has its own version probing (`ffmpeg -version` etc.), switch to `plugin.app` | — |

Plugins not in this round: preferences, visualizer (from 3b); the other plugins have no items this round, but if they use `Window.onClose` (delivered in round 2) they can adopt it along the way (polling in info's playback properties window, checkbox state in capture's burst window).

### 4.2 Entry conditions

- Maintainer delivery order (§2): authors A, B, E start after the first batch (window, app info, confirm dialog, chapters and loops) passes review; authors C, D start after the second batch (bindings, plugin management, audio output, secondary subtitles) passes review.
- Authors can do the parts that do not depend on others first: UI, menus, saving settings, tests under TestHost.
- No more than 3 authors run at the same time (4 including the coordinator).

## 5. Acceptance

### 5.1 Static checks and tests

All commands in workflow §6.1; no plugin's test count goes down; new plugins pass `check/build/test`; the menu snapshot only gains items added this round (`doc/process/rounds/menus-snapshot.test.ts`).

### 5.2 Real GUI tests (coordinator)

All required scenarios from workflow §6.2, plus this round's:

| Area | Scenario | Evidence |
| --- | --- | --- |
| Window | After 1× / 2× the video area matches the source video size (with sidebar on and off); when larger than the screen, `clamped` and the window stays in the visible area; menu items grayed out in fullscreen; after always-on-top, switching to another app's window keeps the player on top, and it is restored after window is disabled; "Hide pointer while playing" hides it after 3 seconds in window mode and shows it again on move | Screenshots, window sizes |
| App info | The About window shows the Vestra version, mpv, the FFmpeg library versions and the Slint version, matching `mpv --version` | Screenshot |
| Plugin management | The list contains all plugins with their state; after disabling video its menus disappear and picture effects are undone, and it stays disabled after restarting the player; enabling restores it; after uninstalling a plugin installed from the workshop it disappears from the list and the data directory is deleted according to the option; menu item grayed out when disabling itself | Screenshots |
| Bindings | After rebinding Space to K, K plays/pauses and Space no longer triggers, kept after restart; restore defaults; on conflict the occupying command is shown; after "Ignore default shortcuts" the default keys stop working and rebinds still work; after binding left single click to "Play/Pause", a single click toggles and double click is still fullscreen | Physical key log |
| Audio output | Device list matches "System Settings ▶ Sound"; after switching to headphones sound comes from the headphones; unplugging the headphones falls back to the speakers and `active` changes, plugging them back switches back; the NES generated stream follows the same way (or the fallback recorded in §6.2) | Listening notes |
| Chapters and loops | For an mkv with chapters the menu lists chapters and can jump; A-B loop jumps back precisely at point B; the loop is cleared after changing files | Screenshots, times |
| Secondary subtitles | Chinese and English tracks shown at the same time, with the secondary subtitle in both positions, top / above the primary; primary and secondary cannot be the same track | Screenshots |
| Confirm dialog | A system confirm dialog appears when clearing the playlist, and cancel does not clear; a confirmation with a checkbox remembers the choice | Screenshots |

Defect ownership is as in workflow §6.2.

## 6. Records

### 6.1 Baseline

(filled in at P0)

### 6.2 Maintainer delivery and signature adjustments

#### P1 batch 1 (2026-10-21, maintainer): window, app info, confirm dialog, chapters and loops

**Implementation list** (each item: host + SDK types and wrapper + TestHost + reference docs + unit tests)

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §3.1 | `window.state` / `onChange` (R-W1) | `host.slint` adds `video-width/height` (`canvas` size); the layout facts in `window_tick` gain `videoArea`, pushed with the existing `window.state` event only on change; new method `window.state` (no capability needed) | `sdk/plugin/window.ts`: read once before activation (in parallel with `player.state`); after the plugin first reads `state` / `onChange`, it follows events; `host.mainWindow`, `host.setMainWindow` |
| §3.1 | `setVideoArea` (R-W2) | `window_control::fit` (adds the bottom bar and attached sidebar, scales down proportionally to the visible screen area, scales up to the minimum 318×120 / window 320×182); `platform_window::resize_content` (`NSWindow` `setFrame_display`, keeps the top-left corner, moves back into the visible area); replies only when the layout shows the target size (±1) or after 1 second | refresh `state` after resolve; reference §6.6 |
| §3.1 | `setAlwaysOnTop` (R-W3) | `platform_window::set_floating`: all normal-level windows of the app ↔ `NSFloatingWindowLevel` (safe `setLevel` method); re-applied when the window count changes (new tool window, floating sidebar); `window_control::LastWrite` last write wins, reverts after the plugin is disabled | `host.mainWindow.onTop` |
| §3.1 | `setCursorAutoHide` (R-W4) | `cursor-hidden = playing && ((fullscreen && bottom bar idle) \|\| ("playing" && Policy::idle && (fullscreen \|\| main window is the active window)))`; `Policy::idle` uses the same idle and occlusion rules as the fullscreen bottom bar; `LastWrite`, reverts to `"fullscreen"` on disable | `host.mainWindow.cursorAutoHide` |
| §3.1 | Capability `window.control` | both `plugins.rs` and `scripts/plugin.mjs` | reference §2 |
| §3.2 | `plugin.app` (R-W6) | the backend reads `mpv-version` and `ffmpeg-version` at startup; `build.rs` takes the Slint version from `Cargo.lock` and the build hash from `git rev-parse --short HEAD` (not set if unavailable, `build: null`); after `plugins::set_app_info` it is passed to each plugin process through the environment variable `VESTRA_APP_INFO` (no RPC) | `runner.ts` reads the environment variable and passes it as the 5th argument of `createPlugin`; `host.app`; reference §3.5 |
| §3.6 | `player.chapters` (R-M1) | `source_backend::marks`: `chapter-list` → `{ title, start }` (≤ 4096 entries, empty title is `null`); empty with no media or for generated streams | `host.chapters` |
| §3.6 | `setLoop` / `PlayerState.loop` (R-M2) | `ab-loop-a/b/count`; validates `0 ≤ a < b ≤ duration` (`invalid`), seekable (`unsupported`), media ready (`unavailable`), generated streams `unsupported`; conditional commit reuses `check_expected`; `source.state.loop` is the requested value | `setLoop` writes `host.source.loop` |
| §3.6 | R-H13 | `Sources::loop_state`: clears `ab-loop-*` and the request when the source generation changes; clears when `count` is used up (mpv `remaining-ab-loops == 0`), and `state.loop` becomes `null` | TestHost clears it when opening a new source |
| §3.8 | `dialogs.confirm` (rulings 3.3) | `confirm.rs`: `NSAlert` (`addButtonWithTitle`, `setHasDestructiveAction` on the confirm button, `setKeyEquivalent("\u{1b}")` on the cancel button, `setShowsSuppressionButton` as the checkbox); `beginSheetModalForWindow_completionHandler` (`block2::RcBlock`) when there is a main window, otherwise `runModal`; shares `dialog_busy` with file dialogs; lengths counted in characters | `host.dialogAnswers.push({ confirmed, checked })`, `host.confirms`; reference §7 |
| §3.9 | Docs | — | reference §2, §3.3, §3.5 (new), §5.1, §5.2, §6.6 (new), §7, §14, §17 batch 1; examples §22–§24 |

**Signature adjustments and implementation choices** (all signatures match the rulings)

1. `PlayerState.loop` is written as optional in `types.ts` (`loop?: … | null`): playlist's tests build `PlayerState` objects by hand (`tests/queue.test.ts`), a required field would make `npm run check` fail, and plugin directories are not the maintainer's to change. The SDK always provides the field (`null` when there is no loop); the reference docs say so. `audioOutput` in batch 2 is handled the same way.
2. `plugin.app` is on `Scope` (child scopes have it too, like the other modules), not only on the `Plugin` root.
3. `libav` only has `{ ffmpeg: <ffmpeg-version> }`: libmpv 0.41 has no property reporting the versions of libavcodec and the other libraries; reference §3.5 says to use `tools.run("ffmpeg", ["-version"])` when needed. Measured values: `mpv: "mpv v0.41.0"`, `libav: { ffmpeg: "9.0.2" }`, `slint: "1.16.1"`, `build: "289b26b"`.
4. Remaining count: mpv 0.41 does have a `remaining-ab-loops` property (the rulings said "if it cannot be provided, do not provide it"). Per the ruling's signature it is not exposed; the host only uses it internally to detect the end of the loop; it can be added later as a compatible field if needed. The meaning of `count` is documented as "number of jumps back" (with `count: 2` the a–b segment plays 3 times in total).
5. Following `window.state` is lazy: each plugin reads it once before activation, but only subscribes to events after the plugin has read `state` or registered `onChange` (dragging to resize the window changes the layout every frame, and not every plugin should receive that).
6. The "attached sidebar" in `setVideoArea` is judged by preference (`preferred && !detached`), not by whether it is currently shown: the sidebar appears once the window grows, so its width is included in advance; if it does not fit it is hidden by the existing rules (`reason: "space"`).
7. TestHost's `setLoop` checks for generated streams (`unsupported`) before the condition; the host checks the condition first. This only affects the error code when both fail.
8. The protocol order assertion in `runner.test.ts` is updated to `["window.state", "commands.register"]` (the new read before activation).

**Checks** (actually run in the worktree)

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 76/76 (4 new in `tests/sdk/round3a.test.ts`); plugin tests audio 30, bilibili 15, capture 24, history 22, info 20, nes 11, playback 45, playlist 61, subtitle-online 13, subtitles 46, video 22, workshop 17, youtube 14 all pass (workshop `vendor/sdk` resynced with the command at the top of the test, in the same commit).
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 187/187 (new: 2 for `window_control`, 1 for `confirm`, 1 for chapter parsing).
- Headless run (`--headless --silent` + a temporary probe plugin, not in the repo; a 10-second mkv with two chapters): `plugin.app` as above; `chapters()` → `[{Opening, 0}, {Main, 4}]`; after `setLoop({ a: 1, b: 2, count: 2 })` the position goes 1→1.93→1→1.87→1→2.53 (two jumps back, then continues), after which `state.loop` is `null`; `setLoop({ a: 5, b: 60 })` → `invalid`; after reopening the same file `state.loop` is `null`; in headless mode `confirm`, `setAlwaysOnTop`, `setVideoArea` → `unavailable`.
- Host Rust: `src/host/src/*.rs` 26 306 → 27 068 (+762, including tests), plus 40 lines of `build.rs`; SDK and runner `src/sdk/plugin/*.ts` + `src/runtime/` +330 (TestHost 120 of that), `src/sdk/*.ts` +41.

**GUI test points for the coordinator** (needs a plugin that calls these interfaces: author A's window, B's marks, or a temporary probe)

1. `setVideoArea({ width: 1280, height: 720 })`: the video area is exactly 1280×720 (measure on a screenshot), the window's top-left corner does not move; call again with the sidebar open: window width = 1280 + 3 + sidebar width; ask for 3840×2160 on a 1440×900 screen: `clamped: true`, the window is fully in the visible area (not covering the menu bar or Dock), aspect 16:9; ask for 100×50: video area 318×120 (window 320×182) and `clamped`; `unavailable` in fullscreen.
2. After `setAlwaysOnTop(true)` click Finder or another app: the main window and floating sidebar stay on top; a tool window opened now (such as playback properties) is also on top; `false` or disabling the plugin restores the normal level; system file dialogs work as usual.
3. `setCursorAutoHide("playing")`: when playing in window mode, a pointer resting on the video for 3 seconds disappears and reappears on move; it does not disappear when paused; not when the pointer is on the bottom bar; not while a menu is open; not after switching to another app (main window not active); after disabling the plugin it only disappears in fullscreen.
4. `dialogs.confirm`: appears as a sheet on the main window (moves with the window), Return = confirm, Esc = cancel; with `destructive: true` the confirm button text is red; with `checkbox`, checking it gives `checked: true`; calling `confirm` or `openFile` again while the dialog is open gives `busy`; shortcuts do not fire while the dialog is open.
5. `plugin.app`: matches the first line of `mpv --version` in a terminal and `ffmpeg -version` (when using the same Homebrew libraries).
6. Chapters and loops: for an mkv with chapters, `chapters()` matches `mpv --term-playing-msg='${chapter-list}'`; the A-B loop returns to A at point B (error about one frame); no more jumps back after changing files.

#### Priority fixes (coordinator GUI test feedback)

- **Round 2 fix: subtitle state after external load** (`80aac8a`, based on `289b26b`, for cherry-picking into `rebuild/features-r2`; `26336bc` on this branch). Root cause in the SDK: while a new source is loading, the host still reports the old source's selection stamp (revision 2 after an external subtitle is loaded); once the new source is ready, selection starts from revision 0 of the new generation; `SubtitleService.accept` discarded every ready state as "same source generation, smaller revision", so the plugin stayed in `loading`, and `expected()` still carried the old generation's stamp, so every change was `stale`. Changed to compare revisions only within the same selection generation. Regression test in `tests/sdk/round2.test.ts`; headless repro scripts (load SRT / ASS, remove after loading, B as mkv / mp4 without subtitles) all reach `ready` within 1 second and `select` succeeds.
- **Round 3a fix: Option symbols, Option + named keys, stale Slint modifiers** (`0e3dfbc`). ① When Option types a non-alphanumeric character (German ⌥5 = `[`, ⌥7 = `|`, ⌥⇧7 = `\`, ⌥L = `@`), match by that character first, then by the physical key `Alt+…`; when both have bindings the character wins; letters still match by physical key; US ⌥[ is still `Alt+[`. ② Named keys (Delete, Backspace, arrow keys, Home/End, PageUp/Down, Enter, Tab, Escape, Space, Insert, F1–F24) are recognized by physical key, so ⌥⌦ without text is also `Alt+Delete`. ③ Slint tracks modifier state only from the modifier keys' own key events; when winit `ModifiersChanged` reports a modifier released and Slint did not get it, the host sends a `KeyReleased` to that window in the main loop, so a stuck ⌥ does not affect later context menus. Reference §4.1 documents the layout rules and the physical key for `Delete`; `VESTRA_DEBUG_KEYS` also prints `typed=`.

#### P1 batch 2 (2026-10-21, maintainer): binding management, plugin management, audio output, secondary subtitles

**Implementation list**

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §3.4 | `plugin.bindings.*` (R-K1, capability `bindings.manage`) | `bindings::Table`: `infos()` (defaults / current / overridden / shadowed recorded by `rebuild()`), `orphans()`, `canonical()`, `revision`; `binding_manager.rs`: `bindings.list/set/reset`, the overlay is written to `ui_state` (debounced save), then `set_overlay`; `labels` uses the same `bindings::label` as the menu text | `bindingsApi` in `sdk/plugin/bindings.ts`; `host.bindings.overlay` (`press` matches by overlay) |
| §3.4 | `bindings.changed` | pushed after the binding table revision changes (at most once per 200 ms); `events.subscribe` requires `bindings.manage` instead of `events` for this event | `Runtime.on(event, listener, needs)` |
| §3.4 | `Mouse:LeftClick` (R-K3, R-H11) | input name (in both host and SDK); a left-button press in the video area in `host.slint` calls `left-press(x, y, modifiers)`, registered only when there is an active binding; `window_tick` fires after the system double-click interval (`NSEvent::doubleClickInterval`) if the pointer moved ≤ 4 points; a second press within the interval cancels it (double click works as usual) | `host.press("Mouse:LeftClick")` |
| §3.3 | `packages.list` / `onChange` (R-G1) | `package_manager::package_list`: running table (activation state → starting / running / failed) + install store (current / previous, disabled set) + development loads of the current run; `failures` records activation failures, runtime faults and unexpected process exits; `packages.changed` is compared every 200 ms and only computed when there are subscribers; this event needs no capability | `host.packages`, `host.setPackages` |
| §3.3 | `setEnabled` (R-G2), R-H10 | disable = `disable(owner)`; for installed plugins written into catalog `disabled` (in the same atomic replace) and skipped at startup; development loads recorded only for the current run; enable = `load_package` / `load`; disabling itself `invalid`, trial load `unsupported`, unknown `missing`, installing `busy`; an install or rollback commit clears the disabled mark | `host.packageCalls` |
| §3.3 | `uninstall` (R-G3) | installed plugins only; disable first, `Store::uninstall` removes the record and receipt in one catalog replace, then `gc`; `removeData` deletes `<data root>/<id>` after the process is reaped (`Reaped`); the overlay is left alone (orphan); reuses `requestId` receipt deduplication and `PackageChange` | the SDK reuses `change()` (expected revision + request id) |
| §3.5 | `audioDevices` / `setAudioDevice` / `audioOutput` (R-A2–A4) | `source_backend`: `audio-device-list` (drops `auto`; when `coreaudio/` entries exist only those are listed, since macOS also has duplicate `avfoundation/` entries); every second checks whether the requested device is in the list: if so `audio-device = <id>`, otherwise `auto`, switching back after reconnect; `source.state.audioOutput = { requested, active }` | `host.audioDevices`; `setAudioDevice` updates `host.source.audioOutput` |
| §3.5 | R-H9 generated streams follow the device | cpal CoreAudio's `DeviceId` is the device UID and maps one to one to mpv's `coreaudio/<UID>` (`device_by_id`). `producer_audio::Audio`: the render state is put in `Arc<Mutex<Callback>>`, and the data callback only does `try_lock` (contention only at the moment of switching devices, outputting one silent buffer); `retarget()` pauses the old stream, builds and plays a stream on the new device, with queue, clock and pause state unchanged; `DeviceNotAvailable` on the selected device is no longer fatal, and the once-per-second follow switches back to the default | reference §5.2 |
| §3.7 | Secondary subtitles (R-T3, remaining R-T7 item) | `subtitle_state` adds a `secondary` selection slot: `select_secondary`, `register_as_secondary`, primary and secondary mutually exclusive (`invalid` in both directions), cleared on resource removal, when the owning plugin is disabled, when an embedded track disappears, and on source change; `subtitle_backend`: `role` for `subtitles.select`, `select: "secondary"` for `commit`, image tracks as secondary `invalid`, `set_target` sets `secondary-sid=no`, `sid`, `secondary-sid` in order and confirms each; `secondary-sid=no` when a new source is ready; `SubtitleStyle.secondaryPosition` → `secondary-sub-pos` | `SubtitlesState.secondary`; `host.subtitles.selection.secondary` |
| §3.9 | Docs | — | reference §2, §3.4 (data directory rule), §4.1, §4.2 (new), §5.1, §5.2, §9, §12, §14, §17 batch 2; examples §25 (shortcut settings), §26 (plugin management and audio output) |

**Signature adjustments and implementation choices** (all signatures match the rulings)

1. `PlayerState.audioOutput`, like `loop` in batch 1, is written as optional in `types.ts`; the SDK always provides it.
2. `setAudioDevice` accepts an id that does not currently exist (`active: null`, takes effect once connected): the audio plugin restores the saved device on activation, when the device may not be plugged in yet. A bad format (empty, longer than 512, control characters) is `invalid`.
3. `audioDevices` lists only `coreaudio/` devices (if there are any): on macOS mpv lists every device again under `avfoundation/`, and the same device appearing twice means nothing to the user.
4. `packages.list` / `onChange` need no capability, `setEnabled` / `uninstall` need `plugins.manage`: the host's capability table used to put all of `packages.*` under `plugins.manage`; now `packages.list` is opened separately, and subscribing to the `packages.changed` event does not require `events` either.
5. `PluginInfo.version` is `null` for development loads without a `version` field. `directory` is the package directory (canonical path); the data directory is derived as `<data root>/<id>`, and reference §3.4 states "data root = `dirname(plugin.dataDirectory)`".
6. `uninstall` completes synchronously: the returned `PackageChange` is `committed` from the start; the `removeData` deletion happens after the plugin process exits (it may still be writing its settings file while being disabled).
7. `secondaryPosition: "aboveMain"` uses an estimate: `secondary-sub-pos = 100 − bottom margin% − height% of two primary subtitle lines` (font size relative to a 1080p reference, line height 1.25); 87 for font size 36 and margin 5%. When the primary subtitle actually has only one line, the secondary is one line further from it than needed; the GUI test confirms how it looks.
8. Screenshots "with subtitles" (`media.capture({ processed })`) only include the primary subtitle.
9. TestHost's `packages.installed` has a new default handler (returns revisions 1/0), only so that `uninstall`'s expected revision is usable in tests; existing tests that override with `host.handle` are not affected.

**Checks**

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 81/81 (4 new in `round3a.test.ts`: bindings, plugin management, audio output, secondary subtitles); plugin tests audio 30, bilibili 15, capture 24, history 22, info 20, nes 11, playback 45, playlist 61, subtitle-online 13, subtitles 46, video 22, workshop 17, youtube 14 all pass (workshop `vendor/sdk` resynced). One timing case in `test:sdk` failed once intermittently and passed on rerun.
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 195/195 (new in batch 2: binding table management 1, reference validation 1, install store disable and uninstall 1, device list 1, secondary subtitle selection slot 1, secondary subtitle position 1).
- Headless run (two development-loaded probe plugins): `bindings.list` has 18 commands, the plugin's Space shows as occupied by `host/play-pause`; after `set(host/play-pause, ["K","Mouse:LeftClick"])` `current` and `labels` update and the conflict disappears; a nonexistent command gives `missing`; `reset()` restores; `onChange` is coalesced into 2 calls. `packages.list` shows the two development loads as `running`; disabling itself `invalid`; after disabling the other one `packages.changed` pushes `disabled`, and after enabling it returns to `running`; uninstalling a development load `unsupported`. `audioDevices` → `[{coreaudio/BuiltInSpeakerDevice, MacBook Air Speakers}]`; after selecting it `audioOutput` has the same value for `{requested, active}`, and selecting a nonexistent id gives `active: null`. Secondary subtitles: after `load({ select: "secondary" })` `secondary` is that track with `trackConfirmation: applied`; then setting an embedded track as secondary succeeds; `secondaryPosition: "aboveMain"` enters the effective style; after changing the source `secondary: null`.
- Host Rust: `src/host/src/*.rs` 27 068 → 28 577 (batch 2 +1 241, priority key fix +268); this round in total 26 306 → 28 577 = **+2 271**, plus 40 lines of `build.rs`, within the budget of about 3 000 lines. SDK and runner (`src/sdk/plugin/*.ts` + `src/runtime/`) +882 this round in total, wire types in `src/sdk/*.ts` +117.

**GUI test points for the coordinator**

1. Bindings: use the hotkeys plugin or a probe to change "Play/Pause" to K: K works, Space no longer triggers, and the menu shows K on the right; after restarting the player it is still K (the UI state file has `host/play-pause` under `bindings`); after `reset` Space is back; for conflicting commands, `shadowed` in `list` names the occupying command.
2. Left single click: add `Mouse:LeftClick` to "Play/Pause": a single click on the video toggles after about 0.25–0.5 seconds (set by the system "Double-click speed" setting); double click is still fullscreen and does not also trigger a single click; pressing and dragging more than 4 points does not trigger; after removing the binding a single click has no delay and only focuses.
3. Plugin management: after disabling video its menus disappear and picture effects are undone; after restarting the player video is still disabled (for installed plugins; a `--plugin` development load is loaded again after restart); enabling restores it; after uninstalling a plugin installed from the workshop it disappears from the list, and with `removeData: true` `~/Library/Application Support/Vestra/plugin-data/<id>/` is deleted; disabling itself gives `invalid`.
4. Audio output: device list matches "System Settings ▶ Sound" (each device once); after selecting headphones sound comes from the headphones; after unplugging the headphones it returns to the speakers within about 1 second and `active` becomes `null`, and after plugging back in it switches back within about 1 second; NES playback follows the same way (there may be one buffer of silence at the moment of plugging or unplugging), and NES does not stop when the selected device is unplugged.
5. Secondary subtitles: two embedded tracks, Chinese and English, with Chinese as primary and English as secondary shown at the same time; `secondaryPosition` `top` is at the top of the picture and `aboveMain` is above the primary subtitle (check whether the spacing looks right); selecting the primary track as secondary gives `invalid`; a PGS track cannot be secondary; the secondary subtitle disappears after changing files.

- **Round 3a fix: the host hands Option + named keys to Slint** (coordinator test: after ⌥⌦ was passed to Slint, it neither reached the focus chain nor let the panel's pointer events continue until the next normal key). The winit hook no longer passes press, repeat and release of Option + named keys (⌦, ⌫, arrow keys, Home/End, PageUp/Down, Enter, Tab, Esc, Insert, F1–F24; except ⌥Space, which types a non-breaking space) to Slint; instead the main loop re-dispatches them as normal `KeyPressed` / `KeyPressRepeated` / `KeyReleased`, after first recording the physical key and the winit modifiers at press time; so the events go through Slint's normal focus chain, the scope is decided by focus (`panel:sidebar` with list focus), and `from_key` resolves to `Alt+Delete`. `VESTRA_DEBUG_KEYS` prints `keys: inject to slint …`.

#### Pending real keyboard verification: ⌥⌦ and the panel context menu after it (coordinator ruling)

- **Symptom** (System Events synthesizes `key code 117 using option down`, with a playlist row selected and the list focused): winit receives `Pressed/Released physical=Delete logical=Named(Delete) text=None modifiers=ALT`, but there are no `keys: slint` lines on the Slint side, and `Alt+Delete` "Remove all" does not fire; afterwards, right-clicking on the list, the playlist sends no `ui.menu` and no menu appears, until Esc is pressed once (Esc arrives normally through `panel:sidebar`) and things recover. Without ⌥ everything is normal; German Option symbols such as ⌥5 / ⌥⇧7 (`0e3dfbc`) pass in testing.
- **Three fixes**:
  1. `0e3dfbc`: named keys are resolved by physical key (⌥⌦ without text is also `Alt+Delete`); when winit `ModifiersChanged` reports a modifier released and Slint did not get it, a `KeyReleased` is sent. Test: the release was delivered (`release to slint ['\u{12}', '\u{13}'] window=3 live=true`), the problem remains.
  2. `bc879c1`: diagnostics only (modifier release injection, `ui.menu` requests and refusals). Confirmed that on right click the plugin does not call `ui.menu` at all.
  3. `bcc372b`: Option + named keys are not passed to Slint; the main loop re-dispatches them as normal `KeyPressed/KeyReleased` (`inject to slint '\u{7f}' Pressed/Released window=3` appears), still no `keys: slint` lines. Kept: unit tests pass, and other key paths are not affected.
- **Current status**: pending real keyboard verification. Per workflow §3 "second failure on the same path", the coordinator ruled to add no more probes and not to block 3a. Assessment: System Events synthetic events do not send separate Option key down/up events, so Slint's internal modifier and key state cannot be corrected with synthetic events; a real keyboard may not reproduce it.
- **If the user reproduces it with a real keyboard, the next plan**: bypass Slint entirely for Option + named keys and fire bindings directly in the host by focus scope. The host needs to know which scope has focus: record focus enter/leave on the outer FocusScope of the bottom bar and sidebar generated by `composition.rs` (or record the last focused area on pointer press in the panels and the video area); the winit hook then uses `table.resolve(Alt+<key>, panel)` and `fire`, and on a hit `PreventDefault`. Also check the context menu: whether the plugin TouchArea's `pointer-event` is still called (whether Slint pointer grab / `pressed` state is stuck); if needed, send one pointer `Exit` in the hook after Option + named keys.

#### Ruling: author D's three items (secondary subtitles, audio output)

1. **Setting the secondary subtitle position when the primary is PGS**: change the core (small change, no new interface). `secondaryPosition` stays a field of `SubtitleStyle`; the host used to reject the whole `setStyle` when the primary subtitle could not be styled (`unsupported`), and now accepts it when a secondary subtitle is selected (the style applies to the text secondary subtitle and has no effect on the image primary). Without a secondary subtitle it is still `unsupported`. Reason: do not split out a separate interface for one field (conservative), and the old rejection rule makes no sense when there is a secondary subtitle.
2. **Secondary subtitle font size 85% of the primary**: rejected, the requirement is lowered. Current host state: the secondary and primary subtitles share one style layer (font, size, color, outline); mpv 0.41 has no secondary subtitle font size option (only `secondary-sub-pos`, `secondary-sub-delay`, `secondary-sub-ass-override`, `secondary-sub-visibility`), so the host cannot give it a size different from the primary. The author states "the secondary subtitle font size is the same as the primary" in the matching section of the requirements document. Reference §9 already says so.
3. **Restore the system default device when the audio plugin is disabled**: accepted, change the core. Consistent with `setAlwaysOnTop` and `setCursorAutoHide`: when the plugin that set the device is disabled, the core returns to the system default (mpv and generated streams follow within 1 second). No signature change; reference §5.2 already says so.

Also: TestHost's `subtitles.remove` now also clears `selection.secondary` when it points to the removed external track (consistent with the host), covered by `round3a.test.ts`.

### 6.3 Author delivery

(filled in by each author on completion)

### 6.4 Acceptance results

(filled in at P3)

### Coordinator acceptance (2026-10-21)

All checks pass: check, format, boundaries, test:sdk, 17 plugin test suites (423 tests in total), cargo fmt/clippy/test (196 tests).

**GUI tests passed**:
- German layout: ⌥5 "[", "]", ⌥⇧7 "\" trigger the A-B loop.
- marks: chapter menu (a chapter without a title shows "章节 3" (Chapter 3)), chapter jumping.
- window: pressing 1 gives 0.5× (video area 640×360); ⌃T cycles the three always-on-top modes.
- info: About window (logo, version, components).
- playlist: system confirm dialog for "Remove all" (red button, "Don't ask again").
- Subtitles: primary and secondary subtitles shown at the same time (secondary at the top); the primary track is grayed out in the secondary subtitle menu.
- Audio: output device list and switching (this machine only has the built-in speakers; plugging and unplugging not tested).
- plugins: disable, enable (with confirm dialog and OSD).
- hotkeys: select a row, add key K, restore defaults.

**Fixed on the spot**:
- hotkeys row click area: `ToolTipArea` had no size set, so it was computed as 0×0 and clicks did not reach the row (author C).
- The confirm dialog is a sheet attached to the main window, and after it appears the tool window is covered: the plugin calls `show()` on its own window again after the dialog closes. The core request "attach the confirm dialog to the tool window that calls it" is deferred to 3b for evaluation.

**Pending real keyboard verification**: ⌥⌦ and the broken context menu after it (see the record above, 366da09).
