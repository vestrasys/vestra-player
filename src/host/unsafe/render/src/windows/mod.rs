//! Windows: libmpv draws with Direct3D 11 and Slint shows the frames through
//! wgpu on Direct3D 12, without a copy (round record
//! doc/process/rounds/2026-10-windows-wgpu.md; the spike that proved it:
//! 2026-10-wgpu-d3d-spike.md).
//!
//! - Every Slint window draws with one wgpu device on the Direct3D 12
//!   backend ([`WindowDevice`], handed to Slint as
//!   `WGPUConfiguration::Manual`), made once per process and never dropped.
//! - libmpv renders on its own thread through its Direct3D 11 render API, on
//!   a device of the same adapter (matched by LUID), into a ring of three
//!   textures that Direct3D 12 made shareable. d3d11va decodes on that
//!   device, so frames never leave the GPU.
//! - A window shows the newest texture as a `slint::Image` made from its
//!   `wgpu::Texture`. Two shared fences order the two APIs on the GPU:
//!   each draw waits for `ready` (mpv's frame finished) and signals `done`;
//!   mpv's device waits for the last `done` of a texture before rendering
//!   into it again. The UI thread never waits on the CPU, and by default
//!   neither does the render thread (`VESTRA_GPU_CPU_WAIT=1`: it waits for
//!   each frame to finish before publishing it, as up to host 0.5.18).
//! - As on macOS the mpv render context lives as long as the `Renderer`, so
//!   a window rebuild (sidebar detach) never ends playback.
//! - A watchdog ([`watchdog`]) treats fences that stop moving like a lost
//!   device (`VESTRA_GPU_WATCHDOG`).

mod d3d;
mod mpv;
mod ring;
#[cfg(test)]
mod tests;
mod watchdog;

pub use d3d::Choice;
use d3d::{D3d11, Gpu, SharedFence, Waiter};
use ring::{RING, Ring, Slot};
use rsmpv::Mpv;
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::{thread, time::Duration, time::Instant};

type Size = (u32, u32);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ------------------------------------------------------ the windows' device

/// The process's one window device, set once the windows really draw with
/// it ([`WindowDevice::install`]). Never dropped then: Slint's windows, the
/// render thread and the textures they share all refer to it, and wgpu's
/// instance must outlive its device (`Gpu`).
static GPU: OnceLock<Arc<Gpu>> = OnceLock::new();

/// The window device was lost (a driver reset, a removed GPU), or GPU
/// rendering stalled (the watchdog), which the host handles the same way.
static LOST: AtomicBool = AtomicBool::new(false);

/// The renderers started in this process ([`release_gpu_waits`]).
static LIVE: Mutex<Vec<std::sync::Weak<Shared>>> = Mutex::new(Vec::new());

/// The player quits (any thread; call it before the windows and their
/// device go): every live renderer releases its shared fences and stops,
/// so no GPU wait on them (the windows' queue waiting for a frame mpv's GPU
/// never finishes, a stall with `VESTRA_GPU_WATCHDOG=0` or before the
/// watchdog's limit) can hold up the presents, the swapchains' teardown or
/// the device's drain. Round record 2026-10-render-modes §11.6.
pub fn release_gpu_waits() {
    let live: Vec<Arc<Shared>> = lock(&LIVE).iter().filter_map(std::sync::Weak::upgrade).collect();
    for shared in live {
        shared.release_for_quit();
    }
}

/// What [`WindowDevice::on_error`] runs when the device is lost; the
/// watchdog runs it too when GPU rendering stalls.
static ON_LOST: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

/// A test switch (`VESTRA_TEST_REMOVE_DEVICE`): remove the installed window
/// device as a driver reset would; `false` without one or without
/// Direct3D 12.5.
pub fn remove_window_device_for_test() -> bool {
    GPU.get().is_some_and(|gpu| gpu.remove_for_test())
}

/// The installed window device's wgpu device and queue (the software
/// rendering texture of windows without a notifier).
pub fn window_wgpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    GPU.get().map(|gpu| (gpu.device.clone(), gpu.queue.clone()))
}

/// The installed window device's adapter: its LUID and name. GPU rendering
/// (mpv's Direct3D 11 device) runs on the same adapter.
pub fn window_adapter() -> Option<(u64, String)> {
    GPU.get().map(|gpu| (gpu.facts.luid, gpu.facts.name.clone()))
}

/// Whether the window device was lost (or GPU rendering stalled, see
/// [`watchdog`]): the windows can no longer draw, and the host restarts the
/// player (see `platform/windows.rs`).
pub fn device_lost() -> bool {
    LOST.load(Ordering::Acquire)
}

/// The wgpu device (Direct3D 12) every Slint window draws with, and that the
/// renderer shares its textures with.
pub struct WindowDevice(Arc<Gpu>);

impl WindowDevice {
    /// Open a device (the installed one, if any). `adapter`
    /// (`VESTRA_GPU_ADAPTER`): unset for DXGI's first adapter, an index,
    /// `high-performance`, `minimum-power` or
    /// `warp`. Dropped unless [`install`](Self::install)ed (an OpenGL
    /// fallback keeps no Direct3D 12 device).
    pub fn open(adapter: Option<&str>) -> Result<Self, String> {
        if let Some(gpu) = GPU.get() {
            return Ok(Self(gpu.clone()));
        }
        Ok(Self(Arc::new(Gpu::open(Choice::parse(adapter))?)))
    }
    /// The windows draw with this device: keep it for the process, for
    /// [`Renderer::start`].
    pub fn install(&self) {
        let _ = GPU.set(self.0.clone());
    }
    /// The adapter has no GPU of its own (WARP, a remote-desktop adapter).
    pub fn software(&self) -> bool {
        self.0.facts.software
    }
    /// The adapter, for the `render:` diagnostics.
    pub fn describe(&self) -> String {
        let facts = &self.0.facts;
        format!(
            "{} (LUID {:016x}; chosen by {}; swapchain {})",
            facts.name, facts.luid, facts.how, self.0.swapchain
        )
    }
    /// Report the device's errors to `sink` instead of wgpu's default, which
    /// panics on an error nobody captured (a window draw that wgpu refuses
    /// must not end the player): each distinct message a few times. A lost
    /// device (a driver reset) sets [`device_lost`], is reported, and calls
    /// `lost`: the windows cannot draw any more, so the host ends the event
    /// loop and restarts the player. The renderer's watchdog calls `lost`
    /// too when GPU rendering stalls.
    pub fn on_error(
        &self,
        sink: impl Fn(String) + Send + Sync + 'static,
        lost: impl Fn() + Send + Sync + 'static,
    ) {
        let lost: Arc<dyn Fn() + Send + Sync> = Arc::new(lost);
        let _ = ON_LOST.set(lost.clone());
        let sink = Arc::new(sink);
        let seen = Mutex::new(Vec::<String>::new());
        let errors = sink.clone();
        self.0.device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            let text: String = error.to_string().chars().take(300).collect();
            let mut seen = lock(&seen);
            if seen.len() < 16 && !seen.contains(&text) {
                errors(format!("Direct3D 12 (wgpu) error: {text}"));
                seen.push(text);
            }
        }));
        self.0.device.set_device_lost_callback(move |reason, message| {
            // `Destroyed`: the device was dropped on purpose (an unused one).
            if reason == wgpu::DeviceLostReason::Destroyed {
                return;
            }
            LOST.store(true, Ordering::Release);
            sink(format!("the Direct3D 12 device was lost ({reason:?}: {message})"));
            lost();
        });
    }
    /// For `slint::BackendSelector::require_wgpu_30`.
    pub fn configuration(&self) -> slint::wgpu_30::WGPUConfiguration {
        slint::wgpu_30::WGPUConfiguration::Manual {
            instance: self.0.instance.clone(),
            adapter: self.0.adapter.clone(),
            device: self.0.device.clone(),
            queue: self.0.queue.clone(),
        }
    }
}

