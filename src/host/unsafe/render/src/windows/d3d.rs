//! The Direct3D side (doc/process/rounds/2026-10-windows-wgpu.md):
//!
//! - the adapter (DXGI), and on it the windows' wgpu device on the
//!   Direct3D 12 backend ([`Gpu`]; Slint's `WGPUConfiguration::Manual` takes
//!   it, so every window draws with this one device);
//! - mpv's Direct3D 11 device on the same adapter, matched by LUID
//!   ([`D3d11`]: video support, multithread protected);
//! - the ring's shared textures, made by Direct3D 12 (a committed resource on
//!   a shared heap with simultaneous access), opened by Direct3D 11 for mpv to
//!   render into, and wrapped for wgpu so Slint shows them ([`shared_slot`]);
//! - two shared fences made by Direct3D 12 and opened by Direct3D 11
//!   ([`SharedFence`]): `ready` (Direct3D 11 signals it after mpv's frame;
//!   the windows' draw waits for it on the GPU) and `done` (the draw signals
//!   it; Direct3D 11 waits for it on the GPU before mpv renders into the same
//!   texture again).
//!
//! Every NT handle is closed as soon as the other API has opened the object
//! ([`Owned`]): the opened object holds its own reference.

use wgpu_hal::api::Dx12;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE, LUID, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_1;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::Graphics::Dxgi::{
    DXGI_GPU_PREFERENCE, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_GPU_PREFERENCE_MINIMUM_POWER,
    IDXGIFactory6,
};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory4};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{Interface, PCWSTR};

pub type Size = (u32, u32);

/// Microsoft's PCI vendor id: the Basic Render Driver (WARP) and the
/// remote-desktop adapters, which have no GPU of their own.
const MICROSOFT: u32 = 0x1414;

fn luid64(luid: LUID) -> u64 {
    (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart)
}

/// Which adapter the windows and the renderer use (`VESTRA_GPU_ADAPTER`).
///
/// The default does what mpv (`--d3d11-adapter` empty: a null adapter, so
/// Direct3D's default one), Chromium's Dawn without a power preference and
/// Firefox do on hybrid laptops: the adapter Windows lists first for this
/// process. That order follows the per-app choice in Settings > System >
/// Display > Graphics, so the user picks the GPU there, as for those
/// programs. None of them switches by power source on its own (wgpu round
/// record §17). The adapter is chosen once per process; a change needs a
/// restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// DXGI's first adapter: the one Windows lists first for this process,
    /// which follows the per-app graphics preference and drives the screen
    /// on hybrid laptops by default (round record 2026-10-windows-render
    /// C.3).
    Default,
    /// DXGI's adapter with this index.
    Index(u32),
    /// The first adapter of `IDXGIFactory6::EnumAdapterByGpuPreference`
    /// with `DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE` (external, then discrete,
    /// then integrated GPUs), whatever the per-app setting says.
    HighPerformance,
    /// The same with `DXGI_GPU_PREFERENCE_MINIMUM_POWER` (integrated first).
    MinimumPower,
    /// The WARP software rasterizer (tests, and a forced fallback).
    Warp,
}
impl Choice {
    /// `None`/empty: `Default`; `warp`; `high-performance`; `minimum-power`;
    /// a number: that index. Anything else: `Default`.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("warp") => Self::Warp,
            Some("high-performance") => Self::HighPerformance,
            Some("minimum-power") => Self::MinimumPower,
            Some(index) => index.parse().map_or(Self::Default, Self::Index),
            None => Self::Default,
        }
    }
    /// How the adapter was chosen (the `render:` line).
    fn how(self) -> String {
        match self {
            Self::Default => {
                "Windows' order for this process (Settings > Display > Graphics)".into()
            }
            Self::Index(index) => format!("VESTRA_GPU_ADAPTER={index}"),
            Self::HighPerformance => "VESTRA_GPU_ADAPTER=high-performance".into(),
            Self::MinimumPower => "VESTRA_GPU_ADAPTER=minimum-power".into(),
            Self::Warp => "VESTRA_GPU_ADAPTER=warp".into(),
        }
    }
}

/// What the adapter is.
#[derive(Clone, Debug)]
pub struct Facts {
    pub name: String,
    pub luid: u64,
    /// No GPU of its own (WARP, Basic Render, a remote-desktop adapter).
    pub software: bool,
    /// How it was chosen, and the other GPUs present (the `render:` line).
    pub how: String,
}

