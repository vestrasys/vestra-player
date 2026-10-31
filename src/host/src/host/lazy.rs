//! Lazy activation, step A1 (doc/process/rounds/2026-10-lazy-activation.md
//! §2): an installed plugin that holds no live resource is stopped after
//! `VESTRA_IDLE_MS` without use ("parked"), its registrations kept in the
//! host's registries and in a recording; at the next start the recording
//! is replayed instead of starting the plugin. The first use of a recorded
//! entry (a menu command or key, an open its resolver takes, a drop, a
//! settings page's callback) starts it again under a new owner generation;
//! the entries move to it (`Capability::rekey`), its live registrations
//! replace them, and the trigger is delivered at its `runtime.ready`.
//!
//! Step B1 (§3): whether a plugin is resident is observed, not declared.
//! At each `runtime.ready` the host notes whether it holds a live resource
//! (an event listener, a panel, a chain...; the SDK's mirror subscriptions
//! do not count); the recording keeps it (`residentAtReady`), written at a
//! park, at that ready when it is resident, and at quit. The next start
//! launches a resident plugin and replays a lazy one. A plugin without a
//! recording (its first run) starts at most `FIRST_RUNS` at a time and, if
//! lazy at its ready, is parked at once: a background index.
//!
//! Off unless `VESTRA_LAZY=1` (the coordinator turns it on after the
//! hardware checks).
use super::*;
use crate::capabilities::{self, Call};
use crate::i18n::Catalog;
use std::collections::{BTreeSet, VecDeque};

/// First runs (no recording) in flight at once (§2.4).
const FIRST_RUNS: usize = 8;
/// How long the classification at ready waits for the playback worker.
const CLASSIFY_WAIT_MS: u64 = 5_000;

/// How long a park waits for its process (§5.4 item 6; disable and quit
/// keep 300 ms).
pub(crate) const PARK_WAIT: Duration = Duration::from_secs(2);
/// `VESTRA_IDLE_MS` when unset.
const IDLE_MS: u64 = 60_000;
/// The recording format.
const SCHEMA: u32 = 1;
/// How often the idle tick looks at the running plugins.
const CHECK_MS: u64 = 250;
/// A wake slower than this shows "Starting <name>…" (§5.2).
const STARTING_OSD_MS: u64 = 400;
/// How long a wake waits for the process its plugin was parked from (as
/// `packages.reload` waits, `RELOAD_WAIT_MS`).
const STOP_WAIT_MS: u64 = 20_000;
/// What a parked plugin is woken by (the recorded entries' triggers).
const TRIGGERS: &[&str] = &[
    "commands.execute",
    "source.resolveRequest",
    "drops.drop",
    "ui.callback",
];

/// A package as registrations see it: a running plugin's
/// (`process::Plugin::origin`) or a parked one's.
#[derive(Clone)]
pub(crate) struct Origin {
    pub id: String,
    pub directory: PathBuf,
    pub catalog: Option<Arc<Catalog>>,
    pub data_directory: Option<PathBuf>,
}
impl Origin {
    /// A string it declared, in the interface language.
    pub fn text(&self, source: &str) -> String {
        self.catalog
            .as_ref()
            .map_or(source, |catalog| catalog.text(source))
            .to_owned()
    }
    /// Text values in the interface language, with its catalog.
    pub fn texts(&self) -> i18n::text::Texts<'_> {
        i18n::text::Texts {
            catalog: self.catalog.as_deref(),
            locale: i18n::locale(),
        }
    }
    /// Where its images may come from: its package, its data directory.
    pub fn image_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.directory.clone()];
        if let Some(data) = self
            .data_directory
            .as_deref()
            .and_then(|d| dunce::canonicalize(d).ok())
        {
            roots.push(data);
        }
        roots
    }
}

/// What a recording is valid for (§2.1): the installed instance, the
/// package digest and version; the player version and build are checked
/// too (a new player records again, the safe side).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub guid: String,
    pub digest: String,
    pub version: Option<String>,
}

/// `<package store>/activation/<instance GUID>.json`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Recording {
    pub schema: u32,
    pub player: String,
    #[serde(default)]
    pub build: Option<String>,
    pub digest: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Whether it held a live resource at its last `runtime.ready` (§3.4):
    /// the next start launches it instead of replaying `calls`.
    pub resident_at_ready: bool,
    pub recorded_at: u64,
    pub calls: Vec<Call>,
}
impl Recording {
    fn new(key: &Key, calls: Vec<Call>, resident_at_ready: bool) -> Self {
        let recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Self {
            schema: SCHEMA,
            player: env!("CARGO_PKG_VERSION").into(),
            build: option_env!("VESTRA_BUILD").map(str::to_owned),
            digest: key.digest.clone(),
            version: key.version.clone(),
            resident_at_ready,
            recorded_at,
            calls,
        }
    }
    /// Valid for this package and this player.
    fn matches(&self, key: &Key) -> bool {
        self.schema == SCHEMA
            && self.player == env!("CARGO_PKG_VERSION")
            && self.build.as_deref() == option_env!("VESTRA_BUILD")
            && self.digest == key.digest
            && self.version == key.version
    }
}

