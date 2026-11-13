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
//! | per-system tables | `SYSTEM` → `standard_window_shortcut` (table in `shortcuts.rs`), `ui_monospace_font` (`monospace_font`), `ui_cjk_fallback_families` / `ui_bundled_cjk_fallback` (table in `fonts.rs`); the tables are the same code on every system; `KEY_LABELS` (how menus write shortcuts), `MENU_SHOWS_CMD` (whether they show `Cmd` bindings) |
//! | windows | `shown`, `set_main_window` / `window_attributes` / `tool_windows_tick` (tool windows stay above the main window: Windows: owned windows, set at creation through Slint's winit hook; macOS: a level above the main window's while the player is active; elsewhere nothing, winit 0.30 has no transient parent), `tool_window_focused` (a tool window has the focus, so a confirmation without an origin (`host/origins.rs`) is that window's and not the main window's: the owner `confirm` picks without one; Windows: winit's `has_focus` over the host's windows; macOS: the key window is one of winit's other than the main window; elsewhere never), `app_in_foreground`, `focus_own_window` (Windows: plain `SetForegroundWindow` through `vestra-win32`; macOS: `NSApp.isActive`, winit focus; elsewhere always in front, winit focus), `capture_windows`, `associate_new`, `set_floating`, `set_dark_title_bars`, `window_count`, `resize_content`, `screen_for`, `pointer_in`, `modifier_keys`, `double_click_seconds`, `system_animations` (whether the user lets the interface animate: Windows' Animation effects through `vestra-win32`, macOS' Reduce motion; elsewhere on) |
//! | clipboard | `clipboard_read_text`, `clipboard_write_text`, `clipboard_write_image` |
//! | dialogs | `confirm` (a tool window's confirmations, owned by the window the request came from (`host/origins.rs`) else the focused one; the main window's are drawn in the player, `host/review_sheet.rs`), `show_open_confirm` (the open confirmation into view when another is refused `busy`: Windows, the owner's last active popup to the front; macOS, the window with the sheet), `SYSTEM_ALERTS` (whether `confirm` shows anything; elsewhere every confirmation is on the sheet) |
//! | power | `sleep_launcher` |
//! | files and links | `reveal`, `open_path`, `open_url` (blocking, run on a worker thread) |
//! | opening the player | `on_open_files` (macOS: open-documents Apple events through `vestra-open-files`; elsewhere files come on the command line), `single_instance` / `single_instance_restarted` → `Launch` / `Primary` (shared, in `instance.rs`), `allow_handoff_foreground` (Windows: `AllowSetForegroundWindow`) |
//! | secrets | `secret_get`, `secret_set`, `secret_delete` (blocking) |
//! | facts | `os_facts` (blocking), `gpu_adapters` (Windows: DXGI through `vestra-win32`; elsewhere none yet), `memory_status` (Windows: private bytes and commit charge through `vestra-win32`; elsewhere none yet), `physical_memory` → `physical_memory_mib` (Windows: `GlobalMemoryStatusEx` through `vestra-win32`; macOS: `NSProcessInfo`; Linux: `/proc/meminfo`; 8 GiB when unknown), `PressureSource` → `MemoryPressure` (the plugin supervisor's input: Windows: thresholds on available physical memory and the system's low-memory notification through `vestra-win32`; Linux: PSI; macOS: none yet), `on_session_end` (Windows: `WM_QUERYENDSESSION`/`WM_ENDSESSION` through `vestra-win32`; elsewhere nothing, the quit request ends the event loop), `displays` (UI thread),`app_data_dir`, `SLINT_CONTROL_IS_COMMAND`, `CTRL_ACCEPTS_CMD`, `preferred_languages` (here, through sys-locale on every system) |
//! | installation | `install_layout` (pure, in `layout.rs`, for every system) → `current_install_layout` |
//! | child processes | `tool_command`, `hide_console` (Windows: `CREATE_NO_WINDOW` when the host has no console; elsewhere nothing) |
//! | storage | `sync_dir` (Unix: `fsync` below; Windows: `FlushFileBuffers` on a directory handle) |
//! | generated streams | `av_listen` → `AvListener` / `AvStream` (Unix: `av_unix` below; Windows: `av_tcp`) |
//! | offline media | `HELD_FILE_INPUT` (how FFmpeg names its stdin, the held source file), `disk_space` (free bytes and volume of a path: Windows through `vestra-win32`; elsewhere `df -Pk`) |
//! | opening windows | `cloak_until_drawn`, `window_drawing` (Windows: each window is cloaked from its show to its first draw, so it never opens white, through `vestra-win32`; elsewhere nothing) |
//! | full screen | `fullscreen_resized` (Windows: a full-screen window is kept one pixel off its display's rectangle; elsewhere nothing) |
//! | video texture | `VIDEO_TEXTURE` (Windows: software-rendered pictures through one reused texture, OpenGL or wgpu, `vestra-video-texture`; elsewhere a new image per frame), `same_video_image` (Windows: also compares wgpu textures; elsewhere `Image` equality) |
//! | redraw pacing | `VBLANK_CLOCK`, `vblank_waiter` → `VBlankWaiter` (Windows: vertical blanks through `vestra-win32`; elsewhere `no_vblank` below: Slint paces, with a display link on macOS) |
//! | rendering | `window_refresh` (here, through winit on every system), `select_wgpu_windows`, `GPU_RENDER`, `GPU_WINDOWS_WGPU`, `GPU_OPT_IN`, `gpu_start` → `GpuRenderer` / `GpuHandle` (implements `playback::renderer::GpuBackend`) / `GpuFrame` / `GpuDraw`, `gpu_started`, `gpu_failed`, `window_device_lost` (macOS: OpenGL windows, `vestra-gpu-render` with OpenGL; Windows: wgpu/Direct3D 12 windows, `vestra-gpu-render` with Direct3D 11 into shared textures; elsewhere `no_gpu` below refuses, so the host renders in software) |
//! | video upscaling | `upscaler_probe` → `UpscalerProbe` (blocking: it wakes a sleeping discrete GPU), `gpu_render_adapter`, `upscaler_filter`, `UPSCALER_HWDEC` (Windows: NVIDIA RTX Video Super Resolution through mpv's `d3d11vpp`, checked through `vestra-win32`; elsewhere `no_upscaler` below: `platform`) |
//! | motion smoothing | `motion_probe` → `MotionProbe` (no device is created), `motion_filter` (Windows: AMD AMF's `frc_amf` through mpv's `lavfi`; elsewhere `no_motion` below: `platform`) |
//! | picture-in-picture | `pip_supported`, `PipWindow` (UI thread; macOS: AVKit through `vestra-pip`; elsewhere `pip_unsupported.rs`: `unsupported`) |
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

