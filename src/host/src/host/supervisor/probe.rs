//! Test switches (doc/architecture/supervisor.md §2.2; MOVE of
//! `lazy/probe.rs` and `lazy.rs`'s `VESTRA_TEST_EXECUTE`, B2c), read in any
//! build and not passed on to a player restarted after a device loss:
//!
//! - `VESTRA_TEST_REGISTRY=<file>` (the registry diff over every built-in,
//!   record doc/process/rounds/2026-10-lazy-activation.md §5.3: "a test
//!   parks and wakes every built-in and diffs the registries";
//!   tests/lazy_activation.rs): one JSON line per plugin and moment
//!   (`ready`, `parking`, `parked`, `replayed`, `woken`, `quit`) with what
//!   the host's registries hold for it: its commands in registration order
//!   (declaration, shown text, bar button, state, conditions), their key
//!   bindings (defaults, current, shadowed by whom), its settings pages, its
//!   drop providers (and the extensions an earlier provider takes) and the
//!   wake events it listens to, each with its recorded mark. No owner
//!   numbers: they change with every generation.
//! - `VESTRA_TEST_PARK=all`: every installed plugin
//!   counts as lazy at its ready (what it holds live is ignored) and records
//!   what a recording can hold (`capabilities::recordable`), so resident ones
//!   are parked too. **Isolated profile only**: such recordings are
//!   incomplete; they carry `testPark: true` and only a run with the same
//!   switch reads them back (any other run ignores them and records again).
//! - `VESTRA_TEST_WAKE=<ms>`: that long after the
//!   host started (milliseconds from host start, not from start-up), every
//!   parked plugin is woken (as a shown settings page wakes it: nothing is
//!   held for its ready).
//! - `VESTRA_TEST_EXECUTE=<manifest id>/<command id>@<ms>[,...]`: run a
//!   plugin command that long after start, as a menu click would, or
//!   `@park+<ms>` that long after the plugin's next park. Headless tests
//!   have no other way in without input.
//!
//! Source resolvers live in the playback worker and are not listed (no
//! built-in registers one; `owner.resources` reports a lazy plugin's).
use super::*;
use crate::capabilities::{self, Call};
use std::io::Write;

#[derive(Default)]
pub(crate) struct Probe {
    file: Option<PathBuf>,
    pub park_all: bool,
    wake_at: Option<u64>,
}
impl Probe {
    pub fn from_env() -> Self {
        Self {
            file: std::env::var_os("VESTRA_TEST_REGISTRY")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            park_all: park_all(),
            wake_at: std::env::var("VESTRA_TEST_WAKE")
                .ok()
                .and_then(|ms| ms.parse().ok()),
        }
    }
}

/// `VESTRA_TEST_PARK=all`, read once.
pub(crate) fn park_all() -> bool {
    static PARK_ALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PARK_ALL.get_or_init(|| std::env::var("VESTRA_TEST_PARK").as_deref() == Ok("all"))
}

