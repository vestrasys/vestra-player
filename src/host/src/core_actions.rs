//! The basic playback actions the core owns (R-H1): their menu declarations,
//! enabled/busy/checked state, and what each one does to the source, with the
//! same on-screen text the playback plugin used. Pure data; `backend.rs`
//! executes the plans and `host/actions.rs` registers the commands.
#![forbid(unsafe_code)]

use crate::commands::{CommandState, Declaration};
use crate::i18n::{tr, tr_args};
use serde_json::{Value, json};

/// What `host.run` accepts (the SDK's `HostAction`). All but `fullscreen`
/// are the playback commands declared here; `fullscreen` (host 0.5.2) is
/// the host's own Toggle Fullscreen command (`WINDOW_ACTIONS`).
pub const ACTIONS: [&str; 15] = [
    "play-pause",
    "stop",
    "previous",
    "next",
    "frame-back",
    "frame-forward",
    "seek-backward",
    "seek-forward",
    "rate-down",
    "rate-up",
    "rate-normal",
    "volume-up",
    "volume-down",
    "mute",
    "fullscreen",
];
/// Actions of the main window, not of playback: `host/actions.rs` registers their
/// commands (`register_host_commands`) and runs them like the command's key
/// (toggle fullscreen: in when windowed, out when fullscreen), without the
/// playback state, busy flags or `declarations` here.
pub const WINDOW_ACTIONS: [&str; 1] = ["fullscreen"];
pub const MIN_RATE: f64 = 0.25;
pub const MAX_RATE: f64 = 4.0;
pub const DEFAULT_STEP: f64 = 5.0;
fn unsupported() -> String {
    tr("Not supported by the current source")
}

/// Actions that drop a repeated trigger while one is in flight (the plugin's
/// busy flags); relative seeks and volume steps queue and accumulate instead.
pub fn exclusive(action: &str) -> bool {
    matches!(
        action,
        "play-pause"
            | "stop"
            | "frame-back"
            | "frame-forward"
            | "rate-down"
            | "rate-up"
            | "rate-normal"
            | "mute"
    )
}

/// What the actions need to know, read from a `source.state` value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    pub phase_ready: bool,
    pub phase_ended: bool,
    pub phase_empty: bool,
    pub paused: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub can_rate: bool,
    pub can_restart: bool,
    pub can_close: bool,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub rate: f64,
    pub volume: f64,
    pub muted: bool,
    pub media: bool,
    pub video: bool,
    pub next: bool,
    pub previous: bool,
}
impl Facts {
    pub fn from_state(state: &Value) -> Self {
        let flag = |value: &Value| value.as_bool().unwrap_or(false);
        let can = &state["capabilities"];
        let phase = state["phase"].as_str().unwrap_or("empty");
        Self {
            phase_ready: phase == "ready",
            phase_ended: phase == "ended",
            phase_empty: phase == "empty",
            paused: flag(&state["paused"]),
            can_pause: flag(&can["pause"]),
            can_seek: flag(&can["seek"]),
            can_rate: flag(&can["rate"]),
            can_restart: flag(&can["restart"]),
            can_close: flag(&can["close"]),
            position: state["position"].as_f64(),
            duration: state["duration"].as_f64(),
            rate: state["rate"].as_f64().unwrap_or(1.0),
            volume: state["output"]["volume"].as_f64().unwrap_or(100.0),
            muted: flag(&state["output"]["muted"]),
            media: state["kind"] == "media",
            video: flag(&state["hasVideo"]),
            next: flag(&state["sequence"]["next"]),
            previous: flag(&state["sequence"]["previous"]),
        }
    }
    fn can_toggle(&self) -> bool {
        (self.can_pause && self.phase_ready) || (self.phase_ended && self.can_restart)
    }
    fn can_step_seek(&self) -> bool {
        self.can_seek && self.phase_ready && self.position.is_some()
    }
    fn can_rate(&self) -> bool {
        self.can_rate && self.phase_ready
    }
}

