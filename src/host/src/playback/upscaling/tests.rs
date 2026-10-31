use super::*;
use crate::platform::{UpscalerGpu, UpscalerRefusal};
use crate::playback::render_mode::{Facts as RenderFacts, Preference, decide};

const RTX: u64 = 0x4070;
const IGPU: u64 = 0x780;

fn rtx() -> UpscalerProbe {
    Ok(vec![UpscalerGpu {
        id: RTX,
        name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
    }])
}

fn refused() -> UpscalerProbe {
    Err(UpscalerRefusal {
        code: "no-gpu",
        message: "no NVIDIA GPU".into(),
    })
}

/// GPU rendering on the RTX GPU, d3d11va decoding: everything holds.
fn facts<'a>(
    probe: &'a UpscalerProbe,
    reason: &'a RenderReason,
    rendering: &'a Value,
) -> Facts<'a> {
    Facts {
        probe,
        gpu_now: true,
        render_reason: reason,
        rendering,
        render_gpu: Some((RTX, "NVIDIA GeForce RTX 4070 Laptop GPU".into())),
        needed_hwdec: "d3d11va",
        hwdec: Some("auto-safe".into()),
        hwdec_current: Some("d3d11va".into()),
        os: "windows",
    }
}

fn geometry(target: (f64, f64), size: (f64, f64)) -> Geometry {
    Geometry {
        target,
        size,
        display: size,
    }
}

#[test]
fn the_factor_fills_the_area_and_only_upscales() {
    // 1080p in a maximised 2560×1440 window: ×1.33.
    assert_eq!(factor(geometry((2560., 1440.), (1920., 1080.))), Some(1.33));
    // 1080p on a 4K screen, full screen: ×2.
    assert_eq!(factor(geometry((3840., 2160.), (1920., 1080.))), Some(2.));
    // Letterboxed: a 2.39:1 film in a 16:9 area fits by width.
    assert_eq!(factor(geometry((3840., 2160.), (1920., 804.))), Some(2.));
    // Shown at its own size or smaller: nothing to do (scale=1 bypasses).
    assert_eq!(factor(geometry((1920., 1080.), (1920., 1080.))), None);
    assert_eq!(factor(geometry((1280., 720.), (3840., 2160.))), None);
    // A rounding to 1.00 is not an upscale.
    assert_eq!(factor(geometry((1922., 1081.), (1920., 1080.))), None);
    // Anamorphic DVD (720×576 shown as 1024×576) in a 1920×1080 area: the
    // larger axis, so neither side is short of pixels.
    let dvd = Geometry {
        target: (1920., 1080.),
        size: (720., 576.),
        display: (1024., 576.),
    };
    assert_eq!(factor(dvd), Some(2.67));
    // A tiny video on a huge area stops at the renderer's 8192 pixels.
    assert_eq!(factor(geometry((16000., 9000.), (160., 90.))), Some(51.2));
    // Nothing known: nothing.
    assert_eq!(factor(geometry((0., 0.), (1920., 1080.))), None);
    assert_eq!(factor(geometry((f64::NAN, 1080.), (1920., 1080.))), None);
}

