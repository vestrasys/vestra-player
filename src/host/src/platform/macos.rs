//! macOS: AppKit through objc2 (windows, pasteboard, alerts), the system's
//! `caffeinate` (sleep assertions) and `open` (Finder, default apps and
//! browser), the login keychain (through keyring), NSProcessInfo and
//! `sysctl` (system facts). Window observation covers this application's own
//! windows only: no activation, no other applications.

use super::{
    Display, HostWindowId, NativeSnapshot, OsFacts, Rect, Result, ScreenFacts, Unavailable,
};
use crate::clipboard::Image;
use crate::host::RpcError;
use crate::power::{Assertion, Hold, Launcher};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub use super::keyring_store::{secret_delete, secret_get, secret_set};

/// How menus write shortcuts (`bindings::label`).
pub const KEY_LABELS: super::KeyLabels = super::KeyLabels::Symbols;
/// Which standard window shortcuts apply (`shortcuts.rs`).
pub const SYSTEM: super::System = super::System::MacOs;
/// Slint reports ⌘ as `control` and ⌃ as `meta` on Apple platforms.
pub const SLINT_CONTROL_IS_COMMAND: bool = true;
/// A `Ctrl` binding also fires with ⌘ (bindings written for both systems).
pub const CTRL_ACCEPTS_CMD: bool = true;
/// Menus show ⌘ bindings (`bindings::label_shown`).
pub const MENU_SHOWS_CMD: bool = true;
/// No fullscreen title strip: a fullscreen space shows the system menu
/// bar and the window title bar when the pointer reaches the top.
pub const FULLSCREEN_TITLE_STRIP: bool = false;
/// `confirm` shows a system alert (an `NSAlert` sheet).
pub const SYSTEM_ALERTS: bool = true;

// ------------------------------------------------------------------ windows

fn app() -> Result<objc2::rc::Retained<NSApplication>> {
    let marker = MainThreadMarker::new().ok_or(Unavailable(
        "native window observation requires the main thread",
    ))?;
    Ok(NSApplication::sharedApplication(marker))
}

/// AppKit lists the application's windows itself; a new tool window gets
/// its level at the next [`tool_windows_tick`].
pub fn shown(_instance: &slint_interpreter::ComponentInstance) {
    TOOLS_APPLIED.set(None);
}

// ------------------------------------------------------------- tool windows

