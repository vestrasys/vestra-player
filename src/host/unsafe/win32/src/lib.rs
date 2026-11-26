//! The Win32 calls that no mature safe crate wraps for the host's Windows
//! platform module (`src/host/src/platform/windows.rs`): the pointer position
//! while a file is dragged over a window (winit 0.30 reports no position for
//! drags), the modifier keys held right now, the double-click time, the
//! submenu show delay, whether windows animate (client_area_animation),
//! the thread's sleep-prevention state, a window's
//! dark title bar, cloaking a window until its first frame (`set_cloaked`,
//! `cloak_on_show`: a thread-local `WH_CALLWNDPROC` hook) and every window
//! of the process once its GPU device was lost (`cloak_process_windows`),
//! letting the running player take the foreground for a
//! second launch's files, whether this process owns the foreground window
//! and bringing one of its windows to the front (`foreground_is_ours`,
//! `set_foreground`), the displays' work areas and a window's normal frame
//! in physical pixels (`display_areas`, `normal_placement`,
//! `set_normal_placement`: the main window restored across launches),
//! waiting for a display's vertical blank
//! (`VBlank`, the redraw clock) and keeping a full-screen window from
//! matching its display exactly (`unmatch_display`) and out of independent
//! flip (`ComposeGuard`: a one-pixel window that keeps DWM composing), whether an open
//! file is on a network file system (`file_is_remote`), the free space of
//! the volume holding a path (`disk_space`), waiting for a
//! process to end (`wait_for_process_exit`), whether a process runs and
//! since when (`process_creation_time`: the owner of an export's folder of
//! intermediate parts), the thread's CPU cycles
//! (`thread_cycles`), the process's private memory and the system commit
//! charge (`memory_status`, the host log's memory line), the physical
//! memory and the system's low-memory notification (`physical_memory`,
//! `LowMemoryNotification`: the plugin supervisor's process budget and
//! memory pressure), the session
//! ending (`on_session_end`, a thread-local `WH_CALLWNDPROC` hook), the DXGI adapters with their driver versions
//! (`dxgi_adapters`, through the `windows` crate's COM interfaces; no device
//! is created), and whether an adapter's driver offers RTX Video Super
//! Resolution (`video_super_resolution`: a short-lived Direct3D 11 device
//! and video processor), and a confirmation as a task dialog with its
//! question as the main instruction, a native checkbox and no standard
//! icon, so no system sound (`task_dialog`), and keeping a system DLL
//! loaded for the life of the process (`pin_module`: `d3dcompiler_47.dll`
//! under wgpu's Direct3D 12 windows), and a thread timer that runs only
//! while a window of the thread is in the system's modal move/size loop
//! (`on_move_size`, a thread-local `WH_CALLWNDPROC` hook: the host's tick
//! keeps running there). Each is one call with plain integer arguments, a
//! borrowed handle, or local out-parameters; the public
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
use windows_sys::Win32::Graphics::Gdi::{
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
};
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetCursorPos, GetWindowRect, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos, WINDOWPLACEMENT,
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

/// `file` is open on a network file system (an SMB share, by UNC path or a
/// mapped drive, WebDAV, NFS…), asked of the open handle as mpv's
/// `check_stream_network` (stream/stream_file.c) does: the volume's device is
/// a network file system or is marked remote. False when the system does not
/// answer.
pub fn file_is_remote(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileFsDeviceInformation, NtQueryVolumeInformationFile,
    };
    use windows_sys::Wdk::System::SystemServices::{
        FILE_FS_DEVICE_INFORMATION, FILE_REMOTE_DEVICE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    /// winioctl.h `FILE_DEVICE_NETWORK_FILE_SYSTEM`.
    const FILE_DEVICE_NETWORK_FILE_SYSTEM: u32 = 0x14;
    let mut info = FILE_FS_DEVICE_INFORMATION::default();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: the handle is `file`'s, open for the whole call (borrowed);
    // `status` and `info` are valid, writable locals and `info`'s size is
    // passed with it, so the call writes only inside them.
    let result = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&raw mut info).cast(),
            size_of::<FILE_FS_DEVICE_INFORMATION>() as u32,
            FileFsDeviceInformation,
        )
    };
    result >= 0
        && (info.DeviceType == FILE_DEVICE_NETWORK_FILE_SYSTEM
            || info.Characteristics & FILE_REMOTE_DEVICE != 0)
}

/// The bytes free for this user on the volume that holds `path` (an
/// existing file or directory) and that volume's root (`C:\\`,
/// `\\\\server\\share\\`, a mount folder), to tell two paths on one volume
/// apart. `None` when the system does not answer.
pub fn disk_space(path: &std::path::Path) -> Option<(u64, String)> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = [0u16; 1024];
    // SAFETY: `wide` is a NUL-terminated UTF-16 string alive for the call;
    // `root` is a writable local buffer whose length in characters is passed
    // with it, so the call writes only inside it.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut free = 0u64;
    // SAFETY: `root` now holds a NUL-terminated volume path; `free` is a
    // writable local; the two other out-parameters may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            root.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    let length = root.iter().position(|c| *c == 0).unwrap_or(root.len());
    let root = std::ffi::OsString::from_wide(&root[..length]);
    Some((free, root.to_string_lossy().into_owned()))
}

/// The system double-click time in milliseconds.
pub fn double_click_ms() -> u32 {
    // SAFETY: no arguments, no preconditions.
    unsafe { GetDoubleClickTime() }
}

/// Whether the user lets windows animate (Settings ▸ Accessibility ▸
/// Visual effects ▸ Animation effects, the "Show animations in Windows"
/// switch: `SPI_GETCLIENTAREAANIMATION`). `true` when the system does not
/// answer.
pub fn client_area_animation() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut on: windows_sys::core::BOOL = 1;
    // SAFETY: `pvParam` points to a local BOOL that outlives the call, as
    // SPI_GETCLIENTAREAANIMATION requires; `uiParam` and `fWinIni` are 0.
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    ok == 0 || on != 0
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

/// Cloak or uncloak window `hwnd` (a raw HWND value, a top-level window of
/// this process): a cloaked window keeps its place, size and visibility but
/// the desktop compositor does not show it (DWMWA_CLOAK, Windows 8 and
/// later). Returns false when the system refused (`hwnd` is no longer a
/// window, or not a top-level window of this process).
pub fn set_cloaked(hwnd: isize, cloaked: bool) -> bool {
    use windows_sys::Win32::Graphics::Dwm::DWMWA_CLOAK;
    let value: i32 = i32::from(cloaked);
    // SAFETY: DwmSetWindowAttribute reads `size_of::<i32>()` bytes (a BOOL)
    // from the pointer, which points at `value`, alive for the call. An
    // invalid or foreign HWND makes it return an error HRESULT, it does not
    // touch memory through it.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_CLOAK as u32,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    };
    result >= 0
}

/// Cloak every visible top-level window of this process ([`set_cloaked`]),
/// from any thread: the windows' GPU device was lost, nothing can draw in
/// them any more, and the restarted player replaces them. Neither the
/// enumeration nor the cloak sends the windows a message, so it works while
/// the thread that owns them is busy (or stuck in a driver call). Returns
/// how many were cloaked.
pub fn cloak_process_windows() -> usize {
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        let mut process = 0u32;
        // SAFETY: `window` comes from EnumWindows and is passed by value (a
        // window destroyed meanwhile makes the call return 0); the
        // out-parameter points at the local `process`.
        let thread = unsafe { GetWindowThreadProcessId(window, &mut process) };
        // SAFETY: GetCurrentProcessId takes no arguments.
        let me = unsafe { GetCurrentProcessId() };
        // SAFETY: IsWindowVisible takes the handle by value and only reads
        // the window's state (0 for a window destroyed meanwhile).
        let visible = unsafe { IsWindowVisible(window) } != 0;
        let ours = thread != 0 && process == me && visible;
        if ours {
            // SAFETY: `found` is the address of the `Vec<isize>` that
            // `cloak_process_windows` passed; EnumWindows calls back on that
            // thread, synchronously, while the vector is alive and not
            // otherwise borrowed.
            unsafe { (*(found as *mut Vec<isize>)).push(window as isize) };
        }
        1
    }
    let mut found: Vec<isize> = Vec::new();
    // SAFETY: the callback matches WNDENUMPROC; the LPARAM is the address of
    // `found`, alive (and not used otherwise) until EnumWindows returns,
    // which calls the callback only before it returns.
    unsafe { EnumWindows(Some(each), (&raw mut found) as LPARAM) };
    found
        .into_iter()
        .filter(|&window| set_cloaked(window, true))
        .count()
}

/// What [`cloak_on_show`] installed on this thread: the window class it
/// cloaks (UTF-16, no terminator) and whom it tells.
struct CloakOnShow {
    class: Vec<u16>,
    cloaked: Box<dyn Fn(isize)>,
}

thread_local! {
    static CLOAK_ON_SHOW: std::cell::RefCell<Option<CloakOnShow>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether a message sent to a window is its own show from hidden
/// (`WM_SHOWWINDOW` with `wParam` TRUE and `lParam` 0, not a show because
/// its owner was restored). Checked first: the hook sees every message.
fn is_own_show(message: u32, wparam: usize, lparam: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_SHOWWINDOW;
    message == WM_SHOWWINDOW && wparam != 0 && lparam == 0
}

/// Whether a window being shown is a top-level window (`style` without
/// `WS_CHILD`) that is not visible yet (a draw will follow to uncloak it).
fn hidden_top_level(style: u32, visible: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::WS_CHILD;
    style & WS_CHILD == 0 && !visible
}

/// The `WH_CALLWNDPROC` hook of [`cloak_on_show`]: sees each message sent to
/// a window of this thread before the window does.
///
/// # Safety
///
/// Called only by the system as a `WH_CALLWNDPROC` hook procedure: with
/// `code == HC_ACTION`, `lparam` points at a `CWPSTRUCT` valid for the call.
unsafe extern "system" fn cloak_on_show_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CWPSTRUCT, CallNextHookEx, GWL_STYLE, GetClassNameW, GetWindowLongW, HC_ACTION,
        IsWindowVisible,
    };
    if code == HC_ACTION as i32 && lparam != 0 {
        // SAFETY: for WH_CALLWNDPROC with HC_ACTION, `lparam` points at the
        // system's CWPSTRUCT for this message, valid for the hook call (this
        // function's contract); it is only read, before the call returns.
        let sent = unsafe { &*(lparam as *const CWPSTRUCT) };
        let hwnd = sent.hwnd;
        // SAFETY: both take the HWND by value (a window of this thread, being
        // sent a message) and touch no memory of ours.
        let window = || unsafe {
            (
                GetWindowLongW(hwnd, GWL_STYLE) as u32,
                IsWindowVisible(hwnd) != 0,
            )
        };
        if is_own_show(sent.message, sent.wParam, sent.lParam) && {
            let (style, visible) = window();
            hidden_top_level(style, visible)
        } {
            // `try_with`: nothing while the thread's locals are torn down.
            let _ = CLOAK_ON_SHOW.try_with(|hook| {
                let Ok(hook) = hook.try_borrow() else {
                    return;
                };
                let Some(hook) = hook.as_ref() else {
                    return;
                };
                let mut name = [0u16; 64];
                // SAFETY: GetClassNameW writes at most `name.len()` UTF-16
                // units (with the terminator) into `name`, a live local.
                let length = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
                let length = usize::try_from(length).unwrap_or(0);
                if name[..length] == hook.class[..] && set_cloaked(hwnd as isize, true) {
                    // A panic must not unwind into the system's call; the
                    // caller then never learned of the window, so it is
                    // uncloaked at once rather than left hidden for good.
                    let told = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (hook.cloaked)(hwnd as isize);
                    }));
                    if told.is_err() {
                        set_cloaked(hwnd as isize, false);
                    }
                }
            });
        }
    }
    // SAFETY: passes this hook's arguments on unchanged, as every hook must
    // (the first argument is ignored since Windows NT).
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// What a session-end message says (`on_session_end`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// `WM_QUERYENDSESSION`: the system asks whether the session may end
    /// (shutdown, restart, logoff, an update's restart).
    Query,
    /// `WM_ENDSESSION` with `wParam` TRUE: it ends; the process may be
    /// ended any time after this message returns.
    Ending,
    /// `WM_ENDSESSION` with `wParam` FALSE: the end was cancelled.
    Cancelled,
}

