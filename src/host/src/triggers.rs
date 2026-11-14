//! Triggers (doc/architecture/events-and-triggers.md §3, §4.1, §4.2,
//! §4.10): the conditions plugins hand over with `plugin.trigger()
//! .firesOn(condition)`, kept and evaluated here, with or without the
//! plugin's process; `enter` when one turns true, `leave` when it turns
//! false (T1), as `triggers.fired` notifications the Host delivers
//! (`host/triggers.rs`: sent to a running plugin, held for one that starts,
//! a wake for one in standby).
//!
//! Identity (§4.2): an unnamed trigger is its place among the unnamed ones
//! `activate()` declares (`TriggerId::Unnamed`), a named one its name. A
//! plugin's kept conditions belong to its installed instance (manifest id
//! here; the file is the instance's), not to a process generation: they
//! stay through standby, a crash and a player restart (§4.10).
//!
//! Storage (design §6, M4, decided in step A2): one small file per
//! installed instance, `<package store>/triggers/<instance GUID>.json`,
//! written (a temporary file renamed over it) in the call that changed
//! what it holds, so a condition given at run time survives a player crash
//! a moment later. It is not the standby recording: a recording is written
//! only at standby and discarded at any package or player upgrade (F4),
//! while a named condition must outlive a plugin upgrade. The file says
//! which package version gave its conditions; a spawn of another version
//! clears the unnamed ones (`spawned`). Conditions with a range list are
//! never written: they describe one source and are not kept across a player
//! restart (T12). "Entered" is never written (T13).
use crate::conditions::{Condition, Facts, Moment, Refusal, Truth, groups};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Triggers per plugin (R10): a request beyond it is refused with `limit`.
pub const MAX_TRIGGERS: usize = 64;
/// The longest trigger name.
pub const MAX_NAME: usize = 64;
/// The notification's event name.
pub const EVENT: &str = "triggers.fired";
/// The file format.
const SCHEMA: u32 = 1;
/// How far (media seconds at 1×; times the rate above it) a
/// `source.state` position may lie behind the model's newest report and
/// still be a stale one: a push is 100 ms, the UI tick 16 ms, with margin.
const STALE_POSITION: f64 = 0.25;

/// A trigger's identity within its plugin (§4.2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TriggerId {
    /// Its place among the unnamed triggers `activate()` declares.
    Unnamed(u32),
    Named(String),
}
impl TriggerId {
    /// The wire form: the place (a number) or the name (a string).
    pub fn parse(value: &Value) -> Result<Self, Refusal> {
        match value {
            Value::Number(number) => match number.as_u64() {
                Some(place) if (place as usize) < MAX_TRIGGERS => Ok(Self::Unnamed(place as u32)),
                Some(_) => Err(Refusal {
                    code: "limit",
                    message: format!("a plugin has at most {MAX_TRIGGERS} triggers"),
                }),
                None => Err("trigger is a name or a place".into()),
            },
            Value::String(name)
                if !name.is_empty()
                    && name.chars().count() <= MAX_NAME
                    && !name.chars().any(char::is_control) =>
            {
                Ok(Self::Named(name.clone()))
            }
            _ => Err(format!("trigger is a name of 1–{MAX_NAME} characters or a place").into()),
        }
    }
    pub fn wire(&self) -> Value {
        match self {
            Self::Unnamed(place) => json!(place),
            Self::Named(name) => json!(name),
        }
    }
    /// How it reads in a diagnostic line.
    pub fn label(&self) -> String {
        match self {
            Self::Unnamed(place) => format!("#{place}"),
            Self::Named(name) => format!("\"{name}\""),
        }
    }
}

/// Which way a condition flipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Enter,
    Leave,
}
impl Edge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Leave => "leave",
        }
    }
}

