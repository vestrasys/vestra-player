//! Test switches for the registry diff over every built-in (record
//! doc/process/rounds/2026-10-lazy-activation.md §5.3: "a test parks and
//! wakes every built-in and diffs the registries";
//! tests/lazy_activation.rs). Read in any build, like `VESTRA_TEST_EXECUTE`:
//!
//! - `VESTRA_TEST_REGISTRY=<file>`: one JSON line per plugin and moment
//!   (`ready`, `parking`, `parked`, `replayed`, `woken`, `quit`) with what
//!   the host's registries hold for it: its commands in registration order
//!   (declaration, shown text, bar button, state, conditions), their key
//!   bindings (defaults, current, shadowed by whom), its settings pages, its
//!   drop providers (and the extensions an earlier provider takes) and the
//!   wake events it listens to, each with its recorded mark. No owner
//!   numbers: they change with every generation.
//! - `VESTRA_TEST_PARK=all`: every installed plugin counts as lazy at its
//!   ready (what it holds live is ignored) and records what a recording can
//!   hold (`capabilities::recordable`), so resident ones are parked too.
//! - `VESTRA_TEST_WAKE=<ms>`: that long after the host started, every
//!   parked plugin is woken (as a shown settings page wakes it: nothing is
//!   held for its ready).
//!
//! Source resolvers live in the playback worker and are not listed (no
//! built-in registers one; `owner.resources` reports a lazy plugin's).
use super::*;
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
            park_all: std::env::var("VESTRA_TEST_PARK").as_deref() == Ok("all"),
            wake_at: std::env::var("VESTRA_TEST_WAKE")
                .ok()
                .and_then(|ms| ms.parse().ok()),
        }
    }
}

impl Host {
    /// What a park of `owner` records: its recordable registrations, or
    /// why it cannot be parked; with `VESTRA_TEST_PARK=all`, what a
    /// recording can hold whatever it holds live.
    pub(super) fn lazy_record(
        &self,
        owner: u64,
        backend: &Value,
    ) -> std::result::Result<Vec<Call>, &'static str> {
        if self.lazy.probe.park_all {
            return Ok(capabilities::recordable(self, owner, backend));
        }
        capabilities::record(self, owner, backend)
    }
    /// `VESTRA_TEST_WAKE`: once due, every parked plugin wakes.
    pub(super) fn probe_tick(&mut self, now: u64) {
        if self.lazy.probe.wake_at.is_none_or(|at| now < at) {
            return;
        }
        self.lazy.probe.wake_at = None;
        let parked = self.lazy.parked.keys().copied().collect::<Vec<_>>();
        diag!("lazy: VESTRA_TEST_WAKE: {} parked", parked.len());
        for owner in parked {
            self.wake(owner);
        }
    }
    /// `VESTRA_TEST_REGISTRY`: `owner`'s registries now, as one line.
    pub(crate) fn probe_registry(&self, owner: u64, moment: &str) {
        let Some(file) = &self.lazy.probe.file else {
            return;
        };
        let id = match self.lazy.parked.get(&owner) {
            Some(parked) => parked.id.clone(),
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
        if self.lazy.probe.file.is_none() {
            return;
        }
        for owner in self.plugins.keys().chain(self.lazy.parked.keys()) {
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
                .lazy
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
