//! `media.control`: taking snapshots and running export jobs (media_jobs.rs,
//! in the playback worker, which refuses the methods it does not know). An
//! export's input is a snapshot or the parts of remembered files
//! (`media.sources`).
use super::{Capability, Handled, Info, Recorded, forward_media};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct MediaControl;
static INFO: Info = Info {
    name: "media.control",
    builtin_only: false,
};
static METHODS: &[&str] = &["snapshot.*", "jobs.*"];
impl Capability for MediaControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        // An export of remembered files' parts (`jobs.start { parts }`) and
        // its check (`jobs.check { parts }`) read the plugin's source
        // references, so they carry its store key; a snapshot's check
        // (`jobs.check { snapshotId }`) carries it as well (below).
        let parts = !params["parts"].is_null();
        // Every check carries the store key: a snapshot's check counts in the
        // plugin's share of the request threads as its parts checks do.
        if method == "jobs.check" || (method == "jobs.start" && parts) {
            return super::media_sources::forward_with_store(core, reply, method, params);
        }
        forward_media(core, reply, method, params)
    }
}