/// The main window's draws on the GPU (diagnostics). Three marks on the
/// windows' queue per draw, and a thread that notes when the GPU reaches
/// each:
///
/// - start: signalled when the draw starts (`ui_draw_started`, before
///   Slint's acquire and draw);
/// - drawn: staged for wgpu's next submit, which is the draw's own (no
///   rendering notifier: Slint submits a window draw once), so it is reached
///   when the draw's commands are done, before the present;
/// - presented: signalled after the present (`ui_draw_submitted`).
///
/// Per draw: `wait` (start's submission until the GPU reached it: work
/// queued ahead, the engine busy elsewhere), `draw` (start to drawn) and
/// `present` (drawn to presented).
struct UiGpu {
    fence: d3d::QueueFence,
    next: AtomicU64,
    marks: Mutex<mpsc::Sender<(u64, Instant, Mark)>>,
    stats: Mutex<UiGpuStats>,
    /// The open draw's drawn and presented values (UI thread).
    open: Mutex<Option<(u64, Instant)>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Start,
    Drawn,
    Presented,
}

/// (count, max, total) in ms of each part since `ui_gpu_text`.
#[derive(Default)]
struct UiGpuStats {
    wait: (u64, f64, f64),
    draw: (u64, f64, f64),
    present: (u64, f64, f64),
}

fn ui_gpu() -> Option<&'static Arc<UiGpu>> {
    static UI_GPU: OnceLock<Option<Arc<UiGpu>>> = OnceLock::new();
    UI_GPU
        .get_or_init(|| {
            let gpu = GPU.get()?;
            let fence = gpu.queue_fence().ok()?;
            let (marks, received) = mpsc::channel::<(u64, Instant, Mark)>();
            let tracker = Arc::new(UiGpu {
                fence,
                next: AtomicU64::new(0),
                marks: Mutex::new(marks),
                stats: Mutex::default(),
                open: Mutex::default(),
            });
            let inner = tracker.clone();
            let spawned = thread::Builder::new().name("vestra-ui-gpu".into()).spawn(move || {
                let Ok(waiter) = Waiter::new() else { return };
                let add = |part: &mut (u64, f64, f64), since: Instant, until: Instant| {
                    let ms = until.saturating_duration_since(since).as_secs_f64() * 1e3;
                    *part = (part.0 + 1, part.1.max(ms), part.2 + ms);
                };
                // When the GPU reached the open draw's start and drawn marks.
                let (mut started, mut drawn) = (None::<Instant>, None::<Instant>);
                for (value, submitted, mark) in received {
                    if !waiter.wait_queue(&inner.fence, value, 1000) {
                        (started, drawn) = (None, None);
                        continue;
                    }
                    let reached = Instant::now();
                    let mut stats = lock(&inner.stats);
                    match mark {
                        Mark::Start => {
                            add(&mut stats.wait, submitted, reached);
                            (started, drawn) = (Some(reached), None);
                        }
                        Mark::Drawn => {
                            if let Some(began) = started {
                                add(&mut stats.draw, began, reached);
                            }
                            drawn = Some(reached);
                        }
                        Mark::Presented => {
                            if let Some(at) = drawn.take() {
                                add(&mut stats.present, at, reached);
                            }
                            started = None;
                        }
                    }
                }
            });
            spawned.ok().map(|_| tracker)
        })
        .as_ref()
}

/// The main window starts a draw (diagnostics, ` ui-gpu`): the start mark
/// now, the drawn mark staged for the draw's submit.
pub fn ui_draw_started() {
    let (Some(tracker), Some(gpu)) = (ui_gpu(), GPU.get()) else {
        return;
    };
    let start = tracker.next.fetch_add(3, Ordering::Relaxed) + 1;
    if !gpu.signal_queue_fence(&tracker.fence, start) {
        return;
    }
    let now = Instant::now();
    let _ = lock(&tracker.marks).send((start, now, Mark::Start));
    // A drawn mark left by a dispatch that never closed must not fire late.
    gpu.unstage_queue_fence(&tracker.fence);
    if gpu.stage_queue_fence(&tracker.fence, start + 1) {
        *lock(&tracker.open) = Some((start + 1, now));
    }
}

/// The main window's draw was presented (diagnostics, ` ui-gpu`).
pub fn ui_draw_submitted() {
    let (Some(tracker), Some(gpu)) = (ui_gpu(), GPU.get()) else {
        return;
    };
    let Some((drawn, at)) = lock(&tracker.open).take() else {
        return;
    };
    // Still staged: no draw was submitted (nothing to time).
    if gpu.unstage_queue_fence(&tracker.fence) {
        return;
    }
    let presented = drawn + 1;
    if gpu.signal_queue_fence(&tracker.fence, presented) {
        let marks = lock(&tracker.marks);
        let _ = marks.send((drawn, at, Mark::Drawn));
        let _ = marks.send((presented, at, Mark::Presented));
    }
}

/// `VESTRA_FRAME_LOG`: the main window's draws on the GPU since the last
/// call (` ui-gpu wait … draw … present …`), or "".
pub fn ui_gpu_text() -> String {
    let Some(tracker) = GPU.get().and(ui_gpu()) else {
        return String::new();
    };
    let stats = std::mem::take(&mut *lock(&tracker.stats));
    let part = |(count, max, total): (u64, f64, f64)| {
        if count == 0 {
            "-".to_owned()
        } else {
            format!("max {max:.1} mean {:.1} ms", total / count as f64)
        }
    };
    format!(
        " ui-gpu wait {} draw {} present {}",
        part(stats.wait),
        part(stats.draw),
        part(stats.present)
    )
}

// ------------------------------------------------------------ the renderer

/// What a published frame carries.
#[derive(Clone)]
struct Payload {
    texture: wgpu::Texture,
    /// The `ready` value that marks it finished.
    ready: u64,
    size: Size,
}

/// The ring and, per texture of its current generation, the `done` value
/// the last window draw that showed it signals.
#[derive(Default)]
struct Exchange {
    ring: Ring<Payload>,
    done: [u64; RING],
}

struct Fences {
    ready: SharedFence,
    done: SharedFence,
}

type Notify = Box<dyn Fn() + Send>;
/// A redraw request for attached windows.
struct Redraw {
    /// The one entry for all windows shown without a notifier.
    direct: bool,
    call: Notify,
}
type Sink = Box<dyn Fn(String) + Send + Sync>;

/// The render thread's numbers since the frame log last read them.
#[derive(Default)]
struct Stats {
    renders: u64,
    render_max: f64,
    render_total: f64,
    /// Frames whose completion on the GPU was timed (from the `ready`
    /// signal until the GPU reached it): on the render thread with the CPU
    /// wait, else on the completion thread ([`Completion`]).
    completes: u64,
    complete_max: f64,
    complete_total: f64,
    /// mpv's GPU waited for a window's draw before reusing a texture.
    done_waits: u64,
    /// No free texture for 100 ms.
    stuck: u64,
    /// Without the CPU wait: the render thread waited for an earlier frame
    /// before rendering the next one (more than [`IN_FLIGHT`] in flight).
    throttled: u64,
}

struct Shared {
    stop: AtomicBool,
    size: Mutex<Size>,
    /// `Some(period)`: render at most once per period and report each frame
    /// as presented (mpv's display-synced timing, for interpolation).
    pace: Mutex<Option<Duration>>,
    exchange: Mutex<Exchange>,
    /// Set by the render thread before `Renderer::start` returns.
    fences: OnceLock<Fences>,
    /// Set by mpv's update callback, a resize and stop; cleared by the
    /// render thread.
    pending: (Mutex<bool>, Condvar),
    /// Ask the last few attached windows to redraw (or, without a
    /// notifier, to update their image).
    redraw: Mutex<Vec<Redraw>>,
    /// Called after each new frame ([`Handle::on_frame`]).
    notify: Mutex<Vec<Notify>>,
    /// (epoch, hold): after [`Handle::reshow`], windows show no frame up to
    /// `hold`, then set the next frame's image again.
    reshow: Mutex<(u64, u64)>,
    /// Frames shown in a window, each once however many windows show it
    /// (`count_shown`), and the newest frame counted.
    shown: AtomicU64,
    shown_sequence: AtomicU64,
    /// The newest window attached without a notifier (`attach_direct`).
    direct_generation: AtomicU64,
    /// Bumped by [`Handle::retarget`]: windows hand their current image to
    /// `show` again (with a notifier at their next draw; without one at
    /// once).
    retargets: AtomicU64,

