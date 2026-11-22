//! The playback thread's part of the second batch of events
//! (`crate::fact_events`; events-and-triggers.md §4.6): `video.reconfigured`,
//! `audio.reconfigured` and `subtitles.changed`. mpv is read here, on the
//! thread that owns it, never on the core.
//!
//! A reconfiguration (mpv's `video-reconfig`, `audio-reconfig`) only marks
//! the facts due; they are read and sent after the iteration's
//! `source.state` push of the same source went out (`backend.rs`), so a
//! listener never hears of a source's video before that source's
//! `source.opening`. mpv sends a reconfiguration more often than the
//! parameters change (a seek, a filter that keeps the size): an event goes
//! out only when the facts differ from the last ones sent for that source,
//! and always for a new source's first.
#![forbid(unsafe_code)]

use rsmpv::Mpv;
use serde_json::{Value, json};

/// What the playback thread keeps between iterations.
#[derive(Debug, Default)]
pub(crate) struct MediaFacts {
    video_due: bool,
    audio_due: bool,
    /// The last facts sent, with their source id.
    video: Option<(u64, Value)>,
    audio: Option<(u64, Value)>,
    /// The cheap key the subtitle state was last compared at, and the part
    /// of it the event last reported.
    subtitle_key: Option<Value>,
    subtitles: Option<Value>,
}

impl MediaFacts {
    /// An mpv event: a reconfiguration marks its facts due.
    pub(crate) fn event(&mut self, event: &rsmpv::Event) {
        match event {
            rsmpv::Event::VideoReconfig => self.video_due = true,
            rsmpv::Event::AudioReconfig => self.audio_due = true,
            _ => {}
        }
    }
    /// Whether a reconfiguration waits to be read.
    pub(crate) fn due(&self) -> bool {
        self.video_due || self.audio_due
    }
    /// The reconfiguration events of source `generation` due now, read
    /// from mpv, as (event name, data).
    pub(crate) fn reconfigured(
        &mut self,
        mpv: &Mpv,
        generation: u64,
    ) -> Vec<(&'static str, Value)> {
        let int = |name: &str| mpv.get_property::<i64>(name).ok();
        let float = |name: &str| mpv.get_property::<f64>(name).ok();
        let text = |name: &str| {
            mpv.get_property::<String>(name)
                .ok()
                .filter(|s| !s.is_empty())
        };
        let mut out = Vec::new();
        if std::mem::take(&mut self.video_due)
            && let Some(facts) = video(generation, &int, &float, &text)
            && let Some(data) = changed(&mut self.video, generation, facts)
        {
            out.push((crate::fact_events::VIDEO_RECONFIGURED, data));
        }
        if std::mem::take(&mut self.audio_due)
            && let Some(facts) = audio(generation, &int, &text)
            && let Some(data) = changed(&mut self.audio, generation, facts)
        {
            out.push((crate::fact_events::AUDIO_RECONFIGURED, data));
        }
        out
    }
    /// `subtitles.changed` at a `source.state` push: `key` is the subtitle
    /// state's cheap key (`SubtitleBackend::change_key`); only when it
    /// moved is the whole state read (`state`), and an event goes out when
    /// its tracks or its primary or secondary selection differ from the
    /// last one sent. Only for a loaded source (`ready` or `ended`): while
    /// one loads its list is not known yet. Data: `{ state }`, the
    /// `subtitles.state` reply.
    pub(crate) fn subtitles(&mut self, key: Value, state: impl FnOnce() -> Value) -> Option<Value> {
        if self.subtitle_key.as_ref() == Some(&key) {
            return None;
        }
        self.subtitle_key = Some(key);
        let state = state();
        if !matches!(state["source"]["phase"].as_str(), Some("ready" | "ended")) {
            return None;
        }
        let reported = subtitle_part(&state);
        if self.subtitles.as_ref() == Some(&reported) {
            return None;
        }
        self.subtitles = Some(reported);
        Some(json!({ "state": state }))
    }
}

