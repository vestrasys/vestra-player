# Round 2: core batch one (playback, tracks, processing nodes, basic action handover) + plugin adoption

Branch: `rebuild/features-r2` (created by the coordinator from the commit after `rebuild/features-r1` was merged into `main`). Basis: the [negotiation record](2026-10-features-negotiation.md) "Step 3: Rulings" (below: "the rulings"), the [feature requirements](../../product/feature-requirements.md) (below: "the requirements document"), and the [community workflow](../community-workflow.md) §3, §4, §5. The overall round plan is in ruling 3.5: 2 → 3a → 3b → 4a → 4b.

## 1. Scope

This round does only the items marked "2" in the rulings. It does not start items from 3a onward and adds no interfaces outside the rulings. If during implementation a signature from the rulings turns out to be impossible, the maintainer records the reason and the adjusted signature in §6 of this document before handing it to the authors, and does not widen the scope on their own.

| Category | Items |
| --- | --- |
| Basic action handover (§5 top-down) | R-H1, R-P3, R-P4, R-P1 + R-P2 (`player.step`) |
| Playback information | R-P6 + R-P7 (`PlaybackStats` extension); host behavior "push phase changes immediately" (the alternative to R-P5) |
| Audio tracks | R-T1, R-T2 |
| Subtitles | R-T4 (`select` only `"primary"`, `"none"`), R-T6 (with `streamIndex`), R-T7 (without `secondaryPosition`), R-T8, R-H12 |
| Processing nodes | R-V1–R-V7 (eight video nodes), R-A1, R-A5 |
| Binding defects | R-H5, R-H4 |
| General capabilities (host) | clipboard `plugin.clipboard`, sleep prevention `plugin.power` |
| General capabilities (runtime) | `plugin.tools`, `plugin.storage`, `plugin.system` |
| Documentation contract | R-U4: write "the bottom bar is not clipped" into reference §6.3 |

Not in this round: secondary subtitles (R-T3), chapters and A-B loop (R-M1, R-M2, R-H13), audio output devices (R-A2–R-A4, R-H9), confirmation dialogs, frame doubling (R-V8), and all other items from 3a onward.

## 2. Two tracks and order

| Phase | Done by | Content | Done when |
| --- | --- | --- | --- |
| P0 | Coordinator | Create the branch; record the baseline (§6.1 static checks, test count per plugin, host and SDK line counts) | Baseline written into §6 |
| P1a | Maintainer | Batch A (§3.2–§3.6): compatible extensions and general capabilities | All static checks pass; reference docs and TestHost in sync; coordinator review passed |
| P1b | Maintainer | Batch B (§3.1): basic action handover, landed behind a switch per §5 | Same as above; with the switch on, the old playback commands still work (see the §3.1 switch-over rules) |
| P2 | Plugin authors | §4 assignments. Authors who depend on batch A start after the P1a review passes; the playback handover migration starts after the P1b review passes | `check/build/test` pass for each plugin |
| P3 | Maintainer | Remove the handover switch and code that only serves the old path (§5 P4) | No code references the switch; full static checks |
| P4 | Coordinator | §5 acceptance, commit and push, merge into `main` | — |

The maintainer can do P1a and P1b in sequence; authors can start the parts that do not depend on the handover without waiting for P1b (§3 entry rules).

## 3. Maintainer checklist (P1)

Each item must complete all of: host implementation (`src/host/`), SDK types and implementation (`src/sdk/`), TestHost simulation (`src/sdk/plugin/testing.ts`), reference docs (`doc/plugins/reference.md`), unit tests; new capability names are added to manifest validation (on both the host and the build tool side). First-party Rust on the host side must not use `unsafe`. Signatures follow rulings 3.1 / 3.3; below are only implementation notes.

### 3.1 Batch B: basic action handover (R-H1, R-P3, R-P4, R-P1 + R-P2)

**Interface**

```ts
type HostAction =
  | "play-pause" | "stop" | "previous" | "next"
  | "frame-back" | "frame-forward"
  | "seek-backward" | "seek-forward"
  | "rate-down" | "rate-up" | "rate-normal"
  | "volume-up" | "volume-down" | "mute";
plugin.host.run(action: HostAction): Promise<void>;                        // requires commands; unavailable when disabled
player.setStepSeek(options: { seconds: number }): Promise<void>;          // requires playback.control; 0.1–3600, one decimal place
player.step(unit: "frame" | "keyframe", direction: "backward" | "forward",
            opts?: { ifCurrent?: PlayerState }): Promise<void>;            // requires playback.control
```

**Host changes**

- Core action registration: the same mechanism as the existing host commands (where `fullscreen`, `exit-fullscreen`, `quit` are registered in `main.rs`, around lines 1440–1510), `owner = HOST`. Menu paths, order, group, default bindings and `repeat` match the table in the requirements document section "Basic playback actions handed over to the core"; `rate-normal` corresponds to "Playback ▶ Speed ▶ Normal" (220, Z).
- Behavior is moved as-is from `src/plugins/playback/controller.ts` to the host, with unchanged text: play/pause (replay when `ended`), relative seek accumulation (accumulate from the latest unfinished target, clamp between 0 and the duration, step from `setStepSeek`), rate ±0.1 (rounded to 0.01, clamped to 0.25–4), volume ±5 (clamped to 0–200, set the volume first, then unmute), previous/next handed to the playback sequence provider, stop = close the source. Each action's `enabled` / `busy` / `reason` and OSD text are checked one by one against the existing playback tests.
- `frame-back` / `frame-forward` call the same implementation as `player.step`; repeated triggers while one is in flight are dropped (implemented in the host action; `player.step` itself only performs one step).
- `player.step`: `frame` → mpv `frame-step` / `frame-back-step`; `keyframe` → `seek ±0 relative+keyframes` (direction from the sign; confirm by testing that it "strictly passes the current keyframe"; if not, use `seek <current ± 0.001> absolute+keyframes` instead and record it in §6). No video track, generated stream, or not seekable when stepping backward: `unsupported`.
- `setStepSeek`: recorded under the caller; restored to 5 when the caller is deactivated; with several callers the last write wins.
- `host.run`: accepts only the ids in the table above, any other string is `invalid`; equivalent to running that core command (same `enabled` check, same OSD).
- Overlay migration: when `ui_state.rs` reads the overlay, rewrite the following old keys to new keys once and save: `vestra.playback/toggle-pause → host/play-pause`, `close-source → stop`, `previous → previous`, `next → next`, `backward-5 → seek-backward`, `forward-5 → seek-forward`, `rate-down`, `rate-up`, `rate-10 → rate-normal`, `volume-up`, `volume-down`, `mute`. If the new key already exists, keep the new key and drop the old one.

**Switch-over rules (workflow §4.2 item 4, §5)**

This is a breaking change: the core actions and the 12 existing playback commands use the same menu paths and keys, and registering both causes `conflict`. Do a centralized switch-over:

