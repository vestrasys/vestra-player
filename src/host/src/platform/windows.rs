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
use crate::clipboard::Image;
use crate::host::RpcError;
use crate::power::{Assertion, Launcher};
use slint::winit_030::{WinitWindowAccessor, winit};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

pub use super::keyring_store::{secret_delete, secret_get, secret_set};

/// How menus write shortcuts (`bindings::label`).
pub const KEY_LABELS: super::KeyLabels = super::KeyLabels::Text;
/// Which standard window shortcuts apply (`shortcuts.rs`).
pub const SYSTEM: super::System = super::System::Windows;
/// Slint reports Ctrl as `control` here.
pub const SLINT_CONTROL_IS_COMMAND: bool = false;
/// `Ctrl` is Ctrl; `Cmd` is the Windows (logo) key.
pub const CTRL_ACCEPTS_CMD: bool = false;
/// Menus show no `Cmd` binding: the system takes the Windows key's
/// combinations (Win+Q opens search), and a key named "Cmd" means nothing
/// here. Alt+F4 and the close button quit (`bindings::label_shown`).
pub const MENU_SHOWS_CMD: bool = false;

// ------------------------------------------------------------------ windows

thread_local! {
    /// Every window the host showed, newest last (UI thread only).
    static WINDOWS: RefCell<Vec<(i64, Weak<winit::window::Window>)>> = const { RefCell::new(Vec::new()) };
    /// `set_floating`, for windows shown later.
    static FLOATING: Cell<bool> = const { Cell::new(false) };
    /// `set_dark_title_bars`, for windows shown later; the default palette is dark.
    static DARK: Cell<bool> = const { Cell::new(true) };
}

/// One window's title bar dark or light (DWMWA_USE_IMMERSIVE_DARK_MODE).
fn title_bar(window: &winit::window::Window, dark: bool) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    if let RawWindowHandle::Win32(handle) = handle.as_raw()
        && !vestra_win32::set_dark_title_bar(handle.hwnd.get(), dark)
    {
        diag!("platform: the system refused a dark/light title bar");
    }
}

/// The executable's icon group (`windows/resources.rs` writes `windows/vestra.ico`,
/// 16 to 256 px, as group 1): the logo Explorer and the taskbar show.
const ICON_GROUP: u16 = 1;

/// The player's icon at `logical` px on a display of `scale`, loaded from
/// the executable's icon group (the system picks the closest of its sizes).
fn icon(
    logical: f64,
    scale: f64,
) -> std::result::Result<winit::window::Icon, winit::window::BadIcon> {
    use winit::platform::windows::IconExtWindows;
    let side = (logical * scale).round().max(1.) as u32;
    winit::window::Icon::from_resource(ICON_GROUP, Some(winit::dpi::PhysicalSize::new(side, side)))
}

/// The player's icon on a window: the small one (16 px logical: title bar)
/// and the large one (32 px: Alt+Tab, the taskbar button), at the window's
/// scale (1.75: 28 and 56 px). winit's windows have none of their own, so
/// every host window (main, floating sidebar, options, plugin windows) gets
/// it here (`shown`). The taskbar groups by the executable, not the icon.
fn set_icons(window: &winit::window::Window) {
    use winit::platform::windows::WindowExtWindows;
    let scale = window.scale_factor();
    match (icon(16., scale), icon(32., scale)) {
        (Ok(small), Ok(large)) => {
            window.set_window_icon(Some(small));
            window.set_taskbar_icon(Some(large));
            diag!("ui: window icon set for {:?}", window.title());
        }
        (Err(error), _) | (_, Err(error)) => {
            diag!("ui: no window icon for {:?}: {error}", window.title());
        }
    }
}

fn level(on: bool) -> winit::window::WindowLevel {
    if on {
        winit::window::WindowLevel::AlwaysOnTop
    } else {
        winit::window::WindowLevel::Normal
    }
}

/// The winit window behind `window`, if it exists yet.
fn native_of(window: &slint::Window) -> Option<Arc<winit::window::Window>> {
    use std::future::Future;
    let future = window.winit_window();
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(Ok(native)) => Some(native),
        _ => None,
    }
}

