# GPU rendering, picture-in-picture and Windows gaps

Branch: `rebuild/render-pip` (from `main` 6a85fb6). Basis: [community workflow](../community-workflow.md) §1.5 (cross-platform rule), §4.3, §5. Prototypes and measurements are in [round 4a task sheet](2026-10-features-r4a.md) §7, prototype branch `rebuild/prototypes`.

## 1. User rulings (2026-10-21)

- **GPU rendering: lock to OpenGL**, because it has to be cross-platform. libmpv renders with OpenGL, Slint is locked to the femtovg renderer, and software rendering stays as a fallback, chosen by an environment variable or automatically when startup fails. Option B (CALayer) only works on macOS and is not adopted.
- **Picture-in-picture: do it**. macOS uses the system picture-in-picture (AVKit), fixed according to the root cause found by offline reproduction: the display layer must be the same size as the video area and sit at its normal place in the window. The offline probe is [`assets/pip-probe.swift`](assets/pip-probe.swift) (`swiftc -O pip-probe.swift -o probe && ./probe normal|small`): the standard setup works, and a small 160×90 layer reproduces the white border. Windows and Linux return `unsupported` for now; later the host will build its own borderless always-on-top small window.
- **Windows gaps**: snapshot export, produced streams (`openStream`), confirmation dialog button text. These are listed in the [portability task sheet](2026-10-portability.md) §4.2; on this machine only compilation can be verified.

## 2. Division of work

| Line | Owner | Content |
| --- | --- | --- |
| A | Maintainer A | First merge the render sub-crate from `rebuild/prototypes` (it conflicts with the portability round; the maintainer resolves it; the layer sub-crate is not merged). Then implement GPU rendering properly:<br>· rendering-related OS code goes into platform modules, `unsafe` stays only in `src/host/unsafe/render/`;<br>· software rendering fallback;<br>· hardware decoding (hwdec) on, and update `stats` and the "Hardware decoding" row in the info panel;<br>· `video.render()` (R-V10: HDR tone mapping, shaders, motion interpolation) and `PlaybackStats.video.transfer`;<br>· handle prototype leftovers: one-frame flash on source change, frame log and stats, NES produced streams, floating sidebar.<br>After that, do the Windows gaps as a second commit |
| B | Maintainer B | Picture-in-picture: take the pip sub-crate from `rebuild/prototypes` and fix it according to the root cause. The core API is `window.setPictureInPicture(on)` and `WindowState.pictureInPicture`; buttons in the picture-in-picture window must be passed back to playback; non-macOS returns `unsupported` |
| C | Author | After A and B deliver: the video plugin wires up E16, E17, E18 motion interpolation; the window plugin lets H07 use picture-in-picture, with the mini size as the fallback on non-macOS |
| D | Coordinator | Real-machine verification, then merge |

## 3. Records

(Added as each stage is done)

### 3.1 Line B delivery: picture-in-picture (Maintainer B, 2026-10-21)

**Conclusion**: `window.setPictureInPicture(on)` and `WindowState.pictureInPicture` are implemented (macOS 14+), and the placement of the display layer was redone according to the root cause. All static checks pass; the Vestra GUI was not operated, so whether the small window is filled needs the coordinator's test (§3.1.5).

#### 3.1.1 Offline probe results (verify first, then change code)

The probe was extended to 9 variants (described at the top of [`assets/pip-probe.swift`](assets/pip-probe.swift)); each was run once on this machine, looking only at the log the probe prints itself:

