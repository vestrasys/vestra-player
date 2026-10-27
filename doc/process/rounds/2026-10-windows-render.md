# Windows video rendering: why 4K60 plays at ~30 fps, and how to render on the GPU

Researcher report, 2026-10-25. Read-only; nothing in the repository was changed. Sources are cited as `file:line`. Repository paths are relative to `/Users/ziyang/Develop/vestra-player`. `REG` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f`. `MPVDEV` = the shipped Windows libmpv: `scratchpad/dl-mpv/mpv-dev.7z`, whose sha256 `efc719ef…a633` matches the pin in `src/packaging/deps.json:19-23` (shinchiro 20261021, mpv `e471496c56`), extracted to `scratchpad/mpvdev-x/`. Only `strings` was run on the DLL; it was never executed.

Problem as reported: Windows 0.5.0, a 4K60 AV1 mkv. Task Manager shows hardware decoding, but playback is visibly about 30 fps. The info overlay shows no dropped frames.

Coordinator's data point: on an Apple M2, 0.5.0 with `VESTRA_SOFTWARE_RENDER=1` plays 4K60 HEVC full screen at 60/60 (`VESTRA_FRAME_LOG=1`, target 2940x1846), but uses about 320% CPU. GPU rendering is also 60/60.

---

## 0. Two facts that change the premise

**F1. On Windows the Slint UI almost certainly draws with OpenGL (femtovg on WGL), not with Slint's software renderer and softbuffer.**

- When GPU rendering is not wanted, `main.rs:4907-4916` selects the winit backend with no renderer name (`winit().select()`).
- In that case `BackendSelector` fills the renderer only from `SLINT_BACKEND` (`REG/i-slint-backend-selector-1.18.1/api.rs:278-289`). Then `create_renderer` takes `(None, None) => default_renderer_factory` (`REG/i-slint-backend-winit-1.18.1/lib.rs:1305`).
- With Slint's default features, `renderer-femtovg` and `renderer-software` are both built (`REG/slint-1.18.1/Cargo.toml` `[features] default`, and `Cargo.lock` lists both renderers for i-slint-backend-winit). `default_renderer_factory` then picks `GlutinFemtoVGRenderer` before software (`lib.rs:139-158`).
- glutin prefers WGL and falls back to EGL (`renderer/femtovg/glcontext.rs:76-77`), with vsync swap interval 1 (`glcontext.rs:208-213`).
- Slint falls back to its software renderer only when the OpenGL 2 probe fails (`renderer/femtovg.rs:42`, `renderer/femtovg/glprobe.rs:22-31`). That happens on VMs, over RDP, or with a basic display driver.

So "Slint software composition, then softbuffer/GDI" applies only on such machines. On a normal desktop, the video is software (mpv, CPU) and the UI is GPU (femtovg). This must be checked on the user's machine; see §1.4.

**F2. The libmpv we ship on Windows already has a Direct3D 11 render API.**

- `MPVDEV/include/mpv/render.h:52`, `:430-437` (`MPV_RENDER_PARAM_D3D11_INIT_PARAMS = 21`, `MPV_RENDER_PARAM_D3D11_FBO = 22`), `:481` (`MPV_RENDER_API_TYPE_D3D11 "d3d11"`), and `client.h:251` (API 2.6).
- It came in upstream commit `4255130de4` "libmpv: add D3D11 render API backend", 2026-10-15 (`DOCS/client-api-changes.rst` 2.6, "mpv 0.42.0"). That commit adds `include/mpv/render_d3d11.h` and `video/out/d3d11/libmpv_d3d11.c`, which is built on vo_gpu's `ra_d3d11` and not on libplacebo. I fetched these from GitHub at `e471496c56`.
- `render_d3d11.h` is **not** in the shinchiro mpv-dev package, so we would have to vendor it (118 lines, two structs).
- What it offers:
  - `mpv_d3d11_init_params { void *device }` (an `ID3D11Device*`) and `mpv_d3d11_fbo { void *tex }` (an `ID3D11Texture2D*` with `BIND_RENDER_TARGET`).
  - Target formats `B8G8R8A8`, `R8G8B8A8`, `R10G10B10A2` and `R16G16B16A16_FLOAT`; no sRGB formats.
  - mpv_render_* may be called from any thread.
  - **d3d11va decodes on the device we supply** (multithread protection on). That is zero-copy. Copy-back is needed only for single-threaded devices.
- The macOS bundle's header (`scratchpad/deps-ci/include/mpv/render.h:468-470`, mpv 0.41) has only `opengl` and `sw`.
- The DLL's feature list (strings, line 215584) includes `d3d11 d3d-hwaccel egl-angle egl-angle-win32 gl-win32 gl-dxinterop gl-dxinterop-d3d9 libplacebo shaderc spirv-cross zimg vulkan`.
- **This should be re-checked on Windows before any work starts.** It is a 10-day-old upstream API in a git build, and I could not run the DLL.

---

## 1. Where the time goes on Windows today

### 1.1 The path, step by step

| # | Step | Thread | Code | 4K source → 2560x1440 target | 4K source → 3840x2160 target |
| --- | --- | --- | --- | --- | --- |
| 1 | Decoding on the GPU (d3d11va), then **copy-back** to RAM: `hwdec=auto-copy-safe`, which becomes `d3d11va-copy` | mpv decoder | `renderer.rs:104-124`, `backend.rs:72` | NV12 12.4 MB or P010 24.9 MB per frame (4K AV1 is often 10-bit). Readback, staging sync and memcpy: about **2-5 ms** CPU | same |
| 2 | mpv SW render: conversion YUV→RGB24, scaling, OSD and subtitles, 10→8 bit dither | render thread (`backend.rs:132-170`) | `render_software(.., SwPixelFormat::Rgb24, width*3, ..)` `backend.rs:154-160` | zimg (in the build) with mpv's default scaler. Lanczos downscale plus conversion: about **30-60 core-ms**. zimg slices it over threads (mpv `--zimg-threads=auto`), so 5-12 ms wall time on 6-8 threads | no resize, conversion and dither only: about **15-30 core-ms** |
| 3 | A new `SharedPixelBuffer::new` **for every frame** (zero-filled `Vec`) | render thread | `backend.rs:154` | 11.1 MB: zero-fill plus about 2,700 fresh-page faults. Large Windows heap blocks come from VirtualAlloc and are freed on drop. About **1.5-3 ms** | 24.9 MB, about 6,100 faults: about **3-6 ms** |
| 4 | mpv's alignment rule (see below) | render thread | `MPVDEV render.h:398-404` | Stride 7680 is a multiple of 64. A Windows heap pointer is only 16-byte aligned, which can trigger mpv's copy fallback. **0-3 ms** | stride 11520 is fine; pointer as before. **0-5 ms** |
| 5 | Hand-off to the UI: `FrameSlot::put`, then the `invoke_from_event_loop` waker, then `show_frame` | render thread → UI | `render_target.rs:274-297`, `main.rs:5155-5163`, `main.rs:1121-1143` | small | small |
| 6 | `Image::from_rgb8`, then `set_property("video-frame")`. femtovg makes a **new GL texture for each frame**, because buffer images are not cached across frames. It uploads 24-bit `GL_RGB`, which many Windows GL drivers repack to RGBA on the CPU | **UI thread** | `main.rs:1135-1138`; `REG/i-slint-renderer-femtovg-1.18.1/images.rs:305-325`, `itemrenderer.rs:1232-1270` | 11 MB upload plus texture alloc and free: about **3-8 ms** | 25 MB: about **6-15 ms** |
| 7 | Slint redraw. On Windows this is **timer-throttled**: a Slint `Timer` at `1000/refresh` ms (an integer, so 16 ms at 60 Hz), not tied to vsync. macOS uses a display link. Then `SwapBuffers` with interval 1, which blocks until vsync | UI thread | `REG/i-slint-backend-winit-1.18.1/frame_throttle.rs:13-28`, `:47-63`; `winitwindowadapter.rs:1849-1854` | Drawing is GPU work; the CPU cost is small. The swap may wait up to 16.7 ms | same |
| 8 | The host's own 16 ms tick (plugin events, `settle_layout`, sizing) shares the UI thread | UI thread | `main.rs:5165-5175`, `main.rs:1440-1475` | depends on the plugins | same |

Notes on the table:

- mpv's own warnings, in the shipped header:
  - SW render is "extremely simple (but slow) … everything … is done on the CPU, single-threaded" (`MPVDEV render.h:129-142`).
  - rgb24 "is strongly discouraged because it is very slow" (`:378`).
  - "Both stride and pointer value should be a multiple of 64 … in the worst case, will copy the entire target frame" (`:398-400`).
  - The header suggests the `sw-fast` profile. The DLL carries it as `[sw-fast] sws-scaler=bilinear sws-fast=yes zimg-scaler=bilinear zimg-dither=no` (strings, line 215656).
- Sizing (`render_target.rs:46-71`): the target is the physical video area, capped at the source size and at 3840x2160. A 4K source in a 1440p window renders at 2560x1440, and full screen on a 4K monitor renders at 3840x2160. mpv already renders at window size, not source size, so there is no gain there.
- Partial redraw does not apply. femtovg repaints the whole window each frame, and the video area changes every frame anyway. The GPU cost of composing the UI is negligible.
- I ruled out the Windows timer granularity: winit 0.30.13 waits on a high-resolution waitable timer (`REG/winit-0.30.13/src/platform_impl/windows/event_loop.rs:637-680`), so a 16 ms Slint timer is not stretched to 31 ms.

### 1.2 Costs common to both systems vs costs specific to Windows

- **Common: the CPU is the bottleneck.** The M2 data point (320% CPU at 60 fps, 2940x1846) equals about 53 core-ms per frame across steps 1-6. It fits the estimates above, and it fits the round's own measurement of 4K at 14% CPU with GPU rendering vs 83% with software (`doc/process/rounds/2026-10-render-pip.md:269-274`, windowed).
- Scaling to a typical Windows laptop or desktop CPU: about 0.5-0.8x M2 per-core speed, and often 4-8 performance cores. The same work is then about 65-105 core-ms per frame, which needs 4-6 fully busy cores at 60 fps. That is at or beyond the limit, before audio, the plugins and the decoder's own copy.
- **Specific to Windows, all of them made worse at 4K:**
  - (a) d3d11va-copy readback. On macOS, videotoolbox-copy on unified memory is cheap; on a discrete GPU this is a PCIe readback.
  - (b) A fresh VirtualAlloc for each 25 MB frame, plus page faults. macOS large mallocs behave similarly, but its page-zeroing is cheaper.
  - (c) The heap pointer is not 64-byte aligned (macOS large mallocs are page-aligned), which can add mpv's full-frame copy.
  - (d) The `GL_RGB` texture upload on the UI thread. Apple Silicon's GL path handles this in unified memory; Windows drivers often swizzle on the CPU.
  - (e) The timer-based frame throttle (not display-linked) plus a blocking `SwapBuffers`.
  - (f) If the GL probe failed (F1): the Slint software renderer composes the whole window on the CPU (4K × 4 B = 33 MB) plus a softbuffer GDI blit. On its own that easily gives 30 fps.

### 1.3 Why "~30 fps" with no visible drops

The most likely mechanism is **vsync halving on the UI thread**. Each new frame costs the UI thread its upload (6-15 ms at 4K) plus the draw, plus whatever the host tick costs. The redraw is driven by a 16 ms timer that is not aligned to vsync, and then `SwapBuffers` blocks until the next vsync. When the UI thread's work plus the swap passes 16.7 ms, every other vsync is missed and the screen shows 30 fps.

mpv does not count these as drops. It considers a frame done when `render_software` returns (`frame-drop-count` counts only mpv's own VO drops).

The overlay hides this twice:

- `framesShown` in `playback.stats` is the **render thread's produced count** (`backend.rs:386-388`, `sequence`). It is not even the UI's take count.
- `VESTRA_FRAME_LOG`'s "shown" counts `FrameSlot::take` (`render_target.rs:285-296`). That is a frame handed to Slint, not a frame presented. Two takes between two swaps both count as shown.

The competing explanation is that the render thread itself cannot keep up (CPU-bound steps 1-4). That should show up as `produced < 60` in the frame log, or as mpv frame drops. §1.4 tells the two apart.

### 1.4 How the user can collect the same numbers on Windows (no rebuild needed)

The release build is a GUI-subsystem exe (`main.rs:5`), so stderr has to be redirected. The player is single-instance (`main.rs:4863-4878`), so **close every Vestra window first**, or the launch hands the file to the running player and exits. The installer's default location is `%LOCALAPPDATA%\Programs\Vestra Player\` (`src/packaging/windows/vestra.iss:26-29`, `{autopf}` with lowest privileges).

```powershell
$exe = "$env:LOCALAPPDATA\Programs\Vestra Player\vestra-host.exe"
$clip = "D:\path\clip-4k60-av1.mkv"
function Run($name) {
  Start-Process -Wait -FilePath $exe -ArgumentList "`"$clip`"" `
    -RedirectStandardError "$env:TEMP\vestra-$name.log"
}
$env:VESTRA_FRAME_LOG = "1"
Run "default"                                   # A: as shipped
$env:SLINT_BACKEND = "winit-software"; Run "slint-sw"; Remove-Item Env:SLINT_BACKEND   # B: force Slint SW
$env:SLINT_BACKEND = "winit-femtovg";  Run "slint-gl"; Remove-Item Env:SLINT_BACKEND   # C: force femtovg
$env:VESTRA_HWDEC = "no"; Run "swdec"; Remove-Item Env:VESTRA_HWDEC                    # D: CPU decode (dav1d), no copy-back
```

For each run, play about 20 s windowed (resize the window to about 1280x720) and about 20 s full screen, then quit. Each log line reads `vestra frames: produced P shown S (p/s fps) target WxH` (`render_target.rs:340-343`). Also record, from Task Manager → Details/Performance: CPU % and per-core load, GPU "3D" and "Video Decode", the CPU model, the GPU model, and the monitor's resolution and refresh rate. For the info overlay: decoder, hwdec and the dropped count. To see which renderer is active in run A, Process Explorer → vestra-host.exe → DLLs shows the GL driver if femtovg is active (`nvoglv64.dll`, `atio6axx.dll` or `ig*icd64.dll`).

How to read the results:

- `produced < 60`: the render thread is the bottleneck (decode, copy-back or zimg; CPU-bound).
- `produced ≈ 60, shown < 60`: the UI thread is the bottleneck (upload or host tick).
- Both ≈ 60 but the picture still looks like 30: the loss is in presentation (throttle, vsync or composition). Today's counters cannot show this; §4 proposes the counter that does.
- If A matches C, the default renderer is femtovg. If A matches B, the machine fell back to the Slint software renderer (F1's exception), and that alone explains a lot.

### 1.5 Quick wins inside the software path (analysis only)

| Change | Where | Expected gain per frame at 4K | Risk |
| --- | --- | --- | --- |
| `sw-fast` behaviour while rendering in software: `zimg-scaler=bilinear`, `zimg-dither=no` (or `apply-profile sw-fast`) | backend start / `software_hwdec` time | Resize cost down about 40-60% when downscaling (1440p target). Small gain at 1:1 (4K target), where only conversion and dither remain | Slightly softer image and banding on 10-bit sources |
| Reuse a ring of 3 buffers instead of `SharedPixelBuffer::new` each frame (reuse one only when the UI has dropped it, so `make_mut_bytes` does not copy) | `backend.rs:154` | 1.5-6 ms render thread, fewer page faults | Low |
| 4-byte output (`bgr0`/`rgb0`, or mpv's internal `rgba`) into `Rgba8Pixel` | `backend.rs:154-160`, `main.rs:1135` | mpv calls rgb24 "very slow" (`render.h:378`). Also makes the GL upload `RGBA` instead of `RGB`, the driver's fast path: about 2-8 ms combined | The "0" byte is "uninitialized garbage" (`render.h:369-372`), and Slint would treat it as alpha, so the picture could turn transparent or black. Needs a one-time probe or mpv `rgba`, which the header calls "not a good idea to rely on" |
| Stride padded to a multiple of 64 bytes (16 px at 4 B per pixel), cropped with the Image's source clip | same | Avoids mpv's full-frame copy: 0-5 ms | The pointer alignment still cannot be controlled with `SharedPixelBuffer` |
| Adaptive render cap (e.g. at most 1920x1080 when the produced or presented rate falls behind) | `render_target::target` (`render_target.rs:46-71`) | zimg and upload cost ÷2.25-4 | Visibly softer on a 4K display |
| Persistent GL texture plus `glTexSubImage2D`/PBO upload in a rendering notifier | needs unsafe GL code | Removes the texture alloc and RGB swizzle from the UI thread | Same unsafe and GL work as the real GPU path, so better spent there |

Realistic outcome: the first three together could recover roughly 30-50% of the per-frame cost.

- In a ≤1440p window on a 6-8-core desktop CPU, 60 fps is plausible.
- At 4K full screen with 10-bit AV1 on a laptop CPU, it is not reliable, and CPU use would still be 200-400%.
- Adding the adaptive cap would reach 60 fps almost everywhere, at the cost of sharpness.

**A cheap interim fix can make 1440p acceptable, but it will not make 4K60 good. The fix is GPU rendering.** If the measurement shows the Slint software renderer is active (F1's exception), none of this helps much.

---

## 2. Windows GPU rendering options

How the macOS design works today, which is what we want to reuse (`src/host/unsafe/render/src/lib.rs`):

- mpv runs on its own thread in a private offscreen context (`:245-314`) and renders into a **ring of 3 shared surfaces** (IOSurface).
- Each Slint femtovg window, in `BeforeRendering`, binds the newest surface in its own GL context, blits it into its own `GL_TEXTURE_2D`, and hands Slint a `BorrowedOpenGLTexture` (`:395-437`, `:467-530`).
- The handle API (`set_size`, `set_display_fps`, `frames`, `shown`, `size`, `reshow`, `latest`, `on_frame`, `attach`) is mirrored by `no_gpu` (`platform/mod.rs:300-357`).
- The host enters it only through `platform::gpu_start`, and `backend.rs:627-659` falls back to software on failure.

### (a) mpv render API on OpenGL (ANGLE or native WGL), shared with Slint femtovg

- Slint 1.18.1 on Windows uses glutin WGL with `FallbackEgl` (`glcontext.rs:76-77`). There is no way to make it use ANGLE without patching Slint.
- Native WGL:
  - mpv cannot render in a private WGL context and share the result with Slint's context. `wglShareLists` must be called before the target context creates objects, and Slint creates its context without a hook.
  - The alternative is rendering mpv **inside Slint's context** in `BeforeRendering`. That ties mpv's single render context to one window, and libmpv ends playback when that context is freed (`lib.rs:8-10`), while the sidebar detach rebuilds the main window. So it does not fit the existing design.
  - Hardware decoding: `render_gl.h:90` says "Windows: ANGLE is required" for direct (zero-copy) hwdec. On WGL only d3d11va-copy works, or dxva2 via `gl-dxinterop-d3d9` (D3D9; AV1 support on DXVA2 is patchy).
- ANGLE: `egl-angle-win32` is in the build, but we would have to ship libEGL.dll and libGLESv2.dll. Sharing with Slint's WGL context still needs DX interop.
- Verdict: feasible, but **worse than (c′) below in every respect**. Effort 5-8 agent-days. Plugins: glsl-shaders and tone-mapping work as on macOS.

### (b) `--wid` child window / mpv's own D3D11 swapchain under Slint

- Classic player approach: `vo=gpu-next`, d3d11va zero-copy, real HDR output (`d3d11-output-csp`), full libplacebo, and all of `video.render` works.
- But the child HWND covers the parent's client area. Slint draws OSD, the info overlay, the floating bottom bar, subtitles from plugins and in-window popups and menus into the parent surface, so in the video area **they all end up below the video**. This is the same problem the round rejected on macOS ("sublayers are always drawn above the contents", `2026-10-render-pip.md:38`).
- Workarounds:
  - a transparent layered top-level window for the UI, with input routing, z-order, DPI and fullscreen problems;
  - DirectComposition, which Slint does not expose.
- It also gives up `vo=libmpv`. C-SIZE, display-layer zoom, publish-gate orientation, produced streams (NES), and PiP/snapshot frames all assume the host owns the frames.
- Verdict: **not recommended**. 8-15+ agent-days, with UI regressions.

### (c) Slint Skia or wgpu renderer plus mpv D3D11 render API

- Slint 1.18.1 Skia has no D3D surface (`REG/i-slint-renderer-skia-1.18.1/`: `opengl_surface`, `software_surface`, `vulkan`, `wgpu_29/30`).
- The `GraphicsAPI` variants are `NativeOpenGL`, `WGPU29` and `WGPU30` (`REG/i-slint-core-1.18.1/api.rs:268-306`). So "Skia/D3D" means **wgpu (D3D12 or Vulkan)**.
- A D3D11 texture shared with an NT handle and a keyed mutex can be opened in D3D12 and wrapped through `wgpu-hal` `create_texture_from_hal`, then shown as a wgpu texture image (unstable-wgpu-30 API).
- Verdict: feasible, but it means switching the whole Windows UI renderer from femtovg/GL to wgpu (against the round's "lock to OpenGL/femtovg" decision, `2026-10-render-pip.md:7`), relying on Slint's unstable wgpu features, adding wgpu-hal plus `windows` unsafe code, and doing cross-API synchronization. 10-15 agent-days. A good long-term option if Slint adds D3D import or the project moves to wgpu everywhere.

### (c′) mpv **D3D11 render API**, shared into Slint femtovg through **WGL_NV_DX_interop2** (recommended)

The macOS design with the platform pieces swapped:

| macOS (`vestra-gpu-render`) | Windows equivalent |
| --- | --- |
| Private CGL context on the render thread | Private `ID3D11Device` (`VIDEO_SUPPORT`, `ID3D10Multithread::SetMultithreadProtected(TRUE)`) on the render thread |
| `OwnedRenderContext::new_opengl` | `mpv_render_context_create` with `api_type="d3d11"` and `D3D11_INIT_PARAMS{device}`. rsmpv 0.2.1 has no binding, so raw `rsmpv_sys` calls with param ids 21 and 22 (`rsmpv-sys-0.1.1/src/render.rs` stops at 20); `Mpv::as_raw` exists (`rsmpv-0.2.1/src/lib.rs:160`) |
| Ring of 3 IOSurfaces plus FBO | Ring of 3 `ID3D11Texture2D` (`B8G8R8A8_UNORM`, `BIND_RENDER_TARGET|SHADER_RESOURCE`), rendered with `D3D11_FBO{tex}`, then `Flush` |
| `IOSurfaceIsInUse` skip | Per-texture "locked by a window" flag (interop lock state) |
| Window: `CGLTexImageIOSurface2D` into `GL_TEXTURE_RECTANGLE`, blit to own `GL_TEXTURE_2D` | Window: `wglDXOpenDeviceNV(device)` once per GL context, `wglDXRegisterObjectNV(tex → GL name, GL_TEXTURE_2D)` per ring texture, `wglDXLockObjectsNV`, blit, `wglDXUnlockObjectsNV`. The entry points come from Slint's `NativeOpenGL { get_proc_address }`, which is `wglGetProcAddress` for extensions |
| `BorrowedOpenGLTexture` to Slint | same, unchanged |
| `report_swap` pacing for interpolation | same, unchanged (`mpv_render_context_report_swap`) |
| `GpuFrame = IOSurfaceRef` (PiP) | `ID3D11Texture2D` (PiP is `unsupported` on Windows; a future host-made PiP window could use it) |

- **Zero-copy hwdec**: `hwdec=auto-safe` (`renderer.rs:115-124` with `software=false`) picks d3d11va on mpv's own device. The decoded NV12/P010 frames are sampled directly by vo_gpu's ra_d3d11. The only per-frame copy is one GPU-side RGBA blit per window, as on macOS.
- **HDR**: tone mapping to SDR works as on macOS. The `R10G10B10A2` and `RGBA16F` targets exist, but Slint's WGL swapchain is 8-bit SDR, so there is no HDR passthrough. That matches macOS.
- **What plugins see**: `video.render` keeps working, because the options are vo_gpu options.
  - Shaders: `glsl-shaders` should work, since ra_d3d11 translates GLSL through shaderc and spirv-cross, both in the build.
  - Peak detection: `hdr-compute-peak` should actually work, since D3D11 FL11 has compute shaders. On macOS it is disabled (`2026-10-render-pip.md:125`).
  - **Shader error detection:** `video_render.rs:215-236` recognises a compile failure only from a `libmpv_render` error line that starts with `ERROR:` (GL compiler format). shaderc and spirv-cross report errors in other formats, so on Windows a bad shader would stay "applied". This needs a Windows log sample and a pattern in shared code.
  - **Reference:** the docs saying the render options are "unsupported on Windows" become false.
  - The plugin API does not change.
- **Risks**:
  1. The D3D11 render API is 10 days old upstream and only in git builds. We pin the shinchiro build, so it is stable for us, but it should be smoke-tested first.
  2. WGL_NV_DX_interop2 needs a driver that supports it (NVIDIA, AMD and Intel all do on current drivers) and **the same adapter** for GL and D3D. Hybrid laptops (Optimus/PowerXpress) can put the GL context on one GPU and the default D3D adapter on the other. Mitigations: create the D3D device on the adapter whose output holds the window, and fall back to software when `wglDXOpenDeviceNV` fails. The fallback path already exists (`backend.rs:627-659`, `renderer.rs:62-67`).
  3. Device-removed / TDR handling.
  4. The interop lock is the synchronization point between D3D and GL. Lock only around the blit; never hold a lock across frames.
  5. No Windows machine here. CI Windows runners have no GL 2+, so femtovg falls back to software and the interop cannot be CI-tested. mpv's D3D11 render-to-texture can be tested headless on WARP (`d3d11-warp` exists in the DLL), like the macOS `#[ignore]` tests (`lib.rs:583-622`).
