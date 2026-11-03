//! The shared runner: the trusted (built-in) plugins in one Node process
//! (`src/runtime/shared-runner.ts`, design and protocol in
//! doc/process/rounds/2026-10-shared-runner.md §5, §7, §8).
//!
//! One stdin/stdout pair carries every member's messages; each line is the
//! member's own message with the owner generation `g` spliced in as the first
//! key (`{"g":12,"id":7,…}`), `g` 0 being the runner itself (`runner.hello`,
//! `plugin.start`, `runner.ping`/`pong`, `runner.stopped`, `runner.fault`,
//! `runner.stop`). The host keeps one generation per plugin, as for an own
//! process; the runner has a [`RunnerId`] that no registry sees.
//!
//! This module is the process layer: spawning, the reader and writer
//! threads, a member's send and stop, the heartbeat and the restart policy
//! ([`Policy`]). Who runs here (`Host::trust`) and what a restart does
//! (`Host::restart_shared`) live in `package_manager.rs`.
#![forbid(unsafe_code)]

use crate::host::Event;
use crate::process::{MAX_MESSAGE_BYTES, StopReason, stop_message};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// One incarnation of the shared runner (a new one after each restart).
pub type RunnerId = u32;

/// The longest line the runner may write: a member's message (at most
/// [`MAX_MESSAGE_BYTES`], which the runner enforces before writing) plus the
/// envelope's `"g":<digits>,`.
pub const LINE_LIMIT: usize = MAX_MESSAGE_BYTES + 64;
/// Lines one member may have queued for the runner: an own process's queue
/// (`process/mod.rs` `OUTBOUND_CAPACITY`). It fills only while the runner reads
/// no input, so past it the host restarts the runner with every member
/// (`Host::transport_failed`), this one included.
const MEMBER_QUEUE: usize = 1024;
/// The runner's one queue; a member reaching [`MEMBER_QUEUE`] restarts the
/// runner long before 16 of them fill it.
const QUEUE: usize = 16 * MEMBER_QUEUE;
/// V8's old-space cap for the runner: a leaking member ends the runner (and
/// is restarted with the others) at 1 GiB instead of exhausting the machine.
pub const MAX_OLD_SPACE: &str = "--max-old-space-size=1024";
/// How long a stopped member's cleanup may take before it counts as leaked:
/// an own process's stop window (`process/mod.rs` `Drop for Plugin`).
const STOP_WAIT: Duration = Duration::from_millis(300);
/// How long a stopped member's cleanup may run in the runner before it
/// abandons it and reports the member gone (`STOP_TIMEOUT_MS` in
/// `src/runtime/boot.ts`, 5 s), plus a margin.
const MEMBER_GONE: Duration = Duration::from_secs(6);
/// Heartbeat (user ruling 2026-10-27): a ping every 2 s
/// (`VESTRA_SHARED_PING_MS` for tests); five unanswered = hung.
pub const PING_MS: u64 = 2_000;
pub const MISSED_LIMIT: u32 = 5;
/// Restart loop guard (user ruling 2026-10-27): the third restart within a
/// minute switches the trusted plugins to own processes until the player
/// quits.
pub const RESTART_LIMIT: usize = 3;
pub const RESTART_WINDOW_MS: u64 = 60_000;
/// The runner's heap past which the host logs a warning (once per runner).
const HEAP_WARNING: u64 = 512 * 1024 * 1024;

/// Whether the trusted plugins may share a runner at all: not with
/// `VESTRA_SHARED=0` (every plugin in its own process, as before 0.5.6).
pub fn enabled() -> bool {
    std::env::var_os("VESTRA_SHARED").is_none_or(|value| value != "0")
}