/// The package a plugin's kept conditions were given by: its digest and
/// version, as a recording judges "another version" (F4). `None`: not an
/// installed package (a development load).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub digest: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// One notification for the Host to deliver.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// The plugin (manifest id).
    pub plugin: String,
    pub trigger: TriggerId,
    pub seq: u64,
    pub edge: Edge,
    /// `{ start, end, index }` when its position list matched.
    pub range: Option<Value>,
    /// The position it was evaluated at, when a position report caused it.
    pub position: Option<f64>,
}
impl Notification {
    /// The backlog's merge key (§4.9): per trigger the newest `enter` and
    /// the newest `leave`, in their order.
    pub fn merge_key(&self) -> String {
        format!("{EVENT} {} {}", self.trigger.label(), self.edge.as_str())
    }
    /// The event's data, with `state` (the wire `source.state`).
    pub fn data(&self, state: &Value) -> Value {
        let mut state = state.clone();
        if let Some(position) = self.position
            && state.is_object()
        {
            state["position"] = json!(position);
        }
        let mut data = json!({"trigger":self.trigger.wire(),"seq":self.seq,
            "edge":self.edge.as_str(),"state":state});
        if let Some(range) = &self.range {
            data["range"] = range.clone();
        }
        data
    }
}

/// One kept condition.
#[derive(Debug)]
struct Kept {
    /// As given (compared for T5, written to the file).
    wire: Value,
    condition: Condition,
    groups: u8,
    seq: u64,
    /// The source its range list belongs to (T12).
    bound: Option<u64>,
    /// "Entered" (T1): `Some(range)` while it holds.
    entered: Option<Option<usize>>,
    /// The process generation that gave it (0: read from the file).
    given_by: u64,
    /// The position at its last evaluation (T2: what continuous playback
    /// passed since).
    last_position: Option<f64>,
    /// A notification of it could not be delivered and "entered" went
    /// back (`undelivered`): the next `triggers.declare` evaluates it.
    retry: bool,
}

#[derive(Debug, Default)]
struct Plugin {
    /// Its instance (the file's name); `None`: kept in memory only.
    instance: Option<String>,
    /// The package that gave its conditions.
    package: Option<Package>,
    /// How many unnamed triggers that package's `activate()` declares.
    unnamed: Option<u32>,
    kept: BTreeMap<TriggerId, Kept>,
    /// What its file holds now (not written again when equal).
    written: Option<Vec<u8>>,
}

/// The file of one instance.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct File {
    schema: u32,
    #[serde(default)]
    package: Option<Package>,
    #[serde(default)]
    unnamed: Option<u32>,
    triggers: Vec<Saved>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    trigger: Value,
    when: Value,
    seq: u64,
}

/// The registry: every plugin's kept conditions and the facts they are
/// evaluated against.
#[derive(Debug, Default)]
pub struct Triggers {
    plugins: BTreeMap<String, Plugin>,
    /// The last sequence number given out (T6).
    seq: u64,
    /// Where the files are (`None`: nothing is written, tests).
    directory: Option<PathBuf>,
    facts: Facts,
    moment: Moment,
    /// The clock at the last evaluation of the time conditions (`time`).
    time_checked: f64,
    /// The newest position report of the position model: (source, position).
    reported: Option<(u64, f64)>,
    /// Diagnostic lines for the Host's log, taken by `diagnostics`.
    told: Vec<String>,
}