- **Effort**:
  - standalone probe (a winit+glutin window, mpv d3d11 into a texture, interop into GL, a 4K60 file): 3-4 agent-days;
  - integration into `vestra-gpu-render` and `platform/windows.rs` with fallbacks: 4-6;
  - Windows test iterations with the user: 2-3;
  - **total about 9-13 agent-days**.
- **Lower tier inside the same design, for when interop fails**: D3D11 render, then a staging readback, then a GL upload. Still GPU scaling and conversion, so far less CPU than now, but two 33 MB copies per frame. Worth having only if the interop proves unreliable on a class of drivers.

### (d) libplacebo directly

- libplacebo is linked statically inside `libmpv-2.dll` and not exported.
- Using it would mean our own FFmpeg decode, A/V sync, subtitles and OSD in place of mpv's player core, and losing every mpv-backed feature the plugins use.
- Verdict: **not viable** (months, not days).

---

## 3. Recommendation and step plan

**Recommendation: (c′), the mpv D3D11 render API shown through WGL_NV_DX_interop2 in Slint femtovg, with the existing software fallback.** Do the measurements and the low-risk software-path fixes first, so there is a baseline and a better fallback.

1. **Measure (0.5 day, user plus coordinator)**: run §1.4 on the user's machine. Decide between render-thread, UI-thread and presentation bottleneck, and confirm femtovg vs Slint software (F1).
2. **Add a presented-frame counter (1 day, maintainer, shared code)**: see §4. Ship it in the next patch, so every later step has a real metric.
3. **Software-path fixes (1-2 days, maintainer)**: buffer ring, `sw-fast` zimg options while rendering in software, and (behind a probe) 4-byte output with a padded stride. Measure again with step 1's procedure. This also benefits Linux, headless, and every fallback machine.
4. **Probe before integrating (3-4 days)**: a standalone program in the style of `doc/process/rounds/assets/pip-probe.swift`.
   - Smoke-test the shipped libmpv's `d3d11` render API first: create a context and render one lavfi frame into a texture. **This is the go/no-go gate.**
   - Then verify interop on NVIDIA, AMD and Intel, plus one hybrid laptop if available.
