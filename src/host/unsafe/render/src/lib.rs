//! libmpv's GPU rendering shown in Slint windows (the host's GPU rendering,
//! SDK site, Concepts: render options; the host reaches it only through its
//! platform modules). One implementation per system, with the same safe API
//! (`Renderer`, `Handle`, `Draw`):
//!
//! - macOS (`macos.rs`): libmpv draws with OpenGL into IOSurfaces that each
//!   window's OpenGL (femtovg) context copies;
//! - Windows (`windows/`): libmpv draws with Direct3D 11 into a ring of
//!   textures shared with the windows' Direct3D 12 device, which Slint's
//!   femtovg renderer on wgpu shows directly (no copy).
//!
//! All `unsafe` stays in this crate; the public API is safe.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