fn file(directory: &Path, guid: &str) -> PathBuf {
    directory.join(format!("{guid}.json"))
}

/// The recording of `key`'s instance, if there is a valid one; a stale or
/// corrupt one is ignored (the plugin starts as before, §2.2).
pub(crate) fn read(directory: &Path, key: &Key) -> Option<Recording> {
    let path = file(directory, &key.guid);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<Recording>(&bytes) {
        Ok(recording) if recording.matches(key) => Some(recording),
        Ok(_) => {
            diag!("lazy: {} is of another package or player", path.display());
            None
        }
        Err(error) => {
            diag!("lazy: {}: {error}", path.display());
            None
        }
    }
}

/// Writes `key`'s recording (a temporary file renamed over the old one).
pub(crate) fn write(
    directory: &Path,
    key: &Key,
    calls: Vec<Call>,
    resident_at_ready: bool,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = file(directory, &key.guid);
    let temporary = path.with_extension("json.tmp");
    let recording = Recording::new(key, calls, resident_at_ready);
    std::fs::write(&temporary, serde_json::to_vec(&recording)?)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

/// Removes `guid`'s recording (uninstall, reload, upgrade).
pub(crate) fn remove(directory: &Path, guid: &str) {
    let path = file(directory, guid);
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        diag!("lazy: {}: {error}", path.display());
    }
}

/// A plugin without a process whose registrations the host keeps.
pub(crate) struct Parked {
    pub id: String,
    pub guid: String,
    /// Its package: its catalog (registered for its settings pages' Slint,
    /// `i18n::catalog_for`: an `Arc` of its own, since the stopped plugin
    /// unregisters its own on another thread), and where its images come
    /// from (`Host::image_roots`).
    pub origin: Origin,
    /// Its name, for "Starting <name>…".
    pub name: String,
    /// Triggers that came while it was parked; held until it is started.
    pub triggers: Vec<Value>,
    /// Since when they wait for the process it was parked from to go.
    pub waiting: Option<u64>,
    /// What it was refused while it ran (`packages.list` `warnings`, and
    /// as text `error`): kept, so a park alone changes nothing there.
    pub warnings: Vec<Value>,
    pub warning: Option<String>,
}
impl Parked {
    fn new(id: String, guid: String, origin: Origin, name: String) -> Self {
        if let Some(catalog) = &origin.catalog {
            i18n::register(&origin.directory, catalog.clone());
        }
        Self {
            id,
            guid,
            origin,
            name,
            triggers: Vec::new(),
            waiting: None,
            warnings: Vec::new(),
            warning: None,
        }
    }
    /// It is no longer parked: its catalog goes (if still this one).
    fn release(&self) {
        if let Some(catalog) = &self.origin.catalog {
            i18n::unregister(&self.origin.directory, catalog);
        }
    }
}

/// A parked plugin started by a trigger, until its `runtime.ready`.
pub(crate) struct Waking {
    pub name: String,
    /// The triggers to deliver at its ready, in order.
    pub queue: Vec<Value>,
    pub since: u64,
    pub announced: bool,
}