/// An adapter's name, LUID and whether it has no GPU of its own.
fn adapter_facts(adapter: &IDXGIAdapter1) -> Result<(String, u64, bool), String> {
    // SAFETY: a live adapter; GetDesc1 fills a plain struct.
    let desc = unsafe { adapter.GetDesc1() }.map_err(|e| e.to_string())?;
    let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
    let software =
        desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 || desc.VendorId == MICROSOFT;
    let name = String::from_utf16_lossy(&desc.Description[..end]);
    Ok((name, luid64(desc.AdapterLuid), software))
}

fn dxgi_adapter(choice: Choice) -> Result<(IDXGIAdapter1, Facts), String> {
    // SAFETY: plain factory creation; an owned interface comes back.
    let factory: IDXGIFactory4 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let by_preference = |preference: DXGI_GPU_PREFERENCE| -> windows::core::Result<IDXGIAdapter1> {
        // IDXGIFactory6: Windows 10 1803 and later.
        let factory: IDXGIFactory6 = factory.cast()?;
        // SAFETY: a live factory; an owned adapter or an error comes back.
        unsafe { factory.EnumAdapterByGpuPreference(0, preference) }
    };
    let adapter: IDXGIAdapter1 = match choice {
        // SAFETY: a live factory; each call returns an owned adapter or an
        // error (DXGI_ERROR_NOT_FOUND past the last one).
        Choice::Default => unsafe { factory.EnumAdapters1(0) },
        // SAFETY: as above.
        Choice::Index(index) => unsafe { factory.EnumAdapters1(index) },
        Choice::HighPerformance => by_preference(DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE),
        Choice::MinimumPower => by_preference(DXGI_GPU_PREFERENCE_MINIMUM_POWER),
        // SAFETY: as above.
        Choice::Warp => unsafe { factory.EnumWarpAdapter() },
    }
    .map_err(|e| format!("no DXGI adapter {choice:?}: {e}"))?;
    let (name, luid, software) = adapter_facts(&adapter)?;
    // The other GPUs, so the log says what another choice would give.
    let mut others = Vec::new();
    for index in 0..16 {
        // SAFETY: as above; past the last adapter an error ends the list.
        let Ok(other) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        if let Ok((other_name, other_luid, false)) = adapter_facts(&other)
            && other_luid != luid
        {
            others.push(format!("{index} {other_name}"));
        }
    }
    let how = match others.as_slice() {
        [] => choice.how(),
        others => format!("{}; other GPUs: {}", choice.how(), others.join(", ")),
    };
    Ok((adapter, Facts { name, luid, software, how }))
}

/// The windows' wgpu device on the Direct3D 12 backend, on one adapter.
///
/// The fields drop in declaration order, which this type owns for every
/// user: the raw Direct3D references first, then wgpu's queue and device,
/// the instance last (dropping the instance before its device crashed the
/// spike, round record 2026-10-wgpu-d3d-spike §4). The host keeps its one
/// `Gpu` for the whole process (see `mod.rs`).
pub struct Gpu {
    d3d12: ID3D12Device,
    /// The command queue wgpu submits this device's work on.
    raw_queue: ID3D12CommandQueue,
    dxgi: IDXGIAdapter1,
    pub queue: wgpu::Queue,
    pub device: wgpu::Device,
    pub adapter: wgpu::Adapter,
    pub instance: wgpu::Instance,
    pub facts: Facts,
    /// wgpu's Direct3D 12 swapchain options (frame-latency object, presentation system).
    pub swapchain: String,
}

