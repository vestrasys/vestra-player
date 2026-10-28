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
