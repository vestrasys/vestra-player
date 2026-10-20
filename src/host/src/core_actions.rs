//! The basic playback actions the core owns (R-H1): their menu declarations,
//! enabled/busy/checked state, and what each one does to the source, with the
//! same on-screen text the playback plugin used. Pure data; `backend.rs`
//! executes the plans and `main.rs` registers the commands.
#![forbid(unsafe_code)]

use crate::commands::{CommandState, Declaration};
use serde_json::Value;

pub const ACTIONS: [&str; 14] = [
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
];
pub const MIN_RATE: f64 = 0.25;
pub const MAX_RATE: f64 = 4.0;
pub const DEFAULT_STEP: f64 = 5.0;
const UNSUPPORTED: &str = "当前来源不支持此操作";

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

pub fn seek_title(forward: bool, step: f64) -> String {
    format!(
        "{}{}秒",
        if forward { "前进" } else { "后退" },
        number_text(step)
    )
}

/// Menu entries: the same paths, orders and bindings the playback plugin had.
pub fn declarations(step: f64) -> Vec<(&'static str, Declaration)> {
    let make = |path: &[&str], order: i32, group: &str, bindings: &[&str], repeat: bool| {
        let mut declaration = Declaration::new(path.last().unwrap(), path);
        declaration.order = order;
        declaration.group = group.into();
        declaration.bindings = bindings.iter().map(|b| b.to_string()).collect();
        declaration.repeat = repeat;
        declaration
    };
    let backward = seek_title(false, step);
    let forward = seek_title(true, step);
    let mut normal = make(&["播放", "速度", "正常"], 220, "domain", &["Z"], false);
    normal.radio = true;
    vec![
        ("stop", make(&["关闭"], 120, "open", &["F4"], false)),
        (
            "play-pause",
            make(&["播放", "播放/暂停"], 201, "domain", &["Space"], false),
        ),
        (
            "previous",
            make(
                &["播放", "上一个"],
                202,
                "domain",
                &["PageUp", "Mouse:X1"],
                false,
            ),
        ),
        (
            "next",
            make(
                &["播放", "下一个"],
                203,
                "domain",
                &["PageDown", "Mouse:X2"],
                false,
            ),
        ),
        (
            "frame-back",
            make(&["播放", "跳转", "上一帧"], 204, "frame", &["D"], true),
        ),
        (
            "frame-forward",
            make(&["播放", "跳转", "下一帧"], 205, "frame", &["F"], true),
        ),
        (
            "seek-backward",
            make(
                &["播放", "跳转", &backward],
                210,
                "step",
                &["ArrowLeft"],
                true,
            ),
        ),
        (
            "seek-forward",
            make(
                &["播放", "跳转", &forward],
                211,
                "step",
                &["ArrowRight"],
                true,
            ),
        ),
        ("rate-normal", normal),
        (
            "rate-down",
            make(&["播放", "速度", "减速 0.1"], 221, "domain", &["X"], false),
        ),
        (
            "rate-up",
            make(&["播放", "速度", "加速 0.1"], 222, "domain", &["C"], false),
        ),
        (
            "volume-up",
            make(
                &["声音", "音量 +"],
                501,
                "domain",
                &["ArrowUp", "Mouse:WheelUp"],
                true,
            ),
        ),
        (
            "volume-down",
            make(
                &["声音", "音量 −"],
                502,
                "domain",
                &["ArrowDown", "Mouse:WheelDown"],
                true,
            ),
        ),
        (
            "mute",
            make(&["声音", "静音"], 503, "domain", &["M"], false),
        ),
    ]
}

/// Busy implies disabled; the reason is shown only when it cannot be done.
fn ready(enabled: bool, checked: Option<bool>, busy: bool, reason: &str) -> CommandState {
    CommandState {
        enabled: enabled && !busy,
        checked,
        busy,
        reason: (!enabled).then(|| reason.to_owned()),
    }
}

