//! Experimental current-application native window observation, not a final API.
//! All rectangles are Cocoa logical points, with a lower-left origin in the
//! global screen coordinate system (secondary screens may have negative origins).
//! No backing-pixel conversion, resize, activation or external-app access occurs.
#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostWindowId(i64);

#[derive(Clone, Debug)]
pub struct NativeSnapshot {
    #[cfg(target_os = "macos")]
    numbers: std::collections::BTreeSet<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFacts {
    pub visible_frame: Rect,
    pub full_frame: Rect,
    /// Dimensionless scale; rectangles above and below remain logical points.
    pub backing_scale_factor: f64,
    pub native_frame: Rect,
    pub active: bool,
    /// contentRectForFrameRect(native_frame): the content rectangle in the same
    /// screen coordinates as the supplied native_frame, not contentView-local
    /// bounds. Frame/content differences describe current native window chrome.
    pub content_rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unavailable(pub &'static str);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Unavailable {}

type Result<T> = std::result::Result<T, Unavailable>;

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    fn app() -> Result<objc2::rc::Retained<NSApplication>> {
        let marker = MainThreadMarker::new().ok_or(Unavailable(
            "native window observation requires the main thread",
        ))?;
        Ok(NSApplication::sharedApplication(marker))
    }

    pub fn capture_windows() -> Result<NativeSnapshot> {
        let windows = app()?.windows();
        // Include hidden windows too: showing an existing native window is not
        // evidence that the caller created a new window belonging to this host.
        Ok(NativeSnapshot {
            numbers: windows
                .iter()
                .map(|window| window.windowNumber() as i64)
                .collect(),
        })
    }

    pub fn associate_new(before: &NativeSnapshot) -> Result<HostWindowId> {
        let windows = app()?.windows();
        let mut added = windows.iter().filter(|window| {
            window.isVisible() && !before.numbers.contains(&(window.windowNumber() as i64))
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

    pub fn screen_for(id: HostWindowId) -> Result<ScreenFacts> {
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
        let screen = window
            .screen()
            .ok_or(Unavailable("associated native window has no screen"))?;
        let visible = screen.visibleFrame();
        let full = screen.frame();
        let frame = window.frame();
        let content = window.contentRectForFrameRect(frame);
        let facts = ScreenFacts {
            visible_frame: Rect {
                x: visible.origin.x,
                y: visible.origin.y,
                width: visible.size.width,
                height: visible.size.height,
            },
            full_frame: Rect {
                x: full.origin.x,
                y: full.origin.y,
                width: full.size.width,
                height: full.size.height,
            },
            backing_scale_factor: screen.backingScaleFactor(),
            active: window.isKeyWindow(),
            native_frame: Rect {
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.size.width,
                height: frame.size.height,
            },
            content_rect: Rect {
                x: content.origin.x,
                y: content.origin.y,
                width: content.size.width,
                height: content.size.height,
            },
        };
        for rect in [
            facts.visible_frame,
            facts.full_frame,
            facts.native_frame,
            facts.content_rect,
        ] {
            if !rect.x.is_finite()
                || !rect.y.is_finite()
                || !rect.width.is_finite()
                || !rect.height.is_finite()
                || rect.width <= 0.0
                || rect.height <= 0.0
            {
                return Err(Unavailable(
                    "native window or screen geometry is unavailable",
                ));
            }
        }
        if !facts.backing_scale_factor.is_finite() || facts.backing_scale_factor <= 0.0 {
            return Err(Unavailable("native screen backing scale is unavailable"));
        }
        Ok(facts)
    }
}

/// Capture immediately before showing a newly created host window. Association
/// depends on a unique creation interval; concurrent creation is explicitly an
/// error. Native IDs are process/lifetime facts, never persistent identifiers.
#[cfg(target_os = "macos")]
pub use native::{associate_new, capture_windows, screen_for};

#[cfg(not(target_os = "macos"))]
pub fn capture_windows() -> Result<NativeSnapshot> {
    Err(Unavailable(
        "native window observation is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn associate_new(_before: &NativeSnapshot) -> Result<HostWindowId> {
    Err(Unavailable(
        "native window association is unavailable on this platform",
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn screen_for(_id: HostWindowId) -> Result<ScreenFacts> {
    Err(Unavailable(
        "native screen facts are unavailable on this platform",
    ))
}
