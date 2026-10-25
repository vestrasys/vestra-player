# Round 4a: render size follows the video area (C-SIZE), display layer, frame doubling and render pacing + plugin integration

Branch: `rebuild/features-r4a` (created by the coordinator from the commit after round 3b was merged into `main`). Basis: the 4a schedule in "Step 3: Rulings" of the [negotiation record](2026-10-features-negotiation.md) (below: "the rulings") and its 3.5 "Core size estimate"; E11, E13, E18, H07 of the [feature requirements](../../product/feature-requirements.md) (below: "the requirements document"); §3, §4.3, §7 of the [community workflow](../community-workflow.md); the format and rules of the [round 3b task sheet](2026-10-features-r3b.md) (below: "r3b").

This round started with a feasibility study (§1). The rulings put picture-in-picture (R-W5, R-H8, `window.state.pictureInPicture`) in 4a, on the condition "first confirm that the AVKit bindings are usable in Safe Rust". The study concluded they are **not usable**, so picture-in-picture is not in this round and is escalated to the user under workflow §7 (§7.1). C-GPU in 4b is not feasible either; it goes to the user for a ruling as well (§7.2).

## 1. Preliminary study (2026-10-21, maintainer)

Environment: Apple M2 (8 cores), built-in Retina display 2560×1664, macOS 26.2 (Darwin 25.2); Homebrew mpv / libmpv 0.41.0 (FFmpeg 9.0.2, libplacebo 7.360); host dependencies `rsmpv =0.2.1` (`render` feature), `slint =1.16.1`, `objc2 =0.6.5`, `objc2-app-kit =0.3.2`. All prototypes live in the scratchpad (`…/scratchpad/r4/`) and are not in the repository.

### 1.1 C-SIZE: software rendering follows the video area size: **feasible**

Current state: the `backend.rs` render thread always calls `render_software` at 960×540 RGB24. A 16 ms timer on the UI thread `take()`s the latest frame, copies it into a `SharedPixelBuffer` with `clone_from_slice` and sets it on `video-frame`; `host.slint` then scales it with `image-fit: contain`.

Evidence:

1. **No need to rebuild the render context.** rsmpv's `RenderContext::render_software(width, height, format, stride, &mut [u8])` takes the size on every call (a safe function the host already uses). Changing the size only means a different buffer, not a new `mpv_render_context`. The `sizebench` prototype simulates window dragging by changing the size on every frame (1600×900 → 2107×1173, 40 sizes in total): with a 1080p source there were no dropped frames, and CPU was the same as at a fixed 1920×1080 (54% vs 55%).
2. **Display-layer properties work with software rendering.** The `viewprobe` prototype renders a 16:9 source into an 800×800 target and counts fully black rows and columns:

   | mpv property | Black rows / columns | Meaning |
   | --- | --- | --- |
   | Default (`keepaspect=yes`) | 350 / 0 | Bars top and bottom |
   | `keepaspect=no` | 0 / 0 | Stretched to fill |
   | `panscan=1` | 0 / 0 | Filled and cropped |
   | `video-aspect-override=1.3333` | 200 / 0 | 4:3 |
   | `video-zoom=1` (×2) | 0 / 0 | Zoomed in |
   | `video-zoom=-1` (×0.5) | 575 / 400 | Zoomed out, bars on all sides |

   So as long as the render target has the same aspect ratio as the video area, R-V9 gives correct results; this is also the only precondition named in E11 of the requirements document.
3. **CPU cost** (`/usr/bin/time` over the whole process, including about 1 s of startup, 24 fps, `hwdec=no`, no audio output; unit is percent of one core):

   | Render target | 1080p source | 2160p source |
   | --- | --- | --- |
   | 64×36 (≈ decoding only) | 31% | 55% |
   | 960×540 (current) | 40% | 72% |
   | 1920×1080 | 50–55% | 89–92% |
   | 2880×1620 (Retina fullscreen level) | 95–99% | 109–118% |
   | 3840×2160 | 117% | 121–129% |

   RGB24 and `rgb0` cost the same: `rgb0` also has to fill alpha per pixel, which adds 2.4 ms per frame at 4K. So we keep RGB24 + `Image::from_rgb8`. At every size the render thread steadily produced 24 fps with no dropped frames. Note that `render_software` by default blocks until the frame's target display time (rsmpv does not expose `BLOCK_FOR_TARGET_TIME`), so per-frame time does not show the real cost; the table uses process CPU.
4. **HiDPI**: Slint `window.scale_factor()` (already used by the host, safe) times the logical video area (`video-width` / `video-height` in `host.slint`) gives physical pixels. On this machine the default window is 780×671 logical points, the video area is about 1560×1000 physical pixels, and fullscreen is about 2560×1500.
5. **Handing the frame to the UI needs no copy**: `slint::SharedPixelBuffer<Rgb8Pixel>` is `Send` (`SharedVector<T>: Send` when `T: Send + Sync`, see i-slint-core `sharedvector.rs`). The render thread can render straight into its bytes, and the UI thread only calls `Image::from_rgb8(buffer)`. This removes the current per-frame `clone_from_slice` (25 MB per frame for 4K RGB24). `slint::invoke_from_event_loop` is a safe function and can wake the UI when a frame arrives, instead of polling every 16 ms.