thread_local! {
    /// The main window's component (`set_main_window`).
    static MAIN: std::cell::RefCell<Option<slint::Weak<slint_interpreter::ComponentInstance>>> =
        const { std::cell::RefCell::new(None) };
    /// `set_floating`.
    static FLOATING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// What [`tool_windows_tick`] applied last: the player active, floating,
    /// the number of windows and the main window's view.
    static TOOLS_APPLIED: std::cell::Cell<Option<(bool, bool, usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

/// `instance` is the main window's component (at the start, and after each
/// composition change); every other window of winit's is a tool window
/// ([`tool_windows_tick`]).
pub fn set_main_window(instance: &slint_interpreter::ComponentInstance) {
    use slint::ComponentHandle;
    MAIN.set(Some(instance.as_weak()));
    TOOLS_APPLIED.set(None);
}

/// Windows are created as Slint asks: AppKit has no owner at creation;
/// tool windows get their level from [`tool_windows_tick`].
pub fn window_attributes(
    attributes: slint::winit_030::winit::window::WindowAttributes,
) -> slint::winit_030::winit::window::WindowAttributes {
    attributes
}

/// The address of the main window's content view (winit's view), which
/// tells its NSWindow from the others.
fn main_view() -> Option<usize> {
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let instance = MAIN.with_borrow(|main| main.as_ref()?.upgrade())?;
    instance
        .window()
        .with_winit_window(|native| match native.window_handle().ok()?.as_raw() {
            RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as usize),
            _ => None,
        })
        .flatten()
}

/// The level of a tool window: one level above the main window's while the
/// player is the active application, so a click in the main window leaves
/// it in front (what floating panels do in Preview or VLC); the main
/// window's own while another application is active, so it does not float
/// above that application's windows (unless the player's windows all do:
/// always on top, `set_floating`).
fn tool_level(active: bool, floating: bool) -> objc2_app_kit::NSWindowLevel {
    use objc2_app_kit::{NSFloatingWindowLevel, NSNormalWindowLevel};
    match (active, floating) {
        (true, false) => NSFloatingWindowLevel,
        (true, true) => NSFloatingWindowLevel + 1,
        (false, false) => NSNormalWindowLevel,
        (false, true) => NSFloatingWindowLevel,
    }
}

/// Whether `window` is one of winit's (the host's Slint windows, not an
/// alert, a sheet or the picture-in-picture source window) other than the
/// main window (content view `main_view`).
fn is_tool_window(window: &objc2_app_kit::NSWindow, main_view: usize) -> bool {
    let winit = window.class().name().to_bytes() == b"WinitWindow";
    let view = window
        .contentView()
        .map(|view| objc2::rc::Retained::as_ptr(&view) as usize);
    winit && view.is_some_and(|view| view != main_view)
}

/// Keep the tool windows (a plugin's window, Options, the floating sidebar)
/// above the main window while the player is active ([`tool_level`]):
/// AppKit has no owned windows, and a child window (`addChildWindow`)
/// would move with the main window. Checked every tick (cheap), applied
/// when the activation, the always-on-top setting or the windows change.
/// Nothing before the main window exists.
pub fn tool_windows_tick() {
    let Ok(app) = app() else { return };
    let Some(main) = main_view() else { return };
    let windows = app.windows();
    let state = (app.isActive(), FLOATING.get(), windows.count(), main);
    if TOOLS_APPLIED.replace(Some(state)) == Some(state) {
        return;
    }
    let level = tool_level(state.0, state.1);
    for window in windows.iter() {
        if is_tool_window(&window, main) && window.level() != level {
            window.setLevel(level);
        }
    }
}

/// Whether the key window is a tool window (one of winit's other than the
/// main window: a plugin's window, Options, the floating sidebar).
/// [`confirm`] gives its sheet to that window, and otherwise to the main
/// window: this is the same test (the visible key window without a sheet).
pub fn tool_window_focused() -> bool {
    let Ok(app) = app() else { return false };
    let Some(main) = main_view() else {
        return false;
    };
    app.keyWindow().is_some_and(|window| {
        window.isVisible() && window.attachedSheet().is_none() && is_tool_window(&window, main)
    })
}

/// Whether the player is the active application (the user works in one of
/// its windows).
pub fn app_in_foreground() -> bool {
    app().is_ok_and(|app| app.isActive())
}

/// Minimize `window` (Slint's own minimize: no winit fullscreen quirk here).
pub fn minimize_window(window: &slint::Window) {
    window.set_minimized(true);
}

/// Bring `window` to the front with the focus while the player is the
/// active application: winit's `focus_window` (key window, ordered front).
pub fn focus_own_window(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|native| native.focus_window());
}

/// Capture immediately before showing a newly created host window. Hidden
/// windows count too: showing an existing native window is not evidence that
/// the caller created a new one.
pub fn capture_windows() -> Result<NativeSnapshot> {
    let windows = app()?.windows();
    Ok(NativeSnapshot {
        ids: windows
            .iter()
            .map(|window| window.windowNumber() as i64)
            .collect(),
    })
}

/// The one window that became visible since `before`. Association depends on
/// a unique creation interval; concurrent creation is an error.
pub fn associate_new(before: &NativeSnapshot) -> Result<HostWindowId> {
    let windows = app()?.windows();
    let mut added = windows.iter().filter(|window| {
        window.isVisible() && !before.ids.contains(&(window.windowNumber() as i64))
    });
    let first = added
        .next()
        .ok_or(Unavailable("no newly visible native window was created"))?;
    if added.next().is_some() {
        return Err(Unavailable(
            "multiple newly visible native windows prevent unique association",
        ));
    }
    Ok(HostWindowId(first.windowNumber() as i64))
}

