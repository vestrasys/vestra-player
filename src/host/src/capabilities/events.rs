//! `events`: the player's events pushed to a plugin as they happen.
use super::{Call, Capability, Handled, Info, Recorded};
use crate::host::{Host, Origin, Reply, RpcError, string};
use serde_json::{Value, json};

pub struct Events;
static INFO: Info = Info {
    name: "events",
    builtin_only: false,
};
static METHODS: &[&str] = &["events.subscribe", "events.unsubscribe"];
const EVENTS: &[&str] = &[
    "playback.backend",
    "source.state",
    "window.state",
    "packages.changed",
    "bindings.changed",
    "commands.keys",
    "timeline.changed",
    "appearance.changed",
    "resolvers.fileTypes",
    "video.upscaling.changed",
    "video.motion.changed",
    "source.opening",
    "source.ready",
    "source.ended",
];
/// Events a parked plugin is woken for (lazy activation §6.9): they follow
/// a user's or a plugin's deliberate change (an install, a key edit, the
/// palette, a resolver), never playback, so a listener of one does not keep
/// its plugin running; the host records it and wakes the plugin to deliver
/// the event. `source.state` (every 100 ms while playing), `window.state`
/// (every resize step, menu and focus change), `playback.backend` (every
/// mpv event) and `timeline.changed` (every source and mark) keep it.
/// The discrete source events wake too (events-and-triggers.md §4.6): each
/// is sent once per thing that happened to a source.
pub(crate) const WAKE_EVENTS: &[&str] = &[
    "packages.changed",
    "bindings.changed",
    // Sent to one plugin only, when its own keys changed
    // (`Host::push_command_keys`).
    "commands.keys",
    "appearance.changed",
    "resolvers.fileTypes",
    "source.opening",
    "source.ready",
    "source.ended",
];
/// The wake events that report one change of one source
/// (`crate::source_events`), not a state: the backlog merges them per
/// source (`wake_trigger`), and the start-up mute holds them for its end
/// instead of comparing states (`OnDemand::announced` is for the others).
pub(crate) const SOURCE_EVENTS: &[&str] = &[
    crate::source_events::OPENING,
    crate::source_events::READY,
    crate::source_events::ENDED,
];

/// The backlog trigger for wake event `event` with `data`: merged with a
/// held one of the same key (newest wins). The key is the event's name;
/// for a source event its name and source id, so two sources in a row
/// keep one each (design §4.9).
pub(crate) fn wake_trigger(event: &str, data: &Value) -> crate::supervisor::Trigger<Value> {
    let message = json!({"event":event,"data":data});
    if SOURCE_EVENTS.contains(&event) {
        let key = format!("{event} {}", crate::source_events::source_id(data));
        return crate::supervisor::Trigger::event(key, message);
    }
    crate::supervisor::Trigger::event(event, message)
}
/// The longest reason kept (the SDK's description of a listener), and how
/// many per event.
const WHY_LIMIT: usize = 300;
const WHYS_LIMIT: usize = 16;

/// `events.subscribe { why }`: the reasons, a list of strings (or one).
fn reasons(why: &Value) -> Vec<String> {
    let text = |value: &Value| {
        value
            .as_str()
            .map(|why| why.chars().take(WHY_LIMIT).collect())
    };
    match why {
        Value::Array(items) => items.iter().filter_map(text).take(WHYS_LIMIT).collect(),
        other => text(other).into_iter().collect(),
    }
}

/// `owner`'s listeners of wake events as recorded calls (its other
/// listeners left out).
pub(super) fn wake_calls(host: &Host, owner: u64) -> Vec<Call> {
    let Some(plugin) = host.plugins.get(&owner) else {
        return Vec::new();
    };
    plugin
        .subscriptions
        .iter()
        .filter(|(event, active)| **active && WAKE_EVENTS.contains(&event.as_str()))
        .map(|(event, _)| Call::new("events.subscribe", json!({ "event": event })))
        .collect()
}

