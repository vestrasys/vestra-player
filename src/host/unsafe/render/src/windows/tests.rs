//! The handover on WARP (Direct3D 11 WARP for mpv, Direct3D 12 WARP for
//! wgpu): no GPU needed, so `cargo test` runs it everywhere, CI's Windows job
//! included. The `#[ignore]`d tests need a real GPU and the test clips: the
//! coordinator runs them (one at a time, at most a minute each):
//!
//! ```text
//! cargo test --locked -p vestra-gpu-render -- --ignored --test-threads 1
//! ```

use super::*;
use std::io::Write;

fn warp_gpu() -> Arc<Gpu> {
    warp().expect("Direct3D 12 WARP")
}

fn until(seconds: u64, what: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while !what() && start.elapsed() < Duration::from_secs(seconds) {
        thread::sleep(Duration::from_millis(10));
    }
    what()
}

fn player(hwdec: &str) -> Arc<Mpv> {
    let mpv = Mpv::builder().and_then(|b| {
        b.set_property("vo", "libmpv")?
            .set_property("ao", "null")?
            .set_property("hwdec", hwdec)?
            .set_property("keep-open", "yes")?
            .build()
    });
    Arc::new(mpv.expect("mpv"))
}

/// White over black, 64x64.
const SOURCE: &str =
    "av://lavfi:color=c=white:s=64x64:d=30,drawbox=y=32:w=64:h=32:color=black:t=fill";

/// The red value at the left of row `y` of a tightly packed RGBA picture.
fn red(pixels: &[u8], width: u32, y: u32) -> u8 {
    pixels[(y * width * 4) as usize]
}

#[test]
fn warp_mpv_renders_into_a_shared_texture_that_wgpu_reads_identically() {
    let gpu = &*warp_gpu();
    assert!(gpu.facts.software, "WARP has no GPU of its own: {}", gpu.facts.name);
    let d3d11 = D3d11::new(gpu, None).expect("Direct3D 11 WARP");
    let fences = Fences {
        ready: SharedFence::new(gpu, &d3d11).expect("ready"),
        done: SharedFence::new(gpu, &d3d11).expect("done"),
    };
    let slot = d3d::shared_slot(gpu, &d3d11, (64, 64)).expect("shared texture");
    let mpv = player("no");
    let render = mpv::RenderContext::new(&mpv, &d3d11.device).expect("mpv d3d11");
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || render.update()), "no frame due");
    render.render(&slot.d3d11, false).expect("render");
    let waiter = Waiter::new().unwrap();
    assert!(d3d11.signal(&fences.ready, 1) && waiter.wait(&fences.ready, 1, 5000));
    let (via11, via12) = (
        d3d::readback::d3d11(&d3d11, &slot.d3d11, (64, 64)),
        d3d::readback::wgpu(gpu, &slot.texture),
    );
    assert_eq!(via11.len(), via12.len());
    assert!(via11 == via12, "Direct3D 11 and wgpu/Direct3D 12 read different pixels");
    let (top, bottom) = (red(&via12, 64, 2), red(&via12, 64, 61));
    assert!(top > 200 && bottom < 50, "top {top} bottom {bottom}: not the picture, or upside down");
    // A window draw's fences on the windows' queue: wait for `ready`, then
    // signal `done`.
    assert!(gpu.wait(&fences.ready, 1) && gpu.signal(&fences.done, 7));
    assert!(until(5, || fences.done.completed() >= 7), "done {}", fences.done.completed());
    drop(render);
}

