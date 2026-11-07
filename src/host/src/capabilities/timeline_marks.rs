//! `timeline.marks`: a plugin's layer of marks and ranges on the progress
//! bar (timeline.rs). Reading them is `playback.read` (playback_read.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct TimelineMarks;
static INFO: Info = Info {
    name: "timeline.marks",
    label: "Show marks on the progress bar",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[
    Method::this("timeline.set"),
    Method::this("timeline.setRanges"),
];
impl Capability for TimelineMarks {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.timeline.holds(owner) {
            return Recorded::Live("a timeline layer");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.timeline_dispatch(reply.owner, method, params)
            .map(Some)
    }
}