/// The heartbeat interval (`VESTRA_SHARED_PING_MS`, else [`PING_MS`]).
pub fn ping_ms() -> u64 {
    std::env::var("VESTRA_SHARED_PING_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| (50..=60_000).contains(ms))
        .unwrap_or(PING_MS)
}

/// A line for the runner: `{"g":<g>,` and the message's own keys.
pub fn envelope(g: u64, message: &Value) -> Option<String> {
    if !message.is_object() {
        return None;
    }
    let inner = message.to_string();
    Some(if inner == "{}" {
        format!("{{\"g\":{g}}}")
    } else {
        format!("{{\"g\":{g},{}", &inner[1..])
    })
}

/// `g` from a line's first bytes (`{"g":<digits>`), which the runner always
/// writes first: enough to blame an oversize line on its plugin.
pub fn prefix_generation(bytes: &[u8]) -> Option<u64> {
    let rest = bytes.strip_prefix(b"{\"g\":")?;
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 16 || !matches!(rest.get(digits), Some(b',' | b'}')) {
        return None;
    }
    std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()
}

/// A line from the runner: its generation and the message without `g`.
pub fn parse_line(bytes: &[u8]) -> std::result::Result<(u64, Value), String> {
    let g = prefix_generation(bytes).ok_or("a line without a leading generation")?;
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid protocol: {e}"))?;
    let object = value
        .as_object_mut()
        .ok_or("a line that is not an object")?;
    object.remove("g");
    Ok((g, value))
}

struct Line {
    g: u64,
    message: Value,
    /// The member's count of queued lines, decremented once written.
    queued: Option<Arc<AtomicUsize>>,
}

/// What the host, the members and the reader thread share.
pub struct Link {
    pub id: RunnerId,
    outbound: mpsc::SyncSender<Line>,
    state: Mutex<LinkState>,
}
#[derive(Default)]
struct LinkState {
    /// False once the runner's stdout closed or the host let it go: a
    /// member's stop then returns at once.
    alive: bool,
    /// Generations whose lines are accepted (started, not yet stopped).
    members: BTreeSet<u64>,
    /// Members waiting for their `runner.stopped`: `timedOut`.
    waiters: BTreeMap<u64, mpsc::Sender<bool>>,
    /// Members whose cleanup outlived the stop window.
    leaked: usize,
}
impl Link {
    fn state(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn is_member(&self, g: u64) -> bool {
        self.state().members.contains(&g)
    }
    fn stopped(&self, g: u64, timed_out: bool) {
        let mut state = self.state();
        if let Some(waiter) = state.waiters.remove(&g) {
            let _ = waiter.send(timed_out);
        }
    }
    /// The runner is gone or let go: nobody waits for it any more.
    fn close(&self) {
        let mut state = self.state();
        state.alive = false;
        state.waiters.clear();
    }
    /// Members whose cleanup did not finish in time, this incarnation.
    pub fn leaked(&self) -> usize {
        self.state().leaked
    }
    /// Live members.
    pub fn members(&self) -> usize {
        self.state().members.len()
    }
}

/// A plugin's place in the shared runner (`process::Plugin`'s runner).
pub struct Member {
    link: Arc<Link>,
    g: u64,
    queued: Arc<AtomicUsize>,
}
impl Member {
    pub fn runner(&self) -> RunnerId {
        self.link.id
    }
    pub fn send(&self, message: Value) -> Result<()> {
        if self.queued.load(Ordering::Acquire) >= MEMBER_QUEUE {
            anyhow::bail!("plugin output backpressure: {MEMBER_QUEUE} messages queued");
        }
        self.queued.fetch_add(1, Ordering::AcqRel);
        let sent = self.link.outbound.try_send(Line {
            g: self.g,
            message,
            queued: Some(self.queued.clone()),
        });
        match sent {
            Ok(()) => Ok(()),
            // The runner is gone (its writer stopped): the message is lost
            // with it, and the runner's exit restarts this member. An error
            // here would stop the member before the restart could see it.
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(1, Ordering::AcqRel);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::AcqRel);
                anyhow::bail!("shared runner output backpressure: its queue is full")
            }
        }
    }
    /// The whole stop at once (`begin_stop`, then `MemberStop::wait`).
    #[cfg(test)]
    pub fn stop(&self, reason: StopReason) {
        self.begin_stop(reason).wait();
    }
    /// `runtime.stop` for this member; the wait for its `runner.stopped` is
    /// the returned [`MemberStop`]'s, where an own process would be waited
    /// for and killed: polled by the plugin reaper beside every other
    /// stopping plugin (`process/reaper.rs`, supervisor.md §7.1), or waited
    /// for at once (`Drop for Plugin`, at quit). The runner lives on. A
    /// cleanup longer than an own process's window ([`STOP_WAIT`]) counts as
    /// a leak, and the wait goes on until the runner reports the member gone
    /// (its own deadline, [`MEMBER_GONE`]): the host reports the plugin
    /// reaped only then, so a reload or an install does not start the new
    /// instance while a cleanup that finishes within the runner's 5 s still
    /// writes the same data directory. A cleanup the runner abandons at 5 s,
    /// or a stop not reported within [`MEMBER_GONE`] (the runner's input
    /// backed up, for one), can still overlap the new instance; both are
    /// logged. An own process is killed instead; a member cannot be.
    /// `reason` goes with the `runtime.stop` (`process::StopReason`).
    pub fn begin_stop(&self, reason: StopReason) -> MemberStop {
        let (done, wait) = mpsc::channel();
        let mut stop = MemberStop {
            link: self.link.clone(),
            g: self.g,
            wait: None,
            began: Instant::now(),
            leaked: false,
        };
        {
            let mut state = self.link.state();
            if !state.alive {
                return stop;
            }
            state.waiters.insert(self.g, done);
        }
        stop.began = Instant::now();
        let sent = self.link.outbound.try_send(Line {
            g: self.g,
            message: stop_message(reason),
            queued: None,
        });
        // Not sent: the runner is gone, nothing waits.
        stop.wait = sent.is_ok().then_some(wait);
        stop
    }
}