5. **Integrate (4-6 days)**, following the cross-platform rule (§1.5 of the workflow doc):
   - All `unsafe` goes in `src/host/unsafe/render/` (a `#[cfg(windows)]` module next to the macOS one, the same public `Renderer`/`Handle` API, and the `windows` crate for D3D11/DXGI). Alternatively add it to `unsafe/win32`, but keeping both renderers in one crate keeps `Handle` identical.
   - `platform/windows.rs`: `GPU_RENDER = true`, `gpu_start`, the `GpuRenderer`/`GpuHandle`/`GpuFrame` types.
   - `platform/mod.rs`: `no_gpu` becomes `cfg(not(any(macos, windows)))`.
   - Shared code (`backend.rs`, `main.rs` `attach_gpu`, `renderer.rs`) needs no OS code. `main.rs:4908-4916` already selects femtovg when GPU is wanted.
   - `renderer::gpu_name` gets per-platform text (e.g. "GPU rendering (Direct3D 11)") from the platform module, through i18n.
   - Make the shader-failure detection in `video_render.rs` accept shaderc errors.
   - Update the reference §5.4 and §8.2 and the "unsupported on Windows" statements; there are no plugin API changes.
6. **Acceptance on a real Windows machine**, run by the coordinator or the user:
   - 4K60 AV1 10-bit, 4K60 HEVC 10-bit and 4K60 H.264 play at **≥59.5 presented fps** (the §4 counter) windowed 1440p and full screen on a 60 Hz monitor (and 120/144 Hz if available). Fewer than 1 dropped or repeated frame per 10 s; mpv `frame-drop-count` and `decoder-frame-drop-count` stay 0 over 60 s.
   - **CPU ≤ 25%** of the machine (Task Manager) for 4K60 full screen, and `hwdec-current = d3d11va` (not `-copy`).
   - OSD, info overlay, bottom bar, subtitles and menus above the picture; resize, sidebar detach (window rebuild), fullscreen toggle, source change without a flash, NES produced stream, `video.render` (tone mapping, shader, interpolation) and snapshot.
   - Fallback: `VESTRA_SOFTWARE_RENDER=1`, a forced interop failure, and an RDP session or VM (GL probe fails) each end in software rendering with one `render: …` stderr line, and playback keeps going.
   - Record CPU and fps for GPU vs software, as in `2026-10-render-pip.md:269-274`.

---

## 4. A metric that shows the real displayed frame rate

What the overlay shows today, and why it misleads:

- `framesShown` (`backend.rs:386-388`, `:766`) = frames **mpv rendered** (the SW sequence plus GPU publishes).
- `framesDropped` = mpv `frame-drop-count` (`backend.rs:767`), which counts frames mpv's VO skipped.
- Neither sees frames lost after mpv: UI coalescing in `FrameSlot` (`render_target.rs:254-297`), vsync halving, or Slint frames that redraw an unchanged video image.

mpv's other counters do not help here:

- `vo-delayed-frame-count` and `mistimed-frame-count` are meaningful only with display-sync (`video-sync=display-*`).
- `estimated-display-fps` and `vsync-jitter` need `mpv_render_context_report_swap` at real swap time (`render.h:527-533`, `:726-737`). The SW path never calls it, and the macOS GPU path calls it only while pacing for interpolation (`unsafe/render/src/lib.rs:299-301`).
- `decoder-frame-drop-count` (in the DLL) is worth adding to stats to separate decoder drops, but it does not measure presentation either.

Proposal, all in shared host code and OS-independent:

1. **Host presented counter.** Count Slint renders that put a *new* video frame on screen:
   - femtovg (both systems): in the window's rendering notifier at `RenderingState::AfterRendering` (`REG/i-slint-core-1.18.1/api.rs:348`), compare the video sequence last set on the `video-frame` property with the one at the previous render, and increment `presented` when it changed. Slint allows **one** notifier per window (`AlreadySet`, `api.rs:374-379`), and the GPU path already owns it (`unsafe/render/src/lib.rs:436`). So the GPU `View::update` already counts copies into the window (`shown`, `lib.rs:513`), and the software path needs its own notifier only when no GPU renderer is attached.
   - Slint's software renderer returns `Unsupported` for notifiers (`REG/i-slint-core-1.18.1/renderer.rs:348-353`). For it, count winit `RedrawRequested` through `slint::winit_030::WinitWindowAccessor::on_winit_window_event` (the `unstable-winit-030` feature is already on, `Cargo.toml`), or report `null`.
   - Also count **repeats**: renders where a newer frame had been produced than the one shown. A rising repeat count with presented ≈ 30 is the vsync-halving signature.
2. **Expose it** in `PlaybackStats` as an additive field such as `presentedFps` (rolling 1 s), plus `framesPresented`. Per the SDK rule this is a patch bump, even though it is additive.
   - Keep `framesShown` but document it as "frames rendered by the video renderer", or rename it in the overlay text.
   - The info plugin (`src/plugins/info/main.ts:39-43`, `properties.ts:135-151`) then shows "Displayed: 30.0 fps (source 60.0)", next to the existing "Display frame rate".
3. **`VESTRA_FRAME_LOG`**: add a third column, `presented`, so the log reads `produced / taken / presented`. The current "shown" is really "taken". That gives one log line answering §1.4's question.

This is still "presented to the compositor", not photons. With WGL there are no DXGI frame statistics; after (c′) a DXGI-level check is still impossible because Slint owns the swapchain. The presented-new-frame rate is the best signal the host can measure on every OS, and it is the one that would have shown this bug.

---

## Insufficient evidence / to verify

- That the user's machine runs femtovg rather than the Slint software renderer (F1). Run §1.4 A vs B vs C.
- That the shipped `libmpv-2.dll` really runs `api_type="d3d11"`. The header and the upstream commit say so, but the DLL was not executed here, and there is no distinctive string for the new backend in it.
- The per-step millisecond figures in §1.1 are estimates from data sizes, mpv's documented behaviour and the M2 total (53 core-ms per frame). None of them was measured on Windows.
- WGL_NV_DX_interop2 behaviour per vendor and on hybrid GPUs. To be checked in the step 4 probe.
- The exact shaderc/spirv-cross error format for the `video.render` shader-failure detection.

---

## Appendix A (2026-10-25, after the user's Windows logs): the ~30 Hz cap and a 0.5.1 fix

### A.1 What the logs show

