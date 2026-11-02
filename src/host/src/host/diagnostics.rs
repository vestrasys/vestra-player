//! The UI thread's part of the host log and of the plugin shaders' crash
//! guard (doc/architecture/README.md §4.3):
//!
//! - plugin shader lists (`video.shaders.set`, `video.render.set`'s
//!   `shaders`) are followed by plugin and number of files, never paths; a
//!   change goes to the host log;
//! - the guard (`render_state`'s `shaders` record) is armed (`applying`)
//!   only while the lists are drawn: a video track plays and
//!   [`FIRST_FRAMES`] new frames were presented since the list changed. It
//!   settles (`ok`) after [`SHADER_STABLE_SECS`] of such playback (the timer
//!   starts again with a new source), and at once when playback pauses or
//!   stops, the video track goes (audio only), the lists empty, the player
//!   quits, or the session ends (Windows `WM_QUERYENDSESSION` /
//!   `WM_ENDSESSION`). A shutdown, logoff or kill while paused or idle is
//!   therefore never taken for a shader crash;
//! - the file is written by a thread of its own ([`ShaderWriter`]: the
//!   newest request wins), never on the UI thread, except at quit and at the
//!   session's end, which wait for it; a new list from the same plugins
//!   while armed writes nothing (a slider that sets lists as it moves);
//! - a run that keeps plugin shaders off (the last one ended while they
//!   were on, `GuardFile::shader_start`) refuses non-empty lists with
//!   [`PREVIOUS_CRASH`] and says so once on the OSD;
//! - every [`MEMORY_EVERY`] the memory line goes to the host log.
//!
//! The lists are followed as requested: one the worker then refuses (the
//! limit, a write mpv refused) still counts until it is replaced, so the
//! guard may be armed for nothing, never the other way round.
use super::*;
use crate::playback::render_state::{GuardFile, GuardKey, SHADER_STABLE_SECS, ShaderBlock};
use std::sync::{Condvar, Mutex, PoisonError};

/// The error code of a plugin shader list refused because the last run
/// ended while plugin shaders were on.
pub(crate) const PREVIOUS_CRASH: &str = "previous-crash";
/// How often the memory line goes to the host log.
pub(crate) const MEMORY_EVERY: Duration = Duration::from_secs(60);
/// New frames presented after a list changed before it counts as drawn
/// (the first one may still have been rendered with the old list).
pub(crate) const FIRST_FRAMES: u64 = 2;
/// `host.error` codes whose message carries no user data (no file, link,
/// title or label): only these log their (redacted) message.
const MESSAGE_CODES: &[&str] = &["source-geometry"];

/// A `host.error` for the host log: its code (identifier characters only),
/// and its message, redacted, only for [`MESSAGE_CODES`]; no other field.
pub(crate) fn host_error_line(data: &Value) -> String {
    let code = crate::diag::token(data["code"].as_str().unwrap_or_default(), 64);
    if MESSAGE_CODES.contains(&code.as_str()) {
        format!(
            "host.error {code}: {}",
            crate::diag::redact(data["message"].as_str().unwrap_or_default())
        )
    } else {
        format!("host.error {code}")
    }
}

/// A shader handle for the host log: `render`, or `shaders:` with the
/// plugin's id reduced to identifier characters.
fn handle_for_log(handle: &str) -> String {
    match handle.split_once(':') {
        Some((kind, id)) => format!("{kind}:{}", crate::diag::token(id, 64)),
        None => handle.to_owned(),
    }
}

/// A write of the shaders' record.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Applying(Vec<String>, u64),
    Settle,
}

#[derive(Debug, Default)]
struct WriterState {
    /// The newest request not taken yet (it replaces an older one).
    next: Option<Op>,
    busy: bool,
    started: bool,
    /// Requests written (tests).
    written: u64,
}

#[derive(Debug, Default)]
struct WriterShared {
    state: Mutex<WriterState>,
    changed: Condvar,
    /// A session end that was cancelled (Windows `WM_ENDSESSION` FALSE):
    /// the record was settled when it was asked, so an armed guard writes
    /// `applying` again at the next tick.
    cancelled: std::sync::atomic::AtomicBool,
    /// The session is ending (`WM_QUERYENDSESSION`, `WM_ENDSESSION` TRUE):
    /// no `applying` is written any more, until a cancel.
    ending: std::sync::atomic::AtomicBool,
    /// The `exit:` line for the session's end was written (once).
    exit_logged: std::sync::atomic::AtomicBool,
}

