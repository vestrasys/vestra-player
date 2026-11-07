//! Timeline mark layers (R-M3, `plugin.timeline`): one layer per plugin,
//! merged in time order for every reader. A new source clears every layer;
//! a plugin's layer goes when it stops. A layer holds point marks and
//! (player 0.7.6) ranges, each list replaced on its own (`timeline.set`,
//! `timeline.setRanges`); readers get both lists.
#![forbid(unsafe_code)]
use crate::host::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MAX_MARKS: usize = 1000;
/// Ranges in one layer: a seek line with more bands than this is
/// unreadable, and the reader draws every one.
pub const MAX_RANGES: usize = 100;
pub const MAX_TITLE: usize = 100;
/// `timeline.changed` is pushed at most this often.
const PUSH_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Mark {
    pub time: f64,
    pub kind: &'static str,
    pub title: Option<String>,
    /// The publisher's manifest id.
    pub plugin: String,
}

/// A span of the media on the seek line (`timeline.setRanges`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
    /// `selection` (a span the user picked to act on, such as a clip to
    /// export) or `region` (a part of the media the plugin describes).
    pub style: &'static str,
    pub title: Option<String>,
    /// The publisher's manifest id.
    pub plugin: String,
}

#[derive(Default)]
pub struct Timeline {
    layers: BTreeMap<u64, Vec<Mark>>,
    ranges: BTreeMap<u64, Vec<Range>>,
    revision: u64,
    sent: u64,
    sent_at: Option<u64>,
}

fn title(value: &Value) -> std::result::Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(title) if title.chars().count() <= MAX_TITLE => Ok(Some(title.clone())),
        _ => Err(format!(
            "title must be a string of ≤ {MAX_TITLE} characters"
        )),
    }
}

/// Validate one `layer.set` list: ≤ 1000 marks, known kinds, titles ≤ 100
/// characters, times within `[0, duration]` (only ≥ 0 while unknown).
pub fn parse(
    marks: &Value,
    manifest: &str,
    duration: Option<f64>,
) -> std::result::Result<Vec<Mark>, String> {
    let list = marks.as_array().ok_or("marks must be a list")?;
    if list.len() > MAX_MARKS {
        return Err(format!("a layer holds at most {MAX_MARKS} marks"));
    }
    list.iter()
        .map(|mark| {
            let time = mark["time"]
                .as_f64()
                .filter(|t| t.is_finite() && *t >= 0.)
                .ok_or("time must be a number ≥ 0")?;
            if duration.is_some_and(|d| time > d + 1e-6) {
                return Err(format!("time {time} is after the end of the media"));
            }
            let kind = match mark["kind"].as_str() {
                Some("chapter") => "chapter",
                Some("bookmark") => "bookmark",
                Some("marker") => "marker",
                _ => return Err("kind is chapter, bookmark or marker".into()),
            };
            Ok(Mark {
                time,
                kind,
                title: title(&mark["title"])?,
                plugin: manifest.into(),
            })
        })
        .collect()
}

/// Validate one `layer.setRanges` list: ≤ 100 ranges, known styles, titles
/// ≤ 100 characters, `0 ≤ start < end`, and `end` within the media while
/// its duration is known.
pub fn parse_ranges(
    ranges: &Value,
    manifest: &str,
    duration: Option<f64>,
) -> std::result::Result<Vec<Range>, String> {
    let list = ranges.as_array().ok_or("ranges must be a list")?;
    if list.len() > MAX_RANGES {
        return Err(format!("a layer holds at most {MAX_RANGES} ranges"));
    }
    list.iter()
        .map(|range| {
            let time = |name: &str| {
                range[name]
                    .as_f64()
                    .filter(|t| t.is_finite() && *t >= 0.)
                    .ok_or(format!("{name} must be a number ≥ 0"))
            };
            let (start, end) = (time("start")?, time("end")?);
            if end <= start {
                return Err(format!("end {end} must be after start {start}"));
            }
            if duration.is_some_and(|d| end > d + 1e-6) {
                return Err(format!("end {end} is after the end of the media"));
            }
            let style = match range["style"].as_str() {
                Some("selection") => "selection",
                Some("region") => "region",
                _ => return Err("style is selection or region".into()),
            };
            Ok(Range {
                start,
                end,
                style,
                title: title(&range["title"])?,
                plugin: manifest.into(),
            })
        })
        .collect()
}