/// Which session message `message`/`wparam` is, if any.
fn session_end(message: u32, wparam: usize) -> Option<SessionEnd> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION};
    match message {
        WM_QUERYENDSESSION => Some(SessionEnd::Query),
        WM_ENDSESSION if wparam != 0 => Some(SessionEnd::Ending),
        WM_ENDSESSION => Some(SessionEnd::Cancelled),
        _ => None,
    }
}

/// What [`on_session_end`] calls.
type SessionHook = Box<dyn Fn(SessionEnd)>;

thread_local! {
    static ON_SESSION_END: std::cell::RefCell<Option<SessionHook>> =
        const { std::cell::RefCell::new(None) };
    /// The last message told, so the copies sent to each top-level window
    /// of the thread are told once.
    static SESSION_TOLD: std::cell::Cell<Option<SessionEnd>> = const { std::cell::Cell::new(None) };
}

/// The `WH_CALLWNDPROC` hook of [`on_session_end`].
///
/// # Safety
///
/// Called only by the system as a `WH_CALLWNDPROC` hook procedure: with
/// `code == HC_ACTION`, `lparam` points at a `CWPSTRUCT` valid for the call.
unsafe extern "system" fn session_end_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CWPSTRUCT, CallNextHookEx, HC_ACTION};
    if code == HC_ACTION as i32 && lparam != 0 {
        // SAFETY: for WH_CALLWNDPROC with HC_ACTION, `lparam` points at the
        // system's CWPSTRUCT for this message, valid for the hook call (this
        // function's contract); it is only read, before the call returns.
        let sent = unsafe { &*(lparam as *const CWPSTRUCT) };
        if let Some(what) = session_end(sent.message, sent.wParam)
            && SESSION_TOLD.try_with(|told| told.replace(Some(what))) != Ok(Some(what))
        {
            let _ = ON_SESSION_END.try_with(|hook| {
                if let Ok(hook) = hook.try_borrow()
                    && let Some(hook) = hook.as_ref()
                {
                    // A panic must not unwind into the system's call.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook(what)));
                }
            });
        }
    }
    // SAFETY: passes this hook's arguments on unchanged, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// From now on, call `ended` on this thread when the system sends its
/// windows `WM_QUERYENDSESSION` or `WM_ENDSESSION` (shutdown, restart,
/// logoff, an update's restart): winit does not report them. The window
/// procedures still answer as before (the session may end). Once per
/// thread (a thread-local `WH_CALLWNDPROC` hook, kept until the thread
/// ends); false when already installed or the system refused.
pub fn on_session_end(ended: impl Fn(SessionEnd) + 'static) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CALLWNDPROC};
    if ON_SESSION_END.with_borrow(Option::is_some) {
        return false;
    }
    ON_SESSION_END.set(Some(Box::new(ended)));
    // SAFETY: a thread-local hook (no module handle, this thread's id) whose
    // procedure is a plain function of this crate, alive for the process.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(session_end_hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if hook.is_null() {
        ON_SESSION_END.set(None);
        return false;
    }
    true
}

/// From now on, cloak (`set_cloaked`) each top-level window of window class
/// `class` that this thread shows from hidden, just before it shows, and
/// call `cloaked` with it; the caller uncloaks it once it has drawn. Without
/// this, a window shown before its first frame reaches the screen shows the
/// system's empty client area (white) during the open animation; Chromium
/// and Windows Terminal cloak their new windows until their first frame for
/// the same reason. Installed once per thread (a thread-local
/// `WH_CALLWNDPROC` hook, kept until the thread ends); false when it is
/// already installed or the system refused.
pub fn cloak_on_show(class: &str, cloaked: impl Fn(isize) + 'static) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CALLWNDPROC};
    if CLOAK_ON_SHOW.with_borrow(Option::is_some) {
        return false;
    }
    CLOAK_ON_SHOW.set(Some(CloakOnShow {
        class: class.encode_utf16().collect(),
        cloaked: Box::new(cloaked),
    }));
    // SAFETY: a thread-local hook (no module handle, this thread's id) whose
    // procedure is a plain function of this crate, alive for the process.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(cloak_on_show_hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if hook.is_null() {
        CLOAK_ON_SHOW.set(None);
        return false;
    }
    true
}

/// What [`on_move_size`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveSize {
    /// `WM_ENTERSIZEMOVE`: a window of this thread entered the system's
    /// modal move or size loop (its title bar or border held, or Move/Size
    /// from its window menu); the timer runs from now on.
    Entered,
    /// The timer fired, inside the loop.
    Timer,
    /// `WM_EXITSIZEMOVE`: the loop ended; the timer is stopped.
    Exited,
}

/// Which move/size message `message` is, if any.
fn move_size(message: u32) -> Option<MoveSize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE};
    match message {
        WM_ENTERSIZEMOVE => Some(MoveSize::Entered),
        WM_EXITSIZEMOVE => Some(MoveSize::Exited),
        _ => None,
    }
}

/// What [`on_move_size`] installed on this thread: whom it tells, the
/// timer's period and its id while it runs.
struct MoveSizeHook {
    told: Box<dyn Fn(MoveSize)>,
    period_ms: u32,
    timer: std::cell::Cell<usize>,
}

thread_local! {
    static ON_MOVE_SIZE: std::cell::RefCell<Option<MoveSizeHook>> =
        const { std::cell::RefCell::new(None) };
}

/// Tell [`on_move_size`]'s callback `what`; a panic does not unwind into
/// the system's call. Nothing while the thread's locals are torn down.
fn tell_move_size(what: MoveSize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
    let _ = ON_MOVE_SIZE.try_with(|hook| {
        let Ok(hook) = hook.try_borrow() else {
            return;
        };
        let Some(hook) = hook.as_ref() else {
            return;
        };
        match what {
            MoveSize::Entered if hook.timer.get() == 0 => {
                // SAFETY: a thread timer (no window, id chosen by the
                // system) whose procedure is a plain function of this
                // crate, alive for the process; no memory is passed.
                let id = unsafe {
                    SetTimer(
                        std::ptr::null_mut(),
                        0,
                        hook.period_ms,
                        Some(move_size_timer),
                    )
                };
                hook.timer.set(id);
            }
            MoveSize::Exited if hook.timer.get() != 0 => {
                // SAFETY: the id SetTimer returned for this thread's timer
                // without a window; a stale id makes the call fail, nothing
                // else.
                unsafe { KillTimer(std::ptr::null_mut(), hook.timer.replace(0)) };
            }
            _ => {}
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (hook.told)(what)));
    });
}

/// The thread timer of [`on_move_size`]: `DispatchMessage` calls it for its
/// `WM_TIMER`, from the move/size loop's own message loop.
///
/// # Safety
///
/// Called only by the system as a `TIMERPROC`; no argument is used.
unsafe extern "system" fn move_size_timer(_window: HWND, _message: u32, _id: usize, _time: u32) {
    tell_move_size(MoveSize::Timer);
}

/// The `WH_CALLWNDPROC` hook of [`on_move_size`].
///
/// # Safety
///
/// Called only by the system as a `WH_CALLWNDPROC` hook procedure: with
/// `code == HC_ACTION`, `lparam` points at a `CWPSTRUCT` valid for the call.
unsafe extern "system" fn move_size_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CWPSTRUCT, CallNextHookEx, HC_ACTION};
    if code == HC_ACTION as i32 && lparam != 0 {
        // SAFETY: for WH_CALLWNDPROC with HC_ACTION, `lparam` points at the
        // system's CWPSTRUCT for this message, valid for the hook call (this
        // function's contract); it is only read, before the call returns.
        let sent = unsafe { &*(lparam as *const CWPSTRUCT) };
        if let Some(what) = move_size(sent.message) {
            tell_move_size(what);
        }
    }
    // SAFETY: passes this hook's arguments on unchanged, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// From now on, while a window of this thread is in the system's modal
/// move or size loop (`WM_ENTERSIZEMOVE` to `WM_EXITSIZEMOVE`: its title
/// bar or a border held, or Move/Size from its window menu), call `told`
/// every `period_ms` from a thread timer, and once at the loop's start and
/// end. That loop dispatches messages itself, so an event loop that waits
/// for its own timers (winit's) gets no turn of its own until it ends; the
/// timer's `WM_TIMER` is dispatched by the loop. Nothing runs outside the
/// loop. Once per thread (a thread-local `WH_CALLWNDPROC` hook, kept until
/// the thread ends); false when already installed or the system refused.
pub fn on_move_size(period_ms: u32, told: impl Fn(MoveSize) + 'static) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CALLWNDPROC};
    if ON_MOVE_SIZE.with_borrow(Option::is_some) {
        return false;
    }
    ON_MOVE_SIZE.set(Some(MoveSizeHook {
        told: Box::new(told),
        period_ms,
        timer: std::cell::Cell::new(0),
    }));
    // SAFETY: a thread-local hook (no module handle, this thread's id) whose
    // procedure is a plain function of this crate, alive for the process.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(move_size_hook),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    if hook.is_null() {
        ON_MOVE_SIZE.set(None);
        return false;
    }
    true
}

