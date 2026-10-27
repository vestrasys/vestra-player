# 2026-10 round: seek-bar speed on local files

Started 2026-10-26. **This document is the only progress record for this round.** Status: research and measurements done; the host performance fix (section 5) is on main (83bf54d, #57); the user ruled on the seek-bar mode (option B, section 8); the host and SDK side of the ruling is on branch `seekbar-keyframes` (worktree `C:\vestra\wt-seekkf`, from main 83bf54d) with the SDK patch `sdk-1.5.22.patch`, waiting for the coordinator; the playback plugin's side is next (section 8.4).

## User report (2026-10-26)

Clicking or dragging the seek bar on **local** videos is sometimes slow. The user's impression: PotPlayer is fast but not very accurate; mpv seems both fast and accurate (low confidence). The user asked how this should be tuned.

Earlier rulings that apply (round `2026-10-seek-keyframes.md`): step seeks can be keyframe-only, PotPlayer-style, on by default; **no "seek twice"** for a key press (show a keyframe, then correct to the exact frame).

## 1. What a click and a drag send today

| Step | Where | What happens |
| --- | --- | --- |
| Pointer down | `src/plugins/playback/ui/bar.slint:284-286` | `dragging(true)`, then `seek-changed(fraction)` |
| Every pointer move while pressed | `bar.slint:293` | `seek-changed(fraction)` (no rate limit in the UI) |
| Pointer up | `bar.slint:287-289` | `seek-changed(fraction)` **again**, then `dragging(false)` |
| Plugin | `src/plugins/playback/bar.ts:187-189`, `controller.ts:110-131` | each `seekChanged` calls `Scrub.seek(fraction × duration)`; one `Scrub` per gesture (`startDrag`) |
| SDK `ScrubHandle` | `@vestra-player/sdk` 1.5.20 `src/plugin/player.ts` (`class ScrubHandle`), built on `Latest` | latest value wins: one `source.submit` seek in flight, newer values replace the queued one; after each acknowledgement it waits until 34 ms have passed. **No de-duplication** of equal targets. |
| Host | `src/host/src/source_backend.rs` `Control::Seek` | `seek <t> absolute+exact`. The submit is acknowledged as soon as mpv queued the command, so a drag sends **up to ~30 exact seeks per second** to mpv. |
| mpv | `player/playloop.c` `queue_seek` / `execute_queued_seek` | see section 2: a queued seek waits for the running one only 0.3 s, then replaces it |

Consequences:
- **A click is two identical exact seeks** (press and release, ~100 ms apart). mpv runs the second one from the keyframe again (section 2a).
- **A drag is a stream of exact seeks.** On a long GOP each one needs more than 0.3 s of decoding, so mpv abandons it for the next one and no new frame appears until the pointer stops (the "frozen" drag).

## 2. How mpv does it (sources at mpv e471496c56, the libmpv we ship, `src/packaging/deps.json`)

Permalink prefix **M** = `https://github.com/mpv-player/mpv/blob/e471496c56f0f49c133bd88b50aed62b845f1909/`.

a. **Seek queue.** `queue_seek` (M`player/playloop.c#L470-L511`): an absolute seek replaces the pending one; relative seeks add up. `execute_queued_seek` (M`player/playloop.c#L514-L539`): a seek with `MPSEEK_FLAG_DELAY` waits while `mp_time_sec() - start_timestamp < 0.3` and no frame of the running seek has been shown; after 0.3 s it starts anyway and the running hr-seek is abandoned. Every `seek` command sets that flag (`cmd_seek`, M`player/command.c#L5985-L6036`). A keyframe seek cancels a running hr-seek at once (`start_timestamp = -1e9`).

b. **When a seek is exact.** M`player/playloop.c#L351-L354`: `+keyframes` never, `+exact` always, no flag: exact for `absolute` (because `hr-seek=default`, M`options/options.c#L1047`) but **keyframe for `absolute-percent`** when there is video. Docs: M`DOCS/man/options.rst#L382-L398`.

c. **`hr-seek-framedrop`** is `yes` by default (M`options/options.c#L1048`, M`DOCS/man/options.rst#L413-L420`): frames before the target are decoded with `skip_frame = AVDISCARD_NONREF` (M`video/decode/vd_lavc.c#L1189-L1198`, M`filters/f_decoder_wrapper.c#L1003-L1007`), so every reference frame from the keyframe to the target is still decoded. **With a `-copy` hwdec, every decoded frame is downloaded to system memory before it is dropped** (M`video/decode/vd_lavc.c#L1390-L1391`). `hr-seek-demuxer-offset` is 0 by default (M`DOCS/man/options.rst#L400-L411`). Nothing turns framedrop off for `vf` filters or copy-back.

d. **Demuxer cache.** `cache=auto` means no cache for local files, so seeks always go to the file (M`demux/demux.c#L2655-L2672`). A cache would only save the read, not the decoding.

e. **The OSC** (M`player/lua/osc.lua#L2551-L2578`, option M`player/lua/osc.lua#L37`, docs M`DOCS/man/osc.rst#L192-L200`, `seekbarkeyframes` default `yes`):
- pointer down: `seek <pct> absolute-percent+exact` (**a click is exact**);
- pointer move while pressed: `seek <pct> absolute-percent` (**keyframe**), repeated values are dropped ("mouse move events may pile up … throw away identical seeks");
- pointer up: **nothing**. After a drag the picture stays on the keyframe of the last move.

So "mpv is fast and accurate" is: exact clicks, keyframe drags, and on the user's files the click is fast because mpv's default `hwdec=no` decodes in software (section 4, finding 2).

**Other players** (from the read-only research):
- **uosc** (`tomasklaen/uosc` 074b086, `src/uosc/elements/Timeline.lua#L124-L175`): down = `absolute+exact`; while dragging, `absolute+keyframes` when the cursor moves fast and `absolute+exact` when it moves slowly; release sends nothing.
- **IINA**: the slider sends `absolute-percent+exact` on every change (`Preference.swift#L1244`, `PlayerCore.swift#L994-L1005` at 96f605f).
- **VLC**: `input-fast-seek` (keyframe seeks) defaults to off (`src/libvlc-module.c#L646-L648`, 3.0.x).
- **MPC-HC / MPC-BE**: fast seek to the nearest keyframe is on by default (`AppSettings.cpp#L205-L206` at clsid2/mpc-hc 6898c67; MPC-BE `AppSettings.cpp#L761` at 78a0e25). Whether that covers the seek bar is unverified.
- **PotPlayer**: a VideoHelp forum post says Preferences > Playback has "jump to keyframe", checked by default (https://forum.videohelp.com/showthread.php?p=2529914). **Unverified**: there is no official documentation. This matches the user's impression (fast, not exact).

## 3. Measurements

**Harness.** A small Rust program outside the repository (`C:\vestra\seekbench`, kept outside the repository with its raw output in `out\`) drives the shipped libmpv (`C:\vestra\deps\libmpv-2.dll`) without a window, the way the host does on Windows: `vo=libmpv` with a software render context rendering every new frame at 1920×1080 RGB24, `hwdec=auto-copy-safe` (lands on `d3d11va-copy`), `keep-open`, the host's initial `vf` chain, `ao=null`, paused. Variants: **ours** (the host's options), **novf** (without the `vf` chain), **mpvdef** (mpv defaults: `hwdec=no`, no `vf`), **fd0** (ours with `hr-seek-framedrop=no`). Times are from the `seek` command to the first newly rendered frame. Single seeks: 12 fixed targets spread over 5–90 % of each clip. Drag: 30 seeks 34 ms apart (the SDK's pacing) across 25 % of the clip (at most 30 s), 4 drags per row; "updates" = new frames during the 1 s drag, "release" = last command to the final frame.

Machine: Ryzen 9 7940HS, Windows 11, decoding on the default adapter. The user and other agents were using the machine, so repeated runs differ by up to ~1.5× (worst on the 10 s GOP clips); the ordering of the options never changed. Medians are from the first full run (`matrix`) unless noted.

### 3.1 Single seeks (click), ms, median / p95

| Clip (keyframe interval) | exact, ours | exact, mpv defaults (`hwdec=no`) | keyframe, ours |
| --- | --- | --- | --- |
| H.264 4K60, 2 s | 240 / 342 | 247 / 318 | 89 / 93 |
| H.264 4K60, **10 s** | **2668 / 4156** | 522 / 884 | 52 / 90 |
| HEVC 10-bit 4K60, 2 s | 211 / 333 | 597 / 785 | 37 / 50 |
| HEVC 10-bit 4K60, **10 s** | **1711 / 3602** | 680 / 1625 | 62 / 77 |
| AV1 10-bit 4K60, 2 s | 615 / 914 | 390 / 511 | 56 / 66 |
| H.264 1080p60, 2 s | 131 / 176 | 120 / 151 | 35 / 60 |
| H.264 1080p, 10 min, 2 s | 194 / 350 | 47 / 67 | 63 / 174 |

- While playing instead of paused (ours, exact): 10 s GOP H.264 3294 / 4932, HEVC 1168 / 1788. Keyframe: 75 / 83 and 39 / 42.
- `vf` chain: no measurable effect (novf within noise of ours on every clip).
- `hr-seek-framedrop=no`: slower (10 s GOP H.264 3227 vs 2497 in the same run; 2 s GOP 413 vs 339). The default `yes`, which we keep, helps.
- `vo=null` (no render path at all) gives the same exact-seek times as ours (`d3d11va-copy` 10 s GOP H.264: 2790 / 4104), so the renderer is not the cost. Non-copy `d3d11va` could not be measured headless (mpv fell back to software without a D3D11 output).

### 3.2 A click as the bar sends it today (two identical exact seeks, 100 ms apart), ours

| Clip | one seek | two identical seeks (today) |
| --- | --- | --- |
| H.264 4K60, 10 s GOP | 2495 | 2955 (+460) |
| HEVC 10-bit 4K60, 10 s GOP | 1733 | 2208 (+475) |
| H.264 4K60, 2 s GOP | 338 | 650 (×1.9: the first lands, then the same seek runs again) |

Same process and targets per row; time to the last `playback-restart`.

### 3.3 Drag (30 seeks in 1 s), ours

| Clip | today: exact each step | host-side coalescing (one exact seek in flight) | keyframe each step | keyframe steps, exact on release |
| --- | --- | --- | --- | --- |
| H.264 4K60, 2 s | 10 updates/s, release 289 ms | 9, 320 | 22, 202 | 23, 248 |
| H.264 4K60, 10 s | **3 updates/s (frozen up to 3.1 s), release 3236 ms** | 1, 4251 | 22, 204 | 22, release ≈ one exact seek (2.5–6 s) |
| HEVC 4K60, 2 s | 9, 404 | 9, 413 | 28, 212 | 25, 269 |
| HEVC 4K60, 10 s | 3 (frozen up to 2.4 s), 2652 | 1, 5463 | 23, 218 | 15, release ≈ one exact seek (2.5 s) |
| AV1 4K60, 2 s | 4, 1021 | 4, 1152 | 26, 191 | 28, 493 |
| H.264 1080p60, 2 s | 16, 164 | 17, 187 | 29, 173 | 29, 159 |
| H.264 1080p 10 min, 2 s | 14, 247 | 11, 197 | 29, 175 | 29, 146 |

"Keyframe steps, exact on release" was re-measured with a stricter end condition (wait until mpv's `seeking` is false); in that run the 10 s GOP release took 6.3 s (H.264) and 2.5 s (HEVC), which is one exact seek on a loaded machine. Keyframe drags land up to one GOP before the pointer (for example 27.6 s → 20.0 s on the 10 s GOP clips).

## 4. Findings: causes specific to us

1. **Fixed on this branch: a click seeks twice.** The bar sends the same target on press and release; nothing dedupes it (bar, plugin, SDK `Latest`, host). mpv repeats the decode: +0.46 s on 10 s GOP 4K clips, ×1.9 on 2 s GOP 4K (3.2). The same happens at the end of every drag (the release repeats the last move).
2. **Hardware decoding with copy-back makes exact seeks on long GOPs 3–5× slower** than mpv's default software decoding (10 s GOP H.264: 2.7 s vs 0.5 s; HEVC: 1.7 s vs 0.7 s; also AV1 and the 1080p file). mpv downloads every decoded frame before hr-seek drops it (2c), and one hardware decoder runs serially where software decoding uses all cores. On 2 s GOP HEVC, hardware is faster (0.21 vs 0.60 s). This is the Windows software-render path (`hwdec=auto-copy-safe`, `renderer::hwdec(None, true)`); playback itself needs hardware decoding (CPU and battery), so this is not changed here. The Windows GPU render path (Appendix C, #51) decodes without copy-back; **re-measure exact seeks there** before deciding anything about decoding.
3. **Exact seeks on every drag step + mpv's 0.3 s rule = a frozen picture on long GOPs** (3.3). This is the design of the bar, not a host bug. Host-side coalescing (never interrupt a running exact seek) was measured and is **not** better: fewer updates and a later final frame, because each finished seek is already stale. Not implemented.
4. Not causes: our `vf` chain, `hr-seek-framedrop` (default kept), the software renderer (the same with `vo=null`), the demuxer cache, and the host's command handling (a seek reaches mpv within the 2 ms worker loop).

## 5. Change on this branch (host only, no API or behaviour change)

`src/host/src/source_backend.rs`:
- `SeekFlight`: the last `Control::Seek` sent to mpv (entry, target). A `Control::Seek` with **the same target on the same entry while mpv is still seeking** (`seeking` is true and no `PlaybackRestart` since) is accepted as usual (same ticket, stamp and status) but not sent to mpv again. Any other target, any repeat after the first landed, and every other seek are sent as before.
- The flight ends on `PlaybackRestart`, `StartFile`, `EndFile`, any non-control intent (open, restart, close), a failed command, and `Sources::seek_elsewhere()`.

`src/host/src/backend.rs`: the host's own direct seeks call `seek_elsewhere()` (keyframe step seek start, `StepPlan::Command` frame and keyframe steps), so a scrub repeat after one of them is always sent. Follow-up keyframe step seeks cannot follow a scrub seek: a `source.submit` seek ends the keyframe flight (`source_request`).

Why this is not a behaviour change: the result is the same frame at the same position; only the duplicate decode is skipped. No wire, SDK or plugin change. Expected gain: −0.3 to −0.5 s per click on long-GOP 4K files, about −50 % on 2 s GOP 4K files, and the same at the end of every drag.

Tests (`source_backend::seek_flight_tests`):
- `only_the_same_target_on_the_same_entry_repeats`, `a_restart_a_new_file_or_a_host_seek_ends_the_flight` (unit).
- `a_click_on_the_seek_bar_seeks_once` (`#[ignore]`, needs ffmpeg with libx264): opens a generated 1080p60 clip with a 10 s GOP through `Sources::dispatch`, submits the same seek twice 100 ms apart, and counts mpv `Seek` events: 1, and 1 again for the same target after it landed. With the check disabled the test fails (2 seeks), verified once.

## 6. Options for the user (needs a ruling: changes what the seek bar does)

| Option | Click | Drag | Measured (4K, 2 s GOP / 10 s GOP) | Accuracy |
| --- | --- | --- | --- | --- |
| **A. mpv style, exact on release** (recommended) | exact | keyframe on each move; one exact seek on release | click 0.24 / 1.7–2.7 s (with this branch's fix); drag 22–28 updates/s on every clip; release 0.25 / 1.7–2.7 s (one exact seek) | exact where the pointer stops; keyframes only while moving |
| A′. pure mpv OSC | exact | keyframe; nothing on release | release 0.2 s everywhere | after a drag, up to one GOP (2–10 s) before the pointer |
| **B. PotPlayer style**: a "jump to keyframes only" option for the seek bar too | keyframe | keyframe | click 0.04–0.09 s on every clip; drag 22–29 updates/s, release 0.2 s | up to one GOP before the pointer (10 s on the gop10 clips) |
| C. stay exact, only faster | exact | exact | click as A; drag stays frozen on long GOPs (3 updates/s, release 2.7–3.2 s) | exact |

Notes:
- A is a drag preview followed by one exact landing. That is close to the "seek twice" the user rejected for key presses (each press = one seek of one kind), so it needs the user's explicit OK. A′ and B have no second seek.
- A and B can be combined: B's option off = A, on = all keyframe. That mirrors the step-seek option of 0.2.1.
- C needs no ruling; it is this branch. The bigger lever for C is decoding (finding 2), to be re-measured on the GPU render path.

**Who changes what.** A, A′ and B need a keyframe seek through the scrub, which the scope guard of the keyframe round deliberately left out (`2026-10-seek-keyframes.md`, maintainer reply §5). Smallest shape: an **addition** `Scrub.seek(seconds, { keyframes?: boolean })` (SDK patch, `@player` next host version), the wire field `control: { kind: "seek", seconds, keyframes? }` and `Control::Seek { seconds, keyframes }` in the host (`absolute+keyframes`), and the playback plugin chooses per call (`bar.ts`: keyframes on moves, exact on press/release for A) and sets `minPlayerVersion`. No removal or change to existing calls; the user-visible behaviour change is the reason for the ruling. Option B also adds a setting to the playback plugin (like "Jump to keyframes only").

**Question for the user:** On the seek bar, should a drag show keyframes and land exactly where you let go (A, like mpv but exact at the end), or should the bar always jump to keyframes like PotPlayer (B, fastest, up to one keyframe interval off), or stay exact everywhere (C)?

## 7. GUI check list (coordinator; this branch's host)

Clips: `C:\vestra\clips\h264-2160p60-gop10.mp4`, `hevc-2160p60-10bit-gop10.mp4`, `h264-2160p60.mp4`.
1. Click the seek bar at several places on the 10 s GOP H.264 clip, paused and playing. Expect: the picture arrives about 0.3–0.5 s sooner than on main; the time shown matches the clicked point (exact).
2. On `h264-2160p60.mp4` while playing: a click lands once, with no small jump back right after it lands (before: the second identical seek replayed the same point).
3. Click the same spot twice, about 1 s apart: both clicks seek (the second goes back to the spot).
4. Drag and release on all three clips: the final position is the release point, exact, as on main. (The drag itself is still frozen on the 10 s GOP clips; that is option C.)
5. While pressing the bar, press → (keyframe step) and release without moving: the position ends at the bar point (the host's own seek ends the dedupe).
6. Arrow-key steps, D/F frame steps, Previous/Next Keyframe, Jump to Time, A-B loop: unchanged.
7. Log check (`VESTRA_DEBUG_KEYS` not needed): no `host.error` while clicking.

## Mac test list (CI paused)

- `cargo test --locked -- --include-ignored a_click_on_the_seek_bar_seeks_once` on macOS (needs ffmpeg with libx264 on PATH).
- GUI items 1–4 above on macOS (GPU render path, `videotoolbox`): the duplicate seek happens there too, so the click should also be faster.
- Section 8.4 items 1–4 on macOS once the playback plugin uses the option.

## 8. Seek-bar keyframe mode: §4 record (2026-10-27)

### 8.1 User ruling (2026-10-27, binding)

The seek bar follows the **same "Jump to keyframes only" switch as the arrow keys** (playback 0.2.1 `settings.seekKeyframes`, default on): switch on → clicks and drags on the bar seek by keyframes (option B, PotPlayer style, no second exact seek); off → exact, as today. No new option, no "seek twice". This supersedes the question in section 6; option A is not done.

### 8.2 §4.1 request (requesting plugin `vestra.playback`, `src/plugins/playback/bar.ts` → `controller.ts` → `Scrub.seek`)

- **User action and expected result:** with the switch on, a click or a drag step on the bar shows the keyframe at or before the pointer at once (0.04–0.09 s on every measured clip, section 3.1) instead of decoding to the exact frame (0.2–2.7 s), and a drag keeps updating on long-GOP files (22–29 updates/s instead of 3, section 3.3).
- **Why the existing API cannot do it:** `Scrub.seek(seconds)` and `player.seek` are always `absolute+exact` in the host (`Control::Seek`, scope guard of the keyframe round, `2026-10-seek-keyframes.md` §5). `setStepSeek({ keyframes })` affects only the two step actions. A plugin cannot emulate a keyframe seek: it has no keyframe index (the host reads none either; mpv finds the keyframe in the demuxer during the seek) and no seek mode on the wire, so the only alternative, "seek exactly to a guessed keyframe time", is both wrong and as slow as today.
- **Desired usage:** `scrub.seek(seconds, { keyframes: settings.seekKeyframes })` on press, every move and release; one mode for the whole gesture.
- **Ownership and failure:** the gesture stays the plugin's `Scrub`; the host owns the mode per seek and the duplicate skip. Failure, cancellation and a missing capability are as for `Scrub.seek` today (`false` when superseded, a rejection on a host failure). An older player ignores the option and seeks exactly.
- **Contract:** no existing call changes; plugins that do not pass the option are unaffected. Affects `vestra.playback` only.
- **Minimal acceptance:** section 8.4.

### 8.3 Maintainer reply and ruling (§4.2, §4.3)

- **Classification:** capability gap; the host has the state (mpv's seek modes) and the plugin cannot have it.
- **Alternative considered:** plugin-side emulation (compute a keyframe time, seek exactly to it). Rejected: no keyframe positions are available to a plugin, and an exact seek to a keyframe time still decodes from the demuxer's seek point, so it is not faster. Reducing the requirement (option C, exact only) was measured in section 3 and is what the user rejected.
- **Ruling: addition** (§1.6 rule 1, patch release), in the smallest shape named in section 6:
  - **Wire:** `control: { kind: "seek", seconds, keyframes?: boolean }`; the SDK sends `keyframes` only when true.
  - **Host 0.5.3:** `Control::Seek { seconds, keyframes }` (`session.rs`, `#[serde(default)]`, written back without the flag when false, so the operation-book shape plugins read is unchanged); `source_backend.rs` sends `absolute+keyframes` or `absolute+exact`. `SeekFlight` keys on **(entry, target, mode)**: a press-and-release in one mode still seeks once, and an exact seek to the target of a keyframe seek is always sent (keyed on the target alone it would be skipped and the picture would stay on the keyframe). `backend.rs` is unchanged: the host's own seeks still call `seek_elsewhere()`, and a scrub seek of either mode still ends a keyframe step flight (`source_request`), so `relative+keyframes` accumulation and `key_seek` are as before. `main.rs` traces `seek+keyframes` in the `source.submit` input trace.
  - **SDK 1.5.22:** `Scrub.seek(seconds: number, options?: { keyframes?: boolean }): Promise<boolean>`; `TestHost.seeks` records `{ seconds, keyframes }` and lands on the exact target in both modes; CHANGELOG "Who must change: nobody"; docs *Playback*; also the pending *UI* pitfall patch (an `if` that flips under the pointer). Patch file: `sdk-1.5.22.patch` in the maintainer's scratchpad (applies on SDK main 1.5.21, `1200200`), verified in a fresh clone with `npm ci`, `check`, `format:check`, `lint`, `docs:check`, `examples`, `test` (191 tests, 190 passed, 1 skipped) and `npm pack --dry-run`.
  - **Not added:** a `keyframes` option on `player.seek` (Jump to Time, chapters, bookmarks, the A-B loop stay exact; nobody asked), and no new playback setting.
- **Tests (host):** `source_backend::seek_flight_tests::only_the_same_target_on_the_same_entry_in_the_same_mode_repeats`, `the_seek_flag_maps_to_the_mpv_mode` (wire default, boolean check, write-back shape, mpv arguments); the `#[ignore]` real-mpv test `a_click_on_the_seek_bar_seeks_once` now also seeks the bar in keyframe mode to 15.0 on the 10 s GOP clip (one seek for press and release, lands on the keyframe at 10.0) and then sends a keyframe seek and an exact seek to 17.0 back to back (lands at 17.0). The real-mpv test asserts landings: with the flight keyed on the target alone it still passed once on this machine, because mpv had not started the keyframe seek (`seeking` false) when the exact one arrived 1 ms later, so the mode key is proved by the unit test, not by timing. `tests/sdk/host-agreement.test.ts` checks that `session.rs` reads `keyframes` as an optional boolean, that `source_backend.rs` maps it to the two mpv modes, and, once the installed SDK is 1.5.22 or later, that `Scrub.seek` sends the field only when true (with 1.5.20 installed it checks that no seek control carries the field, so nothing fails before the upgrade).

### 8.4 Acceptance

**Playback author** (`vestra.playback`, after the coordinator upgrades `src/package.json` to SDK 1.5.22):

1. `controller.ts`: every `scrub.seek(target)` of the bar gesture (press, moves, release) passes `{ keyframes: settings.value.seekKeyframes }`; the mode does not change inside one gesture. Tier seeks, Jump to Time, Jump to Start and `reopen.ts` stay exact unless the author decides otherwise within the ruling (the ruling covers the bar).
2. `manifest.json`: `"minPlayerVersion": "0.5.3"`.
3. Tests with `TestHost.seeks`: with the switch on, a click records `{ seconds, keyframes: true }`; with it off, `keyframes: false`; the shown position while dragging is the draft (the host reports the keyframe's time after landing).
4. Settings text: the "Jump to keyframes only" description says it covers the arrow keys and the seek bar.
5. Until the upgrade, nothing in the player fails: `npm run test:sdk` passes with SDK 1.5.20 (the host-agreement test is gated on the installed version), and the plugin cannot compile the new option, so its side waits.

**Coordinator:**

1. SDK: apply `sdk-1.5.22.patch` on SDK main, commit, tag `v1.5.22`, push (publish.yml publishes); then in `src/`: `npm install @vestra-player/sdk@1.5.22`, run §6.1.
2. Player: merge `seekbar-keyframes` (host 0.5.3) with the SDK upgrade and the playback change; `cargo test --locked -- --include-ignored a_click_on_the_seek_bar_seeks_once` once on Windows (`-Ffmpeg`) and on the Mac list.
3. GUI (clips of section 7; switch on): a click on the 10 s GOP clips shows a picture within ~0.1 s and the time shown is up to 10 s before the click point; a drag on them updates continuously and stops on a keyframe; switch off: items 1–4 of section 7 as before (exact). Arrow keys, D/F, Previous/Next Keyframe, Jump to Time, A-B loop: unchanged in both modes. No `host.error` in the log.
