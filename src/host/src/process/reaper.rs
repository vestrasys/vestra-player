//! The plugin reaper (doc/architecture/supervisor.md §7.1, A3): one
//! `plugin-reaper` thread waits for every plugin process the host stops,
//! side by side, instead of one thread per stop: an own process until it
//! exits or its grace (`Plugin::set_stop_wait`) is over, then it is killed;
//! a shared-runner member until the runner reports it gone
//! (`MemberStop`); a retired shared runner until it exits or
//! `shared_runner`'s 300 ms are over (once the `shared-runner-reap`
//! thread). The host hands each over (`reap`, `child`) and hears
//! `Event::Reaped(owner)` once a plugin's process is gone (the
//! supervisor's `process_reaped`). Polled every [`POLL`], as each stop's
//! own thread polled before.
//!
//! At quit (B4, T31) every plugin is handed over here too, and
//! `finish_by` cuts every stop's grace to the quit's one deadline and
//! answers once none is left; `kill` cuts one stop's grace short now. A
//! plugin dropped where it is still waits on that thread (`Drop for
//! Plugin`). Each stop reports the exit code of its own process.
#![forbid(unsafe_code)]

use crate::host::Event;
use crate::process::{Plugin, Stop};
use std::{
    process::Child,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// How often the stopping processes are polled (each stop's thread polled
/// `try_wait` every 5 ms before).
const POLL: Duration = Duration::from_millis(5);

/// What the host hands over.
enum Item {
    /// A stopped plugin; `Event::Reaped(owner)` when it is gone, when
    /// `report`.
    Plugin {
        owner: u64,
        plugin: Box<Plugin>,
        report: bool,
    },
    /// A retired shared runner: killed after `wait`.
    Child { child: Child, wait: Duration },
    /// Kill that owner's process now (`Action::Kill`: a quit, a stuck
    /// process).
    Kill(u64),
    /// Every stop under way ends by this deadline (the quit's one
    /// deadline, supervisor.md §7.2).
    Hasten(Instant),
    /// Answered once nothing is waited for any more.
    Flush(mpsc::Sender<()>),
}

/// One stop being waited for.
struct Waiting {
    /// The owner to report, if any.
    owner: Option<u64>,
    stop: Stop,
    /// Dropped once its process is gone (nothing left to wait for).
    _plugin: Option<Box<Plugin>>,
}

/// The host's handle on the reaper thread.
#[derive(Clone)]
pub struct Reaper {
    items: mpsc::Sender<Item>,
    events: mpsc::Sender<Event>,
}
impl Reaper {
    /// Starts the `plugin-reaper` thread; `Event::Reaped` goes to `events`.
    pub fn spawn(events: mpsc::Sender<Event>) -> Self {
        let (items, received) = mpsc::channel();
        let sender = events.clone();
        let spawned = thread::Builder::new()
            .name("plugin-reaper".into())
            .spawn(move || run(&received, &sender));
        if let Err(error) = spawned {
            // Every stop then waits on a thread of its own, as before.
            diag!("plugin reaper: {error}");
        }
        Self { items, events }
    }
    /// `plugin` (`owner`) was stopped: its stop begins now, and
    /// `Event::Reaped(owner)` comes once its process is gone.
    pub fn reap(&self, owner: u64, plugin: Plugin) {
        self.hand_over(Item::Plugin {
            owner,
            plugin: Box::new(plugin),
            report: true,
        });
    }
    /// `plugin` never ran as a plugin of the host (its load failed after
    /// the spawn): stopped without an event.
    pub fn discard(&self, plugin: Plugin) {
        self.hand_over(Item::Plugin {
            owner: 0,
            plugin: Box::new(plugin),
            report: false,
        });
    }
    /// A retired shared runner: waited for `wait`, then killed.
    pub fn child(&self, child: Child, wait: Duration) {
        self.hand_over(Item::Child { child, wait });
    }
    /// `owner`'s process is killed now (its grace is cut short).
    pub fn kill(&self, owner: u64) {
        self.hand_over(Item::Kill(owner));
    }
    /// Every stop under way ends by `deadline`; the answer comes once none
    /// is left (or never, when the thread is gone: the caller waits with a
    /// timeout).
    pub fn finish_by(&self, deadline: Instant) -> mpsc::Receiver<()> {
        let (done, answer) = mpsc::channel();
        self.hand_over(Item::Hasten(deadline));
        self.hand_over(Item::Flush(done));
        answer
    }
    fn hand_over(&self, item: Item) {
        if matches!(item, Item::Kill(_) | Item::Hasten(_) | Item::Flush(_)) {
            let _ = self.items.send(item);
            return;
        }
        if let Err(mpsc::SendError(item)) = self.items.send(item) {
            // The reaper thread is gone: a thread of its own, as before.
            let events = self.events.clone();
            thread::spawn(move || {
                let mut waiting = begin(item);
                while !waiting.stop.poll(Instant::now()) {
                    thread::sleep(POLL);
                }
                if let Some(owner) = waiting.owner {
                    let _ = events.send(Event::Reaped(owner, waiting.stop.code()));
                }
            });
        }
    }
}

/// The stop of `item` begins: the stop line is sent, the grace starts.
fn begin(item: Item) -> Waiting {
    match item {
        Item::Plugin {
            owner,
            mut plugin,
            report,
        } => Waiting {
            owner: report.then_some(owner),
            stop: plugin.begin_stop(),
            _plugin: Some(plugin),
        },
        Item::Kill(_) | Item::Hasten(_) | Item::Flush(_) => Waiting {
            owner: None,
            stop: Stop::Gone,
            _plugin: None,
        },
        Item::Child { child, wait } => Waiting {
            owner: None,
            stop: Stop::Own {
                child,
                _outbound: mpsc::sync_channel(0).0,
                deadline: Instant::now() + wait,
                killed: false,
                code: None,
                exited: None,
                stderr_done: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            },
            _plugin: None,
        },
    }
}

/// The reaper thread: waits for new stops while none is under way, else
/// polls them all every [`POLL`] (taking new ones as they come); each one
/// over is reported and dropped. Ends when the host is gone.
fn run(items: &mpsc::Receiver<Item>, events: &mpsc::Sender<Event>) {
    let mut waiting: Vec<Waiting> = Vec::new();
    let mut flushes: Vec<mpsc::Sender<()>> = Vec::new();
    loop {
        if waiting.is_empty() {
            for done in flushes.drain(..) {
                let _ = done.send(());
            }
        }
        let item = if waiting.is_empty() {
            match items.recv() {
                Ok(item) => Some(item),
                Err(_) => return,
            }
        } else {
            match items.recv_timeout(POLL) {
                Ok(item) => Some(item),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                // The host is gone: what is left is waited for still (its
                // processes are killed after their grace).
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(POLL);
                    None
                }
            }
        };
        let mut next = item;
        loop {
            let Some(item) = next.take().or_else(|| items.try_recv().ok()) else {
                break;
            };
            match item {
                Item::Kill(owner) => {
                    let now = Instant::now();
                    for stop in waiting.iter_mut().filter(|w| w.owner == Some(owner)) {
                        stop.stop.hasten(now);
                    }
                }
                Item::Hasten(deadline) => {
                    for stop in &mut waiting {
                        stop.stop.hasten(deadline);
                    }
                }
                Item::Flush(done) => flushes.push(done),
                item => waiting.extend(begin_guarded(item)),
            }
        }
        let now = Instant::now();
        waiting.retain_mut(|stop| {
            // A panic in one stop must not lose the others: that one is
            // dropped unreported (its unit keeps tracking the process and
            // lands stuck after 20 s, as for a process that never goes).
            let over =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stop.stop.poll(now)));
            match over {
                Ok(false) => true,
                Ok(true) => {
                    if let Some(owner) = stop.owner {
                        let _ = events.send(Event::Reaped(owner, stop.stop.code()));
                    }
                    false
                }
                Err(_) => {
                    diag!(
                        "plugin reaper: a stop of owner {:?} panicked; no longer waited for",
                        stop.owner
                    );
                    false
                }
            }
        });
    }
}

