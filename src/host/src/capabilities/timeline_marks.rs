//! `timeline.marks`: a plugin's layer of marks on the progress bar
//! (timeline.rs). Reading the marks is `playback.read` (playback_read.rs).
use super::{Capability, Handled, Info, Method, Residency};
use crate::{Host, Reply};
use serde_json::Value;

pub struct TimelineMarks;
static INFO: Info = Info {
    name: "timeline.marks",
    label: "Show marks on the progress bar",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[Method::this("timeline.set")];
impl Capability for TimelineMarks {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.timeline_dispatch(reply.owner, method, params)
            .map(Some)
    }
}
