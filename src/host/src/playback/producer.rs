//! Finite trusted-plugin binary AV transport; no emulator business.
#![forbid(unsafe_code)]
use crate::platform::{AvListener, AvStream};
use crate::{
    host::RpcError,
    playback::producer_audio::{Audio, Push, Writer},
    playback::producer_binary::{self as wire, Kind, Spec},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
    /// When the plugin closed the frame connection between two frames.
    closed: Mutex<Option<Instant>>,
}
/// How long a connection closed between frames may wait for its source to
/// be closed or replaced before it counts as a failure. A plugin that stops
/// its stream closes the connection and the source together (SDK 1.1.2 and
/// earlier: connection first), so neither order shows "打开失败"; a plugin
/// that crashed while playing is still reported, this much later.
const CLOSE_GRACE: Duration = Duration::from_millis(500);
/// The frame connection ended cleanly at a frame boundary.
#[derive(Debug)]
struct TransportClosed;
impl std::fmt::Display for TransportClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("producer transport closed")
    }
}
impl std::error::Error for TransportClosed {}
/// The failure to report: a protocol or I/O error at once; a clean close
/// only once it has lasted `CLOSE_GRACE` with the source still selected.
fn reported_failure(error: Option<String>, closed_for: Option<Duration>) -> Option<String> {
    error.or_else(|| {
        closed_for
            .filter(|d| *d >= CLOSE_GRACE)
            .map(|_| TransportClosed.to_string())
    })
}
impl Shared {
    fn failure(&self) -> Option<String> {
        let error = self
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let closed = *self
            .closed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reported_failure(error, closed.map(|at| at.elapsed()))
    }
}
struct Producer {
    owner: u64,
    spec: Spec,
    audio: Audio,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
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
        {
            // A Unix socket, or loopback TCP with a token (platform module).
            let (listener, address) = crate::platform::av_listen().map_err(failure)?;
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
                        if error.is::<TransportClosed>() {
                            *s.closed
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                Some(Instant::now());
                            return;
                        }
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
                    epoch: 1,
                    last_video: None,
                },
            );
            let mut prepared = json!({"producerId":id,"epoch":1,"sampleRate":48000,"channels":2,"sampleFormat":"f32le","videoFormat":"rgba8","maxAudioFrames":4800,"audioCapacityFrames":9600,"videoCapacity":8});
            // `socket` (and `token` for loopback TCP).
            if let (Some(prepared), Some(address)) = (prepared.as_object_mut(), address.as_object())
            {
                prepared.extend(address.clone());
            }
            Ok(prepared)
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
            json!({"producerId":id,"epoch":a.epoch,"acknowledged":a.acknowledged,"paused":a.paused,"consumedFrames":a.consumed_frames,"underrunFrames":a.underrun_frames,"deviceErrors":a.device_errors,"health":a.health,"xrunEvents":a.xruns,"deviceChangeEvents":a.device_changes,"errorDetails":a.error_details,"errorOverflow":a.error_overflow,"buffer":a.buffer,"receivedFrames":p.shared.received.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1,"hasVideo":p.shared.video.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|v|v.epoch==a.epoch && v.time<=a.consumed_frames),"error":p.shared.failure(),"active":self.active==Some(id)}),
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
                diag!("host.error producer audio device: {error:#}");
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
        if let Some(error) = p.shared.failure() {
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
/// `boundary`: the buffer starts a frame, so a close before its first byte
/// is the plugin ending the stream (`TransportClosed`), not a cut frame.
fn read_exact(
    stream: &mut AvStream,
    buffer: &mut [u8],
    stop: &AtomicBool,
    boundary: bool,
) -> anyhow::Result<bool> {
    let mut offset = 0;
    while offset < buffer.len() {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        match stream.read(&mut buffer[offset..]) {
            Ok(0) if boundary && offset == 0 => return Err(TransportClosed.into()),
            Ok(0) => anyhow::bail!("producer transport closed mid-frame"),
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
fn receive(
    listener: AvListener,
    mut writer: Writer,
    spec: Spec,
    shared: &Shared,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let mut stream = loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        match listener.accept()? {
            Some(stream) => break stream,
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut epoch = 0;
    let mut sequence = None;
    let mut video_time = None;
    loop {
        let mut bytes = [0; 32];
        if !read_exact(&mut stream, &mut bytes, stop, true)? {
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
        if !read_exact(&mut stream, &mut payload, stop, false)? {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin stopping its stream may close the connection a moment before
    /// the source (SDK 1.1.2 and earlier): no failure unless the source stays
    /// selected for `CLOSE_GRACE`. Real errors are reported at once.
    #[test]
    fn a_clean_close_waits_for_the_source_to_go_and_errors_do_not() {
        assert_eq!(reported_failure(None, None), None);
        assert_eq!(
            reported_failure(None, Some(Duration::from_millis(10))),
            None
        );
        assert_eq!(
            reported_failure(None, Some(CLOSE_GRACE)).as_deref(),
            Some("producer transport closed")
        );
        assert_eq!(
            reported_failure(Some("producer stale epoch".into()), None).as_deref(),
            Some("producer stale epoch")
        );
        assert_eq!(
            reported_failure(Some("producer transport closed mid-frame".into()), None).as_deref(),
            Some("producer transport closed mid-frame")
        );
    }
}
