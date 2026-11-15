//! The `when` language (SDK 1.5.40, `plugin.when`; grown in SDK 1.10.0 /
//! player 0.10.0, doc/architecture/events-and-triggers.md §4.3): a
//! condition on state the player owns, evaluated here, so a plugin that is
//! not running (in standby, doc/architecture/supervisor.md §4) still shows
//! a menu that follows playback, fullscreen and the appearance, and is
//! woken when a trigger's condition flips (`crate::triggers`).
//!
//! Wire form (`commands.register` / `commands.update` `when`, and
//! `triggers.set` `when`): an object of exactly one key:
//!
//! - `phase: [..]`: the source's phase is one of these (`empty`, `loading`,
//!   `ready`, `ended`, `error`)
//! - `paused: bool`, `kind: "media" | "stream"`, `fullscreen: bool`
//! - `can: "<control>"`: the source allows it (`PlayerState.can` names)
//! - `theme: "<id>"`, `language: "<preference>"` (`system` or a locale)
//! - numeric, one half-open interval `{ min?, max? }` (`min ≤ value < max`,
//!   at least one bound): `duration` (seconds; false while unknown),
//!   `volume`, `rate`
//! - identity: `audioTrack: <id>`, `subtitleTrack: "<id>"` (the primary
//!   one, the SDK's `SubtitleTrack.id`), `audioDevice: "<id>"` (the active
//!   output), `muted`, `maximized`, `pip`, `looping` (an A-B loop is set):
//!   `bool`
//! - `time: { min?, max? }`: the wall clock, Unix milliseconds
//! - `position: { ranges: [[start, end?], ..], lead? }`: the position is in
//!   one of the ranges, or less than `lead` media seconds before its start;
//!   bound to one source by its trigger (`crate::triggers`)
//! - `all: [..]`, `any: [..]`, `not: Condition`
//!
//! Two-valued logic: a fact that cannot be read counts as false. A range
//! list is one node however long it is; at most one per condition. The
//! position and time atoms are for triggers only: a command's state
//! refuses them (they need a source binding and a clock the command
//! registry does not have). Every other atom works in a command's state
//! too, `subtitleTrack` included: the command registry gets the same facts
//! as the triggers (`Host::condition_facts`) and follows the subtitle
//! track's changes.
use serde_json::Value;
use std::sync::Arc;

/// The source controls a condition may name: `PlayerState.can` in the SDK.
const CONTROLS: &[&str] = &[
    "pause", "seek", "rate", "restart", "close", "snapshot", "next", "previous",
];
const PHASES: &[&str] = &["empty", "loading", "ready", "ended", "error"];
/// Runaway guards: a condition is a few nodes in practice.
const MAX_NODES: usize = 32;
const MAX_DEPTH: usize = 6;
/// Ranges in one position list: the timeline layer's number (one constant,
/// `crate::timeline::MAX_RANGES`, SDK `TIMELINE_LIMITS.ranges`).
pub const MAX_RANGES: usize = crate::timeline::MAX_RANGES;

/// The groups of facts a condition reads (design §4.10): a change in one
/// group evaluates again only the conditions that read it.
pub mod groups {
    /// The source's discrete facts (phase, pause, kind, controls, duration,
    /// volume, rate, audio track and device, muted, loop) and its id.
    pub const SOURCE: u8 = 1;
    /// Fullscreen, maximized, picture-in-picture.
    pub const WINDOW: u8 = 2;
    /// Theme and language.
    pub const APPEARANCE: u8 = 4;
    /// The primary subtitle track.
    pub const SUBTITLES: u8 = 8;
    /// The position (the position model's reports).
    pub const POSITION: u8 = 16;
    /// The wall clock.
    pub const TIME: u8 = 32;
}

/// A numeric atom's interval: `min ≤ value < max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub min: Option<f64>,
    pub max: Option<f64>,
}
impl Interval {
    pub fn contains(&self, value: f64) -> bool {
        self.min.is_none_or(|min| min <= value) && self.max.is_none_or(|max| value < max)
    }
}

