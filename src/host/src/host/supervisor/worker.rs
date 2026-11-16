//! The spawn worker (doc/architecture/supervisor.md §1): a plugin's own Node
//! process is created on the `plugin-spawner` thread, not on the UI thread.
//! Creating one costs about 30 ms on Windows (a 200-wake storm: one spawn
//! per start inside the host tick), which alone breaks the UI frame-rate rule
//! once a few start in one tick. The thread runs `Plugin::load_manifest` in
//! request order, puts each process it created in `Spawner::ready` and answers
//! with `Event::Spawned`; the unit stays `Activating{Spawn}` meanwhile, and
//! what its process says before the Host has it is held for it
//! (`Host::spawned`, `held_for_spawn`), so `process_spawned` comes before any
//! event of the pid. A spawn that panics is a failed spawn. At quit the
//! processes created but not yet taken are stopped with the rest
//! (`take_ready`). Shared-runner members (a line to the runner), development
//! loads and a start a package RPC answers with its spawn's error (`start_now`)
//! stay on the UI thread.
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// One own process to create.
pub(crate) struct SpawnJob {
    pub owner: u64,
    pub directory: PathBuf,
    pub manifest: process::Manifest,
    pub key: process::DataKey,
    pub heap_mib: Option<u32>,
}

/// Processes created and not taken yet, by owner.
type Ready<T> = Arc<Mutex<BTreeMap<u64, T>>>;

/// The Host's handle on the spawn thread.
pub(crate) struct Spawner {
    jobs: mpsc::Sender<SpawnJob>,
    /// The player quits: jobs not begun are dropped, and a process created
    /// meanwhile goes to the reaper at once.
    quitting: Arc<AtomicBool>,
    /// What the thread created, until the Host takes it (`Event::Spawned`)
    /// or the quit stops it.
    ready: Ready<Plugin>,
}
impl Spawner {
    /// Starts the `plugin-spawner` thread (`None`: it could not start; the
    /// Host then spawns on its own thread, as before).
    pub fn start(events: mpsc::Sender<Event>, reaper: process::reaper::Reaper) -> Option<Self> {
        let (jobs, received) = mpsc::channel::<SpawnJob>();
        let quitting = Arc::new(AtomicBool::new(false));
        let ready: Ready<Plugin> = Arc::default();
        let (quit, shelf) = (quitting.clone(), ready.clone());
        let load_events = events.clone();
        let spawned = std::thread::Builder::new()
            .name("plugin-spawner".into())
            .spawn(move || {
                serve(
                    received,
                    &quit,
                    &shelf,
                    |job| {
                        Plugin::load_manifest(
                            &job.directory,
                            job.owner,
                            load_events.clone(),
                            job.manifest,
                            job.key,
                            process::Placement::Own {
                                heap_mib: job.heap_mib,
                            },
                        )
                    },
                    |owner, result| {
                        let _ = events.send(Event::Spawned(owner, result));
                    },
                    |plugin| reaper.discard(plugin),
                );
            });
        match spawned {
            Ok(_) => Some(Self {
                jobs,
                quitting,
                ready,
            }),
            Err(error) => {
                diag!("plugin spawner: {error}");
                None
            }
        }
    }
    /// Queues `job`; the job back when the thread is gone.
    pub fn spawn(&self, job: SpawnJob) -> std::result::Result<(), Box<SpawnJob>> {
        self.jobs
            .send(job)
            .map_err(|mpsc::SendError(job)| Box::new(job))
    }
    /// The process created for `owner`, taken (`Event::Spawned`).
    pub fn take(&self, owner: u64) -> Option<Plugin> {
        self.ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&owner)
    }
    /// The player quits (`shutdown_plugins`): nothing more is created, and
    /// every process created and not taken yet is handed back to be
    /// stopped with the rest.
    pub fn quit(&self) -> Vec<(u64, Plugin)> {
        self.quitting.store(true, Ordering::Release);
        std::mem::take(
            &mut *self
                .ready
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_iter()
        .collect()
    }
}

/// The spawn thread's loop: each job in order, through `load` (a panic in
/// it is a failed spawn, reported as one); a process created is put in
/// `ready` and `deliver`ed as `Ok`, unless the player quits, when it goes
/// to `discard`; a failure is `deliver`ed as `Err`.
fn serve<T, J>(
    jobs: mpsc::Receiver<J>,
    quit: &AtomicBool,
    ready: &Ready<T>,
    load: impl Fn(J) -> Result<T>,
    deliver: impl Fn(u64, std::result::Result<(), String>),
    discard: impl Fn(T),
) where
    J: Owned,
{
    for job in jobs {
        if quit.load(Ordering::Acquire) {
            continue;
        }
        let owner = job.owner();
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(job)));
        match loaded {
            Ok(Ok(made)) => {
                let mut shelf = ready
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if quit.load(Ordering::Acquire) {
                    drop(shelf);
                    discard(made);
                    continue;
                }
                shelf.insert(owner, made);
                drop(shelf);
                deliver(owner, Ok(()));
            }
            Ok(Err(error)) => deliver(owner, Err(format!("{error:#}"))),
            Err(_) => {
                diag!("plugin spawner: the spawn of owner {owner} panicked");
                deliver(
                    owner,
                    Err("its process could not be created (the spawn panicked)".into()),
                );
            }
        }
    }
}