/// Let another process (the running player a second launch hands its files
/// to) bring its window to the front: the foreground rights of the process
/// the user just started pass on. `false` when the system refused.
pub fn allow_any_foreground() -> bool {
    // SAFETY: AllowSetForegroundWindow takes a process id by value
    // (ASFW_ANY: any process) and has no memory-safety preconditions.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) != 0 }
}

/// Whether the foreground window (the one the user works in) belongs to
/// this process. `false` when there is none (a secure desktop, a window
/// being activated).
pub fn foreground_is_ours() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    // SAFETY: GetForegroundWindow takes no arguments and returns a handle
    // value (possibly null), which is not dereferenced here.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return false;
    }
    let mut process = 0u32;
    // SAFETY: `window` is passed by value (a stale handle makes the call
    // return 0, it is not dereferenced); the out-parameter points at the
    // local `process`, alive and writable for the call.
    let thread = unsafe { GetWindowThreadProcessId(window, &mut process) };
    // SAFETY: GetCurrentProcessId takes no arguments.
    thread != 0 && process == unsafe { GetCurrentProcessId() }
}

/// Bring window `hwnd` (a raw HWND value) to the front and give it the
/// keyboard focus with a plain `SetForegroundWindow`: no synthetic input,
/// so the system's foreground lock applies and the call succeeds only
/// while this process may take the foreground (it owns the foreground
/// window, or the user just acted in it). `false` when the system refused.
pub fn set_foreground(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    // SAFETY: SetForegroundWindow takes the handle by value; an invalid or
    // foreign HWND makes it return 0, it does not touch memory through it.
    unsafe { SetForegroundWindow(hwnd as HWND) != 0 }
}

/// The popup window `hwnd` (a raw HWND value) most recently activated, such
/// as the dialog it owns while one is open (`GetLastActivePopup`): `hwnd`
/// itself when it owns none, or for an invalid handle.
pub fn last_active_popup(hwnd: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetLastActivePopup;
    // SAFETY: GetLastActivePopup takes the handle by value and returns a
    // handle value, which is not dereferenced here; an invalid handle makes
    // it return null.
    let popup = unsafe { GetLastActivePopup(hwnd as HWND) };
    if popup.is_null() {
        hwnd
    } else {
        popup as isize
    }
}

/// Minimize window `hwnd` (a raw HWND value) as the system does
/// (`ShowWindow(SW_MINIMIZE)`, what the taskbar and Win+Down do), not
/// through winit's minimized flag: winit maximizes a window flagged maximized
/// before minimizing it, and its flag stays set after a restore that is not a
/// `WM_SYSCOMMAND`, which then skips restoring the frame styles when the
/// window leaves fullscreen. `false` for an invalid handle.
pub fn minimize(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SW_MINIMIZE, ShowWindow};
    // SAFETY: both take the handle by value; IsWindow rejects an invalid
    // one, and ShowWindow returns 0 for it, neither touches memory through it.
    unsafe {
        if IsWindow(hwnd as HWND) == 0 {
            return false;
        }
        ShowWindow(hwnd as HWND, SW_MINIMIZE);
    }
    true
}

/// CPU cycles the calling thread has run so far (`QueryThreadCycleTime`):
/// the frame log tells a busy UI thread from one that waits. 0 on failure.
pub fn thread_cycles() -> u64 {
    use windows_sys::Win32::System::Threading::GetCurrentThread;
    use windows_sys::Win32::System::WindowsProgramming::QueryThreadCycleTime;
    let mut cycles = 0u64;
    // SAFETY: the current thread's pseudo-handle (no close needed) and a
    // live local out-parameter.
    let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) };
    if ok != 0 { cycles } else { 0 }
}

/// The process's private bytes and the system's memory (`memory_status`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryStatus {
    /// This process's private committed bytes (Task Manager's "Commit size").
    pub private_bytes: u64,
    /// The system commit charge, in bytes.
    pub commit_used: u64,
    /// The system commit limit (physical memory plus page files), in bytes.
    pub commit_limit: u64,
    /// Physical memory in use, percent.
    pub physical_load: u32,
}

/// This process's private bytes (`K32GetProcessMemoryInfo`), the system
/// commit charge and limit (`K32GetPerformanceInfo`: `CommitTotal` and
/// `CommitLimit`, in pages, as Task Manager's "Committed") and the physical
/// memory load (`GlobalMemoryStatusEx`). `None` when a call fails.
pub fn memory_status() -> Option<MemoryStatus> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetPerformanceInfo, K32GetProcessMemoryInfo, PERFORMANCE_INFORMATION,
        PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut process = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the current process's pseudo-handle (no close needed) and a
    // live local out-parameter whose size is passed in `cb`; the EX
    // structure begins with the plain one, as the API documents.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut process).cast::<PROCESS_MEMORY_COUNTERS>(),
            process.cb,
        )
    };
    if ok == 0 {
        return None;
    }
    let mut performance = PERFORMANCE_INFORMATION {
        cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };
    // SAFETY: a live local out-parameter whose size is passed in `cb`.
    if unsafe { K32GetPerformanceInfo(&mut performance, performance.cb) } == 0 {
        return None;
    }
    let mut system = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: a live local out-parameter with `dwLength` set, as required.
    if unsafe { GlobalMemoryStatusEx(&mut system) } == 0 {
        return None;
    }
    let page = performance.PageSize as u64;
    Some(MemoryStatus {
        private_bytes: process.PrivateUsage as u64,
        commit_used: (performance.CommitTotal as u64).saturating_mul(page),
        commit_limit: (performance.CommitLimit as u64).saturating_mul(page),
        physical_load: system.dwMemoryLoad,
    })
}

/// The machine's physical memory (`physical_memory`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalMemory {
    /// Installed physical memory the system can use, in bytes.
    pub total: u64,
    /// Physical memory available now (free, standby and zeroed pages), in
    /// bytes.
    pub available: u64,
}

/// The physical memory in all and available now (`GlobalMemoryStatusEx`:
/// `ullTotalPhys`, `ullAvailPhys`): the plugin supervisor's process budget
/// and memory pressure, as Chrome's Windows memory pressure monitor reads
/// it. `None` when the call fails.
pub fn physical_memory() -> Option<PhysicalMemory> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut system = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: a live local out-parameter with `dwLength` set, as required.
    if unsafe { GlobalMemoryStatusEx(&mut system) } == 0 {
        return None;
    }
    Some(PhysicalMemory {
        total: system.ullTotalPhys,
        available: system.ullAvailPhys,
    })
}

/// The system's low-memory notification
/// (`CreateMemoryResourceNotification(LowMemoryResourceNotification)`):
/// signaled while the memory manager considers available physical memory
/// low. Read without waiting (`QueryMemoryResourceNotification`). The
/// handle is an [`OwnedHandle`](std::os::windows::io::OwnedHandle): the
/// value is `Send` and `Sync` (the host's core may own it on a thread of
/// its own) and the handle is closed on
/// drop (`CloseHandle`), once.
#[derive(Debug)]
pub struct LowMemoryNotification(std::os::windows::io::OwnedHandle);

impl LowMemoryNotification {
    /// `None` when the system refuses the handle.
    pub fn new() -> Option<Self> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Memory::{
            CreateMemoryResourceNotification, LowMemoryResourceNotification,
        };
        // SAFETY: a notification type by value; the returned handle (null
        // on failure) is ours.
        let handle = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: `handle` is open (non-null: the call succeeded) and owned
        // by nothing else; `OwnedHandle` takes it over and closes it
        // exactly once, when this value drops.
        Some(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// Whether available physical memory is low now; `None` when the query
    /// fails.
    pub fn signaled(&self) -> Option<bool> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Memory::QueryMemoryResourceNotification;
        let mut state = 0;
        // SAFETY: the live notification handle this value owns and a live
        // local out-parameter.
        let ok = unsafe { QueryMemoryResourceNotification(self.0.as_raw_handle(), &mut state) };
        (ok != 0).then_some(state != 0)
    }
}

/// End this process at once with exit code `code` (`TerminateProcess` on
/// the current process): no atexit handlers, no thread or DLL detach
/// notifications, so a driver DLL stuck after a device loss cannot hold up
/// the exit as it could `ExitProcess`. Never returns.
pub fn terminate_current_process(code: u32) -> ! {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: the current process's pseudo-handle (no close needed) and an
    // exit code by value. TerminateProcess on the calling process does not
    // return when it succeeds.
    unsafe { TerminateProcess(GetCurrentProcess(), code) };
    // Only if the system refused (it does not for the current process).
    std::process::exit(code as i32)
}

/// Wait up to `timeout_ms` for process `pid` to end (a player restarting
/// itself waits for the process it replaces). `true` once it ended or when
/// it is already gone; `false` on a timeout.
pub fn wait_for_process_exit(pid: u32, timeout_ms: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: OpenProcess takes plain values and returns a handle we own
    // (null when the process is gone or not accessible).
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return true;
    }
    // SAFETY: a live process handle of ours, waited on and closed exactly once.
    unsafe {
        let ended = WaitForSingleObject(process, timeout_ms) == WAIT_OBJECT_0;
        CloseHandle(process);
        ended
    }
}

/// Whether process `pid` runs, and since when: `Ok(Some(t))` with its
/// creation time (`FILETIME` ticks, which tell a reused process id from the
/// process that had it), `Ok(None)` when no process has that id or it has
/// ended, `Err(())` when the system does not say (access refused, a failed
/// wait).
#[allow(clippy::result_unit_err)]
pub fn process_creation_time(pid: u32) -> Result<Option<u64>, ()> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };
    // SAFETY: OpenProcess takes plain values and returns a handle we own
    // (null when the process is gone or not accessible); GetLastError reads
    // this thread's last error right after it.
    let (process, error) = unsafe {
        let process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        (process, GetLastError())
    };
    if process.is_null() {
        // No process with that id.
        return if error == ERROR_INVALID_PARAMETER {
            Ok(None)
        } else {
            Err(())
        };
    }
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: a live process handle of ours, queried with local
    // out-parameters and closed exactly once. A zero wait only polls.
    let (wait, times) = unsafe {
        let wait = WaitForSingleObject(process, 0);
        let times = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(process);
        (wait, times != 0)
    };
    if wait == WAIT_OBJECT_0 {
        // Ended; a handle someone still holds keeps it openable.
        return Ok(None);
    }
    // Only a timeout says it runs; a failed wait says nothing.
    if wait != WAIT_TIMEOUT || !times {
        return Err(());
    }
    Ok(Some(
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
    ))
}