/// How long `shown` waits for a window's native window (ticks of
/// [`PENDING_TICK`]): 5 s.
const PENDING_TRIES: u32 = 300;
const PENDING_TICK: std::time::Duration = std::time::Duration::from_millis(16);

thread_local! {
    /// Shown windows whose native window does not exist yet, and the tries left.
    static PENDING: RefCell<Vec<(slint::Weak<slint_interpreter::ComponentInstance>, u32)>> =
        const { RefCell::new(Vec::new()) };
    /// Polls [`PENDING`] while it is not empty.
    static PENDING_TIMER: slint::Timer = slint::Timer::default();
}

/// Record a window the host just showed and give it the player's title bar
/// (palette, level, icon). winit has no list of an application's windows,
/// so every `show()` reports here. Slint creates the native window in the
/// event loop after `show()`, so a window without one yet is set up as soon
/// as it has one (polled every frame, up to 5 s).
pub fn shown(instance: &slint_interpreter::ComponentInstance) {
    use slint::ComponentHandle;
    if let Some(native) = native_of(instance.window()) {
        set_up(&native);
        return;
    }
    PENDING.with_borrow_mut(|pending| pending.push((instance.as_weak(), PENDING_TRIES)));
    PENDING_TIMER.with(|timer| {
        if !timer.running() {
            timer.start(slint::TimerMode::Repeated, PENDING_TICK, poll_pending);
        }
    });
}

fn poll_pending() {
    use slint::ComponentHandle;
    let ready: Vec<Arc<winit::window::Window>> = PENDING.with_borrow_mut(|pending| {
        let mut ready = Vec::new();
        pending.retain_mut(|(weak, tries)| {
            let Some(instance) = weak.upgrade() else {
                return false;
            };
            if let Some(native) = native_of(instance.window()) {
                ready.push(native);
                return false;
            }
            *tries -= 1;
            if *tries == 0 {
                diag!(
                    "platform: a shown window got no native window in 5 s; no icon or title bar set"
                );
            }
            *tries > 0
        });
        ready
    });
    for native in &ready {
        set_up(native);
    }
    if PENDING.with_borrow(Vec::is_empty) {
        PENDING_TIMER.with(slint::Timer::stop);
    }
}

