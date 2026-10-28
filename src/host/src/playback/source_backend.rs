//! Candidate source contract over actual libmpv entry identities.
use crate::{
    RpcError,
    playback::session::{
        Admission, Control, EndReason, IntentTicket, OperationBook, Phase, SourceEvent,
        SourceIntent, SourceSession,
    },
};
use rsmpv::{EndFileReason, Event as MpvEvent, Mpv, Node};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs::File, path::Path, sync::Arc};

/// A local file played under a lease that snapshot/clip workers read, with
/// libmpv reading a second handle verified to be the leased object
/// (`Lease::matches_file`). Unix: libmpv reads that handle as `fd://`.
/// Windows: an fd cannot cross into libmpv's C runtime, so it reads it
/// through the `vestra-lease://` stream protocol (lease_stream.rs). libmpv
/// sees no path either way; leased loads force the lavf demuxer
/// (`load_options`), so mkv ordered chapters/editions are not followed.
struct LocalMedia {
    // This open description is used exclusively by libmpv, never by workers.
    #[cfg(unix)]
    playback: File,
    #[cfg(windows)]
    playback: crate::playback::lease_stream::Registration,
    lease: Arc<crate::playback::source_lease::Lease>,
    /// Windows: the file is on a network file system, asked of the
    /// playback handle (`lease_stream::on_network`); libmpv sees it
    /// itself through `fd://` on Unix.
    remote: bool,
}
impl LocalMedia {
    /// Ordinary self-contained media on a local path: the lease route. Keep
    /// path-relative playlist/disc/reference loaders on their path route.
    fn eligible(source: &str) -> bool {
        let extension = Path::new(source)
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        matches!(
            extension.as_str(),
            "mp4"
                | "m4v"
                | "mov"
                | "mkv"
                | "webm"
                | "avi"
                | "mp3"
                | "m4a"
                | "flac"
                | "wav"
                | "ogg"
                | "opus"
        ) && !source.contains("://")
            && Path::new(source).is_file()
    }
    /// `streams`: Windows' lease protocol is registered on this player.
    fn open(source: &str, streams: bool) -> Result<Option<Arc<Self>>, String> {
        if !Self::eligible(source) || (cfg!(windows) && !streams) {
            return Ok(None);
        }
        let opened = (|| -> Result<(File, crate::playback::source_lease::Lease), String> {
            let playback = File::open(source).map_err(|e| e.to_string())?;
            let lease = crate::playback::source_lease::Lease::open(Path::new(source))
                .map_err(|e| e.to_string())?;
            if !lease.matches_file(&playback).map_err(|e| e.to_string())? {
                return Err("source changed between independent opens".into());
            }
            Ok((playback, lease))
        })();
        let (playback, lease) = match opened {
            Ok(opened) => opened,
            // Windows: libmpv opens the path itself and reports its own
            // error; the file plays without snapshots if it can.
            Err(error) if cfg!(windows) => {
                diag!("host.error source lease: {error}");
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        #[cfg(windows)]
        let remote = crate::playback::lease_stream::on_network(&playback);
        #[cfg(not(windows))]
        let remote = false;
        #[cfg(windows)]
        let playback = crate::playback::lease_stream::Registration::new(playback);
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (playback, lease, remote);
            return Ok(None);
        }
        #[cfg(any(unix, windows))]
        Ok(Some(Arc::new(Self {
            playback,
            lease: Arc::new(lease),
            remote,
        })))
    }
    #[cfg(unix)]
    fn url(&self) -> String {
        use std::os::fd::AsRawFd;
        format!("fd://{}", self.playback.as_raw_fd())
    }
    #[cfg(windows)]
    fn url(&self) -> String {
        self.playback.url()
    }
    #[cfg(not(any(unix, windows)))]
    fn url(&self) -> String {
        unreachable!("local leased sources unavailable on this target")
    }
}

#[derive(Default)]
pub struct Sources {
    session: SourceSession,
    resolvers: crate::playback::source_resolvers::Registry,
    resolutions: BTreeMap<u64, Resolution>,
    resolution_next: u64,
    resolution_events: Vec<(u64, String, Value)>,
    books: BTreeMap<u64, OperationBook>,
    entries: BTreeMap<i64, IntentTicket>,
    active: Option<(i64, IntentTicket)>,
    loaded: Option<(i64, IntentTicket)>,
    eof_rearm: bool,
    local_media: BTreeMap<i64, Arc<LocalMedia>>,
    current_local: Option<(u64, Arc<LocalMedia>)>,
    lease_ready: Option<(i64, u64)>,
    /// Windows: the lease stream protocol is registered on the player.
    lease_streams: bool,
    /// `player.setLoop`: (a, b, count) as requested; mpv counts down in
    /// `remaining-ab-loops`, which is not passed on (ruling R-M2).
    ab_loop: Option<(f64, f64, Option<u64>)>,
    /// The source generation the A-B loop belongs to (R-H13).
    loop_generation: u64,
    /// `player.setAudioDevice` for this run (`None` = system default), the
    /// device in use (`None` = default), and when presence was last checked.
    audio_requested: Option<String>,
    /// The plugin whose `setAudioDevice` is in effect; its stop reverts it.
    audio_owner: Option<u64>,
    audio_active: Option<String>,
    audio_checked: Option<std::time::Instant>,
    /// The `Control::Seek` mpv is still running (`SeekFlight`).
    seek_flight: SeekFlight,
    /// Repeats of a running seek not sent (tests read it).
    #[cfg(test)]
    seek_repeats_skipped: usize,
    #[cfg(feature = "producer-audio")]
    producers: crate::playback::producer::Producers,
    #[cfg(feature = "producer-audio")]
    provider_ticket: Option<IntentTicket>,
    #[cfg(feature = "producer-audio")]
    provider_health: Option<Value>,
    #[cfg(feature = "producer-audio")]
    controls: crate::playback::producer_controls::Controls,
    #[cfg(feature = "producer-audio")]
    awaiting_audio: Option<crate::playback::producer_controls::Pending>,
    #[cfg(feature = "producer-audio")]
    control_origin: Option<std::time::Instant>,
    #[cfg(feature = "producer-audio")]
    pending_error: Option<(IntentTicket, String)>,
    #[cfg(feature = "producer-audio")]
    last_ack: Option<(u64, crate::playback::producer_controls::Notice, Value)>,
    #[cfg(feature = "producer-audio")]
    input: Option<InputBinding>,
    #[cfg(feature = "producer-audio")]
    input_epoch: u64,
}
/// The last `Control::Seek` sent to mpv while it is still running, so that a
/// repeat of the same target in the same mode is not sent again
/// (seekbar-speed round).
///
/// The seek bar sends the same target on press and on release, so a click is
/// two identical seeks about 100 ms apart. mpv starts the second one from the
/// keyframe again: after the first one's frame, or after 0.3 s when the first
/// is still decoding (`execute_queued_seek`), which adds 0.3 to 0.45 s on 4K
/// clips with long GOPs and doubles the decode on short ones. Skipping the
/// repeat lands on the same frame sooner. A repeat after the first seek
/// finished (its `PlaybackRestart`) is sent as before.
///
/// The key is (entry, target, keyframes): a keyframe seek lands on the last
/// keyframe at or before the target, an exact seek on the target itself, so
/// they are different seeks even to the same number. An exact seek to the
/// target of the keyframe seek still running is sent (and the other way
/// round), otherwise a plugin that previews with keyframes and lands exactly
/// would stay on the keyframe.
#[derive(Default)]
struct SeekFlight(Option<(i64, u64, bool)>);
impl SeekFlight {
    /// True when `seconds` in this mode repeats the seek still running on this entry.
    fn repeats(&self, entry: Option<i64>, seconds: f64, keyframes: bool) -> bool {
        entry.is_some_and(|entry| self.0 == Some((entry, seconds.to_bits(), keyframes)))
    }
    fn sent(&mut self, entry: Option<i64>, seconds: f64, keyframes: bool) {
        self.0 = entry.map(|entry| (entry, seconds.to_bits(), keyframes));
    }
    /// The seek finished, or anything else moved or replaced the source.
    fn clear(&mut self) {
        self.0 = None;
    }
}
/// The mpv `seek` for a `Control::Seek`: `absolute+exact` decodes from the
/// keyframe before `seconds` up to that frame; `absolute+keyframes` (SDK
/// 1.5.22 `Scrub.seek(seconds, { keyframes: true })`, the seek bar with
/// "Jump to keyframes only") shows the last keyframe at or before `seconds`
/// at once, up to one keyframe interval early.
fn seek_command(seconds: f64, keyframes: bool) -> [String; 3] {
    [
        "seek".into(),
        seconds.to_string(),
        if keyframes {
            "absolute+keyframes"
        } else {
            "absolute+exact"
        }
        .into(),
    ]
}
struct Resolution {
    requester: u64,
    operation: u64,
    resolver: u64,
    resolver_id: String,
    source: String,
    paused: bool,
    ticket: IntentTicket,
    deadline: std::time::Instant,
}
#[cfg(feature = "producer-audio")]
struct InputBinding {
    delivery: crate::playback::producer_input_delivery::Delivery,
    keys: Vec<String>,
}
#[cfg(feature = "producer-audio")]
impl InputBinding {
    fn physical_key(
        &mut self,
        scope: &crate::playback::producer_input::Scope,
        code: &str,
        down: bool,
        now: u64,
    ) -> Result<(), crate::playback::producer_input_delivery::Error> {
        match self.delivery.key(scope, code, down, false, now) {
            Err(crate::playback::producer_input_delivery::Error::Busy) => {
                self.delivery.revoke(
                    crate::playback::producer_input::RevokeReason::TransportFailure,
                    now,
                )?;
                Err(crate::playback::producer_input_delivery::Error::Busy)
            }
            result => result.map(|_| ()),
        }
    }
}
impl Sources {
    #[cfg(feature = "producer-audio")]
    fn input_error(error: crate::playback::producer_input_delivery::Error) -> RpcError {
        use crate::playback::producer_input_delivery::Error;
        RpcError::new(
            match error {
                Error::Stale => "stale",
                Error::Unavailable => "unavailable",
                Error::Busy => "busy",
                Error::TimedOut => "timeout",
                _ => "invalid",
            },
            format!("producer input: {error:?}"),
        )
    }
    #[cfg(feature = "producer-audio")]
    pub fn input_routing(&self) -> Option<crate::playback::producer_input::Routing> {
        let input = self.input.as_ref()?;
        let notice = input.delivery.notice();
        (self.producers.active() == Some(notice.scope.producer_id)).then_some(
            crate::playback::producer_input::Routing {
                scope: notice.scope,
                keys: input.keys.clone(),
                captured: notice.captured,
            },
        )
    }
    #[cfg(feature = "producer-audio")]
    fn input_facts(&self) -> Value {
        self.input.as_ref().map_or(Value::Null, |input| json!({
            "state": input.delivery.notice(), "pending":input.delivery.pending(), "keys":input.keys,
        }))
    }
    #[cfg(feature = "producer-audio")]
    fn input_failed(&mut self, message: String) {
        if let Some(mut ticket) = self.provider_ticket {
            ticket.stamp = self.session.state().stamp;
            ticket.cycle = self.session.state().cycle;
            self.pending_error = Some((ticket, message));
        }
    }
    /// Called only by the host's native input adapter, never routed from plugins.
    #[cfg(feature = "producer-audio")]
    pub fn host_input(&mut self, params: &Value) -> Result<(), RpcError> {
        let now = self.now_ms();
        let paused = self
            .producers
            .active_facts()
            .is_none_or(|(paused, _)| paused);
        let result = match params["kind"].as_str() {
            Some("capture") if !paused && self.phase() == Phase::Ready => {
                let scope: crate::playback::producer_input::Scope =
                    serde_json::from_value(params["scope"].clone())
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                let input = self
                    .input
                    .as_mut()
                    .ok_or_else(|| RpcError::new("unavailable", "input is not declared"))?;
                if scope != input.delivery.notice().scope {
                    return Err(RpcError::new("stale", "input scope changed"));
                }
                input.delivery.capture(now).map(|_| ())
            }
            Some("key") if !paused => {
                let scope = serde_json::from_value(params["scope"].clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                let code = params["code"]
                    .as_str()
                    .ok_or_else(|| RpcError::invalid("physical code required"))?;
                let down = params["down"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("down required"))?;
                self.input
                    .as_mut()
                    .ok_or_else(|| RpcError::new("unavailable", "input is not declared"))?
                    .physical_key(&scope, code, down, now)
            }
            Some("revoke") => {
                let reason = serde_json::from_value(params["reason"].clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                if let Some(input) = &mut self.input {
                    input.delivery.revoke(reason, now).map(|_| ())
                } else {
                    return Ok(());
                }
            }
            _ => return Err(RpcError::new("unavailable", "input capture is inactive")),
        };
        result.map_err(|error| {
            if error == crate::playback::producer_input_delivery::Error::Busy
                && params["kind"] == "key"
            {
                // A physical transition cannot be retried by the OS. Replace
                // any full pending queue with a pinned release, rather than
                // silently losing a key-up and leaving the core pressed.
                return RpcError::new(
                    "input-overflow",
                    "physical transition queue unavailable; input revoked",
                );
            }
            if matches!(
                error,
                crate::playback::producer_input_delivery::Error::TimedOut
                    | crate::playback::producer_input_delivery::Error::Exhausted
            ) {
                self.input_failed(format!("producer input delivery: {error:?}"));
            }
            Self::input_error(error)
        })
    }
    /// A local accepted object bound to the actual current native load, not a path reopen.
    pub fn lease(
        &self,
        generation: u64,
    ) -> Result<Arc<crate::playback::source_lease::Lease>, RpcError> {
        if generation != self.generation() {
            return Err(RpcError::new("stale", "source generation changed"));
        }
        if !matches!(self.phase(), Phase::Ready | Phase::Ended) {
            return Err(RpcError::new("source-unavailable", "source is not ready"));
        }
        let (entry, ticket) = self.active.ok_or_else(|| {
            RpcError::new("source-unavailable", "native source binding unavailable")
        })?;
        if self.lease_ready != Some((entry, generation)) {
            return Err(RpcError::new(
                "source-unavailable",
                "seekable ordinary container is not confirmed",
            ));
        }
        let media = self
            .local_media
            .get(&entry)
            .filter(|_| ticket.stamp.generation == generation)
            .ok_or_else(|| RpcError::new("source-unavailable", "local source lease unavailable"))?;
        Ok(media.lease.clone())
    }
    pub fn phase(&self) -> Phase {
        self.session.state().phase
    }
    /// `audio.addTrack { path, select, expected? }` → the new track;
    /// `audio.selectTrack { id, expected? }` → null. Both act on the current
    /// media load only, conditional on the source stamp like other controls.
    /// A conditional control's `expected` stamp must still be the source's.
    fn check_expected(&self, params: &Value) -> Result<(), RpcError> {
        if !params["expected"].is_null() {
            let expected: crate::playback::session::SourceStamp =
                serde_json::from_value(params["expected"].clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
            if expected != self.session.state().stamp {
                return Err(RpcError::new("stale", "source changed"));
            }
        }
        Ok(())
    }
    fn producer(&self) -> bool {
        #[cfg(feature = "producer-audio")]
        return self.provider_ticket.is_some();
        #[cfg(not(feature = "producer-audio"))]
        false
    }
    /// `player.chapters` → `{ sourceId, chapters: [{ title, start }] }`;
    /// `player.setLoop { loop: { a, b, count? } | null, expected? }`.
    fn marks(&mut self, mpv: &Mpv, method: &str, params: &Value) -> Result<Value, RpcError> {
        let media = !self.producer() && self.presentation_load().is_some();
        if method == "player.chapters" {
            let chapters = match media.then(|| mpv.get_property::<Node>("chapter-list")) {
                Some(Ok(node)) => chapter_list(node),
                _ => Vec::new(),
            };
            return Ok(json!({"sourceId":self.generation(),"chapters":chapters}));
        }
        self.check_expected(params)?;
        if self.producer() {
            return Err(RpcError::new(
                "unsupported",
                "a generated stream cannot loop",
            ));
        }
        if !media {
            return Err(RpcError::new("unavailable", "A-B loops need ready media"));
        }
        let spec = &params["loop"];
        if spec.is_null() {
            self.ab_loop = None;
            clear_loop(mpv);
            return Ok(Value::Null);
        }
        if !mpv.get_property::<bool>("seekable").unwrap_or(false) {
            return Err(RpcError::new("unsupported", "the source cannot seek"));
        }
        let duration = mpv.get_property::<f64>("duration").unwrap_or(f64::NAN);
        let (a, b) = (
            spec["a"].as_f64().unwrap_or(f64::NAN),
            spec["b"].as_f64().unwrap_or(f64::NAN),
        );
        if !(a >= 0. && a < b && b <= duration + 0.001) {
            return Err(RpcError::invalid("loop needs 0 <= a < b <= duration"));
        }
        let count = match &spec["count"] {
            Value::Null => None,
            value => Some(
                value
                    .as_u64()
                    .filter(|n| (1..=1_000_000).contains(n))
                    .ok_or_else(|| RpcError::invalid("count must be 1..1000000"))?,
            ),
        };
        let backend = |e: rsmpv::Error| RpcError::new("backend", e.to_string());
        mpv.set_property("ab-loop-a", a).map_err(backend)?;
        mpv.set_property("ab-loop-b", b).map_err(backend)?;
        match count {
            Some(n) => mpv.set_property("ab-loop-count", n as i64),
            None => mpv.set_property("ab-loop-count", "inf"),
        }
        .map_err(backend)?;
        self.ab_loop = Some((a, b, count));
        self.loop_generation = self.generation();
        Ok(Value::Null)
    }
    /// `PlayerState.loop`. A new source clears the loop points (R-H13); a
    /// loop whose count mpv has used up ends.
    fn loop_state(&mut self, mpv: &Mpv) -> Value {
        if self.loop_generation != self.generation() {
            self.loop_generation = self.generation();
            self.ab_loop = None;
            clear_loop(mpv);
        }
        if self.ab_loop.is_some_and(|(.., count)| count.is_some())
            && mpv
                .get_property::<i64>("remaining-ab-loops")
                .is_ok_and(|remaining| remaining == 0)
        {
            self.ab_loop = None;
            clear_loop(mpv);
        }
        self.ab_loop.map_or(
            Value::Null,
            |(a, b, count)| json!({"a":a,"b":b,"count":count}),
        )
    }
    /// `player.audioDevices` → `[{ id, name }]`; `player.setAudioDevice { id }`.
    fn audio_device(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        if method == "player.audioDevices" {
            return Ok(json!(
                audio_devices(mpv)
                    .into_iter()
                    .map(|(id, name)| json!({"id":id,"name":name}))
                    .collect::<Vec<_>>()
            ));
        }
        self.audio_requested = match &params["id"] {
            Value::Null => None,
            Value::String(id)
                if !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control) =>
            {
                Some(id.clone())
            }
            _ => return Err(RpcError::invalid("id must be a device id or null")),
        };
        self.audio_owner = self.audio_requested.is_some().then_some(owner);
        self.audio_checked = None;
        self.follow_device(mpv);
        Ok(Value::Null)
    }
    /// R-A3/R-H9: play on the requested device while it is present, else on
    /// the system default; checked every second. mpv and generated streams
    /// follow the same choice.
    pub fn follow_device(&mut self, mpv: &Mpv) {
        if self
            .audio_checked
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        self.audio_checked = Some(std::time::Instant::now());
        let target = self
            .audio_requested
            .clone()
            .filter(|id| audio_devices(mpv).iter().any(|(present, _)| present == id));
        let wanted = target.as_deref().unwrap_or("auto");
        if mpv.get_property::<String>("audio-device").ok().as_deref() != Some(wanted)
            && let Err(error) = mpv.set_property("audio-device", wanted)
        {
            diag!("host.error audio-device {wanted}: {error}");
        }
        #[cfg(feature = "producer-audio")]
        self.producers.follow_device(target.as_deref());
        self.audio_active = target;
    }
    fn audio_track(&mut self, mpv: &Mpv, method: &str, params: &Value) -> Result<Value, RpcError> {
        self.check_expected(params)?;
        if self.producer() || self.presentation_load().is_none() {
            return Err(RpcError::new(
                "unavailable",
                "audio tracks need ready media",
            ));
        }
        let listed = |mpv: &Mpv| {
            crate::playback::media_tracks::audio_tracks(mpv)
                .map_err(|e| RpcError::new("backend", e.to_string()))
        };
        if method == "audio.selectTrack" {
            let id = params["id"]
                .as_i64()
                .ok_or_else(|| RpcError::invalid("id requires an integer"))?;
            if !listed(mpv)?.iter().any(|track| track.track_id == id) {
                return Err(RpcError::invalid("no such audio track"));
            }
            mpv.set_property("aid", id)
                .map_err(|e| RpcError::new("backend", e.to_string()))?;
            return Ok(Value::Null);
        }
        let path = params["path"]
            .as_str()
            .ok_or_else(|| RpcError::invalid("path requires a string"))?;
        if !Path::new(path).is_absolute() || path.contains("://") || !Path::new(path).is_file() {
            return Err(RpcError::invalid("path must be an existing absolute file"));
        }
        let select = params["select"].as_bool().unwrap_or(true);
        let before = listed(mpv)?;
        mpv.command(&["audio-add", path, if select { "select" } else { "auto" }])
            .map_err(|e| RpcError::new("backend", e.to_string()))?;
        let added = added_external(&before, &listed(mpv)?)
            .ok_or_else(|| RpcError::new("backend", "mpv did not list the added audio track"))?;
        Ok(json!(added))
    }
    /// Only call after an accepted actual FileLoaded event. Not a frame fence.
    pub fn ready_load(&self) -> Option<(i64, IntentTicket)> {
        self.active.filter(|(_, ticket)| {
            self.session.state().phase == Phase::Ready
                && ticket.stamp.generation == self.generation()
        })
    }
    /// Windows: lets `mpv` read leased local files (lease_stream.rs); until
    /// then, or if it fails, local files play by path without snapshots.
    /// Elsewhere nothing to do (`fd://`).
    pub fn register_lease_streams(&mut self, mpv: &Mpv) {
        #[cfg(windows)]
        match crate::playback::lease_stream::register(mpv) {
            Ok(()) => self.lease_streams = true,
            Err(error) => diag!("host.error lease stream protocol: {error}"),
        }
        #[cfg(not(windows))]
        let _ = mpv;
    }
    /// libmpv entry `entry` reads a leased local file (its container stream
    /// numbers are the worker's: the load forces lavf).
    pub fn leased(&self, entry: i64) -> bool {
        self.local_media.contains_key(&entry)
    }
    /// Actual FileLoaded binding survives an ended->seek playback cycle change.
    pub fn presentation_load(&self) -> Option<(i64, IntentTicket)> {
        self.loaded.filter(|(entry, ticket)| {
            matches!(self.phase(), Phase::Ready | Phase::Ended)
                && ticket.stamp.generation == self.generation()
                && self.active.is_some_and(|(active, _)| active == *entry)
        })
    }
    pub fn generation(&self) -> u64 {
        self.session.state().stamp.generation
    }

    #[cfg(feature = "producer-audio")]
    fn now_ms(&mut self) -> u64 {
        self.control_origin
            .get_or_insert_with(std::time::Instant::now)
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    #[cfg(feature = "producer-audio")]
    fn finish_provider_control(
        &mut self,
        pending: crate::playback::producer_controls::Pending,
        result: Result<(), String>,
    ) {
        if let Some(book) = self.books.get_mut(&pending.requester) {
            let _ = book.finish(pending.operation_id, result);
        }
    }
    #[cfg(feature = "producer-audio")]
    fn cancel_provider_control(&mut self, message: &str) {
        if let Some(p) = self.controls.cancel() {
            self.finish_provider_control(p, Err(message.into()));
        }
        if let Some(p) = self.awaiting_audio.take() {
            self.finish_provider_control(p, Err(message.into()));
        }
    }
    #[cfg(feature = "producer-audio")]
    pub fn tick_provider_controls(&mut self) {
        let now = self.now_ms();
        if self
            .input
            .as_ref()
            .is_some_and(|input| input.delivery.expired(now))
        {
            self.input_failed("producer physical-input acknowledgement timed out".into());
        }
        let expired = self.controls.expire(now);
        let audio_expired = self
            .awaiting_audio
            .as_ref()
            .is_some_and(|p| now >= p.deadline_ms);
        let expired = expired.or_else(|| {
            if audio_expired {
                self.awaiting_audio.take()
            } else {
                None
            }
        });
        if let Some(p) = expired {
            let ticket = p.ticket;
            self.finish_provider_control(p, Err("provider frame-bound control timed out".into()));
            self.pending_error = Some((ticket, "provider frame-bound control timed out".into()));
        }
        if let Some(p) = &self.awaiting_audio
            && self.producers.audio_acknowledged(
                p.notice.producer_id,
                p.notice.epoch,
                p.notice.paused,
            )
        {
            let p = self.awaiting_audio.take().unwrap();
            self.finish_provider_control(p, Ok(()));
        }
    }
    #[cfg(feature = "producer-audio")]
    pub fn producer_failure(&mut self) -> Option<String> {
        self.tick_provider_controls();
        let error = self
            .pending_error
            .take()
            .filter(|(ticket, _)| {
                ticket.stamp == self.session.state().stamp
                    && ticket.cycle == self.session.state().cycle
            })
            .map(|(_, error)| error)
            .or_else(|| self.producers.active_failure())?;
        self.cancel_provider_control(&error);
        self.provider_health = self.producers.active_health();
        if let Some(ticket) = self.provider_ticket {
            let position = self.producers.active_facts().map(|(_, p)| p);
            self.session.observe(ticket, true, position, None);
            self.session.event(SourceEvent::LoadFailed {
                ticket,
                message: error.clone(),
            });
        }
        self.producers.stop_active();
        self.input = None;
        Some(error)
    }
    #[cfg(feature = "producer-audio")]
    pub fn producer_frame(&mut self) -> Option<crate::playback::producer::Frame> {
        self.producers.frame()
    }
    /// The registered file resolvers' extensions (`Registry::file_types`).
    pub fn file_types(&self) -> Vec<String> {
        self.resolvers.file_types()
    }
    #[cfg(feature = "producer-audio")]
    pub fn provider_active(&self) -> bool {
        self.producers.active().is_some()
    }
    #[cfg(feature = "producer-audio")]
    pub fn producer_output(&mut self, volume: f64, muted: bool) {
        self.producers.output(volume, muted);
    }
    pub fn snapshot(&mut self, mpv: &Mpv) -> Value {
        let ab_loop = self.loop_state(mpv);
        let output = json!({"requested":self.audio_requested,"active":self.audio_active});
        #[cfg(feature = "producer-audio")]
        if let Some(ticket) = self.provider_ticket {
            if let Some((paused, position)) = self.producers.active_facts() {
                self.session.observe(ticket, paused, Some(position), None);
            }
            if let Some(health) = self.producers.active_health() {
                self.provider_health = Some(health);
            }
            let ready = self.phase() == Phase::Ready && self.producers.active().is_some();
            let mut state = serde_json::to_value(self.session.state()).unwrap();
            state["kind"] = json!("producer");
            state["audioHealth"] = json!(self.provider_health);
            state["controlPending"] = json!(
                self.awaiting_audio.is_some()
                    || self
                        .producers
                        .active_owner()
                        .and_then(|owner| self.controls.notice_for(owner))
                        .is_some()
            );
            state["container"] = Value::Null;
            state["audioTracks"] = Value::Null;
            state["capabilities"] = json!({"pause":ready,"seek":false,"rate":false,"restart":false,"close":true,"snapshotSource":false});
            state["output"] = json!({"volume":mpv.get_property::<f64>("volume").unwrap_or(100.),"muted":mpv.get_property::<bool>("mute").unwrap_or(false)});
            state["rate"] = json!(1.);
            state["loop"] = Value::Null;
            state["audioOutput"] = output;
            return state;
        }
        // keep-open preserves the final frame and does not emit EndFile at
        // natural EOF. Read the EOF fact only against the actual playing entry.
        if let Some((entry, _)) = self.active {
            let playing = mpv
                .get_property::<i64>("playlist-playing-pos")
                .ok()
                .filter(|n| *n >= 0)
                .and_then(|index| {
                    mpv.get_property::<i64>(&format!("playlist/{index}/id"))
                        .ok()
                });
            let eof = mpv.get_property::<bool>("eof-reached").unwrap_or(false);
            if !eof {
                self.eof_rearm = false;
            }
            if playing == Some(entry)
                && self.session.state().phase == Phase::Ready
                && !self.eof_rearm
                && eof
            {
                self.session.event(SourceEvent::EndFile {
                    entry,
                    reason: EndReason::Natural,
                });
            }
        }
        if let Some((_, ticket)) = self.active {
            self.session.observe(
                ticket,
                mpv.get_property("pause").unwrap_or(false),
                mpv.get_property("time-pos").ok(),
                mpv.get_property("duration").ok(),
            );
        }
        self.lease_ready = None;
        let container = mpv.get_property::<String>("file-format").ok();
        let seekable = mpv.get_property::<bool>("seekable").unwrap_or(false);
        if let Some((entry, ticket)) = self.active
            && ticket.stamp.generation == self.generation()
            && matches!(self.phase(), Phase::Ready | Phase::Ended)
            && seekable
            && container.as_deref().is_some_and(|name| {
                matches!(
                    name,
                    "mov,mp4,m4a,3gp,3g2,mj2"
                        | "matroska,webm"
                        | "mkv"
                        | "avi"
                        | "mp3"
                        | "flac"
                        | "wav"
                        | "ogg"
                )
            })
        {
            self.lease_ready = Some((entry, self.generation()));
        }
        let mut result = serde_json::to_value(self.session.state()).unwrap();
        result["kind"] = json!("media");
        result["container"] = json!(container);
        let tracks = self
            .presentation_load()
            .and_then(|_| mpv.get_property::<Node>("track-list").ok());
        result["audioTracks"] = match &tracks {
            Some(_) => crate::playback::media_tracks::audio_tracks(mpv)
                .map(|tracks| json!(tracks))
                .unwrap_or(Value::Null),
            None => Value::Null,
        };
        // Frame stepping needs moving pictures (not audio with cover art).
        result["hasVideo"] = json!(
            tracks
                .as_ref()
                .is_some_and(crate::playback::media_tracks::moving_video)
        );
        // The backdrop (R-U3): mpv shows a picture, cover art included.
        result["videoOutput"] = json!(
            tracks
                .as_ref()
                .is_some_and(crate::playback::media_tracks::any_video)
        );
        let ready = matches!(self.session.state().phase, Phase::Ready | Phase::Ended);
        result["capabilities"] = json!({"pause":ready,"seek":ready&&mpv.get_property::<bool>("seekable").unwrap_or(false),"rate":ready,"restart":self.session.state().source.is_some(),"close":self.session.state().source.is_some()});
        result["output"] = json!({"volume":mpv.get_property::<f64>("volume").unwrap_or(100.),"muted":mpv.get_property::<bool>("mute").unwrap_or(false)});
        result["rate"] = json!(mpv.get_property::<f64>("speed").unwrap_or(1.));
        result["loop"] = ab_loop;
        result["audioOutput"] = output;
        result["capabilities"]["snapshotSource"] = json!(self.lease(self.generation()).is_ok());
        result["snapshotProfile"] = json!({"timeDomain":"mpv-media","fileOriginValidation":"deferred-worker-origin-match-required"});
        result
    }
    /// The host sent mpv a seek of its own (a keyframe step seek, a frame or
    /// keyframe step): a later `Control::Seek` is always sent.
    pub fn seek_elsewhere(&mut self) {
        self.seek_flight.clear();
    }
    pub fn event(&mut self, event: &MpvEvent) -> bool {
        if matches!(
            event,
            MpvEvent::PlaybackRestart | MpvEvent::StartFile { .. } | MpvEvent::EndFile { .. }
        ) {
            self.seek_flight.clear();
        }
        match event {
            MpvEvent::StartFile { playlist_entry_id } => {
                // loadfile returns its actual native entry id before its events are drained.
                self.active = self
                    .entries
                    .get(playlist_entry_id)
                    .map(|t| (*playlist_entry_id, *t));
                self.active.is_some_and(|(entry, ticket)| {
                    self.session.event(SourceEvent::StartFile { entry, ticket })
                })
            }
            MpvEvent::FileLoaded => {
                let changed = self
                    .active
                    .is_some_and(|(_, ticket)| self.session.event(SourceEvent::Ready { ticket }));
                if changed && let Some((entry, ticket)) = self.active {
                    self.loaded = Some((entry, ticket));
                    // Actual new load is ready: older native streams have released their files.
                    self.local_media.retain(|id, _| *id == entry);
                }
                changed
            }
            MpvEvent::EndFile {
                playlist_entry_id,
                reason,
                error,
                ..
            } => {
                self.entries.remove(playlist_entry_id);
                self.local_media.remove(playlist_entry_id);
                if self
                    .loaded
                    .is_some_and(|(entry, _)| entry == *playlist_entry_id)
                {
                    self.loaded = None;
                }
                let why = match reason {
                    EndFileReason::Eof => EndReason::Natural,
                    EndFileReason::Error => EndReason::Failed(format!("{error:?}")),
                    EndFileReason::Stop => EndReason::Stopped,
                    _ => EndReason::Unknown,
                };
                let changed = self.session.event(SourceEvent::EndFile {
                    entry: *playlist_entry_id,
                    reason: why,
                });
                if self.active.is_some_and(|(id, _)| id == *playlist_entry_id) {
                    self.active = None;
                }
                changed
            }
            _ => false,
        }
    }
    fn finish_resolution(&mut self, resolution: &Resolution, result: Result<(), String>) {
        if let Err(message) = &result {
            self.session.event(SourceEvent::LoadFailed {
                ticket: resolution.ticket,
                message: message.clone(),
            });
        }
        if let Some(book) = self.books.get_mut(&resolution.requester) {
            let _ = book.finish(resolution.operation, result);
        }
    }
    pub fn resolver_events(&mut self) -> Vec<(u64, String, Value)> {
        let stale = self
            .resolutions
            .iter()
            .filter(|(_, r)| {
                r.deadline <= std::time::Instant::now()
                    || r.ticket.stamp != self.session.state().stamp
                    || r.ticket.cycle != self.session.state().cycle
                    || !self.books.contains_key(&r.requester)
                    || !self.resolvers.contains(r.resolver, &r.resolver_id)
            })
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for id in stale {
            let r = self.resolutions.remove(&id).unwrap();
            self.finish_resolution(
                &r,
                Err("source resolution cancelled, expired or superseded".into()),
            );
            self.resolution_events.push((
                r.resolver,
                "source.resolveCancel".into(),
                json!({"requestId":id,"resolverId":r.resolver_id}),
            ));
        }
        std::mem::take(&mut self.resolution_events)
    }
    fn resolver_dispatch(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        match method {
            "sourceResolvers.register" => {
                let declaration = serde_json::from_value(params.clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                self.resolvers.register(owner, declaration)?;
                Ok(Value::Null)
            }
            "sourceResolvers.unregister" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| RpcError::invalid("resolver id required"))?;
                self.resolvers.remove(owner, id);
                Ok(Value::Null)
            }
            "sourceResolvers.respond" => {
                let id = params["requestId"]
                    .as_u64()
                    .ok_or_else(|| RpcError::invalid("requestId required"))?;
                let r = self
                    .resolutions
                    .get(&id)
                    .ok_or_else(|| RpcError::new("stale", "resolution no longer pending"))?;
                if r.resolver != owner {
                    return Err(RpcError::new(
                        "owner",
                        "resolution belongs to another owner",
                    ));
                }
                let r = self.resolutions.remove(&id).unwrap();
                if r.ticket.stamp != self.session.state().stamp
                    || r.ticket.cycle != self.session.state().cycle
                    || r.deadline <= std::time::Instant::now()
                    || !self.resolvers.contains(owner, &r.resolver_id)
                {
                    self.finish_resolution(
                        &r,
                        Err("source resolution superseded or expired".into()),
                    );
                    return Err(RpcError::new(
                        "stale",
                        "source resolution superseded or expired",
                    ));
                }
                let files = self.resolvers.files(owner, &r.resolver_id);
                let result = (|| -> Result<Value, String> {
                    if let Some(error) = params["error"].as_str() {
                        // The resolver's own words: they are what the user sees.
                        let error = error.chars().take(2048).collect::<String>();
                        return Err(if error.trim().is_empty() {
                            "resolver failed".into()
                        } else {
                            error
                        });
                    }
                    if files {
                        let stream: crate::playback::source_resolvers::Stream =
                            serde_json::from_value(params["result"].clone())
                                .map_err(|_| "invalid resolved stream descriptor")?;
                        return self.select_resolved_stream(owner, stream.producer()?, &r);
                    }
                    let media: crate::playback::source_resolvers::Media =
                        serde_json::from_value(params["result"].clone())
                            .map_err(|_| "invalid resolved media descriptor")?;
                    let (source, options) = media.playback(&r.source)?;
                    let entry = load_with_options(mpv, &source, r.paused, &options)?;
                    self.current_local = None;
                    self.entries.clear();
                    self.entries.insert(entry, r.ticket);
                    Ok(Value::Null)
                })();
                self.finish_resolution(&r, result.as_ref().map(|_| ()).map_err(Clone::clone));
                result.map_err(|message| RpcError::new("resolution", message))
            }
            _ => Err(RpcError::new(
                "unsupported",
                "unknown source resolver method",
            )),
        }
    }
    /// A file resolver's answer: select the producer its plugin prepared as
    /// the source of the pending open. The open keeps its path as `source`,
    /// its stamp, operation and `startPaused`; mpv was stopped when the
    /// resolution began. Answers the stamp the stream belongs to.
    #[cfg(feature = "producer-audio")]
    fn select_resolved_stream(
        &mut self,
        owner: u64,
        producer_id: u64,
        r: &Resolution,
    ) -> Result<Value, String> {
        self.producers
            .ready(producer_id, owner)
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        self.producers
            .activate(producer_id, r.paused)
            .map_err(|e| e.message)?;
        self.active = None;
        self.loaded = None;
        self.entries.clear();
        self.current_local = None;
        self.lease_ready = None;
        self.provider_ticket = Some(r.ticket);
        self.session
            .event(SourceEvent::ProviderReady { ticket: r.ticket });
        Ok(json!({ "stamp": r.ticket.stamp }))
    }
    #[cfg(not(feature = "producer-audio"))]
    fn select_resolved_stream(
        &mut self,
        _owner: u64,
        _producer_id: u64,
        _r: &Resolution,
    ) -> Result<Value, String> {
        Err("unsupported: producer not enabled in this build".into())
    }
    pub fn disable(&mut self, owner: u64) {
        self.resolvers.disable(owner);
        // Like the window level: a stopped plugin's device choice goes with it.
        if self.audio_owner == Some(owner) {
            self.audio_owner = None;
            self.audio_requested = None;
            self.audio_checked = None;
        }
        #[cfg(feature = "producer-audio")]
        {
            if self.producers.active_owner() == Some(owner) {
                self.input = None;
                self.cancel_provider_control("provider owner disabled");
                let mut book = OperationBook::new(owner);
                let _ = self
                    .session
                    .submit(&mut book, 1, SourceIntent::Close { expected: None });
                self.provider_ticket = None;
            }
            self.producers.disable(owner);
        }
        self.books.remove(&owner);
    }
    pub fn dispatch(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> Result<Option<Value>, RpcError> {
        if method.starts_with("sourceResolvers.") {
            return self.resolver_dispatch(mpv, owner, method, params).map(Some);
        }
        if matches!(method, "audio.addTrack" | "audio.selectTrack") {
            return self.audio_track(mpv, method, params).map(Some);
        }
        if matches!(method, "player.audioDevices" | "player.setAudioDevice") {
            return self.audio_device(mpv, owner, method, params).map(Some);
        }
        if matches!(method, "player.chapters" | "player.setLoop") {
            return self.marks(mpv, method, params).map(Some);
        }
        #[cfg(feature = "producer-audio")]
        {
            self.tick_provider_controls();
            match method {
                "producer.declareInput" => {
                    let id = params["producerId"]
                        .as_u64()
                        .ok_or_else(|| RpcError::invalid("producerId required"))?;
                    if self.producers.active() != Some(id)
                        || self.producers.active_owner() != Some(owner)
                    {
                        return Err(RpcError::new(
                            "unavailable",
                            "input requires this owner's active source",
                        ));
                    }
                    let keys: Vec<String> = serde_json::from_value(params["keys"].clone())
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                    if keys
                        .iter()
                        .any(|key| !crate::playback::producer_input::supported_code(key))
                    {
                        return Err(RpcError::new(
                            "unsupported",
                            "physical code is not supported by this adapter",
                        ));
                    }
                    if let Some(input) = &self.input {
                        if input.keys == keys {
                            return Ok(Some(self.input_facts()));
                        }
                        return Err(RpcError::new(
                            "conflict",
                            "input is already declared for this source",
                        ));
                    }
                    let epoch = self
                        .input_epoch
                        .checked_add(1)
                        .filter(|n| *n <= 9_007_199_254_740_991)
                        .ok_or_else(|| RpcError::new("busy", "input epochs exhausted"))?;
                    let ticket = IntentTicket {
                        stamp: self.session.state().stamp,
                        cycle: self.session.state().cycle,
                        owner: self.provider_ticket.unwrap().owner,
                    };
                    let state = crate::playback::producer_input::State::new(
                        crate::playback::producer_input::Scope {
                            owner,
                            producer_id: id,
                            ticket,
                            input_epoch: epoch,
                        },
                        keys.clone(),
                    )
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                    self.input = Some(InputBinding {
                        delivery: crate::playback::producer_input_delivery::Delivery::new(state),
                        keys,
                    });
                    self.input_epoch = epoch;
                    return Ok(Some(self.input_facts()));
                }
                "producer.ackInput" => {
                    let notice = serde_json::from_value(params["notice"].clone())
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                    let now = self.now_ms();
                    self.input
                        .as_mut()
                        .ok_or_else(|| RpcError::new("unavailable", "input scope absent"))?
                        .delivery
                        .ack(owner, &notice, now)
                        .map_err(Self::input_error)?;
                    return Ok(Some(
                        json!({"status":"acknowledged","sequence":notice.sequence}),
                    ));
                }
                "producer.prepare" => return self.producers.prepare(owner, params).map(Some),
                "producer.release" => {
                    return self
                        .producers
                        .release(
                            owner,
                            params["producerId"]
                                .as_u64()
                                .ok_or_else(|| RpcError::invalid("producerId required"))?,
                        )
                        .map(|_| Some(Value::Null));
                }
                "producer.state" => {
                    let id = params["producerId"]
                        .as_u64()
                        .ok_or_else(|| RpcError::invalid("producerId required"))?;
                    let mut state = self.producers.state(id, owner)?;
                    state["pendingControl"] = json!(
                        self.controls
                            .notice_for(owner)
                            .filter(|n| n.producer_id == id)
                    );
                    state["input"] = if self.producers.active() == Some(id) {
                        self.input_facts()
                    } else {
                        Value::Null
                    };
                    return Ok(Some(state));
                }
                "producer.ack" => {
                    let notice: crate::playback::producer_controls::Notice =
                        serde_json::from_value(params["control"].clone())
                            .map_err(|e| RpcError::invalid(e.to_string()))?;
                    if let Some((peer, previous, result)) = &self.last_ack
                        && *peer == owner
                        && *previous == notice
                    {
                        return Ok(Some(result.clone()));
                    }
                    let pending = self
                        .controls
                        .ack(owner, &notice, self.session.state().stamp)
                        .map_err(|e| {
                            RpcError::new(
                                match e {
                                    crate::playback::producer_controls::Error::Unavailable => {
                                        "unavailable"
                                    }
                                    _ => "stale",
                                },
                                format!("{e:?}"),
                            )
                        })?;
                    if let Err(error) = self.producers.acknowledge_control(
                        notice.producer_id,
                        notice.epoch,
                        notice.paused,
                    ) {
                        let ticket = pending.ticket;
                        self.finish_provider_control(pending, Err(error.message.clone()));
                        self.pending_error = Some((ticket, error.message.clone()));
                        return Err(error);
                    }
                    self.awaiting_audio = Some(pending);
                    let result = json!({"status":"acknowledged","control":notice});
                    self.last_ack = Some((owner, notice, result.clone()));
                    self.tick_provider_controls();
                    return Ok(Some(result));
                }
                _ => {}
            }
        }
        if method == "source.state" {
            return Ok(Some(self.snapshot(mpv)));
        }
        if method == "source.operation" {
            let id = params["operationId"]
                .as_u64()
                .ok_or_else(|| RpcError::invalid("operationId requires positive integer"))?;
            let book = self
                .books
                .entry(owner)
                .or_insert_with(|| OperationBook::new(owner));
            return book
                .query(id)
                .map(|s| Some(json!(s)))
                .map_err(|e| RpcError::new("unknown-expired", format!("{e:?}")));
        }
        if method != "source.submit" {
            return Ok(None);
        }
        let id = params["operationId"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| RpcError::invalid("operationId requires positive integer"))?;
        let intent = serde_json::from_value::<SourceIntent>(params["intent"].clone())
            .map_err(|e| RpcError::invalid(e.to_string()))?;
        #[cfg(feature = "producer-audio")]
        let preflight = if matches!(intent, SourceIntent::Control { .. })
            && (self.pending_error.is_some()
                || self.producers.active_failure().is_some()
                || self.awaiting_audio.is_some()
                || self
                    .producers
                    .active_owner()
                    .and_then(|owner| self.controls.notice_for(owner))
                    .is_some())
        {
            Err(crate::playback::session::RequestError::Busy)
        } else {
            match &intent {
                SourceIntent::OpenProvider { producer_id, .. } => {
                    self.producers.ready(*producer_id, owner).map_err(|e| {
                        crate::playback::session::RequestError::Backend(format!(
                            "{}: {}",
                            e.code, e.message
                        ))
                    })
                }
                SourceIntent::Restart { .. }
                | SourceIntent::Control {
                    control: Control::Seek { .. } | Control::Rate { .. },
                    ..
                } if self.producers.active().is_some() => {
                    Err(crate::playback::session::RequestError::Backend(
                        "unsupported: producer seek/rate/restart unsupported".into(),
                    ))
                }
                _ => Ok(()),
            }
        };
        #[cfg(not(feature = "producer-audio"))]
        let preflight = if matches!(intent, SourceIntent::OpenProvider { .. }) {
            Err(crate::playback::session::RequestError::Backend(
                "unsupported: producer not enabled in this build".into(),
            ))
        } else {
            Ok(())
        };
        let resolution_source = match &intent {
            SourceIntent::Open { source, .. } => Some(source.clone()),
            SourceIntent::Restart { .. } => self.session.state().source.clone(),
            _ => None,
        };
        let resolver = resolution_source
            .as_deref()
            .and_then(|source| self.resolvers.find(source));
        let preflight = preflight.and_then(|()| {
            if resolver.is_some() && self.resolutions.len() >= 32 {
                Err(crate::playback::session::RequestError::Busy)
            } else {
                Ok(())
            }
        });
        let book = self
            .books
            .entry(owner)
            .or_insert_with(|| OperationBook::new(owner));
        let ticket = match self
            .session
            .submit_checked(book, id, intent.clone(), |_| preflight)
        {
            Admission::Execute(t) => t,
            Admission::Replay(s) => return Ok(Some(json!(s))),
            Admission::Rejected(e) => {
                return Err(RpcError::new(
                    match e {
                        crate::playback::session::RequestError::Stale => "stale",
                        crate::playback::session::RequestError::UnknownExpired => "unknown-expired",
                        crate::playback::session::RequestError::Busy => "busy",
                        crate::playback::session::RequestError::Conflict => "conflict",
                        crate::playback::session::RequestError::Backend(ref message)
                            if message.starts_with("unavailable:") =>
                        {
                            "unavailable"
                        }
                        crate::playback::session::RequestError::Backend(ref message)
                            if message.starts_with("conflict:") =>
                        {
                            "conflict"
                        }
                        crate::playback::session::RequestError::Backend(_) => "unsupported",
                        _ => "invalid",
                    },
                    format!("{e:?}"),
                ));
            }
        };
        #[cfg(feature = "producer-audio")]
        if matches!(
            intent,
            SourceIntent::Open { .. } | SourceIntent::Close { .. }
        ) {
            self.producers.stop_active();
            self.provider_ticket = None;
        }
        #[cfg(feature = "producer-audio")]
        if matches!(
            intent,
            SourceIntent::Open { .. }
                | SourceIntent::OpenProvider { .. }
                | SourceIntent::Close { .. }
        ) {
            self.input = None;
            self.cancel_provider_control("source superseded the provider control");
        }
        if !matches!(intent, SourceIntent::Control { .. }) {
            self.seek_flight.clear();
        }
        if let Some((resolver, resolver_id)) = resolver {
            self.resolution_next += 1;
            let request_id = self.resolution_next;
            let source = resolution_source.unwrap();
            let paused = matches!(
                &intent,
                SourceIntent::Open {
                    start_paused: true,
                    ..
                }
            );
            if let Err(error) = mpv.command(&["stop"]) {
                let error = error.to_string();
                let _ = self
                    .books
                    .get_mut(&owner)
                    .unwrap()
                    .finish(id, Err(error.clone()));
                self.session.event(SourceEvent::LoadFailed {
                    ticket,
                    message: error.clone(),
                });
                return Err(RpcError::new("backend", error));
            }
            self.active = None;
            self.loaded = None;
            self.entries.clear();
            self.current_local = None;
            self.lease_ready = None;
            self.resolution_events.push((
                resolver,
                "source.resolveRequest".into(),
                json!({"requestId":request_id,"resolverId":resolver_id,"url":source}),
            ));
            self.resolutions.insert(
                request_id,
                Resolution {
                    requester: owner,
                    operation: id,
                    resolver,
                    resolver_id,
                    source,
                    paused,
                    ticket,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
                },
            );
            return Ok(Some(json!({"status":"pending","ticket":ticket})));
        }
        let result: Result<(), String> = match &intent {
            SourceIntent::OpenProvider {
                producer_id,
                start_paused,
                ..
            } => {
                #[cfg(feature = "producer-audio")]
                {
                    self.producers
                        .ready(*producer_id, owner)
                        .map_err(|e| e.message)
                        .and_then(|_| mpv.command(&["stop"]).map_err(|e| e.to_string()))
                        .and_then(|_| {
                            self.producers
                                .activate(*producer_id, *start_paused)
                                .map_err(|e| e.message)
                        })
                        .map(|_| {
                            self.active = None;
                            self.loaded = None;
                            self.entries.clear();
                            self.current_local = None;
                            self.lease_ready = None;
                            self.provider_ticket = Some(ticket);
                            self.session.event(SourceEvent::ProviderReady { ticket });
                        })
                }
                #[cfg(not(feature = "producer-audio"))]
                {
                    let _ = (producer_id, start_paused);
                    Err("producer support not enabled in this build".into())
                }
            }
            SourceIntent::Open {
                source,
                start_paused,
                ..
            } => LocalMedia::open(source, self.lease_streams).and_then(|media| {
                #[cfg(feature = "producer-audio")]
                {
                    self.producers.deactivate();
                    self.provider_ticket = None;
                }
                let url = media
                    .as_ref()
                    .map(|media| media.url())
                    .unwrap_or_else(|| source.clone());
                // Keep the previous native load's File until its EndFile/new Ready.
                let leased = media.as_ref().map(|media| (source.as_str(), media.remote));
                let entry = load(mpv, &url, *start_paused, leased)?;
                self.current_local = media
                    .as_ref()
                    .map(|media| (ticket.stamp.generation, media.clone()));
                if let Some(media) = media {
                    self.local_media.insert(entry, media);
                }
                self.entries.clear();
                self.entries.insert(entry, ticket);
                Ok(())
            }),
            SourceIntent::Restart { .. } => {
                let media = self
                    .current_local
                    .as_ref()
                    .filter(|(generation, _)| *generation == ticket.stamp.generation)
                    .map(|(_, media)| media.clone());
                let source = media
                    .as_ref()
                    .map(|media| media.url())
                    .unwrap_or_else(|| self.session.state().source.clone().unwrap_or_default());
                let path = self.session.state().source.clone().unwrap_or_default();
                let leased = media.as_ref().map(|media| (path.as_str(), media.remote));
                load(mpv, &source, false, leased).map(|entry| {
                    if let Some(media) = media {
                        self.local_media.insert(entry, media);
                    }
                    self.entries.clear();
                    self.entries.insert(entry, ticket);
                })
            }
            SourceIntent::Close { .. } => {
                #[cfg(feature = "producer-audio")]
                {
                    self.producers.deactivate();
                    self.provider_ticket = None;
                }
                mpv.command(&["stop"])
                    .map(|_| {
                        self.active = None;
                        self.current_local = None;
                        self.loaded = None;
                        self.entries.clear();
                    })
                    .map_err(|e| e.to_string())
            }
            SourceIntent::Control { control, .. } => {
                #[cfg(feature = "producer-audio")]
                if self.producers.active().is_some() {
                    let Control::Pause { paused } = control else {
                        unreachable!("provider capability preflight");
                    };
                    let preparation = (|| {
                        if *paused {
                            if let Some(input) = &mut self.input {
                                let now = self
                                    .control_origin
                                    .as_ref()
                                    .map_or(0, |origin| origin.elapsed().as_millis() as u64);
                                input
                                    .delivery
                                    .revoke(
                                        crate::playback::producer_input::RevokeReason::Pause,
                                        now,
                                    )
                                    .map_err(Self::input_error)?;
                            }
                            self.producers.pause(true)?;
                        }
                        self.producers.control_target()
                    })();
                    let (producer, provider, epoch, base) = match preparation {
                        Ok(target) => target,
                        Err(error) => {
                            let _ = self
                                .books
                                .get_mut(&owner)
                                .unwrap()
                                .finish(id, Err(error.message.clone()));
                            self.pending_error = Some((ticket, error.message.clone()));
                            return Err(error);
                        }
                    };
                    let now = self.now_ms();
                    if let Err(error) = self.controls.begin(
                        owner, id, provider, producer, ticket, epoch, base, *paused, now,
                    ) {
                        let message = format!("provider control admission: {error:?}");
                        let book = self.books.get_mut(&owner).unwrap();
                        let _ = book.finish(id, Err(message.clone()));
                        self.pending_error = Some((ticket, message.clone()));
                        return Err(RpcError::new("backend", message));
                    }
                    return Ok(Some(json!(
                        crate::playback::session::OperationStatus::Pending { ticket }
                    )));
                }
                match control {
                    Control::Pause { paused } => mpv.set_property("pause", *paused).inspect(|()| {
                        crate::input_trace::mark(&format!(
                            "playback worker: libmpv pause={paused} set"
                        ));
                    }),
                    Control::Seek { seconds, keyframes } => {
                        let entry = self.active.map(|(entry, _)| entry);
                        if self.seek_flight.repeats(entry, *seconds, *keyframes)
                            && mpv.get_property::<bool>("seeking").unwrap_or(false)
                        {
                            #[cfg(test)]
                            {
                                self.seek_repeats_skipped += 1;
                            }
                            Ok(())
                        } else {
                            let command = seek_command(*seconds, *keyframes);
                            let sent = mpv.command(&command);
                            match sent {
                                Ok(()) => self.seek_flight.sent(entry, *seconds, *keyframes),
                                Err(_) => self.seek_flight.clear(),
                            }
                            sent
                        }
                    }
                    Control::Rate { rate } => mpv.set_property("speed", *rate),
                }
                .map_err(|e| e.to_string())
            }
        };
        if let Err(message) = &result
            && matches!(
                intent,
                SourceIntent::Open { .. }
                    | SourceIntent::OpenProvider { .. }
                    | SourceIntent::Restart { .. }
            )
        {
            self.session.event(SourceEvent::LoadFailed {
                ticket,
                message: message.clone(),
            });
        }
        if result.is_ok()
            && matches!(
                intent,
                SourceIntent::Control {
                    control: Control::Seek { .. },
                    ..
                }
            )
            && let Some((entry, _)) = self.active
            && let Some(active) = self.session.confirm_seek_from_end(ticket, entry)
        {
            self.active = Some((entry, active));
            self.entries.insert(entry, active);
            self.eof_rearm = true;
        }
        let status = self
            .books
            .get_mut(&owner)
            .unwrap()
            .finish(id, result.clone())
            .map_err(|e| RpcError::new("host", format!("{e:?}")))?;
        result.map_err(|e| RpcError::new("backend", e))?;
        Ok(Some(json!(status)))
    }
}
/// mpv `chapter-list` as `[{ title, start }]` (at most 4096, empty title = null).
fn chapter_list(node: Node) -> Vec<Value> {
    let Node::Array(entries) = node else {
        return Vec::new();
    };
    entries
        .into_iter()
        .take(4096)
        .filter_map(|entry| {
            let Node::Map(fields) = entry else {
                return None;
            };
            let field = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, v)| v);
            let start = match field("time")? {
                Node::Double(time) if time.is_finite() => *time,
                Node::Int64(time) => *time as f64,
                _ => return None,
            };
            let title = match field("title") {
                Some(Node::String(title)) if !title.is_empty() => {
                    Some(title.chars().take(1024).collect::<String>())
                }
                _ => None,
            };
            Some(json!({"title":title,"start":start.max(0.)}))
        })
        .collect()
}
/// mpv `audio-device-list` as `(id, name)`, without `auto` (the system
/// default). Where CoreAudio devices are listed, only those: macOS lists the
/// same device once more under `avfoundation/`.
fn audio_devices(mpv: &Mpv) -> Vec<(String, String)> {
    let Ok(Node::Array(entries)) = mpv.get_property::<Node>("audio-device-list") else {
        return Vec::new();
    };
    device_list(entries)
}
fn device_list(entries: Vec<Node>) -> Vec<(String, String)> {
    let all = entries
        .into_iter()
        .take(256)
        .filter_map(|entry| {
            let Node::Map(fields) = entry else {
                return None;
            };
            let text = |key: &str| match fields.iter().find(|(name, _)| name == key) {
                Some((_, Node::String(text))) => Some(text.clone()),
                _ => None,
            };
            let id = text("name").filter(|id| id != "auto" && !id.is_empty())?;
            let name = text("description").unwrap_or_else(|| id.clone());
            Some((id, name))
        })
        .collect::<Vec<_>>();
    if all.iter().any(|(id, _)| id.starts_with("coreaudio/")) {
        all.into_iter()
            .filter(|(id, _)| id.starts_with("coreaudio/"))
            .collect()
    } else {
        all
    }
}
fn clear_loop(mpv: &Mpv) {
    for (name, value) in [
        ("ab-loop-a", "no"),
        ("ab-loop-b", "no"),
        ("ab-loop-count", "inf"),
    ] {
        if let Err(error) = mpv.set_property(name, value) {
            diag!("host.error {name}: {error}");
        }
    }
}
/// The one external track `audio-add` created, if exactly one appeared.
fn added_external(
    before: &[crate::playback::media_tracks::AudioTrack],
    after: &[crate::playback::media_tracks::AudioTrack],
) -> Option<crate::playback::media_tracks::AudioTrack> {
    let mut new = after
        .iter()
        .filter(|track| track.external && !before.iter().any(|old| old.track_id == track.track_id));
    let first = new.next()?.clone();
    new.next().is_none().then_some(first)
}
/// `leased`: `source` is a [`LocalMedia`] url (`fd://` on Unix,
/// `vestra-lease://` on Windows) for the local file at this path, and
/// whether that file is on a network file system ([`LocalMedia::remote`]).
fn load(
    mpv: &Mpv,
    source: &str,
    paused: bool,
    leased: Option<(&str, bool)>,
) -> Result<i64, String> {
    match leased {
        Some((path, remote)) => load_options(
            mpv,
            source,
            paused,
            &leased_options(mpv, path, remote),
            true,
        ),
        None => load_options(mpv, source, paused, "", false),
    }
}

/// What libmpv derives from a local file's path, given for a leased load,
/// whose url carries none: the demuxer cache of a file on a network file
/// system (`remote`, Windows; Unix `fd://` lets libmpv see the remote file
/// system itself), and cover art beside an audio file ([`cover_art`]).
fn leased_options(mpv: &Mpv, path: &str, remote: bool) -> String {
    let mut options = String::new();
    if remote {
        options.push_str(",cache=yes");
    }
    let auto = mpv
        .get_property::<String>("cover-art-auto")
        .unwrap_or_default();
    let whitelist = mpv
        .get_property::<String>("cover-art-whitelist")
        .unwrap_or_default();
    let image_exts = mpv.get_property::<String>("image-exts").unwrap_or_default();
    // One path list (`;` on Windows, `:` elsewhere): a repeated
    // `cover-art-files-append` in loadfile options keeps only the last.
    let separator = if cfg!(windows) { ';' } else { ':' };
    let images = cover_art(Path::new(path), &auto, &whitelist, &image_exts)
        .iter()
        .map(|image| image.to_string_lossy().into_owned())
        .filter(|image| !image.contains(separator))
        .collect::<Vec<_>>()
        .join(&separator.to_string());
    if !images.is_empty() {
        // `%n%` quotes a value of n bytes (mpv's key=value list syntax).
        options.push_str(&format!(",cover-art-files=%{}%{images}", images.len()));
    }
    options
}

/// The pictures mpv's `cover-art-auto=exact` loads for `media` when it reads
/// it by path, best first: images with the same name (any case) and then
/// those named in `cover-art-whitelist` (`cover.jpg`, `Folder.png`, …) in its
/// order, among files with an extension of `image-exts` (mpv's list of image
/// extensions, any case). libmpv adds them to audio files only (no video
/// stream); a leased load picks by extension instead. Nothing for `no`; the
/// host sets no other mode (`fuzzy` and `all` are treated as `exact`).
fn cover_art(
    media: &Path,
    auto: &str,
    whitelist: &str,
    image_exts: &str,
) -> Vec<std::path::PathBuf> {
    const AUDIO: [&str; 6] = ["mp3", "m4a", "flac", "wav", "ogg", "opus"];
    let lower = |name: Option<&std::ffi::OsStr>| {
        name.and_then(|name| name.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default()
    };
    if auto == "no" || !AUDIO.contains(&lower(media.extension()).as_str()) {
        return Vec::new();
    }
    let stem = lower(media.file_stem());
    let (Some(dir), false) = (media.parent(), stem.is_empty()) else {
        return Vec::new();
    };
    let whitelist = whitelist
        .split(',')
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let images = image_exts
        .split(',')
        .filter(|ext| !ext.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let path = entry.path();
            if !images.contains(&lower(path.extension())) {
                return None;
            }
            let name = lower(path.file_stem());
            let rank = if name == stem {
                0
            } else {
                1 + whitelist.iter().position(|listed| *listed == name)?
            };
            Some((rank, lower(path.file_name()), path))
        })
        .collect::<Vec<_>>();
    found.sort();
    found.into_iter().map(|(_, _, path)| path).collect()
}
fn load_with_options(mpv: &Mpv, source: &str, paused: bool, extra: &str) -> Result<i64, String> {
    load_options(mpv, source, paused, extra, false)
}
fn load_options(
    mpv: &Mpv,
    source: &str,
    paused: bool,
    extra: &str,
    leased: bool,
) -> Result<i64, String> {
    let pause = if paused { "pause=yes" } else { "pause=no" };
    // ff-index is documented as reliable only with the mature lavf demuxer.
    // Apply this per leased, self-contained local load, not arbitrary URL loaders.
    let options = if leased || source.starts_with("fd://") {
        format!("{pause},demuxer=lavf,demuxer-lavf-probe-info=yes,rebase-start-time=yes")
    } else {
        pause.into()
    };
    let options = format!("{options}{extra}");
    let result = mpv
        .command_ret(&["loadfile", source, "replace", "-1", &options])
        .map_err(|e| e.to_string())?;
    if let Node::Map(fields) = result
        && let Some((_, Node::Int64(entry))) =
            fields.iter().find(|(key, _)| key == "playlist_entry_id")
    {
        return Ok(*entry);
    }
    Err("libmpv did not return playlist_entry_id; unsupported backend".into())
}

#[cfg(all(test, feature = "producer-audio"))]
mod input_adapter_tests {
    use super::*;
    #[test]
    fn full_native_keyup_becomes_pinned_empty_release_instead_of_stuck_key() {
        use crate::{
            playback::producer_input::{Scope, State},
            playback::producer_input_delivery::{Delivery, Error},
        };
        let scope = Scope {
            owner: 1,
            producer_id: 1,
            input_epoch: 1,
            ticket: IntentTicket {
                owner: 2,
                stamp: crate::playback::session::SourceStamp {
                    generation: 1,
                    intent_version: 1,
                },
                cycle: 1,
            },
        };
        let mut input = InputBinding {
            delivery: Delivery::new(State::new(scope.clone(), vec!["KeyX".into()]).unwrap()),
            keys: vec!["KeyX".into()],
        };
        let old = input.delivery.capture(0).unwrap();
        for index in 0..31 {
            input
                .physical_key(&scope, "KeyX", index % 2 == 0, 0)
                .unwrap();
        }
        assert_eq!(input.delivery.notice().pressed, vec!["KeyX"]);
        assert_eq!(
            input.physical_key(&scope, "KeyX", false, 0),
            Err(Error::Busy)
        );
        let release = input.delivery.pending().unwrap().clone();
        assert!(!release.captured);
        assert!(release.pressed.is_empty());
        assert_eq!(input.delivery.ack(1, &old, 0), Err(Error::Stale));
        assert_eq!(input.delivery.capture(1), Err(Error::Busy));
        input.delivery.ack(1, &release, 1).unwrap();
        assert!(input.delivery.pending().is_none());
        assert!(input.delivery.notice().pressed.is_empty());
    }
}

#[cfg(test)]
mod seek_flight_tests {
    use super::*;
    #[test]
    fn only_the_same_target_on_the_same_entry_in_the_same_mode_repeats() {
        let mut flight = SeekFlight::default();
        assert!(!flight.repeats(Some(1), 12.5, false), "nothing in flight");
        flight.sent(Some(1), 12.5, false);
        assert!(flight.repeats(Some(1), 12.5, false), "press then release");
        assert!(!flight.repeats(Some(1), 12.500001, false), "a drag step");
        assert!(!flight.repeats(Some(2), 12.5, false), "another file");
        assert!(!flight.repeats(None, 12.5, false), "no file");
        assert!(
            !flight.repeats(Some(1), 12.5, true),
            "a keyframe seek to the exact target: another seek"
        );
        flight.sent(Some(1), 13.0, false);
        assert!(
            !flight.repeats(Some(1), 12.5, false),
            "the newest seek counts"
        );
        assert!(flight.repeats(Some(1), 13.0, false));
        flight.clear();
        assert!(
            !flight.repeats(Some(1), 13.0, false),
            "finished: sent again"
        );
        flight.sent(None, 13.0, false);
        assert!(!flight.repeats(None, 13.0, false));
        // Keyframe mode: the same key rules, and an exact seek to the same
        // target is not a repeat (it lands on the target, not the keyframe).
        flight.sent(Some(1), 27.6, true);
        assert!(flight.repeats(Some(1), 27.6, true), "press then release");
        assert!(!flight.repeats(Some(1), 27.7, true), "a drag step");
        assert!(
            !flight.repeats(Some(1), 27.6, false),
            "exact after keyframes: sent"
        );
    }
    #[test]
    fn the_seek_flag_maps_to_the_mpv_mode() {
        // The wire: absent means exact (older plugins), true means keyframes.
        let parse = |control: Value| serde_json::from_value::<Control>(control).unwrap();
        assert_eq!(
            parse(json!({"kind":"seek","seconds":9.5})),
            Control::Seek {
                seconds: 9.5,
                keyframes: false
            }
        );
        assert_eq!(
            parse(json!({"kind":"seek","seconds":9.5,"keyframes":false})),
            Control::Seek {
                seconds: 9.5,
                keyframes: false
            }
        );
        assert_eq!(
            parse(json!({"kind":"seek","seconds":9.5,"keyframes":true})),
            Control::Seek {
                seconds: 9.5,
                keyframes: true
            }
        );
        assert!(
            serde_json::from_value::<Control>(json!({"kind":"seek","seconds":9.5,"keyframes":1}))
                .is_err(),
            "keyframes must be a boolean"
        );
        // Written back (the operation book) without the flag when it is off,
        // so the shape plugins built on SDK 1.5.21 and earlier read is unchanged.
        assert_eq!(
            serde_json::to_value(Control::Seek {
                seconds: 9.5,
                keyframes: false
            })
            .unwrap(),
            json!({"kind":"seek","seconds":9.5})
        );
        assert_eq!(
            serde_json::to_value(Control::Seek {
                seconds: 9.5,
                keyframes: true
            })
            .unwrap(),
            json!({"kind":"seek","seconds":9.5,"keyframes":true})
        );
        // The mpv command.
        assert_eq!(seek_command(9.5, false), ["seek", "9.5", "absolute+exact"]);
        assert_eq!(
            seek_command(9.5, true),
            ["seek", "9.5", "absolute+keyframes"]
        );
    }
    #[test]
    fn a_restart_a_new_file_or_a_host_seek_ends_the_flight() {
        for event in [
            MpvEvent::PlaybackRestart,
            MpvEvent::StartFile {
                playlist_entry_id: 1,
            },
            MpvEvent::EndFile {
                reason: EndFileReason::Stop,
                error: None,
                playlist_entry_id: 1,
                playlist_insert_id: 0,
                playlist_insert_num_entries: 0,
            },
        ] {
            let mut sources = Sources::default();
            sources.seek_flight.sent(Some(1), 5.0, false);
            sources.event(&event);
            assert!(
                !sources.seek_flight.repeats(Some(1), 5.0, false),
                "{event:?}"
            );
        }
        let mut sources = Sources::default();
        sources.seek_flight.sent(Some(1), 5.0, false);
        sources.event(&MpvEvent::Seek);
        assert!(
            sources.seek_flight.repeats(Some(1), 5.0, false),
            "the seek's own start keeps it"
        );
        sources.seek_elsewhere();
        assert!(!sources.seek_flight.repeats(Some(1), 5.0, false));
    }
    /// A seek-bar click (the same target on press and on release) makes mpv
    /// seek once; the same target again after it landed seeks again. A
    /// keyframe bar seek (`keyframes: true`) lands on the keyframe at or
    /// before the target, and an exact seek to that target right after it is
    /// sent and lands exactly. Needs `ffmpeg` with libx264:
    /// `cargo test --locked -- --ignored a_click_on_the_seek_bar_seeks_once`.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_click_on_the_seek_bar_seeks_once() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("vestra-seekonce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gop10.mp4");
        // A keyframe every 10 s: an exact seek decodes for a while.
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1920x1080:rate=60")
            .args(["-t", "20", "-c:v", "libx264", "-preset", "ultrafast"])
            .args(["-g", "600", "-keyint_min", "600", "-sc_threshold", "0"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(made.success());
        let mpv = Mpv::builder()
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .set_property("keep-open", true)
            .unwrap()
            .build()
            .unwrap();
        let mut sources = Sources::default();
        let submit = |sources: &mut Sources, id: u64, intent: Value| {
            sources
                .dispatch(
                    &mpv,
                    1,
                    "source.submit",
                    &json!({"operationId":id,"intent":intent}),
                )
                .unwrap();
        };
        // Pumps events into `sources` until `until` or 20 s; returns the seeks mpv started.
        let pump = |sources: &mut Sources, until: &dyn Fn(&MpvEvent) -> bool| {
            let mut seeks = 0;
            let end = Instant::now() + Duration::from_secs(20);
            while Instant::now() < end {
                while let Some(event) = mpv.poll_event() {
                    sources.event(&event);
                    seeks += usize::from(matches!(event, MpvEvent::Seek));
                    if until(&event) {
                        return seeks;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("no event in time");
        };
        submit(
            &mut sources,
            1,
            json!({"kind":"open","source":file.to_str().unwrap(),"startPaused":true}),
        );
        pump(&mut sources, &|e| matches!(e, MpvEvent::PlaybackRestart));
        // A click: the press, then the release with the same target once mpv
        // has started the press's seek (its Seek event; the flight is cleared
        // only when it lands). Returns the seeks mpv started from the press
        // to its landing and then during a quiet second, and whether the host
        // held the release back (it saw the seek still running). A duplicate
        // the host sent can start before or after the first landing (mpv may
        // hold a queued seek for up to 0.3 s), so the quiet window catches it.
        let click = |sources: &mut Sources, ids: (u64, u64), intent: &Value| {
            submit(sources, ids.0, intent.clone());
            let mut seeks = pump(sources, &|e| matches!(e, MpvEvent::Seek));
            let skipped = sources.seek_repeats_skipped;
            submit(sources, ids.1, intent.clone());
            let held = sources.seek_repeats_skipped == skipped + 1;
            seeks += pump(sources, &|e| matches!(e, MpvEvent::PlaybackRestart));
            let quiet = Instant::now() + Duration::from_secs(1);
            while Instant::now() < quiet {
                while let Some(event) = mpv.poll_event() {
                    sources.event(&event);
                    seeks += usize::from(matches!(event, MpvEvent::Seek));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            (seeks, held)
        };
        // An exact seek 9.9 s past the keyframe decodes ~590 1080p frames,
        // far longer than the moment between the press's Seek and the release.
        let seek = json!({"kind":"control","control":{"kind":"seek","seconds":9.9}});
        let (seeks, held) = click(&mut sources, (2, 3), &seek);
        assert!(held, "the release arrived while the seek ran: not sent");
        assert_eq!(seeks, 1, "press and release: one seek");
        assert!((mpv.get_property::<f64>("time-pos").unwrap() - 9.9).abs() < 0.02);
        submit(&mut sources, 4, seek);
        let again = pump(&mut sources, &|e| matches!(e, MpvEvent::PlaybackRestart));
        assert_eq!(again, 1, "after it landed the same target seeks again");
        // The bar in keyframe mode: 15.0 lands on the keyframe at 10.0 (the
        // clip's keyframes are at 0 and 10 s), press and release seek once.
        // A keyframe seek is quick: if it had already landed when the release
        // came, the release rightly seeks again.
        let key_seek =
            json!({"kind":"control","control":{"kind":"seek","seconds":15.0,"keyframes":true}});
        let (seeks, held) = click(&mut sources, (5, 6), &key_seek);
        if held {
            assert_eq!(seeks, 1, "a keyframe click seeks once");
        } else {
            assert_eq!(seeks, 2, "a release after the landing seeks once more");
        }
        let landed = mpv.get_property::<f64>("time-pos").unwrap();
        assert!(
            (landed - 10.0).abs() < 0.05,
            "a keyframe seek to 15.0 lands on the keyframe at 10.0, not {landed}"
        );
        // A keyframe seek and an exact seek to the same target back to back:
        // the flight keys on the mode too, so the exact one is sent while the
        // keyframe seek runs (keyed on the target alone it could be skipped
        // and the picture stay on the keyframe at 10.0; whether mpv is already
        // `seeking` when the second one arrives depends on timing, so the key
        // itself is asserted by the unit test above). mpv may merge the two
        // queued seeks into one, so only the landing is asserted here.
        submit(
            &mut sources,
            7,
            json!({"kind":"control","control":{"kind":"seek","seconds":17.0,"keyframes":true}}),
        );
        submit(
            &mut sources,
            8,
            json!({"kind":"control","control":{"kind":"seek","seconds":17.0}}),
        );
        pump(&mut sources, &|e| matches!(e, MpvEvent::PlaybackRestart));
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end
            && (mpv.get_property::<bool>("seeking").unwrap_or(true)
                || (mpv.get_property::<f64>("time-pos").unwrap() - 17.0).abs() > 0.02)
        {
            while let Some(event) = mpv.poll_event() {
                sources.event(&event);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let landed = mpv.get_property::<f64>("time-pos").unwrap();
        assert!(
            (landed - 17.0).abs() < 0.02,
            "exact after keyframes to the same target lands exactly, not at {landed}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod audio_track_tests {
    use super::*;
    use crate::playback::media_tracks::AudioTrack;
    fn track(track_id: i64, external: bool) -> AudioTrack {
        AudioTrack {
            track_id,
            stream_index: None,
            selected: false,
            external,
            codec: Some("mp3".into()),
            title: None,
            language: None,
        }
    }
    #[test]
    fn audio_devices_skip_auto_and_duplicate_backends() {
        let device = |name: &str, description: &str| {
            Node::Map(vec![
                ("name".into(), Node::String(name.into())),
                ("description".into(), Node::String(description.into())),
            ])
        };
        let list = device_list(vec![
            device("auto", "Autoselect device"),
            device("coreaudio/BuiltInSpeakerDevice", "MacBook Speakers"),
            device("avfoundation/BuiltInSpeakerDevice", "MacBook Speakers"),
            device("coreaudio/USB-DAC", "DAC"),
        ]);
        assert_eq!(
            list,
            vec![
                (
                    "coreaudio/BuiltInSpeakerDevice".into(),
                    "MacBook Speakers".into()
                ),
                ("coreaudio/USB-DAC".into(), "DAC".into()),
            ]
        );
        assert_eq!(
            device_list(vec![device("pulse/x", "X")]),
            vec![("pulse/x".into(), "X".into())]
        );
    }
    #[test]
    fn chapters_read_title_and_start() {
        let chapter = |title: Option<&str>, time: Node| {
            let mut fields = vec![("time".to_owned(), time)];
            if let Some(title) = title {
                fields.push(("title".into(), Node::String(title.into())));
            }
            Node::Map(fields)
        };
        let list = chapter_list(Node::Array(vec![
            chapter(Some("Opening"), Node::Double(0.)),
            chapter(Some(""), Node::Double(90.5)),
            chapter(None, Node::Int64(300)),
            chapter(Some("bad"), Node::String("x".into())),
        ]));
        assert_eq!(
            list,
            vec![
                json!({"title":"Opening","start":0.0}),
                json!({"title":null,"start":90.5}),
                json!({"title":null,"start":300.0}),
            ]
        );
        assert!(chapter_list(Node::None).is_empty());
    }
    #[test]
    fn only_one_new_external_track_counts_as_the_added_one() {
        let before = [track(1, false)];
        assert_eq!(
            added_external(&before, &[track(1, false), track(2, true)])
                .unwrap()
                .track_id,
            2
        );
        assert!(added_external(&before, &[track(1, false)]).is_none());
        assert!(added_external(&before, &[track(1, false), track(2, false)]).is_none());
        assert!(
            added_external(&before, &[track(1, false), track(2, true), track(3, true)]).is_none()
        );
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use crate::playback::source_resolvers::tests::rom;
    const PLAYER: u64 = 1;
    const EMULATOR: u64 = 2;
    fn mpv() -> Mpv {
        Mpv::builder()
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .build()
            .unwrap()
    }
    fn call(
        sources: &mut Sources,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        sources
            .dispatch(mpv, owner, method, &params)
            .map(|value| value.unwrap_or(Value::Null))
    }
    /// The player opens `source` with operation `id`; the resolution request sent.
    fn open(sources: &mut Sources, mpv: &Mpv, id: u64, source: &str, paused: bool) -> Value {
        let status = call(
            sources,
            mpv,
            PLAYER,
            "source.submit",
            json!({"operationId":id,"intent":{"kind":"open","source":source,"startPaused":paused}}),
        )
        .unwrap();
        assert_eq!(status["status"], "pending");
        let events = sources.resolver_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, EMULATOR);
        assert_eq!(events[0].1, "source.resolveRequest");
        events[0].2.clone()
    }
    fn registered() -> (Sources, Mpv) {
        let mpv = mpv();
        let mut sources = Sources::default();
        call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.register",
            json!({"id":"nes","extensions":["nes"]}),
        )
        .unwrap();
        (sources, mpv)
    }
    fn operation(sources: &mut Sources, mpv: &Mpv, id: u64) -> Value {
        call(
            sources,
            mpv,
            PLAYER,
            "source.operation",
            json!({"operationId":id}),
        )
        .unwrap()
    }
    #[test]
    fn a_file_open_waits_for_its_resolver_and_keeps_the_path() {
        let (mut sources, mpv) = registered();
        let request = open(&mut sources, &mpv, 1, &rom("Game.NES"), false);
        assert_eq!(request["resolverId"], "nes");
        assert_eq!(request["url"], rom("Game.NES"));
        let state = sources.session.state();
        assert_eq!(state.phase, Phase::Loading);
        assert_eq!(state.source.as_deref(), Some(rom("Game.NES").as_str()));
        // Other files still go to mpv.
        assert!(sources.resolvers.find(&rom("a.mkv")).is_none());
    }
    #[test]
    fn the_resolvers_error_is_shown_in_its_own_words() {
        let (mut sources, mpv) = registered();
        let request = open(&mut sources, &mpv, 1, &rom("bad.nes"), false);
        let error = call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.respond",
            json!({"requestId":request["requestId"],"error":"不是 iNES 文件"}),
        )
        .unwrap_err();
        assert_eq!(error.code, "resolution");
        let state = sources.session.state();
        assert_eq!(state.phase, Phase::Error);
        assert_eq!(
            state.terminal_reason,
            Some(EndReason::Failed("不是 iNES 文件".into()))
        );
        assert_eq!(
            operation(&mut sources, &mpv, 1),
            json!({"status":"failed","error":{"backend":"不是 iNES 文件"}})
        );
    }
    #[test]
    fn a_file_resolver_answers_with_a_stream_only() {
        let (mut sources, mpv) = registered();
        let request = open(&mut sources, &mpv, 1, &rom("a.nes"), false);
        let media = json!({"kind":"media","originalUrl":rom("a.nes"),"primary":{"url":"https://cdn.example/a.mp4"}});
        let error = call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.respond",
            json!({"requestId":request["requestId"],"result":media}),
        )
        .unwrap_err();
        assert_eq!(error.message, "invalid resolved stream descriptor");
        assert_eq!(sources.session.state().phase, Phase::Error);
        // A producer it does not hold is refused the same way, as a failed open.
        let request = open(&mut sources, &mpv, 2, &rom("a.nes"), false);
        let error = call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.respond",
            json!({"requestId":request["requestId"],"result":{"kind":"stream","producerId":9}}),
        )
        .unwrap_err();
        assert_eq!(error.code, "resolution");
        assert_eq!(sources.session.state().phase, Phase::Error);
        assert_eq!(
            sources.session.state().source.as_deref(),
            Some(rom("a.nes").as_str())
        );
    }
    #[test]
    fn closing_or_disabling_cancels_the_resolution() {
        let (mut sources, mpv) = registered();
        let request = open(&mut sources, &mpv, 1, &rom("a.nes"), false);
        call(
            &mut sources,
            &mpv,
            PLAYER,
            "source.submit",
            json!({"operationId":2,"intent":{"kind":"close"}}),
        )
        .unwrap();
        let events = sources.resolver_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, "source.resolveCancel");
        assert_eq!(events[0].2["requestId"], request["requestId"]);
        let late = call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.respond",
            json!({"requestId":request["requestId"],"result":{"kind":"stream","producerId":1}}),
        )
        .unwrap_err();
        assert_eq!(late.code, "stale");
        assert_eq!(sources.session.state().phase, Phase::Empty);
        // The resolver's plugin stops while resolving: cancelled, and the
        // extension goes back to mpv.
        let request = open(&mut sources, &mpv, 3, &rom("b.nes"), false);
        sources.disable(EMULATOR);
        let events = sources.resolver_events();
        assert_eq!(events[0].1, "source.resolveCancel");
        assert_eq!(events[0].2["requestId"], request["requestId"]);
        assert_eq!(sources.session.state().phase, Phase::Error);
        assert_eq!(operation(&mut sources, &mpv, 3)["status"], json!("failed"));
        assert!(sources.resolvers.find(&rom("b.nes")).is_none());
    }
    #[test]
    fn web_resolver_errors_drop_the_old_prefix_too() {
        let mpv = mpv();
        let mut sources = Sources::default();
        call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.register",
            json!({"id":"web","schemes":["https"],"hosts":["media.example"]}),
        )
        .unwrap();
        let request = open(&mut sources, &mpv, 1, "https://media.example/v/1", false);
        let _ = call(
            &mut sources,
            &mpv,
            EMULATOR,
            "sourceResolvers.respond",
            json!({"requestId":request["requestId"],"error":"需要登录"}),
        );
        assert_eq!(
            sources.session.state().terminal_reason,
            Some(EndReason::Failed("需要登录".into()))
        );
    }
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Ordinary local media gets a lease on Unix and Windows alike (Windows
    /// had none before 0.5.8: snapshots were never available there); libmpv
    /// reads it as `fd://` or `vestra-lease://`, never by path.
    #[test]
    fn ordinary_local_media_is_leased_on_unix_and_windows() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.MP4");
        std::fs::write(&clip, b"not decoded here").unwrap();
        let path = clip.to_str().unwrap();
        let media = LocalMedia::open(path, true).unwrap();
        if cfg!(any(unix, windows)) {
            let media = media.expect("a leased local file");
            assert_eq!(media.lease.facts().len, 16);
            let url = media.url();
            #[cfg(windows)]
            {
                assert!(url.starts_with("vestra-lease://"), "{url}");
                let mut stream = crate::playback::lease_stream::open(&url).unwrap();
                let mut bytes = [0; 16];
                assert_eq!(stream.read(&mut bytes).unwrap(), 16);
                assert_eq!(&bytes, b"not decoded here");
                // Without the protocol on the player: played by path, no lease.
                assert!(LocalMedia::open(path, false).unwrap().is_none());
            }
            #[cfg(unix)]
            assert!(url.starts_with("fd://"), "{url}");
        }
        let playlist = dir.path().join("list.m3u");
        std::fs::write(&playlist, b"clip.MP4\n").unwrap();
        for other in [
            playlist.to_str().unwrap(),
            dir.path().join("missing.mp4").to_str().unwrap(),
            dir.path().to_str().unwrap(),
            "https://example.com/a.mp4",
        ] {
            assert!(LocalMedia::open(other, true).unwrap().is_none(), "{other}");
        }
    }

    /// Windows: when the file cannot be leased (here another program holds
    /// it without sharing), the open goes on without a lease; libmpv opens
    /// the path and reports its own error if it cannot either.
    #[cfg(windows)]
    #[test]
    fn a_file_that_cannot_be_leased_opens_without_a_lease() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        std::fs::write(&clip, b"bytes").unwrap();
        let exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&clip)
            .unwrap();
        let path = clip.to_str().unwrap();
        assert!(LocalMedia::open(path, true).unwrap().is_none());
        drop(exclusive);
        assert!(LocalMedia::open(path, true).unwrap().is_some());
    }

    fn ffmpeg() -> std::path::PathBuf {
        std::env::var_os("VESTRA_FFMPEG").map_or_else(|| "ffmpeg".into(), Into::into)
    }

    fn make(args: &[&str], path: &Path) {
        let made = std::process::Command::new(ffmpeg())
            .args(["-loglevel", "error", "-y"])
            .args(args)
            .arg(path)
            .status()
            .unwrap();
        assert!(made.success());
    }

    fn clip(path: &Path) {
        make(
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
            ],
            path,
        );
    }

