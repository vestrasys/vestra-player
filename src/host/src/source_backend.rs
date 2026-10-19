//! Candidate source contract over actual libmpv entry identities.
use crate::{
    RpcError,
    session::{
        Admission, Control, EndReason, IntentTicket, OperationBook, Phase, SourceEvent,
        SourceIntent, SourceSession,
    },
};
use rsmpv::{EndFileReason, Event as MpvEvent, Mpv, Node};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs::File, path::Path, sync::Arc};

struct LocalMedia {
    // This open description is used exclusively by libmpv, never by workers.
    playback: File,
    lease: Arc<crate::source_lease::Lease>,
}
impl LocalMedia {
    fn open(source: &str) -> Result<Option<Arc<Self>>, String> {
        #[cfg(unix)]
        {
            // Keep path-relative playlist/disc/reference loaders on their existing
            // path route; this finite lease route is for ordinary self-contained media.
            let extension = Path::new(source)
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(
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
            ) || source.contains("://")
                || !Path::new(source).is_file()
            {
                return Ok(None);
            }
            let playback = File::open(source).map_err(|e| e.to_string())?;
            let lease =
                crate::source_lease::Lease::open(Path::new(source)).map_err(|e| e.to_string())?;
            if !lease.matches_file(&playback).map_err(|e| e.to_string())? {
                return Err("source changed between independent opens".into());
            }
            Ok(Some(Arc::new(Self {
                playback,
                lease: Arc::new(lease),
            })))
        }
        #[cfg(not(unix))]
        {
            let _ = source;
            Ok(None)
        }
    }
    #[cfg(unix)]
    fn url(&self) -> String {
        use std::os::fd::AsRawFd;
        format!("fd://{}", self.playback.as_raw_fd())
    }
    #[cfg(not(unix))]
    fn url(&self) -> String {
        unreachable!("local fd sources unavailable on this target")
    }
}

