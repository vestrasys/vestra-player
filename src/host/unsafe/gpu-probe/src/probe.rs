//! The probe itself (Windows). See lib.rs for what it checks and how to run it.

use glow::HasContext;
use rsmpv_sys as mpv;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    FILETIME, GetLastError, HMODULE, HWND, LPARAM, LRESULT, LUID, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP,
    D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_GPU_PREFERENCE_MINIMUM_POWER,
    DXGI_GPU_PREFERENCE_UNSPECIFIED, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
    IDXGIFactory6,
};
use windows::Win32::Graphics::Gdi::{GetDC, HDC, ReleaseDC};
use windows::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL,
    PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat, wglCreateContext, wglDeleteContext,
    wglGetProcAddress, wglMakeCurrent,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_OWNDC, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WNDCLASSW,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{BOOL, HRESULT, Interface, PCSTR, w};

/// `MPV_RENDER_PARAM_D3D11_INIT_PARAMS` and `MPV_RENDER_PARAM_D3D11_FBO`
/// (render.h of the shipped libmpv, API 2.6; rsmpv-sys stops at 20).
const MPV_RENDER_PARAM_D3D11_INIT_PARAMS: mpv::mpv_render_param_type = 21;
const MPV_RENDER_PARAM_D3D11_FBO: mpv::mpv_render_param_type = 22;
/// Render targets in flight, as in the macOS renderer: one being drawn, one
/// being shown, one spare.
const RING: usize = 3;

/// `mpv_d3d11_init_params` (vendor/render_d3d11.h, mpv e471496c56).
#[repr(C)]
struct MpvD3d11InitParams {
    device: *mut c_void,
}
/// `mpv_d3d11_fbo` (vendor/render_d3d11.h).
#[repr(C)]
struct MpvD3d11Fbo {
    tex: *mut c_void,
}

const USAGE: &str = "usage: gpu-probe info | gpu-probe adapters | gpu-probe interop default|warp|<index> | gpu-probe run [--adapter default|warp|<index>] \
[--src <path|url>] [--size WxH] [--seconds S] [--warmup S] [--hwdec MODE] [--gl] [--host-vf] \
[--block] [--sync] [--lag N] [--png out.png] [--log LEVEL] [--vf FILTERS] [--at SECONDS]";

pub fn main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") => info(),
        Some("adapters") => list_adapters(),
        Some("interop") => interop_one(&args[1..]),
        Some("run") => Options::parse(&args[1..]).and_then(|options| run(&options)),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

// ------------------------------------------------------------------ options

#[derive(Clone, Copy, Debug)]
enum Adapter {
    Default,
    Warp,
    Index(u32),
}

struct Options {
    adapter: Adapter,
    src: String,
    size: (u32, u32),
    seconds: f64,
    warmup: f64,
    hwdec: String,
    gl: bool,
    host_vf: bool,
    block: bool,
    /// Wait for the GPU to finish each frame on the render thread (event query).
    sync: bool,
    /// Blit the frame rendered this many frames earlier (0 = the one just rendered).
    lag: usize,
    png: Option<String>,
    log: String,
    /// mpv's `vf` (appended after the host chain with --host-vf), set as an
    /// option before loadfile; e.g. `d3d11vpp=scale=2:scaling-mode=nvidia`.
    vf: Option<String>,
    /// After the measurement, pause on the frame at this time (exact seek)
    /// and read that back, so every run captures (--png) the same frame.
    at: Option<f64>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Options {
            adapter: Adapter::Default,
            src: "av://lavfi:testsrc2=s=1920x1080:r=60".into(),
            size: (1920, 1080),
            seconds: 5.0,
            warmup: 1.5,
            hwdec: "auto-safe".into(),
            gl: false,
            host_vf: false,
            block: false,
            sync: false,
            lag: 1,
            png: None,
            log: "info".into(),
            vf: None,
            at: None,
        };
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let mut value = || args.next().cloned().ok_or(format!("{flag} needs a value"));
            match flag.as_str() {
                "--adapter" => {
                    options.adapter = match value()?.as_str() {
                        "default" => Adapter::Default,
                        "warp" => Adapter::Warp,
                        n => Adapter::Index(n.parse().map_err(|_| format!("bad adapter {n}"))?),
                    }
                }
                "--src" => options.src = value()?,
                "--size" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                    options.size =
                        (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                }
                "--seconds" => options.seconds = value()?.parse().map_err(|_| "bad --seconds")?,
                "--warmup" => options.warmup = value()?.parse().map_err(|_| "bad --warmup")?,
                "--hwdec" => options.hwdec = value()?,
                "--gl" => options.gl = true,
                "--host-vf" => options.host_vf = true,
                "--block" => options.block = true,
                "--sync" => options.sync = true,
                "--lag" => options.lag = value()?.parse().map_err(|_| "bad --lag")?,
                "--png" => options.png = Some(value()?),
                "--log" => options.log = value()?,
                "--vf" => options.vf = Some(value()?),
                "--at" => {
                    let at: f64 = value()?.parse().map_err(|_| "bad --at")?;
                    if !(at.is_finite() && at >= 0.0) {
                        return Err("bad --at".into());
                    }
                    options.at = Some(at);
                }
                _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
            }
        }
        Ok(options)
    }

    /// The `vf` option to set before loadfile, if any.
    fn filter_chain(&self) -> Option<String> {
        // The host's initial filter chain (source_geometry::INITIAL_FILTERS).
        const HOST: &str =
            "@vestra-orientation:lavfi=[null],@vestra-orientation-metadata:format=rotate=0";
        match (self.host_vf, &self.vf) {
            (false, None) => None,
            (true, None) => Some(HOST.into()),
            (false, Some(vf)) => Some(vf.clone()),
            (true, Some(vf)) => Some(format!("{HOST},{vf}")),
        }
    }

    /// The level asked of mpv: at least `v` with --vf, so the filter's
    /// verbose lines reach the filter log summary even when the printed
    /// level (--log) is lower. Without --vf it is --log, as before.
    fn requested_log(&self) -> &str {
        if self.vf.is_some() && log_rank(&self.log) < log_rank("v") { "v" } else { &self.log }
    }
}

/// mpv log levels in increasing verbosity (client.h); unknown counts as info.
fn log_rank(level: &str) -> u8 {
    match level {
        "no" => 0,
        "fatal" => 1,
        "error" => 2,
        "warn" => 3,
        "status" => 5,
        "v" => 6,
        "debug" => 7,
        "trace" => 8,
        _ => 4,
    }
}

/// An mpv log line that is evidence for the filter chain and the decoder:
/// prefixes `vf`, `vd`, `d3d11vpp` (also as a path component, as in
/// `vf/d3d11vpp`), `user_filter_wrapper` (the filter's option parsing),
/// `d3d11va` (the render context's hwdec device, `libmpv_render/d3d11va`),
/// `ffmpeg/video`, and the NVIDIA/Intel super resolution and RTX Video HDR
/// messages of vf_d3d11vpp.
fn is_filter_line(prefix: &str, text: &str) -> bool {
    prefix
        .split('/')
        .any(|part| matches!(part, "vf" | "vd" | "d3d11vpp" | "d3d11va" | "user_filter_wrapper"))
        || prefix.starts_with("ffmpeg/video")
        || text.contains("d3d11vpp")
        || text.contains("Super Resolution")
        || text.contains("RTX Video")
}

// --------------------------------------------------------------------- DXGI

fn wide(text: &[u16]) -> String {
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}
fn luid_text(luid: LUID) -> String {
    format!("{:08x}:{:08x}", luid.HighPart as u32, luid.LowPart)
}

struct AdapterInfo {
    index: u32,
    name: String,
    vendor: u32,
    luid: LUID,
    software: bool,
    outputs: Vec<String>,
}

fn factory() -> Result<IDXGIFactory1, String> {
    // SAFETY: plain COM factory creation; the result is an owned interface.
    unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))
}

