use super::*;
use crate::host::Reply;
use crate::platform::{MotionGpu, MotionRefusal};
use crate::playback::render_mode::{Facts as RenderFacts, Preference, decide};

fn amd() -> MotionProbe {
    Ok(MotionGpu {
        name: "AMD Radeon 780M Graphics".into(),
    })
}

fn no_amd() -> MotionProbe {
    Err(MotionRefusal {
        code: "no-gpu",
        message: "no AMD GPU".into(),
    })
}

/// GPU rendering, a 24 fps video on a 60 Hz display, nothing in the way.
fn facts<'a>(probe: &'a MotionProbe, reason: &'a RenderReason, rendering: &'a Value) -> Facts<'a> {
    Facts {
        engine: Engine::Amd,
        probe,
        has_filter: true,
        other: None,
        refused: None,
        gpu_now: true,
        render_reason: reason,
        rendering,
        upscaling: false,
        failed: None,
        overload: None,
        source_fps: Some(24.),
        display_fps: 60.,
        any_rate: false,
        any_size: false,
        video: Some((1920., 1080.)),
        doubling: false,
        broken: None,
        format: None,
        learned: None,
    }
}

#[test]
fn the_reasons_come_in_order_and_say_what_to_do() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    for mode in [Mode::Off, Mode::Double, Mode::Display] {
        assert_eq!(check(&facts(&probe, &reason, &rendering), mode), Ok(()));
    }
    // The system's check first; Off always works.
    let none = no_amd();
    let error = check(&facts(&none, &reason, &rendering), Mode::Double).unwrap_err();
    assert_eq!(
        (error.code(), error.message().as_str()),
        ("no-gpu", "no AMD GPU")
    );
    assert!(error.fixed());
    assert_eq!(check(&facts(&none, &reason, &rendering), Mode::Off), Ok(()));
    // The filter mpv refused.
    let error = check(
        &Facts {
            refused: Some("no such filter"),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "filter");
    assert!(error.fixed());
    // Match Display needs GPU rendering; Double does not.
    let off = RenderReason::OffByDefault;
    let software = json!({"engine":"amd","mode":"software","nextMode":"software","gpu":{"available":true,"reason":null}});
    let cpu = Facts {
        gpu_now: false,
        source_fps: Some(30.),
        ..facts(&probe, &off, &software)
    };
    let error = check(&cpu, Mode::Display).unwrap_err();
    assert_eq!(error.code(), "rendering");
    assert!(error.fixed());
    assert!(
        error
            .message()
            .contains("turn on GPU Rendering in Picture ▸ Rendering and restart the player"),
        "{}",
        error.message()
    );
    assert_eq!(check(&cpu, Mode::Double), Ok(()));
    // Starting: not fixed.
    let pending = RenderReason::Pending;
    let error = check(
        &Facts {
            gpu_now: false,
            ..facts(&probe, &pending, &rendering)
        },
        Mode::Display,
    )
    .unwrap_err();
    assert_eq!(error, Reason::Starting);
    assert!(!error.fixed());
    // AI upscaling, then this video's failure and overload: none fixed.
    let error = check(
        &Facts {
            upscaling: true,
            failed: Some("AMFFRC->Init() failed with error 4"),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "upscaling");
    assert!(!error.fixed());
    let error = check(
        &Facts {
            failed: Some("AMFFRC->Init() failed with error 4"),
            overload: Some("behind"),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "failed");
    assert!(error.message().contains("error 4"), "{}", error.message());
    let error = check(
        &Facts {
            overload: Some("behind"),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Display,
    )
    .unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("overload", false));
    assert_eq!(
        error.json(),
        json!({"code":"overload","message":"behind"}),
        "the API shape"
    );
}

#[test]
fn the_rate_must_fit_the_display_and_software_rendering_needs_twice_the_rate() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    let at = |fps: f64, display: f64, gpu: bool| Facts {
        source_fps: Some(fps),
        display_fps: display,
        gpu_now: gpu,
        // The size rule apart (its own test).
        video: None,
        ..facts(&probe, &reason, &rendering)
    };
    // GPU rendering: any double up to the display.
    assert_eq!(check(&at(23.976, 60., true), Mode::Double), Ok(()));
    assert_eq!(check(&at(30., 60., true), Mode::Display), Ok(()));
    assert_eq!(check(&at(30., 144., true), Mode::Display), Ok(()));
    // Past the display: nothing to gain.
    let error = check(&at(30., 50., true), Mode::Double).unwrap_err();
    assert_eq!(error.code(), "rate");
    assert_eq!(
        error.message(),
        "this video plays at 30 fps; doubled it would be 60 fps, more than the display's 50 Hz"
    );
    // Out of the converter's range.
    // Above 30 fps: not measured, not offered (1080p60 on 120 Hz dropped 2–3 %).
    let error = check(&at(60., 144., true), Mode::Double).unwrap_err();
    assert!(
        error.message().contains("10 to 30 fps"),
        "{}",
        error.message()
    );
    assert!(check(&at(5., 60., true), Mode::Double).is_err());
    // Software rendering: Double only where twice the rate is the display's
    // (within 1 %).
    assert_eq!(check(&at(30., 60., false), Mode::Double), Ok(()));
    assert_eq!(check(&at(29.97, 60., false), Mode::Double), Ok(()));
    assert_eq!(check(&at(30., 59.94, false), Mode::Double), Ok(()));
    let error = check(&at(24., 60., false), Mode::Double).unwrap_err();
    assert_eq!(error.code(), "rate");
    assert!(
        error
            .message()
            .contains("24 fps doubled is 48 fps on a 60 Hz display"),
        "{}",
        error.message()
    );
    // The test switch lifts the display rule, not the range.
    let any = |fps: f64| Facts {
        any_rate: true,
        ..at(fps, 60., false)
    };
    assert_eq!(check(&any(24.), Mode::Double), Ok(()));
    assert!(check(&any(120.), Mode::Double).is_err());
    // An unknown rate is not judged yet.
    let unknown = Facts {
        source_fps: None,
        ..at(24., 60., false)
    };
    assert_eq!(check(&unknown, Mode::Double), Ok(()));
}

#[test]
fn a_window_is_behind_when_slow_or_dropping() {
    // 48 fps for 4 s with no drops: fine.
    assert_eq!(judge(48., &[48., 47.9, 48.1], 0, 4.), None);
    // Up to 5 % dropped: fine (9.6 of 192).
    assert_eq!(judge(48., &[48.], 9, 4.), None);
    // More: behind.
    assert_eq!(
        judge(48., &[48.], 25, 4.).as_deref(),
        Some("48 of 48 frames a second, 25 frames dropped in 4 s")
    );
    // Below 90 % of the doubled rate: behind.
    assert!(judge(48., &[40., 41.], 0, 4.).is_some());
    // Nothing measured: no verdict.
    assert_eq!(judge(48., &[], 100, 4.), None);
}

#[test]
fn modes_parse_and_the_strongest_applies() {
    for mode in [Mode::Off, Mode::Double, Mode::Display] {
        assert_eq!(Mode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(Mode::parse("triple"), None);
    let mut motion = Motion::with_probe(amd);
    assert_eq!(motion.wanted(), Mode::Off);
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.owners.insert(2, (Engine::Amd, Mode::Display));
    // The holder is settled at each update (`refresh_holder`).
    motion.refresh_holder();
    assert_eq!(motion.wanted(), Mode::Display);
    motion.owners.remove(&2);
    assert_eq!(motion.wanted(), Mode::Double);
    // Another engine's request does not count while AMD's holds it.
    motion.owners.insert(3, (Engine::Nvidia, Mode::Display));
    motion.refresh_holder();
    assert_eq!(motion.wanted(), Mode::Double);
}

#[test]
fn the_worker_waits_only_while_motion_smoothing_is_asked_for() {
    let mut motion = Motion::with_probe(amd);
    let start = Instant::now();
    assert_eq!(motion.due_in(start), None);
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    assert_eq!(motion.due_in(start), Some(Duration::ZERO));
    motion.checked = Some(start);
    assert_eq!(
        motion.due_in(start + Duration::from_millis(200)),
        Some(Duration::from_millis(300))
    );
}

// ------------------------------------------------------- with a real libmpv

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

/// Software rendering by choice: Double only at half the display's rate
/// (60 Hz when unknown).
fn software() -> std::sync::Arc<Render> {
    Render::new(&decide(
        &RenderFacts {
            gpu_backend: true,
            ..RenderFacts::default()
        },
        Preference::Software,
    ))
}

/// A paused test video at `rate`, once mpv knows its rate.
/// A test video at `rate`, played until mpv estimates the chain's output
/// rate (the converter's input, `Motion::input`), then paused.
fn play(mpv: &Mpv, rate: u32) {
    mpv.command(&[
        "loadfile",
        &format!("av://lavfi:testsrc=size=320x240:rate={rate}"),
    ])
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while source_fps(mpv).is_none()
        || mpv
            .get_property::<f64>("estimated-vf-fps")
            .map_or(true, |fps| (fps - f64::from(rate)).abs() > 1.)
    {
        assert!(Instant::now() < deadline, "no rate");
        std::thread::sleep(Duration::from_millis(20));
    }
    mpv.set_property("pause", true).unwrap();
}

fn command(owner: u64, method: &str, params: Value) -> Command {
    Command {
        reply: Reply { owner, id: 1 },
        method: method.into(),
        params,
    }
}

/// What the backend would have done.
#[derive(Debug, Default)]
struct Recorded {
    held: Vec<bool>,
    resample: Vec<Option<f64>>,
}
impl Effects for Recorded {
    fn frames_in_memory(&mut self, held: bool) {
        self.held.push(held);
    }
    fn resample(&mut self, refresh: Option<f64>) {
        if self.resample.last() != Some(&refresh) {
            self.resample.push(refresh);
        }
    }
}

/// The platform's filter is Windows only; the chain handling is the same
/// for any filter, so these tests put in one every mpv build accepts.
fn stand_in(_engine: Engine, large: bool, performance: bool) -> Option<String> {
    Some(format!(
        "lavfi=[scale=w=iw{}:h=ih{}]",
        if performance { "/2*2" } else { "" },
        if large { "/2*2" } else { "" }
    ))
}

fn with_stand_in() -> Motion {
    let mut motion = Motion::with_probe(amd);
    motion.filter = stand_in;
    motion.any_rate = false;
    // The test videos play at 30 fps; measured as the player measures it
    // where a test needs that (`input` None there).
    motion.input = Some(30.);
    motion.generation = Some(1);
    motion
}

fn reply(received: &mpsc::Receiver<Event>) -> Result<Value, RpcError> {
    while let Ok(event) = received.try_recv() {
        if let Event::Reply(_, result) = event {
            return result;
        }
    }
    panic!("no reply");
}

#[test]
fn mpv_takes_the_platform_filter() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let mut motion = Motion::with_probe(amd);
    let mut effects = Recorded::default();
    let applied = Applied {
        engine: Engine::Amd,
        mode: Mode::Double,
        large: false,
        performance: false,
    };
    let added = motion.add(&mpv, &mut effects, applied);
    if cfg!(target_os = "windows") {
        // mpv accepts the text; the graph is built (and AMF asked) only
        // when a video plays.
        assert!(added);
        let filter = mpv.get_property::<String>("vf").unwrap();
        assert!(filter.contains("frc_amf"), "{filter}");
        assert_eq!(labels(&mpv).last().map(String::as_str), Some(LABEL));
    } else {
        assert!(!added);
        assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    }
}

#[test]
fn double_goes_in_last_with_frames_in_memory_and_off_takes_it_out() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut motion = with_stand_in();
    let mut effects = Recorded::default();
    let on = command(
        1,
        "video.motion.set",
        json!({"engine":"amd","mode":"double"}),
    );
    assert!(motion.command(&mpv, &render, &on, &events, &mut effects));
    let state = reply(&received).unwrap();
    assert_eq!(state["mode"], "double", "{state}");
    assert_eq!(state["active"], "double", "{state}");
    assert_eq!(state["double"], Value::Null, "{state}");
    assert_eq!(state["display"]["code"], "rendering", "{state}");
    assert_eq!(state["sourceFps"], 30., "{state}");
    assert_eq!(state["gpu"], "AMD Radeon 780M Graphics", "{state}");
    assert_eq!(state["inChain"], true, "{state}");
    assert_eq!(labels(&mpv).last().map(String::as_str), Some(LABEL));
    assert_eq!(effects.held, [true]);
    assert!(effects.resample.is_empty(), "Double asks for no resampling");
    // A plugin's first chain lands after it; the next look moves it back.
    mpv.command(&["vf", "add", "@vestra-owned:lavfi=[null]"])
        .unwrap();
    motion.checked = None;
    motion.poll(&mpv, &render, &events, &mut effects, 1);
    let chain = labels(&mpv);
    assert_eq!(
        &chain[chain.len() - 2..],
        ["vestra-owned", LABEL],
        "{chain:?}"
    );
    // Off: out, and the frames go back to the GPU.
    let off = command(1, "video.motion.set", json!({"engine":"amd","mode":"off"}));
    assert!(motion.command(&mpv, &render, &off, &events, &mut effects));
    while received.try_recv().is_ok() {}
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    assert_eq!(effects.held, [true, false]);
    assert_eq!(motion.due_in(Instant::now()), None);
}

#[test]
fn set_is_refused_where_it_cannot_work_and_waits_where_it_may() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 24);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut effects = Recorded::default();
    // Match Display under software rendering: refused for this run.
    let mut motion = with_stand_in();
    let display = command(
        1,
        "video.motion.set",
        json!({"engine":"amd","mode":"display"}),
    );
    motion.command(&mpv, &render, &display, &events, &mut effects);
    let error = reply(&received).unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(error.message.contains("GPU rendering"), "{}", error.message);
    // Double at 24 fps on 60 Hz in software: kept, waiting for a rate that
    // fits (the next video), the reason in the state.
    let double = command(
        1,
        "video.motion.set",
        json!({"engine":"amd","mode":"double"}),
    );
    motion.command(&mpv, &render, &double, &events, &mut effects);
    let state = reply(&received).unwrap();
    assert_eq!(
        (state["mode"].as_str(), state["active"].as_str()),
        (Some("double"), Some("off"))
    );
    assert_eq!(state["double"]["code"], "rate", "{state}");
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    // No AMD GPU: refused.
    let mut none = Motion::with_probe(no_amd);
    none.command(&mpv, &render, &double, &events, &mut effects);
    assert_eq!(reply(&received).unwrap_err().code, "unsupported");
    // An unknown mode: invalid.
    let triple = command(
        1,
        "video.motion.set",
        json!({"engine":"amd","mode":"triple"}),
    );
    motion.command(&mpv, &render, &triple, &events, &mut effects);
    assert_eq!(reply(&received).unwrap_err().code, "invalid");
}

