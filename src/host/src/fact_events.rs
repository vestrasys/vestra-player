//! The second batch of events (release B):
//! facts that changed, each sent once per change, never as a stream. All
//! eight are wake events (`api::events::FACT_EVENTS`): a listener
//! of one keeps nothing running, and a plugin in standby is woken for one.
//!
//! Where each is decided, by the core thread's rules
//!: the playback thread reads mpv, never
//! the core (no I/O on the core); the interface's facts are taken when
//! they are settled (`UiReport::FactsSettled`), never in between; and each
//! event follows what it derives from, so a listener of both reads the
//! state first.
//!
//! | Event | Decided on | From |
//! | --- | --- | --- |
//! | `video.reconfigured` | the playback thread (`playback::media_facts`) | mpv's `video-reconfig`, after the source's state went out |
//! | `audio.reconfigured` | the playback thread (`playback::media_facts`) | mpv's `audio-reconfig`, the same |
//! | `subtitles.changed` | the playback thread (`playback::media_facts`) | the subtitle state at each `source.state` push |
//! | `audio.devices.changed` | the playback thread (`SourceBackend::follow_device`) | the device list, checked every second |
//! | `audio.tracks.changed` | the core ([`audio_tracks_changed`]) | two consecutive `source.state` broadcasts |
//! | `window.changed` | the core ([`WindowChange`]) | the window facts at `FactsSettled`; a resize waits for [`QUIET_MS`] without a size change |
//! | `layout.changed` | the core ([`LayoutChange`]) | `layout.state` at `FactsSettled`, an overlay's pop-out left out |
//! | `app.started` | the core ([`app_started`]) | the end of the start-up mute |
#![forbid(unsafe_code)]

use serde_json::{Value, json};

pub(crate) const VIDEO_RECONFIGURED: &str = "video.reconfigured";
pub(crate) const AUDIO_RECONFIGURED: &str = "audio.reconfigured";
pub(crate) const AUDIO_TRACKS_CHANGED: &str = "audio.tracks.changed";
pub(crate) const SUBTITLES_CHANGED: &str = "subtitles.changed";
pub(crate) const WINDOW_CHANGED: &str = "window.changed";
pub(crate) const LAYOUT_CHANGED: &str = "layout.changed";
pub(crate) const AUDIO_DEVICES_CHANGED: &str = "audio.devices.changed";
pub(crate) const APP_STARTED: &str = "app.started";

/// How long the window's size must stay unchanged before a resize is
/// reported (`window.changed` once at the end of a gesture, design §4.6).
/// The platforms' windowing gives no portable "resize ended" (only
/// Windows has `WM_EXITSIZEMOVE`, and a keyboard snap or a layout change
/// sends none), so the end is a quiet period, as browsers' resize
/// handlers debounce it.
pub(crate) const QUIET_MS: u64 = 300;

/// The source a fact of one source belongs to: the backlog merges those
/// per event name and source (design §4.9). `None` for the others.
pub(crate) fn source_of(event: &str, data: &Value) -> Option<u64> {
    match event {
        VIDEO_RECONFIGURED | AUDIO_RECONFIGURED | AUDIO_TRACKS_CHANGED => {
            Some(data["sourceId"].as_u64().unwrap_or(0))
        }
        SUBTITLES_CHANGED => Some(data["state"]["source"]["generation"].as_u64().unwrap_or(0)),
        _ => None,
    }
}

/// `audio.tracks.changed` after the `source.state` broadcast `next`
/// (`previous` the one before it): the source's audio track list, when it
/// is known and differs from the previous snapshot's list (another source,
/// a track added, the selection moved). Not when the list goes away (a
/// source closes, a new one loads): `source.ended` and `source.opening`
/// say that. Data: `{ sourceId, audioTracks }`, the tracks in the wire
/// shape (the SDK hands the listener `AudioTrack`s).
pub(crate) fn audio_tracks_changed(previous: &Value, next: &Value) -> Option<Value> {
    let tracks = &next["audioTracks"];
    if !tracks.is_array() || next["source"].is_null() {
        return None;
    }
    let id = |state: &Value| state["stamp"]["generation"].as_u64().unwrap_or(0);
    if id(previous) == id(next) && previous["audioTracks"] == *tracks {
        return None;
    }
    Some(json!({ "sourceId": id(next), "audioTracks": tracks }))
}