/// A position atom's ranges (design §4.3): sorted once, searched by
/// binary search.
#[derive(Debug, PartialEq)]
pub struct RangeList {
    /// The ranges as given (`index` is the place here): start, end
    /// (`None`: open at the end), media seconds.
    pub given: Vec<(f64, Option<f64>)>,
    /// Media seconds each range starts earlier.
    pub lead: f64,
    /// Indexes into `given`, by effective start (`start − lead`), then index.
    order: Vec<usize>,
    /// The effective starts, in `order`.
    starts: Vec<f64>,
    /// The running maximum of the ends (open: infinite), in `order`.
    reach: Vec<f64>,
    /// Every finite end, sorted.
    ends: Vec<f64>,
}
impl RangeList {
    pub fn new(given: Vec<(f64, Option<f64>)>, lead: f64) -> Self {
        let mut order: Vec<usize> = (0..given.len()).collect();
        order.sort_by(|a, b| {
            (given[*a].0 - lead)
                .total_cmp(&(given[*b].0 - lead))
                .then(a.cmp(b))
        });
        let starts = order.iter().map(|i| given[*i].0 - lead).collect();
        let mut reach = Vec::with_capacity(order.len());
        let mut furthest = f64::NEG_INFINITY;
        for i in &order {
            furthest = furthest.max(given[*i].1.unwrap_or(f64::INFINITY));
            reach.push(furthest);
        }
        let mut ends: Vec<f64> = given.iter().filter_map(|(_, end)| *end).collect();
        ends.sort_by(f64::total_cmp);
        Self {
            given,
            lead,
            order,
            starts,
            reach,
            ends,
        }
    }
    /// The range `position` is in (`start − lead ≤ position < end`): the
    /// one with the earliest start when several are (T10).
    pub fn matched(&self, position: f64) -> Option<usize> {
        let begun = self.starts.partition_point(|start| *start <= position);
        let first = self.reach[..begun].partition_point(|end| *end <= position);
        (first < begun).then(|| self.order[first])
    }
    /// The nearest place after `position` where what `matched` says can
    /// change: an effective start or an end (`None`: none is left).
    pub fn next_boundary(&self, position: f64) -> Option<f64> {
        let start = self
            .starts
            .get(self.starts.partition_point(|s| *s <= position))
            .copied();
        let end = self
            .ends
            .get(self.ends.partition_point(|e| *e <= position))
            .copied();
        match (start, end) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    /// Every place in `(from, to]` where what `matched` says can change,
    /// in order, each once (continuous playback passed them, T2).
    pub fn boundaries_between(&self, from: f64, to: f64) -> Vec<f64> {
        let mut found: Vec<f64> = Vec::new();
        for sorted in [&self.starts, &self.ends] {
            let first = sorted.partition_point(|x| *x <= from);
            let last = sorted.partition_point(|x| *x <= to);
            found.extend_from_slice(&sorted[first..last]);
        }
        found.sort_by(f64::total_cmp);
        found.dedup();
        found
    }
    /// The range at `index`, as the handler gets it: `{ start, end, index }`.
    pub fn range(&self, index: usize) -> Value {
        let (start, end) = self.given[index];
        serde_json::json!({"start":start,"end":end,"index":index})
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Phase(Vec<String>),
    Paused(bool),
    Kind(String),
    Can(String),
    Fullscreen(bool),
    Theme(String),
    Language(String),
    Duration(Interval),
    Volume(Interval),
    Rate(Interval),
    AudioTrack(i64),
    SubtitleTrack(String),
    AudioDevice(String),
    Muted(bool),
    Maximized(bool),
    Pip(bool),
    Looping(bool),
    Time(Interval),
    Position(Arc<RangeList>),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

/// The state conditions read: the player's own (source, window,
/// appearance, the primary subtitle track), never a plugin's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub phase: String,
    pub paused: bool,
    /// `media` or `stream` (a generated source).
    pub kind: String,
    /// The controls the source allows (`CONTROLS` names).
    pub can: Vec<String>,
    pub fullscreen: bool,
    pub theme: String,
    /// `system` or a locale.
    pub language: String,
    /// Seconds; `None` while unknown.
    pub duration: Option<f64>,
    pub volume: f64,
    pub rate: f64,
    pub muted: bool,
    /// The selected audio track's id (`AudioTrack.id`).
    pub audio_track: Option<i64>,
    /// The active audio output device.
    pub audio_device: Option<String>,
    /// An A-B loop is set.
    pub looping: bool,
    /// The main window is maximized; never while fullscreen.
    pub maximized: bool,
    pub pip: bool,
    /// The primary subtitle track's id (`SubtitleTrack.id`), from the
    /// playback worker (`Event::SubtitleTrack`), not from `source.state`.
    pub subtitle_track: Option<String>,
}
impl Facts {
    /// From the last `source.state` (wire form), the last `window.state`,
    /// the palette and the saved language preference.
    pub fn new(source: &Value, window: &Value, theme: &str, language: &str) -> Self {
        let flag = |value: &Value| value.as_bool().unwrap_or(false);
        let capabilities = &source["capabilities"];
        let can = CONTROLS
            .iter()
            .filter(|control| match **control {
                "snapshot" => flag(&capabilities["snapshotSource"]),
                "next" | "previous" => flag(&source["sequence"][*control]),
                other => flag(&capabilities[other]),
            })
            .map(|control| (*control).to_owned())
            .collect();
        let audio_track = source["audioTracks"].as_array().and_then(|tracks| {
            tracks
                .iter()
                .find(|track| track["selected"] == true)
                .and_then(|track| track["trackId"].as_i64())
        });
        Self {
            phase: source["phase"].as_str().unwrap_or("empty").to_owned(),
            paused: flag(&source["paused"]),
            kind: if source["kind"] == "producer" {
                "stream"
            } else {
                "media"
            }
            .to_owned(),
            can,
            fullscreen: flag(&window["fullscreen"]),
            theme: theme.to_owned(),
            language: language.to_owned(),
            duration: source["duration"].as_f64().filter(|d| d.is_finite()),
            volume: source["output"]["volume"].as_f64().unwrap_or(100.),
            rate: source["rate"].as_f64().unwrap_or(1.),
            muted: flag(&source["output"]["muted"]),
            audio_track,
            audio_device: source["audioOutput"]["active"].as_str().map(str::to_owned),
            looping: !source["loop"].is_null(),
            // As `WindowState.maximized` (`host/state.rs`): never while
            // fullscreen, whatever the window under it is.
            maximized: flag(&window["maximized"]) && !flag(&window["fullscreen"]),
            pip: flag(&window["pictureInPicture"]["active"]),
            subtitle_track: None,
        }
    }
}

/// What a trigger's evaluation reads besides the facts: the current
/// source's id, the position (`None`: none is known) and the wall clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moment {
    pub source: u64,
    pub position: Option<f64>,
    /// Unix milliseconds.
    pub time: f64,
}

/// One evaluation: whether the condition holds and, when its position
/// atom holds, the range the position is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Truth {
    pub holds: bool,
    pub range: Option<usize>,
}