/// Writes the shaders' record off the UI thread (`write_replacing` syncs
/// the file to disk). One thread, started at the first request; the newest
/// request wins. Without a file nothing happens.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShaderWriter {
    file: GuardFile,
    key: GuardKey,
    shared: Arc<WriterShared>,
}

impl ShaderWriter {
    fn new(file: GuardFile, key: GuardKey) -> Self {
        Self {
            file,
            key,
            shared: Arc::default(),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, WriterState> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
    fn send(&self, op: Op) {
        if self.file.is_off()
            || (matches!(op, Op::Applying(..))
                && self
                    .shared
                    .ending
                    .load(std::sync::atomic::Ordering::Acquire))
        {
            return;
        }
        let mut state = self.lock();
        state.next = Some(op);
        if !state.started {
            let writer = self.clone();
            state.started = std::thread::Builder::new()
                .name("vestra-shader-guard".into())
                .spawn(move || writer.run())
                .is_ok();
        }
        self.shared.changed.notify_all();
    }
    fn execute(&self, op: Op) {
        match op {
            Op::Applying(plugins, at) => self.file.shaders_applying(&self.key, plugins, at),
            Op::Settle => self.file.shaders_settle(),
        }
    }
    fn run(self) {
        loop {
            let op = {
                let mut state = self.lock();
                while state.next.is_none() {
                    state = self
                        .shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                state.busy = true;
                state.next.take()
            };
            if let Some(op) = op {
                self.execute(op);
            }
            let mut state = self.lock();
            state.busy = false;
            state.written += 1;
            self.shared.changed.notify_all();
        }
    }
    /// Settle now, on this thread, after a write under way and instead of
    /// one waiting (quit, the session's end: the process may end next).
    pub(crate) fn settle_now(&self) {
        if self.file.is_off() {
            return;
        }
        let mut state = self.lock();
        while state.busy {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.next = None;
        state.busy = true;
        drop(state);
        self.file.shaders_settle();
        let mut state = self.lock();
        state.busy = false;
        state.written += 1;
        self.shared.changed.notify_all();
    }
    /// Wait up to `limit` for the requests handed over to be on disk (a
    /// device loss restarts the player and this process ends next: a
    /// pending `applying` must not be lost). Whether everything was
    /// written.
    pub(crate) fn persist(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut state = self.lock();
        while state.next.is_some() || state.busy {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
    /// Wait until every request was written (tests).
    #[cfg(test)]
    fn flush(&self) -> u64 {
        let mut state = self.lock();
        while state.next.is_some() || state.busy {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.written
    }
}

/// What the guard looks at each tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Observed {
    /// Playback runs (ready, not paused).
    pub playing: bool,
    /// The source has a moving video track (`source.state` `hasVideo`).
    pub video: bool,
    /// The source's generation (a new source starts the timer again).
    pub source: u64,
    /// New frames presented so far (`Presentation::counts`).
    pub presented: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    /// No lists.
    #[default]
    Off,
    /// Lists are set; waiting for them to be drawn while a video plays.
    Pending { mark: u64 },
    /// Drawn: `applying` is written; counting playback.
    Armed { played: Duration },
    /// Ran [`SHADER_STABLE_SECS`] of playback: `ok` until the lists change.
    Settled,
}

/// The plugin shaders' crash guard of this run.
#[derive(Debug, Default)]
pub(crate) struct ShaderGuard {
    writer: ShaderWriter,
    /// This run keeps plugin shaders off.
    blocked: Option<ShaderBlock>,
    /// A list was refused for it; the OSD has not said so yet.
    refused: bool,
    /// The OSD said so (once per run).
    told: bool,
    /// The lists that are on: (owner, handle) → (plugin, files).
    lists: BTreeMap<(u64, String), (String, usize)>,
    phase: Phase,
    /// The plugins `applying` was last written for, until it is settled.
    written: Option<Vec<String>>,
    source: u64,
    last_tick: Option<Instant>,
}

impl ShaderGuard {
    /// `blocked`: what `GuardFile::shader_start` said at start.
    pub(crate) fn new(file: GuardFile, key: GuardKey, blocked: Option<ShaderBlock>) -> Self {
        Self {
            writer: ShaderWriter::new(file, key),
            blocked,
            ..Self::default()
        }
    }
    /// What the session's end settles with (`main.rs`).
    pub(crate) fn writer(&self) -> ShaderWriter {
        self.writer.clone()
    }
    /// Why a list of `files` shader files is refused now, if it is: only
    /// non-empty lists while this run keeps plugin shaders off (clearing
    /// always works).
    pub(crate) fn refuse(&mut self, files: usize, now: u64) -> Option<RpcError> {
        let block = self.blocked.as_ref().filter(|_| files > 0)?;
        let whose = if block.plugins.is_empty() {
            String::new()
        } else {
            format!(" ({})", block.plugins.join(", "))
        };
        let message = match block.until {
            None => format!(
                "plugin shaders are off for this start: the player ended unexpectedly while they were on{whose}; the next start applies them again"
            ),
            Some(until) => format!(
                "plugin shaders are off: the player ended unexpectedly twice within a day while they were on{whose}; they apply again after a graphics driver or player update, or in {} days",
                until.saturating_sub(now).div_ceil(24 * 60 * 60)
            ),
        };
        if !self.refused && !self.told {
            logged!("shaders: a list was refused ({message})");
        }
        self.refused = true;
        Some(RpcError::new(PREVIOUS_CRASH, message))
    }
    /// The OSD line is due (once per run, after the first refusal, once a
    /// window can show it).
    pub(crate) fn take_osd(&mut self, can_show: bool) -> bool {
        let due = self.refused && !self.told && can_show;
        self.told |= due;
        due
    }
    /// The plugins whose lists are on, in order, once each.
    fn plugins(&self) -> Vec<String> {
        let mut plugins: Vec<String> = self.lists.values().map(|(id, _)| id.clone()).collect();
        plugins.sort();
        plugins.dedup();
        plugins
    }
    fn files(&self) -> usize {
        self.lists.values().map(|(_, files)| files).sum()
    }
    /// `ok` if `applying` was written (off the UI thread).
    fn settle(&mut self) {
        if self.written.take().is_some() {
            self.writer.send(Op::Settle);
        }
    }
    /// The lists changed: wait for them to be drawn again, unless they are
    /// already armed for the same plugins (no write; the timer restarts).
    fn changed(&mut self, presented: u64) {
        if self.lists.is_empty() {
            self.phase = Phase::Off;
            self.settle();
        } else if matches!(self.phase, Phase::Armed { .. })
            && self.written.as_ref() == Some(&self.plugins())
        {
            self.phase = Phase::Armed {
                played: Duration::ZERO,
            };
        } else {
            self.phase = Phase::Pending { mark: presented };
        }
    }
    /// A plugin's list `handle` (`render`, or `shaders:<id>`) now has
    /// `files` files, before the worker gets it (`presented`: new frames
    /// presented so far).
    pub(crate) fn set(
        &mut self,
        owner: u64,
        handle: &str,
        plugin: &str,
        files: usize,
        presented: u64,
    ) {
        let key = (owner, handle.to_owned());
        let before = self.lists.get(&key).map_or(0, |(_, files)| *files);
        if files == 0 && before == 0 {
            return;
        }
        if files == 0 {
            self.lists.remove(&key);
        } else {
            self.lists.insert(key, (plugin.to_owned(), files));
        }
        logged!(
            "shaders: {plugin} {} has {files} file(s); {} in all",
            handle_for_log(handle),
            self.files()
        );
        self.changed(presented);
    }
    /// A plugin stopped or was parked: the worker removes its lists.
    pub(crate) fn owner_gone(&mut self, owner: u64, presented: u64) {
        let before = self.lists.len();
        self.lists.retain(|(holder, _), _| *holder != owner);
        if self.lists.len() == before {
            return;
        }
        logged!(
            "shaders: plugin {owner} stopped; {} file(s) in all",
            self.files()
        );
        if self.lists.is_empty() {
            self.changed(presented);
        }
    }
    /// Every tick.
    pub(crate) fn tick(&mut self, seen: Observed, now: Instant) {
        let since = self.last_tick.replace(now);
        let drawn = seen.playing && seen.video;
        if std::mem::replace(&mut self.source, seen.source) != seen.source
            && let Phase::Armed { played } = &mut self.phase
        {
            *played = Duration::ZERO;
        }
        if self
            .writer
            .shared
            .cancelled
            .swap(false, std::sync::atomic::Ordering::AcqRel)
            && let Some(plugins) = self.written.clone()
        {
            self.writer
                .send(Op::Applying(plugins, crate::playback::render_state::now()));
        }
        match self.phase {
            Phase::Off | Phase::Settled => {}
            Phase::Pending { .. } if !drawn => {
                // A list that changed while armed, then paused or stopped.
                self.settle();
            }
            Phase::Pending { mark } => {
                if drawn && seen.presented >= mark.saturating_add(FIRST_FRAMES) {
                    let plugins = self.plugins();
                    if self.written.as_ref() != Some(&plugins) {
                        self.writer.send(Op::Applying(
                            plugins.clone(),
                            crate::playback::render_state::now(),
                        ));
                        self.written = Some(plugins);
                    }
                    self.phase = Phase::Armed {
                        played: Duration::ZERO,
                    };
                }
            }
            Phase::Armed { played } if drawn => {
                let played = played + since.map_or(Duration::ZERO, |since| now - since);
                if played >= Duration::from_secs(SHADER_STABLE_SECS) {
                    diag!(
                        "shaders: {SHADER_STABLE_SECS} s of playback with plugin shaders on; the crash guard settled"
                    );
                    self.settle();
                    self.phase = Phase::Settled;
                } else {
                    self.phase = Phase::Armed { played };
                }
            }
            Phase::Armed { .. } => {
                // Paused, stopped or no video: nothing is drawn with them.
                self.settle();
                self.phase = Phase::Pending {
                    mark: seen.presented,
                };
            }
        }
    }
    /// The player quits normally: settled on this thread.
    pub(crate) fn quit(&mut self) {
        self.written = None;
        self.writer.settle_now();
    }
}

/// The session asks to end, ends, or its end was cancelled. Asked or
/// ending: settled at once, on the UI thread inside the message (Windows may
/// end the process soon after), and no `applying` is written any more;
/// ending also writes the `exit:` line, once. Cancelled: a `session:` line
/// (no `exit:` came before it), and an armed guard writes `applying` again
/// at the next tick.
pub(crate) fn session_end(writer: &ShaderWriter, event: platform::SessionEvent) {
    use std::sync::atomic::Ordering;
    let shared = &writer.shared;
    match event {
        platform::SessionEvent::Query | platform::SessionEvent::Ending => {
            shared.ending.store(true, Ordering::Release);
            writer.settle_now();
            if event == platform::SessionEvent::Ending
                && !shared.exit_logged.swap(true, Ordering::AcqRel)
            {
                logged!("exit: the session ends (shutdown, restart or logoff)");
            }
        }
        platform::SessionEvent::Cancelled => {
            shared.ending.store(false, Ordering::Release);
            shared.exit_logged.store(false, Ordering::Release);
            shared.cancelled.store(true, Ordering::Release);
            logged!("session: the end of the session was cancelled; the player goes on");
        }
    }
}

/// The writer the device-loss restart waits for (`register_shader_writer`).
static RESTART_WRITER: Mutex<Option<ShaderWriter>> = Mutex::new(None);

/// The writer `persist_shader_guard` waits for (`main.rs`, once).
pub(crate) fn register_shader_writer(writer: ShaderWriter) {
    *RESTART_WRITER
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(writer);
}

/// Before a device loss restarts the player (`host/restart.rs`,
/// `restart_player`; any thread): a shader record handed to the writer is
/// put on disk first, for at most 2 s, so the `applying` of shaders that
/// were on survives this process and keeps them off in the restarted one.
pub(crate) fn persist_shader_guard() {
    let writer = RESTART_WRITER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(writer) = writer
        && !writer.persist(Duration::from_secs(2))
    {
        diag!("shaders: the crash guard's last write did not finish before the restart");
    }
}

impl Host {
    /// New frames presented so far.
    fn presented(&self) -> u64 {
        self.backend.render.presentation.counts().1
    }
    /// Before a plugin shader list of `files` files goes to the worker
    /// (`capabilities::processing_video`): refused while this run keeps
    /// plugin shaders off; otherwise followed (GPU rendering only: under
    /// software rendering the worker refuses non-empty lists).
    pub(crate) fn plugin_shaders(
        &mut self,
        owner: u64,
        handle: &str,
        files: usize,
    ) -> std::result::Result<(), RpcError> {
        let now = crate::playback::render_state::now();
        if let Some(error) = self.shader_guard.refuse(files, now) {
            return Err(error);
        }
        if files > 0 && !self.backend.render.now().0 {
            return Ok(());
        }
        let plugin = self
            .plugins
            .get(&owner)
            .map(|plugin| plugin.manifest.id.clone())
            .unwrap_or_else(|| format!("plugin {owner}"));
        let presented = self.presented();
        self.shader_guard
            .set(owner, handle, &plugin, files, presented);
        Ok(())
    }
    /// A plugin stopped or was parked (its lists go with it).
    pub(crate) fn plugin_shaders_gone(&mut self, owner: u64) {
        let presented = self.presented();
        self.shader_guard.owner_gone(owner, presented);
    }
    /// Every tick: the shader guard, its OSD line, and the memory line every
    /// [`MEMORY_EVERY`] while the host log is open.
    pub(crate) fn diagnostics_tick(&mut self) {
        let now = Instant::now();
        let source = &self.last_source;
        let seen = Observed {
            playing: source["phase"] == "ready" && source["paused"] == false,
            video: source["hasVideo"] == true,
            source: source["stamp"]["generation"].as_u64().unwrap_or(0),
            presented: self.presented(),
        };
        self.shader_guard.tick(seen, now);
        if self.shader_guard.take_osd(self.shell.is_some()) {
            self.show_osd(&i18n::tr(
                "Plugin shaders are off: the player ended unexpectedly while they were on",
            ));
        }
        if crate::diag::log_open()
            && self
                .memory_logged
                .is_none_or(|at| now.saturating_duration_since(at) >= MEMORY_EVERY)
        {
            self.memory_logged = Some(now);
            if let Some(status) = platform::memory_status() {
                logged!("{}", crate::diag::memory_line(&status));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::SessionEvent;
    use crate::playback::render_state::{ShaderState, now as unix_now};
    use std::sync::atomic::Ordering;

    fn key() -> GuardKey {
        GuardKey {
            adapter: "AMD Radeon 780M".into(),
            driver: "32.0.32015.7029".into(),
            app: "0.6.3".into(),
        }
    }

    /// The record as written, once the writer is idle.
    fn state(guard: &ShaderGuard, file: &GuardFile) -> Option<ShaderState> {
        guard.writer.flush();
        file.read().shaders.map(|record| record.state)
    }

    fn guard() -> (tempfile::TempDir, GuardFile, ShaderGuard) {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let guard = ShaderGuard::new(file.clone(), key(), None);
        (dir, file, guard)
    }

    fn playing(presented: u64) -> Observed {
        Observed {
            playing: true,
            video: true,
            source: 1,
            presented,
        }
    }

    /// Review item 2: `applying` only once a video plays and frames with
    /// the list were presented; paused or idle is never `applying`.
    #[test]
    fn the_guard_arms_only_once_the_list_is_drawn_and_settles_when_playback_stops() {
        let (_dir, file, mut guard) = guard();
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        // Set while nothing plays: nothing written, however long.
        guard.set(1, "shaders:x", "acme.a", 2, 0);
        guard.tick(Observed::default(), at(0));
        guard.tick(Observed::default(), at(100));
        assert_eq!(state(&guard, &file), None);
        // Paused on a video, frames redrawn: still nothing.
        let paused = Observed {
            playing: false,
            ..playing(10)
        };
        guard.tick(paused, at(101));
        assert_eq!(state(&guard, &file), None);
        // Playing, but only one new frame since the list: not yet.
        guard.tick(playing(1), at(102));
        assert_eq!(state(&guard, &file), None);
        guard.tick(playing(FIRST_FRAMES), at(103));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        assert_eq!(file.read().shaders.unwrap().plugins, ["acme.a"]);
        // Paused before 10 s: settled at once; resumed: armed again.
        guard.tick(
            Observed {
                playing: false,
                ..playing(5)
            },
            at(105),
        );
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        guard.tick(playing(6), at(106));
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok), "one frame");
        guard.tick(playing(7), at(107));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        // 10 s of playback: ok, and it stays ok while the list stays.
        guard.tick(playing(8), at(112));
        guard.tick(playing(9), at(117));
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        guard.tick(Observed::default(), at(118));
        guard.tick(playing(30), at(119));
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        // Quitting while armed settles on the calling thread.
        guard.set(1, "shaders:x", "acme.a", 1, 30);
        guard.tick(playing(32), at(120));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        guard.quit();
        assert_eq!(file.read().shaders.unwrap().state, ShaderState::Ok);
    }

    /// Review item 5: audio only never arms, and a stop of the video track
    /// settles; a new source starts the 10 s again.
    #[test]
    fn audio_only_never_arms_and_a_new_source_restarts_the_timer() {
        let (_dir, file, mut guard) = guard();
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let audio = |presented| Observed {
            video: false,
            ..playing(presented)
        };
        guard.set(1, "render", "acme.a", 1, 0);
        guard.tick(audio(0), at(0));
        guard.tick(audio(5), at(30));
        assert_eq!(state(&guard, &file), None, "audio only");
        guard.tick(playing(5), at(31));
        guard.tick(playing(7), at(32));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        // 8 s on the first source, then another source: 8 more are not
        // enough.
        guard.tick(playing(8), at(40));
        let next = Observed {
            source: 2,
            ..playing(9)
        };
        guard.tick(next, at(41));
        guard.tick(next, at(48));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        guard.tick(next, at(52));
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        // The video track goes while armed: settled.
        guard.set(1, "render", "acme.a", 2, 9);
        guard.tick(playing(11), at(53));
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        guard.tick(audio(11), at(54));
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
    }

    /// Review item 3: lists set as a slider moves write nothing while the
    /// same plugins' guard is armed; writes happen off the UI thread.
    #[test]
    fn new_lists_from_the_same_plugins_while_armed_write_nothing() {
        let (_dir, file, mut guard) = guard();
        let start = Instant::now();
        guard.set(1, "shaders:x", "acme.a", 1, 0);
        guard.tick(playing(FIRST_FRAMES), start);
        let written = guard.writer.flush();
        assert_eq!(written, 1);
        for step in 0..50 {
            guard.set(1, "shaders:x", "acme.a", 1 + step % 3, 10);
            guard.tick(
                playing(20 + step as u64),
                start + Duration::from_millis(step as u64 * 16),
            );
        }
        assert_eq!(guard.writer.flush(), written, "no write per step");
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        // Another plugin joins: written again once its list is drawn.
        guard.set(2, "render", "acme.b", 1, 100);
        guard.tick(playing(101), start + Duration::from_secs(1));
        assert_eq!(guard.writer.flush(), written);
        guard.tick(playing(102), start + Duration::from_secs(1));
        assert_eq!(guard.writer.flush(), written + 1);
        assert_eq!(file.read().shaders.unwrap().plugins, ["acme.a", "acme.b"]);
        // A list changes while armed, then playback pauses before it is
        // drawn: settled all the same.
        guard.set(1, "shaders:x", "acme.a", 0, 103);
        guard.tick(
            Observed {
                playing: false,
                ..playing(103)
            },
            start + Duration::from_secs(2),
        );
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        // Every list emptied: settled.
        guard.owner_gone(2, 103);
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
    }

    /// The session ends while armed: settled at once; a cancelled end arms
    /// it again at the next tick.
    #[test]
    fn the_end_of_the_session_settles_and_a_cancel_arms_again() {
        let (_dir, file, mut guard) = guard();
        guard.set(1, "render", "acme.a", 1, 0);
        guard.tick(playing(FIRST_FRAMES), Instant::now());
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        session_end(&guard.writer(), SessionEvent::Query);
        assert_eq!(file.read().shaders.unwrap().state, ShaderState::Ok);
        let shared = &guard.writer.shared;
        assert!(!shared.exit_logged.load(Ordering::Acquire), "no exit yet");
        session_end(&guard.writer(), SessionEvent::Cancelled);
        guard.tick(playing(3), Instant::now());
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        // Ending: one exit line, however often it is told.
        session_end(&guard.writer(), SessionEvent::Ending);
        session_end(&guard.writer(), SessionEvent::Ending);
        assert!(guard.writer.shared.exit_logged.load(Ordering::Acquire));
        assert_eq!(file.read().shaders.unwrap().state, ShaderState::Ok);
    }

    /// Re-review item 2: once the session ends, no tick writes `applying`
    /// again (a list drawn later, a pending one), until a cancel.
    #[test]
    fn nothing_is_armed_after_the_session_ended() {
        let (_dir, file, mut guard) = guard();
        guard.set(1, "render", "acme.a", 1, 0);
        guard.tick(playing(FIRST_FRAMES), Instant::now());
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        session_end(&guard.writer(), SessionEvent::Ending);
        // Paused and resumed, a new plugin's list drawn: nothing written.
        guard.tick(Observed::default(), Instant::now());
        guard.set(2, "render", "acme.b", 1, 10);
        guard.tick(playing(20), Instant::now());
        guard.tick(playing(21), Instant::now());
        assert_eq!(state(&guard, &file), Some(ShaderState::Ok));
        assert_eq!(file.read().shaders.unwrap().plugins, ["acme.a"]);
        // Cancelled: armed again.
        session_end(&guard.writer(), SessionEvent::Cancelled);
        guard.tick(playing(22), Instant::now());
        assert_eq!(state(&guard, &file), Some(ShaderState::Applying));
        assert_eq!(file.read().shaders.unwrap().plugins, ["acme.a", "acme.b"]);
    }

    /// Re-review item 1: before a device-loss restart the pending
    /// `applying` reaches the disk (the writer is waited for, not skipped).
    #[test]
    fn a_pending_applying_is_on_disk_once_persisted() {
        let (_dir, file, mut guard) = guard();
        guard.set(1, "render", "acme.a", 1, 0);
        guard.tick(playing(FIRST_FRAMES), Instant::now());
        // No `flush`: what the restart path does.
        assert!(guard.writer().persist(Duration::from_secs(5)));
        assert_eq!(file.read().shaders.unwrap().state, ShaderState::Applying);
        // A writer that is busy too long: `persist` gives up and says so.
        let stuck = guard.writer();
        stuck.lock().busy = true;
        assert!(!stuck.persist(Duration::from_millis(20)));
        stuck.lock().busy = false;
        // Without a registered writer the restart path does nothing.
        persist_shader_guard();
    }

    /// The run after a crash with shaders on: non-empty lists are refused
    /// with `previous-crash`, clearing still works, and the OSD line comes
    /// once, when a window can show it.
    #[test]
    fn a_blocked_run_refuses_lists_and_tells_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let block = ShaderBlock {
            plugins: vec!["acme.sharpen".into()],
            until: None,
        };
        let mut guard = ShaderGuard::new(file.clone(), key(), Some(block));
        assert!(!guard.take_osd(true), "nothing refused yet");
        assert!(guard.refuse(0, 0).is_none(), "clearing works");
        let error = guard.refuse(1, 0).unwrap();
        assert_eq!(error.code, PREVIOUS_CRASH);
        assert!(
            error.message.contains("for this start"),
            "{}",
            error.message
        );
        assert!(error.message.contains("acme.sharpen"), "{}", error.message);
        assert!(!guard.take_osd(false), "no window yet");
        assert!(guard.take_osd(true));
        assert!(guard.refuse(2, 0).is_some());
        assert!(!guard.take_osd(true), "once per run");
        // Off for days: the message says until when.
        let now = unix_now();
        let mut guard = ShaderGuard::new(
            file,
            key(),
            Some(ShaderBlock {
                plugins: vec![],
                until: Some(now + 3 * 24 * 60 * 60),
            }),
        );
        let error = guard.refuse(1, now).unwrap();
        assert!(error.message.contains("in 3 days"), "{}", error.message);
    }

    /// Review item 4: only the code, plus the message for codes known to
    /// carry no user data; plugin ids reduced like codes.
    #[test]
    fn host_errors_reach_the_log_without_user_data() {
        assert_eq!(
            host_error_line(
                &json!({"code":"source-load","message":"cannot open holiday.mkv","generation":3})
            ),
            "host.error source-load"
        );
        assert_eq!(
            host_error_line(
                &json!({"code":"source-geometry","message":"orientation filter failed: x.mkv"})
            ),
            "host.error source-geometry: orientation filter failed: …"
        );
        assert_eq!(
            host_error_line(&json!({"code":"x y\nz/holiday.mkv","message":"a"})),
            "host.error xyzholidaymkv"
        );
        assert_eq!(host_error_line(&Value::Null), "host.error unknown");
        assert_eq!(
            handle_for_log("shaders:my holiday.mkv"),
            "shaders:myholidaymkv"
        );
        assert_eq!(handle_for_log("render"), "render");
    }

    #[test]
    fn without_a_file_nothing_is_written() {
        let mut guard = ShaderGuard::default();
        guard.set(1, "render", "a", 2, 0);
        guard.tick(playing(5), Instant::now());
        guard.quit();
        assert_eq!(guard.writer.flush(), 0);
        assert_eq!(GuardFile::default().read(), Default::default());
    }
}