/// JavaScript's `String(n)` for the values used here: `5`, `0.5`, `10.5`.
pub fn number_text(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}
/// `1`, `1.5`, `1.25`: at most two decimals.
pub fn rate_text(rate: f64) -> String {
    number_text((rate * 100.0).round() / 100.0)
}
/// `MM:SS`, `H:MM:SS` from one hour, `--:--` when unknown.
pub fn time(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds.filter(|s| s.is_finite()) else {
        return "--:--".into();
    };
    let whole = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    if whole >= 3600 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The relative seek item: its key (`Back 5 s`) and its menu text.
pub fn seek_title(forward: bool, step: f64) -> (String, String) {
    let step = number_text(step);
    if forward {
        (
            format!("Forward {step} s"),
            tr_args("Forward {} s", &json!([step])),
        )
    } else {
        (
            format!("Back {step} s"),
            tr_args("Back {} s", &json!([step])),
        )
    }
}

/// Menu entries: the same paths, orders and bindings the playback plugin
/// had. Paths are the keys (English); each item's menu text is the player's
/// translation.
pub fn declarations(step: f64) -> Vec<(&'static str, Declaration)> {
    let make = |category: &str,
                path: &[&str],
                shown: String,
                order: i32,
                group: &str,
                bindings: &[&str],
                repeat: bool| {
        let mut declaration = Declaration::new(path.last().unwrap(), path);
        declaration.order = order;
        declaration.group = group.into();
        declaration.bindings = bindings.iter().map(|b| b.to_string()).collect();
        declaration.repeat = repeat;
        declaration.localize(|_| shown.clone());
        categorize(&mut declaration, category);
        declaration
    };
    // Menu redesign step 4 (playback's panel, request of its author): the
    // seek, frame and speed actions leave Playback ▸ Seek and Speed for the
    // command palette, found by category and title ("Seek: Next Frame"),
    // with their keys unchanged; the Playback panel has their controls.
    let palette = |category: &str, title: &str, shown: String, bindings: &[&str], repeat: bool| {
        make(category, &[title], shown, 0, "domain", bindings, repeat).without_path()
    };
    let (backward, backward_shown) = seek_title(false, step);
    let (forward, forward_shown) = seek_title(true, step);
    let mut normal = palette("Speed", "Normal", tr("Normal"), &["Z"], false);
    normal.radio = true;
    vec![
        (
            "stop",
            make(
                "Playback",
                &["Close"],
                tr("Close"),
                120,
                "open",
                &["F4"],
                false,
            ),
        ),
        (
            "play-pause",
            make(
                "Playback",
                &["Playback", "Play/Pause"],
                tr("Play/Pause"),
                201,
                "domain",
                // A click on the video (user ruling 2026-10-26); it fires on
                // release while no double click is bound (`video_click`).
                &["Space", "Mouse:LeftClick"],
                false,
            ),
        ),
        (
            "previous",
            make(
                "Playback",
                &["Playback", "Previous"],
                tr("Previous"),
                202,
                "domain",
                &["PageUp", "Mouse:X2"],
                false,
            ),
        ),
        (
            "next",
            make(
                "Playback",
                &["Playback", "Next"],
                tr("Next"),
                203,
                "domain",
                &["PageDown", "Mouse:X1"],
                false,
            ),
        ),
        (
            "frame-back",
            palette("Seek", "Previous Frame", tr("Previous Frame"), &["D"], true),
        ),
        (
            "frame-forward",
            palette("Seek", "Next Frame", tr("Next Frame"), &["F"], true),
        ),
        (
            "seek-backward",
            palette("Seek", &backward, backward_shown, &["ArrowLeft"], true),
        ),
        (
            "seek-forward",
            palette("Seek", &forward, forward_shown, &["ArrowRight"], true),
        ),
        ("rate-normal", normal),
        (
            "rate-down",
            palette("Speed", "Slower 0.1", tr("Slower 0.1"), &["X"], false),
        ),
        (
            "rate-up",
            palette("Speed", "Faster 0.1", tr("Faster 0.1"), &["C"], false),
        ),
        (
            "volume-up",
            make(
                "Audio",
                &["Audio", "Volume +"],
                tr("Volume +"),
                501,
                "domain",
                &["ArrowUp", "Mouse:WheelUp"],
                true,
            ),
        ),
        (
            "volume-down",
            make(
                "Audio",
                &["Audio", "Volume −"],
                tr("Volume −"),
                502,
                "domain",
                &["ArrowDown", "Mouse:WheelDown"],
                true,
            ),
        ),
        (
            "mute",
            make(
                "Audio",
                &["Audio", "Mute"],
                tr("Mute"),
                503,
                "domain",
                &["M"],
                false,
            ),
        ),
    ]
}

/// The categories of the player's own commands: the command palette and
/// Keyboard Shortcuts name a command `Category: Title` in the built-in
/// plugins' words ("Seek: Back 5 s", "Window: Toggle Fullscreen").
pub const CATEGORIES: [&str; 7] = [
    "Playback", "Seek", "Speed", "Audio", "Window", "File", "Help",
];

/// A category of [`CATEGORIES`] in the interface language.
fn category_text(key: &str) -> String {
    match key {
        "Playback" => tr("Playback"),
        "Seek" => tr("Seek"),
        "Speed" => tr("Speed"),
        "Audio" => tr("Audio"),
        "Window" => tr("Window"),
        "File" => tr("File"),
        "Help" => tr("Help"),
        _ => key.to_owned(),
    }
}

/// Give a player command its category (one of [`CATEGORIES`]): the key and
/// its shown text. Call after [`Declaration::localize`], which fills the
/// shown category from the title's text.
pub fn categorize(declaration: &mut Declaration, key: &str) {
    debug_assert!(CATEGORIES.contains(&key), "{key}");
    declaration.category = Some(key.into());
    declaration.display.category = category_text(key);
}

/// Busy implies disabled; the reason is shown only when it cannot be done.
fn ready(enabled: bool, checked: Option<bool>, busy: bool, reason: String) -> CommandState {
    CommandState {
        enabled: enabled && !busy,
        checked,
        busy,
        reason: (!enabled).then_some(reason),
    }
}

pub fn state(action: &str, facts: &Facts, busy: bool) -> CommandState {
    match action {
        "play-pause" => ready(
            facts.can_toggle(),
            facts.phase_ready.then_some(facts.paused),
            busy,
            unsupported(),
        ),
        "stop" => ready(
            facts.can_close && !facts.phase_empty,
            None,
            busy,
            unsupported(),
        ),
        "previous" => ready(facts.previous, None, false, tr("No previous item")),
        "next" => ready(facts.next, None, false, tr("No next item")),
        "frame-back" | "frame-forward" => match frame_check(facts, action == "frame-forward") {
            Ok(()) => ready(true, None, busy, unsupported()),
            Err(reason) => ready(false, None, busy, reason),
        },
        "seek-backward" | "seek-forward" => {
            ready(facts.can_step_seek(), None, false, unsupported())
        }
        "rate-normal" => ready(
            facts.can_rate(),
            Some(facts.rate == 1.0),
            busy,
            unsupported(),
        ),
        "rate-down" | "rate-up" => ready(facts.can_rate(), None, busy, unsupported()),
        "volume-up" | "volume-down" => ready(true, None, false, unsupported()),
        "mute" => ready(true, Some(facts.muted), busy, unsupported()),
        _ => ready(false, None, false, unsupported()),
    }
}

/// What an action does once it is admitted.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Restart,
    Pause(bool),
    Seek(f64),
    Rate(f64),
    /// Volume first, then mute; `None` keeps the volume.
    Output {
        volume: Option<f64>,
        muted: bool,
    },
    Close,
    Frame {
        forward: bool,
    },
    Sequence {
        next: bool,
    },
}

