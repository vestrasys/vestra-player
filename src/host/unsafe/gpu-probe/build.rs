//! Export the hybrid-GPU hints from `gpu-probe-dgpu` only. An executable exports
//! nothing by default, and the NVIDIA and AMD drivers look these symbols up in
//! the process image's export table when the first OpenGL context is created.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        for symbol in ["NvOptimusEnablement", "AmdPowerXpressRequestHighPerformance"] {
            println!("cargo:rustc-link-arg-bin=gpu-probe-dgpu=/EXPORT:{symbol},DATA");
        }
    }
}