/// mpv's next frame into a texture waits on the GPU for the window draw
/// that showed it (`done`), and goes on once the windows' queue signals it.
#[test]
fn warp_done_holds_mpv_back_until_the_window_draw_signals_it() {
    let gpu = &*warp_gpu();
    let d3d11 = D3d11::new(gpu, None).expect("Direct3D 11 WARP");
    let fences = Fences {
        ready: SharedFence::new(gpu, &d3d11).expect("ready"),
        done: SharedFence::new(gpu, &d3d11).expect("done"),
    };
    let slot = d3d::shared_slot(gpu, &d3d11, (64, 64)).expect("shared texture");
    let mpv = player("no");
    let render = mpv::RenderContext::new(&mpv, &d3d11.device).expect("mpv d3d11");
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || render.update()), "no frame due");
    let waiter = Waiter::new().unwrap();
    assert!(d3d11.gpu_wait(&fences.done, 3), "the wait is enqueued");
    render.render(&slot.d3d11, false).expect("render");
    assert!(d3d11.signal(&fences.ready, 1));
    assert!(!waiter.wait(&fences.ready, 1, 300), "mpv's frame went past an unsignalled done");
    assert_eq!(fences.ready.completed(), 0);
    // The window's draw ends: the windows' queue signals done.
    assert!(gpu.signal(&fences.done, 3));
    assert!(waiter.wait(&fences.ready, 1, 5000), "mpv's frame did not follow done");
    drop(render);
}

#[test]
fn warp_renderer_publishes_shows_resizes_and_honours_reshow() {
    let gpu = warp_gpu();
    let mpv = player("no");
    let publish = Arc::new(AtomicBool::new(false));
    let renderer =
        Renderer::start_on(gpu.clone(), mpv.clone(), (64, 64), publish.clone()).expect("renderer");
    let handle = renderer.handle();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    handle.on_diag(move |line| lock(&sink).push(line));
    assert!(lock(&lines).iter().any(|line| line.contains("Direct3D 11")), "{:?}", lock(&lines));
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(handle.frames(), 0, "the publish gate is closed");
    publish.store(true, Ordering::Release);
    assert!(until(10, || handle.frames() > 0), "no frame once open");
    let (_, frame) = handle.latest().expect("latest");
    assert_eq!(frame.size(), (64, 64));
    // Without the CPU wait the frame may still be on mpv's GPU queue.
    let ready = &handle.0.fences.get().unwrap().ready;
    assert!(until(5, || ready.completed() >= frame.0.ready));
    let pixels = d3d::readback::wgpu(&gpu, &frame.0.texture);
    assert!(red(&pixels, 64, 2) > 200 && red(&pixels, 64, 61) < 50);

    // A window's draws: each takes the newest frame and waits for it; after
    // the draw `done` is signalled and the frame released.
    let shared = &handle.0;
    let mut view = View::default();
    let (image, new) = view.before(shared);
    assert!(image.is_some() && new, "the first draw shows the newest frame");
    view.after(shared);
    let (image, new) = view.before(shared);
    assert!(image.is_none() && !new, "the same frame again: nothing to set");
    view.after(shared);
    let signalled = shared.signalled.load(Ordering::Relaxed);
    let fences = shared.fences.get().unwrap();
    assert!(until(5, || fences.done.completed() >= signalled));
    assert_eq!(handle.shown(), 1);
    assert!(handle.failed().is_none());

    // After a reshow nothing published so far is shown again.
    handle.reshow();
    let (image, new) = view.before(shared);
    assert!(image.is_none() && !new && view.claim.is_none(), "held after a reshow");
    let before = handle.frames();
    mpv.command(&["seek", "0", "absolute"]).expect("seek");
    assert!(until(10, || handle.frames() > before), "no frame after the seek");
    let (image, new) = view.before(shared);
    assert!(image.is_some() && new, "the next frame is set again");
    view.after(shared);

    // A new size takes effect once it held (`SETTLE`), as a new generation.
    handle.set_size((48, 48));
    handle.set_size((32, 16));
    assert!(until(10, || handle.size() == Some((32, 16))), "no resize: {:?}", handle.size());
    let (image, _) = view.before(shared);
    assert!(image.is_some(), "a new generation's texture is a new image");
    view.after(shared);
    assert!(handle.log_text().starts_with(" gpu: render max"));
    drop(renderer);
}