/// Why a condition was refused: `limit` (more ranges than allowed) or
/// `invalid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}
impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self {
            code: "invalid",
            message,
        }
    }
}
impl From<&str> for Refusal {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

/// What `read` counts while it walks a condition.
#[derive(Default)]
struct Walk {
    nodes: usize,
    positions: usize,
}

impl Condition {
    /// Reads the wire form; `Err` names the problem (`invalid`).
    pub fn parse(value: &Value) -> Result<Self, String> {
        Self::parse_coded(value).map_err(|refusal| refusal.message)
    }
    /// Reads the wire form; `Err` with its code (`invalid`, `limit`).
    pub fn parse_coded(value: &Value) -> Result<Self, Refusal> {
        let mut walk = Walk::default();
        Self::read(value, 0, &mut walk)
    }
    fn read(value: &Value, depth: usize, walk: &mut Walk) -> Result<Self, Refusal> {
        walk.nodes += 1;
        if walk.nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(format!(
                "a condition has at most {MAX_NODES} parts, {MAX_DEPTH} levels deep"
            )
            .into());
        }
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or("a condition is an object with one key")?;
        let (key, value) = object.iter().next().expect("one key");
        let text = |what: &str| {
            value
                .as_str()
                .filter(|text| !text.is_empty() && text.len() <= 64)
                .map(str::to_owned)
                .ok_or(format!("condition {key} is {what}"))
        };
        let flag = || {
            value
                .as_bool()
                .ok_or(format!("condition {key} is true or false"))
        };
        let list = |walk: &mut Walk| -> Result<Vec<Self>, Refusal> {
            let items = value
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or(format!("condition {key} is a list of conditions"))?;
            items
                .iter()
                .map(|item| Self::read(item, depth + 1, walk))
                .collect()
        };
        Ok(match key.as_str() {
            "phase" => {
                let phases = value
                    .as_array()
                    .filter(|items| !items.is_empty())
                    .and_then(|items| {
                        items
                            .iter()
                            .map(|item| item.as_str().filter(|p| PHASES.contains(p)))
                            .collect::<Option<Vec<_>>>()
                    })
                    .ok_or(format!(
                        "condition phase is a list of {}",
                        PHASES.join(", ")
                    ))?;
                Self::Phase(phases.into_iter().map(str::to_owned).collect())
            }
            "paused" => Self::Paused(flag()?),
            "fullscreen" => Self::Fullscreen(flag()?),
            "kind" => match value.as_str() {
                Some(kind @ ("media" | "stream")) => Self::Kind(kind.to_owned()),
                _ => return Err("condition kind is media or stream".into()),
            },
            "can" => match value.as_str() {
                Some(control) if CONTROLS.contains(&control) => Self::Can(control.to_owned()),
                _ => {
                    return Err(format!("condition can is one of {}", CONTROLS.join(", ")).into());
                }
            },
            "theme" => match value.as_str() {
                Some(theme) if crate::ui::theme::THEMES.contains(&theme) => {
                    Self::Theme(theme.to_owned())
                }
                _ => {
                    return Err(format!(
                        "condition theme is one of {}",
                        crate::ui::theme::THEMES.join(", ")
                    )
                    .into());
                }
            },
            "language" => {
                let language = text("a language preference")?;
                if !crate::i18n::valid_preference(&language) {
                    return Err(format!(
                        "condition language is \"system\" or one of {}",
                        crate::i18n::LOCALES.join(", ")
                    )
                    .into());
                }
                Self::Language(language)
            }
            "duration" => Self::Duration(interval(key, value)?),
            "volume" => Self::Volume(interval(key, value)?),
            "rate" => Self::Rate(interval(key, value)?),
            "time" => Self::Time(interval(key, value)?),
            "audioTrack" => Self::AudioTrack(
                value
                    .as_i64()
                    .ok_or("condition audioTrack is a track id (an integer)")?,
            ),
            "subtitleTrack" => Self::SubtitleTrack(
                value
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or("condition subtitleTrack is a track id (a string)")?
                    .to_owned(),
            ),
            "audioDevice" => Self::AudioDevice(
                value
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or("condition audioDevice is a device id (a string)")?
                    .to_owned(),
            ),
            "muted" => Self::Muted(flag()?),
            "maximized" => Self::Maximized(flag()?),
            "pip" => Self::Pip(flag()?),
            "looping" => Self::Looping(flag()?),
            "position" => {
                walk.positions += 1;
                if walk.positions > 1 {
                    return Err("a condition has at most one position list".into());
                }
                Self::Position(Arc::new(range_list(value)?))
            }
            "all" => Self::All(list(walk)?),
            "any" => Self::Any(list(walk)?),
            "not" => Self::Not(Box::new(Self::read(value, depth + 1, walk)?)),
            other => return Err(format!("unknown condition {other}").into()),
        })
    }
    /// Whether it holds for `facts` (a command's state: no position, no
    /// clock, which a command refuses anyway).
    pub fn holds(&self, facts: &Facts) -> bool {
        self.eval(facts, None, None).holds
    }
    /// Evaluates it against `facts` and `moment`; `bound` is the source its
    /// position list belongs to (T12: false for any other source).
    pub fn eval(&self, facts: &Facts, moment: Option<&Moment>, bound: Option<u64>) -> Truth {
        let plain = |holds: bool| Truth { holds, range: None };
        match self {
            Self::Phase(phases) => plain(phases.contains(&facts.phase)),
            Self::Paused(paused) => plain(facts.paused == *paused),
            Self::Kind(kind) => plain(facts.kind == *kind),
            Self::Can(control) => plain(facts.can.contains(control)),
            Self::Fullscreen(fullscreen) => plain(facts.fullscreen == *fullscreen),
            Self::Theme(theme) => plain(facts.theme == *theme),
            Self::Language(language) => plain(facts.language == *language),
            Self::Duration(interval) => plain(facts.duration.is_some_and(|d| interval.contains(d))),
            Self::Volume(interval) => plain(interval.contains(facts.volume)),
            Self::Rate(interval) => plain(interval.contains(facts.rate)),
            Self::AudioTrack(id) => plain(facts.audio_track == Some(*id)),
            Self::SubtitleTrack(id) => plain(facts.subtitle_track.as_ref() == Some(id)),
            Self::AudioDevice(id) => plain(facts.audio_device.as_ref() == Some(id)),
            Self::Muted(muted) => plain(facts.muted == *muted),
            Self::Maximized(maximized) => plain(facts.maximized == *maximized),
            Self::Pip(pip) => plain(facts.pip == *pip),
            Self::Looping(looping) => plain(facts.looping == *looping),
            Self::Time(interval) => plain(moment.is_some_and(|m| interval.contains(m.time))),
            Self::Position(list) => {
                let range = moment
                    .filter(|m| bound == Some(m.source))
                    .and_then(|m| m.position)
                    .and_then(|position| list.matched(position));
                Truth {
                    holds: range.is_some(),
                    range,
                }
            }
            Self::All(all) => {
                let mut range = None;
                for each in all {
                    let truth = each.eval(facts, moment, bound);
                    if !truth.holds {
                        return plain(false);
                    }
                    range = range.or(truth.range);
                }
                Truth { holds: true, range }
            }
            Self::Any(any) => {
                let mut truth = plain(false);
                for each in any {
                    let one = each.eval(facts, moment, bound);
                    truth.holds |= one.holds;
                    truth.range = truth.range.or(one.range);
                }
                truth
            }
            Self::Not(not) => plain(!not.eval(facts, moment, bound).holds),
        }
    }
    /// The groups of facts it reads (`groups`).
    pub fn groups(&self) -> u8 {
        match self {
            Self::Phase(_)
            | Self::Paused(_)
            | Self::Kind(_)
            | Self::Can(_)
            | Self::Duration(_)
            | Self::Volume(_)
            | Self::Rate(_)
            | Self::AudioTrack(_)
            | Self::AudioDevice(_)
            | Self::Muted(_)
            | Self::Looping(_) => groups::SOURCE,
            Self::Fullscreen(_) | Self::Maximized(_) | Self::Pip(_) => groups::WINDOW,
            Self::Theme(_) | Self::Language(_) => groups::APPEARANCE,
            Self::SubtitleTrack(_) => groups::SUBTITLES,
            Self::Time(_) => groups::TIME,
            // Its source too: a new source makes it false (T12).
            Self::Position(_) => groups::POSITION | groups::SOURCE,
            Self::All(each) | Self::Any(each) => each.iter().fold(0, |g, c| g | c.groups()),
            Self::Not(not) => not.groups(),
        }
    }
    /// Its position list, if it has one.
    pub fn position(&self) -> Option<&Arc<RangeList>> {
        match self {
            Self::Position(list) => Some(list),
            Self::All(each) | Self::Any(each) => each.iter().find_map(Self::position),
            Self::Not(not) => not.position(),
            _ => None,
        }
    }
    /// The nearest bound of its time atoms after `now` (Unix ms): where
    /// its truth can change by the clock alone.
    pub fn next_time(&self, now: f64) -> Option<f64> {
        match self {
            Self::Time(interval) => [interval.min, interval.max]
                .into_iter()
                .flatten()
                .filter(|bound| *bound > now)
                .min_by(f64::total_cmp),
            Self::All(each) | Self::Any(each) => each
                .iter()
                .filter_map(|c| c.next_time(now))
                .min_by(f64::total_cmp),
            Self::Not(not) => not.next_time(now),
            _ => None,
        }
    }
}