fn adapters() -> Result<Vec<AdapterInfo>, String> {
    let factory = factory()?;
    let mut list = Vec::new();
    for index in 0.. {
        // SAFETY: EnumAdapters1 returns DXGI_ERROR_NOT_FOUND past the last adapter.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break };
        // SAFETY: the adapter is live; GetDesc1 fills a plain struct.
        let desc = unsafe { adapter.GetDesc1() }.map_err(|e| e.to_string())?;
        let mut outputs = Vec::new();
        for o in 0.. {
            // SAFETY: as for EnumAdapters1.
            let Ok(output) = (unsafe { adapter.EnumOutputs(o) }) else { break };
            // SAFETY: the output is live; GetDesc fills a plain struct.
            if let Ok(d) = unsafe { output.GetDesc() } {
                let r = d.DesktopCoordinates;
                outputs.push(format!(
                    "{} {}x{} desktop={}",
                    wide(&d.DeviceName),
                    r.right - r.left,
                    r.bottom - r.top,
                    d.AttachedToDesktop.as_bool()
                ));
            }
        }
        list.push(AdapterInfo {
            index,
            name: wide(&desc.Description),
            vendor: desc.VendorId,
            luid: desc.AdapterLuid,
            software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
            outputs,
        });
    }
    Ok(list)
}

fn adapter_by_preference(preference: DXGI_GPU_PREFERENCE) -> Option<String> {
    let factory: IDXGIFactory6 = factory().ok()?.cast().ok()?;
    // SAFETY: index 0 always exists when any adapter exists; owned result.
    let adapter: IDXGIAdapter1 =
        unsafe { factory.EnumAdapterByGpuPreference(0, preference) }.ok()?;
    // SAFETY: live adapter; plain struct out.
    let desc = unsafe { adapter.GetDesc1() }.ok()?;
    Some(format!("{} (LUID {})", wide(&desc.Description), luid_text(desc.AdapterLuid)))
}

struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    name: String,
    vendor: u32,
    luid: LUID,
    level: D3D_FEATURE_LEVEL,
    video: bool,
}

fn create_device(choice: Adapter) -> Result<Device, String> {
    let adapter: Option<IDXGIAdapter> = match choice {
        Adapter::Index(i) => {
            // SAFETY: as in `adapters`.
            let a =
                unsafe { factory()?.EnumAdapters1(i) }.map_err(|e| format!("adapter {i}: {e}"))?;
            Some(a.cast().map_err(|e| e.to_string())?)
        }
        _ => None,
    };
    let driver: D3D_DRIVER_TYPE = match choice {
        Adapter::Index(_) => D3D_DRIVER_TYPE_UNKNOWN,
        Adapter::Warp => D3D_DRIVER_TYPE_WARP,
        Adapter::Default => D3D_DRIVER_TYPE_HARDWARE,
    };
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut last = String::new();
    // VIDEO_SUPPORT first (d3d11va needs it); without it only as a fallback
    // (WARP may not offer video).
    for video in [true, false] {
        let mut flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        if video {
            flags |= D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
        }
        let (mut device, mut context, mut level) = (None, None, D3D_FEATURE_LEVEL::default());
        // SAFETY: the out-pointers point at live locals; the adapter (if
        // any) is live for the call; the feature-level slice is valid.
        let made = unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                driver,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut level),
                Some(&mut context),
            )
        };
        match (made, device, context) {
            (Ok(()), Some(device), Some(context)) => {
                // SAFETY: the device is live; the multithread interface is
                // part of every D3D11.4 runtime (Windows 10+).
                unsafe {
                    let mt: ID3D11Multithread = device.cast().map_err(|e| e.to_string())?;
                    let _ = mt.SetMultithreadProtected(true);
                }
                // SAFETY: the device is live; GetAdapter/GetDesc fill owned values.
                let (name, vendor, luid) = unsafe {
                    let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
                    let desc =
                        dxgi.GetAdapter().and_then(|a| a.GetDesc()).map_err(|e| e.to_string())?;
                    (wide(&desc.Description), desc.VendorId, desc.AdapterLuid)
                };
                return Ok(Device { device, context, name, vendor, luid, level, video });
            }
            (made, _, _) => last = format!("{made:?}"),
        }
    }
    Err(format!("D3D11CreateDevice({choice:?}): {last}"))
}

fn texture(
    device: &ID3D11Device,
    (w, h): (u32, u32),
    staging: bool,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if staging {
            0
        } else {
            (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32
        },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: `desc` is a valid descriptor and the out-pointer a live local.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|e| e.to_string())?;
    texture.ok_or("CreateTexture2D gave nothing".into())
}

/// The texture's pixels as tightly packed BGRA rows, top row first.
fn read_back(d: &Device, source: &ID3D11Texture2D, (w, h): (u32, u32)) -> Result<Vec<u8>, String> {
    let staging = texture(&d.device, (w, h), true)?;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    // SAFETY: both textures belong to `d.device` and have the same size and
    // format. Map returns a pointer valid for RowPitch * h bytes until Unmap,
    // and each row read stays inside it.
    unsafe {
        d.context.CopyResource(&staging, source);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        d.context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| e.to_string())?;
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                w as usize * 4,
            );
            pixels[y * w as usize * 4..][..w as usize * 4].copy_from_slice(row);
        }
        d.context.Unmap(&staging, 0);
    }
    Ok(pixels)
}

// ----------------------------------------------------------------------- GL

type OpenDevice = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
type CloseDevice = unsafe extern "system" fn(*mut c_void) -> BOOL;
type RegisterObject =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32) -> *mut c_void;
type UnregisterObject = unsafe extern "system" fn(*mut c_void, *mut c_void) -> BOOL;
type LockObjects = unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> BOOL;
type ExtensionsString = unsafe extern "system" fn(HDC) -> *const c_char;
type GetUnsignedBytev = unsafe extern "system" fn(u32, *mut u8);

const WGL_ACCESS_READ_ONLY_NV: u32 = 0;
const GL_DEVICE_LUID_EXT: u32 = 0x9599;
const GL_DEVICE_NODE_MASK_EXT: u32 = 0x959A;

/// `WGL_NV_DX_interop` entry points of the current context's driver.
struct Interop {
    open: OpenDevice,
    close: CloseDevice,
    register: RegisterObject,
    unregister: UnregisterObject,
    lock: LockObjects,
    unlock: LockObjects,
}

