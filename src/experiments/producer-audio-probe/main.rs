//! Finite real-device CPAL prerequisite. No host integration or acoustic claim.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::json;
use std::{
    fs,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const RATE: u32 = 48000;
const CHANNELS: usize = 2;
fn pcm(path: &Path) -> Result<Vec<f32>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 || bytes.len() % 8 != 0 {
        bail!("bounded stereo f32le PCM required");
    }
    let values = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect::<Vec<_>>();
    if values.len() < RATE as usize * CHANNELS
        || values.iter().any(|v| !v.is_finite() || v.abs() > 1.)
    {
        bail!("one second finite normalized PCM required");
    }
    Ok(values)
}
fn signature(samples: &[f32]) -> u64 {
    samples
        .iter()
        .take(64)
        .fold(14695981039346656037u64, |h, v| {
            (h ^ u64::from(v.to_bits())).wrapping_mul(1099511628211)
        })
}
fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        bail!("producer-audio-probe FIRST.f32le SECOND.f32le RESULT.json");
    }
    let samples = Arc::new([pcm(Path::new(&args[0]))?, pcm(Path::new(&args[1]))?]);
    let device = cpal::default_host()
        .default_output_device()
        .context("default output device unavailable")?;
    let supported = device
        .supported_output_configs()?
        .find(|c| {
            c.channels() == 2
                && c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate() <= RATE
                && c.max_sample_rate() >= RATE
        })
        .context("48kHz stereo f32 output unsupported")?;
    let config = supported.with_sample_rate(RATE).config();
    let phase = Arc::new(AtomicU8::new(0));
    let observed = Arc::new(AtomicU8::new(255));
    let frames = Arc::new(std::array::from_fn::<_, 3, _>(|_| AtomicU64::new(0)));
    let nonzero = Arc::new(std::array::from_fn::<_, 3, _>(|_| AtomicU64::new(0)));
    let callbacks = Arc::new(AtomicU64::new(0));
    let max_gap = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let first_signatures = Arc::new(std::array::from_fn::<_, 3, _>(|_| AtomicU64::new(0)));
    let signatures = first_signatures.clone();
    let (p, o, f, n, c, g, pcm) = (
        phase.clone(),
        observed.clone(),
        frames.clone(),
        nonzero.clone(),
        callbacks.clone(),
        max_gap.clone(),
        samples.clone(),
    );
    let error_count = errors.clone();
    let (error_send, error_receive) = std::sync::mpsc::sync_channel(8);
    let error_phase = phase.clone();
    let mut cursor = 0usize;
    let mut old = 255u8;
    let mut last = None::<Instant>;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _| {
            let now = Instant::now();
            if let Some(last) = last {
                g.fetch_max(
                    now.duration_since(last).as_nanos().min(u64::MAX as u128) as u64,
                    Ordering::Relaxed,
                );
            }
            last = Some(now);
            let phase = p.load(Ordering::Acquire).min(2);
            let first_buffer = phase != old;
            if first_buffer {
                cursor = 0;
                old = phase;
            }
            if phase == 1 {
                data.fill(0.);
            } else {
                let source = &pcm[usize::from(phase == 2)];
                for sample in data.iter_mut() {
                    *sample = source.get(cursor).copied().unwrap_or(0.);
                    cursor = cursor.saturating_add(1);
                }
            }
            let index = usize::from(phase);
            if first_buffer {
                signatures[index].store(signature(data), Ordering::Relaxed);
            }
            f[index].fetch_add((data.len() / CHANNELS) as u64, Ordering::Relaxed);
            n[index].fetch_add(
                data.iter().filter(|v| **v != 0.).count() as u64,
                Ordering::Relaxed,
            );
            c.fetch_add(1, Ordering::Relaxed);
            // Acknowledgement is after this complete buffer has been filled. It is
            // not an acoustic/device-buffer drain fence.
            o.store(phase, Ordering::Release);
        },
        move |error| {
            error_count.fetch_add(1, Ordering::Relaxed);
            let _ = error_send.try_send((error_phase.load(Ordering::Relaxed), error));
        },
        None,
    )?;
    let started = Instant::now();
    stream.play()?;
    let wait_phase = |value| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while observed.load(Ordering::Acquire) != value {
            if Instant::now() >= deadline {
                bail!("device callback did not acknowledge phase {value}");
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    };
    wait_phase(0)?;
    thread::sleep(Duration::from_millis(600));
    phase.store(1, Ordering::Release);
    wait_phase(1)?;
    let before = frames[0].load(Ordering::Relaxed);
    thread::sleep(Duration::from_millis(400));
    let paused_stable = frames[0].load(Ordering::Relaxed) == before;
    phase.store(2, Ordering::Release);
    wait_phase(2)?;
    thread::sleep(Duration::from_millis(600));
    stream.pause()?;
    drop(stream);
    let frame_counts = frames
        .iter()
        .map(|v| v.load(Ordering::Relaxed))
        .collect::<Vec<_>>();
    let nonzero_counts = nonzero
        .iter()
        .map(|v| v.load(Ordering::Relaxed))
        .collect::<Vec<_>>();
    let actual_signatures = first_signatures
        .iter()
        .map(|v| v.load(Ordering::Relaxed))
        .collect::<Vec<_>>();
    let expected_signatures = [
        signature(&samples[0]),
        signature(&[0.; 64]),
        signature(&samples[1]),
    ];
    let passed = actual_signatures == expected_signatures
        && actual_signatures[0] != actual_signatures[2]
        && paused_stable
        && frame_counts.iter().all(|v| *v > 1000)
        && nonzero_counts[0] > 0
        && nonzero_counts[1] == 0
        && nonzero_counts[2] > 0
        && errors.load(Ordering::Relaxed) == 0;
    let error_details = error_receive
        .try_iter()
        .map(|(phase, error)| json!({"phase":phase,"message":error.to_string()}))
        .collect::<Vec<_>>();
    let result = json!({"pass":passed,"cpal":"0.18.2","device":device.to_string(),"sampleRate":RATE,"channels":CHANNELS,"sampleFormat":"f32","wallSeconds":started.elapsed().as_secs_f64(),"callbackCount":callbacks.load(Ordering::Relaxed),"maxCallbackGapMs":max_gap.load(Ordering::Relaxed) as f64/1e6,"phaseFrames":frame_counts,"phaseNonzeroSamples":nonzero_counts,"oldEpochConsumedWhilePaused":!paused_stable,"deviceErrors":errors.load(Ordering::Relaxed),"errorDetails":error_details,"first64SampleSignatures":actual_signatures,"expectedFirst64SampleSignatures":expected_signatures,"limits":["Callback-filled buffers and counters, not acoustic output/latency/continuity proof.","Preloaded bounded decoded PCM; binary transport/backpressure/host source switching are not implemented.","Pause writes zero and stops source consumption; new epoch resets the callback cursor after acknowledgement, not a hardware drain fence."]});
    fs::write(Path::new(&args[2]), serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    if !passed {
        bail!("finite device callback checks failed");
    }
    Ok(())
}