/// The failure path (H2): the render thread releases both shared fences for
/// good, reports why, and stops; windows then claim nothing and signal
/// nothing.
#[test]
fn warp_a_failed_renderer_releases_its_fences_and_windows_stop_claiming() {
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    let shared = &handle.0;
    let mut view = View::default();
    assert!(view.before(shared).0.is_some());
    shared.fail_now.store(true, Ordering::Release);
    mpv::Wake::wake(&**shared);
    assert!(until(10, || handle.failed().is_some()), "the renderer did not fail");
    assert!(handle.failed().unwrap().contains("a test asked"));
    let fences = shared.fences.get().unwrap();
    assert_eq!((fences.ready.completed(), fences.done.completed()), (u64::MAX, u64::MAX));
    let signalled = shared.signalled.load(Ordering::Relaxed);
    view.after(shared);
    assert_eq!(shared.signalled.load(Ordering::Relaxed), signalled, "no done after a failure");
    let (image, new) = view.before(shared);
    assert!(image.is_none() && !new, "no frame after a failure");
    assert!(view.claim.is_none());
    drop(renderer);
}

/// The watchdog (doc/architecture/README.md §4.3): mpv's device waits on the GPU for a
/// `done` value no window signals, so `ready` stops moving; after the limit
/// the renderer stops as after a lost device, with both fences released.
/// Without a stall (playing, then paused) it never fires.
#[test]
fn warp_the_watchdog_stops_a_renderer_whose_fences_stop_moving() {
    let mpv = player("no");
    let limit = Some(Duration::from_millis(600));
    let renderer =
        Renderer::start_watched(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()), limit)
            .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    let shared = &handle.0;
    // Playing, then paused: well past the limit, no stall.
    thread::sleep(Duration::from_millis(1200));
    mpv.set_property("pause", true).expect("pause");
    thread::sleep(Duration::from_millis(1200));
    assert!(handle.failed().is_none(), "{:?}", handle.failed());
    // Playing again, then the deadlock.
    mpv.set_property("pause", false).expect("play");
    shared.stall_now.store(true, Ordering::Release);
    mpv::Wake::wake(&**shared);
    assert!(until(15, || handle.failed().is_some()), "the watchdog did not fire");
    let reason = handle.failed().unwrap();
    assert!(reason.contains("stalled"), "{reason}");
    let fences = shared.fences.get().unwrap();
    // Released: nothing waits on either fence any more. The stalled frame's
    // own `ready` signal was already queued behind the wait on `done` that
    // the release ends, so it may land after the release and set `ready`
    // back to that frame's value (seen once under a loaded `cargo test
    // --workspace`): that value, never one below what was enqueued.
    let ready = shared.ready_enqueued.load(Ordering::Acquire);
    let done = shared.done_enqueued.load(Ordering::Acquire);
    assert!(
        until(5, || fences.ready.completed() >= ready && fences.done.completed() >= done),
        "ready {} of {ready}, done {} of {done}",
        fences.ready.completed(),
        fences.done.completed()
    );
    assert_eq!(fences.done.completed(), u64::MAX, "no one signals done after the release");
    drop(renderer);
}

/// Review L3: what `stalled` itself does, with no frame queued that could
/// signal afterwards: both fences end at `u64::MAX` at once (a release of
/// `ready` or `done` left out fails here), the reason is kept, and a second
/// stall does not replace it.
#[test]
fn warp_a_stall_releases_both_fences_at_once() {
    let gpu = warp_gpu();
    let d3d11 = D3d11::new(&gpu, None).expect("Direct3D 11 WARP");
    let shared = Shared::new(gpu.clone(), (16, 16));
    let fences = shared.fences.get_or_init(|| Fences {
        ready: SharedFence::new(&gpu, &d3d11).expect("ready"),
        done: SharedFence::new(&gpu, &d3d11).expect("done"),
    });
    assert!(d3d11.signal(&fences.ready, 3) && gpu.signal(&fences.done, 2));
    assert!(until(5, || fences.ready.completed() == 3 && fences.done.completed() == 2));
    stalled(&shared, "GPU rendering stalled: a test".into());
    assert_eq!(fences.ready.completed(), u64::MAX, "ready released");
    assert_eq!(fences.done.completed(), u64::MAX, "done released");
    stalled(&shared, "a second reason".into());
    assert_eq!(lock(&shared.failed).as_deref(), Some("GPU rendering stalled: a test"));
}