impl Timeline {
    pub fn set(&mut self, owner: u64, marks: Vec<Mark>) {
        let changed = if marks.is_empty() {
            self.layers.remove(&owner).is_some()
        } else {
            self.layers.insert(owner, marks.clone()).as_ref() != Some(&marks)
        };
        if changed {
            self.revision += 1;
        }
    }
    pub fn set_ranges(&mut self, owner: u64, ranges: Vec<Range>) {
        let changed = if ranges.is_empty() {
            self.ranges.remove(&owner).is_some()
        } else {
            self.ranges.insert(owner, ranges.clone()).as_ref() != Some(&ranges)
        };
        if changed {
            self.revision += 1;
        }
    }
    /// `owner` has a layer (marks or ranges).
    pub fn holds(&self, owner: u64) -> bool {
        self.layers.contains_key(&owner) || self.ranges.contains_key(&owner)
    }
    pub fn remove_owner(&mut self, owner: u64) {
        self.set(owner, Vec::new());
        self.set_ranges(owner, Vec::new());
    }
    pub fn clear(&mut self) {
        if !self.layers.is_empty() || !self.ranges.is_empty() {
            self.layers.clear();
            self.ranges.clear();
            self.revision += 1;
        }
    }
    /// All layers, by time; equal times keep layer (owner) and list order.
    pub fn merged(&self) -> Vec<Mark> {
        let mut marks: Vec<Mark> = self.layers.values().flatten().cloned().collect();
        marks.sort_by(|a, b| a.time.total_cmp(&b.time));
        marks
    }
    /// All layers' ranges, by start; equal starts keep layer and list order.
    pub fn merged_ranges(&self) -> Vec<Range> {
        let mut ranges: Vec<Range> = self.ranges.values().flatten().cloned().collect();
        ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
        ranges
    }
    /// What readers get (`timeline.marks`, `timeline.changed`).
    pub fn snapshot(&self) -> Value {
        json!({"marks":self.merged(),"ranges":self.merged_ranges()})
    }
    /// The merged lists when they changed and the last push is ≥ 100 ms old.
    pub fn due(&mut self, now: u64) -> Option<Value> {
        if self.revision == self.sent || self.sent_at.is_some_and(|at| now < at + PUSH_MS) {
            return None;
        }
        self.sent = self.revision;
        self.sent_at = Some(now);
        Some(self.snapshot())
    }
}