/// A spawn job's owner.
trait Owned {
    fn owner(&self) -> u64;
}
impl Owned for SpawnJob {
    fn owner(&self) -> u64 {
        self.owner
    }
}

/// The owner whose process an event comes from (its first requests, its
/// end): such an event waits while that owner's spawn is not answered.
pub(crate) fn from_process(event: &Event) -> Option<u64> {
    match event {
        Event::Request(owner, _)
        | Event::Prepared(owner, ..)
        | Event::Exited(owner)
        | Event::RuntimeFault(owner, _)
        | Event::OutOfMemory(owner) => Some(*owner),
        _ => None,
    }
}

/// A start whose process the spawn thread creates (`Host::spawning`).
pub(crate) struct Spawning {
    pub unit: UnitId,
    /// The standby holder whose entries move to it once it runs.
    pub from: Option<u64>,
    /// The holder's data, let go at the spawn: held again if the start
    /// fails and the core keeps the unit.
    pub held: Option<Held>,
    /// What started it (for a failure's record).
    pub kind: Option<crate::supervisor::JobKind>,
    /// The trigger that woke it, for the log.
    pub first: Option<String>,
    /// What its process said before the Host had it, in order.
    pub events: Vec<Event>,
    /// The core stopped it meanwhile (`Terminate`): stopped when it comes.
    /// (why, its grace, whether its entry points stay: a start timeout).
    pub canceled: Option<(crate::supervisor::StopReason, u64, bool)>,
}