/// The bytes free for this user on the volume holding `path` (an existing
/// file or directory) and that volume's identity (its root or mount
/// point), so two paths on one volume are counted together. `None` when
/// the system does not answer (an export then skips its space check).
#[cfg(target_os = "windows")]
pub fn disk_space(path: &std::path::Path) -> Option<(u64, String)> {
    vestra_win32::disk_space(path)
}
#[cfg(not(target_os = "windows"))]
pub fn disk_space(path: &std::path::Path) -> Option<(u64, String)> {
    use std::io::Read;
    // `/bin/df`, not whatever `df` is first on PATH; `--` so a path that
    // starts with `-` is a path. 5 s at most (a hung network mount): then
    // the space is unknown, as when the system does not answer.
    let mut child = std::process::Command::new("/bin/df")
        .arg("-Pk")
        .arg("--")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    if !status.success() {
        return None;
    }
    parse_df(&text)
}

/// POSIX `df -Pk`: a header line, then device, blocks, used, available
/// (1024-byte blocks) and capacity (`NN%`), the mount point last. Read from
/// the capacity column (the `NN%` after three numbers), so a device or
/// mount name with spaces, even one with `NN%` in it, does not shift them.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn parse_df(text: &str) -> Option<(u64, String)> {
    let fields: Vec<&str> = text.lines().nth(1)?.split_whitespace().collect();
    let number = |field: &str| !field.is_empty() && field.bytes().all(|b| b.is_ascii_digit());
    // The capacity is the first `NN%` that follows three numbers (blocks,
    // used, available) after at least one device field.
    let capacity = (4..fields.len()).find(|&i| {
        fields[i].strip_suffix('%').is_some_and(number)
            && fields[i - 3..i].iter().all(|field| number(field))
    })?;
    let available = fields.get(capacity.checked_sub(1)?)?.parse::<u64>().ok()?;
    let mount = fields.get(capacity + 1..)?.join(" ");
    (!mount.is_empty()).then(|| (available.saturating_mul(1024), mount))
}

#[cfg(test)]
mod disk_space_tests {
    use super::parse_df;

    #[test]
    fn df_is_read_from_its_capacity_column() {
        let plain = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s1 971350180 500000000 400000000 56% /\n";
        assert_eq!(parse_df(plain), Some((400_000_000 * 1024, "/".into())));
        // Spaces in the device and the mount point.
        let spaced = "Filesystem 1024-blocks Used Available Capacity Mounted on\nmap auto home 0 0 1234 0% /Volumes/My Disk 50%\n";
        assert_eq!(
            parse_df(spaced),
            Some((1234 * 1024, "/Volumes/My Disk 50%".into()))
        );
        assert_eq!(parse_df("Filesystem\n"), None);
        assert_eq!(parse_df("h\n/dev/x 1 2 3 4 /\n"), None);
    }
}

/// The systems the platform layer has per-system tables for (standard
/// window shortcuts, the monospace font); each platform module names its own
/// as `SYSTEM`. The tables are pure functions of it, so every system's
/// entries are tested on every machine. A build names only its own variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum System {
    MacOs,
    Windows,
    /// Linux and the other desktop systems of `other.rs`.
    Linux,
}

/// How menus write a binding (`bindings::label`); each platform module names
/// its own as `KEY_LABELS`. A build names only its own variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum KeyLabels {
    /// `Ctrl+Shift+S`, `Alt+→`.
    Text,
    /// The macOS menu style: ⌃⌥⇧⌘ in that order without `+` (`⇧⌘S`, `⌥→`),
    /// and a `Ctrl` binding as ⌘ (what it answers to, `CTRL_ACCEPTS_CMD`).
    /// Named keys stay words (`Enter`, `Tab`, `Esc`).
    Symbols,
}

mod shortcuts;
pub use shortcuts::*;