impl Capability for Events {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// A listener is a receiver the host pushes to (§3.2); a mirror is not:
    /// the SDK reads the state again when the plugin starts (§3.1). A
    /// listener of a wake event is recorded instead (§6.9).
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        let Some(plugin) = host.plugins.get(&owner) else {
            return Recorded::Nothing;
        };
        let listened = plugin
            .subscriptions
            .iter()
            .filter(|(_, active)| **active)
            .map(|(event, _)| event.as_str())
            .collect::<Vec<_>>();
        if listened.iter().any(|event| !WAKE_EVENTS.contains(event)) {
            return Recorded::Live("an event listener");
        }
        if listened.is_empty() {
            return Recorded::Nothing;
        }
        Recorded::Calls(wake_calls(host, owner))
    }
    /// The woken plugin subscribes again itself.
    fn rekey(&self, host: &mut Host, from: u64, _: u64) {
        host.on_demand.wakes.remove(&from);
    }
    /// A parked plugin keeps its wake events; otherwise they go.
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if !keep_recorded {
            host.on_demand.wakes.remove(&owner);
            return;
        }
        let Some(plugin) = host.plugins.get(&owner) else {
            return;
        };
        let wakes = plugin
            .subscriptions
            .iter()
            .filter(|(event, active)| **active && WAKE_EVENTS.contains(&event.as_str()))
            .map(|(event, _)| event.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if !wakes.is_empty() {
            // What it last had of each, while start-up mutes them.
            host.note_parked_listener(owner, &wakes);
            host.on_demand.wakes.insert(owner, wakes);
        }
    }
    fn replay(&self, host: &mut Host, owner: u64, _: &Origin, call: &Call) -> Handled {
        let event = string(&call.params, "event")?;
        if call.method != "events.subscribe" || !WAKE_EVENTS.contains(&event) {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        host.on_demand
            .wakes
            .entry(owner)
            .or_default()
            .insert(event.to_owned());
        Ok(None)
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        let event = string(params, "event")?;
        if !EVENTS.contains(&event) {
            return Err(RpcError::new("unsupported", "unknown event"));
        }
        let plugin = host.plugins.get_mut(&reply.owner).unwrap();
        if method == "events.subscribe" {
            // Again for the same event replaces the mark: the SDK switches
            // between mirror and listener as listeners come and go.
            let mirror = params["mirror"] == true;
            plugin.subscriptions.insert(event.into(), !mirror);
            // SDK 1.5.40: everything that listens, and where the plugin
            // added it.
            let whys = reasons(&params["why"]);
            if mirror || whys.is_empty() {
                plugin.listeners.remove(event);
            } else {
                plugin.listeners.insert(event.into(), whys);
            }
        } else {
            plugin.subscriptions.remove(event);
            plugin.listeners.remove(event);
        }
        Ok(Some(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::TriggerKind;

    fn ready(id: u64) -> Value {
        json!({"state":{"stamp":{"generation":id},"phase":"ready"}})
    }

    /// Design §4.9: a per-source event merges by its name and source id,
    /// so two sources in a row keep one each; other wake events by name.
    #[test]
    fn source_events_merge_per_source_in_the_backlog() {
        let key = |trigger: &crate::supervisor::Trigger<Value>| match &trigger.kind {
            TriggerKind::Event(key) => key.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            key(&wake_trigger("source.ready", &ready(3))),
            "source.ready 3"
        );
        assert_eq!(
            key(&wake_trigger("packages.changed", &json!([]))),
            "packages.changed"
        );
        // The message is the event as the plugin receives it.
        let trigger = wake_trigger("source.ended", &ready(2));
        assert_eq!(trigger.payload["event"], "source.ended");
        assert_eq!(trigger.payload["data"], ready(2));
        for event in SOURCE_EVENTS {
            assert!(
                WAKE_EVENTS.contains(event) && EVENTS.contains(event),
                "{event}"
            );
        }
    }
}