fn window(id: HostWindowId) -> Result<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    let windows = app()?.windows();
    let mut matches = windows
        .iter()
        .filter(|window| window.windowNumber() as i64 == id.0);
    let window = matches
        .next()
        .ok_or(Unavailable("associated native window no longer exists"))?;
    if matches.next().is_some() {
        return Err(Unavailable("native window identity is not unique"));
    }
    Ok(window)
}

// ------------------------------------------------------- picture-in-picture

/// AVKit's sample-buffer picture-in-picture (macOS 14+).
pub fn pip_supported() -> bool {
    vestra_pip::Pip::supported()
}

/// The system picture-in-picture window and its source layer, which sits in
/// a borderless child window below the main window, on the video area (see
/// `vestra-pip`). UI thread only.
pub struct PipWindow(vestra_pip::Pip);

impl PipWindow {
    pub fn new() -> std::result::Result<Self, RpcError> {
        vestra_pip::Pip::new().map(Self).ok_or_else(|| {
            RpcError::new(
                "unsupported",
                crate::i18n::tr("Picture-in-picture needs macOS 14 or later"),
            )
        })
    }
}

impl crate::ui::pip::Surface for PipWindow {
    type Frame = GpuFrame;
    fn place(&self, id: HostWindowId, area: Rect) -> bool {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        let area = NSRect::new(
            NSPoint::new(area.x, area.y),
            NSSize::new(area.width, area.height),
        );
        window(id).is_ok_and(|window| self.0.place(&window, area))
    }
    fn push_rgb(&self, width: usize, height: usize, rgb: &[u8]) -> std::result::Result<(), String> {
        self.0.enqueue(width, height, rgb)
    }
    fn describe(&self, id: HostWindowId) -> String {
        window(id).map_or_else(|e| e.to_string(), |window| self.0.describe(&window))
    }
    /// Zero copy: the GPU renderer's BGRA IOSurface, wrapped as is. The
    /// renderer does not draw into a surface the system still shows.
    fn push_surface(&self, surface: &GpuFrame) -> std::result::Result<(), String> {
        self.0.enqueue_surface(surface)
    }
    fn start(&self) -> bool {
        self.0.start()
    }
    fn stop(&self) {
        self.0.stop()
    }
    fn set_playback(&self, playback: &crate::ui::pip::Playback) {
        self.0.set_playback(
            playback.paused,
            playback.position,
            playback.duration,
            playback.rate,
        )
    }
    fn events(&self) -> Vec<super::PipEvent> {
        use super::PipEvent;
        self.0
            .events()
            .into_iter()
            .map(|event| match event {
                vestra_pip::Event::Pause(pause) => PipEvent::Pause(pause),
                vestra_pip::Event::Skip(seconds) => PipEvent::Skip(seconds),
                vestra_pip::Event::Started => PipEvent::Started,
                vestra_pip::Event::Stopped { restored } => PipEvent::Stopped { restored },
                vestra_pip::Event::Failed(reason) => PipEvent::Failed(reason),
            })
            .collect()
    }
}

/// Title bars (and the system's alerts and panels) dark or light, following
/// the player's palette rather than the system setting: the application's
/// appearance, which every window, including later ones, inherits.
pub fn set_dark_title_bars(dark: bool) {
    use objc2_app_kit::NSAppearance;
    use objc2_foundation::NSString;
    let Ok(app) = app() else { return };
    // The values of NSAppearanceNameDarkAqua / NSAppearanceNameAqua (reading
    // the extern statics would need `unsafe`).
    let name = if dark {
        "NSAppearanceNameDarkAqua"
    } else {
        "NSAppearanceNameAqua"
    };
    let appearance = NSAppearance::appearanceNamed(&NSString::from_str(name));
    app.setAppearance(appearance.as_deref());
}

