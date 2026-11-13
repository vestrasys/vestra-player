//! The mpv pass-through on the control worker (`mpv.command`, `mpv.get`,
//! `mpv.set`, `mpv.observe`, `mpv.unobserve`; capability `mpv`, SDK 1.8.0;
//! doc/architecture/README.md §4.9). The capability (capabilities/mpv.rs)
//! checked each request's shape and the closed refusal rules
//! (`mpv_rules.rs`) and added the caller's manifest id (`plugin`); here is
//! what needs mpv and the other plugins:
//!
//! - **Commands** go to mpv as a node array (`mpv_command_node`), never
//!   through its string parser: no prefixes, no property expansion.
//! - **Filters.** A plugin adds, replaces, toggles or removes only filters
//!   with its own label (`vf`/`af` `add`, `pre`, `toggle`, `remove` of
//!   `@label…`). A label belongs, per stage, to the plugin that first put
//!   it in the chain (`conflict` for another). The chain is read before
//!   and after the command: every entry that came, went or changed must
//!   carry that label, otherwise the chain is put back and the command is
//!   `refused`. So nobody parses a lavfi text for a second filter hidden
//!   behind a comma, and the player's own filters (`@vestra-*`) cannot be
//!   touched by any spelling.
//! - **Where a plugin's filter sits.** Where mpv puts it: `add` appends,
//!   `pre` prepends, a label already in the chain is replaced in place.
//!   The player's own filters keep their places by themselves: the
//!   upscaler and motion smoothing move back to the end of the video chain
//!   at their next look (`Upscaling::keep_last`, `Motion::keep_last`), so
//!   they always get the picture last, at its final size and rate; the
//!   typed nodes' graph (`@vestra-owned`) stays where it was first added.
//!   Only `pre` puts a filter before the player's orientation filter: it
//!   then gets the frames as decoded, not yet turned upright.
//! - **Observers.** One mpv observation per property, shared by the
//!   plugins that observe it. An observer gets the current value at once
//!   (`null` when the property has none), then each change, at most every
//!   100 ms per property (the newest value wins; a value equal to the one
//!   it got last is not sent again). The event `mpv.property` goes to the
//!   observer only. At most 64 properties per plugin.
//! - **Property writes stay.** They are the player's settings from then
//!   on; nothing is rolled back when the plugin stops.
//! - **A stopped plugin** loses its labelled filters (removed from the
//!   chains) and its observers. One that holds either is never put in
//!   standby (`holds`).
//!
//! **The worker's other work pays nothing for it.** Until a plugin holds a
//! label or an observer this is two empty maps and two `false` flags: the
//! worker's loop asks `watching()` before it looks at an mpv event for an
//! observation, and `waiting()` before it reads the clock for a push or
//! shortens its wait; neither allocates or calls mpv. Requests are handled
//! only for the `mpv.` methods; mpv is asked for a chain only inside a
//! plugin's own filter command.
use crate::{host::RpcError, playback::mpv_rules};
use rsmpv::{Mpv, Node, PropertyData};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// `Event::PropertyChange` userdata of the pass-through's observations
/// start here (the worker's own are 1 and 2).
pub const OBSERVATION_BASE: u64 = 1000;
/// The event an observer gets: `{ name, value }`.
pub const EVENT: &str = "mpv.property";
/// What `owner.resources` lists for a plugin that holds something here.
pub const HELD: &str = "an mpv filter or observer";
const INTERVAL: Duration = Duration::from_millis(mpv_rules::OBSERVE_INTERVAL_MS);

/// One plugin's view of an observed property.
#[derive(Default)]
struct Observer {
    /// The value it got last, and when.
    sent: Option<(Value, Instant)>,
    /// A newer value, waiting for the interval.
    pending: Option<Value>,
}

/// mpv's observation of one property and the plugins it is for.
struct Watched {
    id: u64,
    observers: BTreeMap<u64, Observer>,
}

/// The plugin a filter label belongs to.
struct Holder {
    owner: u64,
    /// Its manifest id, for the `conflict` message.
    plugin: String,
}