/// The user's preferred interface languages, most preferred first, as BCP 47
/// tags the way the system reports them (`zh-Hans-CN`, `de-DE`, `en_US`):
/// macOS the language list of System Settings, Windows the user's display
/// languages, Linux `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`. sys-locale
/// keeps the per-system calls; `i18n::resolve` picks the locale.
pub fn preferred_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
}

mod layout;
pub use layout::*;

mod fonts;
pub use fonts::*;

mod instance;
pub use instance::*;

/// The installation this executable runs from (`layout.rs`); `None` for a
/// development build.
pub fn current_install_layout() -> Option<InstallLayout> {
    let exe = std::env::current_exe().ok()?;
    install_layout(imp::SYSTEM, &exe, std::path::Path::is_file)
}

/// A command for a console tool the host runs itself (FFmpeg, ffprobe),
/// without a console window (`hide_console`).
pub fn tool_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    hide_console(&mut command);
    command
}

/// The monospace family `Theme.font-mono` names on `system`: one that ships
/// with it (Slint falls back per character for scripts it lacks, CJK
/// included). Android and iOS get theirs when they get a module.
pub fn monospace_font(system: System) -> &'static str {
    match system {
        System::MacOs => "Menlo",
        System::Windows => "Consolas",
        System::Linux => "DejaVu Sans Mono",
    }
}

/// The running system's monospace family (`theme::apply_fonts`).
pub fn ui_monospace_font() -> &'static str {
    monospace_font(imp::SYSTEM)
}

/// The running system's CJK fallback families for `script` in an interface
/// in `locale` (`ui_fonts::install`; table in `fonts.rs`).
pub fn ui_cjk_fallback_families(script: CjkScript, locale: &str) -> Vec<&'static str> {
    cjk_fallback_families(imp::SYSTEM, script, locale)
}

/// Whether the bundled CJK font ends the running system's fallback chains.
pub fn ui_bundled_cjk_fallback() -> bool {
    bundled_cjk_fallback(imp::SYSTEM)
}

/// The running system's standard window shortcut for `press`, if any.
pub fn standard_window_shortcut(press: &KeyPress) -> Option<WindowAction> {
    window_shortcut(imp::SYSTEM, press)
}

// The system picture-in-picture window exists only on macOS so far; Windows
// and Linux will get a borderless always-on-top window of the player's own.
#[cfg(not(target_os = "macos"))]
#[path = "pip_unsupported.rs"]
mod pip_imp;
#[cfg(not(target_os = "macos"))]
pub use pip_imp::*;

/// What happened in the picture-in-picture window (`PipWindow` events).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum PipEvent {
    /// Its play (`false`) or pause (`true`) button.
    Pause(bool),
    /// Its skip buttons: seconds, negative backwards.
    Skip(f64),
    /// The window is showing.
    Started,
    /// The window closed; `restored` through its "back" button.
    Stopped { restored: bool },
    /// The system refused to open it, with its reason.
    Failed(String),
}

/// A native window of this process, for this process's lifetime only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostWindowId(pub(crate) i64);

/// The windows that existed before a new one was shown (`associate_new`).
#[derive(Clone, Debug, Default)]
pub struct NativeSnapshot {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub(crate) ids: BTreeSet<i64>,
}

/// A rectangle in desktop pixels (physical, as the system and winit report
/// window frames and displays).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DesktopRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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

/// A graphics adapter as the driver check sees it (`gpu_adapters`;
/// `playback::render_denylist`), without system-specific concepts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuAdapter {
    pub name: String,
    /// PCI vendor and device (`0x1002`, `0x15BF`).
    pub vendor_id: u32,
    pub device_id: u32,
    /// Dotted, as the system reports it (`31.0.14005.21001`).
    pub driver_version: Option<String>,
    /// No GPU of its own (a software rasterizer or a remote session's).
    pub software: bool,
    /// The system's id of the adapter for this boot (Windows: the LUID;
    /// 0 where unknown). Not stable across restarts: never a guard key.
    pub luid: u64,
}

/// The adapter a mode uses, resolved once at start (`gpu_adapters`) and
/// opened as it is (`select_wgpu_windows`), so the crash guard and the
/// denylist look at the adapter the device is made on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterPick {
    /// Its index in `GpuAdapters::list`; `None`: no adapter (an index or a
    /// preference `VESTRA_GPU_ADAPTER` names that does not exist).
    pub index: Option<usize>,
    /// Its identity for opening it (Windows: the LUID).
    pub luid: Option<u64>,
    /// The software rasterizer (`VESTRA_GPU_ADAPTER=warp`).
    pub warp: bool,
    /// Why it was chosen (the `render:` lines).
    pub how: String,
}

/// The system's graphics adapters and which ones the two modes would use
/// (`gpu_adapters`).
#[derive(Clone, Debug, Default)]
pub struct GpuAdapters {
    /// In the order the system offers them to this process.
    pub list: Vec<GpuAdapter>,
    /// GPU rendering's (Windows: the high-performance GPU unless the user
    /// set a preference for Vestra in Windows Settings, or what
    /// `VESTRA_GPU_ADAPTER` chooses).
    pub gpu: AdapterPick,
    /// CPU mode's Direct3D 12 windows' (Windows' order, or
    /// `VESTRA_GPU_ADAPTER`).
    pub windows: AdapterPick,
}

/// The adapters in the order the system offers them to this process, and
/// which ones GPU rendering and CPU mode's GPU windows would use, before any
/// window or device. Windows: DXGI through `vestra-win32`; elsewhere none
/// yet.
#[cfg(not(target_os = "windows"))]
pub fn gpu_adapters() -> GpuAdapters {
    GpuAdapters::default()
}