/// `begin`, a panic in it reported and that stop dropped (the rest go on).
fn begin_guarded(item: Item) -> Option<Waiting> {
    let owner = match &item {
        Item::Plugin { owner, report, .. } => report.then_some(*owner),
        _ => None,
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| begin(item))) {
        Ok(waiting) => Some(waiting),
        Err(_) => {
            diag!(
                "plugin reaper: the stop of owner {owner:?} panicked as it began; not waited for"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A retired runner's process that does not exit is killed after its
    /// wait, beside another one handed over later, and nothing is reported
    /// for either; the thread then waits for the next stop.
    #[test]
    fn children_are_killed_after_their_wait_side_by_side() {
        let (events, received) = mpsc::channel();
        let reaper = Reaper::spawn(events);
        let sleeper = || {
            let mut command = if cfg!(windows) {
                let mut command = std::process::Command::new("ping");
                command.args(["-n", "30", "127.0.0.1"]);
                command
            } else {
                let mut command = std::process::Command::new("sleep");
                command.arg("30");
                command
            };
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let began = Instant::now();
        let (first, second) = (sleeper(), sleeper());
        let (one, two) = (first.id(), second.id());
        reaper.child(first, Duration::from_millis(200));
        reaper.child(second, Duration::from_millis(300));
        // Both gone well before either would have exited by itself.
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(one) || alive(two) {
            assert!(Instant::now() < deadline, "not killed");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(began.elapsed() >= Duration::from_millis(200));
        assert!(
            received.try_recv().is_err(),
            "a retired runner is not reported"
        );
    }

    /// Whether process `pid` still runs (the platform's process list).
    fn alive(pid: u32) -> bool {
        let output = if cfg!(windows) {
            std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .unwrap()
        } else {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .unwrap()
        };
        if cfg!(windows) {
            String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
        } else {
            output.status.success()
        }
    }
}