/// GL entry point: wglGetProcAddress for extensions and GL > 1.1, else the
/// export of opengl32.dll (GL 1.1), as glutin does.
fn gl_proc(name: &CStr) -> *const c_void {
    // SAFETY: `name` is NUL-terminated; a context is current whenever this is
    // called (wglGetProcAddress returns null otherwise).
    let p = unsafe { wglGetProcAddress(PCSTR(name.as_ptr().cast())) };
    let p = p.map_or(0usize, |f| f as usize);
    if !matches!(p, 0 | 1 | 2 | 3 | usize::MAX) {
        return p as *const c_void;
    }
    // SAFETY: opengl32.dll is loaded (this binary imports wgl* from it); the
    // name is NUL-terminated.
    unsafe {
        let Ok(module) = GetModuleHandleW(w!("opengl32.dll")) else { return std::ptr::null() };
        GetProcAddress(module, PCSTR(name.as_ptr().cast()))
            .map_or(std::ptr::null(), |f| f as *const c_void)
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: forwarding the arguments Windows passed us unchanged.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A WGL context on a window that is never shown (no WS_VISIBLE, no
/// ShowWindow, WS_EX_NOACTIVATE): it cannot appear or take focus. Current on
/// the creating thread while it lives.
struct Gl {
    hwnd: HWND,
    hdc: HDC,
    context: HGLRC,
    gl: glow::Context,
    wgl_extensions: String,
    interop: Option<Interop>,
}

impl Gl {
    fn new() -> Result<Self, String> {
        // SAFETY: standard window-class registration and window creation
        // with static strings and a valid window procedure; failure of
        // RegisterClassW on a second call (class exists) is harmless. The
        // window is never shown. GetDC on our own CS_OWNDC window stays valid
        // until DestroyWindow. The pixel format descriptor is fully
        // initialized. The created context is made current on this thread.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("VestraGpuProbeHidden"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("VestraGpuProbeHidden"),
                w!("vestra gpu probe (hidden)"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                64,
                64,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| format!("CreateWindowExW: {e}"))?;
            let hdc = GetDC(Some(hwnd));
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cAlphaBits: 8,
                cDepthBits: 24,
                cStencilBits: 8,
                ..Default::default()
            };
            let format = ChoosePixelFormat(hdc, &pfd);
            let fail = |what: String| {
                ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
                what
            };
            if format == 0 {
                return Err(fail("ChoosePixelFormat failed".into()));
            }
            SetPixelFormat(hdc, format, &pfd).map_err(|e| fail(format!("SetPixelFormat: {e}")))?;
            let context =
                wglCreateContext(hdc).map_err(|e| fail(format!("wglCreateContext: {e}")))?;
            wglMakeCurrent(hdc, context).map_err(|e| fail(format!("wglMakeCurrent: {e}")))?;
            let gl = glow::Context::from_loader_function_cstr(gl_proc);
            let ext = gl_proc(c"wglGetExtensionsStringARB");
            let wgl_extensions = if ext.is_null() {
                String::new()
            } else {
                let ext: ExtensionsString = std::mem::transmute(ext);
                let s = ext(hdc);
                if s.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(s).to_string_lossy().into_owned()
                }
            };
            let load = |name: &CStr| {
                let p = gl_proc(name);
                (!p.is_null()).then_some(p)
            };
            let interop = (|| {
                Some(Interop {
                    open: std::mem::transmute::<*const c_void, OpenDevice>(load(
                        c"wglDXOpenDeviceNV",
                    )?),
                    close: std::mem::transmute::<*const c_void, CloseDevice>(load(
                        c"wglDXCloseDeviceNV",
                    )?),
                    register: std::mem::transmute::<*const c_void, RegisterObject>(load(
                        c"wglDXRegisterObjectNV",
                    )?),
                    unregister: std::mem::transmute::<*const c_void, UnregisterObject>(load(
                        c"wglDXUnregisterObjectNV",
                    )?),
                    lock: std::mem::transmute::<*const c_void, LockObjects>(load(
                        c"wglDXLockObjectsNV",
                    )?),
                    unlock: std::mem::transmute::<*const c_void, LockObjects>(load(
                        c"wglDXUnlockObjectsNV",
                    )?),
                })
            })();
            Ok(Gl { hwnd, hdc, context, gl, wgl_extensions, interop })
        }
    }

    fn string(&self, name: u32) -> String {
        // SAFETY: the context is current; GL_VENDOR/RENDERER/VERSION are valid.
        unsafe { self.gl.get_parameter_string(name) }
    }

    fn has(&self, extension: &str) -> bool {
        self.gl.supported_extensions().contains(extension)
            || self.wgl_extensions.split(' ').any(|e| e == extension)
    }

    /// The GL device's LUID and node mask (GL_EXT_external_objects_win32),
    /// the exact key to match a DXGI adapter.
    fn luid(&self) -> Option<(LUID, i32)> {
        if !self.has("GL_EXT_external_objects_win32") && !self.has("GL_EXT_memory_object_win32") {
            return None;
        }
        let p = gl_proc(c"glGetUnsignedBytevEXT");
        if p.is_null() {
            return None;
        }
        let mut bytes = [0u8; 8];
        // SAFETY: the context is current, the entry point is the driver's,
        // and GL_DEVICE_LUID_EXT writes exactly LUID_SIZE_EXT (8) bytes.
        let mask = unsafe {
            let get: GetUnsignedBytev = std::mem::transmute(p);
            get(GL_DEVICE_LUID_EXT, bytes.as_mut_ptr());
            self.gl.get_parameter_i32(GL_DEVICE_NODE_MASK_EXT)
        };
        let low = u32::from_le_bytes(bytes[..4].try_into().ok()?);
        let high = i32::from_le_bytes(bytes[4..].try_into().ok()?);
        Some((LUID { LowPart: low, HighPart: high }, mask))
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        // SAFETY: the context and window were created in `new` and are
        // released once, the context after it stops being current.
        unsafe {
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
            let _ = wglDeleteContext(self.context);
            ReleaseDC(Some(self.hwnd), self.hdc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// A D3D texture registered with the GL context, plus a GL texture that the
/// "window" shows (what Slint would get as a BorrowedOpenGLTexture) and the
/// two framebuffers of the blit.
struct Shared<'a> {
    gl: &'a Gl,
    device: *mut c_void,
    object: *mut c_void,
    interop_texture: glow::NativeTexture,
    own: glow::NativeTexture,
    read: glow::NativeFramebuffer,
    draw: glow::NativeFramebuffer,
    size: (u32, u32),
}

/// Step trace on stderr (flushed) with GPU_PROBE_TRACE set, to locate a
/// driver crash.
fn trace(step: &str) {
    use std::io::Write;
    if std::env::var_os("GPU_PROBE_TRACE").is_some() {
        let mut e = std::io::stderr();
        let _ = writeln!(e, "    trace: {step}");
        let _ = e.flush();
    }
}

fn last_error() -> String {
    // SAFETY: reads this thread's last-error value.
    let e = unsafe { GetLastError() };
    format!("GetLastError={} (0x{:08x})", e.0, e.0)
}

/// `wglDXOpenDeviceNV` on a D3D device: once per GL context, shared by all
/// registered textures (what a Slint window would hold).
struct InteropDevice<'a> {
    gl: &'a Gl,
    handle: *mut c_void,
}
impl<'a> InteropDevice<'a> {
    fn open(gl: &'a Gl, d: &Device) -> Result<Self, String> {
        let i = gl.interop.as_ref().ok_or("no WGL_NV_DX_interop entry points")?;
        trace("wglDXOpenDeviceNV");
        // SAFETY: the GL context is current; the D3D device is live and the
        // caller keeps it alive longer than this handle.
        let handle = unsafe { (i.open)(d.device.as_raw()) };
        if handle.is_null() {
            return Err(format!("wglDXOpenDeviceNV failed: {}", last_error()));
        }
        Ok(InteropDevice { gl, handle })
    }
}
impl Drop for InteropDevice<'_> {
    fn drop(&mut self) {
        if let Some(i) = self.gl.interop.as_ref() {
            // SAFETY: opened in `open`, closed once, after every Shared that
            // borrows it has unregistered (they borrow `self`).
            let _ = unsafe { (i.close)(self.handle) };
        }
    }
}

impl<'a> Shared<'a> {
    /// wglDXRegisterObjectNV + the GL objects.
    fn new(
        device: &'a InteropDevice<'a>,
        texture: &ID3D11Texture2D,
        size: (u32, u32),
    ) -> Result<Self, String> {
        let gl = device.gl;
        let i = gl.interop.as_ref().ok_or("no WGL_NV_DX_interop entry points")?;
        let device = device.handle;
        // SAFETY: the GL context is current; the interop device handle is
        // open for the borrow; the texture is a live COM object that outlives
        // `Shared` (the caller keeps it); all GL objects are created in the
        // current context and deleted in Drop.
        unsafe {
            let g = &gl.gl;
            let interop_texture = g.create_texture()?;
            trace("wglDXRegisterObjectNV");
            let object = (i.register)(
                device,
                texture.as_raw(),
                interop_texture.0.get(),
                glow::TEXTURE_2D,
                WGL_ACCESS_READ_ONLY_NV,
            );
            if object.is_null() {
                let error = last_error();
                g.delete_texture(interop_texture);
                return Err(format!("wglDXRegisterObjectNV failed: {error}"));
            }
            let own = g.create_texture()?;
            g.bind_texture(glow::TEXTURE_2D, Some(own));
            g.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size.0 as i32,
                size.1 as i32,
                0,
                glow::BGRA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            g.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            g.bind_texture(glow::TEXTURE_2D, None);
            let read = g.create_framebuffer()?;
            let draw = g.create_framebuffer()?;
            g.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw));
            g.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(own),
                0,
            );
            g.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            let shared = Shared { gl, device, object, interop_texture, own, read, draw, size };
            // Attach the interop texture while it is locked (its contents are
            // defined only then) and check completeness once.
            trace("wglDXLockObjectsNV (first)");
            shared.lock()?;
            trace("attach + completeness");
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
            g.framebuffer_texture_2d(
                glow::READ_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(interop_texture),
                0,
            );
            let status = g.check_framebuffer_status(glow::READ_FRAMEBUFFER);
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            shared.unlock()?;
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("interop framebuffer incomplete: 0x{status:x}"));
            }
            Ok(shared)
        }
    }

    fn lock(&self) -> Result<(), String> {
        let i = self.gl.interop.as_ref().ok_or("no interop")?;
        let mut objects = [self.object];
        // SAFETY: the device handle and object are live (registered in `new`).
        if unsafe { (i.lock)(self.device, 1, objects.as_mut_ptr()) }.as_bool() {
            Ok(())
        } else {
            Err(format!("wglDXLockObjectsNV failed: {}", last_error()))
        }
    }

    fn unlock(&self) -> Result<(), String> {
        let i = self.gl.interop.as_ref().ok_or("no interop")?;
        let mut objects = [self.object];
        // SAFETY: as for `lock`.
        if unsafe { (i.unlock)(self.device, 1, objects.as_mut_ptr()) }.as_bool() {
            Ok(())
        } else {
            Err(format!("wglDXUnlockObjectsNV failed: {}", last_error()))
        }
    }

    /// Lock, copy the D3D texture into our own GL texture, unlock: the work a
    /// Slint window does per frame. Returns the (lock, blit, unlock) CPU ms.
    fn blit(&self) -> Result<[f64; 3], String> {
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        self.lock()?;
        let lock = ms(t);
        let t = Instant::now();
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        let g = &self.gl.gl;
        // SAFETY: the context is current and the objects are ours.
        unsafe {
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.read));
            g.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.draw));
            g.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            g.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        }
        let blit = ms(t);
        let t = Instant::now();
        self.unlock()?;
        Ok([lock, blit, ms(t)])
    }

    fn finish(&self) {
        // SAFETY: the context is current.
        unsafe { self.gl.gl.finish() }
    }

    /// Our GL texture's pixels, BGRA, GL row 0 first.
    fn read_own(&self) -> Vec<u8> {
        let (w, h) = self.size;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let g = &self.gl.gl;
        // SAFETY: the context is current; the buffer holds w*h*4 bytes, the
        // exact size of a BGRA/UNSIGNED_BYTE read with pack alignment 4.
        unsafe {
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.draw));
            g.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            g.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::BGRA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            g.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        }
        pixels
    }
}

