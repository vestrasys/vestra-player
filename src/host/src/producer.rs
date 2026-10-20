//! Finite trusted-plugin binary AV transport; no emulator business.
#![forbid(unsafe_code)]
use crate::{
    RpcError,
    producer_audio::{Audio, Push, Writer},
    producer_binary::{self as wire, Kind, Spec},
};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
const SAFE: u64 = 9_007_199_254_740_991;
struct Video {
    epoch: u64,
    time: u64,
    pixels: Vec<u8>,
}
#[derive(Default)]
struct Shared {
    video: Mutex<VecDeque<Video>>,
    received: Mutex<(u64, u64)>,
    desired_epoch: AtomicU64,
    error: Mutex<Option<String>>,
}
struct Producer {
    owner: u64,
    spec: Spec,
    audio: Audio,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _directory: tempfile::TempDir,
    epoch: u64,
    last_video: Option<u64>,
}
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Default)]
pub struct Producers {
    items: BTreeMap<u64, Producer>,
    active: Option<u64>,
    next: u64,
    retiring: Vec<JoinHandle<()>>,
    /// The output device generated streams play on (`None` = system default).
    device: Option<String>,
}
fn failure(message: impl ToString) -> RpcError {
    RpcError::new("producer", message.to_string())
}
impl Producers {
    pub fn prepare(&mut self, owner: u64, params: &Value) -> Result<Value, RpcError> {
        self.reap();
        if self.items.len() >= 2
            // A provider may prepare its next source while the old source is
            // still active. Only one uncommitted candidate per owner is kept;
            // the global two-stream limit still bounds device/queue resources.
            || self
                .items
                .iter()
                .any(|(id, p)| p.owner == owner && self.active != Some(*id))
            || self.retiring.len() >= 8
        {
            return Err(RpcError::new("busy", "producer capacity reached"));
        }
        let width = params["width"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| RpcError::invalid("width required"))?;
        let height = params["height"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| RpcError::invalid("height required"))?;
        let spec = Spec { width, height };
        spec.validate().map_err(failure)?;
        #[cfg(not(unix))]
        {
            let _ = (owner, spec);
            Err(RpcError::new(
                "unsupported",
                "finite producer transport requires Unix sockets on this build",
            ))
        }
        #[cfg(unix)]
        {
            let directory = tempfile::Builder::new()
                .prefix("vestra-av-")
                .tempdir_in("/tmp")
                .map_err(failure)?;
            let path = directory.path().join("av.sock");
            let listener = UnixListener::bind(&path).map_err(failure)?;
            listener.set_nonblocking(true).map_err(failure)?;
            let (audio, writer) = Audio::open(self.device.as_deref()).map_err(failure)?;
            audio.reset(1, true, 0).map_err(failure)?;
            let shared = Arc::new(Shared::default());
            shared.desired_epoch.store(1, Ordering::Release);
            *shared
                .received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = (1, 0);
            let stop = Arc::new(AtomicBool::new(false));
            let s = shared.clone();
            let ending = stop.clone();
            let worker = thread::Builder::new()
                .name("producer-binary".into())
                .spawn(move || {
                    if let Err(error) = receive(listener, writer, spec, &s, &ending)
                        && !ending.load(Ordering::Acquire)
                    {
                        *s.error
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(error.to_string().chars().take(1024).collect());
                    }
                })
                .map_err(failure)?;
            self.next = self
                .next
                .checked_add(1)
                .filter(|n| *n <= SAFE)
                .ok_or_else(|| RpcError::new("busy", "producer identities exhausted"))?;
            let id = self.next;
            self.items.insert(
                id,
                Producer {
                    owner,
                    spec,
                    audio,
                    shared,
                    stop,
                    thread: Some(worker),
                    _directory: directory,
                    epoch: 1,
                    last_video: None,
                },
            );
            Ok(
                json!({"producerId":id,"socket":path,"epoch":1,"sampleRate":48000,"channels":2,"sampleFormat":"f32le","videoFormat":"rgba8","maxAudioFrames":4800,"audioCapacityFrames":9600,"videoCapacity":8}),
            )
        }
    }
    pub fn state(&self, id: u64, owner: u64) -> Result<Value, RpcError> {
        let p = self
            .items
            .get(&id)
            .filter(|p| p.owner == owner)
            .ok_or_else(|| RpcError::new("unavailable", "producer absent"))?;
        let a = p.audio.facts();
        Ok(
            json!({"producerId":id,"epoch":a.epoch,"acknowledged":a.acknowledged,"paused":a.paused,"consumedFrames":a.consumed_frames,"underrunFrames":a.underrun_frames,"deviceErrors":a.device_errors,"health":a.health,"xrunEvents":a.xruns,"deviceChangeEvents":a.device_changes,"errorDetails":a.error_details,"errorOverflow":a.error_overflow,"buffer":a.buffer,"receivedFrames":p.shared.received.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1,"hasVideo":p.shared.video.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|v|v.epoch==a.epoch && v.time<=a.consumed_frames),"error":*p.shared.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner),"active":self.active==Some(id)}),
        )
    }
    pub fn ready(&self, id: u64, owner: u64) -> Result<(), RpcError> {
        if self.active == Some(id) {
            return Err(RpcError::new("conflict", "producer already active"));
        }
        let state = self.state(id, owner)?;
        if !state["error"].is_null() {
            return Err(failure(state["error"].to_string()));
        }
        if !matches!(
            state["health"].as_str(),
            Some("ok" | "recoverableXrun" | "deviceChanged")
        ) || state["acknowledged"] != true
            || state["receivedFrames"].as_u64().unwrap_or(0) < 1024
            || state["hasVideo"] != true
        {
            return Err(RpcError::new(
                "unavailable",
                "producer first AV buffers are preparing",
            ));
        }
        Ok(())
    }
    pub fn activate(&mut self, id: u64, paused: bool) -> Result<(), RpcError> {
        if self.active != Some(id) {
            self.stop_active();
        }
        let p = self
            .items
            .get_mut(&id)
            .ok_or_else(|| failure("producer absent"))?;
        p.audio.set_paused(p.epoch, paused).map_err(failure)?;
        self.active = Some(id);
        Ok(())
    }
    /// R-H9: every stream moves to the chosen output device (or the default).
    pub fn follow_device(&mut self, device: Option<&str>) {
        self.device = device.map(str::to_owned);
        for p in self.items.values_mut() {
            if let Err(error) = p.audio.retarget(device) {
                eprintln!("host.error producer audio device: {error:#}");
            }
        }
    }
    pub fn output(&mut self, volume: f64, muted: bool) {
        for p in self.items.values() {
            let _ = p
                .audio
                .set_output((volume / 100.).clamp(0., 2.) as f32, muted);
        }
    }
    pub fn control_target(&self) -> Result<(u64, u64, u64, u64), RpcError> {
        let id = self.active.ok_or_else(|| failure("no active producer"))?;
        let p = self
            .items
            .get(&id)
            .ok_or_else(|| failure("producer absent"))?;
        let facts = p.audio.facts();
        Ok((id, p.owner, p.epoch, facts.consumed_frames))
    }
    pub fn acknowledge_control(&self, id: u64, epoch: u64, paused: bool) -> Result<(), RpcError> {
        let p = self
            .items
            .get(&id)
            .filter(|p| self.active == Some(id) && p.epoch == epoch)
            .ok_or_else(|| RpcError::new("stale", "producer control changed"))?;
        p.audio.set_paused(epoch, paused).map_err(failure)
    }
    pub fn audio_acknowledged(&self, id: u64, epoch: u64, paused: bool) -> bool {
        self.items.get(&id).is_some_and(|p| {
            let f = p.audio.facts();
            self.active == Some(id)
                && f.epoch == epoch
                && f.paused == paused
                && f.acknowledged
                && !f.health.fatal()
        })
    }
    pub fn active(&self) -> Option<u64> {
        self.active
    }
    pub fn active_owner(&self) -> Option<u64> {
        self.active
            .and_then(|id| self.items.get(&id))
            .map(|p| p.owner)
    }
    pub fn active_health(&self) -> Option<Value> {
        let p = self.items.get(&self.active?)?;
        let f = p.audio.facts();
        Some(
            json!({"status":f.health,"deviceErrors":f.device_errors,"xrunEvents":f.xruns,"deviceChangeEvents":f.device_changes,"errorDetails":f.error_details,"errorOverflow":f.error_overflow,"buffer":f.buffer}),
        )
    }
    pub fn active_failure(&self) -> Option<String> {
        let p = self.items.get(&self.active?)?;
        if let Some(error) = p
            .shared
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Some(error);
        }
        let f = p.audio.facts();
        f.health
            .fatal()
            .then(|| format!("producer audio {:?}: {:?}", f.health, f.error_details))
    }
    pub fn active_facts(&self) -> Option<(bool, f64)> {
        let p = self.items.get(&self.active?)?;
        let f = p.audio.facts();
        f.acknowledged
            .then_some((f.paused, f.consumed_frames as f64 / 48000.))
    }
    pub fn pause(&mut self, paused: bool) -> Result<(), RpcError> {
        let p = self
            .items
            .get_mut(&self.active.ok_or_else(|| failure("no active producer"))?)
            .ok_or_else(|| failure("producer absent"))?;
        if paused {
            let f = p.audio.facts();
            p.epoch = p
                .epoch
                .checked_add(1)
                .filter(|n| *n <= SAFE)
                .ok_or_else(|| failure("producer epoch exhausted"))?;
            p.shared.desired_epoch.store(p.epoch, Ordering::Release);
            *p.shared
                .received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = (p.epoch, 0);
            p.audio
                .reset(p.epoch, true, f.consumed_frames)
                .map_err(failure)?;
            p.shared
                .video
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();

            p.last_video = None;
        } else {
            p.audio.set_paused(p.epoch, false).map_err(failure)?;
        }
        Ok(())
    }
    pub fn release(&mut self, owner: u64, id: u64) -> Result<(), RpcError> {
        if let Some(p) = self.items.get(&id)
            && p.owner != owner
        {
            return Err(RpcError::new("unavailable", "producer absent"));
        }
        if self.active == Some(id) {
            return Err(RpcError::new("conflict", "close the source before release"));
        }
        self.retire(id);
        Ok(())
    }
    pub fn stop_active(&mut self) {
        if let Some(id) = self.active.take() {
            self.retire(id);
        }
    }
    fn retire(&mut self, id: u64) {
        if let Some(mut p) = self.items.remove(&id) {
            p.stop.store(true, Ordering::Release);
            if let Some(t) = p.thread.take() {
                self.retiring.push(t);
            }
        }
        self.reap();
    }
    pub fn deactivate(&mut self) {
        if let Some(id) = self.active.take()
            && let Some(p) = self.items.get(&id)
        {
            let _ = p.audio.set_paused(p.epoch, true);
        }
    }
    pub fn frame(&mut self) -> Option<Frame> {
        self.reap();
        let p = self.items.get_mut(&self.active?)?;
        let a = p.audio.facts();
        let mut queue = p
            .shared
            .video
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut frame = None;
        while queue
            .front()
            .is_some_and(|v| v.epoch != a.epoch || v.time <= a.consumed_frames)
        {
            let v = queue.pop_front()?;
            if v.epoch == a.epoch && v.time <= a.consumed_frames {
                frame = Some(v);
            }
        }
        let v = frame?;
        if p.last_video == Some(v.time) {
            return None;
        }
        p.last_video = Some(v.time);
        let mut rgb = Vec::with_capacity(p.spec.width as usize * p.spec.height as usize * 3);
        for rgba in v.pixels.as_chunks::<4>().0 {
            rgb.extend_from_slice(&rgba[..3]);
        }
        Some(Frame {
            width: p.spec.width,
            height: p.spec.height,
            pixels: rgb,
        })
    }
    pub fn disable(&mut self, owner: u64) {
        if self.active_owner() == Some(owner) {
            self.deactivate();
        }
        let ids = self
            .items
            .iter()
            .filter_map(|(id, p)| (p.owner == owner).then_some(*id))
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(mut p) = self.items.remove(&id) {
                p.stop.store(true, Ordering::Release);
                if let Some(t) = p.thread.take() {
                    self.retiring.push(t);
                }
            }
        }
        self.reap();
    }
    fn reap(&mut self) {
        let mut i = 0;
        while i < self.retiring.len() {
            if self.retiring[i].is_finished() {
                let _ = self.retiring.swap_remove(i).join();
            } else {
                i += 1;
            }
        }
    }
}
impl Drop for Producers {
    fn drop(&mut self) {
        for p in self.items.values_mut() {
            p.stop.store(true, Ordering::Release);
            if let Some(t) = p.thread.take() {
                self.retiring.push(t);
            }
        }
        self.items.clear();
        for t in self.retiring.drain(..) {
            let _ = t.join();
        }
    }
}
#[cfg(unix)]
fn read_exact(
    stream: &mut UnixStream,
    buffer: &mut [u8],
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    let mut offset = 0;
    while offset < buffer.len() {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        match stream.read(&mut buffer[offset..]) {
            Ok(0) => anyhow::bail!("producer transport closed"),
            Ok(n) => offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}
#[cfg(unix)]
fn receive(
    listener: UnixListener,
    mut writer: Writer,
    spec: Spec,
    shared: &Shared,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let mut stream = loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(e.into()),
        }
    };
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut epoch = 0;
    let mut sequence = None;
    let mut video_time = None;
    loop {
        let mut bytes = [0; 32];
        if !read_exact(&mut stream, &mut bytes, stop)? {
            return Ok(());
        }
        let h = wire::decode_header(&bytes, spec).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if h.epoch > shared.desired_epoch.load(Ordering::Acquire) {
            anyhow::bail!("producer future epoch");
        }
        if h.epoch < epoch {
            anyhow::bail!("producer stale epoch");
        }
        if h.epoch > epoch {
            epoch = h.epoch;
            sequence = None;
            video_time = None;
        }
        if sequence.is_some_and(|n| h.sequence <= n) {
            anyhow::bail!("producer sequence must increase");
        }
        sequence = Some(h.sequence);
        let mut payload = vec![0; h.payload_bytes as usize];
        if !read_exact(&mut stream, &mut payload, stop)? {
            return Ok(());
        }
        match h.kind {
            Kind::Audio => {
                let samples =
                    wire::decode_audio(&payload).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let frames = samples.as_chunks::<2>().0;
                loop {
                    if stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    match writer.push(h.epoch, h.sample_time, frames)? {
                        Push::Accepted => {
                            let mut received = shared
                                .received
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if shared.desired_epoch.load(Ordering::Acquire) == h.epoch
                                && received.0 == h.epoch
                            {
                                received.1 += frames.len() as u64;
                            }
                            break;
                        }
                        Push::Full | Push::AwaitEpoch => thread::sleep(Duration::from_millis(5)),
                        Push::Stale => break,
                    }
                }
            }
            Kind::Video => {
                if video_time.is_some_and(|t| h.sample_time < t) {
                    anyhow::bail!("video sample clock must not regress");
                }
                video_time = Some(h.sample_time);
                let mut video = shared
                    .video
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                while video.len() >= 8 {
                    video.pop_front();
                }
                if h.epoch != shared.desired_epoch.load(Ordering::Acquire) {
                    continue;
                }
                video.push_back(Video {
                    epoch: h.epoch,
                    time: h.sample_time,
                    pixels: payload,
                });
            }
        }
    }
}
