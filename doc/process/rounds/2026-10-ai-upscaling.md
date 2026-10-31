# 2026-10 round: AI upscaling experiment (RTX Video Super Resolution)

Branch-only work: built on the Windows wgpu branch (`windows-wgpu-render-2`). Windows GPU rendering stays off main (user ruling), so this does too.

## 1. Ruling (user, 2026-10-27)

- No "DLSS 5" plugin: the community tools rely on a leaked runtime and a caller-check shim, and score below bicubic on video.
- Instead an AI upscaling experiment with NVIDIA RTX Video Super Resolution (VSR), Windows + NVIDIA only, planned by the coordinator.
- Product shape: a plugin whose options open from the context menu; on other systems or without a supported NVIDIA GPU the whole item is greyed with the reason; it installs everywhere.

## 2. Research (researcher, 2026-10-28; sources in the report, summarised here)

- The bundled libmpv (`v0.41.0-1227-ge471496c5`, shinchiro 2026-10-21) has `vf=d3d11vpp` with `scale=` and `scaling-mode=nvidia` (mpv 0.39+). mpv sets the NVIDIA PPE stream extension (`{d43ce1b3-1f4b-48ac-baee-c3c25375e6f7}`, version 1, method 2, enable 1); on failure it only warns and keeps scaling normally.
- `scale=1` (the default) bypasses the video processor entirely: the factor must be computed from the video area and the video size, and the processor is rebuilt when it changes.
- It works with the libmpv render API on a client device, as long as hardware decoding is `d3d11va` on that device (not copy-back); the branch's D3D11 device has video support and multithread protection.
- mpv reports no "VSR is active" signal: "enabled" means only that the call succeeded. Evidence of it working: pixels differ from `scaling-mode=standard`, better SSIM/PSNR against the 4K source, NVIDIA GPU 3D load, the NVIDIA app's status indicator.
- Hybrid laptop: VSR runs only on the NVIDIA GPU. The branch keeps one adapter for the whole process (windows and mpv share textures by LUID), so VSR means running the whole player on the RTX 4070 (`VESTRA_GPU_ADAPTER`, or the Windows per-app GPU preference) and letting Windows copy frames to the 780M panel. That copy has never been measured.
- Support query (VLC's approach): NVIDIA adapter (VendorId `0x10DE`), driver WDDM version above 15.3000 (530+), then `VideoProcessorGetStreamExtension(PPE)` on a test processor returns non-zero. Chromium additionally disables VSR on battery. Not detectable: the NVIDIA app switch, quality level, battery policy.

## 3. Plan

| Step | Owner | Content | Done when |
| --- | --- | --- | --- |
| V0 | Maintainer | `gpu-probe` gains `--vf <filters>` and prints mpv's `vf`/`vd`/`d3d11vpp` log lines and `video-out-params`; offscreen, no window | probe builds; tests |
| V1 | Coordinator (hardware, ≤ 60 s per run) | 1080p reference from the 4K clip; on the 4070: `d3d11vpp=scale=2:scaling-mode=nvidia` vs `scaling-mode=standard`, PNG of one frame each, SSIM/PSNR against the 4K frame, 4070 3D load; on the 780M: the same with `nvidia` (expect a warning and identical pixels) | evidence that VSR changes the picture on this machine, or that it does not |
| V2 | Coordinator | the branch player on the 4070 (`VESTRA_GPU_ADAPTER`), windowed and maximised, 4K60 and 1080p60: presented fps and the cross-adapter copy cost (frame log, GPU engine counters) | numbers for the hybrid case |
| V3 | Maintainer | host capability, OS-independent: "video upscaling: available / unsupported + reason" (the Windows platform module does the query above; other systems return `unsupported`); the filter appended last in the chain with the factor from the video area; SDK addition (user approved the capability in principle for this feature) | tests; review |
| V4 | Plugin author | the plugin: context-menu options (off / on, quality note), greyed with the host's reason; documentation of what users must enable in the NVIDIA app | review; coordinator GUI check |

Stop after V1 if the picture does not change on this machine, and after V2 if the hybrid copy costs too much; report to the user either way.

## 4. Risks

The whole process moves to the discrete GPU (restart needed to switch; power and heat); no reliable "active" signal; the factor must follow the video area; 10-bit/HDR sources may need conversion to NV12 first; lavfi filters before `d3d11vpp` may force frame downloads; the support query wakes the discrete GPU (do it when the menu opens, cache the result); NVIDIA has no first-hand documentation of the input range or laptop limits; community reports of crashes after a few minutes (mpv PR #14155).

## 5. V1 result (coordinator, 2026-10-28, `vsr-check.ps1`, three offscreen runs of 15 s)

| Run | Adapter | mpv log | 4070 3D load | Pixels |
| --- | --- | --- | --- | --- |
| `scaling-mode=nvidia` | RTX 4070 Laptop | "NVIDIA RTX Super Resolution enabled." | 0 % | identical to `standard` (PSNR inf) |
| `scaling-mode=standard` | RTX 4070 Laptop | — | 4.4 % mean | — |
| `scaling-mode=nvidia` | 780M | "NVIDIA RTX Super Resolution enabled." | — | AMD's own scaler |

- The extension call succeeds on both GPUs (the AMD driver also accepts the NVIDIA GUID), so "enabled" proves nothing; the support query must use `VideoProcessorGetStreamExtension` as VLC does.
- On the 4070 the picture did not change: VSR was not active. Likely causes, not yet told apart: the NVIDIA app's RTX Video "Super Resolution" switch is off (it is per user and the probe cannot read it), or the driver applies VSR only to presented video (the probe renders offscreen). Power: on AC.
- Next: the user checks NVIDIA app ▸ System ▸ Video ▸ RTX Video Enhancement ▸ Super Resolution (on, quality 4, status indicator on); then the same runs again, and a presented run (the branch player on the 4070) if offscreen stays identical. Per §3 the experiment stops if it still does not change.

## 6. V1 again with Super Resolution switched on (2026-10-28)

The NVIDIA app had RTX Video Super Resolution **off** (default). The coordinator switched it on (quality High, status indicator on) and repeated the runs on real footage (Big Buck Bunny 4K60, a 10 s excerpt; 1080p reference made with `scale=flags=area`; `d3d11vpp=scale=2`, 4070):

| Output vs the 4K frame | SSIM (All) |
| --- | --- |
| `scaling-mode=nvidia` (VSR) | 0.9856 |
| `scaling-mode=standard` | 0.9931 |
| ffmpeg bicubic (CPU) | 0.9949 |
| ffmpeg lanczos (CPU) | 0.9947 |

- VSR now changes the picture (on the synthetic bars clip: PSNR 36.8 dB between VSR and standard, previously identical). Crops show visibly sharper fur and edges than standard; fidelity metrics score it lower, as expected for a detail-synthesising scaler. The 3D engine counter stays at 0 % during VSR (it runs on the video engine), so 3D load is not a usable signal.
- V1 passes: VSR works on this machine once the user's driver switch is on. The plugin must say so ("turn on Super Resolution in the NVIDIA app"); we cannot read the switch.

## 7. V2: the branch player on the 4070 (2026-10-28, `VESTRA_GPU_ADAPTER=1`, H.264 2160p60 windowed, panel on the 780M)

Both the windows (wgpu D3D12) and mpv (D3D11) on the RTX 4070; Windows copies the frames to the 780M panel. Per second: produced 60, shown 59–61, presented 55–61 (one second at 50 during start-up); `ui-gpu` draw mean 3.7–3.8 ms, mpv render 0.9 ms. On the 780M the same clip presented 39–42 (wgpu record §16). The cross-adapter copy does not stop 60 fps; the remaining drops (1–5 frames a second) are for the wgpu round's pacing work.

## 8. V3: the host capability and the SDK addition (API maintainer, 2026-10-31)

Branch `upscaling-v3` (from main `bcada4e`, host 0.5.14 with M3) and SDK branch `upscaling-137` (from 1.5.36); nothing committed. Versions: **host 0.5.16** (0.5.15 is the CPU-fix maintainer's), **SDK 1.5.37** (prepared, not tagged). The coordinator reconciles the host number at merge.

### 8.1 API

Capability **`video.upscaling`** ("Upscale the video with the GPU's video upscaler"), last in `capabilities.json`. **Not risky**: it changes only how the picture is scaled while the plugin runs, reads and stores no data, and a stopped plugin's request is revoked, like a `processing.video` filter (not risky either); the GPU's power use is what the user turns it on for. Residency `live` (the filter lives as long as the plugin). `requires` nothing. BUILTIN tables untouched (no built-in command).

| Request | Answer |
| --- | --- |
| `video.upscaling.state` | `UpscalingState` |
| `video.upscaling.set { enabled }` | `UpscalingState`; `invalid` for a non-boolean; `unsupported` for `true` where it cannot work in this run |

`UpscalingState` = `{ available, reason: { code, message } | null, enabled, requested, factor }`: `enabled` is this plugin's own request, `requested` that the player has the filter in the chain now (for any plugin), `factor` its scale. SDK: `plugin.video.upscaling.state()` / `.set(enabled)` (`VideoUpscaling`, `UpscalingState`, `UpscalingReason`, `UpscalingReasonCode`), `TestHost.upscaling`, the concept page _Video and audio processing_, _Platforms_, the generated capability and player-version tables, `llms*.txt`, CHANGELOG ("Who must change: nobody").

Reason codes, in the order checked (first failure wins); the codes name no system, the messages do:

| Code | When | Fixed for the run (`set(true)` refused) |
| --- | --- | --- |
| `platform` | not Windows | yes |
| `no-gpu` | no NVIDIA adapter (`0x10DE`, not software) | yes |
| `driver` | NVIDIA user-mode driver below `x.x.15.3000` (530; the message gives NVIDIA's number, `531.18`) | yes |
| `gpu-unsupported` | `VideoProcessorGetStreamExtension(PPE)` on a 1080p → 2160p test processor fails or returns 0, or the check itself failed | yes |
| `rendering` | GPU rendering is not drawing now. The message carries the rendering reason and what to do, from the host's `rendering.state`: "turn on Video ▸ GPU Rendering (Experimental) and restart the player"; "restart the player" when the preference is already `gpu`; the GPU reason when `gpu.available` is false; "stopped during this run" after a fallback | yes, except while GPU rendering is still starting (`pending`) |
| `adapter` | GPU rendering runs on an adapter the check did not pass for (the window device's LUID): "choose the high-performance GPU for Vestra in Windows Settings ▸ System ▸ Display ▸ Graphics and restart the player" | yes |
| `decoding` | mpv's `hwdec` is a copy-back mode (for example while a plugin's filter needs frames in memory) or `no`, or `hwdec-current` is not `d3d11va` | no: the request is kept and applied when it passes |
| `unchecked` | only in the answer to `set(false)` before any check (it never starts one; review L3) | — |

No "active" signal: `requested` only means the filter is in the chain. The docs (the type, the concept page, _Platforms_) say the NVIDIA app's Super Resolution switch must be on and cannot be read.

### 8.2 Design

- **Platform split.** `platform::upscaler_probe`, `gpu_render_adapter`, `upscaler_filter`, `UPSCALER_HWDEC` (a row in the `platform/mod.rs` table). Windows (`platform/windows.rs`): DXGI adapters → NVIDIA ones → driver check → `vestra_win32::video_super_resolution(luid)` (new; the only new `unsafe`, each block with a SAFETY comment; the crate's `windows` dependency gains the Direct3D 11 features). Elsewhere `no_upscaler` answers `platform` without touching the GPU. `vestra_gpu_render::window_adapter()` (new, safe) gives the window device's LUID and name.
- **Lazy, cached check.** The control worker (`playback/upscaling.rs`) starts the check on its own thread at the first request and keeps the answer for the process; requests wait for it and are answered from the worker's poll. Nothing runs at start-up. The rest (rendering, adapter, decoding) is read at each request and every 200 ms while upscaling is asked for.
- **Filter.** `@vestra-upscale:d3d11vpp=scale=F:scaling-mode=nvidia`, last in `vf`. F = the display size (`video-params` `dw`×`dh`, before the filter chain: a test confirms `video-params` does not see the chain's output, so the upscaler never feeds its own factor; swapped for a quarter rotation) fitted into the render target (`osd-dimensions`), the larger axis over the storage size (`w`×`h`), rounded to hundredths, capped at 8192 px a side; none at ≤ 1 (`scale=1` bypasses the processor). A first filter and a removal happen at once; a factor 2 % or more off replaces the filter in place once it held 300 ms; a filter that is no longer last (a plugin's first `@vestra-owned` chain is appended after it) is moved back. A factor mpv refused is not retried until the factor changes. `VESTRA_UPSCALER_MODE=standard` (test switch) uses `scaling-mode=standard`, for case U5.
- **Ownership.** A set of owners: the filter stays while any plugin has it on; `set(false)` and `owner.disabled` (plugin stop) remove the owner at once, and the filter with the last one; a stopped plugin's pending requests are dropped; a later `set(false)` wins over an earlier `set(true)` still waiting for the check.
- **Host side.** `capabilities/video_upscaling.rs` forwards only `enabled`, plus the host's `rendering.state` (`Host::rendering_state`, now `pub(crate)`), to the worker; a plugin cannot supply the rendering facts.
- **`backend.rs`: three small hunks, nothing else touched there** (for the merge with the CPU-fix maintainer's wait-loop change): (1) `let mut upscaling = crate::playback::upscaling::Upscaling::default();` after `let mut actions = CoreActions::default();`; (2) `upscaling.poll(&mpv, &out_render, &events);` after `jobs.poll();` at the top of the worker loop; (3) before `if command.method == "owner.disabled"` in the command loop, `if upscaling.command(&mpv, &out_render, &command, &events) { continue; }` (it returns `false` for `owner.disabled` after releasing, so the existing handling still runs). **After the merge with 0.5.15** (the worker waits on `backend::Wake`): (1) also hands the worker's `Wake` to the check thread (`Upscaling::wake_with`), so waiting requests are answered as soon as the check ends; (4) the wait's timeout is also capped by `Upscaling::due_in`: nothing while nobody asked for upscaling and no filter is in the chain, else the next 200 ms look at the size and the decoding or the end of a new size's 300 ms settle, whichever is first. A deadline, not `polling`: a resize does not wake the worker, and a timer of about 5 wake-ups a second while upscaling is on costs far less than a 2 ms poll; the 100 ms state push is not relied on.
- **Hardware test plugin** `src/tests/hardware/upscaling-probe` (`test.upscaling-probe`, load with `--plugin`): Video ▸ Upscaling Probe ▸ State / On / Off (Ctrl+Alt+U / I / O); every answer goes to the OSD and to the log as `upscaling-probe <step>: {…}`. Not a product plugin.

### 8.3 Checks (Windows, this machine; no GPU run)

Host: `cargo fmt --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings`, also with `--target aarch64-apple-darwin`; `cargo test --locked --workspace` (vestra-host 600 passed, 13 ignored; vestra-win32 5 passed, 3 ignored; vestra-gpu-render 11 passed). New tests: the factor (fill, letterbox, anamorphic, cap, ≤ 1), the reason order and messages, the settle steps, the filter last, in place and moved back with a real libmpv (`vo=null`), `video-params` before the chain, `set(true)` refused or kept, a later `set(false)` wins, a stop revokes; Windows: the driver threshold and NVIDIA's version text, the filter text; win32: the supported rule, a missing LUID is an error. `node scripts/check-unsafe-policy.mjs` ok; typos clean.
`src/` with the SDK branch (`npm install --no-save C:\vestra\sdk`; the workshop's own `npm ci`): `check`, `format:check`, `boundaries`, `portability` pass; `test:sdk` 58 of 59. The one failure is expected: _CAPABILITY_INFO's store wording is the store's_, because `plugins/store/labels.ts` has no `video.upscaling` label yet (the store author's file, 8.5). New agreement tests: `WireUpscalingState`'s fields are `Upscaling::state`'s, `UpscalingReasonCode` is the host's codes. SDK repository: `npm ci`, `check`, `format:check`, `lint`, `test` (252 passed), `doc-lint`, `docs:check`, `docs` (TypeDoc), `docs:llms -- --check`, `npm pack --dry-run`, typos pass.

### 8.4 Hardware cases for the coordinator (each ≤ 60 s, one at a time, stop on any hang or driver reset)

Common: this branch's player in release, `. C:\vestra\env.ps1 -Ffmpeg`, `VESTRA_FRAME_LOG=1`, `--plugin src\tests\hardware\upscaling-probe`, a 1080p60 clip (the V1 Big Buck Bunny 1080p reference), the usual diagnostics log. "GPU mode" is `VESTRA_RENDER=gpu`; "on the 4070" adds `VESTRA_GPU_ADAPTER=high-performance`. The NVIDIA app's Super Resolution switch on (as in §6).

| Case | Setup | Steps | Expected |
| --- | --- | --- | --- |
| U0 check only (read-only, no window) | — | `cargo test --release -p vestra-win32 -- --ignored --nocapture super_resolution_on_each_adapter` | 4070: `result 0x00000000, value ≠ 0, supported true`; 780M: `supported false` or an error; Basic Render: an error. **If the 780M says supported, stop**: the check does not discriminate. |
| U1 CPU mode (default) | no `VESTRA_RENDER` | open the clip; State; State again; On | `available false`, `rendering`, the message ends "turn on Video ▸ GPU Rendering (Experimental) and restart the player"; one `upscaling: RTX Video Super Resolution is offered on NVIDIA GeForce RTX 4070 Laptop GPU …` log line (the second State adds none: cached); On → `unsupported` with the same message; no `upscaling: d3d11vpp` line |
| U2 GPU mode on the 780M | `VESTRA_RENDER=gpu` | open the clip; State | `available false`, `adapter`, naming the 780M and the 4070 and the Windows Settings path |
| U3 GPU mode on the 4070, windowed | `VESTRA_RENDER=gpu`, `VESTRA_GPU_ADAPTER=high-performance`, a window smaller than 1920×1080 | open the clip; State; On | `available true`; after On `enabled true`, `requested false`, `factor null` (the area is smaller than the video); no filter line |
| U4 maximised | as U3, On | maximise; wait 1 s; State; 10 s of frame log | `upscaling: d3d11vpp=scale=F:scaling-mode=nvidia (was off)` once, F = area height / 1080 (±0.01); libmpv's `vf`/`d3d11vpp` lines include "NVIDIA RTX Super Resolution enabled."; State `requested true`, `factor F`; presented ≥ 55 fps; `hwdec-current` d3d11va (stats overlay). Restore and maximise again: one replacement per settled size, none during a border drag |
| U5 pixels vs standard | as U4, paused at 0:05 | screen capture of the video area in the default run and in a run with `VESTRA_UPSCALER_MODE=standard`, same crop; compare | PSNR between the two < ∞ (about 35–40 dB as in §6; identical pixels mean VSR is not active: check the NVIDIA app switch); the NVIDIA status indicator shows Super Resolution in the default run only |
| U6 off | as U4 | Off; State | `upscaling: off` at once; `enabled false`, `requested false`; presented fps the same or better |
| U7 decoding | as U4 | Video ▸ a picture adjustment (brightness up: a filter that needs frames in memory); State; reset the adjustment; State | `upscaling: off`, State `decoding` (copy-back); after the reset the filter comes back by itself (`upscaling: d3d11vpp…`), `requested true` |
| U8 a plugin stop revokes | as U4 | stop the probe plugin (Options ▸ Plugins, or end its node process) | `upscaling: off` within 1 s; playback goes on |

### 8.5 For other roles (not done here)

- **Store plugin author**: `plugins/store/labels.ts` needs `"video.upscaling": { text: "Upscale the video with the GPU's video upscaler" }` (not risky) and its translations; until then `test:sdk`'s store wording test fails (as for `rendering.manage` in Store 0.2.4).
- **V4 plugin author**: declare `video.upscaling` and `"minPlayerVersion": "0.5.16"`; call `state()` when the item is first needed (the first call wakes the discrete GPU), not on a timer; handle `unsupported` from `set(true)`; say next to the item that Super Resolution must be on in the NVIDIA app.
- **Coordinator at merge**: the host version (0.5.16 here, 0.5.15 for the CPU fix); the three `backend.rs` hunks next to the wait-loop change; `src/package.json` to SDK 1.5.37 once tagged.
- **Lazy activation, later (review L6)**: `video.upscaling` is residency `live` on purpose. When idle stop comes (MV3-style ruling), a plugin holding upscaling on must not be parked: stopping it revokes its request (`owner.disabled`) and the picture changes under the user. Idle stop must treat an `enabled` upscaling request like any other live resource.

### 8.6 Review fixes (2026-10-31, after `1bce48d`)

- **M1**: returning to a factor mpv refused (refused B → size C, a settle starts → back to B within 300 ms) left `wanted` set, so `due_in` stayed 0 and the worker's `Wake` wait spun until the size changed. Both "refused" arms of `Upscaling::step` now clear `wanted`; test `returning_to_a_refused_factor_leaves_no_settle_and_no_spin`.
- **L1**: the SDK's `set` `@throws`, the reason table and CHANGELOG now say it: `set(true)` is refused for every reason except `decoding` and `rendering` while GPU rendering is still starting, which are accepted and kept. `TestHost.upscaling.renderingStarting` models the second.
- **L2**: the SDK examples ask on the first pick of the item (`state ??= await …state()`), not at activation; the TestHost example asserts `checked` is still false after start.
- **L3**: `set(false)` is answered at once and never starts the GPU check; before any check it reports `available: false` with the new reason code `unchecked` (also in the SDK union and the agreement test, which reads it from `Reason::code`).
- **L5**: when DXGI cannot list the adapters, the probe reports `gpu-unsupported` with the error, not `no-gpu`.