/// Every ordinary window of this application floats above other apps'
/// windows (or returns to the normal level). Panels and other levels are
/// left alone, and so are the tool windows, whose level follows the main
/// window's ([`tool_windows_tick`]). Returns how many windows the
/// application has.
pub fn set_floating(on: bool) -> Result<usize> {
    use objc2_app_kit::{NSFloatingWindowLevel, NSNormalWindowLevel};
    let windows = app()?.windows();
    FLOATING.set(on);
    let main = main_view();
    let (from, to) = if on {
        (NSNormalWindowLevel, NSFloatingWindowLevel)
    } else {
        (NSFloatingWindowLevel, NSNormalWindowLevel)
    };
    for window in windows.iter() {
        let tool = main.is_some_and(|main| is_tool_window(&window, main));
        if window.level() == from && !tool {
            window.setLevel(to);
        }
    }
    tool_windows_tick();
    Ok(windows.count())
}

pub fn window_count() -> usize {
    app().map_or(0, |app| app.windows().count())
}

/// Resize the content to `width`×`height` points keeping the top-left
/// corner, then move the frame back inside the screen's visible area.
pub fn resize_content(id: HostWindowId, width: f64, height: f64) -> Result<()> {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let window = window(id)?;
    let screen = window
        .screen()
        .ok_or(Unavailable("associated native window has no screen"))?;
    let visible = screen.visibleFrame();
    let content = window.contentRectForFrameRect(window.frame());
    let top = content.origin.y + content.size.height;
    let wanted = NSRect::new(
        NSPoint::new(content.origin.x, top - height),
        NSSize::new(width, height),
    );
    let mut frame = window.frameRectForContentRect(wanted);
    let right = visible.origin.x + visible.size.width;
    let ceiling = visible.origin.y + visible.size.height;
    if frame.origin.x + frame.size.width > right {
        frame.origin.x = right - frame.size.width;
    }
    frame.origin.x = frame.origin.x.max(visible.origin.x);
    if frame.origin.y + frame.size.height > ceiling {
        frame.origin.y = ceiling - frame.size.height;
    }
    frame.origin.y = frame.origin.y.max(visible.origin.y);
    window.setFrame_display(frame, true);
    Ok(())
}

/// The pointer in the window's content, top-left origin (logical points).
/// AppKit tracks it during a file drag, when winit reports no motion.
pub fn pointer_in(id: HostWindowId) -> Result<(f64, f64)> {
    let window = window(id)?;
    let point = window.mouseLocationOutsideOfEventStream();
    let content = window.contentRectForFrameRect(window.frame());
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(Unavailable("pointer location is unavailable"));
    }
    Ok((point.x, content.size.height - point.y))
}

/// Option, Shift and Command held right now (`NSEvent.modifierFlags`).
pub fn modifier_keys() -> (bool, bool, bool) {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};
    let flags = NSEvent::modifierFlags_class();
    (
        flags.contains(NSEventModifierFlags::Option),
        flags.contains(NSEventModifierFlags::Shift),
        flags.contains(NSEventModifierFlags::Command),
    )
}

/// The system double-click interval in seconds.
pub fn double_click_seconds() -> f64 {
    objc2_app_kit::NSEvent::doubleClickInterval()
}