These are the logs in `~/.claude/uploads/0e44c0a1-…/` (the `swdec` run wrote no file). I averaged the per-second lines `vestra frames: produced P shown S … target WxH` (`render_target.rs:340-343`) by target size:

| Run | Target 1150x1524 (windowed, tall video area) | Target 3040x1828 (transition, few lines) | Target 3840x2160 (full screen on a 4K monitor) |
| --- | --- | --- | --- |
| default (femtovg) | produced 59.2, **shown 26.9** (40 lines) | 27.0 / 7.9 (4) | produced 58.7, **shown 26.3** (31) |
| `SLINT_BACKEND=winit-software` | 52.8 / **25.0** (15) | 59.9 / 21.1 (4) | 59.9 / **32.5** (30; up to 36.6) |
| `SLINT_BACKEND=winit-femtovg` | 56.8 / **28.9** (32) | 59.6 / 25.7 (2) | 60.0 / **30.7** (15; up to 34.9) |

What follows from the numbers:

1. **The render thread keeps up** in every mode and size (produced ≈ 60). mpv's software rendering is not what limits playback on this machine.
2. **"shown" does not depend on the number of pixels.** A 1.75 MP target (1150x1524) is no faster than an 8.3 MP one (3840x2160). That rules out per-pixel costs (zimg, upload, Slint software composition) as the limit.
3. **It is about 30 in every renderer**, with the Slint software renderer slightly *above* 30 at full screen (up to 36.6). femtovg (vsync-blocking swaps) is at or below 30.
4. Also: the default run needs `C:\Program Files\Vestra Player\` (per-machine install, `runner.ts` path in the logs) rather than `%LOCALAPPDATA%\Programs`, so §1.4's `$exe` must point there. `[vestra.video] HostError: render options need GPU rendering` is expected under software rendering.

How the log's "shown" is counted: it is `FrameSlot::take` on the **UI thread** (`render_target.rs:285-296`). That take runs from the waker's `invoke_from_event_loop` (`main.rs:5155-5163`), which is a posted message, not a timer. "shown ≈ 27-30" therefore means the UI thread **handles the frame message only about 30 times a second**. In other words, the UI thread is busy or blocked about half the time; it is not oversleeping.

### A.2 Checking the coordinator's hypothesis (15.625 ms timer granularity) in code

| Question | Evidence | Result |
| --- | --- | --- |
| How do Slint timers and animations wait on Windows? | `about_to_wait` sets `ControlFlow::wait_duration(next_timer)` (`REG/i-slint-backend-winit-1.18.1/event_loop.rs:260-265`); animations request a redraw each loop turn (`:248-257`) | Waits are `WaitUntil` deadlines |
| How precise is `WaitUntil` in winit 0.30.13 on Windows? | `wait_for_messages_impl` creates a **`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`** timer and waits on it with `MsgWaitForMultipleObjectsEx(…, QS_ALLINPUT, MWMO_INPUTAVAILABLE)` (`REG/winit-0.30.13/src/platform_impl/windows/event_loop.rs:637-680`, `:686-762`). It falls back to millisecond timeouts only if the timer cannot be created, which means Windows 10 before 1803 (`:645-647`) | **Not** subject to the 15.6 ms tick on Windows 10 1803+ / Windows 11 |
| Are redraws requested when a frame arrives? | `show_frame` → `set_property("video-frame")` (`main.rs:1138`) → Slint `request_redraw` → `TimerBasedFrameThrottle` (`frame_throttle.rs:13-28`, `:47-75`; `winitwindowadapter.rs:1849-1854`) → a Repeated timer at `1000/refresh` ms → `winit_window.request_redraw()` → `RedrawWindow(RDW_INTERNALPAINT)` → `WM_PAINT` (`winit …/windows/window.rs:152-157`, `event_loop.rs:1276-1297`) | Yes. Each redraw is delayed by at most one timer period. The period is set from the **monitor refresh rate** that winit reports (`EnumDisplaySettingsExW` `dmDisplayFrequency`, `winit …/windows/monitor.rs:179-193`) |
| How does the timer reschedule? | `activate_timer`: `timeout = now() + duration`, with real time (`REG/i-slint-core-1.18.1/timers.rs:326-331`, `:476-481`) | 60 Hz is reachable at 60 Hz refresh |
| The host's own tick | Slint `Timer` 16 ms Repeated (`main.rs:5165-5172`); frames do not wait for it, it only catches a missed wake (`main.rs:5208-5211`) | Not on the frame path |
| Does anything in our process raise the timer resolution? | libmpv imports `timeBeginPeriod` and `NtSetTimerResolution` (DLL strings). mpv `osdep/timer-win32.c` at `e471496c56` raises it only around its own sleeps (`mp_start_hires_timers` / `mp_end_hires_timers`) and itself uses high-resolution waitable timers | Intermittently raised, process-wide |

**Conclusion: the code does not support "15.6 ms granularity is the cause" on Windows 10 1803+ or Windows 11.**

- Even with coarse timers, the frame message would wake the loop at once (`QS_ALLINPUT`). "shown" would then stay near 60 and only renders would drop.
- The data contradicts a hard 31.25 ms gate: values of 33-36.6 appear.
- `timeBeginPeriod(1)` alone is therefore **unlikely** to fix this. The one case where it would matter is a Windows build older than 1803, where winit falls back to millisecond timeouts.

**The hypothesis that fits all the data, including the "~30 Hz" UI animations: the display runs at 30 Hz** (or Windows reports 30 to winit).

- 4K monitors on HDMI 1.4, some docks, KVMs or cables, and power-saving modes give 3840x2160 @ 30 Hz.
- Then the throttle interval is `1000000/30000 = 33 ms`, so **every** Slint redraw, video or animation, is at 30 Hz in **every** renderer (`frame_throttle.rs:53-58`).
- femtovg additionally blocks `SwapBuffers` on a 30 Hz vsync (`glcontext.rs:208-213`), which keeps the UI thread busy, so takes ≈ 27-30.
- The Slint software renderer does not vsync, so takes can go a bit above 30 (up to 36.6), as observed.
- On macOS (display link, `frame_throttle.rs:22-25`, and a 60/120 Hz panel) none of this happens.

The second candidate, if the display really is at 60 Hz, is UI-thread load during playback. That would mean the host tick plus a Slint render plus the per-frame texture taking about 33 ms on this CPU. Code reading cannot confirm or rule this out; A.4 test T2 tells the two apart.

### A.3 What the user should check first (no rebuild needed)

- **T1. Refresh rate:**
  - Windows Settings → System → Display → Advanced display → "Choose a refresh rate" for the 4K monitor.
  - The rate Vestra itself sees: the info plugin's system section prints each display as `… @ N Hz` (`src/plugins/info/sysinfo.ts:35-40`, data from `platform::displays`, `src/host/src/platform/windows.rs:444-468`). It also prints the Windows version and build, which answers the "1803+" question.
  - Optionally, a browser frame-rate test page.
  - **If it says 30 Hz:** use a DisplayPort cable or an HDMI 2.0+ port and set 60 Hz. No code change is needed, and both the video and the animations should go to 60.
- **T2. Animation with nothing playing:** open the playlist panel with no file loaded (or paused, ideally with the window on a 1080p monitor if there is one).
  - Smooth while idle, ~30 Hz only while playing: the UI thread is loaded during playback (see A.4 step 3).
  - ~30 Hz even idle on a confirmed 60 Hz display: a scheduling problem; then the timer experiment in A.4 step 2 is worth doing.
- **T3. Repeat §1.4 run A with the window on a 60 Hz monitor** (if a second monitor exists). If "shown" goes to about 60 there, T1 is confirmed.

### A.4 Minimal fix for 0.5.1 (ordered by evidence; all within the cross-platform rule)

1. **Diagnostics (needed in every case; about 0.5-1 agent-day, shared code only).** Extend the `VESTRA_FRAME_LOG=1` line (`render_target.rs:332-349`) with:
   - the refresh rate in use (`Render::refresh()`, set at `main.rs:5123-5127`);
   - `renders/s`: count `AfterRendering` in a rendering notifier when no GPU renderer holds it, or count winit `RedrawRequested` through `slint::winit_030` for the Slint software renderer;
   - `presented` (the new-frame renders of §4);
   - the **longest and the average host tick in ms** (time the 16 ms timer body, `main.rs:5171-5235`);
   - the longest Slint render in ms (`BeforeRendering`→`AfterRendering`).

   With this, one log line says which of "30 Hz display", "UI thread saturated" or "render thread" it is. No OS code; it also improves the macOS log.
2. **If T1 shows 60 Hz and T2 shows idle stutter (a scheduling problem): `timeBeginPeriod(1)` while video plays, `timeEndPeriod(1)` when it stops.**
   - The FFI call goes in `src/host/unsafe/win32` (the first-party `unsafe` rule). It is called from `platform/windows.rs`, next to the existing playback power assertion (`power_tick`, `main.rs:3053-3064`, which drives `sleep_launcher`). On macOS and Linux it is a no-op function.
   - Add an env switch for A/B testing (e.g. `VESTRA_TIMER_PERIOD=0|1`).
   - Expected effect, given A.2: none on Windows 10 1803+ and 11; it lifts a 32 Hz cap only on older builds.
   - Power: since Windows 10 2004 the request is per process (others are unaffected). It costs more frequent timer interrupts while playing, roughly a few percent of idle battery on laptops (Microsoft's documented trade-off), and mpv already does this around its sleeps.
   - **Lowest priority; do not ship it as "the fix".**
3. **If the UI thread is saturated (T2 smooth when idle, log shows a long tick or render):** take the cheapest load off the UI thread.
   - The 4-byte RGBA frame (§1.5): femtovg then uploads `RGBA` instead of `RGB`, skipping the driver's CPU swizzle. Needs the alpha probe.
   - Make the host-tick sub-steps that the log shows as long run only on change (for example `refresh_menu` only on revision change, which it already does; `settle_layout` and `window_tick` property reads).
   - Lowest-risk order: the log first, then change only what it shows as long.
4. **Not useful for 0.5.1:**
   - "Redraw on frame arrival" (bypassing Slint's throttle). The throttle is internal to Slint's winit backend (`frame_throttle.rs`) and adds at most one period of latency, not a rate cap, on a 60 Hz display.
   - A "present-driven loop": that is the GPU work (§2, option c′), not a patch.

### A.5 How to verify without a Windows machine, and what the user measures

- **Here (macOS):**
  - unit tests for the extended log line and the counters (pure functions in `render_target.rs`);
  - `npm run portability` (the host compiles for Windows, including the `unsafe/win32` addition if step 2 is done);
  - `cargo clippy -p vestra-win32`;
  - a GUI run on macOS to check the new log columns.
- **CI (infrastructure maintainer, optional):** a GitHub Windows runner has an interactive desktop but no GL 2+, so Slint falls back to software. A 10 s smoke run of a lavfi 60 fps source (for example `av://lavfi:testsrc2=s=640x360:r=60`) with `VESTRA_FRAME_LOG=1` would show Windows scheduling with a trivial target. If "shown" is about 60 there, timer granularity is ruled out on a real Windows kernel without the user. The runner's virtual display reports its own refresh rate, which the new log column would print.
- **The user (after 0.5.1 with the diagnostics):**
  - T1-T3 above;
  - then §1.4 run A again, windowed and full screen, 20 s each, sending the log;
  - the info plugin's display line (Hz) and Windows build;
  - Task Manager CPU %;
  - and, if step 2 shipped, the same run with `VESTRA_TIMER_PERIOD=0` and `=1`.

  Acceptance for this issue: `shown` and `presented` ≥ 59 on a 60 Hz display at the 1150x1524 target, and the playlist animation subjectively smooth. 4K full screen in software may still be CPU-bound; that part is the GPU work.