/// A member's stop under way (`Member::begin_stop`): done when the runner
/// reports it gone, its link closes, or [`MEMBER_GONE`] passes; a cleanup
/// still running after [`STOP_WAIT`] counts as leaked. Either way the
/// member leaves the link (its lines are no longer accepted).
pub struct MemberStop {
    link: Arc<Link>,
    g: u64,
    /// The runner's `runner.stopped` (`timedOut`); `None`: nothing to wait
    /// for (the runner is gone, or the stop could not be queued).
    wait: Option<mpsc::Receiver<bool>>,
    began: Instant,
    /// Counted and logged as leaked already.
    leaked: bool,
}
impl MemberStop {
    /// Without blocking: whether the stop is over at `now` (the plugin
    /// reaper's poll).
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(wait) = &self.wait else {
            self.finish();
            return true;
        };
        match wait.try_recv() {
            Ok(timed_out) => {
                if timed_out {
                    self.abandoned();
                }
                self.finish();
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finish();
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                let elapsed = now.saturating_duration_since(self.began);
                if !self.leaked && elapsed >= STOP_WAIT {
                    self.leak();
                }
                if elapsed < MEMBER_GONE {
                    return false;
                }
                self.not_reported();
                self.finish();
                true
            }
        }
    }
    /// Blocking until the stop is over (a plugin dropped where it is, at
    /// quit): what `Member::stop` always did.
    pub fn wait(mut self) {
        let Some(wait) = self.wait.take() else {
            self.finish();
            return;
        };
        let mut outcome = wait.recv_timeout(STOP_WAIT);
        if matches!(outcome, Err(mpsc::RecvTimeoutError::Timeout)) {
            self.leak();
            outcome = wait.recv_timeout(MEMBER_GONE.saturating_sub(self.began.elapsed()));
        }
        match outcome {
            Ok(true) => self.abandoned(),
            Err(mpsc::RecvTimeoutError::Timeout) => self.not_reported(),
            _ => {}
        }
        self.finish();
    }
    fn leak(&mut self) {
        self.leaked = true;
        self.link.state().leaked += 1;
        diag!(
            "shared runner {}: member {} still cleaning up after {} ms; waiting for the runner",
            self.link.id,
            self.g,
            STOP_WAIT.as_millis()
        );
    }
    fn abandoned(&self) {
        diag!(
            "shared runner {}: member {}'s cleanup was abandoned after the runner's deadline",
            self.link.id,
            self.g
        );
    }
    fn not_reported(&self) {
        diag!(
            "shared runner {}: member {} not reported gone within {} s",
            self.link.id,
            self.g,
            MEMBER_GONE.as_secs()
        );
    }
    fn finish(&self) {
        let mut state = self.link.state();
        state.waiters.remove(&self.g);
        state.members.remove(&self.g);
    }
}

/// The running shared runner, owned by the host (`Host.shared`).
pub struct SharedRunner {
    pub link: Arc<Link>,
    child: Option<Child>,
    /// `runner.hello`'s data, once it arrived.
    pub hello: Option<Value>,
    pub heartbeat: Heartbeat,
    /// The last `runner.pong`'s `stats` (for the diagnostics of a restart).
    pub stats: Value,
    heap_warned: bool,
    /// Since when it has had no member (`SharedRunner::idle`).
    empty_since: Option<u64>,
}
impl SharedRunner {
    /// Start the runner: `node --max-old-space-size=1024 shared-runner.js`.
    /// Its lines arrive as `Event::Request(g, …)` for members and
    /// `Event::Runner(id, …)` for the runner; its end as
    /// `Event::RunnerExited(id)`.
    pub fn spawn(id: RunnerId, events: mpsc::Sender<Event>, now: u64) -> Result<Self> {
        let paths = crate::process::runtime_paths::get();
        let mut child = paths
            .node_command()
            .arg(MAX_OLD_SPACE)
            .arg(&paths.shared_runner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| paths.node_failure())?;
        let stdin = child.stdin.take().context("shared runner stdin")?;
        let stdout = child.stdout.take().context("shared runner stdout")?;
        let (outbound, output) = mpsc::sync_channel::<Line>(QUEUE);
        let link = Arc::new(Link {
            id,
            outbound,
            state: Mutex::new(LinkState {
                alive: true,
                ..Default::default()
            }),
        });
        thread::Builder::new()
            .name("shared-runner-write".into())
            .spawn(move || write_lines(stdin, output))?;
        let reader = link.clone();
        thread::Builder::new()
            .name("shared-runner-read".into())
            .spawn(move || {
                read_lines(BufReader::new(stdout), &reader, &events);
                reader.close();
                let _ = events.send(Event::RunnerExited(reader.id));
            })?;
        diag!("shared runner {id}: started (pid {})", child.id());
        Ok(Self {
            link,
            child: Some(child),
            hello: None,
            heartbeat: Heartbeat::new(now),
            stats: Value::Null,
            heap_warned: false,
            empty_since: None,
        })
    }