/// The part of a subtitle state `subtitles.changed` reports a change of:
/// the source, the tracks, the primary and secondary selection.
fn subtitle_part(state: &Value) -> Value {
    json!({
        "source": state["source"]["generation"],
        "tracks": state["tracks"],
        "target": state["selection"]["target"],
        "secondary": state["selection"]["secondary"],
    })
}

/// `facts` for `generation`, when they differ from `last` (or `last` is of
/// another source); `last` is then them.
fn changed(last: &mut Option<(u64, Value)>, generation: u64, facts: Value) -> Option<Value> {
    if last
        .as_ref()
        .is_some_and(|(source, sent)| *source == generation && *sent == facts)
    {
        return None;
    }
    *last = Some((generation, facts.clone()));
    Some(facts)
}

/// `video.reconfigured`'s data, `None` without video (no decoded size):
/// `width`, `height` the decoded size (mpv `video-params/w`, `h`; what a
/// geometry editor's picture shape needs); `displayWidth`,
/// `displayHeight` the size shown, after aspect, rotation and the video
/// filters (`video-out-params/dw`, `dh`, else `video-params/dw`, `dh`;
/// what sizing the window to the video needs); `fps` the frame rate the
/// file declares (`container-fps`), `null` when it declares none;
/// `rotation` the rotation the file asks for in degrees
/// (`video-params/rotate`); `hdr` whether the transfer is PQ or HLG
/// (`video-params/gamma`).
fn video(
    generation: u64,
    int: &dyn Fn(&str) -> Option<i64>,
    float: &dyn Fn(&str) -> Option<f64>,
    text: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let positive = |name: &str| int(name).filter(|v| *v > 0);
    let width = positive("video-params/w")?;
    let height = positive("video-params/h")?;
    let display = |out: &str, params: &str| positive(out).or_else(|| positive(params)).unwrap_or(0);
    let fps = float("container-fps").filter(|fps| fps.is_finite() && *fps > 0.);
    let hdr = matches!(text("video-params/gamma").as_deref(), Some("pq" | "hlg"));
    Some(json!({
        "sourceId": generation,
        "width": width,
        "height": height,
        "displayWidth": display("video-out-params/dw", "video-params/dw"),
        "displayHeight": display("video-out-params/dh", "video-params/dh"),
        "fps": fps,
        "rotation": int("video-params/rotate").unwrap_or(0).rem_euclid(360),
        "hdr": hdr,
    }))
}

