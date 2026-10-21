//! macOS: AppKit through objc2 (windows, pasteboard, alerts), the system's
//! `caffeinate` (sleep assertions) and `open` (Finder, default apps and
//! browser), the login keychain (through keyring), NSProcessInfo and
//! `sysctl` (system facts). Window observation covers this application's own
//! windows only: no activation, no other applications.

use super::{
    Display, HostWindowId, NativeSnapshot, OsFacts, Rect, Result, ScreenFacts, Unavailable,
};
use crate::RpcError;
use crate::clipboard::Image;
use crate::power::{Assertion, Hold, Launcher};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub use super::keyring_store::{secret_delete, secret_get, secret_set};

/// Slint reports ⌘ as `control` and ⌃ as `meta` on Apple platforms.
pub const SLINT_CONTROL_IS_COMMAND: bool = true;
/// A `Ctrl` binding also fires with ⌘ (bindings written for both systems).
pub const CTRL_ACCEPTS_CMD: bool = true;

// ------------------------------------------------------------------ windows

fn app() -> Result<objc2::rc::Retained<NSApplication>> {
    let marker = MainThreadMarker::new().ok_or(Unavailable(
        "native window observation requires the main thread",
    ))?;
    Ok(NSApplication::sharedApplication(marker))
}

/// Nothing to record: AppKit lists the application's windows itself.
pub fn shown(_window: &slint::Window) {}

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
                "此系统不支持画中画（需要 macOS 14 或更高版本）",
            )
        })
    }
    /// The zero-copy path for frames the GPU renderer drew into an IOSurface
    /// (BGRA): the merge point with the GPU render line (round 2026-10
    /// render-pip §3).
    #[allow(dead_code)]
    pub fn push_surface(
        &self,
        surface: &vestra_pip::IOSurfaceRef,
    ) -> std::result::Result<(), String> {
        self.0.enqueue_surface(surface)
    }
}

impl crate::pip::Surface for PipWindow {
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
    fn start(&self) -> bool {
        self.0.start()
    }
    fn stop(&self) {
        self.0.stop()
    }
    fn set_playback(&self, playback: &crate::pip::Playback) {
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

/// Every ordinary window of this application floats above other apps'
/// windows (or returns to the normal level). Panels and other levels are
/// left alone. Returns how many windows the application has.
pub fn set_floating(on: bool) -> Result<usize> {
    use objc2_app_kit::{NSFloatingWindowLevel, NSNormalWindowLevel};
    let windows = app()?.windows();
    let (from, to) = if on {
        (NSNormalWindowLevel, NSFloatingWindowLevel)
    } else {
        (NSFloatingWindowLevel, NSNormalWindowLevel)
    };
    for window in windows.iter() {
        if window.level() == from {
            window.setLevel(to);
        }
    }
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

/// Show an NSAlert; `done(confirmed, checked)` runs on the main thread when
/// the user answered. A sheet on the active host window (a tool window that
/// asked, the floating sidebar), else on `parent`, else app-modal (`done`
/// then runs before this returns): the sheet never hides its caller.
pub fn confirm(
    options: &crate::confirm::Options,
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
    let key = NSApplication::sharedApplication(marker)
        .keyWindow()
        .filter(|window| window.isVisible() && window.attachedSheet().is_none());
    match key.or_else(|| parent.and_then(|id| window(id).ok())) {
        Some(window) => {
            let handler = block2::RcBlock::new(answer);
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
        }
        None => answer(alert.runModal()),
    }
    Ok(())
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

/// The UI font that has the CJK glyphs Slint needs (`ui::font`).
pub fn ui_font() -> Option<(&'static str, &'static str)> {
    const HIRAGINO: &str = "/System/Library/Fonts/Hiragino Sans GB.ttc";
    Path::new(HIRAGINO)
        .is_file()
        .then_some(("Hiragino Sans GB", HIRAGINO))
}

/// FFmpeg's name for its stdin, which is the held source file (seekable).
pub const HELD_FILE_INPUT: &str = "/dev/fd/0";

/// The offline workers' FFmpeg when `VESTRA_FFMPEG` is not set.
pub fn default_ffmpeg() -> PathBuf {
    PathBuf::from("/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg")
}

// ---------------------------------------------------------------- rendering

/// libmpv draws with OpenGL in its own offscreen CGL context into IOSurfaces;
/// each Slint (femtovg) window copies the newest one into its own texture.
/// The raw GL and CGL calls are in the `vestra-gpu-render` crate.
pub use vestra_gpu_render::{Handle as GpuHandle, Renderer as GpuRenderer};
pub const GPU_RENDER: bool = true;

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

    #[test]
    fn opening_a_missing_application_path_fails_with_the_reason() {
        let error = open_path(Path::new("/nonexistent/vestra-open-test")).unwrap_err();
        assert_eq!(error.code, "failed");
        assert!(!error.message.is_empty());
    }
}
