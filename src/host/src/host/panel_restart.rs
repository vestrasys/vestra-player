//! Panels kept while their plugin restarts after a crash (user ruling
//! 2026-11-10, option A; doc/architecture/panel-slots.md "A crash and its
//! restart"). When a plugin with mounted panels crashes and the supervisor
//! restarts it by itself (the built-ins', a Resident plugin's `OnFailure`
//! rule, or the shared runner's restart), its panels stay in the shell
//! during the restart's delay instead of being removed: the shell is not
//! compiled again, and the restarted run's `ui.mount` of the same panel
//! (same file, component, bridge global and slot) takes the kept branch
//! over without a compile. Each removal and re-mount used to compile the
//! whole shell (0.3 to 0.6 s on the UI thread), for up to five restarts.
//!
//! Meanwhile the panel is dimmed under one host line ("<name> is
//! restarting…", `PanelRestarting` in ui/panel-restarting.slint) that takes
//! the pointer, and its callbacks are dropped (the process is gone; a held
//! callback would reach the next run as a stale click). A restart that
//! does not come (the start limit hit, the plugin disabled or uninstalled,
//! standby) removes the panels as before, one compile; so does a restarted
//! run that does not mount a kept panel again within [`REMOUNT_LIMIT`] of
//! its activation.
//!
//! Keys: a panel's key is its run's (`panel_key`). At the restarted run's
//! spawn the kept panels move to the new run's keys (`ui_mount::rekey`,
//! as a woken plugin's tabs do); the shell still has their branches under
//! the old keys, recorded here as aliases (`compiled_key`) until the shell
//! is compiled again.
use super::*;
use std::collections::BTreeSet;

/// How long after its activation a restarted run may take to mount a kept
/// panel again before the panel goes (as `STARTUP_MOUNT_LIMIT`).
pub(crate) const REMOUNT_LIMIT: Duration = Duration::from_secs(5);

/// One plugin's panels kept over its restart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Restarting {
    /// The run the panels belong to: the crashed one, then the restarted
    /// one from its spawn on.
    pub(crate) owner: u64,
    /// Its panels not mounted again yet (keys under `owner`).
    pub(crate) keys: BTreeSet<u64>,
    /// The line drawn over them.
    pub(crate) text: String,
    /// When the restarted run was found activated (ms, `now_ms`).
    pub(crate) ready_at: Option<u64>,
    /// The run that crashed last (no process): what removes a panel the
    /// restarted run did not mount again is sent as its.
    pub(crate) crashed: u64,
}

/// The bookkeeping of kept panels (no Slint, unit-tested).
#[derive(Debug, Default)]
pub(crate) struct Restarts {
    /// By plugin id: the owner changes at each restart, the id does not.
    pub(crate) plugins: BTreeMap<String, Restarting>,
    /// A kept panel's key → the key its branch in the current shell was
    /// compiled under (its crashed run's): cleared when the shell compiles.
    alias: BTreeMap<u64, u64>,
}