---

## Appendix B (2026-10-25, Windows machine): redraw pacing and the per-frame cost

API maintainer, on the Windows machine: Ryzen 9 7940HS; the AMD 780M drives the only screen, a 2560x1600 panel that supports 60-240 Hz and runs at 60 Hz, scale 175%; RTX 4070 Laptop idle. `REG` as above. Slint 1.18.1 is the newest release on crates.io on 2026-10-25.

### B.1 Measurements (coordinator, release build of main e887b7b, `VESTRA_FRAME_LOG=1`, ~11 s averages)

Windowed, target 1366x1066, while the user was using the machine with Chrome in front:

| clip | produced | shown (taken) | presented | redraws (old picture) | render mean/max ms | CPU |
| --- | --- | --- | --- | --- | --- | --- |
| AV1 4K60 10-bit | 60.5 | 58.5 | 46.6 | 7.4 | 3.6 / 15 | 31% |
| HEVC 4K60 10-bit | 60.5 | 57.9 | 45.3 | 8.4 | 3.7 / 16 | 32% |
| H.264 4K60 | 60.5 | 59.6 | 48.3 | 7.8 | 3.0 / 10 | 24% |
| H.264 1080p60 | 60.5 | 60.4 | 52.4 | 5.9 | 3.2 / 9 | 16% |
| H.264 23.976 | 24.1 | 24.1 | 24.1 | 22.9 | 2.2 / 7 | 9% |

Second series (logs `c-*`, `d-*`, `e-*`):