#[test]
fn a_disabled_filter_goes_for_this_video_and_comes_back_with_the_next() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.poll(&mpv, &render, &events, &mut effects, 1);
    let on = command(
        1,
        "video.motion.set",
        json!({"engine":"amd","mode":"double"}),
    );
    motion.command(&mpv, &render, &on, &events, &mut effects);
    reply(&received).unwrap();
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    // mpv's log as the converter fails on Direct3D 11 frames.
    motion.log("ffmpeg", "error", "AMFFRC->Init() failed with error 4\n");
    motion.log(
        "vf",
        "error",
        "Disabling filter vestra-motion because it has failed.\n",
    );
    motion.poll(&mpv, &render, &events, &mut effects, 1);
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    let state = motion.state(&mpv, &render, 1, Engine::Amd, &Value::Null);
    assert_eq!(state["double"]["code"], "failed", "{state}");
    assert!(
        state["double"]["message"]
            .as_str()
            .unwrap()
            .contains("AMFFRC->Init() failed with error 4"),
        "{state}"
    );
    assert_eq!(state["mode"], "double");
    // The change was published.
    let published = received
        .try_iter()
        .filter_map(|event| match event {
            Event::Broadcast(name, data) if name == CHANGED => Some(data),
            _ => None,
        })
        .last()
        .unwrap();
    assert_eq!(published["active"], "off");
    assert!(published.get("mode").is_none(), "each plugin's own");
    // Another video: tried again.
    motion.generation = Some(2);
    motion.new_video();
    motion.input = Some(30.);
    motion.poll(&mpv, &render, &events, &mut effects, 2);
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
}

#[test]
fn falling_behind_steps_to_the_faster_search_then_off() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    let first = mpv.get_property::<String>("vf").unwrap();
    // A verdict (as `watch` gives it after a window behind): the faster
    // search, rebuilt at the next look.
    motion.performance = true;
    motion.update(&mpv, &render, &mut effects, Instant::now());
    let faster = mpv.get_property::<String>("vf").unwrap();
    assert_ne!(first, faster);
    assert!(faster.contains("/2*2"), "{faster}");
    assert_eq!(motion.applied.map(|a| a.performance), Some(true));
    // Behind again: off for this video, with the reason.
    motion.overload = Some("the GPU could not keep up".into());
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    let state = motion.state(&mpv, &render, 1, Engine::Amd, &Value::Null);
    assert_eq!(state["double"]["code"], "overload");
    // The next video starts with the full search again.
    motion.new_video();
    motion.input = Some(30.);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(motion.applied.map(|a| a.performance), Some(false));
}

#[test]
fn upscaling_and_a_stopped_plugin_take_the_filter_out() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, _received) = mpsc::channel();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    // AI upscaling turned on: motion smoothing waits.
    motion.note_upscaling(true);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    let state = motion.state(&mpv, &render, 1, Engine::Amd, &Value::Null);
    assert_eq!(state["double"]["code"], "upscaling");
    motion.note_upscaling(false);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    // The plugin stops: its request and the filter go.
    let stopped = command(1, "owner.disabled", json!({}));
    assert!(!motion.command(&mpv, &render, &stopped, &events, &mut effects));
    assert!(motion.owners.is_empty());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    assert_eq!(effects.held.last(), Some(&false));
}

