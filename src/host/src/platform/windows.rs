//! Windows: the host's own windows through winit (Slint's backend), the
//! clipboard through arboard, message boxes through rfd, files and links
//! through opener, secrets in the Credential Manager through keyring, system
//! facts from the registry, and the Win32 calls no safe crate wraps
//! (pointer, modifier keys, double-click time, sleep prevention) from
//! the `vestra-win32` crate (the only Windows code allowed to use raw FFI).
//!
//! Status: cross-checked from macOS (`npm run portability`); not yet run on
//! Windows. Differences from macOS are listed in the plugin reference.

use super::{
    Display, HostWindowId, NativeSnapshot, OsFacts, Rect, Result, ScreenFacts, Unavailable,
};
use crate::RpcError;
use crate::clipboard::Image;
use crate::power::{Assertion, Launcher};
use slint::winit_030::{WinitWindowAccessor, winit};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

pub use super::keyring_store::{secret_delete, secret_get, secret_set};

/// Which standard window shortcuts apply (`shortcuts.rs`).
pub const SYSTEM: super::System = super::System::Windows;
/// Slint reports Ctrl as `control` here.
pub const SLINT_CONTROL_IS_COMMAND: bool = false;
/// `Ctrl` is Ctrl; `Cmd` is the Windows (logo) key.
pub const CTRL_ACCEPTS_CMD: bool = false;

// ------------------------------------------------------------------ windows

thread_local! {
    /// Every window the host showed, newest last (UI thread only).
    static WINDOWS: RefCell<Vec<(i64, Weak<winit::window::Window>)>> = const { RefCell::new(Vec::new()) };
    /// `set_floating`, for windows shown later.
    static FLOATING: Cell<bool> = const { Cell::new(false) };
}

fn level(on: bool) -> winit::window::WindowLevel {
    if on {
        winit::window::WindowLevel::AlwaysOnTop
    } else {
        winit::window::WindowLevel::Normal
    }
}

/// Record a window the host just showed. winit has no list of an
/// application's windows, so every `show()` reports here.
pub fn shown(window: &slint::Window) {
    use std::future::Future;
    let future = window.winit_window();
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let std::task::Poll::Ready(Ok(native)) = future.as_mut().poll(&mut context) else {
        eprintln!("platform: a shown window has no winit window yet");
        return;
    };
    let id = u64::from(native.id()) as i64;
    if FLOATING.get() {
        native.set_window_level(level(true));
    }
    WINDOWS.with_borrow_mut(|list| {
        list.retain(|(known, weak)| *known != id && weak.strong_count() > 0);
        list.push((id, Arc::downgrade(&native)));
    });
}

fn live() -> Vec<(i64, Arc<winit::window::Window>)> {
    WINDOWS.with_borrow_mut(|list| {
        list.retain(|(_, weak)| weak.strong_count() > 0);
        list.iter()
            .filter_map(|(id, weak)| Some((*id, weak.upgrade()?)))
            .collect()
    })
}

fn window(id: HostWindowId) -> Result<Arc<winit::window::Window>> {
    live()
        .into_iter()
        .find(|(known, _)| *known == id.0)
        .map(|(_, window)| window)
        .ok_or(Unavailable("associated native window no longer exists"))
}

pub fn capture_windows() -> Result<NativeSnapshot> {
    Ok(NativeSnapshot {
        ids: live().into_iter().map(|(id, _)| id).collect(),
    })
}

pub fn associate_new(before: &NativeSnapshot) -> Result<HostWindowId> {
    let windows = live();
    let mut added = windows
        .iter()
        .filter(|(id, window)| !before.ids.contains(id) && window.is_visible() != Some(false));
    let (first, _) = added
        .next()
        .ok_or(Unavailable("no newly visible native window was created"))?;
    if added.next().is_some() {
        return Err(Unavailable(
            "multiple newly visible native windows prevent unique association",
        ));
    }
    Ok(HostWindowId(*first))
}

/// Every window of the host stays above other applications' windows (or
/// returns to the normal level), including windows shown later.
pub fn set_floating(on: bool) -> Result<usize> {
    FLOATING.set(on);
    let windows = live();
    for (_, window) in &windows {
        window.set_window_level(level(on));
    }
    Ok(windows.len())
}

pub fn window_count() -> usize {
    live().len()
}