#[test]
fn the_reasons_come_in_order_and_say_what_to_do() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = rtx();
    assert_eq!(check(&facts(&probe, &reason, &rendering)), Ok(()));

    // The system's check first, as the platform said it.
    let none = refused();
    let error = check(&facts(&none, &reason, &rendering)).unwrap_err();
    assert_eq!(
        (error.code(), error.message().as_str()),
        ("no-gpu", "no NVIDIA GPU")
    );
    assert!(error.fixed());

    // Software rendering: off by default (Windows), with the switch.
    let off = RenderReason::OffByDefault;
    let software =
        json!({"mode":"software","nextMode":"software","gpu":{"available":true,"reason":null}});
    let error = check(&Facts {
        gpu_now: false,
        ..facts(&probe, &off, &software)
    })
    .unwrap_err();
    assert_eq!(error.code(), "rendering");
    assert!(error.fixed());
    assert!(
        error
            .message()
            .contains("turn on Video ▸ GPU Rendering (Experimental) and restart the player"),
        "{}",
        error.message()
    );
    // Turned on, not restarted yet.
    let pending =
        json!({"mode":"software","nextMode":"gpu","gpu":{"available":true,"reason":null}});
    let error = check(&Facts {
        gpu_now: false,
        ..facts(&probe, &off, &pending)
    })
    .unwrap_err();
    assert!(
        error.message().contains("restart the player"),
        "{}",
        error.message()
    );
    assert!(!error.message().contains("turn on"), "{}", error.message());
    // GPU rendering cannot be chosen: the rendering reason is passed on.
    let denied = json!({"mode":"software","nextMode":"software","gpu":{"available":false,"reason":{"code":"denylist","message":"the graphics driver is on the player's denylist"}}});
    let error = check(&Facts {
        gpu_now: false,
        ..facts(&probe, &off, &denied)
    })
    .unwrap_err();
    assert!(error.message().contains("denylist"), "{}", error.message());
    // A fallback during the run.
    let fallback = RenderReason::WindowFallback("a window could not show GPU frames".into());
    let error = check(&Facts {
        gpu_now: false,
        ..facts(&probe, &fallback, &software)
    })
    .unwrap_err();
    assert!(
        error.message().contains("stopped during this run"),
        "{}",
        error.message()
    );
    // Starting: not fixed (a plugin activating at start may ask).
    let starting = RenderReason::Pending;
    let error = check(&Facts {
        gpu_now: false,
        ..facts(&probe, &starting, &rendering)
    })
    .unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("rendering", false));

    // Hybrid laptop: GPU rendering on the integrated GPU.
    let error = check(&Facts {
        render_gpu: Some((IGPU, "AMD Radeon 780M Graphics".into())),
        ..facts(&probe, &reason, &rendering)
    })
    .unwrap_err();
    assert_eq!(error.code(), "adapter");
    assert!(error.fixed());
    let message = error.message();
    assert!(message.contains("AMD Radeon 780M Graphics"), "{message}");
    assert!(message.contains("NVIDIA GeForce RTX 4070"), "{message}");
    assert!(
        message.contains("Windows Settings ▸ System ▸ Display ▸ Graphics"),
        "{message}"
    );

    // Copy-back (another plugin's filter needs frames in memory), off,
    // software or another decoder for this video: not fixed.
    for (hwdec, current, words) in [
        ("auto-copy-safe", None, "copy-back (auto-copy-safe)"),
        ("no", None, "hardware decoding is off"),
        ("auto-safe", Some("no"), "decoded in software"),
        (
            "auto-safe",
            Some("d3d11va-copy"),
            "decoded with d3d11va-copy",
        ),
    ] {
        let error = check(&Facts {
            hwdec: Some(hwdec.into()),
            hwdec_current: current.map(Into::into),
            ..facts(&probe, &reason, &rendering)
        })
        .unwrap_err();
        assert_eq!((error.code(), error.fixed()), ("decoding", false));
        assert!(error.message().contains(words), "{}", error.message());
        assert!(error.message().contains("(d3d11va)"), "{}", error.message());
    }
    // Before a video decodes, only the option counts.
    assert_eq!(
        check(&Facts {
            hwdec_current: None,
            ..facts(&probe, &reason, &rendering)
        }),
        Ok(())
    );
}

#[test]
fn reasons_have_the_api_shape() {
    let reason = Reason::Adapter("x".into());
    assert_eq!(reason.json(), json!({"code":"adapter","message":"x"}));
    assert_eq!(Reason::Starting.code(), "rendering");
}

// ------------------------------------------------------- with a real libmpv

/// One mpv at a time in this module (as processing.rs's tests).
fn one_mpv_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static MPV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    MPV.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mpv() -> Mpv {
    Mpv::builder()
        .unwrap()
        .set_property("vo", "null")
        .unwrap()
        .set_property("ao", "null")
        .unwrap()
        .set_property("vf", crate::playback::source_geometry::INITIAL_FILTERS)
        .unwrap()
        .build()
        .unwrap()
}

/// A GPU decision that has not started yet (`Pending`): what a plugin sees
/// while the player starts. No GPU is used.
fn starting() -> std::sync::Arc<Render> {
    Render::new(&decide(
        &RenderFacts {
            gpu_backend: true,
            ..RenderFacts::default()
        },
        Preference::Gpu,
    ))
}