/// A GPU on which the system's video upscaler works (`upscaler_probe`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpscalerGpu {
    /// The adapter's identity, as `gpu_render_adapter` reports it.
    pub id: u64,
    pub name: String,
}

/// Why the system's video upscaler cannot run on any GPU: a stable code
/// (`platform`, `no-gpu`, `driver`, `gpu-unsupported`; the API's
/// `UpscalingReasonCode`) and the English text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpscalerRefusal {
    pub code: &'static str,
    pub message: String,
}

/// What `upscaler_probe` found: the GPUs it works on (at least one), or why
/// it works on none.
pub type UpscalerProbe = std::result::Result<Vec<UpscalerGpu>, UpscalerRefusal>;

/// No video upscaler on this system (macOS, Linux): `platform`.
#[cfg(not(target_os = "windows"))]
mod no_upscaler {
    /// Answers at once; nothing is asked of the GPU.
    pub fn upscaler_probe() -> super::UpscalerProbe {
        Err(super::UpscalerRefusal {
            code: "platform",
            message: format!(
                "the player has no video upscaler on {}; it uses NVIDIA RTX Video Super Resolution on Windows",
                crate::playback::render_mode::system_name(std::env::consts::OS)
            ),
        })
    }
    /// The adapter GPU rendering runs on: not known here.
    pub fn gpu_render_adapter() -> Option<(u64, String)> {
        None
    }
    /// No upscaling filter.
    pub fn upscaler_filter(_factor: f64) -> Option<String> {
        None
    }
    /// The hardware decoding the upscaler needs: none.
    pub const UPSCALER_HWDEC: &str = "";
}
#[cfg(not(target_os = "windows"))]
pub use no_upscaler::*;

/// The GPU that converts the frame rate for motion smoothing
/// (`motion_probe`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionGpu {
    pub name: String,
}

/// Why motion smoothing cannot run on this system: a stable code
/// (`platform`, `no-gpu`, `runtime`; the API's `MotionReasonCode`) and the
/// English text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionRefusal {
    pub code: &'static str,
    pub message: String,
}

/// What `motion_probe` found.
pub type MotionProbe = std::result::Result<MotionGpu, MotionRefusal>;

/// No frame-rate converter on this system (macOS, Linux): `platform`.
#[cfg(not(target_os = "windows"))]
mod no_motion {
    pub fn motion_probe() -> super::MotionProbe {
        Err(super::MotionRefusal {
            code: "platform",
            message: format!(
                "the player has no frame-rate converter on {}; motion smoothing uses AMD's AMF on Windows",
                crate::playback::render_mode::system_name(std::env::consts::OS)
            ),
        })
    }
    /// No motion smoothing filter.
    pub fn motion_filter(_large: bool, _performance: bool) -> Option<String> {
        None
    }
}
#[cfg(not(target_os = "windows"))]
pub use no_motion::*;

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

/// The refresh rate of the display `window` is on (the frame log), from
/// winit's monitor of the window: `None` without one or without a rate.
pub fn window_refresh(window: &slint::Window) -> Option<f64> {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|window| window.current_monitor()?.refresh_rate_millihertz())
        .flatten()
        .map(|mhz| f64::from(mhz) / 1000.)
        .filter(|hz| *hz >= 1.)
}

/// The refusal for something this system's implementation does not do.
#[cfg_attr(any(target_os = "macos", target_os = "windows"), allow(dead_code))]
fn unsupported(what: &str) -> crate::host::RpcError {
    crate::host::RpcError::new(
        "unsupported",
        format!("{what} is not supported on {}", std::env::consts::OS),
    )
}