| variant | Approach | renderSize | Notes |
| --- | --- | --- | --- |
| normal | Layer fills the content view | 640×360 | Baseline |
| small | 160×90 sublayer | 160×90 | The white-border case |
| covered | Same as normal, with an opaque view on top | 640×360 | **Covered but correct size: works** |
| area | Layer = video area (minus 120 for the sidebar, 48 for the bottom bar) | 520×292 | renderSize is the rectangle of the video picture inside the layer (16:9, proportional) |
| gl | Simulates Vestra: flipped layer-backed view, NSOpenGLContext draws opaque red, the layer is inserted as sublayer 0 of the view's layer | 520×292 | Size is correct, but the log shows the GL picture is the contents of `_NSOpenGLViewBackingLayer`, and **sublayers are always drawn above the contents**: putting it into the winit view would cover the on-screen messages, info blocks and floating bottom bar drawn by Slint |
| glresize | Same as gl, window resized to 960×540 before starting | 840×472 | Follows the resize |
| hidden | `isHidden = true` | none | AVKit reports no renderSize; unusable |
| clear | `opacity = 0` | 640×360 | Not adopted (whether it is visible in the small window cannot be judged from the log) |
| child | Layer in a borderless **child window**, ordered **below** the main window and fully hidden by it | 640×360 | Starts normally (fine even when the child window's `occlusionState` does not include visible) |
| childgl | Final approach: GL main window + child window framed on the screen rectangle of the video area; after picture-in-picture opens, the main window is resized and the child window follows | 520×292 | Starts normally |
| grow / bigframe | Resize the window after picture-in-picture opens / 1920×1080 frames + display immediately (how Vestra feeds frames) | 640×360 | AVKit does not move or scale the source layer; resizing while open does not report renderSize again |

Also: when closing actively with `stopPictureInPicture`, AVKit also calls `restoreUserInterface…` first (when the host uses this to tell "return" from "close", it must exclude closes it started itself; see below).

#### 3.1.2 Approach

- **Display layer**: `AVSampleBufferDisplayLayer` is a sublayer of the content view of a borderless child window, filling the child window. The child window is attached **below** the main window with `addChildWindow:ordered:NSWindowBelow`, and every tick its frame is set to the screen rectangle of the video area (Slint gains the outputs `video-x` and `video-y` alongside the existing `video-width/height`, converted to Cocoa screen coordinates). So the layer has the same size and position as the video area, follows window / fullscreen / sidebar changes, and is hidden by the opaque video area of the main window without affecting any overlay. The child window takes no mouse input, has no shadow, is not in the Window menu, and is `FullScreenAuxiliary | IgnoresCycle`. When the main window is dragged, the system carries the child window along.
- **The controller** is created only after the child window is first on screen (AVKit lays out its own view from the source layer geometry). Rebuilding the main window (mounting / unmounting UI) only changes the parent window; the layer and controller stay, and **picture-in-picture is not interrupted** (limitation 7 of the prototype is solved).
- **Frames (CPU path)**: the software-rendered RGB24 that `show_frame` receives is copied into an IOSurface BGRA buffer from a `CVPixelBufferPool` (a pool per size, instead of a new buffer every frame), wrapped as a display-immediately `CMSampleBuffer` and enqueued. Frames are fed only while picture-in-picture is starting / open; when paused it starts from the last frame (`Image::to_rgb8`).
- **Timebase**: the layer renderer's own timebase is set from the playback position and speed (0 when paused), and reset only when it drifts by more than 0.5 s; seekable sources report `[0, duration]`, so the small window has a progress bar and skip buttons; live / non-seekable sources report an infinite range (play / pause only).
- **Button pass-back**: `setPlaying` → run the core action `play-pause` only if it differs from the current state; `skipByInterval` → `seek-forward` / `seek-backward`, with the step from the interval the system gives (`run_core_action_with` gains a step parameter; other calls are unchanged).
- **Enter / exit sync**: the controller delegate reports `didStart`, `didStop`, `failedToStart`, `restoreUserInterface`. `setPictureInPicture(true)` resolves after `didStart`, `false` resolves after `didStop`; not done within 5 seconds → `failed`. Small-window close button (no restore, and not started by the host) → the host pauses; the "return" button does not pause. A new source has no video, the source closes, or the plugin that opened it is disabled → the host closes the small window. Calling `true` in fullscreen → leave fullscreen first, then start once that completes. During picture-in-picture the main window's video area shows grey text "Playing in picture-in-picture" (new `host.slint` property `picture-in-picture`).
- **Platform**: platform-independent rules are in `src/host/src/pip.rs` (`PictureInPicture<S: Surface>`, 6 unit tests with a fake Surface covering: check order, waiting for frame / placement / fullscreen, buttons, close button pauses and return does not, failure / timeout / video gone, plugin disabled). OS implementations: `PipWindow` in `platform/macos.rs` (wrapping `vestra-pip`); Windows and Linux share `platform/pip_unsupported.rs` (`PipWindow` is an empty enum, `new()` returns `unsupported`, `pip_supported()` is `false`). When the host later builds its own borderless always-on-top small window on Windows / Linux, only `Surface` needs implementing there.
- **unsafe**: only in `src/host/unsafe/pip/` (`vestra-pip` 0.1.0). `src/lib.rs` is 703 lines (`use_small_heuristics = "Max"` formatting, same as the render sub-crate; includes 3 tests), with 25 `unsafe` blocks (2 in tests), 3 `unsafe impl` (NSObjectProtocol inside `define_class!`, and two AVKit delegate protocols), and 10 `#[unsafe(...)]` attributes; every block and every `unsafe impl` has `// SAFETY:` (a script checked none are missing). There is also `examples/check.rs` (105 lines, 2 `unsafe` blocks): the crate's own standalone check program, see below. The host main crate keeps `forbid(unsafe_code)`, and `grep -rnw unsafe src/host/src` is empty.
- **feature**: the prototype's `pip` feature is removed, and `vestra-pip` becomes an ordinary macOS dependency (the default build includes picture-in-picture). The `VESTRA_PIP` / `VESTRA_PIP_OVERLAY` diagnostic switches are removed along with the prototype.

**Crate self-check (not the Vestra GUI)**: `cargo run -p vestra-pip --example check` opens a 640×360 window covered by a red view, places `Pip` on a 520×312 "video area" at the top left, feeds 1920×1080 RGB, starts, resizes the window while open, and stops actively. Result on this machine: `Started renderSize (520, 292)`, `Stopped { restored: true }`, matching the probe's childgl.

#### 3.1.3 Points of contact with line A (GPU rendering)

- With `gpu-render` on (or the GPU path once line A makes it official), `show_frame` is no longer called, picture-in-picture gets no frames, and `setPictureInPicture(true)` will be `failed` after 5 seconds ("picture-in-picture did not start").
- A zero-copy interface is reserved: `vestra_pip::Pip::enqueue_surface(&IOSurfaceRef)` (wrapped with `CVPixelBufferCreateWithIOSurface`, no copy; covered by tests), and on the host side `platform::PipWindow::push_surface(&vestra_pip::IOSurfaceRef)` (temporarily `allow(dead_code)`).
- To do when merging: (1) where the GPU render thread returns to the main thread after "a frame is out" (in the prototype, `Handle::on_frame` → `invoke_from_event_loop`), hand that IOSurface to picture-in-picture when `self.pip.feeding()`: add a `push_surface` method to `pip::Surface` (macOS calls `PipWindow::push_surface`, `pip_unsupported.rs` does `match *self {}`), and set `fed` in `PictureInPicture` as `push` does; (2) the IOSurface must be BGRA and must not be rewritten by the render thread while AVKit displays it: rotation must skip the one that is `IOSurfaceIsInUse` (the prototype layer-video already does this), otherwise the small window may tear; (3) the "last frame" used to start when paused becomes the latest IOSurface (`Handle::latest()`); (4) remove `allow(dead_code)` from `push_surface`.
- Render size: the small window's resolution is currently the render size (C-SIZE follows the main window's video area). When the user drags the small window larger than the main window's video area, it gets blurry; on the GPU path the render size could take the larger of "video area" and "small window". AVKit no longer calls back renderSize when the small window is resized while open (probe grow), so another source is needed (not done for now).

#### 3.1.4 Integration notes for the author (window plugin H07)