impl Gpu {
    pub fn open(choice: Choice) -> Result<Self, String> {
        let (dxgi, facts) = dxgi_adapter(choice)?;
        // wgpu's own environment variables apply (as for Slint's own
        // instances): `WGPU_DX12_USE_FRAME_LATENCY_WAITABLE_OBJECT`
        // (`Wait`, the default: the swapchain acquire waits on DXGI's
        // frame-latency object; `None`; `DontWait`) and
        // `WGPU_DX12_PRESENTATION_SYSTEM` among them. Direct3D 12 only.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        descriptor.backends = wgpu::Backends::DX12;
        let options = format!(
            "{:?}, {:?}",
            descriptor.backend_options.dx12.latency_waitable_object,
            descriptor.backend_options.dx12.presentation_system
        );
        let instance = wgpu::Instance::new(descriptor);
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
        let adapter = adapters
            .into_iter()
            .find(|adapter| {
                // SAFETY: the instance has only the Direct3D 12 backend; the
                // hal adapter is only read here, while `adapter` lives.
                let hal = unsafe { adapter.as_hal::<Dx12>() };
                // SAFETY: a live DXGI adapter; GetDesc1 fills a plain struct.
                hal.and_then(|hal| unsafe { hal.as_raw().GetDesc1() }.ok())
                    .is_some_and(|desc| luid64(desc.AdapterLuid) == facts.luid)
            })
            .ok_or_else(|| format!("no Direct3D 12 adapter for {}", facts.name))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vestra"),
            required_features: wgpu::Features::empty(),
            // Windows and video textures up to the adapter's own limit (as
            // Slint asks for its own devices).
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| format!("Direct3D 12 device on {}: {e}", facts.name))?;
        // SAFETY: a Direct3D 12 device (the only backend); the raw device is
        // cloned (AddRef) and used only while `Gpu` keeps wgpu's device.
        let d3d12 = unsafe { device.as_hal::<Dx12>() }
            .map(|hal| hal.raw_device().clone())
            .ok_or("not a Direct3D 12 device")?;
        // SAFETY: as above, for the queue (the one wgpu-hal submits on).
        let raw_queue = unsafe { queue.as_hal::<Dx12>() }
            .map(|hal| hal.as_raw().clone())
            .ok_or("not a Direct3D 12 queue")?;
        Ok(Self {
            d3d12,
            raw_queue,
            dxgi,
            queue,
            device,
            adapter,
            instance,
            facts,
            swapchain: options,
        })
    }

    /// Before a window's draw: the windows' queue waits on the GPU until
    /// `fence` reaches `value` (mpv's frame finished).
    ///
    /// Invariant (why the raw queue is right here): wgpu-hal's Direct3D 12
    /// backend executes every submit of this wgpu queue on this one
    /// `ID3D12CommandQueue`, in order, and femtovg-wgpu submits a window's
    /// draw synchronously between Slint's `BeforeRendering` and
    /// `AfterRendering`. A `Wait` enqueued in `BeforeRendering` therefore
    /// gates the whole draw (and anything later), and a `Signal` enqueued in
    /// `AfterRendering` ([`Gpu::signal`]) follows all of it, however many
    /// submits the draw made. Nothing depends on which submit comes next.
    pub fn wait(&self, fence: &SharedFence, value: u64) -> bool {
        // SAFETY: a live queue of this device; the fence was made on the same
        // device (`SharedFence::new`) and outlives every operation enqueued
        // with it here (the windows' views keep the renderer's `Shared`, which
        // owns it, alive). Wait only enqueues a GPU-side wait.
        unsafe { self.raw_queue.Wait(&fence.d3d12, value) }.is_ok()
    }

    /// A test switch (`VESTRA_TEST_REMOVE_DEVICE`): remove the device as a
    /// driver reset would (`ID3D12Device5::RemoveDevice`). `false` without
    /// Direct3D 12.5.
    pub fn remove_for_test(&self) -> bool {
        let Ok(device) = self.d3d12.cast::<ID3D12Device5>() else {
            return false;
        };
        // SAFETY: a live device; removing it is what the switch is for, and
        // every later use of it fails with DXGI_ERROR_DEVICE_REMOVED.
        unsafe { device.RemoveDevice() };
        true
    }

    /// After a window's draw: `fence` reaches `value` once the GPU finished
    /// everything submitted so far on the windows' queue (see [`Gpu::wait`]).
    pub fn signal(&self, fence: &SharedFence, value: u64) -> bool {
        // SAFETY: as for `wait`; Signal only enqueues a GPU-side signal.
        unsafe { self.raw_queue.Signal(&fence.d3d12, value) }.is_ok()
    }
}

/// A fence only the windows' queue signals: when the windows' draws finish
/// on the GPU (diagnostics, `ui_draw_submitted`).
pub struct QueueFence(ID3D12Fence);