/// Whether the user lets the interface animate: Accessibility ▸ Display ▸
/// Reduce motion is off.
pub fn system_animations() -> bool {
    !objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

pub fn screen_for(id: HostWindowId) -> Result<ScreenFacts> {
    let window = window(id)?;
    let screen = window
        .screen()
        .ok_or(Unavailable("associated native window has no screen"))?;
    let rect = |r: objc2_foundation::NSRect| Rect {
        x: r.origin.x,
        y: r.origin.y,
        width: r.size.width,
        height: r.size.height,
    };
    let frame = window.frame();
    ScreenFacts {
        visible_frame: rect(screen.visibleFrame()),
        full_frame: rect(screen.frame()),
        backing_scale_factor: screen.backingScaleFactor(),
        active: window.isKeyWindow(),
        native_frame: rect(frame),
        content_rect: rect(window.contentRectForFrameRect(frame)),
    }
    .checked()
}

// ---------------------------------------------------------------- clipboard

const TEXT: &str = "public.utf8-plain-text";

fn uti(kind: Image) -> &'static str {
    match kind {
        Image::Png => "public.png",
        Image::Jpeg => "public.jpeg",
    }
}
fn refused() -> RpcError {
    RpcError::new("failed", "the system refused the pasteboard write")
}
pub fn clipboard_read_text() -> std::result::Result<Option<String>, RpcError> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    Ok(NSPasteboard::generalPasteboard()
        .stringForType(&NSString::from_str(TEXT))
        .map(|text| text.to_string()))
}
pub fn clipboard_write_text(text: &str) -> std::result::Result<(), RpcError> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board
        .setString_forType(&NSString::from_str(text), &NSString::from_str(TEXT))
        .then_some(())
        .ok_or_else(refused)
}
/// The file's bytes as they are, under the image's own type.
pub fn clipboard_write_image(kind: Image, bytes: Vec<u8>) -> std::result::Result<(), RpcError> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    let data = NSData::from_vec(bytes);
    board
        .setData_forType(Some(&data), &NSString::from_str(uti(kind)))
        .then_some(())
        .ok_or_else(refused)
}

// ------------------------------------------------------------------ dialogs

/// The NSWindow of a host window: the one whose content view is its winit
/// view (as [`main_view`] tells the main window).
fn ns_window_of(window: &slint::Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let view = window
        .with_winit_window(|native| match native.window_handle().ok()?.as_raw() {
            RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as usize),
            _ => None,
        })
        .flatten()?;
    app().ok()?.windows().iter().find(|each| {
        each.contentView()
            .is_some_and(|content| objc2::rc::Retained::as_ptr(&content) as usize == view)
    })
}

/// Show an NSAlert; `done(confirmed, checked)` runs on the main thread when
/// the user answered. A sheet on `owner` (the tool window the request came
/// from, `host/origins`), else on the active host window (a tool window
/// that asked, the floating sidebar), else on `parent`, else app-modal
/// (`done` then runs before this returns): the sheet never hides its caller.
pub fn confirm(
    options: &crate::ui::confirm::Options,
    owner: Option<&slint::Window>,
    parent: Option<HostWindowId>,
    done: impl Fn(bool, bool) + Send + 'static,
) -> std::result::Result<(), RpcError> {
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSControlStateValueOn};
    use objc2_foundation::NSString;
    let marker = MainThreadMarker::new()
        .ok_or_else(|| RpcError::new("unavailable", "dialogs need the main thread"))?;
    let alert = NSAlert::new(marker);
    alert.setMessageText(&NSString::from_str(&options.message));
    if let Some(detail) = &options.detail {
        alert.setInformativeText(&NSString::from_str(detail));
    }
    let confirm = alert.addButtonWithTitle(&NSString::from_str(&options.confirm));
    confirm.setHasDestructiveAction(options.destructive);
    let cancel = alert.addButtonWithTitle(&NSString::from_str(&options.cancel));
    cancel.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let checkbox = options.checkbox.as_ref().and_then(|title| {
        alert.setShowsSuppressionButton(true);
        let button = alert.suppressionButton()?;
        button.setTitle(&NSString::from_str(title));
        Some(button)
    });
    let answer = move |response: isize| {
        let checked = checkbox
            .as_ref()
            .is_some_and(|button| button.state() == NSControlStateValueOn);
        done(response == NSAlertFirstButtonReturn, checked);
    };
    let free =
        |window: &objc2_app_kit::NSWindow| window.isVisible() && window.attachedSheet().is_none();
    let origin = owner.and_then(ns_window_of).filter(|window| free(window));
    let key = NSApplication::sharedApplication(marker)
        .keyWindow()
        .filter(|window| free(window));
    match origin
        .or(key)
        .or_else(|| parent.and_then(|id| window(id).ok()))
    {
        Some(window) => {
            let handler = block2::RcBlock::new(answer);
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
        }
        None => answer(alert.runModal()),
    }
    Ok(())
}