#[derive(Default)]
pub struct Sources {
    session: SourceSession,
    resolvers: crate::source_resolvers::Registry,
    resolutions: BTreeMap<u64, Resolution>,
    resolution_next: u64,
    resolution_events: Vec<(u64, String, Value)>,
    books: BTreeMap<(u64, bool), OperationBook>,
    entries: BTreeMap<i64, IntentTicket>,
    active: Option<(i64, IntentTicket)>,
    loaded: Option<(i64, IntentTicket)>,
    legacy_id: BTreeMap<u64, u64>,
    eof_rearm: bool,
    local_media: BTreeMap<i64, Arc<LocalMedia>>,
    current_local: Option<(u64, Arc<LocalMedia>)>,
    lease_ready: Option<(i64, u64)>,
    #[cfg(feature = "producer-audio-probe")]
    producers: crate::producer::Producers,
    #[cfg(feature = "producer-audio-probe")]
    provider_ticket: Option<IntentTicket>,
    #[cfg(feature = "producer-audio-probe")]
    provider_health: Option<Value>,
    #[cfg(feature = "producer-audio-probe")]
    controls: crate::producer_controls::Controls,
    #[cfg(feature = "producer-audio-probe")]
    pending_legacy: bool,
    #[cfg(feature = "producer-audio-probe")]
    awaiting_audio: Option<(crate::producer_controls::Pending, bool)>,
    #[cfg(feature = "producer-audio-probe")]
    control_origin: Option<std::time::Instant>,
    #[cfg(feature = "producer-audio-probe")]
    pending_error: Option<(IntentTicket, String)>,
    #[cfg(feature = "producer-audio-probe")]
    last_ack: Option<(u64, crate::producer_controls::Notice, Value)>,
    #[cfg(feature = "producer-audio-probe")]
    input: Option<InputBinding>,
    #[cfg(feature = "producer-audio-probe")]
    input_epoch: u64,
}
struct Resolution {
    requester: u64,
    operation: u64,
    legacy: bool,
    resolver: u64,
    resolver_id: String,
    source: String,
    paused: bool,
    ticket: IntentTicket,
    deadline: std::time::Instant,
}
#[cfg(feature = "producer-audio-probe")]
struct InputBinding {
    delivery: crate::producer_input_delivery::Delivery,
    keys: Vec<String>,
}
#[cfg(feature = "producer-audio-probe")]
impl InputBinding {
    fn physical_key(
        &mut self,
        scope: &crate::producer_input::Scope,
        code: &str,
        down: bool,
        now: u64,
    ) -> Result<(), crate::producer_input_delivery::Error> {
        match self.delivery.key(scope, code, down, false, now) {
            Err(crate::producer_input_delivery::Error::Busy) => {
                self.delivery
                    .revoke(crate::producer_input::RevokeReason::TransportFailure, now)?;
                Err(crate::producer_input_delivery::Error::Busy)
            }
            result => result.map(|_| ()),
        }
    }
}
impl Sources {
    #[cfg(feature = "producer-audio-probe")]
    fn input_error(error: crate::producer_input_delivery::Error) -> RpcError {
        use crate::producer_input_delivery::Error;
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
    #[cfg(feature = "producer-audio-probe")]
    pub fn input_routing(&self) -> Option<crate::producer_input::Routing> {
        let input = self.input.as_ref()?;
        let notice = input.delivery.notice();
        (self.producers.active() == Some(notice.scope.producer_id)).then_some(
            crate::producer_input::Routing {
                scope: notice.scope,
                keys: input.keys.clone(),
                captured: notice.captured,
            },
        )
    }
    #[cfg(feature = "producer-audio-probe")]
    fn input_facts(&self) -> Value {
        self.input.as_ref().map_or(Value::Null, |input| json!({
            "state": input.delivery.notice(), "pending":input.delivery.pending(), "keys":input.keys,
        }))
    }
    #[cfg(feature = "producer-audio-probe")]
    fn input_failed(&mut self, message: String) {
        if let Some(mut ticket) = self.provider_ticket {
            ticket.stamp = self.session.state().stamp;
            ticket.cycle = self.session.state().cycle;
            self.pending_error = Some((ticket, message));
        }
    }
    /// Called only by the host's native input adapter, never routed from plugins.
    #[cfg(feature = "producer-audio-probe")]
    pub fn host_input(&mut self, params: &Value) -> Result<(), RpcError> {
        let now = self.now_ms();
        let paused = self
            .producers
            .active_facts()
            .is_none_or(|(paused, _)| paused);
        let result = match params["kind"].as_str() {
            Some("capture") if !paused && self.phase() == Phase::Ready => {
                let scope: crate::producer_input::Scope =
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
            if error == crate::producer_input_delivery::Error::Busy && params["kind"] == "key" {
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
                crate::producer_input_delivery::Error::TimedOut
                    | crate::producer_input_delivery::Error::Exhausted
            ) {
                self.input_failed(format!("producer input delivery: {error:?}"));
            }
            Self::input_error(error)
        })
    }
    /// A local accepted object bound to the actual current native load, not a path reopen.
    pub fn lease(&self, generation: u64) -> Result<Arc<crate::source_lease::Lease>, RpcError> {
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
    /// Only call after an accepted actual FileLoaded event. Not a frame fence.
    pub fn ready_load(&self) -> Option<(i64, IntentTicket)> {
        self.active.filter(|(_, ticket)| {
            self.session.state().phase == Phase::Ready
                && ticket.stamp.generation == self.generation()
        })
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

    #[cfg(feature = "producer-audio-probe")]
    fn now_ms(&mut self) -> u64 {
        self.control_origin
            .get_or_insert_with(std::time::Instant::now)
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    #[cfg(feature = "producer-audio-probe")]
    fn finish_provider_control(
        &mut self,
        pending: crate::producer_controls::Pending,
        legacy: bool,
        result: Result<(), String>,
    ) {
        if let Some(book) = self.books.get_mut(&(pending.requester, legacy)) {
            let _ = book.finish(pending.operation_id, result);
        }
    }
    #[cfg(feature = "producer-audio-probe")]
    fn cancel_provider_control(&mut self, message: &str) {
        if let Some(p) = self.controls.cancel() {
            self.finish_provider_control(p, self.pending_legacy, Err(message.into()));
        }
        if let Some((p, legacy)) = self.awaiting_audio.take() {
            self.finish_provider_control(p, legacy, Err(message.into()));
        }
    }
    #[cfg(feature = "producer-audio-probe")]
    pub fn tick_provider_controls(&mut self) {
        let now = self.now_ms();
        if self
            .input
            .as_ref()
            .is_some_and(|input| input.delivery.expired(now))
        {
            self.input_failed("producer physical-input acknowledgement timed out".into());
        }
        let expired = self.controls.expire(now).map(|p| (p, self.pending_legacy));
        let audio_expired = self
            .awaiting_audio
            .as_ref()
            .is_some_and(|(p, _)| now >= p.deadline_ms);
        let expired = expired.or_else(|| {
            if audio_expired {
                self.awaiting_audio.take()
            } else {
                None
            }
        });
        if let Some((p, legacy)) = expired {
            let ticket = p.ticket;
            self.finish_provider_control(
                p,
                legacy,
                Err("provider frame-bound control timed out".into()),
            );
            self.pending_error = Some((ticket, "provider frame-bound control timed out".into()));
        }
        if let Some((p, _)) = &self.awaiting_audio
            && self.producers.audio_acknowledged(
                p.notice.producer_id,
                p.notice.epoch,
                p.notice.paused,
            )
        {
            let (p, legacy) = self.awaiting_audio.take().unwrap();
            self.finish_provider_control(p, legacy, Ok(()));
        }
    }
    #[cfg(feature = "producer-audio-probe")]
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
    #[cfg(feature = "producer-audio-probe")]
    pub fn producer_frame(&mut self) -> Option<crate::producer::Frame> {
        self.producers.frame()
    }
    #[cfg(feature = "producer-audio-probe")]
    pub fn provider_active(&self) -> bool {
        self.producers.active().is_some()
    }
    #[cfg(feature = "producer-audio-probe")]
    pub fn producer_output(&mut self, volume: f64, muted: bool) {
        self.producers.output(volume, muted);
    }
    pub fn source(&self) -> Option<String> {
        self.session.state().source.clone()
    }
    pub fn snapshot(&mut self, mpv: &Mpv) -> Value {
        #[cfg(feature = "producer-audio-probe")]
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
        result["audioTracks"] = match self.presentation_load() {
            Some(_) => crate::media_tracks::audio_tracks(mpv)
                .map(|tracks| json!(tracks))
                .unwrap_or(Value::Null),
            None => Value::Null,
        };
        let ready = matches!(self.session.state().phase, Phase::Ready | Phase::Ended);
        result["capabilities"] = json!({"pause":ready,"seek":ready&&mpv.get_property::<bool>("seekable").unwrap_or(false),"rate":ready,"restart":self.session.state().source.is_some(),"close":self.session.state().source.is_some()});
        result["output"] = json!({"volume":mpv.get_property::<f64>("volume").unwrap_or(100.),"muted":mpv.get_property::<bool>("mute").unwrap_or(false)});
        result["rate"] = json!(mpv.get_property::<f64>("speed").unwrap_or(1.));
        result["capabilities"]["snapshotSource"] = json!(self.lease(self.generation()).is_ok());
        result["snapshotProfile"] = json!({"timeDomain":"mpv-media","fileOriginValidation":"deferred-worker-origin-match-required"});
        result
    }
    pub fn event(&mut self, event: &MpvEvent) -> bool {
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
        if let Some(book) = self
            .books
            .get_mut(&(resolution.requester, resolution.legacy))
        {
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
                    || !self.books.contains_key(&(r.requester, r.legacy))
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
                let result = (|| -> Result<(), String> {
                    if let Some(error) = params["error"].as_str() {
                        return Err(format!(
                            "resolver failed: {}",
                            error.chars().take(2048).collect::<String>()
                        ));
                    }
                    let media: crate::source_resolvers::Media =
                        serde_json::from_value(params["result"].clone())
                            .map_err(|_| "invalid resolved media descriptor")?;
                    let (source, options) = media.playback(&r.source)?;
                    let entry = load_with_options(mpv, &source, r.paused, &options)?;
                    self.current_local = None;
                    self.entries.clear();
                    self.entries.insert(entry, r.ticket);
                    Ok(())
                })();
                self.finish_resolution(&r, result.clone());
                result.map_err(|message| RpcError::new("resolution", message))?;
                Ok(Value::Null)
            }
            _ => Err(RpcError::new(
                "unsupported",
                "unknown source resolver method",
            )),
        }
    }
    pub fn disable(&mut self, owner: u64) {
        self.resolvers.disable(owner);
        #[cfg(feature = "producer-audio-probe")]
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
        self.books.retain(|(id, _), _| *id != owner);
        self.legacy_id.remove(&owner);
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
        #[cfg(feature = "producer-audio-probe")]
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
                        .any(|key| !crate::producer_input::supported_code(key))
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
                    let state = crate::producer_input::State::new(
                        crate::producer_input::Scope {
                            owner,
                            producer_id: id,
                            ticket,
                            input_epoch: epoch,
                        },
                        keys.clone(),
                    )
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                    self.input = Some(InputBinding {
                        delivery: crate::producer_input_delivery::Delivery::new(state),
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
                    let notice: crate::producer_controls::Notice =
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
                                    crate::producer_controls::Error::Unavailable => "unavailable",
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
                        self.finish_provider_control(
                            pending,
                            self.pending_legacy,
                            Err(error.message.clone()),
                        );
                        self.pending_error = Some((ticket, error.message.clone()));
                        return Err(error);
                    }
                    self.awaiting_audio = Some((pending, self.pending_legacy));
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
                .entry((owner, false))
                .or_insert_with(|| OperationBook::new(owner));
            return book
                .query(id)
                .map(|s| Some(json!(s)))
                .map_err(|e| RpcError::new("unknown-expired", format!("{e:?}")));
        }
        let legacy = method != "source.submit";
        let (id, intent) = if !legacy {
            let id = params["operationId"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or_else(|| RpcError::invalid("operationId requires positive integer"))?;
            let intent = serde_json::from_value::<SourceIntent>(params["intent"].clone())
                .map_err(|e| RpcError::invalid(e.to_string()))?;
            (id, intent)
        } else {
            let intent = match method {
                "playback.open" => SourceIntent::Open {
                    source: params["source"]
                        .as_str()
                        .ok_or_else(|| RpcError::invalid("source requires string"))?
                        .into(),
                    start_paused: false,
                    expected: None,
                },
                "playback.seek" => SourceIntent::Control {
                    control: Control::Seek {
                        seconds: params["seconds"]
                            .as_f64()
                            .ok_or_else(|| RpcError::invalid("seconds requires number"))?,
                    },
                    expected: None,
                },
                "playback.set" => match params["property"].as_str() {
                    Some("pause") => SourceIntent::Control {
                        control: Control::Pause {
                            paused: params["value"]
                                .as_bool()
                                .ok_or_else(|| RpcError::invalid("pause requires boolean"))?,
                        },
                        expected: None,
                    },
                    Some("speed") => SourceIntent::Control {
                        control: Control::Rate {
                            rate: params["value"]
                                .as_f64()
                                .ok_or_else(|| RpcError::invalid("speed requires number"))?,
                        },
                        expected: None,
                    },
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            };
            let id = self.legacy_id.entry(owner).or_default();
            *id += 1;
            (*id, intent)
        };
        #[cfg(feature = "producer-audio-probe")]
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
            Err(crate::session::RequestError::Busy)
        } else {
            match &intent {
                SourceIntent::OpenProvider { producer_id, .. } => {
                    self.producers.ready(*producer_id, owner).map_err(|e| {
                        crate::session::RequestError::Backend(format!("{}: {}", e.code, e.message))
                    })
                }
                SourceIntent::Restart { .. }
                | SourceIntent::Control {
                    control: Control::Seek { .. } | Control::Rate { .. },
                    ..
                } if self.producers.active().is_some() => {
                    Err(crate::session::RequestError::Backend(
                        "unsupported: producer seek/rate/restart unsupported".into(),
                    ))
                }
                _ => Ok(()),
            }
        };
        #[cfg(not(feature = "producer-audio-probe"))]
        let preflight = if matches!(intent, SourceIntent::OpenProvider { .. }) {
            Err(crate::session::RequestError::Backend(
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
                Err(crate::session::RequestError::Busy)
            } else {
                Ok(())
            }
        });
        let book = self
            .books
            .entry((owner, legacy))
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
                        crate::session::RequestError::Stale => "stale",
                        crate::session::RequestError::UnknownExpired => "unknown-expired",
                        crate::session::RequestError::Busy => "busy",
                        crate::session::RequestError::Conflict => "conflict",
                        crate::session::RequestError::Backend(ref message)
                            if message.starts_with("unavailable:") =>
                        {
                            "unavailable"
                        }
                        crate::session::RequestError::Backend(ref message)
                            if message.starts_with("conflict:") =>
                        {
                            "conflict"
                        }
                        crate::session::RequestError::Backend(_) => "unsupported",
                        _ => "invalid",
                    },
                    format!("{e:?}"),
                ));
            }
        };
        #[cfg(feature = "producer-audio-probe")]
        if matches!(
            intent,
            SourceIntent::Open { .. } | SourceIntent::Close { .. }
        ) {
            self.producers.stop_active();
            self.provider_ticket = None;
        }
        #[cfg(feature = "producer-audio-probe")]
        if matches!(
            intent,
            SourceIntent::Open { .. }
                | SourceIntent::OpenProvider { .. }
                | SourceIntent::Close { .. }
        ) {
            self.input = None;
            self.cancel_provider_control("source superseded the provider control");
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
                    .get_mut(&(owner, legacy))
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
                    legacy,
                    resolver,
                    resolver_id,
                    source,
                    paused,
                    ticket,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
                },
            );
            return Ok(Some(if legacy {
                json!({"session":ticket.stamp.generation})
            } else {
                json!({"status":"pending","ticket":ticket})
            }));
        }
        let result: Result<(), String> = match &intent {
            SourceIntent::OpenProvider {
                producer_id,
                start_paused,
                ..
            } => {
                #[cfg(feature = "producer-audio-probe")]
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
                #[cfg(not(feature = "producer-audio-probe"))]
                {
                    let _ = (producer_id, start_paused);
                    Err("producer support not enabled in this build".into())
                }
            }
            SourceIntent::Open {
                source,
                start_paused,
                ..
            } => LocalMedia::open(source).and_then(|media| {
                #[cfg(feature = "producer-audio-probe")]
                {
                    self.producers.deactivate();
                    self.provider_ticket = None;
                }
                let url = media
                    .as_ref()
                    .map(|media| media.url())
                    .unwrap_or_else(|| source.clone());
                // Keep the previous native load's File until its EndFile/new Ready.
                let entry = load(mpv, &url, *start_paused)?;
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
                load(mpv, &source, false).map(|entry| {
                    if let Some(media) = media {
                        self.local_media.insert(entry, media);
                    }
                    self.entries.clear();
                    self.entries.insert(entry, ticket);
                })
            }
            SourceIntent::Close { .. } => {
                #[cfg(feature = "producer-audio-probe")]
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
                #[cfg(feature = "producer-audio-probe")]
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
                                    .revoke(crate::producer_input::RevokeReason::Pause, now)
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
                                .get_mut(&(owner, legacy))
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
                        let book = self.books.get_mut(&(owner, legacy)).unwrap();
                        let _ = book.finish(id, Err(message.clone()));
                        self.pending_error = Some((ticket, message.clone()));
                        return Err(RpcError::new("backend", message));
                    }
                    self.pending_legacy = legacy;
                    return Ok(Some(if legacy {
                        Value::Null
                    } else {
                        json!(crate::session::OperationStatus::Pending { ticket })
                    }));
                }
                match control {
                    Control::Pause { paused } => mpv.set_property("pause", *paused),
                    Control::Seek { seconds } => {
                        mpv.command(&["seek", &seconds.to_string(), "absolute+exact"])
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
            .get_mut(&(owner, legacy))
            .unwrap()
            .finish(id, result.clone())
            .map_err(|e| RpcError::new("host", format!("{e:?}")))?;
        result.map_err(|e| RpcError::new("backend", e))?;
        Ok(Some(if legacy {
            if method == "playback.open" {
                json!({"session":ticket.stamp.generation})
            } else {
                Value::Null
            }
        } else {
            json!(status)
        }))
    }
}
fn load(mpv: &Mpv, source: &str, paused: bool) -> Result<i64, String> {
    load_with_options(mpv, source, paused, "")
}
fn load_with_options(mpv: &Mpv, source: &str, paused: bool, extra: &str) -> Result<i64, String> {
    let pause = if paused { "pause=yes" } else { "pause=no" };
    // ff-index is documented as reliable only with the mature lavf demuxer.
    // Apply this per leased, self-contained local load, not arbitrary URL loaders.
    let options = if source.starts_with("fd://") {
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

#[cfg(all(test, feature = "producer-audio-probe"))]
mod input_adapter_tests {
    use super::*;
    #[test]
    fn full_native_keyup_becomes_pinned_empty_release_instead_of_stuck_key() {
        use crate::{
            producer_input::{Scope, State},
            producer_input_delivery::{Delivery, Error},
        };
        let scope = Scope {
            owner: 1,
            producer_id: 1,
            input_epoch: 1,
            ticket: IntentTicket {
                owner: 2,
                stamp: crate::session::SourceStamp {
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
