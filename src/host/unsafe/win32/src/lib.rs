//! The Win32 calls that no mature safe crate wraps for the host's Windows
//! platform module (`src/host/src/platform/windows.rs`): the pointer position
//! while a file is dragged over a window (winit 0.30 reports no position for
//! drags), the modifier keys held right now, the double-click time, the
//! submenu show delay, the thread's sleep-prevention state, a window's
//! dark title bar, letting the running player take the foreground for a
//! second launch's files, whether this process owns the foreground window
//! and bringing one of its windows to the front (`foreground_is_ours`,
//! `set_foreground`), waiting for a display's vertical blank
//! (`VBlank`, the redraw clock) and keeping a full-screen window from
//! matching its display exactly (`unmatch_display`), whether an open
//! file is on a network file system (`file_is_remote`), waiting for a
//! process to end (`wait_for_process_exit`), the thread's CPU cycles
//! (`thread_cycles`), the DXGI adapters with their driver versions
//! (`dxgi_adapters`, through the `windows` crate's COM interfaces; no device
//! is created), and whether an adapter's driver offers RTX Video Super
//! Resolution (`video_super_resolution`: a short-lived Direct3D 11 device
//! and video processor). Each is one call with plain integer arguments, a
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
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MONITORINFO};
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetCursorPos, GetWindowRect, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos,
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

/// The system double-click time in milliseconds.
pub fn double_click_ms() -> u32 {
    // SAFETY: no arguments, no preconditions.
    unsafe { GetDoubleClickTime() }
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

/// A DXGI adapter as the driver check sees it (round record
/// 2026-10-render-modes §3.1).
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
/// Resolution (AI upscaling round record §2).
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
    /// accepts NVIDIA's GUID too (round record §5).
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

#[cfg(test)]
mod tests {
    use super::{
        SuperResolution, VBlank, dxgi_adapters, file_is_remote, version_parts,
        video_super_resolution,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn super_resolution_needs_a_successful_query_with_a_value() {
        let answer = |result, value| SuperResolution { result, value }.supported();
        assert!(answer(0, 1));
        assert!(!answer(0, 0), "succeeded without saying yes");
        // E_INVALIDARG with a stale value: not supported.
        assert!(!answer(0x8007_0057_u32 as i32, 1));
    }

    /// An adapter that does not exist is an error, not "unsupported".
    #[test]
    fn super_resolution_on_a_missing_adapter_is_an_error() {
        let error = video_super_resolution(u64::MAX).unwrap_err();
        assert!(error.contains("no adapter"), "{error}");
    }

    /// Real machine, read-only (AI upscaling round record §8, case U0):
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
}