/// GPU rendering on systems without an implementation yet (Linux and the
/// rest): `gpu_start` refuses, so the host renders in software and never
/// holds a `GpuHandle`; its `GpuBackend` methods are inert.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod no_gpu {
    use std::sync::{Arc, atomic::AtomicBool};

    pub const GPU_RENDER: bool = false;
    pub const GPU_WINDOWS_WGPU: bool = false;
    pub const GPU_OPT_IN: bool = false;
    pub struct GpuRenderer(());
    #[derive(Clone)]
    pub struct GpuHandle(());
    /// A published GPU frame; there are none here.
    pub enum GpuFrame {}
    /// A draw of a window showing GPU frames (as on macOS); never sent here.
    #[allow(dead_code)]
    pub enum GpuDraw {
        Begin,
        End { copied: bool },
    }
    pub fn gpu_start(
        _mpv: Arc<rsmpv::Mpv>,
        _size: (u32, u32),
        _publish: Arc<AtomicBool>,
    ) -> Result<GpuRenderer, String> {
        Err(format!(
            "GPU rendering is not available on {} yet",
            std::env::consts::OS
        ))
    }
    impl GpuRenderer {
        pub fn handle(&self) -> GpuHandle {
            GpuHandle(())
        }
    }
    impl crate::playback::renderer::GpuBackend for GpuHandle {
        type Frame = Box<GpuFrame>;
        fn set_size(&self, _size: (u32, u32)) {}
        fn set_display_fps(&self, _fps: Option<f64>) {}
        fn frames(&self) -> u64 {
            0
        }
        fn shown(&self) -> u64 {
            0
        }
        fn size(&self) -> Option<(u32, u32)> {
            None
        }
        fn reshow(&self) {}
        fn latest(&self) -> Option<(u64, Box<GpuFrame>)> {
            None
        }
        fn on_frame(&self, _notify: impl Fn() + Send + 'static) {}
        fn attach<T: slint::ComponentHandle + 'static>(
            &self,
            _component: &T,
            _show: impl Fn(slint::Image) + 'static,
            _failed: impl FnOnce(String) + 'static,
            _drawn: impl FnMut(GpuDraw) + 'static,
        ) -> Result<(), String> {
            Err("GPU rendering is not available".into())
        }
        fn retarget<T: slint::ComponentHandle + 'static>(&self, _component: &T) {}
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use no_gpu::*;

/// GPU mode's wgpu windows (`render_mode::WindowKind::Wgpu`): Windows only
/// (`windows.rs`); `decide` never asks for them here (`GPU_WINDOWS_WGPU`).
#[cfg(not(target_os = "windows"))]
pub fn select_wgpu_windows(
    _winit: impl Fn() -> slint::BackendSelector,
    _adapter: &AdapterPick,
) -> std::result::Result<(), String> {
    Err(format!("no wgpu windows on {}", std::env::consts::OS))
}

/// The GPU renderer started: route its diagnostics (Windows: `render:`
/// lines of its device and failures; nothing here).
#[cfg(not(target_os = "windows"))]
pub fn gpu_started(_handle: &GpuHandle) {}

/// Whether the windows' GPU device was lost (Windows; never here).
#[cfg(not(target_os = "windows"))]
pub fn window_device_lost() -> bool {
    false
}

/// The calling thread's CPU cycles (Windows only; 0 here).
#[cfg(not(target_os = "windows"))]
pub fn thread_cycles() -> u64 {
    0
}

/// The host log's memory line (`diag::memory_line`): this process's
/// private bytes and the system commit charge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryStatus {
    pub private_bytes: u64,
    pub commit_used: u64,
    pub commit_limit: u64,
    /// Physical memory in use, percent, where the system says.
    pub physical_load: Option<u32>,
}

/// What `on_session_end` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub enum SessionEvent {
    /// The system asks whether the session may end.
    Query,
    /// It ends; the process may be ended any time after.
    Ending,
    /// The end was cancelled.
    Cancelled,
}

/// The session ends (Windows: `WM_QUERYENDSESSION`/`WM_ENDSESSION` through
/// `vestra-win32`). macOS: a logout or shutdown asks the app to quit, which
/// ends the event loop like Quit, so nothing more is needed here.
#[cfg(not(target_os = "windows"))]
pub fn on_session_end(_ended: impl Fn(SessionEvent) + 'static) {}

/// Windows only (`vestra-win32`); macOS would need the Mach task calls in
/// an unsafe crate, so no memory line there yet.
#[cfg(not(target_os = "windows"))]
pub fn memory_status() -> Option<MemoryStatus> {
    None
}

/// Memory pressure as the system reports it: the plugin supervisor's input
/// (doc/architecture/supervisor.md §8, "Memory pressure").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressure {
    #[default]
    None,
    Low,
    Critical,
}

/// The physical memory the supervisor's process budget assumes when the
/// system does not say: 8 GiB (supervisor.md §8).
pub const PHYSICAL_MIB_FALLBACK: u64 = 8 * 1024;

/// Installed physical memory in MiB (Windows: `GlobalMemoryStatusEx`
/// through `vestra-win32`; macOS: `NSProcessInfo.physicalMemory`; Linux:
/// `/proc/meminfo`), or [`PHYSICAL_MIB_FALLBACK`].
pub fn physical_memory_mib() -> u64 {
    imp::physical_memory()
        .map(|bytes| bytes >> 20)
        .filter(|mib| *mib > 0)
        .unwrap_or(PHYSICAL_MIB_FALLBACK)
}

/// The Windows memory pressure thresholds, on available physical
/// memory: below 4 GiB installed, moderate under 500 MiB available and
/// critical under 200 MiB; from 4 GiB, 1000 MiB and 400 MiB.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn pressure_from_available(total_mib: u64, available_mib: u64) -> MemoryPressure {
    let (moderate, critical) = if total_mib >= 4096 {
        (1000, 400)
    } else {
        (500, 200)
    };
    if available_mib < critical {
        MemoryPressure::Critical
    } else if available_mib < moderate {
        MemoryPressure::Low
    } else {
        MemoryPressure::None
    }
}

/// `/proc/pressure/memory` (PSI) as a level, with common default stall
/// thresholds over the last 10 s: low
/// when some task stalled on memory ≥ 20 % of the time, critical when all
/// did ≥ 70 %. `None` for a text that is not
/// PSI.
#[cfg_attr(all(any(windows, target_os = "macos"), not(test)), allow(dead_code))]
fn psi_pressure(text: &str) -> Option<MemoryPressure> {
    let avg10 = |kind: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(kind)?.strip_prefix(' '))?
            .split_whitespace()
            .find_map(|field| field.strip_prefix("avg10="))?
            .parse::<f64>()
            .ok()
    };
    let some = avg10("some")?;
    let full = avg10("full").unwrap_or(0.);
    Some(if full >= 70. {
        MemoryPressure::Critical
    } else if some >= 20. {
        MemoryPressure::Low
    } else {
        MemoryPressure::None
    })
}