- Capability: the existing `window.control`.
- Availability: `plugin.window.state.pictureInPicture.supported` is `true` (macOS 14+) and there is video (`player.state.videoSize` is not empty). When `supported` is `false` (Windows, Linux, headless), keep the mini size (`setVideoArea` + `setAlwaysOnTop`) as the fallback; menu text and shortcuts stay the same.
- Toggle: `await plugin.window.setPictureInPicture(!plugin.window.state.pictureInPicture.active)`. When it resolves, `state` is already updated; when called in fullscreen, the core leaves fullscreen first, so the plugin does not need to. Suggested messages: on success "Picture-in-picture: on" / "Picture-in-picture: off"; on failure "Cannot enter picture-in-picture: <message>". Pressing again before the previous call returns gives `busy`; the plugin can ignore it (or disable the command itself while awaiting).
- Checked state: use `state.pictureInPicture.active`, and follow it with `plugin.window.onChange`. When the user clicks "return" or close in the small window, the source has no video, or the source closes, `active` turns `false` by itself, and the plugin can show "Picture-in-picture: off" (when there is no video it can show "Picture-in-picture: off (no video)").
- Not needed from the plugin: small-window button pass-back, pausing on the close button, the grey text in the main window, leaving picture-in-picture when the plugin is disabled; the core handles all of these.
- Tests: `TestHost`'s `host.mainWindow.pictureInPicture` (`{ supported, active }`); `setPictureInPicture` switches immediately and leaves fullscreen; `host.setMainWindow({ pictureInPicture: { supported: true, active: false } })` simulates the user closing the small window; `supported: false` simulates Windows. Existing tests that `deepEqual` the whole `window.state` get one extra field (the SDK's `round3a` tests are updated; plugin tests are unaffected).

#### 3.1.5 GUI test points for the coordinator

Build: `cd src/host && cargo build --locked --release` (picture-in-picture is in the default build; no feature or environment variable needed). Use one 1080p source and one 4:3 / portrait source. Before the window plugin is wired up, any test plugin with `window.control` can call `plugin.window.setPictureInPicture(true)`.

| # | Scenario | Expected / record |
| --- | --- | --- |
| P1 | Play 1080p, call `setPictureInPicture(true)` | The system small window appears within 1 second; **the picture fills the small window proportionally, with no white area** (this is the root cause this round verifies); the call resolves and `state.pictureInPicture.active` is `true`; the main window's video area turns into the grey text "Playing in picture-in-picture", and no extra window or picture is visible in the main window; the title bar is normal |
| P2 | Drag the small window's corners to enlarge and shrink it; drag it to other corners | The picture always fills with correct proportions (it may get blurry when enlarged; record how much) |
| P3 | In the small window, click pause / play, skip forward / back 15 seconds, drag the progress bar (if any) | The host pauses / resumes / seeks along; when pausing with Space in the host, the small window's button follows |
| P4 | Small window "return" button; open again and click the small window's close button | Return: the small window closes, playback continues, the main window shows the picture again; close: the small window closes and **pauses**. In both cases `active` becomes `false` |
| P5 | With picture-in-picture open: resize the main window, toggle the sidebar, minimize the main window, switch to another app / another Space | The small window keeps playing (also when the main window is minimized); after restoring the main window the picture is normal; when closing the small window, the animation back to the main window lands on the video area |
| P6 | Call `setPictureInPicture(true)` in fullscreen; enter and leave fullscreen with picture-in-picture open | Fullscreen is left first, then the small window appears; entering and leaving fullscreen does not crash, and the picture is normal after closing the small window |
| P7 | With picture-in-picture open, mount / unmount a UI plugin (rebuilds the main window) | The small window **does not close** and keeps playing; the new main window shows the grey text |
| P8 | With picture-in-picture open, change source (video → video; video → audio only); close the source | Video → video: the small window stays and shows the new picture, with the proportions following; audio only or closed: the small window closes automatically, `active` becomes `false` |
| P9 | Disable the plugin that called it | The small window closes, without pausing |
| P10 | Open picture-in-picture after always-on-top (`setAlwaysOnTop(true)`) | Nothing abnormal; no black child window shows under the main window |
| P11 | CPU: sample 20 seconds each in the P1 state and with picture-in-picture closed | Record the extra cost of the per-frame copy (now using a buffer pool) |

If P1 still has a white border or shows only one corner: record the small window size and the area the picture takes, and run `cargo run -p vestra-pip --example check` in a terminal for comparison (a standalone small program with the same child-window approach).

#### 3.1.6 Checks