/// One `VESTRA_TEST_EXECUTE` entry.
pub(crate) struct TestExecute {
    manifest: String,
    command: String,
    at: When,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum When {
    At(u64),
    AfterPark(u64),
}

pub(crate) fn test_executes(text: &str) -> Vec<TestExecute> {
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

impl Host {
    /// What a park of `owner` records: its recordable registrations, or
    /// why it cannot be parked; with `VESTRA_TEST_PARK=all`, what a
    /// recording can hold whatever it holds live.
    pub(super) fn standby_record(
        &self,
        owner: u64,
        backend: &Value,
    ) -> std::result::Result<Vec<Call>, &'static str> {
        if self.on_demand.probe.park_all {
            return Ok(capabilities::recordable(self, owner, backend));
        }
        capabilities::record(self, owner, backend)
    }
    /// `VESTRA_TEST_WAKE`: once due, every parked plugin wakes.
    pub(super) fn probe_tick(&mut self, now: u64) {
        if self.on_demand.probe.wake_at.is_none_or(|at| now < at) {
            return;
        }
        self.on_demand.probe.wake_at = None;
        let parked = self.on_demand.held.keys().copied().collect::<Vec<_>>();
        diag!("lazy: VESTRA_TEST_WAKE: {} parked", parked.len());
        for owner in parked {
            self.page_shown(owner);
        }
    }
    /// `VESTRA_TEST_EXECUTE` entries timed from the park of `id` start now.
    pub(super) fn probe_parked(&mut self, id: &str, now: u64) {
        if let Some(test) = self
            .on_demand
            .tests
            .iter_mut()
            .find(|t| t.manifest == id && matches!(t.at, When::AfterPark(_)))
            && let When::AfterPark(ms) = test.at
        {
            test.at = When::At(now + ms);
        }
    }
    /// `VESTRA_TEST_EXECUTE`: the due commands run, by manifest id.
    pub(super) fn test_execute(&mut self, now: u64) {
        if self.on_demand.tests.is_empty() {
            return;
        }
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.on_demand.tests)
            .into_iter()
            .partition(|test| matches!(test.at, When::At(at) if at <= now));
        self.on_demand.tests = waiting;
        for test in due {
            let owner = self
                .plugins
                .iter()
                .find(|(_, p)| p.manifest.id == test.manifest)
                .map(|(owner, _)| *owner)
                .or_else(|| self.parked_id(&test.manifest));
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
    /// `VESTRA_TEST_REGISTRY`: `owner`'s registries now, as one line.
    pub(crate) fn probe_registry(&self, owner: u64, moment: &str) {
        let Some(file) = &self.on_demand.probe.file else {
            return;
        };
        let id = match self.on_demand.held.get(&owner) {
            Some(held) => held.id.clone(),
            None => self.plugin_id(owner),
        };
        let line = json!({"moment":moment,"id":id,"registry":self.registry_of(owner)});
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .and_then(|mut out| writeln!(out, "{line}"));
        if let Err(error) = written {
            diag!("lazy: VESTRA_TEST_REGISTRY {}: {error}", file.display());
        }
    }
    /// At quit: every running and parked plugin's registries.
    pub(super) fn probe_quit(&self) {
        if self.on_demand.probe.file.is_none() {
            return;
        }
        for owner in self.plugins.keys().chain(self.on_demand.held.keys()) {
            self.probe_registry(*owner, "quit");
        }
    }
    fn registry_of(&self, owner: u64) -> Value {
        let commands = self
            .commands
            .entries_of(owner)
            .into_iter()
            .map(|(entry, recorded, sent)| {
                let declaration = &entry.declaration;
                let button = declaration.button.as_ref().map(|button| {
                    let icon = match &button.icon {
                        commands::ButtonIcon::Name(name) => json!({ "name": name }),
                        commands::ButtonIcon::Image { path, max_side } => {
                            json!({"image":path,"maxSide":max_side})
                        }
                    };
                    json!({"icon":icon,"order":button.order})
                });
                json!({
                    "id": entry.key.local_id,
                    "recorded": recorded,
                    "title": declaration.title,
                    "path": declaration.path,
                    "order": declaration.order,
                    "group": declaration.group,
                    "radio": declaration.radio,
                    "bindings": declaration.bindings,
                    "focus": declaration.focus,
                    "repeat": declaration.repeat,
                    "shown": {"title": declaration.display.title, "path": declaration.display.path},
                    "button": button,
                    "state": entry.state,
                    "when": sent.map(|sent| sent.when.clone()),
                })
            })
            .collect::<Vec<_>>();
        let bindings = self
            .input
            .borrow()
            .table
            .infos()
            .into_iter()
            .filter(|info| info.owner == owner)
            .map(|info| {
                let texts = |inputs: &[bindings::Input]| {
                    inputs.iter().map(bindings::text).collect::<Vec<_>>()
                };
                json!({
                    "id": info.id,
                    "panel": info.panel,
                    "defaults": texts(&info.defaults),
                    "current": texts(&info.current),
                    "overridden": info.overridden,
                    "shadowed": info.shadowed.iter().map(|(input, manifest, id)| {
                        format!("{} by {manifest}/{id}", bindings::text(input))
                    }).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        let pages = self
            .settings
            .pages_of(owner)
            .into_iter()
            .map(|(page, recorded)| {
                json!({
                    "id": page.id,
                    "recorded": recorded,
                    "category": page.category,
                    "title": page.title,
                    "order": page.order,
                    "file": page.file,
                    "component": page.component,
                    "global": page.global,
                    "properties": page.properties,
                })
            })
            .collect::<Vec<_>>();
        let drops = self
            .drops
            .providers_of(owner)
            .into_iter()
            .map(|(provider, recorded, taken)| {
                json!({
                    "id": provider.id,
                    "recorded": recorded,
                    "extensions": provider.extensions,
                    "taken": taken,
                })
            })
            .collect::<Vec<_>>();
        let wakes = match self.plugins.get(&owner) {
            Some(plugin) => plugin
                .subscriptions
                .iter()
                .filter(|(event, active)| {
                    **active && capabilities::WAKE_EVENTS.contains(&event.as_str())
                })
                .map(|(event, _)| event.clone())
                .collect::<Vec<_>>(),
            None => self
                .on_demand
                .wakes
                .get(&owner)
                .map(|events| events.iter().cloned().collect())
                .unwrap_or_default(),
        };
        json!({
            "commands": commands,
            "bindings": bindings,
            "pages": pages,
            "drops": drops,
            "wakes": wakes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
