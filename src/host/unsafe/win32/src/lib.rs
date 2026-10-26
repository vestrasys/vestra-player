//! The Win32 calls that no mature safe crate wraps for the host's Windows
//! platform module (`src/host/src/platform/windows.rs`): the pointer position
//! while a file is dragged over a window (winit 0.30 reports no position for
//! drags), the modifier keys held right now, the double-click time, the
//! submenu show delay, the thread's sleep-prevention state, a window's
//! dark title bar, letting the running player take the foreground for a
//! second launch's files, waiting for a display's vertical blank
//! (`VBlank`, the redraw clock) and keeping a full-screen window from
//! matching its display exactly (`unmatch_display`). Each is one
//! call with plain integer arguments or one local out-parameter; the public
//! API is safe. Compiled only on Windows (cross-checked from macOS by
//! `npm run portability`, not yet run on Windows).
#![cfg(windows)]

use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMHDC, D3DKMT_WAITFORVERTICALBLANKEVENT,
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromHdc, D3DKMTWaitForVerticalBlankEvent,
};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Foundation::{HWND, LUID, POINT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFOEXW,
    MonitorFromWindow,
};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MONITORINFO};
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetCursorPos, GetWindowRect, SPI_GETMENUSHOWDELAY,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos, SystemParametersInfoW,
};

/// The pointer in screen coordinates (physical pixels), `None` when the
/// system does not report it (for example on a secure desktop).
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid, writable POINT for the duration of the call.
    let ok = unsafe { GetCursorPos(&mut point) };
    (ok != 0).then_some((point.x, point.y))
}

/// Alt, Shift and Ctrl as the current thread's input state has them.
pub fn modifier_keys() -> (bool, bool, bool) {
    // SAFETY: GetKeyState takes a virtual-key code by value and has no
    // memory-safety preconditions. A negative result (high bit) means "down".
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    (down(VK_MENU), down(VK_SHIFT), down(VK_CONTROL))
}

/// The system double-click time in milliseconds.
pub fn double_click_ms() -> u32 {
    // SAFETY: no arguments, no preconditions.
    unsafe { GetDoubleClickTime() }
}

/// The system submenu show delay in milliseconds (`SPI_GETMENUSHOWDELAY`),
/// `None` when the system does not answer.
pub fn menu_show_delay_ms() -> Option<u32> {
    let mut delay: u32 = 0;
    // SAFETY: SPI_GETMENUSHOWDELAY writes one DWORD to the pointer, which
    // points at `delay`, a valid writable u32 for the duration of the call;
    // `uiParam` is unused (0) and no setting is changed (fWinIni 0).
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETMENUSHOWDELAY, 0, (&mut delay as *mut u32).cast(), 0)
    };
    (ok != 0).then_some(delay)
}

/// Keep the system (and, with `display`, the display) awake until
/// [`allow_sleep`] is called on the same thread; the state belongs to the
/// calling thread and ends with it. Returns false when the system refused.
pub fn keep_awake(display: bool) -> bool {
    let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
    if display {
        flags |= ES_DISPLAY_REQUIRED;
    }
    // SAFETY: flags by value; no memory-safety preconditions.
    unsafe { SetThreadExecutionState(flags) != 0 }
}

/// End what [`keep_awake`] started on this thread.
pub fn allow_sleep() {
    // SAFETY: flags by value; no memory-safety preconditions.
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS);
    }
}