- Windowed with no other window on screen: presented 59.1-59.8 (AV1 4K60 and 1080p60; foreground or background; femtovg or Slint's software renderer). So the losses above need something else drawing on the screen, and the user requires the display rate regardless.
- Full screen, target 2560x1600, AV1 4K60: default 54.3 presented (render mean 6.8-7.2 ms), femtovg 48.5 / 52.9 (7.2-8.8 ms), Slint software renderer 59.1 in one run and 43-57 in another. 1080p60 full screen (target 1920x1200): 59.4, render 4.3 ms. Render time grows with the target size.
- One full-screen run (`e-def-fs2`) collapsed for 7 s: produced ~6, presented 1/s, render max 688 ms.

### B.2 Cause 1: Slint's timer frame throttle (pacing)

On every winit platform except Wayland and Apple, Slint paces redraws with `TimerBasedFrameThrottle` (`REG/i-slint-backend-winit-1.18.1/frame_throttle.rs:13-75`):

1. A change (`set_property("video-frame")`, an animation, hover) calls the window adapter's `request_redraw` (`winitwindowadapter.rs:1849-1854`), which starts a **Repeated** Slint timer of `1000000 / refresh_mHz` integer milliseconds: **16 ms at 60 Hz** (`frame_throttle.rs:53-58`).
2. Each tick calls `winit_window.request_redraw()` **unconditionally** and only then checks whether a redraw is still pending (`frame_throttle.rs:59-72`). winit delivers `RedrawRequested` after the callback, so the tick after a draw always asks for one more: **every change is drawn twice**. That is the 24 fps row exactly (24.1 new + 22.9 old-picture draws). Upstream fixed it on 2026-10-20 (slint-ui/slint `881a0bc`, issue #14281), not released yet.
3. The timer is re-armed from when it ran (`REG/i-slint-core-1.18.1/timers.rs:371-372`, `:476-481`: `now + duration`), not from the display: 16 ms plus the wake-up delay plus whatever the UI thread was doing (a draw, a `SwapBuffers` that blocks until a vertical blank, `glcontext.rs:208-213`). It drifts against the 16.7 ms refresh, so some refreshes get two draws and some none.
4. mpv delivers frames on its own audio-synced clock. A draw takes the newest frame (`FrameSlot` keeps one), so with draw instants drifting through the arrivals some frames are never drawn and some draws repeat the old one. The playlist animation runs on the same drifting timer.

macOS uses a display link (`frame_throttle.rs:22-26`, `apple_display_link.rs`), hence 60/60 there. No Slint setting changes this; upstream master still uses the timer on Windows (without the double draw).

### B.3 Cause 2: femtovg re-creates and uploads the video texture twice per frame (cost)

Each software-rendered frame is set as a new `slint::Image`. femtovg 0.27 (`REG/femtovg-0.27.0/src/renderer/opengl/gl_texture.rs:28-107`, `:196-256`) then creates a **new texture** for it: it allocates a zeroed buffer of the whole frame, uploads it with `glTexImage2D` (`GL_RGB`), uploads the real pixels again with `glTexSubImage2D` (`GL_RGB`, `UNPACK_ALIGNMENT 1`) and deletes the previous texture. All of it on the UI thread inside the draw.

Measured on this machine's driver (scratch program, invisible window for a WGL context, AMD 780M, GL 3.3.0 driver 22.40.05.21; 40 frames, `glFinish` included):

| target | femtovg today | reused texture, one `glTexSubImage2D` RGB | RGBA | BGRA | PBO + BGRA |
| --- | --- | --- | --- | --- | --- |
| 1366x1066 | 4.0 ms (max 18.8) | 0.8 ms | 1.0 | 1.2 | 1.0 |
| 2560x1600 | **7.4 ms** (max 15.1) | **1.1 ms** | 1.5 | 1.4 | 2.1 |
| 3840x2160 | **14.3 ms** (max 34.3) | **2.1 ms** | 2.9 | 2.5 | 4.1 |

The 7.4 ms at 2560x1600 matches the full-screen render mean (6.8-8.8 ms). At 3840x2160 this alone takes most of a 16.7 ms refresh. Changing the pixel format or using a PBO does not help on this driver; keeping the texture does (6-7x).

The render thread is not the limit: mpv's software render with the host's options (`vo=libmpv`, `hwdec=auto-copy-safe` → `d3d11va-copy`, the orientation lavfi filter, `rgb24` into a fresh buffer; scratch program, real-time playback) produced **60.0-60.2 fps at both 2560x1600 and 3840x2160** for the AV1, HEVC and H.264 4K60 clips, 0-1 mpv drops.

### B.4 The 688 ms stall

In `e-def-fs2` the `refresh` column reads **240.0 Hz** for exactly the 7 stalled seconds and 60.0 Hz otherwise; most other full-screen runs show 240 Hz for 1-2 s around the switch to full screen (`c-def-fs`, `c-gl-fs`, `e-*-fs`), windowed runs never. The panel offers 60-240 Hz (`Win32_VideoController` MaxRefreshRate 240). So entering full screen makes the display switch mode or variable refresh (AMD FreeSync / Windows variable refresh rate for a full-screen window), and while it settles the driver blocks the GL calls of the draw for hundreds of milliseconds. It is outside the host's code; with the reused texture there are no texture allocations left in the draw, which may shorten it, but that is not verified. To confirm: repeat the full-screen run with FreeSync and Windows "Variable refresh rate" off (Adrenalin and Settings → Display → Graphics); if the stall goes away, the remedy is a setting, or keeping the full-screen window from qualifying as an exclusive full-screen app.

### B.5 Fix

Options for the pacing:

| | design | verdict |
| --- | --- | --- |
| (a) | on each new frame, ask winit for a redraw directly and let `SwapBuffers` pace | **Rejected.** Slint 1.18.1's still-running timer adds an unconditional draw per tick, so up to two draws per frame; femtovg would block in `SwapBuffers`, Slint's software renderer (no vsync) would draw twice per refresh, and animations would stay on the drifting timer. |
| (b) | a vertical blank clock that wakes the UI thread and draws then (Slint's Apple design) | **Chosen.** |
| (c) | upgrade or patch Slint | No release has a fix. Backporting `881a0bc` with `[patch]` (a vendored fork) only removes the double draw; draws would still drift. |

**Pacing** (`src/host/src/frame_pacing.rs`, OS-neutral; `platform::VBLANK_CLOCK` / `vblank_waiter`; `unsafe` only in `vestra-win32::VBlank`):

- A winit `CustomApplicationHandler` (`with_winit_custom_application_handler`, feature `unstable-winit-030` already in use) sees every `RedrawRequested` before Slint. For the main window, while the clock runs, it holds the request and owes a draw to the next vertical blank.
- A thread waits with `D3DKMTWaitForVerticalBlankEvent` on the adapter output of the display that holds the window (re-checked about once a second, so it follows the window; any refresh rate; the display's own adapter on this hybrid machine) and, while armed, wakes the UI thread once per blank. There, if a draw is owed or `Window::has_active_animations()` (the test Slint's display link uses), the window draws; otherwise the clock stops.
- A new video picture is owed directly. A second picture that arrives before the first was drawn waits in the frame slot and is set right after that draw, so arrival jitter near a blank delays a picture by one refresh instead of losing it.
- With the clock stopped, the first request draws at once (no added latency for a single change) and starts the clock.
- Safety: an owed draw that no blank served within 50 ms is drawn from the host tick; a failed wait reopens the clock once, else Slint's pacing returns with a `frame pacing:` stderr line; no clock (remote desktop) means no pacing. `VESTRA_FRAME_PACING=slint` turns it off (A/B, field fallback).
- Only the main window is paced. macOS and Linux: `VBLANK_CLOCK = false`, the handler is not installed, every call is a no-op.
- Verified without a window: the clock wakes every 16.697 ms (min 16.50, max 16.90 over 120 blanks; `cargo test -p vestra-win32 -- --ignored`).

**Reused video texture** (new unsafe crate `src/host/unsafe/texture`, `vestra-video-texture`, registered in `check-unsafe-policy.mjs`; enabled by `platform::VIDEO_TEXTURE`, true on Windows only because it was measured only here):

- With software rendering in an OpenGL (femtovg) window, the main window's rendering notifier keeps one RGB texture of the picture's size and, right before Slint draws, uploads the newest picture into it with one `glTexSubImage2D`; the video area shows it as a borrowed OpenGL texture. The texture is re-created only when the picture size changes. mpv's `rgb24` output stays as it is (the fastest upload measured).
- Slint's software renderer has no OpenGL; there the pixels are still set as an image (it composes them on the CPU, so its full-screen cost remains).

**Diagnostics**: `VESTRA_RENDER_TARGET=WxH` (even sides, at most 3840x2160) makes the software renderer draw every frame at that size whatever the window, so the 4K per-frame cost can be measured on this 2560x1600 screen. Documented in `doc/architecture/README.md` next to `VESTRA_FRAME_LOG`.

Expected effect: one draw per refresh at a fixed phase right after the blank, ~1-2 ms of upload instead of 7-14 ms, so a 60 fps video gets every frame drawn at 2560x1600 and 3840x2160; 24 fps drawn once per frame on a stable cadence; animations advance once per refresh. Remaining old-picture redraws: Slint 1.18.1's double draw still owes one extra draw after an isolated change (24 fps: ~24 a second, now at blanks), until Slint ships `881a0bc`. Not verified in the GUI yet: whether other applications drawing still cost frames (B.7).

### B.6 Acceptance of the first fix build (coordinator, stage `C:\vestra\stage-fix`, logs `f-*`, `g-*`, `h-*`, `i-*`, `k-*`, `l-*`)

Passed (steady state, presented per second):

| case | old build | fix |
| --- | --- | --- |
| AV1 4K60, windowed / full screen 2560x1600 | 46.6 / 48.5-54.3 | **60.2 / 60.1** |
| HEVC 4K60 full screen | - | **60.0** |
| 1080p60 full screen (6 runs) | 59.4 | **59.9-60.1** |
| render mean (UI thread draw) | ~7 ms | **1.4-1.5 ms** |
| CPU of the machine | 31-34% | **23-28%** |
| a busy browser canvas animation beside the player | 59.6-59.9 | **60.1-60.2** |
| sidebar slide while playing | 55-60 | **60** |
| Slint software renderer, full screen 2560x1600 | 43-59 | **59.3** |
| `VESTRA_FRAME_PACING=slint` (Slint's pacing in the fix build), windowed / full screen | | 57.6 / 57.5 |

- 23.976 fps: presented 24.1; old-picture redraws still ~24-27 a second (Slint 1.18.1's double draw, B.2 point 2; upstream fix pending, left as is).
- The earlier windowed 46 with the user at the machine could not be reproduced; the old log shows host ticks at only 55-57 a second then (timers stretched), which the vertical blank pacing does not depend on.

Not passed:

1. **Full-screen entry stall** (B.4): in every run the `refresh` column reads 240 Hz for 1-2 s after the double-click while the UI nearly stops (ticks 3/s, presented 1-2/s); in 2 of ~12 runs it lasted 8 s or more (`f-h264-4k-fs`, `f-1080-fs` with the fix, `e-def-fs2` with the old build).
2. **`VESTRA_RENDER_TARGET=3840x2160`**: the UI presents everything produced, but produced falls short: AV1 43.2 windowed / 43.7 full screen, HEVC 53.1, H.264 57.4, Slint software renderer with AV1 52.5, with the machine's CPU at 25-32%.
3. The fix runs of the trace build logged no events: a bug in the trace build (it switched the trace on only when the pacing was off). Only the `VESTRA_FRAME_PACING=slint` trace runs (`g-tr-*-slint`) are usable; they confirm B.2: 58 draws a second at 60 fps content, with 2 frames a second overwritten before any draw.

### B.7 The full-screen stall is a display mode switch (evidence)

- Slint asks winit for **borderless** full screen (`REG/i-slint-backend-winit-1.18.1/winitwindowadapter.rs:1964`: `Fullscreen::Borderless(None)`), never exclusive, and the host never sets a display mode. The window-resize command of the window plugin is refused while full screen (`main.rs`, `Not available in fullscreen`), so it cannot race the switch.
- In `g-tr-av1-fs-slint` (Slint's pacing, the trace's own vertical blank thread) the display delivered **no vertical blank for 660 ms**, `SwapBuffers` blocked up to **697 ms**, and the **render thread got no frame from mpv for 694 ms** (decoding with d3d11va on the same GPU). In `f-1080-fs` the render thread also slows to 6-14 frames a second during the 240 Hz phase while the UI's draws stay short (1-2 ms). The whole GPU and display pipeline stops, not the host's UI code: the signature of a display timing change (the panel blanks while the mode is set).
- `refresh` is winit's current display mode (`EnumDisplaySettingsExW`): it really becomes 240 Hz, already in the second of the double-click, and returns to 60 Hz when the stall ends. The panel supports 60-240 Hz and the desktop runs at 60 Hz.
- So a window that exactly covers the display is being promoted (by the graphics driver or the system: AMD FreeSync / Windows variable refresh rate for full-screen applications, or Windows Dynamic Refresh Rate) to the panel's native 240 Hz variable-refresh timing, and demoted back. Each switch blanks the display for ~0.5-0.7 s; the long cases switch back and forth.
- Not verifiable here without a window or an admin registry read: which of FreeSync, "Variable refresh rate" or "Dynamic refresh rate" does it. The A/B runs in B.9 decide.
- The trace build's 240 "vertical blanks" a second in `g-tr-*-slint` are an artefact of that build (one logger thread per main-window rebuild, all logging the same blank); the real interval is 16.7 ms.

Host-side mitigation (in the fix build, `platform::fullscreen_resized`, `vestra-win32::unmatch_display`): when winit has made the window cover its display exactly, the host makes it **one pixel taller** (the extra row is past the bottom edge, invisible), from the shell's winit event hook, before Slint draws at the new size. winit resets a full-screen window's rectangle only when it moves to another display (`REG/winit-0.30.13/src/platform_impl/windows/event_loop.rs:1343-1360`), so the size sticks. A window that does not match the display rectangle is not treated as a full-screen application by the driver's or the system's full-screen detection, so no timing switch happens (confirmed in B.9). `VESTRA_FULLSCREEN_MATCH=shorter` (one pixel short) and `=exact` (winit's rectangle, the old behaviour) remain for comparisons. `taller` is the default although `shorter` also avoids the switch: `shorter` leaves the display's bottom pixel row outside the window, so an auto-hide taskbar pops up over the picture when the pointer reaches the bottom edge, and the full-screen bottom bar's trigger band (FS-BAR-1) loses its last row. On a desktop with another display directly below, the extra row of `taller` shows on that display (open item F4, B.10).

### B.8 The 4K render-thread limit: a new 25 MB buffer per frame

Measured headless with mpv's software render and the host's options at 3840x2160 (scratch program, 8 s of real-time playback per row; "pure" = `video-timing-offset=0`, so the call returns without waiting for the frame time):

| buffer per frame | AV1 10-bit | HEVC 10-bit | H.264 |
| --- | --- | --- | --- |
| a plain `Vec` (my earlier test) | 60.0 fps, pure 8.1 ms (p95 13.5) | 60.2, 8.0 (12.3) | 60.2, 7.6 (9.3) |
| **a new `SharedPixelBuffer`** per frame, freed on another thread (what the host did) | **30.0 fps, 27.7 ms** (p95 34.4), 263 mpv drops | **29.2 fps, 27.9 ms** | - |
| **4 reused `SharedPixelBuffer`s** | **60.2 fps, 8.0 ms** (p95 11.6) | **60.0 fps, 8.7 ms** (p95 13.4) | - |

- `SharedPixelBuffer::new` zero-fills a fresh 25 MB block every frame (its data also sits behind a header, so mpv may copy the frame once more to align it), and the UI thread frees the previous one (the UI-side `set` time rose from 0.1 to 1.4 ms at 4K). That costs ~20 ms a frame; the earlier test used a lazily zeroed `Vec`, which is why it showed 60.
- Ruled out by measurement: real audio output (60), a concurrent 60 Hz 4K GL upload load on the same iGPU (60), software decoding (dav1d: pure 5.5 ms, cheaper than d3d11va-copy's 8.1 ms but not needed), `bgr0` instead of `rgb24` (7.5 vs 8.1 ms, not worth the format change).
- Fix: `render_target::BufferRing` (4 buffers, round-robin; the UI releases a buffer within a frame or two, so when it comes round `make_mut_bytes` writes in place; if the UI still held it, copy-on-write keeps it correct). Unit-tested for in-place reuse and resizing.
- The remaining per-frame cost at 3840x2160 is ~8 ms (p95 12-13 ms) of mpv's software render on one thread, within a 16.7 ms refresh but without much margin; the GPU path (§3) removes it.

### B.9 GUI runs for the coordinator (second fix build)

Builds: `C:\vestra\maint-out\vestra-host.exe` (pacing + reused texture + buffer ring + full-screen guard) and `C:\vestra\maint-out\vestra-host-trace.exe` (the same plus the corrected temporary trace: events now also with the pacing on, one vertical blank logger, and `Z<w>x<h>` for each resize and `G` when the guard changed the window). Analysis: `node C:\vestra\maint-out\pace-analyze.mjs <log> [skip seconds]` (now also prints the resizes and the longest gaps between vertical blanks, draws, frames and UI wake-ups after the first resize).

1. 4K render limit: AV1, HEVC and H.264 with `-Env @{VESTRA_RENDER_TARGET='3840x2160'}`, windowed and `-Full`; target produced ≥ 59.5.
2. Full-screen entry, six `-Full` runs each with the AV1 clip:
   - `-Env @{VESTRA_PACE_TRACE='1'}` (guard on, default `taller`);
   - `-Env @{VESTRA_PACE_TRACE='1'; VESTRA_FULLSCREEN_MATCH='shorter'}`;
   - `-Env @{VESTRA_PACE_TRACE='1'; VESTRA_FULLSCREEN_MATCH='exact'}` (old behaviour, the control).
   Look at the `refresh` column and the script's "after the first resize" line. Pass: `refresh` stays 60, no gap between vertical blanks above ~20 ms, draws continuous.
3. If `exact` stalls and `taller` does not, the guard is the fix. If both stall, A/B the system settings with `exact`: Settings → System → Display → Advanced display → refresh rate (is it "Dynamic"?) set to a fixed 60 Hz; Settings → Display → Graphics → "Variable refresh rate" off; AMD Adrenalin FreeSync off; one setting at a time, three `-Full` runs each.
4. Recheck the playlist slide and the bottom bar in full screen (the window is one pixel taller than the screen).

Results (coordinator, second fix build, trace build for step 2):

| case | result |
| --- | --- |
| Forced 3840x2160, AV1 / HEVC / H.264 4K60, windowed and full screen | **60.0-60.2 presented** (was 43.2 / 53.1 / 57.4 produced) |
| Full-screen entry, `exact` (old behaviour), 6 runs | **6/6** switched to 240 Hz with ~690 ms gaps between vertical blanks (`n-exact-1`: `SwapBuffers` max 800 ms, vertical blank gap max 688 ms, 10 refresh intervals with two pictures) |
| Full-screen entry, default (`taller`), 6 runs | **0/6** switches; 611 of 617 refresh intervals with exactly one draw; longest vertical blank gap 22.8 ms |
| Full-screen entry, `shorter`, 6 runs | **0/6** switches |
| Full-screen bottom bar and sidebar (screenshots) | intact |
| Sidebar slide while playing | 60 presented |

The guard is the fix for the stall; the system settings A/B of step 3 was not needed. `taller` stays the default (B.7: `shorter` breaks the auto-hide taskbar and the last row of the bottom bar's trigger band).

### B.10 Remaining work

- Slint with `881a0bc`: removes the remaining extra draw after an isolated change (24 fps); keep the vertical blank pacing.
- Other windows (detached sidebar, settings) keep Slint's pacing.
- Slint's software renderer (no OpenGL: VMs, remote desktop) still composes the whole window on the CPU each frame.
- mpv's software render costs ~8 ms a frame at 3840x2160 on one thread; the GPU path (§3) removes it and the readback.
- A/V offset up to one refresh, as before; display-synced playback belongs with the GPU work.

Open items from the independent review (fixed before merge: the texture upload now sets and restores the whole unpack state; a display clock that returns without waiting, 16 waits in a row under 1 ms, falls back to Slint's pacing with one stderr line; a candidate main window's texture is kept only after it was shown):

- **F2** `vestra-video-texture`: the borrowed texture's image may stay on the video area after `RenderingTeardown` deleted the texture (until the next frame sets a new one). Harmless while the window is torn down with it on Windows; settle it before enabling `VIDEO_TEXTURE` on macOS (a GUI check there too).
- **F4** Full-screen guard: on a desktop with a display directly below, the extra row shows there; check `MonitorFromPoint` just below the display and pick another edge (or `shorter`) then. Also a main window rebuilt while full screen comes up at W×(H+1) of the old window; check it stays one pixel off and does not grow.
- **F5** `BufferRing`: count copy-on-write events (a buffer still held by the UI when it comes round) in the frame log; consider `RING = 5`; release the ring's buffers while playback is stopped.
- **F6** `VBlankWaiter`: log when reopening the clock fails after the window moved to another display (it keeps the old display's clock silently today).
- **F9** macOS: not compiled locally (no macOS target here); CI's macOS job checks it. The macOS paths are the no-op `no_vblank` module, `VIDEO_TEXTURE = false`, the no-op `fullscreen_resized`, and the shared `BufferRing`.

## Appendix C: GPU render gate (Windows, 2026-10-26)

API maintainer; the go/no-go gate of §3 step 4 for option (c′), ordered by the user's ruling of 2026-10-26 (Windows GPU rendering goes into 0.6.0; integration comes after this gate). Same machine as Appendix B: Ryzen 9 7940HS, 16 logical CPUs; the AMD Radeon 780M drives the only screen; RTX 4070 Laptop GPU. Drivers: AMD 31.0.14005.21001 (GL `4.6.0 Compatibility Profile Context 22.40.05.21.231025`, an October 2023 driver), NVIDIA 32.0.16.1062 (GL `4.6.0 NVIDIA 610.62`). libmpv: the pinned shinchiro build in `C:\vestra\deps` (mpv `e471496c56`, render API 2.6).

**Probe.** `src/host/unsafe/gpu-probe/` (crate `vestra-gpu-probe`). It is a standalone crate with its own `[workspace]` and `Cargo.lock`, so it is not a member of `src/host`'s workspace and the player never builds or links it (`cargo metadata` in `src/host` still lists the same six packages; `check-unsafe-policy.mjs` passes). It has the unsafe-crate lints, and every `unsafe` block carries a SAFETY comment (57 blocks, checked with the policy script's own rule 3 code). `vendor/render_d3d11.h` is mpv's header at `e471496c56` (ISC; not in the shinchiro package). It has two binaries:

- `gpu-probe` runs in a normal process.
- `gpu-probe-dgpu` is the same program, but it exports `NvOptimusEnablement` and `AmdPowerXpressRequestHighPerformance` (`build.rs`, `/EXPORT`), which ask the drivers to put the process's OpenGL on the discrete GPU.

```text
. C:\vestra\env.ps1 -Target C:\vestra\target-gpu -Ffmpeg
cargo build --release --manifest-path src/host/unsafe/gpu-probe/Cargo.toml
gpu-probe info                      # adapters, hidden GL context, interop matrix (no mpv)
gpu-probe interop <index|warp>      # one matrix cell; GPU_PROBE_TRACE=1 traces each wglDX call
gpu-probe run --adapter default|warp|<index> --src <file|av://lavfi:…> --size 3840x2160
              --seconds 10 --warmup 2 [--gl] [--sync] [--lag 1] [--host-vf] [--block] [--png out.png]
```

`run` creates the device (`VIDEO_SUPPORT | BGRA_SUPPORT`, FL 11.1, `ID3D11Multithread::SetMultithreadProtected(TRUE)`) and mpv (`vo=libmpv`, `ao=null`, `hwdec=auto-safe`, `loop-file=inf`). It then calls `mpv_render_context_create` with `api_type="d3d11"` and `D3D11_INIT_PARAMS` (param 21), and renders every new frame into a ring of three `B8G8R8A8_UNORM` textures through `D3D11_FBO` (param 22). For each render it times the CPU wall time of `mpv_render_context_render` and the GPU time (timestamp queries). It also records the process CPU time over the measured window. At the end it reads the last frame back through a staging texture into a PNG.

With `--gl`, a hidden window is created: `CreateWindowExW` without `WS_VISIBLE`, `WS_EX_NOACTIVATE`, never shown, so it can never take focus. Its WGL context opens the same D3D device once (`wglDXOpenDeviceNV`) and registers each ring texture (`wglDXRegisterObjectNV`, `GL_TEXTURE_2D`, read-only). For every frame it then locks the texture rendered `--lag` frames earlier, blits it into its own `GL_TEXTURE_2D` (what Slint would get as a `BorrowedOpenGLTexture`), and unlocks it. The three steps are timed separately. At the end the GL copy is read back and compared with the D3D readback pixel for pixel. `--sync` makes the render thread wait for the GPU to finish each frame (event query), the D3D counterpart of the macOS renderer's `glFinish`.

**Conditions.** The user was working on the machine (GPU-heavy work). Each run sampled `\GPU Engine(*)\Utilization Percentage` for the probe process and for everyone else (one sample every ~2 s). Other processes used ≤ 2% 3D on the 780M and 1-13% (peaks up to 23%) on the 4070 during the runs; I saw no interference except possibly the one stall noted in C.3. Runs were 5-10 s measured after a 2 s warm-up; the probe never made a sound (`ao=null`) and never showed a window.

### C.1 Verdict: **GO**

Both questions are answered yes on this machine, with three conditions that the integration must meet (C.4). The two conditions that are not obvious are these:

- **Pick the D3D adapter that matches the GL context by LUID before any `wglDX*` call.** The AMD driver crashes the process on a mismatch.
- **Have the render thread wait until the GPU has finished a frame before publishing it.** Otherwise, on AMD the window's lock blocks the UI thread for ~10 ms per 4K frame.

### C.2 G1: the shipped libmpv renders through the D3D11 render API

| run (3840x2160 target unless noted) | adapter | `hwdec-current` / format | fps (measured) | `render()` CPU ms mean / p95 | GPU ms mean / p95 | process CPU | probe GPU load (3D / video) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `testsrc2` 1080p60, 1920x1080 target | 780M | `no` (raw frames) / yuv420p | 60.18 | 1.23 / 1.92 | (timer bug, fixed after this run) | 1.1% | 15% / - |
| HEVC 4K60 10-bit | 780M | **`d3d11va`** / `d3d11[p010]` | 60.10 | 0.78 / 1.33 | 8.87 / 10.11 | **1.0%** | 55% / 57% |
| AV1 4K60 10-bit | 780M | **`d3d11va`** / `d3d11[p010]` | 60.09 | 0.57 / 1.32 | 8.93 / 10.83 | **0.5%** | 50% / 45% |
| HEVC 4K60 10-bit | RTX 4070 | **`d3d11va`** / `d3d11[p010]` | 60.00 | 0.87 / 1.29 | 5.47 / 8.10 | **0.8%** | 29% / 24% |
| AV1 4K60 10-bit (with `--gl`) | RTX 4070 | **`d3d11va`** / `d3d11[p010]` | 60.05 | 0.69 / 1.28 | 4.86 / 9.16 | 3.3% (with GL, see C.3) | 31% / 26% |
| HEVC 4K60, the host's `vf` chain (`--host-vf`) | 780M | **`d3d11va`** / `d3d11[p010]` | 60.21 | 0.84 / 1.34 | 9.03 / 10.24 | 1.0% | 55% / 57% |
| HEVC 4K60, `BLOCK_FOR_TARGET_TIME=1` (the host's unpaced mode) | 780M | `d3d11va` | 60.00 | 16.6 (blocks until display time) | - | 1.1% | 55% / 57% |
| `testsrc2`, 1920x1080 target | **WARP** | `no` | 19.98 | 50 / 122 | 49 / 116 | 47% | CPU |
| HEVC 4K60, 1920x1080 target | **WARP** | `d3d11va-copy` (WARP has no video support, so mpv decodes on a hardware device of its own and copies) | 6.38 | 157 / 176 | 157 / 172 | 67% | CPU |

- **The API works as documented.** `mpv_render_context_create(api_type="d3d11")` succeeds in 70-293 ms on all three adapters, and `mpv_render_context_render` never returned an error in any run (≈ 9,000 frames in total).
- **Zero-copy**: `hwdec-current = d3d11va` (not `-copy`) with `video-params/pixelformat = d3d11`, `hw-pixelformat = p010` on both GPUs for HEVC and AV1 10-bit: mpv decodes on the device we pass and samples the decoder's textures directly.
  - The host's initial filter chain (`source_geometry::INITIAL_FILTERS`: `lavfi=[null]` + `format`) keeps it zero-copy.
- **4K60 cost**: about 1% CPU of the machine (0.08-0.16 cores) and under 1 ms of CPU per `render()` call. That compares with 24-32% CPU for the software path in B.1.
  - GPU time per 4K frame: 8.9 ms on the 780M and 5.5 ms on the 4070. The 4070 is mostly idle and clocks down; its p95 is 8-9 ms.
  - The 780M then runs at about 50-55% 3D plus 45-57% of its video engine at 4K60. That leaves room for Slint's composition, but not a lot. A 1440p target needs proportionally less.
- **Drops**: 8-20 `frame-drop-count` per run, all within the first 2 s (the count at the end of the warm-up equals the final count in every run without `--sync`). `decoder-frame-drop-count` is 0 everywhere. Startup drops are worth a look during integration (§4 counter), but they are not a gate issue.
- **Picture**: every readback was checked as an image. testsrc2's bars came out upright with the right colours (also on WARP), and the HEVC and AV1 frames looked correct (the AV1 screen-recording clip shows its own frame counter). The PNG files are in the maintainer's scratchpad and are not committed; `--png` reproduces them.
- **WARP** (CI's headless path, `D3D_DRIVER_TYPE_WARP`): the render API works and the picture is right. It is fast enough for a correctness test that renders a few lavfi frames into a texture and checks the pixels, but not for any timing test.

### C.3 G2: the texture shown in OpenGL through `WGL_NV_DX_interop2`

**Which adapter the GL context lands on** (`gpu-probe info` and `gpu-probe-dgpu info`):

| process | GL_RENDERER | `GL_DEVICE_LUID_EXT` → DXGI adapter | DXGI `EnumAdapters1(0)` and `D3D11CreateDevice(NULL)` |
| --- | --- | --- | --- |
| normal (`gpu-probe`; the player today) | AMD Radeon(TM) 780M (`ATI Technologies Inc.`) | `00000000:00011132` = the 780M, node mask 0 | the 780M |
| with the dGPU export hints (`gpu-probe-dgpu`) | NVIDIA GeForce RTX 4070 Laptop GPU (`NVIDIA Corporation`) | `00000000:00012414` = the 4070, node mask 1 | the 4070 (the hints also reorder DXGI: the 4070 is listed first and owns the output) |

- **Extensions**: both drivers expose `WGL_NV_DX_interop` and `WGL_NV_DX_interop2`, `GL_EXT_memory_object_win32` and `GL_EXT_semaphore_win32`.
  - `WGL_NV_gpu_affinity` is absent on both (it is a Quadro feature).
  - `WGL_AMD_gpu_association` is present on AMD only.
  - Neither driver lists `GL_EXT_external_objects_win32`, but `glGetUnsignedBytevEXT(GL_DEVICE_LUID_EXT)` from `GL_EXT_memory_object_win32` works on both. It gives the exact DXGI `AdapterLuid`, which is the key for matching the adapters.
- In both process configurations the default D3D adapter was the GL adapter: Windows' hybrid policy and the driver hints act on DXGI and GL together.

**The hybrid matrix** (a 256x256 texture cleared to a known colour on the D3D side, read in GL through the interop):

| GL context on | D3D device on | result |
| --- | --- | --- |
| 780M | 780M | OK: open + register + lock + blit + read 6-9 ms the first time, all pixels match |
| 780M | RTX 4070 | **process crash**. `wglDXOpenDeviceNV` returns a non-null handle, then `wglDXRegisterObjectNV` faults with an access violation in `atio6axx.dll` (Event Log 1000, `0xc0000005`, offset `0x21552b0`, reproduced 3/3) |
| 780M | WARP | **process crash**, same place |
| RTX 4070 | RTX 4070 | OK, all pixels match |
| RTX 4070 | 780M | clean failure: `wglDXOpenDeviceNV` returns NULL, `GetLastError = 0xC0070032` |
| RTX 4070 | WARP | clean failure, same code |

Neither driver falls back to a copy across adapters. A mismatched pair either fails (NVIDIA) or kills the process (AMD), so the host must never call `wglDX*` without a confirmed match.

**Per-frame cost and correctness at 3840x2160** (the window side: lock the newest finished texture, blit into its own texture, unlock; CPU ms p50 / p95):

| GPUs (GL = D3D) | clip | render thread waits for the GPU (`--sync`) | `wglDXLockObjectsNV` | blit (submit) | `wglDXUnlockObjectsNV` | fps |
| --- | --- | --- | --- | --- | --- | --- |
| 780M | HEVC | no, lag 1 | **9.73 / 10.76** | 0.02 | 1.20 / 1.46 | 60.03 |
| 780M | HEVC | no, lag 0 | 9.67 / 10.50 | 0.02 | 1.17 / 1.55 | 60.06 |
| 780M | HEVC, 1080p target | no | 9.18 / 10.58 | 0.02 | 0.56 / 0.71 | 60.05 |
| 780M | testsrc2 (no decoder) | no | 8.59 / 9.85 | 0.02 | 1.10 / 1.25 | 60.07 |
| 780M | HEVC | **yes** (wait 8.7 ms on the render thread) | **1.16 / 1.70** | 0.02 | 1.22 / 2.46 | 59.40 |
| 780M | AV1 | **yes** (two runs) | **1.02-1.06 / 1.36** | 0.02 | 1.27-1.31 / 1.60-1.64 | 57.42 / 60.03 |
| 4070 | HEVC | no | 0.16 / 0.32 | 0.02 | 0.20 / 0.32 | 60.04 |
| 4070 | HEVC | yes (wait 7.2 ms) | 0.21 / 0.35 | 0.02 | 0.23 / 0.34 | 60.05 |
| 4070 | AV1 | no | 0.17 / 0.31 | 0.02 | 0.21 / 0.37 | 60.05 |

- **Pixels**: in every run the GL copy is **bit-identical** to the D3D readback (100% of pixels, max channel difference 0). GL row 0 is the D3D top row, so Slint gets the texture with `BorrowedOpenGLTextureOrigin::TopLeft`, as the macOS renderer and `vestra-video-texture` already do.
- **AMD's lock waits for all D3D rendering in flight on the device**, not for the registered texture.
  - It costs ~9.7 ms whether the texture is the one just rendered or one rendered a frame earlier (lag 0 vs 1).
  - The cost does not depend on the size (9.2 ms with a 1080p target) or on the decoder (8.6 ms with lavfi). It roughly equals the GPU time of the frame just submitted plus ~2 ms.
  - When the render thread waits for completion first, the lock drops to ~1 ms. The wait (8.7 ms) then happens on the render thread, where it costs nothing.
  - `glFinish` after the unlock adds nothing on AMD, so the lock and unlock are synchronous.
- **NVIDIA's lock and unlock return at once.** The cross-API wait moves to the GPU: a `glFinish` right after the unlock took 8.4 ms (p50 8.6) without `--sync` and 1.5 ms with it. The render-thread wait therefore also keeps NVIDIA's GL queue (and `SwapBuffers`) from stalling behind mpv's frame. The 4K blit itself costs ~1.1 ms of GPU.
- **UI-thread budget per 4K frame** with the render-thread wait: ~2.5 ms on the 780M and ~0.45 ms on the 4070, against a 16.7 ms refresh.
- **CPU with GL**: 5.0-5.4% (AMD) and 3.3-3.7% (NVIDIA). Without GL it is 0.5-1.0%. The difference is busy-waiting: inside AMD's lock, in NVIDIA's `glFinish`, and in the probe's own `yield_now` loop on the event query. The integration should wait on an `ID3D11Fence` event (`ID3D11DeviceContext4::Signal` + `SetEventOnCompletion`, Windows 10 1703+) and never call `glFinish`.
- **One stall**: the first AMD AV1 `--sync` run had one 355 ms `wglDXUnlockObjectsNV` and a 127 ms completion wait (26 drops, 57.4 fps). The repeat ran clean (60.03 fps, unlock max 12.6 ms). The cause is unknown: possibly a driver hiccup or the user's GPU work (others' 3D load on the 4070 peaked at 19% in that run). Watch for it during acceptance.
- **The probe's own limits**:
  - One thread does mpv's render, the completion wait and the window's lock/blit/unlock; the real design splits the render thread from the UI thread. The 59.4 fps of the HEVC `--sync` run (5 drops after warm-up, one 57 ms gap) is that serial loop, not the API.
  - The GL context is a legacy `wglCreateContext` on a hidden window, not Slint's `wglCreateContextAttribsARB` context. It uses the same driver (ICD), but it is not the same context.
  - Nothing was presented (no `SwapBuffers`), so this is not a presented-fps measurement.
  - Not tested: an Intel GPU, a newer AMD driver, device removal (TDR), HDR targets, Windows' per-app "Graphics preference" setting (it would mean writing the user's registry).

### C.4 Conditions for the integration

1. **Adapter match, checked by LUID, before any interop call.**
   - The renderer creates its D3D device on DXGI adapter 0, which matched the GL context in both process configurations.
   - When a window first draws, its view queries `GL_DEVICE_LUID_EXT`; if that is unavailable, it compares `GL_VENDOR` with the DXGI `VendorId`.
   - On a mismatch, or with no interop entry points, the window reports `failed` and the host falls back to software (`backend.rs` fallback).
   - Never call `wglDXOpenDeviceNV` on an unconfirmed pair (AMD crashes).
2. **The render thread finishes the GPU work before publishing.**
   - Use a fence event wait (not a spin) after `mpv_render_context_render`; only finished ring textures are published.
   - The window locks only the newest finished texture, and skips (keeps showing its own copy) if the renderer has taken that texture back. This mirrors the macOS `IOSurfaceIsInUse` skip.
   - The UI thread never waits on mpv.
3. **Lock only around the blit** (one lock and one unlock per new frame), as in §2 (c′) risk 4; `glFinish` is never called.

There are no plugin API changes. The `video.render` options become supported on Windows, which is an addition (§1.6 rule 1), not a behaviour change.

### C.5 Refined integration plan and estimate (replaces §3 step 5)

| # | work | where | estimate |
| --- | --- | --- | --- |
| 1 | Windows module of `vestra-gpu-render` with the same `Renderer`/`Handle` API: D3D11 device (`VIDEO_SUPPORT`, multithread-protected) on adapter 0, recording its LUID; `mpv_render_context_create` with params 21/22 through `rsmpv_sys` (the two structs from `render_d3d11.h`); a ring of 3 `B8G8R8A8` textures; the render thread with the update callback, `report_swap` pacing and the fence-event wait; publish and redraw callbacks as on macOS | `src/host/unsafe/render` (cfg(windows) module; `windows` 0.62 with the D3D11/DXGI/OpenGL features) | 1.5-2 days |
| 2 | Window side in the rendering notifier: the LUID/vendor check; `wglDXOpenDeviceNV` once per window context; register ring textures lazily; lock the newest finished texture, blit into the window's own texture, unlock; `BorrowedOpenGLTexture` `TopLeft`; on `RenderingTeardown` unregister and close (the sidebar detach rebuilds the window while the mpv render context lives on, as on macOS) | same crate | 1-1.5 days |
| 3 | `platform/windows.rs`: `GPU_RENDER = true`, `gpu_start`, the `GpuRenderer`/`GpuHandle`/`GpuFrame` types. `platform/mod.rs`: `no_gpu` only for other systems. `renderer::gpu_name` text "GPU rendering (Direct3D 11)" through i18n | host platform module | 0.5 day |
| 4 | Pacing: route the GPU renderer's redraw requests through the Windows vertical-blank pacing (`frame_pacing`, Appendix B), not Slint's timer (B.2). The GPU view takes the window's one rendering notifier in place of `vestra-video-texture`, as on macOS | shared host code + platform module | 0.5-1 day |
| 5 | Shader-failure detection for shaderc/spirv-cross messages (`video_render.rs`), from a real Windows log sample (the probe can produce one with `--log v` and a broken `glsl-shaders`) | shared host code | 0.5 day |
| 6 | Tests: a WARP test that renders lavfi frames into a texture and checks the pixels (works headless; it could run in CI's Windows job, ~1-2 s); keep the probe's interop matrix as an `#[ignore]` real-machine test | `unsafe/render` tests | 0.5 day |
| 7 | Docs: reference §5.4 and §8.2, the "unsupported on Windows" statements, the architecture notes | `doc/` | 0.5 day |
| 8 | Acceptance per §3 step 6 on this machine with the coordinator (GUI): 4K60 AV1/HEVC/H.264 windowed and full screen, ≥ 59.5 presented fps, CPU ≤ 25%, `hwdec-current = d3d11va`; the fallbacks (`VESTRA_SOFTWARE_RENDER=1`, a forced LUID mismatch, RDP) | coordinator + maintainer | 1-2 days |

**Total: about 6-8.5 agent-days**, down from the 9-13 of §2 (c′), because the probe (3-4 days there) is done and the remaining unknowns are settled. After integration, delete this probe crate or reduce it to the test in row 6.

Open points for the coordinator or the user:

- **Which GPU the player should use on hybrid laptops.** Today the process lands on the iGPU that drives the screen (the 780M), and the integration keeps that. The 780M carries 4K60 at ~55% 3D plus composition. Exporting the dGPU hints from `vestra-host.exe` would move both GL and D3D to the 4070 (lower GPU time, higher power). This is a product choice and not needed for 0.6.0.
- **Coverage.** An Intel GPU and a current AMD driver are untested. The LUID check plus the software fallback makes an unknown driver fail safely, except for a driver that crashes on a *matched* pair, which nothing here suggests.
- **Removing the probe.** `src/host/unsafe/gpu-probe` is not in `check-unsafe-policy.mjs`'s `UNSAFE_CRATES`, because rule 1 checks only workspace members. Once committed, rule 3 (SAFETY comments) covers its files as tracked sources. Removing it after integration avoids an unregistered unsafe directory.

## E. Parked (user ruling, 2026-10-27)

Windows GPU rendering is parked: nothing of it goes to main, not even as an option (branch `gpu-render` kept as is; open reviewer finding: shared NT handles not closed after the GL import). Reasons: on the AMD 780M it never held 60 fps (GPU queueing of mpv's ~9 ms 4K render and the UI draw on the same engine), it needed ~150 unsafe blocks for the D3D11→OpenGL bridge, and its main promise (zero-copy) is defeated by any CPU `vf` filter anyway. The software path holds 60 fps on this machine.

Why the bridge exists: on Windows hardware decoding is D3D-only and mpv's most mature renderer is D3D11, while Slint draws with OpenGL; the two meet only through vendor extensions (WGL_NV_DX_interop, GL_EXT_memory_object_win32). Every cross-platform toolkit that draws with OpenGL on Windows has this (Flutter avoids it with ANGLE on D3D11); D3D-only players (PotPlayer, MPC-HC) don't. macOS has an OS-provided zero-copy path (VideoToolbox → IOSurface → CGL). A Vulkan-only path was researched (`2026-10-vulkan-windows.md`) and rejected for now.

If revisited: first measure the parked D3D11 path on the RTX 4070; the least risky integration would keep mpv on its D3D11 render API and switch Slint on Windows to its wgpu renderer (D3D12 backend), sharing a D3D texture — a bridge between Microsoft APIs instead of D3D↔OpenGL.