impl Gpu {
    pub fn queue_fence(&self) -> Result<QueueFence, String> {
        // SAFETY: a live device; valid flags; an owned fence comes back.
        unsafe { self.d3d12.CreateFence(0, D3D12_FENCE_FLAG_NONE) }
            .map(QueueFence)
            .map_err(|e| format!("CreateFence: {e}"))
    }
    /// Stage `fence` reaching `value` after wgpu's next submit on the
    /// windows' queue (the next window draw), before anything enqueued after
    /// that submit (its present). `false` without a Direct3D 12 queue.
    pub fn stage_queue_fence(&self, fence: &QueueFence, value: u64) -> bool {
        // SAFETY: the queue is this device's Direct3D 12 queue and the fence
        // this device's (`queue_fence`); staging only appends to the queue's
        // list for its next submit.
        let Some(queue) = (unsafe { self.queue.as_hal::<Dx12>() }) else {
            return false;
        };
        queue.add_signal_fence(fence.0.clone(), value);
        true
    }
    /// Take back a staged signal of `fence` that no submit used yet; `true`
    /// if there was one (then no draw was submitted since it was staged).
    pub fn unstage_queue_fence(&self, fence: &QueueFence) -> bool {
        // SAFETY: as for `stage_queue_fence`; only edits the pending list.
        unsafe { self.queue.as_hal::<Dx12>() }
            .is_some_and(|queue| queue.remove_signal_fence(&fence.0))
    }
    /// `fence` reaches `value` once the GPU finished everything submitted so
    /// far on the windows' queue.
    pub fn signal_queue_fence(&self, fence: &QueueFence, value: u64) -> bool {
        // SAFETY: a live queue and a fence of this device, kept alive by the
        // caller for the process (a static); Signal only enqueues.
        unsafe { self.raw_queue.Signal(&fence.0, value) }.is_ok()
    }
}

// ------------------------------------------------------- mpv's D3D11 device

/// mpv's Direct3D 11 device: on the [`Gpu`]'s adapter, multithread protected
/// (mpv's decoder uses it from its own thread), with video support when the
/// adapter has it (d3d11va decodes on it, so frames never leave the GPU).
pub struct D3d11 {
    pub device: ID3D11Device,
    device1: ID3D11Device1,
    device5: ID3D11Device5,
    context: ID3D11DeviceContext,
    context4: ID3D11DeviceContext4,
    pub video: bool,
    pub priority: i32,
}

impl D3d11 {
    pub fn new(gpu: &Gpu, priority: Option<i32>) -> Result<Self, String> {
        let adapter: IDXGIAdapter = gpu.dxgi.cast().map_err(|e| e.to_string())?;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut last = String::new();
        for video in [true, false] {
            let mut flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            if video {
                flags |= D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
            }
            let mut device = None;
            // SAFETY: the adapter lives for the call, the feature levels are
            // a valid slice, and the out-pointer is a live local.
            let made = unsafe {
                D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    flags,
                    Some(&levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    None,
                )
            };
            let device: ID3D11Device = match (made, device) {
                (Ok(()), Some(device)) => device,
                (made, _) => {
                    last = format!("{made:?}");
                    continue;
                }
            };
            let multithread: ID3D11Multithread = device.cast().map_err(|e| e.to_string())?;
            // SAFETY: a live device.
            let _ = unsafe { multithread.SetMultithreadProtected(true) };
            let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
            if let Some(priority) = priority {
                // SAFETY: a live device; the value is in DXGI's range (-7..=7).
                let _ = unsafe { dxgi.SetGPUThreadPriority(priority) };
            }
            // SAFETY: a live device.
            let priority = unsafe { dxgi.GetGPUThreadPriority() }.unwrap_or(0);
            // SAFETY: a live device; an owned reference comes back.
            let context = unsafe { device.GetImmediateContext() }.map_err(|e| e.to_string())?;
            let need = |what: &str, e: windows::core::Error| format!("{what} (Direct3D 11.4): {e}");
            return Ok(Self {
                device1: device.cast().map_err(|e| need("ID3D11Device1", e))?,
                device5: device.cast().map_err(|e| need("ID3D11Device5", e))?,
                context4: context.cast().map_err(|e| need("ID3D11DeviceContext4", e))?,
                device,
                context,
                video,
                priority,
            });
        }
        Err(format!("D3D11CreateDevice: {last}"))
    }

    /// Before mpv renders into a texture again: its GPU waits until `fence`
    /// reaches `value` (the last window draw that showed the texture).
    pub fn gpu_wait(&self, fence: &SharedFence, value: u64) -> bool {
        // SAFETY: a live context and a fence of this device. Only the render
        // thread uses the immediate context (mpv's decoder uses the video
        // context, under the multithread protection).
        unsafe { self.context4.Wait(&fence.d3d11, value) }.is_ok()
    }

    /// Why the device stopped working (`GetDeviceRemovedReason`), if it did.
    pub fn removed(&self) -> Option<String> {
        // SAFETY: a live device; the call only reads its state.
        let reason = unsafe { self.device.GetDeviceRemovedReason() };
        reason.err().map(|error| error.to_string())
    }