pub fn state(action: &str, facts: &Facts, busy: bool) -> CommandState {
    match action {
        "play-pause" => ready(
            facts.can_toggle(),
            facts.phase_ready.then_some(facts.paused),
            busy,
            UNSUPPORTED,
        ),
        "stop" => ready(
            facts.can_close && !facts.phase_empty,
            None,
            busy,
            UNSUPPORTED,
        ),
        "previous" => ready(facts.previous, None, false, "没有上一个"),
        "next" => ready(facts.next, None, false, "没有下一个"),
        "frame-back" | "frame-forward" => match frame_check(facts, action == "frame-forward") {
            Ok(()) => ready(true, None, busy, UNSUPPORTED),
            Err(reason) => ready(false, None, busy, reason),
        },
        "seek-backward" | "seek-forward" => ready(facts.can_step_seek(), None, false, UNSUPPORTED),
        "rate-normal" => ready(facts.can_rate(), Some(facts.rate == 1.0), busy, UNSUPPORTED),
        "rate-down" | "rate-up" => ready(facts.can_rate(), None, busy, UNSUPPORTED),
        "volume-up" | "volume-down" => ready(true, None, false, UNSUPPORTED),
        "mute" => ready(true, Some(facts.muted), busy, UNSUPPORTED),
        _ => ready(false, None, false, UNSUPPORTED),
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
        return Err(state.reason.unwrap_or_else(|| UNSUPPORTED.into()));
    }
    Ok(match action {
        "play-pause" if facts.phase_ended => (Plan::Restart, Some("播放".into())),
        "play-pause" => (
            Plan::Pause(!facts.paused),
            Some(if facts.paused { "播放" } else { "已暂停" }.into()),
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
            let text = format!(
                "{} {}秒: {} / {}",
                if forward { "前进" } else { "后退" },
                number_text(step),
                time(Some(target)),
                time(facts.duration)
            );
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
                Some(format!("播放速度: {}x", rate_text(rate))),
            )
        }
        "volume-up" | "volume-down" => {
            let delta = if action == "volume-up" { 5.0 } else { -5.0 };
            let volume = (facts.volume + delta).round().clamp(0.0, 200.0);
            (
                Plan::Output {
                    volume: Some(volume),
                    muted: false,
                },
                Some(format!("音量: {}%", number_text(volume))),
            )
        }
        "mute" => (
            Plan::Output {
                volume: None,
                muted: !facts.muted,
            },
            Some(
                if facts.muted {
                    "静音: 关"
                } else {
                    "静音: 开"
                }
                .into(),
            ),
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

pub const NO_VIDEO: &str = "没有视频";
pub const NO_FRAME_BACK: &str = "当前来源不支持逐帧后退";
pub const LAST_FRAME: &str = "已是最后一帧";
pub const FIRST_FRAME: &str = "已是第一帧";

/// Whether one frame that way is possible now (B07), or the reason. Forward
/// needs playing-ready media with moving video (a live stream can step
/// forward); backward also works after the end but needs a seekable source.
pub fn frame_check(facts: &Facts, forward: bool) -> Result<(), &'static str> {
    if facts.phase_empty || !(facts.phase_ready || facts.phase_ended) {
        return Err(UNSUPPORTED);
    }
    if !facts.media {
        return Err(if forward { UNSUPPORTED } else { NO_FRAME_BACK });
    }
    if !facts.video {
        return Err(NO_VIDEO);
    }
    if forward && !facts.phase_ready {
        return Err(UNSUPPORTED);
    }
    if !forward && !facts.can_seek {
        return Err(NO_FRAME_BACK);
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
) -> Result<StepPlan, (&'static str, &'static str)> {
    let forward = match direction {
        "forward" => true,
        "backward" => false,
        _ => return Err(("invalid", "direction is backward or forward")),
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
                    if facts.video { UNSUPPORTED } else { NO_VIDEO },
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
        _ => Err(("invalid", "unit is frame or keyframe")),
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
        (true, Some(position)) => format!(
            "定位: {} {}",
            if forward { "下一帧" } else { "上一帧" },
            precise_time(position)
        ),
        _ if forward => LAST_FRAME.into(),
        _ => FIRST_FRAME.into(),
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
            assert!(declarations.iter().any(|(each, _)| *each == id), "{id}");
        }
        assert_eq!(find("play-pause").path, ["播放", "播放/暂停"]);
        assert_eq!(find("play-pause").order, 201);
        assert_eq!(find("stop").path, ["关闭"]);
        assert_eq!(find("stop").bindings, ["F4"]);
        assert_eq!(find("stop").group, "open");
        assert_eq!(find("seek-backward").path, ["播放", "跳转", "后退5秒"]);
        assert_eq!(find("seek-backward").bindings, ["ArrowLeft"]);
        assert!(
            find("seek-forward").repeat && find("volume-up").repeat && find("frame-forward").repeat
        );
        assert!(!find("play-pause").repeat && !find("mute").repeat);
        assert_eq!(find("previous").bindings, ["PageUp", "Mouse:X1"]);
        assert_eq!(
            find("volume-down").bindings,
            ["ArrowDown", "Mouse:WheelDown"]
        );
        assert_eq!(find("volume-down").path, ["声音", "音量 −"]);
        assert!(find("rate-normal").radio);
        assert_eq!(
            (
                find("rate-normal").order,
                find("rate-down").order,
                find("rate-up").order
            ),
            (220, 221, 222)
        );
        assert_eq!(find("mute").order, 503);
        assert_eq!(
            declarations_titles(10.5),
            ("后退10.5秒".to_owned(), "前进10.5秒".to_owned())
        );
        for (_, declaration) in &declarations {
            for binding in &declaration.bindings {
                crate::bindings::parse(binding).unwrap();
            }
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
            Some("没有上一个")
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
            (Plan::Pause(true), Some("已暂停".into()))
        );
        let paused = Facts {
            paused: true,
            ..facts
        };
        assert_eq!(
            plan("play-pause", &paused, 5.0, None).unwrap().1.as_deref(),
            Some("播放")
        );
        let ended = Facts {
            phase_ready: false,
            phase_ended: true,
            ..facts
        };
        assert_eq!(
            plan("play-pause", &ended, 5.0, None).unwrap(),
            (Plan::Restart, Some("播放".into()))
        );
        assert_eq!(
            plan("seek-forward", &facts, 5.0, None).unwrap(),
            (Plan::Seek(42.0), Some("前进 5秒: 00:42 / 01:40".into()))
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
            Some("后退 10秒: 00:27 / 01:40")
        );
        assert_eq!(
            plan("rate-up", &facts, 5.0, None).unwrap(),
            (Plan::Rate(1.1), Some("播放速度: 1.1x".into()))
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
            Some("播放速度: 1x")
        );
        assert_eq!(
            plan("volume-up", &facts, 5.0, None).unwrap(),
            (
                Plan::Output {
                    volume: Some(85.0),
                    muted: false
                },
                Some("音量: 85%".into())
            )
        );
        let loud = Facts {
            volume: 198.0,
            ..facts
        };
        assert_eq!(
            plan("volume-up", &loud, 5.0, None).unwrap().1.as_deref(),
            Some("音量: 200%")
        );
        let quiet = Facts {
            volume: 3.0,
            ..facts
        };
        assert_eq!(
            plan("volume-down", &quiet, 5.0, None).unwrap().1.as_deref(),
            Some("音量: 0%")
        );
        assert_eq!(
            plan("mute", &facts, 5.0, None).unwrap().1.as_deref(),
            Some("静音: 开")
        );
        let muted = Facts {
            muted: true,
            ..facts
        };
        assert_eq!(
            plan("mute", &muted, 5.0, None).unwrap().1.as_deref(),
            Some("静音: 关")
        );
        assert_eq!(
            plan("previous", &facts, 5.0, None).unwrap_err(),
            "没有上一个"
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
        assert_eq!(frame_check(&live, false), Err(NO_FRAME_BACK));
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
        assert_eq!(frame_check(&ended, true), Err(UNSUPPORTED));
        assert_eq!(
            plan(&ended, "frame", "backward").unwrap(),
            StepPlan::FromEnd
        );
        // Audio only, a generated stream, nothing open.
        let audio = Facts {
            video: false,
            ..facts
        };
        assert_eq!(frame_check(&audio, true), Err(NO_VIDEO));
        assert_eq!(frame_check(&audio, false), Err(NO_VIDEO));
        let stream = Facts {
            media: false,
            ..facts
        };
        assert_eq!(frame_check(&stream, false), Err(NO_FRAME_BACK));
        assert_eq!(frame_check(&stream, true), Err(UNSUPPORTED));
        let empty = Facts::from_state(&json!({"phase":"empty","capabilities":{}}));
        assert_eq!(frame_check(&empty, true), Err(UNSUPPORTED));
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
            "定位: 下一帧 00:01:23.456"
        );
        assert_eq!(
            frame_notice(false, true, Some(3723.004)),
            "定位: 上一帧 01:02:03.004"
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
}