impl Triggers {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            ..Self::default()
        }
    }
    /// The lines to log since the last call.
    pub fn diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.told)
    }
    /// Whether any condition is kept (the Host's ticks do nothing without).
    pub fn is_empty(&self) -> bool {
        self.plugins.values().all(|p| p.kept.is_empty())
    }
    /// The kept conditions of `plugin`: (trigger, condition as given,
    /// sequence number, entered), for `host.triggers` in tests.
    #[cfg(test)]
    pub fn kept(&self, plugin: &str) -> Vec<(TriggerId, Value, u64, bool)> {
        self.plugins.get(plugin).map_or_else(Vec::new, |p| {
            p.kept
                .iter()
                .map(|(id, k)| (id.clone(), k.wire.clone(), k.seq, k.entered.is_some()))
                .collect()
        })
    }

    // ---- what the plugin does ----------------------------------------

    /// `triggers.set { trigger, when }` of `plugin`'s run `owner`: the
    /// condition is kept (T4) and evaluated at once (T3); the same
    /// condition again changes nothing (T5); another replaces it in one
    /// step without a `leave` (T6). Its sequence number, and what flipped.
    pub fn set(
        &mut self,
        plugin: &str,
        owner: u64,
        trigger: TriggerId,
        when: &Value,
        now: f64,
    ) -> Result<(u64, Vec<Notification>), Refusal> {
        let condition = Condition::parse_coded(when)?;
        self.moment.time = now;
        let bound = condition.position().map(|_| self.moment.source);
        let entry = self.plugins.entry(plugin.to_owned()).or_default();
        if let Some(kept) = entry.kept.get_mut(&trigger)
            && kept.wire == *when
            && kept.bound == bound
        {
            kept.given_by = owner;
            return Ok((kept.seq, Vec::new()));
        }
        if !entry.kept.contains_key(&trigger) && entry.kept.len() >= MAX_TRIGGERS {
            return Err(Refusal {
                code: "limit",
                message: format!("a plugin keeps at most {MAX_TRIGGERS} trigger conditions"),
            });
        }
        self.seq += 1;
        let seq = self.seq;
        let groups = condition.groups();
        // A replaced range list for the same source (T6) whose position is
        // still in the very range it was entered with (the same start, end
        // and lead; its place may move, a list grows as a scan goes on) is
        // still entered: the replacement is no false-to-true edge, so no
        // second `enter` for one range (T1). Anything else is entered anew
        // by T3, without a `leave` for the old one.
        let (entered, last_position) = entry
            .kept
            .get(&trigger)
            .filter(|old| old.bound.is_some() && old.bound == bound)
            .and_then(|old| {
                let (Some(Some(was)), Some(before), Some(after)) =
                    (old.entered, old.condition.position(), condition.position())
                else {
                    return None;
                };
                let truth = condition.eval(&self.facts, Some(&self.moment), bound);
                let now = truth.range?;
                (truth.holds
                    && before.lead == after.lead
                    && before.given.get(was) == after.given.get(now))
                .then_some((Some(Some(now)), old.last_position))
            })
            .unwrap_or((None, None));
        entry.kept.insert(
            trigger.clone(),
            Kept {
                wire: when.clone(),
                condition,
                groups,
                seq,
                bound,
                entered,
                given_by: owner,
                last_position,
                retry: false,
            },
        );
        self.save(plugin);
        let mut out = Vec::new();
        self.evaluate_one(plugin, &trigger, None, &mut out);
        Ok((seq, out))
    }
    /// `triggers.off { trigger }`: the condition goes, without a `leave`
    /// (T7). Its new sequence number (T6).
    pub fn off(&mut self, plugin: &str, trigger: &TriggerId) -> u64 {
        self.seq += 1;
        if let Some(entry) = self.plugins.get_mut(plugin)
            && entry.kept.remove(trigger).is_some()
        {
            self.save(plugin);
        }
        self.seq
    }
    /// The SDK had nobody to handle a notification of `trigger` (T14): its
    /// condition goes, with one diagnostic line.
    pub fn unhandled(&mut self, plugin: &str, trigger: &TriggerId) -> u64 {
        if self
            .plugins
            .get(plugin)
            .is_some_and(|p| p.kept.contains_key(trigger))
        {
            self.told.push(format!(
                "triggers: {plugin} trigger {} has no enter or leave: its condition is removed",
                trigger.label()
            ));
        }
        self.off(plugin, trigger)
    }
    /// `triggers.declare { names, unnamed }` at the end of the synchronous
    /// part of `activate()` of run `owner` (T14): a kept name it did not
    /// declare goes; a number of unnamed triggers other than the one this
    /// version declared before removes the unnamed conditions this run did
    /// not give again (they may belong to other code).
    pub fn declare(&mut self, plugin: &str, owner: u64, names: &[String], unnamed: u32) {
        let entry = self.plugins.entry(plugin.to_owned()).or_default();
        let mut told = Vec::new();
        entry.kept.retain(|id, kept| match id {
            TriggerId::Named(name) if !names.contains(name) => {
                told.push(format!(
                    "triggers: {plugin} no longer declares trigger \"{name}\": its condition is removed"
                ));
                false
            }
            TriggerId::Unnamed(place) if *place >= unnamed && kept.given_by != owner => {
                told.push(format!(
                    "triggers: {plugin} declares {unnamed} unnamed triggers: the condition of #{place} is removed"
                ));
                false
            }
            _ => true,
        });
        if let Some(before) = entry.unnamed.filter(|before| *before != unnamed) {
            entry.kept.retain(|id, kept| {
                let keep = !matches!(id, TriggerId::Unnamed(_)) || kept.given_by == owner;
                if !keep {
                    told.push(format!(
                        "triggers: {plugin} declares {unnamed} unnamed triggers, not {before}: the kept condition of {} is removed",
                        id.label()
                    ));
                }
                keep
            });
        }
        entry.unnamed = Some(unnamed);
        self.told.extend(told);
        self.save(plugin);
    }
    /// After `declare`: `plugin`'s conditions whose notification could not
    /// be delivered (`undelivered`) are evaluated now that its triggers
    /// are declared, so a flip lost while it had no process (a fault's
    /// evaluation, T13) or while its backlog was full reaches it.
    pub fn redeliver(&mut self, plugin: &str, now: f64) -> Vec<Notification> {
        let Some(entry) = self.plugins.get(plugin) else {
            return Vec::new();
        };
        let ids: Vec<TriggerId> = entry
            .kept
            .iter()
            .filter(|(_, kept)| kept.retry)
            .map(|(id, _)| id.clone())
            .collect();
        let mut out = Vec::new();
        if ids.is_empty() {
            return out;
        }
        self.moment.time = now;
        for id in ids {
            self.evaluate_one(plugin, &id, None, &mut out);
        }
        out
    }
    /// The Host could not deliver `plugin`'s notification `data` (the
    /// `triggers.fired` data; `why`: no process to take it, the plugin
    /// disabled or failed, its backlog or the start lane full). One
    /// diagnostic line; and, when it is still the condition that fired it
    /// (the same `seq`), "entered" goes back to what the plugin last heard,
    /// so the next evaluation fires it again instead of the latch keeping
    /// it quiet until the condition turns the other way and back: a lost
    /// `enter` un-latches it, a lost `leave` latches it again. The next
    /// `triggers.declare` of the plugin evaluates it too (`redeliver`).
    pub fn undelivered(&mut self, plugin: &str, data: &Value, why: &str) {
        let (Ok(id), Some(seq), Some(edge)) = (
            TriggerId::parse(&data["trigger"]),
            data["seq"].as_u64(),
            data["edge"].as_str(),
        ) else {
            return;
        };
        self.told.push(format!(
            "triggers: {plugin} trigger {} {edge} not delivered ({why}): it fires again at its next evaluation",
            id.label()
        ));
        let range = data["range"]["index"].as_u64().map(|index| index as usize);
        let Some(kept) = self
            .plugins
            .get_mut(plugin)
            .and_then(|p| p.kept.get_mut(&id))
        else {
            return;
        };
        if kept.seq != seq {
            return;
        }
        match edge {
            "enter" if kept.entered == Some(range) => kept.entered = None,
            "leave" if kept.entered.is_none() => kept.entered = Some(range),
            _ => return,
        }
        kept.retry = true;
    }

    // ---- what happens to the plugin ----------------------------------

    /// The installed instance of `plugin` is known (start-up, or its first
    /// condition): where its file is; its conditions are read from it once
    /// (a player restart keeps them, T13: not "entered").
    pub fn load(&mut self, plugin: &str, instance: Option<String>) {
        let entry = self.plugins.entry(plugin.to_owned()).or_default();
        if entry.instance == instance {
            return;
        }
        entry.instance = instance.clone();
        let (Some(directory), Some(instance)) = (&self.directory, instance) else {
            return;
        };
        let path = file(directory, &instance);
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let read = match serde_json::from_slice::<File>(&bytes) {
            Ok(read) if read.schema == SCHEMA => read,
            Ok(_) | Err(_) => {
                self.told.push(format!(
                    "triggers: {} is not readable: ignored",
                    path.display()
                ));
                return;
            }
        };
        let entry = self.plugins.get_mut(plugin).expect("inserted above");
        entry.package = read.package;
        entry.unnamed = read.unnamed;
        entry.written = Some(bytes);
        for saved in read.triggers {
            let (Ok(id), Ok(condition)) = (
                TriggerId::parse(&saved.trigger),
                Condition::parse_coded(&saved.when),
            ) else {
                continue;
            };
            if condition.position().is_some() {
                continue;
            }
            self.seq = self.seq.max(saved.seq);
            entry.kept.insert(
                id,
                Kept {
                    wire: saved.when,
                    groups: condition.groups(),
                    condition,
                    seq: saved.seq,
                    bound: None,
                    entered: None,
                    given_by: 0,
                    last_position: None,
                    retry: false,
                },
            );
        }
    }
    /// A process of `plugin` was spawned from `package` (before it sends
    /// anything): when its conditions were given by another version, the
    /// unnamed ones go (§4.2: an unnamed trigger is a place in other code);
    /// named ones stay until its declaration says otherwise.
    pub fn spawned(&mut self, plugin: &str, package: Option<Package>) {
        let Some(entry) = self.plugins.get_mut(plugin) else {
            return;
        };
        if entry.package == package {
            return;
        }
        let before = entry.kept.len();
        entry.kept.retain(|id, _| matches!(id, TriggerId::Named(_)));
        if entry.kept.len() < before {
            self.told.push(format!(
                "triggers: {plugin} is another version: {} unnamed condition(s) removed",
                before - entry.kept.len()
            ));
        }
        entry.package = package;
        entry.unnamed = None;
        self.save(plugin);
    }
    /// A development load of `plugin` was reloaded (its code may have
    /// changed, §4.2): as another version.
    pub fn reloaded(&mut self, plugin: &str) {
        if let Some(entry) = self.plugins.get_mut(plugin) {
            entry.package = Some(Package {
                digest: String::new(),
                version: None,
            });
        }
        self.spawned(plugin, None);
    }
    /// `plugin`'s process ended by a fault (T13): "entered" is forgotten
    /// and what holds now enters again.
    pub fn fault(&mut self, plugin: &str, now: f64) -> Vec<Notification> {
        self.moment.time = now;
        let Some(entry) = self.plugins.get_mut(plugin) else {
            return Vec::new();
        };
        for kept in entry.kept.values_mut() {
            kept.entered = None;
            kept.last_position = None;
        }
        let ids: Vec<_> = entry.kept.keys().cloned().collect();
        let mut out = Vec::new();
        for id in ids {
            self.evaluate_one(plugin, &id, None, &mut out);
        }
        out
    }
    /// The user disabled or uninstalled `plugin`: everything goes, its file
    /// too.
    pub fn clear(&mut self, plugin: &str) {
        let Some(entry) = self.plugins.remove(plugin) else {
            return;
        };
        if let (Some(directory), Some(instance)) = (&self.directory, &entry.instance) {
            let _ = std::fs::remove_file(file(directory, instance));
        }
    }
    /// Start-up: what is kept is evaluated once (T13), when the start-up
    /// mute ends.
    pub fn evaluate_all(&mut self, now: f64) -> Vec<Notification> {
        self.moment.time = now;
        self.time_checked = now;
        let mut out = Vec::new();
        let ids: Vec<(String, TriggerId)> = self
            .plugins
            .iter()
            .flat_map(|(p, e)| e.kept.keys().map(move |id| (p.clone(), id.clone())))
            .collect();
        for (plugin, id) in ids {
            self.evaluate_one(&plugin, &id, None, &mut out);
        }
        out
    }

    // ---- what the player does ----------------------------------------

    /// The facts changed (`source.state`, `window.state`,
    /// `appearance.changed`, the subtitle track): the source's id and
    /// position come with them (`position` is the state's own, ≤ 100 ms
    /// old: used for what is evaluated now, never a reason to evaluate).
    /// Only the conditions that read a changed group are evaluated.
    pub fn set_facts(
        &mut self,
        facts: Facts,
        source: u64,
        position: Option<f64>,
        now: f64,
    ) -> Vec<Notification> {
        let mut changed = changed_groups(&self.facts, &facts);
        if source != self.moment.source {
            changed |= groups::SOURCE;
        }
        // The state's position is up to one push (100 ms of playback) old:
        // just behind the newest report of the position model for this
        // source it is stale, not a step back (a real step back, a seek, is
        // the model's own jump report). Taken as it is, a source fact that
        // changed in the same push would evaluate a range entered at the
        // report as left, and the model's next report enter it again (T1).
        let stale = |p: f64| {
            self.reported.is_some_and(|(s, r)| {
                s == source && p < r && r - p <= STALE_POSITION * facts.rate.max(1.)
            })
        };
        let position = match position {
            Some(p) if stale(p) => self.reported.map(|(_, r)| r),
            other => other,
        };
        if source != self.moment.source {
            self.reported = None;
        }
        self.facts = facts;
        self.moment.source = source;
        self.moment.position = position;
        self.moment.time = now;
        if changed == 0 {
            return Vec::new();
        }
        self.evaluate(changed, None)
    }
    /// The position model reported (`playback/position_model.rs`):
    /// `position` of `source` now; `continuous`: playback passed every
    /// place between the last evaluation and here (T2).
    pub fn position(
        &mut self,
        source: u64,
        position: f64,
        continuous: bool,
        now: f64,
    ) -> Vec<Notification> {
        if source != self.moment.source {
            // A report of a source the facts have not seen yet: the next
            // `source.state` evaluates it.
            return Vec::new();
        }
        self.moment.position = Some(position);
        self.reported = Some((source, position));
        self.moment.time = now;
        self.evaluate(groups::POSITION, continuous.then_some(position))
    }
    /// The clock is `now` (Unix ms): the time conditions whose bound it
    /// reached are evaluated. The nearest bound ahead (`next_time`) says
    /// when to call it again.
    pub fn time(&mut self, now: f64) -> Vec<Notification> {
        let due = self.next_time(self.time_checked).is_some_and(|due| due <= now)
            // The clock went back (a changed system time).
            || now < self.time_checked;
        self.moment.time = now;
        self.time_checked = now;
        if !due {
            return Vec::new();
        }
        self.evaluate(groups::TIME, None)
    }
    /// The nearest bound of any time condition after `now`.
    pub fn next_time(&self, now: f64) -> Option<f64> {
        self.plugins
            .values()
            .flat_map(|p| p.kept.values())
            .filter(|k| k.groups & groups::TIME != 0)
            .filter_map(|k| k.condition.next_time(now))
            .min_by(f64::total_cmp)
    }
    /// What the position model watches for: the current source and the
    /// next boundary of each range list bound to it, after the position it
    /// was last evaluated at (a boundary the position passed since is
    /// reported at once, so none is skipped); `(0, [])`: nothing.
    pub fn watch(&self) -> (u64, Vec<f64>) {
        let source = self.moment.source;
        let mut any = false;
        let mut boundaries: Vec<f64> = Vec::new();
        for kept in self.plugins.values().flat_map(|p| p.kept.values()) {
            let Some(list) = kept.condition.position() else {
                continue;
            };
            if kept.bound != Some(source) || source == 0 {
                continue;
            }
            any = true;
            let from = kept.last_position.unwrap_or(f64::NEG_INFINITY);
            boundaries.extend(list.next_boundary(from));
        }
        if !any {
            return (0, Vec::new());
        }
        boundaries.sort_by(f64::total_cmp);
        boundaries.dedup();
        (source, boundaries)
    }

    // ---- evaluation ---------------------------------------------------

    /// Evaluates every condition that reads one of `mask`; `passed`: a
    /// continuous position report reached this position (T2).
    fn evaluate(&mut self, mask: u8, passed: Option<f64>) -> Vec<Notification> {
        let mut out = Vec::new();
        let (facts, moment) = (&self.facts, self.moment);
        for (plugin, entry) in &mut self.plugins {
            for (id, kept) in &mut entry.kept {
                if kept.groups & mask != 0 {
                    evaluate_kept(facts, moment, plugin, id, kept, passed, &mut out);
                }
            }
        }
        out
    }
    fn evaluate_one(
        &mut self,
        plugin: &str,
        id: &TriggerId,
        passed: Option<f64>,
        out: &mut Vec<Notification>,
    ) {
        let (facts, moment) = (&self.facts, self.moment);
        if let Some(kept) = self
            .plugins
            .get_mut(plugin)
            .and_then(|p| p.kept.get_mut(id))
        {
            evaluate_kept(facts, moment, plugin, id, kept, passed, out);
        }
    }

    // ---- the file ------------------------------------------------------

    /// Writes what `plugin`'s file holds when it changed: its conditions
    /// without a range list. Empty, the file goes.
    fn save(&mut self, plugin: &str) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        let Some(entry) = self.plugins.get_mut(plugin) else {
            return;
        };
        let Some(instance) = entry.instance.clone() else {
            return;
        };
        let triggers: Vec<Saved> = entry
            .kept
            .iter()
            .filter(|(_, k)| k.condition.position().is_none())
            .map(|(id, k)| Saved {
                trigger: id.wire(),
                when: k.wire.clone(),
                seq: k.seq,
            })
            .collect();
        let path = file(&directory, &instance);
        if triggers.is_empty() && entry.unnamed.is_none() {
            if entry.written.take().is_some() {
                let _ = std::fs::remove_file(&path);
            }
            return;
        }
        let bytes = serde_json::to_vec(&File {
            schema: SCHEMA,
            package: entry.package.clone(),
            unnamed: entry.unnamed,
            triggers,
        })
        .expect("serializable");
        if entry.written.as_ref() == Some(&bytes) {
            return;
        }
        match write(&path, &bytes) {
            Ok(()) => entry.written = Some(bytes),
            Err(error) => self.told.push(format!(
                "triggers: {} could not be written: {error}",
                path.display()
            )),
        }
    }
}

