//! First runs (MOVE of `lazy.rs`'s `FirstQueue` and first-run places,
//! B2c): a plugin without a checkpoint starts at most `FIRST_RUNS` at a
//! time, built-ins first. The Host still starts first runs itself
//! (`start_installed`); B2d replaces this with Start jobs of `cause:
//! FirstRun` in the core's lanes (`Tunables::first_run_limit`, §4.4), and
//! these unit tests move with it.
use super::*;
use std::collections::{BTreeSet, VecDeque};

/// First runs (no recording) in flight at once (§2.4; the core's
/// `Tunables::first_run_limit` from B2d).
pub(crate) const FIRST_RUNS: usize = 8;

/// First runs waiting for a place: the built-ins at the front, the others
/// after them, each in the order queued; a set and a count beside the
/// order, as the start-up queue asks per plugin.
#[derive(Default)]
pub(crate) struct FirstQueue {
    /// Ids, and whether each is a built-in.
    order: VecDeque<(String, bool)>,
    queued: BTreeSet<String>,
    /// The built-ins, all at the front of `order`.
    builtins: usize,
}
impl FirstQueue {
    /// A built-in goes after the queued built-ins (before every other
    /// plugin), another plugin at the end; one already queued stays.
    fn push(&mut self, id: String, builtin: bool) {
        if !self.queued.insert(id.clone()) {
            return;
        }
        if builtin {
            self.order.insert(self.builtins, (id, builtin));
            self.builtins += 1;
        } else {
            self.order.push_back((id, builtin));
        }
    }
    pub fn pop_front(&mut self) -> Option<String> {
        let (id, builtin) = self.order.pop_front()?;
        self.queued.remove(&id);
        if builtin {
            debug_assert!(self.builtins > 0, "a built-in taken that was not counted");
            self.builtins -= 1;
        }
        Some(id)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.queued.contains(id)
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// The first runs: waiting for a place, and in flight (started, not yet
/// resident or parked and gone).
#[derive(Default)]
pub(crate) struct FirstRuns {
    pub queue: FirstQueue,
    pub(super) running: BTreeSet<u64>,
}
impl FirstRuns {
    /// A first run may start now (fewer than `FIRST_RUNS` in flight).
    pub fn free(&self) -> bool {
        self.running.len() < FIRST_RUNS
    }
    /// Installed plugin `owner` started for its first run.
    pub fn started(&mut self, owner: u64) {
        self.running.insert(owner);
    }
    /// Queue a first run: a built-in before every other plugin (third-party
    /// first runs must not hold the places while their mounts, sequence and
    /// resolvers wait for the built-ins, `builtins_starting`).
    pub fn queue(&mut self, id: String, builtin: impl Fn(&str) -> bool) {
        let builtin = builtin(&id);
        self.queue.push(id, builtin);
    }
    /// Whether a built-in waits for its first run.
    pub fn builtin_queued(&self) -> bool {
        self.queue.builtins > 0
    }
}

impl Host {
    /// First runs free their place once resident, or once parked and their
    /// process is gone (or gone otherwise); the queue starts in the freed
    /// places (every `idle_check`, today's `lazy_first_runs_tick`).
    pub(super) fn first_runs_tick(&mut self) {
        let running = std::mem::take(&mut self.on_demand.first_runs.running);
        let now = self.now_ms();
        self.on_demand.first_runs.running = running
            .into_iter()
            .filter(|owner| {
                // Running and not failed (a failed activation is never
                // classified), or parked and its process still going.
                if self.plugins.contains_key(owner) {
                    !matches!(
                        self.activation_status(*owner, now),
                        Ok(activation::Status::Failed { .. } | activation::Status::TimedOut)
                    )
                } else {
                    self.on_demand.held.contains_key(owner) && self.process_going(*owner)
                }
            })
            .collect();
        while self.on_demand.first_runs.free()
            && let Some(id) = self.on_demand.first_runs.queue.pop_front()
        {
            self.start_installed(Some(&id));
            // Queued again: it waits for a built-in still to start.
            if self.on_demand.first_runs.queue.contains(&id) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_queue_puts_builtins_first_and_counts_them() {
        let mut first = FirstRuns::default();
        let builtin = |id: &str| id.starts_with("vestra.");
        for id in [
            "b.one", "vestra.x", "a.two", "vestra.a", "b.one", "vestra.x",
        ] {
            first.queue(id.into(), builtin);
        }
        assert!(first.builtin_queued());
        assert!(first.queue.contains("a.two"));
        let mut order = Vec::new();
        while let Some(id) = first.queue.pop_front() {
            assert_eq!(first.builtin_queued(), id == "vestra.x");
            order.push(id);
        }
        assert_eq!(order, ["vestra.x", "vestra.a", "b.one", "a.two"]);
        assert!(!first.queue.contains("a.two") && first.queue.is_empty());
        first.queue("a.two".into(), builtin);
        assert!(first.queue.contains("a.two") && !first.builtin_queued());
    }

    #[test]
    fn eight_first_runs_at_a_time() {
        let mut first = FirstRuns::default();
        for owner in 1..=8 {
            assert!(first.free());
            first.started(owner);
        }
        assert!(!first.free());
    }
}
