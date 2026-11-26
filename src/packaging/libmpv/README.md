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