/// `window.changed`'s data from the window facts (`window.state`'s):
/// fullscreen, maximized, the video area and picture-in-picture, the
/// fields of the SDK's `WindowState`.
pub(crate) fn window_shape(facts: &Value) -> Value {
    let area = &facts["videoArea"];
    let pip = &facts["pictureInPicture"];
    json!({
        "fullscreen": facts["fullscreen"] == true,
        "maximized": facts["maximized"] == true && facts["fullscreen"] != true,
        "videoArea": {
            "width": area["width"].as_f64().unwrap_or(0.),
            "height": area["height"].as_f64().unwrap_or(0.),
        },
        "pictureInPicture": {
            "supported": pip["supported"] == true,
            "active": pip["active"] == true,
        },
    })
}

/// When `window.changed` goes out (core). A flip (fullscreen, maximized,
/// picture-in-picture) goes at once, with the size it has then; a change
/// of the size alone waits until the size has stayed the same for
/// [`QUIET_MS`], so a resize gesture sends one event at its end and not
/// one per step. The first facts of the run are the start, not a change.
#[derive(Debug, Default)]
pub(crate) struct WindowChange {
    /// What the last event said (or the run's first facts).
    sent: Option<Value>,
    /// The facts last seen, and when their size last changed.
    seen: Option<Value>,
    /// A size change waits until then (core clock, ms).
    due: Option<u64>,
}

impl WindowChange {
    /// The settled facts are `shape` (`window_shape`) at `now`: the
    /// event to send now, if any.
    pub(crate) fn facts(&mut self, shape: Value, now: u64) -> Option<Value> {
        let Some(sent) = &self.sent else {
            self.sent = Some(shape.clone());
            self.seen = Some(shape);
            return None;
        };
        if *sent == shape {
            self.due = None;
            self.seen = Some(shape);
            return None;
        }
        let flip = ["fullscreen", "maximized", "pictureInPicture"]
            .iter()
            .any(|field| sent[*field] != shape[*field]);
        if flip {
            return self.send(shape);
        }
        // The size alone: (re)start the quiet period at each new size.
        if self.seen.as_ref() != Some(&shape) || self.due.is_none() {
            self.due = Some(now + QUIET_MS);
        }
        self.seen = Some(shape);
        None
    }
    /// The core's periodic step at `now`: a size change whose quiet
    /// period ended goes out now.
    pub(crate) fn tick(&mut self, now: u64) -> Option<Value> {
        if self.due.is_none_or(|due| now < due) {
            return None;
        }
        let shape = self.seen.clone()?;
        self.send(shape)
    }
    /// Whether a size change waits (the core's step looks only then).
    pub(crate) fn waiting(&self) -> bool {
        self.due.is_some()
    }
    fn send(&mut self, shape: Value) -> Option<Value> {
        self.due = None;
        self.seen = Some(shape.clone());
        if self.sent.as_ref() == Some(&shape) {
            return None;
        }
        self.sent = Some(shape.clone());
        Some(shape)
    }
}

/// When `layout.changed` goes out (core): the panel positions as
/// `layout.state` reports them, at `FactsSettled`, when they differ from
/// the last ones sent (a slot shown or hidden, docked or detached, another
/// tab active, a panel added or removed). The first of the run is the
/// start, not a change. An overlay's pop-out is not a change
/// ([`overlay_settled`]): a fullscreen column revealed by the pointer at
/// the edge and hidden again, the fullscreen bottom bar shown and hidden
/// by the pointer. Data: `layout.state`'s reply as it is (the SDK hands
/// the listener `{ slots }`).
#[derive(Debug, Default)]
pub(crate) struct LayoutChange {
    /// What the last event's layout compares as (`overlay_settled`).
    sent: Option<Value>,
}

impl LayoutChange {
    pub(crate) fn layout(&mut self, layout: Value) -> Option<Value> {
        let first = self.sent.is_none();
        let settled = overlay_settled(&layout);
        if self.sent.as_ref() == Some(&settled) {
            return None;
        }
        self.sent = Some(settled);
        (!first).then_some(layout)
    }
}

/// `layout` as `layout.changed` compares it: a position hidden only as an
/// overlay (`reason` `fullscreen`: a column collapsed at the edge in
/// fullscreen; `idle`: the fullscreen bottom bar away from the pointer)
/// reads as shown, the state the pointer pops it out to. A pop-out and its
/// hiding are transient, like the steps of a resize for `window.changed`,
/// and as a wake event each would start the parked listeners.
pub(crate) fn overlay_settled(layout: &Value) -> Value {
    let mut settled = layout.clone();
    for name in ["bottom", "sidebar", "adjust"] {
        let slot = &mut settled[name];
        if matches!(slot["reason"].as_str(), Some("fullscreen" | "idle")) {
            slot["effective"] = json!(true);
            slot["reason"] = Value::Null;
        }
    }
    settled
}