impl Drop for Shared<'_> {
    fn drop(&mut self) {
        let Some(i) = self.gl.interop.as_ref() else { return };
        let g = &self.gl.gl;
        // SAFETY: everything was created in `new` in the current context and
        // is released exactly once; the interop device outlives the borrow.
        unsafe {
            let _ = (i.unregister)(self.device, self.object);
            g.delete_framebuffer(self.read);
            g.delete_framebuffer(self.draw);
            g.delete_texture(self.own);
            g.delete_texture(self.interop_texture);
        }
    }
}

// --------------------------------------------------------------------- info

fn vendor_name(id: u32) -> &'static str {
    match id {
        0x1002 => "AMD",
        0x10DE => "NVIDIA",
        0x8086 => "Intel",
        0x1414 => "Microsoft",
        _ => "?",
    }
}

fn info() -> Result<(), String> {
    println!("== DXGI adapters (IDXGIFactory1::EnumAdapters1 order)");
    let list = adapters()?;
    for a in &list {
        println!(
            "  [{}] {} vendor=0x{:04x} ({}) LUID={} software={} outputs={:?}",
            a.index,
            a.name,
            a.vendor,
            vendor_name(a.vendor),
            luid_text(a.luid),
            a.software,
            a.outputs
        );
    }
    for (label, p) in [
        ("unspecified", DXGI_GPU_PREFERENCE_UNSPECIFIED),
        ("minimum power", DXGI_GPU_PREFERENCE_MINIMUM_POWER),
        ("high performance", DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE),
    ] {
        println!(
            "  EnumAdapterByGpuPreference({label}) -> {}",
            adapter_by_preference(p).unwrap_or("-".into())
        );
    }
    match create_device(Adapter::Default) {
        Ok(d) => println!(
            "  D3D11CreateDevice(NULL adapter, HARDWARE) -> {} LUID={}",
            d.name,
            luid_text(d.luid)
        ),
        Err(e) => println!("  D3D11CreateDevice(NULL adapter) failed: {e}"),
    }

    println!(
        "== Hidden OpenGL context (CreateWindowExW without WS_VISIBLE, legacy wglCreateContext)"
    );
    let gl = Gl::new()?;
    println!("  GL_VENDOR   = {}", gl.string(glow::VENDOR));
    println!("  GL_RENDERER = {}", gl.string(glow::RENDERER));
    println!("  GL_VERSION  = {}", gl.string(glow::VERSION));
    let gl_luid = gl.luid();
    match gl_luid {
        Some((luid, mask)) => {
            let matched = list
                .iter()
                .find(|a| a.luid == luid)
                .map_or("none".into(), |a| format!("[{}] {}", a.index, a.name));
            println!(
                "  GL_DEVICE_LUID_EXT = {} node mask {mask} -> DXGI adapter {matched}",
                luid_text(luid)
            );
        }
        None => println!("  GL_DEVICE_LUID_EXT: not available"),
    }
    for e in [
        "WGL_NV_DX_interop",
        "WGL_NV_DX_interop2",
        "WGL_NV_gpu_affinity",
        "WGL_AMD_gpu_association",
        "WGL_ARB_create_context",
        "GL_EXT_memory_object",
        "GL_EXT_memory_object_win32",
        "GL_EXT_external_objects_win32",
        "GL_EXT_semaphore_win32",
    ] {
        println!("  {e}: {}", gl.has(e));
    }

    println!("== Interop matrix: this GL context x every D3D11 device (cleared 256x256 B8G8R8A8)");
    let mut choices: Vec<Adapter> =
        list.iter().filter(|a| !a.software).map(|a| Adapter::Index(a.index)).collect();
    choices.push(Adapter::Warp);
    for choice in choices {
        match interop_check(&gl, choice) {
            Ok(text) => println!("  {choice:?}: {text}"),
            Err(e) => println!("  {choice:?}: FAIL {e}"),
        }
    }
    Ok(())
}

/// `gpu-probe adapters`: the DXGI adapter list only (no device, no GL), for
/// scripts that pick `--adapter <index>` by vendor.
fn list_adapters() -> Result<(), String> {
    for a in adapters()? {
        println!(
            "[{}] {} vendor=0x{:04x} ({}) LUID={} software={}",
            a.index,
            a.name,
            a.vendor,
            vendor_name(a.vendor),
            luid_text(a.luid),
            a.software
        );
    }
    Ok(())
}

/// `gpu-probe interop <adapter>`: one cell of the info matrix, traced.
fn interop_one(args: &[String]) -> Result<(), String> {
    let choice = match args.first().map(String::as_str) {
        Some("default") | None => Adapter::Default,
        Some("warp") => Adapter::Warp,
        Some(n) => Adapter::Index(n.parse().map_err(|_| format!("bad adapter {n}"))?),
    };
    let gl = Gl::new()?;
    let luid = gl.luid().map_or("n/a".into(), |(l, _)| luid_text(l));
    println!("GL: {} LUID {luid}", gl.string(glow::RENDERER));
    let text = interop_check(&gl, choice)?;
    println!("{choice:?}: {text}");
    Ok(())
}

/// Clear a D3D texture to a known colour on `choice`, read it in GL through
/// the interop, and compare.
fn interop_check(gl: &Gl, choice: Adapter) -> Result<String, String> {
    let d = create_device(choice)?;
    let size = (256, 256);
    let tex = texture(&d.device, size, false)?;
    let mut view = None;
    // SAFETY: the texture is a render target of this device; the out-pointer
    // is a live local; the clear colour is 4 floats.
    unsafe {
        d.device.CreateRenderTargetView(&tex, None, Some(&mut view)).map_err(|e| e.to_string())?;
        let view = view.ok_or("no RTV")?;
        d.context.ClearRenderTargetView(&view, &[0.75, 0.5, 0.25, 1.0]);
        d.context.Flush();
    }
    let started = Instant::now();
    let device = InteropDevice::open(gl, &d)?;
    let shared = Shared::new(&device, &tex, size)?;
    shared.blit()?;
    let pixels = shared.read_own();
    let open_ms = started.elapsed().as_secs_f64() * 1e3;
    let expected = [64u8, 128, 191, 255]; // BGRA of (0.75, 0.5, 0.25, 1)
    let center = &pixels[(128 * 256 + 128) * 4..][..4];
    let ok = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));
    Ok(format!(
        "D3D on {} (LUID {}, FL 0x{:x}, video={}): open+register+lock+blit+read OK in {open_ms:.1} ms, center BGRA={center:?}, all pixels match={ok}",
        d.name,
        luid_text(d.luid),
        d.level.0,
        d.video
    ))
}

