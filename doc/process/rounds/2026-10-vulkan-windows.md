# Research note: can Windows use Vulkan end to end (decode, mpv render, Slint UI)?

Researcher, 2026-10-27. Read-only. Commands run on this machine: `vulkaninfo --summary`, `vulkaninfo` (full dump saved next to this note as `vulkaninfo-full.txt`), `ffmpeg -hide_banner -hwaccels`, `ffmpeg -h decoder=…`, `ffmpeg -buildconf`, byte searches of `C:\vestra\deps\libmpv-2.dll`. No GPU decode, render or interop was run. Context: `doc/process/rounds/2026-10-windows-render.md` §2 and Appendix C on main; Appendix D on branch `gpu-render`.

## Short answer

Not as a production direction now. Three findings decide it:

1. **The 780M (the GPU driving this machine's screen) has no Vulkan Video at all on the installed driver.** Its Vulkan device exposes 144 extensions, none of them `VK_KHR_video_*`, and no queue family has `QUEUE_VIDEO_DECODE_BIT_KHR`. The RTX 4070 exposes decode for H.264, H.265, AV1 and VP9.
2. **libmpv's render API has no Vulkan backend**, in our pinned build or in mpv master as of 2026-10-26: only `d3d11`, `opengl`, `sw`. mpv's `hwdec=vulkan` requires a Vulkan `ra_ctx`, which only exists when mpv owns the window (`vo=gpu`/`gpu-next`, `--gpu-api=vulkan`). An RFC that added a Vulkan render API was closed the same day, and the gpu-next render backend PR is still open and covers only OpenGL.
3. **The remaining D3D11 problem is not an API problem.** D.10/D.12 show the shortfall on the 780M is GPU scheduling between mpv's ~9 ms 4K render and Slint's draw on one 3D engine. A Vulkan path puts the same two jobs on the same engine.

A fully Vulkan path would remove the cross-API bridge only on NVIDIA (and maybe on newer AMD/Intel drivers), only with a self-built libmpv fork, and only after moving Slint on Windows to its unstable wgpu API. On the 780M as installed it would decode with `d3d11va-copy`, which is worse than today's D3D11 zero-copy.

## 1. Decode: Vulkan Video

**This machine (`vulkaninfo`, loader 1.4.341):**

| GPU | driver | Vulkan API | `VK_KHR_video_decode_*` | decode queue |
| --- | --- | --- | --- | --- |
| AMD Radeon 780M | AMD proprietary 22.40.05.21 (LLPC), = 31.0.14005.21001, Oct 2023 | 1.3.237 | **none** (0 of 144 device extensions) | none (families: graphics+compute, compute, transfer) |
| RTX 4070 Laptop | NVIDIA 610.62 | 1.4.341 | `video_queue`, `decode_queue`, `decode_h264`, `decode_h265`, `decode_av1`, `decode_vp9`, `video_maintenance1/2`, plus encode | queue family 3: `QUEUE_VIDEO_DECODE_BIT_KHR`, H.264/H.265/AV1/VP9 |

Both expose `VK_KHR_external_memory_win32`, `VK_KHR_external_semaphore_win32` and `VK_KHR_timeline_semaphore`. LUIDs: 780M `37250100-00000000`, 4070 `13380100-00000000`.

**FFmpeg.** Our standalone FFmpeg is BtbN n9.0.2-31 (`--enable-vulkan --enable-libplacebo`). `-hwaccels` lists `vulkan`. `-h decoder=` shows `vulkan` among the hardware devices for h264, hevc, av1 and vp9 (and it is the only one for ffv1 and prores). The FFmpeg linked statically into `libmpv-2.dll` also contains `h264_vulkan`, `hevc_vulkan`, `av1_vulkan` and `vp9_vulkan`.

**mpv.** `libmpv-2.dll` (`git-e471496c56`) contains `vulkan-copy`, `winvk` (the Windows Vulkan context), `gpu-next`, `vulkan-device` and `VK_KHR_video_decode_queue`. So the build can do `hwdec=vulkan`. mpv's guide ([discussion #13909](https://github.com/mpv-player/mpv/discussions/13909)) gives the requirements: FFmpeg 6.1+, libplacebo 6.292+, mpv 0.37+, `--vo=gpu-next --gpu-api=vulkan --hwdec=vulkan`, plus `--gpu-context=winvk` on Windows.

**Drivers on Windows:**

- **AMD.** [22.11.2](https://www.amd.com/en/resources/support-articles/release-notes/RN-RAD-WIN-22-11-2-VLK-VIDEO-CORE-DECODE.html) was a special driver: H.264/H.265 decode, RX 5000/6000 only. Mainline decode came in 23.7.1 (secondary source). mpv's guide says "23.9.3 or newer, RDNA 1/2/3" but marks the 7000 series unconfirmed. AV1 decode on AMD Windows appeared only in a beta when the extension was released ([Khronos](https://www.khronos.org/blog/khronos-releases-vulkan-video-av1-decode-extension-vulkan-sdk-now-supports-h.264-h.265-encode)). **The installed 780M driver is an OEM 22.40 branch (Oct 2023) and exposes nothing (measured).** Whether a current Adrenalin build exposes decode, and AV1, on Phoenix (780M) is **insufficient evidence**: I found no release note or report for 780M/890M. One `vulkaninfo` after a driver update would settle it, but updating the driver is the user's call, given today's 0x14F bugcheck.
- **NVIDIA.** 535+ per the mpv guide. Measured here on 610.62: all four codecs.
- **Intel.** The mpv guide calls it "unclear and untested" (101.4311+ exposes the extensions).
- **Coverage (indicative only, a self-selected sample).** [gpuinfo.org Windows extension list](https://vulkan.gpuinfo.org/listextensions.php?platform=windows): `decode_queue`/`decode_h264` 53.6% of reports, `decode_h265` 49.6%, `decode_av1` 34.3%, `decode_vp9` 32.2%.

## 2. Render handoff: does libmpv render to Vulkan?

- **Our pinned header** (`C:\vestra\deps\include\mpv\render.h:52-54, 481-485`) lists D3D11, OpenGL and SW only.
- **mpv master** (`77e8f9cfa1`, 2026-10-26) is the same. `include/mpv/` has `render.h`, `render_d3d11.h` and `render_gl.h`, and no `render_vk.h`.
  - `video/out/gpu/libmpv_gpu.c` registers the context backends `d3d11` and `gl`, and builds its renderer with `gl_video_init`. That is vo_gpu's renderer, not gpu-next's libplacebo `pl_renderer`.
  - `vo_libmpv.c` `render_backends[] = { &render_backend_gpu, &render_backend_sw }`.
  - The D3D11 render API landed on 2026-10-15 (`4255130de4`).
- **Upstream attempts:**
  - [#18258 "[RFC] vo_libmpv/gpu-next: add Vulkan render API"](https://github.com/mpv-player/mpv/pull/18258) (`render_vk.h`, client-supplied instance/device/queue/VkImage/semaphores; based on andrewrabert's fork; tested only on Linux/Wayland with NVIDIA). It was closed within 2 h. kasper93 said "first port the vo_gpu_next to ra primitives, validate this and then expose this in libmpv api"; Dudemanguy said "Not really salvageable."
  - [#16818 "vo_libmpv: introduce 'gpu-next' render backend"](https://github.com/mpv-player/mpv/pull/16818): open since 2025-09, OpenGL only, "designed to easily accommodate other backends like Vulkan or D3D11 in the future". IINA's author has a Metal variant of it.
- **hwdec is tied to the renderer's API:**
  - `hwdec_vulkan.c` needs `ra_vk_ctx_get` plus `ra_pl_get`, that is, a Vulkan ra context. The libmpv render API (D3D11 or GL) therefore cannot use `hwdec=vulkan`.
  - `hwdec_d3d11va.c:111` returns -1 unless `ra_is_d3d11`. Under a Vulkan renderer, d3d11va decode is copy-back (`d3d11va-copy`).
  - So on the 780M as installed, a Vulkan renderer means copying decoded frames back.
- **Options:**
  - (i) Wait for upstream.
  - (ii) Fork libmpv with a `render_vk` backend and build it ourselves. This leaves the pinned shinchiro build, and #18258 shows the patch is not upstream quality.
  - (iii) `--wid` with mpv's own Vulkan swapchain. The round's §2(b) applies unchanged: the child HWND covers Slint's OSD, overlays, bottom bar and menus, and `vo=libmpv` features are lost. mpv.net does this (`src/MpvNet/Player.cs:100` `SetPropertyLong("wid", …)`) with its UI in separate windows.
  - (iv) libplacebo directly. It is static inside `libmpv-2.dll` and not exported (§2(d)); not viable.

## 3. Slint on Vulkan (1.18.1 sources in the cargo registry)

- **Renderers:**
  - femtovg has an `opengl` backend and a `wgpu` backend (`i-slint-renderer-femtovg-1.18.1/wgpu.rs`; slint feature `renderer-femtovg-wgpu`).
  - Skia's Vulkan goes **through wgpu**: `renderer-skia-vulkan`, `lib.rs:209-222,476` `default_vulkan`, and `vulkan_handle.rs` takes the raw VkDevice/VkImage from wgpu-hal. Skia has no native D3D surface.
  - Vello (1.19, unreleased) gains wgpu 30 texture images.
- **Importing a GPU image:**
  - `impl TryFrom<wgpu_30::Texture> for Image` (`i-slint-core-1.18.1/graphics/wgpu_30.rs:101-112`, feature `unstable-wgpu-30`) requires `Rgba8Unorm`/`Rgba8UnormSrgb` and `TEXTURE_BINDING | RENDER_ATTACHMENT`.
  - There is **no VkImage or Vulkan-handle import** in Slint. The route is a raw VkImage, then `wgpu_hal::vulkan::Device::texture_from_raw` (`wgpu-hal-30.0.1/src/vulkan/device.rs:296`), then `wgpu::Device::create_texture_from_hal` (`wgpu-30.0.1/src/api/device.rs:341`), then `Image::try_from`.
- **Sharing a device:**
  - `WGPUConfiguration::Manual { instance, adapter, device, queue }` (`i-slint-core-1.18.1/graphics/wgpu_29.rs:78-87`, same in 30) lets the app supply the wgpu device.
  - `wgpu_hal::vulkan::Adapter::device_from_raw` (`adapter.rs:2570`) wraps an app-created VkDevice, so wgpu and mpv/libplacebo *could* share one VkDevice.
- **A cheaper cross-API import also exists:** `wgpu_hal::vulkan::Device::texture_from_d3d11_shared_handle` (`device.rs:453`, needs `VK_KHR_external_memory_win32`, which both GPUs here expose). This is the vendor-supported Vulkan import of a D3D11 texture, in place of `WGL_NV_DX_interop2` / `GL_EXT_memory_object_win32`.
- **Slint changelog** (master, 1.19 unreleased): no Vulkan import planned. The wgpu APIs remain `unstable-*`.

## 4. Composing a full path

**A. "All Vulkan, one device":**

- FFmpeg Vulkan decode on mpv's device (`hwdec=vulkan`).
- mpv/libplacebo renders into an app VkImage (needs a forked `render_vk`).
- wgpu is built from the same VkDevice (`device_from_raw`, `Manual` config) and wraps the image (`texture_from_raw`). Slint draws with femtovg-wgpu or Skia-wgpu-Vulkan.
- Sync: timeline semaphores, or the same queue.
- This genuinely removes the cross-API bridge. Prerequisites: a libmpv fork, Slint's unstable wgpu on Windows (against the round's femtovg/GL lock), and Vulkan Video on the GPU.
- On the 780M as installed: decode is `d3d11va-copy` or software, so it is worse than today.

**B. `--wid`:** rejected in §2(b) for the overlay z-order. Nothing in Vulkan changes that.

**C. Hybrid (not asked, but the realistic "Vulkan" variant):**

- Keep mpv's proven D3D11 render API and d3d11va zero-copy.
- Run Slint on wgpu (Vulkan or DX12) and import the ring textures with `texture_from_d3d11_shared_handle`.
- The bridge moves from WGL to Vulkan external memory, which avoids the ES2 and WGL class of bugs (D.6) but still crosses APIs.
- It does not change D.10's single-engine contention.
- This is the round's option (c).

**Who does this:**

- **media_kit (Flutter) on Windows**: libmpv `MPV_RENDER_API_TYPE_OPENGL` on ANGLE (D3D11), and a D3D11 texture shared to Flutter with `kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle` (`media_kit_video/windows/video_output.cc:56,90,313`, `angle_surface_manager.cc:221,294`). It falls back to `MPV_RENDER_API_TYPE_SW` capped at 1080p. That is D3D, not Vulkan.
- **mpv.net**: `--wid`.
- **IINA**: OpenGL render API on macOS; experimental Metal and gpu-next work in #16818.
- **Celluloid and Haruna**: Linux GL.
- I found no shipping app that combines libmpv and libplacebo with a Vulkan UI toolkit on Windows. The closest is mpvpaper with the #18258 patch, on Linux.

## 5. Risks and cost

- **Driver maturity.**
  - AMD Windows Vulkan Video is the weakest: a late start, desktop RDNA first, AV1 unclear, and nothing on this 780M driver.
  - The mpv thread has AMD Windows reports such as `VK_ERROR_OUT_OF_HOST_MEMORY` at device creation.
  - NVIDIA is solid. Intel is unclear.
- **Users covered.** Roughly half of Windows GPUs for H.264, about a third for AV1 (gpuinfo, indicative). Every other machine needs the D3D11 or software fallback, so the D3D11 path, or at least the software path, stays anyway: Vulkan adds a path, it does not replace one.
- **Effort (rough):**

  | work | estimate |
  | --- | --- |
  | libmpv fork with `render_vk` plus our own Windows build pipeline (shinchiro toolchain) | 4-7 days, plus ongoing rebase |
  | Slint Windows renderer to wgpu, with a shared VkDevice, image wrap and semaphores | 5-8 days |
  | tests and acceptance | 2-4 days |
  | **total** | **about 11-19 agent-days** |

  The parked D3D11 path is already built, and its default-on list (D.12) is mostly measurement.
- **Bridge:** removed only in A on Vulkan-Video GPUs. In C it moves, and on the 780M as installed it remains a copy.

## Recommendation

- **No integration spike.** The direction is blocked upstream (no libmpv Vulkan render API) and on the user's own display GPU (no Vulkan Video on the 780M driver).
- The D3D11 path's open problem is iGPU GPU scheduling, which Vulkan does not address. The higher-value next step stays D.12 item 1: measure the existing D3D11 path on the RTX 4070.

**Optional information-only measurement for the coordinator** (about 30 min, only if the user accepts running GPU decode on this machine after today's bugcheck):

- **S0. Decode capability.** RTX 4070:

  ```
  ffmpeg -hide_banner -init_hw_device vulkan=vk:1 -hwaccel vulkan -hwaccel_device vk -hwaccel_output_format vulkan -i <clip> -f null - -benchmark
  ```

  - Run it for the AV1, HEVC and H.264 4K60 clips in `C:\vestra\clips`.
  - Record fps, `-benchmark` utime and the errors.
  - On the 780M (`vk:0`), expect an immediate "no decode queue" failure, which confirms the vulkaninfo result.
  - Compare with `-hwaccel d3d11va` on the same clips.
- **S1. mpv's own Vulkan renderer (ceiling of the `--wid` route).**
  - Use `mpv.exe` from the same shinchiro release (20261021; not in `C:\vestra\deps`):

    ```
    mpv --vo=gpu-next --gpu-api=vulkan --gpu-context=winvk --hwdec=vulkan --vulkan-device="NVIDIA GeForce RTX 4070 Laptop GPU" --log-file=… <clip>
    ```

  - Read `hwdec-current`, the dropped frames and CPU over 60 s.
  - On the 780M, the same run with `--hwdec=d3d11va-copy` shows the copy-back cost.
- **S2 (only if the user wants option A seriously).**
  - A headless probe in the style of `gpu-probe`: build libmpv with #18258's `render_vk` patch, create one VkDevice, wrap it in wgpu (`device_from_raw`), and render lavfi then 4K clips into a VkImage.
  - Wrap it with `texture_from_raw`, compare the pixels against a readback, and time `render` and the semaphore waits on the 4070 and the 780M.
  - About 4-6 days, most of it the libmpv build.

## Insufficient evidence

- Whether a current Adrenalin driver exposes Vulkan Video decode (and AV1) on the 780M/Phoenix under Windows.
- Intel Windows Vulkan Video quality.
- Whether Vulkan queue priority or a separate queue on the 780M (one graphics family) could relieve D.10's contention: not measured, speculative.