/// Evaluates one kept condition against `facts` and `moment` and adds
/// what flipped to `out` (T1, T2, T10).
fn evaluate_kept(
    facts: &Facts,
    moment: Moment,
    plugin: &str,
    id: &TriggerId,
    kept: &mut Kept,
    passed: Option<f64>,
    out: &mut Vec<Notification>,
) {
    kept.retry = false;
    let list = kept.condition.position().cloned();
    let bound_here = kept.bound.is_some() && kept.bound == Some(moment.source);
    let mut push = |kept: &mut Kept, truth: Truth, position: Option<f64>| {
        for (edge, range) in step(&mut kept.entered, truth) {
            out.push(Notification {
                plugin: plugin.to_owned(),
                trigger: id.clone(),
                seq: kept.seq,
                edge,
                range: range.and_then(|r| list.as_ref().map(|l| l.range(r))),
                position,
            });
        }
    };
    // Continuous playback passed every boundary between the last
    // evaluation and here: each is evaluated in its order, so a range
    // shorter than a check step is entered and left once (T2).
    if let (Some(to), Some(list), true) = (passed, &list, bound_here)
        && let Some(from) = kept.last_position.filter(|from| *from <= to)
    {
        for x in list.boundaries_between(from, to) {
            if x >= to {
                break;
            }
            let at = Moment {
                position: Some(x),
                ..moment
            };
            let truth = kept.condition.eval(facts, Some(&at), kept.bound);
            push(kept, truth, Some(x));
        }
    }
    let truth = kept.condition.eval(facts, Some(&moment), kept.bound);
    push(kept, truth, passed);
    kept.last_position = if bound_here { moment.position } else { None };
}