/// A host window's native window exists: palette, level and icon, and the list.
fn set_up(native: &Arc<winit::window::Window>) {
    let id = u64::from(native.id()) as i64;
    if FLOATING.get() {
        native.set_window_level(level(true));
    }
    title_bar(native, DARK.get());
    set_icons(native);
    WINDOWS.with_borrow_mut(|list| {
        list.retain(|(known, weak)| *known != id && weak.strong_count() > 0);
        list.push((id, Arc::downgrade(native)));
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

/// Title bars dark or light, following the player's palette rather than the
/// system setting, including windows shown later.
pub fn set_dark_title_bars(dark: bool) {
    DARK.set(dark);
    for (_, window) in live() {
        title_bar(&window, dark);
    }
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
    options: &crate::ui::confirm::Options,
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
    let labels = crate::ui::confirm::button_labels(options);
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
                crate::ui::confirm::confirmed_by(Some(&label), &labels)
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

// ------------------------------------------------------- opening the player

/// Explorer puts the file of "Open With" and associations on the command
/// line of a new process (`launch.rs`, `single_instance`); no event.
pub fn on_open_files(_deliver: impl Fn(Vec<PathBuf>) + 'static) {}

/// A second launch lets the running player take the foreground: Windows
/// lets only the process the user just started do it.
pub fn allow_handoff_foreground() {
    let _ = vestra_win32::allow_any_foreground();
}

// -------------------------------------------------------------------- facts

/// DXGI's adapters in Windows' order for this process (the per-app graphics
/// preference decides the first), with the user-mode driver's version
/// (`vestra_win32::dxgi_adapters`). Microsoft's own adapters (WARP, Basic
/// Render, vendor `0x1414`) count as software. None when DXGI is missing.
pub fn gpu_adapters() -> Vec<super::GpuAdapter> {
    const MICROSOFT: u32 = 0x1414;
    match vestra_win32::dxgi_adapters() {
        Ok(adapters) => adapters
            .into_iter()
            .map(|adapter| super::GpuAdapter {
                software: adapter.software || adapter.vendor_id == MICROSOFT,
                driver_version: adapter
                    .driver_version
                    .map(|parts| parts.map(|part| part.to_string()).join(".")),
                name: adapter.name,
                vendor_id: adapter.vendor_id,
                device_id: adapter.device_id,
            })
            .collect(),
        Err(error) => {
            diag!("render: the graphics adapters are unknown ({error})");
            Vec::new()
        }
    }
}

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

// ------------------------------------------------------------ full screen

/// How a full-screen window is kept from matching its display exactly
/// (`VESTRA_FULLSCREEN_MATCH`: `taller`, the default, one pixel past the
/// bottom edge; `shorter`, one pixel short of it; `exact`, winit's own
/// rectangle). A window that covers a display exactly can be switched by the
/// graphics driver to the panel's native variable-refresh timing (on the
/// 60-240 Hz test panel: 60 → 240 Hz and back), which blanks the display and
/// blocks every present for up to seconds (round doc, Appendix B).
fn fullscreen_extra() -> Option<i32> {
    static EXTRA: std::sync::OnceLock<Option<i32>> = std::sync::OnceLock::new();
    *EXTRA.get_or_init(
        || match std::env::var("VESTRA_FULLSCREEN_MATCH").as_deref() {
            Ok("exact") => None,
            Ok("shorter") => Some(-1),
            _ => Some(1),
        },
    )
}

/// After a resize of `window`: a borderless full-screen window that covers
/// its display exactly is made one pixel taller (or shorter, see
/// `fullscreen_extra`). winit keeps a full-screen window on its display but
/// does not reset its size while it stays on the same display.
pub fn fullscreen_resized(window: &slint::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(extra) = fullscreen_extra() else {
        return;
    };
    window.with_winit_window(|window| {
        if window.fullscreen().is_none() {
            return;
        }
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            vestra_win32::unmatch_display(handle.hwnd.get(), extra);
        }
    });
}

// ------------------------------------------------------------ redraw pacing

/// Software rendering shows the pictures through one reused texture (OpenGL
/// or wgpu, `vestra-video-texture`) instead of a new image per frame.
pub const VIDEO_TEXTURE: bool = true;

/// Slint's winit backend paces redraws here with a timer that is not tied
/// to the display; the host paces the main window itself (`frame_pacing.rs`).
pub const VBLANK_CLOCK: bool = true;

/// The vertical blanks of the display a window is on (`vestra_win32::VBlank`),
/// following the window to another display. Used on the pacing thread.
pub struct VBlankWaiter {
    clock: vestra_win32::VBlank,
    hwnd: isize,
    waits: u32,
}
impl VBlankWaiter {
    /// Block until the next vertical blank; `false` when the display clock
    /// failed and could not be opened again.
    pub fn wait(&mut self) -> bool {
        // About once a second at 60 Hz: has the window moved to another display?
        self.waits = self.waits.wrapping_add(1);
        if self.waits.is_multiple_of(64)
            && self.clock.moved(self.hwnd)
            && let Some(clock) = vestra_win32::VBlank::open(self.hwnd)
        {
            self.clock = clock;
        }
        if self.clock.wait() {
            return true;
        }
        match vestra_win32::VBlank::open(self.hwnd) {
            Some(clock) => {
                self.clock = clock;
                self.clock.wait()
            }
            None => false,
        }
    }
}

/// The display clock of `window` (shown, with a native window); `None` when
/// the display has none (for example in a remote desktop session).
pub fn vblank_waiter(window: &slint::Window) -> Option<VBlankWaiter> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let hwnd = window
        .with_winit_window(|window| match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
        .flatten()?;
    let clock = vestra_win32::VBlank::open(hwnd)?;
    Some(VBlankWaiter {
        clock,
        hwnd,
        waits: 0,
    })
}

// ---------------------------------------------------------------- rendering

/// A frame the GPU renderer published (`GpuHandle::latest`): a texture of the
/// windows' Direct3D 12 device (picture-in-picture is `unsupported` here, so
/// nothing reads it yet).
pub use vestra_gpu_render::Frame as GpuFrame;
/// libmpv draws with Direct3D 11 on a device of the windows' adapter into a
/// ring of textures shared with the windows' Direct3D 12 device; each window
/// (femtovg on wgpu) shows the newest one directly. The raw Direct3D calls
/// are in the `vestra-gpu-render` crate.
pub use vestra_gpu_render::{Draw as GpuDraw, Handle as GpuHandle, Renderer as GpuRenderer};
pub const GPU_RENDER: bool = true;
/// GPU mode's windows draw with femtovg on wgpu (Direct3D 12), not OpenGL.
pub const GPU_WINDOWS_WGPU: bool = true;
/// GPU rendering is experimental here: only the user's preference `gpu`
/// (the Video menu's switch) or `VESTRA_RENDER=gpu` turns it on
/// (`render_mode::Facts::gpu_opt_in`; round record 2026-10-render-modes §4 M3).
pub const GPU_OPT_IN: bool = true;

/// The host's contract with the GPU renderer (round record
/// 2026-10-render-modes §2.2), by delegation to `vestra-gpu-render`.
impl crate::playback::renderer::GpuBackend for GpuHandle {
    type Frame = Box<GpuFrame>;
    fn set_size(&self, size: (u32, u32)) {
        vestra_gpu_render::Handle::set_size(self, size);
    }
    fn set_display_fps(&self, fps: Option<f64>) {
        vestra_gpu_render::Handle::set_display_fps(self, fps);
    }
    fn frames(&self) -> u64 {
        vestra_gpu_render::Handle::frames(self)
    }
    fn shown(&self) -> u64 {
        vestra_gpu_render::Handle::shown(self)
    }
    fn size(&self) -> Option<(u32, u32)> {
        vestra_gpu_render::Handle::size(self)
    }
    fn reshow(&self) {
        vestra_gpu_render::Handle::reshow(self);
    }
    fn latest(&self) -> Option<(u64, Self::Frame)> {
        vestra_gpu_render::Handle::latest(self).map(|(sequence, frame)| (sequence, Box::new(frame)))
    }
    fn on_frame(&self, notify: impl Fn() + Send + 'static) {
        vestra_gpu_render::Handle::on_frame(self, notify);
    }
    fn attach<T: slint::ComponentHandle + 'static>(
        &self,
        component: &T,
        show: impl Fn(slint::Image) + 'static,
        failed: impl FnOnce(String) + 'static,
        drawn: impl FnMut(GpuDraw) + 'static,
    ) -> std::result::Result<(), String> {
        vestra_gpu_render::Handle::attach(self, component, show, failed, drawn)
    }
    fn retarget<T: slint::ComponentHandle + 'static>(&self, component: &T) {
        vestra_gpu_render::Handle::retarget(self, component);
    }
    fn log_text(&self) -> String {
        vestra_gpu_render::Handle::log_text(self)
    }
}

/// Start drawing `mpv` at `size` pixels; frames are shown while `publish`.
/// Refused (the host renders in software) when the windows do not draw with
/// Direct3D 12 on a GPU (`select_wgpu_windows`) or the adapter cannot
/// share textures and fences between Direct3D 11 and 12.
pub fn gpu_start(
    mpv: Arc<rsmpv::Mpv>,
    size: (u32, u32),
    publish: Arc<std::sync::atomic::AtomicBool>,
) -> std::result::Result<GpuRenderer, String> {
    GpuRenderer::start(mpv, size, publish)
}

/// The renderer's own diagnostics as `render:` lines.
pub fn gpu_started(handle: &GpuHandle) {
    handle.on_diag(|line| diag!("render: {line}"));
}

/// Why the GPU renderer stopped on its own (its device was removed, a
/// fence failed); the host then renders in software.
pub fn gpu_failed(handle: &GpuHandle) -> Option<String> {
    handle.failed()
}

/// GPU mode's windows (`render_mode::WindowKind::Wgpu`; before the event
/// loop, not headless): every window draws with femtovg on wgpu, on one
/// Direct3D 12 device of DXGI's first adapter (`VESTRA_GPU_ADAPTER`:
/// another index, `high-performance`, `minimum-power` or `warp`), which GPU
/// rendering shares its textures with. Only applies the kind: whether GPU
/// mode is wanted is `render_mode::decide`'s. `Err` with why when the
/// windows cannot draw that way (no Direct3D 12, an adapter without a GPU of
/// its own such as WARP in a remote session or a virtual machine, or Slint
/// refusing): the host then falls back to software rendering and CPU mode's
/// windows. `VESTRA_WINDOW_RENDERER=wgpu` (a test switch) accepts an adapter
/// without a GPU. A lost device later ends the event loop
/// (`window_device_lost`; the host restarts the player).
pub fn select_wgpu_windows(
    winit: impl Fn() -> slint::BackendSelector,
) -> std::result::Result<(), String> {
    let adapter = std::env::var("VESTRA_GPU_ADAPTER").ok();
    let device = vestra_gpu_render::WindowDevice::open(adapter.as_deref())
        .map_err(|error| format!("no Direct3D 12 device ({error})"))?;
    let any_adapter =
        std::env::var("VESTRA_WINDOW_RENDERER").is_ok_and(|value| value.trim() == "wgpu");
    if device.software() && !any_adapter {
        // Dropped here: the fallback keeps no Direct3D 12 device.
        return Err(format!("{} has no GPU of its own", device.describe()));
    }
    // Before the selection: a device lost from here on is reported and
    // ends the event loop.
    device.on_error(
        |line| diag!("render: {line}"),
        || {
            // Restart now (whatever the teardown does), then end the loop:
            // the windows cannot draw any more.
            if let Some(hook) = LOST_HOOK.get() {
                hook();
            }
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        },
    );
    winit()
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(device.configuration())
        .select()
        .map_err(|error| format!("the Direct3D 12 window renderer failed ({error})"))?;
    device.install();
    WGPU_WINDOWS.store(true, std::sync::atomic::Ordering::Relaxed);
    // Test switch (R2): remove the device after N s, as a driver reset
    // would, to exercise the loss and restart path on real hardware.
    if let Some(seconds) = std::env::var("VESTRA_TEST_REMOVE_DEVICE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.)
    {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
            let removed = vestra_gpu_render::remove_window_device_for_test();
            diag!("render: VESTRA_TEST_REMOVE_DEVICE: window device removed: {removed}");
        });
    }
    diag!(
        "render: the windows draw with Direct3D 12 (wgpu) on {}",
        device.describe()
    );
    Ok(())
}

/// What to do when the windows' Direct3D 12 device is lost, on the thread
/// that notices (the host starts the restarted player at once); set before
/// `select_wgpu_windows`.
static LOST_HOOK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// Run `hook` when the windows' device is lost (once per process).
pub fn on_window_device_lost(hook: impl Fn() + Send + Sync + 'static) {
    let _ = LOST_HOOK.set(Box::new(hook));
}

/// The windows draw with wgpu (set by `select_wgpu_windows`).
static WGPU_WINDOWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a window may show video through a rendering notifier. Not with
/// wgpu windows by default: a notifier makes Slint 1.18.1's femtovg-wgpu
/// recompile its pipelines on every draw (`vestra-gpu-render`
/// `uses_rendering_notifier`, `VESTRA_WGPU_NOTIFIER=1` to compare). Then the
/// host counts draws through `frame_pacing::on_draw` and software pictures
/// go through `direct_video_feed`.
pub fn window_notifiers() -> bool {
    !WGPU_WINDOWS.load(std::sync::atomic::Ordering::Relaxed)
        || vestra_gpu_render::uses_rendering_notifier()
}

/// The software rendering texture of a wgpu window without a notifier.
pub fn direct_video_feed() -> Option<vestra_video_texture::Feed> {
    let (device, queue) = vestra_gpu_render::window_wgpu()?;
    vestra_video_texture::direct(&device, &queue)
}

/// GPU frames in a window without a notifier take the main window's frame
/// pacing (`frame_pacing::accept_picture`): a new frame is taken only once
/// the previous picture was drawn (`vestra-gpu-render`
/// `gate_direct_frames`). `None`: every frame at once.
pub fn gate_gpu_frames(accept: Option<Box<dyn Fn() -> bool>>) {
    vestra_gpu_render::gate_direct_frames(accept);
}

/// After a draw: take a GPU frame the pacing refused before it.
pub fn update_gpu_frames() {
    vestra_gpu_render::update_direct_views();
}

/// The calling thread's CPU cycles so far (the frame log's `cpu` segment).
pub fn thread_cycles() -> u64 {
    vestra_win32::thread_cycles()
}

/// The main window starts a draw (frame log: ` ui-gpu`).
pub fn ui_draw_started() {
    vestra_gpu_render::ui_draw_started();
}

/// The main window's draw went to the GPU (frame log: ` ui-gpu`).
pub fn ui_draw_submitted() {
    vestra_gpu_render::ui_draw_submitted();
}

/// `VESTRA_FRAME_LOG`: the main window's draws' GPU completion.
pub fn ui_gpu_text() -> String {
    vestra_gpu_render::ui_gpu_text()
}

/// Wait for process `pid` to end (a restarted player waits for the one it
/// replaces); `true` once it has or when it is gone.
pub fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) -> bool {
    vestra_win32::wait_for_process_exit(
        pid,
        timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
    )
}