/// `MemTotal` of `/proc/meminfo`, in bytes.
#[cfg_attr(all(any(windows, target_os = "macos"), not(test)), allow(dead_code))]
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let kib = line.split_whitespace().next()?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

/// The main window starts a draw (Windows only).
#[cfg(not(target_os = "windows"))]
pub fn ui_draw_started() {}

/// The main window's draw went to the GPU (Windows only).
#[cfg(not(target_os = "windows"))]
pub fn ui_draw_submitted() {}

/// The main window's draws' GPU completion (Windows only; "" here).
#[cfg(not(target_os = "windows"))]
pub fn ui_gpu_text() -> String {
    String::new()
}

/// What to do when the windows' GPU device is lost (Windows only).
#[cfg(not(target_os = "windows"))]
pub fn on_window_device_lost(_hook: impl Fn() + Send + Sync + 'static) {}

/// Whether windows may show video through a rendering notifier (always
/// here; Windows: not with wgpu windows, see `windows.rs`).
#[cfg(not(target_os = "windows"))]
pub fn window_notifiers() -> bool {
    true
}

/// The software rendering texture of a window without a notifier (Windows
/// only).
#[cfg(not(target_os = "windows"))]
pub fn direct_video_feed() -> Option<vestra_video_texture::Feed> {
    None
}

/// Whether two video area images are the same picture source (Windows also
/// compares wgpu textures, which Slint's `Image` equality does not).
#[cfg(not(target_os = "windows"))]
pub fn same_video_image(shown: &slint::Image, next: &slint::Image) -> bool {
    shown == next
}

/// Pace GPU frames of windows without a notifier (Windows only).
#[cfg(not(target_os = "windows"))]
pub fn gate_gpu_frames(_accept: Option<Box<dyn Fn() -> bool>>) {}

/// The player quits: release the GPU renderer's waits (Windows only; the
/// macOS renderer has no cross-API fences).
#[cfg(not(target_os = "windows"))]
pub fn release_gpu_waits() {}

/// Take a GPU frame the pacing refused (Windows only).
#[cfg(not(target_os = "windows"))]
pub fn update_gpu_frames() {}

/// Wait for process `pid` to end (a restarted player and the one it
/// replaces): `true` once it has (Windows; restarts happen only there).
#[cfg(not(target_os = "windows"))]
pub fn wait_for_process_exit(_pid: u32, _timeout: std::time::Duration) -> bool {
    true
}

/// End this process now with `code`, running nothing more of it (a
/// teardown stuck after a device loss; Windows terminates without DLL
/// detach, restarts happen only there).
#[cfg(not(target_os = "windows"))]
pub fn exit_now(code: u32) -> ! {
    std::process::exit(code as i32)
}

/// Why the GPU renderer stopped on its own (Windows; never here).
#[cfg(not(target_os = "windows"))]
pub fn gpu_failed(_handle: &GpuHandle) -> Option<String> {
    None
}

/// Redraw pacing where Slint already ties redraws to the display (macOS
/// and iOS: a display link) or no display clock exists yet (Linux): the
/// host does not pace (`frame_pacing.rs`); the Windows version is in
/// `windows.rs`.
#[cfg(not(target_os = "windows"))]
mod no_vblank {
    /// The host does not pace redraws on this system.
    pub const VBLANK_CLOCK: bool = false;
    /// Never made here (`vblank_waiter` answers `None`).
    #[allow(dead_code)]
    pub struct VBlankWaiter(());
    impl VBlankWaiter {
        pub fn wait(&mut self, _moved: bool) -> bool {
            false
        }
    }
    pub fn vblank_waiter(_window: &slint::Window) -> Option<VBlankWaiter> {
        None
    }
}
#[cfg(not(target_os = "windows"))]
pub use no_vblank::*;

/// Software rendering shows each picture as a new image here (femtovg makes
/// a texture per frame). The reused texture of `vestra-video-texture` is
/// plain OpenGL, but so far it was measured and checked only on Windows;
/// the Windows version is in `windows.rs`.
#[cfg(not(target_os = "windows"))]
pub const VIDEO_TEXTURE: bool = false;

/// After a resize of a window: nothing to adjust for full screen here; the
/// Windows version (a full-screen window must not match its display exactly)
/// is in `windows.rs`.
#[cfg(not(target_os = "windows"))]
pub fn fullscreen_resized(_window: &slint::Window) {}

/// Windows hides each window until its first draw (no white frame when it
/// opens; `windows.rs`). Elsewhere nothing: Slint's winit backend already
/// reveals a macOS window at its first frame.
#[cfg(not(target_os = "windows"))]
pub fn cloak_until_drawn() {}

/// A window is about to draw (winit `RedrawRequested` let through): nothing
/// here; on Windows a window hidden until its first draw shows after it.
#[cfg(not(target_os = "windows"))]
pub fn window_drawing(_id: u64) {}

/// The displays' work areas (without the taskbar or the Dock), the primary
/// one marked: only Windows lists them (`windows.rs`); elsewhere none, and
/// a restored frame is checked against winit's displays.
#[cfg(not(target_os = "windows"))]
pub fn work_areas() -> Vec<(DesktopRect, bool)> {
    Vec::new()
}

/// The main window's normal frame in desktop pixels (also while it is
/// maximized): only Windows saves it (`windows.rs`, the window placement);
/// elsewhere the logical frame is saved.
#[cfg(not(target_os = "windows"))]
pub fn normal_frame(_window: &slint::Window) -> Option<DesktopRect> {
    None
}

/// Where to put a window before its native window exists so that its
/// frame's top-left corner lands at `frame`: macOS moves it down by the
/// title bar (`macos.rs`); unchanged elsewhere (Windows takes it as the
/// frame's).
#[cfg(not(target_os = "macos"))]
pub fn creation_position(frame: slint::LogicalPosition) -> slint::LogicalPosition {
    frame
}

/// Put `window` at the normal frame `frame` (desktop pixels), maximized
/// over it when `maximized`: Windows only (`windows.rs`); false elsewhere.
#[cfg(not(target_os = "windows"))]
pub fn set_normal_frame(_window: &slint::Window, _frame: DesktopRect, _maximized: bool) -> bool {
    false
}

/// Keep the next window shown hidden past its first draw, until
/// [`placed`]: Windows only (`windows.rs`, the main window kept cloaked
/// until its restored frame and maximized state are set); nothing here.
#[cfg(not(target_os = "windows"))]
pub fn hold_next_shown() {}

/// `window` has its restored frame: show it at its next draw (Windows,
/// `windows.rs`); nothing here.
#[cfg(not(target_os = "windows"))]
pub fn placed(_window: &slint::Window) {}

/// Durability barrier for the entries of `directory`: after it returns, the
/// names created, renamed or removed in it survive a crash (the package
/// store calls it after each atomic rename). Unix: `fsync` of the directory.
/// The Windows version is in `windows.rs`.
#[cfg(unix)]
pub fn sync_dir(directory: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(directory)?.sync_all()
}

/// `openStream`'s local transport on Unix (macOS, Linux): a Unix socket in a
/// fresh private directory under `/tmp` (macOS limits socket paths to 104
/// bytes). The answer to `producer.prepare` carries its path as `socket`.
#[cfg(unix)]
mod av_unix {
    use std::io::{self, Read};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::Duration;

    pub struct AvListener {
        listener: UnixListener,
        _directory: tempfile::TempDir,
    }
    pub struct AvStream(UnixStream);
    pub fn av_listen() -> io::Result<(AvListener, serde_json::Value)> {
        let directory = tempfile::Builder::new()
            .prefix("vestra-av-")
            .tempdir_in("/tmp")?;
        let path = directory.path().join("av.sock");
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let address = serde_json::json!({ "socket": path });
        Ok((
            AvListener {
                listener,
                _directory: directory,
            },
            address,
        ))
    }
    impl AvListener {
        /// The plugin's connection, `None` while it has not connected.
        pub fn accept(&self) -> io::Result<Option<AvStream>> {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    // macOS hands out accepted sockets non-blocking like the
                    // listener; reads use a timeout instead.
                    stream.set_nonblocking(false)?;
                    Ok(Some(AvStream(stream)))
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(error) => Err(error),
            }
        }
    }
    impl AvStream {
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.0.set_read_timeout(timeout)
        }
    }
    impl Read for AvStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0.read(buffer)
        }
    }
}
#[cfg(unix)]
pub use av_unix::*;

