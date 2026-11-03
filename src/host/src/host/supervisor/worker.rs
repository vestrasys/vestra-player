//! The spawn worker (doc/architecture/supervisor.md §2.2, §6.1, A19; B4):
//! a plugin's own Node process is created on the `plugin-spawner` thread,
//! not on the UI thread. Creating one costs about 30 ms on Windows (B3's
//! 200-wake storm: one spawn per start inside the host tick), which alone
//! breaks the UI frame-rate rule once a few start in one tick. The thread
//! runs `Plugin::load_manifest` in request order and answers each with
//! `Event::Spawned`; the unit stays `Activating{Spawn}` meanwhile, and what
//! its process says before the Host has it is held for it
//! (`Host::spawned`), so `process_spawned` comes before any event of the
//! pid (§12.3). Shared-runner members (a line to the runner) and
//! development loads stay on the UI thread, as does a start a package RPC
//! answers with its spawn's error (`start_now`).
use super::*;
use std::sync::{
    Arc,
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

/// The Host's handle on the spawn thread.
pub(crate) struct Spawner {
    jobs: mpsc::Sender<SpawnJob>,
    /// The player quits: jobs not begun are dropped, and a process created
    /// meanwhile goes to the reaper at once.
    quitting: Arc<AtomicBool>,
}
impl Spawner {
    /// Starts the `plugin-spawner` thread (`None`: it could not start; the
    /// Host then spawns on its own thread, as before).
    pub fn start(events: mpsc::Sender<Event>, reaper: process::reaper::Reaper) -> Option<Self> {
        let (jobs, received) = mpsc::channel::<SpawnJob>();
        let quitting = Arc::new(AtomicBool::new(false));
        let quit = quitting.clone();
        let spawned = std::thread::Builder::new()
            .name("plugin-spawner".into())
            .spawn(move || {
                for job in received {
                    if quit.load(Ordering::Acquire) {
                        continue;
                    }
                    let result = Plugin::load_manifest(
                        &job.directory,
                        job.owner,
                        events.clone(),
                        job.manifest,
                        job.key,
                        process::Placement::Own {
                            heap_mib: job.heap_mib,
                        },
                    );
                    match result {
                        Ok(plugin) if quit.load(Ordering::Acquire) => reaper.discard(plugin),
                        Ok(plugin) => {
                            if let Err(mpsc::SendError(Event::Spawned(_, Ok(plugin)))) =
                                events.send(Event::Spawned(job.owner, Ok(Box::new(plugin))))
                            {
                                reaper.discard(*plugin);
                            }
                        }
                        Err(error) => {
                            let _ =
                                events.send(Event::Spawned(job.owner, Err(format!("{error:#}"))));
                        }
                    }
                }
            });
        match spawned {
            Ok(_) => Some(Self { jobs, quitting }),
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
    /// The player quits (`shutdown_plugins`).
    pub fn quit(&self) {
        self.quitting.store(true, Ordering::Release);
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
    pub canceled: Option<(crate::supervisor::StopReason, u64)>,
}

impl Host {
    /// Whether this start's own process is created on the spawn thread: with
    /// on-demand start on (the wakes, first runs and restarts of a storm,
    /// §6.1), unless a package RPC answers with its spawn's error now
    /// (`start_now`). With it off every start stays on this thread, as in
    /// B3 (§17.13).
    pub(crate) fn spawn_off_thread(&self) -> bool {
        self.on_demand.enabled && self.start_error.is_none() && !self.on_demand.quitting
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
    pub(crate) fn spawned(&mut self, owner: u64, result: std::result::Result<Box<Plugin>, String>) {
        let Some(spawning) = self.on_demand.spawning.remove(&owner) else {
            // Not asked for (a quit's): stopped.
            if let Ok(plugin) = result {
                self.reaper.discard(*plugin);
            }
            return;
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
                if let Some((reason, grace)) = spawning.canceled {
                    diag!("supervisor: {id}'s process came after its start was stopped");
                    self.terminate(owner, reason, grace);
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
                for event in spawning.events {
                    self.handle(event);
                }
                self.supervisor_run();
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