    /// mpv 0.41 by path, `cover-art-auto=exact` (checked with libmpv): the
    /// same name in any case first, then whitelisted names in list order;
    /// not `Song-extra.jpg` or `Song.mp3.jpg` (those are `fuzzy`), and none
    /// for a video file. Image files are those `image-exts` names.
    #[test]
    fn cover_art_follows_mpvs_exact_rule() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "Song.mp3",
            "song.jpg",
            "Song.png",
            "cover.jpg",
            "Folder.png",
            "AlbumArt.webp",
            "Song-extra.jpg",
            "Song.mp3.jpg",
            "other.jpg",
            "Song.txt",
            "clip.mp4",
            "clip.jpg",
        ] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("front.jpg")).unwrap();
        let whitelist = "AlbumArt,Album,cover,front,AlbumArtSmall,Folder,.folder,thumb";
        let defaults = player().get_property::<String>("image-exts").unwrap();
        assert!(defaults.split(',').any(|ext| ext == "webp"), "{defaults}");
        let names_in = |media: &str, auto: &str, exts: &str| {
            cover_art(&dir.path().join(media), auto, whitelist, exts)
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let names = |media: &str, auto: &str| names_in(media, auto, &defaults);
        assert_eq!(
            names("Song.mp3", "exact"),
            [
                "song.jpg",
                "Song.png",
                "AlbumArt.webp",
                "cover.jpg",
                "Folder.png"
            ]
        );
        assert!(names("Song.mp3", "no").is_empty());
        assert!(names("clip.mp4", "exact").is_empty());
        assert_eq!(
            cover_art(Path::new("Song.mp3"), "exact", "", &defaults).len(),
            0
        );
        // A changed `image-exts` (any case) changes what counts as a picture.
        assert_eq!(
            names_in("Song.mp3", "exact", "PNG,txt"),
            ["Song.png", "Song.txt", "Folder.png"]
        );
        assert!(names_in("Song.mp3", "exact", "").is_empty());
    }