/// When window `hwnd` covers its display exactly (a borderless full-screen
/// window), make it `extra` pixels taller (negative: shorter) at the bottom,
/// so that it no longer matches the display's rectangle; past another edge
/// when a display lies directly below, where the extra row would show
/// (`unmatched`). Returns whether it changed the window. Nothing for a
/// window of any other size.
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
    let edges = |rect: RECT| (rect.left, rect.top, rect.right, rect.bottom);
    let another_display = |(left, top, right, bottom): Edges| {
        let strip = RECT {
            left,
            top,
            right,
            bottom,
        };
        // SAFETY: `strip` is a valid RECT read for the call; MonitorFromRect
        // keeps no pointer to it and answers null when no display meets it.
        !unsafe { MonitorFromRect(&strip, MONITOR_DEFAULTTONULL) }.is_null()
    };
    let Some((left, top, right, bottom)) =
        unmatched(edges(window), edges(info.rcMonitor), extra, another_display)
    else {
        return false;
    };
    // SAFETY: SetWindowPos takes the HWND and plain integers by value (no
    // z-order change, no activation); an invalid HWND makes it fail.
    unsafe {
        SetWindowPos(
            hwnd as HWND,
            std::ptr::null_mut(),
            left,
            top,
            right - left,
            bottom - top,
            SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

/// A rectangle as (left, top, right, bottom), in screen pixels.
type Edges = (i32, i32, i32, i32);

/// `unmatch_display`'s rectangle math: `None` unless `window` covers
/// `display` exactly (so a window already off by its pixels, for example a
/// main window rebuilt at the old window's size, stays as it is and never
/// grows). `extra` > 0: the window reaches `extra` pixels past the bottom
/// edge, or, where that strip lies on another display (`another_display`
/// of the strip: the pixels would show there), past the top, right or left
/// edge, the first one free; with another display on every side it is
/// `extra` pixels short at the bottom instead. `extra` < 0: short at the
/// bottom.
fn unmatched(
    window: Edges,
    display: Edges,
    extra: i32,
    another_display: impl Fn(Edges) -> bool,
) -> Option<Edges> {
    if window != display || extra == 0 {
        return None;
    }
    let (left, top, right, bottom) = display;
    let shorter = (left, top, right, bottom - extra.abs());
    if extra < 0 {
        return Some(shorter);
    }
    [
        (
            (left, bottom, right, bottom + extra),
            (left, top, right, bottom + extra),
        ),
        (
            (left, top - extra, right, top),
            (left, top - extra, right, bottom),
        ),
        (
            (right, top, right + extra, bottom),
            (left, top, right + extra, bottom),
        ),
        (
            (left - extra, top, left, bottom),
            (left - extra, top, right, bottom),
        ),
    ]
    .into_iter()
    .find(|(strip, _)| !another_display(*strip))
    .map_or(Some(shorter), |(_, taller)| Some(taller))
}

/// A one-pixel, almost transparent window over the top-left pixel of the
/// display of a full-screen window (its owner), so that the Desktop Window
/// Manager keeps composing that display instead of promoting the owner's
/// flip-model swapchain to independent flip (or an overlay plane): with any
/// other visible window over a swapchain, DWM has to compose. On the test
/// laptop's AMD-driven panel that promotion froze the visible mouse pointer
/// for seconds. Layered, click-through (`WS_EX_TRANSPARENT`), never
/// activated (`WS_EX_NOACTIVATE`), a tool window (no taskbar button, not in
/// Alt+Tab), owned (always above its owner, gone with it). Black at
/// `alpha`/255. Created and dropped on the owner's thread (the type is not
/// `Send`: it holds an `HWND`).
pub struct ComposeGuard {
    hwnd: HWND,
    owner: HWND,
}

impl ComposeGuard {
    /// The guard over the display of window `owner`, shown at once; `None`
    /// when the window could not be created.
    pub fn open(owner: isize, alpha: u8) -> Option<Self> {
        use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, LWA_ALPHA, RegisterClassW, SetLayeredWindowAttributes,
            WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
            WS_POPUP,
        };
        static CLASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let class = windows_sys::w!("VestraComposeGuard");
        // SAFETY: GetModuleHandleW(null) answers this executable's module
        // without a reference count to release.
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let registered = *CLASS.get_or_init(|| {
            let description = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(DefWindowProcW),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                // SAFETY: a stock object, never deleted.
                hbrBackground: unsafe { GetStockObject(BLACK_BRUSH) } as HBRUSH,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class,
            };
            // SAFETY: `description` is a valid WNDCLASSW read for the call;
            // its class name is a static string and its window procedure the
            // system's default one. Registered once per process.
            unsafe { RegisterClassW(&description) != 0 }
        });
        if !registered {
            return None;
        }
        // SAFETY: a class registered above with a static name; no title,
        // menu or creation data; the owner is a plain handle (an invalid one
        // makes the call fail). The window is owned by the returned value
        // and destroyed on this thread in `drop`.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                owner as HWND,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let guard = Self {
            hwnd,
            owner: owner as HWND,
        };
        // SAFETY: our own live window; plain values.
        if unsafe { SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) } == 0 {
            return None;
        }
        guard.place().then_some(guard)
    }

    /// Move the guard to the top-left pixel of its owner's display (after
    /// the owner moved to another display) and show it, without activating
    /// it or changing the owner's z-order.
    pub fn place(&self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW;
        // SAFETY: MonitorFromWindow takes the HWND by value; it does not
        // touch memory through it.
        let monitor = unsafe { MonitorFromWindow(self.owner, MONITOR_DEFAULTTONEAREST) };
        let empty = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: empty,
            rcWork: empty,
            dwFlags: 0,
        };
        // SAFETY: `info` is a writable MONITORINFO whose cbSize says so.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return false;
        }
        // SAFETY: our own live window and plain integers; no activation, and
        // no z-order change (an owned window stays above its owner).
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                info.rcMonitor.left,
                info.rcMonitor.top,
                1,
                1,
                SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) != 0
        }
    }

    /// The guard's window still exists (Windows destroys an owned window
    /// with its owner).
    pub fn alive(&self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow, IsWindow};
        // SAFETY: both take the HWND by value; a stale handle answers false
        // (or another owner, when the handle was reused).
        unsafe { IsWindow(self.hwnd) != 0 && GetWindow(self.hwnd, GW_OWNER) == self.owner }
    }
}

impl Drop for ComposeGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
        if self.alive() {
            // SAFETY: our own window, created on this thread, destroyed once.
            unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

/// Whether `hwnd` names a live window.
pub fn window_exists(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
    // SAFETY: IsWindow takes the handle by value and rejects an invalid one.
    unsafe { IsWindow(hwnd as HWND) != 0 }
}

/// The display (HMONITOR value) that holds most of window `hwnd`; the
/// primary one for a window that no longer exists. Only compared, never
/// dereferenced.
pub fn window_display(hwnd: isize) -> isize {
    monitor_of(hwnd)
}

/// The display that holds most of a window, as the display configuration
/// sees it (`display_output`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayOutput {
    /// The GDI device name (`\\.\DISPLAY1`).
    pub device: String,
    /// The connector is built in (`internal_output`): a laptop's own panel.
    pub internal: bool,
    /// The LUID of the adapter that scans the display out (the display
    /// configuration path's source adapter), as `DxgiAdapter::luid`.
    pub adapter_luid: u64,
}

/// Whether a display configuration output technology is a built-in panel
/// connector: `INTERNAL` (what Windows reports for a laptop's own panel on
/// most drivers), embedded DisplayPort, embedded UDI, or LVDS (used only
/// for built-in panels).
pub fn internal_output(technology: i32) -> bool {
    use windows_sys::Win32::Devices::Display::{
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    };
    matches!(
        technology,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
    )
}