All run in this worktree, all pass: under `src/`, `npm run check`, `format:check`, `boundaries`, `test:sdk` (100, including 3 in the new `tests/sdk/pip.test.ts`), `plugins:build`, `npm run plugin -- test <name>` for each of the 19 plugins (all 0 failures; workshop's `vendor/sdk` was resynchronized with the command at the top of `vendor-sdk.test.mjs`), `npm run portability` (clean: the host compiles for the Windows target, and the warning that `PipEvent` is never constructed on Windows is gone); under `src/host/`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` (240, including 6 in `pip::`), plus clippy with `--features gpu-render`, `cargo clippy -p vestra-pip --all-targets -D warnings`, `cargo test -p vestra-pip` (3); `grep -rnw unsafe src/host/src` is empty. Note on disk space: free space on this machine was at one point only about 0.5 GB, so `npm run portability` used the existing cache in the main repository's `src/host/target/portability` (a temporary symlink, removed after the check).

### 3.2 Line A delivery 1: GPU rendering (Maintainer A, 2026-10-21, commit "Render: …")

**Prototype merged in**: only the changes to the render sub-crate were taken from `rebuild/prototypes` (`Handle::latest()`, `on_frame()`, skipping IOSurfaces still used by the window server when rotating). The layer and pip sub-crates were not merged. The prototype's whole `layer-video` integration in `main.rs` was discarded; the `gpu-render` integration was rewritten following the platform modules of the portability round (see below).

**Structure**

| Location | Content |
| --- | --- |
| `src/host/unsafe/render/` (`vestra-gpu-render`) | The only place with `unsafe`. New: a publish gate (`Renderer::start(mpv, size, publish)`; while the gate is closed it renders as usual but does not publish), rate limiting by refresh rate with `report_swap` (`set_display_fps`, for motion interpolation), `reshow()` (after the host replaces the video-area image, the window no longer shows the earlier frame, and the next frame sets the image again), `shown()`/`size()` (frame log), `attach(…, failed)` (called back when the window renderer is not OpenGL). `on_frame` is separate from the redraw list and cannot be pushed out by "the last 4 windows". 621 lines (72 of tests), 18 `unsafe` blocks (1 more in tests), each with a SAFETY comment; 2 tests |
| `platform/macos.rs` | `GPU_RENDER = true`, `gpu_start`, `GpuRenderer`/`GpuHandle` (the crate's types) |
| `no_gpu` in `platform/mod.rs` | Windows, Linux: `GPU_RENDER = false`, `gpu_start` returns an error; the types have no values, and the methods never run |
| `renderer.rs` (new) | `Render`: render mode, fallback requests, produced-stream flag, refresh rate; `wanted()`, `hwdec()`; 2 tests |
| `video_render.rs` (new) | Validation for `video.render()`, mpv option mapping, single owner, failure recovery, detection of shader compile failures; 4 tests |
| `backend.rs` | The render thread tries GPU first (`GpuRun`), and switches to software rendering on failure or when the window asks for fallback; `video.render.*`; stats `backend`, `framesShown`, `hwdec`, `transfer` |
| `main.rs` | Slint selects `renderer_name("femtovg")` (if that fails, fall back to Slint's default renderer + software rendering); `attach_gpu` no longer has a cfg; display refresh rate; the frame log merges both counts |
| `Cargo.toml` | Remove the `gpu-render` feature; `vestra-gpu-render` is an ordinary macOS dependency; the `pip` feature is left to Maintainer B |

The host main crate is still `forbid(unsafe_code)`, and `grep -rnw unsafe src/host/src` is empty.

**Behavior**

- GPU rendering by default (macOS). Software rendering fallback: `--headless`; `VESTRA_SOFTWARE_RENDER=1` (`0` or an empty value does not count); Windows, Linux; Slint cannot select femtovg; mpv's offscreen OpenGL context cannot be created; the window's render notifier is not `NativeOpenGL` or cannot create GL objects (then the render thread drops the GPU renderer and switches to software rendering, and `hwdec` goes back to `no`). Every fallback writes one line `render: …` to standard error.
- Hardware decoding: with GPU rendering, `hwdec=auto-safe` (overridden by `VESTRA_HWDEC`); for formats it cannot decode, mpv falls back to software decoding by itself; `stats().video.hwdec` is the raw value of `hwdec-current` (`"no"` = software decoding). Software rendering keeps `hwdec=no`.
- `video.render()` (R-V10, signature as ruled in negotiation step 3: `plugin.video.render(): RenderOptions`, `set(Partial<{ toneMapping; targetPeak; peakDetect; shaders; interpolation }>)`, single owner, four `FilterStatus` states). Under software rendering `set` returns `unsupported`. The mapping is in reference §8.2. Measured (crate test environment, mpv 0.41 + macOS OpenGL 4.1): all seven `tone-mapping` values, `target-peak`, `hdr-compute-peak`, `gamut-mapping-mode` and `display-fps-override` are accepted; the mpv log says `Disabling HDR peak computation (… compute shaders=0, SSBO=0)`, so `peakDetect` has no effect on macOS (stated in the reference); a bad shader reports `ERROR: 0:39: …` at `libmpv_render` level `error` on the first render, and the host uses that to push the owner's status to `failed` (`video.render.status`). `off` has no mpv equivalent and maps to `tone-mapping=clip` + `gamut-mapping-mode=clip` + no peak detection (not exactly the "washed out" look in the requirements; written into the reference; if the author wants "no processing at all", that needs separate discussion, for example setting `target-trc` to PQ, but that would affect SDR sources).
- `PlaybackStats.video.transfer` = the raw value of `video-params/gamma`.
- Prototype leftovers:
  - One-frame flash on source change: the GPU renderer also goes through the publish gate (nothing is published while orientation is being prepared); when the source enters `empty`, the video area is cleared and `reshow()` is called, so the next source starts from its own first frame without first showing the last frame of the previous source.
  - Frame log and stats: `framesShown` = software count + GPU publish count; with `VESTRA_FRAME_LOG=1`, "produced / shown" under GPU is publish count / window copy count, and the size is the GPU frame size; `backend` is accurate.
  - NES produced stream overwritten on resize: each round the backend first writes the flag "a produced stream is using the video area", and the window's GPU callback does not set the image while the flag is true; every produced-stream frame and the clear frame after stopping call `reshow()`, and GPU frames after that display as usual.
  - Floating sidebar shows no video: the floating sidebar window (`FloatingPanel`) never had a video area and shows no video under software rendering either; this is by design, not a GPU gap. Detaching the sidebar rebuilds the main window, the new main window calls `attach_gpu` as usual, and the first redraw copies the latest frame. GUI confirmation needed that the main window always has a picture while the sidebar floats (R5 below).
- The source cap of C-SIZE exists only under software rendering; GPU renders at the video area's physical pixels, and mpv does the scaling.

**Integration notes for the author (line C, video plugin)**

- E16 "HDR tone mapping": `const render = plugin.video.render();` (the manifest already has `processing.video`). Radio choice → `render.set({ toneMapping })`; "Target brightness" → `targetPeak` (100–400); "Dynamic peak detection" → `peakDetect` (no effect on macOS; the menu can stay). Non-HDR check: only when `(await plugin.player.stats()).video?.transfer` is `"pq"` or `"hlg"` is it HDR. Persist as `render.*` per the requirements, and on activation `set` the saved values in one call.
- E17 "Pixel shaders": `render.set({ shaders: [...] })`; must be absolute paths, the files must exist, at most 16; when the master switch is off, `set({ shaders: [] })` (the plugin keeps the list itself). A compile failure is a `failed` pushed later (`code: "failed"`, `message` starts with `shader compile failed: `, mpv does not report the file name); show "Shader failed to load" per the requirements. "Open shader folder" uses `plugin.system.openPath` (no more `spawn("open")`).
- E18 "Motion interpolation": `render.set({ interpolation: true/false })`.
- Disable conditions: when `render.status` is `failed` and `error.code === "unsupported"` (software rendering, Windows), the three groups of commands are disabled, with the reason "Requires GPU rendering (currently software rendering)"; replace the old constant `GPU = "等待 GPU 渲染原型结论"` ("waiting for the GPU rendering prototype conclusion"). `conflict` means another plugin holds the render options. Example in `doc/plugins/examples.md` §34.
- info plugin (owned by the author): the "Hardware decoding" row can switch to reading `stats().video.hwdec` (`"no"` shows "Not used", anything else shows the decoder name); the `backend` field already distinguishes the two render modes accurately. This row is outside what line A may write, and was not changed this round.

**For Maintainer B**: once GPU rendering is on by default, `show_frame` is only called for produced streams, so picture-in-picture should switch to `GpuHandle::latest()` (sequence number and IOSurface of the latest frame) and `on_frame()` (called back once for every frame the render thread publishes), and can use `CVPixelBufferCreateWithIOSurface` directly, with no copy. In the macOS platform module `GpuHandle` is the crate's `Handle`, so both methods can be called directly; in the host it is obtained through `self.backend.render.gpu()`.

**GUI test points for the coordinator** (release: `cd src/host && CARGO_TARGET_DIR=/tmp/vestra-review-target/render cargo build --locked --release`)

Sources:

```sh
ffmpeg -f lavfi -i testsrc2=size=3840x2160:rate=30 -t 30 -c:v libx264 -pix_fmt yuv420p -preset veryfast /tmp/t2160.mp4
ffmpeg -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 30 -c:v libx264 -pix_fmt yuv420p /tmp/t1080.mp4
```

CPU measurement (in another terminal while the player plays in the foreground; `PID` is the `vestra-host` process; drop the first sample and average):

```sh
PID=$(pgrep -x vestra-host | head -1)
top -l 21 -s 1 -pid $PID -stats cpu | awk '/^%CPU/{getline v; n++; if(n>1){s+=v; if(v>m)m=v}} END{printf "mean=%.0f%% max=%.0f%%\n", s/(n-1), m}'
```

| # | Scenario | Expected / record |
| --- | --- | --- |
| R1 | Default launch, open the 4K source; measure CPU for 20 seconds each windowed and fullscreen; then repeat launching with `VESTRA_SOFTWARE_RENDER=1` | Prototype reference: GPU windowed 19%, fullscreen 14%; software 81%, 152%. `backend` in the info panel / `stats()` is "GPU rendering (OpenGL)" and "Software rendering" respectively, `hwdec` is `videotoolbox` / `no` |
| R2 | Picture orientation, color, sharpness; OSD, info blocks, context menu, fullscreen floating bottom bar and subtitles are all above the picture | Same as software rendering |
| R3 | Change source: open another file while playing, then open the first one again; close the file and open again; open a portrait video with a rotation tag | A picture every time; no flash of the previous source's last frame first; the portrait video does not flash an unrotated frame |
| R4 | Resize: drag the window corner continuously for 10 seconds; enter and leave fullscreen | The picture follows, no tearing, no misalignment |
| R5 | UI rebuild and floating sidebar: disable / enable a plugin with mounted UI while playing; detach the playlist sidebar into a floating window and attach it back | Playback is not interrupted, and the main window always has a picture (the floating window itself has no video area) |
| R6 | Produced stream: open NES, drag the window to resize and enter/leave fullscreen while playing; stop NES and open an ordinary video | The NES picture is not covered by black or an old video; the ordinary video then displays normally |
| R7 | Launch with `VESTRA_FRAME_LOG=1` and play 1080p | One line per second `produced ~30 shown ~30`, and `target` is the physical size of the video area |
| R8 | Render options (can be skipped before the plugin is wired up, or build a temporary plugin as in examples §34): switch `toneMapping`, add an invert shader, add a shader with a syntax error, turn on `interpolation` | Playback is not interrupted when switching; invert takes effect; the bad shader's status becomes `failed` with the compiler error; with motion interpolation the picture does not stutter, and the `VESTRA_FRAME_LOG=1` frame count is close to the display refresh rate |
| R9 | Snapshot: `--snapshot` or a snapshot with effects | Record whether the snapshot contains video (whether the GPU texture gets into `take_snapshot`, not verified by the prototype; the plugin's snapshot uses offline FFmpeg and is unaffected) |

**Checks** (macOS, all actually run): `src/`: `npm run check`, `format:check`, `boundaries` pass; `test:sdk` 102 pass (5 new in `tests/sdk/render.test.ts`); `plugins:build` passes; 19 plugin test suites, 488 pass in total, 0 failures (workshop's `vendor/sdk` resynchronized); `npm run portability`: `portability: clean` (the host compiles for the Windows target, with the GPU part going through `no_gpu`). `src/host/`: `cargo fmt --check` passes; `cargo clippy --locked --all-targets -- -D warnings` passes (also run with `--features pip` and `-p vestra-gpu-render`, both pass); `cargo test --locked` 240 pass; `cargo test -p vestra-gpu-render` 2 pass; `grep -rnw unsafe src/host/src` is empty. Not run: GUI (done by the coordinator as agreed), HDR source testing (no HDR10 source on this machine).

### 3.3 Line A delivery 2: Windows gaps (Maintainer A, 2026-10-21, commit "Windows gaps: …")

The three items in [portability task sheet](2026-10-portability.md) §4.2. On this machine only cross-compilation checks are possible; nothing has run on Windows.

| Gap | Approach | Location |
| --- | --- | --- |
| Snapshot and clip export | The offline worker process still passes the held source file to FFmpeg as standard input (lease and before/after validation unchanged); only the input name comes from the platform module: Unix `/dev/fd/0`, Windows `fd:` (the `fd` protocol from FFmpeg 6.1 on, which can seek on a file handle; `pipe:` cannot). `encode`, MP4 validation and embedded subtitle extraction drop `cfg(unix)`, and the Windows "unavailable" stubs are removed; `ffprobe` looks for `ffprobe.exe` in the same directory using `EXE_SUFFIX` | `platform::HELD_FILE_INPUT`; `media_worker.rs`, `native_subtitle.rs` |
| `openStream` | The transport goes into the platform module: Unix still uses a Unix socket in a private directory under `/tmp` (`av_unix`); Windows uses loopback TCP (`av_tcp`, std only), because Node does not support Unix socket paths on Windows. Any process on the machine can connect to a loopback port, so the player generates a 32-byte random token (`getrandom`, 0.3.4, already in the lock file); after connecting, the SDK sends the token first, and connections with the wrong token are dropped. The reply to `producer.prepare` gains an optional field `token`, and `socket` is `tcp://127.0.0.1:<port>`; the SDK connects according to the address form (`connectTo`), not according to `process.platform`. Also: on Unix, accepted connections are set back to blocking mode (macOS inherits non-blocking from the listener, so the old read timeout did nothing and the read loop spun) | `av_unix`, `av_tcp`, `av_listen` in `platform/mod.rs`; all `cfg(unix)` removed from `producer.rs`; `sdk/producer.ts` |
| Confirmation dialog button text | rfd enables `common-controls-v6` (`TaskDialogIndirect`, which allows custom buttons), and `build.rs` embeds a Common Controls v6 application manifest on MSVC targets with the linker arguments `/MANIFEST:EMBED /MANIFESTINPUT:windows/vestra.manifest` (a program with this feature enabled cannot start without the manifest). No new crate. When the two buttons have the same text, a no-break space is appended to the cancel button so the answer is not ambiguous (`confirm::button_labels` / `confirmed_by`, with unit tests). The checkbox is still a second "Yes / No" dialog (rfd does not offer task dialog checkboxes) | `Cargo.toml`, `build.rs`, `windows/vestra.manifest`, `platform/windows.rs`, `confirm.rs` |

Also: host warnings for the Windows target dropped from 3 to 1 (`no_gpu` became an ordinary type with no effect, so there is no more "unreachable code"; the `media_worker` cancellation test is no longer Unix-only). The remaining one is the `playback` field in `source_backend.rs` (the `fd://` lease, another item in the §4.2 table, not in scope this round).

**SDK / TestHost / docs**: `PreparedProducer.token?`, `connectTo`; `TestHost.producerTransport = "tcp"` (the fake producer checks the token, `producer.admitted`); new `tests/sdk/windows-gaps.test.ts` (2 items: the full `openStream` flow over TCP with token, and `connectTo` treating only `tcp://127.0.0.1:` as loopback); new host tests: the loopback transport accepts only connections with the token, button text disambiguation. Reference §5.2 (transport), §7 (Windows task dialog), §10 (Windows export and FFmpeg 6.1), §14, §16.7, §17 "Render round: Windows gaps".

**For the coordinator**: there is no Windows machine, so the following can only be verified once a Windows environment is available: (1) the linked `vestra-host.exe` starts (the manifest is embedded: `mt.exe -inputresource:vestra-host.exe;#1 -out:m.xml` can export a manifest containing `Microsoft.Windows.Common-Controls`); (2) plugin confirmation dialogs show custom buttons; (3) snapshot export (`VESTRA_FFMPEG` pointing at a 6.1+ `ffmpeg.exe`); (4) NES produced streams show picture and sound. On macOS this commit does not change behavior (except that the produced-stream read loop no longer spins), so regression can cover just R6 (NES) and one plugin snapshot export.

**Checks** (macOS, all actually run): `src/`: `npm run check`, `format:check`, `boundaries` pass; `test:sdk` 104 pass; `plugins:build` passes; 19 plugin test suites, 488 pass in total, 0 failures (workshop's `vendor/sdk` resynchronized; in each of the predecessor's two full runs one timing test failed intermittently, playback once and subtitles "cancel during upload" once, and both pass when rerun alone; after taking over, one more full run passed completely); `npm run portability`: `OK: the host compiles for Windows`, `portability: clean`. `src/host/`: `cargo fmt --check` passes; `cargo clippy --locked --all-targets -- -D warnings` passes (also run with `--features pip`); `cargo test --locked` 243 pass; `grep -rnw unsafe src/host/src` is empty. Not run: Windows linking and running (no MSVC linker and no Windows environment).

### 3.4 Line A delivery 3: merge picture-in-picture, feed GPU frames to picture-in-picture (Maintainer A, taking over, 2026-10-21, commits "Merge rebuild/render-pip …" and "Render: feed picture-in-picture from the GPU path")

The previous Maintainer A finished the code and records for §3.2 and §3.3 but did not commit §3.3. After taking over, it was reviewed first: the §3.3 changes are kept as they were, except that a misplaced doc comment in `platform/windows.rs` was moved back to `default_ffmpeg`; all checks were rerun before committing. Then `rebuild/render-pip` (Maintainer B's 11b467b) was merged, and GPU frames were wired up according to §3.1.3.

**Merge**: conflicts were in `Cargo.toml` (both unsafe sub-crates are ordinary macOS dependencies, and neither of the prototype features `gpu-render` and `pip` exists any more), the overview table in `platform/mod.rs`, reference §17 (both entries kept), and §3 of this sheet (B's §3.1 keeps its number because the probe source refers to it; A's two deliveries moved to §3.2 and §3.3). `main.rs` merged automatically: the prototype's whole `#[cfg(feature = "pip")]` section was replaced by B's final implementation, and in `show_frame` software frames still go to picture-in-picture. The merge commit was checked separately with all checks (see below).

**The four items of §3.1.3**

| Item | Approach |
| --- | --- |
| (1) GPU frame callback feeds picture-in-picture | `pip::Surface` gains an associated type `Frame` and `push_surface(&Self::Frame)`; `PictureInPicture::push_surface(sequence, frame)` sets `fed` as `push` does, and each sequence number is sent only once (reset on close and reopen). The first time a host tick sees a GPU renderer, it registers `GpuHandle::on_frame` once: for every frame the render thread publishes, it calls `invoke_from_event_loop` once only while picture-in-picture wants frames (`pip.feeding()`, written into an `AtomicBool` every tick; unhandled wake-ups are coalesced), and the UI thread takes `GpuHandle::latest()` and feeds it. While a produced stream (NES) is using the video area, no GPU frames are sent; its pixel frames still go through `show_frame` |
| (2) BGRA, no rewriting of an IOSurface in use | The render sub-crate's ring was already `'BGRA'` IOSurfaces and already skips the one that is `IOSurfaceIsInUse` when rotating (present since the prototype was merged in §3.2). The ring has 3 buffers; when all are in use it still writes the next one (better occasional tearing than a frozen picture); see G3 in GUI testing |
| (3) Start from the latest buffer when paused | In `pip_tick`, when `needs_frame()`: GPU rendering sends `latest()`, software rendering still uses `last_image` |
| (4) Remove `allow(dead_code)` | The inherent method `PipWindow::push_surface` is removed and becomes the `Surface` implementation, calling `vestra_pip::Pip::enqueue_surface` (`CVPixelBufferCreateWithIOSurface`, zero copy) |

**Signature changes** (plugin API unchanged, host-internal only): `pip::Surface::{type Frame, push_surface}`; the platform module gains `GpuFrame` (on macOS `vestra_gpu_render::IOSurfaceRef`, the same `objc2-io-surface` 0.3.2 type as `vestra_pip::IOSurfaceRef`; an empty enum on other systems); the `no_gpu` `GpuHandle` gains `latest()` (always `None`) and `on_frame()` (does nothing); `push_surface` in `pip_unsupported.rs` is `match *self {}`. The host main crate is still `forbid(unsafe_code)`, and the unsafe sub-crates were not changed this time. New unit test `pip::gpu_frames_feed_the_window_once_each_and_only_while_it_exists`. Reference §6.6 and §17 (picture-in-picture entry) gain one sentence: with GPU rendering, the small window shows GPU frames, at the resolution of the main window video area's render size.

**For the author**: integration notes unchanged; for the video plugin E16/E17/E18 see "Integration notes for the author" in §3.2, for window plugin H07 see §3.1.4. One addition: once E18 motion interpolation is on, the GPU renderer produces frames at the display refresh rate, and picture-in-picture receives just as many frames.

**GUI test points for the coordinator** (the default build is GPU rendering plus picture-in-picture: `cd src/host && cargo build --locked --release`; R1–R9 see §3.2, P1–P11 see §3.1.5, CPU measurement command see §3.2). P1–P11 were written for software rendering; GPU rendering is now the default, so please run the P series once with a default launch, and check the following additionally:

| # | Scenario | Expected / record |
| --- | --- | --- |
| G1 | Default launch (GPU), play 1080p, open picture-in-picture | The small window appears within 1 second, the picture fills it, color and orientation correct (GPU frames are BGRA; record it if red and blue are swapped); `stats().backend` is still "GPU rendering (OpenGL)" |
| G2 | Open picture-in-picture while paused | The small window starts with the current picture (taking the latest frame), not black; then click play in the small window and the picture moves |
| G3 | With picture-in-picture open, play 4K, turn on `interpolation` (motion interpolation), drag the small window larger | No tearing, no flashback to old frames (the render ring has only 3 buffers, and when all are in use it writes the next one; record any tearing, and enlarge the ring if necessary) |
| G4 | With picture-in-picture open, change source (video → video), resize the main window, enter and leave fullscreen | The small window switches to the new picture with correct proportions; after resizing, the small window's resolution follows the video area |
| G5 | With picture-in-picture open, open NES, then stop NES and open an ordinary video | The NES picture goes into the small window (via software frames), with no old GPU frames slipping in; the ordinary video is normal afterwards |
| G6 | Launch with `VESTRA_SOFTWARE_RENDER=1` and repeat P1, P3 | Same as §3.1.5 (software frame path) |
| G7 | CPU: 4K playback, measure 20 seconds each with picture-in-picture off / on under GPU rendering; measure once more with picture-in-picture on under software rendering | Use the `top` command from §3.2. Expected: under GPU the difference between picture-in-picture on and off is small (zero copy, just one extra wake-up and enqueue); under software rendering, picture-in-picture on adds the cost of the RGB→BGRA copy. Record all three numbers |

**Checks** (macOS, actually run on this machine, once for each of the three commits, with the same results; the numbers below are for the last commit): `src/`: `npm run check`, `format:check`, `boundaries` pass; `test:sdk` 107 pass; `plugins:build` passes; 19 plugin test suites, 488 pass in total, 0 failures (workshop's `vendor/sdk` is byte-for-byte identical to `src/sdk`, so no resync was needed after the merge); `npm run portability`: `OK: the host compiles for Windows`, `portability: clean`. `src/host/`: `cargo fmt --check` passes; `cargo clippy --locked --all-targets -- -D warnings` passes (also run with `-p vestra-pip -p vestra-gpu-render`, passes); `cargo test --locked` 250 pass (243 at the Windows gaps commit, 249 at the merge commit); `grep -rnw unsafe src/host/src` is empty. Not run: GUI (done by the coordinator), Windows linking and running.

### 3.5 Picture-in-picture fix: the small window shows only one corner of the picture (Maintainer B, 2026-10-21, commit "PiP fix: …")

**Coordinator's test of 11b467b** (software rendering, 1080p): the small window was tall (about 205×270 points) and showed only the top-left part of the picture (the first 3 of the 7 color bars, enlarged), with the lower half all black; the standalone program `check` mostly filled it.

**Merge first**: merged `rebuild/render-pip` in the worktree (including A's GPU rendering and 0ab8cde "GPU frames feed picture-in-picture"); conflicts were only one each in `main.rs`, `pip.rs` and `platform/macos.rs` (each side added a field or method), and both sides were kept.

**Data printed in the host** (`VESTRA_PIP_LOG=1`, one line printed whenever it changes: video area (points), render size, child window and layer, frame, renderSize, small window size). Before the fix, with a 780×671-point video area on a 2× screen:

```
video area (0,0 780x671) pt | parent content (335,220 780x671) scale 2 | child frame (335,220 780x671) scale 2
layer (0,0 780x671) in (0,0 780x671) | frame (1560, 1342) | renderSize (780, 671) | active true
```

All rectangles are correct: the child window sits exactly on the screen rectangle of the video area, with no pixel / point confusion and no y flip. The problem was elsewhere:

1. **AVKit does not scale (root cause)**. Comparing screenshots (reproduced by both `check` and the offline probe; the probe's `normal`, the approach Apple recommends, behaves the same): the small window (`PIPPanel`, a window of this process, content about 328×185 points) shows the source layer **1 point to 1 point**, taking the block at the bottom-left corner of the layer and cropping the rest; when the layer is smaller than the small window, the surroundings are white (the prototype's "white border" has the same cause; judging by renderSize in §3.1 was wrong). AVKit's own `AVPictureInPictureSampleBufferDisplayLayerView` inside the small window keeps the source layer's size (780×439) and does not scale with the small window. Tried without effect: the layer's `contentsScale`, the layer as the view's root layer, running inside a .app bundle, hiding `AVPictureInPictureCALayerHostView` (the approach from Apple forum FB22411168, no effect on this machine), changing the frame/bounds of AVKit's view (horizontal right, vertical misplaced). What works: **while the small window is open, set the source layer to the size of the small window's content** (probe experiment: a 320×180 layer in a 328×185 small window almost fills it).
2. **Frames have black bars**. The host's frame is the size of the video area (1560×1342), with a 16:9 picture letterboxed inside. AVKit sets the small window's shape from the frame's shape, so when the video area is tall the small window is tall too, with black bars (the tall small window the coordinator saw).

**Fix**

- `vestra-pip`: every tick, `Pip::place` looks for `PIPPanel` among this process's windows; while the small window is open the layer frame = the small window's content size (following when the small window is resized by dragging), and after closing it goes back to the video area. `describe()` prints the diagnostic line above.
- Host: when picture-in-picture wants frames, both renderers render at **the picture's own shape**, i.e. the largest rectangle of that shape inside the video area (`render_target::fit`), so frames have no black bars. The picture shape comes from mpv's `osd-dimensions` (the last frame minus the margins; both renderers are `vo=libmpv`, so rotation, aspect override and zoom are already included); the control thread reads it every 100 ms into `render_target::Shared`. Frames rendered at this size have no margins, so the measured shape does not change and stays stable; on source change it follows the new shape by itself. Frames whose shape does not match the size last sent to the renderer are not sent to picture-in-picture (`pip_shape`, to avoid the small window opening with the old shape); produced-stream (NES) frames are unaffected. During picture-in-picture the main window shows grey text and no video, so the changed render size is not visible; after closing it goes back to the video area size.
- Software frames and GPU frames (IOSurface, zero copy) follow the same size rule, with no cropping or copying needed.

**Data after the fix** (same machine, `check` and the host; screenshots confirm the small window is filled, all 7 color bars are complete, there are no black or white bars, and the main window is normal after closing):

| Scenario | Render size (video area → sent) | Frame | renderSize | Small window / layer |
| --- | --- | --- | --- | --- |
| Software rendering, video area 780×671, playing | 1560×1342 → 1560×878 | 1560×878 | 780×439 | 328×185 / 328×185 |
| GPU rendering, same | 1560×1342 → 1560×878 | 1560×878 (229 frames, all IOSurface) | 780×439 | 328×185 / 328×185 |
| Software rendering, video area 400×600 (tall), opened while paused | 800×1200 → 800×450 | 800×450 (one frame re-rendered while paused) | 400×225 | 328×185 / 328×185 |
| GPU rendering, same | 800×1200 → 800×450 | 800×450 (IOSurface) | 400×225 | 328×185 / 328×185 |
| `check -- --frame 1560x878 --area 780x671` | — | 1560×878 | 780×439 | 329×185 / 329×185 |

**The `check` example** gains `--frame WxH --area WxH [--still]`: the main window content is the video area, frames use the given size (the host now sends frames in the picture's shape), and the diagnostic line is printed while open. Without arguments it behaves as before.

**Retest for the coordinator**: redo P1, P2 (drag the small window larger and smaller; the picture should always fill), P5, P8 (video → video switching to a source with a different aspect ratio; the small window's shape follows), G1, G2, G6; also open the sidebar so the video area becomes tall and then open picture-in-picture (exactly this time's scenario). When data is needed, launch with `VESTRA_PIP_LOG=1`.

**Checks** (actually run on this machine): under `src/host/`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo clippy --locked -p vestra-pip -p vestra-gpu-render --all-targets -- -D warnings` pass; `cargo test --locked` 251 pass (new `render_target::picture_in_picture_renders_the_picture_shape_inside_the_area`); `cargo test -p vestra-pip` 3 pass; under `src/`, `npm run portability` clean; `grep -rnw unsafe src/host/src` is empty, and all 25 `unsafe` blocks in `vestra-pip` have `// SAFETY:`. GUI: the 5 scenarios in the table above were confirmed by launching and taking screenshots on this machine (the host used a temporary test plugin, not committed, to open a 1080p color-bar source and toggle picture-in-picture). No TS changes.

## 4. Coordinator verification (2026-10-21)

**All checks pass**: 19 plugin test suites (503 tests), test:sdk, cargo test (251 tests); `npm run portability` result clean; zero `unsafe` in the host main crate.

**GPU rendering (release, on by default)**: picture orientation, color and fullscreen are all correct, and hardware decoding uses videotoolbox. Host CPU with a 4K source:

| 4K | GPU rendering | Software rendering (`VESTRA_SOFTWARE_RENDER=1`) |
| --- | --- | --- |
| Windowed | 14% | 83% |
| Fullscreen | 18% | — |

**HDR**: PQ sources are tone-mapped by default; switching to "Off" takes effect immediately and shows an OSD message.

**Picture-in-picture**:
- The small window is 16:9 and the picture fills it completely; it still fills after dragging larger; the main window recovers after closing. Both the software and GPU paths work.
- Turning it on from the window plugin menu "Screen ▶ Picture-in-Picture" works.
- The root cause is that under macOS 26 AVKit does not scale sample-buffer picture-in-picture; Apple has similar feedback FB22411168; the approach is to make the source layer follow the small window's size and render at the picture's own aspect ratio.

**Fixed on the spot**: when the window plugin activates, the main window does not exist yet, so it read supported=false; changed to follow `onChange`.

**Not done**:
- Running on a real Windows machine.
- `menus.md` has not been updated with the "Pixel Shaders ▶" and "Picture-in-Picture" menus added this round.