/// `audio.reconfigured`'s data, `None` without audio (no sample rate):
/// `sampleRate` in Hz, `channels` the channel count and `format` mpv's
/// sample format name, of the audio as decoded (mpv `audio-params`).
fn audio(
    generation: u64,
    int: &dyn Fn(&str) -> Option<i64>,
    text: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let rate = int("audio-params/samplerate").filter(|v| *v > 0)?;
    Some(json!({
        "sourceId": generation,
        "sampleRate": rate,
        "channels": int("audio-params/channel-count").filter(|v| *v > 0),
        "format": text("audio-params/format"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }
    fn read_video(generation: u64, map: &HashMap<String, String>) -> Option<Value> {
        let int = |name: &str| map.get(name).and_then(|v| v.parse::<i64>().ok());
        let float = |name: &str| map.get(name).and_then(|v| v.parse::<f64>().ok());
        let text = |name: &str| map.get(name).cloned();
        video(generation, &int, &float, &text)
    }

    #[test]
    fn video_facts_name_both_sizes_the_rate_rotation_and_hdr() {
        let map = props(&[
            ("video-params/w", "3840"),
            ("video-params/h", "2160"),
            ("video-out-params/dw", "1920"),
            ("video-out-params/dh", "1080"),
            ("container-fps", "23.976"),
            ("video-params/rotate", "90"),
            ("video-params/gamma", "pq"),
        ]);
        assert_eq!(
            read_video(3, &map).unwrap(),
            json!({"sourceId":3,"width":3840,"height":2160,"displayWidth":1920,
                "displayHeight":1080,"fps":23.976,"rotation":90,"hdr":true})
        );
        // No output parameters yet: the decoder's display size; no rate.
        let map = props(&[
            ("video-params/w", "720"),
            ("video-params/h", "576"),
            ("video-params/dw", "1024"),
            ("video-params/dh", "576"),
            ("video-params/gamma", "bt.1886"),
        ]);
        let facts = read_video(1, &map).unwrap();
        assert_eq!(facts["displayWidth"], 1024);
        assert_eq!(facts["fps"], Value::Null);
        assert_eq!(facts["hdr"], false);
        assert_eq!(facts["rotation"], 0);
        // No video: nothing.
        assert!(read_video(1, &props(&[("container-fps", "25")])).is_none());
    }

    #[test]
    fn audio_facts_need_a_sample_rate() {
        let map = props(&[
            ("audio-params/samplerate", "48000"),
            ("audio-params/channel-count", "6"),
            ("audio-params/format", "floatp"),
        ]);
        let int = |name: &str| map.get(name).and_then(|v| v.parse::<i64>().ok());
        let text = |name: &str| map.get(name).cloned();
        assert_eq!(
            audio(2, &int, &text).unwrap(),
            json!({"sourceId":2,"sampleRate":48000,"channels":6,"format":"floatp"})
        );
        let no_int = |_: &str| None::<i64>;
        let no_text = |_: &str| None::<String>;
        assert!(audio(2, &no_int, &no_text).is_none());
    }

    /// A reconfiguration that changes nothing sends nothing; a new source's
    /// first facts always go, even when they equal the last source's.
    #[test]
    fn facts_go_once_per_change_and_per_source() {
        let mut last = None;
        let a = json!({"width": 1920});
        assert_eq!(changed(&mut last, 1, a.clone()), Some(a.clone()));
        assert_eq!(changed(&mut last, 1, a.clone()), None);
        let b = json!({"width": 1280});
        assert_eq!(changed(&mut last, 1, b.clone()), Some(b.clone()));
        assert_eq!(changed(&mut last, 2, b.clone()), Some(b));
    }

    fn subtitle_state(generation: u64, phase: &str, selected: Value, tracks: usize) -> Value {
        json!({
            "source": {"generation": generation, "phase": phase},
            "selection": {"stamp": {"generation": generation, "revision": 1}, "target": selected, "secondary": null},
            "tracks": (0..tracks).map(|i| json!({"target": {"kind": "embedded", "id": i}})).collect::<Vec<_>>(),
            "presentation": {"revision": 0},
        })
    }

    #[test]
    fn subtitles_go_out_when_tracks_or_selection_change_of_a_loaded_source() {
        let mut facts = MediaFacts::default();
        let embedded = |id: u64| json!({"kind": "embedded", "id": id});
        // Loading: not yet.
        assert!(
            facts
                .subtitles(json!(1), || subtitle_state(1, "loading", Value::Null, 0))
                .is_none()
        );
        // Ready with two tracks, none selected.
        let ready = facts
            .subtitles(json!(2), || subtitle_state(1, "ready", Value::Null, 2))
            .unwrap();
        assert_eq!(ready["state"]["tracks"].as_array().unwrap().len(), 2);
        // The same key: the state is not even read.
        assert!(
            facts
                .subtitles(json!(2), || panic!("read for an unchanged key"))
                .is_none()
        );
        // Another key, the same reported part (a style change): nothing.
        let mut styled = subtitle_state(1, "ready", Value::Null, 2);
        styled["presentation"]["revision"] = json!(3);
        assert!(facts.subtitles(json!(3), || styled).is_none());
        // A track selected.
        assert!(
            facts
                .subtitles(json!(4), || subtitle_state(1, "ready", embedded(1), 2))
                .is_some()
        );
        // A track added.
        assert!(
            facts
                .subtitles(json!(5), || subtitle_state(1, "ready", embedded(1), 3))
                .is_some()
        );
        // The next source, with the same tracks: its own.
        assert!(
            facts
                .subtitles(json!(6), || subtitle_state(2, "ready", embedded(1), 3))
                .is_some()
        );
    }
}