/// The run's first AI Upscaling request waits for the upscaler's check.
/// Once the check answered, the converter leaves (copy-back released)
/// before the answer, as for a later request; a request that will be
/// refused (no NVIDIA GPU), or one a waiting `set(false)` cancelled,
/// leaves it in the chain. Review follow-up 2026-11-12.
#[test]
fn the_first_upscaling_request_takes_the_converter_out_before_its_answer() {
    use crate::platform::{UpscalerGpu, UpscalerProbe, UpscalerRefusal};
    use crate::playback::upscaling::Upscaling;
    fn rtx() -> UpscalerProbe {
        Ok(vec![UpscalerGpu {
            id: 0x4070,
            name: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
        }])
    }
    fn no_nvidia() -> UpscalerProbe {
        Err(UpscalerRefusal {
            code: "no-gpu",
            message: "no NVIDIA GPU".into(),
        })
    }
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    // GPU rendering still starting: upscaling waits for it (not refused),
    // motion smoothing's Double does not need it.
    let render = Render::new(&decide(
        &RenderFacts {
            gpu_backend: true,
            ..RenderFacts::default()
        },
        Preference::Gpu,
    ));
    let in_chain = |mpv: &Mpv| labels(mpv).iter().any(|l| l == LABEL);
    let on = command(
        1,
        "video.upscaling.set",
        json!({"enabled":true,"rendering":Value::Null}),
    );
    let off = command(
        1,
        "video.upscaling.set",
        json!({"enabled":false,"rendering":Value::Null}),
    );
    // The worker's order (`backend.rs`): take the check, hand over, answer.
    // Returns the answers to `set`, and whether the converter was in the
    // chain when they were sent.
    let run = |upscaling: &mut Upscaling,
               motion: &mut Motion,
               effects: &mut Recorded,
               commands: &[&Command]|
     -> Vec<(Result<Value, RpcError>, bool)> {
        let (events, received) = mpsc::channel();
        assert!(!upscaling.would_accept(&mpv, &render), "before the check");
        for command in commands {
            assert!(upscaling.command(&mpv, &render, command, &events));
        }
        let mut answers = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while answers.len() < commands.len() {
            assert!(Instant::now() < deadline, "no answer");
            upscaling.take_check();
            motion.hand_over(upscaling, &mpv, &render, effects);
            let was_in = in_chain(&mpv);
            upscaling.poll(&mpv, &render, &events, Vec::new);
            while let Ok(event) = received.try_recv() {
                if let Event::Reply(_, result) = event {
                    answers.push((result, was_in));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        answers
    };
    let start = |effects: &mut Recorded| {
        let mut motion = with_stand_in();
        motion.probe[Engine::Amd] = Some(amd());
        motion.owners.insert(1, (Engine::Amd, Mode::Double));
        motion.update(&mpv, &render, effects, Instant::now());
        assert!(in_chain(&mpv));
        assert_eq!(effects.held, [true]);
        motion
    };

    // Accepted: out of the chain and copy-back released before the answer.
    let mut effects = Recorded::default();
    let mut motion = start(&mut effects);
    let mut upscaling = Upscaling::with_probe(rtx);
    let answers = run(&mut upscaling, &mut motion, &mut effects, &[&on]);
    let (state, was_in) = &answers[0];
    assert_eq!(state.as_ref().unwrap()["enabled"], true);
    assert!(!was_in, "the converter left before the answer");
    assert_eq!(effects.held, [true, false]);
    motion.note_upscaling(upscaling.asked());
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!in_chain(&mpv));
    // Off again: the converter comes back.
    motion.note_upscaling(false);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(in_chain(&mpv));
    mpv.command(&["vf", "remove", &format!("@{LABEL}")])
        .unwrap();

    // Refused: the converter stays.
    let mut effects = Recorded::default();
    let mut motion = start(&mut effects);
    let mut upscaling = Upscaling::with_probe(no_nvidia);
    let answers = run(&mut upscaling, &mut motion, &mut effects, &[&on]);
    let (error, was_in) = &answers[0];
    assert_eq!(error.as_ref().unwrap_err().code, "unsupported");
    assert!(was_in);
    assert!(in_chain(&mpv));
    assert_eq!(effects.held, [true]);
    mpv.command(&["vf", "remove", &format!("@{LABEL}")])
        .unwrap();

    // Cancelled by a set(false) that waited too: the converter stays.
    let mut effects = Recorded::default();
    let mut motion = start(&mut effects);
    let mut upscaling = Upscaling::with_probe(rtx);
    let answers = run(&mut upscaling, &mut motion, &mut effects, &[&on, &off]);
    assert!(
        answers
            .iter()
            .all(|(state, was_in)| { *was_in && state.as_ref().unwrap()["enabled"] == false })
    );
    assert!(in_chain(&mpv));
    assert_eq!(effects.held, [true]);
}

/// The filter waits for frames in memory: with a hardware decoding mode
/// that is not copy-back, the hold is asked for and the filter stays out.
#[test]
fn the_filter_waits_until_the_frames_are_in_memory() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    mpv.set_property("hwdec", "d3d11va,videotoolbox,vaapi")
        .unwrap();
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(effects.held, [true], "copy-back asked for first");
    assert!(!labels(&mpv).iter().any(|l| l == LABEL), "not before");
    // Software decoding (`no`: frames in memory), once the decoder reopened.
    mpv.set_property("hwdec", "no").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !labels(&mpv).iter().any(|l| l == LABEL) {
        assert!(Instant::now() < deadline, "the filter never went in");
        motion.update(&mpv, &render, &mut effects, Instant::now());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(effects.held, [true]);
    motion.owners.clear();
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(effects.held, [true, false]);
}

/// Out of the chain while the publish gate is closed (a new source
/// opens), back once it opens; copy-back stays held meanwhile.
#[test]
fn the_filter_stays_out_while_a_new_source_opens() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    motion.follow_gate(gate.clone());
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    gate.store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(motion.due_in(Instant::now()), Some(Duration::ZERO));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    assert_eq!(effects.held, [true], "copy-back kept");
    assert_eq!(motion.input_fps(), None, "the new source is measured anew");
    gate.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(motion.due_in(Instant::now()), Some(Duration::ZERO));
    // Playing, as mpv estimates the new source's rate only from frames it
    // passes on; nothing goes in before that.
    mpv.set_property("pause", false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !labels(&mpv).iter().any(|l| l == LABEL) {
        assert!(Instant::now() < deadline, "not back in the chain");
        motion.update(&mpv, &render, &mut effects, Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let input = motion.input_fps().unwrap();
    assert!((input - 30.).abs() < 1., "{input}");
}

/// The player's file change with the converter, on libmpv's Direct3D 11
/// renderer (WARP, as the GPU renderer draws), headless: the converter in,
/// the next file (`loadfile replace`), out while it opens, the orientation
/// filter again (`vf pre`), the converter in again. A watchdog ends the
/// test when mpv stops answering. Real machine: needs an AMD GPU and
/// `VESTRA_MOTION_CLIPS` (a folder with m1080p30.mp4 and m1080p24.mp4).
#[cfg(windows)]
#[test]
#[ignore = "real machine: AMD GPU and clips"]
fn warp_file_change_with_the_converter() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(clips) = std::env::var_os("VESTRA_MOTION_CLIPS") else {
        diag!("VESTRA_MOTION_CLIPS not set");
        return;
    };
    let clip = |name: &str| {
        std::path::Path::new(&clips)
            .join(name)
            .to_string_lossy()
            .into_owned()
    };
    let mpv = Arc::new(
        Mpv::builder()
            .unwrap()
            .set_property("vo", "libmpv")
            .unwrap()
            .set_property(
                "ao",
                std::env::var("MOTION_AO").unwrap_or("null".into()).as_str(),
            )
            .unwrap()
            .set_property(
                "hwdec",
                std::env::var("MOTION_HWDEC")
                    .unwrap_or("auto-safe".into())
                    .as_str(),
            )
            .unwrap()
            .set_property("vf", crate::playback::source_geometry::INITIAL_FILTERS)
            .unwrap()
            .build()
            .unwrap(),
    );
    mpv.request_log_messages("v").unwrap();
    mpv.set_property("keep-open", true).unwrap();
    mpv.set_property("volume", 0.).unwrap();
    // As the host opens local files (`MOTION_LEASE`): leased, through stream_cb.
    let lease = std::env::var_os("MOTION_LEASE").is_some();
    crate::playback::lease_stream::register(&mpv).unwrap();
    let mut leases = Vec::new();
    let mut open = |name: &str| -> (String, String) {
        if !lease {
            return (clip(name), "pause=no".into());
        }
        let file = std::fs::File::open(clip(name)).unwrap();
        let registration = crate::playback::lease_stream::Registration::new(file);
        let url = registration.url();
        leases.push(registration);
        (
            url,
            "pause=no,demuxer=lavf,demuxer-lavf-probe-info=yes,rebase-start-time=yes".into(),
        )
    };
    let publish = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let renderer = vestra_gpu_render::start_on_warp(mpv.clone(), (1920, 1080), publish).unwrap();
    let handle = renderer.handle();
    // Watchdog: the step counter must move.
    let step = Arc::new(AtomicU64::new(0));
    let watched = step.clone();
    std::thread::spawn(move || {
        let mut last = (u64::MAX, Instant::now());
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = watched.load(Ordering::Relaxed);
            if now == 999 {
                return;
            }
            if now != last.0 {
                last = (now, Instant::now());
            } else if last.1.elapsed() > Duration::from_secs(10) {
                diag!("HANG at step {now}");
                std::process::exit(3);
            }
        }
    });
    let filter = format!(
        "@{LABEL}:{}",
        crate::platform::motion_filter(crate::playback::motion::Engine::Amd, false, false).unwrap()
    );
    let drain = |mpv: &Mpv, what: &str| {
        while let Some(event) = mpv.poll_event() {
            if let rsmpv::Event::LogMessage(m) = &event {
                if m.level == "error" || m.prefix == "vf" || m.text.contains("AMF") {
                    diag!("{what} [{}] {}: {}", m.prefix, m.level, m.text.trim_end());
                }
            } else {
                diag!("{what} event {event:?}");
            }
        }
    };
    let wait = |mpv: &Mpv, seconds: f64, what: &str| {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < until {
            drain(mpv, what);
            std::thread::sleep(Duration::from_millis(20));
        }
        diag!(
            "{what}: pos {:?} vf-fps {:?} frames {} hwdec {:?}",
            mpv.get_property::<f64>("time-pos").ok(),
            mpv.get_property::<f64>("estimated-vf-fps").ok(),
            handle.frames(),
            mpv.get_property::<String>("hwdec-current").ok()
        );
    };
    let orientation = "@vestra-orientation:lavfi=[null,sidedata=mode=delete:type=DISPLAYMATRIX]";
    step.store(1, Ordering::Relaxed);
    let (first, options) = open("m1080p30.mp4");
    mpv.command(&["loadfile", &first, "replace", "-1", &options])
        .unwrap();
    wait(&mpv, 1.5, "file 1");
    mpv.command(&["vf", "pre", orientation]).unwrap();
    if std::env::var("MOTION_HWDEC").is_err() {
        mpv.set_property("hwdec", "auto-copy-safe").unwrap();
    }
    wait(&mpv, 1.5, "copy-back");
    step.store(2, Ordering::Relaxed);
    mpv.command(&["vf", "add", &filter]).unwrap();
    wait(&mpv, 3., "converter 1");
    // On the GPU renderer (its Direct3D 11 interop loaded by the first
    // second's native decoding) the graph configures and doubles: the
    // `hwdec_interop` the platform filter names keeps the device away.
    let first = mpv.get_property::<f64>("estimated-vf-fps").unwrap_or(0.);
    assert!(first > 50., "the first converter doubles 30 fps: {first}");
    step.store(3, Ordering::Relaxed);
    let (second, options) = open("m1080p24.mp4");
    mpv.command(&["loadfile", &second, "replace", "-1", &options])
        .unwrap();
    // `MOTION_KEEP`: the converter stays in the chain across the change;
    // `MOTION_GAP`: seconds between the steps (the host's are milliseconds).
    let keep = std::env::var_os("MOTION_KEEP").is_some();
    let gap: f64 = std::env::var("MOTION_GAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.);
    if !keep {
        mpv.command(&["vf", "remove", &format!("@{LABEL}")])
            .unwrap();
    }
    wait(&mpv, gap, "file 2 opens");
    step.store(4, Ordering::Relaxed);
    mpv.command(&["vf", "pre", orientation]).unwrap();
    wait(&mpv, gap, "orientation 2");
    step.store(5, Ordering::Relaxed);
    if !keep {
        mpv.command(&["vf", "add", &filter]).unwrap();
    }
    step.store(6, Ordering::Relaxed);
    for second in 0..5 {
        step.store(10 + second, Ordering::Relaxed);
        wait(&mpv, 1., "converter 2");
    }
    step.store(999, Ordering::Relaxed);
    let fps = mpv.get_property::<f64>("estimated-vf-fps").unwrap_or(0.);
    assert!(fps > 40., "the second converter runs: {fps}");
}

/// The size rule, calibrated to the measurements: software rendering up to
/// 1080p30 → 60, GPU Double up to 4K24 → 48, Match Display up to 1080p30.
#[test]
fn a_video_too_large_for_the_renderer_is_not_offered() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    let at = |w: f64, h: f64, fps: f64, gpu: bool| Facts {
        video: Some((w, h)),
        source_fps: Some(fps),
        gpu_now: gpu,
        any_rate: true,
        ..facts(&probe, &reason, &rendering)
    };
    // Software rendering.
    assert_eq!(check(&at(1920., 1080., 30., false), Mode::Double), Ok(()));
    let error = check(&at(3840., 2160., 24., false), Mode::Double).unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("size", false));
    assert_eq!(
        error.message(),
        "this video (3840×2160 at 24 fps) is too large for motion smoothing with software rendering; it works up to 1080p at 30 fps"
    );
    assert!(check(&at(2560., 1440., 30., false), Mode::Double).is_err());
    // GPU rendering, Double: 4K24 passes, 4K30 does not.
    assert_eq!(check(&at(3840., 2160., 24., true), Mode::Double), Ok(()));
    assert_eq!(check(&at(3840., 2160., 23.976, true), Mode::Double), Ok(()));
    assert_eq!(
        check(&at(3840., 2160., 30., true), Mode::Double)
            .unwrap_err()
            .code(),
        "size"
    );
    // Match Display: 1080p30 passes, 4K24 does not.
    assert_eq!(check(&at(1920., 1080., 30., true), Mode::Display), Ok(()));
    // 1080p on a 144 Hz display: Match Display draws 144 frames a second.
    let fast = Facts {
        display_fps: 144.,
        ..at(1920., 1080., 24., true)
    };
    assert_eq!(check(&fast, Mode::Display).unwrap_err().code(), "size");
    assert_eq!(check(&fast, Mode::Double), Ok(()));
    let error = check(&at(3840., 2160., 24., true), Mode::Display).unwrap_err();
    assert!(
        error.message().contains("with Match Display"),
        "{}",
        error.message()
    );
    // Unknown size: not judged yet.
    let unknown = Facts {
        video: None,
        ..at(3840., 2160., 30., true)
    };
    assert_eq!(check(&unknown, Mode::Double), Ok(()));
}

#[test]
fn nvidias_size_limit_is_4k30_for_every_renderer_and_mode() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    let at = |w: f64, h: f64, fps: f64, gpu: bool| Facts {
        engine: Engine::Nvidia,
        gpu_now: gpu,
        video: Some((w, h)),
        source_fps: Some(fps),
        any_rate: true,
        ..facts(&probe, &reason, &rendering)
    };
    for (gpu, mode) in [
        (false, Mode::Double),
        (true, Mode::Double),
        (true, Mode::Display),
    ] {
        assert_eq!(
            check(&at(3840., 2160., 30., gpu), mode),
            Ok(()),
            "{gpu} {mode:?}"
        );
        let error = check(&at(4096., 2160., 30., gpu), mode).unwrap_err();
        assert_eq!(error.code(), "size");
        assert!(
            error.message().contains("4K at 30 fps"),
            "{}",
            error.message()
        );
    }
}

