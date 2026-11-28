# libmpv for Windows, built here

`build-windows.sh` builds the player's `libmpv-2.dll`: FFmpeg 9.1 and mpv (the commit and options of the shinchiro build we used before), linked statically into one DLL that imports only Windows DLLs and `vulkan-1.dll`. `pin-local.ps1` puts a build into a local deps directory. User decision (T8): we build libmpv ourselves and keep the patch below in this repository; no upstream pull request.

Why we build it: motion smoothing's NVIDIA engine needs FFmpeg's `fruc_vulkan` filter (FFmpeg 9.1 and later, built with a GLSL compiler, which the shinchiro build lacks) and the patch below.

## The patch: `mpv-lavfi-hwdevice.patch`

**What it does.** mpv's `lavfi` video filter, which runs an FFmpeg filter graph inside mpv, gets two new options: `hwdevice=<type>` and `hwdevice_name=<device>`. With them, the filter opens a GPU device of that type for itself (for us: a Vulkan device on the NVIDIA GPU the player checked, by its name). It hands that device to the FFmpeg filters in its graph, and it closes the device again when the filter is removed. Without the options, nothing changes.

**Why it is needed.** An FFmpeg filter that works on the GPU, like `fruc_vulkan`, needs a GPU device to upload frames to. In mpv, a filter graph can only borrow the device of mpv's video output, the part that draws the picture. Vestra draws the picture itself with Direct3D 11 (or the CPU), so mpv's output has no Vulkan device to lend. FFmpeg cannot turn a Direct3D 11 device into a Vulkan one either. So without the patch, `fruc_vulkan` has no device and the graph fails to start.

**Feature or bug fix?** A small feature that mpv lacks. mpv works as designed: it assumes a GPU filter uses the same GPU API as the video output. We use a different API for drawing (Direct3D 11) than for the converter (Vulkan). Plain FFmpeg has the same ability on its command line (`-init_hw_device` and `-filter_hw_device`); mpv has no way to say it.

**Why this way.** It is the smallest change that works:
- It affects only a filter that asks for it.
- The device exists only while that filter runs: nothing at start-up, no Vulkan device when motion smoothing is off.
- mpv's video output and its drawing are left alone.

The device is created when the filter is added, before its graph is built, because FFmpeg's `hwupload` checks for it when it starts. The device is kept across seeks and freed with the filter.

**Size of the change.** About 100 lines in one file, `filters/f_lavfi.c`, against mpv `e471496c56`. A newer mpv needs the patch re-applied, and checked if `f_lavfi.c` changed.

## Shipping it (T8: a local build, pinned as a release asset)

Remote CI is paused, and building libmpv takes about 25 minutes plus a MSYS2 install, so the build stays on a machine. What CI would otherwise produce is pinned and checked:

