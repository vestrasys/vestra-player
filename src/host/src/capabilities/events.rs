//! `events`: the player's events pushed to a plugin as they happen.
use super::{Capability, Handled, Info, Method, Needs, Recorded, Residency};
use crate::host::{Host, Reply, RpcError, string};
use serde_json::Value;

pub struct Events;
static INFO: Info = Info {
    name: "events",
    label: "Receive playback changes as they happen",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
/// Plugin and binding changes have their own (or no) capability.
const BY_EVENT: Needs = Needs::Param {
    key: "event",
    cases: &[
        ("packages.changed", None),
        ("bindings.changed", Some("bindings.manage")),
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
    "timeline.changed",
    "appearance.changed",
    "resolvers.fileTypes",
];
impl Capability for Events {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// A listener is a receiver the host pushes to (§3.2); a mirror is not:
    /// the SDK reads the state again when the plugin starts (§3.1).
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        match host.plugins.get(&owner) {
            Some(plugin) if plugin.subscriptions.values().any(|active| *active) => {
                Recorded::Live("an event listener")
            }
            _ => Recorded::Nothing,
        }
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
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
        } else {
            plugin.subscriptions.remove(event);
        }
        Ok(Some(Value::Null))
    }
}