1. When implementing the core actions, the maintainer adds the startup switch `VESTRA_CORE_ACTIONS=1` (host environment variable). **With the switch off (the default) the host does not register these 14 actions**, and playback works exactly as before; `host.run`, `setStepSeek` and `step` are not affected by the switch and are always available.
2. TestHost adds `new TestHost({ coreActions: true })`: simulates the core action table (records calls by id, provides `host.coreAction(id)` to assert OSD and arguments), for the playback migration tests.
3. The playback author migrates with the switch on (§4 author A). The migrated playback **cannot** ship alone with the switch off, so the playback migration and the P3 switch removal land in the same commit, merged by the coordinator.
4. P3: the maintainer removes the switch, and the core actions are always registered; test fixtures that only serve the old path are removed.
5. Migration example (goes into the migration notes in the reference docs):

   ```ts
   // Before: playback registers its own commands, bottom bar buttons call the controller
   plugin.commands.add({ id: "toggle-pause", path: ["播放", "播放/暂停"], order: 201, bindings: ["Space"], run: () => controller.togglePause() });
   bar.on("playPause", () => controller.togglePause());
   // After: the core registers the commands, bottom bar buttons run core actions
   bar.on("playPause", () => plugin.host.run("play-pause"));
   ```

**Test requirements**

- Host: unit tests for the enable conditions, accumulation and clamping, and OSD text of the 14 actions (assertions ported one by one from the existing playback tests); overlay migration (old keys rewritten, new keys take precedence); the four combinations of `step` and `unsupported`; reverting `setStepSeek`.
- SDK: the error codes of `host.run`; argument validation and `ifCurrent` for `step` and `setStepSeek`.
- Menu snapshot: with the switch on, compare with `menus-after.json`; apart from new items it matches the state before the handover (same approach as `doc/process/rounds/menus-snapshot.test.ts`).

### 3.2 Playback information and phase push (R-P6, R-P7, R-P5 alternative)

- `backend.rs::stats` adds the fields listed in ruling R-P7, all passed through from mpv properties, `null` when unreadable: `video-codec` (decoder), `video-params/pixelformat`, `video-params/colormatrix`, `video-out-params/dw`, `dh`, `estimated-display-fps`, `video-bitrate`; `audio-codec`, `audio-params/format`, `audio-params/hr-channels`, `audio-bitrate`; `avsync × 1000`, `demuxer-cache-duration`. Property names follow the mpv 0.41 documentation; those that test as empty are noted in the reference docs.
- Immediate phase push: in the `backend.rs` event loop, right after the source phase (`session.rs::Phase`) changes, call `sources.snapshot` once and broadcast `source.state`; the periodic push stays the same. Test: the broadcast after a phase change does not wait for the 100 ms timer (use an injected clock or a count assertion).
- TestHost: `host.stats` already exists; its type is extended accordingly.

### 3.3 Audio tracks and subtitles (R-T1, R-T2, R-T4, R-T6, R-T7, R-T8, R-H12)

- `player.addAudioTrack(path, { select = true, ifCurrent })`: local paths go through the existing source path validation; mpv `audio-add <path> select|auto`; on success returns the new `AudioTrack` (waits until the track appears in `track-list`), on failure `backend` (mpv reason). External audio tracks have `external: true`, `exportable: false`.
- `player.selectAudioTrack(id, { ifCurrent })`: mpv `aid`; an id not in `audioTracks` is `invalid`.
- `subtitles.load` options: `format: "srt" | "ass"` (default `srt`; `ass` is recognized by content via `[Script Info]` and `[Events]`, SSA is treated as ASS); bounded ASS validation (UTF-8, ≤ 2 MiB, Dialogue ≤ 20 000), temporary file extension `.ass`; `title` (1–200 characters, otherwise `invalid`); `select: "primary" | "none"` (default `primary`, i.e. the current behavior). Changes are concentrated in `subtitle_upload.rs` and `subtitle_format.rs`.
- `SubtitleTrack.codec`, `text`, `streamIndex`: `subtitle_backend.rs` already has codec and `ff-index` when reading `track-list`; they only need to be exposed. The decision table for `text` (`subrip`, `ass`, `ssa`, `webvtt`, `mov_text`, `text` are text; everything else is `false`) goes into the reference docs. `streamIndex` per ruling 3.2 item 4: a value for embedded tracks under lavf, `null` otherwise.
- New `SubtitleStyle` fields: `bold` → `sub-bold`; `shadow` 0–4 → `sub-shadow-offset` (scaled to a 1080p reference height, the same way as the existing outline), `sub-shadow-color` fixed at 50% black; `box` 0–100 → `sub-border-style=opaque-box` + `sub-back-color` black with transparency, `null` restores `outline-and-shadow`; `assOverride` → `sub-ass-override=scale` (keep) / `force`. All optional; when omitted, behavior matches today. Changes in `subtitle_style.rs`.
- R-H12: the host no longer hard-codes `sub-ass-override=force` at startup; the default is `scale` (keep). This is a default behavior change; it goes into the change notes of the reference docs, and the GUI regression covers "embedded ASS + subtitle plugin style".
- `offsetMs` validation widened to ±600 000.
- TestHost: `host.subtitles` records the new `load` options; sample track data carries `codec`, `text`, `streamIndex`; `host.audioTracks` can be set, `addAudioTrack` / `selectAudioTrack` have default handling, and failures can be injected with `host.handle`.

### 3.4 Processing nodes (R-V1–R-V7, R-A1, R-A5)

Add nodes in `effects.rs`. Each node goes into both the live graph and the offline graph (`render_lavfi`), with range validation and runtime markers; runtime markers are given only to parameters confirmed by testing to be changeable with `vf-command` / `af-command`.

| Node | FFmpeg | Range | Runtime parameters |
| --- | --- | --- | --- |
| `hue { degrees }` | `hue=h=` | −180–180 | `h` (to be tested) |
| `sharpen { amount; size }` | `unsharp` (`small` 3×3, `large` 7×7) | amount 0–2 | none |
| `denoise { lumaSpatial; chromaSpatial; lumaTemporal; chromaTemporal }` | `hqdn3d` | 0–20 | none |
| `deblock { strength }` | `deblock=filter=weak\|strong` | — | none |
| `deband { threshold; range }` | `deband=1thr=…:r=…` | threshold 0.003–0.5, range 1–64 | none |
| `transform { rotate; flipH; flipV }` | combination of `transpose` / `hflip` / `vflip` | enum | none |
| `deinterlace { when; output }` | `bwdif=mode=send_frame\|send_field:deint=interlaced\|all` | enum | none |
| `reframe { mode; aspect; x; y }` | `crop` / `pad`, with `iw`, `ih` expressions | aspect 0.25–4, x, y −1–1 | none |
| audio `channels { mode }` | `pan` (left, right, mono, swap); with more than 2 channels, a leading `aresample=ochl=stereo` downmix | enum | none (mode changes go through reconfiguration; verify there is no noticeable audio gap) |

- `transform` applies to the picture after the host's orientation handling (`source_geometry.rs`); write tests that cover the output size when 90° is combined with container rotation.
- Audio `delayMs` widened to ±10 000 (multiples of 10 ms); the positive and negative budgets in `processing.rs` are widened to match, with boundary tests added.
- The 64-node total limit is unchanged; the worst-case chain length of the video plugin goes into the reference docs.
- SDK: `VideoNode` / `AudioNode` union types and validation; TestHost's `host.processing` can assert the new nodes.

### 3.5 Binding defects (R-H5, R-H4)

