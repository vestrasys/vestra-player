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
    assert!(view.direct(shared).is_some(), "the newest frame is shown");
    let first = view.claim.expect("held");
    let before = handle.frames();
    assert!(until(10, || handle.frames() > before), "no newer frame");
    assert!(view.direct(shared).is_some(), "the newer frame is shown");
    assert!(view.claim.is_some_and(|slot| slot != first || slot.generation != first.generation));
    let signalled = shared.signalled.load(Ordering::Relaxed);
    assert!(signalled > 0, "the old texture got its done signal");
    let fences = shared.fences.get().unwrap();
    assert!(until(5, || fences.done.completed() >= signalled));
    assert!(view.direct(shared).is_none() || handle.frames() > before + 1);
    view.after(shared);
    drop(renderer);
}
#[test]
fn the_adapter_choice_reads_the_environment_value() {
    assert_eq!(Choice::parse(None), Choice::Default);
    assert_eq!(Choice::parse(Some("")), Choice::Default);
    assert_eq!(Choice::parse(Some("warp")), Choice::Warp);
    assert_eq!(Choice::parse(Some(" 1 ")), Choice::Index(1));
    assert_eq!(Choice::parse(Some("dgpu")), Choice::Default);
}

// ------------------------------------------------- real-machine measurements

/// The spike's measurement (round record 2026-10-wgpu-d3d-spike §3) on the
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
    // The numbers for the record (`--nocapture`); a closed stderr is ignored.
    let _ = writeln!(
        std::io::stderr(),
        "{}: produced {produced:.1}/s, new frames taken {taken:.1}/s, hwdec {hwdec}{}",
        gpu.facts.name,
        handle.log_text()
    );
    assert!(produced >= 58., "produced {produced:.1}/s");
    assert!(taken >= 40., "taken {taken:.1}/s");
    let (_, frame) = handle.latest().unwrap();
    let pixels = d3d::readback::wgpu(&gpu, &frame.0.texture);
    assert!(pixels.iter().any(|&byte| byte > 16), "the frame is black");
    drop(renderer);
}