// ---------------------------------------------------------------------- mpv

/// Signalled by mpv's update callback (which may only signal, render.h).
struct Wake {
    flag: Mutex<bool>,
    cond: Condvar,
}
impl Wake {
    fn wait(&self, timeout: Duration) {
        let guard = self.flag.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut guard, _) = self
            .cond
            .wait_timeout_while(guard, timeout, |set| !*set)
            .unwrap_or_else(PoisonError::into_inner);
        *guard = false;
    }
}
extern "C" fn on_update(data: *mut c_void) {
    // SAFETY: `data` is the `Wake` pointer registered in `run`, which stays
    // alive until after the render context (and so this callback) is freed.
    let wake = unsafe { &*(data as *const Wake) };
    *wake.flag.lock().unwrap_or_else(PoisonError::into_inner) = true;
    wake.cond.notify_one();
}

struct Handle(*mut mpv::mpv_handle);
// SAFETY: the libmpv client API is thread-safe for one handle (client.h).
unsafe impl Send for Handle {}
// SAFETY: as for Send.
unsafe impl Sync for Handle {}

impl Handle {
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) =
            (CString::new(name).unwrap_or_default(), CString::new(value).unwrap_or_default());
        // SAFETY: live handle, NUL-terminated strings copied by mpv.
        let r = unsafe { mpv::mpv_set_option_string(self.0, n.as_ptr(), v.as_ptr()) };
        if r < 0 { Err(format!("set {name}={value}: {}", error_text(r))) } else { Ok(()) }
    }
    fn get(&self, name: &str) -> String {
        let n = CString::new(name).unwrap_or_default();
        // SAFETY: live handle; the returned string is owned by us and freed
        // with mpv_free after copying.
        unsafe {
            let p = mpv::mpv_get_property_string(self.0, n.as_ptr());
            if p.is_null() {
                return "(unavailable)".into();
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            mpv::mpv_free(p.cast());
            s
        }
    }
    fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> =
            args.iter().map(|a| CString::new(*a).unwrap_or_default()).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // SAFETY: a NULL-terminated array of NUL-terminated strings that
        // outlive the call.
        let r = unsafe { mpv::mpv_command(self.0, pointers.as_mut_ptr()) };
        if r < 0 { Err(format!("{args:?}: {}", error_text(r))) } else { Ok(()) }
    }
}

fn error_text(code: c_int) -> String {
    // SAFETY: mpv_error_string returns a static string for any code.
    unsafe { CStr::from_ptr(mpv::mpv_error_string(code)).to_string_lossy().into_owned() }
}

/// What the event thread does with mpv's log lines: print those up to
/// `print_rank` (as before), and with `keep` also keep the filter/decoder
/// lines for the summary at the end.
struct LogSink {
    print_rank: u8,
    keep: bool,
    /// The kept lines and the number of further matching lines not kept.
    kept: Mutex<(Vec<String>, usize)>,
}
/// Filter log lines kept for the summary; the rest are only counted.
const KEPT_LINES: usize = 400;