    fn player() -> Mpv {
        Mpv::builder()
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .set_property("keep-open", true)
            .unwrap()
            .set_property("cover-art-auto", "exact")
            .unwrap()
            .build()
            .unwrap()
    }

    fn submit(sources: &mut Sources, mpv: &Mpv, id: u64, intent: Value) {
        sources
            .dispatch(
                mpv,
                1,
                "source.submit",
                &json!({"operationId":id,"intent":intent}),
            )
            .unwrap();
    }

    /// Pumps events into `sources` until one matches `until` (20 s at most).
    fn pump(sources: &mut Sources, mpv: &Mpv, until: impl Fn(&MpvEvent) -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < end, "no event in time");
            while let Some(event) = mpv.poll_event() {
                sources.event(&event);
                if until(&event) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Real local files: a leased load offers snapshots (the GUI's Video >
    /// Snapshot items); the playing file can be renamed and, after the next
    /// file or Close, deleted; without the protocol a file plays by path
    /// without snapshots. Needs FFmpeg with libx264 (`VESTRA_FFMPEG`, else
    /// `ffmpeg` on PATH):
    /// `cargo test --locked -- --ignored a_local_file_offers_snapshots`.
    #[test]
    #[ignore = "needs ffmpeg with libx264"]
    fn a_local_file_offers_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.mp4");
        let second = dir.path().join("second.mp4");
        clip(&first);
        clip(&second);
        let mpv = player();
        let mut sources = Sources::default();
        sources.register_lease_streams(&mpv);
        let restarted = |event: &MpvEvent| matches!(event, MpvEvent::PlaybackRestart);
        let open =
            |path: &Path| json!({"kind":"open","source":path.to_str().unwrap(),"startPaused":true});
        submit(&mut sources, &mpv, 1, open(&first));
        pump(&mut sources, &mpv, restarted);
        let state = sources.snapshot(&mpv);
        assert_eq!(state["container"], "mov,mp4,m4a,3gp,3g2,mj2", "{state}");
        assert_eq!(state["capabilities"]["snapshotSource"], true, "{state}");
        let path = mpv.get_property::<String>("path").unwrap();
        assert!(
            path.starts_with("fd://") || path.starts_with("vestra-lease://"),
            "{path}"
        );
        let lease = sources.lease(sources.generation()).unwrap();
        assert_eq!(lease.facts().len, std::fs::metadata(&first).unwrap().len());
        drop(lease);
        // Playing: the file can be renamed, and playback and the lease go on.
        let moved = dir.path().join("moved.mp4");
        std::fs::rename(&first, &moved).unwrap();
        let seek = json!({"kind":"control","control":{"kind":"seek","seconds":1.0}});
        submit(&mut sources, &mpv, 2, seek);
        pump(&mut sources, &mpv, restarted);
        let state = sources.snapshot(&mpv);
        assert_eq!(state["capabilities"]["snapshotSource"], true, "{state}");
        // The next file: the previous one can be deleted.
        submit(&mut sources, &mpv, 3, open(&second));
        pump(&mut sources, &mpv, restarted);
        std::fs::remove_file(&moved).unwrap();
        let state = sources.snapshot(&mpv);
        assert_eq!(state["capabilities"]["snapshotSource"], true, "{state}");
        // Close: the file can be deleted.
        submit(&mut sources, &mpv, 4, json!({"kind":"close"}));
        pump(&mut sources, &mpv, |event| {
            matches!(event, MpvEvent::EndFile { .. })
        });
        std::fs::remove_file(&second).unwrap();
        // Audio: the cover art beside it is loaded as mpv would by path.
        let song = dir.path().join("Song.mp3");
        make(&["-f", "lavfi", "-i", "sine=d=2"], &song);
        for image in ["song.jpg", "cover.jpg"] {
            let picture = ["-f", "lavfi", "-i", "color=c=red:s=16x16", "-frames:v", "1"];
            make(&picture, &dir.path().join(image));
        }
        submit(&mut sources, &mpv, 5, open(&song));
        pump(&mut sources, &mpv, restarted);
        let pictures = (0..mpv.get_property::<i64>("track-list/count").unwrap())
            .filter_map(|n| {
                let name = mpv
                    .get_property::<String>(&format!("track-list/{n}/external-filename"))
                    .ok()?;
                let selected = mpv
                    .get_property::<bool>(&format!("track-list/{n}/selected"))
                    .unwrap();
                let file = Path::new(&name).file_name()?.to_str()?.to_owned();
                Some((file, selected))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pictures,
            [
                ("song.jpg".to_owned(), true),
                ("cover.jpg".to_owned(), false)
            ]
        );
        // Without the protocol (Windows): plays by path, no snapshots.
        if cfg!(windows) {
            let third = dir.path().join("third.mp4");
            clip(&third);
            let plain = player();
            let mut unleased = Sources::default();
            submit(&mut unleased, &plain, 1, open(&third));
            pump(&mut unleased, &plain, restarted);
            let state = unleased.snapshot(&plain);
            assert_eq!(state["phase"], "ready", "{state}");
            assert_eq!(state["capabilities"]["snapshotSource"], false, "{state}");
        }
    }
}