/// The plan and the on-screen text. `seek_base` is the newest relative-seek
/// target still in progress (consecutive presses add up). `Err` = disabled,
/// with the reason.
pub fn plan(
    action: &str,
    facts: &Facts,
    step: f64,
    seek_base: Option<f64>,
) -> Result<(Plan, Option<String>), String> {
    let state = state(action, facts, false);
    if !state.enabled {
        return Err(state.reason.unwrap_or_else(unsupported));
    }
    Ok(match action {
        "play-pause" if facts.phase_ended => (Plan::Restart, Some(tr("Playing"))),
        "play-pause" => (
            Plan::Pause(!facts.paused),
            Some(if facts.paused {
                tr("Playing")
            } else {
                tr("Paused")
            }),
        ),
        "stop" => (Plan::Close, None),
        "previous" => (Plan::Sequence { next: false }, None),
        "next" => (Plan::Sequence { next: true }, None),
        "frame-back" => (Plan::Frame { forward: false }, None),
        "frame-forward" => (Plan::Frame { forward: true }, None),
        "seek-backward" | "seek-forward" => {
            let forward = action == "seek-forward";
            let delta = if forward { step } else { -step };
            let base = seek_base.or(facts.position).unwrap_or(0.0);
            let target = (base + delta)
                .max(0.0)
                .min(facts.duration.unwrap_or(f64::INFINITY));
            let args = json!([number_text(step), time(Some(target)), time(facts.duration)]);
            let text = if forward {
                tr_args("Forward {} s: {} / {}", &args)
            } else {
                tr_args("Back {} s: {} / {}", &args)
            };
            (Plan::Seek(target), Some(text))
        }
        "rate-normal" | "rate-down" | "rate-up" => {
            let rate = match action {
                "rate-normal" => 1.0,
                _ => {
                    let delta = if action == "rate-up" { 0.1 } else { -0.1 };
                    (((facts.rate + delta) * 100.0).round() / 100.0).clamp(MIN_RATE, MAX_RATE)
                }
            };
            (
                Plan::Rate(rate),
                Some(tr_args("Speed: {}x", &json!([rate_text(rate)]))),
            )
        }
        "volume-up" | "volume-down" => {
            let delta = if action == "volume-up" { 5.0 } else { -5.0 };
            // G-13a: up stops at 100 (`AudioVolumeMaxVal=100`); a louder volume set
            // through `setOutput` is kept, not raised further. Down steps by 5.
            let volume = if delta > 0.0 {
                (facts.volume + delta).round().min(100.0).max(facts.volume)
            } else {
                (facts.volume + delta).round().clamp(0.0, 200.0)
            };
            (
                Plan::Output {
                    volume: Some(volume),
                    muted: false,
                },
                Some(tr_args("Volume: {}%", &json!([number_text(volume)]))),
            )
        }
        "mute" => (
            Plan::Output {
                volume: None,
                muted: !facts.muted,
            },
            Some(if facts.muted {
                tr("Mute: Off")
            } else {
                tr("Mute: On")
            }),
        ),
        other => return Err(format!("unknown action {other}")),
    })
}

/// `player.setStepSeek`: 0.1–3600 seconds in tenths.
pub fn valid_step(seconds: f64) -> bool {
    seconds.is_finite()
        && (0.1..=3600.0).contains(&seconds)
        && ((seconds * 10.0).round() - seconds * 10.0).abs() < 1e-9
}

/// `host.run`'s optional `seconds` (SDK 1.5.19): a step seek of that step
/// instead of the `setStepSeek` one, only for the two step-seek actions.
pub fn run_step(action: &str, seconds: &Value) -> Result<Option<f64>, String> {
    if seconds.is_null() {
        return Ok(None);
    }
    if !matches!(action, "seek-backward" | "seek-forward") {
        return Err(format!(
            "seconds applies only to seek-backward and seek-forward, not {action}"
        ));
    }
    seconds
        .as_f64()
        .filter(|s| valid_step(*s))
        .map(Some)
        .ok_or_else(|| "seconds must be 0.1..3600 in tenths".into())
}

/// Keyframe step seeks (`setStepSeek({ keyframes: true })`, seek-keyframes
/// round): they apply to the two step-seek actions on a source with moving
/// video that can seek; everything else keeps the exact `plan`.
pub fn keyframe_seek(action: &str, facts: &Facts, keyframes: bool) -> bool {
    keyframes && matches!(action, "seek-backward" | "seek-forward") && facts.video && facts.can_seek
}

/// The mpv command of one keyframe step seek: always relative, so mpv
/// rounds forward seeks to a keyframe at or after the target and backward
/// ones to one at or before it (`absolute+keyframes` always rounds back and
/// would stick on the keyframe it started from).
pub fn key_seek_command(amount: f64) -> Vec<String> {
    vec![
        "seek".into(),
        format!("{amount:.6}"),
        "relative+keyframes".into(),
    ]
}

/// The text a keyframe step seek shows once it landed: the label is the
/// last press's step, the time where it landed.
pub fn key_seek_notice(forward: bool, step: f64, landed: f64, duration: Option<f64>) -> String {
    let args = json!([number_text(step), time(Some(landed)), time(duration)]);
    if forward {
        tr_args("Forward {} s: {} / {}", &args)
    } else {
        tr_args("Back {} s: {} / {}", &args)
    }
}

/// What a keyframe step seek does next once mpv settled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyNext {
    /// Another `relative+keyframes` seek by this amount (presses that came
    /// in meanwhile).
    Seek(f64),
    /// The one exact fallback to this absolute target: a forward keyframe
    /// seek that did not move forward (no keyframe after the target, the
    /// last GOP).
    Exact(f64),
    /// Landed: complete the presses.
    Done,
}