1. **Build.** In MSYS2 UCRT64: `./setup-msys2.sh install` installs exactly the packages in `msys2-packages.lock` (name, version, file and SHA-256 of each; the MSYS2 base archive named in its header). Then run `./build-windows.sh <work dir> all`. `all` refuses to start when the installed packages differ from the lock. The result is `libmpv-dev-x86_64-ffmpeg<ver>-mpv-<commit>.zip`: `libmpv-2.dll`, `include/mpv`, `BUILD-INFO.txt` (the hashes of every input) and the patch. The DLL is stripped and has no link timestamp, so the same inputs give the same DLL. The inputs include the work directory's path: source file names (`__FILE__` in assertions and log lines) and FFmpeg's and mpv's configure lines (`--prefix`) are in the DLL, so the pinned DLL (`libmpv-ffmpeg9.1-20261122`) comes out byte for byte only from the work directory it was built in (`<work dir>/t8/impl/build2`, MSYS2 at `<work dir>/t8/impl/msys64`). The next libmpv build will map these paths away (`-ffile-prefix-map`, `-fmacro-prefix-map` and a neutral `--prefix`); the paths of MSYS2's own build machine in its prebuilt libraries (`D:/M/msys64/…`) stay either way.
2. **Pin.** `deps.json` `upstream.windows-mpv` gets the asset's URL and SHA-256, the DLL's SHA-256, and the MSYS2 package archive (`./setup-msys2.sh archive <dir>`, then tar). The archive keeps the build reproducible after MSYS2 prunes old versions from its repository.
3. **Publish the libmpv release:** `publish.ps1 -Step libmpv -Assets <dir> -Publish`. This makes a pre-release `libmpv-ffmpeg<ver>-<date>` with both files; it refuses files that do not match `deps.json`.
4. **The DLL's source (GPL; optional, for the public release).** `./build-windows.sh <work dir> sources` (not part of `all`) makes `libmpv-sources-ffmpeg<ver>-mpv-<commit>.tar`: FFmpeg's, mpv's and libaribcaption's source archives as downloaded, the patch and these scripts, the MSYS2 source packages of every library linked into the DLL (`msys2-linked.txt`, written by `./setup-msys2.sh linked <work dir>` from the DLL's link line; `msys2-sources.lock`, by `./setup-msys2.sh sources <dir> lock`), and the build's own record of its configure line, meson options and link line. Nothing uses it yet: the source is published together with the player's public release and open-sourcing (user ruling, T8). Then `deps.json` `upstream.windows-mpv` pins it (`sources`, `sources_sha256`), `publish.ps1 -Step sources -Assets <dir> -Publish` attaches it to the libmpv release (next to the DLL, so its URL is known before the deps zip that names it is made), and `collect-windows-deps.mjs` writes the URL and SHA-256 into the shipped `components.json` when the pin is there.
5. **Publish the deps release:** `publish.ps1 -Step deps -Publish`. This is the Windows half of `deps.yml` on this machine: `collect-windows-deps.mjs` fetches and checks the pinned libmpv asset (zip and DLL), the Vulkan loader, and BtbN's FFmpeg tools from the current deps release's Windows zip (BtbN's own autobuild is gone after a few weeks); then it makes `mpv.lib` and `vestra-deps-windows-x64.zip`. The macOS assets of the current deps release are copied unchanged. It creates `deps-<date>.N` and rewrites `deps.json`'s `release` and `assets`.
   - With remote CI, `gh workflow run deps.yml -f platforms=windows` does the same from the same pin.

Without `-Publish`, `publish.ps1` only prints what it would upload. `pin-local.ps1` remains for trying a build in a local deps directory without publishing anything.

# libmpv for macOS, built here

`build-macos.sh` builds the player's macOS `libmpv.2.dylib` (arm64) from the **same mpv commit as Windows** (`MPV_COMMIT`, pinned by SHA-256 as in `build-windows.sh`), with the Windows patch (`mpv-lavfi-hwdevice.patch`) and one more: `mpv-vt-frc.patch`, Apple's frame interpolation as an mpv video filter (`vtfrc`). It links against the installed Homebrew FFmpeg and libraries (FFmpeg is not rebuilt; Homebrew's own mpv is not used). The mpv CLI is built from the same tree for measurements; it is not shipped.

Why we build it: motion smoothing's Apple engine needs the `vtfrc` filter, and Homebrew's mpv is a release without either patch.

## The patch: `mpv-vt-frc.patch`

**What it does.** A new video filter, `--vf=vtfrc[=engine=auto|lowlatency|regular:quality=normal|quality:cut=20]`, that doubles the frame rate with VideoToolbox's `VTFrameProcessor`: for each pair of consecutive frames it outputs the first frame and one interpolated frame at the midpoint PTS.

- **Engines.** `lowlatency` is `VTLowLatencyFrameInterpolation` (macOS 26+): 8-bit 4:2:0 (`420v`) in and out, a few ms per frame, but a size limit (below). `regular` is `VTFrameRateConversion` (macOS 15.4+): any size, but half-float RGBA (`RGhA`) only, so frames are converted with `VTPixelTransferSession` on the way in and back on the way out; `quality=` is its quality prioritization. `auto` (default) takes `lowlatency` when it is there and its probe passes, else `regular`.
- **The low-latency size limit** is not published and no macOS 26 API reports it. Measured on an M2 (macOS 26.2): width × height ≤ 921,600 (1280×720's pixel count, any aspect: 1600×576 and 720×1280 work, 1281×720 and 1920×482 do not). A session over the limit *starts* without an error and fails its first frame with -19730 ("Processor is not initialized"). So the session start interpolates one pair of black frames as a probe; a failed probe means `regular` (under `auto`) or a failed filter (under `lowlatency`). This also covers Macs with other limits.
- **Input.** VideoToolbox hardware frames (`IMGFMT_VIDEOTOOLBOX`, NV12 or P010 CVPixelBuffers) go to the processor without a copy when it takes them: same pixel format, IOSurface-backed, and the padding its `sourcePixelBufferAttributes` ask for (the low-latency interpolator wants the height padded to a multiple of 16, e.g. 534 → 544, and fails with -19740 on the decoder's unpadded buffers; those are copied into a padded buffer by `VTPixelTransferSession`). Software NV12/P010 frames (copy-back decoding) are copied into an IOSurface-backed CVPixelBuffer pool; other formats (software decoding's yuv420p) are converted to NV12 by mpv's autoconvert. The output has the input's format: hardware in, hardware out (interpolated frames in IOSurface-backed, Metal-compatible buffers, so mpv's VideoToolbox interop maps them); software in, software out.
- **Scene cuts.** Both processors interpolate across a hard cut, which gives a ghosted mix of the two shots. A pair whose 32×18 luma thumbnails differ by more than `cut` (mean absolute difference on an 8-bit scale; 0 turns it off) repeats its first frame instead of an in-between frame. The regular processor keeps state between pairs: a pair is sent as `Sequential` only when its first frame is the last frame the processor interpolated, otherwise (after a cut, a failed conversion or call) as `Random`, as Apple's header asks; without this the pair after a cut came out as a two-shot ghost. Measured on Tears of Steel and Sintel: cuts about 27 (dark to bright) to 47, fast motion at most 13, the median pair 1–3.
- **HDR** (PQ, HLG) is refused: the filter fails on it (`HDR video is not supported`). 10-bit SDR (P010) works; the low-latency interpolator works in 8 bits, so its in-between frames are 8-bit.
- **Threading.** The session start and all processing run on two serial dispatch queues (the processor; the conversions), never on mpv's filter thread. Until the session is ready, frames pass through unchanged, so a slow start does not stall playback. Two pairs are in flight, so one pair's conversions overlap the previous pair's processing.
- **Stalls.** A pair not back after 1 s means the processor stalled (seen once, on the first regular run on this Mac: 109 s on one frame). The pairs in flight then go out without in-between frames and new frames pass through until the processor answers again, so playback does not stop.
- **Seek, format change, EOF.** A seek (filter reset) drops the pairs; running work finishes on its own and its results are dropped. A new size or format starts a new session (frames pass through meanwhile). EOF waits for the pairs in flight and then passes on. Removing the filter never waits: a session outlives its filter while work queued on it runs (refcounted) and ends then.
- **Errors.** The filter logs one clear line under its own prefix (`vtfrc`) and fails, so mpv disables it (`Disabling filter <label> because it has failed.`): when no engine is available (at creation: the `vf add` is refused), when the session cannot start (`frame interpolation unavailable for WxH: …`, or `low-latency interpolator unavailable: …`), on an unsupported format or HDR, after 5 failed interpolations in a row, and when a failed low-latency call reaches the process's limit of 5 (`… giving up`). A single failed pair passes its frames through with a warning.
- **A workaround.** On macOS 26.2 a low-latency call that fails with -19740 (frames without the padding it wants) releases a buffer it does not own: the previous frame's retain count drops from 2 to 1 while every reference is alive, and releasing ours afterwards crashed (also in mpv's renderer). The size refusal -19730 ("Processor is not initialized", the probe's expected answer above the size limit) does not: counts unchanged, and 30 refused sessions were created and released cleanly in a test program. Not seen with the regular processor. So after a failed low-latency call with any code other than -19730, at a frame or in the probe, the filter leaks everything that call saw: three 8-bit buffers and the frame objects, about 4 MB at 1280×720. These are counted for the process (dropped pairs after a seek or stall included); from the 5th on, the low-latency interpolator is not used again in this process (`auto` takes the regular one, `lowlatency` fails, a filter whose pair leaked then gives up), so the leak stays below about 20 MB per process.
- **Probing once per size.** A size the low-latency interpolator refused (at its start or its probe) is remembered for the process: later sessions of that size go straight to the regular interpolator (`… (known from an earlier session)` in the ready line), so removing and adding the filter does not probe again. Measured: 50 `vf remove`/`vf add` cycles at 1080p `auto` (copy-back, `footprint` every 2 s): the process's footprint, lowest per fifth of the run, 423, 242, 120, 122, 125 MB; no growth (before this change the reviewer measured 165 → 683 MB).
- **Statistics.** When the session ends the filter logs pairs, interpolated frames (and how many went to the regular processor as `Random`), repeated scene cuts, failures, and the mean time per frame in the processor and from submission to result (`--msg-level=vtfrc=debug` logs every pair).

**Size of the change.** About 1,200 lines: one new file, `video/filter/vf_vtfrc.m` (Objective-C, manual reference counting as the rest of mpv), and 16 lines in `meson.build`, `meson.options` (`-Dvideotoolbox-frc`) and `filters/user_filters.{c,h}`, against mpv `e471496c56` with `mpv-lavfi-hwdevice.patch` applied first.

## Log lines the player relies on

The host (`src/host/src/playback/motion.rs`) reads these `vtfrc` log lines (prefix `vtfrc`); a change to the patch must keep their wording, level and meaning, or change the host with it:

| Level | Text (start of line) | When |
| --- | --- | --- |
| error | `VideoToolbox frame interpolation (<engine>) is not available on this system` | at `vf add`: no engine on this Mac or system (before macOS 15.4, Intel); the filter is not created |
| v (verbose) | `starting a session for <W>x<H> <format>` | a new session starts (first frame, new size or format); frames pass through until it is ready |
| info | `<engine> interpolator ready for <W>x<H> after <N> ms` | the session is ready; doubling starts |
| warn | `the processor has not answered for <N> ms: frames pass through until it does` | a stall (above) |

## Building and packaging

`./build-macos.sh <work dir> all` (or the steps `tools`, `mpv`, `package`): meson and ninja from pinned wheels into `<work dir>/venv`; mpv fetched, checked, patched, configured with Homebrew's meson options (lua=luajit, javascript, libarchive, uchardet, vulkan; no build date, no docs) and installed into `<work dir>/stage/usr/local` (`lib/libmpv.2.dylib`, `include/mpv`, `bin/mpv`, `BUILD-INFO.txt`, and `vestra-libmpv.json`, `licenses/`, `patches/` for the packaging). The binaries name the neutral prefix `/usr/local`, and source paths are mapped, so the work directory is not in them (one `__FILE__` from a header compiled through Swift's importer still is). `MACOSX_DEPLOYMENT_TARGET` defaults to 15.0; the filter checks for macOS 15.4 and 26 at run time. It needs the Xcode command line tools, Homebrew with mpv's dependencies (`brew deps mpv`) and python3; nothing is installed into the system.

`package` runs `collect-macos-deps.mjs --libmpv <work dir>/stage/usr/local --out <work dir>/deps`: the same relocatable deps directory as from Homebrew's mpv (dylib closure with `@rpath`, ad-hoc signed, Homebrew's `ffmpeg` and `ffprobe`, `components.json`), with this libmpv as the seed. Its `mpv` component comes from `vestra-libmpv.json` (version `0.41.0+e471496c56`, license, the source archive's URL and SHA-256, the patches and their SHA-256), with mpv's license files and both patches under `licenses/mpv/`; with `--sources <dir>` the mpv archive (checked) and the patches are copied there instead of `brew fetch`. Without `--libmpv` the collector does what it did.

**Shipping it.** `deps.yml` (job `macos`, on `macos-15`) builds with this script and collects with `--libmpv`: Homebrew bottles for mpv's dependencies and FFmpeg, the runner's Xcode 26 for the macOS 26 SDK (`build-macos.sh` refuses an older SDK unless `ALLOW_OLD_SDK=1`: without it the filter has no low-latency interpolator), deployment target 15.0. With `--sources` the sources tar also carries mpv's archive, both patches and `build-macos.sh` with this README (`mpv-build-*`, listed in `components.json` as `source.build_files`). The job then checks that the shipped libmpv has `vtfrc`, targets macOS 15 and imports `VTFrameProcessor`, `VTFrameRateConversion*` and `VTLowLatencyFrameInterpolation*` weakly: libmpv loads on macOS 15.0, and the filter checks the system when it is created (`@available`): before 15.4 it is not created ("not available on this system", the `vf add` is refused); 15.4 to 25 have only the regular interpolator.

## Measurements (M2 MacBook Air, 8 GB, macOS 26.2, 2026-11-28)

Real-time playback of 70 s (the first 10 s not counted), silent (`ao=null`), through libmpv's render API on OpenGL into an offscreen framebuffer (the player's path, VideoToolbox interop included); Blender open movies, 24 fps H.264, fast-motion scenes. "Drops" are mpv's `frame-drop-count` over the last 60 s; the filter's output rate (`estimated-vf-fps`) was 48.00 in every run. Time per frame: in the processor / from submission to result (two pairs in flight).

| Video | Decoding | Engine (auto picks) | Drops / 60 s | Processor | From submission | Process CPU, GPU |
| --- | --- | --- | --- | --- | --- | --- |
| Tears of Steel 1280×534 | VideoToolbox | lowlatency | 0 | 9.3 ms | 14.0 ms | 21 %, 10 % |
| Tears of Steel 1280×534 | copy-back | lowlatency | 0 | 8.3 ms | 12.3 ms | 20 %, 10 % |
| Tears of Steel 1280×534 | VideoToolbox | regular (forced) | 2,325 | 42.9 ms | 84.0 ms | 55 %, 52 % |
| Tears of Steel 1920×800 | VideoToolbox | regular | 0 | 35.6 ms | 62.0 ms | 59 %, 68 % |
| Tears of Steel 1920×800 | copy-back | regular | 0 | 34.1 ms | 59.5 ms | 66 %, 69 % |
| Sintel 1920×818 | VideoToolbox | regular | 0 | 36.0 ms | 62.7 ms | 61 %, 69 % |
| Sintel 1920×818 | copy-back | regular | 0 | 35.2 ms | 61.5 ms | 69 %, 71 % |
| Tears of Steel 1920×800 | VideoToolbox | regular, quality=quality | 658 | 41.6 ms | 80.5 ms | 70 %, 69 % |
| Tears of Steel 3840×1600 (scaled from 1080p) | VideoToolbox | regular | not real time (38 s of video in 110 s) | 121.8 ms | 233.2 ms | 63 %, 68 % |

The pair after a scene cut goes to the regular processor as `Random` (above), which takes longer than a `Sequential` pair: at 1080p this can cost 1–3 dropped frames per 60 s on a scene with many cuts (the re-review measured this; a run of the same scene after the fix had 0). Accepted as the cost of a correct picture (coordinator's ruling); no tuning.

Without the filter the 1080p video decodes at 9 % CPU and 5 % GPU. Session start: low-latency 70–150 ms, regular 350–650 ms once its model is on the machine (the coordinator's first bench measured about 9 s; my first regular run here stalled 109 s on one frame after a quick start). `engine=lowlatency` above its size limit fails at once (`refused on the first frame`) and mpv disables the filter. 24 fps doubled needs at most 41.7 ms per pair: the regular processor holds 1080p in normal quality with about 6 ms to spare, not in `quality`, not at 4K; oddly it is slower on the 1280×534 film (43 ms) than on 1920×800, which does not matter since `auto` takes the low-latency one there.

## Known limits (follow-ups)

- A cancelled session start (the filter removed while its session starts) still runs to completion on its queue before the session is freed.
- The build is not byte-reproducible: one source path (a header's `__FILE__` compiled through Swift's importer) is in the binary, the Homebrew libraries are not pinned, and `min_macos` follows the bottles (26.0 from a local macOS 26 Homebrew; 15.x on `deps.yml`'s macOS 15 runner).
- Full-range NV12 (`420f`) is converted to video range (`420v`) for the low-latency interpolator and back, so its in-between frames lose precision in the range conversion; not measured.