/// A stall with the watchdog off, then the player
/// quits. Releasing the fences ends every GPU wait (the frame mpv's queue
/// held finishes), is not reported as a failure, stops the windows'
/// claims, and the renderer then drops at once.
#[test]
fn warp_quitting_during_a_stall_releases_the_fences_and_stops_at_once() {
    let mpv = player("no");
    let renderer =
        Renderer::start_watched(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()), None)
            .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    let shared = handle.0.clone();
    let mut view = View::default();
    assert!(view.direct(&shared, &|| true).is_some());
    shared.stall_now.store(true, Ordering::Release);
    mpv::Wake::wake(&*shared);
    let fences = shared.fences.get().unwrap();
    // A frame is enqueued that the GPU never finishes.
    assert!(
        until(10, || shared.ready_enqueued.load(Ordering::Acquire) > fences.ready.completed()),
        "no stalled frame"
    );
    thread::sleep(Duration::from_millis(300));
    assert!(shared.ready_enqueued.load(Ordering::Acquire) > fences.ready.completed(), "stalled");
    shared.release_for_quit();
    assert_eq!((fences.ready.completed(), fences.done.completed()), (u64::MAX, u64::MAX));
    assert!(handle.failed().is_none(), "quitting is not a failure");
    let signalled = shared.signalled.load(Ordering::Relaxed);
    view.after(&shared);
    assert_eq!(shared.signalled.load(Ordering::Relaxed), signalled, "no done after quitting");
    assert!(view.direct(&shared, &|| true).is_none(), "no claim after quitting");
    let dropping = Instant::now();
    drop(renderer);
    assert!(dropping.elapsed() < Duration::from_millis(1500), "{:?}", dropping.elapsed());
}

/// Without the CPU wait (the default) the render thread publishes frames
/// whose `ready` value a window's queue waits for, and their completion is
/// still timed (`complete`) off the render thread.
#[test]
fn warp_frames_are_published_without_the_cpu_wait_and_timed_off_thread() {
    assert!(!cpu_wait_on(), "VESTRA_GPU_CPU_WAIT is set for the tests");
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 2), "no frames");
    // Paused: the newest frame stays the newest while it is read back.
    mpv.set_property("pause", true).expect("pause");
    thread::sleep(Duration::from_millis(300));
    let shared = &handle.0;
    let fences = shared.fences.get().unwrap();
    let enqueued = shared.ready_enqueued.load(Ordering::Acquire);
    assert!(enqueued >= 3);
    assert!(until(5, || fences.ready.completed() >= enqueued), "frames do not finish");
    assert!(until(5, || lock(&shared.stats).completes > 0), "completion not timed");
    let text = handle.log_text();
    assert!(text.contains(" throttled "), "{text}");
    let (_, frame) = handle.latest().expect("latest");
    let pixels = d3d::readback::wgpu(&warp_gpu(), &frame.0.texture);
    assert!(red(&pixels, 64, 2) > 200 && red(&pixels, 64, 61) < 50);
    drop(renderer);
}

/// Paused, nothing new renders; kept warm, mpv's paused frame is rendered
/// and published once per period (the same picture), until it is turned off.
#[test]
fn warp_a_renderer_kept_warm_republishes_the_paused_frame() {
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 2), "no frames");
    mpv.set_property("pause", true).expect("pause");
    thread::sleep(Duration::from_millis(300));
    let paused = handle.frames();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(handle.frames(), paused, "paused: nothing new");
    handle.keep_warm(Some(Duration::from_millis(25)));
    assert!(until(5, || handle.warm_frames() >= 10), "not kept warm");
    assert_eq!(handle.frames(), paused, "warm frames are not video frames");
    let (_, frame) = handle.latest().expect("latest");
    let pixels = d3d::readback::wgpu(&warp_gpu(), &frame.0.texture);
    assert!(red(&pixels, 64, 2) > 200 && red(&pixels, 64, 61) < 50, "the paused picture");
    handle.keep_warm(None);
    thread::sleep(Duration::from_millis(200));
    let stopped = handle.warm_frames();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(handle.warm_frames(), stopped, "no longer kept warm");
    assert_eq!(handle.frames(), paused);
    drop(renderer);
}