#[test]
fn engines_parse_and_an_engine_without_a_filter_says_so() {
    for engine in Engine::ALL {
        assert_eq!(Engine::from_name(engine.as_str()), Some(engine));
    }
    assert_eq!(Engine::from_name("intel"), None);
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    let error = check(
        &Facts {
            engine: Engine::Nvidia,
            has_filter: false,
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "engine");
    assert!(error.fixed());
    assert!(error.message().contains("NVIDIA"), "{}", error.message());
    // The other engine holding the converter: it waits, not refused.
    let error = check(
        &Facts {
            engine: Engine::Nvidia,
            other: Some(Engine::Amd),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "other-engine");
    assert!(!error.fixed());
    assert!(error.message().starts_with("AMD motion smoothing is on"));
}

#[test]
fn one_converter_runs_at_a_time_and_the_first_engine_keeps_it() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    let set = |owner: u64, engine: &str, mode: &str| {
        command(
            owner,
            "video.motion.set",
            json!({"engine":engine,"mode":mode}),
        )
    };
    // NVIDIA first.
    assert!(motion.command(
        &mpv,
        &render,
        &set(1, "nvidia", "double"),
        &events,
        &mut effects
    ));
    let state = reply(&received).unwrap();
    assert_eq!(state["engine"], "nvidia", "{state}");
    assert_eq!(state["active"], "double", "{state}");
    assert_eq!(state["inChain"], true, "{state}");
    assert_eq!(motion.applied.map(|a| a.engine), Some(Engine::Nvidia));
    // AMD asked by another plugin: kept, waiting.
    while received.try_recv().is_ok() {}
    motion.command(
        &mpv,
        &render,
        &set(2, "amd", "double"),
        &events,
        &mut effects,
    );
    let state = reply(&received).unwrap();
    assert_eq!(state["engine"], "amd", "{state}");
    assert_eq!(state["mode"], "double", "{state}");
    assert_eq!(state["active"], "off", "{state}");
    assert_eq!(state["inChain"], false, "{state}");
    assert_eq!(state["double"]["code"], "other-engine", "{state}");
    assert!(
        state["double"]["message"]
            .as_str()
            .unwrap()
            .starts_with("NVIDIA motion smoothing is on"),
        "{state}"
    );
    // Each engine's change goes out with its engine.
    let engines: Vec<String> = received
        .try_iter()
        .filter_map(|event| match event {
            Event::Broadcast(name, state) if name == CHANGED => {
                state["engine"].as_str().map(str::to_owned)
            }
            _ => None,
        })
        .collect();
    assert!(
        engines.iter().all(|e| e == "amd" || e == "nvidia"),
        "{engines:?}"
    );
    // NVIDIA off: AMD's request takes the converter, in place.
    motion.command(
        &mpv,
        &render,
        &set(1, "nvidia", "off"),
        &events,
        &mut effects,
    );
    while received.try_recv().is_ok() {}
    assert_eq!(motion.applied.map(|a| a.engine), Some(Engine::Amd));
    assert_eq!(labels(&mpv).iter().filter(|l| *l == LABEL).count(), 1);
    // Copy-back went on once and stayed on through the hand-over.
    assert_eq!(effects.held, [true]);
    // Off for an engine the plugin did not ask leaves its request.
    motion.command(
        &mpv,
        &render,
        &set(2, "nvidia", "off"),
        &events,
        &mut effects,
    );
    assert_eq!(motion.owners.get(&2), Some(&(Engine::Amd, Mode::Double)));
    // An unknown engine is invalid.
    let other = command(1, "video.motion.state", json!({"engine":"intel"}));
    while received.try_recv().is_ok() {}
    motion.command(&mpv, &render, &other, &events, &mut effects);
    assert_eq!(reply(&received).unwrap_err().code, "invalid");
}

#[test]
fn nvidias_converter_without_a_vulkan_device_is_run_long_at_once() {
    assert!(device_failure(
        Engine::Nvidia,
        "could not create hwdevice=vulkan (NVIDIA): Generic error in an external library",
        1
    ));
    assert!(device_failure(
        Engine::Nvidia,
        "Vulkan device does not support the VK_NV_optical_flow extension",
        1
    ));
    assert!(!device_failure(
        Engine::Nvidia,
        "failed to configure the filter graph",
        3
    ));
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Nvidia] = Some(amd());
    motion.owners.insert(1, (Engine::Nvidia, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    motion.log(
        "lavfi",
        "fatal",
        "could not create hwdevice=vulkan (NVIDIA): Generic error in an external library",
    );
    motion.log(
        "vf",
        "error",
        "Disabling filter vestra-motion because it has failed.",
    );
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    assert!(motion.broken[Engine::Nvidia].is_some());
    assert_eq!(
        motion.broken[Engine::Amd],
        None,
        "AMD's engine is untouched"
    );
    assert_eq!(effects.held, [true, false], "copy-back ends with it");
    // The next request is refused for the run, with the reason.
    let on = command(
        2,
        "video.motion.set",
        json!({"engine":"nvidia","mode":"double"}),
    );
    motion.command(&mpv, &render, &on, &events, &mut effects);
    let error = reply(&received).unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(error.message.contains("Vulkan"), "{}", error.message);
}

/// Frame Doubling, an unmeasured format and a converter that failed the
/// same way twice: each its own reason.
#[test]
fn doubling_format_and_a_repeated_failure_have_their_reasons() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = amd();
    let error = check(
        &Facts {
            doubling: true,
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("doubling", false));
    let error = check(
        &Facts {
            format: Some("10-bit".into()),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("format", false));
    assert!(error.message().contains("10-bit"), "{}", error.message());
    let error = check(
        &Facts {
            broken: Some("AMFFRC->Init() failed with error 4"),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!((error.code(), error.fixed()), ("runtime", true));
    // The fixed set is FIXED_CODES (the SDK's TestHost and docs agree).
    assert!(!Reason::Starting.fixed());
    assert!(Reason::Engine(Engine::Nvidia).fixed());
}

#[test]
fn the_same_failure_on_two_videos_ends_tries_for_the_run() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    for video in 0..2 {
        motion.new_video();
        motion.input = Some(30.);
        motion.update(&mpv, &render, &mut effects, Instant::now());
        assert!(labels(&mpv).iter().any(|l| l == LABEL), "video {video}");
        motion.log("ffmpeg", "error", "AMFFRC->Init() failed with error 4");
        motion.log(
            "vf",
            "error",
            "Disabling filter vestra-motion because it has failed.",
        );
        motion.update(&mpv, &render, &mut effects, Instant::now());
    }
    motion.new_video();
    motion.input = Some(30.);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(
        !labels(&mpv).iter().any(|l| l == LABEL),
        "not tried a third time"
    );
    let state = motion.state(&mpv, &render, 1, Engine::Amd, &Value::Null);
    assert_eq!(state["double"]["code"], "runtime", "{state}");
}

/// Match Display's resampling and copy-back never outlive the filter:
/// removed by the gate, then a check that fails on the next video; a failed
/// re-add after someone took the filter out; Off while the gate is closed.
#[test]
fn resampling_and_copy_back_end_on_every_path_out() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let open = |on: bool| gate.store(on, std::sync::atomic::Ordering::Release);
    let display = || Applied {
        engine: Engine::Amd,
        mode: Mode::Display,
        large: false,
        performance: false,
    };
    let start = |motion: &mut Motion, effects: &mut Recorded| {
        motion.probe[Engine::Amd] = Some(amd());
        motion.owners.insert(1, (Engine::Amd, Mode::Double));
        motion.update(&mpv, &render, effects, Instant::now());
        // As if Match Display ran (software rendering here cannot).
        motion.applied = Some(display());
        motion.resample(effects, Some(60.));
    };

    // 1. The gate takes the filter out; the next video fails a check.
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.follow_gate(gate.clone());
    start(&mut motion, &mut effects);
    open(false);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(motion.applied, None);
    assert_eq!(motion.resampling, Some(60.), "kept while the source opens");
    open(true);
    motion.note_upscaling(true);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(motion.resampling, None);
    assert_eq!(effects.resample.last(), Some(&None));
    assert_eq!(effects.held.last(), Some(&false));

    // 2. Someone took the filter out; putting it back fails.
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    start(&mut motion, &mut effects);
    mpv.command(&["vf", "remove", &format!("@{LABEL}")])
        .unwrap();
    motion.filter = |_, _, _| Some("lavfi=[no_such_filter_here]".into());
    motion.keep_last(&mpv, &mut effects, display());
    assert_eq!(motion.resampling, None);
    assert_eq!(effects.held.last(), Some(&false));
    let _ = mpv.command(&["vf", "remove", &format!("@{LABEL}")]);

    // 3. Off (or the plugin stops) while the gate is closed: all of it at
    // once, and nothing is left for a look that would never come.
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.follow_gate(gate.clone());
    start(&mut motion, &mut effects);
    open(false);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    motion.owners.clear();
    assert!(
        motion.due_in(Instant::now()).is_some(),
        "still holding: looked at"
    );
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(motion.resampling, None);
    assert_eq!(effects.held.last(), Some(&false));
    assert_eq!(motion.due_in(Instant::now()), None);
    open(true);
}

#[test]
fn bit_depth_reads_the_pixel_format_name() {
    for (name, depth) in [
        ("nv12", 8),
        ("yuv420p", 8),
        ("yuvj420p", 8),
        ("rgb24", 8),
        ("p010", 10),
        ("yuv420p10", 10),
        ("yuv420p10le", 10),
        ("gbrp12le", 12),
        ("d3d11", 8),
        ("dxva2_vld", 8),
        ("", 8),
    ] {
        assert_eq!(bit_depth(name), depth, "{name}");
    }
}

/// Native hardware decoding names the surface, copy-back the format: the
/// depth is the same both ways, so a 10-bit video is refused once, not
/// turned on and off every look (copy-back on: 10-bit; off: 8-bit read).
#[test]
fn the_depth_is_the_same_decoded_natively_or_with_copy_back() {
    for (pixelformat, hw, depth) in [
        ("d3d11", "p010", 10),
        ("dxva2_vld", "p010", 10),
        ("d3d11", "nv12", 8),
        ("p010", "", 10),
        ("nv12", "", 8),
        ("d3d11", "", 8),
        ("dxva2_vld", "", 8),
    ] {
        assert_eq!(stream_depth(pixelformat, hw), depth, "{pixelformat} {hw}");
    }
    // The loop: native (before copy-back) and copy-back (after) agree.
    assert_eq!(stream_depth("d3d11", "p010"), stream_depth("p010", ""));
    assert_eq!(stream_depth("d3d11", "nv12"), stream_depth("nv12", ""));
}

#[test]
fn only_a_repeated_amf_start_failure_is_run_long() {
    assert!(device_failure(
        Engine::Amd,
        "AMFFRC->Init() failed with error 4",
        2
    ));
    assert!(!device_failure(
        Engine::Amd,
        "AMFFRC->Init() failed with error 4",
        1
    ));
    assert!(!device_failure(
        Engine::Amd,
        "failed to configure the filter graph",
        2
    ));
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    for _ in 0..2 {
        motion.new_video();
        motion.input = Some(30.);
        motion.update(&mpv, &render, &mut effects, Instant::now());
        motion.log("lavfi", "error", "failed to configure the filter graph");
        motion.log(
            "vf",
            "error",
            "Disabling filter vestra-motion because it has failed.",
        );
        motion.update(&mpv, &render, &mut effects, Instant::now());
    }
    assert_eq!(
        motion.broken[Engine::Amd],
        None,
        "another failure stays per video"
    );
    motion.new_video();
    motion.input = Some(30.);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(labels(&mpv).iter().any(|l| l == LABEL), "tried again");
}

/// `player.stats()` reports the rate before the converter, so Frame
/// Doubling, which measures the source there, does not read 60 for a 30 fps
/// video and back off.
#[test]
fn stats_report_the_rate_before_the_converter() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    mpv.command(&["loadfile", "av://lavfi:testsrc=size=160x120:rate=30"])
        .unwrap();
    // A doubling filter under motion's label (the platform's needs AMD).
    mpv.command(&["vf", "add", &format!("@{LABEL}:lavfi=[fps=fps=60]")])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while mpv
        .get_property::<f64>("estimated-vf-fps")
        .map_or(true, |fps| fps < 55.)
    {
        assert!(Instant::now() < deadline, "no doubled output");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The measured input rate while the converter is in; the container's
    // while it is not known.
    assert_eq!(stats_fps(&mpv, Some(30.)), Some(30.));
    let fps = stats_fps(&mpv, None).unwrap();
    assert!((fps - 30.).abs() < 0.5, "{fps}");
    // A frame step back is one input frame.
    let step = step_fps(&mpv).unwrap();
    assert!((step - 30.).abs() < 3., "{step}");
    mpv.command(&["vf", "remove", &format!("@{LABEL}")])
        .unwrap();
    let after = stats_fps(&mpv, None).unwrap();
    assert!(after > 25., "{after}");
}

/// Field-rate deinterlacing (or any filter before the converter that raises
/// the rate) doubles 25i or 30i to 50 or 60 before the converter: the rate
/// rule judges the converter's real input (measured with the converter out
/// of the chain), so motion smoothing is not offered (100 or 120 fps on a
/// 60 Hz display, falling behind for seconds).
#[test]
fn the_rate_rule_judges_the_converters_real_input() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    // A 30 fps source and a plugin chain that outputs one frame per field.
    mpv.command(&["vf", "add", "@vestra-owned:lavfi=[fps=fps=60]"])
        .unwrap();
    mpv.command(&["loadfile", "av://lavfi:testsrc=size=320x240:rate=30"])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while mpv
        .get_property::<f64>("estimated-vf-fps")
        .map_or(true, |fps| fps < 55.)
    {
        assert!(Instant::now() < deadline, "no field-rate output");
        std::thread::sleep(Duration::from_millis(20));
    }
    mpv.set_property("pause", true).unwrap();
    assert_eq!(
        source_fps(&mpv).map(f64::round),
        Some(30.),
        "the container says 30"
    );
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.input = None;
    motion.any_rate = true;
    motion.probe[Engine::Amd] = Some(amd());
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert!(!labels(&mpv).iter().any(|l| l == LABEL), "not offered");
    let state = motion.state(&mpv, &render, 1, Engine::Amd, &Value::Null);
    assert_eq!(state["double"]["code"], "rate", "{state}");
    assert!(
        (state["sourceFps"].as_f64().unwrap() - 60.).abs() < 2.,
        "{state}"
    );
    assert!(effects.held.is_empty(), "no copy-back for it");
}

/// Until the input rate is measured nothing goes in; once in, the input is
/// kept (mpv's estimate is then the doubled rate).
#[test]
fn the_input_rate_is_measured_out_of_the_chain_and_kept_in_it() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let mut effects = Recorded::default();
    let mut motion = with_stand_in();
    motion.probe[Engine::Amd] = Some(amd());
    motion.input = None;
    motion.owners.insert(1, (Engine::Amd, Mode::Double));
    assert_eq!(motion.input_fps(), None);
    motion.update(&mpv, &render, &mut effects, Instant::now());
    let input = motion.input_fps().unwrap();
    assert!((input - 30.).abs() < 1., "{input}");
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    motion.update(&mpv, &render, &mut effects, Instant::now());
    assert_eq!(motion.input_fps(), Some(input), "kept while in the chain");
    // The stand-in passes 30 fps on: half of it is not a rise, but it is
    // above a kept 10 fps by more than RERATE, so it becomes the input.
    // Playing, as mpv estimates only from frames it passes on.
    mpv.set_property("pause", false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while mpv
        .get_property::<f64>("estimated-vf-fps")
        .map_or(true, |fps| (fps - 30.).abs() > 1.)
    {
        assert!(Instant::now() < deadline, "no steady rate");
        std::thread::sleep(Duration::from_millis(20));
    }
    motion.input = Some(10.);
    while motion.input_fps() == Some(10.) {
        assert!(Instant::now() < deadline, "no new input");
        motion.update(&mpv, &render, &mut effects, Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let risen = motion.input_fps().unwrap();
    assert!((risen - 15.).abs() < 1., "{risen}");
    motion.new_video();
    assert_eq!(
        motion.input_fps(),
        None,
        "measured again for the next video"
    );
}

#[test]
fn overload_names_what_could_not_keep_up_by_renderer() {
    // GPU rendering: the GPU (the converter and the drawing).
    let gpu = overload_message(Engine::Amd, true, "51.2 of 60 fps");
    assert!(gpu.starts_with("the GPU could not keep up"), "{gpu}");
    assert!(gpu.ends_with("51.2 of 60 fps"), "{gpu}");
    // Software rendering: the CPU copying and drawing the doubled frames
    // (the GPU was 25–40 % busy, 2026-11-23), and what helps. Either engine.
    let cpu = overload_message(Engine::Amd, false, "51.2 of 60 fps");
    assert_eq!(
        cpu,
        "rendering on the CPU could not keep up with this video's doubled frames (51.2 of 60 fps); GPU Rendering (Picture ▸ Rendering) helps"
    );
    assert!(!cpu.contains("GPU could not"), "{cpu}");
}

fn apple_m2() -> MotionProbe {
    Ok(MotionGpu {
        name: "Apple M2".into(),
    })
}

#[test]
fn apples_engine_parses_names_its_vendor_and_waits_for_another() {
    assert_eq!(Engine::from_name(APPLE), Some(Engine::Apple));
    assert_eq!(Engine::Apple.as_str(), "apple");
    assert_eq!(Engine::Apple.vendor(), "Apple");
    assert_eq!(Engine::Apple.converter(), "vtfrc");
    assert_eq!(Engine::ALL.len(), 3);
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = apple_m2();
    let apple = |other| Facts {
        engine: Engine::Apple,
        other,
        ..facts(&probe, &reason, &rendering)
    };
    assert_eq!(check(&apple(None), Mode::Double), Ok(()));
    assert_eq!(check(&apple(None), Mode::Display), Ok(()));
    let error = check(&apple(Some(Engine::Amd)), Mode::Double).unwrap_err();
    assert_eq!(error.code(), "other-engine");
    let error = check(
        &Facts {
            other: Some(Engine::Apple),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert!(error.message().starts_with("Apple motion smoothing is on"));
    // The player's own error for an unknown engine names all three.
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    let (events, received) = mpsc::channel();
    let mut motion = with_stand_in();
    let bad = command(1, "video.motion.state", json!({"engine":"intel"}));
    assert!(motion.command(&mpv, &software(), &bad, &events, &mut Recorded::default()));
    let error = reply(&received).unwrap_err();
    assert_eq!(
        error.message,
        r#"engine must be "amd", "nvidia" or "apple""#
    );
}

#[test]
fn apple_takes_10_bit_sdr_but_not_hdr() {
    assert_eq!(format_reason(Engine::Apple, 10, false), None);
    assert_eq!(
        format_reason(Engine::Apple, 8, true).as_deref(),
        Some("HDR")
    );
    assert_eq!(
        format_reason(Engine::Amd, 10, false).as_deref(),
        Some("10-bit")
    );
    assert_eq!(
        format_reason(Engine::Nvidia, 8, true).as_deref(),
        Some("HDR")
    );
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = apple_m2();
    let error = check(
        &Facts {
            engine: Engine::Apple,
            format: Some("HDR".into()),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "format");
    assert!(!error.fixed());
    assert!(
        error.message().contains("SDR video only"),
        "{}",
        error.message()
    );
}

/// Apple's size limit is learned in the run, not from a table: nothing is
/// refused up front (4K included), and once a size fell far behind, that
/// size and larger are `size` for this renderer and mode while smaller
/// ones are still tried.
#[test]
fn apples_size_limit_is_what_fell_far_behind_in_this_run() {
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = apple_m2();
    let at = |video, learned| Facts {
        engine: Engine::Apple,
        video: Some(video),
        source_fps: Some(24.),
        learned,
        ..facts(&probe, &reason, &rendering)
    };
    // No static limit: the AMD table's 4K24 edge does not apply.
    assert_eq!(check(&at((3840., 2160.), None), Mode::Double), Ok(()));
    assert_eq!(check(&at((3840., 2160.), None), Mode::Display), Ok(()));
    let learned = Learned {
        load: 3840. * 1600. * 48.,
        words: "3840×1600 at 24 fps".into(),
    };
    let error = check(&at((3840., 2160.), Some(&learned)), Mode::Double).unwrap_err();
    assert_eq!(error.code(), "size");
    assert!(!error.fixed());
    assert_eq!(
        error.message(),
        "this video (3840×2160 at 24 fps) is too large for Apple motion smoothing on this computer: videos of about 3840×1600 at 24 fps fell far behind twice in this run; smaller videos are still tried, and choosing the mode again tries this size again"
    );
    assert_eq!(
        check(&at((3840., 1600.), Some(&learned)), Mode::Double)
            .unwrap_err()
            .code(),
        "size"
    );
    assert_eq!(
        check(&at((1920., 1080.), Some(&learned)), Mode::Double),
        Ok(())
    );
}

/// A size is learned only from two far-behind playbacks of about that
/// size with GPU rendering, never from one window, a passing slowdown, a
/// much smaller or larger video, or AMD's and NVIDIA's (measured tables);
/// a plugin setting the mode again forgets it.
#[test]
fn an_apple_size_is_learned_from_two_playbacks_far_behind_and_forgotten_on_a_retry() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let mut motion = with_stand_in();
    motion.probe[Engine::Apple] = Some(apple_m2());
    let key = (Engine::Apple, false);
    // Behind but not far (a passing slowdown): no strike.
    motion.strike(&mpv, Engine::Apple, (Mode::Double, 60.), 0.15);
    assert!(motion.strikes.is_empty());
    // AMD and NVIDIA keep their measured tables.
    motion.strike(&mpv, Engine::Amd, (Mode::Double, 60.), 0.4);
    assert!(motion.strikes.is_empty());
    // Far behind once: a strike, nothing learned.
    motion.strike(&mpv, Engine::Apple, (Mode::Double, 60.), 0.33);
    assert_eq!(motion.strikes[&key].len(), 1);
    assert!(motion.learned.is_empty());
    // Far behind again on the same playback: still one strike.
    motion.strike(&mpv, Engine::Apple, (Mode::Double, 60.), 0.4);
    assert_eq!(motion.strikes[&key].len(), 1);
    assert!(motion.learned.is_empty());
    // A much larger video on another playback is not about the same size.
    motion.strikes.get_mut(&key).unwrap()[0].load *= 4.;
    motion.generation = Some(2);
    motion.strike(&mpv, Engine::Apple, (Mode::Double, 60.), 0.33);
    assert!(motion.learned.is_empty());
    // About the same size on a third playback: learned, the smaller load.
    motion.generation = Some(3);
    motion.strike(&mpv, Engine::Apple, (Mode::Double, 60.), 0.33);
    let learned = &motion.learned[&key];
    assert_eq!(learned.words, "320×240 at 30 fps");
    assert_eq!(learned.load, 320. * 240. * 60.);
    // Match Display is another limit, not learned.
    assert!(!motion.learned.contains_key(&(Engine::Apple, true)));
    // Only with GPU rendering: software rendering does not apply it.
    let error = motion.check_now(&mpv, &software(), &Value::Null, Engine::Apple, Mode::Double);
    assert_ne!(error.err().map(|e| e.code()), Some("size"));
    // The user tries again: a plugin sets the mode, and the limit goes.
    let (events, received) = mpsc::channel();
    let on = command(
        1,
        "video.motion.set",
        json!({"engine":"apple","mode":"double"}),
    );
    let mut effects = Recorded::default();
    motion.command(&mpv, &software(), &on, &events, &mut effects);
    assert!(reply(&received).is_ok());
    assert!(motion.learned.is_empty());
    assert!(motion.strikes.is_empty());
}

/// A window in which `vtfrc` said it stalled, or one with software
/// rendering, is no strike (the apply path decides that before `strike`).
#[test]
fn a_stalled_window_is_marked_and_a_new_session_waits_for_ready_again() {
    let mut motion = with_stand_in();
    motion.applied = Some(Applied {
        engine: Engine::Apple,
        mode: Mode::Double,
        large: false,
        performance: false,
    });
    motion.log(
        "vtfrc",
        "warn",
        "the processor has not answered for 1000 ms: frames pass through until it does",
    );
    assert!(motion.watch.stalled);
    // A ready session, then a new one (another video or size): the gate
    // applies again, the warm-up starts afresh.
    motion.converter_ready = true;
    motion.watch.since = Some(Instant::now());
    motion.log(
        "vtfrc",
        "v",
        "starting a session for 1280x534 nv12 (videotoolbox)",
    );
    assert!(!motion.converter_ready);
    assert!(
        motion
            .added
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
    );
    assert!(motion.watch.since.is_none() && !motion.watch.stalled);
}

#[test]
fn apples_converter_cannot_start_where_the_system_has_no_interpolation() {
    let refused = "VideoToolbox frame interpolation (any) is not available on this system (regular: macOS 15.4, low-latency: macOS 26; Apple silicon)";
    assert!(device_failure(Engine::Apple, refused, 1));
    // A size or a video it cannot take stays per video.
    assert!(!device_failure(
        Engine::Apple,
        "frame interpolation unavailable for 8192x4320: regular interpolator: not supported on this Mac",
        5
    ));
    assert!(!device_failure(
        Engine::Apple,
        "HDR video is not supported (pq)",
        5
    ));
    assert!(!device_failure(Engine::Amd, refused, 5));
    // After mpv refused the filter, that line makes it `runtime`.
    let mut motion = with_stand_in();
    motion.refused[Engine::Apple] = Some("filter refused".into());
    motion.refusing = Some(Engine::Apple);
    motion.log("vtfrc", "error", refused);
    assert_eq!(motion.broken[Engine::Apple].as_deref(), Some(refused));
    assert_eq!(motion.refused[Engine::Apple], None);
    let (reason, rendering) = (RenderReason::Default, Value::Null);
    let probe = apple_m2();
    let error = check(
        &Facts {
            engine: Engine::Apple,
            broken: Some(refused),
            ..facts(&probe, &reason, &rendering)
        },
        Mode::Double,
    )
    .unwrap_err();
    assert_eq!(error.code(), "runtime");
    assert!(error.fixed());
    assert!(
        error
            .message()
            .starts_with("Apple's frame interpolation cannot start on this Mac")
    );
}

#[test]
fn apples_overload_names_the_frame_interpolation_not_a_faster_search() {
    let gpu = overload_message(Engine::Apple, true, "30.1 of 48 fps");
    assert_eq!(
        gpu,
        "this computer's frame interpolation could not keep up with this video: 30.1 of 48 fps"
    );
    assert!(overload_message(Engine::Apple, false, "x").starts_with("rendering on the CPU"));
    assert!(!Engine::Apple.faster_step());
    assert!(Engine::Amd.faster_step() && Engine::Nvidia.faster_step());
}

#[test]
fn apples_converter_lines_count_and_its_session_ready_is_heard() {
    let mut motion = with_stand_in();
    motion.applied = Some(Applied {
        engine: Engine::Apple,
        mode: Mode::Double,
        large: false,
        performance: false,
    });
    motion.log(
        "vtfrc",
        "v",
        "starting a session for 1920x800 nv12 (videotoolbox)",
    );
    assert!(!motion.converter_ready);
    motion.log(
        "vtfrc",
        "info",
        "regular interpolator ready for 1920x800 after 410 ms",
    );
    assert!(motion.converter_ready);
    motion.log("vtfrc", "error", "HDR video is not supported (pq)");
    assert_eq!(
        motion.last_error.as_deref(),
        Some("HDR video is not supported (pq)")
    );
}

/// What a test that drives the real `vtfrc` is doing now, for its watchdog.
#[derive(Clone)]
struct Steps(std::sync::Arc<std::sync::Mutex<(&'static str, Instant)>>);

impl Steps {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new((
            "start",
            Instant::now(),
        ))))
    }
    /// The next step, also on stderr directly (not captured by the test
    /// harness, so a CI log shows it even when the test never ends).
    fn now(&self, step: &'static str) {
        let mut at = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        say(&format!(
            "{} after {} ms: {step}",
            at.0,
            at.1.elapsed().as_millis()
        ));
        *at = (step, Instant::now());
    }
    fn current(&self) -> (&'static str, Duration) {
        let at = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (at.0, at.1.elapsed())
    }
}

/// A line on stderr that the test harness does not capture.
fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "vtfrc test: {line}");
}

/// Run a test that drives the real `vtfrc` on its own thread with a hard
/// deadline: mpv blocked inside the filter (its core thread waiting on
/// VideoToolbox) fails the test with the step it hung in, instead of
/// hanging the whole test run. The thread is left behind then.
fn with_deadline(seconds: u64, body: impl FnOnce(&Steps) + Send + 'static) {
    let steps = Steps::new();
    let (done, finished) = mpsc::channel();
    let inner = steps.clone();
    let worker = std::thread::spawn(move || {
        body(&inner);
        let _ = done.send(());
    });
    match finished.recv_timeout(Duration::from_secs(seconds)) {
        Ok(()) => worker.join().unwrap(),
        // The body panicked: its own message.
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(panic) = worker.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let (step, since) = steps.current();
            panic!(
                "mpv did not come back for {} s in step \"{step}\" (stuck there {} ms): its core thread is blocked inside vtfrc/VideoToolbox on this machine",
                seconds,
                since.as_millis()
            );
        }
    }
}

/// Drain mpv's events into motion's log, a bounded number at a time.
fn feed(mpv: &Mpv, motion: &mut Motion) {
    for _ in 0..10_000 {
        let Some(event) = mpv.poll_event() else {
            return;
        };
        if let rsmpv::Event::LogMessage(m) = &event {
            if m.prefix == "vtfrc" && m.level != "debug" && m.level != "trace" {
                say(&format!("[vtfrc] {}: {}", m.level, m.text.trim_end()));
            }
            motion.log(&m.prefix, &m.level, &m.text);
        }
    }
}

/// A virtual Mac (GitHub's macOS runner): the player does not offer
/// Apple's engine there (`platform::virtual_mac`), so the tests that drive
/// the real `vtfrc` skip, said on stderr, unless `VESTRA_VTFRC_ON_VM=1`
/// asks to drive it all the same (to see under the deadline which step
/// does not come back).
fn skipped_on_a_virtual_mac() -> bool {
    #[cfg(target_os = "macos")]
    if crate::platform::virtual_mac() && std::env::var_os("VESTRA_VTFRC_ON_VM").is_none() {
        say(
            "SKIPPED: a virtual Mac, where the player does not offer Apple's engine; VESTRA_VTFRC_ON_VM=1 drives vtfrc all the same",
        );
        return true;
    }
    false
}

/// How long the real `vtfrc` may take to be ready in a test before the run
/// is inconclusive: the regular interpolator took about 9 s for its first
/// session on an M2.
const TEST_READY: Duration = Duration::from_secs(20);

/// Apple's `vtfrc` in this libmpv (ours, src/packaging/libmpv): in mpv's
/// chain without copy-back, its session ready, the rate doubled, a new
/// session for another size waited for again. A libmpv without it
/// (Homebrew's) refuses the filter: `filter`. Where the interpolator never
/// gets ready within [`TEST_READY`] (a virtual Mac), the doubling is
/// inconclusive and said so on stderr, and the host's side is checked
/// instead: the gate gives up, the fallback takes the filter out with
/// `overload`, and off and mpv's teardown come back. Every mpv call runs
/// under a hard deadline ([`with_deadline`]). macOS only, silent
/// (`ao=null`, `vo=null`).
#[test]
fn apples_converter_doubles_in_mpv_without_copy_back() {
    if !cfg!(target_os = "macos") || skipped_on_a_virtual_mac() {
        return;
    }
    let _one = one_mpv_at_a_time();
    with_deadline(120, |steps| {
        steps.now("mpv starts and plays a test video");
        let mpv = mpv();
        mpv.request_log_messages("v").unwrap();
        play(&mpv, 30);
        let render = software();
        let (events, received) = mpsc::channel();
        let mut motion = Motion::with_probe(apple_m2);
        motion.any_rate = false;
        motion.input = Some(30.);
        motion.generation = Some(1);
        let mut effects = Recorded::default();
        steps.now("vf add vtfrc (the filter is created: VideoToolbox's isSupported)");
        let on = command(
            1,
            "video.motion.set",
            json!({"engine":"apple","mode":"double"}),
        );
        assert!(motion.command(&mpv, &render, &on, &events, &mut effects));
        let state = reply(&received).unwrap();
        assert!(
            effects.held.is_empty(),
            "no copy-back for Apple: {:?}",
            effects.held
        );
        if !labels(&mpv).iter().any(|l| l == LABEL) {
            // This libmpv has no `vtfrc`.
            assert_eq!(state["double"]["code"], "filter", "{state}");
            say(&format!("no vtfrc in this libmpv: {state}"));
            steps.now("mpv is destroyed");
            drop(mpv);
            return;
        }
        assert_eq!(state["active"], "double", "{state}");
        assert_eq!(state["gpu"], "Apple M2", "{state}");
        steps.now("playing until the session is ready and the rate doubles");
        mpv.set_property("pause", false).unwrap();
        let until = Instant::now() + TEST_READY;
        let mut doubled = 0.;
        while Instant::now() < until {
            feed(&mpv, &mut motion);
            motion.poll(&mpv, &render, &events, &mut effects, 1);
            doubled = mpv.get_property::<f64>("estimated-vf-fps").unwrap_or(0.);
            if motion.converter_ready && doubled > 55. {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !motion.converter_ready {
            say(&format!(
                "INCONCLUSIVE: vtfrc's session was not ready within {} s on this machine (output {doubled:.1} fps); the doubling is not tested here, the host's handling of a converter that never gets ready is",
                TEST_READY.as_secs()
            ));
            steps.now("the gate gives up and the fallback takes the filter out");
            motion.apple_ready = Duration::ZERO;
            let until = Instant::now() + WARMUP + WINDOW + Duration::from_secs(4);
            while Instant::now() < until && motion.overload.is_none() {
                feed(&mpv, &mut motion);
                motion.poll(&mpv, &render, &events, &mut effects, 1);
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(
                motion.overload.is_some() || motion.failed.is_some(),
                "a converter that never gets ready is taken out"
            );
            assert!(!labels(&mpv).iter().any(|l| l == LABEL));
            steps.now("mpv is destroyed");
            drop(mpv);
            return;
        }
        assert!(doubled > 55., "30 fps doubled: {doubled}");
        assert!(motion.failed.is_none(), "{:?}", motion.failed);
        let state = motion.state(&mpv, &render, 1, Engine::Apple, &Value::Null);
        assert_eq!(state["searchMode"], "native", "{state}");
        assert_eq!(state["inChain"], true, "{state}");
        // The next video, another size, with the filter left in the chain:
        // vtfrc starts a new session, and the warm-up waits for it again.
        steps.now("the next video, another size: a new session");
        mpv.command(&["loadfile", "av://lavfi:testsrc=size=640x360:rate=30"])
            .unwrap();
        let until = Instant::now() + TEST_READY;
        let (mut restarted, mut ready_again) = (false, false);
        while Instant::now() < until && !ready_again {
            feed(&mpv, &mut motion);
            restarted |= !motion.converter_ready;
            ready_again = restarted && motion.converter_ready;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(restarted, "the new session put the gate back");
        assert!(ready_again, "and its ready line opened it");
        steps.now("vf remove (off)");
        let off = command(
            1,
            "video.motion.set",
            json!({"engine":"apple","mode":"off"}),
        );
        assert!(motion.command(&mpv, &render, &off, &events, &mut effects));
        assert!(!labels(&mpv).iter().any(|l| l == LABEL));
        assert!(effects.held.iter().all(|held| !held), "{:?}", effects.held);
        steps.now("mpv is destroyed");
        drop(mpv);
        steps.now("done");
    });
}

/// Removing `vtfrc` and destroying mpv while its session is still starting
/// (the regular interpolator on a large video takes the longest) come back
/// at once: the session's start runs on its own queue, and the filter only
/// lets go of it. A libmpv without `vtfrc` skips this. Hard deadline.
#[test]
fn removing_vtfrc_while_its_session_starts_does_not_wait() {
    if !cfg!(target_os = "macos") || skipped_on_a_virtual_mac() {
        return;
    }
    let _one = one_mpv_at_a_time();
    with_deadline(60, |steps| {
        steps.now("mpv plays a 4K test video");
        let mpv = mpv();
        mpv.request_log_messages("v").unwrap();
        mpv.command(&["loadfile", "av://lavfi:testsrc=size=3840x2160:rate=24"])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while video_size(&mpv).is_none() {
            assert!(Instant::now() < deadline, "no video");
            std::thread::sleep(Duration::from_millis(20));
        }
        steps.now("vf add vtfrc (regular interpolator)");
        let added = Instant::now();
        if mpv
            .command(&["vf", "add", &format!("@{LABEL}:vtfrc=engine=regular")])
            .is_err()
        {
            say("no vtfrc in this libmpv (or no frame interpolation here): skipped");
            return;
        }
        let add_ms = added.elapsed().as_millis();
        // The first frame starts the session.
        let until = Instant::now() + Duration::from_secs(5);
        let mut motion = with_stand_in();
        let mut starting = false;
        while Instant::now() < until && !starting {
            while let Some(event) = mpv.poll_event() {
                if let rsmpv::Event::LogMessage(m) = &event
                    && m.prefix == "vtfrc"
                {
                    starting |= m.text.contains("starting a session for");
                    motion.log(&m.prefix, &m.level, &m.text);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        steps.now("vf remove while the session starts");
        let removed = Instant::now();
        mpv.command(&["vf", "remove", &format!("@{LABEL}")])
            .unwrap();
        let remove_ms = removed.elapsed().as_millis();
        steps.now("mpv is destroyed");
        let destroyed = Instant::now();
        drop(mpv);
        let destroy_ms = destroyed.elapsed().as_millis();
        say(&format!(
            "vf add {add_ms} ms, session started: {starting}, vf remove {remove_ms} ms, destroy {destroy_ms} ms"
        ));
        assert!(remove_ms < 2000, "vf remove waited {remove_ms} ms");
        assert!(destroy_ms < 5000, "mpv's teardown waited {destroy_ms} ms");
        steps.now("done");
    });
}

/// A real clip through Apple's converter as the player drives it (VideoToolbox
/// decoding, no copy-back, the fallback watching), silent and headless:
/// `VESTRA_APPLE_CLIP=<file>` with a libmpv that has `vtfrc`
/// (`DYLD_LIBRARY_PATH=<deps>/lib`). Reports the output rate, the ready
/// line and whether the fallback stepped in.
#[test]
#[ignore = "real machine: Apple silicon, a clip and our libmpv"]
fn apple_real_clip_through_the_converter() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(clip) = std::env::var_os("VESTRA_APPLE_CLIP") else {
        diag!("VESTRA_APPLE_CLIP not set");
        return;
    };
    let seconds: u64 = std::env::var("VESTRA_APPLE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let _one = one_mpv_at_a_time();
    with_deadline(seconds + 60, move |steps| {
        steps.now("the clip opens");
        let mpv = mpv();
        mpv.request_log_messages("v").unwrap();
        mpv.set_property("hwdec", "videotoolbox").unwrap();
        mpv.command(&["loadfile", &clip.to_string_lossy()]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while video_size(&mpv).is_none() || source_fps(&mpv).is_none() {
            assert!(Instant::now() < deadline, "the clip does not open");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(1500));
        let render = software();
        let (events, received) = mpsc::channel();
        let mut motion = Motion::with_probe(apple_m2);
        // Software rendering by itself needs twice the rate to be the
        // display's; the converter is what is measured here.
        motion.any_rate = true;
        motion.generation = Some(1);
        let mut effects = Recorded::default();
        motion.poll(&mpv, &render, &events, &mut effects, 1);
        let on = command(
            1,
            "video.motion.set",
            json!({"engine":"apple","mode":"double"}),
        );
        steps.now("vf add vtfrc");
        motion.command(&mpv, &render, &on, &events, &mut effects);
        let state = reply(&received).unwrap();
        diag!("apple clip: {state}");
        steps.now("playing");
        let until = Instant::now() + Duration::from_secs(seconds);
        let mut samples = Vec::new();
        while Instant::now() < until {
            feed(&mpv, &mut motion);
            motion.poll(&mpv, &render, &events, &mut effects, 1);
            if motion.converter_ready {
                samples.push(mpv.get_property::<f64>("estimated-vf-fps").unwrap_or(0.));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let state = motion.state(&mpv, &render, 1, Engine::Apple, &Value::Null);
        let tail = &samples[samples.len().saturating_sub(50)..];
        let mean = tail.iter().sum::<f64>() / tail.len().max(1) as f64;
        diag!(
            "apple clip: {:?} hwdec {:?}, input {:?}, output mean of the last 5 s {mean:.1}, held {:?}, overload {:?}, learned {:?}, state {state}",
            video_size(&mpv),
            mpv.get_property::<String>("hwdec-current").ok(),
            motion.input,
            effects.held,
            motion.overload,
            motion.learned
        );
        steps.now("mpv is destroyed");
        drop(mpv);
    });
}

/// A converter of Apple's that never says it is ready (a session start the
/// system never answers) is not left waiting: until `apple_ready` its
/// output is not judged, after it the frames it passes through are judged
/// as any other output, and the fallback takes the filter out with
/// `overload`. A stand-in filter that passes the frames through plays the
/// never-ready `vtfrc`.
#[test]
fn an_apple_converter_that_never_gets_ready_is_taken_out_after_the_gate() {
    let _one = one_mpv_at_a_time();
    let mpv = mpv();
    play(&mpv, 30);
    let render = software();
    let (events, received) = mpsc::channel();
    let mut motion = Motion::with_probe(apple_m2);
    motion.filter = stand_in;
    motion.any_rate = false;
    motion.input = Some(30.);
    motion.generation = Some(1);
    let mut effects = Recorded::default();
    let on = command(
        1,
        "video.motion.set",
        json!({"engine":"apple","mode":"double"}),
    );
    assert!(motion.command(&mpv, &render, &on, &events, &mut effects));
    assert_eq!(reply(&received).unwrap()["active"], "double");
    mpv.set_property("pause", false).unwrap();
    let run = |motion: &mut Motion, effects: &mut Recorded, seconds: u64| {
        let until = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < until && motion.overload.is_none() {
            motion.checked = None;
            motion.poll(&mpv, &render, &events, effects, 1);
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // Within the gate: nothing judged, although the output is half.
    motion.apple_ready = Duration::from_secs(3600);
    run(&mut motion, &mut effects, (WARMUP + WINDOW).as_secs() + 2);
    assert!(motion.overload.is_none(), "{:?}", motion.overload);
    assert!(labels(&mpv).iter().any(|l| l == LABEL));
    // The gate given up: judged, and out with `overload`.
    motion.apple_ready = Duration::ZERO;
    run(&mut motion, &mut effects, (WARMUP + WINDOW).as_secs() + 4);
    let overload = motion.overload.clone().expect("taken out");
    assert!(overload.contains("could not keep up"), "{overload}");
    assert!(!labels(&mpv).iter().any(|l| l == LABEL));
    // Software rendering here: no strike, nothing learned.
    assert!(motion.strikes.is_empty() && motion.learned.is_empty());
}

/// The watchdog of the real-`vtfrc` tests fails with the step it hung in.
#[test]
#[should_panic(expected = "in step \"waiting forever\"")]
fn the_vtfrc_watchdog_names_the_step_it_hung_in() {
    with_deadline(1, |steps| {
        steps.now("waiting forever");
        std::thread::sleep(Duration::from_secs(3));
    });
}