/// The display configuration's view of the display (HMONITOR value from
/// [`window_display`]): its connector and the adapter that scans it out,
/// through `QueryDisplayConfig` (active paths), the path whose source has
/// the display's GDI device name. A display cloned to several targets
/// counts as internal when any of them is. An error when the display or
/// its path cannot be found.
pub fn display_output(display: isize) -> Result<DisplayOutput, String> {
    use windows_sys::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
        GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    };
    // SAFETY: MONITORINFOEXW is plain data; all zeroes is a valid value.
    let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` is a writable MONITORINFOEXW whose cbSize says so; a
    // stale HMONITOR makes the call fail without touching memory through it.
    if unsafe { GetMonitorInfoW(display as HMONITOR, (&raw mut info).cast()) } == 0 {
        return Err("GetMonitorInfoW failed".into());
    }
    let device = utf16_until_nul(&info.szDevice);
    let (paths, _modes) = loop {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        // SAFETY: two writable counts for the call.
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if status != 0 {
            return Err(format!("GetDisplayConfigBufferSizes: error {status}"));
        }
        // SAFETY: both are plain data; all zeroes is a valid value.
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> =
            vec![unsafe { std::mem::zeroed() }; path_count as usize];
        // SAFETY: as above.
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> =
            vec![unsafe { std::mem::zeroed() }; mode_count as usize];
        // SAFETY: the arrays hold the counts given with them and are
        // writable; the call writes at most that many entries and the
        // counts it wrote. No topology id is asked for (null).
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        // ERROR_INSUFFICIENT_BUFFER: the configuration changed in between.
        if status == 122 {
            continue;
        }
        if status != 0 {
            return Err(format!("QueryDisplayConfig: error {status}"));
        }
        paths.truncate(path_count as usize);
        modes.truncate(mode_count as usize);
        break (paths, modes);
    };
    let mut found: Option<DisplayOutput> = None;
    for path in &paths {
        // SAFETY: plain data; all zeroes is a valid value.
        let mut name: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { std::mem::zeroed() };
        name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        name.header.adapterId = path.sourceInfo.adapterId;
        name.header.id = path.sourceInfo.id;
        // SAFETY: `name` is a writable request whose header gives its type
        // and size; the call fills in the source's GDI device name.
        if unsafe { DisplayConfigGetDeviceInfo(&mut name.header) } != 0 {
            continue;
        }
        if utf16_until_nul(&name.viewGdiDeviceName) != device {
            continue;
        }
        let internal = internal_output(path.targetInfo.outputTechnology);
        let luid = path.sourceInfo.adapterId;
        let adapter_luid = (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart);
        match &mut found {
            Some(output) => output.internal |= internal,
            None => {
                found = Some(DisplayOutput {
                    device: device.clone(),
                    internal,
                    adapter_luid,
                });
            }
        }
    }
    found.ok_or_else(|| format!("no active display path for {device}"))
}

/// A NUL-terminated UTF-16 array as a string.
fn utf16_until_nul(chars: &[u16]) -> String {
    let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
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

/// A DXGI adapter as the driver check sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DxgiAdapter {
    pub name: String,
    /// PCI vendor, device, subsystem and revision.
    pub vendor_id: u32,
    pub device_id: u32,
    pub subsys_id: u32,
    pub revision: u32,
    pub luid: u64,
    /// The DXGI software flag (WARP, Basic Render).
    pub software: bool,
    /// The user-mode driver's version, four parts, from
    /// `CheckInterfaceSupport(IDXGIDevice)`; `None` when DXGI does not say.
    pub driver_version: Option<[u16; 4]>,
}

/// The DXGI adapters in Windows' order for this process
/// (`IDXGIFactory1::EnumAdapters1`: the per-app graphics preference picks
/// the first). No device is created. An error only when DXGI itself is
/// unavailable.
pub fn dxgi_adapters() -> Result<Vec<DxgiAdapter>, String> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIDevice, IDXGIFactory1,
    };
    use windows::core::Interface;
    // SAFETY: plain factory creation; an owned interface or an error comes
    // back.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let mut adapters = Vec::new();
    for index in 0..64 {
        // SAFETY: a live factory; each call returns an owned adapter, or
        // DXGI_ERROR_NOT_FOUND past the last one.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        // SAFETY: a live adapter; GetDesc1 fills a plain struct it returns
        // by value.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        // SAFETY: a live adapter and a static interface id; the call only
        // writes the version it returns.
        let umd = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok();
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        adapters.push(DxgiAdapter {
            name: String::from_utf16_lossy(&desc.Description[..end]),
            vendor_id: desc.VendorId,
            device_id: desc.DeviceId,
            subsys_id: desc.SubSysId,
            revision: desc.Revision,
            luid: (u64::from(desc.AdapterLuid.HighPart as u32) << 32)
                | u64::from(desc.AdapterLuid.LowPart),
            software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
            driver_version: umd.map(version_parts),
        });
    }
    Ok(adapters)
}

/// The LUID of the first adapter `IDXGIFactory6::EnumAdapterByGpuPreference`
/// gives for `high_performance` (else minimum power): the adapter
/// `VESTRA_GPU_ADAPTER=high-performance|minimum-power` makes GPU rendering
/// use. No device is created. An error before Windows 10 1803 (no
/// `IDXGIFactory6`) or without DXGI.
pub fn dxgi_preferred_adapter(high_performance: bool) -> Result<u64, String> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        DXGI_GPU_PREFERENCE_MINIMUM_POWER, IDXGIAdapter1, IDXGIFactory6,
    };
    let preference = if high_performance {
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE
    } else {
        DXGI_GPU_PREFERENCE_MINIMUM_POWER
    };
    // SAFETY: plain factory creation; an owned interface or an error comes
    // back (IDXGIFactory6 from Windows 10 1803 on).
    let factory: IDXGIFactory6 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("IDXGIFactory6: {e}"))?;
    // SAFETY: a live factory; an owned adapter or an error comes back.
    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByGpuPreference(0, preference) }
        .map_err(|e| format!("EnumAdapterByGpuPreference: {e}"))?;
    // SAFETY: a live adapter; GetDesc1 fills a plain struct it returns by
    // value.
    let desc = unsafe { adapter.GetDesc1() }.map_err(|e| format!("GetDesc1: {e}"))?;
    Ok((u64::from(desc.AdapterLuid.HighPart as u32) << 32) | u64::from(desc.AdapterLuid.LowPart))
}

/// A `LARGE_INTEGER` driver version as its four 16-bit parts, high first.
fn version_parts(value: i64) -> [u16; 4] {
    let value = value as u64;
    [48, 32, 16, 0].map(|shift| (value >> shift) as u16)
}

/// NVIDIA's PPE stream extension (`{d43ce1b3-1f4b-48ac-baee-c3c25375e6f7}`),
/// the one mpv's `d3d11vpp=scaling-mode=nvidia` sets for RTX Video Super
/// Resolution.
const NVIDIA_PPE: windows::core::GUID =
    windows::core::GUID::from_u128(0xd43ce1b3_1f4b_48ac_baee_c3c25375e6f7);

/// What [`video_super_resolution`] found: the driver's answer to
/// `VideoProcessorGetStreamExtension(PPE)` on a test video processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuperResolution {
    /// The call's result (an `HRESULT`: negative is a failure).
    pub result: i32,
    /// The value the driver wrote (0 when it wrote none).
    pub value: u32,
}

impl SuperResolution {
    /// The driver offers it: the query succeeded with a non-zero value
    /// (VLC's check). Setting the extension proves nothing: AMD's driver
    /// accepts NVIDIA's GUID too.
    pub fn supported(&self) -> bool {
        self.result >= 0 && self.value != 0
    }
}

/// Ask the driver of the adapter with this LUID whether it offers RTX Video
/// Super Resolution, as VLC does: a Direct3D 11 device with video support on
/// that adapter, a 1080p → 2160p video processor, then
/// `VideoProcessorGetStreamExtension` with NVIDIA's PPE GUID. Everything is
/// released before it returns. It wakes a sleeping discrete GPU: call it
/// when the answer is needed, and keep the answer. `Err` when the adapter is
/// gone or Direct3D 11 has no video processor on it.
pub fn video_super_resolution(luid: u64) -> Result<SuperResolution, String> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11CreateDevice,
        ID3D11Device, ID3D11VideoContext, ID3D11VideoDevice,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};
    use windows::core::Interface;
    // SAFETY: plain factory creation; an owned interface or an error comes
    // back.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let mut found = None;
    for index in 0..64 {
        // SAFETY: a live factory; an owned adapter, or DXGI_ERROR_NOT_FOUND
        // past the last one.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        // SAFETY: a live adapter; GetDesc1 fills a plain struct.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let id = (u64::from(desc.AdapterLuid.HighPart as u32) << 32)
            | u64::from(desc.AdapterLuid.LowPart);
        if id == luid {
            found = Some(adapter);
            break;
        }
    }
    let adapter: IDXGIAdapter = found
        .ok_or_else(|| format!("no adapter with LUID {luid:016x}"))?
        .cast()
        .map_err(|e| e.to_string())?;
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    // SAFETY: the adapter lives for the call, the feature levels are a valid
    // slice, and the out-pointer is a live local.
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .map_err(|e| format!("D3D11CreateDevice (video support): {e}"))?;
    let device = device.ok_or("D3D11CreateDevice returned no device")?;
    let video: ID3D11VideoDevice = device
        .cast()
        .map_err(|e| format!("no Direct3D 11 video device: {e}"))?;
    // SAFETY: a live device; an owned reference comes back.
    let context = unsafe { device.GetImmediateContext() }.map_err(|e| e.to_string())?;
    let video_context: ID3D11VideoContext = context
        .cast()
        .map_err(|e| format!("no Direct3D 11 video context: {e}"))?;
    let rate = DXGI_RATIONAL {
        Numerator: 60,
        Denominator: 1,
    };
    let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: rate,
        InputWidth: 1920,
        InputHeight: 1080,
        OutputFrameRate: rate,
        OutputWidth: 3840,
        OutputHeight: 2160,
        Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    };
    // SAFETY: a live video device and a valid descriptor that outlives the
    // call; an owned enumerator or an error comes back.
    let enumerator = unsafe { video.CreateVideoProcessorEnumerator(&desc) }
        .map_err(|e| format!("CreateVideoProcessorEnumerator: {e}"))?;
    // SAFETY: a live video device and enumerator; an owned processor or an
    // error comes back.
    let processor = unsafe { video.CreateVideoProcessor(&enumerator, 0) }
        .map_err(|e| format!("CreateVideoProcessor: {e}"))?;
    let mut value: u32 = 0;
    // SAFETY: a live video context and processor, stream 0 (the processor
    // has one), a static GUID, and `value` is a live, writable u32 whose size
    // is passed with it, so the driver writes only inside it.
    let result = unsafe {
        video_context.VideoProcessorGetStreamExtension(
            &processor,
            0,
            &NVIDIA_PPE,
            std::mem::size_of::<u32>() as u32,
            (&raw mut value).cast(),
        )
    };
    Ok(SuperResolution { result, value })
}

/// A rectangle in screen coordinates (physical pixels): left, top, right,
/// bottom.
pub type ScreenRect = (i32, i32, i32, i32);

/// One display: its whole area and its work area (without the taskbar and
/// docked toolbars), screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayArea {
    pub monitor: ScreenRect,
    pub work: ScreenRect,
    pub primary: bool,
}

const EMPTY_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

fn edges(rect: RECT) -> ScreenRect {
    (rect.left, rect.top, rect.right, rect.bottom)
}

fn monitor_area(monitor: HMONITOR) -> Option<DisplayArea> {
    use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: EMPTY_RECT,
        rcWork: EMPTY_RECT,
        dwFlags: 0,
    };
    // SAFETY: `info` is a writable MONITORINFO whose cbSize says so; an
    // invalid monitor handle makes the call fail without touching it.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    Some(DisplayArea {
        monitor: edges(info.rcMonitor),
        work: edges(info.rcWork),
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

/// Every display attached to the desktop now.
pub fn display_areas() -> Vec<DisplayArea> {
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC};
    unsafe extern "system" fn each(
        monitor: HMONITOR,
        _dc: HDC,
        _clip: *mut RECT,
        found: LPARAM,
    ) -> windows_sys::core::BOOL {
        // SAFETY: `found` is the address of the `Vec` below, alive and not
        // otherwise borrowed while EnumDisplayMonitors runs (synchronously,
        // on this thread).
        let found = unsafe { &mut *(found as *mut Vec<HMONITOR>) };
        found.push(monitor);
        1
    }
    let mut monitors: Vec<HMONITOR> = Vec::new();
    // SAFETY: no device context and no clip rectangle (every display); the
    // callback is a plain function of this crate and `monitors` outlives the
    // call, which returns only after the last callback.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut monitors as *mut Vec<HMONITOR> as LPARAM,
        )
    };
    monitors.into_iter().filter_map(monitor_area).collect()
}

/// Workspace coordinates (`WINDOWPLACEMENT`'s) are relative to the primary
/// display's work area: what to add to them for screen coordinates.
fn workspace_offset() -> (i32, i32) {
    use windows_sys::Win32::Graphics::Gdi::MonitorFromPoint;
    // SAFETY: plain values; the primary display is always answered.
    let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    monitor_area(primary).map_or((0, 0), |area| {
        (area.work.0 - area.monitor.0, area.work.1 - area.monitor.1)
    })
}

/// Where window `hwnd` is when not maximized or minimized (its normal frame,
/// also while it is maximized or minimized), screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NormalPlacement {
    pub frame: ScreenRect,
    /// Maximized, or minimized from maximized.
    pub maximized: bool,
    pub minimized: bool,
}

fn empty_placement() -> WINDOWPLACEMENT {
    WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        flags: 0,
        showCmd: 0,
        ptMinPosition: POINT { x: -1, y: -1 },
        ptMaxPosition: POINT { x: -1, y: -1 },
        rcNormalPosition: EMPTY_RECT,
    }
}

/// [`NormalPlacement`] of window `hwnd` (`GetWindowPlacement`); `None` when
/// it is no longer a window.
pub fn normal_placement(hwnd: isize) -> Option<NormalPlacement> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, WPF_RESTORETOMAXIMIZED,
    };
    let mut placement = empty_placement();
    // SAFETY: `placement` is a writable WINDOWPLACEMENT whose length says so;
    // an invalid HWND makes the call fail without touching memory through it.
    if unsafe { GetWindowPlacement(hwnd as HWND, &mut placement) } == 0 {
        return None;
    }
    let (dx, dy) = workspace_offset();
    let (left, top, right, bottom) = edges(placement.rcNormalPosition);
    let minimized = placement.showCmd == SW_SHOWMINIMIZED as u32;
    Some(NormalPlacement {
        frame: (left + dx, top + dy, right + dx, bottom + dy),
        maximized: placement.showCmd == SW_SHOWMAXIMIZED as u32
            || (minimized && placement.flags & WPF_RESTORETOMAXIMIZED != 0),
        minimized,
    })
}

/// Give window `hwnd` the normal frame `frame` (screen coordinates) and
/// show it there, or maximized on the display it is on when `maximized`
/// (`SetWindowPlacement`, as Chromium and Firefox restore a window). Set
/// twice: a frame on a display of another scale first moves the window
/// there, and the window's DPI change resizes it by the scales' ratio; the
/// second call, on that display, gives the exact frame.
///
/// A window already minimized (the shortcut's "Run: Minimized": the
/// process's first `ShowWindow` takes the `STARTUPINFO` show state)
/// stays minimized and inactive, as Chromium and Firefox keep it; it
/// restores at `frame`, maximized when `maximized`
/// (`WPF_RESTORETOMAXIMIZED`). The first call is then made only for a frame
/// on another display, without activating the window (the main window is
/// still cloaked then, `cloak_on_show`). False when the system refused.
pub fn set_normal_placement(hwnd: isize, frame: ScreenRect, maximized: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SetWindowPlacement};
    let (dx, dy) = workspace_offset();
    let (left, top, right, bottom) = frame;
    let normal = RECT {
        left: left - dx,
        top: top - dy,
        right: right - dx,
        bottom: bottom - dy,
    };
    // SAFETY: IsIconic takes the handle by value; an invalid HWND answers
    // false.
    let minimized = unsafe { IsIconic(hwnd as HWND) } != 0;
    let other_display = minimized && {
        let screen = RECT {
            left,
            top,
            right,
            bottom,
        };
        // SAFETY: plain values; MonitorFromWindow uses a minimized window's
        // restored frame, and an invalid HWND answers no monitor.
        let (now, there) = unsafe {
            (
                MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST),
                MonitorFromRect(&screen, MONITOR_DEFAULTTONEAREST),
            )
        };
        now != there
    };
    let (flags, steps) = placement_steps(maximized, minimized, other_display);
    let mut placement = empty_placement();
    placement.rcNormalPosition = normal;
    placement.flags = flags;
    steps.into_iter().flatten().all(|show| {
        placement.showCmd = show as u32;
        // SAFETY: `placement` is a filled WINDOWPLACEMENT whose length says
        // so, read during the call; an invalid HWND makes it fail.
        unsafe { SetWindowPlacement(hwnd as HWND, &placement) != 0 }
    })
}

/// [`set_normal_placement`]'s `WINDOWPLACEMENT` flags and show commands, in
/// order, for a window `minimized` now whose frame is on `other_display`.
fn placement_steps(
    maximized: bool,
    minimized: bool,
    other_display: bool,
) -> (u32, [Option<i32>; 2]) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
        WPF_RESTORETOMAXIMIZED,
    };
    if minimized {
        let flags = if maximized { WPF_RESTORETOMAXIMIZED } else { 0 };
        let first = other_display.then_some(SW_SHOWNOACTIVATE);
        return (flags, [first, Some(SW_SHOWMINNOACTIVE)]);
    }
    let last = if maximized {
        SW_SHOWMAXIMIZED
    } else {
        SW_SHOWNORMAL
    };
    (0, [Some(SW_SHOWNORMAL), Some(last)])
}

/// A confirmation as a task dialog ([`task_dialog`]).
pub struct TaskDialog<'a> {
    /// The owner window (a raw HWND value): disabled while the dialog is
    /// open, and the dialog is centred on it.
    pub owner: Option<isize>,
    pub title: &'a str,
    /// The question, in the dialog's large main-instruction text.
    pub instruction: &'a str,
    /// The text under the question.
    pub content: Option<&'a str>,
    /// The default button (Enter).
    pub confirm: &'a str,
    /// The cancel button (Esc and the close box pick it too).
    pub cancel: &'a str,
    /// A checkbox under the text (the dialog's verification checkbox).
    pub checkbox: Option<&'a str>,
    /// The system's warning icon, given as an icon handle: a task dialog
    /// plays the system sound of a standard icon (`TD_WARNING_ICON`,
    /// `TD_INFORMATION_ICON`…) but not of an icon handle.
    pub warning_icon: bool,
}

/// How the user closed a [`TaskDialog`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskDialogAnswer {
    /// The confirm button was picked.
    pub confirmed: bool,
    /// The checkbox's state when the dialog closed, whichever way it closed.
    pub checked: bool,
}

type TaskDialogIndirectFn = unsafe extern "system" fn(
    *const windows_sys::Win32::UI::Controls::TASKDIALOGCONFIG,
    *mut i32,
    *mut i32,
    *mut windows_sys::core::BOOL,
) -> windows_sys::core::HRESULT;
type LoadIconWithScaleDownFn = unsafe extern "system" fn(
    windows_sys::Win32::Foundation::HINSTANCE,
    windows_sys::core::PCWSTR,
    i32,
    i32,
    *mut windows_sys::Win32::UI::WindowsAndMessaging::HICON,
) -> windows_sys::core::HRESULT;

/// A function of Common Controls 6 (`comctl32.dll` as the process's
/// activation context resolves it: version 6 only with the application
/// manifest that asks for it). Looked up at run time, so a binary without
/// that manifest (a unit-test binary) still starts.
fn comctl32_function(name: &std::ffi::CStr) -> Option<unsafe extern "system" fn() -> isize> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let library: Vec<u16> = "comctl32.dll\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated wide string alive for the call. The module
    // stays loaded for the life of the process (no FreeLibrary), so the
    // address returned below stays valid.
    let module = unsafe { LoadLibraryW(library.as_ptr()) };
    if module.is_null() {
        return None;
    }
    // SAFETY: a module handle from LoadLibraryW and a NUL-terminated name.
    unsafe { GetProcAddress(module, name.as_ptr().cast()) }
}

/// Show `dialog` and wait for the answer: a task dialog (Common Controls
/// 6) with the question as its main instruction, two custom buttons, an
/// optional native checkbox, and no standard icon, so no system sound.
/// Blocks the calling thread in the dialog's own message loop: call it on
/// a thread of its own. `Err` when the dialog could not be shown (Common
/// Controls 6 missing, the system refused).
pub fn task_dialog(dialog: &TaskDialog) -> Result<TaskDialogAnswer, String> {
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT, TDF_USE_HICON_MAIN,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, IDI_WARNING, SM_CXICON};
    const CONFIRM: i32 = 1000;
    const CANCEL: i32 = 1001;
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain([0]).collect() };
    let title = wide(dialog.title);
    let instruction = wide(dialog.instruction);
    let content = dialog.content.map(wide);
    let checkbox = dialog.checkbox.map(wide);
    let labels = [wide(dialog.confirm), wide(dialog.cancel)];
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: CONFIRM,
            pszButtonText: labels[0].as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: CANCEL,
            pszButtonText: labels[1].as_ptr(),
        },
    ];
    let show = comctl32_function(c"TaskDialogIndirect")
        .ok_or("TaskDialogIndirect is missing: Common Controls 6 is not active")?;
    // SAFETY: TaskDialogIndirect has this signature (CommCtrl.h); the
    // pointer comes from GetProcAddress for that very name.
    let show: TaskDialogIndirectFn = unsafe { std::mem::transmute(show) };
    let owner = dialog.owner.unwrap_or(0) as HWND;
    let mut icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON = std::ptr::null_mut();
    if dialog.warning_icon
        && let Some(load) = comctl32_function(c"LoadIconWithScaleDown")
    {
        // SAFETY: LoadIconWithScaleDown has this signature (CommCtrl.h); the
        // pointer comes from GetProcAddress for that very name.
        let load: LoadIconWithScaleDownFn = unsafe { std::mem::transmute(load) };
        // SAFETY: both take plain values; an invalid or null HWND makes
        // GetDpiForWindow return 0.
        let size = unsafe {
            let dpi = match GetDpiForWindow(owner) {
                0 => 96,
                dpi => dpi,
            };
            GetSystemMetricsForDpi(SM_CXICON, dpi)
        };
        // SAFETY: a system icon (null instance, IDI_ resource id) at a
        // positive size into a live local out-parameter; on failure `icon`
        // stays null and the dialog shows no icon.
        if unsafe { load(std::ptr::null_mut(), IDI_WARNING, size, size, &mut icon) } < 0 {
            icon = std::ptr::null_mut();
        }
    }
    let mut flags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT;
    if !owner.is_null() {
        flags |= TDF_POSITION_RELATIVE_TO_WINDOW;
    }
    if !icon.is_null() {
        flags |= TDF_USE_HICON_MAIN;
    }
    let text = |text: &Option<Vec<u16>>| text.as_ref().map_or(std::ptr::null(), |t| t.as_ptr());
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: flags,
        pszWindowTitle: title.as_ptr(),
        // No icon unless the handle above: `pszMainIcon` null.
        Anonymous1: TASKDIALOGCONFIG_0 { hMainIcon: icon },
        pszMainInstruction: instruction.as_ptr(),
        pszContent: text(&content),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: CONFIRM,
        pszVerificationText: text(&checkbox),
        ..Default::default()
    };
    let (mut button, mut checked) = (0i32, 0 as windows_sys::core::BOOL);
    // SAFETY: `config` and every string and button it points to are locals
    // alive until the call returns (it is modal); the out-parameters are
    // live locals, the radio-button one may be null.
    let result = unsafe { show(&config, &mut button, std::ptr::null_mut(), &mut checked) };
    if !icon.is_null() {
        // SAFETY: an icon this function loaded and no longer uses.
        unsafe { DestroyIcon(icon) };
    }
    if result < 0 {
        return Err(format!("TaskDialogIndirect failed: HRESULT {result:#010x}"));
    }
    Ok(TaskDialogAnswer {
        confirmed: button == CONFIRM,
        checked: checked != 0,
    })
}

/// Load `name` (a system DLL, for example `d3dcompiler_47.dll`) and pin it:
/// it stays mapped for the life of the process whatever `FreeLibrary`
/// calls follow. The host pins `d3dcompiler_47.dll` before wgpu's Direct3D
/// 12 windows start (`platform::select_wgpu_windows`): wgpu holds the
/// compiler's COM object, and AMD's AMF runtime unloads the DLL when
/// motion smoothing's frame-rate converter is torn down, after which the
/// next pipeline called into unmapped memory (access violation, measured
/// 2026-11-12). `Err` with the system's
/// error when it cannot be loaded or pinned.
pub fn pin_module(name: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{GetLastError, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW, LoadLibraryW,
    };
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated wide string alive for the call; the handle
    // is never freed (the module is pinned below).
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    if module.is_null() {
        // SAFETY: no arguments; reads this thread's last error.
        return Err(format!("LoadLibraryW({name}) failed: {}", unsafe {
            GetLastError()
        }));
    }
    let mut pinned: HMODULE = std::ptr::null_mut();
    // SAFETY: the same NUL-terminated name, and an out pointer to a local
    // the call writes the handle to.
    let ok =
        unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_PIN, wide.as_ptr(), &mut pinned) };
    if ok == 0 {
        // SAFETY: no arguments; reads this thread's last error.
        return Err(format!("pinning {name} failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok(())
}

/// Set an environment variable of this process (`SetEnvironmentVariableW`,
/// which the Windows API keeps consistent across threads; Rust's
/// `std::env::set_var` needs `unsafe` for the C runtime's copy, which the
/// Vulkan loader does not read: it reads the process block through
/// `GetEnvironmentVariable`). Child processes started later inherit it.
/// `Err` with the system's error.
pub fn set_process_env(name: &str, value: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Environment::SetEnvironmentVariableW;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (name_w, value_w) = (wide(name), wide(value));
    // SAFETY: two NUL-terminated wide strings alive for the call.
    if unsafe { SetEnvironmentVariableW(name_w.as_ptr(), value_w.as_ptr()) } == 0 {
        // SAFETY: no arguments; reads this thread's last error.
        let error = unsafe { GetLastError() };
        return Err(format!("SetEnvironmentVariableW({name}) failed: {error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Edges, SuperResolution, VBlank, display_areas, dxgi_adapters, file_is_remote,
        hidden_top_level, is_own_show, minimize, unmatched, version_parts, video_super_resolution,
    };

    /// Built-in panel connectors count as internal; external ones do not.
    #[test]
    fn built_in_connectors_are_internal() {
        use super::internal_output;
        use windows_sys::Win32::Devices::Display::{
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_OTHER, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
        };
        for internal in [
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
        ] {
            assert!(internal_output(internal), "{internal}");
        }
        for external in [
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST,
            DISPLAYCONFIG_OUTPUT_TECHNOLOGY_OTHER,
        ] {
            assert!(!internal_output(external), "{external}");
        }
    }

    /// Real machine, no window: every display's connector and scan-out
    /// adapter (run with `--ignored --nocapture` to see them).
    #[test]
    #[ignore = "real displays"]
    #[allow(clippy::disallowed_macros)] // a report for the terminal
    fn every_display_has_an_output() {
        use windows_sys::Win32::Foundation::{LPARAM, RECT};
        use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            list: LPARAM,
        ) -> windows_sys::core::BOOL {
            // SAFETY: `list` is the Vec passed below, alive for the call.
            unsafe { (*(list as *mut Vec<isize>)).push(monitor as isize) };
            1
        }
        let mut displays: Vec<isize> = Vec::new();
        // SAFETY: no device context or clip rectangle; the callback only
        // pushes onto `displays`, alive for the call.
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect),
                (&raw mut displays) as LPARAM,
            )
        };
        let adapters = dxgi_adapters().unwrap();
        assert!(!displays.is_empty());
        for display in displays {
            let output = super::display_output(display).unwrap();
            let adapter = adapters.iter().find(|a| a.luid == output.adapter_luid);
            eprintln!(
                "{} internal={} adapter={:?} vendor={:#06x}",
                output.device,
                output.internal,
                adapter.map(|a| &a.name),
                adapter.map_or(0, |a| a.vendor_id)
            );
            assert!(adapter.is_some(), "{output:?}");
        }
    }
    use std::time::{Duration, Instant};

    #[test]
    fn minimize_refuses_a_handle_that_is_no_window() {
        assert!(!minimize(0));
    }

    /// The extra pixel goes
    /// where no other display shows it.
    #[test]
    fn the_extra_pixel_avoids_a_neighbouring_display() {
        let display = (0, 0, 1920, 1080);
        let alone = |_: Edges| false;
        assert_eq!(
            unmatched(display, display, 1, alone),
            Some((0, 0, 1920, 1081)),
            "below by default"
        );
        assert_eq!(
            unmatched(display, display, -1, alone),
            Some((0, 0, 1920, 1079))
        );
        assert_eq!(unmatched(display, display, 0, alone), None);
        // A display directly below (only partly: it starts at x = 1000).
        let below = |(left, top, right, _): Edges| top >= 1080 && right > 1000 && left < 3000;
        assert_eq!(
            unmatched(display, display, 1, below),
            Some((0, -1, 1920, 1080)),
            "past the top instead"
        );
        // Displays below and above: past the right edge.
        let stacked = |(_, top, _, bottom): Edges| top >= 1080 || bottom <= 0;
        assert_eq!(
            unmatched(display, display, 1, stacked),
            Some((0, 0, 1921, 1080))
        );
        // Below, above and right: past the left edge.
        let three = |(left, top, _, bottom): Edges| top >= 1080 || bottom <= 0 || left >= 1920;
        assert_eq!(
            unmatched(display, display, 1, three),
            Some((-1, 0, 1920, 1080))
        );
        // Surrounded: one pixel short at the bottom.
        assert_eq!(
            unmatched(display, display, 1, |_| true),
            Some((0, 0, 1920, 1079))
        );
        // A second display at negative coordinates (left of the primary).
        let side = (-2560, 0, 0, 1440);
        assert_eq!(unmatched(side, side, 1, alone), Some((-2560, 0, 0, 1441)));
    }

    /// A window already off by the guard's pixel (for example a main window
    /// rebuilt at the old window's W×(H+1)) is left as it is: it does not grow.
    #[test]
    fn a_window_already_off_its_display_is_left_alone() {
        let display = (0, 0, 1920, 1080);
        for window in [
            (0, 0, 1920, 1081),
            (0, -1, 1920, 1080),
            (0, 0, 1920, 1079),
            (100, 100, 900, 700),
        ] {
            assert_eq!(unmatched(window, display, 1, |_| false), None, "{window:?}");
        }
    }

    #[test]
    fn super_resolution_needs_a_successful_query_with_a_value() {
        let answer = |result, value| SuperResolution { result, value }.supported();
        assert!(answer(0, 1));
        assert!(!answer(0, 0), "succeeded without saying yes");
        // E_INVALIDARG with a stale value: not supported.
        assert!(!answer(0x8007_0057_u32 as i32, 1));
    }

    #[test]
    fn session_messages_are_told_apart() {
        use super::{SessionEnd, session_end};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_ENDSESSION, WM_QUERYENDSESSION, WM_SHOWWINDOW,
        };
        assert_eq!(session_end(WM_QUERYENDSESSION, 0), Some(SessionEnd::Query));
        assert_eq!(session_end(WM_ENDSESSION, 1), Some(SessionEnd::Ending));
        assert_eq!(session_end(WM_ENDSESSION, 0), Some(SessionEnd::Cancelled));
        assert_eq!(session_end(WM_SHOWWINDOW, 1), None);
    }

    /// The memory line's numbers: this process uses some private memory,
    /// within the system's commit charge, within its limit.
    #[test]
    fn memory_status_reads_this_process_and_the_commit_charge() {
        let status = super::memory_status().expect("both calls succeed");
        assert!(status.private_bytes > 0, "{status:?}");
        assert!(status.commit_used >= status.private_bytes, "{status:?}");
        assert!(status.commit_limit >= status.commit_used, "{status:?}");
        assert!(status.physical_load <= 100, "{status:?}");
    }

    /// The supervisor's budget and pressure inputs: some memory is
    /// installed, and no more is available than installed.
    #[test]
    fn physical_memory_reads_the_total_and_what_is_available() {
        let memory = super::physical_memory().expect("the call succeeds");
        assert!(memory.total > 0, "{memory:?}");
        assert!(memory.available <= memory.total, "{memory:?}");
    }

    /// The low-memory notification opens and answers a query (whatever
    /// the machine's state); it closes on drop.
    #[test]
    fn low_memory_notification_answers_a_query() {
        let notification = super::LowMemoryNotification::new().expect("a handle");
        assert!(notification.signaled().is_some());
        drop(notification);
    }

    /// An adapter that does not exist is an error, not "unsupported".
    #[test]
    fn super_resolution_on_a_missing_adapter_is_an_error() {
        let error = video_super_resolution(u64::MAX).unwrap_err();
        assert!(error.contains("no adapter"), "{error}");
    }

    /// Real machine, read-only:
    /// `cargo test -p vestra-win32 -- --ignored --nocapture super_resolution_on_each`
    /// prints every adapter's answer. It wakes a sleeping discrete GPU for a
    /// moment; it renders nothing and changes no setting. Expected: the RTX
    /// GPU supported, AMD/Intel GPUs not (or an error), WARP an error.
    #[test]
    #[ignore = "real GPUs"]
    #[allow(clippy::disallowed_macros)] // a report for the terminal
    fn super_resolution_on_each_adapter() {
        for adapter in dxgi_adapters().unwrap() {
            let started = Instant::now();
            let answer = video_super_resolution(adapter.luid);
            eprintln!(
                "{} ({:04X}:{:04X}, LUID {:016x}, driver {:?}): {} in {:?}",
                adapter.name,
                adapter.vendor_id,
                adapter.device_id,
                adapter.luid,
                adapter.driver_version,
                match &answer {
                    Ok(found) => format!(
                        "result 0x{:08X}, value {}, supported {}",
                        found.result as u32,
                        found.value,
                        found.supported()
                    ),
                    Err(error) => format!("error: {error}"),
                },
                started.elapsed()
            );
        }
    }

    /// `on_move_size`: the hook sees a window's move/size loop messages and
    /// the timer runs only between them (a message-only window of the
    /// test's thread, messages sent as the system sends them; no loop is
    /// entered, so the test pumps the timer's messages itself).
    #[test]
    fn the_move_size_timer_runs_only_inside_the_loop() {
        use super::{MoveSize, move_size, on_move_size};
        use std::cell::RefCell;
        use std::rc::Rc;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, HWND_MESSAGE, MSG, PM_REMOVE,
            PeekMessageW, SendMessageW, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_SIZE,
        };
        assert_eq!(move_size(WM_ENTERSIZEMOVE), Some(MoveSize::Entered));
        assert_eq!(move_size(WM_EXITSIZEMOVE), Some(MoveSize::Exited));
        assert_eq!(move_size(WM_SIZE), None);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        assert!(on_move_size(10, move |what| log.borrow_mut().push(what)));
        assert!(!on_move_size(10, |_| {}), "once per thread");
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: a message-only window of the system's STATIC class; the
        // class name is a live NUL-terminated local, the other pointers null.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        let pump = |for_ms: u64| {
            let until = Instant::now() + Duration::from_millis(for_ms);
            while Instant::now() < until {
                // SAFETY: `message` is a live local the call fills; a
                // dispatched WM_TIMER calls this crate's timer procedure.
                unsafe {
                    let mut message: MSG = std::mem::zeroed();
                    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        DispatchMessageW(&message);
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        pump(60);
        assert!(seen.borrow().is_empty(), "nothing outside the loop");
        // SAFETY: a message sent to the window created above, no pointers.
        unsafe { SendMessageW(window, WM_ENTERSIZEMOVE, 0, 0) };
        pump(200);
        // SAFETY: as above.
        unsafe { SendMessageW(window, WM_EXITSIZEMOVE, 0, 0) };
        let inside = seen.borrow().len();
        pump(100);
        let seen = seen.borrow();
        assert_eq!(seen.first(), Some(&MoveSize::Entered));
        assert_eq!(seen[inside - 1], MoveSize::Exited);
        let timers = seen.iter().filter(|&&what| what == MoveSize::Timer).count();
        assert!(timers >= 3, "the timer ran inside the loop: {seen:?}");
        assert_eq!(seen.len(), inside, "nothing after the loop: {seen:?}");
        // SAFETY: destroys the window this test created, on its thread.
        unsafe { DestroyWindow(window) };
    }

    /// `cloak_on_show` cloaks a top-level window's own show from hidden
    /// only: not a child window, not a show because its owner was restored
    /// (`SW_PARENTOPENING`), not a hide, not a window already visible (no
    /// draw would follow to uncloak it), not another message.
    #[test]
    fn only_a_hidden_top_level_windows_own_show_is_cloaked() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SW_PARENTOPENING, WM_SHOWWINDOW, WM_SIZE, WS_CHILD, WS_OVERLAPPEDWINDOW,
        };
        let top = WS_OVERLAPPEDWINDOW;
        // As the hook decides: the message first, then the window.
        let shows_hidden_top_level = |message, wparam, lparam, style, visible| {
            is_own_show(message, wparam, lparam) && hidden_top_level(style, visible)
        };
        assert!(shows_hidden_top_level(WM_SHOWWINDOW, 1, 0, top, false));
        assert!(
            !shows_hidden_top_level(WM_SHOWWINDOW, 0, 0, top, false),
            "a hide"
        );
        assert!(
            !shows_hidden_top_level(WM_SHOWWINDOW, 1, SW_PARENTOPENING as isize, top, false),
            "owner restored"
        );
        assert!(
            !shows_hidden_top_level(WM_SHOWWINDOW, 1, 0, top | WS_CHILD, false),
            "a child window"
        );
        assert!(
            !shows_hidden_top_level(WM_SHOWWINDOW, 1, 0, top, true),
            "already visible"
        );
        assert!(!shows_hidden_top_level(WM_SIZE, 1, 0, top, false));
    }

    #[test]
    fn a_driver_version_splits_into_four_parts() {
        let value = (31i64 << 48) | (14005 << 16) | 21001;
        assert_eq!(version_parts(value), [31, 0, 14005, 21001]);
        assert_eq!(version_parts(-1), [u16::MAX; 4]);
    }

    /// Every Windows has at least Microsoft's Basic Render adapter or a GPU.
    #[test]
    fn dxgi_lists_the_adapters_with_names() {
        let adapters = dxgi_adapters().unwrap();
        assert!(!adapters.is_empty());
        assert!(adapters.iter().all(|adapter| !adapter.name.is_empty()));
    }

    /// A test process has no visible top-level window of its own (its
    /// console belongs to the console host): nothing is cloaked, and other
    /// processes' windows are never touched.
    #[test]
    fn a_process_without_windows_cloaks_nothing() {
        assert_eq!(super::cloak_process_windows(), 0);
    }

    /// A file on a local disk (this test executable) is not remote.
    #[test]
    fn a_local_file_is_not_remote() {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        assert!(!file_is_remote(&file));
    }

    /// Real machine (`cargo test -p vestra-win32 -- --ignored`): the same
    /// disk through the local administrative share (SMB loopback) is remote.
    /// A machine without the share (not an administrator, the server service
    /// off) passes.
    #[test]
    #[ignore = "needs the local administrative share"]
    #[allow(clippy::disallowed_macros)] // a skip note for the terminal
    fn a_file_opened_through_a_share_is_remote() {
        let Ok(file) = std::fs::File::open(r"\\localhost\C$\Windows\win.ini") else {
            eprintln!("skipped: no local administrative share on this machine");
            return;
        };
        assert!(file_is_remote(&file));
    }

    /// Real machine (`cargo test -p vestra-win32 -- --ignored`): the primary
    /// display's vertical blanks come at its refresh rate. Prints the
    /// intervals; a machine without a kernel display adapter (remote
    /// desktop) has no clock and passes.
    #[test]
    #[ignore = "needs a display"]
    #[allow(clippy::disallowed_macros)] // a measurement for the terminal
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

    /// The desktop has one primary display, and each work area lies inside
    /// its display.
    #[test]
    fn display_areas_have_one_primary_and_work_areas_inside() {
        let areas = display_areas();
        assert_eq!(
            areas.iter().filter(|area| area.primary).count(),
            1,
            "{areas:?}"
        );
        for area in &areas {
            let (left, top, right, bottom) = area.work;
            let (m_left, m_top, m_right, m_bottom) = area.monitor;
            assert!(left >= m_left && top >= m_top && right <= m_right && bottom <= m_bottom);
            assert!(right > left && bottom > top, "{area:?}");
        }
    }

    /// `set_normal_placement`'s show commands: a shown window gets its frame
    /// on its display first, then its state; a minimized one stays
    /// minimized and inactive, restoring maximized when it was, and moves
    /// to another display (without activation) only when it must.
    #[test]
    fn a_minimized_window_is_placed_without_restoring_it() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
            WPF_RESTORETOMAXIMIZED,
        };
        let steps = super::placement_steps;
        assert_eq!(
            steps(false, false, false),
            (0, [Some(SW_SHOWNORMAL), Some(SW_SHOWNORMAL)])
        );
        assert_eq!(
            steps(true, false, true),
            (0, [Some(SW_SHOWNORMAL), Some(SW_SHOWMAXIMIZED)])
        );
        assert_eq!(
            steps(false, true, false),
            (0, [None, Some(SW_SHOWMINNOACTIVE)])
        );
        assert_eq!(
            steps(true, true, false),
            (WPF_RESTORETOMAXIMIZED, [None, Some(SW_SHOWMINNOACTIVE)])
        );
        assert_eq!(
            steps(true, true, true),
            (
                WPF_RESTORETOMAXIMIZED,
                [Some(SW_SHOWNOACTIVATE), Some(SW_SHOWMINNOACTIVE)]
            )
        );
    }

    /// A top-level window of the test thread that nobody sees: created
    /// hidden, cloaked, destroyed on drop.
    struct UnseenWindow(isize);

    impl UnseenWindow {
        fn new() -> Self {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                CW_USEDEFAULT, CreateWindowExW, WS_OVERLAPPEDWINDOW,
            };
            // SAFETY: the class and title are static NUL-terminated UTF-16
            // strings (a system class: no registration, no instance); no
            // parent, menu or creation data. The handle is owned by this
            // value and destroyed on this thread in `drop`.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    windows_sys::w!("STATIC"),
                    windows_sys::w!("vestra-win32 placement test"),
                    WS_OVERLAPPEDWINDOW,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "CreateWindowExW failed");
            let window = Self(hwnd as isize);
            assert!(super::set_cloaked(window.0, true));
            window
        }
    }

    impl Drop for UnseenWindow {
        fn drop(&mut self) {
            use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
            // SAFETY: the window was created on this thread by `new` and is
            // destroyed once.
            unsafe { DestroyWindow(self.0 as _) };
        }
    }

    /// Real window round trip: what `set_normal_placement` sets,
    /// `normal_placement` reads back (frame in screen coordinates, the
    /// workspace offset undone both ways; maximized). The window is
    /// cloaked and minimized without activation first, so the test never
    /// shows a window nor takes the focus (a shown, not minimized window
    /// would be activated by `SW_SHOWNORMAL`); placing it also checks that
    /// a minimized window stays so ("Run: Minimized").
    #[test]
    fn a_placement_set_reads_back_the_same() {
        use super::{NormalPlacement, normal_placement, set_normal_placement};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWMINNOACTIVE, ShowWindow};
        let window = UnseenWindow::new();
        // Twice: a process's first ShowWindow may take its STARTUPINFO's
        // show state instead.
        for _ in 0..2 {
            // SAFETY: a window of this thread, alive; a plain show command.
            unsafe { ShowWindow(window.0 as _, SW_SHOWMINNOACTIVE) };
        }
        let (left, top, ..) = display_areas()
            .into_iter()
            .find(|area| area.primary)
            .expect("a primary display")
            .work;
        for (frame, maximized) in [
            ((left + 100, top + 80, left + 740, top + 560), false),
            ((left + 120, top + 90, left + 900, top + 610), true),
            ((left + 60, top + 40, left + 700, top + 520), false),
        ] {
            assert!(set_normal_placement(window.0, frame, maximized));
            assert_eq!(
                normal_placement(window.0),
                Some(NormalPlacement {
                    frame,
                    maximized,
                    minimized: true,
                })
            );
        }
    }
}