/// Resize the client area to `width`×`height` logical points keeping the
/// top-left corner, then move the window back inside its monitor.
pub fn resize_content(id: HostWindowId, width: f64, height: f64) -> Result<()> {
    use winit::dpi::{LogicalSize, PhysicalPosition};
    let window = window(id)?;
    let scale = window.scale_factor();
    let (inner, outer) = (window.inner_size(), window.outer_size());
    let chrome = (
        f64::from(outer.width) - f64::from(inner.width),
        f64::from(outer.height) - f64::from(inner.height),
    );
    let _ = window.request_inner_size(LogicalSize::new(width, height));
    let (Some(monitor), Ok(position)) = (window.current_monitor(), window.outer_position()) else {
        return Ok(());
    };
    let (origin, size) = (monitor.position(), monitor.size());
    let frame = (width * scale + chrome.0, height * scale + chrome.1);
    let fit = |start: i32, length: f64, low: i32, span: u32| {
        let (low, high) = (f64::from(low), f64::from(low) + f64::from(span));
        let mut start = f64::from(start);
        if start + length > high {
            start = high - length;
        }
        start.max(low).round() as i32
    };
    let x = fit(position.x, frame.0, origin.x, size.width);
    let y = fit(position.y, frame.1, origin.y, size.height);
    if (x, y) != (position.x, position.y) {
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
    Ok(())
}

/// The pointer in the window's client area, top-left origin (logical
/// points), also while a file is dragged over it.
pub fn pointer_in(id: HostWindowId) -> Result<(f64, f64)> {
    let window = window(id)?;
    let (x, y) =
        vestra_win32::cursor_position().ok_or(Unavailable("pointer location is unavailable"))?;
    let origin = window
        .inner_position()
        .map_err(|_| Unavailable("window position is unavailable"))?;
    let scale = window.scale_factor();
    Ok((
        f64::from(x - origin.x) / scale,
        f64::from(y - origin.y) / scale,
    ))
}

/// Alt, Shift and the primary modifier (Ctrl here, ⌘ on macOS).
pub fn modifier_keys() -> (bool, bool, bool) {
    vestra_win32::modifier_keys()
}

pub fn double_click_seconds() -> f64 {
    f64::from(vestra_win32::double_click_ms()) / 1000.
}

/// G-15: the system submenu delay (`SPI_GETMENUSHOWDELAY`, 400 ms by default).
pub fn menu_show_delay_ms() -> u64 {
    vestra_win32::menu_show_delay_ms().map_or(crate::menu::SHOW_DELAY_MS, u64::from)
}

pub fn screen_for(id: HostWindowId) -> Result<ScreenFacts> {
    let window = window(id)?;
    let monitor = window
        .current_monitor()
        .ok_or(Unavailable("associated native window has no screen"))?;
    let scale = window.scale_factor();
    let rect = |x: i32, y: i32, width: u32, height: u32| Rect {
        x: f64::from(x) / scale,
        y: f64::from(y) / scale,
        width: f64::from(width) / scale,
        height: f64::from(height) / scale,
    };
    let unknown = |_| Unavailable("native window geometry is unavailable");
    let (outer, inner) = (
        window.outer_position().map_err(unknown)?,
        window.inner_position().map_err(unknown)?,
    );
    let (outer_size, inner_size) = (window.outer_size(), window.inner_size());
    let (origin, size) = (monitor.position(), monitor.size());
    let screen = rect(origin.x, origin.y, size.width, size.height);
    ScreenFacts {
        visible_frame: screen,
        full_frame: screen,
        backing_scale_factor: scale,
        native_frame: rect(outer.x, outer.y, outer_size.width, outer_size.height),
        active: window.has_focus(),
        content_rect: rect(inner.x, inner.y, inner_size.width, inner_size.height),
    }
    .checked()
}

// ---------------------------------------------------------------- clipboard

fn clipboard() -> std::result::Result<arboard::Clipboard, RpcError> {
    arboard::Clipboard::new().map_err(|error| RpcError::new("failed", error.to_string()))
}
pub fn clipboard_read_text() -> std::result::Result<Option<String>, RpcError> {
    match clipboard()?.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(RpcError::new("failed", error.to_string())),
    }
}
pub fn clipboard_write_text(text: &str) -> std::result::Result<(), RpcError> {
    clipboard()?
        .set_text(text)
        .map_err(|error| RpcError::new("failed", error.to_string()))
}
/// Decoded to RGBA: arboard writes it as a device-independent bitmap (and
/// PNG) that other applications paste.
pub fn clipboard_write_image(kind: Image, bytes: Vec<u8>) -> std::result::Result<(), RpcError> {
    let format = match kind {
        Image::Png => image::ImageFormat::Png,
        Image::Jpeg => image::ImageFormat::Jpeg,
    };
    let pixels = image::load_from_memory_with_format(&bytes, format)
        .map_err(|error| RpcError::invalid(format!("image cannot be decoded: {error}")))?
        .to_rgba8();
    let (width, height) = pixels.dimensions();
    clipboard()?
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(pixels.into_raw()),
        })
        .map_err(|error| RpcError::new("failed", error.to_string()))
}