    /// The last `done` value handed to a window draw (UI thread order).
    signalled: AtomicU64,
    /// The last `ready` value mpv's device enqueued, and the last `done`
    /// value the windows' queue enqueued (the watchdog compares them with
    /// what the GPU reached).
    ready_enqueued: AtomicU64,
    done_enqueued: AtomicU64,
    /// mpv's render calls, odd while the render thread is inside one (the
    /// watchdog).
    calls: AtomicU64,
    /// The render thread waits on the CPU for each frame before publishing
    /// it (`VESTRA_GPU_CPU_WAIT=1`); set by the render thread.
    cpu_wait: AtomicBool,
    /// `render:` lines, kept until the host takes them.
    diag: Mutex<(Vec<String>, Option<Sink>)>,
    stats: Mutex<Stats>,
    /// Why the render thread stopped (`Handle::failed`).
    failed: Mutex<Option<String>>,
    /// The player quits ([`release_gpu_waits`]): the fences are released,
    /// nothing renders, claims or signals any more (not a failure).
    quitting: AtomicBool,
    /// Tests: make the render thread take its failure path.
    #[cfg(test)]
    fail_now: AtomicBool,
    /// Tests: make mpv's device wait on the GPU for a `done` value that is
    /// never signalled (a fence deadlock, for the watchdog).
    #[cfg(test)]
    stall_now: AtomicBool,
    /// Last: the textures and fences above drop before the device.
    gpu: Arc<Gpu>,
}