/// Draw the title bar of window `hwnd` (a raw HWND value) dark or light,
/// whatever the system setting (DWMWA_USE_IMMERSIVE_DARK_MODE, Windows 10
/// 20H1 and later; the earlier Windows 10 builds used attribute 19 for the
/// same thing). Returns false when the system refused, for example on a
/// version without dark title bars or when `hwnd` is no longer a window.
pub fn set_dark_title_bar(hwnd: isize, dark: bool) -> bool {
    /// DWMWA_USE_IMMERSIVE_DARK_MODE before Windows 10 20H1 (build 19041).
    const BEFORE_20H1: u32 = 19;
    let value: i32 = i32::from(dark);
    let set = |attribute: u32| {
        // SAFETY: DwmSetWindowAttribute reads `size_of::<i32>()` bytes (a
        // BOOL) from the pointer, which points at `value`, alive for the
        // call. An invalid or foreign HWND makes it return an error HRESULT,
        // it does not touch memory through it.
        unsafe {
            DwmSetWindowAttribute(
                hwnd as HWND,
                attribute,
                (&value as *const i32).cast(),
                size_of::<i32>() as u32,
            )
        }
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE as u32) >= 0 || set(BEFORE_20H1) >= 0
}

/// Let another process (the running player a second launch hands its files
/// to) bring its window to the front: the foreground rights of the process
/// the user just started pass on. `false` when the system refused.
pub fn allow_any_foreground() -> bool {
    // SAFETY: AllowSetForegroundWindow takes a process id by value
    // (ASFW_ANY: any process) and has no memory-safety preconditions.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) != 0 }
}