    /// Admit plugin `g`: `plugin.start` with what an own process gets from
    /// its arguments and environment (`process/mod.rs` `load_manifest`).
    pub fn start(
        &mut self,
        g: u64,
        id: &str,
        directory: &Path,
        data_directory: Option<&Path>,
        app: &Value,
    ) -> Result<Member> {
        self.link.state().members.insert(g);
        let start = json!({"event":"plugin.start","data":{
            "g": g,
            "id": id,
            "directory": directory,
            "dataDirectory": data_directory,
            "locale": crate::i18n::locale(),
            "app": app,
        }});
        let sent = self.link.outbound.try_send(Line {
            g: 0,
            message: start,
            queued: None,
        });
        if let Err(error) = sent {
            self.link.state().members.remove(&g);
            anyhow::bail!("shared runner: {error}");
        }
        self.empty_since = None;
        Ok(Member {
            link: self.link.clone(),
            g,
            queued: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// A control line from the runner (`g` 0) other than `runner.stopped`
    /// (which the reader thread hands to the waiting member itself).
    /// Returns a `runner.fault`'s `(g, message)`.
    pub fn control(&mut self, message: &Value) -> Option<(u64, String)> {
        match message["event"].as_str() {
            Some("runner.hello") => {
                diag!("shared runner {}: hello {}", self.link.id, message["data"]);
                self.hello = Some(message["data"].clone());
                None
            }
            Some("runner.pong") => {
                self.heartbeat.pong(message["data"]["seq"].as_u64());
                self.stats = message["data"]["stats"].clone();
                if std::env::var_os("VESTRA_TRACE").is_some() {
                    diag!("shared runner {}: {}", self.link.id, self.stats);
                }
                let heap = self.stats["heapUsed"].as_u64().unwrap_or(0);
                if heap >= HEAP_WARNING && !self.heap_warned {
                    self.heap_warned = true;
                    diag!(
                        "host.warning shared runner {}: heap {} MB",
                        self.link.id,
                        heap / (1024 * 1024)
                    );
                }
                None
            }
            Some("runner.fault") => Some((
                message["data"]["g"].as_u64().unwrap_or(0),
                message["data"]["message"]
                    .as_str()
                    .unwrap_or("uncaught exception")
                    .chars()
                    .take(512)
                    .collect(),
            )),
            _ => {
                diag!("shared runner {}: unknown line {message}", self.link.id);
                None
            }
        }
    }

    /// The heartbeat at a host tick: a ping when due; `Err` with the reason
    /// once the runner counts as hung (no hello within the hello window,
    /// or [`MISSED_LIMIT`] pings unanswered).
    pub fn tick(&mut self, now: u64, interval: u64) -> std::result::Result<(), String> {
        if self.hello.is_none() {
            return if now.saturating_sub(self.heartbeat.since) >= interval * MISSED_LIMIT as u64 {
                Err(format!(
                    "no hello within {} s",
                    interval * MISSED_LIMIT as u64 / 1000
                ))
            } else {
                Ok(())
            };
        }
        match self.heartbeat.tick(now, interval) {
            Beat::Wait => Ok(()),
            Beat::Ping(seq) => {
                let _ = self.link.outbound.try_send(Line {
                    g: 0,
                    message: json!({"event":"runner.ping","data":{"seq":seq}}),
                    queued: None,
                });
                Ok(())
            }
            Beat::Hung => Err(format!(
                "no heartbeat for {} s",
                interval * MISSED_LIMIT as u64 / 1000
            )),
        }
    }

    /// Without a member for a second: it is let go (`retire`) until the next
    /// trusted plugin starts.
    pub fn idle(&mut self, now: u64) -> bool {
        if self.link.members() > 0 {
            self.empty_since = None;
            return false;
        }
        let since = *self.empty_since.get_or_insert(now);
        now.saturating_sub(since) >= 1_000
    }

    /// Let it go without waiting on the caller's thread: `runner.stop`
    /// (every member's cleanup, then exit) when `graceful`, else killed at
    /// once; killed after [`STOP_WAIT`] either way, by the plugin reaper
    /// (once a `shared-runner-reap` thread of its own, supervisor.md §7.1).
    /// Graceful only when it has no member left (`idle`), so its
    /// `runner.stop` carries no reason.
    pub fn retire(mut self, graceful: bool, reaper: &crate::process::reaper::Reaper) {
        self.link.close();
        let Some(mut child) = self.child.take() else {
            return;
        };
        if graceful {
            let _ = self.link.outbound.try_send(Line {
                g: 0,
                message: json!({"event":"runner.stop"}),
                queued: None,
            });
        } else {
            let _ = child.kill();
        }
        reaper.child(child, STOP_WAIT);
    }
}
impl SharedRunner {
    /// The player quits within one deadline (supervisor.md §7.2, T31):
    /// `runner.stop` with reason `quit` (every member's cleanup runs in
    /// the runner at once), killed after `wait` by the plugin reaper rather
    /// than waited for on the caller's thread (its `Drop`).
    pub fn quit(mut self, wait: Duration, reaper: &crate::process::reaper::Reaper) {
        let Some(child) = self.child.take() else {
            return;
        };
        let _ = self.link.outbound.try_send(Line {
            g: 0,
            message: runner_stop(StopReason::Quit),
            queued: None,
        });
        self.link.close();
        reaper.child(child, wait);
    }
}
impl Drop for SharedRunner {
    /// The player quits: `runner.stop` with reason `quit` (the members'
    /// cleanups run concurrently in the runner), [`STOP_WAIT`], then killed —
    /// the same window an own process gets.
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = self.link.outbound.try_send(Line {
                g: 0,
                message: runner_stop(StopReason::Quit),
                queued: None,
            });
            wait_or_kill(&mut child);
        }
        self.link.close();
    }
}

/// `runner.stop` for every member: each one's cleanup gets `reason`.
fn runner_stop(reason: StopReason) -> Value {
    json!({"event":"runner.stop","data":reason.data()})
}

fn wait_or_kill(child: &mut Child) {
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn write_lines(mut stdin: impl Write, output: mpsc::Receiver<Line>) {
    for line in output {
        let Some(text) = envelope(line.g, &line.message) else {
            diag!("shared runner: a message that is not an object was not sent");
            continue;
        };
        let written = stdin
            .write_all(text.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        if let Some(queued) = &line.queued {
            queued.fetch_sub(1, Ordering::AcqRel);
        }
        // The runner is gone; its reader reports the exit.
        if written.is_err() {
            break;
        }
    }
}

/// The runner's stdout, line by line, until it closes or writes a line no
/// member can own (then the runner is restarted: nothing else writes
/// there, `runner.fault` with `g` 0).
fn read_lines(mut reader: impl BufRead, link: &Link, events: &mpsc::Sender<Event>) {
    let fault = |message: String| {
        let _ = events.send(Event::Runner(
            link.id,
            json!({"event":"runner.fault","data":{"g":0,"message":message}}),
        ));
    };
    // Read to the end in every case, never closing the pipe early: a closed
    // pipe fails the runner's next write, and it would stop every member as
    // `host-lost` before the host's `runner.stop` (`quit`) or the restart.
    // Once the host no longer listens (quitting: its receiver goes first)
    // only `runner.stopped` is still handled; after a fault, nothing.
    let mut listening = true;
    let mut faulted = false;
    loop {
        let mut bytes = Vec::new();
        let read = Read::by_ref(&mut reader)
            .take(LINE_LIMIT as u64 + 1)
            .read_until(b'\n', &mut bytes);
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) if faulted => {}
            Ok(_) if bytes.len() > LINE_LIMIT => match prefix_generation(&bytes) {
                // That plugin is stopped; the runner and the others go on.
                Some(g) if g > 0 && link.is_member(g) => {
                    if listening {
                        let _ = events.send(Event::RuntimeFault(
                            g,
                            format!("protocol message exceeds {} MiB", MAX_MESSAGE_BYTES >> 20),
                        ));
                    }
                    if !bytes.ends_with(b"\n") && reader.skip_until(b'\n').unwrap_or(0) == 0 {
                        return;
                    }
                }
                _ => {
                    fault("an oversize line without a plugin generation".into());
                    faulted = true;
                }
            },
            Ok(_) => match parse_line(&bytes) {
                Ok((0, message)) => {
                    if message["event"] == "runner.stopped" {
                        if let Some(g) = message["data"]["g"].as_u64() {
                            link.stopped(g, message["data"]["timedOut"] == true);
                        }
                    } else if listening {
                        listening = events.send(Event::Runner(link.id, message)).is_ok();
                    }
                }
                Ok((g, message)) => {
                    // A plugin already stopped (its last lines), or none of
                    // this runner's: never another owner's.
                    if listening && link.is_member(g) {
                        listening = events.send(crate::process::request(g, message)).is_ok();
                    }
                }
                Err(error) => {
                    fault(error);
                    faulted = true;
                }
            },
        }
    }
}

/// The heartbeat's state (`SharedRunner::tick`).
#[derive(Debug)]
pub struct Heartbeat {
    /// When the runner started, then when the last ping went out.
    since: u64,
    seq: u64,
    /// The last ping not answered yet.
    awaiting: Option<u64>,
    missed: u32,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Beat {
    Wait,
    Ping(u64),
    Hung,
}
impl Heartbeat {
    pub fn new(now: u64) -> Self {
        Self {
            since: now,
            seq: 0,
            awaiting: None,
            missed: 0,
        }
    }
    pub fn tick(&mut self, now: u64, interval: u64) -> Beat {
        if self.seq > 0 && now.saturating_sub(self.since) < interval {
            return Beat::Wait;
        }
        if self.awaiting.is_some() {
            self.missed += 1;
            if self.missed >= MISSED_LIMIT {
                return Beat::Hung;
            }
        }
        self.seq += 1;
        self.since = now;
        self.awaiting = Some(self.seq);
        Beat::Ping(self.seq)
    }
    pub fn pong(&mut self, seq: Option<u64>) {
        if seq.is_some() && seq == self.awaiting {
            self.awaiting = None;
            self.missed = 0;
        }
    }
}

/// The restart policy of one player run (user rulings 2026-10-27).
#[derive(Debug, Default)]
pub struct Policy {
    /// Restarts within the last [`RESTART_WINDOW_MS`].
    restarts: VecDeque<u64>,
    /// After [`RESTART_LIMIT`] restarts within the window: own processes for
    /// every plugin until the player quits.
    pub failed_open: bool,
    /// Plugins a fault was attributed to: their own process for this run.
    pub quarantined: BTreeSet<String>,
    /// The next runner's id.
    next: RunnerId,
}
impl Policy {
    pub fn next_id(&mut self) -> RunnerId {
        self.next += 1;
        self.next
    }
    /// A restart at `now`: `true` when it is the one too many (fail open).
    pub fn restart(&mut self, now: u64) -> bool {
        while self
            .restarts
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= RESTART_WINDOW_MS)
        {
            self.restarts.pop_front();
        }
        self.restarts.push_back(now);
        if self.restarts.len() >= RESTART_LIMIT {
            self.failed_open = true;
        }
        self.failed_open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn the_envelope_puts_g_first_without_reserializing_the_message() {
        let message = json!({"id":7,"result":{"a":1}});
        let line = envelope(12, &message).unwrap();
        assert!(line.starts_with("{\"g\":12,"), "{line}");
        let (g, back) = parse_line(line.as_bytes()).unwrap();
        assert_eq!((g, back), (12, message));
        assert_eq!(envelope(3, &json!({})).unwrap(), "{\"g\":3}");
        assert_eq!(envelope(3, &json!([1])), None);
    }

