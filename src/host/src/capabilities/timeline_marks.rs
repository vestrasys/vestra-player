//! `timeline.marks`: a plugin's layer of marks and ranges on the progress
//! bar (timeline.rs). Reading them is `playback.read` (playback_read.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct TimelineMarks;
static INFO: Info = Info {
    name: "timeline.marks",
    builtin_only: false,
};
static METHODS: &[&str] = &["timeline.set", "timeline.setRanges"];
impl Capability for TimelineMarks {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// A layer is plain data the host draws by itself: kept through standby
    /// by the installed instance, it holds nothing that keeps the plugin
    /// running (events-and-triggers.md §4.8; `Host::stop_run` keeps it at a
    /// park, removes it at any other stop).
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        core.timeline.rekey(from, to);
    }
    /// A held owner's standby ends for good (disabled, uninstalled,
    /// reloaded, upgraded, or its wake failed): its kept layer goes.
    fn revoke(&self, core: &mut Core, owner: u64, keep_recorded: bool) {
        if !keep_recorded {
            core.timeline.remove_owner(owner);
        }
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        core.timeline_dispatch(reply.owner, method, params)
            .map(Some)
    }
}
