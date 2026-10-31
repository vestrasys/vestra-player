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
//! Off unless `VESTRA_LAZY=1` in this first step (the record's B1 step adds
//! what the default needs: mirror subscriptions, recordings at quit, the
//! first-run throttle and an idle state in `packages.list`).
use super::*;
use crate::capabilities::{self, Call};
use crate::i18n::Catalog;

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
    /// Whether it held a live resource at its `runtime.ready` (§3.4). A
    /// recording is written only at a park in this step, so it is `false`.
    pub resident_at_ready: bool,
    pub recorded_at: u64,
    pub calls: Vec<Call>,
}
impl Recording {
    fn new(key: &Key, calls: Vec<Call>) -> Self {
        let recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Self {
            schema: SCHEMA,
            player: env!("CARGO_PKG_VERSION").into(),
            build: option_env!("VESTRA_BUILD").map(str::to_owned),
            digest: key.digest.clone(),
            version: key.version.clone(),
            resident_at_ready: false,
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
            && !self.resident_at_ready
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
pub(crate) fn write(directory: &Path, key: &Key, calls: Vec<Call>) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = file(directory, &key.guid);
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(&Recording::new(key, calls))?)?;
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
    pub directory: PathBuf,
    /// Registered for its settings pages' Slint (`i18n::catalog_for`).
    pub catalog: Option<Arc<Catalog>>,
    /// Its name, for "Starting <name>…".
    pub name: String,
}

/// A parked plugin started by a trigger, until its `runtime.ready`.
pub(crate) struct Waking {
    pub name: String,
    /// The triggers to deliver at its ready, in order.
    pub queue: Vec<Value>,
    pub since: u64,
    pub announced: bool,
}

/// `VESTRA_TEST_EXECUTE=<manifest id>/<command id>@<ms>[,...]`: run a
/// plugin command that long after start, as a menu click would (headless
/// tests; the player has no other way in without input).
struct TestExecute {
    manifest: String,
    command: String,
    at: u64,
}

fn test_executes(text: &str) -> Vec<TestExecute> {
    text.split(',')
        .filter_map(|entry| {
            let (command, at) = entry.trim().rsplit_once('@')?;
            let (manifest, command) = command.split_once('/')?;
            Some(TestExecute {
                manifest: manifest.into(),
                command: command.into(),
                at: at.parse().ok()?,
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
        if let Some(waking) = self.lazy.waking.get_mut(&owner) {
            // A resolution cancelled before the plugin was ready follows
            // its request.
            if trigger || event == Some("source.resolveCancel") {
                waking.queue.push(message.clone());
                return false;
            }
            return true;
        }
        if self.lazy.parked.contains_key(&owner) {
            if trigger {
                self.wake(owner, message.clone());
            }
            return false;
        }
        if trigger && self.plugins.contains_key(&owner) {
            let now = self.now_ms();
            self.lazy.touch(owner, now);
        }
        true
    }
    /// Start parked `from` for `trigger` (§2.2): a new owner generation, the
    /// recorded entries moved to it, the trigger held for its ready.
    fn wake(&mut self, from: u64, trigger: Value) {
        let Some(parked) = self.lazy.parked.remove(&from) else {
            return;
        };
        if let Some(catalog) = &parked.catalog {
            i18n::unregister(&parked.directory, catalog);
        }
        match self.start_package(&parked.id) {
            Ok(to) => {
                diag!(
                    "lazy: {} woken by {} (owner {from} → {to})",
                    parked.id,
                    event(&trigger).unwrap_or("?")
                );
                capabilities::rekey(self, from, to);
                let now = self.now_ms();
                self.lazy.touch(to, now);
                self.lazy.waking.insert(
                    to,
                    Waking {
                        name: parked.name,
                        queue: vec![trigger],
                        since: now,
                        announced: false,
                    },
                );
            }
            Err(error) => {
                diag!("lazy: {} could not start: {error}", parked.id);
                self.failures
                    .insert(parked.id, error.to_string().chars().take(512).collect());
                self.forget_entries(from);
            }
        }
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
    /// Plugin `id`, if parked, stops being parked: its entries go, and its
    /// recording too when `drop_recording`. Whether it was parked.
    pub(crate) fn forget_parked(&mut self, id: &str, drop_recording: bool) -> bool {
        let Some(owner) = self.lazy.parked_id(id) else {
            return false;
        };
        if let Some(parked) = self.lazy.parked.remove(&owner) {
            if let Some(catalog) = &parked.catalog {
                i18n::unregister(&parked.directory, catalog);
            }
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
        if let Some(catalog) = &catalog {
            i18n::register(directory, catalog.clone());
        }
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
                if let Some(catalog) = &catalog {
                    i18n::unregister(directory, catalog);
                }
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
            Parked {
                id: id.into(),
                guid: key.guid.clone(),
                directory: directory.to_owned(),
                catalog,
                name,
            },
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
    /// Every host tick: "Starting…" for a slow wake, the test hook, and,
    /// every `CHECK_MS`, `owner.resources` for each plugin idle that long.
    pub(crate) fn lazy_tick(&mut self) {
        let now = self.now_ms();
        self.lazy_test_execute(now);
        if !self.lazy.enabled {
            return;
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
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        for owner in owners {
            if self.lazy.asking.contains_key(&owner) || !self.lazy_idle(owner, now) {
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
    /// What the host itself knows of `owner` being idle (§2.3): installed,
    /// ready, unused for `VESTRA_IDLE_MS`, nothing of its in flight.
    fn lazy_idle(&mut self, owner: u64, now: u64) -> bool {
        if self.lazy.waking.contains_key(&owner) || self.lazy_key_of(owner).is_none() {
            return false;
        }
        let since = *self.lazy.active.entry(owner).or_insert(now);
        if now.saturating_sub(since) < self.lazy.idle_ms {
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
        let Some(asked) = self.lazy.asking.remove(&owner) else {
            return;
        };
        let now = self.now_ms();
        if !self.plugins.contains_key(&owner)
            || self.lazy.active.get(&owner).is_some_and(|at| *at > asked)
            || !self.lazy_idle(owner, now)
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
        let directory = plugin.directory.clone();
        let catalog = plugin.catalog.clone();
        let name =
            crate::packages::package_manager::display_name(&plugin.manifest, catalog.as_deref());
        let count = calls.len();
        if let Err(error) = write(&self.activation_dir(), &key, calls) {
            diag!("lazy: {id} not parked: its recording: {error}");
            let now = self.now_ms();
            self.lazy.touch(owner, now);
            return;
        }
        self.stop(owner, true);
        // Its settings pages still show in its language.
        if let Some(catalog) = &catalog {
            i18n::register(&directory, catalog.clone());
        }
        diag!("lazy: {id} parked ({count} recorded calls)");
        self.lazy.parked.insert(
            owner,
            Parked {
                id,
                guid: key.guid,
                directory,
                catalog,
                name,
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
            .partition(|test| test.at <= now);
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
        write(directory.path(), &key(), calls.clone()).unwrap();
        let read_back = read(directory.path(), &key()).unwrap();
        assert_eq!(read_back.calls, calls);
        assert!(!read_back.resident_at_ready);
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

    #[test]
    fn test_executes_name_a_plugin_command_and_a_time() {
        let tests = test_executes("a.b/hello@1500, c.d/x/y@20,bad,e.f/g@soon");
        let read = tests
            .iter()
            .map(|t| (t.manifest.as_str(), t.command.as_str(), t.at))
            .collect::<Vec<_>>();
        assert_eq!(read, [("a.b", "hello", 1500), ("c.d", "x/y", 20)]);
        assert!(test_executes("").is_empty());
    }
}