impl Shared {
    fn new(gpu: Arc<Gpu>, size: Size) -> Self {
        Self {
            stop: AtomicBool::new(false),
            size: Mutex::new(size),
            pace: Mutex::new(None),
            exchange: Mutex::default(),
            fences: OnceLock::new(),
            pending: Default::default(),
            redraw: Mutex::default(),
            notify: Mutex::default(),
            reshow: Mutex::default(),
            shown: AtomicU64::new(0),
            shown_sequence: AtomicU64::new(0),
            direct_generation: AtomicU64::new(0),
            retargets: AtomicU64::new(0),
            signalled: AtomicU64::new(0),
            ready_enqueued: AtomicU64::new(0),
            done_enqueued: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            cpu_wait: AtomicBool::new(false),
            diag: Mutex::default(),
            stats: Mutex::default(),
            failed: Mutex::default(),
            quitting: AtomicBool::new(false),
            #[cfg(test)]
            fail_now: AtomicBool::new(false),
            #[cfg(test)]
            stall_now: AtomicBool::new(false),
            gpu,
        }
    }
    /// GPU rendering stopped for good: it failed, or the player quits.
    fn halted(&self) -> bool {
        self.quitting.load(Ordering::Acquire) || lock(&self.failed).is_some()
    }
    /// The player quits: release both fences (every GPU wait on them, on
    /// mpv's queue and the windows', passes, so presenting, a swapchain's
    /// teardown and the device's drain cannot wait for a frame that never
    /// finishes, as in a stall with the watchdog off) and stop the render
    /// thread; the windows stop claiming frames.
    fn release_for_quit(&self) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stop.store(true, Ordering::Release);
        if let Some(fences) = self.fences.get() {
            fences.ready.release_all();
            fences.done.release_all();
        }
        mpv::Wake::wake(self);
    }
    fn wait(&self, timeout: Duration) {
        let pending = lock(&self.pending.0);
        let wait = self.pending.1.wait_timeout_while(pending, timeout, |pending| !*pending);
        *wait.unwrap_or_else(PoisonError::into_inner).0 = false;
    }
    fn diag(&self, line: String) {
        let mut diag = lock(&self.diag);
        match &diag.1 {
            Some(sink) => sink(line),
            None if diag.0.len() < 32 => diag.0.push(line),
            None => (),
        }
    }
    /// A window shows frame `sequence`: counted once, whichever window and
    /// however often (several attached windows, a rebuilt main window).
    fn count_shown(&self, sequence: u64) {
        if self.shown_sequence.fetch_max(sequence, Ordering::AcqRel) < sequence {
            self.shown.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn ask_redraw(&self) {
        lock(&self.redraw).iter().for_each(|redraw| (redraw.call)());
    }
}

impl mpv::Wake for Shared {
    fn wake(&self) {
        *lock(&self.pending.0) = true;
        self.pending.1.notify_one();
    }
}

/// Owns the render thread and with it mpv's (only) render context, and the
/// watchdog thread.
pub struct Renderer {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    watchdog: Option<thread::JoinHandle<()>>,
}

/// The UI side of a [`Renderer`]: cloneable and `Send`.
#[derive(Clone)]
pub struct Handle(Arc<Shared>);

impl Renderer {
    /// Render `mpv` (`vo=libmpv`, no other render context) at `size` pixels
    /// for the windows of [`WindowDevice`] (open it first). While `publish`
    /// is false frames are drawn but not shown. An error: GPU rendering is
    /// not possible here (no window device, an adapter without a GPU, no
    /// Direct3D 11.4 or no texture sharing); the host renders in software.
    pub fn start(mpv: Arc<Mpv>, size: Size, publish: Arc<AtomicBool>) -> Result<Self, String> {
        let gpu = GPU.get().cloned().ok_or("the windows do not draw with Direct3D 12")?;
        if gpu.facts.software {
            return Err(format!("{} has no GPU of its own", gpu.facts.name));
        }
        Self::start_on(gpu, mpv, size, publish)
    }
    fn start_on(
        gpu: Arc<Gpu>,
        mpv: Arc<Mpv>,
        size: Size,
        publish: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let limit = watchdog::limit(std::env::var("VESTRA_GPU_WATCHDOG").ok().as_deref());
        Self::start_watched(gpu, mpv, size, publish, limit)
    }
    /// As `start_on`, with the watchdog's limit (`None`: no watchdog).
    fn start_watched(
        gpu: Arc<Gpu>,
        mpv: Arc<Mpv>,
        size: Size,
        publish: Arc<AtomicBool>,
        limit: Option<Duration>,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared::new(gpu, clamp(size)));
        {
            let mut live = lock(&LIVE);
            live.retain(|weak| weak.strong_count() > 0);
            live.push(Arc::downgrade(&shared));
        }
        let (ready, started) = mpsc::sync_channel(1);
        let inner = shared.clone();
        let thread = thread::Builder::new().name("vestra-gpu-render".into());
        let thread =
            thread.spawn(move || produce(mpv, inner, publish, ready)).map_err(|e| e.to_string())?;
        let mut renderer = Self { shared, thread: Some(thread), watchdog: None };
        started.recv().map_err(|_| "render thread died".to_string())??;
        if let Some(limit) = limit {
            let inner = renderer.shared.clone();
            let thread = thread::Builder::new().name("vestra-gpu-watchdog".into());
            renderer.watchdog = thread.spawn(move || watch(&inner, limit)).ok();
        }
        Ok(renderer)
    }
    pub fn handle(&self) -> Handle {
        Handle(self.shared.clone())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        mpv::Wake::wake(&*self.shared);
        if let Some(thread) = self.watchdog.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How often the watchdog looks at the fences.
const WATCH_POLL: Duration = Duration::from_millis(100);

/// The watchdog thread: until the renderer stops or fails, look at the
/// fences and the render call every [`WATCH_POLL`]; a stall ends GPU
/// rendering like a lost device ([`stalled`]).
fn watch(shared: &Arc<Shared>, limit: Duration) {
    let mut clocks = watchdog::Watch::new(limit);
    loop {
        thread::park_timeout(WATCH_POLL);
        if shared.stop.load(Ordering::Acquire) || lock(&shared.failed).is_some() {
            return;
        }
        let Some(fences) = shared.fences.get() else { continue };
        // What was enqueued before what the GPU reached: a value enqueued
        // in between is seen as reached, never as behind.
        let ready = shared.ready_enqueued.load(Ordering::Acquire);
        let done = shared.done_enqueued.load(Ordering::Acquire);
        let sample = watchdog::Sample {
            ready: (ready, fences.ready.completed()),
            done: (done, fences.done.completed()),
            call: shared.calls.load(Ordering::Acquire),
        };
        if let Some(reason) = clocks.observe(Instant::now(), sample) {
            stalled(shared, reason);
            return;
        }
    }
}

/// GPU rendering stalled: release every GPU wait on the shared fences (so
/// neither queue waits forever and the windows draw again), stop GPU
/// rendering (`Handle::failed`), and, where the host installed a device-loss
/// handler ([`WindowDevice::on_error`]), treat it as a lost device: the
/// host records it in the crash guard and restarts the player.
fn stalled(shared: &Shared, reason: String) {
    {
        // Marked first: the render thread's own failure (its next signal
        // fails once the fences are released) must not replace it.
        let mut failed = lock(&shared.failed);
        if failed.is_some() {
            return;
        }
        *failed = Some(reason.clone());
    }
    shared.diag(format!("{reason}; GPU rendering stops as after a lost device"));
    if let Some(fences) = shared.fences.get() {
        fences.ready.release_all();
        fences.done.release_all();
    }
    shared.ask_redraw();
    if let Some(lost) = ON_LOST.get() {
        LOST.store(true, Ordering::Release);
        lost();
    }
}

/// Times each frame's completion on the GPU off the render thread (the
/// frame log's `complete`) when the render thread does not wait for it.
struct Completion {
    frames: Option<mpsc::Sender<(u64, Instant)>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Completion {
    fn start(shared: Arc<Shared>) -> Self {
        let (frames, received) = mpsc::channel::<(u64, Instant)>();
        let thread = thread::Builder::new().name("vestra-gpu-complete".into()).spawn(move || {
            let Ok(waiter) = Waiter::new() else { return };
            for (value, signalled) in received {
                if wait_ready(&shared, &waiter, value, FRAME_WAIT) {
                    let ms = signalled.elapsed().as_secs_f64() * 1e3;
                    let mut stats = lock(&shared.stats);
                    stats.completes += 1;
                    stats.complete_max = stats.complete_max.max(ms);
                    stats.complete_total += ms;
                }
            }
        });
        Self { frames: Some(frames), thread: thread.ok() }
    }
    /// Frame `value` was signalled now.
    fn signalled(&self, value: u64) {
        if let Some(frames) = &self.frames {
            let _ = frames.send((value, Instant::now()));
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        drop(self.frames.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// `VESTRA_GPU_CPU_WAIT=1`: the render thread waits on the CPU until each
/// frame finished on the GPU before publishing it (up to host 0.5.18, for
/// A/B measurements). Otherwise only the GPU-side fences order the frames
/// (round record 2026-10-render-modes §11 item 3).
fn cpu_wait_on() -> bool {
    std::env::var("VESTRA_GPU_CPU_WAIT").as_deref() == Ok("1")
}

/// Frames mpv's device may have in flight without the CPU wait: before it
/// renders frame n+1, frame n-1 must have finished (as DXGI's default frame
/// latency bounds a swapchain), so a GPU slower than the video does not
/// queue frames without end.
const IN_FLIGHT: u64 = 2;

/// Wait on the CPU up to `timeout` for `ready` to reach `value`, in steps,
/// giving up once the renderer stops: a renderer dropped while the fences
/// do not move (a stall, `VESTRA_GPU_WATCHDOG=0`) must not wait 2 s per
/// wait. `false` when it was not reached.
fn wait_ready(shared: &Shared, waiter: &Waiter, value: u64, timeout: Duration) -> bool {
    let Some(fences) = shared.fences.get() else { return false };
    let deadline = Instant::now() + timeout;
    loop {
        let stopped = shared.stop.load(Ordering::Acquire);
        let left = if stopped {
            Duration::ZERO
        } else {
            deadline.saturating_duration_since(Instant::now()).min(WATCH_POLL)
        };
        if waiter.wait(&fences.ready, value, left.as_millis().max(1) as u32) {
            return true;
        }
        if stopped || Instant::now() >= deadline {
            return false;
        }
    }
}

/// How long the render thread waits for one frame on the CPU.
const FRAME_WAIT: Duration = Duration::from_secs(2);

/// `VESTRA_TEST_GPU_STALL=<seconds>` (a hardware test switch for the
/// watchdog): that long after the first frame, mpv's device waits on the GPU
/// for a `done` value no window ever signals, as a fence deadlock in a
/// driver would.
fn test_stall_after() -> Option<Duration> {
    std::env::var("VESTRA_TEST_GPU_STALL")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.)
        .map(Duration::from_secs_f64)
}

/// A `done` value no window will signal (the stall test switches): far
/// above any value one run reaches, below `u64::MAX`, the value
/// `SharedFence::release_all` sets, which releases the wait.
fn never_signalled(fences: &Fences) -> u64 {
    fences.done.completed().saturating_add(1 << 40)
}

/// One WARP device per process (Direct3D 12's software rasterizer), for tests:
#[cfg(any(test, feature = "test-support"))]
/// the instance must outlive the device, and WARP is slow to set up.
fn warp() -> Result<Arc<Gpu>, String> {
    static WARP: OnceLock<Result<Arc<Gpu>, String>> = OnceLock::new();
    WARP.get_or_init(|| Gpu::open(Choice::Warp).map(Arc::new)).clone()
}

/// For the host's tests only (feature `test-support`, no window device, no
/// GPU needed): a renderer on WARP, so a test can see what libmpv's
/// Direct3D 11 renderer logs.
#[cfg(feature = "test-support")]
pub fn start_on_warp(
    mpv: Arc<Mpv>,
    size: Size,
    publish: Arc<AtomicBool>,
) -> Result<Renderer, String> {
    Renderer::start_on(warp()?, mpv, size, publish)
}

fn clamp(size: Size) -> Size {
    (size.0.clamp(1, 8192), size.1.clamp(1, 8192))
}

/// `VESTRA_GPU_PRIORITY`: mpv's device's GPU thread priority (-7 lowest to
/// 7 highest). Unset: -7, as in the spike's and the parked branch's
/// measurements; `0` is the driver's default; an invalid value: unchanged.
fn gpu_priority() -> Option<i32> {
    match std::env::var("VESTRA_GPU_PRIORITY") {
        Err(_) => Some(-7),
        Ok(value) => value.trim().parse::<i32>().ok().filter(|p| (-7..=7).contains(p)),
    }
}

/// The render thread: mpv renders each due frame into a free texture of the
/// ring and the thread publishes it with its `ready` value; with
/// `VESTRA_GPU_CPU_WAIT=1` it first waits (an event) until the GPU finished
/// the frame.
///
/// Without that wait (the default) the GPU-side fences carry the ordering
/// (round record 2026-10-render-modes §11 item 3):
///
/// - a window samples a texture only after its queue waited for the frame's
///   `ready` value (`View::before`/`direct`), so it never shows a frame
///   mpv's GPU is still drawing;
/// - mpv draws into a texture again only after its device waited for the
///   `done` value of the last window draw that showed it, and never into
///   the newest or a claimed one (`ring.rs`), so slot reuse is ordered on
///   the GPU as before;
/// - a ring generation is dropped (a resize) and the thread ends only after
///   mpv's last frame finished, so no render target goes while mpv's GPU
///   still writes it;
/// - at most [`IN_FLIGHT`] frames are on the GPU (a CPU wait for frame n-1
///   before frame n+1), so a slow GPU cannot queue frames without end;
/// - a removed device is asked for after each frame (no completion
///   timeout notices it any more); its fences complete with `u64::MAX`, and
///   a fence that never moves without a removal is the watchdog's.
fn produce(
    mpv: Arc<Mpv>,
    shared: Arc<Shared>,
    publish: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let gpu = shared.gpu.clone();
    let made = D3d11::new(&gpu, gpu_priority()).and_then(|d3d11| {
        let fences = Fences {
            ready: SharedFence::new(&gpu, &d3d11)?,
            done: SharedFence::new(&gpu, &d3d11)?,
        };
        // Sharing a texture works on this adapter before anything is promised.
        d3d::shared_slot(&gpu, &d3d11, (16, 16))?;
        let waiter = Waiter::new()?;
        let render = mpv::RenderContext::new(&mpv, &d3d11.device)?;
        Ok((d3d11, fences, waiter, render))
    });
    let (d3d11, fences, waiter, render) = match made {
        Ok(made) => made,
        Err(error) => return drop(ready.send(Err(error))),
    };
    let fences = shared.fences.get_or_init(|| fences);
    // The callback's target, `shared`, outlives `render` (dropped first below).
    render.on_update(&shared);
    // mpv's `BLOCK_FOR_TARGET_TIME`: off (render a due frame at once, as the
    // update callback asks), unless `VESTRA_MPV_BLOCK=1` (the first version's
    // behaviour: wait inside the render call for the frame's display time).
    let block = std::env::var("VESTRA_MPV_BLOCK").as_deref() == Ok("1");
    // Without the windows' fence waits only the CPU wait orders the frames.
    let cpu_wait = cpu_wait_on() || !fences_on();
    shared.cpu_wait.store(cpu_wait, Ordering::Relaxed);
    shared.diag(format!(
        "mpv renders with Direct3D 11 on {} (LUID {:016x}, video support {}, GPU thread priority {}, \
         block for target time {}, CPU wait for each frame {}); the windows show its frames through \
         shared Direct3D 12 textures",
        gpu.facts.name,
        gpu.facts.luid,
        if d3d11.video { "yes" } else { "no" },
        d3d11.priority,
        if block { "on" } else { "off" },
        if cpu_wait { "on (VESTRA_GPU_CPU_WAIT=1)" } else { "off" }
    ));
    let _ = ready.send(Ok(()));
    // Without the CPU wait each frame's completion is timed off this thread.
    let completion = (!cpu_wait).then(|| Completion::start(shared.clone()));
    let stall_after = test_stall_after();
    let mut stall_at = None::<Instant>;
    let (mut slots, mut slots_size) = (Vec::<d3d::Slot>::new(), None::<Size>);
    // A new size first seen when; the ring changes once it has held for
    // `SETTLE` (a border drag asks for a new size every frame).
    let mut asked_size = None::<(Size, Instant)>;
    let (mut drawn, mut ready_value, mut timeouts) = (None::<Instant>, 0u64, 0u64);
    // GPU rendering cannot go on: release every GPU wait on the shared
    // fences (neither queue may wait forever) and tell the host, which
    // switches to software rendering (`Handle::failed`).
    // The first failure stands: after the watchdog released the fences
    // (`u64::MAX`), this thread's next signal fails too.
    let fail = |reason: String| {
        if shared.halted() {
            return;
        }
        let reason = match d3d11.removed() {
            Some(removed) => format!("{reason} (device removed: {removed})"),
            None => reason,
        };
        shared.diag(format!("{reason}; GPU rendering stops"));
        fences.ready.release_all();
        fences.done.release_all();
        lock(&shared.failed).get_or_insert(reason);
        shared.ask_redraw();
    };
    // Ring allocations that failed in a row (without a removed device).
    let mut ring_failures = 0u32;
    while !shared.stop.load(Ordering::Acquire) {
        #[cfg(test)]
        if shared.fail_now.load(Ordering::Acquire) {
            fail("a test asked the renderer to fail".into());
            break;
        }
        // The watchdog stopped GPU rendering (it released the fences).
        if shared.halted() {
            break;
        }
        let pace = *lock(&shared.pace);
        // Paced: wait for the rest of the period, so frames come at most at
        // the display rate however often mpv asks.
        let left = pace.zip(drawn).map(|(period, at)| period.saturating_sub(at.elapsed()));
        match left {
            Some(left) if !left.is_zero() => thread::sleep(left),
            _ => shared.wait(Duration::from_millis(20)),
        }
        let size = *lock(&shared.size);
        let mut resized = false;
        if slots_size == Some(size) {
            asked_size = None;
        } else {
            let since = match asked_size {
                Some((asked, since)) if asked == size => since,
                _ => asked_size.insert((size, Instant::now())).1,
            };
            // Until then the old ring keeps rendering (Slint scales it).
            if slots.is_empty() || since.elapsed() >= SETTLE {
                let made: Result<Vec<_>, _> =
                    (0..RING).map(|_| d3d::shared_slot(&gpu, &d3d11, size)).collect();
                match made {
                    // The old generation's textures go now (mpv's side) and
                    // once no frame or window image refers to them (wgpu's).
                    // Without the CPU wait, mpv's last frame in them may
                    // still be on the GPU: let it finish first (a resize is
                    // rare; Direct3D 11 would defer the release anyway).
                    Ok(made) => {
                        if !cpu_wait {
                            let _ = wait_ready(&shared, &waiter, ready_value, FRAME_WAIT);
                        }
                        slots = made;
                        ring_failures = 0;
                    }
                    // A removed device, or the third failure in a row, ends
                    // GPU rendering; otherwise the old ring (if any) keeps
                    // rendering and the size is tried again after `SETTLE`.
                    Err(error) => {
                        ring_failures += 1;
                        let reason = format!("no ring at {}x{}: {error}", size.0, size.1);
                        if d3d11.removed().is_some() || ring_failures >= 3 {
                            fail(reason);
                            break;
                        }
                        shared.diag(format!("{reason}; trying again"));
                        asked_size = Some((size, Instant::now()));
                        continue;
                    }
                }
                (slots_size, resized) = (Some(size), true);
                let mut exchange = lock(&shared.exchange);
                exchange.ring.renew();
                exchange.done = [0; RING];
            }
        }
        if slots.len() != RING {
            continue;
        }
        let published = lock(&shared.exchange).ring.sequence();
        if !(render.update() || (resized && published > 0)) {
            continue;
        }
        // A free texture. Windows hold a claim only during a draw; one left
        // by a draw that never finished is released at the window's next
        // draw, so after 100 ms the windows are asked to draw.
        let (waited, mut asked) = (Instant::now(), 0u32);
        let index = loop {
            if let Some(index) = lock(&shared.exchange).ring.target() {
                break Some(index);
            }
            if shared.stop.load(Ordering::Acquire) {
                break None;
            }
            if waited.elapsed() >= Duration::from_millis(100) * (asked + 1) {
                asked += 1;
                shared.ask_redraw();
            }
            thread::sleep(Duration::from_millis(1));
        };
        let Some(index) = index else { break };
        // The last window draw that showed this texture may still be on the
        // GPU: mpv's commands wait for it there.
        let need = lock(&shared.exchange).done[index];
        let done_wait = need > fences.done.completed();
        if done_wait && !d3d11.gpu_wait(&fences.done, need) {
            fail("mpv's device could not wait for a window's draw".into());
            break;
        }
        // Test switches for the watchdog: a `done` value no window signals
        // (released only by `SharedFence::release_all`).
        #[cfg(test)]
        if shared.stall_now.swap(false, Ordering::AcqRel) {
            let _ = d3d11.gpu_wait(&fences.done, never_signalled(fences));
        }
        if let Some(after) = stall_after {
            let at = *stall_at.get_or_insert_with(|| Instant::now() + after);
            if Instant::now() >= at && ready_value > 0 {
                shared.diag("VESTRA_TEST_GPU_STALL: mpv's device now waits for a fence value no window signals".into());
                let _ = d3d11.gpu_wait(&fences.done, never_signalled(fences));
                stall_at = Some(Instant::now() + Duration::from_secs(24 * 60 * 60));
            }
        }
        // Without the CPU wait: at most `IN_FLIGHT` frames on the GPU.
        let mut throttled = false;
        if !cpu_wait && ready_value >= IN_FLIGHT {
            let bound = ready_value + 1 - IN_FLIGHT;
            if fences.ready.completed() < bound {
                throttled = true;
                if !wait_ready(&shared, &waiter, bound, FRAME_WAIT) {
                    if d3d11.removed().is_some() {
                        fail("a frame did not finish on the GPU".into());
                        break;
                    }
                    // Stopping: no render and no signal after a wait given up.
                    if shared.stop.load(Ordering::Acquire) {
                        break;
                    }
                }
            }
        }
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let began = Instant::now();
        shared.calls.fetch_add(1, Ordering::AcqRel);
        let rendered = render.render(&slots[index].d3d11, block && pace.is_none());
        shared.calls.fetch_add(1, Ordering::AcqRel);
        if let Err(error) = rendered {
            if d3d11.removed().is_some() {
                fail(format!("mpv's render failed ({error})"));
                break;
            }
            continue;
        }
        let rendered = began.elapsed();
        // The watchdog stopped GPU rendering meanwhile: its release set the
        // fences to `u64::MAX`, and no signal may follow (a lower value),
        // nor may a frame be published.
        if shared.halted() {
            break;
        }
        // Published only with its `ready` signal enqueued: a window's queue
        // waits for that value.
        if let Err(error) = d3d11.try_signal(&fences.ready, ready_value + 1) {
            fail(format!("mpv's device could not signal a finished frame ({error})"));
            break;
        }
        ready_value += 1;
        shared.ready_enqueued.store(ready_value, Ordering::Release);
        let completed = match &completion {
            // The GPU-side fences order everything (record §11 item 3):
            // windows wait for `ready` on their queue, mpv's device waits
            // for `done` before it renders into a texture again. No
            // completion timeout notices a removed device here: ask.
            Some(completion) => {
                completion.signalled(ready_value);
                if let Some(removed) = d3d11.removed() {
                    fail(format!("mpv's device was removed ({removed})"));
                    break;
                }
                None
            }
            None => {
                let completing = Instant::now();
                if !wait_ready(&shared, &waiter, ready_value, FRAME_WAIT) {
                    if d3d11.removed().is_some() {
                        fail("a frame did not finish on the GPU".into());
                        break;
                    }
                    timeouts += 1;
                    if timeouts == 1 {
                        shared.diag("a frame did not finish within 2 s on the GPU".into());
                    }
                }
                Some(completing.elapsed())
            }
        };
        {
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            let mut stats = lock(&shared.stats);
            stats.renders += 1;
            stats.render_max = stats.render_max.max(ms(rendered));
            stats.render_total += ms(rendered);
            if let Some(completed) = completed.map(ms) {
                stats.completes += 1;
                stats.complete_max = stats.complete_max.max(completed);
                stats.complete_total += completed;
            }
            stats.done_waits += u64::from(done_wait);
            stats.stuck += u64::from(asked > 0);
            stats.throttled += u64::from(throttled);
        }
        if pace.is_some() {
            render.report_swap();
        }
        drawn = Some(Instant::now());
        if publish.load(Ordering::Acquire) && !shared.halted() {
            let texture = slots[index].texture.clone();
            let payload = Payload { texture, ready: ready_value, size: slots_size.unwrap_or(size) };
            lock(&shared.exchange).ring.publish(index, payload);
            shared.ask_redraw();
            lock(&shared.notify).iter().for_each(|notify| notify());
        }
    }
    // mpv's last frame finishes before its textures go (after a failure
    // the fence is released, so this returns at once); a renderer stopped
    // while its fences do not move (a stall with the watchdog off) gives up
    // after a short wait instead: both are released, as on a failure, so
    // nothing waits on them any more.
    if !waiter.wait(&fences.ready, ready_value, 250) {
        fences.ready.release_all();
        fences.done.release_all();
    }
    drop(completion);
    // mpv's render context goes before the textures, the device and
    // `shared` (the update callback's target).
    drop(render);
}

/// How long a new video-area size must hold before the ring is remade at it.
const SETTLE: Duration = Duration::from_millis(150);

/// A frame the renderer published ([`Handle::latest`]).
pub struct Frame(Payload);
impl Frame {
    /// Its size in pixels.
    pub fn size(&self) -> Size {
        self.0.size
    }
}

impl Handle {
    /// The physical size to render at (the video area).
    pub fn set_size(&self, size: Size) {
        if std::mem::replace(&mut *lock(&self.0.size), clamp(size)) != clamp(size) {
            mpv::Wake::wake(&*self.0);
        }
    }
    /// `Some(fps)`: render at most `fps` frames a second and report each as
    /// presented (mpv `video-sync=display-*`); `None`: mpv's own timing.
    pub fn set_display_fps(&self, fps: Option<f64>) {
        let period =
            fps.filter(|f| f.is_finite() && *f >= 1.).map(|f| Duration::from_secs_f64(1. / f));
        *lock(&self.0.pace) = period;
        mpv::Wake::wake(&*self.0);
    }
    /// Why the render thread stopped (the device was removed, a fence could
    /// not be signalled), if it did: the host renders in software then.
    pub fn failed(&self) -> Option<String> {
        lock(&self.0.failed).clone()
    }
    /// Frames published so far.
    pub fn frames(&self) -> u64 {
        lock(&self.0.exchange).ring.sequence()
    }
    /// Window draws that showed a newer frame.
    pub fn shown(&self) -> u64 {
        self.0.shown.load(Ordering::Relaxed)
    }
    /// The newest frame's size in pixels.
    pub fn size(&self) -> Option<Size> {
        lock(&self.0.exchange).ring.latest().map(|frame| frame.payload.size)
    }
    /// The caller replaced the image on the video area: windows show no
    /// frame published so far and set the next one's image again.
    pub fn reshow(&self) {
        let hold = self.frames();
        let mut reshow = lock(&self.0.reshow);
        *reshow = (reshow.0 + 1, hold);
    }
    /// The newest frame and its number.
    pub fn latest(&self) -> Option<(u64, Frame)> {
        let exchange = lock(&self.0.exchange);
        exchange.ring.latest().map(|frame| (frame.sequence, Frame(frame.payload.clone())))
    }
    /// Call `notify` on the render thread after each new frame.
    pub fn on_frame(&self, notify: impl Fn() + Send + 'static) {
        lock(&self.0.notify).push(Box::new(notify));
    }
    /// The renderer's diagnostics (its device, failures), from any thread;
    /// lines from before this call come at once.
    pub fn on_diag(&self, sink: impl Fn(String) + Send + Sync + 'static) {
        let mut diag = lock(&self.0.diag);
        diag.0.drain(..).for_each(&sink);
        diag.1 = Some(Box::new(sink));
    }
    /// `VESTRA_FRAME_LOG`: the render thread since the last call (mpv's
    /// render call, the wait for the GPU to finish the frame, mpv's GPU
    /// waiting for a window's draw, no free texture).
    pub fn log_text(&self) -> String {
        let stats = std::mem::take(&mut *lock(&self.0.stats));
        if stats.renders == 0 {
            return String::new();
        }
        let count = stats.renders as f64;
        let completes = stats.completes.max(1) as f64;
        // Appended last, so readers of the older segment keep working.
        let throttled = if self.0.cpu_wait.load(Ordering::Relaxed) {
            String::new()
        } else {
            format!(" throttled {}", stats.throttled)
        };
        format!(
            " gpu: render max {:.1} mean {:.1} ms complete max {:.1} mean {:.1} ms done-waits {} stuck {}{throttled}",
            stats.render_max,
            stats.render_total / count,
            stats.complete_max,
            stats.complete_total / completes,
            stats.done_waits,
            stats.stuck
        )
    }
    /// Show the video in `component`'s window (call before it is shown):
    /// `show` gets the image to set on the video area of whatever component
    /// the window shows then, whenever the shown texture changes (each new
    /// frame, after [`reshow`](Self::reshow), after
    /// [`retarget`](Self::retarget)). Each new frame asks the last four
    /// attached windows to redraw.
    /// `failed` runs once if the window cannot show the textures (it does not
    /// draw with the [`WindowDevice`]). `drawn` sees each draw of the window
    /// (this notifier is the window's only one).
    pub fn attach<T>(
        &self,
        component: &T,
        show: impl Fn(Image) + 'static,
        failed: impl FnOnce(String) + 'static,
        mut drawn: impl FnMut(Draw) + 'static,
    ) -> Result<(), String>
    where
        T: ComponentHandle + 'static,
    {
        if !uses_rendering_notifier() {
            return self.attach_direct(component, show);
        }
        self.redraw_on(component);
        let (shared, mut view) = (self.0.clone(), None::<View>);
        let (mut failed, mut fresh) = (Some(failed), false);
        let (mut current, mut retargets) =
            (None::<Image>, self.0.retargets.load(Ordering::Relaxed));
        let notifier = move |state, api: &GraphicsAPI| match state {
            RenderingState::RenderingSetup => {
                let ours = matches!(api, GraphicsAPI::WGPU30 { device, .. } if *device == shared.gpu.device);
                view = ours.then(View::default);
                if !ours && let Some(failed) = failed.take() {
                    failed(
                        "the window does not draw with the renderer's Direct3D 12 device".into(),
                    );
                }
            }
            RenderingState::BeforeRendering => {
                drawn(Draw::Begin);
                fresh = false;
                let now = shared.retargets.load(Ordering::Relaxed);
                let retargeted = std::mem::replace(&mut retargets, now) != now;
                if let Some(view) = view.as_mut() {
                    let (image, new) = view.before(&shared);
                    fresh = new;
                    if let Some(image) = image {
                        current = Some(image.clone());
                        show(image);
                    } else if retargeted
                        && view.showing.is_some()
                        && let Some(image) = &current
                    {
                        // Another component took the window's place: same
                        // texture (not one a `reshow` took off the video area).
                        show(image.clone());
                    }
                }
            }
            RenderingState::AfterRendering => {
                if let Some(view) = view.as_mut() {
                    view.after(&shared);
                }
                drawn(Draw::End { copied: std::mem::take(&mut fresh) })
            }
            RenderingState::RenderingTeardown => {
                if let Some(mut view) = view.take() {
                    view.after(&shared);
                }
                current = None;
            }
            _ => (),
        };
        component.window().set_rendering_notifier(notifier).map_err(|e| e.to_string())
    }
    /// Another component took the place of an attached one inside the same
    /// window (UI thread; its rendering notifier, or without one its direct
    /// update, stays): new frames ask it to redraw, and the window hands its
    /// current image to `show` again, for the new component's video area.
    pub fn retarget<T>(&self, component: &T)
    where
        T: ComponentHandle + 'static,
    {
        self.0.retargets.fetch_add(1, Ordering::Relaxed);
        if uses_rendering_notifier() {
            self.redraw_on(component);
            return;
        }
        let weak = component.as_weak();
        DIRECT_ALIVE.with(|alive| {
            if let Some(alive) = &*alive.borrow() {
                *alive.borrow_mut() = Box::new(move || weak.upgrade().is_some());
            }
        });
        // Paused playback publishes nothing new: show the current texture now.
        wake_direct_views();
    }
    /// Each new frame asks `component`'s window to redraw (the last four
    /// components given here; a dropped one ignores it).
    fn redraw_on<T>(&self, component: &T)
    where
        T: ComponentHandle + 'static,
    {
        let weak = component.as_weak();
        let mut redraw = lock(&self.0.redraw);
        redraw.push(Redraw {
            direct: false,
            call: Box::new(move || {
                let _ = weak.upgrade_in_event_loop(|component| component.window().request_redraw());
            }),
        });
        let stale = redraw.len().saturating_sub(4);
        drop(redraw.drain(..stale));
    }
}

impl Handle {
    /// `attach` without a rendering notifier (the default): each new frame
    /// wakes the UI thread, which sets the newest texture's image between
    /// draws (`View::direct`). `drawn` is not called: the host counts the
    /// window's draws itself (`uses_rendering_notifier`).
    fn attach_direct<T>(&self, component: &T, show: impl Fn(Image) + 'static) -> Result<(), String>
    where
        T: ComponentHandle + 'static,
    {
        let (shared, mut view) = (self.0.clone(), View::default());
        // Whether the window is still there: its component, or the one that
        // took its place in the same window (`retarget`).
        let weak = component.as_weak();
        let alive: Alive = Rc::new(RefCell::new(Box::new(move || weak.upgrade().is_some())));
        DIRECT_ALIVE.with(|newest| *newest.borrow_mut() = Some(alive.clone()));
        // The newest attached window shows the video; an earlier one (a
        // rebuilt main window that is still alive) lets its texture go.
        let generation = shared.direct_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (mut current, mut retargets) =
            (None::<Image>, shared.retargets.load(Ordering::Relaxed));
        let update = move || -> bool {
            let newest = shared.direct_generation.load(Ordering::Acquire) == generation;
            if !newest || !(alive.borrow())() {
                // The window is gone or superseded: its texture is free again.
                view.after(&shared);
                return false;
            }
            let now = shared.retargets.load(Ordering::Relaxed);
            let retargeted = std::mem::replace(&mut retargets, now) != now;
            if let Some(image) = view.direct(&shared, &direct_accepts) {
                current = Some(image.clone());
                show(image);
            } else if retargeted
                && view.showing.is_some()
                && let Some(image) = &current
            {
                // Another component took the window's place: same texture.
                show(image.clone());
            }
            true
        };
        DIRECT.with(|views| views.borrow_mut().push(Box::new(update)));
        let mut redraw = lock(&self.0.redraw);
        if !redraw.iter().any(|entry| entry.direct) {
            redraw.push(Redraw { direct: true, call: Box::new(wake_direct_views) });
        }
        drop(redraw);
        // Frames published before the window came.
        wake_direct_views();
        Ok(())
    }
}

/// Whether a window shown without a notifier is still there.
type Alive = Rc<RefCell<Box<dyn Fn() -> bool>>>;

thread_local! {
    /// The windows shown without a rendering notifier (UI thread): each
    /// updates its image; `false` once its window is gone.
    static DIRECT: RefCell<Vec<Box<dyn FnMut() -> bool>>> = const { RefCell::new(Vec::new()) };
    /// The newest such window's liveness check, which `retarget` points at
    /// the component that took its place.
    static DIRECT_ALIVE: RefCell<Option<Alive>> = const { RefCell::new(None) };
    /// Whether a window shown without a notifier may take a new frame now
    /// ([`gate_direct_frames`]); none: always.
    static DIRECT_GATE: RefCell<Option<Box<dyn Fn() -> bool>>> = const { RefCell::new(None) };
}
/// An update of the direct windows is queued on the UI thread.
static DIRECT_QUEUED: AtomicBool = AtomicBool::new(false);

/// Have the UI thread update every window shown without a notifier
/// (coalesced: one queued update at a time).
fn wake_direct_views() {
    if DIRECT_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = slint::invoke_from_event_loop(|| {
        DIRECT_QUEUED.store(false, Ordering::Release);
        DIRECT.with(|views| views.borrow_mut().retain_mut(|update| update()));
    });
}

/// The host's frame pacing for windows shown without a notifier (UI
/// thread): `accept` says whether such a window may take a new frame now.
/// The host refuses while the previous picture still waits for its draw, so
/// two frames published within one refresh are both shown, one refresh
/// apart, instead of the second replacing the first; after that draw it
/// calls [`update_direct_views`]. `None`: every new frame is taken at once.
pub fn gate_direct_frames(accept: Option<Box<dyn Fn() -> bool>>) {
    DIRECT_GATE.with(|gate| *gate.borrow_mut() = accept);
}

/// Update the windows shown without a notifier (posted to the UI thread):
/// after a draw, for a frame [`gate_direct_frames`] refused.
pub fn update_direct_views() {
    wake_direct_views();
}

fn direct_accepts() -> bool {
    DIRECT_GATE.with(|gate| gate.borrow().as_ref().is_none_or(|accept| accept()))
}

/// Whether windows show the frames through a rendering notifier
/// (`VESTRA_WGPU_NOTIFIER=1`, the first design: claim and fence per draw).
/// Off by default: any rendering notifier makes Slint 1.18.1's femtovg
/// flush the window's clear separately before `BeforeRendering`, and
/// femtovg-wgpu then drops every render pipeline the clear did not use
/// (`pipeline_cache.retain` after each flush), so each draw recreates and
/// recompiles them (round record §9). The host counts draws itself then.
pub fn uses_rendering_notifier() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("VESTRA_WGPU_NOTIFIER").as_deref() == Ok("1"))
}

/// `VESTRA_GPU_FENCES=0` (diagnostics only): no GPU waits or signals on the
/// shared fences from the windows' side; mpv's frames are still finished on
/// the CPU before they are published (this turns the render thread's CPU
/// wait on), but a texture may be rendered into
/// while a draw still samples it (possible tearing).
fn fences_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("VESTRA_GPU_FENCES").as_deref() != Ok("0"))
}

/// A draw of an attached window, for the host's presentation counters.
pub enum Draw {
    /// The window starts drawing.
    Begin,
    /// The window drew; `copied`: it showed a newer frame than its last draw.
    End { copied: bool },
}

/// One window's side (UI thread).
#[derive(Default)]
struct View {
    /// The images of the current generation's textures, made on first use.
    images: Vec<(Slot, Image)>,
    /// The texture whose image is on the video area.
    showing: Option<Slot>,
    /// This draw's texture.
    claim: Option<Slot>,
    sequence: u64,
    epoch: u64,
}

impl View {
    /// Before a draw: take the newest frame for it and have the windows'
    /// queue wait on the GPU until the frame is finished (`ready`). The
    /// image to set when the shown texture changes, and whether the frame
    /// is newer than the last draw's.
    fn before(&mut self, shared: &Shared) -> (Option<Image>, bool) {
        // A claim left by a draw that never finished.
        self.after(shared);
        let Some(fences) = shared.fences.get() else {
            return (None, false);
        };
        if shared.halted() {
            // The renderer stopped; the host shows software pictures now.
            self.showing = None;
            return (None, false);
        }
        let (epoch, hold) = *lock(&shared.reshow);
        let stale = self.epoch != epoch;
        let mut exchange = lock(&shared.exchange);
        let Some(latest) = exchange.ring.latest() else {
            return (None, false);
        };
        if stale && latest.sequence <= hold {
            // The caller's picture stays until a newer frame comes.
            self.showing = None;
            return (None, false);
        }
        let Some(frame) = exchange.ring.claim() else {
            return (None, false);
        };
        if fences_on() && !shared.gpu.wait(&fences.ready, frame.payload.ready) {
            exchange.ring.release(frame.slot);
            return (None, false);
        }
        drop(exchange);
        self.claim = Some(frame.slot);
        let new = stale || frame.sequence > self.sequence;
        (self.sequence, self.epoch) = (frame.sequence, epoch);
        if stale || self.showing != Some(frame.slot) {
            self.images.retain(|(slot, _)| slot.generation == frame.slot.generation);
            let cached = self.images.iter().find(|(slot, _)| *slot == frame.slot);
            let image = match cached {
                Some((_, image)) => Some(image.clone()),
                None => Image::try_from(frame.payload.texture.clone()).ok().inspect(|image| {
                    self.images.push((frame.slot, image.clone()));
                }),
            };
            let Some(image) = image else {
                // Not shown: no claim, and the old texture's image (no
                // longer claimed) goes off the video area.
                if let Some(slot) = self.claim.take() {
                    lock(&shared.exchange).ring.release(slot);
                }
                self.showing = None;
                return (Some(Image::default()), false);
            };
            self.showing = Some(frame.slot);
            if new {
                shared.count_shown(frame.sequence);
            }
            return (Some(image), new);
        }
        if new {
            shared.count_shown(frame.sequence);
        }
        (None, new)
    }
    /// Without a rendering notifier (UI thread, between draws): show the
    /// newest frame if it is not shown yet. The window keeps the shown
    /// texture claimed until the next one replaces it; then `done` is
    /// signalled for the old one, after every draw that sampled it (they were
    /// all submitted before this point on the same thread and queue), and the
    /// windows' queue waits for the new frame's `ready` before the next draw.
    /// The image to set, if the shown texture changed.
    fn direct(&mut self, shared: &Shared, accept: &dyn Fn() -> bool) -> Option<Image> {
        let fences = shared.fences.get()?;
        if shared.halted() {
            self.after(shared);
            self.showing = None;
            return None;
        }
        let (epoch, hold) = *lock(&shared.reshow);
        let stale = self.epoch != epoch;
        let exchange = lock(&shared.exchange);
        let latest = exchange.ring.latest()?;
        if stale && latest.sequence <= hold {
            // The caller's picture replaced ours: the texture is free again.
            drop(exchange);
            self.after(shared);
            self.showing = None;
            return None;
        }
        if !stale && self.showing == Some(latest.slot) {
            return None;
        }
        drop(exchange);
        // Paced (`gate_direct_frames`): not while the shown picture still
        // waits for its draw; the frame stays in the ring and the host
        // updates again after that draw.
        if !accept() {
            return None;
        }
        let frame = lock(&shared.exchange).ring.claim()?;
        if fences_on() && !shared.gpu.wait(&fences.ready, frame.payload.ready) {
            lock(&shared.exchange).ring.release(frame.slot);
            return None;
        }
        self.images.retain(|(slot, _)| slot.generation == frame.slot.generation);
        let cached = self.images.iter().find(|(slot, _)| *slot == frame.slot);
        let image = match cached {
            Some((_, image)) => Some(image.clone()),
            None => Image::try_from(frame.payload.texture.clone()).ok().inspect(|image| {
                self.images.push((frame.slot, image.clone()));
            }),
        };
        // The old texture: `done` after its draws, then released.
        self.after(shared);
        let Some(image) = image else {
            lock(&shared.exchange).ring.release(frame.slot);
            self.showing = None;
            return Some(Image::default());
        };
        self.claim = Some(frame.slot);
        self.showing = Some(frame.slot);
        (self.sequence, self.epoch) = (frame.sequence, epoch);
        shared.count_shown(frame.sequence);
        Some(image)
    }
    /// The draw was submitted: `done` follows it on the windows' queue, and
    /// the texture may be rendered into again once the GPU reaches that
    /// value.
    fn after(&mut self, shared: &Shared) {
        let Some(slot) = self.claim.take() else {
            return;
        };
        if shared.halted() {
            // The fences were released for good; nothing waits any more.
            lock(&shared.exchange).ring.release(slot);
            return;
        }
        if !fences_on() {
            lock(&shared.exchange).ring.release(slot);
            return;
        }
        let signal = shared.signalled.fetch_add(1, Ordering::Relaxed) + 1;
        let signalled =
            shared.fences.get().is_some_and(|fences| shared.gpu.signal(&fences.done, signal));
        if signalled {
            shared.done_enqueued.fetch_max(signal, Ordering::AcqRel);
        }
        let mut exchange = lock(&shared.exchange);
        if signalled && slot.generation == exchange.ring.generation() {
            let done = &mut exchange.done[slot.index];
            *done = (*done).max(signal);
        }
        exchange.ring.release(slot);
    }
}