#[test]
fn warm_frames_are_due_a_period_after_the_last_render() {
    let (t, ms) = (Instant::now(), Duration::from_millis);
    assert_eq!(warm_left(None, Some(t), t), None);
    assert_eq!(warm_left(Some(ms(40)), None, t), Some(Duration::ZERO));
    assert_eq!(warm_left(Some(ms(40)), Some(t), t + ms(10)), Some(ms(30)));
    assert_eq!(warm_left(Some(ms(40)), Some(t), t + ms(50)), Some(Duration::ZERO));
    // Interpolation paces to the display: the shorter period.
    assert_eq!(warm_period(Some(ms(40)), Some(ms(16))), Some(ms(16)));
    assert_eq!(warm_period(Some(ms(40)), None), Some(ms(40)));
    assert_eq!(warm_period(None, Some(ms(16))), None);
}

/// Without a rendering notifier a window keeps its texture claimed until a
/// newer frame replaces it; then the old one gets its `done` signal.
#[test]
fn warp_a_window_without_a_notifier_keeps_its_texture_until_the_next_frame() {
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", SOURCE]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    let shared = &handle.0;
    let mut view = View::default();
    assert!(view.direct(shared, &|| true).is_some(), "the newest frame is shown");
    let first = view.claim.expect("held");
    let before = handle.frames();
    assert!(until(10, || handle.frames() > before), "no newer frame");
    // Paced, while the shown picture still waits for its draw: the newer
    // frame stays in the ring and the shown texture stays claimed.
    assert!(view.direct(shared, &|| false).is_none(), "refused");
    assert_eq!(view.claim, Some(first), "still the first texture");
    assert!(view.direct(shared, &|| true).is_some(), "the newer frame is shown");
    assert!(view.claim.is_some_and(|slot| slot != first || slot.generation != first.generation));
    let signalled = shared.signalled.load(Ordering::Relaxed);
    assert!(signalled > 0, "the old texture got its done signal");
    let fences = shared.fences.get().unwrap();
    assert!(until(5, || fences.done.completed() >= signalled));
    assert!(view.direct(shared, &|| true).is_none() || handle.frames() > before + 1);
    view.after(shared);
    drop(renderer);
}
#[test]
fn the_adapter_choice_reads_the_environment_value() {
    assert_eq!(Choice::parse(None), Choice::Default);
    assert_eq!(Choice::parse(Some("")), Choice::Default);
    assert_eq!(Choice::parse(Some("warp")), Choice::Warp);
    assert_eq!(Choice::parse(Some(" 1 ")), Choice::Index(1));
    assert_eq!(Choice::parse(Some("high-performance")), Choice::HighPerformance);
    assert_eq!(Choice::parse(Some(" minimum-power")), Choice::MinimumPower);
    assert_eq!(Choice::parse(Some("dgpu")), Choice::Default);
}

// ------------------------------------------------- real-machine measurements

/// The first prototype's measurement on the
/// GPU this process gets by default, with a real 4K60 clip: mpv publishes at
/// the clip's rate through the shared ring, a simulated window takes the
/// newest frame 60 times a second, and the last frame reads the same
/// through both APIs. `VESTRA_GPU_CLIP` names the clip (default: the HEVC
/// 4K60 10-bit test clip of the Windows test machine).
#[test]
#[ignore = "needs a GPU and a 4K60 clip; the coordinator runs it (one at a time, about 12 s)"]
fn real_gpu_hands_4k60_frames_over_at_the_clip_rate() {
    let clip = std::env::var("VESTRA_GPU_CLIP")
        .unwrap_or_else(|_| r"C:\vestra\clips\hevc-2160p60-10bit.mp4".into());
    let gpu = Arc::new(Gpu::open(Choice::Default).expect("Direct3D 12"));
    assert!(!gpu.facts.software, "{} is not a GPU", gpu.facts.name);
    let mpv = player("auto-safe");
    mpv.set_property("loop-file", "inf").unwrap();
    let publish = Arc::new(AtomicBool::new(true));
    let renderer =
        Renderer::start_on(gpu.clone(), mpv.clone(), (3840, 2160), publish).expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", &clip]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    thread::sleep(Duration::from_secs(2));
    let (shared, mut view) = (handle.0.clone(), View::default());
    let (start, frames, shown) = (Instant::now(), handle.frames(), handle.shown());
    while start.elapsed() < Duration::from_secs(8) {
        let tick = Instant::now();
        let _ = view.before(&shared);
        gpu.queue.submit([]);
        view.after(&shared);
        thread::sleep(Duration::from_micros(16_667).saturating_sub(tick.elapsed()));
    }
    let seconds = start.elapsed().as_secs_f64();
    let produced = (handle.frames() - frames) as f64 / seconds;
    let taken = (handle.shown() - shown) as f64 / seconds;
    let hwdec: String = mpv.get_property("hwdec-current").unwrap_or_default();
    // The numbers (`--nocapture`); a closed stderr is ignored.
    let _ = writeln!(
        std::io::stderr(),
        "{}: produced {produced:.1}/s, new frames taken {taken:.1}/s, hwdec {hwdec}{}",
        gpu.facts.name,
        handle.log_text()
    );
    assert!(produced >= 58., "produced {produced:.1}/s");
    assert!(taken >= 40., "taken {taken:.1}/s");
    let (_, frame) = handle.latest().unwrap();
    let ready = &handle.0.fences.get().unwrap().ready;
    assert!(until(5, || ready.completed() >= frame.0.ready));
    let pixels = d3d::readback::wgpu(&gpu, &frame.0.texture);
    assert!(pixels.iter().any(|&byte| byte > 16), "the frame is black");
    drop(renderer);
}

