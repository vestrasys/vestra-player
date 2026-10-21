//! Timeline mark layers (R-M3, `plugin.timeline`): one layer per plugin,
//! merged in time order for every reader. A new source clears every layer;
//! a plugin's layer goes when it stops.
#![forbid(unsafe_code)]
use super::*;

pub const MAX_MARKS: usize = 1000;
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

#[derive(Default)]
pub struct Timeline {
    layers: BTreeMap<u64, Vec<Mark>>,
    revision: u64,
    sent: u64,
    sent_at: Option<u64>,
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
            let title = match &mark["title"] {
                Value::Null => None,
                Value::String(title) if title.chars().count() <= MAX_TITLE => Some(title.clone()),
                _ => {
                    return Err(format!(
                        "title must be a string of ≤ {MAX_TITLE} characters"
                    ));
                }
            };
            Ok(Mark {
                time,
                kind,
                title,
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
    pub fn remove_owner(&mut self, owner: u64) {
        self.set(owner, Vec::new());
    }
    pub fn clear(&mut self) {
        if !self.layers.is_empty() {
            self.layers.clear();
            self.revision += 1;
        }
    }
    /// All layers, by time; equal times keep layer (owner) and list order.
    pub fn merged(&self) -> Vec<Mark> {
        let mut marks: Vec<Mark> = self.layers.values().flatten().cloned().collect();
        marks.sort_by(|a, b| a.time.total_cmp(&b.time));
        marks
    }
    /// The merged list when it changed and the last push is ≥ 100 ms old.
    pub fn due(&mut self, now: u64) -> Option<Vec<Mark>> {
        if self.revision == self.sent || self.sent_at.is_some_and(|at| now < at + PUSH_MS) {
            return None;
        }
        self.sent = self.revision;
        self.sent_at = Some(now);
        Some(self.merged())
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
            return Ok(json!({"marks":self.timeline.merged()}));
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
        let marks = parse(&params["marks"], &manifest, duration).map_err(RpcError::invalid)?;
        self.timeline.set(owner, marks);
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
        if let Some(marks) = self.timeline.due(now) {
            self.broadcast("timeline.changed", json!({"marks":marks}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(time: f64, kind: &str) -> Value {
        json!({"time":time,"kind":kind})
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
        assert_eq!(timeline.due(0).unwrap().len(), 3);
        // Unchanged: nothing to push; changed within 100 ms: wait.
        timeline.set(1, a);
        assert!(timeline.due(500).is_none());
        timeline.remove_owner(2);
        assert_eq!(timeline.due(510).unwrap().len(), 2);
        timeline.remove_owner(1);
        assert!(timeline.due(550).is_none());
        assert!(timeline.due(610).unwrap().is_empty());
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
        assert!(timeline.due(800).unwrap().is_empty());
        timeline.clear();
        assert!(timeline.due(1000).is_none());
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
