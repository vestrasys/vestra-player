//! `media.control`: taking snapshots and running export jobs (media_jobs.rs,
//! in the playback worker, which refuses the methods it does not know). An
//! export's input is a snapshot or the parts of remembered files
//! (`media.sources`, also required for those).
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply, RpcError};
use serde_json::Value;

pub struct MediaControl;
static INFO: Info = Info {
    name: "media.control",
    label: "Take snapshots and export clips",
    risky: false,
    requires: &["media.read"],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[Method::this("snapshot.*"), Method::this("jobs.*")];
impl Capability for MediaControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        // An export of remembered files' parts (`jobs.start { parts }`) and
        // its check (`jobs.check { parts }`) read the plugin's source
        // references: they need `media.sources` too, and carry the plugin's
        // store key. A snapshot's check (`jobs.check { snapshotId }`) does not.
        let parts = !params["parts"].is_null();
        if matches!(method, "jobs.check" | "jobs.start")
            && parts
            && !super::plugin(host, reply)?.allows("media.sources")
        {
            return Err(RpcError::new(
                "capability",
                "capability must be declared in manifest: media.sources (an export of parts)",
            ));
        }
        // Every check carries the store key: a snapshot's check counts in the
        // plugin's share of the request threads as its parts checks do.
        if method == "jobs.check" || (method == "jobs.start" && parts) {
            return super::media_sources::forward_with_store(host, reply, method, params);
        }
        forward_media(host, reply, method, params)
    }
}