/// One evaluation's flips (T1, T10): what was entered and what holds now.
fn step(entered: &mut Option<Option<usize>>, truth: Truth) -> Vec<(Edge, Option<usize>)> {
    match (*entered, truth.holds) {
        (None, true) => {
            *entered = Some(truth.range);
            vec![(Edge::Enter, truth.range)]
        }
        (Some(range), false) => {
            *entered = None;
            vec![(Edge::Leave, range)]
        }
        (Some(range), true) if range != truth.range => {
            *entered = Some(truth.range);
            vec![(Edge::Leave, range), (Edge::Enter, truth.range)]
        }
        _ => Vec::new(),
    }
}

/// The groups whose facts differ.
fn changed_groups(a: &Facts, b: &Facts) -> u8 {
    let mut changed = 0;
    if a.phase != b.phase
        || a.paused != b.paused
        || a.kind != b.kind
        || a.can != b.can
        || a.duration != b.duration
        || a.volume != b.volume
        || a.rate != b.rate
        || a.muted != b.muted
        || a.audio_track != b.audio_track
        || a.audio_device != b.audio_device
        || a.looping != b.looping
    {
        changed |= groups::SOURCE;
    }
    if a.fullscreen != b.fullscreen || a.maximized != b.maximized || a.pip != b.pip {
        changed |= groups::WINDOW;
    }
    if a.theme != b.theme || a.language != b.language {
        changed |= groups::APPEARANCE;
    }
    if a.subtitle_track != b.subtitle_track {
        changed |= groups::SUBTITLES;
    }
    changed
}

fn file(directory: &Path, instance: &str) -> PathBuf {
    directory.join(format!("{instance}.json"))
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
#[path = "triggers_sim.rs"]
mod sim;
#[cfg(test)]
#[path = "triggers_tests.rs"]
mod tests;