fn events(handle: Arc<Handle>, stop: Arc<AtomicBool>, sink: Arc<LogSink>) {
    loop {
        // SAFETY: the handle stays alive until this thread is joined.
        let event = unsafe { &*mpv::mpv_wait_event(handle.0, 0.25) };
        match event.event_id {
            mpv::MPV_EVENT_SHUTDOWN => return,
            mpv::MPV_EVENT_NONE if stop.load(Ordering::Acquire) => return,
            mpv::MPV_EVENT_LOG_MESSAGE => {
                // SAFETY: for LOG_MESSAGE, data points at an
                // mpv_event_log_message whose strings live until the next
                // mpv_wait_event.
                let m = unsafe { &*(event.data as *const mpv::mpv_event_log_message) };
                // SAFETY: as above; the strings are NUL-terminated.
                let (prefix, level, text) = unsafe {
                    (CStr::from_ptr(m.prefix), CStr::from_ptr(m.level), CStr::from_ptr(m.text))
                };
                let (level, prefix, text) =
                    (level.to_string_lossy(), prefix.to_string_lossy(), text.to_string_lossy());
                let line = format!("  [mpv {level} {prefix}] {text}");
                if log_rank(&level) <= sink.print_rank {
                    print!("{line}");
                }
                if sink.keep && is_filter_line(&prefix, &text) {
                    let mut kept = sink.kept.lock().unwrap_or_else(PoisonError::into_inner);
                    if kept.0.len() < KEPT_LINES {
                        kept.0.push(line);
                    } else {
                        kept.1 += 1;
                    }
                }
            }
            mpv::MPV_EVENT_END_FILE => println!("  [event] end-file"),
            mpv::MPV_EVENT_FILE_LOADED => println!("  [event] file-loaded"),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------- run

fn process_cpu_seconds() -> f64 {
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: the pseudo-handle of the current process and live out-params.
    let _ = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 * 1e-7;
    t(k) + t(u)
}

/// D3D timestamp queries around one render: GPU time without stalling.
struct GpuTimer {
    disjoint: ID3D11Query,
    begin: ID3D11Query,
    end: ID3D11Query,
    pending: bool,
}
impl GpuTimer {
    fn new(device: &ID3D11Device) -> Result<Self, String> {
        let make = |kind| {
            let mut q = None;
            let desc = D3D11_QUERY_DESC { Query: kind, MiscFlags: 0 };
            // SAFETY: valid descriptor; live out-pointer.
            unsafe { device.CreateQuery(&desc, Some(&mut q)) }.map_err(|e| e.to_string())?;
            q.ok_or_else(|| "CreateQuery".to_string())
        };
        Ok(GpuTimer {
            disjoint: make(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
            begin: make(D3D11_QUERY_TIMESTAMP)?,
            end: make(D3D11_QUERY_TIMESTAMP)?,
            pending: false,
        })
    }
    /// Non-blocking GetData: Some(bytes) when the query is done.
    fn get<T: Default>(context: &ID3D11DeviceContext, query: &ID3D11Query) -> Option<T> {
        let mut value = T::default();
        // SAFETY: calls the vtable directly to see S_FALSE ("not ready"),
        // which the wrapper folds into Ok; the buffer is a live T of the size
        // the query type returns (u64 or the disjoint struct).
        let hr: HRESULT = unsafe {
            (Interface::vtable(context).GetData)(
                context.as_raw(),
                query.as_raw(),
                (&mut value as *mut T).cast(),
                size_of::<T>() as u32,
                D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
            )
        };
        (hr.0 == 0).then_some(value)
    }
    /// The GPU milliseconds of the previous use, if done.
    fn poll(&mut self, context: &ID3D11DeviceContext) -> Option<f64> {
        if !self.pending {
            return None;
        }
        let d: D3D11_QUERY_DATA_TIMESTAMP_DISJOINT = Self::get(context, &self.disjoint)?;
        let b: u64 = Self::get(context, &self.begin)?;
        let e: u64 = Self::get(context, &self.end)?;
        self.pending = false;
        (!d.Disjoint.as_bool() && d.Frequency > 0)
            .then(|| (e - b) as f64 * 1e3 / d.Frequency as f64)
    }
}

#[derive(Default)]
struct Series(Vec<f64>);
impl Series {
    fn summary(&self) -> String {
        if self.0.is_empty() {
            return "n=0".into();
        }
        let mut v = self.0.clone();
        v.sort_by(f64::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        format!(
            "n={} mean={mean:.2} p50={:.2} p95={:.2} p99={:.2} max={:.2} ms",
            v.len(),
            at(0.5),
            at(0.95),
            at(0.99),
            at(1.0)
        )
    }
}

fn picture_stats(bgra: &[u8]) -> String {
    let mut colors = std::collections::HashSet::new();
    let (mut sum, mut dark) = (0f64, 0usize);
    let n = bgra.len() / 4;
    for p in bgra.as_chunks::<4>().0.iter().step_by(7) {
        colors.insert([p[0] >> 3, p[1] >> 3, p[2] >> 3]);
        let y = 0.0722 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.2126 * p[2] as f64;
        sum += y;
        if y < 8.0 {
            dark += 1;
        }
    }
    let sampled = n.div_ceil(7);
    format!(
        "mean luma {:.1}, {:.1}% near-black, {} distinct 15-bit colours (1/7 sample)",
        sum / sampled as f64,
        100.0 * dark as f64 / sampled as f64,
        colors.len()
    )
}

fn write_png(path: &str, bgra: &[u8], (w, h): (u32, u32)) -> Result<(), String> {
    let rgba: Vec<u8> =
        bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    // A 1/4 preview for quick viewing.
    let (pw, ph) = (w / 4, h / 4);
    let small: Vec<u8> = (0..ph)
        .flat_map(|y| (0..pw).map(move |x| ((y * 4 * w + x * 4) * 4) as usize))
        .flat_map(|i| [bgra[i + 2], bgra[i + 1], bgra[i], 255])
        .collect();
    let preview = path.replace(".png", "-small.png");
    let file = std::fs::File::create(&preview).map_err(|e| format!("{preview}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&small).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

fn compare(d3d: &[u8], gl: &[u8], (w, h): (u32, u32)) -> String {
    let row = w as usize * 4;
    let flipped: Vec<u8> = gl.chunks_exact(row).rev().flatten().copied().collect();
    let score = |other: &[u8]| {
        let mut max = 0u8;
        let mut equal = 0usize;
        for (a, b) in d3d.as_chunks::<4>().0.iter().zip(other.as_chunks::<4>().0.iter()) {
            let diff = a[..3].iter().zip(&b[..3]).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            max = max.max(diff);
            equal += usize::from(diff == 0);
        }
        (equal, max)
    };
    let (same_eq, same_max) = score(gl);
    let (flip_eq, flip_max) = score(&flipped);
    let n = (w * h) as usize;
    format!(
        "GL row 0 = D3D row 0: {:.4}% identical (max channel diff {same_max}); GL rows flipped: {:.4}% identical (max {flip_max})",
        100.0 * same_eq as f64 / n as f64,
        100.0 * flip_eq as f64 / n as f64
    )
}

/// One non-blocking `mpv_render_context_render` into `target` (a render
/// target of the device the render context was created on).
fn render_into(render: *mut mpv::mpv_render_context, target: &ID3D11Texture2D) -> c_int {
    let mut fbo = MpvD3d11Fbo { tex: target.as_raw() };
    let mut block: c_int = 0;
    let mut rp = [
        mpv::mpv_render_param {
            type_: MPV_RENDER_PARAM_D3D11_FBO,
            data: (&mut fbo as *mut MpvD3d11Fbo).cast(),
        },
        mpv::mpv_render_param {
            type_: mpv::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
            data: (&mut block as *mut c_int).cast(),
        },
        mpv::mpv_render_param { type_: mpv::MPV_RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
    ];
    // SAFETY: the render context is live and used from this thread only; the
    // param array is terminated and its values outlive the call.
    unsafe { mpv::mpv_render_context_render(render, rp.as_mut_ptr()) }
}

/// Pause, seek exactly to `at`, wait until mpv shows that frame, and render
/// it into `target`. Returns a note on where it stopped.
fn capture_at(
    handle: &Handle,
    render: *mut mpv::mpv_render_context,
    wake: &Wake,
    target: &ID3D11Texture2D,
    at: f64,
) -> String {
    let started = Instant::now();
    // Not fatal: without video (say a filter failed) there is nothing to
    // seek, but the results and the mpv log still matter.
    if let Err(e) = handle
        .command(&["set", "pause", "yes"])
        .and_then(|()| handle.command(&["seek", &format!("{at}"), "absolute+exact"]))
    {
        render_into(render, target);
        return format!("WARNING: {e}; the readback is the frame mpv shows now");
    }
    // Shown: the first frame at or after `at` (hr-seek keeps frames down to
    // 5 ms before the target); anything later than 100 ms is not it.
    let shown = |pos: f64| pos >= at - 0.01 && pos < at + 0.1;
    let (mut settled_at, mut frames) = (None::<Instant>, 0u32);
    while started.elapsed() < Duration::from_secs(5) {
        wake.wait(Duration::from_millis(20));
        // SAFETY: live render context, this (render) thread only.
        let flags = unsafe { mpv::mpv_render_context_update(render) };
        if flags & mpv::MPV_RENDER_UPDATE_FRAME as u64 != 0 {
            // Consume the frame (as a player would) so mpv moves on.
            render_into(render, target);
            frames += 1;
            settled_at = None;
            continue;
        }
        let pos: Option<f64> = handle.get("time-pos").parse().ok();
        if frames > 0 && handle.get("seeking") == "no" && pos.is_some_and(shown) {
            // Settled once nothing new arrived for 150 ms.
            let since = *settled_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(150) {
                let r = render_into(render, target);
                if r < 0 {
                    return format!("WARNING: mpv_render_context_render: {}", error_text(r));
                }
                return format!(
                    "paused on time-pos={} (estimated-frame-number={}) after {:.0} ms, {frames} frame(s) after the seek",
                    handle.get("time-pos"),
                    handle.get("estimated-frame-number"),
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
        }
    }
    // Render whatever is current, but say so.
    render_into(render, target);
    format!(
        "WARNING: the seek did not settle within 5 s (time-pos={}, seeking={}); the readback may be another frame",
        handle.get("time-pos"),
        handle.get("seeking")
    )
}

fn run(o: &Options) -> Result<(), String> {
    let d = create_device(o.adapter)?;
    println!(
        "D3D11 device: {} LUID={} FL=0x{:x} VIDEO_SUPPORT={} multithread-protected",
        d.name,
        luid_text(d.luid),
        d.level.0,
        d.video
    );
    let gl = if o.gl { Some(Gl::new()?) } else { None };
    if let Some(gl) = &gl {
        println!(
            "GL context (hidden window): {} / {} / {}; GL LUID {}",
            gl.string(glow::VENDOR),
            gl.string(glow::RENDERER),
            gl.string(glow::VERSION),
            gl.luid().map_or("n/a".into(), |(l, _)| luid_text(l))
        );
    }

    let targets: Vec<ID3D11Texture2D> =
        (0..RING).map(|_| texture(&d.device, o.size, false)).collect::<Result<_, _>>()?;
    // SAFETY: plain libmpv handle creation; null is checked.
    let handle = Arc::new(Handle(unsafe { mpv::mpv_create() }));
    if handle.0.is_null() {
        return Err("mpv_create failed".into());
    }
    handle.set("vo", "libmpv")?;
    handle.set("ao", "null")?; // never make sound on the user's machine
    handle.set("hwdec", &o.hwdec)?;
    handle.set("loop-file", "inf")?;
    handle.set("keep-open", "yes")?;
    handle.set("terminal", "no")?;
    // As an option before loadfile: mpv builds the chain when the file loads,
    // after the render context below exists, so d3d11vpp finds the d3d11va
    // device of that context.
    if let Some(vf) = o.filter_chain() {
        handle.set("vf", &vf)?;
    }
    if o.at.is_some() {
        // Exact seeks (the capture at the end): the frame shown is the first
        // one at or after the target, the same in every run.
        handle.set("hr-seek", "yes")?;
    }
    // SAFETY: live handle.
    let r = unsafe { mpv::mpv_initialize(handle.0) };
    if r < 0 {
        return Err(format!("mpv_initialize: {}", error_text(r)));
    }
    let level = CString::new(o.requested_log()).unwrap_or_default();
    // SAFETY: live handle, NUL-terminated level.
    unsafe { mpv::mpv_request_log_messages(handle.0, level.as_ptr()) };
    let stop = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(LogSink {
        print_rank: log_rank(&o.log),
        keep: o.vf.is_some(),
        kept: Mutex::new((Vec::new(), 0)),
    });
    let event_thread = {
        let (handle, stop, sink) = (handle.clone(), stop.clone(), sink.clone());
        std::thread::spawn(move || events(handle, stop, sink))
    };

    // The render context on our device.
    let mut init = MpvD3d11InitParams { device: d.device.as_raw() };
    let mut params = [
        mpv::mpv_render_param {
            type_: mpv::MPV_RENDER_PARAM_API_TYPE,
            data: c"d3d11".as_ptr().cast_mut().cast(),
        },
        mpv::mpv_render_param {
            type_: MPV_RENDER_PARAM_D3D11_INIT_PARAMS,
            data: (&mut init as *mut MpvD3d11InitParams).cast(),
        },
        mpv::mpv_render_param { type_: mpv::MPV_RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
    ];
    let mut render: *mut mpv::mpv_render_context = std::ptr::null_mut();
    let created = Instant::now();
    // SAFETY: the param array is terminated with INVALID; the pointed-to
    // values outlive the call; mpv adds its own reference to the device.
    let r = unsafe { mpv::mpv_render_context_create(&mut render, handle.0, params.as_mut_ptr()) };
    if r < 0 {
        stop.store(true, Ordering::Release);
        let _ = event_thread.join();
        // SAFETY: live handle, destroyed once.
        unsafe { mpv::mpv_terminate_destroy(handle.0) };
        return Err(format!("mpv_render_context_create(d3d11): {} ({r})", error_text(r)));
    }
    println!(
        "mpv_render_context_create(api_type=d3d11): OK in {:.1} ms",
        created.elapsed().as_secs_f64() * 1e3
    );
    let wake = Box::new(Wake { flag: Mutex::new(false), cond: Condvar::new() });
    // SAFETY: `wake` outlives the render context (freed below before it).
    unsafe {
        mpv::mpv_render_context_set_update_callback(
            render,
            Some(on_update),
            (&*wake as *const Wake).cast_mut().cast(),
        )
    };
    handle.command(&["loadfile", &o.src])?;

    let interop = match &gl {
        Some(gl) => Some(InteropDevice::open(gl, &d)?),
        None => None,
    };
    let shared: Vec<Shared> = match &interop {
        Some(device) => {
            targets.iter().map(|t| Shared::new(device, t, o.size)).collect::<Result<_, _>>()?
        }
        None => Vec::new(),
    };
    if !shared.is_empty() {
        println!(
            "wglDXOpenDeviceNV once + wglDXRegisterObjectNV(GL_TEXTURE_2D) x {RING}: OK; blitting the frame rendered {} frame(s) earlier",
            o.lag
        );
    }
    // Ring index of each rendered frame, newest last.
    let mut rendered: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let (mut lock_ms, mut copy_ms, mut unlock_ms) =
        (Series::default(), Series::default(), Series::default());
    let mut sync_ms = Series::default();
    let event = {
        let mut q = None;
        let desc = D3D11_QUERY_DESC { Query: D3D11_QUERY_EVENT, MiscFlags: 0 };
        // SAFETY: valid descriptor; live out-pointer.
        unsafe { d.device.CreateQuery(&desc, Some(&mut q)) }.map_err(|e| e.to_string())?;
        q.ok_or("CreateQuery(EVENT)")?
    };
    // 1 = mpv blocks until the frame's display time (the host's unpaced mode).
    let mut block: c_int = c_int::from(o.block);
    let mut timers: Vec<GpuTimer> =
        (0..8).map(|_| GpuTimer::new(&d.device)).collect::<Result<_, _>>()?;
    let (mut cpu_ms, mut gpu_ms, mut blit_ms, mut finish_ms, mut gaps) = (
        Series::default(),
        Series::default(),
        Series::default(),
        Series::default(),
        Series::default(),
    );
    let start = Instant::now();
    let total = Duration::from_secs_f64(o.warmup + o.seconds);
    let warm = Duration::from_secs_f64(o.warmup);
    let (mut measured_from, mut cpu_from, mut frames, mut all_frames, mut last_frame) =
        (None::<Instant>, 0.0, 0u64, 0u64, None::<Instant>);
    let mut errors = 0u32;
    while start.elapsed() < total {
        wake.wait(Duration::from_millis(100));
        // SAFETY: live render context, called from this (render) thread only.
        let flags = unsafe { mpv::mpv_render_context_update(render) };
        if flags & mpv::MPV_RENDER_UPDATE_FRAME as u64 == 0 {
            continue;
        }
        let measuring = start.elapsed() >= warm;
        if measuring && measured_from.is_none() {
            measured_from = Some(Instant::now());
            cpu_from = process_cpu_seconds();
            println!(
                "  at end of warm-up: hwdec-current={} frame-drop-count={} decoder-frame-drop-count={}",
                handle.get("hwdec-current"),
                handle.get("frame-drop-count"),
                handle.get("decoder-frame-drop-count")
            );
        }
        let slot = (all_frames % timers.len() as u64) as usize;
        if let Some(ms) = timers[slot].poll(&d.context)
            && measuring
        {
            gpu_ms.0.push(ms);
        }
        let timer = &mut timers[slot];
        let can_time = !timer.pending;
        let index = (all_frames % RING as u64) as usize;
        let mut fbo = MpvD3d11Fbo { tex: targets[index].as_raw() };
        let mut rp = [
            mpv::mpv_render_param {
                type_: MPV_RENDER_PARAM_D3D11_FBO,
                data: (&mut fbo as *mut MpvD3d11Fbo).cast(),
            },
            mpv::mpv_render_param {
                type_: mpv::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: (&mut block as *mut c_int).cast(),
            },
            mpv::mpv_render_param {
                type_: mpv::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let t0 = Instant::now();
        // SAFETY: the queries belong to this device; mpv runs on the same
        // immediate context only inside mpv_render_context_render, on this
        // thread, so nothing else uses the context concurrently except mpv's
        // decoder under the multithread lock. The param array is terminated
        // and its values outlive the call; the texture is a render target of
        // the device mpv was given.
        let r = unsafe {
            if can_time {
                d.context.Begin(&timer.disjoint);
                d.context.End(&timer.begin);
            }
            let r = mpv::mpv_render_context_render(render, rp.as_mut_ptr());
            if can_time {
                d.context.End(&timer.end);
                d.context.End(&timer.disjoint);
                timer.pending = true;
                // Submit the end timestamps now; otherwise they wait in the
                // command buffer until the next frame's flush.
                d.context.Flush();
            }
            r
        };
        let elapsed = t0.elapsed().as_secs_f64() * 1e3;
        if r < 0 {
            errors += 1;
            if errors < 5 {
                println!("mpv_render_context_render: {}", error_text(r));
            }
            continue;
        }
        if o.sync {
            // The render thread's equivalent of the macOS renderer's glFinish:
            // wait (on this thread) until the GPU has finished the frame.
            let t = Instant::now();
            // SAFETY: the query belongs to this device; GetData is polled
            // until it reports completion (S_OK).
            unsafe {
                d.context.End(&event);
                d.context.Flush();
                while GpuTimer::get::<BOOL>(&d.context, &event).is_none() {
                    std::thread::yield_now();
                }
            }
            if measuring {
                sync_ms.0.push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
        if all_frames == 0 {
            println!(
                "  first frame: hwdec-current={} video-params={} video-out-params={}",
                handle.get("hwdec-current"),
                handle.get("video-params"),
                handle.get("video-out-params")
            );
        }
        all_frames += 1;
        rendered.push_back(index);
        if rendered.len() > RING {
            rendered.pop_front();
        }
        // The "window": show the frame rendered `lag` frames ago.
        let shown = rendered.len().checked_sub(1 + o.lag).map(|at| rendered[at]);
        if let Some(s) = shown.and_then(|i| shared.get(i)) {
            let t1 = Instant::now();
            let [l, c, u] = s.blit()?;
            let b = t1.elapsed().as_secs_f64() * 1e3;
            s.finish();
            if measuring {
                lock_ms.0.push(l);
                copy_ms.0.push(c);
                unlock_ms.0.push(u);
                blit_ms.0.push(b);
                finish_ms.0.push(t1.elapsed().as_secs_f64() * 1e3);
            }
        }
        let now = Instant::now();
        if measuring {
            frames += 1;
            cpu_ms.0.push(elapsed);
            if let Some(last) = last_frame {
                gaps.0.push((now - last).as_secs_f64() * 1e3);
            }
        }
        last_frame = Some(now);
    }
    let wall = measured_from.map_or(0.0, |t| t.elapsed().as_secs_f64());
    let cpu = process_cpu_seconds() - cpu_from;
    if let Some(at) = o.at {
        // Measurement done: pause on the frame at `at` and render it into a
        // target of its own, which becomes "the last rendered frame" below.
        let slot = (all_frames % RING as u64) as usize;
        let note = capture_at(&handle, render, &wake, &targets[slot], at);
        println!("  --at {at}: {note}");
        rendered.push_back(slot);
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    // Drain the outstanding GPU timers.
    // SAFETY: flushes this thread's own immediate-context work.
    unsafe { d.context.Flush() };
    std::thread::sleep(Duration::from_millis(50));
    for t in &mut timers {
        if let Some(ms) = t.poll(&d.context) {
            gpu_ms.0.push(ms);
        }
    }

    println!(
        "== Results ({} x {}, {} s measured after {} s warm-up)",
        o.size.0, o.size.1, o.seconds, o.warmup
    );
    for p in [
        "current-vo",
        "hwdec",
        "hwdec-current",
        "hwdec-interop",
        "video-codec",
        "video-params/pixelformat",
        "video-params/hw-pixelformat",
        "video-params/w",
        "video-params/h",
        "container-fps",
        "estimated-vf-fps",
        "frame-drop-count",
        "decoder-frame-drop-count",
        "vo-delayed-frame-count",
        "mistimed-frame-count",
        "vf",
        "video-params",
        "video-out-params",
        "time-pos",
        "estimated-frame-number",
    ] {
        println!("  {p} = {}", handle.get(p));
    }
    println!(
        "  adapter = {} vendor=0x{:04x} ({}) LUID={}",
        d.name,
        d.vendor,
        vendor_name(d.vendor),
        luid_text(d.luid)
    );
    let (out_w, out_h) = (handle.get("video-out-params/w"), handle.get("video-out-params/h"));
    let same = out_w == o.size.0.to_string() && out_h == o.size.1.to_string();
    println!(
        "  render target {}x{}, filter output (video-out-params) {out_w}x{out_h}: {}",
        o.size.0,
        o.size.1,
        if same { "same size, drawn 1:1" } else { "mpv scales it to the target" }
    );
    println!(
        "  frames rendered: {frames} in {wall:.2} s = {:.2} fps ({all_frames} incl. warm-up, {errors} render errors)",
        frames as f64 / wall.max(1e-9)
    );
    println!("  frame interval: {}", gaps.summary());
    println!("  mpv_render_context_render CPU wall: {}", cpu_ms.summary());
    println!("  mpv render GPU time (timestamp queries): {}", gpu_ms.summary());
    if o.sync {
        println!("  render thread wait for GPU completion (event query): {}", sync_ms.summary());
    }
    if !shared.is_empty() {
        println!("  interop wglDXLockObjectsNV: {}", lock_ms.summary());
        println!("  interop glBlitFramebuffer (CPU submit): {}", copy_ms.summary());
        println!("  interop wglDXUnlockObjectsNV: {}", unlock_ms.summary());
        println!("  interop lock+blit+unlock CPU: {}", blit_ms.summary());
        println!("  interop lock+blit+unlock+glFinish: {}", finish_ms.summary());
    }
    if measured_from.is_some() {
        println!(
            "  process CPU: {cpu:.2} s over {wall:.2} s = {:.2} cores = {:.1}% of {cores} logical CPUs",
            cpu / wall.max(1e-9),
            100.0 * cpu / wall.max(1e-9) / cores
        );
    } else {
        // Paused (--at) or no frame after the warm-up: nothing was measured.
        println!("  process CPU: n/a (no frame rendered after the warm-up)");
    }

    // The last rendered frame, from D3D and (with --gl, after one more blit
    // of that same texture) from GL.
    let last = rendered.back().copied().unwrap_or(0);
    if rendered.is_empty() {
        println!("  WARNING: no frame was rendered; the readback is of an empty target");
    }
    let pixels = read_back(&d, &targets[last], o.size)?;
    println!("  D3D readback: {}", picture_stats(&pixels));
    if let Some(path) = &o.png {
        write_png(path, &pixels, o.size)?;
        println!("  wrote {path} (and -small preview)");
    }
    if let Some(s) = shared.get(last) {
        s.blit()?;
        let gl_pixels = s.read_own();
        println!("  GL readback: {}", picture_stats(&gl_pixels));
        println!("  D3D vs GL: {}", compare(&pixels, &gl_pixels, o.size));
    }

    drop(shared);
    drop(interop);
    stop.store(true, Ordering::Release);
    // SAFETY: live handle; wakes the event thread so it sees `stop`.
    unsafe { mpv::mpv_wakeup(handle.0) };
    let _ = event_thread.join();
    if sink.keep {
        let kept = sink.kept.lock().unwrap_or_else(PoisonError::into_inner);
        println!(
            "== mpv filter/decoder log (prefixes vf, vd, d3d11vpp, ffmpeg/video; Super Resolution, RTX Video; mpv level {})",
            o.requested_log()
        );
        for line in &kept.0 {
            print!("{line}");
        }
        if kept.1 > 0 {
            println!("  ... {} more matching lines not kept", kept.1);
        }
    }
    // SAFETY: the render context is freed before the handle, both once; the
    // update callback cannot run after mpv_render_context_free returns, so
    // `wake` may drop after it.
    unsafe {
        mpv::mpv_render_context_free(render);
        mpv::mpv_terminate_destroy(handle.0);
    }
    drop(wake);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn vf_and_at_are_parsed_and_default_to_none() {
        let o = parse(&[]).unwrap();
        assert!(o.vf.is_none() && o.at.is_none() && o.filter_chain().is_none());
        assert_eq!(o.requested_log(), "info");
        let o = parse(&["--vf", "d3d11vpp=scale=2:scaling-mode=nvidia", "--at", "5"]).unwrap();
        assert_eq!(o.filter_chain().as_deref(), Some("d3d11vpp=scale=2:scaling-mode=nvidia"));
        assert_eq!(o.at, Some(5.0));
        assert!(parse(&["--at", "-1"]).is_err());
        assert!(parse(&["--at", "NaN"]).is_err());
        assert!(parse(&["--at", "x"]).is_err());
        assert!(parse(&["--vf"]).is_err());
    }

    #[test]
    fn host_vf_comes_first() {
        let o = parse(&["--host-vf", "--vf", "d3d11vpp=scale=2"]).unwrap();
        let chain = o.filter_chain().unwrap();
        assert!(chain.starts_with("@vestra-orientation:lavfi=[null],"));
        assert!(chain.ends_with("rotate=0,d3d11vpp=scale=2"));
        assert!(parse(&["--host-vf"]).unwrap().filter_chain().unwrap().ends_with("rotate=0"));
    }

    #[test]
    fn vf_raises_only_the_requested_log_level() {
        let o = parse(&["--vf", "d3d11vpp"]).unwrap();
        assert_eq!((o.requested_log(), log_rank(&o.log)), ("v", log_rank("info")));
        let o = parse(&["--vf", "d3d11vpp", "--log", "debug"]).unwrap();
        assert_eq!(o.requested_log(), "debug");
        let o = parse(&["--log", "warn"]).unwrap();
        assert_eq!(o.requested_log(), "warn");
        assert!(log_rank("warn") < log_rank("info") && log_rank("info") < log_rank("v"));
    }

    #[test]
    fn filter_lines() {
        for prefix in [
            "vf",
            "vd",
            "d3d11vpp",
            "vf/d3d11vpp",
            "ffmpeg/video",
            "user_filter_wrapper",
            "libmpv_render/d3d11va",
        ] {
            assert!(is_filter_line(prefix, "x"), "{prefix}");
        }
        assert!(is_filter_line("cplayer", "NVIDIA RTX Super Resolution enabled.\n"));
        assert!(is_filter_line("x", "Failed to enable NVIDIA RTX Super Resolution: E_FAIL\n"));
        assert!(is_filter_line("x", "NVIDIA RTX Video HDR not supported.\n"));
        for prefix in ["vo/libmpv", "ffmpeg/audio", "vdpau", "cplayer"] {
            assert!(!is_filter_line(prefix, "x"), "{prefix}");
        }
    }
}