/// `dialog` (rfd's) as a sheet on `owner`'s window, the window the request
/// came from (`origins::dialog_place`); without the owner rfd would attach
/// it to the application's main window, else its first. A sheet stays a
/// sheet: rfd runs it app-modally only when the application is not
/// running yet.
pub fn own_file_dialog(
    dialog: rfd::AsyncFileDialog,
    owner: Option<&slint::Window>,
) -> rfd::AsyncFileDialog {
    use slint::winit_030::WinitWindowAccessor;
    let Some(owner) = owner else {
        return dialog;
    };
    owner
        .with_winit_window(|native| dialog.clone().set_parent(native))
        .unwrap_or(dialog)
}

/// Bring the open confirmation or file dialog into view (another request
/// was refused `busy`, or a key came while it is open): the window its
/// `NSAlert` or file panel sheet is attached to comes to the front as the
/// key window while the player is the active application (an app-modal
/// alert is in front already).
pub fn show_open_dialog(foreground: bool) {
    let Ok(app) = app() else { return };
    if !foreground {
        return;
    }
    if let Some(window) = app
        .windows()
        .iter()
        .find(|window| window.attachedSheet().is_some())
    {
        window.makeKeyAndOrderFront(None);
    }
}

// -------------------------------------------------------------------- power

/// `/usr/bin/caffeinate -d -i -w <host pid>` (video) or `-i -w <pid>`
/// (audio): the assertion ends with the host process even if it crashes.
pub fn caffeinate_args(hold: Hold, pid: u32) -> Vec<String> {
    let mut args = Vec::new();
    if hold.display {
        args.push("-d".to_owned());
    }
    args.extend(["-i".to_owned(), "-w".to_owned(), pid.to_string()]);
    args
}

struct Caffeinate(std::process::Child);
impl Assertion for Caffeinate {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn sleep_launcher() -> Launcher {
    let pid = std::process::id();
    Box::new(move |hold| {
        let child = Command::new("/usr/bin/caffeinate")
            .args(caffeinate_args(hold, pid))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| std::io::Error::new(error.kind(), format!("caffeinate: {error}")))?;
        Ok(Box::new(Caffeinate(child)) as Box<dyn Assertion>)
    })
}

// ---------------------------------------------------------- files and links

const OPEN: &str = "/usr/bin/open";
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// `/usr/bin/open <args>` without a shell: `timeout` after 10 s, `failed`
/// with the last stderr line on a non-zero exit. Every path argument is
/// absolute and the URL starts with its scheme, so none reads as an option.
fn open(args: &[&std::ffi::OsStr]) -> std::result::Result<(), RpcError> {
    use std::io::Read;
    let mut child = Command::new(OPEN)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| RpcError::new("unavailable", format!("{OPEN}: {error}")))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.take(64 * 1024).read_to_string(&mut text);
        }
        text
    });
    let deadline = Instant::now() + OPEN_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RpcError::new(
                    "timeout",
                    format!("{OPEN} ran longer than {} ms", OPEN_TIMEOUT.as_millis()),
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(RpcError::new("failed", format!("{OPEN}: {error}"))),
        }
    };
    let stderr = reader.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let last = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned);
    Err(RpcError::new(
        "failed",
        last.unwrap_or_else(|| format!("{OPEN} exited with {status}")),
    ))
}

/// Select all paths in the Finder with one `open -R`, so several files of one
/// folder are highlighted in the same window.
pub fn reveal(paths: &[PathBuf]) -> std::result::Result<(), RpcError> {
    let mut args = vec![std::ffi::OsStr::new("-R")];
    args.extend(paths.iter().map(|path| path.as_os_str()));
    open(&args)
}
pub fn open_path(path: &Path) -> std::result::Result<(), RpcError> {
    open(&[path.as_os_str()])
}
pub fn open_url(url: &str) -> std::result::Result<(), RpcError> {
    open(&[std::ffi::OsStr::new(url)])
}