    #[test]
    fn a_generation_is_read_only_from_the_first_key() {
        assert_eq!(prefix_generation(b"{\"g\":42,\"id\":1"), Some(42));
        assert_eq!(prefix_generation(b"{\"g\":0}"), Some(0));
        assert_eq!(prefix_generation(b"{\"id\":1,\"g\":4}"), None);
        assert_eq!(prefix_generation(b"{\"g\":4x"), None);
        assert_eq!(prefix_generation(b"{\"g\":,"), None);
        assert_eq!(prefix_generation(b"{\"g\":12345678901234567,"), None);
        assert!(parse_line(b"{\"id\":1,\"g\":4}").is_err());
        assert!(parse_line(b"{\"g\":4,").is_err());
    }

    fn link(members: &[u64]) -> (Link, mpsc::Receiver<Line>) {
        let (outbound, output) = mpsc::sync_channel(QUEUE);
        let link = Link {
            id: 1,
            outbound,
            state: Mutex::new(LinkState {
                alive: true,
                members: members.iter().copied().collect(),
                ..Default::default()
            }),
        };
        (link, output)
    }

    fn read(link: &Link, input: Vec<u8>) -> Vec<Event> {
        let (events, received) = mpsc::channel();
        read_lines(Cursor::new(input), link, &events);
        drop(events);
        received.into_iter().collect()
    }

