//! The Win32 calls that no mature safe crate wraps for the host's Windows
//! platform module (`src/host/src/platform/windows.rs`): the pointer position
//! while a file is dragged over a window (winit 0.30 reports no position for
//! drags), the modifier keys held right now, the double-click time, the
//! submenu show delay and the thread's sleep-prevention state. Each is one
//! call with plain integer arguments or one local out-parameter; the public
//! API is safe. Compiled only on Windows (cross-checked from macOS by
//! `npm run portability`, not yet run on Windows).
#![cfg(windows)]

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SPI_GETMENUSHOWDELAY, SystemParametersInfoW,
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