// ------------------------------------------------------------------ dialogs

/// A task dialog owned by the active host window (else `parent`): the
/// message and detail as its text, the plugin's own confirm and cancel labels
/// (rfd's `common-controls-v6`; build.rs embeds the Common Controls v6
/// manifest it needs), a warning icon for destructive actions. A checkbox
/// becomes a second Yes/No dialog after confirming (task dialogs through rfd
/// have no checkbox). `done` runs on a worker thread.
pub fn confirm(
    options: &crate::confirm::Options,
    parent: Option<HostWindowId>,
    done: impl Fn(bool, bool) + Send + 'static,
) -> std::result::Result<(), RpcError> {
    use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult, MessageLevel};
    let owner = live()
        .into_iter()
        .find(|(_, window)| window.has_focus())
        .map(|(_, window)| window)
        .or_else(|| parent.and_then(|id| window(id).ok()));
    let dialog = |text: String, buttons: MessageButtons, level: MessageLevel| {
        let dialog = AsyncMessageDialog::new()
            .set_title("Vestra Player")
            .set_description(text)
            .set_buttons(buttons)
            .set_level(level);
        match &owner {
            Some(window) => dialog.set_parent(window.as_ref()),
            None => dialog,
        }
    };
    let text = match &options.detail {
        Some(detail) => format!("{}\n\n{detail}", options.message),
        None => options.message.clone(),
    };
    let level = if options.destructive {
        MessageLevel::Warning
    } else {
        MessageLevel::Info
    };
    // Built here, on the UI thread that owns the windows; only waiting for
    // the answers happens on the worker.
    let labels = crate::confirm::button_labels(options);
    let first = dialog(
        text,
        MessageButtons::OkCancelCustom(labels.0.clone(), labels.1.clone()),
        level,
    )
    .show();
    let second = options
        .checkbox
        .clone()
        .map(|checkbox| dialog(checkbox, MessageButtons::YesNo, MessageLevel::Info).show());
    std::thread::spawn(move || {
        let confirmed = match pollster::block_on(first) {
            MessageDialogResult::Custom(label) => {
                crate::confirm::confirmed_by(Some(&label), &labels)
            }
            _ => false,
        };
        let checked = confirmed
            && second.is_some_and(|ask| pollster::block_on(ask) == MessageDialogResult::Yes);
        done(confirmed, checked);
    });
    Ok(())
}

// -------------------------------------------------------------------- power

/// The UI thread's execution state; `stop` clears it. It also ends with the
/// thread (and so with the host process).
struct Awake;
impl Assertion for Awake {
    fn stop(&mut self) {
        vestra_win32::allow_sleep();
    }
}

pub fn sleep_launcher() -> Launcher {
    Box::new(|hold| {
        if vestra_win32::keep_awake(hold.display) {
            Ok(Box::new(Awake) as Box<dyn Assertion>)
        } else {
            Err(std::io::Error::other("SetThreadExecutionState refused"))
        }
    })
}

// ---------------------------------------------------------- files and links

fn failed(error: opener::OpenError) -> RpcError {
    RpcError::new("failed", error.to_string())
}

/// Explorer selects one item per window: one window per folder, selecting
/// that folder's first path.
pub fn reveal(paths: &[PathBuf]) -> std::result::Result<(), RpcError> {
    let mut folders: Vec<Option<&Path>> = Vec::new();
    for path in paths {
        let folder = path.parent();
        if !folders.contains(&folder) {
            folders.push(folder);
            opener::reveal(path).map_err(failed)?;
        }
    }
    Ok(())
}
pub fn open_path(path: &Path) -> std::result::Result<(), RpcError> {
    opener::open(path).map_err(failed)
}
pub fn open_url(url: &str) -> std::result::Result<(), RpcError> {
    opener::open_browser(url).map_err(failed)
}

// -------------------------------------------------------------------- facts