/// A numeric atom's `{ min?, max? }`.
fn interval(key: &str, value: &Value) -> Result<Interval, Refusal> {
    let shape = || format!("condition {key} is {{ min?, max? }} with finite numbers, min < max");
    let object = value
        .as_object()
        .filter(|o| !o.is_empty() && o.keys().all(|k| k == "min" || k == "max"))
        .ok_or_else(shape)?;
    let bound = |name: &str| match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(number) => number
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(shape),
    };
    let interval = Interval {
        min: bound("min")?,
        max: bound("max")?,
    };
    match (interval.min, interval.max) {
        (None, None) => Err(shape().into()),
        (Some(min), Some(max)) if min >= max => Err(shape().into()),
        _ => Ok(interval),
    }
}

/// A position atom's `{ ranges: [[start, end?], ..], lead? }`.
fn range_list(value: &Value) -> Result<RangeList, Refusal> {
    let shape = "condition position is { ranges: [[start, end?], ...], lead? }";
    let object = value
        .as_object()
        .filter(|o| o.keys().all(|k| k == "ranges" || k == "lead"))
        .ok_or(shape)?;
    let ranges = object
        .get("ranges")
        .and_then(Value::as_array)
        .ok_or(shape)?;
    if ranges.len() > MAX_RANGES {
        return Err(Refusal {
            code: "limit",
            message: format!("a position list holds at most {MAX_RANGES} ranges"),
        });
    }
    let lead = match object.get("lead") {
        None | Some(Value::Null) => 0.,
        Some(lead) => lead
            .as_f64()
            .filter(|lead| lead.is_finite() && *lead >= 0.)
            .ok_or("condition position lead is a number of seconds, 0 or more")?,
    };
    let given = ranges
        .iter()
        .map(|range| {
            let pair = range
                .as_array()
                .filter(|pair| (1..=2).contains(&pair.len()))
                .ok_or(shape)?;
            let start = pair[0]
                .as_f64()
                .filter(|start| start.is_finite() && *start >= 0.)
                .ok_or("a range starts at a finite number of seconds, 0 or more")?;
            let end = match pair.get(1) {
                None | Some(Value::Null) => None,
                Some(end) => Some(
                    end.as_f64()
                        .filter(|end| end.is_finite() && *end > start)
                        .ok_or("a range ends after its start")?,
                ),
            };
            Ok((start, end))
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    Ok(RangeList::new(given, lead))
}

/// A command's conditions and the reason its plugin sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Conditions {
    pub enabled: Option<Condition>,
    pub checked: Option<Condition>,
    /// The state's `reason` as sent (translated): with an `enabled`
    /// condition it shows only while the command is disabled or busy.
    pub reason: Option<String>,
}
impl Conditions {
    /// `when` of a register or update: `None` when absent or empty.
    pub fn parse(value: &Value) -> Result<Option<Self>, String> {
        if value.is_null() {
            return Ok(None);
        }
        let object = value.as_object().ok_or("when is { enabled?, checked? }")?;
        if let Some(key) = object.keys().find(|k| *k != "enabled" && *k != "checked") {
            return Err(format!("when has no field {key}: only enabled and checked"));
        }
        let field = |name: &str| {
            object
                .get(name)
                .map(|value| {
                    let condition = Condition::parse(value)?;
                    if condition.groups() & (groups::POSITION | groups::TIME) != 0 {
                        return Err(format!(
                            "{name}: a position or time condition is for a trigger (plugin.trigger), not a command's state"
                        ));
                    }
                    Ok(condition)
                })
                .transpose()
        };
        let (enabled, checked) = (field("enabled")?, field("checked")?);
        Ok((enabled.is_some() || checked.is_some()).then_some(Self {
            enabled,
            checked,
            reason: None,
        }))
    }
    /// The state shown: `state` (as sent) with the condition fields
    /// evaluated against `facts`.
    pub fn apply(
        &self,
        state: &crate::commands::CommandState,
        facts: &Facts,
    ) -> crate::commands::CommandState {
        let mut shown = state.clone();
        if let Some(enabled) = &self.enabled {
            shown.enabled = enabled.holds(facts);
            shown.reason = (!shown.enabled || shown.busy)
                .then(|| self.reason.clone())
                .flatten();
        }
        if let Some(checked) = &self.checked {
            shown.checked = Some(checked.holds(facts));
        }
        shown
    }
}

#[cfg(test)]
#[path = "conditions_tests.rs"]
mod tests_a2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandState;
    use serde_json::json;

    fn facts() -> Facts {
        Facts::new(
            &json!({"phase":"ready","paused":false,"kind":"file",
                "capabilities":{"pause":true,"seek":true,"snapshotSource":true},
                "sequence":{"next":true,"previous":false}}),
            &json!({"fullscreen":false}),
            "vestra",
            "system",
        )
    }

    #[test]
    fn facts_read_the_wire_state_in_the_sdk_names() {
        let facts = facts();
        assert_eq!(facts.phase, "ready");
        assert_eq!(facts.kind, "media");
        assert_eq!(facts.can, ["pause", "seek", "snapshot", "next"]);
        let empty = Facts::new(&Value::Null, &Value::Null, "vestra", "system");
        assert_eq!((empty.phase.as_str(), empty.fullscreen), ("empty", false));
        assert!(empty.can.is_empty());
        let stream = Facts::new(&json!({"kind":"producer"}), &Value::Null, "", "");
        assert_eq!(stream.kind, "stream");
    }

    #[test]
    fn conditions_evaluate_and_combine() {
        let facts = facts();
        let holds = |value: Value| Condition::parse(&value).unwrap().holds(&facts);
        assert!(holds(json!({"phase":["ready","ended"]})));
        assert!(!holds(json!({"phase":["empty"]})));
        assert!(holds(json!({"paused":false})));
        assert!(holds(json!({"kind":"media"})));
        assert!(holds(json!({"can":"snapshot"})));
        assert!(!holds(json!({"can":"rate"})));
        assert!(!holds(json!({"fullscreen":true})));
        assert!(holds(json!({"theme":"vestra"})));
        assert!(holds(json!({"language":"system"})));
        // `when.playing`, and capture's "a frame to save".
        assert!(holds(json!({"all":[{"phase":["ready"]},{"paused":false}]})));
        assert!(holds(
            json!({"all":[{"can":"snapshot"},{"phase":["ready","ended"]},
            {"not":{"kind":"stream"}}]})
        ));
        assert!(holds(
            json!({"any":[{"fullscreen":true},{"theme":"vestra"}]})
        ));
        assert!(!holds(json!({"not":{"theme":"vestra"}})));
    }

    #[test]
    fn malformed_conditions_are_refused() {
        for bad in [
            json!(true),
            json!({}),
            json!({"phase":"ready"}),
            json!({"phase":["playing"]}),
            json!({"paused":1}),
            json!({"kind":"file"}),
            json!({"can":"fly"}),
            json!({"theme":""}),
            json!({"theme":"dark"}),
            json!({"language":"xx"}),
            json!({"all":[]}),
            json!({"phase":["ready"],"paused":true}),
            json!({"source":true}),
        ] {
            assert!(Condition::parse(&bad).is_err(), "{bad}");
        }
        let mut deep = json!({"paused":true});
        for _ in 0..8 {
            deep = json!({ "not": deep });
        }
        assert!(Condition::parse(&deep).is_err());
        let wide = json!({"any": vec![json!({"paused":true}); 40]});
        assert!(Condition::parse(&wide).is_err());
        assert!(Conditions::parse(&json!({"busy":{"paused":true}})).is_err());
        assert_eq!(Conditions::parse(&json!({})).unwrap(), None);
        assert_eq!(Conditions::parse(&Value::Null).unwrap(), None);
    }

    #[test]
    fn an_enabled_condition_shows_the_reason_only_while_disabled() {
        let conditions = Conditions {
            reason: Some("No frame to capture".into()),
            ..Conditions::parse(
                &json!({"enabled":{"phase":["ready"]},"checked":{"fullscreen":true}}),
            )
            .unwrap()
            .unwrap()
        };
        let sent = CommandState {
            enabled: true,
            checked: Some(true),
            busy: false,
            reason: Some("No frame to capture".into()),
        };
        let ready = conditions.apply(&sent, &facts());
        assert_eq!(
            (ready.enabled, ready.checked, ready.reason),
            (true, Some(false), None)
        );
        let empty = Facts {
            phase: "empty".into(),
            ..facts()
        };
        let shown = conditions.apply(&sent, &empty);
        assert!(!shown.enabled);
        assert_eq!(shown.reason.as_deref(), Some("No frame to capture"));
    }
}
