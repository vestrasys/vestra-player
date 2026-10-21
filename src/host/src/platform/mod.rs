//! Everything that differs between operating systems (community workflow
//! §1.5). One implementation per system, chosen with `cfg(target_os)`:
//!
//! - `macos.rs`: AppKit through objc2, the system's own `caffeinate` and
//!   `open`, the login keychain;
//! - `windows.rs`: winit (through Slint), arboard, rfd, opener, keyring, the
//!   registry, and the few Win32 calls of the `vestra-win32` crate;
//! - `other.rs`: Linux and the rest. Everything answers `unsupported`.
//!
//! Common code calls only what this module exports. Each implementation
//! provides the same functions with the same contracts:
//!
//! | area | functions |
//! | --- | --- |
//! | windows | `shown`, `capture_windows`, `associate_new`, `set_floating`, `window_count`, `resize_content`, `screen_for`, `pointer_in`, `modifier_keys`, `double_click_seconds` |
//! | clipboard | `clipboard_read_text`, `clipboard_write_text`, `clipboard_write_image` |
//! | dialogs | `confirm` |
//! | power | `sleep_launcher` |
//! | files and links | `reveal`, `open_path`, `open_url` (blocking, run on a worker thread) |
//! | secrets | `secret_get`, `secret_set`, `secret_delete` (blocking) |
//! | facts | `os_facts` (blocking), `displays` (UI thread), `app_data_dir`, `ui_font`, `default_ffmpeg`, `SLINT_CONTROL_IS_COMMAND`, `CTRL_ACCEPTS_CMD` |
//!
//! Window rectangles are logical points. macOS uses the Cocoa lower-left
//! origin of the global screen space, Windows the top-left origin of the
//! virtual desktop; callers only compare sizes and read `active`.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "other.rs"]
mod imp;

pub use imp::*;

/// A native window of this process, for this process's lifetime only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostWindowId(pub(crate) i64);

/// The windows that existed before a new one was shown (`associate_new`).
#[derive(Clone, Debug, Default)]
pub struct NativeSnapshot {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub(crate) ids: BTreeSet<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    fn usable(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFacts {
    /// The part of the screen windows may use (macOS: without the menu bar
    /// and Dock; Windows: the whole monitor, winit reports no work area).
    pub visible_frame: Rect,
    pub full_frame: Rect,
    /// Dimensionless scale; rectangles above and below remain logical points.
    pub backing_scale_factor: f64,
    pub native_frame: Rect,
    pub active: bool,
    /// The content rectangle in the same coordinates as `native_frame`; the
    /// difference between the two is the window's own chrome.
    pub content_rect: Rect,
}
impl ScreenFacts {
    /// Every rectangle has a finite position and a positive size.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    fn checked(self) -> Result<Self> {
        if ![
            self.visible_frame,
            self.full_frame,
            self.native_frame,
            self.content_rect,
        ]
        .iter()
        .all(Rect::usable)
        {
            return Err(Unavailable(
                "native window or screen geometry is unavailable",
            ));
        }
        if !self.backing_scale_factor.is_finite() || self.backing_scale_factor <= 0.0 {
            return Err(Unavailable("native screen backing scale is unavailable"));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unavailable(pub &'static str);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Unavailable {}

pub type Result<T> = std::result::Result<T, Unavailable>;

/// The largest content size that fits the visible area of the window's screen.
pub fn max_content(facts: &ScreenFacts) -> (f64, f64) {
    (
        facts.visible_frame.width - (facts.native_frame.width - facts.content_rect.width),
        facts.visible_frame.height - (facts.native_frame.height - facts.content_rect.height),
    )
}

/// What `plugin.app.system()` reports about the operating system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsFacts {
    /// `"macos"`, `"windows"`, `"linux"` or `"other"`.
    pub os: &'static str,
    /// The product name, such as `macOS` or `Windows 11`.
    pub name: String,
    pub version: Option<String>,
    pub build: Option<String>,
    /// The hardware model (`Mac15,6`, the firmware's product name), if known.
    pub model: Option<String>,
}

/// One display for `plugin.app.system()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub name: Option<String>,
    /// Pixels in the current mode (logical size × `scale`).
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub refresh_rate: Option<f64>,
    pub primary: bool,
}

/// `os` for the build target.
#[cfg_attr(any(target_os = "macos", target_os = "windows"), allow(dead_code))]
fn os_name() -> &'static str {
    match std::env::consts::OS {
        os @ ("macos" | "windows" | "linux") => os,
        _ => "other",
    }
}

/// The refusal for something this system's implementation does not do.
#[cfg_attr(any(target_os = "macos", target_os = "windows"), allow(dead_code))]
fn unsupported(what: &str) -> crate::RpcError {
    crate::RpcError::new(
        "unsupported",
        format!("{what} is not supported on {}", std::env::consts::OS),
    )
}

/// `secrets.*` on systems with a keyring backend: the login keychain
/// (generic passwords) on macOS, the Credential Manager on Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod keyring_store {
    use crate::RpcError;

    fn entry(service: &str, account: &str) -> Result<keyring::Entry, RpcError> {
        keyring::Entry::new(service, account).map_err(error)
    }
    fn error(error: keyring::Error) -> RpcError {
        let code = match &error {
            keyring::Error::NoStorageAccess(_) => "unavailable",
            keyring::Error::TooLong(..) | keyring::Error::Invalid(..) => "invalid",
            _ => "failed",
        };
        RpcError::new(code, error.to_string())
    }
    pub fn secret_get(service: &str, account: &str) -> Result<Option<String>, RpcError> {
        match entry(service, account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(other) => Err(error(other)),
        }
    }
    pub fn secret_set(service: &str, account: &str, value: &str) -> Result<(), RpcError> {
        entry(service, account)?.set_password(value).map_err(error)
    }
    pub fn secret_delete(service: &str, account: &str) -> Result<(), RpcError> {
        match entry(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(other) => Err(error(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: f64, height: f64) -> Rect {
        Rect {
            x: 0.,
            y: 0.,
            width,
            height,
        }
    }

    #[test]
    fn content_limits_subtract_the_window_chrome_and_bad_geometry_is_refused() {
        let facts = ScreenFacts {
            visible_frame: rect(1440., 875.),
            full_frame: rect(1440., 900.),
            backing_scale_factor: 2.,
            native_frame: rect(800., 628.),
            active: true,
            content_rect: rect(800., 600.),
        };
        assert_eq!(max_content(&facts), (1440., 847.));
        assert!(facts.checked().is_ok());
        let flat = ScreenFacts {
            content_rect: rect(800., 0.),
            ..facts
        };
        assert!(flat.checked().is_err());
        let scale = ScreenFacts {
            backing_scale_factor: f64::NAN,
            ..facts
        };
        assert!(scale.checked().is_err());
        assert!(["macos", "windows", "linux", "other"].contains(&os_name()));
    }
}