// ------------------------------------------------------- opening the player

/// Files LaunchServices asks the running player to open (Finder "Open With",
/// double-click, the Dock icon): open-documents Apple events, at launch too
/// (`vestra-open-files`). Call once on the main thread before the event loop.
pub fn on_open_files(deliver: impl Fn(Vec<PathBuf>) + 'static) {
    if !vestra_open_files::on_open_files(deliver) {
        diag!("open files: the open-documents handler is not installed");
    }
}

/// LaunchServices activates the app it sends documents to; nothing to pass on.
pub fn allow_handoff_foreground() {}

// -------------------------------------------------------------------- facts

/// The product version as `sw_vers -productVersion` prints it (`26.2`,
/// `15.1.1`) and the build from NSProcessInfo, the model from `sysctl`.
pub fn os_facts() -> OsFacts {
    use objc2_foundation::NSProcessInfo;
    let info = NSProcessInfo::processInfo();
    let v = info.operatingSystemVersion();
    let version = if v.patchVersion > 0 {
        format!("{}.{}.{}", v.majorVersion, v.minorVersion, v.patchVersion)
    } else {
        format!("{}.{}", v.majorVersion, v.minorVersion)
    };
    let model = Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.model"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|model| !model.is_empty());
    OsFacts {
        os: "macos",
        name: "macOS".into(),
        version: Some(version),
        build: build_of(&info.operatingSystemVersionString().to_string()),
        model,
    }
}

/// Installed physical memory, in bytes (`NSProcessInfo.physicalMemory`).
pub fn physical_memory() -> Option<u64> {
    Some(objc2_foundation::NSProcessInfo::processInfo().physicalMemory())
}

/// No memory pressure source on macOS yet ("Memory
/// pressure"): the system's pressure events come through a dispatch
/// source, which would need an unsafe crate. The process budget still
/// applies.
#[derive(Debug, Default)]
pub struct PressureSource;
impl PressureSource {
    pub fn new() -> Self {
        Self
    }
    /// Never known here.
    pub fn level(&mut self) -> Option<super::MemoryPressure> {
        None
    }
}

/// Where to put a window before its native window exists so that its
/// frame's top-left corner lands at `frame` (logical points, as
/// `ui_state` saves it). winit takes a creation position as the top-left
/// corner of the *content* (`initWithContentRect:`), so the title bar goes
/// above it: the position moves down by the title bar's height, which AppKit
/// gives for the style winit uses for a decorated window (titled, closable,
/// miniaturizable, resizable; about 28 points). Unchanged off the
/// main thread.
pub fn creation_position(frame: slint::LogicalPosition) -> slint::LogicalPosition {
    use objc2_app_kit::{NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Some(marker) = MainThreadMarker::new() else {
        return frame;
    };
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;
    let content = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 600.));
    let outer = NSWindow::frameRectForContentRect_styleMask(content, style, marker);
    let title_bar = (outer.size.height - content.size.height).max(0.);
    slint::LogicalPosition::new(frame.x, frame.y + title_bar as f32)
}

/// NSScreen's list (the menu-bar screen first) with the names System
/// Settings shows. Main thread only; empty elsewhere.
pub fn displays() -> Vec<Display> {
    use objc2_app_kit::NSScreen;
    let Some(marker) = MainThreadMarker::new() else {
        return Vec::new();
    };
    NSScreen::screens(marker)
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let (frame, scale) = (screen.frame(), screen.backingScaleFactor());
            let fps = screen.maximumFramesPerSecond();
            Display {
                name: Some(screen.localizedName().to_string()).filter(|name| !name.is_empty()),
                width: (frame.size.width * scale).round() as u32,
                height: (frame.size.height * scale).round() as u32,
                scale,
                refresh_rate: (fps > 0).then_some(fps as f64),
                primary: index == 0,
            }
        })
        .collect()
}