- In addition to `bindings.rs::from_slint_key`, add a path that recognizes keys by winit `PhysicalKey` (`main.rs` already gets `physical_key` in `KeyboardInput`): with Option held, match `Alt+<key>` by the physical key name instead of relying on the character Option produces; dead keys (⌥E, ⌥N, ⌥U, ⌥I, ⌥`) also trigger.
- Numeric keypad: `Numpad0`–`Numpad9`, `NumpadPlus`, `NumpadMinus`, `NumpadMultiply`, `NumpadDivide`, `NumpadEnter`, `NumpadDecimal` are separate input names; main keyboard digits do not match the keypad and vice versa. Syntax validation (in both host and SDK) is updated to match.
- Verify and fix the problem where `Ctrl+[` arrives as `\u{1b}` and is recognized as `Ctrl+Escape`.
- Tests: Option + letter, Option + symbol, dead keys, keypad vs main keyboard, `Ctrl+[`. Acceptance needs a real machine (coordinator GUI test).

### 3.6 General capabilities

**Host: clipboard** (capability `clipboard`)

```ts
plugin.clipboard.readText(): Promise<string | null>;   // null when there is no text
plugin.clipboard.writeText(text: string): Promise<void>; // ≤ 1 MiB
plugin.clipboard.writeImage(path: string): Promise<void>; // PNG/JPEG, ≤ 16 MB, absolute path
```

- Use `NSPasteboard` from `objc2-app-kit` (enable that feature; `generalPasteboard`, `clearContents`, `setString_forType`, `setData_forType`, `stringForType` are all safe methods). Types are built from the UTI strings `public.utf8-plain-text`, `public.png`, `public.jpeg`; no extern statics are read.
- Error codes: `invalid` (type, size, non-absolute path), `unavailable` (headless mode), `failed` (the system refused the write).
- TestHost: `host.clipboard: { text: string | null; images: string[] }`; `text` can be preset to test `readText`.

**Host: sleep prevention** (requires `playback.control`)

```ts
plugin.power.preventSleep(mode: "off" | "playing"): Promise<void>;
```

- It takes effect if any plugin requests `playing`; when a plugin is deactivated its request is withdrawn.
- The host holds an assertion while "the source phase is ready and not paused" (including generated streams); with video frames it starts `/usr/bin/caffeinate -d -i -w <host pid>`, with audio only `-i -w <host pid>`; when the conditions change it ends the old process and starts a new one; it ends along with the host on exit.
- Tests: the expected process arguments produced by the state machine (set of requesters × playback state × video or not); process startup uses an injectable launcher, unit tests do not actually start `caffeinate`.
- TestHost: `host.power: "off" | "playing"` (the merged effective value).

**runtime (SDK layer): external tools, settings files, folders**

Implemented in `src/sdk/plugin/`, provided through the `plugin` object, no host protocol added. Signatures in ruling 3.3.

- `plugin.tools.run(tool, args, { timeoutMs, maxOutput, signal })`: `spawn` without a shell; the executable is looked up via `VESTRA_FFMPEG` / `VESTRA_FFPROBE` / PATH (lookup result cached); timeout, `signal` and scope release all end the process (SIGTERM first, SIGKILL after 300 ms); `maxOutput` defaults to 16 MiB, exceeding it is `invalid`; `plugin.tools.available(tool)`.
- `plugin.storage.json(name, { defaults, validate, debounceMs })`: `name` must be `[a-z0-9-]{1,64}`; on a corrupt read or failed validation the file is renamed to `<name>.json.bad-<timestamp>`; `save` is debounced (300 ms by default), then temporary file + rename; `flush()`; `dispose` and plugin deactivation flush first.
- `plugin.system.reveal(path)` → `/usr/bin/open -R <path>`; `openPath(path)` → `/usr/bin/open <path>`; nonexistent is `missing`; timeout 10 seconds.
- TestHost: `host.tools.handle(tool, (args) => ({ stdout, stderr }) | HostError)` injects tool results (default `unavailable`), `host.tools.calls` records arguments; `storage` uses real files directly in `host.dataDirectory`; `host.system.revealed`, `host.system.opened` record paths.

### 3.7 Documentation

- `doc/plugins/reference.md`: all of the interfaces above; §6.3 adds "content of a bottom bar panel can be drawn into the video area above the slot (negative `y`); the host does not clip it; the sidebar and floating windows are above it", with an assertion in the composition tests of `composition.rs` (the bottom bar's outer layer has no `clip`); §4.1 binding syntax adds the keypad names and an Option note; the change notes list the default behavior changes (`sub-ass-override`, ownership of basic actions).
- `doc/plugins/examples.md`: one short example each for `host.run`, `tools.run`, `storage.json` (each at most 30 lines, the §5 P2 standard).
- Migration notes: the old id → new id table for the basic action handover and the migration example from §3.1.

## 4. Plugin adoption assignments (P2)

Author rules are the same as in round 1 ([2026-10-features-r1.md](2026-10-features-r1.md) §2): write only in your own plugin directory; implement according to the matching section of the requirements document; if an item still needs a core change, skip it and write a request per §4.1. Two additions:

- **Migrate to general capabilities**: once the general capabilities adopted this round (clipboard, sleep prevention, `tools`, `storage`, `system`) are available, the corresponding home-made implementations in plugins **must** be removed and replaced with the new capabilities; the report lists the removed files and line counts. `sips`, the keychain, `fs.watch` and system information commands are not in this list and stay.
- **Downgraded items**: for features the rulings rejected or narrowed, implement the alternative from the rulings and update the matching section of the requirements document at the same time (reviewed by the coordinator at merge).

### 4.1 Assignment table

| Author | Plugin | New features (number → dependency) | Migrate to general capabilities | Downgrades and docs |
| --- | --- | --- | --- | --- |
| A | playback | B07 previous/next frame, B08 keyframe (`player.step`); B06 first-level step (`setStepSeek`); basic action handover migration (after P1b): remove 12 commands (`toggle-pause`, `close-source`, `previous`, `next`, `backward-5`, `forward-5`, `rate-down`, `rate-up`, `rate-10`, `volume-up`, `volume-down`, `mute`), bottom bar buttons use `host.run`, remove the accumulation and clamping logic in `controller.ts` that the core takes over; rename the level 2–4 seek ids to `backward-2…4` / `forward-2…4` | `pbcopy`/`pbpaste` in `system.ts` → `clipboard.writeText` / `readText` (I08 copy, A02 prefilled link); `caffeinate` in `sleep.ts` → `power.preventSleep` (B29, remove `sleep.ts`); `settings.ts` → `storage.json` | The `enabled` of B07/B08 uses a cache of one `stats()` query after ready (ruling R-P1); I07 still waits for 3b (`ui.image`); the hover tooltips of I06 and I07 are drawn in the video area with negative `y` per R-U4 |
| B | playlist | B26 becomes "fast advance to the next item": compute the next item and `fs.stat` ahead of time, `open` only once in `onEnd` (benefits from the immediate phase push) | `finder.ts` → `system.reveal` (C23); the ffprobe child process in `probe.ts` → `tools.run("ffprobe")`; `store.ts` → `storage.json` (keep the existing `.bad` semantics) | B26 menu name and acceptance change to "same-format switch < 150 ms" (requirements document); `confirm.ts` stays this round, replaced by `dialogs.confirm` in 3a |
| C | subtitles, subtitle-online (new) | subtitles: D17 external ASS and the "original style" switch (`load({ format: "ass" })`, `assOverride`); D18 multiple formats (ASS uploaded as-is; VTT, SAMI, MicroDVD converted to SRT in the plugin; `title`); D09 bold, D11 shadow and background box (`bold`, `shadow`, `box`); D05 range ±600 000; B11 previous/next line (external tracks loaded by this plugin are parsed directly; embedded text tracks use `tools.run("ffmpeg", ["-map", "0:" + streamIndex, …])`, timeout about 1 s per 100 MB of file size, minimum 15 s; disabled when `streamIndex` is `null` or the source is a network source). subtitle-online: all of D16 (`format`, `title`, `select` of `load`; network via `fetch`; credentials in the keychain) | subtitles settings read/write → `storage.json`; new settings in subtitle-online use `storage.json` directly, child processes use `tools.run` | A08 secondary subtitles move to 3a as a whole; VTT in the D18 menu is converted in the plugin (each of the two plugins carries its own copy of the conversion code) |
| D | video, audio | video: E02 hue (`hue`), E04 sharpen/edge sharpen (`sharpen` small/large), E05 three denoise presets (`denoise` presets), E06 deblock/deband, E08 flip and E09 rotate (`transform`), E10 deinterlace (`deinterlace`), E12 stretch/crop (`reframe`); the matching controls on the "Color", "Effects" and "Picture" tabs of the four-tab window (controls for E11, E13, E16–E18 are greyed out first, with the reason shown). audio: A11 load external audio (`addAudioTrack`, including automatic loading of same-name files), G03 select audio track and G04 cycle tracks (`selectAudioTrack`), G05 channels (`channels`), G09 range widened to ±10 000 | settings read/write of both plugins → `storage.json` | E10 removes the algorithm choice (fixed BWDIF); E13 removes the four frame width/height commands; E18 frame doubling moves to 4a; E17 removes the `S` key note; G16 moves to 3a |
| E | capture, info, history | capture: no new features. info: I05 playback properties (`stats()` extended fields; file information via `tools.run("ffprobe")`; system information via `sw_vers`, `sysctl`, `system_profiler`; copy via `clipboard.writeText`; the mpv and FFmpeg version line shows "Unknown" until `plugin.app` in 3a). history: no new features | capture: `osascript` image writing in `system.ts` → `clipboard.writeImage` (F03), `open` → `system.openPath` (F09), `settings.ts` → `storage.json`; `sips` stays (F07). history: settings and record read/write → `storage.json` | info I05 "Hardware decoding" row fixed to "Not used"; I09 moves to 3a |

Plugins not in this round: window, marks, hotkeys, preferences, plugins, visualizer (from 3a). youtube, bilibili, nes and workshop have no items this round; if they have home-made settings file read/write, they may migrate it to `storage.json` along the way this round (not required; mention it in the report).

### 4.2 Entry

- Authors B, C, D, E: start as soon as the P1a review passes.
- Author A: B07, B08, B06 first level and the general capability migration start after P1a; the basic action handover migration starts after P1b and is merged in the same batch as the maintainer's P3 (§3.1 switch-over rule 3).
- At most 3 authors run at the same time (4 including the coordinator): A, C, D first, then B, E.

## 5. Acceptance

### 5.1 Static checks and tests

All commands from workflow §6.1, plus: the menu snapshot test (before/after handover comparison), and the test count per plugin before and after migration (must not decrease).

### 5.2 Real GUI tests (coordinator)

All required scenarios from workflow §6.2, plus for this round:

| Area | Scenario | Evidence |
| --- | --- | --- |
| Basic actions | After deactivating playback, Space, ←/→, D/F, ↑/↓, M, PageUp/PageDown, X/C/Z, F4 still work and the OSD text is unchanged; after reactivating, bottom bar buttons and shortcuts behave the same | Screenshot + OSD text |
| Step | After setting level 1 to 10 seconds, ← goes back 10 seconds; after deactivating playback it returns to 5 seconds | OSD |
| Frame step | During playback F pauses and advances one frame; pressing D five times goes back about 5/fps; holding F and releasing stops advancing; menu greyed out for audio only | Screenshot, time |
| Keyframe | On an H.264 source press Ctrl+Shift+→ ten times; time increases monotonically and matches the keyframes listed by `ffprobe -skip_frame nokey` | Time log |
| Option key | Alt+O, Alt+H, Alt+B, Alt+K, Alt+A, Ctrl+Alt+Y trigger on a real machine; dead-key combinations trigger; keypad 5 and main keyboard 5 are distinct | Log of physical key presses |
| Subtitles | ASS with `\pos`, `\fad` displays in its original style, and switches to the plugin style when the switch is turned off; bold, shadow and background box are visible; GB18030 SRT and VTT load correctly; previous/next line works on embedded subtitles | Screenshot |
| Picture nodes | Each of the eight nodes takes effect; the "with effects" screenshot includes these effects; dragging the hue slider does not interrupt playback | Screenshot, exported output |
| Audio | Load an external mp3 and it is selected; switch audio tracks; check left/right/mono/swap with a test tone that differs between left and right; delay −5000 takes effect | Listening notes + parameters |
| General capabilities | After copying the file name, pasting into a text editor gives correct Chinese text; after copying a screenshot to the clipboard, paste it in Preview; while playing video, `pmset -g assertions` shows Vestra's display assertion, which disappears after pausing; Show in Finder selects the right file | Paste result, `pmset` output |
| Fast advance | Gap between two same-format flac files < 150 ms (host log timestamps) | Log |
| Bottom bar not clipped | The progress bar hover time tooltip is drawn above, in the video area, visible in both fullscreen and windowed mode | Screenshot |
| Deactivation | Deactivating video withdraws only its nodes; deactivating audio withdraws the channel node, but the external audio track stays until the source changes | Screenshot |

Defect ownership is the same as in workflow §6.2. When everything passes, the coordinator commits, pushes and merges into `main`.

## 6. Records

### 6.1 Baseline

(Filled in at P0)

### 6.2 Maintainer deliveries and signature adjustments

#### P1a batch A (2026-10-20, maintainer)

**Implementation list** (each item: host + SDK types and wrappers + TestHost + reference docs + unit tests)

| Task item | Content | Host | SDK / TestHost |
| --- | --- | --- | --- |
| §3.2 | `PlaybackStats` extension (R-P6, R-P7) | `backend.rs::stats` passes through mpv properties | New wire fields optional, `player.stats()` fills in `null`; old `host.stats` usage still works |
| §3.2 | Immediate push on phase change | `backend.rs::StatePush`: a phase or source change sends `source.state` in the same cycle, the 100 ms period is unchanged; unit tests do not depend on timing | No contract change |
| §3.3 | `addAudioTrack`, `selectAudioTrack` (R-T1, R-T2) | `source_backend.rs::audio_track`, wire methods `audio.addTrack` / `audio.selectTrack` (`playback.control`) | Conditioned on `player.state` by default; TestHost default handling |
| §3.3 | `format`, `title`, `select` of `subtitles.load` (R-T4) | `subtitle_format::validate_ass` (UTF-8, ≤ 2 MiB, two sections, 1–20 000 Dialogue lines); ASS written as-is to an `.ass` temporary file; `select: "none"` only registers without selecting; `title` goes into `sub-add` | Defaults not sent on the wire; `host.subtitleLoads` |
| §3.3 | `SubtitleTrack.codec`, `text`, `streamIndex` (R-T6) | `streamIndex` only for embedded tracks with `current-demuxer == lavf` and `path` being `fd://` (the host lease path, i.e. the lavf condition of media_tracks) | Wire fields optional |
| §3.3 | `SubtitleStyle.bold/shadow/box/assOverride`, `offsetMs` ±600 000 (R-T7, R-T8) | `subtitle_style.rs`; offline screenshot `force_style` synced for Bold/Shadow/BorderStyle | Types |
| §3.3 | R-H12 | Startup default `sub-ass-override=scale` (keep) | Change notes |
| §3.4 | 8 video nodes + `channels`, `delayMs` ±10 000 (R-V1–R-V7, R-A1, R-A5) | `effects.rs` nodes can expand into several filters (labels `name@nK`, `name@nKxJ`); only `hue.h` is a runtime parameter; `processing.rs` / `offline_audio.rs` budgets widened | `nodeProblem` validation, TestHost `processing.replace` uses the same rules |
| §3.5 | R-H5, R-H4, Ctrl control codes | `bindings::from_key` + a winit hook records the physical key (`observe_key`); dead keys are resolved directly in the hook | `NAMED` adds the keypad names |
| §3.6 | Clipboard | `clipboard.rs` (safe methods of `objc2-app-kit` `NSPasteboard`, new `objc2-foundation` dependency, already in the lock file); capability `clipboard` (in both host `plugins.rs` and the build tool) | `host.clipboard` |
| §3.6 | Sleep prevention | `power.rs` (state machine with an injectable launcher); `/usr/bin/caffeinate [-d] -i -w <pid>`; "has video" = a video frame was received within the last second | `host.power` |
| §3.6 | `tools`, `storage`, `system` (runtime) | None | `sdk/plugin/local.ts` (real child processes), `services.ts`; TestHost injects fake implementations via `Transport.local` |
| §3.7 | Docs | `composition.rs` test asserts the bottom bar's outer layer has no `clip` | Reference §2, §3, §4.1, §5, §6.3, §8, §9, §13, §14, §16 (new), §17 change notes; examples §18–§20 |

The defect added by the coordinator (Ctrl+Y recognized as Shift+Tab) has been folded into §3.5: with Ctrl/⌘ held, control codes are always treated as Ctrl+letter (0x1b is `Ctrl+[`); only without modifiers are they mapped to Tab/Backspace/Enter/Escape/Shift+Tab; when a physical key is available, the physical key is used (Ctrl+Tab is still Tab; pressing a modifier alone does not count as input). The unit test `control_codes_are_ctrl_letters_only_with_ctrl` covers five codes with and without modifiers.

**Signature adjustments and implementation choices** (all signatures match the rulings; below are behavior details)

1. `box`: the task item says "`sub-border-style=opaque-box` + `sub-back-color`". In the mpv 0.41 documentation, the opaque-box color comes from `sub-outline-color` and wraps each line, and `sub-back-color` only colors the shadow box; switched to `background-box` (color from `sub-back-color`, padding from `sub-shadow-offset`, libass extension BorderStyle=4). As a result `shadow` has no effect when there is a box (both share `sub-shadow-offset`); this is written in reference §9.
2. The `channels` downmix uses `aformat=channel_layouts=stereo` (the filter graph inserts resampling automatically) instead of `aresample=ochl=stereo`; same effect, and it does not depend on swr option names; `"stereo"` renders as `anull` (channels unchanged).
3. When a `denoise` value is 0, 0.001 is sent: hqdn3d treats 0 as "derive the default from the other values".
4. `deband` uses the same threshold for all four planes (the task item only mentions `1thr`).
5. The aspect ratio of `reframe` uses the display aspect ratio (pixel width × sar); sizes are rounded to even numbers.
6. `PlaybackStats.video.decoder` comes from `current-tracks/video/decoder` (short name `h264`); the existing `codec` is already the description string of `video-codec`. Testing shows `estimated-display-fps` is `null` under libmpv software rendering and `video-bitrate` is `null` for about 1 second after playback starts; noted in the reference docs.
7. The TS wire types (`PlaybackStats` in `sdk/wire.ts`, tracks in `sdk/subtitles.ts`) make the new fields optional: plugin tests construct these wire objects directly (the info and subtitles tests), and making them required would make the existing plugins' `check` fail; the fields of the public types (`types.ts`) are complete.
8. Wire method names: `audio.addTrack`, `audio.selectTrack`, `clipboard.readText|writeText|writeImage`, `power.preventSleep`.
9. The build tool did not validate capability names before; following "keep both sides in sync", a table identical to the one in `plugins.rs` was added (`plugin -- check/build/test` fails on an unknown capability).

**Known gaps**

- Dead-key combinations (⌥E, ⌥N, ⌥U, ⌥I, ⌥\`) only trigger when the video area of the main window has focus (the winit hook cannot see the focused Slint widget; input fields need normal composition); they do not trigger in the floating sidebar window. Non-dead-key Option combinations are resolved by physical key under any focus.
- `Alt+Shift+digit/symbol` derives the symbol name from the US layout; letters come from the current layout (`key_without_modifiers`).
- External ASS tracks cannot be included in a "with subtitles" screenshot (`unsupported`), same as embedded ASS today.
- External audio tracks stay until the source changes; deactivating the plugin does not withdraw them (as ruled).
- The "has video" check for sleep prevention is based on whether a rendered frame was received within the last second: audio with cover art renders only one frame and is treated as audio only.
- When an SRT loaded by a plugin is not given a `title`, the track title is still the temporary file name (existing behavior).
- The `vendor/sdk` mirror test of workshop fails (1/17) because of the new `local.ts`, `services.ts` and other changes in the SDK; it passes after syncing with the command at the top of that test file. This is outside the maintainer's writable scope; please have the coordinator sync it (handled the same way in earlier rounds).
- capture's "one running and four queued" and subtitles' "host refusal of the upload" each failed once intermittently under high machine load and passed when rerun alone (timing-sensitive cases unrelated to this batch).

**Checks** (actually run in the worktree)

- `src/`: `npm run check` 0; `format:check` 0; `boundaries` 0; `test:sdk` 66/66 (12 new in `tests/sdk/round2.test.ts`); `plugins:build` 0; plugin tests audio 15, bilibili 15, capture 13, info 4, nes 11, playback 21, playlist 35, subtitles 19, video 10, youtube 14 all pass; workshop 16/17 (the vendor mirror above).
- `src/host/`: `cargo fmt --check` 0; `cargo clippy --locked --all-targets -- -D warnings` 0; `cargo test --locked` 176/176; `cargo check --no-default-features` 0.
- Headless testing (`--headless` + a temporary probe plugin, not committed): the new `stats` fields have values; `addAudioTrack` (external mp3) returns `{ id: 2, external, exportable: false }` and selects it, `selectAudioTrack(1)` switches back; ASS `load({ format: "ass", title, select: "none" })` succeeds without changing the selection; all eight video nodes applied together are `applied`, after transform 90° + pad 1:1 `displayWidth/Height` is 640×640; continuous `hue` value changes go through `vf-command`; `channels swap` + `delayMs −5000` `applied`; clipboard in headless mode is `unavailable`. In addition, the mpv 0.41 command line was used to verify one by one that each node's filter graph is accepted, that `vf-command … h … hue@n0` succeeds, and that `la` of `unsharp` cannot be changed at runtime.
- Net growth of host Rust: `src/host/src/*.rs` 22 770 → 24 785 lines (+2 015, including tests); SDK and runner `src/sdk/plugin/*.ts` + `src/runtime/` 4 812 → 5 565 lines (+753), plus +192 for wire types and node validation in `src/sdk/*.ts`.

**GUI test points for the coordinator**

1. Sleep prevention: after a temporary plugin or the later playback migration calls `power.preventSleep("playing")` and video plays, `pmset -g assertions | grep -i caffeinate` in a terminal should show `PreventUserIdleDisplaySleep` and `PreventUserIdleSystemSleep` (process caffeinate, `-w` is the player pid); after pausing, the assertions disappear within about 16 ms; playing audio only (mp3/flac) shows only `PreventUserIdleSystemSleep`; after quitting the player, `pgrep caffeinate` finds nothing left.
2. Clipboard: after `clipboard.writeText("中文文件名.mkv")`, pasting with ⌘V in TextEdit is correct; after `writeImage(<screenshot png>)`, "Preview ▶ File ▶ New from Clipboard" gives the same image; after copying some text in another app, `readText()` returns it; after copying an image, `readText()` is `null`.
3. Option key: with the video area focused, ⌥O, ⌥H, ⌥B, ⌥K, ⌥A, ⌃⌥Y each trigger the commands declared as `Alt+O` etc. (subtitles' `Alt+H` can be tested as is); the dead keys ⌥E, ⌥N, ⌥U, ⌥I, ⌥\` trigger `Alt+E` etc.; in an input field such as the playlist search box, pressing ⌥E then e still types é and does not trigger a command.
4. Ctrl control codes: ⌃Y (playback "Reopen Current/Last File") and ⌃I with the playlist list focused (add files) trigger; Tab, Esc, Return and Backspace alone keep their original function; ⌃[ triggers a command declared as `Ctrl+[`.
5. Keypad: declare `Numpad5` for one command and `5` for another; keypad 5 and main keyboard 5 each trigger only their own command; keypad Enter no longer triggers "Fullscreen" (`Enter`); the menu shortcut text shows `Num5`.
6. ASS keeps its style by default (R-H12): open an mkv with embedded ASS (with `\pos`, `\fad`, colored styles); subtitles display at the script's positions and colors; after the subtitles plugin sets a style, ASS still keeps its original style, and SRT tracks use the plugin style; after passing `assOverride: "force"`, ASS switches to the plugin style.
7. New style fields: `bold: true`, `shadow: 3` show visible bold text and shadow; `box: 60` shows a semi-transparent black background box (shadow gone); `box: null` restores outline plus shadow.
8. Bottom bar not clipped: put an element with negative `y` in a bottom bar component; in both windowed and fullscreen mode it is drawn above, in the video area, and covered by the sidebar when the sidebar is expanded.

#### P1a fix: Option / Ctrl combinations do not trigger on a real machine (coordinator GUI test feedback)

Symptom: ⌥O, ⌥H, ⌃Y sent through System Events do not trigger (unit tests pass).

Cause (self-checked against the Slint 1.16.1 and winit 0.30.13 sources):

- Hook installation: `bind_shell` is called again on the new instance after every rebuild of the main UI (plugin panel mounting), so the hook is present; the floating sidebar is hooked separately by `observe_keys`. Not the cause.
- Order: winit's `on_winit_window_event` filter runs before Slint's `process_key_input` within the same `KeyboardInput` dispatch, and the `key-input` callback happens synchronously after it; `physical.take()` is only taken in `key-input`, and key releases and synthetic events do not overwrite it. The assumption holds.
- **Root cause: where the modifier state comes from.** Slint does not handle winit's `ModifiersChanged`; it maintains `event.modifiers` only from press/release events of the modifier keys themselves. System Events' `keystroke … using option/control down` puts the modifier flags on the key event and does not send press events for the modifier keys (a physical keyboard does), so Slint reports `alt = false` / `meta = false`: ⌥O becomes the typed `ø`, and the `\u{19}` of ⌃Y without Ctrl is Shift+Tab by the rules. In `keyDown`, winit first sends `ModifiersChanged` based on the event's own flags and then `KeyboardInput`, so winit's modifier state is correct.

Fix: the hook records winit's modifier state at key press in `bindings::Physical.mods`, and `from_key` uses it when present instead of Slint's flags; unit test `window_system_modifiers_win_over_missing_slint_flags` (all Slint flags false, winit reports Alt / Ctrl).

Diagnostic switch: with `VESTRA_DEBUG_KEYS=1` the host prints to stderr every winit keyboard event (`keys: winit Pressed|Released synthetic=… physical=… logical=… text=… repeat=… modifiers=…`), `ModifiersChanged` (`keys: winit modifiers …`), dead-key resolution results, and every Slint `key-input` (`keys: slint scope=… text=… control/alt/shift/meta=… physical=… → resolution result bound=whether a binding exists`). If it still does not trigger in testing, judge by these lines: no `keys: winit` line = the hook did not receive it; a winit line but no slint line = the event was consumed by the focused widget or the IME; the slint line resolves correctly but `bound=false` = the input is not in the binding table (check the plugin declaration or the user overlay).

#### Ruling: semantics of a failed assertion start in `power.preventSleep` (reported by author A)

Problem: `preventSleep` only registers the request; the host starts `caffeinate` only when the playback conditions are met, and a start failure is only written to the host log; the plugin is not told, so item 5 of requirement B29, the "cannot prevent the screen saver" message, cannot be implemented.

Conclusion (§4.3 conservative principle): **no notification event; document the failure semantics.** Reference §16.2 adds: a start failure is only written to the host log (`host.error power …`), the plugin is not notified, there is no retry under the same conditions, and it is retried when the conditions change. Reason: `/usr/bin/caffeinate` is a program shipped with the system, and a start failure does not happen in practice on supported macOS; it does not meet the bar of "a general capability gap that needs host internal state"; adding an event channel (subscribe, unsubscribe, scope, TestHost simulation) for a failure that almost never occurs is not worth it. Based on this, author A downgrades item 5 of B29 (removes the "cannot prevent the screen saver" message) and notes it in the matching section of the requirements document. If a reproducible failure scenario appears later, raise it with evidence per §4.1.

#### Ruling: `system.reveal` accepts several paths (author B, playlist C23)

Request: `system.reveal(path | path[])`, one `/usr/bin/open -R a b c`; `missing` if none exist, skip the ones that do not exist. Evidence: with 3 files selected in the same folder and Ctrl+F pressed, the single-path interface can highlight only 1 (calling it once per file opens several Finder windows or leaves only the last one), while the original `open -R a b c` highlights all of them — a user-visible missing feature that the alternative cannot provide.

Conclusion: **accepted** (compatible extension, only in the runtime layer, no host protocol added). Signature `reveal(paths: string | string[]): Promise<void>`; at most 256; paths are first made absolute (so a path starting with `-` is not taken as an `open` option) and deduplicated; nonexistent ones are skipped, and if none exist it is `missing`. `openPath` is unchanged. TestHost's `host.system.revealed` records each revealed path in order (existing assertions are not affected). Implemented with the batch B commit; reference §16.5 updated.

#### P1b batch B (maintainer)

**Implementation list**

| Item | Host | SDK / TestHost |
| --- | --- | --- |
| 14 core actions (R-H1) | `core_actions.rs`: declarations (path, order, group, bindings and repeat match the current playback; new `frame-back/forward` 204/205 group `frame`, first-level seek group `step`, `rate-normal` single choice), enabled/checked/reason, execution plans and OSD text (moved one by one from `controller.ts`); `backend.rs::CoreActions` runs them in order on the control thread (the host's own operation book, owner 0); `main.rs` registers them, state is updated from `source.state` and the sequence flags, `ActionDone` shows the OSD | `host.run` |
| Switch `VESTRA_CORE_ACTIONS=1` | When off (the default) the 14 commands are not registered; `host.run`, `player.step`, `setStepSeek` are always available | `new TestHost({ coreActions: true })`, `host.coreAction(id)`, `host.coreRuns` |
| `player.step` (R-P1, R-P2) | Wire method `player.step` (`playback.control`); `frame` → `frame-step` / `frame-back-step`, `keyframe` → `seek ±0.001 relative+keyframes`; no active video (`source.state.hasVideo`, a new internal field: a selected video track that is not cover art), generated stream, or not seekable when stepping backward → `unsupported` | `host.steps` |
| `player.setStepSeek` (R-P3) | Recorded per plugin, last write wins, falls back after the plugin is deactivated; when the value changes, the two seek commands are registered again (title "Back N s") | `host.stepSeek` |
| `host.run` (R-P4) | Wire method `host.run` (`commands`); unknown id `invalid`; disabled `unavailable` (the reason as message); replies only after execution; while the same exclusive action is in flight, the duplicate is dropped and replied to directly | Refreshes `player.state` after execution |
| Overlay migration | With the switch on, at startup the 12 `vestra.playback/<old id>` keys are rewritten to `host/<new id>` and saved; if the new key already exists, the new key is kept | — |
| Docs | — | Reference §5.2 (`step`, `setStepSeek`), §5.5 (new, core action table), §4.1, §17 batch B change notes (old → new id table, migration example, switch); example §21 |

**Signature adjustments and implementation choices** (signatures match the rulings)

1. Keyframe: the task item says "`seek ±0 relative+keyframes`; if testing shows it does not hold, use `seek <current ± 0.001> absolute+keyframes`". mpv 0.41 testing (H.264 source, keyframes 8.333/16.667/24.021/32.355 …): `seek 0 relative+keyframes` going forward actually goes back to an earlier keyframe; `absolute+keyframes` (current ± 0.001) is also wrong; **`seek ±0.001 relative+keyframes` strictly passes the current keyframe in both directions** (21.5 → 24.021 → 32.355 …; 56.376 → 48.043 → 40.688 …), so it is used.
2. `frame-back/forward` show no OSD in the core actions (the existing playback commands have no matching item, and the requirements do not define the text).
3. "Latest unfinished target" for relative seeks: if the previous core seek was on the same source and mpv `seeking` is true or it was < 300 ms ago, accumulate from its target; otherwise accumulate from the current position.
4. When a core action fails (the source refuses, etc.), the message is shown on the video (same as the bottom bar); failures of `host.run` are returned to the calling plugin and not shown.
5. Volume ±5 is based on the current mpv volume on the control thread (requests run in order on the same thread, equivalent to playback's draft value rule).

**Checks**

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 69/69 (3 new in `tests/sdk/core-actions.test.ts`); plugin tests audio 22, bilibili 15, capture 22, info 9, nes 11, playback 37, playlist 56, subtitles 30, video 10, youtube 14 all pass; workshop 16/17 (the SDK changed again; `vendor/sdk` needs to be re-synced with the command at the top of the test, handled by the coordinator).
- `src/host/`: `cargo fmt --check`, `clippy --all-targets -D warnings` 0; `cargo test` 183/183 (6 in `core_actions.rs`: declarations match the playback menu, enable conditions and reasons, plans and OSD text with clamping/accumulation, the four `step` combinations and `unsupported`, step validation and format, overlay migration); builds with `--no-default-features`.
- Headless testing (`VESTRA_CORE_ACTIONS=1 --headless` + a temporary probe plugin): `play-pause` pauses; two `seek-forward` in a row 0.5 → 10.5 (accumulated); after `setStepSeek(10)`, `seek-backward` 10.5 → 0.5; `frame-forward` / `step("frame")` +1/30 s each, `step("frame","backward")` −1/30 s; `step("keyframe","forward")` twice 0.53 → 8.333 → 16.667, `backward` → 8.333; `rate-up` 1.1, `rate-normal` 1, `volume-up` 105, `mute` on; without a sequence provider `next` → `unavailable No next item`; after `stop`, `play-pause` → `unavailable`, `step` → `unsupported`.
- Host Rust: `src/host/src/*.rs` 24 785 → 26 032 (batch B and fixes +1 247, round 2 total +3 262, about 9% over the budget of about 3 000, mostly tests; removing the switch in P3 will reduce the code slightly); SDK and runner 5 565 → 5 977.

**GUI test points for the coordinator** (start with `VESTRA_CORE_ACTIONS=1`, without loading playback, or with the migrated playback)

1. Menus: "Close" (F4), Playback ▶ Play/Pause (Space), Previous/Next, Seek ▶ Previous Frame (D) / Next Frame (F) / Back 5 s (←) / Forward 5 s (→), Speed ▶ Normal (Z) / Slower 0.1 (X) / Faster 0.1 (C), Audio ▶ Volume + (↑) / Volume − (↓) / Mute (M) are all present, in the same positions as before the handover.
2. Without playback loaded: Space, ←/→ (repeat while held), D/F, ↑/↓, M, PageUp/PageDown (with playlist), X/C/Z, F4 work, and the OSD text is the same as with playback ("Paused", "Forward 5 s: 00:37 / 01:40", "Speed: 1.1x", "Volume: 85%", "Mute: On").
3. Frame step: during playback F pauses and advances one frame; pressing D five times goes back about 5/fps; with audio only (mp3) the "Previous Frame/Next Frame" menu items are greyed out.
4. Keyframe: on an H.264 source, call `player.step("keyframe","forward")` ten times; time increases monotonically and matches the keyframe times from `ffprobe -v error -skip_frame nokey -select_streams v:0 -show_entries frame=pts_time -of csv=p=0 <file>`.
5. Step: after a plugin calls `setStepSeek({ seconds: 10 })`, the menu changes to "Back 10 s/Forward 10 s" and ← goes back 10 seconds; after deactivating that plugin it returns to "5 s".
6. Overlay migration: put `"vestra.playback/toggle-pause": ["P"]` in `bindings` of the UI state file; after starting with the switch, the file contains `"host/play-pause": ["P"]`, and P plays/pauses.

#### Ruling: `system.openUrl` (author C, subtitle-online "Open in browser")

Request: `plugin.system.openUrl(url)`, accepts only http/https, opens in the default browser; no capability required. Evidence: `openPath` first checks that the path exists, so a URL is always `missing`; reference §16 requires plugins to stop calling `open` themselves, so plugins have no compliant alternative.

Conclusion: **accepted** (compatible extension in the runtime layer, no host protocol added). It meets §4.3 item 4, clause four: opening a web page is a general basic capability (help links, about pages and update pages all use it), and each plugin spawning `open` itself would be duplicated and inconsistent. Signature `openUrl(url: string): Promise<void>`; parsed and normalized as a WHATWG URL, only `http:` / `https:` allowed, ≤ 8192 characters, otherwise `invalid`; `/usr/bin/open <normalized address>`, 10-second `timeout`, failure `failed`; reachability of the URL is not checked. TestHost records it in `host.system.urls`. Implemented with the P3 commit; reference §16.5 updated.

#### Keyboard layouts (coordinator test feedback: German QWERTZ)

Confirmed: letters and symbols of Option and Ctrl combinations come from winit `key_without_modifiers` (the unmodified character of the current layout), so on QWERTZ ⌃Y (physical KeyZ) is `Ctrl+Y` and ⌥Z (physical KeyY) is `Alt+Z`, matching the keycaps; covered by unit tests. `Alt+Shift+digit/symbol`: macOS does not report the layout's shifted character while Option is held, so it can only be derived from the US layout (on QWERTZ ⌥⇧. is `Alt+>`, although Shift+. types `:` on that layout); symbol keys that the US layout does not have (ß, ´ etc.) do not match when pressed together with Option+Shift. This is a known trade-off; reference §4.1 already has a "Keyboard layouts" paragraph.

#### P3 switch removal (maintainer)

- Host: removed `VESTRA_CORE_ACTIONS` and `Host.core_actions`; the 14 core actions are always registered, and the overlay migration runs at every startup (an already migrated overlay no longer changes).
- TestHost: removed the `new TestHost({ coreActions })` option; core commands are always registered, in the new `host.coreCommands` (not in `host.commands`: the tests of audio, capture, info, nes, video and youtube enumerate `host.commands` and assert their own menus, and putting them there would make all of them fail); `host.press` and `host.conflicts` put core commands first (the core wins conflicts, consistent with the binding table).
- Docs: reference §5.5, §14, §17 drop the switch notes; §17 states "plugins that still register the old paths or old keys get `conflict` on menu registration, and their keys are taken by the core".
- Impact (tested on the plugin code before the merge): all 10 plugin test suites other than playback pass; **2 playback failures** ("menu follows the allocation table": `host.conflicts` shows conflicts of F4, Space etc. with the core actions; "previous/next go through the player sequence": PageUp/PageDown are caught by the core actions). These are exactly the old commands author A's migration removes, so it must be merged in the same batch as the playback migration. On the real host, an unmigrated playback fails to activate because of menu path conflicts.

#### Ruling: core frame step and B07, position when `step` resolves, `Window.onClose`, `storage` permissions (authors A, D, E)

1. **B07 frame step (author A): changed in the core.** Previous/next frame are now owned by the core; the behavior details (messages, boundaries, per-direction enabling) are product behavior already set in the requirements, and the plugin cannot add them (a plugin cannot change the messages and enabled state of core commands); the change is small. Implementation:
   - After completion the message is "Next frame: 00:01:23.456" / "Previous frame: …" (the position after completion, `HH:MM:SS.mmm`); if the position did not move, "Already at the last frame" / "Already at the first frame"; pressing previous frame on the first frame shows the message directly without sending a command.
   - The two directions are checked separately (`core_actions::frame_check`): no video track → "No video"; next frame needs `ready` (live streams are fine, not after playback has ended); previous frame needs `ready` or `ended` and seekable, otherwise "The current source cannot step back a frame" (generated streams, non-seekable live streams); previous frame after playback has ended uses one exact seek (current position − 1/fps) to bring the source back to `ready`.
   - Repeated triggers while one is in flight are dropped (the core action stays busy until `ActionDone`), which satisfies item 3.
2. **Position when `step` resolves (author A): now guaranteed.** The original implementation replied right after sending the mpv command, when the position might not have been updated yet. Now the host waits until `time-pos` changes and `seeking` is false (or playback reaches the end, or 1.5 seconds with no movement) before replying, and the SDK then refreshes the state, so after `await player.step(...)`, `player.state.position` is the new position. Only one step is in flight at a time; a second call is `busy`. Written in reference §5.2. Headless testing: the position read after each `step` returns is the new position (0.500 → 0.533 → 0.500; keyframes 8.333 → 16.667 → 8.333).
3. **`Window.onClose` (author E, info polling, capture burst window; author D raised the same kind of issue in round 1): accepted, small fix this round.** This is a defect in the existing contract: after the user closes the window, `opened` is still `true`, so the state is not accurate (fixed as a defect per §4.2 item 1), and two plugins need it. Implementation: the host intercepts the close request of a tool window, closes the view and sends `ui.closed` to the owning plugin; the SDK sets `opened` to `false` and calls `onClose(listener)`; `show()` reopens it afterwards. A plugin's own `close()` does not trigger it. TestHost: `host.window(id).userClose()`.
4. **`storage.json` file permissions (author E, history): accepted, default 0600.** No `mode` option (conservative): the data directory belongs to that plugin only, and there is no reason for settings files to be readable by other users; privacy data such as history is covered by the default. Temporary files are created with 0600 (and `chmod`), and keep it after the rename.

### 6.3 Author deliveries

(Filled in by each author on completion: completed items, uncompleted items and reasons, removed home-made implementations, API requests)

### 6.4 Acceptance results

(Filled in at P4)

### 6.5 Coordinator acceptance (2026-10-21)

All checks pass: check, format, boundaries, test:sdk, 13 plugin test suites (about 345 tests), cargo fmt/clippy/test (183 tests).

**GUI testing passed**:
- Core basic actions: Space, ←/→, D/F frame step, message "Next frame: 00:00:02.640".
- Keyframe seek ⌃⇧→.
- Keyboard: Option combinations (⌥A, ⌥O, ⌥H, ⌥L); ⌃Y reopen. The log confirms correct resolution under the German QWERTZ layout as well.
- Audio: ⌥A switches audio tracks.
- Video: ⌃P vertical flip; the four tabs of picture adjustment.
- Subtitles: external ASS keeps its original style (yellow text at the top left); after changing the source and selecting an embedded track, End jumps to the next line of dialogue.
- Windows: playback properties window; time span settings (after changing level 1 to 10 seconds, → goes forward 10 seconds).

**Fixed on the spot**:
- The rotation row on the video "Picture" tab and the audio "Channels and Output" tab overflowed, and the drop-down font sizes were inconsistent (author D). The cause was the built-in minimum width of fluent `ComboBox`; replaced with a compact drop-down inside the plugin.
- Host/SDK defect: after loading an external subtitle, the subtitle state of the next source was stuck in loading. Author C located it with a headless probe; the maintainer found that the SDK compared revision numbers across different selection generations, fixed it, and added a regression test (1466d87).

**Pulled forward from round 3a**: `Window.onClose` and storage 0600 were already fixed in this round (046e4cb).