/// Software rendering by choice.
fn software() -> std::sync::Arc<Render> {
    Render::new(&decide(
        &RenderFacts {
            gpu_backend: true,
            ..RenderFacts::default()
        },
        Preference::Software,
    ))
}

fn command(owner: u64, method: &str, params: Value) -> Command {
    Command {
        reply: Reply { owner, id: 1 },
        method: method.into(),
        params,
    }
}

/// The next reply, once the system's check (a thread) answered.
fn reply(
    upscaling: &mut Upscaling,
    mpv: &Mpv,
    render: &Render,
    events: &mpsc::Sender<Event>,
    received: &mpsc::Receiver<Event>,
) -> (u64, Result<Value, RpcError>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(event) = received.try_recv() {
            if let Event::Reply(reply, result) = event {
                return (reply.owner, result);
            }
        }
        assert!(Instant::now() < deadline, "no reply");
        upscaling.poll(mpv, render, events);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_filter_goes_last_replaces_in_place_and_comes_back_after_another_filter() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut upscaling = Upscaling::with_probe(rtx);
    assert!(upscaling.add(&mpv, 1.5));
    let chain = labels(&mpv);
    assert_eq!(chain.last().map(String::as_str), Some(LABEL), "{chain:?}");
    let filter = mpv.get_property::<String>("vf").unwrap();
    // mpv quotes the value (`scale=%4%1.50`).
    assert!(filter.contains("1.50:scaling-mode=nvidia"), "{filter}");
    // A new factor replaces the filter where it stands.
    assert!(upscaling.add(&mpv, 2.));
    assert_eq!(labels(&mpv).iter().filter(|l| *l == LABEL).count(), 1);
    assert!(
        mpv.get_property::<String>("vf")
            .unwrap()
            .contains("2.00:scaling-mode=nvidia")
    );
    // A plugin's first filter chain lands after it; it moves back.
    mpv.command(&["vf", "add", "@vestra-owned:lavfi=[null]"])
        .unwrap();
    assert_eq!(
        labels(&mpv).last().map(String::as_str),
        Some("vestra-owned")
    );
    upscaling.keep_last(&mpv, 2.);
    let chain = labels(&mpv);
    assert_eq!(
        &chain[chain.len() - 2..],
        ["vestra-owned", LABEL],
        "{chain:?}"
    );
    // Replacing the plugin's chain later keeps the order.
    mpv.command(&["vf", "add", "@vestra-owned:lavfi=[hflip]"])
        .unwrap();
    assert_eq!(labels(&mpv).last().map(String::as_str), Some(LABEL));
    // Removed, and the orientation filters stay.
    upscaling.remove(&mpv);
    let chain = labels(&mpv);
    assert!(!chain.iter().any(|l| l == LABEL), "{chain:?}");
    assert!(chain.iter().any(|l| l == "vestra-orientation"), "{chain:?}");
    assert_eq!(upscaling.applied, None);
}

#[test]
fn a_new_size_waits_until_it_settles() {
    let mut upscaling = Upscaling::with_probe(rtx);
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    // The first filter at once.
    assert_eq!(upscaling.step(Some(1.5), at(0)), Step::Add(1.5));
    upscaling.applied = Some(1.5);
    // A border drag: every new factor restarts the wait.
    assert_eq!(upscaling.step(Some(1.7), at(10)), Step::Stay);
    assert_eq!(upscaling.step(Some(1.8), at(100)), Step::Stay);
    assert_eq!(upscaling.step(Some(1.8), at(300)), Step::Stay);
    assert_eq!(upscaling.step(Some(1.8), at(400)), Step::Add(1.8));
    upscaling.applied = Some(1.8);
    upscaling.wanted = None;
    // Within 2 %: mpv's scaler covers it; only the order is checked.
    assert_eq!(upscaling.step(Some(1.82), at(500)), Step::KeepLast(1.8));
    // Too small to upscale, upscaling turned off or unavailable: at once.
    assert_eq!(upscaling.step(None, at(510)), Step::Remove);
    upscaling.applied = None;
    assert_eq!(upscaling.step(None, at(520)), Step::Stay);
    // A factor mpv refused is not tried again until the factor changes.
    upscaling.refused = Some(2.);
    assert_eq!(upscaling.step(Some(2.), at(600)), Step::Stay);
    assert_eq!(upscaling.step(Some(2.5), at(610)), Step::Add(2.5));
}