Plan (details in §4.1): derive the target size from the physical video area, then cap it by the source resolution (do not render more pixels on the CPU than the source has; leave the remaining upscaling to Slint's texture sampling); throttle size changes; when paused, actively redraw one frame after a size change.

Risks:

- **Texture upload on the UI thread is not measured** (subagents do not operate the GUI). femtovg uploads one RGB8 image per frame: 6 MB for 1080p fullscreen, 25 MB for 4K. The GUI test must check whether the UI thread keeps up with 60 fps (§6.2).
- With the source-resolution cap, fullscreen upscaling is done by Slint bilinear sampling, which is slightly softer than mpv's scaler. Acceptable: much sharper than the current 960×540 upscale. The requirements document's "quality after zooming" refers to E13 zoom; with zoom > 1 the cap is relaxed by zoom (§4.1).
- A 4K source in Retina fullscreen uses more than one core of process CPU (about 120–130%, real time on M2). This is the inherent cost of software rendering and cannot be avoided before C-GPU (§7.2); it is about 50 percentage points more than now.
- While dragging, the old frame's aspect ratio does not match the new video area, so `contain` shows a thin black bar for one or two frames, without distortion. Acceptable.

### 1.2 Picture-in-picture (R-W5, R-H8): **not feasible** in Safe Rust

Evidence (`objc2-av-kit` / `objc2-av-foundation` / `objc2-core-media` at the latest crates.io version 0.3.2, the same generation as the host's objc2 0.6.5 / `objc2-*` 0.3.2; sources fetched with `cargo fetch` and inspected, and compiled in the `#![forbid(unsafe_code)]` prototype `pipprobe`):

1. **Every AVKit method is an `unsafe fn`**: the bindings generated by `objc2-av-kit` 0.3.2 have 119 `pub unsafe fn` and 0 safe `pub fn`. This includes `AVPictureInPictureController::isPictureInPictureSupported`, `initWithContentSource`, `startPictureInPicture`, `stopPictureInPicture`, and on the sample buffer path `AVPictureInPictureControllerContentSource::initWithSampleBufferDisplayLayer_playbackDelegate` and `invalidatePlaybackState`.
2. **AVFoundation's `AVSampleBufferDisplayLayer` is also all `unsafe fn`** (`new`, `enqueueSampleBuffer`, `flush`, `setControlTimebase`, `isReadyForDisplay`, …). The whole crate has 3357 `unsafe fn` and 6 safe functions (all geometry helpers such as `AVMakeRectWithAspectRatioInsideRect`). The objc2 rule is that any method not manually audited is marked `unsafe`; the AppKit `NSWindow` / `NSEvent` methods the host uses have been audited and are safe, AVKit / AVFoundation have not.
3. **No safe API to construct frames**: `CMSampleBuffer::create_ready_with_image_buffer` is an `unsafe fn` (its parameters are `NonNull<CMSampleTimingInfo>` and an output pointer `NonNull<*mut CMSampleBuffer>`); `CVPixelBufferCreate` / `CVPixelBufferCreateWithBytes` are in an `extern "C-unwind"` block and need `unsafe` to call; `CVPixelBufferGetBaseAddress` itself is safe but only returns `*mut c_void`, so writing pixels requires `unsafe`; `CMTimebase` creation (`create_with_master_clock`) and `set_rate` / `set_time` are `unsafe fn`, and `CMTimebaseCreateWithSourceClock` is not generated yet (it is a `// TODO` in the source).
4. **Delegate class**: `AVPictureInPictureSampleBufferPlaybackDelegate` has to be implemented with `define_class!`. Instantiating it with `msg_send![super(this), init]` requires `unsafe` (E0133).
5. Measured in `pipprobe`: calling `isPictureInPictureSupported`, `AVSampleBufferDisplayLayer::new`, `isReadyForDisplay`, `CVPixelBufferCreate` and the delegate's `init` all fail with `error[E0133]: call to unsafe function … requires unsafe block`.

Conclusion: every step of the sample buffer picture-in-picture chain (checking support, creating the layer, creating the controller, building frames, enqueueing, the timebase) needs `unsafe`; there is no way to write it with safe wrappers only. This is exactly the case in ruling R-H8 "if not possible, escalate under §7". Options in §7.1.

Side finding (unrelated to this round's implementation, but it affects how the "no `unsafe`" rule is checked): `unsafe impl NSObjectProtocol for X {}` written inside the `define_class!` macro **compiles** under `#![forbid(unsafe_code)]` (the lint does not report code produced by macro expansion; measured in prototype probe 3). So `forbid(unsafe_code)` does not catch every `unsafe` keyword. Suggestion: add `grep -rn '\bunsafe\b' src/host/src` to the static checks in §6.1 (the result should be empty except for the `#![forbid(unsafe_code)]` line); the host currently has no `define_class!` / `msg_send!`, and this check is currently empty.

### 1.3 C-GPU (R-H3, 4b): **not feasible** in Safe Rust

Evidence:

1. **libmpv has only two render APIs**: `render.h` in mpv 0.41 defines only `MPV_RENDER_API_TYPE_OPENGL` and `MPV_RENDER_API_TYPE_SW`; there is no Metal / Vulkan embedding interface (Vulkan / Metal are only used in mpv's own window: `gpu-context=macvk`). So an "equivalent Metal path" does not exist; OpenGL is the only option.
2. **rsmpv's OpenGL constructors are `unsafe fn`, on purpose**: `RenderContext::new_opengl` / `new_opengl_drm` are `pub unsafe fn`. The documentation says "the GL context must be current at creation, on every call and on drop", a dynamic condition the type system cannot express; violating it is undefined behavior. Upstream has no plan to make it safe; this is not a missing wrapper.
3. **Bringing an external GL texture into Slint is an `unsafe fn`**: `BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture` and the deprecated `Image::from_borrowed_gl_2d_rgba_texture` are both `pub unsafe fn` (i-slint-core 1.16.1 `graphics/image.rs` lines 997 and 871), because an invalid texture id causes undefined behavior in the driver. `set_rendering_notifier` itself is safe, but the `GraphicsAPI::NativeOpenGL { get_proc_address }` it provides only returns `*const c_void`; creating an FBO / texture means turning that into function pointers (for example `glow::Context::from_loader_function`), which is also `unsafe`.
4. Slint has a safe `wgpu` texture import (`unstable-wgpu-27/28` features, `Image::try_from(wgpu::Texture)`), but mpv cannot render into a wgpu texture. Connecting a GL texture with a wgpu (Metal) texture on macOS goes through IOSurface interop, which is also `unsafe`.
5. Alternatives on the software path: the local FFmpeg has `tonemap`, but not `zscale` (needed for linearization before and after `tonemap`) or the `libplacebo` filter. `minterpolate` exists, but E18 in the requirements document already excluded it because it is far from real time at 1080p. GLSL shaders (E17) only work in the GPU renderer. So E16, E17 and E18 motion interpolation have no solution on the software path.

Conclusion: C-GPU needs at least three `unsafe` places: `rsmpv::new_opengl`, GL calls and the Slint borrowed texture. This falls under workflow §7 "relaxing Safe Rust" and "the established technical route cannot satisfy a key scenario", so work does not start by default. Options in §7.2. `PlaybackStats.video.transfer` (R-P8) only serves E16 and waits for the ruling together with 4b.

## 2. Scope

Only items marked "4a" in the rulings for which §1 concluded feasible; no 4b work ahead of time, no interfaces outside the rulings. If a signature cannot be met, the maintainer records the reason and the adjusted signature in §8.2 of this document.

| Category | Item | New capability | Requirement |
| --- | --- | --- | --- |
| Render size | R-H2 C-SIZE: render size follows the video area (throttled) | — | Precondition for E11, E13; sharpness of all pictures |
| Render pacing | Ruling 3.5 "render pacing": hand frames to the UI as they arrive instead of polling every 16 ms; verified in the same batch as R-V8 | — | E18 frame doubling 48/60 fps |
| Display layer | R-V9 `plugin.video.view()` (single owner) | Existing `processing.video` | video E11, E13 |
| Processing node | R-V8 `{ kind: "frameBlend"; fps }` (with a performance gate) | Existing `processing.video` | video E18 frame doubling |

Against the 4a row of ruling 3.5: of the 2 interfaces only `video.view()` is done; of the 2 extensions only the `frameBlend` node; of the 3 host behaviors only H2 and render pacing.

**Not in this round**:

- Picture-in-picture (R-W5 `window.setPictureInPicture`, R-H8, `window.state.pictureInPicture`): not feasible per §1.2, waiting for the user's ruling (§7.1). Until then the `pictureInPicture` field is **not published**, not even as a version that is always `{ supported: false }` (the same rule as R-V10 "do not publish an interface ahead of time that is always `unsupported`"). The window plugin keeps the 3a "mini size" form.
- C-GPU (R-H3), `video.render()` (R-V10), `transfer` (R-P8): 4b, not feasible per §1.3, waiting for the user's ruling (§7.2).

## 3. Phases and order

| Phase | Done by | Content | Done when |
| --- | --- | --- | --- |
| P0 | Coordinator | Create the branch; record the baseline (§8.1); pass the two questions in §7 to the user (does not block P1) | Baseline written into §8.1 |
| P1 | Maintainer | All core items in §4 (all compatible extensions; C-SIZE changes picture sharpness, not the contract) | All static checks pass; reference docs and TestHost in sync; coordinator review passed |
| P2 | Plugin authors | Assignments in §5 | `check/build/test` all pass |
| P3 | Coordinator | Acceptance in §6, commit and push, merge into `main` | — |

P1 comes in two batches; authors start according to dependencies:

| Batch | Content | Unlocks | Host Rust estimate |
| --- | --- | --- | --- |
| Batch 1: rendering | §4.1 C-SIZE, §4.2 render pacing, §4.4 `frameBlend` | video E18 frame doubling | about 750 |
| Batch 2: display layer | §4.3 `video.view()` | video E11, E13 | about 450 |

§4.3 depends on §4.1 (ruling R-V9: no display layer before C-SIZE). §4.4 can be written in parallel with §4.1, but its acceptance must come after §4.2 (ruling R-V8: verified in the same batch as render pacing).

## 4. Maintainer checklist (P1)

Each item must include all of: host implementation (`src/host/`), SDK types and implementation (`src/sdk/`), TestHost simulation (`src/sdk/plugin/testing.ts`), reference docs (`doc/plugins/reference.md`), unit tests. First-party Rust must not use `unsafe` (including `unsafe impl` in macro arguments, see the side finding in §1.2).

### 4.0 Size budget

Host Rust (`src/host/src/*.rs`, including tests) grows by **about 1 200 lines** net, with a cap of 3 000 lines (the per-round budget in ruling 3.5). The baseline is 30 966 lines at the end of r3b (the P0 re-measurement is authoritative). About half of the 3 000 lines the rulings gave 4a were for picture-in-picture; with picture-in-picture removed this round is clearly smaller, and no other items are added to fill the gap.

| Item | Host change points | Host Rust estimate | SDK / runtime estimate |
| --- | --- | --- | --- |
| §4.1 C-SIZE | `backend.rs`: shared render-target state, allocate `SharedPixelBuffer` at the target size, actively redraw on size change; `main.rs`: compute the target from the video area and scale factor, throttle, cap; new module `render_target.rs` (pure functions: target computation, cap, rounding to even, throttle decision, easy to unit test) | about 450 | — |
| §4.2 Render pacing | `backend.rs` calls `invoke_from_event_loop` when a frame arrives (coalesced wakeups, atomic flag); `main.rs` moves frame fetching from the 16 ms timer to the wakeup callback; produced / shown counters (diagnostic log) | about 150 | — |
| §4.3 `video.view()` | New module `video_view.rs` (owner, field validation, mpv property mapping, reset to defaults, four-state status); cleanup on disable in `plugins.rs`; protocol method `video.view.set` | about 450 | about 200 (including TestHost) |
| §4.4 `frameBlend` | Node in `effects.rs` (validation, `framerate=fps=N`, placed last in the whole graph, at most one globally); `offline_descriptor.rs` supports the node | about 150 | about 30 (types) |
| Total | | **about 1 200** | **about 230** |

If exceeded: if batch 1 measures over 1 200 lines, the maintainer writes down the reason in §8.2; if the whole round exceeds 3 000 lines, §4.3 moves to the next round (E11, E13 stay grayed out), while §4.1, §4.2, §4.4 are not split. Tests are not cut to reduce the line count.

### 4.1 C-SIZE: render size follows the video area (R-H2)

- **Target size**: the physical video area `(video-width, video-height) × window.scale_factor()`, rounded and then made even, minimum 2×2. When the video area is 0 (minimized, no shell), keep the previous target.
- **Source cap**: let the physical video area be A, the source display size S (`video-out-params/dw`, `dh`), and the display-layer zoom `zoom` (§4.3, default 1). The scale factor is `k = min(1, max(S.w / A.w, S.h / A.h) × max(1, zoom))`, and the target is `A × k`. The aspect ratio always equals the video area's, so `keepaspect`, `panscan` and `aspect` give correct results. Total pixels also have an absolute cap of 3840×2160 (8.3 MP); above it the target is scaled down proportionally. When the source size is unknown, cap at 1920×1080.
  - Reason: in the table in §1.1, once rendered pixels exceed the source, CPU keeps rising while the picture gets no sharper. For example, a 1080p source in Retina fullscreen (about 2560×1500): rendering 1920×1080 costs about 55%, rendering at physical resolution about 99%.
  - Generated streams (NES and other `producer`s) are not affected: their frames still go to the UI at their own size.
- **Throttling**: while the window is being dragged, apply a new target at most every 50 ms, and always apply the final value within 50 ms after changes stop; fullscreen toggles and `ScaleFactorChanged` (window moved to another display) apply immediately. Size changes do not rebuild the `RenderContext` (§1.1 item 1); only the buffer is reallocated.
- **When paused**: after the target changes, the render thread actively calls `render_software` once (mpv redraws the current frame) instead of waiting for `update()`; otherwise, dragging the window while paused would leave the picture at the old aspect ratio.
- **Handing to the UI**: the render thread renders straight into `SharedPixelBuffer<Rgb8Pixel>` (RGB24, stride = width × 3), and the UI thread calls `Image::from_rgb8(buffer)`, with no more `clone_from_slice`. `Frame` now carries the buffer itself; `sequence` stays.
- The video image in `host.slint` keeps `image-fit: contain` (no distortion while an old frame with a different aspect ratio is shown during a drag).
- Orientation handling (the publish gate in `source_geometry.rs`), snapshots and offline export (offline ffmpeg), and `--frame-evidence-dir` diagnostics are not affected; diagnostics write PPM at the actual frame size.
- Stats: `PlaybackStats.backend` stays "libmpv · software rendering"; no new fields.
- Unit tests: target computation (HiDPI, cap, rounding to even, unknown source, relaxation for zoom > 1, absolute cap); throttle decision (continuous changes, final value, fullscreen immediate); redraw on size change while paused (using a fake clock or a counter on the render thread).

### 4.2 Render pacing (ruling 3.5, verified in the same batch as R-V8)

- Current state: the UI thread polls the frame slot with a 16 ms (62.5 Hz) `slint::Timer`. At 48 / 60 fps output the two beats differ, so frames are skipped or repeated.
- Change: after the render thread puts in a new frame, if the "wakeup requested" flag is false, it sets it to true and calls `slint::invoke_from_event_loop(…)`; the callback clears the flag, takes the frame and sets `video-frame`. At most one wakeup is in flight at a time, and the frame slot still keeps only the latest frame (when the UI is slow, old frames are dropped, not queued). The 16 ms timer stays for other work but no longer fetches video frames.
- Diagnostics: the host counts "frames produced by rendering" and "frames set to the UI" internally; with the environment variable `VESTRA_FRAME_LOG=1` it prints one line per second to stderr (not part of the public API; used for GUI acceptance, §6.2).
- When the main window is minimized or covered, the render thread keeps rendering at mpv's pace (same as now).

### 4.3 Display layer `plugin.video.view()` (R-V9, existing capability `processing.video`)

```ts
plugin.video.view(): VideoView;   // needs processing.video; the first plugin to call it owns the display layer, other plugins calling it fail with conflict; a second call from the same plugin throws
interface VideoView extends Disposable {
  set(value: Partial<ViewSettings>): void;      // merged write, last write wins; failures go to onError and status
  readonly status: FilterStatus;                // the same four states as filters
  onStatus(listener: (s: FilterStatus) => void): Disposable;
}
interface ViewSettings {
  aspect: number | null;              // null = from source; 0.25–4 → video-aspect-override (null writes -1)
  fit: "keep" | "stretch" | "fill";   // keep: keepaspect=yes, panscan=0; stretch: keepaspect=no, panscan=0; fill: keepaspect=yes, panscan=1
  zoom: number;                       // 0.25–4 → video-zoom = log2(zoom)
  panX: number; panY: number;         // −1–1 → video-pan-x / video-pan-y (in units of video width / height)
}
```

- The signature matches ruling R-V9 (no `scaleX` / `scaleY`). `conflict` has the same semantics as in `provideSequence`: other plugins can take it only after the owner `dispose`s or is disabled.
- Defaults: `{ aspect: null, fit: "keep", zoom: 1, panX: 0, panY: 0 }`. When the owner `dispose`s or is disabled, the core writes the five properties back to their defaults.
- On source change the core **keeps** the current values (mpv keeps these properties across `loadfile`). E13 "reset on source change" is done by the plugin itself, calling `set({ zoom: 1, panX: 0, panY: 0 })` when `sourceId` changes; the reference docs say so.
- Status: `waiting` when there is no ready media (values are kept and applied once ready); `applying` while mpv properties are being written; `applied` on success; `failed` when mpv rejects. Out-of-range values fail synchronously with `invalid` and do not change the applied values.
- `zoom` also feeds the cap computation in §4.1 (more pixels are rendered when zoomed in, so the picture does not go blurry).
- Display-layer effects do not go into snapshots or exports (snapshots use offline ffmpeg, which never goes through vo properties anyway); a unit test asserts that the offline descriptor contains none of these fields.
- Protocol: new method `video.view.set { values }` (the host decides ownership by calling plugin), returning `{ state }`; whether ownership is taken on the first `set` or when the SDK activates `view()` is up to the maintainer and is written into §8.2.
- TestHost: `host.videoView` (owner and current values); `host.setVideoViewOwner(pluginId)` simulates another plugin holding it first (so `view()` gets `conflict`).

### 4.4 `frameBlend` node (R-V8, existing capability `processing.video`)

```ts
{ kind: "frameBlend"; fps: number }   // integer 24–120; FFmpeg framerate=fps=N; not a runtime field (changing it rebuilds this stage)
```

- At most 1 `frameBlend` across all plugins; the second one is `invalid` (the frame rate can only be changed once). The core puts it at the **end** of the whole video graph (after all plugins' chains), so that temporal filters (denoise, deinterlace) work at the original frame rate. Add a row to the node table in reference §8 and state this placement rule.
- Snapshots and exports "with effects" use the same chain: the exported video's frame rate changes accordingly (an acceptance item of ruling R-V8); `offline_descriptor.rs` must support the node, and a unit test asserts that the export descriptor contains `framerate=fps=N`.
- Measured (§1.1 prototype, `vf=lavfi=[framerate=fps=N]`, 1920×1080 target): 1080p source 24 → 48 fps, CPU 53% → 90%; 24 → 60 fps, 110%. 4K source 24 → 48 fps, 93% → 162%; 24 → 60 fps, 200%. The render thread actually outputs 48.0 / 60.0 fps, `frame-drop-count` is 0. The UI side has to be measured in the GUI after §4.2.
- `PlaybackStats.video.fps` reads `estimated-vf-fps`, so with frame doubling on it shows 48 / 60, consistent with how E18 in the requirements document is accepted; the maintainer confirms in a unit test or headless run that this property reflects the frame rate after filters.

### 4.5 Documentation

- Reference docs: add one sentence to §5.4, "render size follows the video area, capped by source resolution" (no field changes); add `frameBlend`, the placement rule and "at most one" to the node table in §8; new subsection "Display layer `plugin.video.view()`" (fields, mapping, ownership, defaults, kept on source change, not in snapshots); new items in §14 TestHost; §17 change notes for this round (C-SIZE changes picture sharpness and CPU usage, not the contract; picture-in-picture and C-GPU wait for the user's ruling).
- Examples: one short example (≤ 30 lines) each for `video.view()` aspect-ratio switching and a `frameBlend` toggle.

## 5. Plugin integration assignments (P2)

Author rules as in r3b §4: write only in your own plugin directory; implement according to the matching section of the requirements document; if a core capability is missing, write a request under workflow §4.1 and keep working on the parts that do not depend on it; implement downgraded items as ruled and update the matching section of the requirements document.

### 5.1 Assignment table

| Author | Plugin | New features (ID → dependency) | Migration and cleanup | Downgrades and docs | Effort |
| --- | --- | --- | --- | --- | --- |
| A | video | E18 frame doubling (`frameBlend`, target = min(original rate × 2, 60), original rate read from `stats().video.fps`; at ≥ 50 no node is added and "Rate already high enough" is shown; one notice when more than 5% of frames are dropped within 5 seconds); E11 aspect ratio and display mode (`aspect`, `fit` of `video.view()`; "Aspect ▶" menu, Ctrl+F6, "Custom Aspect Ratio…" small window); E13 framing zoom and pan (`zoom`, `panX`, `panY`; "Framing ▶" menu and numpad / alternate keys; reset on source change); un-gray the aspect ratio, display and zoom rows on the "Picture" page and the frame doubling checkbox on the "Effects" page | Remove the `LATER` disabled item for frame doubling in `main.ts`; remove the known limitation "zoom enlarges the 960×540 buffer" from the README | Motion interpolation, HDR and shaders stay disabled for the same reason ("needs GPU rendering, not supported in the current version"), waiting for §7.2; the E18 "mutually exclusive" logic does not trigger until motion interpolation is available | Medium-high (frame doubling after batch 1; E11, E13 after batch 2) |
| — | window | None (H07 keeps the 3a "mini size" form) | — | Add one sentence to H07 in the requirements document, "picture-in-picture waits for the user's ruling (round 4a §7.1)", edited by the coordinator | — |

There is only 1 author. Other plugins have no items this round; C-SIZE makes every picture sharper, and §6.2 covers it with regression scenarios.

### 5.2 Entry

- After batch 1 passes review: A starts E18 frame doubling, and at the same time the parts of E11, E13 that do not depend on the core (menus, the custom aspect ratio small window and its validation, numpad bindings, persistence, TestHost tests).
- After batch 2 passes review: A hooks up `video.view()`.

### 5.3 Requirements document rewritten per the rulings (author A edits the video chapter)

| Section | Current wording in the requirements document | Change per the rulings and this round |
| --- | --- | --- |
| E11 core dependencies | `VideoView` has `scaleX`, `scaleY`; "multiple plugins, last write wins" | Remove `scaleX`, `scaleY`; single owner, a second plugin gets `conflict` |
| E13 core dependencies | References `scaleX`, `scaleY`; "before C-SIZE is done, zoom enlarges the 960×540 buffer" | Remove the two fields; delete that known limitation (C-SIZE is done; render size is relaxed with `zoom` when zooming in) |
| E18 | "Motion interpolation in round 4b" | "Motion interpolation waits for the user's ruling (round 4a §7.2)" |

## 6. Acceptance

### 6.1 Static checks and tests

All commands in workflow §6.1; no plugin's test count goes down; menu snapshots only gain this round's new items (video's "Aspect ▶", "Framing ▶"; frame doubling changes from disabled to enabled); new check `grep -rnw unsafe src/host/src`, which must be empty except for the `#![forbid(unsafe_code)]` line (side finding in §1.2).

### 6.2 Real GUI tests (coordinator)

All required scenarios in workflow §6.2, plus this round's:

| Area | Scenario | Evidence |
| --- | --- | --- |
| C-SIZE sharpness | Screenshot a 1080p test pattern (thin lines, small text) in the default window and fullscreen, and compare with screenshots from before this round: thin lines are no longer blurred like a 960×540 upscale | Before / after screenshots |
| C-SIZE dragging | Drag the window corner continuously for 10 seconds while playing 1080p: the picture does not freeze or distort, at most a thin black bar for one or two frames; drag while paused, and the picture redraws at the new aspect ratio | Screen recording or consecutive screenshots; `VESTRA_FRAME_LOG` output |
| C-SIZE HiDPI | Drag the window between the Retina display and an external 1x display (if available): sharpness changes accordingly, no misalignment | Screenshots |
| C-SIZE CPU | Play a 1080p and a 4K source for 1 minute each (default window, fullscreen), record host CPU in Activity Monitor and compare with the table in §1.1 (±20 percentage points allowed); no dropped frames (`stats().framesDropped` does not grow) | CPU readings, stats |
| Render pacing / frame doubling | Turn on frame doubling on a 24 fps panning clip: `stats().video.fps` is 48; `VESTRA_FRAME_LOG` shows "shown" ≥ 95% of produced frames; motion looks smoother; with a 60 fps source and frame doubling on, there is no node in the chain; with a 4K source and frame doubling on, the dropped-frames notice appears only once | Logs, stats, screenshots |
| Frame doubling export | With frame doubling on, export a 5-second clip "with effects": `ffprobe` shows 48 fps | `ffprobe` output |
| Display layer / E11 | On a 16:9 source choose 4:3: bars left and right; drag the window to near 1:1 and choose "Ignore Aspect Ratio": no bars; "Fill Window": no bars, with cropping; a custom 21:9 enters the cycle sequence; snapshots with effects keep their size and aspect ratio; after disabling video, display goes back to source | Screenshots |
| Display layer / E13 | After zooming to 200% with Numpad9 / Alt+Plus the picture is sharp (not an upscaled low-resolution buffer); pan, reset and center all show an OSD; holding zoom stops at 400%; reset after source change; snapshots are still the unzoomed picture | Screenshots |
| Display layer ownership | A temporary probe plugin calls `view()` before video: every item in video's "Aspect ▶" fails and shows the reason; after the probe is disabled, video can take it | Screenshots, logs |
| Regression | NES shows a picture and clears properly; the backdrop (visualizer) shows when there is no video; the picture is fine after switching the sidebar between floating and attached; subtitle position follows the video area correctly | Screenshots |

Defect ownership as in workflow §6.2.

## 7. User rulings needed

The two questions are independent. Until the user answers, only work that depends on the answer is paused (workflow §7): §4 and §5 of this round are not affected and proceed as planned.

### 7.1 Picture-in-picture (H07 / R-W5 / R-H8)

> **User ruling (2026-10-21)**: build a prototype first, then decide. The prototype goes in a separate subpackage `src/host/unsafe/pip/` (only it may use `unsafe`; the main host package keeps `#![forbid(unsafe_code)]`; static checks require `unsafe` to appear only under `src/host/unsafe/`, and every `unsafe` block has a SAFETY comment), exposes only safe interfaces, capped at 500 lines. The prototype reports the number of `unsafe` blocks and their line count, and the user decides whether to do it for real; otherwise mini size stays.

**Problem**: macOS system picture-in-picture can only be implemented through AVKit's `AVPictureInPictureController` + `AVSampleBufferDisplayLayer`, and the currently available Rust bindings (the objc2 0.3.2 family) mark every method on this chain, and the construction of `CMSampleBuffer` / `CVPixelBuffer` / `CMTimebase`, as `unsafe` (§1.2). It cannot be built under "no `unsafe` in first-party Rust".

| Option | Approach | Cost |
| --- | --- | --- |
| A. No system picture-in-picture | H07 permanently uses the "mini size" already shipped in 3a (smaller video area + always on top); remove the `pictureInPicture` field and W5 | Zero core cost. System picture-in-picture is lost (staying above fullscreen apps, following Space switches, position managed by the system). The user already has a working alternative |
| B. Allow one isolated `unsafe` module | The host adds one module (or a separate crate in the workspace, for example `vestra-pip`) that alone has `#![allow(unsafe_code)]`; all other modules keep `forbid`. Every `unsafe` block gets `// SAFETY:`, unit tests cover frame construction and the timebase; only a safe `Pip::start/stop/enqueue(frame, pts)` is exposed | Changes the established principle "no `unsafe` in first-party Rust" and sets a precedent. Estimated host 1 200–1 600 lines (of which the `unsafe` module is about 500), one RGB→BGRA copy per frame; needs manual review of memory and thread safety (delegate callbacks on the main thread, frames on the render thread). Could be delivered separately as round 4c |
| C. Process boundary (no Rust `unsafe`) | A small helper process written in Swift (no Dock icon), started by the host; frames passed over shared memory or a pipe (960×540 BGRA at 30 fps is about 60 MB/s), play / pause / return as JSON over stdin/stdout | First-party Rust still has no `unsafe`, but it brings in a second native language, the Swift toolchain and extra signing / packaging steps (workflow §7 "established technical route"). Synchronizing timestamps across processes is harder, and the A/V sync acceptance (±40 ms) is at risk. Estimated Swift about 500 lines + host about 600 lines |
| D. Wait for upstream | Submit audits to objc2 to mark the AVKit / AVFoundation methods used by picture-in-picture as safe; `CMSampleBuffer` / `CVPixelBuffer` construction needs hand-written safe wrappers added upstream | Timing unknown (safe wrappers for CF output-pointer functions are not scheduled); meanwhile H07 stays at mini size, which amounts to doing A first |

Maintainer recommendation: **A**, with D as a long-term tracking item. Reason: mini size already covers the main use of "watching in a small window"; B and C both change a principle or the technical route for a single feature, and the cost clearly outweighs the benefit.

The one-line answer needed from the user: **for picture-in-picture, A (mini size only), B (allow an isolated `unsafe` module), C (Swift helper process) or D (wait for upstream, A in the meantime)?**

### 7.2 C-GPU (4b: R-H3, R-V10, R-P8; requirements E16, E17, E18 motion interpolation)

> **User ruling (2026-10-21, revised)**: GPU rendering matters (mainstream desktop players support it; it also affects 4K CPU usage and zero-copy hardware decoding), so build a prototype first: in a separate subpackage `src/host/unsafe/render/`, same isolation rules as §7.1, capped at 500 lines, verifying that libmpv OpenGL can render to a texture that Slint displays. The prototype reports: the number of `unsafe` blocks and their line count, 4K playback CPU compared with software rendering, and whether playback continuity holds (source change and interface rebuild without interruption). When switching for real, software rendering stays as a fallback. E16/E17/E18 motion interpolation come back depending on the prototype's conclusion.

**Problem**: GPU rendering can only go through libmpv's OpenGL render interface (mpv 0.41 has no Metal / Vulkan embedding interface). rsmpv's `new_opengl` is an `unsafe fn` by design, bringing an external GL texture into Slint (`BorrowedOpenGLTextureBuilder`) is also an `unsafe fn`, and GL calls themselves need `unsafe` too (§1.3).

| Option | Approach | Cost |
| --- | --- | --- |
| A. Drop E16 / E17 / E18 motion interpolation | Keep software rendering; no `video.render()`, no `transfer`; the video plugin removes these three items from its menu, or keeps them disabled with a permanent reason | Zero core cost, the Safe Rust principle stays. HDR tone mapping (HDR sources look washed out on SDR displays), custom GLSL shaders and motion-interpolated frames are lost; hardware decoding (`hwdec`) and higher-quality scaling go with them. Frame doubling (4a) still works |
| B. Allow one isolated `unsafe` render module | One module / crate holds the `unsafe`: in Slint's rendering notifier (femtovg = OpenGL) it creates the FBO and texture, calls `rsmpv::new_opengl` / `render_opengl`, and hands the result to the video area with `BorrowedOpenGLTextureBuilder`; rendering has to happen on the UI thread (Slint's GL context), or in a shared context on a separate thread (more `unsafe`) | Changes the established principle. Estimated host 2 000–3 000 lines (of which `unsafe` is about 400–600), a round of its own. Technical risks: depends on Slint continuing to use the OpenGL renderer on macOS (femtovg-wgpu and Skia already exist; if the default renderer changes, this has to be rewritten); macOS has deprecated OpenGL (still usable, up to 4.1); multiple windows (floating sidebar, picture-in-picture) cannot share a borrowed texture. Benefits: `hwdec=videotoolbox`, gpu-next and all of E16/E17/E18 become possible, and 4K CPU drops sharply |
| C. mpv native subview (`wid`) | Set mpv's `wid` to the `NSView` pointer of the video area (an integer conversion, no `unsafe` on the Rust side), and let mpv draw by itself with `vo=gpu-next`, `gpu-context=macvk` | The mpv 0.41 manual documents `--wid` only for X11, win32 and Android; whether libmpv supports it on macOS is **unverified** and needs a GUI prototype by the coordinator. Even if it works: the on-screen messages, info blocks, backdrop and the fullscreen floating bottom bar that Slint draws cannot be drawn on top of a native subview, so these overlays would have to become separate child windows, which amounts to redoing the video area composition (the r3b backdrop and drop hit-testing would both change). Highest risk |
| D. Wait for upstream | Wait for mpv to offer a Vulkan / Metal render API, plus a safe Slint wgpu texture import (Slint already has `unstable-wgpu-28`) and safe mpv→wgpu interop; or wait for rsmpv / Slint to offer safe GL texture sharing | Timing unknown (both rsmpv and Slint made `unsafe` a deliberate design, not a missing wrapper); meanwhile this equals A |

Maintainer recommendation: **A** (recorded as "not for now", D tracked long term). If the user considers HDR playback a key scenario, **B** is recommended, starting with an `unsafe` prototype round of at most 500 lines that confirms the frame timing in Slint femtovg's rendering notifier and the multi-window limits before committing a full round.

The one-line answer needed from the user: **for C-GPU, A (drop HDR / shaders / motion interpolation), B (allow an isolated `unsafe` render module, prototype first), C (try an mpv native subview) or D (wait for upstream, A in the meantime)?**

### 7.3 Prototype results (2026-10-21, maintainer)

Following the user rulings in §7.1 and §7.2, two prototypes were built, each in its own commit: `7100e1a` "Prototype: GPU render (feature gpu-render)" and `36bc717` "Prototype: picture-in-picture (feature pip)". Based on `rebuild/features-r4a` `cf2f448`.

**Isolation (same for both prototypes)**

- `unsafe` appears only in `src/host/unsafe/render/` (crate `vestra-gpu-render`) and `src/host/unsafe/pip/` (crate `vestra-pip`). Both crates are members of a new `[workspace]` in `src/host/Cargo.toml` and share the host's `Cargo.lock`. The host's default `cargo test` / `clippy` still check only the host package.
- The main host package keeps `unsafe_code = "forbid"`. `grep -rnw unsafe src/host/src` is empty except for the `#![forbid(unsafe_code)]` line.
- Every `unsafe` block and `unsafe impl` has a `// SAFETY:` comment (checked block by block with a script, none missing). Private `unsafe fn`s document their preconditions under `# Safety`. Both crates expose only safe types and functions.
- The host hooks them in with cargo features `gpu-render` and `pip`, both off by default. All hook-in code in the host is under `#[cfg(feature = …)]`, so the default build's behavior and code paths are unchanged.
- Checks: with default features, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked` (220 tests) all pass; with `--features gpu-render`, `--features pip` and `--features "gpu-render pip"`, clippy `-D warnings` and the release build all pass; the two crates have 1 test each, both passing (`cargo test -p vestra-gpu-render -p vestra-pip`).

#### 7.3.1 Prototype 1: GPU rendering (`gpu-render`)

**Approach**: libmpv renders on its own thread, in its own offscreen CGL context (OpenGL 4.1 core), through `rsmpv::new_opengl`, into 3 rotating IOSurfaces. Each Slint window, in `BeforeRendering` of `set_rendering_notifier` (femtovg = OpenGL, one GL context per window), binds the latest one as a texture with `CGLTexImageIOSurface2D`, copies it into its own 2D texture with `glBlitFramebuffer`, and sets that on `video-frame` with `BorrowedOpenGLTextureBuilder`. When a frame arrives, `upgrade_in_event_loop` requests a redraw. The render thread still blocks until the frame's target display time; the UI thread only does the GPU copy. Hardware decoding defaults to `hwdec=auto-safe` (measured on this machine: `videotoolbox`) and can be overridden with `VESTRA_HWDEC`. Headless and `VESTRA_SOFTWARE_RENDER=1` still use software rendering as the fallback.

**Why not create the mpv render context directly in Slint's context**: the probe `ctxswap` (scratchpad `r4/gpu/ctxswap`) showed that freeing the mpv render context during playback **stops playback entirely**: `time-pos` becomes unavailable, and neither creating a new context nor setting `vid=auto` recovers it; only a new `loadfile` does. Also, creating a second context while the old one still exists fails (-20). The host rebuilds the shell window on every mount change, and each window is a new GL context, so the mpv render context cannot be tied to a window. Sharing IOSurfaces across contexts solves exactly this: the render context lives as long as the player, and windows only display.

**Numbers**

| Item | Result |
| --- | --- |
| Crate lines | `src/lib.rs` 500 lines (including 1 test, 46 lines), `Cargo.toml` 20 lines. `rustfmt.toml` sets `use_small_heuristics = "Max"` (width is still 100); in the default format it is 532 lines |
| `unsafe` blocks | 19 (1 of them in a test), 131 lines inside the blocks (including braces) |
| Other `unsafe` | 1 `unsafe extern "C"` (7 CGL functions), 2 `unsafe impl Send/Sync`, 4 private `unsafe fn` |
| Host hook-in | `backend.rs` +47, `main.rs` +29 (all under cfg) |
| Safe interface | `Renderer::start(Arc<Mpv>, size)`, `Handle::set_size`, `Handle::frames`, `Handle::attach(&component, show)` |

4K source CPU (no window, `/usr/bin/time` over the whole process, 10 s, `testsrc2` 3840×2160 30 fps H.264, `ao=null`, unit is percent of one core; the software column uses `sizebench` from §1.1, RGB24; all three modes held a steady 30 fps with no dropped frames):

| Render target | Software rendering (current) | GPU, `hwdec=no` | GPU, `videotoolbox` |
| --- | --- | --- | --- |
| 1920×1080 | 95% | 63% | **16%** |
| 2560×1440 | 120% | 68% | **13%** |
| 3840×2160 | 133% | 68% | **12%** |

GPU rendering cuts 4K CPU from 1.2–1.3 cores to about 0.13 cores (about −90%); changing only the renderer, without hardware decoding, still saves half. These numbers do not include Slint's copy and compositing in a window (GPU work), nor the UI-thread cost of femtovg uploading a texture per frame on the software path (§1.1 risk 1), so the real gap in a window should be larger; it has to be confirmed by GUI measurement (steps below).

**Playback continuity**

- Source change: in the crate test the same renderer runs `loadfile` twice in a row, and frames keep coming after the second one. The host does not touch the renderer on source change.
- Resize: in the test the render size changes from 64×64 to 32×16, and the next frame has the new size; a resize while paused redraws one frame.
- Interface rebuild: the mpv render context belongs only to the render thread, and window rebuilds do not touch it, so playback is not interrupted (code analysis, based on the `ctxswap` measurement above). A new window attaches the rendering notifier before `show()`; on its first redraw it copies the latest IOSurface directly, so the picture shows immediately even while paused. On Teardown, the old window only deletes its own texture. When a frame arrives, redraws are requested for the last 4 windows that were attached (closed ones are ignored), so even if a rebuild fails and the old window is kept, it keeps receiving frames. This still needs GUI measurement.
- Orientation: verified in the test with a lavfi picture that is white on top and black at the bottom; the first row of the IOSurface is the top of the picture, matching Slint's `TopLeft`.
- Side verification (E17): in the same test, setting mpv `glsl-shaders` to an inverting shader flipped the top and bottom colors of the output. This shows that libmpv's OpenGL render path supports user GLSL shaders. `interpolation=yes` + `video-sync=display-resample` can be set successfully; the effect was not verified. HDR tone mapping (E16) was not verified, as there was no HDR source.

**Limits and what the prototype does not do** (to be added for the real version)

1. Depends on Slint's femtovg (OpenGL) renderer. If Slint switches to Skia / wgpu, the notifier no longer provides `NativeOpenGL`; the prototype then simply shows no video instead of failing, and the display side has to be rewritten. macOS has deprecated OpenGL (still usable on macOS 26, up to 4.1).
2. Video is shown only in the shell window, not in floating windows.
3. Several details of the software path are not wired up: the publish gate while source orientation is being prepared (a rotated source may flash one unrotated frame); the C-SIZE source cap and the zoom target (GPU renders directly at the physical video area and mpv scales); `VESTRA_FRAME_LOG`, `stats().framesShown`; `stats().backend` still says "software rendering"; producer (NES) frames may be covered by the GPU image when the window is resized.
4. Synchronization is `glFinish` on the render thread plus 3 rotating surfaces. Only a window more than two frames behind would read the surface being written; tearing is theoretically possible and needs GUI observation.
5. Whether snapshots (`take_snapshot`) include the borrowed texture is not verified. Offline export is not affected (FFmpeg decodes independently).

**Conclusion and recommendation: do it, under conditions.** The gain is large: about −90% 4K CPU; `videotoolbox` zero copy; E17 shaders already work; E16 and E18 at least have an implementation path. The cost is an isolated 500-line crate with 19 `unsafe` blocks, plus the dependency on femtovg. Conditions: (1) the GUI measurements below pass (orientation, no tearing, no interruption on rebuild, clearly lower CPU in a window); (2) the user accepts this `unsafe` scope and the constraint "locked to the femtovg renderer"; (3) the real version keeps software rendering as a fallback (headless, on error, user switch) and fills in item 3 above. The real version is estimated at about 700 lines of crate and about 400 lines of host, and can be done as its own round (4b).

#### 7.3.2 Prototype 2: picture-in-picture (`pip`)

**Approach**: the `AVSampleBufferDisplayLayer` is the backing layer of a dedicated layer-hosting `NSView`. This view is the same size as the content view, autoresizes with it, and sits in the window's frame view **behind** the content view: its geometry is real, but it is hidden behind the opaque OpenGL content (see §7.3.4 for the fix). `AVPictureInPictureController` uses a sample buffer content source and is created only after this view is in the window. Each time the host's `show_frame` gets a software-rendered RGB frame, it copies it into an IOSurface-backed BGRA `CVPixelBuffer`, wraps it in a `CMSampleBuffer`, marks it `DisplayImmediately` and enqueues it. This way no timebase (`CMTimebase`) is needed. The playback delegate is implemented with `define_class!`: it turns play / pause and forward / back in the picture-in-picture window into `Request`s, which the host takes in `window_tick` and runs as the core actions `play-pause` / `seek-forward` / `seek-backward`. The content is reported as live (infinite time range), so the picture-in-picture window has no progress bar. With `VESTRA_PIP=1`, the host starts picture-in-picture automatically once AVKit reports that it can start, and prints the two lines `pip: ready …` / `pip: possible, starting` to stderr for diagnosis.

**Numbers**

| Item | Result |
| --- | --- |
| Crate lines | `src/lib.rs` 418 lines (including 1 test, 22 lines), `Cargo.toml` 24 lines (after the §7.3.4 fix; 369 lines before the fix) |
| `unsafe` blocks | 17 (1 of them in a test), 115 lines inside the blocks (including braces) (15 blocks, 119 lines before the fix) |
| Other `unsafe` | 2 `unsafe impl` inside `define_class!` (NSObjectProtocol, the playback delegate protocol), 1 `#[unsafe(super(NSObject))]`, 5 `#[unsafe(method(…))]` |
| Host hook-in | `main.rs` +63 (all under cfg) |
| Safe interface | `Pip::supported/new(&NSWindow)/possible/active/start/stop/set_paused/requests/render_size/enqueue(w, h, rgb)` |
| Verified headless | `isPictureInPictureSupported` returns `true` on this machine; creating the RGB→BGRA pixel buffer and a sample buffer with `DisplayImmediately` succeeds (crate test) |

**System limits encountered**

1. **Needs an NSView host layer with real geometry**: the display layer must be in the window's layer tree, and when AVKit starts it lays out its own host view based on this layer's geometry. The first version used a bare 1×1 sublayer on the content view, which crashed at start (§7.4). After the fix it is a dedicated layer-hosting view behind the content view (§7.3.4). Whether picture-in-picture works with a hidden view needs a GUI retest; if not, compare with `VESTRA_PIP_OVERLAY=1`.
2. **Main thread**: the controller and the delegate can only be used on the main thread (`MainThreadMarker`). The host already delivers frames on the UI thread (that is, the main thread), so there is no extra thread hop.
3. **OS version**: sample buffer picture-in-picture needs macOS 12+. The prototype uses the layer's `sampleBufferRenderer` (macOS 14+), because objc2 marks the older `enqueueSampleBuffer` on the layer as deprecated and `clippy -D warnings` does not allow it. Supporting 12–13 would require allowing deprecated APIs.
4. **Permissions / signing**: picture-in-picture on macOS needs no permission prompt, `Info.plist` key or extra entitlement (the background audio mode is only needed on iOS). Unsigned development builds also compile and create the objects. Whether it can start automatically without a user click (`startPictureInPicture`) is not verified yet; this decides whether R-W5 can be triggered directly by a plugin.
5. **Per-frame copy**: one CPU copy RGB24 → BGRA, and a new `CVPixelBuffer` per frame, without a pool. About 6 MB per frame at 1560×1000. The real version should use `CVPixelBufferPool`; if done together with GPU rendering, it can wrap the rendered IOSurface directly with `CVPixelBufferCreateWithIOSurface`, with zero copy.
6. **Combined with GPU rendering**: with `gpu-render` on, `show_frame` is not called, so picture-in-picture receives no frames (the two features compile together but cannot be used together).
7. **Interface rebuild**: the layer hangs on the old window; the prototype recreates picture-in-picture after a rebuild, so the picture-in-picture window closes and reopens. The real version should move the layer to the new window.
8. Forward / back use the user's configured seek step, not the interval given by picture-in-picture.

**Conclusion and recommendation: do it under conditions, depending on GUI measurement.** The `unsafe` scope is smaller than estimated (17 blocks, 115 lines; §7.1 estimated about 500 lines), and the external interface is safe. Whether it is worth doing depends on GUI measurement: whether system picture-in-picture can start in this host, show a picture and pass button presses back. If item 1 or item 4 fails in practice (the GL view cannot host the layer, or a user click is required to start but the host cannot get that gesture), the recommendation is option A in §7.1, keeping mini size. If it works, the recommendation is to schedule it together with C-GPU and share the IOSurface, removing the per-frame copy.

#### 7.3.3 GUI measurement steps for the coordinator

Build (one binary with both features, switched by environment variables; enabling just one also works):

```sh
cd src/host
CARGO_TARGET_DIR=/tmp/vestra-review-target/r4-proto-gpu cargo build --locked --release --features "gpu-render pip"
BIN=/tmp/vestra-review-target/r4-proto-gpu/release/vestra-host
S=/private/tmp/claude-501/-Users-ziyang-Develop-vestra-player/7b0dd90c-cb6e-48be-9332-6cfbc6f60a9d/scratchpad
```

Sources: `$S/r4/media/t2160-30s.mp4` (3840×2160 30 fps, 30 s), `$S/r4/media/t1080.mp4`. CPU sampling script: `zsh $S/r4/gpu/gui-cpu.sh label seconds`. It samples the host process (`host` or `vestra-host`, or set `PID=…`) with `top` once per second, drops the first sample, and appends the average and maximum to `$S/r4/gpu/gui-cpu.txt`. For a headless comparison, rerun `bash $S/r4/gpu/cpu.sh`.

| # | Scenario | Steps | Expected / record |
| --- | --- | --- | --- |
| G1 | Software rendering baseline | `VESTRA_SOFTWARE_RENDER=1 zsh $S/launch.sh $BIN`, open and play the 4K source; in the default window `zsh $S/r4/gpu/gui-cpu.sh "sw window" 20`, in fullscreen `… "sw fullscreen" 20` | Record CPU, compare with §1.1 (about 120–130%) |
| G2 | GPU + videotoolbox | `zsh $S/launch.sh $BIN`, same source and both window states, labels `gpu window` / `gpu fullscreen` | Picture upright and sharp, matching the G1 screenshots; on-screen messages, info blocks and the fullscreen floating bottom bar still draw on top of the video; CPU clearly lower than G1 (headless: 13%); `stats().framesDropped` does not grow |
| G3 | GPU without hardware decoding | `VESTRA_HWDEC=no zsh $S/launch.sh $BIN`, labels `gpu-nohw …` | Record CPU (headless: 68%) |
| G4 | Dragging / tearing | In the G2 state, drag the window corner continuously for 10 seconds; play 1080p horizontally panning content for 30 seconds | The picture follows the window size without distortion, no tearing or flickering vertical offset |
| G5 | No interruption on interface rebuild | During G2 playback, do something that rebuilds the shell (disable and re-enable a plugin that mounts UI, for example visualizer or a sidebar plugin), once paused and once playing | Playback time keeps moving forward, no reload, no black screen for more than one or two frames; when paused, the new window immediately shows the still picture |
| G6 | Source change | During G2 playback open another file, then the first one again | A picture every time; no leftover old picture |
| G7 | Snapshot | Under G2, take a snapshot with effects / a window snapshot | Record whether the snapshot contains the video (not verified by the prototype) |
| P1 | Picture-in-picture start | `VESTRA_PIP=1 VESTRA_SOFTWARE_RENDER=1 zsh $S/launch.sh $BIN`, open and play the 1080p source; check `$S/launch.out` for `pip: ready (supported: true)` and `pip: possible, starting` | The system picture-in-picture window appears and plays; record whether it started automatically without a click |
| P2 | Picture-in-picture buttons | In the picture-in-picture window click pause / play, forward / back | The host pauses / resumes and seeks accordingly; when paused in the host, the small window's button state follows |
| P3 | Main window | With picture-in-picture open, look at the main window's video area, then close picture-in-picture | The main window picture is normal (GL drawing not affected by the sublayer); after closing, you can return to the main window |
| P4 | Picture-in-picture CPU | In the P1 state `zsh $S/r4/gpu/gui-cpu.sh "pip sw" 20`, compared with picture-in-picture closed | Record the extra cost of the per-frame copy |

If `pip: possible, starting` does not appear in P1, the start condition was never met: AVKit does not consider picture-in-picture possible, or the window is not visible, the host view geometry is invalid, or fewer than 3 frames were enqueued. In that case compare with `VESTRA_PIP_OVERLAY=1` as in R2 of §7.3.4. If it appears but no small window shows, starting needs a user gesture. Both cases are handled as in "Conclusion and recommendation" above.

#### 7.3.4 Picture-in-picture crash fix (maintainer, 2026-10-21)

**Diagnosis**: the `asi` of crash report `host-2026-10-21-100828.ips` says `Invalid view geometry: x is NaN`. The stack is `-[AVPictureInPictureSampleBufferDisplayLayerHostView _updateGeometryIfNeeded]` → `-[NSView setFrame:]` → `_NSViewValidateGeometry`, during layout in the main thread's display cycle (`NSDisplayCycleFlush` → `layoutIfNeeded`). In other words, after `startPictureInPicture`, AVKit computed its host view's frame from the content source layer's geometry and got NaN. In the prototype's first version the layer was a bare 1×1 sublayer under the content view's layer: it had no view of its own, the content view is winit's OpenGL view, and the layer position was just a hand-set 1×1. AVKit cannot derive valid window coordinates from that. Threading was not the problem: the delegate, the controller and the start call are all on the main thread.

**Fix** (separate commit "Prototype fix: picture-in-picture layer geometry"):

1. The display layer is now the backing layer of a dedicated `NSView`: `setLayer` first, then `setWantsLayer(true)`, i.e. a layer-hosting view whose size AppKit manages. The view's frame equals the content view's frame, autoresizes in width and height, and is placed in the window's frame view behind the content view with `addSubview:positioned:NSWindowBelow relativeTo:contentView`. Its geometry is real and valid, and it is hidden by the opaque GL content, so the picture is not affected. `setWantsLayer` is no longer called on winit's view, and nothing is added to its layer any more.
2. Initialization order: first put the layer into the window's layer tree (the view is already in the window, bounds are valid), then create the content source and the controller.
3. Stricter start conditions: `Pip::start()` calls `startPictureInPicture` only when all of these hold, otherwise it returns `false` and the host retries on the next frame: `isPictureInPicturePossible`, the window is visible, all four components of the host view's frame are finite with width and height ≥ 1, and ≥ 3 frames have been enqueued. The log line `pip: possible, starting` is printed once, only when the call is actually made.
4. Diagnostic switch `VESTRA_PIP_OVERLAY=1`: uses instead a **visible** 160×90 view in the lower-left corner of the content view (no autoresizing), to tell "a hidden view is not accepted" apart from other problems.

Checks: with default features, fmt, clippy `-D warnings` and `cargo test --locked` (223 tests) all pass; clippy with `--features pip` and the release build with `--features "gpu-render pip"` pass; `vestra-pip` tests pass; `grep -rnw unsafe src/host/src` is still empty; all 17 `unsafe` blocks have SAFETY comments. The GUI was not operated; whether the crash is fixed needs a retest.

**Retest steps** (build as in §7.3.3: `cargo build --locked --release --features "gpu-render pip"`, `$BIN` / `$S` as above):

| # | Steps | Expected / record |
| --- | --- | --- |
| R1 | `VESTRA_PIP=1 VESTRA_SOFTWARE_RENDER=1 zsh $S/launch.sh $BIN`, open and play the 1080p source, wait 5 seconds | `launch.out` shows `pip: ready (supported: true)` then `pip: possible, starting`; **no crash**; the system picture-in-picture window appears and plays; the main window picture is unchanged (the host view is not visible). If it crashes, take the new `~/Library/Logs/DiagnosticReports/host-*.ips` and look at `asi` and the crashed thread's stack |
| R2 | Only if R1 does not show `pip: possible, starting` or no small window: repeat R1 with `VESTRA_PIP_OVERLAY=1` added | A 160×90 video block in the lower-left corner of the main window; record whether picture-in-picture can start now. If it can, AVKit does not accept the hidden view and the real version needs another placement |
| R3 | After R1 succeeds, continue with P2–P4 of §7.3.3, then resize the window once and enter and leave fullscreen once | Button presses pass back normally; no crash on resize or fullscreen (the host view resizes along) |

Prototype tools (scratchpad, not in the repository): `r4/gpu/ctxswap` (render context lifetime probe), `r4/gpu/gpubench` (windowless GPU benchmark, depends on `vestra-gpu-render` in the repository), `r4/gpu/cpu.sh` (headless CPU comparison), `r4/gpu/gui-cpu.sh` (GUI CPU sampling), `r4/gpu/count_unsafe.py` (counts `unsafe` blocks, lines and SAFETY comments), `r4/gpu/invert.glsl` (E17 probe shader).

## 8. Records

### 8.1 Baseline

(Filled in at P0: static checks, per-plugin test counts; host Rust line count of `src/host/src/*.rs` (30 966 when this task sheet was written); SDK and runner line counts)

### 8.2 Maintainer deliveries and signature adjustments

(Filled in at each P1 batch delivery: implementation list, signature adjustments and implementation trade-offs, checks, measured net host Rust growth, GUI measurement points for the coordinator. At least record: the actual values of the cap rule, the throttle parameters, when `video.view` ownership is taken, the measured frame rate of `frameBlend` in offline export, and whether `estimated-vf-fps` reflects frame doubling)

#### P1 batch 1 (2026-10-21, maintainer): C-SIZE, render pacing, `frameBlend`

Based on `rebuild/features-r3b` `8ea6ab0`.

**Implementation list**

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §4.1 | C-SIZE (R-H2) | New module `render_target.rs` (pure functions and small state machines, easy to unit test): `physical` (logical video area × `scale_factor`, rounded and made even, minimum 2×2, returns `None` for 0 to keep the previous target), `target` (source cap + zoom relaxation + absolute cap), `Throttle` / `Sizing` (throttling, fullscreen and scale factor changes apply immediately), `Shared` (UI writes the video area, batch 2's display layer writes zoom, with a revision number), `Redraw` (redraw once when the target changes while paused). The `backend.rs` render thread allocates a `SharedPixelBuffer<Rgb8Pixel>` at the target size for each frame and renders straight into it; `main.rs` computes the physical area from `video-width/height` in `window_tick` and hands it to the throttle | One paragraph added to reference §5.4; §17 |
| §4.2 | Render pacing | `render_target::FrameSlot`: keeps only the latest frame; when a new frame is put in and "wakeup in flight" is false, it sets it to true and calls the waker; taking a frame clears the flag first. On the UI side the waker is `slint::invoke_from_event_loop`, and the callback calls `Host::show_frame` through the thread-local `FRAME_SINK` (`Image::from_rgb8(buffer)`, no more `clone_from_slice`). The 16 ms timer no longer fetches frames, except once to catch up when "the host was busy at wakeup" (`try_borrow_mut` failed, counted as `missed`). Counters: produced / shown; `VESTRA_FRAME_LOG=1` prints one stderr line per second: `vestra frames: produced N shown M (x/y fps) target WxH` | — |
| §4.4 | `frameBlend` (R-V8) | New node `FrameBlend { fps }` in `effects.rs` (integer 24–120, `framerate@nK=fps=N`, not a runtime field); `Node::last()` + `sequence()`: rendering and in-place updates use the same order, "each chain in stable order, `frameBlend` moved to the end"; `ordered()` checks at most 1 across all resources (the second is `invalid`); offline export (`offline_descriptor.rs`) picks it up automatically through the same `render_lavfi` | `sdk/processing.ts` types, range, integer and "at most one"; TestHost counts across all of this plugin's video filters; reference §8 node table, placement rule, §14, §17; example §32 |

**Signature adjustments and implementation trade-offs** (public signatures match the rulings, no adjustments)

1. **Actual values of the cap rule**: `k = min(1, max(S.w/A.w, S.h/A.h) × max(1, zoom))`, target = `A × k`, then made even; the total pixel cap is 3840×2160 = 8 294 400 (above it, scale down by area ratio; if still over after rounding to even, shrink 2 pixels at a time); unknown source uses 1920×1080. The source size `S` is `video-out-params/dw`, `dh` (after filters and aspect correction), read by the render thread at most every 200 ms when it needs to render. Examples: Retina fullscreen video area 2560×1500 with a 1080p source → 1920×1126; default window 1560×1000 → 1560×1000.
2. **Startup / no interface**: before the UI reports a video area, the target area is 960×540 (same as before), so headless and `--frame-evidence-dir` output sizes are unchanged (PPM is written at the actual frame size).
3. **Throttle parameters**: 50 ms. While changing, apply at most every 50 ms; after changes stop, the last kept value is applied in the first 16 ms cycle that is 50 ms after the previous apply (that is, ≤ 66 ms after stopping). Fullscreen toggles and `scale_factor` changes (display change) apply immediately. When the video area is 0 (minimized), the target is not changed.
4. **Redraw while paused**: on each loop the render thread compares the revision number of `Shared`; if it changed and a real frame has already been rendered, it calls `render_software` once (mpv redraws the current frame) without waiting for `update()`. Size changes before the first frame do not redraw (to avoid a black frame from an empty source). Resizing does not rebuild the `RenderContext`.
5. **Generated streams** (NES): frames are turned into a `SharedPixelBuffer` on the control thread (the copy moves from the UI thread to the control thread), at the same size as before; the clear frame on stop is still a 960×540 black frame.
6. **Only the latest frame is kept**: when the UI is slow, old frames are overwritten (not queued); the gap between `shown` and `produced` is the overwritten frames.
7. **`frameBlend` placement**: in the same vf chain it goes after all plugins' nodes; in offline export it goes at the end of the effects chain, before square pixels and subtitle burn-in (consistent with playback, where vo overlays subtitles after frame doubling). Changing `fps` rebuilds this stage; in-place update indices of other nodes are computed in the new order (covered by unit tests).
8. **`estimated-vf-fps` reflects frame doubling**: with the mpv 0.41 CLI of the same version (`--vo=null --vf=lavfi=[framerate=fps=48]`, `t1080.mp4` at 24 fps) it reads `estimated-vf-fps=48.000000`, `container-fps=24`; without the filter it is 24. So `stats().video.fps` shows 48 once frame doubling is on, and the original frame rate has to be read before turning it on (stated in reference §8 and example §32).
9. **Measured offline export frame rate**: using a graph of the same form generated by `offline_descriptor::video_range` (`framerate@n0=fps=48,scale=…,setsar=1,settb=AVTB,trim=start=1:end=6,setpts=PTS-STARTPTS`), a 5-second `ffmpeg` export gives `ffprobe` `r_frame_rate=48/1`, `nb_frames=240`. `media_worker.rs` does not force `-r`; the frame rate follows the graph.
10. Picture-in-picture fields are **not published** (§2).

**Checks** (actually run in the worktree)

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 88/88 (1 new in `tests/sdk/round4a.test.ts`: `frameBlend` validation and "at most one"); `plugin -- test --all` all pass: audio 35, bilibili 15, capture 25, history 22, hotkeys 17, info 26, marks 24, nes 11, playback 59, playlist 67, plugins 10, preferences 10, subtitle-online 13, subtitles 52, video 22, window 17, workshop 17, youtube 14 (workshop `vendor/sdk` resynced, same commit).
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 215/215 (new: target computation 1, throttling 1, fullscreen and display change applied immediately 1, redraw while paused 1, frame slot coalesced wakeups 1, `frameBlend` placement / uniqueness / in-place update indices 1, offline export contains `framerate=fps=48` and no display-layer properties 1).
- `grep -rnw unsafe src/host/src`: empty.
- Not run: GUI (not operated, as required).
- Host Rust: `src/host/src/*.rs` 30 966 → 31 621 (**+655**, including tests; `render_target.rs` 462). Below this batch's estimate of 750. SDK about +30 (`processing.ts`, TestHost).

**GUI measurement points for the coordinator** (start with `VESTRA_FRAME_LOG=1`)

1. Sharpness: screenshot a 1080p test pattern in the default window and fullscreen and compare thin lines and small text with the baseline; in the log, `target` should be about `1560x1000` in the default window (depending on the window) and `1920x11xx` in fullscreen (capped by the 1080p source); for a 4K source in fullscreen about `2560x15xx`.
2. Dragging: drag the window corner continuously for 10 seconds while playing; the picture does not stall or distort (at most a thin black bar for one or two frames); in the log `target` changes about every 50 ms and settles on the final value after you stop. **Dragging after pausing**: the picture redraws at the new aspect ratio (this is the `Redraw` path; unit tests only cover the decision).
3. Render pacing: with a 24 fps source `produced ≈ shown ≈ 24/s`; after adding `frameBlend 48` with a probe (or with the video plugin once author A is done), `produced ≈ 48/s`, `shown/produced ≥ 95%`, `stats().video.fps` is 48, `framesDropped` does not grow; check 60 once as well. **UI thread texture upload** (§1.1 risk): with a 4K source in fullscreen + frame doubling to 60, check whether `shown` keeps up; if not, record the `shown` numbers for the maintainer.
4. Display change: drag the window to an external 1x display; `target` halves immediately (without waiting for the throttle).
5. CPU: compare with §1.1 using the table in §6.2.
6. Regression: NES shows a picture and clears after stop; after closing the source the backdrop shows, no leftover frame; the picture is fine after switching the sidebar between floating and attached; while the open-file dialog (modal) is open, the playing picture continues to update after the dialog closes (the `missed` catch-up path).
7. Export: with frame doubling on, export 5 seconds "with effects"; `ffprobe` shows 48 fps.

#### P1 batch 2 (2026-10-21, maintainer): display layer `plugin.video.view()`

Based on batch 1 `bfda2ae`, with `rebuild/features-r3b` merged up to `04b2de9` (the §7 user ruling record and the visualizer plugin). **The split in §4.0 was not triggered**.

**Implementation list**

| Task sheet | Content | Host | SDK / TestHost / docs |
| --- | --- | --- | --- |
| §4.3 | `plugin.video.view()` (R-V9, existing capability `processing.video`) | New module `video_view.rs`: `Settings` (defaults, merge validation: unknown fields and out-of-range values are `invalid` and change no values), mpv property mapping (all written as option strings), `View` (owner, `set` / `release`; on a write failure it tries its best to restore the previous five values and fails with `backend`). The control thread handles `video.view.set { values }` → `{ state: "applied" \| "waiting" }` and `video.view.release`, deciding ownership by calling plugin; on `owner.disabled` it releases and writes back the defaults; after each request it writes `zoom` into `render_target::Shared` (the C-SIZE cap is relaxed by zoom). Capability table and forwarding in `main.rs` | `sdk/plugin/view.ts` (`viewProblem` uses the same rules as the host, `Latest` keeps only the latest value, resends on source / stage changes so that `waiting` becomes `applied`); `Scope.video.view`; `ViewSettings`, `VideoView` types; wire `video.viewSet` / `viewRelease`; `host.videoView`, `host.setVideoViewOwner`; reference §8.1 (new), §14, §17; example §33 |

**Signature adjustments and implementation trade-offs** (public signatures match ruling R-V9)

1. **Ownership is taken when `view()` is called**: the SDK sends one `video.view.set` (with the current values = the defaults) right when it builds the handle, and the host records the owner on the first `set`. If another plugin already holds it, the handle is still returned, with `status` `failed` (`conflict`) and passed to `onError`; **activation does not fail** (unlike `provideSequence`: the video plugin must still be able to activate while a probe holds the layer, per the GUI scenario "after the probe is disabled, video can take it"). After a conflict there is no automatic retry (to avoid spinning); the next `set` tries again. A second `view()` from the same plugin throws; after `dispose` it can be called again.
2. The protocol has one method the task sheet did not mention, `video.view.release` (used by SDK `dispose`): the host only honors it for the owner (writes back defaults, releases); calls from other plugins have no effect. Disabling is handled by the host's `owner.disabled` and does not depend on it.
3. **Meaning of `waiting`**: these five mpv items are options kept across `loadfile` and can be written with no media, so the host always writes immediately and only answers `waiting` when there is no ready media; the SDK resends the current values on source or stage changes, and they become `applied` once ready.
4. The SDK always sends all five items (the merged desired values); the host still accepts partial fields (merged).
5. Values are kept on source change (the host does not reset); the E13 reset is done by the plugin, as stated in reference §8.1 and example §33.
6. Not in snapshots or exports: snapshots and exports use the offline ffmpeg graph (batch 1's unit test asserts that the offline graph has none of these properties).
7. Measured mpv values (mpv 0.41 CLI, same option parser): `video-aspect-override=-1` / `1.3333`, `keepaspect=no`, `panscan=1`, `video-zoom=-2` and `video-pan-x=-0.5` are all accepted; with `1.3333`, `VO: 1920x1080 => 1920x1440`. `video-aspect-override` changes `video-out-params/dw`, and the C-SIZE source cap follows.

**Checks** (actually run in the worktree)

- `src/`: `check`, `format:check`, `boundaries`, `plugins:build` 0; `test:sdk` 90/90 (2 new in `round4a.test.ts`: ownership, merging, `invalid` not sent, `dispose` resets; conflict and taking over after release); `plugin -- test --all` all pass: audio 35, bilibili 15, capture 25, history 22, hotkeys 17, info 26, marks 24, nes 11, playback 59, playlist 67, plugins 10, preferences 10, subtitle-online 13, subtitles 52, video 22, visualizer 15 (newly merged), window 17, workshop 17, youtube 14; workshop `vendor/sdk` resynced.
- `src/host/`: `cargo fmt --check`, `clippy --locked --all-targets -D warnings` (including `--no-default-features`) 0; `cargo test --locked` 219/219 (4 new: property mapping, ranges and unknown fields, owner and reset, restore on write failure; the `render_target` redraw-while-paused test now includes zoom).
- `grep -rnw unsafe src/host/src`: empty.
- Not run: GUI.
- Host Rust: 31 621 → 31 974 (**+353**, `video_view.rs` 312); round total 30 966 → 31 974 = **+1 008** (budget about 1 200). SDK and runner about +230 (`view.ts` 149, TestHost, types, wire).

**GUI measurement points for the coordinator** (needs the video plugin after author A's integration, or a temporary probe)

1. 16:9 source with `set({ aspect: 4/3 })`: bars left and right; `aspect: null` restores. Drag the window to near 1:1, then `fit: "stretch"`: no bars, picture distorted; `fit: "fill"`: no bars, cropped top and bottom or left and right. The `target` ratio in `VESTRA_FRAME_LOG` always equals the video area's ratio.
2. After `zoom: 2` the picture is sharp (log `target` larger than at zoom 1, up to the video area size); `zoom: 4` plus `panX: 1`, `panY: -1` to check the pan direction; `zoom: 0.5` gives bars on all sides.
3. Change `aspect` / `zoom` while paused: the picture redraws immediately (batch 1's `Redraw` path).
4. Source change: values are kept (the plugin resets them itself); the size and aspect ratio of snapshots with effects do not depend on the display layer.
5. Ownership: the probe calls `view()` first, and video's "Aspect ▶" fails and shows `conflict`; after the probe is disabled (the host writes back defaults, the picture goes back to source display), choosing again in video takes effect. After disabling video, the default display comes back.

#### Ruling request: do not show mpv cover art while a backdrop is mounted (author D, visualizer, §4.1; separate commit "Round 3b fix")

- Request: while a backdrop is mounted, the host sets mpv `audio-display=no` and `cover-art-auto=no`, and restores them on unmount. Reason: mpv treats embedded cover art / a cover.jpg in the same folder as a video track, so `stats().video` is non-empty with `effective = false`, and visualizer's cover + labels interface never appears. The author verified this with the mpv 0.41 CLI.
- Maintainer analysis (§4.3 steps 2 and 4): there is no workaround inside the plugin (mpv properties belong to the host alone; plugins cannot write mpv properties, nor make the backdrop cover the cover art mpv draws). This is a side effect of r3b ruling 3.2 item 8, "cover-art audio `effective = false`": it was recorded as known behavior at the time, on the premise that "the user sees the cover art mpv draws", but it makes the J11 design of backdrop plugins (drawing the cover and labels themselves) impossible. It is a correction of backdrop behavior; no interface is added.
- Conclusion: **accepted** (behavior correction, signatures unchanged). Rule: while any backdrop is mounted, both items are `no`; after the last backdrop unmounts, mpv defaults are restored (`embedded-first`, `exact`); mpv selects tracks at load time, so it takes effect from the next file opened after mounting. Users without a backdrop (or after disabling visualizer) still see the cover art mpv draws.
- Implementation: in `window_tick`, `main.rs` compares "is a backdrop mounted", and on change sends the internal command `host.coverArt { off }` to the control thread (not in the plugin protocol table; plugins calling it get `unsupported`); `CoverArt` in `backend.rs` writes the two items only when the state changes. 1 unit test (writes only on change, restores defaults). Reference §6.3 rewritten, one entry added to §17. Host Rust 31 974 → 32 060 (+86).
- GUI points: with visualizer enabled, open an MP3 with embedded cover art → the backdrop shows the plugin's cover and three label lines (`stats().video` is `null`); same for a FLAC in a folder with cover.jpg; after disabling visualizer, open the same file again → mpv shows the cover art.

#### Fixes (2026-10-21, maintainer; found in the coordinator's GUI measurement)

- **Command limit** (`8512d2d`): video registers 71 commands when idle, over the per-plugin limit of 64, so activation failed in the real player; all plugins plus the host total 292. The limits are now 160 per plugin and 1024 in total (a guard against runaway registration, not a product limit; leaves room for track lists and parallel trial loads); `TestHost` enforces the same limits, and `COMMAND_LIMITS` is exported from `sdk/commands.ts` with a test checking it matches the host. Reference §4, §13, §14, §17.
- **Options window layout** (`e3e4901`): the bottom button row could stretch and `Button` had no max-height, so the content area and the button row split the height evenly. The button row now has `vertical-stretch: 0` and buttons are a fixed 21px; a layout test measures the geometry on the windowless platform.
- **German layout ⌥+**: on the German layout `+` is its own key without Shift (at the US `]` position), so the physical-key path for ⌥+ resolved to `Alt++` instead of `Alt+Plus`, and video's "Zoom In" `Alt+Plus` did not fire. `from_physical` in `bindings.rs` and the Option symbol candidates now both use `symbol_name` (`+` → `Plus`, consistent with the unmodified path); unit tests: German ⌥+, ⌃⌥+, unmodified `+`, US ⌥⇧=.
- **Default key conflict (no core change, passed to the author)**: ⌃F6 is a macOS system shortcut (keyboard navigation "Move focus to the floating window", System Settings › Keyboard › Keyboard Shortcuts › Keyboard); the system takes it first and the app never receives it. video's "Cycle Aspect Ratio" default `Ctrl+F6` is therefore unavailable by default on Mac (unless the user turns off that system shortcut or rebinds it on the shortcuts page). **The author is advised to use a default key that does not conflict with the system**, and to avoid ⌃F1–⌃F8 (the system keyboard navigation shortcuts of the same group).

### 8.3 User rulings

(The coordinator fills in the answers to §7.1 and §7.2 with dates; requirements H07, E16–E18 and the 4b schedule are updated according to the answers)

### 8.4 Author deliveries

(Filled in by the authors when done)

### 8.5 Acceptance results

(Filled in at P3)

#### 7.4 Prototype results on real hardware (coordinator, 2026-10-21)

**GPU rendering (release build, `gpu-render`)**: orientation and colors are correct, sharp in both window and fullscreen. 4K 30 fps test source, host process CPU sampled with `gui-cpu.sh`:

| 4K playback | Software rendering | GPU rendering |
| --- | --- | --- |
| Window | 81% | 19% |
| Fullscreen | 152% | 14% |

**Picture-in-picture (`pip`)**: the log shows `pip: ready (supported: true)` then `pip: possible, starting`, after which the host crashed with EXC_BREAKPOINT. The crash stack is `-[AVPictureInPictureSampleBufferDisplayLayerHostView layout]` → `-[NSView setFrame:]` → `_NSViewValidateGeometry`, i.e. the picture-in-picture host view hit invalid geometry during layout, very likely related to the 1×1 sublayer. The crash report is at `~/Library/Logs/DiagnosticReports/host-2026-10-21-100828.ips`.

**Conclusion**: the gain of GPU rendering is confirmed and it is ready to become a formal project, pending the user's confirmation. Picture-in-picture: fix the crash first and retest; this does not block other work.

**Picture-in-picture retest (20e1272)**: no more crash; the system picture-in-picture window appears and the picture updates; host CPU about 4% (640×360 source, software rendering). Three problems remain:
- The small window shows only the top-left corner of the picture, extremely magnified, as if the frame size or scaling setting were wrong;
- The small window thinks it is paused; playback state is not synchronized;
- The main window title bar is broken: the title text overlaps the red, yellow and green buttons.

Passed to the maintainer.

### Coordinator acceptance (2026-10-21)

**All checks pass**: check, format, boundaries, test:sdk, 480 tests across 19 plugins, cargo fmt/clippy/test with 223 tests; `grep -rnw unsafe src/host/src` is empty.

**GUI measurement passed**:
- Aspect: choosing 4:3 in the menu shows the picture correctly at 4:3; Shift+F6 cycles through aspect ratios.
- Framing: ⌥+ zooms in, OSD "Framing: larger (105%)".
- C-SIZE: the picture is sharp in fullscreen.
- Cover art: mpv does not show cover art while a backdrop is mounted.

**Fixed on the spot**:
- ⌥+ resolves to Plus on the German layout (891bfa2).
- video "Cycle Aspect Ratio" changed from Ctrl+F6 to Shift+F6: Ctrl+F6 is a macOS system shortcut.

**Prototypes**: GPU rendering and picture-in-picture are both under `src/host/unsafe/`, off in the default build; results in §7.4.
