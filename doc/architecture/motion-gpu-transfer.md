# Motion smoothing: frames through system memory now, GPU-to-GPU later

**Status: decided 2026-11-22 (user).** The NVIDIA motion smoothing engine (T8, player PR #455) moves frames through system memory. We switch to GPU-to-GPU transfers once the upstream work listed below is merged into FFmpeg and mpv. Until then we carry no patch for it.

## The path today

```
d3d11va decode (GPU) → copy back to RAM → upload to Vulkan → fruc_vulkan → download to RAM → CPU or D3D11 rendering
```

FFmpeg's `fruc_vulkan` (NVIDIA optical flow through `VK_NV_optical_flow`) needs Vulkan frames. Our decoder produces D3D11 frames, and our renderer is CPU or Direct3D 11. Upstream FFmpeg, 9.1 included, has no D3D11↔Vulkan mapping or transfer: `hwcontext_vulkan` maps only from VAAPI/DRM and transfers only to and from CUDA. So with stock FFmpeg the only way from d3d11va into Vulkan is system memory. The AMD engine (`frc_amf`) uses the same copy-back decoding.

What it costs: the copies use CPU time and PCIe bandwidth, which is several GB/s at 4K60. That is acceptable on the RTX 4070 we measured on, but it may limit weaker GPUs, and it is one reason why 4K30→60 drops frames under CPU rendering. mpv's maintainers object to silent round trips for this reason (kasper93 on mpv PR #18300).

## The upstream work we wait for

| Where | What | State on 2026-11-22 |
| --- | --- | --- |
| FFmpeg [PR #24316](https://code.ffmpeg.org/FFmpeg/FFmpeg/pulls/24316) ([branch](https://github.com/flowreen/FFmpeg/tree/d3d11-vulkan-transfer)) | Direct GPU frame transfers between D3D11 and Vulkan/CUDA: D3D11 textures shared by NT handle, imported into Vulkan, synchronised with a shared D3D11 fence; NV12/P010 bridged per plane. The author measured about 0.1–0.2 ms per hop against 0.9–2.3 ms through RAM. | Open. Tested on one GPU so far. Known issues: an NT handle is created per transfer, and CUDA device matching can pick the wrong adapter. |
| mpv [PR #18300](https://github.com/mpv-player/mpv/pull/18300) | `f_autoconvert`: keep hardware frames on the GPU when a transfer between two hardware formats is possible, instead of downloading. | Open. The maintainers support the direction (GPU-side transfers, device matching) and are reviewing the design. |
| mpv [PR #18301](https://github.com/mpv-player/mpv/pull/18301) | Companion to #18300. | Open. |

Context: mpv [PR #18426](https://github.com/mpv-player/mpv/pull/18426) (merged) enabled `fruc_vulkan` only when mpv's own Vulkan renderer provides the device. mpv [PR #18634](https://github.com/mpv-player/mpv/pull/18634) (AMF→D3D11) was closed as a duplicate of #18300.

## When they land

1. Move our libmpv build (`src/packaging/libmpv`) to an FFmpeg and an mpv that contain them.
2. Decode with `d3d11va` and transfer D3D11 → Vulkan → D3D11 on the GPU for the NVIDIA engine. The D3D11 renderer stays. Check whether the AMD engine's `frc_amf` path can use the same transfers.
3. Compare the output byte for byte with the system-memory path on our NVIDIA machine, and measure the CPU and frame drops before and after, at 1080p and 4K.
4. Keep the system-memory path only as an explicit fallback that is logged when used.

## Not pursued

- NVIDIA's NvOFFRUC library: the licence forbids open-source use, it is 8-bit only, and the SDK is not maintained.
- Switching the whole player to Vulkan rendering for this.
- Carrying the unmerged upstream patches ourselves (decided 2026-11-22: wait for upstream).

A later option, if upstream stalls: the driver's own optical flow API through its D3D11 interface (BSD-3 headers, runtime in the driver), with our own interpolation shaders.