/// From `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` (version
/// `23H2`, build `22631.4460`) and the firmware's product name.
pub fn os_facts() -> OsFacts {
    let current = windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok();
    let text = |name: &str| {
        current
            .as_ref()
            .and_then(|key| key.get_string(name).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let build = text("CurrentBuildNumber");
    let revision = current.as_ref().and_then(|key| key.get_u32("UBR").ok());
    let number = build.as_deref().and_then(|b| b.parse::<u32>().ok());
    let model = windows_registry::LOCAL_MACHINE
        .open(r"HARDWARE\DESCRIPTION\System\BIOS")
        .ok()
        .and_then(|key| key.get_string("SystemProductName").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    OsFacts {
        os: "windows",
        name: windows_name(number),
        version: text("DisplayVersion").or_else(|| number.map(|n| format!("10.0.{n}"))),
        build: build.map(|build| match revision {
            Some(revision) => format!("{build}.{revision}"),
            None => build,
        }),
        model,
    }
}

/// winit's monitors (seen from any host window), the primary one first.
/// Names are the device names (`\\.\DISPLAY1`).
pub fn displays() -> Vec<Display> {
    let Some((_, window)) = live().into_iter().next() else {
        return Vec::new();
    };
    let primary = window.primary_monitor();
    let mut list: Vec<Display> = window
        .available_monitors()
        .map(|monitor| {
            let size = monitor.size();
            Display {
                name: monitor.name(),
                width: size.width,
                height: size.height,
                scale: monitor.scale_factor(),
                refresh_rate: monitor
                    .refresh_rate_millihertz()
                    .map(|mhz| f64::from(mhz) / 1000.),
                primary: Some(&monitor) == primary.as_ref(),
            }
        })
        .collect();
    list.sort_by_key(|display| !display.primary);
    list
}

/// Windows 11 still says "Windows 10" in `ProductName`; the build decides.
pub fn windows_name(build: Option<u32>) -> String {
    match build {
        Some(build) if build >= 22000 => "Windows 11".into(),
        Some(_) => "Windows 10".into(),
        None => "Windows".into(),
    }
}

/// `%APPDATA%\Vestra`.
/// Durability barrier for the entries of `directory` (`platform::sync_dir`).
///
/// A rename (`MoveFileExW` / `SetFileInformationByHandle`, which `std::fs::rename`
/// and `tempfile` persist use) is atomic on NTFS, which journals metadata, but
/// it is not durable until the journal reaches the disk: without
/// `MOVEFILE_WRITE_THROUGH` a crash can roll it back. `FlushFileBuffers` on a
/// handle to the directory forces that. A directory opens only with
/// `FILE_FLAG_BACKUP_SEMANTICS`, and the flush needs write access
/// (`GENERIC_WRITE`), so the read-only `File::open` of the Unix version fails
/// with "Access is denied". Plain std (`OpenOptionsExt::custom_flags`), no
/// `unsafe`. Errors are returned, as on Unix: the store treats a failed
/// barrier as "published, durability unknown".
pub fn sync_dir(directory: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    /// `FILE_FLAG_BACKUP_SEMANTICS` (winbase.h).
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory)?
        .sync_all()
}

pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|root| PathBuf::from(root).join("Vestra"))
}

/// FFmpeg's name for its stdin, which is the held source file: the `fd`
/// protocol (FFmpeg 6.1 and later) seeks it like a file; `pipe:` would not.
pub const HELD_FILE_INPUT: &str = "fd:";

/// Console programs (Node, FFmpeg) started by a host without a console of
/// its own (the release build is a GUI-subsystem program, `main.rs`) would
/// each open a console window. `CREATE_NO_WINDOW` gives them a hidden one,
/// which their own children (FFmpeg run by a plugin, the workshop's Node)
/// inherit, so those stay hidden too. A host whose stderr is a console (a
/// debug build run from a terminal) leaves its children on that console, so
/// their output is still seen there.
pub fn hide_console(command: &mut std::process::Command) {
    use std::io::IsTerminal;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !std::io::stderr().is_terminal() {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `.cargo/config.toml`: no VC++ redistributable needed.
    #[test]
    #[cfg(target_env = "msvc")]
    fn the_c_runtime_is_linked_in_statically() {
        assert!(cfg!(target_feature = "crt-static"));
    }

    #[test]
    fn the_build_number_names_the_release() {
        assert_eq!(windows_name(Some(22631)), "Windows 11");
        assert_eq!(windows_name(Some(19045)), "Windows 10");
        assert_eq!(windows_name(None), "Windows");
    }
}