impl Core {
    /// Whether this start's own process is created on the spawn thread (the
    /// wakes, first runs and restarts of a storm): unless a package RPC answers
    /// with its spawn's error now (`start_now`).
    pub(crate) fn spawn_off_thread(&self) -> bool {
        self.start_error.is_none() && !self.on_demand.quitting
    }
    /// Queues an own process's creation on the spawn thread; `Err` gives
    /// the job back (no thread: spawn here).
    pub(crate) fn spawn_queued(&mut self, job: SpawnJob) -> std::result::Result<(), Box<SpawnJob>> {
        if self.on_demand.spawner.is_none() {
            self.on_demand.spawner = Spawner::start(self.events.clone(), self.reaper.clone());
        }
        match &self.on_demand.spawner {
            Some(spawner) => spawner.spawn(job),
            None => Err(Box::new(job)),
        }
    }
    /// Whether `owner`'s process is still being created (its events wait).
    pub(crate) fn spawning(&self, owner: u64) -> bool {
        self.on_demand.spawning.contains_key(&owner)
    }
    /// An event of `owner`'s process before the Host has it: held for it.
    pub(crate) fn hold_spawning_event(&mut self, owner: u64, event: Event) {
        if let Some(spawning) = self.on_demand.spawning.get_mut(&owner) {
            spawning.events.push(event);
        }
    }
    /// `Event::Spawned`: the spawn thread created `owner`'s process, or
    /// could not. The core's `process_spawned` comes first, then what the
    /// process said meanwhile.
    pub(crate) fn spawned(&mut self, owner: u64, result: std::result::Result<(), String>) {
        let made = self
            .on_demand
            .spawner
            .as_ref()
            .and_then(|spawner| spawner.take(owner));
        let Some(spawning) = self.on_demand.spawning.remove(&owner) else {
            // Not asked for: stopped.
            if let Some(plugin) = made {
                self.reaper.discard(plugin);
            }
            return;
        };
        // Taken by the quit already (`shutdown_plugins`): nothing to do.
        let result = match (result, made) {
            (Ok(()), Some(plugin)) => Ok(Box::new(plugin)),
            (Ok(()), None) => return,
            (Err(error), _) => Err(error),
        };
        let id = self.unit_id(spawning.unit);
        match result {
            Ok(plugin) => {
                let name = plugin.manifest.name.clone();
                if spawning.canceled.is_none() {
                    // Before `add_plugin` (its `process_spawned`): the
                    // core knows it runs alone from the start.
                    self.supervisor.placed_own(Pid(owner));
                }
                self.add_plugin(owner, *plugin);
                let _ = self.backend.commands.send(Command {
                    reply: Reply { owner, id: 0 },
                    method: "owner.registered".into(),
                    params: json!({"manifestId":id,"manifestName":name}),
                });
                if let Some((reason, grace, keep)) = spawning.canceled {
                    diag!("supervisor: {id}'s process came after its start was stopped");
                    if keep {
                        // Its entry points stay under it (held): stopped as
                        // a park is.
                        self.stop_run(
                            owner,
                            process::StopReason::Failed,
                            Duration::from_millis(grace),
                            true,
                        );
                    } else {
                        self.terminate(owner, reason, grace);
                    }
                    self.supervisor_run();
                    return;
                }
                self.started_now(
                    &id,
                    owner,
                    spawning.from,
                    spawning.first.as_deref(),
                    spawning.kind.as_ref(),
                );
                // Each one after the interface carried out what the ones
                // before it asked, as when each was handled alone, then the
                // supervisor's jobs (plan step S2f(3), `outbox::Then`).
                for event in spawning.events {
                    self.then(crate::host::outbox::Then::Event(event));
                }
                self.then(crate::host::outbox::Then::SupervisorRun);
            }
            Err(error) => {
                self.supervisor.activation_end(Pid(owner));
                let now = self.now_ms();
                self.supervisor.set_now(now);
                if spawning.canceled.is_some() {
                    // Nothing ran: the stop the core waits for is over.
                    self.supervisor.process_reaped(Pid(owner), None);
                    self.supervisor.backend_cleaned(Pid(owner));
                    self.supervisor_run();
                    // Kept in standby after a start timeout: its hold.
                    self.note_hold(spawning.unit);
                    return;
                }
                let error = anyhow::anyhow!(error);
                self.spawn_failed_now(
                    spawning.unit,
                    &id,
                    owner,
                    spawning.from,
                    spawning.held,
                    spawning.kind.as_ref(),
                    &error,
                );
                self.supervisor_run();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Job(u64);
    impl Owned for Job {
        fn owner(&self) -> u64 {
            self.0
        }
    }

    /// Jobs run in order; each process created is ready before its
    /// `Spawned` goes out (the Host takes it then); a spawn that panics is
    /// a failed spawn, not a thread that stops answering; after the quit
    /// nothing more is created and what is made goes to `discard`.
    #[test]
    fn spawns_answer_in_order_ready_first_and_a_panic_fails_the_spawn() {
        let (jobs, received) = mpsc::channel();
        for owner in [3, 4, 5, 6] {
            jobs.send(Job(owner)).unwrap();
        }
        drop(jobs);
        let quit = AtomicBool::new(false);
        let ready: Ready<u64> = Arc::default();
        let delivered = Mutex::new(Vec::new());
        let discarded = Mutex::new(Vec::new());
        serve(
            received,
            &quit,
            &ready,
            |job: Job| match job.0 {
                4 => anyhow::bail!("start Node"),
                5 => panic!("boom"),
                6 => {
                    quit.store(true, Ordering::Release);
                    Ok(60)
                }
                n => Ok(n * 10),
            },
            |owner, result| {
                // Ready before it is announced.
                let shelf = ready.lock().unwrap().get(&owner).copied();
                delivered.lock().unwrap().push((owner, result, shelf));
            },
            |made| discarded.lock().unwrap().push(made),
        );
        let delivered = delivered.into_inner().unwrap();
        assert_eq!(delivered.len(), 3, "{delivered:?}");
        assert_eq!(delivered[0], (3, Ok(()), Some(30)));
        assert_eq!(delivered[1].0, 4);
        assert!(
            delivered[1]
                .1
                .as_ref()
                .is_err_and(|e| e.contains("start Node"))
        );
        assert_eq!(delivered[2].0, 5);
        assert!(
            delivered[2]
                .1
                .as_ref()
                .is_err_and(|e| e.contains("panicked"))
        );
        // Created as the player quit: stopped, never announced.
        assert_eq!(discarded.into_inner().unwrap(), [60]);
        assert_eq!(ready.lock().unwrap().len(), 1);
    }

    /// What a process says (its requests, its end, a fault, V8's heap line)
    /// waits while its spawn is not answered; the Host's own events and the
    /// spawn's answer do not.
    #[test]
    fn only_a_process_s_own_events_wait_for_its_spawn() {
        let waits = [
            Event::Request(7, Value::Null),
            Event::Exited(7),
            Event::RuntimeFault(7, "x".into()),
            Event::OutOfMemory(7),
        ];
        for event in &waits {
            assert_eq!(from_process(event), Some(7));
        }
        for event in [
            Event::Spawned(7, Ok(())),
            Event::Reaped(7, Some(0)),
            Event::BackendCleaned(7),
        ] {
            assert_eq!(from_process(&event), None);
        }
    }
}
