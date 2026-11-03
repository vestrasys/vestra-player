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

/// Whether the user works in one of the player's windows right now (the
/// foreground window is this process's).
pub fn app_in_foreground() -> bool {
    vestra_win32::foreground_is_ours()
}

/// Bring `window` to the front with the focus while the player is in the
/// foreground: a plain `SetForegroundWindow`. winit's `focus_window` would
/// press a synthetic Alt first to get past the foreground lock, which is
/// for taking the foreground from another app, and can leave a menu bar
/// activated.
pub fn focus_own_window(window: &slint::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(native) = native_of(window) else {
        return;
    };
    let Ok(handle) = native.window_handle() else {
        return;
    };
    if let RawWindowHandle::Win32(handle) = handle.as_raw()
        && !vestra_win32::set_foreground(handle.hwnd.get())
    {
        diag!("platform: the system refused to bring a window to the front");
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

// ------------------------------------------------- no white frame at a show

/// winit's window class (`WindowAttributesExtWindows::with_class_name`'s
/// default; the host sets none): every Slint window of the host, and no
/// system dialog (file dialogs, task dialogs) or helper window.
const WINIT_CLASS: &str = "Window Class";

/// How long a window stays cloaked without a draw before it is shown anyway
/// (minimized, a draw skipped, a held redraw that never comes).
const CLOAK_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
/// How often the windows still cloaked are checked against [`CLOAK_LIMIT`].
const CLOAK_CHECK: std::time::Duration = std::time::Duration::from_millis(100);

/// Windows cloaked at their show (`cloak_until_drawn`) that have not drawn
/// yet: raw HWND, when.
#[derive(Default)]
struct Cloaked(Vec<(isize, std::time::Instant)>);
impl Cloaked {
    /// `hwnd` was cloaked at `now` (again: from `now`).
    fn add(&mut self, hwnd: isize, now: std::time::Instant) {
        self.0.retain(|(known, _)| *known != hwnd);
        self.0.push((hwnd, now));
    }
    /// `hwnd` draws: when it was cloaked, if it still is; it is no longer
    /// waited for.
    fn drawn(&mut self, hwnd: isize) -> Option<std::time::Instant> {
        let index = self.0.iter().position(|(known, _)| *known == hwnd)?;
        Some(self.0.remove(index).1)
    }
    /// The windows cloaked `limit` or longer before `now`, no longer waited for.
    fn overdue(&mut self, now: std::time::Instant, limit: std::time::Duration) -> Vec<isize> {
        let mut late = Vec::new();
        self.0.retain(|(hwnd, at)| {
            let waiting = now.saturating_duration_since(*at) < limit;
            if !waiting {
                late.push(*hwnd);
            }
            waiting
        });
        late
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

thread_local! {
    /// `cloak_until_drawn` (UI thread).
    static CLOAKED: RefCell<Cloaked> = RefCell::new(Cloaked::default());
    /// Checks [`CLOAKED`] against [`CLOAK_LIMIT`] while it is not empty.
    static CLOAK_TIMER: slint::Timer = slint::Timer::default();
}

/// No white frame when a window opens. Slint draws a window's first frame
/// before it shows it, but Windows discards what is drawn into a hidden
/// window, so the compositor shows the empty client area (white) until the
/// next draw, during the open animation (two frames of white for the Plugin
/// Store, Options and the main window). As Chromium and Windows Terminal
/// do, each host window is cloaked (DWMWA_CLOAK) just before it shows, its
/// title bar set to the palette then too, and uncloaked after its first
/// draw (`window_drawing`), or after [`CLOAK_LIMIT`] without one. Call once
/// on the UI thread before the first window shows. `VESTRA_CLOAK_ON_SHOW=0`
/// (A/B tests) leaves windows as Slint shows them.
pub fn cloak_until_drawn() {
    if std::env::var("VESTRA_CLOAK_ON_SHOW").as_deref() == Ok("0") {
        diag!("ui: VESTRA_CLOAK_ON_SHOW=0: windows show before their first draw");
        return;
    }
    let installed = vestra_win32::cloak_on_show(WINIT_CLASS, |hwnd| {
        if !vestra_win32::set_dark_title_bar(hwnd, DARK.get()) {
            diag!("platform: the system refused a dark/light title bar");
        }
        CLOAKED.with_borrow_mut(|cloaked| cloaked.add(hwnd, std::time::Instant::now()));
        CLOAK_TIMER.with(|timer| {
            if !timer.running() {
                timer.start(slint::TimerMode::Repeated, CLOAK_CHECK, uncloak_overdue);
            }
        });
    });
    if !installed {
        diag!("ui: windows cannot be cloaked until their first draw; they may open white");
    }
}

fn uncloak_overdue() {
    let late =
        CLOAKED.with_borrow_mut(|cloaked| cloaked.overdue(std::time::Instant::now(), CLOAK_LIMIT));
    for hwnd in late {
        vestra_win32::set_cloaked(hwnd, false);
        diag!(
            "ui: a window showed without a draw after {} ms",
            CLOAK_LIMIT.as_millis()
        );
    }
    if CLOAKED.with_borrow(Cloaked::is_empty) {
        CLOAK_TIMER.with(slint::Timer::stop);
    }
}

/// winit delivers a `RedrawRequested` to window `id` (a winit `WindowId`:
/// the HWND on Windows) that Slint draws now: a window cloaked at its show
/// is uncloaked right after this draw (a posted callback, after Slint's
/// draw and present).
pub fn window_drawing(id: u64) {
    let hwnd = id as isize;
    let Some(at) = CLOAKED.with_borrow_mut(|cloaked| cloaked.drawn(hwnd)) else {
        return;
    };
    let waited = at.elapsed();
    let uncloak = move || {
        vestra_win32::set_cloaked(hwnd, false);
        diag!(
            "ui: a window showed after its first draw ({} ms)",
            waited.as_millis()
        );
    };
    if slint::invoke_from_event_loop(uncloak).is_err() {
        vestra_win32::set_cloaked(hwnd, false);
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

/// The per-app GPU preferences Windows Settings > System > Display >
/// Graphics writes: one string value per program, named by its path.
const USER_GPU_PREFERENCES: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

/// `GpuPreference=N` in a `UserGpuPreferences` value (`GpuPreference=2;`,
/// possibly among other settings such as `SwapEffectUpgradeEnable=1;`).
fn gpu_preference(value: &str) -> Option<u32> {
    value.split(';').find_map(|setting| {
        let (name, number) = setting.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("GpuPreference")
            .then(|| number.trim().parse().ok())
            .flatten()
    })
}

/// An explicit choice for Vestra in Windows Settings: `1` power saving, `2`
/// high performance (and any later non-zero value, such as a specific GPU);
/// `0` is "Let Windows decide", which is no choice.
fn explicit_gpu_preference(value: &str) -> Option<u32> {
    gpu_preference(value).filter(|&preference| preference != 0)
}

/// The value among `entries` (name, data) that is this executable's:
/// `same(name)` says whether a value name is the same file (its path in
/// another form: a short 8.3 path, a `subst` drive, a junction, a mapped
/// drive). The first that is.
fn matching_entry(
    entries: &[(String, String)],
    same: impl Fn(&str) -> bool,
) -> Option<&(String, String)> {
    entries.iter().find(|(name, _)| same(name))
}

/// A path in one comparable form: canonical (long names, links and `subst`
/// resolved), lower case. `None` when it does not exist.
fn comparable(path: &Path) -> Option<String> {
    dunce::canonicalize(path)
        .ok()
        .map(|path| path.to_string_lossy().to_lowercase())
}

/// The user's explicit Windows Settings preference for this executable
/// (`HKCU\…\UserGpuPreferences`), and a line worth logging. The value is
/// looked up by the executable's path; when no value has exactly that name,
/// every value's name is compared as a canonical path (the setting may name
/// the player by another form of its path), and when none is this
/// executable the line says so.
fn user_gpu_preference() -> (Option<u32>, Option<String>) {
    let Ok(exe) = std::env::current_exe() else {
        return (
            None,
            Some(
                "the player's path is unknown, so its Windows Settings GPU preference is not read"
                    .into(),
            ),
        );
    };
    let Ok(key) = windows_registry::CURRENT_USER.open(USER_GPU_PREFERENCES) else {
        return (None, None);
    };
    if let Some(name) = exe.to_str()
        && let Ok(value) = key.get_string(name)
    {
        return (explicit_gpu_preference(&value), None);
    }
    let entries: Vec<(String, String)> = key
        .values()
        .map(|values| {
            values
                .filter_map(|(name, value)| String::try_from(value).ok().map(|value| (name, value)))
                .collect()
        })
        .unwrap_or_default();
    // Only entries for a program of the same file name are resolved (a
    // canonical path may touch an offline network drive): other programs'
    // preferences and settings such as `DirectXUserGlobalSettings` are not.
    let file_name = exe
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let candidates: Vec<(String, String)> = entries
        .into_iter()
        .filter(|(name, _)| same_file_name(name, &file_name))
        .collect();
    if candidates.is_empty() {
        return (None, None);
    }
    let ours = comparable(&exe);
    let found = ours.as_ref().and_then(|ours| {
        matching_entry(&candidates, |name| {
            comparable(Path::new(name)).is_some_and(|theirs| theirs == *ours)
        })
    });
    match found {
        Some((name, value)) => (
            explicit_gpu_preference(value),
            Some(format!(
                "the Windows Settings GPU preference for this player is under {name}"
            )),
        ),
        None => (
            None,
            Some(format!(
                "Windows Settings has a GPU preference for {} but not for this copy of the player ({})",
                candidates[0].0,
                exe.display()
            )),
        ),
    }
}

/// Whether registry value `name` is a path (it has a separator) whose file
/// name is `file_name` (case-insensitive).
fn same_file_name(name: &str, file_name: &str) -> bool {
    !file_name.is_empty()
        && name
            .rsplit_once(['\\', '/'])
            .is_some_and(|(_, last)| last.eq_ignore_ascii_case(file_name))
}

/// How the adapter of a mode is chosen (user ruling 2026-10-31, round
/// record 2026-10-render-modes §12).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Policy {
    /// `VESTRA_GPU_ADAPTER` names one (its value).
    Env(vestra_gpu_render::Choice, String),
    /// Windows' order for the process (why).
    Order(String),
    /// The high-performance GPU (GPU rendering's default).
    HighPerformance,
}

const ORDER: &str = "Windows' order for this process (Settings > Display > Graphics)";

/// `VESTRA_GPU_ADAPTER`: an index, `high-performance`, `minimum-power` or
/// `warp`; empty is unset; anything else is ignored, with a line, as if it
/// were unset (it must not switch the high-performance default off).
fn env_adapter(
    value: Option<&str>,
) -> (Option<(vestra_gpu_render::Choice, String)>, Option<String>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };
    let known = matches!(value, "warp" | "high-performance" | "minimum-power")
        || value.parse::<u32>().is_ok();
    if known {
        (
            Some((
                vestra_gpu_render::Choice::parse(Some(value)),
                format!("VESTRA_GPU_ADAPTER={value}"),
            )),
            None,
        )
    } else {
        (
            None,
            Some(format!(
                "render: VESTRA_GPU_ADAPTER={value} is not an index, high-performance, minimum-power or warp; ignored"
            )),
        )
    }
}

/// The policy of a mode: `VESTRA_GPU_ADAPTER` (`env`) wins; CPU mode
/// follows Windows' order; GPU rendering follows Windows' order only where
/// the user set a preference for Vestra in Windows Settings (`preference`,
/// read only then), and otherwise takes the high-performance GPU.
fn adapter_policy(
    env: Option<(vestra_gpu_render::Choice, String)>,
    gpu_mode: bool,
    preference: impl FnOnce() -> Option<u32>,
) -> Policy {
    if let Some((choice, how)) = env {
        return Policy::Env(choice, how);
    }
    if !gpu_mode {
        return Policy::Order(ORDER.into());
    }
    match preference() {
        Some(preference) => Policy::Order(format!(
            "Vestra's preference in Windows Settings > System > Display > Graphics ({})",
            match preference {
                1 => "Power saving".to_owned(),
                2 => "High performance".to_owned(),
                other => format!("GpuPreference={other}"),
            }
        )),
        None => Policy::HighPerformance,
    }
}

/// The adapter `policy` gives among `adapters` (DXGI's order): `preferred`
/// answers `EnumAdapterByGpuPreference` (`true`: high performance, `false`:
/// minimum power) with a LUID. Where the high-performance default cannot
/// be resolved (no `IDXGIFactory6`, an error, a LUID not in the list), it
/// falls back to Windows' order and says why; an explicit
/// `VESTRA_GPU_ADAPTER` that cannot be resolved gives no adapter.
fn resolve_adapter(
    policy: &Policy,
    adapters: &[super::GpuAdapter],
    preferred: impl Fn(bool) -> std::result::Result<u64, String>,
) -> super::AdapterPick {
    use vestra_gpu_render::Choice;
    let at = |index: Option<usize>, how: String| super::AdapterPick {
        index,
        luid: index.and_then(|index| adapters.get(index)).map(|a| a.luid),
        warp: false,
        how,
    };
    let by_preference = |high: bool| {
        preferred(high).and_then(|luid| {
            adapters
                .iter()
                .position(|a| a.luid == luid)
                .ok_or_else(|| format!("its LUID {luid:016x} is not among the adapters"))
        })
    };
    let first = (!adapters.is_empty()).then_some(0);
    match policy {
        Policy::Order(how) => at(first, how.clone()),
        Policy::HighPerformance => match by_preference(true) {
            Ok(index) => at(
                Some(index),
                "GPU rendering's default, the high-performance GPU (no preference for Vestra in Windows Settings > System > Display > Graphics)".into(),
            ),
            Err(error) => at(
                first,
                format!("{ORDER}, as the high-performance GPU could not be determined ({error})"),
            ),
        },
        Policy::Env(choice, how) => match *choice {
            Choice::Warp => super::AdapterPick {
                index: adapters.iter().position(|adapter| adapter.software),
                luid: None,
                warp: true,
                how: how.clone(),
            },
            Choice::Index(index) => at(
                usize::try_from(index)
                    .ok()
                    .filter(|&index| index < adapters.len()),
                how.clone(),
            ),
            Choice::HighPerformance | Choice::MinimumPower => {
                match by_preference(*choice == Choice::HighPerformance) {
                    Ok(index) => at(Some(index), how.clone()),
                    Err(error) => at(None, format!("{how}, which could not be resolved ({error})")),
                }
            }
            Choice::Default | Choice::Luid(_) => at(first, how.clone()),
        },
    }
}

/// DXGI's adapters in Windows' order for this process (the per-app graphics
/// preference decides the first), with the user-mode driver's version
/// (`vestra_win32::dxgi_adapters`), and the adapters GPU rendering and CPU
/// mode's Direct3D 12 windows use, resolved once here (the environment and
/// the registry are read once) and opened as they are
/// (`select_wgpu_windows`). Microsoft's own adapters (WARP, Basic Render,
/// vendor `0x1414`) count as software. None when DXGI is missing.
pub fn gpu_adapters() -> super::GpuAdapters {
    const MICROSOFT: u32 = 0x1414;
    let adapters = match vestra_win32::dxgi_adapters() {
        Ok(adapters) => adapters,
        Err(error) => {
            diag!("render: the graphics adapters are unknown ({error})");
            let unknown = super::AdapterPick {
                how: format!("nothing: the graphics adapters are unknown ({error})"),
                ..super::AdapterPick::default()
            };
            return super::GpuAdapters {
                list: Vec::new(),
                gpu: unknown.clone(),
                windows: unknown,
            };
        }
    };
    let list: Vec<super::GpuAdapter> = adapters
        .into_iter()
        .map(|adapter| super::GpuAdapter {
            software: adapter.software || adapter.vendor_id == MICROSOFT,
            driver_version: adapter
                .driver_version
                .map(|parts| parts.map(|part| part.to_string()).join(".")),
            name: adapter.name,
            vendor_id: adapter.vendor_id,
            device_id: adapter.device_id,
            luid: adapter.luid,
        })
        .collect();
    let env = std::env::var("VESTRA_GPU_ADAPTER").ok();
    let (env, warning) = env_adapter(env.as_deref());
    if let Some(warning) = warning {
        diag!("{warning}");
    }
    let gpu_policy = adapter_policy(env.clone(), true, || {
        let (preference, note) = user_gpu_preference();
        if let Some(note) = note {
            diag!("render: {note}");
        }
        preference
    });
    let windows_policy = adapter_policy(env, false, || None);
    let preferred = |high: bool| vestra_win32::dxgi_preferred_adapter(high);
    super::GpuAdapters {
        gpu: resolve_adapter(&gpu_policy, &list, preferred),
        windows: resolve_adapter(&windows_policy, &list, preferred),
        list,
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
/// `exact` is the switch for testing Independent Flip without the extra
/// pixel (round record 2026-10-render-modes §11 item 6).
fn fullscreen_extra() -> Option<i32> {
    static EXTRA: std::sync::OnceLock<Option<i32>> = std::sync::OnceLock::new();
    *EXTRA.get_or_init(|| {
        let value = std::env::var("VESTRA_FULLSCREEN_MATCH").ok();
        let (extra, line) = fullscreen_match(value.as_deref());
        if let Some(line) = line {
            diag!("{line}");
        }
        extra
    })
}

/// `VESTRA_FULLSCREEN_MATCH`: the pixels a full-screen window is made
/// taller (`None`: winit's exact rectangle), and a line to log when the
/// value is not the default.
fn fullscreen_match(value: Option<&str>) -> (Option<i32>, Option<String>) {
    match value.map(str::trim) {
        None | Some("" | "taller") => (Some(1), None),
        Some("exact") => (
            None,
            Some("ui: VESTRA_FULLSCREEN_MATCH=exact: a full-screen window covers its display exactly".into()),
        ),
        Some("shorter") => (
            Some(-1),
            Some("ui: VESTRA_FULLSCREEN_MATCH=shorter: a full-screen window is one pixel short of its display".into()),
        ),
        Some(other) => (
            Some(1),
            Some(format!(
                "ui: VESTRA_FULLSCREEN_MATCH={other} is not taller, shorter or exact; ignored"
            )),
        ),
    }
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
    /// Opening the clock of the window's new display failed, and was
    /// reported (`reopen_failure_to_report`).
    reopen_failed: bool,
}

/// Whether a failed reopen of the display clock after a display change is
/// reported now: the first failure of a run of failures (the check repeats
/// about once a second while the window stays there), again only after an
/// open succeeded. `failed` keeps the state between calls.
fn reopen_failure_to_report(failed: &mut bool, opened: bool) -> bool {
    let report = !opened && !*failed;
    *failed = !opened;
    report
}
impl VBlankWaiter {
    /// Block until the next vertical blank; `false` when the display clock
    /// failed and could not be opened again. `moved`: the window moved or
    /// changed its scale factor since the last wait (winit `Moved`,
    /// `ScaleFactorChanged`), so it may be on another display now.
    pub fn wait(&mut self, moved: bool) -> bool {
        // After a move at once (0.5.15; before, up to 64 waits, about a
        // second at 60 Hz), and about once a second for a display change
        // without a move (the display arrangement changed).
        self.waits = self.waits.wrapping_add(1);
        if (moved || self.waits.is_multiple_of(64)) && self.clock.moved(self.hwnd) {
            let opened = vestra_win32::VBlank::open(self.hwnd);
            if reopen_failure_to_report(&mut self.reopen_failed, opened.is_some()) {
                diag!(
                    "frame pacing: the window is on another display, whose clock could not be opened; pacing keeps the previous display's vertical blanks"
                );
            }
            if let Some(clock) = opened {
                self.clock = clock;
            }
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
        reopen_failed: false,
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
    fn progress(&self) -> Option<crate::playback::renderer::Progress> {
        let progress = vestra_gpu_render::Handle::progress(self);
        Some(crate::playback::renderer::Progress {
            calls: progress.calls,
            started: progress.started,
            ready: progress.ready,
            completed: progress.completed,
            rings: progress.rings,
            halted: progress.halted,
        })
    }
    fn device_removed(&self) -> bool {
        vestra_gpu_render::Handle::device_removed(self)
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

/// Direct3D 12 windows (`render_mode::WindowKind::Wgpu`: GPU mode's, and
/// since 0.5.19 CPU mode's; before the event loop, not headless): every
/// window draws with femtovg on wgpu, on one Direct3D 12 device of the
/// adapter `gpu_adapters` resolved for the mode (`adapter`: GPU rendering's
/// is the high-performance GPU unless the user chose one for Vestra in
/// Windows Settings, CPU mode's follows Windows' order,
/// `VESTRA_GPU_ADAPTER` overrides both), opened by its LUID, which GPU
/// rendering shares its textures with; software rendering uploads its
/// pictures into a texture of it. Only
/// applies the kind: what is wanted is `render_mode::decide`'s. `Err` with
/// why when the windows cannot draw that way (no Direct3D 12, an adapter
/// without a GPU of its own such as WARP in a remote session or a virtual
/// machine, or Slint refusing): the host then falls back to software
/// rendering (GPU mode) and CPU mode's fallback windows. `VESTRA_WINDOW_RENDERER=wgpu` (a test switch) accepts an adapter
/// without a GPU. A lost device later ends the event loop
/// (`window_device_lost`; the host restarts the player).
pub fn select_wgpu_windows(
    winit: impl Fn() -> slint::BackendSelector,
    adapter: &super::AdapterPick,
) -> std::result::Result<(), String> {
    // The adapter the crash guard and the denylist looked at, by LUID.
    use vestra_gpu_render::Choice;
    let choice = if adapter.warp {
        Choice::Warp
    } else if let Some(luid) = adapter.luid {
        Choice::Luid(luid)
    } else {
        return Err(format!("no graphics adapter ({})", adapter.how));
    };
    let device = vestra_gpu_render::WindowDevice::open(choice, &adapter.how)
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
            // The windows cannot draw any more: take them off the screen at
            // once, from whatever thread noticed (a black window that stops
            // responding while the teardown runs is worse than none; the
            // restarted player shows its own). Then restart now (whatever
            // the teardown does) and end the loop.
            let cloaked = vestra_win32::cloak_process_windows();
            crate::host::restart_log(&format!(
                "render: {cloaked} window(s) cloaked after the device loss"
            ));
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

/// Whether two video area images show the same texture. Slint 1.18.1's
/// `Image` equality has no case for wgpu textures, so an image made from a
/// texture is never equal even to itself; the textures are compared here.
pub fn same_video_image(shown: &slint::Image, next: &slint::Image) -> bool {
    match (shown.to_wgpu_30_texture(), next.to_wgpu_30_texture()) {
        (Some(shown), Some(next)) => shown == next,
        (None, None) => shown == next,
        _ => false,
    }
}

/// GPU frames in a window without a notifier take the main window's frame
/// pacing (`frame_pacing::accept_picture`): a new frame is taken only once
/// the previous picture was drawn (`vestra-gpu-render`
/// `gate_direct_frames`). `None`: every frame at once.
pub fn gate_gpu_frames(accept: Option<Box<dyn Fn() -> bool>>) {
    vestra_gpu_render::gate_direct_frames(accept);
}

/// The player quits: release the GPU renderer's shared fences and stop it,
/// before the windows and their device go, so a frame that never finishes
/// on the GPU (a stall) cannot hold up the quit (`vestra-gpu-render`
/// `release_gpu_waits`).
pub fn release_gpu_waits() {
    vestra_gpu_render::release_gpu_waits();
}

/// After a draw: take a GPU frame the pacing refused before it.
pub fn update_gpu_frames() {
    vestra_gpu_render::update_direct_views();
}

/// The calling thread's CPU cycles so far (the frame log's `cpu` segment).
pub fn thread_cycles() -> u64 {
    vestra_win32::thread_cycles()
}

/// The session ends, or asks to (shutdown, restart, logoff, an update's
/// restart): `WM_QUERYENDSESSION` is `Query`, `WM_ENDSESSION` TRUE
/// `Ending`, FALSE `Cancelled`. On the UI thread, inside the message;
/// keep it short.
pub fn on_session_end(ended: impl Fn(super::SessionEvent) + 'static) {
    vestra_win32::on_session_end(move |what| {
        ended(match what {
            vestra_win32::SessionEnd::Query => super::SessionEvent::Query,
            vestra_win32::SessionEnd::Ending => super::SessionEvent::Ending,
            vestra_win32::SessionEnd::Cancelled => super::SessionEvent::Cancelled,
        });
    });
}

/// The host log's memory line: this process's private bytes and the system
/// commit charge.
pub fn memory_status() -> Option<super::MemoryStatus> {
    vestra_win32::memory_status().map(|status| super::MemoryStatus {
        private_bytes: status.private_bytes,
        commit_used: status.commit_used,
        commit_limit: status.commit_limit,
        physical_load: Some(status.physical_load),
    })
}

/// Installed physical memory, in bytes (`GlobalMemoryStatusEx`).
pub fn physical_memory() -> Option<u64> {
    vestra_win32::physical_memory().map(|memory| memory.total)
}

/// Memory pressure (supervisor.md §8 "Pressure source"): Chrome's
/// thresholds on available physical memory (`GlobalMemoryStatusEx`), and
/// critical while the system's low-memory notification is signaled
/// (`CreateMemoryResourceNotification`), both through `vestra-win32`.
#[derive(Debug)]
pub struct PressureSource {
    low_memory: Option<vestra_win32::LowMemoryNotification>,
}
impl Default for PressureSource {
    fn default() -> Self {
        Self::new()
    }
}
impl PressureSource {
    pub fn new() -> Self {
        Self {
            low_memory: vestra_win32::LowMemoryNotification::new(),
        }
    }
    /// The level now; `None` when the system does not say.
    pub fn level(&mut self) -> Option<super::MemoryPressure> {
        let memory = vestra_win32::physical_memory()?;
        let level = super::pressure_from_available(memory.total >> 20, memory.available >> 20);
        let low = self
            .low_memory
            .as_ref()
            .and_then(vestra_win32::LowMemoryNotification::signaled)
            .unwrap_or(false);
        Some(if low {
            super::MemoryPressure::Critical
        } else {
            level
        })
    }
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

/// End this process now with `code` (`TerminateProcess` on itself): no
/// atexit handlers and no DLL detach, which a driver stuck after a device
/// loss could hang in as `std::process::exit` would.
pub fn exit_now(code: u32) -> ! {
    vestra_win32::terminate_current_process(code)
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
    let adapters = match vestra_win32::dxgi_adapters() {
        Ok(adapters) => adapters,
        Err(error) => {
            // Not "no NVIDIA GPU": the GPUs could not be listed at all.
            let message = format!("the graphics adapters cannot be listed ({error})");
            diag!("upscaling: {message}");
            return Err(super::UpscalerRefusal {
                code: "gpu-unsupported",
                message,
            });
        }
    };
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

/// `%LOCALAPPDATA%\Vestra`: data the player can make again (mpv's compiled
/// shaders), kept out of the roaming profile.
pub fn app_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("Vestra"))
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

    /// The show sequence of `cloak_until_drawn`: a window cloaked at its show
    /// is uncloaked by its first draw, once; a window never cloaked is left
    /// alone; one that does not draw within the limit is shown anyway, and
    /// one that drew is not shown a second time by the limit.
    #[test]
    fn a_cloaked_window_shows_at_its_first_draw_or_after_the_limit() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let limit = Duration::from_millis(500);
        let mut cloaked = Cloaked::default();
        assert_eq!(cloaked.drawn(7), None, "never cloaked");
        cloaked.add(7, start);
        cloaked.add(9, start + Duration::from_millis(100));
        assert_eq!(cloaked.drawn(7), Some(start));
        assert_eq!(cloaked.drawn(7), None, "uncloaked once");
        assert!(
            cloaked
                .overdue(start + Duration::from_millis(599), limit)
                .is_empty()
        );
        assert_eq!(
            cloaked.overdue(start + Duration::from_millis(600), limit),
            vec![9]
        );
        assert!(cloaked.is_empty());
        assert_eq!(cloaked.drawn(9), None, "already shown by the limit");
        // Shown again (hidden in between): waited for from the new show.
        cloaked.add(7, start);
        cloaked.add(7, start + Duration::from_secs(1));
        assert!(
            cloaked
                .overdue(start + Duration::from_millis(1400), limit)
                .is_empty()
        );
        assert_eq!(cloaked.drawn(7), Some(start + Duration::from_secs(1)));
    }

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

    /// Windows Settings > System > Display > Graphics writes
    /// `GpuPreference=N;` per program; `0` is "Let Windows decide".
    #[test]
    fn the_settings_gpu_preference_is_read_from_its_value() {
        assert_eq!(gpu_preference("GpuPreference=2;"), Some(2));
        assert_eq!(gpu_preference("GpuPreference=1;"), Some(1));
        assert_eq!(gpu_preference("GpuPreference=0;"), Some(0));
        assert_eq!(
            gpu_preference("SwapEffectUpgradeEnable=1;GpuPreference=2;"),
            Some(2)
        );
        assert_eq!(gpu_preference(" gpupreference = 1 ;"), Some(1));
        assert_eq!(gpu_preference("SwapEffectUpgradeEnable=1;"), None);
        assert_eq!(gpu_preference(""), None);
        assert_eq!(gpu_preference("GpuPreference=fast;"), None);
        // Only a choice counts as explicit.
        assert_eq!(explicit_gpu_preference("GpuPreference=0;"), None);
        assert_eq!(explicit_gpu_preference("GpuPreference=1;"), Some(1));
        assert_eq!(explicit_gpu_preference("GpuPreference=2;"), Some(2));
        assert_eq!(explicit_gpu_preference("AutoHDREnable=1;"), None);
    }

    /// User ruling 2026-10-31 (record §12): GPU rendering defaults to the
    /// high-performance GPU, unless the user chose one for Vestra in
    /// Windows Settings; CPU mode follows Windows' order;
    /// `VESTRA_GPU_ADAPTER` overrides both; an unknown value of it is
    /// ignored (and does not switch the default off).
    #[test]
    fn gpu_rendering_defaults_to_the_high_performance_gpu() {
        use vestra_gpu_render::Choice;
        let none = || None;
        assert_eq!(adapter_policy(None, true, none), Policy::HighPerformance);
        for preference in [1, 2] {
            let Policy::Order(how) = adapter_policy(None, true, || Some(preference)) else {
                panic!("{preference}: not Windows' order");
            };
            assert!(how.contains("Windows Settings"), "{how}");
        }
        let how = |preference| match adapter_policy(None, true, || Some(preference)) {
            Policy::Order(how) => how,
            other => panic!("{other:?}"),
        };
        assert!(how(2).contains("High performance"));
        assert!(how(1).contains("Power saving"));
        // CPU mode never reads the preference.
        assert_eq!(
            adapter_policy(None, false, || panic!("not read")),
            Policy::Order(ORDER.into())
        );
        // The environment wins in both modes, preference or not.
        let (one, warning) = env_adapter(Some(" 1 "));
        assert_eq!(warning, None);
        for gpu_mode in [true, false] {
            assert_eq!(
                adapter_policy(one.clone(), gpu_mode, || Some(2)),
                Policy::Env(Choice::Index(1), "VESTRA_GPU_ADAPTER=1".into())
            );
        }
        assert_eq!(
            env_adapter(Some("minimum-power"))
                .0
                .map(|(choice, _)| choice),
            Some(Choice::MinimumPower)
        );
        assert_eq!(
            env_adapter(Some("warp")).0.map(|(choice, _)| choice),
            Some(Choice::Warp)
        );
        // Empty is unset; an unknown value is ignored with a line.
        assert_eq!(env_adapter(Some("")), (None, None));
        let (unknown, warning) = env_adapter(Some("nvidia"));
        assert_eq!(unknown, None);
        assert!(warning.unwrap().contains("VESTRA_GPU_ADAPTER=nvidia"));
        assert_eq!(adapter_policy(unknown, true, none), Policy::HighPerformance);
    }

    fn adapter(luid: u64, software: bool) -> crate::platform::GpuAdapter {
        crate::platform::GpuAdapter {
            name: format!("GPU {luid}"),
            vendor_id: 0x1002,
            device_id: 0x15BF,
            driver_version: None,
            software,
            luid,
        }
    }

    /// Review items 2 and 3 (record §12): the adapter is resolved once to a
    /// concrete one (its index for the guard and the denylist, its LUID for
    /// the device); the high-performance default falls back to Windows'
    /// order when it cannot be resolved, an explicit choice does not.
    #[test]
    fn the_chosen_adapter_is_resolved_once_with_a_fallback_for_the_default() {
        use vestra_gpu_render::Choice;
        let list = [adapter(7, false), adapter(9, false), adapter(3, true)];
        let found = |luid: u64| move |_: bool| Ok::<u64, String>(luid);
        let failed = |_: bool| Err::<u64, String>("no IDXGIFactory6".into());
        // The default: the 4070 (LUID 9) by its LUID.
        let pick = resolve_adapter(&Policy::HighPerformance, &list, found(9));
        assert_eq!(
            (pick.index, pick.luid, pick.warp),
            (Some(1), Some(9), false)
        );
        assert!(pick.how.contains("high-performance GPU"), "{}", pick.how);
        // It cannot be resolved: Windows' order, and why.
        for broken in [
            resolve_adapter(&Policy::HighPerformance, &list, failed),
            resolve_adapter(&Policy::HighPerformance, &list, found(42)),
        ] {
            assert_eq!((broken.index, broken.luid), (Some(0), Some(7)));
            assert!(broken.how.starts_with("Windows' order"), "{}", broken.how);
            assert!(
                broken.how.contains("could not be determined"),
                "{}",
                broken.how
            );
        }
        // Windows' order; no adapters at all.
        let order = resolve_adapter(&Policy::Order(ORDER.into()), &list, failed);
        assert_eq!((order.index, order.luid), (Some(0), Some(7)));
        assert_eq!(
            resolve_adapter(&Policy::Order(ORDER.into()), &[], failed).index,
            None
        );
        // Explicit choices: no fallback.
        let env = |choice| Policy::Env(choice, "VESTRA_GPU_ADAPTER=x".into());
        assert_eq!(
            resolve_adapter(&env(Choice::Index(1)), &list, failed).luid,
            Some(9)
        );
        assert_eq!(
            resolve_adapter(&env(Choice::Index(5)), &list, failed).index,
            None
        );
        assert_eq!(
            resolve_adapter(&env(Choice::MinimumPower), &list, found(7)).index,
            Some(0)
        );
        let unresolved = resolve_adapter(&env(Choice::HighPerformance), &list, failed);
        assert_eq!((unresolved.index, unresolved.luid), (None, None));
        assert!(
            unresolved.how.contains("could not be resolved"),
            "{}",
            unresolved.how
        );
        let warp = resolve_adapter(&env(Choice::Warp), &list, failed);
        assert_eq!((warp.index, warp.warp), (Some(2), true));
    }

    /// Review item 6: the setting may name the player by another form of
    /// its path; values are matched by the comparison given.
    #[test]
    fn a_settings_entry_is_found_under_another_form_of_the_path() {
        let entries = vec![
            (
                r"C:\Games\other.exe".to_owned(),
                "GpuPreference=2;".to_owned(),
            ),
            (
                r"C:\PROGRA~1\Vestra\vestra-host.exe".to_owned(),
                "GpuPreference=1;".to_owned(),
            ),
        ];
        let same = |name: &str| name.to_lowercase().contains("progra~1");
        let (name, value) = matching_entry(&entries, same).unwrap();
        assert!(name.contains("PROGRA~1"));
        assert_eq!(explicit_gpu_preference(value), Some(1));
        assert!(matching_entry(&entries, |_| false).is_none());
        // Only paths of the same file name are looked at.
        assert!(same_file_name(
            r"C:\PROGRA~1\Vestra\VESTRA-HOST.exe",
            "vestra-host.exe"
        ));
        assert!(same_file_name(
            r"\\server\share\vestra-host.exe",
            "vestra-host.exe"
        ));
        assert!(!same_file_name(r"C:\Games\other.exe", "vestra-host.exe"));
        assert!(!same_file_name(
            "DirectXUserGlobalSettings",
            "vestra-host.exe"
        ));
        assert!(
            !same_file_name("vestra-host.exe", "vestra-host.exe"),
            "no path"
        );
        assert!(!same_file_name(r"C:\x\vestra-host.exe", ""));
        // A path that exists compares in one form; one that does not, not at all.
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            comparable(&exe),
            comparable(&exe.with_file_name(exe.file_name().unwrap()))
        );
        assert_eq!(
            comparable(std::path::Path::new(r"C:\no\such\player.exe")),
            None
        );
    }

    /// Item 6 (record §11): `exact` turns the extra pixel off.
    #[test]
    fn the_fullscreen_match_switch_reads_its_values() {
        assert_eq!(fullscreen_match(None), (Some(1), None));
        assert_eq!(fullscreen_match(Some("taller")), (Some(1), None));
        assert_eq!(fullscreen_match(Some(" exact ")).0, None);
        assert_eq!(fullscreen_match(Some("shorter")).0, Some(-1));
        let (extra, line) = fullscreen_match(Some("off"));
        assert_eq!(extra, Some(1));
        assert!(line.unwrap().contains("ignored"));
    }

    /// F6 (round record 2026-10-windows-render, B.10): a failed reopen of
    /// the display clock is reported once per run of failures.
    #[test]
    fn a_failed_clock_reopen_is_reported_once_per_run() {
        let mut failed = false;
        assert!(!reopen_failure_to_report(&mut failed, true));
        assert!(
            reopen_failure_to_report(&mut failed, false),
            "first failure"
        );
        assert!(!reopen_failure_to_report(&mut failed, false), "repeat");
        assert!(!reopen_failure_to_report(&mut failed, false), "repeat");
        assert!(!reopen_failure_to_report(&mut failed, true), "opened");
        assert!(reopen_failure_to_report(&mut failed, false), "a new run");
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