/// The pass-through's state: who owns which filter label, who observes what.
#[derive(Default)]
pub struct Passthrough {
    /// (stage, label) → the plugin that put it in the chain.
    labels: BTreeMap<(&'static str, String), Holder>,
    /// Property name → its observation.
    watched: BTreeMap<String, Watched>,
    /// Observation ids given out so far.
    observations: u64,
    /// A value waits for its interval (`changed` to `due`).
    waiting: bool,
}

fn backend(error: rsmpv::Error) -> RpcError {
    // mpv's words without the wrapper's error number: "property not found".
    let text = error.to_string();
    let words = text
        .rsplit_once(" (")
        .map_or(text.as_str(), |(words, _)| words);
    RpcError::new("backend", format!("mpv: {words}"))
}

/// A JSON value as mpv's node: an integer as one (mpv takes an integer
/// where it wants a number, not the other way round).
pub fn to_node(value: &Value) -> Node {
    match value {
        Value::Null => Node::None,
        Value::Bool(flag) => Node::Flag(*flag),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => Node::Int64(integer),
            None => Node::Double(number.as_f64().unwrap_or(0.)),
        },
        Value::String(text) => Node::String(text.clone()),
        Value::Array(items) => Node::Array(items.iter().map(to_node).collect()),
        Value::Object(fields) => Node::Map(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), to_node(value)))
                .collect(),
        ),
    }
}