impl Restarts {
    /// Keep `keys` of run `owner` of plugin `id` over its restart (a run
    /// that crashed again before mounting them again keeps them still).
    pub(crate) fn keep(&mut self, id: &str, owner: u64, keys: BTreeSet<u64>, text: String) {
        let entry = self
            .plugins
            .entry(id.to_owned())
            .or_insert_with(|| Restarting {
                owner,
                keys: BTreeSet::new(),
                text: String::new(),
                ready_at: None,
                crashed: owner,
            });
        entry.owner = owner;
        entry.crashed = owner;
        entry.keys.extend(keys);
        entry.text = text;
        entry.ready_at = None;
    }
    /// Panel `key` waits for its plugin's restart.
    pub(crate) fn waiting(&self, key: u64) -> bool {
        self.plugins.values().any(|r| r.keys.contains(&key))
    }
    /// The line over panel `key` while it waits.
    pub(crate) fn text_of(&self, key: u64) -> Option<&str> {
        self.plugins
            .values()
            .find(|r| r.keys.contains(&key))
            .map(|r| r.text.as_str())
    }
    /// The run plugin `id`'s kept panels belong to now.
    pub(crate) fn owner_of(&self, id: &str) -> Option<u64> {
        self.plugins.get(id).map(|r| r.owner)
    }
    /// Plugin `id`'s kept panels move to run `to` (its restart started):
    /// each key `old` becomes `rekey(old)`, and its branch stays known by
    /// the key it was compiled under. The moves `(old, new)`, none when
    /// they belong to `to` already.
    pub(crate) fn move_to(
        &mut self,
        id: &str,
        to: u64,
        rekey: impl Fn(u64) -> u64,
    ) -> Vec<(u64, u64)> {
        let Some(entry) = self.plugins.get_mut(id) else {
            return Vec::new();
        };
        if entry.owner == to {
            return Vec::new();
        }
        entry.owner = to;
        entry.ready_at = None;
        let moves = entry
            .keys
            .iter()
            .map(|&old| (old, rekey(old)))
            .collect::<Vec<_>>();
        entry.keys = moves.iter().map(|&(_, new)| new).collect();
        for &(old, new) in &moves {
            let compiled = self.alias.remove(&old).unwrap_or(old);
            self.alias.insert(new, compiled);
        }
        moves
    }
    /// Panel `key` was mounted again (or removed): it no longer waits.
    /// Whether it did.
    pub(crate) fn done(&mut self, key: u64) -> bool {
        let mut found = false;
        self.plugins.retain(|_, r| {
            found |= r.keys.remove(&key);
            !r.keys.is_empty()
        });
        found
    }
    /// Plugin `id` restarts no more: its entry, for its panels to go.
    pub(crate) fn forget(&mut self, id: &str) -> Option<Restarting> {
        self.plugins.remove(id)
    }
    /// The key panel `key`'s branch in the current shell was compiled under.
    pub(crate) fn compiled_key(&self, key: u64) -> u64 {
        self.alias.get(&key).copied().unwrap_or(key)
    }
    /// The panel whose branch was compiled under `branch` (a shell callback
    /// names the branch): an alias's panel, else `branch` itself.
    pub(crate) fn key_of_branch(&self, branch: u64) -> u64 {
        self.alias
            .iter()
            .find(|&(_, &compiled)| compiled == branch)
            .map_or(branch, |(&key, _)| key)
    }
    /// The shell was compiled again: every panel is under its own key.
    pub(crate) fn recompiled(&mut self) {
        self.alias.clear();
    }
    /// Drop what refers to panels no longer mounted.
    pub(crate) fn prune(&mut self, mounted: impl Fn(u64) -> bool) {
        self.alias.retain(|&key, _| mounted(key));
        self.plugins.retain(|_, r| {
            r.keys.retain(|&key| mounted(key));
            !r.keys.is_empty()
        });
    }
}

/// What the supervisor says of a plugin whose panels wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coming {
    /// Its restart runs as `pid`; `ready`: activated.
    Running { pid: u64, ready: bool },
    /// Its restart is due (the delay, the old process's cleanup, a check).
    Waiting,
    /// No restart comes (start limit, disabled, uninstalled, standby).
    No,
}

/// [`Coming`] for a unit's state and whether it has a job that starts it.
pub(crate) fn coming(state: &crate::supervisor::UnitState, starts: bool) -> Coming {
    use crate::supervisor::UnitState;
    match *state {
        UnitState::Active { pid, .. } => Coming::Running {
            pid: pid.0,
            ready: true,
        },
        UnitState::Activating { pid: Some(pid), .. } => Coming::Running {
            pid: pid.0,
            ready: false,
        },
        UnitState::Activating { .. } | UnitState::Deactivating { .. } | UnitState::Inactive
            if starts =>
        {
            Coming::Waiting
        }
        _ => Coming::No,
    }
}

/// Canonical order of a shell's specs, for comparing two of them: the
/// shell does not depend on the order of its panels (each tab is a branch
/// by key; the strips' order is a property), and a key that moved over a
/// restart may sort elsewhere.
pub(crate) fn canonical(mut specs: Vec<composition::CompileSpec>) -> Vec<composition::CompileSpec> {
    specs.sort_by_key(|s| (s.slot as u8, s.owner));
    specs
}

