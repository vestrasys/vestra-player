//! Go/no-go probe for Windows GPU rendering through the D3D11 → OpenGL
//! bridge, since parked for wgpu/D3D12 (doc/process/rounds/history.md §5).
//!
//! * G1: libmpv's Direct3D 11 render API (`api_type = "d3d11"`, render API 2.6)
//!   renders into a `B8G8R8A8` texture on an `ID3D11Device` we create, with
//!   d3d11va decoding on that same device.
//! * G2: an OpenGL context in a hidden window (never shown, never activated)
//!   reads that texture through `WGL_NV_DX_interop2`, as Slint's femtovg
//!   window would.
//!
//! ```text
//! gpu-probe info
//!     DXGI adapters and outputs, the hidden GL context (GL_RENDERER, LUID
//!     through GL_EXT_external_objects_win32, WGL extensions), and for every
//!     D3D adapter: wglDXOpenDeviceNV + register + lock + blit + read back of a
//!     cleared texture (the hybrid-GPU matrix without mpv).
//! gpu-probe run [--adapter default|warp|<index>] [--src <path or URL>]
//!               [--size WxH] [--seconds S] [--warmup S] [--hwdec MODE]
//!               [--gl] [--host-vf] [--png out.png] [--log LEVEL]
//!     mpv renders --src through the D3D11 render API for S seconds after
//!     the warm-up, timing every mpv_render_context_render (CPU wall time
//!     and GPU time by timestamp queries), the process CPU time, and with
//!     --gl the interop lock + blit + unlock per frame; then reads back the
//!     last frame from D3D (and from GL with --gl) and compares them.
//!     [--vf FILTERS] sets mpv's `vf` before loadfile (after the host chain
//!     with --host-vf), asks mpv for at least `v` logs, and prints the
//!     vf/vd/d3d11vpp/ffmpeg-video and "Super Resolution" lines at the end;
//!     [--at T] after the measurement pauses on the frame at T (exact seek)
//!     and reads that back, so --png captures the same frame in every run.
//!     video-params, video-out-params and the adapter
//!     are printed with the results (for the AI upscaling checks).
//!     [--glsl-shaders A;B] sets mpv's `glsl-shaders` (the shader upscaling
//!     plugin's chains), [--set NAME=VALUE] any further mpv option, both
//!     before loadfile, [--set-late NAME=VALUE] a property after
//!     `mpv_initialize` (as the host sets `gpu-shader-cache-dir`); [--sample S] prints every S seconds the frame rate,
//!     render CPU/GPU times, this process's video memory (local and
//!     non-local, `QueryVideoMemoryInfo`), private bytes, the system commit
//!     charge and `GetDeviceRemovedReason`, and stops on a removed device;
//!     [--hog-mb N] commits and touches N MiB first (memory pressure) and
//!     [--max-commit PCT] (default 92) keeps the system commit below PCT:
//!     the hog stops short of it and the run ends once it is passed.
//! gpu-probe adapters
//!     The DXGI adapter list only (index, name, vendor, LUID).
//! ```
//!
//! `gpu-probe-dgpu` is the same program with the hybrid-GPU export hints.
//! Windows only; elsewhere `main` reports that and exits.

#[cfg(windows)]
mod probe;

/// Run the probe with the process arguments; returns the exit code.
pub fn main() -> i32 {
    #[cfg(windows)]
    return probe::main();
    #[cfg(not(windows))]
    {
        eprintln!("gpu-probe: Windows only");
        2
    }
}