/// `openStream`'s local transport where there are no Unix sockets for Node
/// (Windows): TCP on the loopback interface. Any local process can connect to
/// a loopback port, so the plugin first sends the 32-byte `token` from the
/// `producer.prepare` answer; connections without it are dropped. Plain std,
/// so its tests run everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod av_tcp {
    use std::io::{self, Read};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::time::Duration;

    pub const TOKEN_BYTES: usize = 32;
    pub struct AvListener {
        listener: TcpListener,
        token: [u8; TOKEN_BYTES],
    }
    pub struct AvStream(TcpStream);
    /// `{ socket: "tcp://127.0.0.1:<port>", token: "<64 hex digits>" }`.
    pub fn listen(token: [u8; TOKEN_BYTES]) -> io::Result<(AvListener, serde_json::Value)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let hex: String = token.iter().map(|byte| format!("{byte:02x}")).collect();
        let address = serde_json::json!({
            "socket": format!("tcp://127.0.0.1:{port}"),
            "token": hex,
        });
        Ok((AvListener { listener, token }, address))
    }
    impl AvListener {
        /// The plugin's connection once it sent the token; `None` while
        /// nobody connected, or for a connection without the token.
        pub fn accept(&self) -> io::Result<Option<AvStream>> {
            let mut stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            };
            // Accepted sockets inherit the listener's non-blocking mode on Windows.
            stream.set_nonblocking(false)?;
            stream.set_nodelay(true)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut token = [0; TOKEN_BYTES];
            if stream.read_exact(&mut token).is_err() || token != self.token {
                return Ok(None);
            }
            Ok(Some(AvStream(stream)))
        }
    }
    impl AvStream {
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.0.set_read_timeout(timeout)
        }
    }
    impl Read for AvStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0.read(buffer)
        }
    }
}
#[cfg(windows)]
pub use av_tcp::{AvListener, AvStream};
#[cfg(windows)]
pub fn av_listen() -> std::io::Result<(AvListener, serde_json::Value)> {
    let mut token = [0; av_tcp::TOKEN_BYTES];
    getrandom::fill(&mut token).map_err(|error| std::io::Error::other(error.to_string()))?;
    av_tcp::listen(token)
}

