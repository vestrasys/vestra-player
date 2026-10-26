//! `gpu-probe` with the hybrid-GPU hints exported (see build.rs): the NVIDIA
//! (Optimus) and AMD (PowerXpress) drivers then put this process's OpenGL
//! context on the discrete GPU.

/// Read by the NVIDIA driver: 1 = prefer the discrete GPU.
#[cfg(windows)]
#[unsafe(no_mangle)]
pub static NvOptimusEnablement: u32 = 1;
/// Read by the AMD driver: 1 = prefer the high-performance GPU.
#[cfg(windows)]
#[unsafe(no_mangle)]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

fn main() {
    #[cfg(windows)]
    {
        // Keep the statics referenced so the linker cannot drop them.
        std::hint::black_box((&NvOptimusEnablement, &AmdPowerXpressRequestHighPerformance));
    }
    std::process::exit(vestra_gpu_probe::main());
}