/// Keyframe step seeks are serialized from the landed position: one mpv seek
/// at a time, presses meanwhile add up (`owed`, both directions netting out),
/// and the next seek starts from where the previous one landed, so repeated
/// presses never stick on one keyframe.
#[derive(Clone, Debug, PartialEq)]
pub struct KeySeek {
    /// The landed position the issued seek started from, and its target.
    pub base: f64,
    pub target: f64,
    /// Seconds the presses that came in meanwhile still owe (± direction).
    pub owed: f64,
    /// The last press: its direction and step (the on-screen label).
    pub forward: bool,
    pub step: f64,
    /// The exact fallback of the issued seek was sent.
    pub exact: bool,
}

/// `position ± delta` within the source.
fn key_target(position: f64, delta: f64, duration: Option<f64>) -> f64 {
    (position + delta)
        .max(0.0)
        .min(duration.unwrap_or(f64::INFINITY))
}

impl KeySeek {
    /// A press while none is in flight, from the landed `position`: the
    /// state and the relative amount to seek, or `None` when nothing moves
    /// (backward at 0, forward at the end).
    pub fn start(
        position: f64,
        duration: Option<f64>,
        forward: bool,
        step: f64,
    ) -> (Self, Option<f64>) {
        let delta = if forward { step } else { -step };
        let target = key_target(position, delta, duration);
        let amount = target - position;
        let state = Self {
            base: position,
            target,
            owed: 0.0,
            forward,
            step,
            exact: false,
        };
        (state, (amount.abs() > 1e-6).then_some(amount))
    }
    /// A press while one is in flight: owed, not sent.
    pub fn press(&mut self, forward: bool, step: f64) {
        self.owed += if forward { step } else { -step };
        self.forward = forward;
        self.step = step;
    }
    /// mpv settled at `landed` (`at_end`: it reached the end).
    pub fn settled(&mut self, landed: f64, duration: Option<f64>, at_end: bool) -> KeyNext {
        let issued_forward = self.target > self.base;
        if issued_forward && !self.exact && !at_end && landed <= self.base + 1e-3 {
            self.exact = true;
            return KeyNext::Exact(self.target);
        }
        if self.owed.abs() > 1e-6 {
            let target = key_target(landed, self.owed, duration);
            self.owed = 0.0;
            let amount = target - landed;
            if amount.abs() > 1e-6 {
                self.base = landed;
                self.target = target;
                self.exact = false;
                return KeyNext::Seek(amount);
            }
        }
        KeyNext::Done
    }
}

pub fn no_video() -> String {
    tr("No video")
}
fn no_frame_back() -> String {
    tr("The current source cannot step back a frame")
}
fn last_frame() -> String {
    tr("Already at the last frame")
}
pub fn first_frame() -> String {
    tr("Already at the first frame")
}

/// Whether one frame that way is possible now (B07), or the reason. Forward
/// needs playing-ready media with moving video (a live stream can step
/// forward); backward also works after the end but needs a seekable source.
pub fn frame_check(facts: &Facts, forward: bool) -> Result<(), String> {
    if facts.phase_empty || !(facts.phase_ready || facts.phase_ended) {
        return Err(unsupported());
    }
    if !facts.media {
        return Err(if forward {
            unsupported()
        } else {
            no_frame_back()
        });
    }
    if !facts.video {
        return Err(no_video());
    }
    if forward && !facts.phase_ready {
        return Err(unsupported());
    }
    if !forward && !facts.can_seek {
        return Err(no_frame_back());
    }
    Ok(())
}

/// What `player.step` does.
#[derive(Clone, Debug, PartialEq)]
pub enum StepPlan {
    /// An mpv command, then wait for the new position.
    Command(Vec<&'static str>),
    /// After the end: an exact source seek one frame before the position.
    FromEnd,
    /// Already on the first frame: nothing moves.
    AtStart,
}

/// The plan for `player.step`, or `(code, reason)`.
pub fn step_plan(
    facts: &Facts,
    unit: &str,
    direction: &str,
) -> Result<StepPlan, (&'static str, String)> {
    let forward = match direction {
        "forward" => true,
        "backward" => false,
        _ => return Err(("invalid", "direction is backward or forward".into())),
    };
    match unit {
        "frame" => {
            frame_check(facts, forward).map_err(|reason| ("unsupported", reason))?;
            if !forward && facts.position.is_some_and(|p| p <= 0.001) {
                return Ok(StepPlan::AtStart);
            }
            Ok(if forward {
                StepPlan::Command(vec!["frame-step"])
            } else if facts.phase_ended {
                StepPlan::FromEnd
            } else {
                StepPlan::Command(vec!["frame-back-step"])
            })
        }
        "keyframe" => {
            if !(facts.phase_ready && facts.media && facts.video && facts.can_seek) {
                return Err((
                    "unsupported",
                    if facts.video {
                        unsupported()
                    } else {
                        no_video()
                    },
                ));
            }
            // Measured with mpv 0.41: `seek ±0 relative+keyframes` lands on
            // the current or an earlier keyframe; ±1 ms with keyframes
            // strictly passes the current one in either direction.
            Ok(StepPlan::Command(if forward {
                vec!["seek", "0.001", "relative+keyframes"]
            } else {
                vec!["seek", "-0.001", "relative+keyframes"]
            }))
        }
        _ => Err(("invalid", "unit is frame or keyframe".into())),
    }
}

/// `HH:MM:SS.mmm`.
pub fn precise_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1000 % 60,
        millis % 1000
    )
}

/// The frame-step notice once it settled: where it is, or the boundary.
pub fn frame_notice(forward: bool, moved: bool, position: Option<f64>) -> String {
    match (moved, position) {
        (true, Some(position)) if forward => {
            tr_args("Next frame: {}", &json!([precise_time(position)]))
        }
        (true, Some(position)) => tr_args("Previous frame: {}", &json!([precise_time(position)])),
        _ if forward => last_frame(),
        _ => first_frame(),
    }
}

