//! Systems without an implementation yet (Linux and the rest): the host
//! builds and plays, and everything system-specific answers `unsupported`
//! (`Unavailable` for window observation, which callers treat the same way).

use super::{
    Display, HostWindowId, NativeSnapshot, OsFacts, Result, ScreenFacts, Unavailable, unsupported,
};
use crate::clipboard::Image;
use crate::host::RpcError;
use crate::power::Launcher;
use std::path::{Path, PathBuf};

/// How menus write shortcuts (`bindings::label`).
pub const KEY_LABELS: super::KeyLabels = super::KeyLabels::Text;
/// Which standard window shortcuts apply (`shortcuts.rs`).
pub const SYSTEM: super::System = super::System::Linux;
/// Slint reports Ctrl as `control` here.
pub const SLINT_CONTROL_IS_COMMAND: bool = false;
/// `Ctrl` is Ctrl; `Cmd` is the Super key.
pub const CTRL_ACCEPTS_CMD: bool = false;
/// Menus show no `Cmd` binding: the desktop takes most Super-key
/// combinations (`bindings::label_shown`).
pub const MENU_SHOWS_CMD: bool = false;
/// The fullscreen title strip, as on Windows: a fullscreen window has no
/// title bar of its own.
pub const FULLSCREEN_TITLE_STRIP: bool = true;

const WINDOWS: Unavailable = Unavailable("native windows are not supported on this system");

pub fn shown(_instance: &slint_interpreter::ComponentInstance) {}
/// Tool windows are not tied to the main window here: winit 0.30 has no
/// transient-for (X11 `WM_TRANSIENT_FOR`, Wayland `xdg_toplevel.set_parent`)
/// for top-level windows, so the window manager stacks them as ordinary windows.
pub fn set_main_window(_instance: &slint_interpreter::ComponentInstance) {}
/// Windows are created as Slint asks (see [`set_main_window`]).
pub fn window_attributes(
    attributes: slint::winit_030::winit::window::WindowAttributes,
) -> slint::winit_030::winit::window::WindowAttributes {
    attributes
}
pub fn tool_windows_tick() {}
/// Not known here: the window manager decides (its focus-stealing
/// prevention applies to `focus_own_window`).
pub fn app_in_foreground() -> bool {
    true
}
/// Minimize `window` (Slint's own minimize: no winit fullscreen quirk here).
pub fn minimize_window(window: &slint::Window) {
    window.set_minimized(true);
}

/// winit's `focus_window` (a request the window manager may refuse).
pub fn focus_own_window(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|native| native.focus_window());
}
pub fn capture_windows() -> Result<NativeSnapshot> {
    Err(WINDOWS)
}
pub fn associate_new(_before: &NativeSnapshot) -> Result<HostWindowId> {
    Err(WINDOWS)
}
/// Title bars stay as the window system draws them.
pub fn set_dark_title_bars(_dark: bool) {}
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
    _options: &crate::ui::confirm::Options,
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

/// Files come on the command line (`launch.rs`); no event.
pub fn on_open_files(_deliver: impl Fn(Vec<PathBuf>) + 'static) {}
pub fn allow_handoff_foreground() {}

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
/// Installed physical memory, in bytes (`MemTotal` of `/proc/meminfo`;
/// `None` where there is none).
pub fn physical_memory() -> Option<u64> {
    super::meminfo_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}
/// Memory pressure from PSI (`/proc/pressure/memory`; supervisor.md §8). `None` without PSI (kernels before 4.20,
/// `psi=0`, other systems).
#[derive(Debug, Default)]
pub struct PressureSource;
impl PressureSource {
    pub fn new() -> Self {
        Self
    }
    /// The level now; `None` when the system does not say.
    pub fn level(&mut self) -> Option<super::MemoryPressure> {
        super::psi_pressure(&std::fs::read_to_string("/proc/pressure/memory").ok()?)
    }
}
/// `$XDG_DATA_HOME/vestra`, else `~/.local/share/vestra`.
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|root| root.join("vestra"))
}
/// `$XDG_CACHE_HOME/vestra`, else `~/.cache/vestra`: data the player can
/// make again (mpv's compiled shaders).
pub fn app_cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|root| root.join("vestra"))
}
/// FFmpeg's name for its stdin, which is the held source file (seekable).
pub const HELD_FILE_INPUT: &str = "/dev/fd/0";

/// Child processes share the host's session; nothing to hide.
pub fn hide_console(_command: &mut std::process::Command) {}

/// A console tool doing background work (an export's FFmpeg), started
/// through `nice -n 10` so playback and the UI keep their rate while it runs
/// (as HandBrake and OBS run their encoders); directly when `nice` is missing.
pub fn background_tool_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let nice = std::path::Path::new("/usr/bin/nice");
    if nice.is_file() {
        let mut command = std::process::Command::new(nice);
        command.args(["-n", "10"]).arg(program);
        command
    } else {
        std::process::Command::new(program)
    }
}
