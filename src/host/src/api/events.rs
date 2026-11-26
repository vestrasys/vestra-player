//! `events`: the player's events pushed to a plugin as they happen.
use super::{Call, Capability, Handled, Info, Recorded};
use crate::host::{Core, Origin, Reply, RpcError, string};
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
    "video.reconfigured",
    "audio.reconfigured",
    "audio.tracks.changed",
    "subtitles.changed",
    "window.changed",
    "layout.changed",
    "audio.devices.changed",
    "app.started",
];
/// Events a parked plugin is woken for (lazy activation §6.9): they follow
/// a user's or a plugin's deliberate change (an install, a key edit, the
/// palette, a resolver), never playback, so a listener of one does not keep
/// its plugin running; the host records it and wakes the plugin to deliver
/// the event. `source.state` (every 100 ms while playing), `window.state`
/// (every resize step, menu and focus change), `playback.backend` (every
/// mpv event) and `timeline.changed` (every source and mark) keep it.
/// The discrete source events wake too: each
/// is sent once per thing that happened to a source; and so do the facts
/// of the second batch ([`FACT_EVENTS`]), each sent once per change.
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
    "video.reconfigured",
    "audio.reconfigured",
    "audio.tracks.changed",
    "subtitles.changed",
    "window.changed",
    "layout.changed",
    "audio.devices.changed",
    "app.started",
];
/// The second batch of events (release B): a
/// fact that changed, sent once per change (`crate::fact_events`). For the
/// start-up mute they are states, compared as the other wake events are
/// (`OnDemand::announced`).
pub(crate) const FACT_EVENTS: &[&str] = &[
    crate::fact_events::VIDEO_RECONFIGURED,
    crate::fact_events::AUDIO_RECONFIGURED,
    crate::fact_events::AUDIO_TRACKS_CHANGED,
    crate::fact_events::SUBTITLES_CHANGED,
    crate::fact_events::WINDOW_CHANGED,
    crate::fact_events::LAYOUT_CHANGED,
    crate::fact_events::AUDIO_DEVICES_CHANGED,
    crate::fact_events::APP_STARTED,
];
/// A source event or a fact event to a running listener counts as use
/// (design §4.9): a handler runs, so a standby on its way is cancelled
/// rather than the process stopped under it.
pub(crate) fn counts_as_use(event: &str) -> bool {
    SOURCE_EVENTS.contains(&event) || FACT_EVENTS.contains(&event)
}
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
/// keep one each (design §4.9); so does a fact of one source
/// (`fact_events::source_of`).
pub(crate) fn wake_trigger(event: &str, data: &Value) -> crate::supervisor::Trigger<Value> {
    let message = json!({"event":event,"data":data});
    if SOURCE_EVENTS.contains(&event) {
        let key = format!("{event} {}", crate::source_events::source_id(data));
        return crate::supervisor::Trigger::event(key, message);
    }
    if let Some(source) = crate::fact_events::source_of(event, data) {
        return crate::supervisor::Trigger::event(format!("{event} {source}"), message);
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
pub(super) fn wake_calls(core: &Core, owner: u64) -> Vec<Call> {
    let Some(plugin) = core.plugins.get(&owner) else {
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
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        let Some(plugin) = core.plugins.get(&owner) else {
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
        Recorded::Calls(wake_calls(core, owner))
    }
    /// The woken plugin subscribes again itself.
    fn rekey(&self, core: &mut Core, from: u64, _: u64) {
        core.on_demand.wakes.remove(&from);
    }
    /// A parked plugin keeps its wake events; otherwise they go.
    fn revoke(&self, core: &mut Core, owner: u64, keep_recorded: bool) {
        if !keep_recorded {
            core.on_demand.wakes.remove(&owner);
            return;
        }
        let Some(plugin) = core.plugins.get(&owner) else {
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
            core.note_parked_listener(owner, &wakes);
            core.on_demand.wakes.insert(owner, wakes);
        }
    }
    fn replay(&self, core: &mut Core, owner: u64, _: &Origin, call: &Call) -> Handled {
        let event = string(&call.params, "event")?;
        if call.method != "events.subscribe" || !WAKE_EVENTS.contains(&event) {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        core.on_demand
            .wakes
            .entry(owner)
            .or_default()
            .insert(event.to_owned());
        Ok(None)
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        let event = string(params, "event")?;
        if !EVENTS.contains(&event) {
            return Err(RpcError::new("unsupported", "unknown event"));
        }
        let plugin = core.plugins.get_mut(&reply.owner).unwrap();
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

    /// Design §4.6, second batch: each fact event can be subscribed to and
    /// is a wake event (a listener of one keeps nothing running); a fact of
    /// one source merges per source in the backlog, the others by name;
    /// a notification of one to a running listener counts as use.
    #[test]
    fn fact_events_wake_and_merge_per_source() {
        let key = |trigger: &crate::supervisor::Trigger<Value>| match &trigger.kind {
            TriggerKind::Event(key) => key.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(FACT_EVENTS.len(), 8);
        for event in FACT_EVENTS {
            assert!(
                WAKE_EVENTS.contains(event) && EVENTS.contains(event),
                "{event}"
            );
            assert!(counts_as_use(event), "{event}");
        }
        let video = json!({"sourceId":4,"width":1920,"height":1080});
        assert_eq!(
            key(&wake_trigger("video.reconfigured", &video)),
            "video.reconfigured 4"
        );
        let subtitles = json!({"state":{"source":{"generation":5}}});
        assert_eq!(
            key(&wake_trigger("subtitles.changed", &subtitles)),
            "subtitles.changed 5"
        );
        let window = json!({"fullscreen":true});
        assert_eq!(
            key(&wake_trigger("window.changed", &window)),
            "window.changed"
        );
        assert_eq!(
            key(&wake_trigger("app.started", &json!({"version":"1.0.0"}))),
            "app.started"
        );
        assert!(counts_as_use("source.ready"));
        assert!(!counts_as_use("source.state"));
        assert!(!counts_as_use("packages.changed"));
    }
}