    /// After mpv's frame: `fence` reaches `value` when the GPU finished it;
    /// the commands are submitted now.
    #[cfg(test)]
    pub fn signal(&self, fence: &SharedFence, value: u64) -> bool {
        self.try_signal(fence, value).is_ok()
    }

    /// As [`signal`](Self::signal), with the error.
    pub fn try_signal(&self, fence: &SharedFence, value: u64) -> Result<(), String> {
        // SAFETY: as for `gpu_wait`; Flush only submits.
        unsafe {
            let signalled = self.context4.Signal(&fence.d3d11, value);
            self.context.Flush();
            signalled.map_err(|error| error.to_string())
        }
    }
}

// ------------------------------------------------------------ the ring

/// Closes an NT handle when dropped (once the other API opened the object).
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this module created, closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// One shared texture: what mpv renders into, and what Slint shows.
pub struct Slot {
    pub d3d11: ID3D11Texture2D,
    pub texture: wgpu::Texture,
}

/// What wgpu knows about a ring texture. Slint's import needs
/// `TEXTURE_BINDING` and `RENDER_ATTACHMENT` and `Rgba8Unorm`; `COPY_SRC`
/// lets a window snapshot or a test read it back.
fn texture_desc((width, height): Size) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("vestra video frame"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }
}

/// A texture made by Direct3D 12 on a shared heap (render target,
/// simultaneous access: both APIs may use it without ownership transfers),
/// opened by Direct3D 11 and wrapped for wgpu.
pub fn shared_slot(gpu: &Gpu, d3d11: &D3d11, size: Size) -> Result<Slot, String> {
    let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: u64::from(size.0),
        Height: size.1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
            | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
    };
    let mut resource: Option<ID3D12Resource> = None;
    // SAFETY: valid descriptors and a live out-pointer.
    unsafe {
        gpu.d3d12.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_SHARED,
            &desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut resource,
        )
    }
    .map_err(|e| format!("CreateCommittedResource (shared): {e}"))?;
    let resource = resource.ok_or("no shared resource")?;
    // SAFETY: a live shared resource of this device; `Owned` closes the
    // handle after Direct3D 11 opened it.
    let handle = Owned(
        unsafe { gpu.d3d12.CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null()) }
            .map_err(|e| format!("CreateSharedHandle (texture): {e}"))?,
    );
    // SAFETY: an NT handle to a resource of the same adapter (D3d11 is on
    // the Gpu's adapter); the opened texture holds its own reference.
    let d3d11_texture: ID3D11Texture2D = unsafe { d3d11.device1.OpenSharedResource1(handle.0) }
        .map_err(|e| format!("OpenSharedResource1: {e}"))?;
    drop(handle);
    let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
    // SAFETY: the resource was created on this wgpu device's ID3D12Device
    // with exactly the format, size, one mip, one sample and usages stated
    // in `texture_desc` (render target allowed; shader reads need no flag),
    // in state COMMON. Its contents are written by mpv before any window
    // samples it (the `ready` fence), and with simultaneous access the
    // states wgpu tracks for it do not need to match Direct3D 11's use.
    let texture = unsafe {
        let hal = wgpu_hal::dx12::Device::texture_from_raw(
            resource,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureDimension::D2,
            extent,
            1,
            1,
        );
        gpu.device.create_texture_from_hal::<Dx12>(
            hal,
            &texture_desc(size),
            wgpu::TextureUses::UNINITIALIZED,
        )
    };
    Ok(Slot { d3d11: d3d11_texture, texture })
}

/// A fence made by Direct3D 12 and opened by Direct3D 11 (one object).
pub struct SharedFence {
    pub d3d12: ID3D12Fence,
    pub d3d11: ID3D11Fence,
}