/// Interpolation on, then off (Picture ▸ Effects ▸ Frames, 2026-11-10): the
/// frame rate after it is the rate before it. mpv takes a single
/// `mpv_render_context_report_swap` as the promise that every frame is
/// reported; a renderer that reported only while paced left mpv waiting
/// 200 ms for each later frame (5 frames a second) until the player quit.
#[test]
fn warp_interpolation_off_restores_the_frame_rate() {
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", &SOURCE.replace("d=30", "r=60:d=30")]).expect("loadfile");
    assert!(until(10, || handle.frames() > 10), "no frames");
    let rate = |what: &str| {
        thread::sleep(Duration::from_millis(500));
        let dropped = || mpv.get_property::<i64>("frame-drop-count").unwrap_or(0);
        let (start, frames, drops) = (Instant::now(), handle.frames(), dropped());
        thread::sleep(Duration::from_secs(2));
        let rate = (handle.frames() - frames) as f64 / start.elapsed().as_secs_f64();
        let drops = dropped() - drops;
        let _ = writeln!(std::io::stderr(), "{what}: {rate:.1} frames/s, {drops} dropped");
        (rate, drops)
    };
    let interpolate = |on: bool| {
        let set = |name: &str, value: &str| mpv.set_property(name, value).expect(name);
        set("interpolation", if on { "yes" } else { "no" });
        set("video-sync", if on { "display-resample" } else { "audio" });
        set("display-fps-override", if on { "60" } else { "0" });
        handle.set_display_fps(on.then_some(60.));
    };
    let (before, _) = rate("before");
    assert!(before >= 50., "{before:.1} frames/s before interpolation");
    interpolate(true);
    let (on, _) = rate("on");
    assert!(on >= 50., "{on:.1} frames/s with interpolation");
    interpolate(false);
    let (after, dropped) = rate("off");
    assert!(after >= before * 0.9, "{after:.1} frames/s after interpolation, {before:.1} before");
    assert!(dropped <= 2, "{dropped} frames dropped after interpolation");
    drop(renderer);
}