/// When window `hwnd` covers its display exactly (a borderless full-screen
/// window), make it `extra` pixels taller (negative: shorter) at the bottom,
/// so that it no longer matches the display's rectangle. Returns whether it
/// changed the window. Nothing for a window of any other size.
pub fn unmatch_display(hwnd: isize, extra: i32) -> bool {
    // SAFETY: MonitorFromWindow takes the HWND by value and answers the
    // nearest display for any window; it does not touch memory through it.
    let monitor = unsafe { MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    // SAFETY: `info` is a writable MONITORINFO whose cbSize says so.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return false;
    }
    let mut window = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `window` is a writable RECT for the call; an invalid HWND
    // makes it fail without touching memory through it.
    if unsafe { GetWindowRect(hwnd as HWND, &mut window) } == 0 {
        return false;
    }
    let display = info.rcMonitor;
    if (window.left, window.top, window.right, window.bottom)
        != (display.left, display.top, display.right, display.bottom)
    {
        return false;
    }
    let (width, height) = (
        display.right - display.left,
        display.bottom - display.top + extra,
    );
    // SAFETY: SetWindowPos takes the HWND and plain integers by value (no
    // z-order change, no activation); an invalid HWND makes it fail.
    unsafe {
        SetWindowPos(
            hwnd as HWND,
            std::ptr::null_mut(),
            display.left,
            display.top,
            width,
            height,
            SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

/// The vertical blanks of one display, the clock that paces the host's
/// redraws on Windows (`platform::windows`; Slint's winit backend paces them
/// with a timer that is not tied to the display). Waits with
/// D3DKMTWaitForVerticalBlankEvent on the adapter output (video present source) that
/// drives the display, opened from a device context of that display: any
/// refresh rate, and the display's own adapter on hybrid-GPU machines.
pub struct VBlank {
    adapter: u32,
    source: u32,
    /// The display (HMONITOR value) this clock follows.
    monitor: isize,
}

impl VBlank {
    /// The clock of the display that holds most of window `hwnd` (a raw HWND
    /// value; 0 or a stale value: the primary display). `None` when the
    /// display has no kernel adapter to wait on (for example a remote
    /// desktop session).
    pub fn open(hwnd: isize) -> Option<Self> {
        let monitor = monitor_of(hwnd);
        // SAFETY: MONITORINFOEXW is plain data (integers and a UTF-16
        // array); all zeroes is a valid value.
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is a writable MONITORINFOEXW whose cbSize says so,
        // which lets GetMonitorInfoW also write the device name. A stale
        // HMONITOR makes the call fail; it does not touch memory through it.
        if unsafe { GetMonitorInfoW(monitor as HMONITOR, (&raw mut info).cast()) } == 0 {
            return None;
        }
        // SAFETY: `szDevice` is NUL-terminated (zeroed above, and
        // GetMonitorInfoW writes at most its 32 characters including the
        // NUL) and alive for the call; the other pointers may be null for a
        // display device context.
        let dc = unsafe {
            CreateDCW(
                std::ptr::null(),
                info.szDevice.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if dc.is_null() {
            return None;
        }
        let mut open = D3DKMT_OPENADAPTERFROMHDC {
            hDc: dc,
            hAdapter: 0,
            AdapterLuid: LUID {
                LowPart: 0,
                HighPart: 0,
            },
            VidPnSourceId: 0,
        };
        // SAFETY: `open` is a valid, writable D3DKMT_OPENADAPTERFROMHDC with
        // a live display device context; the call fills in the adapter
        // handle and the video present source of that display.
        let status = unsafe { D3DKMTOpenAdapterFromHdc(&mut open) };
        // SAFETY: `dc` came from CreateDCW above and is deleted once; the
        // adapter handle does not depend on it.
        unsafe { DeleteDC(dc) };
        (status >= 0).then_some(Self {
            adapter: open.hAdapter,
            source: open.VidPnSourceId,
            monitor,
        })
    }

    /// Block until the display's next vertical blank. `false` when the wait
    /// failed (the adapter went away, for example after a driver update).
    pub fn wait(&self) -> bool {
        let wait = D3DKMT_WAITFORVERTICALBLANKEVENT {
            hAdapter: self.adapter,
            hDevice: 0,
            VidPnSourceId: self.source,
        };
        // SAFETY: `wait` is a valid D3DKMT_WAITFORVERTICALBLANKEVENT naming
        // the adapter this clock opened (closed only on drop) and its
        // source; the call only reads it.
        unsafe { D3DKMTWaitForVerticalBlankEvent(&wait) >= 0 }
    }

    /// Whether window `hwnd` is now mostly on another display than the
    /// one this clock follows.
    pub fn moved(&self, hwnd: isize) -> bool {
        monitor_of(hwnd) != self.monitor
    }
}

impl Drop for VBlank {
    fn drop(&mut self) {
        let close = D3DKMT_CLOSEADAPTER {
            hAdapter: self.adapter,
        };
        // SAFETY: the adapter handle came from D3DKMTOpenAdapterFromHdc and
        // is closed exactly once, here.
        unsafe { D3DKMTCloseAdapter(&close) };
    }
}

/// The display (HMONITOR value) that holds most of `hwnd`; the primary one
/// for 0 or a window that no longer exists.
fn monitor_of(hwnd: isize) -> isize {
    // SAFETY: MonitorFromWindow takes the HWND by value and answers the
    // primary display for a null or invalid one (MONITOR_DEFAULTTOPRIMARY);
    // it does not touch memory through it.
    unsafe { MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTOPRIMARY) as isize }
}

#[cfg(test)]
mod tests {
    use super::VBlank;
    use std::time::{Duration, Instant};

    /// Real machine (`cargo test -p vestra-win32 -- --ignored`): the primary
    /// display's vertical blanks come at its refresh rate. Prints the
    /// intervals; a machine without a kernel display adapter (remote
    /// desktop) has no clock and passes.
    #[test]
    #[ignore = "needs a display"]
    fn the_primary_display_blanks_at_its_refresh_rate() {
        let Some(clock) = VBlank::open(0) else {
            eprintln!("no vertical blank clock on this machine");
            return;
        };
        assert!(clock.wait());
        let mut intervals = Vec::new();
        let mut last = Instant::now();
        for _ in 0..120 {
            assert!(clock.wait());
            let now = Instant::now();
            intervals.push(now - last);
            last = now;
        }
        let mean = intervals.iter().sum::<Duration>() / intervals.len() as u32;
        let (min, max) = (intervals.iter().min(), intervals.iter().max());
        eprintln!("vblank interval mean {mean:?} min {min:?} max {max:?}");
        // Between 24 Hz and 500 Hz.
        assert!(mean > Duration::from_millis(2) && mean < Duration::from_millis(42));
        assert!(!clock.moved(0));
    }
}