    #[test]
    fn lines_go_to_their_member_and_control_lines_to_the_host() {
        let (link, _output) = link(&[5]);
        let (done, stopped) = mpsc::channel();
        link.state().waiters.insert(5, done);
        let input = b"{\"g\":5,\"id\":1,\"method\":\"commands.register\",\"params\":{}}\n\
            {\"g\":9,\"event\":\"runtime.ready\"}\n\
            {\"g\":0,\"event\":\"runner.hello\",\"data\":{\"pid\":1}}\n\
            {\"g\":0,\"event\":\"runner.stopped\",\"data\":{\"g\":5,\"timedOut\":true}}\n"
            .to_vec();
        let events = read(&link, input);
        assert_eq!(events.len(), 2, "g 9 is no member; stopped is consumed");
        assert!(
            matches!(&events[0], Event::Request(5, m) if m["method"] == "commands.register" && m.get("g").is_none())
        );
        assert!(matches!(&events[1], Event::Runner(1, m) if m["event"] == "runner.hello"));
        // The waiting member learns its cleanup was abandoned; it counts the leak.
        assert_eq!(stopped.try_recv(), Ok(true));
    }

    #[test]
    fn an_oversize_line_stops_its_plugin_and_reading_goes_on() {
        let (link, _output) = link(&[7, 8]);
        let mut input = format!(
            "{{\"g\":7,\"id\":1,\"params\":\"{}\"}}\n",
            "x".repeat(LINE_LIMIT)
        )
        .into_bytes();
        input.extend_from_slice(b"{\"g\":8,\"event\":\"runtime.ready\"}\n");
        let events = read(&link, input);
        assert!(matches!(&events[0], Event::RuntimeFault(7, m) if m.contains("1 MiB")));
        assert!(matches!(&events[1], Event::Request(8, _)));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn a_line_no_plugin_owns_is_a_runner_fault() {
        for input in [
            format!("{}\n", "x".repeat(LINE_LIMIT + 10)).into_bytes(),
            b"junk from a raw stdout write\n{\"g\":8,\"event\":\"runtime.ready\"}\n".to_vec(),
            b"{\"g\":8,\"event\":\n".to_vec(),
        ] {
            let (link, _output) = link(&[8]);
            let events = read(&link, input);
            assert_eq!(events.len(), 1);
            assert!(
                matches!(&events[0], Event::Runner(1, m) if m["event"] == "runner.fault" && m["data"]["g"] == 0)
            );
        }
    }

    #[test]
    fn a_member_queue_is_bounded_and_the_writer_frees_it() {
        let (link, output) = link(&[3]);
        let member = Member {
            link: Arc::new(link),
            g: 3,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        for _ in 0..MEMBER_QUEUE {
            member.send(json!({"event":"x"})).unwrap();
        }
        assert!(member.send(json!({"event":"x"})).is_err());
        let mut written = Vec::new();
        let lines: Vec<_> = output.try_iter().collect();
        let (sink, input) = mpsc::sync_channel(QUEUE);
        for line in lines {
            sink.send(line).unwrap();
        }
        drop(sink);
        write_lines(&mut written, input);
        assert_eq!(member.queued.load(Ordering::Acquire), 0);
        let text = String::from_utf8(written).unwrap();
        assert_eq!(text.lines().count(), MEMBER_QUEUE);
        assert!(text.starts_with("{\"g\":3,\"event\":\"x\"}\n"));
        member.send(json!({"event":"x"})).unwrap();
        // The runner is gone: no error (that would stop the member before
        // the runner's exit restarts it), nothing counted as queued.
        drop(output);
        member.send(json!({"event":"x"})).unwrap();
        assert_eq!(member.queued.load(Ordering::Acquire), 1);
    }

    #[test]
    fn a_member_stop_waits_for_its_stopped_line_or_returns_when_the_runner_is_gone() {
        let (link, output) = link(&[4]);
        let link = Arc::new(link);
        let member = Member {
            link: link.clone(),
            g: 4,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        let answer = link.clone();
        let runner = thread::spawn(move || {
            let line = output.recv().unwrap();
            assert_eq!(
                envelope(line.g, &line.message).unwrap(),
                "{\"g\":4,\"data\":{\"reason\":\"disable\"},\"event\":\"runtime.stop\"}"
            );
            answer.stopped(4, false);
            output
        });
        let began = Instant::now();
        member.stop(StopReason::Disable);
        assert!(began.elapsed() < STOP_WAIT);
        assert_eq!(link.members(), 0);
        assert_eq!(link.leaked(), 0);
        let _output = runner.join().unwrap();
        // A slow cleanup: a leak after the stop window, and the stop returns
        // only once the runner reports the member gone (so a reload waits).
        link.state().members.insert(4);
        let answer = link.clone();
        let late = thread::spawn(move || {
            thread::sleep(STOP_WAIT + Duration::from_millis(400));
            answer.stopped(4, false);
        });
        let began = Instant::now();
        member.stop(StopReason::Idle);
        let took = began.elapsed();
        assert!(took >= STOP_WAIT + Duration::from_millis(400), "{took:?}");
        assert!(took < MEMBER_GONE, "{took:?}");
        assert_eq!(link.leaked(), 1);
        late.join().unwrap();
        // Never reported: the stop gives up at the runner's own deadline.
        link.state().members.insert(4);
        let began = Instant::now();
        member.stop(StopReason::Idle);
        assert!(began.elapsed() >= MEMBER_GONE - Duration::from_millis(50));
        assert_eq!(link.leaked(), 2);
        assert_eq!(link.members(), 0);
        // Gone: no wait at all.
        link.close();
        let began = Instant::now();
        member.stop(StopReason::Idle);
        assert!(began.elapsed() < Duration::from_millis(50));
    }

    /// The plugin reaper's half (`MemberStop::poll`, supervisor.md §7.1):
    /// the same outcomes as the blocking stop, at the times it is polled.
    #[test]
    fn a_polled_member_stop_ends_as_the_blocking_one() {
        let (link, _output) = link(&[4]);
        let link = Arc::new(link);
        let member = Member {
            link: link.clone(),
            g: 4,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        // Reported gone (here: abandoned by the runner) after a leak.
        let mut stop = member.begin_stop(StopReason::Idle);
        let began = Instant::now();
        assert!(!stop.poll(began));
        assert!(!stop.poll(began + STOP_WAIT));
        assert_eq!(link.leaked(), 1);
        assert!(!stop.poll(began + STOP_WAIT * 2), "counted once");
        assert_eq!(link.leaked(), 1);
        link.stopped(4, true);
        assert!(stop.poll(began + STOP_WAIT * 2));
        assert_eq!(link.members(), 0);
        // Never reported: over at the runner's own deadline.
        link.state().members.insert(4);
        let mut stop = member.begin_stop(StopReason::Idle);
        let began = Instant::now();
        assert!(!stop.poll(began + MEMBER_GONE - Duration::from_millis(200)));
        assert!(stop.poll(began + MEMBER_GONE));
        assert_eq!((link.leaked(), link.members()), (2, 0));
        // Gone: over at once.
        link.state().members.insert(4);
        link.close();
        let mut stop = member.begin_stop(StopReason::Idle);
        assert!(stop.poll(Instant::now()));
        assert_eq!(link.members(), 0);
    }

    #[test]
    fn the_reader_drains_to_the_end_when_the_host_is_gone_or_after_a_fault() {
        let lines = b"{\"g\":5,\"id\":1,\"method\":\"osd.show\",\"params\":{}}\n\
            {\"g\":5,\"id\":2,\"method\":\"osd.show\",\"params\":{}}\n\
            {\"g\":0,\"event\":\"runner.stopped\",\"data\":{\"g\":5,\"timedOut\":false}}\n\
            {\"g\":5,\"id\":3,\"method\":\"osd.show\",\"params\":{}}\n"
            .to_vec();
        // Quitting: nobody receives events, `runner.stopped` still counts.
        let (runner, _output) = link(&[5]);
        let (done, stopped) = mpsc::channel();
        runner.state().waiters.insert(5, done);
        let (events, received) = mpsc::channel();
        drop(received);
        let mut cursor = Cursor::new(lines.clone());
        read_lines(&mut cursor, &runner, &events);
        assert_eq!(cursor.position() as usize, lines.len());
        assert_eq!(stopped.try_recv(), Ok(false));
        // A line no plugin owns: one fault, the rest read and dropped.
        let (runner, _output) = link(&[5]);
        let mut input = b"junk\n".to_vec();
        input.extend_from_slice(&lines);
        let (events, received) = mpsc::channel();
        let mut cursor = Cursor::new(input.clone());
        read_lines(&mut cursor, &runner, &events);
        drop(events);
        assert_eq!(cursor.position() as usize, input.len());
        let received: Vec<_> = received.into_iter().collect();
        assert_eq!(received.len(), 1);
        assert!(matches!(&received[0], Event::Runner(1, m) if m["event"] == "runner.fault"));
    }

    #[test]
    fn stops_carry_their_reason_on_the_wire() {
        let reasons = [
            (StopReason::Quit, "quit"),
            (StopReason::Disable, "disable"),
            (StopReason::Uninstall, "uninstall"),
            (StopReason::Reload, "reload"),
            (StopReason::Upgrade, "upgrade"),
            (StopReason::Idle, "idle"),
            (StopReason::Failed, "failed"),
        ];
        for (reason, name) in reasons {
            let (link, output) = link(&[6]);
            link.close();
            // The runner is gone: no wait, nothing sent.
            Member {
                link: Arc::new(link),
                g: 6,
                queued: Arc::new(AtomicUsize::new(0)),
            }
            .stop(reason);
            assert!(output.try_recv().is_err());
            assert_eq!(
                envelope(6, &stop_message(reason)).unwrap(),
                format!(
                    "{{\"g\":6,\"data\":{{\"reason\":\"{name}\"}},\"event\":\"runtime.stop\"}}"
                )
            );
        }
        // The player quits: one `runner.stop` for every member.
        assert_eq!(
            envelope(0, &runner_stop(StopReason::Quit)).unwrap(),
            "{\"g\":0,\"data\":{\"reason\":\"quit\"},\"event\":\"runner.stop\"}"
        );
        // An own process gets the same line without the envelope.
        assert_eq!(
            stop_message(StopReason::Uninstall).to_string(),
            "{\"data\":{\"reason\":\"uninstall\"},\"event\":\"runtime.stop\"}"
        );
    }

    #[test]
    fn five_unanswered_pings_are_a_hang() {
        let mut beat = Heartbeat::new(0);
        assert_eq!(beat.tick(0, 2000), Beat::Ping(1));
        assert_eq!(beat.tick(1999, 2000), Beat::Wait);
        beat.pong(Some(1));
        assert_eq!(beat.tick(2000, 2000), Beat::Ping(2));
        // A stale pong does not count.
        beat.pong(Some(1));
        let mut now = 2000;
        for seq in 3..=6 {
            now += 2000;
            assert_eq!(beat.tick(now, 2000), Beat::Ping(seq));
        }
        now += 2000;
        assert_eq!(beat.tick(now, 2000), Beat::Hung);
        // One answer resets the count.
        let mut beat = Heartbeat::new(0);
        for at in [0, 2000, 4000, 6000] {
            assert!(matches!(beat.tick(at, 2000), Beat::Ping(_)));
        }
        beat.pong(Some(4));
        for at in [8000, 10000, 12000, 14000, 16000] {
            assert!(matches!(beat.tick(at, 2000), Beat::Ping(_)), "{at}");
        }
        assert_eq!(beat.tick(18000, 2000), Beat::Hung);
    }

    #[test]
    fn the_third_restart_within_a_minute_fails_open_for_the_run() {
        let mut policy = Policy::default();
        assert!(!policy.restart(0));
        assert!(!policy.restart(30_000));
        // The first left the window.
        assert!(!policy.restart(61_000));
        assert!(policy.restart(70_000));
        assert!(policy.failed_open);
        assert!(policy.restart(500_000), "for the rest of the run");
        assert_eq!((policy.next_id(), policy.next_id()), (1, 2));
    }

    /// B0 (supervisor.md §7.4, §11): the shared runner's tunables as they
    /// are today.
    #[test]
    fn b0_runner_tunables() {
        assert_eq!(STOP_WAIT, Duration::from_millis(300));
        assert_eq!(MEMBER_GONE, Duration::from_secs(6));
        assert_eq!(MEMBER_QUEUE, 1024);
        assert_eq!(QUEUE, 16 * 1024);
        assert_eq!(MAX_OLD_SPACE, "--max-old-space-size=1024");
        assert_eq!((PING_MS, MISSED_LIMIT), (2_000, 5));
        assert_eq!((RESTART_LIMIT, RESTART_WINDOW_MS), (3, 60_000));
    }

    /// B0: restarts at the edge of the window (one exactly 60 s old has
    /// left it); quarantine is a set of ids, never cleared in a run.
    #[test]
    fn b0_restart_window_edge_and_quarantine() {
        let mut policy = Policy::default();
        assert!(!policy.restart(1_000));
        assert!(!policy.restart(2_000));
        assert!(!policy.restart(61_000));
        assert!(!policy.failed_open);
        assert!(policy.restart(61_999), "2 000, 61 000 and 61 999: three");
        policy.quarantined.insert("vestra.a".into());
        policy.quarantined.insert("vestra.a".into());
        assert_eq!(policy.quarantined.len(), 1);
    }
}