/// Old playback-plugin overlay keys and the core action that replaces each.
pub const OVERLAY_MIGRATION: [(&str, &str); 12] = [
    ("vestra.playback/toggle-pause", "host/play-pause"),
    ("vestra.playback/close-source", "host/stop"),
    ("vestra.playback/previous", "host/previous"),
    ("vestra.playback/next", "host/next"),
    ("vestra.playback/backward-5", "host/seek-backward"),
    ("vestra.playback/forward-5", "host/seek-forward"),
    ("vestra.playback/rate-down", "host/rate-down"),
    ("vestra.playback/rate-up", "host/rate-up"),
    ("vestra.playback/rate-10", "host/rate-normal"),
    ("vestra.playback/volume-up", "host/volume-up"),
    ("vestra.playback/volume-down", "host/volume-down"),
    ("vestra.playback/mute", "host/mute"),
];

/// Rewrite old keys once; an existing new key wins and the old one is dropped.
/// Returns whether anything changed.
pub fn migrate_overlay(overlay: &mut std::collections::BTreeMap<String, Vec<String>>) -> bool {
    let mut changed = false;
    for (old, new) in OVERLAY_MIGRATION {
        if let Some(bindings) = overlay.remove(old) {
            changed = true;
            overlay.entry(new.to_owned()).or_insert(bindings);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Tests run in English (the locale is never set in tests); the
    // Simplified Chinese text is checked against the catalog below.
    const UNSUPPORTED: &str = "Not supported by the current source";
    const NO_VIDEO: &str = "No video";
    const NO_FRAME_BACK: &str = "The current source cannot step back a frame";
    const LAST_FRAME: &str = "Already at the last frame";
    const FIRST_FRAME: &str = "Already at the first frame";

    fn playing() -> Facts {
        Facts::from_state(
            &json!({"phase":"ready","paused":false,"kind":"media","hasVideo":true,
            "capabilities":{"pause":true,"seek":true,"rate":true,"restart":true,"close":true},
            "position":37.0,"duration":100.0,"rate":1.0,"output":{"volume":80.0,"muted":false},
            "sequence":{"next":true,"previous":false}}),
        )
    }

    #[test]
    fn declarations_match_the_playback_plugin_menu() {
        let declarations = declarations(5.0);
        assert_eq!(declarations.len(), 14);
        let find = |id: &str| &declarations.iter().find(|(each, _)| *each == id).unwrap().1;
        for id in ACTIONS {
            assert_eq!(
                declarations.iter().any(|(each, _)| *each == id),
                !WINDOW_ACTIONS.contains(&id),
                "{id}"
            );
        }
        assert_eq!(find("play-pause").path, ["Playback", "Play/Pause"]);
        assert_eq!(find("play-pause").order, 201);
        assert_eq!(find("play-pause").bindings, ["Space", "Mouse:LeftClick"]);
        assert_eq!(find("stop").path, ["Close"]);
        assert_eq!(find("stop").bindings, ["F4"]);
        assert_eq!(find("stop").group, "open");
        // Menu redesign step 4: seek, frame and speed are palette only, with
        // their titles, keys and categories.
        for id in [
            "frame-back",
            "frame-forward",
            "seek-backward",
            "seek-forward",
            "rate-down",
            "rate-up",
            "rate-normal",
        ] {
            assert!(find(id).path.is_empty(), "{id}");
            assert!(find(id).shown_path().is_empty(), "{id}");
            assert!(!find(id).bindings.is_empty(), "{id}");
        }
        assert_eq!(find("seek-backward").shown_title(), "Back 5 s");
        assert_eq!(find("seek-backward").category.as_deref(), Some("Seek"));
        assert_eq!(find("rate-up").category.as_deref(), Some("Speed"));
        assert_eq!(find("seek-backward").bindings, ["ArrowLeft"]);
        assert!(
            find("seek-forward").repeat && find("volume-up").repeat && find("frame-forward").repeat
        );
        assert!(!find("play-pause").repeat && !find("mute").repeat);
        assert_eq!(find("previous").bindings, ["PageUp", "Mouse:X2"]);
        assert_eq!(
            find("volume-down").bindings,
            ["ArrowDown", "Mouse:WheelDown"]
        );
        assert_eq!(find("volume-down").path, ["Audio", "Volume −"]);
        assert!(find("rate-normal").radio);
        assert_eq!(find("mute").order, 503);
        assert_eq!(
            declarations_titles(10.5),
            ("Back 10.5 s".to_owned(), "Forward 10.5 s".to_owned())
        );
        for (_, declaration) in &declarations {
            for binding in &declaration.bindings {
                crate::bindings::parse(binding).unwrap();
            }
        }
    }
    /// Each action names its category in the built-in plugins' words
    /// (Seek: Back 30 s, Speed: 2×, Audio: …): the key, and the shown text
    /// from the player's catalog, not the title `localize` gave it.
    #[test]
    fn every_action_has_its_category() {
        let expected = [
            ("stop", "Playback"),
            ("play-pause", "Playback"),
            ("previous", "Playback"),
            ("next", "Playback"),
            ("frame-back", "Seek"),
            ("frame-forward", "Seek"),
            ("seek-backward", "Seek"),
            ("seek-forward", "Seek"),
            ("rate-normal", "Speed"),
            ("rate-down", "Speed"),
            ("rate-up", "Speed"),
            ("volume-up", "Audio"),
            ("volume-down", "Audio"),
            ("mute", "Audio"),
        ];
        let all = declarations(5.0);
        let actual: Vec<(&str, Option<&str>, Option<&str>)> = all
            .iter()
            .map(|(id, declaration)| {
                (
                    *id,
                    declaration.category.as_deref(),
                    declaration.shown_category(),
                )
            })
            .collect();
        let wanted: Vec<_> = expected
            .iter()
            .map(|(id, category)| (*id, Some(*category), Some(*category)))
            .collect();
        assert_eq!(actual, wanted);
        for category in CATEGORIES {
            assert_eq!(category_text(category), category, "{category}");
        }
    }

    fn declarations_titles(step: f64) -> (String, String) {
        let all = declarations(step);
        let title = |id: &str| {
            all.iter()
                .find(|(each, _)| *each == id)
                .unwrap()
                .1
                .title
                .clone()
        };
        (title("seek-backward"), title("seek-forward"))
    }

    #[test]
    fn enabled_checked_and_reasons_follow_the_source() {
        let facts = playing();
        assert_eq!(state("play-pause", &facts, false).checked, Some(false));
        assert!(state("play-pause", &facts, true).busy);
        assert!(!state("play-pause", &facts, true).enabled);
        assert_eq!(
            state("previous", &facts, false).reason.as_deref(),
            Some("No previous item")
        );
        assert!(state("next", &facts, false).enabled);
        assert_eq!(state("rate-normal", &facts, false).checked, Some(true));
        assert_eq!(state("mute", &facts, false).checked, Some(false));
        let empty = Facts::from_state(&json!({"phase":"empty","capabilities":{}}));
        for action in [
            "play-pause",
            "stop",
            "seek-forward",
            "rate-up",
            "frame-forward",
        ] {
            let state = state(action, &empty, false);
            assert!(!state.enabled, "{action}");
            assert_eq!(state.reason.as_deref(), Some(UNSUPPORTED));
        }
        assert!(state("volume-up", &empty, false).enabled);
        let ended = Facts {
            phase_ready: false,
            phase_ended: true,
            ..facts
        };
        assert!(
            state("play-pause", &ended, false).enabled,
            "replay after the end"
        );
        assert_eq!(state("play-pause", &ended, false).checked, None);
        let audio = Facts {
            video: false,
            ..facts
        };
        assert!(!state("frame-forward", &audio, false).enabled);
        let unknown = Facts {
            position: None,
            ..facts
        };
        assert!(!state("seek-forward", &unknown, false).enabled);
    }

    #[test]
    fn plans_carry_the_playback_plugin_osd_text_and_clamps() {
        let facts = playing();
        assert_eq!(
            plan("play-pause", &facts, 5.0, None).unwrap(),
            (Plan::Pause(true), Some("Paused".into()))
        );
        let paused = Facts {
            paused: true,
            ..facts
        };
        assert_eq!(
            plan("play-pause", &paused, 5.0, None).unwrap().1.as_deref(),
            Some("Playing")
        );
        let ended = Facts {
            phase_ready: false,
            phase_ended: true,
            ..facts
        };
        assert_eq!(
            plan("play-pause", &ended, 5.0, None).unwrap(),
            (Plan::Restart, Some("Playing".into()))
        );
        assert_eq!(
            plan("seek-forward", &facts, 5.0, None).unwrap(),
            (Plan::Seek(42.0), Some("Forward 5 s: 00:42 / 01:40".into()))
        );
        // Consecutive presses accumulate from the newest pending target.
        assert_eq!(
            plan("seek-forward", &facts, 5.0, Some(42.0)).unwrap().0,
            Plan::Seek(47.0)
        );
        assert_eq!(
            plan("seek-backward", &facts, 60.0, None).unwrap().0,
            Plan::Seek(0.0)
        );
        assert_eq!(
            plan("seek-forward", &facts, 600.0, None).unwrap().0,
            Plan::Seek(100.0)
        );
        assert_eq!(
            plan("seek-backward", &facts, 10.0, None)
                .unwrap()
                .1
                .as_deref(),
            Some("Back 10 s: 00:27 / 01:40")
        );
        assert_eq!(
            plan("rate-up", &facts, 5.0, None).unwrap(),
            (Plan::Rate(1.1), Some("Speed: 1.1x".into()))
        );
        let fast = Facts {
            rate: 3.95,
            ..facts
        };
        assert_eq!(
            plan("rate-up", &fast, 5.0, None).unwrap().0,
            Plan::Rate(4.0)
        );
        let slow = Facts { rate: 0.3, ..facts };
        assert_eq!(
            plan("rate-down", &slow, 5.0, None).unwrap().0,
            Plan::Rate(0.25)
        );
        assert_eq!(
            plan("rate-normal", &fast, 5.0, None).unwrap().1.as_deref(),
            Some("Speed: 1x")
        );
        assert_eq!(
            plan("volume-up", &facts, 5.0, None).unwrap(),
            (
                Plan::Output {
                    volume: Some(85.0),
                    muted: false
                },
                Some("Volume: 85%".into())
            )
        );
        let loud = Facts {
            volume: 98.0,
            ..facts
        };
        // G-13a: up stops at 100; above 100 (another plugin's setOutput) it
        // stays; down still steps by 5.
        assert_eq!(
            plan("volume-up", &loud, 5.0, None).unwrap().1.as_deref(),
            Some("Volume: 100%")
        );
        let full = Facts {
            volume: 100.0,
            ..facts
        };
        assert_eq!(
            plan("volume-up", &full, 5.0, None).unwrap().1.as_deref(),
            Some("Volume: 100%")
        );
        let boosted = Facts {
            volume: 150.0,
            ..facts
        };
        assert_eq!(
            plan("volume-up", &boosted, 5.0, None).unwrap().1.as_deref(),
            Some("Volume: 150%")
        );
        assert_eq!(
            plan("volume-down", &boosted, 5.0, None)
                .unwrap()
                .1
                .as_deref(),
            Some("Volume: 145%")
        );
        let quiet = Facts {
            volume: 3.0,
            ..facts
        };
        assert_eq!(
            plan("volume-down", &quiet, 5.0, None).unwrap().1.as_deref(),
            Some("Volume: 0%")
        );
        assert_eq!(
            plan("mute", &facts, 5.0, None).unwrap().1.as_deref(),
            Some("Mute: On")
        );
        let muted = Facts {
            muted: true,
            ..facts
        };
        assert_eq!(
            plan("mute", &muted, 5.0, None).unwrap().1.as_deref(),
            Some("Mute: Off")
        );
        assert_eq!(
            plan("previous", &facts, 5.0, None).unwrap_err(),
            "No previous item"
        );
        assert_eq!(plan("stop", &facts, 5.0, None).unwrap().0, Plan::Close);
    }

    #[test]
    fn frame_steps_follow_b07_directions_reasons_and_notices() {
        let facts = playing();
        let plan = |facts: &Facts, unit, direction| step_plan(facts, unit, direction);
        assert_eq!(
            plan(&facts, "frame", "forward").unwrap(),
            StepPlan::Command(vec!["frame-step"])
        );
        assert_eq!(
            plan(&facts, "frame", "backward").unwrap(),
            StepPlan::Command(vec!["frame-back-step"])
        );
        assert_eq!(
            plan(&facts, "keyframe", "forward").unwrap(),
            StepPlan::Command(vec!["seek", "0.001", "relative+keyframes"])
        );
        assert_eq!(
            plan(&facts, "keyframe", "backward").unwrap(),
            StepPlan::Command(vec!["seek", "-0.001", "relative+keyframes"])
        );
        // A live stream (not seekable) steps forward only.
        let live = Facts {
            can_seek: false,
            ..facts
        };
        assert!(frame_check(&live, true).is_ok());
        assert_eq!(frame_check(&live, false), Err(NO_FRAME_BACK.to_owned()));
        assert_eq!(
            plan(&live, "keyframe", "forward").unwrap_err().0,
            "unsupported"
        );
        // After the end only backwards, by an exact seek.
        let ended = Facts {
            phase_ready: false,
            phase_ended: true,
            ..facts
        };
        assert_eq!(frame_check(&ended, true), Err(UNSUPPORTED.to_owned()));
        assert_eq!(
            plan(&ended, "frame", "backward").unwrap(),
            StepPlan::FromEnd
        );
        // Audio only, a generated stream, nothing open.
        let audio = Facts {
            video: false,
            ..facts
        };
        assert_eq!(frame_check(&audio, true), Err(NO_VIDEO.to_owned()));
        assert_eq!(frame_check(&audio, false), Err(NO_VIDEO.to_owned()));
        let stream = Facts {
            media: false,
            ..facts
        };
        assert_eq!(frame_check(&stream, false), Err(NO_FRAME_BACK.to_owned()));
        assert_eq!(frame_check(&stream, true), Err(UNSUPPORTED.to_owned()));
        let empty = Facts::from_state(&json!({"phase":"empty","capabilities":{}}));
        assert_eq!(frame_check(&empty, true), Err(UNSUPPORTED.to_owned()));
        // The first frame does not move.
        let start = Facts {
            position: Some(0.0),
            ..facts
        };
        assert_eq!(
            plan(&start, "frame", "backward").unwrap(),
            StepPlan::AtStart
        );
        assert_eq!(plan(&facts, "second", "forward").unwrap_err().0, "invalid");
        assert_eq!(plan(&facts, "frame", "up").unwrap_err().0, "invalid");
        // Command states carry the same reasons.
        assert_eq!(
            state("frame-back", &audio, false).reason.as_deref(),
            Some(NO_VIDEO)
        );
        assert!(state("frame-back", &ended, false).enabled);
        assert_eq!(
            state("frame-forward", &ended, false).reason.as_deref(),
            Some(UNSUPPORTED)
        );
        assert_eq!(
            state("frame-back", &live, false).reason.as_deref(),
            Some(NO_FRAME_BACK)
        );
        assert!(state("frame-forward", &live, false).enabled);
        // Notices.
        assert_eq!(
            frame_notice(true, true, Some(83.456)),
            "Next frame: 00:01:23.456"
        );
        assert_eq!(
            frame_notice(false, true, Some(3723.004)),
            "Previous frame: 01:02:03.004"
        );
        assert_eq!(frame_notice(true, false, Some(10.0)), LAST_FRAME);
        assert_eq!(frame_notice(false, false, Some(0.0)), FIRST_FRAME);
    }

    #[test]
    fn step_seconds_and_text_formatting() {
        for ok in [0.1, 5.0, 10.5, 3600.0] {
            assert!(valid_step(ok), "{ok}");
        }
        for bad in [0.0, 0.05, 3600.1, 1.25, f64::NAN] {
            assert!(!valid_step(bad), "{bad}");
        }
        assert_eq!(number_text(5.0), "5");
        assert_eq!(number_text(0.1 + 0.2), "0.3");
        assert_eq!(rate_text(1.25), "1.25");
        assert_eq!(time(Some(3723.0)), "1:02:03");
        assert_eq!(time(None), "--:--");
    }

    #[test]
    fn overlay_keys_move_once_and_new_keys_win() {
        let mut overlay = std::collections::BTreeMap::from([
            (
                "vestra.playback/toggle-pause".to_owned(),
                vec!["P".to_owned()],
            ),
            ("vestra.playback/mute".to_owned(), vec!["N".to_owned()]),
            ("host/mute".to_owned(), vec!["Ctrl+M".to_owned()]),
            ("vestra.playback/jump".to_owned(), vec!["J".to_owned()]),
        ]);
        assert!(migrate_overlay(&mut overlay));
        assert_eq!(overlay["host/play-pause"], ["P"]);
        assert_eq!(overlay["host/mute"], ["Ctrl+M"]);
        assert!(!overlay.contains_key("vestra.playback/mute"));
        assert_eq!(
            overlay["vestra.playback/jump"],
            ["J"],
            "commands that stay in playback keep theirs"
        );
        assert!(!migrate_overlay(&mut overlay));
    }

    /// mpv's `relative+keyframes` on these keyframes: forward to the first
    /// keyframe at or after the target (none: back to the last at or before
    /// it, as mpv's retry does), backward to the last at or before it.
    fn mpv_lands(keyframes: &[f64], from: f64, amount: f64) -> f64 {
        let target = from + amount;
        let before = || {
            keyframes
                .iter()
                .copied()
                .filter(|k| *k <= target)
                .fold(0.0, f64::max)
        };
        if amount > 0.0 {
            keyframes
                .iter()
                .copied()
                .find(|k| *k >= target)
                .unwrap_or_else(before)
        } else {
            before()
        }
    }

    /// Run presses (`(forward, step)`, all inside one flight) from `from`
    /// and settle until done: the positions it landed on, in order.
    fn key_run(keyframes: &[f64], from: f64, presses: &[(bool, f64)]) -> Vec<f64> {
        let duration = Some(45.0);
        let (first, rest) = presses.split_first().unwrap();
        let (mut state, amount) = KeySeek::start(from, duration, first.0, first.1);
        for (forward, step) in rest {
            state.press(*forward, *step);
        }
        let mut position = from;
        let mut landed = Vec::new();
        let mut next = amount.map_or(KeyNext::Done, KeyNext::Seek);
        loop {
            match next {
                KeyNext::Seek(amount) => position = mpv_lands(keyframes, position, amount),
                KeyNext::Exact(target) => position = target,
                KeyNext::Done => return landed,
            }
            landed.push(position);
            next = state.settled(position, duration, false);
        }
    }

    #[test]
    fn keyframe_step_seeks_land_on_keyframes_and_always_move() {
        let keys = [0.0, 10.0, 20.0, 30.0, 40.0];
        // From 12: → 5 lands on 20 (first at or after 17). ← 5 lands on 0,
        // the last at or before 7 (an early acceptance list said 10, which
        // is after the target: the rule "at or before position − step" wins).
        // From 10, ← 5 also lands on 0 (never 10); from 16, ← 5 on 10.
        assert_eq!(key_run(&keys, 12.0, &[(true, 5.0)]), [20.0]);
        assert_eq!(key_run(&keys, 12.0, &[(false, 5.0)]), [0.0]);
        assert_eq!(key_run(&keys, 10.0, &[(false, 5.0)]), [0.0]);
        assert_eq!(key_run(&keys, 16.0, &[(false, 5.0)]), [10.0]);
        // Two quick → 5 from 0: 10, then the owed 5 from 10 → 20 (not 10, 10).
        assert_eq!(
            key_run(&keys, 0.0, &[(true, 5.0), (true, 5.0)]),
            [10.0, 20.0]
        );
        // Tier 1 then tier 2 (5 then 30) from 0: 10, then 30 from 10 → 40.
        assert_eq!(
            key_run(&keys, 0.0, &[(true, 5.0), (true, 30.0)]),
            [10.0, 40.0]
        );
        // Mixed directions net out: → 5 lands on 10, then +5 −5 owes nothing.
        assert_eq!(
            key_run(&keys, 0.0, &[(true, 5.0), (true, 5.0), (false, 5.0)]),
            [10.0]
        );
        // Backward at 0 sends no seek.
        assert_eq!(KeySeek::start(0.0, Some(45.0), false, 5.0).1, None);
        // The last GOP: no keyframe after 42 + 2 → mpv goes back to 40, so the
        // host sends the target once as an exact seek, and only once.
        assert_eq!(key_run(&keys, 42.0, &[(true, 2.0)]), [40.0, 44.0]);
        let (mut state, _) = KeySeek::start(42.0, Some(45.0), true, 2.0);
        assert_eq!(state.settled(40.0, Some(45.0), false), KeyNext::Exact(44.0));
        assert_eq!(state.settled(42.0, Some(45.0), false), KeyNext::Done);
        // Reaching the end is a landing, not a reason for the fallback.
        let (mut state, _) = KeySeek::start(42.0, Some(45.0), true, 2.0);
        assert_eq!(state.settled(42.0, Some(45.0), true), KeyNext::Done);
        // The command and the text.
        assert_eq!(
            key_seek_command(-5.0),
            ["seek", "-5.000000", "relative+keyframes"]
        );
        assert_eq!(
            key_seek_notice(true, 5.0, 20.0, Some(45.0)),
            "Forward 5 s: 00:20 / 00:45"
        );
    }

    #[test]
    fn host_run_seconds_are_checked() {
        assert_eq!(run_step("seek-forward", &Value::Null), Ok(None));
        assert_eq!(run_step("seek-backward", &json!(30)), Ok(Some(30.0)));
        assert_eq!(run_step("play-pause", &Value::Null), Ok(None));
        for (action, seconds) in [
            ("play-pause", json!(5)),
            ("seek-forward", json!(0.05)),
            ("seek-forward", json!(3601)),
            ("seek-forward", json!(1.25)),
            ("seek-forward", json!("5")),
        ] {
            assert!(run_step(action, &seconds).is_err(), "{action} {seconds}");
        }
    }

    #[test]
    fn keyframe_mode_applies_only_to_step_seeks_on_seekable_video() {
        let facts = playing();
        assert!(keyframe_seek("seek-forward", &facts, true));
        assert!(keyframe_seek("seek-backward", &facts, true));
        assert!(!keyframe_seek("seek-forward", &facts, false));
        assert!(!keyframe_seek("frame-forward", &facts, true));
        let audio = Facts {
            video: false,
            ..facts
        };
        assert!(!keyframe_seek("seek-forward", &audio, true));
        let fixed = Facts {
            can_seek: false,
            ..facts
        };
        assert!(!keyframe_seek("seek-forward", &fixed, true));
        // Exact mode is today's plan.
        assert_eq!(
            plan("seek-forward", &facts, 5.0, Some(42.0)).unwrap().0,
            Plan::Seek(47.0)
        );
    }
}