impl SharedFence {
    pub fn new(gpu: &Gpu, d3d11: &D3d11) -> Result<Self, String> {
        // SAFETY: a live device; valid flags.
        let fence: ID3D12Fence = unsafe { gpu.d3d12.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }
            .map_err(|e| format!("CreateFence (shared): {e}"))?;
        // SAFETY: a live shared fence of this device; `Owned` closes the
        // handle after Direct3D 11 opened it.
        let handle = Owned(
            unsafe { gpu.d3d12.CreateSharedHandle(&fence, None, GENERIC_ALL.0, PCWSTR::null()) }
                .map_err(|e| format!("CreateSharedHandle (fence): {e}"))?,
        );
        let mut opened: Option<ID3D11Fence> = None;
        // SAFETY: an NT handle to a fence of the same adapter; a live
        // out-pointer; the opened fence holds its own reference.
        unsafe { d3d11.device5.OpenSharedFence(handle.0, &mut opened) }
            .map_err(|e| format!("OpenSharedFence: {e}"))?;
        Ok(Self { d3d12: fence, d3d11: opened.ok_or("no shared fence")? })
    }
    /// The value the GPU has reached.
    pub fn completed(&self) -> u64 {
        // SAFETY: a live fence.
        unsafe { self.d3d12.GetCompletedValue() }
    }
    /// Set the fence from the CPU: `u64::MAX` releases every GPU wait on it
    /// (the renderer gives up, so neither queue waits forever).
    pub fn release_all(&self) {
        // SAFETY: a live fence; a CPU-side signal.
        let _ = unsafe { self.d3d12.Signal(u64::MAX) };
    }
}

/// Waits on the CPU (an event, not a spin) until a fence reaches a value.
/// Render thread only.
pub struct Waiter(HANDLE);
impl Waiter {
    pub fn new() -> Result<Self, String> {
        // SAFETY: an unnamed auto-reset event, closed in Drop.
        unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
            .map(Self)
            .map_err(|e| format!("CreateEventW: {e}"))
    }
    /// `false` on a timeout or an error. A wake-up from an earlier wait
    /// that timed out (its completion set the auto-reset event later) is
    /// told apart by checking the fence again.
    pub fn wait(&self, fence: &SharedFence, value: u64, timeout_ms: u32) -> bool {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.into());
        loop {
            if fence.completed() >= value {
                return true;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            // SAFETY: a live fence and this live event; the event is set once
            // the fence reaches the value (at once if it already has).
            let woke = unsafe {
                fence.d3d11.SetEventOnCompletion(value, self.0).is_ok()
                    && WaitForSingleObject(self.0, left.as_millis().max(1) as u32) == WAIT_OBJECT_0
            };
            if !woke {
                return fence.completed() >= value;
            }
        }
    }
}
impl Waiter {
    /// As `wait`, for a [`QueueFence`].
    pub fn wait_queue(&self, fence: &QueueFence, value: u64, timeout_ms: u32) -> bool {
        // SAFETY: a live fence and this live event (as in `wait`).
        let woke = unsafe {
            fence.0.SetEventOnCompletion(value, self.0).is_ok()
                && WaitForSingleObject(self.0, timeout_ms) == WAIT_OBJECT_0
        };
        // SAFETY: a live fence.
        woke && unsafe { fence.0.GetCompletedValue() } >= value
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        // SAFETY: created in `new`, closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// ------------------------------------------------------------- readbacks

/// Test readbacks of a ring texture through both APIs.
#[cfg(test)]
pub mod readback {
    use super::*;

    /// The whole texture through Direct3D 11 (tightly packed RGBA rows).
    pub fn d3d11(d3d11: &D3d11, texture: &ID3D11Texture2D, (w, h): Size) -> Vec<u8> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        // SAFETY: a valid descriptor and a live out-pointer.
        unsafe { d3d11.device.CreateTexture2D(&desc, None, Some(&mut staging)) }.unwrap();
        let staging: ID3D11Texture2D = staging.unwrap();
        let mut out = vec![0u8; (w * h * 4) as usize];
        // SAFETY: both textures are this device's, of the same size and
        // format; Map gives RowPitch * h readable bytes until Unmap, and each
        // row copied stays inside its w * 4 bytes.
        unsafe {
            d3d11.context.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            d3d11.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
            for row in 0..h as usize {
                let source = mapped.pData.cast::<u8>().add(row * mapped.RowPitch as usize);
                let target = &mut out[row * w as usize * 4..(row + 1) * w as usize * 4];
                std::ptr::copy_nonoverlapping(source, target.as_mut_ptr(), target.len());
            }
            d3d11.context.Unmap(&staging, 0);
        }
        out
    }

    /// The whole texture through wgpu (tightly packed RGBA rows).
    pub fn wgpu(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
        let size = texture.size();
        let (w, h) = (size.width, size.height);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            size,
        );
        gpu.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = slice.get_mapped_range().unwrap();
        let mut out = vec![0u8; (w * h * 4) as usize];
        for row in 0..h as usize {
            let source = &data[row * padded as usize..row * padded as usize + w as usize * 4];
            out[row * w as usize * 4..(row + 1) * w as usize * 4].copy_from_slice(source);
        }
        drop(data);
        buffer.unmap();
        out
    }
}