/// The windows' Direct3D 12 device was lost (a driver reset): the event
/// loop ended, and the host restarts the player.
pub fn window_device_lost() -> bool {
    vestra_gpu_render::device_lost()
}

// --------------------------------------------------------- video upscaling

/// NVIDIA's PCI vendor id.
const NVIDIA: u32 = 0x10DE;

/// The NVIDIA driver RTX Video Super Resolution needs: 530 or newer, which
/// DXGI reports as `x.x.15.3000` and above (VLC's check; AI upscaling round
/// record §2).
fn nvidia_driver_new_enough(version: [u16; 4]) -> bool {
    (version[2], version[3]) >= (15, 3000)
}

/// An NVIDIA driver version as NVIDIA writes it: `31.0.15.3118` → `531.18`.
fn nvidia_driver_name(version: [u16; 4]) -> String {
    let release = u32::from(version[2] % 10) * 10000 + u32::from(version[3]);
    format!("{}.{:02}", release / 100, release % 100)
}

/// Where RTX Video Super Resolution works (AI upscaling round record §2):
/// every NVIDIA adapter (vendor `0x10DE`) with a driver of 530 or newer
/// whose driver answers the PPE stream extension query on a test video
/// processor (`vestra_win32::video_super_resolution`). Blocking, and it
/// wakes a sleeping discrete GPU: ask when the answer is needed and keep
/// it. Not detectable: the NVIDIA app's Super Resolution switch, its quality
/// level and its battery policy.
pub fn upscaler_probe() -> super::UpscalerProbe {
    let adapters = vestra_win32::dxgi_adapters().unwrap_or_else(|error| {
        diag!("upscaling: the graphics adapters are unknown ({error})");
        Vec::new()
    });
    let mut found = Vec::new();
    let mut refusal = None::<super::UpscalerRefusal>;
    let mut refuse = |code: &'static str, message: String| {
        diag!("upscaling: {message}");
        // The furthest check that failed explains it best.
        let rank = |code: &str| {
            ["no-gpu", "driver", "gpu-unsupported"]
                .iter()
                .position(|c| *c == code)
        };
        if refusal.as_ref().is_none_or(|r| rank(code) > rank(r.code)) {
            refusal = Some(super::UpscalerRefusal { code, message });
        }
    };
    for adapter in adapters
        .iter()
        .filter(|adapter| adapter.vendor_id == NVIDIA && !adapter.software)
    {
        if let Some(version) = adapter.driver_version
            && !nvidia_driver_new_enough(version)
        {
            refuse(
                "driver",
                format!(
                    "the NVIDIA driver {} on {} is too old for RTX Video Super Resolution, which needs 530 or newer; update the driver",
                    nvidia_driver_name(version),
                    adapter.name
                ),
            );
            continue;
        }
        let started = std::time::Instant::now();
        let answer = vestra_win32::video_super_resolution(adapter.luid);
        let took = started.elapsed();
        match answer {
            Ok(answer) if answer.supported() => {
                diag!(
                    "upscaling: RTX Video Super Resolution is offered on {} (LUID {:016x}; checked in {took:?})",
                    adapter.name,
                    adapter.luid
                );
                found.push(super::UpscalerGpu {
                    id: adapter.luid,
                    name: adapter.name.clone(),
                });
            }
            Ok(answer) => refuse(
                "gpu-unsupported",
                format!(
                    "the NVIDIA driver does not offer RTX Video Super Resolution on {}, which needs an RTX GPU (query result 0x{:08X}, value {})",
                    adapter.name, answer.result as u32, answer.value
                ),
            ),
            Err(error) => refuse(
                "gpu-unsupported",
                format!(
                    "RTX Video Super Resolution cannot be checked on {} ({error})",
                    adapter.name
                ),
            ),
        }
    }
    if !found.is_empty() {
        return Ok(found);
    }
    Err(refusal.unwrap_or_else(|| super::UpscalerRefusal {
        code: "no-gpu",
        message: "no NVIDIA GPU: the player upscales video with NVIDIA RTX Video Super Resolution, which needs an NVIDIA RTX GPU".into(),
    }))
}