#[test]
fn unavailable_upscaling_takes_the_filter_out_at_once() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let render = starting();
    let mut upscaling = Upscaling::with_probe(rtx);
    assert!(upscaling.add(&mpv, 1.5));
    upscaling.owners.insert(1);
    upscaling.probe = Probe::Done(rtx());
    // Not available while GPU rendering starts.
    upscaling.update(&mpv, &render, Instant::now());
    assert_eq!(upscaling.applied, None);
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
}

#[test]
fn requests_wait_for_the_check_and_set_is_refused_where_it_cannot_work() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let (events, received) = mpsc::channel();
    let rendering =
        json!({"mode":"software","nextMode":"software","gpu":{"available":true,"reason":null}});

    // Software rendering: the state says why; set(true) is refused.
    let render = software();
    let mut upscaling = Upscaling::with_probe(rtx);
    let state = command(1, "video.upscaling.state", json!({"rendering":rendering}));
    assert!(upscaling.command(&mpv, &render, &state, &events));
    let (owner, state) = reply(&mut upscaling, &mpv, &render, &events, &received);
    let state = state.unwrap();
    assert_eq!(owner, 1);
    assert_eq!(state["available"], false);
    assert_eq!(state["reason"]["code"], "rendering");
    assert_eq!(
        (state["enabled"].clone(), state["requested"].clone()),
        (json!(false), json!(false))
    );
    assert_eq!(state["factor"], Value::Null);
    let set = command(
        1,
        "video.upscaling.set",
        json!({"enabled":true,"rendering":rendering}),
    );
    assert!(upscaling.command(&mpv, &render, &set, &events));
    let error = reply(&mut upscaling, &mpv, &render, &events, &received)
        .1
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(
        error.message.contains("GPU Rendering (Experimental)"),
        "{}",
        error.message
    );
    assert!(upscaling.owners.is_empty());
    // Bad input.
    let bad = command(1, "video.upscaling.set", json!({"enabled":"yes"}));
    assert!(upscaling.command(&mpv, &render, &bad, &events));
    assert_eq!(
        reply(&mut upscaling, &mpv, &render, &events, &received)
            .1
            .unwrap_err()
            .code,
        "invalid"
    );

    // No GPU that can upscale: refused with the platform's reason.
    let mut upscaling = Upscaling::with_probe(refused);
    assert!(upscaling.command(&mpv, &render, &set, &events));
    let error = reply(&mut upscaling, &mpv, &render, &events, &received)
        .1
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(error.message.contains("no NVIDIA GPU"), "{}", error.message);

    // While GPU rendering starts the request is kept (not fixed) and the
    // state says it is enabled but nothing is requested from the driver.
    let render = starting();
    let mut upscaling = Upscaling::with_probe(rtx);
    assert!(upscaling.command(&mpv, &render, &set, &events));
    let state = reply(&mut upscaling, &mpv, &render, &events, &received)
        .1
        .unwrap();
    assert_eq!(state["enabled"], true);
    assert_eq!(state["requested"], false);
    assert_eq!(state["reason"]["code"], "rendering");
    // Another plugin's state: not enabled by it.
    let other = command(2, "video.upscaling.state", json!({}));
    assert!(upscaling.command(&mpv, &render, &other, &events));
    let (owner, state) = reply(&mut upscaling, &mpv, &render, &events, &received);
    assert_eq!(
        (owner, state.unwrap()["enabled"].clone()),
        (2, json!(false))
    );
    // The plugin stops: its request goes; the worker's own handling goes on.
    let stopped = command(1, "owner.disabled", Value::Null);
    assert!(!upscaling.command(&mpv, &render, &stopped, &events));
    assert!(upscaling.owners.is_empty());
    // Other methods are not this module's.
    assert!(!upscaling.command(
        &mpv,
        &render,
        &command(1, "processing.state", json!({})),
        &events
    ));
}