/// The same on the high-performance GPU (the one GPU rendering picks) with
/// a 4K60 clip (`VESTRA_GPU_CLIP`, default the Big Buck Bunny 2160p60 test
/// clip), a simulated window taking frames 60 times a second: the rates
/// and mpv's drop counters off, on and off again.
#[test]
#[ignore = "needs a GPU and a 4K60 clip; the coordinator runs it (one at a time, about 15 s)"]
fn real_gpu_interpolation_on_and_off_at_4k60() {
    let clip = std::env::var("VESTRA_GPU_CLIP")
        .unwrap_or_else(|_| r"C:\vestra\testmedia\bbb_sunflower_2160p_60fps_normal.mp4".into());
    let gpu = Arc::new(Gpu::open(Choice::HighPerformance).expect("Direct3D 12"));
    assert!(!gpu.facts.software, "{} is not a GPU", gpu.facts.name);
    let mpv = player("auto-safe");
    mpv.set_property("loop-file", "inf").unwrap();
    let renderer =
        Renderer::start_on(gpu.clone(), mpv.clone(), (3840, 2160), Arc::new(true.into()))
            .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", &clip]).expect("loadfile");
    assert!(until(10, || handle.frames() > 0), "no frame");
    let (shared, mut view) = (handle.0.clone(), View::default());
    let counter = |name: &str| mpv.get_property::<i64>(name).unwrap_or(0);
    let mut rate = |what: &str| {
        let (start, frames) = (Instant::now(), handle.frames());
        let drops = (counter("frame-drop-count"), counter("decoder-frame-drop-count"));
        let mistimed = counter("mistimed-frame-count");
        while start.elapsed() < Duration::from_secs(4) {
            let tick = Instant::now();
            let _ = view.before(&shared);
            gpu.queue.submit([]);
            view.after(&shared);
            thread::sleep(Duration::from_micros(16_667).saturating_sub(tick.elapsed()));
        }
        let rate = (handle.frames() - frames) as f64 / start.elapsed().as_secs_f64();
        let dropped = counter("frame-drop-count") - drops.0;
        let decoder = counter("decoder-frame-drop-count") - drops.1;
        let mistimed = counter("mistimed-frame-count") - mistimed;
        let _ = writeln!(
            std::io::stderr(),
            "{what}: {rate:.1} frames/s, dropped {dropped} (decoder {decoder}), mistimed {mistimed}"
        );
        (rate, dropped + decoder)
    };
    let interpolate = |on: bool| {
        let set = |name: &str, value: &str| mpv.set_property(name, value).expect(name);
        set("interpolation", if on { "yes" } else { "no" });
        set("video-sync", if on { "display-resample" } else { "audio" });
        set("display-fps-override", if on { "60" } else { "0" });
        handle.set_display_fps(on.then_some(60.));
    };
    thread::sleep(Duration::from_secs(2));
    let (before, _) = rate("before");
    interpolate(true);
    thread::sleep(Duration::from_secs(1));
    let (on, on_dropped) = rate("on");
    interpolate(false);
    thread::sleep(Duration::from_secs(1));
    let (after, dropped) = rate("off");
    let _ = writeln!(std::io::stderr(), "{}{}", gpu.facts.name, handle.log_text());
    assert!(before >= 58., "before {before:.1}/s");
    assert!(on >= 58. && on_dropped <= 4, "on {on:.1}/s, {on_dropped} dropped");
    assert!(after >= 58. && dropped <= 4, "off {after:.1}/s, {dropped} dropped");
    drop(renderer);
}

/// A render call that fails (no removed device) took mpv's frame all the
/// same: it is reported too, or mpv's flip waits 200 ms for each such frame.
#[test]
fn warp_a_failed_render_call_is_reported_too() {
    let mpv = player("no");
    let renderer = Renderer::start_on(warp_gpu(), mpv.clone(), (64, 64), Arc::new(true.into()))
        .expect("renderer");
    let handle = renderer.handle();
    mpv.command(&["loadfile", &SOURCE.replace("d=30", "r=60:d=30")]).expect("loadfile");
    assert!(until(10, || handle.frames() > 10), "no frames (and so reports)");
    let shared = &handle.0;
    shared.render_errors.store(true, Ordering::Release);
    thread::sleep(Duration::from_millis(500));
    let (start, calls) = (Instant::now(), shared.calls.load(Ordering::Acquire));
    thread::sleep(Duration::from_secs(2));
    let rate =
        (shared.calls.load(Ordering::Acquire) - calls) as f64 / 2. / start.elapsed().as_secs_f64();
    let _ = writeln!(std::io::stderr(), "failing render calls: {rate:.1}/s");
    assert!(rate >= 50., "{rate:.1} render calls a second while they fail");
    shared.render_errors.store(false, Ordering::Release);
    let frames = handle.frames();
    assert!(until(5, || handle.frames() > frames + 10), "no frames after the errors");
    assert!(handle.failed().is_none());
    drop(renderer);
}