/// `secrets.*` on systems with a keyring backend: the login keychain
/// (generic passwords) on macOS, the Credential Manager on Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod keyring_store {
    use crate::host::RpcError;

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
    /// The thresholds on available physical memory, for machines
    /// below and from 4 GiB.
    #[test]
    fn available_memory_maps_to_chromes_levels() {
        use super::{MemoryPressure as M, pressure_from_available as level};
        assert_eq!(level(16_384, 4_000), M::None);
        assert_eq!(level(16_384, 999), M::Low);
        assert_eq!(level(16_384, 400), M::Low);
        assert_eq!(level(16_384, 399), M::Critical);
        assert_eq!(level(4_096, 1_000), M::None);
        assert_eq!(level(2_048, 999), M::None);
        assert_eq!(level(2_048, 499), M::Low);
        assert_eq!(level(2_048, 199), M::Critical);
    }

    /// PSI lines as the kernel writes them; "full" may be missing (older
    /// kernels); anything else is not PSI.
    #[test]
    fn psi_maps_stalls_to_levels() {
        use super::{MemoryPressure as M, psi_pressure as level};
        let psi = |some: f64, full: f64| {
            format!(
                "some avg10={some:.2} avg60=0.00 avg300=0.00 total=12
full avg10={full:.2} avg60=0.00 avg300=0.00 total=3
"
            )
        };
        assert_eq!(level(&psi(0., 0.)), Some(M::None));
        assert_eq!(level(&psi(19.99, 5.)), Some(M::None));
        assert_eq!(level(&psi(20., 5.)), Some(M::Low));
        assert_eq!(level(&psi(85., 70.)), Some(M::Critical));
        assert_eq!(
            level("some avg10=31.00 avg60=1.00 avg300=0.10 total=5"),
            Some(M::Low)
        );
        assert_eq!(level(""), None);
        assert_eq!(level("some avg60=1.00"), None);
    }

    #[test]
    fn meminfo_total_is_read_in_bytes() {
        let text = "MemTotal:       16303428 kB
MemFree:         1234 kB
";
        assert_eq!(super::meminfo_total(text), Some(16_303_428 * 1024));
        assert_eq!(super::meminfo_total("MemFree: 1 kB"), None);
    }

    /// The process budget's input is never 0: the system's figure, or
    /// 8 GiB.
    #[test]
    fn physical_memory_is_known_or_the_fallback() {
        assert!(super::physical_memory_mib() > 0);
        assert_eq!(super::PHYSICAL_MIB_FALLBACK, 8_192);
    }

    use super::*;

    #[test]
    fn each_system_names_a_monospace_font_it_ships() {
        assert_eq!(monospace_font(System::MacOs), "Menlo");
        assert_eq!(monospace_font(System::Windows), "Consolas");
        assert_eq!(monospace_font(System::Linux), "DejaVu Sans Mono");
        // The palette's default is the macOS one (the SDK copy shows it).
        let palette = include_str!("../../ui/palette.slint");
        assert!(palette.contains("in property <string> font-mono: \"Menlo\";"));
    }

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

    #[cfg(unix)]
    #[test]
    fn the_unix_stream_transport_hands_over_a_blocking_connection_with_timeouts() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};
        let (listener, address) = av_listen().unwrap();
        let path = std::path::PathBuf::from(address["socket"].as_str().unwrap());
        assert!(path.starts_with("/tmp") && address.get("token").is_none());
        assert!(listener.accept().unwrap().is_none(), "nobody yet");
        let mut plugin = UnixStream::connect(&path).unwrap();
        let mut stream = loop {
            if let Some(stream) = listener.accept().unwrap() {
                break stream;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        // No data: the read waits for the timeout instead of returning at once.
        let start = Instant::now();
        let mut byte = [0; 1];
        assert!(stream.read(&mut byte).is_err());
        assert!(
            start.elapsed() >= Duration::from_millis(40),
            "spinning read"
        );
        plugin.write_all(b"V").unwrap();
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        drop(listener);
        assert!(
            !path.exists(),
            "the private directory goes with the listener"
        );
    }

    #[test]
    fn the_loopback_stream_transport_admits_only_the_token() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let token = [7; av_tcp::TOKEN_BYTES];
        let (listener, address) = av_tcp::listen(token).unwrap();
        assert_eq!(address["token"].as_str().unwrap(), "07".repeat(32));
        let port = address["socket"]
            .as_str()
            .and_then(|s| s.strip_prefix("tcp://127.0.0.1:"))
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap();
        let accept = || loop {
            match listener.accept() {
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
                other => return other,
            }
        };
        assert!(listener.accept().unwrap().is_none(), "nobody yet");
        let mut stranger = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stranger.write_all(&[0; av_tcp::TOKEN_BYTES]).unwrap();
        // The stranger is read and dropped: the next accept sees nothing.
        let first = std::time::Instant::now();
        while first.elapsed() < std::time::Duration::from_millis(200) {
            assert!(listener.accept().unwrap().is_none(), "wrong token admitted");
        }
        let mut plugin = TcpStream::connect(("127.0.0.1", port)).unwrap();
        plugin.write_all(&token).unwrap();
        plugin.write_all(b"VAV1").unwrap();
        let mut stream = accept().unwrap().expect("the token's connection");
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(&header, b"VAV1");
    }
}
