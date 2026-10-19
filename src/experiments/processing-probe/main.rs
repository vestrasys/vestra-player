#![forbid(unsafe_code)]
//! Isolated evidence tool. No plugin API, no product policy, no unsafe FFI.
use anyhow::Result;
use rsmpv::{
    Mpv,
    render::{RenderContext, SwPixelFormat},
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Frames {
    count: u64,
    latest: Vec<u8>,
    times: Vec<f64>,
    signatures: Vec<(f64, u64)>,
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let media = &args[1];
    let directory = PathBuf::from(&args[2]);
    let mode = args.get(3).map(String::as_str).unwrap_or("null");
    std::fs::create_dir_all(&directory)?;
    let mut log = std::fs::File::create(directory.join("events.jsonl"))?;
    let start = Instant::now();
    let builder = Mpv::builder()?
        .set_property("vo", "libmpv")?
        .set_property("hwdec", "no")?
        .set_property("keep-open", true)?
        .set_property("audio-buffer", 0.05_f64)?;
    let builder = if mode == "pcm" {
        builder.set_property("ao", "pcm")?.set_property(
            "ao-pcm-file",
            directory.join("actual-mpv-output.wav").to_str().unwrap(),
        )?
    } else {
        builder.set_property("ao", "null")?
    };
    let mpv = Arc::new(builder.build()?);
    mpv.request_log_messages("v")?;
    let frames = Arc::new(Mutex::new(Frames::default()));
    let out = frames.clone();
    let render_core = mpv.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let force_redraw = Arc::new(AtomicBool::new(false));
    let redraw = force_redraw.clone();
    let (ready, wait) = std::sync::mpsc::channel();
    let renderer = thread::spawn(move || -> Result<()> {
        let mut render = RenderContext::new_software(render_core)?;
        ready.send(())?;
        while !stopping.load(Ordering::Relaxed) {
            if render.update() || redraw.load(Ordering::Relaxed) {
                let mut pixels = vec![0; 640 * 360 * 3];
                render.render_software(640, 360, SwPixelFormat::Rgb24, 640 * 3, &mut pixels)?;
                let mut f = out.lock().unwrap();
                f.count += 1;
                let signature = signature(&pixels);
                f.signatures
                    .push((start.elapsed().as_secs_f64(), signature));
                f.latest = pixels;
                f.times.push(start.elapsed().as_secs_f64());
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    });
    wait.recv()?;
    let mut records = Vec::<Value>::new();
    let mut observe = |phase: &str, duration: f64| -> Result<()> {
        let until = Instant::now() + Duration::from_secs_f64(duration);
        while Instant::now() < until {
            while let Some(event) = mpv.poll_event() {
                writeln!(
                    log,
                    "{}",
                    json!({"wall":start.elapsed().as_secs_f64(),"phase":phase,"event":format!("{event:?}")})
                )?;
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    };
    mpv.command(&["loadfile", media])?;
    observe("loading", 0.8)?;
    // Keep each graph resident; commands target named inner libavfilter instances.
    let vf = "@video:lavfi=[eq@tone=brightness=0:contrast=1:saturation=1:gamma=1,gblur@blur=sigma=0.01:steps=3]";
    let af = "@audio:lavfi=[volume@protect=volume=1,equalizer@band=frequency=1000:width_type=q:width=1:gain=0,treble@high=frequency=3000:width_type=s:width=0.5:gain=0,dynaudnorm@norm=framelen=100:gausssize=5:peak=0.9:maxgain=2]";
    for (name, chain) in [("vf", vf), ("af", af)] {
        if mode == "pause-isolation" && name == "af" {
            continue;
        }
        let at = Instant::now();
        let r = mpv.command(&[name, "set", chain]);
        records.push(json!({"phase":"setup","command":[name,"set",chain],"result":format!("{r:?}"),"ms":at.elapsed().as_secs_f64()*1000.}));
    }
    observe("settle", 0.8)?;
    if mode.ends_with("pause-isolation") {
        mpv.set_property("pause", true)?;
        observe("pause-stable", 0.4)?;
        let initial = frames.lock().unwrap().latest.clone();
        let before = snapshot(&mpv, &frames, &start);
        for (phase, args) in [
            (
                "runtime-brightness",
                vec!["vf-command", "video", "brightness", "-0.3", "eq@tone"],
            ),
            (
                "local-vf-refresh",
                vec![
                    "vf",
                    "add",
                    "@video:lavfi=[eq@tone=brightness=-0.3:contrast=1:saturation=1:gamma=1,gblur@blur=sigma=2:steps=3]",
                ],
            ),
        ] {
            let prior = frames.lock().unwrap().latest.clone();
            let at = Instant::now();
            let command_wall = start.elapsed().as_secs_f64();
            let prior_signature = signature(&prior);
            let result = mpv.command(&args);
            let ms = at.elapsed().as_secs_f64() * 1000.;
            observe(phase, 1.2)?;
            let pixels = frames.lock().unwrap().latest.clone();
            save_ppm(&directory.join(format!("{phase}.ppm")), &pixels)?;
            records.push(json!({"phase":phase,"command":args,"result":format!("{result:?}"),"ms":ms,"before":before,"after":snapshot(&mpv,&frames,&start),"pixelMae":diff(&prior,&pixels),"initialMae":diff(&initial,&pixels),"commandWall":command_wall,"firstPixelChangeMs":frames.lock().unwrap().signatures.iter().find(|(time,hash)| *time>=command_wall && *hash!=prior_signature).map(|(time,_)|(time-command_wall)*1000.)}));
        }
        mpv.command(&["stop"])?;
        stop.store(true, Ordering::Relaxed);
        renderer.join().unwrap()?;
        std::fs::write(
            directory.join("report.json"),
            serde_json::to_vec_pretty(&json!({"mode":mode,"records":records}))?,
        )?;
        println!("pause isolation complete: {}", directory.display());
        return Ok(());
    }

    let tests = [
        ("vf-command", "brightness", "0.2", "eq@tone"),
        ("vf-command", "contrast", "1.2", "eq@tone"),
        ("vf-command", "saturation", "0.3", "eq@tone"),
        ("vf-command", "gamma", "1.2", "eq@tone"),
        ("vf-command", "sigma", "2", "gblur@blur"),
        ("vf-command", "sigmaV", "2", "gblur@blur"),
        ("af-command", "gain", "5", "equalizer@band"),
        ("af-command", "gain", "4", "treble@high"),
        ("af-command", "volume", "0.5", "volume@protect"),
        ("af-command", "maxgain", "3", "dynaudnorm@norm"),
        ("af-command", "peak", "0.7", "dynaudnorm@norm"),
        ("af-command", "enable", "0", "dynaudnorm@norm"),
        ("af-command", "enable", "1", "dynaudnorm@norm"),
    ];
    for (command, key, value, target) in tests {
        let label = if command == "vf-command" {
            "video"
        } else {
            "audio"
        };
        let at = Instant::now();
        let r = mpv.command(&[command, label, key, value, target]);
        records.push(json!({"phase":"parameter","wall":start.elapsed().as_secs_f64(),"command":[command,label,key,value,target],"result":format!("{r:?}"),"ms":at.elapsed().as_secs_f64()*1000.}));
        observe("parameter", 0.08)?;
    }
    let before = snapshot(&mpv, &frames, &start);
    let mut slowest: f64 = 0.;
    let mut failed = 0;
    for i in 0..150 {
        let at = Instant::now();
        let gain = format!("{}", (i % 13) as f64 - 6.);
        let peak = format!("{}", 0.5 + (i % 45) as f64 / 100.);
        for (key, value, target) in [
            ("gain", gain.as_str(), "equalizer@band"),
            ("peak", peak.as_str(), "dynaudnorm@norm"),
        ] {
            if mpv
                .command(&["af-command", "audio", key, value, target])
                .is_err()
            {
                failed += 1;
            }
        }
        slowest = slowest.max(at.elapsed().as_secs_f64() * 1000.);
        observe("audio-slider", 0.033)?;
    }
    records.push(json!({"phase":"audio-slider","before":before,"after":snapshot(&mpv,&frames,&start),"iterations":150,"failed":failed,"slowestPairMs":slowest}));
    mpv.set_property("pause", true)?;
    observe("pause-stable", 0.4)?;
    let paused_before = frames.lock().unwrap().latest.clone();
    let pause_before = snapshot(&mpv, &frames, &start);
    let r = mpv.command(&["vf-command", "video", "brightness", "-0.3", "eq@tone"]);
    observe("paused-command", 0.4)?;
    let paused_after = frames.lock().unwrap().latest.clone();
    let pixel_diff = diff(&paused_before, &paused_after);
    save_ppm(&directory.join("paused-before.ppm"), &paused_before)?;
    save_ppm(&directory.join("paused-command.ppm"), &paused_after)?;
    records.push(json!({"phase":"paused-command","result":format!("{r:?}"),"before":pause_before,"after":snapshot(&mpv,&frames,&start),"pixelMae":pixel_diff}));
    // Compare documented bare mpv adjustment, without seek/unpause.
    let r = mpv.set_property("brightness", 35.0_f64);
    observe("paused-property", 0.4)?;
    let property_after = frames.lock().unwrap().latest.clone();
    save_ppm(&directory.join("paused-property.ppm"), &property_after)?;
    records.push(json!({"phase":"paused-property","result":format!("{r:?}"),"after":snapshot(&mpv,&frames,&start),"pixelMae":diff(&paused_after,&property_after)}));
    force_redraw.store(mode == "redraw", Ordering::Relaxed);
    let graph_before = frames.lock().unwrap().latest.clone();
    let graph_start = snapshot(&mpv, &frames, &start);
    let at = Instant::now();
    let r = mpv.command(&["vf", "add", "@video:lavfi=[eq@tone=brightness=-0.3:contrast=1.2:saturation=0.3:gamma=1.2,gblur@blur=sigma=2:sigmaV=2:steps=3]"]);
    let command_ms = at.elapsed().as_secs_f64() * 1000.;
    observe("paused-local-reconfigure", 0.4)?;
    let graph_after = frames.lock().unwrap().latest.clone();
    save_ppm(
        &directory.join("paused-local-reconfigure.ppm"),
        &graph_after,
    )?;
    records.push(json!({"phase":"paused-local-reconfigure","result":format!("{r:?}"),"ms":command_ms,"before":graph_start,"after":snapshot(&mpv,&frames,&start),"pixelMae":diff(&graph_before,&graph_after)}));
    force_redraw.store(false, Ordering::Relaxed);
    mpv.set_property("pause", false)?;
    observe("resume", 0.4)?;
    for (op, value) in [
        ("add", "@local:lavfi=[volume=0.8]"),
        ("remove", "@local"),
        ("add", "@bad:lavfi=[not_a_real_filter]"),
    ] {
        let before = snapshot(&mpv, &frames, &start);
        let at = Instant::now();
        let r = mpv.command(&["af", op, value]);
        let command_ms = at.elapsed().as_secs_f64() * 1000.;
        observe("topology", 0.4)?;
        records.push(json!({"phase":"topology","command":["af",op,value],"result":format!("{r:?}"),"ms":command_ms,"before":before,"after":snapshot(&mpv,&frames,&start),"af":format!("{:?}",mpv.get_property::<rsmpv::Node>("af"))}));
    }
    observe("finish", 0.2)?;
    mpv.command(&["stop"])?;
    observe("stop", 0.1)?;
    stop.store(true, Ordering::Relaxed);
    renderer.join().unwrap()?;
    let f = frames.lock().unwrap();
    let gaps: Vec<_> = f.times.windows(2).map(|w| w[1] - w[0]).collect();
    let report = json!({"mode":mode,"mpv":mpv.get_property::<String>("mpv-version").ok(),"ffmpeg":mpv.get_property::<String>("ffmpeg-version").ok(),"wallSeconds":start.elapsed().as_secs_f64(),"frames":f.count,"maxFrameGapMs":gaps.iter().copied().fold(0_f64,f64::max)*1000.,"records":records,"frameTimes":f.times});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}",
        json!({"directory":directory,"frames":f.count,"maxFrameGapMs":report["maxFrameGapMs"],"wallSeconds":report["wallSeconds"]})
    );
    Ok(())
}
fn snapshot(mpv: &Mpv, frames: &Mutex<Frames>, start: &Instant) -> Value {
    json!({"wall":start.elapsed().as_secs_f64(),"position":mpv.get_property::<f64>("time-pos").ok(),"paused":mpv.get_property::<bool>("pause").ok(),"frames":frames.lock().unwrap().count})
}
fn diff(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return -1.;
    }
    a.iter()
        .zip(b)
        .map(|(a, b)| (*a as f64 - *b as f64).abs())
        .sum::<f64>()
        / a.len() as f64
}
fn save_ppm(path: &std::path::Path, pixels: &[u8]) -> Result<()> {
    let mut f = std::fs::File::create(path)?;
    write!(f, "P6\n640 360\n255\n")?;
    f.write_all(pixels)?;
    Ok(())
}

fn signature(pixels: &[u8]) -> u64 {
    pixels
        .iter()
        .step_by(23)
        .fold(1469598103934665603_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
        })
}
