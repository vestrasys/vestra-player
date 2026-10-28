//! `events`: the player's events pushed to a plugin as they happen.
use super::{Capability, Handled, Info, Method, Needs, Residency};
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
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        let event = string(params, "event")?;
        if !EVENTS.contains(&event) {
            return Err(RpcError::new("unsupported", "unknown event"));
        }
        let plugin = host.plugins.get_mut(&reply.owner).unwrap();
        if method == "events.subscribe" {
            plugin.subscriptions.insert(event.into());
        } else {
            plugin.subscriptions.remove(event);
        }
        Ok(Some(Value::Null))
    }
}