/// The adapter GPU rendering runs on (its LUID, as `upscaler_probe`
/// reports GPUs, and its name); `None` while the windows do not draw with
/// Direct3D 12.
pub fn gpu_render_adapter() -> Option<(u64, String)> {
    vestra_gpu_render::window_adapter()
}

/// mpv's filter for RTX Video Super Resolution at `factor` (more than 1):
/// `d3d11vpp` scales by it with NVIDIA's scaling mode (mpv 0.39 and later).
/// `VESTRA_UPSCALER_MODE=standard` (a test switch, AI upscaling round record
/// §8 V3 case U5) asks for the driver's standard scaler instead, to compare
/// pixels in the same chain.
pub fn upscaler_filter(factor: f64) -> Option<String> {
    let standard =
        std::env::var("VESTRA_UPSCALER_MODE").is_ok_and(|mode| mode.trim() == "standard");
    Some(d3d11vpp_filter(factor, standard))
}

fn d3d11vpp_filter(factor: f64, standard: bool) -> String {
    let mode = if standard { "standard" } else { "nvidia" };
    format!("d3d11vpp=scale={factor:.2}:scaling-mode={mode}")
}

/// The hardware decoding `d3d11vpp` needs: Direct3D 11 frames on mpv's own
/// device (not copied back).
pub const UPSCALER_HWDEC: &str = "d3d11va";

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

    /// `.cargo/config.toml`: no VC++ redistributable needed. Checked when
    /// the tests compile (a build without the flag does not compile them).
    #[cfg(target_env = "msvc")]
    const _: () = assert!(
        cfg!(target_feature = "crt-static"),
        "build with +crt-static (src/host/.cargo/config.toml; RUSTFLAGS replaces it)"
    );

    #[test]
    fn rtx_video_super_resolution_needs_driver_530() {
        assert!(nvidia_driver_new_enough([31, 0, 15, 3000]), "530.00");
        assert!(nvidia_driver_new_enough([32, 0, 15, 6094]), "560.94");
        assert!(nvidia_driver_new_enough([32, 0, 16, 100]), "a later branch");
        assert!(!nvidia_driver_new_enough([31, 0, 15, 2647]), "526.47");
        assert!(!nvidia_driver_new_enough([30, 0, 14, 7212]), "472.12");
        assert_eq!(nvidia_driver_name([31, 0, 15, 3118]), "531.18");
        assert_eq!(nvidia_driver_name([32, 0, 15, 6094]), "560.94");
        assert_eq!(nvidia_driver_name([30, 0, 14, 7212]), "472.12");
    }

    #[test]
    fn the_upscaler_filter_scales_with_nvidia_mode() {
        assert_eq!(
            d3d11vpp_filter(1.5, false),
            "d3d11vpp=scale=1.50:scaling-mode=nvidia"
        );
        assert_eq!(
            d3d11vpp_filter(2., true),
            "d3d11vpp=scale=2.00:scaling-mode=standard"
        );
    }

    #[test]
    fn the_build_number_names_the_release() {
        assert_eq!(windows_name(Some(22631)), "Windows 11");
        assert_eq!(windows_name(Some(19045)), "Windows 10");
        assert_eq!(windows_name(None), "Windows");
    }

    /// The executable carries the icon group the windows use (the test
    /// program is linked with the same resources), at the sizes Windows
    /// draws at 100 %, 175 % and 200 %.
    #[test]
    fn the_window_icon_loads_from_the_executable_at_every_scale() {
        for scale in [1., 1.25, 1.5, 1.75, 2.] {
            for logical in [16., 32.] {
                assert!(
                    super::icon(logical, scale).is_ok(),
                    "{logical} px at {scale}"
                );
            }
        }
    }
}