/// mpv's node as JSON; what JSON cannot hold (a number that is not finite,
/// raw bytes) is `null`.
pub fn from_node(node: &Node) -> Value {
    match node {
        Node::None | Node::ByteArray(_) => Value::Null,
        Node::Flag(flag) => (*flag).into(),
        Node::Int64(integer) => (*integer).into(),
        Node::Double(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Into::into)
        }
        Node::String(text) => text.clone().into(),
        Node::Array(items) => items.iter().map(from_node).collect(),
        Node::Map(fields) => fields
            .iter()
            .map(|(key, value)| (key.clone(), from_node(value)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
    }
}

fn from_data(data: &PropertyData) -> Value {
    match data {
        PropertyData::String(text) | PropertyData::OsdString(text) => text.clone().into(),
        PropertyData::Flag(flag) => (*flag).into(),
        PropertyData::Int64(integer) => (*integer).into(),
        PropertyData::Double(number) => from_node(&Node::Double(*number)),
        PropertyData::Node(node) => from_node(node),
        _ => Value::Null,
    }
}

/// The filters of a chain (`vf`, `af`) as mpv lists them, each with its
/// map's keys in order (mpv gives a map's keys in no fixed order).
fn chain(mpv: &Mpv, stage: &str) -> Result<Vec<Node>, RpcError> {
    fn ordered(node: Node) -> Node {
        match node {
            Node::Array(items) => Node::Array(items.into_iter().map(ordered).collect()),
            Node::Map(mut fields) => {
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Node::Map(
                    fields
                        .into_iter()
                        .map(|(key, value)| (key, ordered(value)))
                        .collect(),
                )
            }
            other => other,
        }
    }
    match mpv.get_property::<Node>(stage).map_err(backend)? {
        Node::Array(filters) => Ok(filters.into_iter().map(ordered).collect()),
        _ => Ok(Vec::new()),
    }
}

/// A chain entry's label (none: the empty string).
fn label_of(filter: &Node) -> &str {
    match filter {
        Node::Map(fields) => fields
            .iter()
            .find(|(key, _)| key == "label")
            .and_then(|(_, value)| match value {
                Node::String(label) => Some(label.as_str()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => "",
    }
}

/// Whether every entry that is in one of the two chains only (it came,
/// went or changed) carries `label`.
fn only_label_changed(before: &[Node], after: &[Node], label: &str) -> bool {
    let mut rest = after.to_vec();
    let mut changed = Vec::new();
    for filter in before {
        match rest.iter().position(|other| other == filter) {
            Some(at) => {
                rest.remove(at);
            }
            None => changed.push(filter.clone()),
        }
    }
    changed
        .iter()
        .chain(&rest)
        .all(|filter| label_of(filter) == label)
}

impl Passthrough {
    /// Whether any plugin observes a property: only then can an mpv
    /// property change be one of the pass-through's.
    pub fn watching(&self) -> bool {
        !self.watched.is_empty()
    }

    /// Whether a value waits for its interval: only then is there
    /// something for [`Self::due`] and [`Self::due_in`].
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// Whether `owner` holds something only its process serves here: a
    /// labelled filter (it may steer it) or an observer.
    pub fn holds(&self, owner: u64) -> bool {
        self.labels.values().any(|holder| holder.owner == owner)
            || self
                .watched
                .values()
                .any(|watched| watched.observers.contains_key(&owner))
    }

    /// `owner` stopped (or was parked, which one that holds something here
    /// never is): its filters leave the chains, its observers go. Its
    /// property writes stay. Nothing is asked of mpv for a plugin that
    /// holds nothing.
    pub fn stopped(&mut self, mpv: &Mpv, owner: u64) {
        if !self.holds(owner) {
            return;
        }
        let labels = std::mem::take(&mut self.labels);
        for ((stage, label), holder) in labels {
            if holder.owner != owner {
                self.labels.insert((stage, label), holder);
                continue;
            }
            if let Err(error) = mpv.command(&[stage, "remove", &format!("@{label}")]) {
                diag!("mpv pass-through: removing {stage} @{label} of {owner}: {error}");
            }
        }
        let names = self.watched.keys().cloned().collect::<Vec<_>>();
        for name in names {
            self.unobserve(mpv, owner, &name);
        }
    }

    /// One request of `owner`'s (its shape and the rules checked, its
    /// manifest id in `params.plugin`): the answer, and for `mpv.observe`
    /// the first value to push after it.
    pub fn request(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        method: &str,
        params: &Value,
        now: Instant,
    ) -> (Result<Value, RpcError>, Option<Value>) {
        let name = params["name"].as_str().unwrap_or_default();
        match method {
            "mpv.command" => {
                let plugin = params["plugin"].as_str().unwrap_or_default();
                (self.command(mpv, owner, plugin, &params["args"]), None)
            }
            "mpv.get" => (
                mpv.get_property::<Node>(name)
                    .map(|node| json!({ "value": from_node(&node) }))
                    .map_err(backend),
                None,
            ),
            "mpv.set" => (
                mpv.set_property(name, to_node(&params["value"]))
                    .map(|()| Value::Null)
                    .map_err(backend),
                None,
            ),
            "mpv.observe" => match self.observe(mpv, owner, name, now) {
                Ok(value) => (Ok(Value::Null), Some(json!({"name": name, "value": value}))),
                Err(error) => (Err(error), None),
            },
            "mpv.unobserve" => {
                self.unobserve(mpv, owner, name);
                (Ok(Value::Null), None)
            }
            _ => (Err(RpcError::new("unsupported", "unknown method")), None),
        }
    }

    /// The refusal of a label another plugin holds.
    fn taken(&self, owner: u64, stage: &'static str, label: &str) -> Option<RpcError> {
        self.labels
            .get(&(stage, label.to_owned()))
            .filter(|holder| holder.owner != owner)
            .map(|holder| {
                RpcError::new(
                    "conflict",
                    format!("the filter label @{label} belongs to {}", holder.plugin),
                )
            })
    }

    fn command(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        plugin: &str,
        args: &Value,
    ) -> Result<Value, RpcError> {
        let args = args
            .as_array()
            .filter(|args| !args.is_empty())
            .ok_or_else(|| RpcError::invalid("a command is an array, its name first"))?;
        let node = Node::Array(args.iter().map(to_node).collect());
        let run = |mpv: &Mpv| {
            mpv.command_node(&node)
                .map(|result| json!({ "result": from_node(&result) }))
                .map_err(backend)
        };
        if let Some((stage, label)) = mpv_rules::filter_message(args) {
            if let Some(error) = self.taken(owner, stage, &label) {
                return Err(error);
            }
            return run(mpv);
        }
        let Some(change) = mpv_rules::filter_change(args) else {
            return run(mpv);
        };
        if let Some(error) = self.taken(owner, change.stage, change.label) {
            return Err(error);
        }
        let before = chain(mpv, change.stage)?;
        let result = run(mpv);
        let after = chain(mpv, change.stage)?;
        if !only_label_changed(&before, &after, change.label) {
            // More than its one labelled filter changed (a second filter in
            // the value): as it was.
            if let Err(error) = mpv.set_property(change.stage, Node::Array(before)) {
                diag!(
                    "mpv pass-through: restoring {} after a refused change: {error}",
                    change.stage
                );
            }
            return Err(RpcError::new(
                "refused",
                mpv_rules::filters_refusal(change.stage),
            ));
        }
        let key = (change.stage, change.label.to_owned());
        if after.iter().any(|filter| label_of(filter) == change.label) {
            // Its first holder keeps it (a replacement in place is its own).
            self.labels.entry(key).or_insert_with(|| Holder {
                owner,
                plugin: if plugin.is_empty() {
                    owner.to_string()
                } else {
                    plugin.to_owned()
                },
            });
        } else {
            self.labels.remove(&key);
        }
        result
    }

    /// `owner` observes `name` from now on: its current value (`null`
    /// without one), which is the first it gets.
    fn observe(
        &mut self,
        mpv: &Mpv,
        owner: u64,
        name: &str,
        now: Instant,
    ) -> Result<Value, RpcError> {
        let observing = |watched: &Watched| watched.observers.contains_key(&owner);
        let observed = self.watched.values().filter(|w| observing(w)).count();
        if !self.watched.get(name).is_some_and(observing) && observed >= mpv_rules::OBSERVED_MAX {
            return Err(RpcError::new(
                "limit",
                format!(
                    "a plugin observes at most {} mpv properties",
                    mpv_rules::OBSERVED_MAX
                ),
            ));
        }
        if !self.watched.contains_key(name) {
            let id = OBSERVATION_BASE + self.observations;
            mpv.observe_property(id, name, rsmpv::Format::Node)
                .map_err(backend)?;
            self.observations += 1;
            self.watched.insert(
                name.to_owned(),
                Watched {
                    id,
                    observers: BTreeMap::new(),
                },
            );
        }
        let value = mpv
            .get_property::<Node>(name)
            .map_or(Value::Null, |node| from_node(&node));
        if let Some(watched) = self.watched.get_mut(name) {
            watched.observers.insert(
                owner,
                Observer {
                    sent: Some((value.clone(), now)),
                    pending: None,
                },
            );
        }
        Ok(value)
    }

    fn unobserve(&mut self, mpv: &Mpv, owner: u64, name: &str) {
        let Some(watched) = self.watched.get_mut(name) else {
            return;
        };
        watched.observers.remove(&owner);
        if watched.observers.is_empty() {
            let _ = mpv.unobserve_property(watched.id);
            self.watched.remove(name);
        }
    }

    /// An mpv property change with `userdata`: `true` when it is one of the
    /// pass-through's observations (its value waits for [`Self::due`]).
    pub fn changed(&mut self, userdata: u64, data: &PropertyData) -> bool {
        if userdata < OBSERVATION_BASE {
            return false;
        }
        if let Some(watched) = self.watched.values_mut().find(|w| w.id == userdata) {
            let value = from_data(data);
            for observer in watched.observers.values_mut() {
                observer.pending = Some(value.clone());
                self.waiting = true;
            }
        }
        true
    }

    /// The values to push now, as (owner, `{ name, value }`): each
    /// observer's newest value of a property once the interval since its
    /// last one is over; one equal to its last is dropped.
    pub fn due(&mut self, now: Instant) -> Vec<(u64, Value)> {
        let mut pushes = Vec::new();
        let mut waiting = false;
        for (name, watched) in &mut self.watched {
            for (owner, observer) in &mut watched.observers {
                if observer.pending.is_none() {
                    continue;
                }
                if observer
                    .sent
                    .as_ref()
                    .is_some_and(|(_, at)| now.saturating_duration_since(*at) < INTERVAL)
                {
                    waiting = true;
                    continue;
                }
                let Some(value) = observer.pending.take() else {
                    continue;
                };
                if observer
                    .sent
                    .as_ref()
                    .is_some_and(|(sent, _)| *sent == value)
                {
                    continue;
                }
                pushes.push((*owner, json!({"name": name, "value": value})));
                observer.sent = Some((value, now));
            }
        }
        self.waiting = waiting;
        pushes
    }

    /// When the next waiting value is due (none waiting: `None`).
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        if !self.waiting {
            return None;
        }
        self.watched
            .values()
            .flat_map(|watched| watched.observers.values())
            .filter(|observer| observer.pending.is_some())
            .map(|observer| {
                observer.sent.as_ref().map_or(Duration::ZERO, |(_, at)| {
                    INTERVAL.saturating_sub(now.saturating_duration_since(*at))
                })
            })
            .min()
    }
}

#[cfg(test)]
mod tests;
