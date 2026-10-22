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
//! | window shortcuts | `SYSTEM` → `standard_window_shortcut` (the table itself is `shortcuts.rs`, the same on every system) |
//! | windows | `shown`, `capture_windows`, `associate_new`, `set_floating`, `window_count`, `resize_content`, `screen_for`, `pointer_in`, `modifier_keys`, `double_click_seconds`, `menu_show_delay_ms` |
//! | clipboard | `clipboard_read_text`, `clipboard_write_text`, `clipboard_write_image` |
//! | dialogs | `confirm` |
//! | power | `sleep_launcher` |
//! | files and links | `reveal`, `open_path`, `open_url` (blocking, run on a worker thread) |
//! | secrets | `secret_get`, `secret_set`, `secret_delete` (blocking) |
//! | facts | `os_facts` (blocking), `displays` (UI thread), `app_data_dir`, `default_ffmpeg`, `SLINT_CONTROL_IS_COMMAND`, `CTRL_ACCEPTS_CMD` |
//! | generated streams | `av_listen` → `AvListener` / `AvStream` (Unix: `av_unix` below; Windows: `av_tcp`) |
//! | offline media | `HELD_FILE_INPUT` (how FFmpeg names its stdin, the held source file) |
//! | rendering | `GPU_RENDER`, `gpu_start` → `GpuRenderer` / `GpuHandle` / `GpuFrame` (macOS: `vestra-gpu-render`; elsewhere `no_gpu` below refuses, so the host renders in software) |
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

mod shortcuts;
pub use shortcuts::*;

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

/// GPU rendering on systems without an implementation yet (Windows, Linux):
/// `gpu_start` refuses, so the host renders in software and never holds a
/// `GpuHandle`; its methods are inert.
#[cfg(not(target_os = "macos"))]
mod no_gpu {
    use std::sync::{Arc, atomic::AtomicBool};

    pub const GPU_RENDER: bool = false;
    pub struct GpuRenderer(());
    #[derive(Clone)]
    pub struct GpuHandle(());
    /// A published GPU frame; there are none here.
    pub enum GpuFrame {}
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
    impl GpuHandle {
        pub fn set_size(&self, _size: (u32, u32)) {}
        pub fn set_display_fps(&self, _fps: Option<f64>) {}
        pub fn frames(&self) -> u64 {
            0
        }
        pub fn shown(&self) -> u64 {
            0
        }
        pub fn size(&self) -> Option<(u32, u32)> {
            None
        }
        pub fn reshow(&self) {}
        pub fn latest(&self) -> Option<(u64, Box<GpuFrame>)> {
            None
        }
        pub fn on_frame(&self, _notify: impl Fn() + Send + 'static) {}
        pub fn attach<T: slint::ComponentHandle + 'static>(
            &self,
            _component: &T,
            _show: impl Fn(&T, slint::Image) + 'static,
            _failed: impl FnOnce(String) + 'static,
        ) -> Result<(), String> {
            Err("GPU rendering is not available".into())
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub use no_gpu::*;

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