impl Host {
    pub(super) fn timeline_dispatch(
        &mut self,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        if method == "timeline.marks" {
            return Ok(self.timeline.snapshot());
        }
        let source = &self.last_source;
        let generation = source["stamp"]["generation"].as_u64().unwrap_or(0);
        // `ifCurrent`: the marks belong to the source the plugin saw.
        if let Some(expected) = params.get("sourceId").filter(|v| !v.is_null())
            && expected.as_u64() != Some(generation)
        {
            return Ok(json!({"applied":false}));
        }
        let manifest = self
            .plugins
            .get(&owner)
            .map(|plugin| plugin.manifest.id.clone())
            .unwrap_or_default();
        let duration = source["duration"].as_f64().filter(|d| d.is_finite());
        if method == "timeline.setRanges" {
            let ranges =
                parse_ranges(&params["ranges"], &manifest, duration).map_err(RpcError::invalid)?;
            self.timeline.set_ranges(owner, ranges);
        } else {
            let marks = parse(&params["marks"], &manifest, duration).map_err(RpcError::invalid)?;
            self.timeline.set(owner, marks);
        }
        Ok(json!({"applied":true}))
    }
    /// A new source (a different `sourceId`) clears every layer.
    pub(super) fn timeline_source(&mut self, previous: &Value, next: &Value) {
        let generation = |state: &Value| state["stamp"]["generation"].as_u64();
        if generation(previous) != generation(next) {
            self.timeline.clear();
        }
    }
    pub(super) fn timeline_tick(&mut self) {
        let now = self.now_ms();
        if let Some(lists) = self.timeline.due(now) {
            self.broadcast("timeline.changed", lists);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(time: f64, kind: &str) -> Value {
        json!({"time":time,"kind":kind})
    }

    fn range(start: f64, end: f64, style: &str) -> Value {
        json!({"start":start,"end":end,"style":style})
    }

    #[test]
    fn layers_validate_merge_and_push_only_changes() {
        let a = parse(
            &json!([mark(30., "bookmark"), mark(10., "chapter")]),
            "a",
            Some(60.),
        )
        .unwrap();
        assert_eq!(a[1].title, None);
        let b = parse(
            &json!([{"time":10.,"kind":"marker","title":"B"}]),
            "b",
            None,
        )
        .unwrap();
        let mut timeline = Timeline::default();
        timeline.set(2, b);
        timeline.set(1, a.clone());
        let merged = timeline.merged();
        let order: Vec<_> = merged.iter().map(|m| (m.time, m.plugin.as_str())).collect();
        assert_eq!(order, [(10., "a"), (10., "b"), (30., "a")]);
        assert_eq!(
            timeline.due(0).unwrap()["marks"].as_array().unwrap().len(),
            3
        );
        // Unchanged: nothing to push; changed within 100 ms: wait.
        timeline.set(1, a);
        assert!(timeline.due(500).is_none());
        timeline.remove_owner(2);
        assert_eq!(
            timeline.due(510).unwrap()["marks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        timeline.remove_owner(1);
        assert!(timeline.due(550).is_none());
        assert_eq!(timeline.due(610).unwrap(), json!({"marks":[],"ranges":[]}));
        timeline.set(
            1,
            vec![Mark {
                time: 1.,
                kind: "marker",
                title: None,
                plugin: "a".into(),
            }],
        );
        timeline.clear();
        assert_eq!(timeline.due(800).unwrap()["marks"], json!([]));
        timeline.clear();
        assert!(timeline.due(1000).is_none());
    }

    #[test]
    fn ranges_are_their_own_list_of_the_layer() {
        let mut timeline = Timeline::default();
        let clip = parse_ranges(
            &json!([{"start":40.,"end":52.5,"style":"selection","title":"Clip"}]),
            "capture",
            Some(60.),
        )
        .unwrap();
        let intro = parse_ranges(&json!([range(0., 30., "region")]), "intro", None).unwrap();
        assert_eq!(intro[0].title, None);
        timeline.set_ranges(1, clip.clone());
        timeline.set_ranges(2, intro);
        assert!(timeline.holds(1) && timeline.holds(2));
        // Sorted by start, each with its publisher; marks untouched.
        let pushed = timeline.due(0).unwrap();
        assert_eq!(pushed["marks"], json!([]));
        assert_eq!(
            pushed["ranges"],
            json!([
                {"start":0.,"end":30.,"style":"region","title":null,"plugin":"intro"},
                {"start":40.,"end":52.5,"style":"selection","title":"Clip","plugin":"capture"},
            ])
        );
        // Marks and ranges are replaced on their own: an empty mark list
        // keeps the ranges, and the same ranges again push nothing.
        timeline.set(1, Vec::new());
        timeline.set_ranges(1, clip);
        assert!(timeline.due(200).is_none());
        timeline.set(
            1,
            parse(&json!([mark(5., "marker")]), "capture", None).unwrap(),
        );
        assert_eq!(
            timeline.due(300).unwrap()["ranges"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        // Stopping removes both lists of the owner; a new source clears all.
        timeline.remove_owner(1);
        assert!(!timeline.holds(1));
        let pushed = timeline.due(400).unwrap();
        assert_eq!(pushed["marks"], json!([]));
        assert_eq!(pushed["ranges"].as_array().unwrap().len(), 1);
        timeline.clear();
        assert!(!timeline.holds(2));
        assert_eq!(timeline.due(500).unwrap()["ranges"], json!([]));
    }

    #[test]
    fn range_limits_are_invalid() {
        let many: Vec<_> = (0..=MAX_RANGES)
            .map(|i| range(i as f64, i as f64 + 0.5, "region"))
            .collect();
        assert!(parse_ranges(&json!(many[..MAX_RANGES]), "a", None).is_ok());
        assert!(parse_ranges(&json!(many), "a", None).is_err());
        assert!(parse_ranges(&json!([range(0., 60., "selection")]), "a", Some(60.)).is_ok());
        let long = "x".repeat(MAX_TITLE + 1);
        for bad in [
            json!([range(-1., 2., "region")]),
            json!([range(5., 5., "region")]),
            json!([range(6., 5., "region")]),
            json!([range(1., 61., "region")]),
            json!([range(1., 2., "loop")]),
            json!([{"start":1.,"end":2.}]),
            json!([{"start":"1","end":2.,"style":"region"}]),
            json!([{"start":1.,"end":2.,"style":"region","title":long}]),
            json!({"start":1.,"end":2.,"style":"region"}),
        ] {
            assert!(parse_ranges(&bad, "a", Some(60.)).is_err(), "{bad}");
        }
        // Without a duration only the order is checked.
        assert!(parse_ranges(&json!([range(600., 700., "region")]), "a", None).is_ok());
    }

    #[test]
    fn limits_are_invalid() {
        let many: Vec<_> = (0..=MAX_MARKS).map(|i| mark(i as f64, "marker")).collect();
        assert!(parse(&json!(many[..MAX_MARKS]), "a", None).is_ok());
        assert!(parse(&json!(many), "a", None).is_err());
        let title = "字".repeat(MAX_TITLE);
        assert!(
            parse(
                &json!([{"time":0,"kind":"chapter","title":title}]),
                "a",
                None
            )
            .is_ok()
        );
        let long = format!("{title}x");
        assert!(
            parse(
                &json!([{"time":0,"kind":"chapter","title":long}]),
                "a",
                None
            )
            .is_err()
        );
        for bad in [
            json!([mark(-1., "chapter")]),
            json!([mark(61., "chapter")]),
            json!([mark(1., "chapters")]),
            json!([{"time":"1","kind":"chapter"}]),
            json!({"time":1}),
        ] {
            assert!(parse(&bad, "a", Some(60.)).is_err(), "{bad}");
        }
        assert!(parse(&json!([mark(600., "marker")]), "a", None).is_ok());
    }
}
