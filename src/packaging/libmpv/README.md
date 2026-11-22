# libmpv for Windows, built here

`build-windows.sh` builds the player's `libmpv-2.dll`: FFmpeg 9.1 and mpv (the commit and options of the shinchiro build we used before), linked statically into one DLL that imports only Windows DLLs and `vulkan-1.dll`. `pin-local.ps1` puts a build into a local deps directory. User decision (T8): we build libmpv ourselves and keep the patch below in this repository; no upstream pull request.

Why we build it: motion smoothing's NVIDIA engine needs FFmpeg's `fruc_vulkan` filter (FFmpeg 9.1 and later, built with a GLSL compiler, which the shinchiro build lacks) and the patch below.

## The patch: `mpv-lavfi-hwdevice.patch`

**What it does.** mpv's `lavfi` video filter, which runs an FFmpeg filter graph inside mpv, gets two new options: `hwdevice=<type>` and `hwdevice_name=<device>`. With them, the filter opens a GPU device of that type for itself (for us: a Vulkan device on the GPU whose name contains "NVIDIA"). It hands that device to the FFmpeg filters in its graph, and it closes the device again when the filter is removed. Without the options, nothing changes.

**Why it is needed.** An FFmpeg filter that works on the GPU, like `fruc_vulkan`, needs a GPU device to upload frames to. In mpv, a filter graph can only borrow the device of mpv's video output, the part that draws the picture. Vestra draws the picture itself with Direct3D 11 (or the CPU), so mpv's output has no Vulkan device to lend. FFmpeg cannot turn a Direct3D 11 device into a Vulkan one either. So without the patch, `fruc_vulkan` has no device and the graph fails to start.

**Feature or bug fix?** A small feature that mpv lacks. mpv works as designed: it assumes a GPU filter uses the same GPU API as the video output. We use a different API for drawing (Direct3D 11) than for the converter (Vulkan). Plain FFmpeg has the same ability on its command line (`-init_hw_device` and `-filter_hw_device`); mpv has no way to say it.

**Why this way.** It is the smallest change that works:
- It affects only a filter that asks for it.
- The device exists only while that filter runs: nothing at start-up, no Vulkan device when motion smoothing is off.
- mpv's video output and its drawing are left alone.

The device is created when the filter is added, before its graph is built, because FFmpeg's `hwupload` checks for it when it starts. The device is kept across seeks and freed with the filter.

**Size of the change.** About 100 lines in one file, `filters/f_lavfi.c`, against mpv `e471496c56`. A newer mpv needs the patch re-applied, and checked if `f_lavfi.c` changed.