/// `video-params` (what the factor divides by) is the decoder's output,
/// before the filter chain: the upscaler's own output never feeds back
/// into its factor.
#[test]
fn video_params_are_before_the_filter_chain() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    mpv.command(&["vf", "add", &format!("@{LABEL}:lavfi=[scale=640:480]")])
        .unwrap();
    mpv.set_property("pause", true).unwrap();
    mpv.command(&["loadfile", "av://lavfi:testsrc=size=320x240:rate=10"])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let out = loop {
        if let Ok(width) = mpv.get_property::<i64>("video-out-params/w") {
            break width;
        }
        assert!(Instant::now() < deadline, "no video");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(out, 640);
    assert_eq!(mpv.get_property::<i64>("video-params/w").unwrap(), 320);
    assert_eq!(mpv.get_property::<i64>("video-params/h").unwrap(), 240);
}

#[test]
fn a_later_set_false_wins_over_an_earlier_set_true_waiting_for_the_check() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let render = starting();
    let (events, received) = mpsc::channel();
    let mut upscaling = Upscaling::with_probe(rtx);
    let on = command(1, "video.upscaling.set", json!({"enabled":true}));
    let off = command(1, "video.upscaling.set", json!({"enabled":false}));
    // Both arrive before the check answers (it runs on its own thread).
    assert!(upscaling.command(&mpv, &render, &on, &events));
    assert!(upscaling.command(&mpv, &render, &off, &events));
    for _ in 0..2 {
        let state = reply(&mut upscaling, &mpv, &render, &events, &received).1;
        assert_eq!(state.unwrap()["enabled"], false);
    }
    assert!(upscaling.owners.is_empty());
}

#[test]
fn a_stopped_plugin_revokes_the_filter_and_set_false_removes_it_at_once() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let render = starting();
    let (events, _received) = mpsc::channel();
    let mut upscaling = Upscaling::with_probe(rtx);
    upscaling.probe = Probe::Done(rtx());
    // Two plugins asked; the filter is in the chain.
    upscaling.owners.extend([1, 2]);
    assert!(upscaling.add(&mpv, 1.5));
    // One stops: the other still wants it (the filter stays as long as the
    // check holds; here GPU rendering is only starting, so `update` would
    // take it out, which is what the release path must not depend on).
    upscaling.owners.remove(&2);
    upscaling.release(&mpv, &render, 1);
    assert!(upscaling.owners.is_empty());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL), "revoked");
    assert_eq!(upscaling.applied, None);
    // set(false) removes at once, before any answer.
    upscaling.owners.insert(3);
    assert!(upscaling.add(&mpv, 2.));
    let off = command(3, "video.upscaling.set", json!({"enabled":false}));
    assert!(upscaling.command(&mpv, &render, &off, &events));
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
}

#[test]
fn the_worker_may_wait_until_the_next_look_or_the_end_of_a_settle() {
    let mut upscaling = Upscaling::with_probe(rtx);
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    // Nobody asked: no deadline (the worker waits for its own reasons).
    assert_eq!(upscaling.due_in(start), None);
    upscaling.owners.insert(1);
    assert_eq!(
        upscaling.due_in(start),
        Some(Duration::ZERO),
        "never looked"
    );
    upscaling.checked = Some(start);
    assert_eq!(upscaling.due_in(at(50)), Some(Duration::from_millis(150)));
    // A new size seen at 120 ms settles at 420 ms; the next look (200 ms)
    // comes first, then the settle.
    upscaling.wanted = Some((1.8, at(120)));
    assert_eq!(upscaling.due_in(at(150)), Some(Duration::from_millis(50)));
    upscaling.checked = Some(at(200));
    assert_eq!(upscaling.due_in(at(300)), Some(Duration::from_millis(100)));
    assert_eq!(upscaling.due_in(at(500)), Some(Duration::ZERO));
    // Turned off with the filter still in the chain: it must be removed.
    upscaling.owners.clear();
    upscaling.wanted = None;
    upscaling.applied = Some(1.5);
    assert!(upscaling.due_in(at(500)).is_some());
}

#[test]
fn the_check_wakes_the_worker_when_it_answers() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WOKEN: AtomicBool = AtomicBool::new(false);
    let mut upscaling = Upscaling::with_probe(rtx);
    upscaling.wake_with(|| WOKEN.store(true, Ordering::Release));
    upscaling.start_probe();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !WOKEN.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "not woken");
        std::thread::sleep(Duration::from_millis(5));
    }
}
