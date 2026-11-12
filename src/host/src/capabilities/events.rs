//! `events`: the player's events pushed to a plugin as they happen.
use super::{Call, Capability, Handled, Info, Method, Needs, Recorded, Residency};
use crate::host::{Host, Origin, Reply, RpcError, string};
use serde_json::{Value, json};

pub struct Events;
static INFO: Info = Info {
    name: "events",
    label: "Receive playback changes as they happen",
    risky: false,
    requires: &[],
    residency: Residency::Live,
    builtin_only: false,
};
/// Plugin and binding changes have their own (or no) capability.
const BY_EVENT: Needs = Needs::Param {
    key: "event",
    cases: &[
        ("packages.changed", None),
        ("bindings.changed", Some("bindings.manage")),
        // Player 0.7.6: the key labels of the plugin's own commands
        // (`commands.keys`), with the capability that declares them.
        ("commands.keys", Some("commands")),
        // Player 0.5.25: the upscaler's state as it changes (playback/upscaling.rs).
        ("video.upscaling.changed", Some("video.upscaling")),
        // Player 0.8.1: motion smoothing's state as it changes (playback/motion.rs).
        ("video.motion.changed", Some("video.motion")),
    ],
    otherwise: Some("events"),
};
static METHODS: &[Method] = &[
    Method {
        name: "events.subscribe",
        needs: BY_EVENT,
    },
    Method {
        name: "events.unsubscribe",
        needs: BY_EVENT,
    },
];
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
];
/// Events a parked plugin is woken for (lazy activation §6.9): they follow
/// a user's or a plugin's deliberate change (an install, a key edit, the
/// palette, a resolver), never playback, so a listener of one does not keep
/// its plugin running; the host records it and wakes the plugin to deliver
/// the event. `source.state` (every 100 ms while playing), `window.state`
/// (every resize step, menu and focus change), `playback.backend` (every
/// mpv event) and `timeline.changed` (every source and mark) keep it.
pub(crate) const WAKE_EVENTS: &[&str] = &[
    "packages.changed",
    "bindings.changed",
    // Sent to one plugin only, when its own keys changed
    // (`Host::push_command_keys`).
    "commands.keys",
    "appearance.changed",
    "resolvers.fileTypes",
];
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
    fn methods(&self) -> &'static [Method] {
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