/// `VESTRA_TEST_EXECUTE=<manifest id>/<command id>@<ms>[,...]`: a test
/// switch, read in any build like `VESTRA_TEST_REMOVE_DEVICE` (and like it
/// not passed on to a restarted player): run a plugin command that long
/// after start, as a menu click would, or `@park+<ms>` that long after the
/// plugin's next park. Headless tests have no other way in without input.
struct TestExecute {
    manifest: String,
    command: String,
    at: When,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum When {
    At(u64),
    AfterPark(u64),
}

fn test_executes(text: &str) -> Vec<TestExecute> {
    text.split(',')
        .filter_map(|entry| {
            let (command, at) = entry.trim().rsplit_once('@')?;
            let (manifest, command) = command.split_once('/')?;
            Some(TestExecute {
                manifest: manifest.into(),
                command: command.into(),
                at: match at.strip_prefix("park+") {
                    Some(ms) => When::AfterPark(ms.parse().ok()?),
                    None => When::At(at.parse().ok()?),
                },
            })
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Lazy {
    /// `VESTRA_LAZY=1`.
    pub enabled: bool,
    /// `VESTRA_IDLE_MS`.
    pub idle_ms: u64,
    pub parked: BTreeMap<u64, Parked>,
    pub waking: BTreeMap<u64, Waking>,
    /// When each running plugin was last used: a request from it, a
    /// trigger for it, its ready.
    active: BTreeMap<u64, u64>,
    /// `owner.resources` asked, and when.
    asking: BTreeMap<u64, u64>,
    /// Plugins already logged as staying up, with the reason.
    told: BTreeMap<u64, &'static str>,
    next_check: u64,
    tests: Vec<TestExecute>,
    /// Woken owners: the old generation → the new one, for what was on its
    /// way to the old one while the registries moved (a resolution the
    /// worker asked for before `owner.rekey`).
    moved: BTreeMap<u64, u64>,
    /// Ready plugins whose worker half of the classification is asked,
    /// and when.
    classifying: BTreeMap<u64, u64>,
    /// Each running plugin's class at its last ready (§3.4).
    classes: BTreeMap<u64, Class>,
    /// First runs waiting for a place (manifest ids, in start order).
    pub first_queue: VecDeque<String>,
    /// First runs in flight: started, not yet resident or parked and gone.
    first_runs: BTreeSet<u64>,
    /// The owner of each plugin's last parked process (by id), until it is
    /// gone: a start of that plugin waits for it (wake, reload, enable,
    /// install), so two instances never share its data directory.
    stopping: BTreeMap<String, u64>,
}

/// A plugin's class at its `runtime.ready` (§3.4).
pub(crate) struct Class {
    /// Why it is resident (a live resource), `None`: lazy.
    resident: Option<String>,
    /// The playback worker's answer then (`{ live, resolvers }`): its
    /// resolvers, for a recording at quit.
    backend: Value,
}
impl Lazy {
    pub fn from_env() -> Self {
        let enabled = std::env::var("VESTRA_LAZY").as_deref() == Ok("1");
        let idle_ms = std::env::var("VESTRA_IDLE_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(IDLE_MS);
        let tests = test_executes(&std::env::var("VESTRA_TEST_EXECUTE").unwrap_or_default());
        if enabled {
            diag!("lazy: on, idle after {idle_ms} ms");
        }
        Self {
            enabled,
            idle_ms,
            tests,
            ..Self::default()
        }
    }
    /// A running plugin stopped (`Host::stop`).
    pub fn forget_running(&mut self, owner: u64) {
        self.active.remove(&owner);
        self.asking.remove(&owner);
        self.told.remove(&owner);
        self.waking.remove(&owner);
        self.moved.retain(|_, to| *to != owner);
        self.classifying.remove(&owner);
        self.classes.remove(&owner);
    }
    /// A first run may start now (fewer than `FIRST_RUNS` in flight).
    pub fn first_run_free(&self) -> bool {
        self.first_runs.len() < FIRST_RUNS
    }
    /// Installed plugin `owner` started for its first run.
    pub fn first_run(&mut self, owner: u64) {
        self.first_runs.insert(owner);
    }
    /// Queue a first run: a built-in before every other plugin (third-party
    /// first runs must not hold the places while their mounts, sequence and
    /// resolvers wait for the built-ins, `builtins_starting`).
    pub fn queue_first_run(&mut self, id: String, builtin: impl Fn(&str) -> bool) {
        if self.first_queue.contains(&id) {
            return;
        }
        if builtin(&id) {
            let at = self
                .first_queue
                .iter()
                .position(|queued| !builtin(queued))
                .unwrap_or(self.first_queue.len());
            self.first_queue.insert(at, id);
        } else {
            self.first_queue.push_back(id);
        }
    }
    /// Whether a built-in waits for its first run (`builtins` names them).
    pub fn builtin_queued(&self, builtin: impl Fn(&str) -> bool) -> bool {
        self.first_queue.iter().any(|id| builtin(id))
    }
    /// The process plugin `id` was last parked from, while it still goes.
    pub fn stopping(&self, id: &str) -> Option<u64> {
        self.stopping.get(id).copied()
    }
    /// `owner` was used now.
    pub fn touch(&mut self, owner: u64, now: u64) {
        self.active.insert(owner, now);
    }
    /// The parked owner of plugin `id`.
    pub fn parked_id(&self, id: &str) -> Option<u64> {
        self.parked
            .iter()
            .find(|(_, parked)| parked.id == id)
            .map(|(owner, _)| *owner)
    }
}

fn event(message: &Value) -> Option<&str> {
    message["event"].as_str()
}

impl Host {
    /// Whether `message` for `owner` goes to its process now (`Host::send`).
    /// A trigger for a parked owner wakes it; triggers for a waking one wait
    /// for its ready; anything else for a parked owner has no receiver.
    pub(crate) fn lazy_deliver(&mut self, owner: u64, message: &Value) -> bool {
        let event = event(message);
        let trigger = event.is_some_and(|e| TRIGGERS.contains(&e));
        if let Some(&to) = self.lazy.moved.get(&owner) {
            if trigger || event == Some("source.resolveCancel") {
                self.send(to, message.clone());
            }
            return false;
        }
        if let Some(waking) = self.lazy.waking.get_mut(&owner) {
            // A resolution cancelled before the plugin was ready follows
            // its request.
            if trigger || event == Some("source.resolveCancel") {
                waking.queue.push(message.clone());
                return false;
            }
            return true;
        }
        if let Some(parked) = self.lazy.parked.get_mut(&owner) {
            if trigger {
                parked.triggers.push(message.clone());
                self.wake(owner);
            } else if event == Some("source.resolveCancel") && !parked.triggers.is_empty() {
                parked.triggers.push(message.clone());
            }
            return false;
        }
        if trigger && self.plugins.contains_key(&owner) {
            let now = self.now_ms();
            self.lazy.touch(owner, now);
        }
        true
    }
    /// Start parked `from` for its held triggers (§2.2): a new owner
    /// generation, the recorded entries moved to it, the triggers held for
    /// its ready. Not while the process it was parked from is still going
    /// (two instances would share its data directory; reload and install
    /// wait the same way): `lazy_tick` starts it once that process is gone.
    fn wake(&mut self, from: u64) {
        let now = self.now_ms();
        let gone = self.process_gone(from);
        let Some(parked) = self.lazy.parked.get_mut(&from) else {
            return;
        };
        if !gone {
            if parked.waiting.is_none() {
                diag!("lazy: {} waits for its stopped process", parked.id);
                parked.waiting = Some(now);
            }
            return;
        }
        let Some(parked) = self.lazy.parked.remove(&from) else {
            return;
        };
        parked.release();
        match self.start_package(&parked.id) {
            Ok(to) => {
                diag!(
                    "lazy: {} woken by {} (owner {from} → {to})",
                    parked.id,
                    parked
                        .triggers
                        .first()
                        .and_then(event)
                        .unwrap_or("its settings page")
                );
                capabilities::rekey(self, from, to);
                self.lazy.moved.insert(from, to);
                self.lazy.touch(to, now);
                self.lazy.waking.insert(
                    to,
                    Waking {
                        name: parked.name,
                        queue: parked.triggers,
                        since: now,
                        announced: false,
                    },
                );
            }
            Err(error) => self.wake_failed(from, parked, &error.to_string()),
        }
    }
    /// A settings page of `owner` is shown (§2.1): a parked plugin starts,
    /// so the page shows its current values and its callbacks have a
    /// receiver (nothing is held for its ready).
    pub(crate) fn lazy_page_shown(&mut self, owner: u64) {
        if self.lazy.parked.contains_key(&owner) {
            self.wake(owner);
        }
    }
    /// The process `owner` ran in is gone and the worker cleaned up after it
    /// (a replayed owner never had one).
    pub(crate) fn process_gone(&self, owner: u64) -> bool {
        self.cleanup
            .get(&owner)
            .is_none_or(|done| *done == (true, true))
    }
    /// A parked plugin could not start: its entries go, the user is told.
    fn wake_failed(&mut self, from: u64, parked: Parked, error: &str) {
        diag!("lazy: {} could not start: {error}", parked.id);
        self.failures
            .insert(parked.id, error.chars().take(512).collect());
        self.forget_entries(from);
        self.show_osd(&i18n::tr_args("{} could not start", &json!([parked.name])));
    }
    /// A parked owner's registrations go (it is disabled, uninstalled,
    /// reloaded or could not start).
    fn forget_entries(&mut self, owner: u64) {
        capabilities::revoke(self, owner, false);
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.disabled".into(),
            params: Value::Null,
        });
    }
    /// Plugin `id`, if parked, stops being parked: its entries go. With
    /// `drop_recording` its recording goes too, parked or not and whether
    /// lazy activation is on or not (an uninstall, reload, install or
    /// upgrade: what it recorded is of no use any more). Whether it was
    /// parked.
    pub(crate) fn forget_parked(&mut self, id: &str, drop_recording: bool) -> bool {
        if drop_recording {
            self.drop_recording(id);
        }
        let Some(owner) = self.lazy.parked_id(id) else {
            return false;
        };
        if let Some(parked) = self.lazy.parked.remove(&owner) {
            parked.release();
            if drop_recording {
                remove(&self.activation_dir(), &parked.guid);
            }
        }
        self.forget_entries(owner);
        true
    }
    /// At start-up: register `id`'s recorded calls under a new owner with no
    /// process, and park it (§2.2).
    pub(crate) fn lazy_replay(
        &mut self,
        id: &str,
        directory: &Path,
        manifest: &process::Manifest,
        key: &Key,
        recording: Recording,
    ) -> Result<()> {
        self.next_generation += 1;
        let owner = self.next_generation;
        let catalog = i18n::load(directory);
        let origin = Origin {
            id: id.into(),
            directory: directory.to_owned(),
            catalog: catalog.clone(),
            data_directory: process::data_root()
                .map(|root| process::DataKey::Instance(key.guid.clone()).directory(&root)),
        };
        for call in &recording.calls {
            if let Err(error) =
                capabilities::replay(self, owner, &origin, &manifest.capabilities, call)
            {
                self.forget_entries(owner);
                bail!("{}: {}: {}", call.method, error.code, error.message);
            }
        }
        // Everything replayed is recorded (live registrations replace it).
        capabilities::revoke(self, owner, true);
        let name = crate::packages::package_manager::display_name(manifest, catalog.as_deref());
        diag!(
            "lazy: {id} replayed ({} calls) as owner {owner}, not started",
            recording.calls.len()
        );
        self.lazy.parked.insert(
            owner,
            Parked::new(id.into(), key.guid.clone(), origin, name),
        );
        Ok(())
    }
    /// `runtime.ready` of `owner`: a woken plugin's recorded entries that it
    /// did not register again go, then its held triggers are delivered,
    /// each checked against its live state.
    pub(crate) fn lazy_ready(&mut self, owner: u64) {
        let now = self.now_ms();
        self.lazy.touch(owner, now);
        let Some(waking) = self.lazy.waking.remove(&owner) else {
            self.lazy_classify(owner);
            return;
        };
        // The titles of the commands triggered, before their entries may go.
        let titles = waking
            .queue
            .iter()
            .filter(|m| event(m) == Some("commands.execute"))
            .filter_map(|m| m["data"]["id"].as_str())
            .filter_map(|id| {
                let title = self.commands.get(owner, id)?.declaration.shown_title();
                Some((id.to_owned(), title.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        capabilities::prune(self, owner);
        diag!(
            "lazy: owner {owner} ready after {} ms, {} trigger(s)",
            now.saturating_sub(waking.since),
            waking.queue.len()
        );
        // Before its triggers run: what they open is not what it holds at
        // its ready.
        self.lazy_classify(owner);
        for message in waking.queue {
            if event(&message) != Some("commands.execute") {
                self.send(owner, message);
                continue;
            }
            let id = message["data"]["id"].as_str().unwrap_or_default();
            match self.commands.resolve_execution(owner, id) {
                Ok(_) => {
                    if let Err(error) = self.execute_command(owner, id) {
                        diag!("lazy: execute {id}: {error:?}");
                    }
                }
                // As a key does: the command's reason shows.
                Err(commands::CommandError::Disabled {
                    reason: Some(reason),
                }) => self.show_osd(&reason),
                Err(commands::CommandError::Missing(_)) => {
                    let title = titles.get(id).cloned().unwrap_or_else(|| id.to_owned());
                    self.show_osd(&i18n::tr_args("{} is no longer available", &json!([title])));
                }
                Err(error) => diag!("lazy: execute {id}: {error}"),
            }
        }
    }
    /// §3.4: at `runtime.ready`, whether installed plugin `owner` holds a
    /// live resource. The host's half is read now (before any trigger it
    /// was woken by runs); the playback worker's half is asked, and its
    /// answer comes before anything the plugin asks it after this
    /// (`lazy_answer`).
    fn lazy_classify(&mut self, owner: u64) {
        if !self.lazy.enabled || self.lazy_key_of(owner).is_none() {
            return;
        }
        if let Err(reason) = capabilities::record(self, owner, &Value::Null) {
            self.lazy_classified(owner, Some(reason.to_owned()), Value::Null);
            return;
        }
        let asked = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.resources".into(),
            params: Value::Null,
        });
        if asked.is_ok() {
            let now = self.now_ms();
            self.lazy.classifying.insert(owner, now);
        }
    }
    /// `owner`'s class at its ready: kept for its recording; a resident one
    /// is recorded as such now (the next start launches it, even after a
    /// crash); a lazy first run is parked at once (§2.4).
    fn lazy_classified(&mut self, owner: u64, resident: Option<String>, backend: Value) {
        let id = self.plugin_id(owner);
        match &resident {
            Some(reason) => {
                // Which events it listens to (the SDK's mirrors left out).
                let listening = self.plugins.get(&owner).map_or_else(Vec::new, |plugin| {
                    plugin
                        .subscriptions
                        .iter()
                        .filter(|(_, active)| **active)
                        .map(|(event, _)| event.as_str())
                        .collect()
                });
                diag!("lazy: {id} classified resident: {reason} {listening:?}")
            }
            None => diag!("lazy: {id} classified lazy"),
        }
        let first_run = self.lazy.first_runs.contains(&owner);
        if resident.is_some() {
            // Its place goes to the next first run.
            self.lazy.first_runs.remove(&owner);
            if let Some(key) = self.lazy_key_of(owner) {
                let directory = self.activation_dir();
                let recorded = read(&directory, &key).is_some_and(|r| r.resident_at_ready);
                if !recorded && let Err(error) = write(&directory, &key, Vec::new(), true) {
                    diag!("lazy: {id}: its recording: {error}");
                }
            }
        }
        let lazy = resident.is_none();
        self.lazy.classes.insert(owner, Class { resident, backend });
        let now = self.now_ms();
        if lazy && first_run && self.lazy_idle(owner, now, true) {
            let backend = self.lazy.classes[&owner].backend.clone();
            if let Ok(calls) = capabilities::record(self, owner, &backend) {
                self.park(owner, calls);
            }
        }
    }
    /// At quit (§2.1): every running installed plugin classified at its
    /// ready is recorded: a resident one as resident, a lazy one with its
    /// registrations as they are now (left as it was when it holds a live
    /// resource at this moment: an open window, say).
    pub(crate) fn lazy_quit(&mut self) {
        if !self.lazy.enabled {
            return;
        }
        let directory = self.activation_dir();
        let owners = self.lazy.classes.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            let Some(key) = self.lazy_key_of(owner) else {
                continue;
            };
            let id = self.plugin_id(owner);
            let class = &self.lazy.classes[&owner];
            let (calls, resident) = if class.resident.is_some() {
                if read(&directory, &key).is_some_and(|r| r.resident_at_ready) {
                    continue;
                }
                (Vec::new(), true)
            } else {
                match capabilities::record(self, owner, &class.backend) {
                    Ok(calls) => (calls, false),
                    Err(reason) => {
                        diag!("lazy: {id} not recorded at quit: {reason}");
                        continue;
                    }
                }
            };
            match write(&directory, &key, calls, resident) {
                Ok(()) => diag!("lazy: {id} recorded at quit"),
                Err(error) => diag!("lazy: {id}: its recording: {error}"),
            }
        }
    }
    /// Every host tick: "Starting…" for a slow wake, the test hook, and,
    /// every `CHECK_MS`, `owner.resources` for each plugin idle that long.
    pub(crate) fn lazy_tick(&mut self) {
        let now = self.now_ms();
        self.lazy_test_execute(now);
        if !self.lazy.enabled {
            return;
        }
        // Parked plugins whose stopped process is gone start now; one whose
        // process does not go within `STOP_WAIT_MS` fails, as a reload does.
        let due = self
            .lazy
            .parked
            .iter()
            .filter_map(|(owner, parked)| Some((*owner, parked.waiting?)))
            .collect::<Vec<_>>();
        for (owner, since) in due {
            if self.process_gone(owner) {
                self.wake(owner);
            } else if now.saturating_sub(since) >= STOP_WAIT_MS
                && let Some(parked) = self.lazy.parked.remove(&owner)
            {
                parked.release();
                self.wake_failed(owner, parked, "its stopped process did not go within 20 s");
            }
        }
        let slow = self
            .lazy
            .waking
            .values_mut()
            .filter(|w| !w.announced && now.saturating_sub(w.since) >= STARTING_OSD_MS)
            .map(|w| {
                w.announced = true;
                w.name.clone()
            })
            .collect::<Vec<_>>();
        for name in slow {
            self.show_osd(&i18n::tr_args("Starting {}…", &json!([name])));
        }
        if now < self.lazy.next_check {
            return;
        }
        self.lazy.next_check = now + CHECK_MS;
        self.lazy_classify_tick(now);
        self.lazy_first_runs_tick();
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            let immediate = self.lazy_first_lazy(owner);
            if self.lazy.asking.contains_key(&owner) || !self.lazy_idle(owner, now, immediate) {
                continue;
            }
            let asked = self.backend.commands.send(Command {
                reply: Reply { owner, id: 0 },
                method: "owner.resources".into(),
                params: Value::Null,
            });
            if asked.is_ok() {
                self.lazy.asking.insert(owner, now);
            }
        }
    }
    /// The worker did not answer a classification within
    /// `CLASSIFY_WAIT_MS`: resident (the safe side; its first-run place
    /// frees), and said so.
    fn lazy_classify_tick(&mut self, now: u64) {
        let late = self
            .lazy
            .classifying
            .iter()
            .filter(|(_, asked)| now.saturating_sub(**asked) >= CLASSIFY_WAIT_MS)
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        for owner in late {
            self.lazy.classifying.remove(&owner);
            if self.plugins.contains_key(&owner) {
                diag!(
                    "lazy: {}: the playback worker did not answer within {CLASSIFY_WAIT_MS} ms",
                    self.plugin_id(owner)
                );
                let reason = "no answer from the playback worker".to_owned();
                self.lazy_classified(owner, Some(reason), Value::Null);
            }
        }
    }
    /// A first run classified lazy: parked without an idle time (§2.4).
    fn lazy_first_lazy(&self, owner: u64) -> bool {
        self.lazy.first_runs.contains(&owner)
            && self
                .lazy
                .classes
                .get(&owner)
                .is_some_and(|class| class.resident.is_none())
    }
    /// First runs free their place once resident, or once parked and their
    /// process is gone (or gone otherwise); the queue starts in the freed
    /// places. Parked processes that are gone are no longer waited for.
    fn lazy_first_runs_tick(&mut self) {
        let first_runs = std::mem::take(&mut self.lazy.first_runs);
        let now = self.now_ms();
        self.lazy.first_runs = first_runs
            .into_iter()
            .filter(|owner| {
                // Running and not failed (a failed activation is never
                // classified), or parked and its process still going.
                if self.plugins.contains_key(owner) {
                    !matches!(
                        self.activations.status(*owner, now),
                        Ok(package_activation::Status::Failed { .. }
                            | package_activation::Status::TimedOut)
                    )
                } else {
                    self.lazy.parked.contains_key(owner) && !self.process_gone(*owner)
                }
            })
            .collect();
        let gone = self
            .lazy
            .stopping
            .iter()
            .filter(|(_, owner)| self.process_gone(**owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in gone {
            self.lazy.stopping.remove(&id);
        }
        while self.lazy.first_run_free()
            && let Some(id) = self.lazy.first_queue.pop_front()
        {
            self.start_installed(Some(&id));
            // Queued again: it waits for a built-in still to start.
            if self.lazy.first_queue.contains(&id) {
                break;
            }
        }
    }
    /// What the host itself knows of `owner` being idle (§2.3): installed,
    /// ready, unused for `VESTRA_IDLE_MS`, nothing of its in flight.
    /// `immediate`: no idle time (a lazy first run, §2.4).
    fn lazy_idle(&mut self, owner: u64, now: u64, immediate: bool) -> bool {
        let since = *self.lazy.active.entry(owner).or_insert(now);
        if (!immediate && now.saturating_sub(since) < self.lazy.idle_ms)
            || self.lazy.waking.contains_key(&owner)
            || self.lazy.classifying.contains_key(&owner)
            || self.lazy_key_of(owner).is_none()
        {
            return false;
        }
        let id = self.plugins[&owner].manifest.id.clone();
        matches!(
            self.activations.status(owner, now),
            Ok(package_activation::Status::Ready)
        ) && !self.dialog_busy
            && !self.package_busy(&id)
            && !self.commands.busy(owner)
            && !self.settings.busy(owner)
            && !self.activation_ui.iter().any(|(o, _)| *o == owner)
            && !self.source_state_replies.iter().any(|(o, _)| *o == owner)
    }
    /// The playback worker answered `owner.resources`: park `owner` if it is
    /// still idle and nothing it holds is live.
    pub(crate) fn lazy_answer(&mut self, owner: u64, answer: &Value) {
        // The worker's half of the classification at ready (asked first).
        if self.lazy.classifying.remove(&owner).is_some() {
            if !self.plugins.contains_key(&owner) {
                return;
            }
            let resident = answer["live"]
                .as_array()
                .filter(|live| !live.is_empty())
                .map(|live| {
                    format!(
                        "a resource in the playback worker {}",
                        Value::Array(live.clone())
                    )
                });
            self.lazy_classified(owner, resident, answer.clone());
            return;
        }
        let Some(asked) = self.lazy.asking.remove(&owner) else {
            return;
        };
        let now = self.now_ms();
        let immediate = self.lazy_first_lazy(owner);
        if !self.plugins.contains_key(&owner)
            || self.lazy.active.get(&owner).is_some_and(|at| *at >= asked)
            || !self.lazy_idle(owner, now, immediate)
        {
            return;
        }
        let recorded = match answer["live"].as_array().and_then(|live| live.first()) {
            Some(_) => Err("a resource in the playback worker"),
            None => capabilities::record(self, owner, answer),
        };
        let calls = match recorded {
            Ok(calls) => calls,
            Err(reason) => {
                // Asked again after another idle period.
                self.lazy.touch(owner, now);
                if self.lazy.told.insert(owner, reason) != Some(reason) {
                    diag!(
                        "lazy: {} stays running: {reason}{}",
                        self.plugin_id(owner),
                        answer["live"]
                            .as_array()
                            .filter(|live| !live.is_empty())
                            .map(|live| format!(" {}", Value::Array(live.clone())))
                            .unwrap_or_default()
                    );
                }
                return;
            }
        };
        self.park(owner, calls);
    }
    /// Record `owner`, then stop it keeping its registrations (§2.3).
    fn park(&mut self, owner: u64, calls: Vec<Call>) {
        let (Some(key), Some(plugin)) = (self.lazy_key_of(owner), self.plugins.get(&owner)) else {
            return;
        };
        let id = plugin.manifest.id.clone();
        // Its own catalog: the stopping plugin unregisters its `Arc` on
        // the thread that drops it, which must not take the parked one's.
        let origin = Origin {
            catalog: i18n::load(&plugin.directory),
            ..plugin.origin()
        };
        let name = crate::packages::package_manager::display_name(
            &plugin.manifest,
            origin.catalog.as_deref(),
        );
        let count = calls.len();
        // A plugin resident at its ready that let go of everything since is
        // parked, but the next start still launches it (§3.4).
        let resident = self
            .lazy
            .classes
            .get(&owner)
            .is_some_and(|class| class.resident.is_some());
        if let Err(error) = write(&self.activation_dir(), &key, calls, resident) {
            diag!("lazy: {id} not parked: its recording: {error}");
            let now = self.now_ms();
            self.lazy.touch(owner, now);
            return;
        }
        let (warnings, warning) = (self.warnings_of(owner), self.warning_text(owner));
        self.stop(owner, true);
        self.lazy.stopping.insert(id.clone(), owner);
        diag!("lazy: {id} parked ({count} recorded calls)");
        // `VESTRA_TEST_EXECUTE` entries timed from this park.
        let now = self.now_ms();
        if let Some(test) = self
            .lazy
            .tests
            .iter_mut()
            .find(|t| t.manifest == id && matches!(t.at, When::AfterPark(_)))
            && let When::AfterPark(ms) = test.at
        {
            test.at = When::At(now + ms);
        }
        // Its settings pages still show in its language.
        self.lazy.parked.insert(
            owner,
            Parked {
                warnings,
                warning,
                ..Parked::new(id, key.guid, origin, name)
            },
        );
    }
    /// `VESTRA_TEST_EXECUTE`: the due commands run, by manifest id.
    fn lazy_test_execute(&mut self, now: u64) {
        if self.lazy.tests.is_empty() {
            return;
        }
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.lazy.tests)
            .into_iter()
            .partition(|test| matches!(test.at, When::At(at) if at <= now));
        self.lazy.tests = waiting;
        for test in due {
            let owner = self
                .plugins
                .iter()
                .find(|(_, p)| p.manifest.id == test.manifest)
                .map(|(owner, _)| *owner)
                .or_else(|| self.lazy.parked_id(&test.manifest));
            let Some(owner) = owner else {
                diag!("lazy: VESTRA_TEST_EXECUTE: {} is not loaded", test.manifest);
                continue;
            };
            diag!(
                "lazy: VESTRA_TEST_EXECUTE {}/{}",
                test.manifest,
                test.command
            );
            if let Err(error) = self.execute_command(owner, &test.command) {
                diag!("lazy: VESTRA_TEST_EXECUTE: {}", error.message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        Key {
            guid: "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41".into(),
            digest: "abc".into(),
            version: Some("1.0.0".into()),
        }
    }

    #[test]
    fn a_recording_is_read_back_only_for_its_package_and_player() {
        let directory = tempfile::tempdir().unwrap();
        let calls = vec![Call::new(
            "commands.register",
            json!({"id":"hello","revision":1,"declaration":{"title":"Hello"},"state":{"enabled":true}}),
        )];
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        let read_back = read(directory.path(), &key()).unwrap();
        assert_eq!(read_back.calls, calls);
        assert!(!read_back.resident_at_ready);
        // Resident at its ready: read back as such (the start launches it).
        write(directory.path(), &key(), Vec::new(), true).unwrap();
        let resident = read(directory.path(), &key()).unwrap();
        assert!(resident.resident_at_ready && resident.calls.is_empty());
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        // Another digest, version or instance: not this package's.
        for other in [
            Key {
                digest: "other".into(),
                ..key()
            },
            Key {
                version: Some("1.0.1".into()),
                ..key()
            },
        ] {
            assert!(read(directory.path(), &other).is_none());
        }
        let path = file(directory.path(), &key().guid);
        let mut json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // Another player release records again.
        json["player"] = "0.0.1".into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_none());
        // Corrupt: ignored.
        std::fs::write(&path, "{").unwrap();
        assert!(read(directory.path(), &key()).is_none());
        remove(directory.path(), &key().guid);
        assert!(!path.exists());
        remove(directory.path(), &key().guid);
    }

    fn origin(directory: &Path, data: Option<PathBuf>) -> Origin {
        let catalog = Catalog::from_json(r#"{"language":"zh-Hans","messages":{"Hello":"你好"}}"#);
        Origin {
            id: "a.b".into(),
            directory: directory.to_owned(),
            catalog: Some(Arc::new(catalog.unwrap())),
            data_directory: data,
        }
    }

    /// Review M2: a parked plugin keeps its translations although the
    /// stopped plugin unregisters its own catalog later, on another thread.
    #[test]
    fn a_parked_catalog_outlives_the_stopped_plugins_own() {
        let directory = tempfile::tempdir().unwrap();
        let page = directory.path().join("page.slint");
        let running = origin(directory.path(), None);
        let own = running.catalog.clone().unwrap();
        i18n::register(directory.path(), own.clone());
        // As `park`: a catalog of its own (loaded again), then the plugin's
        // drop unregisters the plugin's.
        let parked = Parked::new(
            "a.b".into(),
            "guid".into(),
            origin(directory.path(), None),
            "A".into(),
        );
        i18n::unregister(directory.path(), &own);
        let shown = i18n::catalog_for(&page).expect("still registered");
        assert!(Arc::ptr_eq(&shown, parked.origin.catalog.as_ref().unwrap()));
        assert_eq!(shown.text("Hello"), "你好");
        parked.release();
        assert!(i18n::catalog_for(&page).is_none());
    }

    /// Review M3: a parked plugin's images come from its package and data
    /// directories, as a running one's (`Host::image_roots`).
    #[test]
    fn a_parked_origin_names_its_image_roots() {
        let package = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let origin = origin(package.path(), Some(data.path().to_owned()));
        assert_eq!(
            origin.image_roots(),
            [
                package.path().to_owned(),
                dunce::canonicalize(data.path()).unwrap()
            ]
        );
        let missing = Origin {
            data_directory: Some(data.path().join("gone")),
            ..origin
        };
        assert_eq!(missing.image_roots(), [package.path().to_owned()]);
        assert_eq!(missing.text("Hello"), "你好");
    }

    #[test]
    fn test_executes_name_a_plugin_command_and_a_time() {
        let tests = test_executes("a.b/hello@1500, c.d/x/y@park+20,bad,e.f/g@soon");
        let read = tests
            .iter()
            .map(|t| (t.manifest.as_str(), t.command.as_str(), t.at))
            .collect::<Vec<_>>();
        assert_eq!(
            read,
            [
                ("a.b", "hello", When::At(1500)),
                ("c.d", "x/y", When::AfterPark(20))
            ]
        );
        assert!(test_executes("").is_empty());
    }
}