/// `app.started`'s data: this run's player version and the last run's
/// (`null` on the first run of a profile).
pub(crate) fn app_started(version: &str, previous: Option<&str>) -> Value {
    json!({ "version": version, "previous": previous })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: u64, tracks: Value) -> Value {
        json!({
            "stamp": {"generation": id},
            "source": if id == 0 { Value::Null } else { json!("/m/a.mkv") },
            "audioTracks": tracks,
        })
    }
    fn track(id: i64, selected: bool) -> Value {
        json!({"trackId": id, "selected": selected, "external": false})
    }

    #[test]
    fn audio_tracks_go_out_once_per_list_of_a_source() {
        let one = json!([track(1, true), track(2, false)]);
        // Loading: no list yet. Ready: the list.
        assert!(audio_tracks_changed(&state(0, Value::Null), &state(1, Value::Null)).is_none());
        let first = audio_tracks_changed(&state(1, Value::Null), &state(1, one.clone())).unwrap();
        assert_eq!(first["sourceId"], 1);
        assert_eq!(first["audioTracks"], one);
        // The same list again (a position update): nothing.
        assert!(audio_tracks_changed(&state(1, one.clone()), &state(1, one.clone())).is_none());
        // The selection moved.
        let two = json!([track(1, false), track(2, true)]);
        assert!(audio_tracks_changed(&state(1, one.clone()), &state(1, two.clone())).is_some());
        // A track added.
        let three = json!([track(1, false), track(2, true), track(3, false)]);
        assert!(audio_tracks_changed(&state(1, two.clone()), &state(1, three)).is_some());
        // Another source with an equal list: its own event.
        let other = audio_tracks_changed(&state(1, one.clone()), &state(2, one.clone())).unwrap();
        assert_eq!(other["sourceId"], 2);
        // The list going away (closed, a new one loading): nothing.
        assert!(audio_tracks_changed(&state(1, one.clone()), &state(0, Value::Null)).is_none());
        assert!(audio_tracks_changed(&state(1, one), &state(2, Value::Null)).is_none());
    }

    fn window(fullscreen: bool, width: f64, height: f64) -> Value {
        window_shape(&json!({
            "fullscreen": fullscreen,
            "maximized": false,
            "videoArea": {"width": width, "height": height},
            "pictureInPicture": {"supported": false, "active": false},
            "slots": {"ignored": true},
        }))
    }

    #[test]
    fn the_window_shape_is_the_sdk_window_state() {
        let shape = window_shape(&json!({
            "fullscreen": true,
            "maximized": true,
            "videoArea": {"width": 800, "height": 450},
            "pictureInPicture": {"supported": true, "active": false},
            "appearance": "dark",
        }));
        assert_eq!(
            shape,
            json!({
                "fullscreen": true,
                "maximized": false,
                "videoArea": {"width": 800.0, "height": 450.0},
                "pictureInPicture": {"supported": true, "active": false},
            })
        );
    }

    /// A resize gesture: one event, `QUIET_MS` after its last step.
    #[test]
    fn a_resize_goes_out_once_at_its_end() {
        let mut change = WindowChange::default();
        // The run's first facts: the start, not a change.
        assert!(change.facts(window(false, 800., 450.), 0).is_none());
        assert!(change.tick(10_000).is_none());
        // Twenty steps of a drag, 16 ms apart: nothing yet.
        let mut now = 1000;
        for step in 1..=20 {
            let width = 800. + f64::from(step) * 10.;
            assert!(change.facts(window(false, width, 450.), now).is_none());
            assert!(change.tick(now).is_none(), "step {step}");
            now += 16;
        }
        let last = now - 16;
        // The pointer rests: the same size again, the quiet period runs on.
        assert!(change.facts(window(false, 1000., 450.), now).is_none());
        assert!(change.tick(last + QUIET_MS - 1).is_none());
        let sent = change.tick(last + QUIET_MS).unwrap();
        assert_eq!(sent["videoArea"]["width"], 1000.0);
        // Once only.
        assert!(change.tick(last + QUIET_MS + 100).is_none());
        assert!(!change.waiting());
        // A resize back to the size last sent sends nothing.
        assert!(change.facts(window(false, 900., 450.), 5000).is_none());
        assert!(change.facts(window(false, 1000., 450.), 5016).is_none());
        assert!(change.tick(6000).is_none());
    }

    /// Fullscreen, maximized, picture-in-picture: at once, with the size
    /// they have then; a size change still waiting goes with it.
    #[test]
    fn a_flip_goes_out_at_once() {
        let mut change = WindowChange::default();
        change.facts(window(false, 800., 450.), 0);
        assert!(change.facts(window(false, 820., 450.), 100).is_none());
        let flipped = change.facts(window(true, 1920., 1080.), 120).unwrap();
        assert_eq!(flipped["fullscreen"], true);
        assert_eq!(flipped["videoArea"]["width"], 1920.0);
        // The waiting size change went with it.
        assert!(change.tick(10_000).is_none());
        // Back out of fullscreen.
        let back = change.facts(window(false, 820., 450.), 11_000).unwrap();
        assert_eq!(back["fullscreen"], false);
        let mut pip = window(false, 820., 450.);
        pip["pictureInPicture"]["active"] = json!(true);
        assert!(change.facts(pip, 12_000).is_some());
    }

    #[test]
    fn layout_goes_out_when_it_changes_after_the_first() {
        let mut change = LayoutChange::default();
        let shown = json!({"sidebar": {"effective": true}});
        let hidden = json!({"sidebar": {"effective": false}});
        assert!(change.layout(shown.clone()).is_none());
        assert!(change.layout(shown.clone()).is_none());
        assert_eq!(change.layout(hidden.clone()), Some(hidden));
        assert_eq!(change.layout(shown.clone()), Some(shown));
    }

    /// Review of L1: a fullscreen column revealed by the pointer and hidden
    /// again, and the fullscreen bottom bar shown and hidden by the
    /// pointer, send nothing; turning a column off, or a panel coming,
    /// does.
    #[test]
    fn an_overlay_pop_out_is_not_a_layout_change() {
        let slot = |effective: bool, reason: Value| {
            json!({"owner": 3, "preferred": true, "effective": effective,
                "reason": reason, "detached": false, "panels": []})
        };
        let layout = |sidebar: Value, bottom: Value| json!({"sidebar": sidebar, "bottom": bottom, "adjust": null, "backdrop": null});
        let mut change = LayoutChange::default();
        // Windowed, both shown: the start.
        assert!(
            change
                .layout(layout(slot(true, Value::Null), slot(true, Value::Null)))
                .is_none()
        );
        // Fullscreen: the column collapsed at the edge, the bar idle.
        let collapsed = layout(slot(false, json!("fullscreen")), slot(false, json!("idle")));
        assert!(change.layout(collapsed.clone()).is_none());
        // Pointer at the edge: popped out; and in again. Pointer at the
        // bottom: the bar; and away.
        let popped = layout(slot(true, Value::Null), slot(false, json!("idle")));
        assert!(change.layout(popped).is_none());
        assert!(change.layout(collapsed.clone()).is_none());
        let bar = layout(slot(false, json!("fullscreen")), slot(true, Value::Null));
        assert!(change.layout(bar).is_none());
        assert!(change.layout(collapsed).is_none());
        // The user turns the column off: a change, sent as it is.
        let off = layout(slot(false, json!("userHidden")), slot(false, json!("idle")));
        assert_eq!(change.layout(off.clone()), Some(off));
        // A window too narrow for the column: a change.
        let narrow = layout(slot(false, json!("space")), slot(true, Value::Null));
        assert!(change.layout(narrow).is_some());
    }

    #[test]
    fn facts_of_one_source_name_it() {
        assert_eq!(
            source_of(AUDIO_TRACKS_CHANGED, &json!({"sourceId": 7})),
            Some(7)
        );
        assert_eq!(
            source_of(AUDIO_RECONFIGURED, &json!({"sourceId": 2})),
            Some(2)
        );
        assert_eq!(source_of(LAYOUT_CHANGED, &json!({})), None);
        assert_eq!(source_of(AUDIO_DEVICES_CHANGED, &json!({})), None);
        assert_eq!(
            app_started("0.11.0", Some("0.10.3")),
            json!({"version": "0.11.0", "previous": "0.10.3"})
        );
        assert_eq!(app_started("0.11.0", None)["previous"], Value::Null);
    }
}