impl Host {
    /// Whether run `owner`, stopping now, is restarted by the supervisor by
    /// itself: it is deactivating after a failure, to land inactive, with a
    /// start of it waiting (a crash under `OnFailure`, the shared runner's
    /// restart). Not a reload, a disable, an uninstall, standby or a start
    /// limit hit.
    pub(crate) fn auto_restarting(&self, owner: u64) -> bool {
        use crate::supervisor::{StopReason as Core, Then, UnitState};
        let Some(unit) = self.supervisor.by_pid(Pid(owner)) else {
            return false;
        };
        let deactivating = self.supervisor.unit(unit).is_some_and(|u| {
            matches!(u.state, UnitState::Deactivating { pid, reason: Core::Failed, then: Then::Inactive, .. } if pid.0 == owner)
                && !u.removed
        });
        deactivating
            && self
                .supervisor
                .job(unit)
                .is_some_and(crate::supervisor::Job::will_start)
    }
    /// The key panel `key`'s values are written under in the current shell
    /// (its own, or the one its branch was compiled under before its
    /// plugin restarted); `None`: not in the shell (a tab beyond the front
    /// tabs, or one whose branch is a placeholder's).
    pub(crate) fn shell_branch(&self, key: u64, tabbed: bool) -> Option<u64> {
        let compiled = self.restarts.compiled_key(key);
        (!tabbed || self.shell_compiled.get(&key) == Some(&compiled)).then_some(compiled)
    }
    /// `mount`'s spec as the current shell has it: under the key its branch
    /// was compiled under.
    pub(crate) fn compiled_spec(&self, mount: &Mount) -> composition::CompileSpec {
        let mut spec = mount.spec.clone();
        spec.owner = self.restarts.compiled_key(spec.owner);
        spec
    }
    /// `stop_run` of a run that the supervisor restarts: its panels stay
    /// (dimmed, with the line) instead of going. The keys kept.
    pub(crate) fn keep_panels(&mut self, owner: u64) -> BTreeSet<u64> {
        let Some(plugin) = self.plugins.get(&owner) else {
            return BTreeSet::new();
        };
        let keys = self
            .mounts
            .values()
            .filter(|m| m.plugin == owner)
            .map(|m| m.spec.owner)
            .collect::<BTreeSet<_>>();
        if keys.is_empty() {
            return keys;
        }
        let id = plugin.manifest.id.clone();
        let catalog = i18n::load(&plugin.directory);
        let name = package_manager::display_name(&plugin.manifest, catalog.as_deref());
        let text = i18n::tr_args("{} is restarting…", &json!([name]));
        diag!(
            "shell: {id} restarts after a fault; its {} panel(s) stay meanwhile (no compile)",
            keys.len()
        );
        self.restarts.keep(&id, owner, keys.clone(), text);
        for &key in &keys {
            self.show_restarting(key);
        }
        keys
    }
    /// Plugin `id`'s kept panels move to its run `to` (at its spawn, or at
    /// its first `ui.mount`, whichever comes first).
    pub(crate) fn adopt_panels(&mut self, id: &str, to: u64) {
        if self.restarts.owner_of(id).is_none_or(|owner| owner == to) {
            return;
        }
        let from = self.restarts.owner_of(id).unwrap_or(to);
        // A key the new run holds already (it cannot, before its first
        // mount) would be overwritten: such a kept panel goes instead.
        let clashing = self
            .restarts
            .plugins
            .get(id)
            .map(|r| {
                r.keys
                    .iter()
                    .copied()
                    .filter(|&key| self.mounts.contains_key(&rekey_panel(key, to)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for key in clashing {
            self.restarts.done(key);
            self.drop_panels(from, vec![key]);
        }
        let moves = self.restarts.move_to(id, to, |key| rekey_panel(key, to));
        if moves.is_empty() {
            return;
        }
        crate::capabilities::ui_mount::rekey_panels(self, from, to);
        diag!("shell: {id}'s kept panel(s) now belong to its restart (owner {from} → {to})");
    }
    /// Panel `key` was mounted again by its restarted run, or went: the
    /// line goes.
    pub(crate) fn panel_back(&mut self, key: u64) {
        if self.restarts.done(key) {
            // The line had the keys (a click on it): they go back to the
            // video, as when a column's tab goes (`sync_column`; the detached
            // window's own key scope there). Anywhere else (the video, the
            // command palette, another panel) they stay.
            let property = restarting_focus_property(self.restarts.compiled_key(key));
            let had = |window: &slint_interpreter::ComponentInstance| {
                matches!(window.get_property(&property), Ok(SValue::Bool(true)))
            };
            if let Some(shell) = self.shell.as_ref().filter(|shell| had(shell)) {
                let _ = shell.set_property(&property, false.into());
                let _ = shell.invoke("focus-video", &[]);
            }
            if let Some(floating) = &self.floating {
                let branch = self.shell_compiled.get(&key).copied().unwrap_or(key);
                let property = restarting_focus_property(branch);
                if matches!(
                    floating.instance.get_property(&property),
                    Ok(SValue::Bool(true))
                ) {
                    let _ = floating.instance.set_property(&property, false.into());
                    let _ = floating.instance.invoke("focus-video", &[]);
                }
            }
            self.set_restarting_line(key, "");
        }
    }
    /// The line over kept panel `key`, in the shell and the floating window.
    pub(crate) fn show_restarting(&mut self, key: u64) {
        let text = self.restarts.text_of(key).unwrap_or("").to_owned();
        self.set_restarting_line(key, &text);
    }
    /// The line over panel `key` (`""`: none), in the shell and the floating
    /// window. A line that goes also forgets whether it had the keys
    /// (`owner<key>-restarting-focused`): the overlay is gone, and a later
    /// `panel_back` must not read a stale `true` (whatever cleared it:
    /// `panel_back`, which read it first, or `restart_over`).
    fn set_restarting_line(&self, key: u64, text: &str) {
        let shell_key = self.restarts.compiled_key(key);
        let branch = self.shell_compiled.get(&key).copied().unwrap_or(key);
        let value = SValue::String(text.into());
        let windows = self
            .shell
            .iter()
            .map(|shell| (shell, shell_key))
            .chain(self.floating.iter().map(|f| (&f.instance, branch)));
        // A panel not compiled into a window has no property there.
        for (window, compiled) in windows {
            let _ = window.set_property(&restarting_property(compiled), value.clone());
            if text.is_empty() {
                let _ = window.set_property(&restarting_focus_property(compiled), false.into());
            }
        }
    }
    /// The kept panels follow their plugins' restarts (every tick): moved
    /// to a restarted run, done once mounted again, gone when no restart
    /// comes or the restarted run did not mount them again in time.
    pub(crate) fn panel_restart_tick(&mut self) {
        if self.restarts.plugins.is_empty() {
            return;
        }
        let mounts = &self.mounts;
        self.restarts.prune(|key| mounts.contains_key(&key));
        let now = self.now_ms();
        let ids = self.restarts.plugins.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let state = self.supervisor.by_id(&id).and_then(|unit| {
                let u = self.supervisor.unit(unit)?;
                let starts = self
                    .supervisor
                    .job(unit)
                    .is_some_and(crate::supervisor::Job::will_start);
                (!u.removed).then(|| coming(&u.state, starts))
            });
            match state.unwrap_or(Coming::No) {
                Coming::Waiting => {}
                Coming::Running { pid, ready } => {
                    if self.plugins.contains_key(&pid) {
                        self.adopt_panels(&id, pid);
                    }
                    let Some(entry) = self.restarts.plugins.get_mut(&id) else {
                        continue;
                    };
                    if !ready || entry.owner != pid {
                        continue;
                    }
                    let since = *entry.ready_at.get_or_insert(now);
                    if now.saturating_sub(since) < REMOUNT_LIMIT.as_millis() as u64 {
                        continue;
                    }
                    // Not mounted again (nor on its way): it goes.
                    let (keys, entry_crashed) = (entry.keys.clone(), entry.crashed);
                    let stale = keys
                        .into_iter()
                        .filter(|&key| !self.mount_pending(key))
                        .collect::<Vec<_>>();
                    if stale.is_empty() {
                        continue;
                    }
                    diag!(
                        "shell: {id} restarted without mounting {} kept panel(s) again; removed",
                        stale.len()
                    );
                    for &key in &stale {
                        self.restarts.done(key);
                        if let Some(mount) = self.mounts.get(&key) {
                            self.images
                                .release_view(mount.plugin, mount_view(mount.spec.slot));
                        }
                    }
                    // Under the crashed run: the restarted one never mounted them
                    // and is told nothing of them (no `ui.preparing`).
                    let crashed = entry_crashed;
                    self.drop_panels(crashed, stale);
                }
                Coming::No => self.give_up_panels(&id),
            }
        }
    }
    /// Plugin `id`'s run stops for good (or parks) while its panels wait:
    /// the wait ends; `stop_run` removes (or keeps as placeholders) its
    /// panels as any of its own.
    pub(crate) fn restart_over(&mut self, id: &str) {
        if let Some(entry) = self.restarts.forget(id) {
            for key in entry.keys {
                self.set_restarting_line(key, "");
            }
        }
    }
    /// No restart of plugin `id` comes: its kept panels go, as a stopped
    /// plugin's do (one compile).
    pub(crate) fn give_up_panels(&mut self, id: &str) {
        let Some(entry) = self.restarts.forget(id) else {
            return;
        };
        diag!(
            "shell: {id} does not restart; its {} kept panel(s) go",
            entry.keys.len()
        );
        self.drop_panels(entry.owner, entry.keys.into_iter().collect());
    }
    /// A change of panel `key` waits to be applied.
    pub(crate) fn mount_pending(&self, key: u64) -> bool {
        self.mount_queue.iter().any(|c| c.key == key)
            || self.preparing.as_ref().is_some_and(|(_, c)| c.key == key)
    }
    /// Remove panels `keys` of run `owner` from the shell (queued: one
    /// compile for them), as `stop_run` removes a stopped run's.
    pub(crate) fn drop_panels(&mut self, owner: u64, keys: Vec<u64>) {
        let keys = keys
            .into_iter()
            .filter(|key| self.mounts.contains_key(key))
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return;
        }
        // Each gone tab's column before it goes.
        let tabs_before = keys
            .iter()
            .map(|key| {
                self.mounts
                    .get(key)
                    .filter(|m| m.spec.slot.tabbed())
                    .and_then(|m| policy_slot(m.spec.slot))
                    .map(|slot| self.policy.panels(slot))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        // The floating window opens again without this panel (`sync_floating`).
        if self
            .floating
            .as_ref()
            .is_some_and(|f| keys.iter().any(|key| f.owners.contains(key)))
        {
            self.close_floating();
        }
        for key in &keys {
            self.policy.remove_owner(*key);
        }
        for (key, tabs_before) in keys.into_iter().zip(tabs_before) {
            let Some(mount) = self.mounts.remove(&key) else {
                continue;
            };
            // Another tab of its column stays shown until the shell is rebuilt.
            let last_tab = tabs_before.iter().all(|&o| o == key);
            if let Some(shell) = &self.shell
                && let Some(property) = mount.spec.slot.visible_property()
                && (!mount.spec.slot.tabbed() || last_tab)
            {
                let _ = shell.set_property(property, false.into());
            }
            self.mount_queue.retain(|c| c.key != key);
            self.mount_queue.push_front(MountChange {
                reply: Reply { owner, id: 0 },
                key,
                contribution: None,
                // Told nothing: its run may be a restart that never mounted it.
                announced: true,
            });
        }
    }
}

/// The shell's (and floating window's) property with the line over the
/// panel compiled under `key` ("" while it is live).
pub(crate) fn restarting_property(key: u64) -> String {
    format!("owner{key}-restarting")
}

/// Whether the line over the panel compiled under `key` has the keys (a
/// click on it gave them).
pub(crate) fn restarting_focus_property(key: u64) -> String {
    format!("owner{key}-restarting-focused")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{ActivatingSub, DeactivatingSub, Pid, StopReason, Then, UnitState};

    fn keys(list: &[u64]) -> BTreeSet<u64> {
        list.iter().copied().collect()
    }

    /// A crash keeps the panels; the restart moves them to its keys with
    /// their branches known by the old ones; the re-mount ends the wait;
    /// a later compile forgets the aliases.
    #[test]
    fn kept_panels_move_to_the_restart_and_keep_their_branches() {
        let mut restarts = Restarts::default();
        // Run 5: its bottom bar (20) and sidebar tab (21).
        restarts.keep("a.b", 5, keys(&[20, 21]), "A is restarting…".into());
        assert!(restarts.waiting(20) && restarts.waiting(21));
        assert_eq!(restarts.text_of(21), Some("A is restarting…"));
        assert_eq!(restarts.compiled_key(20), 20);
        // Its restart runs as 9.
        let moves = restarts.move_to("a.b", 9, |key| rekey_panel(key, 9));
        assert_eq!(moves, [(20, 36), (21, 37)]);
        assert_eq!(restarts.owner_of("a.b"), Some(9));
        // A panel it never mounts again goes as the crashed run's.
        assert_eq!(restarts.plugins["a.b"].crashed, 5);
        assert!(restarts.waiting(36) && !restarts.waiting(20));
        assert_eq!(restarts.compiled_key(36), 20);
        assert_eq!(restarts.key_of_branch(21), 37);
        assert_eq!(restarts.key_of_branch(44), 44);
        // Moved already: nothing to do.
        assert!(restarts.move_to("a.b", 9, |key| key).is_empty());
        // It crashes again before mounting, and restarts as 12: the branch
        // is still the first run's.
        restarts.keep("a.b", 9, keys(&[36, 37]), "A is restarting…".into());
        restarts.move_to("a.b", 12, |key| rekey_panel(key, 12));
        assert_eq!(restarts.plugins["a.b"].crashed, 9);
        assert_eq!(restarts.compiled_key(48), 20);
        assert_eq!(restarts.compiled_key(49), 21);
        // Its bottom bar is mounted again; the tab still waits.
        assert!(restarts.done(48));
        assert!(!restarts.done(48));
        assert!(!restarts.waiting(48) && restarts.waiting(49));
        assert_eq!(
            restarts.compiled_key(48),
            20,
            "still drawn by its old branch"
        );
        restarts.recompiled();
        assert_eq!(restarts.compiled_key(48), 48);
        // The last one: the entry goes.
        assert!(restarts.done(49));
        assert!(restarts.plugins.is_empty());
    }

    /// No restart comes: the entry is handed back once for its panels to
    /// go; prune drops panels removed meanwhile.
    #[test]
    fn a_restart_that_does_not_come_hands_its_panels_back() {
        let mut restarts = Restarts::default();
        restarts.keep("a.b", 5, keys(&[20, 21]), String::new());
        restarts.keep("c.d", 6, keys(&[25]), String::new());
        restarts.prune(|key| key != 21);
        assert_eq!(restarts.plugins["a.b"].keys, keys(&[20]));
        let entry = restarts.forget("a.b").unwrap();
        assert_eq!((entry.owner, entry.keys), (5, keys(&[20])));
        assert!(restarts.forget("a.b").is_none());
        restarts.prune(|_| false);
        assert!(restarts.plugins.is_empty());
    }

    /// The supervisor's states read as a restart that runs, is due or does
    /// not come.
    #[test]
    fn unit_states_say_whether_a_restart_comes() {
        let waiting = UnitState::Activating {
            pid: None,
            since: 0,
            sub: ActivatingSub::AutoRestart { until: 10 },
        };
        assert_eq!(coming(&waiting, true), Coming::Waiting);
        let cleanup = UnitState::Deactivating {
            pid: Pid(5),
            reason: StopReason::Failed,
            then: Then::Inactive,
            sub: DeactivatingSub::Cleanup,
        };
        assert_eq!(coming(&cleanup, true), Coming::Waiting);
        // A disable replaced the start: none comes.
        assert_eq!(coming(&cleanup, false), Coming::No);
        assert_eq!(coming(&UnitState::Inactive, false), Coming::No);
        let spawning = UnitState::Activating {
            pid: Some(Pid(9)),
            since: 0,
            sub: ActivatingSub::Ready,
        };
        assert_eq!(
            coming(&spawning, true),
            Coming::Running {
                pid: 9,
                ready: false
            }
        );
        let active = UnitState::Active {
            pid: Pid(9),
            since: 0,
            last_active: 0,
        };
        assert_eq!(
            coming(&active, false),
            Coming::Running {
                pid: 9,
                ready: true
            }
        );
        let failed = UnitState::Failed {
            result: crate::supervisor::FailResult::StartLimitHit,
            text: String::new(),
        };
        assert_eq!(coming(&failed, true), Coming::No);
        let standby = UnitState::Standby {
            holder: Pid(5),
            recorded: true,
            hold_until: None,
        };
        assert_eq!(coming(&standby, true), Coming::No);
    }

    /// The comparison of two shells ignores the panels' order.
    #[test]
    fn shells_compare_by_their_panels_not_their_order() {
        let spec = |owner, slot| composition::CompileSpec {
            owner,
            file: PathBuf::from("p.slint"),
            component: "P".into(),
            global: Some("Bridge".into()),
            slot,
        };
        use composition::Slot;
        let a = vec![spec(48, Slot::Bottom), spec(25, Slot::Sidebar)];
        let b = vec![spec(25, Slot::Sidebar), spec(48, Slot::Bottom)];
        assert_eq!(canonical(a.clone()), canonical(b));
        let mut c = a.clone();
        c[0].component = "Q".into();
        assert_ne!(canonical(a), canonical(c));
    }
}