/// `Version 26.2 (Build 25C56)` → `25C56`.
pub fn build_of(text: &str) -> Option<String> {
    let rest = text.split("(Build ").nth(1)?;
    let build = rest.split(')').next()?.trim();
    (!build.is_empty()).then(|| build.to_owned())
}

/// `~/Library/Application Support/Vestra`.
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Vestra"))
}

/// `~/Library/Caches/Vestra`: data the player can make again (mpv's
/// compiled shaders).
pub fn app_cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches/Vestra"))
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

// ---------------------------------------------------------------- rendering

/// A frame the GPU renderer published (`GpuHandle::latest`): a BGRA IOSurface.
pub use vestra_gpu_render::IOSurfaceRef as GpuFrame;
/// libmpv draws with OpenGL in its own offscreen CGL context into IOSurfaces;
/// each Slint (femtovg) window copies the newest one into its own texture.
/// The raw GL and CGL calls are in the `vestra-gpu-render` crate.
pub use vestra_gpu_render::{Draw as GpuDraw, Handle as GpuHandle, Renderer as GpuRenderer};
pub const GPU_RENDER: bool = true;
/// GPU mode's windows draw with OpenGL (femtovg) here.
pub const GPU_WINDOWS_WGPU: bool = false;
/// GPU rendering is the default here (`render_mode::Facts::gpu_opt_in`).
pub const GPU_OPT_IN: bool = false;

/// The host's GPU contract, delegated to `vestra-gpu-render`'s own methods.
impl crate::playback::renderer::GpuBackend for GpuHandle {
    type Frame = vestra_gpu_render::Frame;
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
        vestra_gpu_render::Handle::latest(self)
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
}

/// Start drawing `mpv` at `size` pixels; frames are shown while `publish`.
pub fn gpu_start(
    mpv: std::sync::Arc<rsmpv::Mpv>,
    size: (u32, u32),
    publish: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::result::Result<GpuRenderer, String> {
    GpuRenderer::start(mpv, size, publish)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caffeinate_follows_the_hold_and_is_bound_to_the_host() {
        assert_eq!(
            caffeinate_args(Hold { display: true }, 42),
            ["-d", "-i", "-w", "42"]
        );
        assert_eq!(
            caffeinate_args(Hold { display: false }, 42),
            ["-i", "-w", "42"]
        );
    }

    #[test]
    fn facts_name_the_system_and_its_build() {
        assert_eq!(
            build_of("Version 26.2 (Build 25C56)").as_deref(),
            Some("25C56")
        );
        assert_eq!(build_of("Version 26.2"), None);
        let facts = os_facts();
        assert_eq!((facts.os, facts.name.as_str()), ("macos", "macOS"));
        assert!(facts.version.unwrap().split('.').count() >= 2);
        assert!(facts.build.is_some());
    }

    /// Tool windows float one level above the main window while the player
    /// is active, and never above other apps' windows more than the main
    /// window does once it is not.
    #[test]
    fn tool_windows_float_above_the_main_window_only_while_active() {
        use objc2_app_kit::{NSFloatingWindowLevel, NSNormalWindowLevel};
        for floating in [false, true] {
            let main = if floating {
                NSFloatingWindowLevel
            } else {
                NSNormalWindowLevel
            };
            assert!(tool_level(true, floating) > main);
            assert_eq!(tool_level(false, floating), main);
            assert!(tool_level(true, floating) < objc2_app_kit::NSModalPanelWindowLevel);
        }
    }

    #[test]
    fn opening_a_missing_application_path_fails_with_the_reason() {
        let error = open_path(Path::new("/nonexistent/vestra-open-test")).unwrap_err();
        assert_eq!(error.code, "failed");
        assert!(!error.message.is_empty());
    }
}
