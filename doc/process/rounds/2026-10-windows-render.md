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
