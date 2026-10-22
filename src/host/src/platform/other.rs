//! Systems without an implementation yet (Linux and the rest): the host
//! builds and plays, and everything system-specific answers `unsupported`
//! (`Unavailable` for window observation, which callers treat the same way).

use super::{
    Display, HostWindowId, NativeSnapshot, OsFacts, Result, ScreenFacts, Unavailable, unsupported,
};
use crate::RpcError;
use crate::clipboard::Image;
use crate::power::Launcher;
use std::path::{Path, PathBuf};

/// Which standard window shortcuts apply (`shortcuts.rs`).
pub const SYSTEM: super::System = super::System::Linux;
/// Slint reports Ctrl as `control` here.
pub const SLINT_CONTROL_IS_COMMAND: bool = false;
/// `Ctrl` is Ctrl; `Cmd` is the Super key.
pub const CTRL_ACCEPTS_CMD: bool = false;

const WINDOWS: Unavailable = Unavailable("native windows are not supported on this system");

pub fn shown(_window: &slint::Window) {}
pub fn capture_windows() -> Result<NativeSnapshot> {
    Err(WINDOWS)
}
pub fn associate_new(_before: &NativeSnapshot) -> Result<HostWindowId> {
    Err(WINDOWS)
}
pub fn set_floating(_on: bool) -> Result<usize> {
    Err(WINDOWS)
}
pub fn window_count() -> usize {
    0
}
pub fn resize_content(_id: HostWindowId, _width: f64, _height: f64) -> Result<()> {
    Err(WINDOWS)
}
pub fn screen_for(_id: HostWindowId) -> Result<ScreenFacts> {
    Err(WINDOWS)
}
pub fn pointer_in(_id: HostWindowId) -> Result<(f64, f64)> {
    Err(WINDOWS)
}
pub fn modifier_keys() -> (bool, bool, bool) {
    (false, false, false)
}
pub fn double_click_seconds() -> f64 {
    0.5
}

/// G-15: no system setting; the Windows default.
pub fn menu_show_delay_ms() -> u64 {
    crate::menu::SHOW_DELAY_MS
}

pub fn clipboard_read_text() -> std::result::Result<Option<String>, RpcError> {
    Err(unsupported("the clipboard"))
}
pub fn clipboard_write_text(_text: &str) -> std::result::Result<(), RpcError> {
    Err(unsupported("the clipboard"))
}
pub fn clipboard_write_image(_kind: Image, _bytes: Vec<u8>) -> std::result::Result<(), RpcError> {
    Err(unsupported("the clipboard"))
}

pub fn confirm(
    _options: &crate::confirm::Options,
    _parent: Option<HostWindowId>,
    _done: impl Fn(bool, bool) + Send + 'static,
) -> std::result::Result<(), RpcError> {
    Err(unsupported("system alerts"))
}

/// Starting fails, so the host logs it once per playback state.
pub fn sleep_launcher() -> Launcher {
    Box::new(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "sleep prevention is not supported on this system",
        ))
    })
}

pub fn reveal(_paths: &[PathBuf]) -> std::result::Result<(), RpcError> {
    Err(unsupported("showing files"))
}
pub fn open_path(_path: &Path) -> std::result::Result<(), RpcError> {
    Err(unsupported("opening files"))
}
pub fn open_url(_url: &str) -> std::result::Result<(), RpcError> {
    Err(unsupported("opening links"))
}

pub fn secret_get(_service: &str, _account: &str) -> std::result::Result<Option<String>, RpcError> {
    Err(unsupported("secret storage"))
}
pub fn secret_set(
    _service: &str,
    _account: &str,
    _value: &str,
) -> std::result::Result<(), RpcError> {
    Err(unsupported("secret storage"))
}
pub fn secret_delete(_service: &str, _account: &str) -> std::result::Result<(), RpcError> {
    Err(unsupported("secret storage"))
}

pub fn os_facts() -> OsFacts {
    OsFacts {
        os: super::os_name(),
        name: std::env::consts::OS.into(),
        version: None,
        build: None,
        model: None,
    }
}
pub fn displays() -> Vec<Display> {
    Vec::new()
}
/// `$XDG_DATA_HOME/vestra`, else `~/.local/share/vestra`.
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|root| root.join("vestra"))
}
/// FFmpeg's name for its stdin, which is the held source file (seekable).
pub const HELD_FILE_INPUT: &str = "/dev/fd/0";

/// Child processes share the host's session; nothing to hide.
pub fn hide_console(_command: &mut std::process::Command) {}
