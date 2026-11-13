//! `media.control`: taking snapshots and running export jobs (media_jobs.rs,
//! in the playback worker, which refuses the methods it does not know). An
//! export's input is a snapshot or the parts of remembered files
//! (media_sources.rs).
use super::{Group, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct MediaControl;
static INFO: Info = Info {
    name: "media.control",
    summary: "Take snapshots and export clips",
    residency: Residency::Live,
};
static METHODS: &[Method] = &[Method::free("snapshot.*"), Method::free("jobs.*")];
impl Group for MediaControl {
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
        // references, under its store key. Every check carries the key: a
        // snapshot's check (`jobs.check { snapshotId }`) counts in the
        // plugin's share of the request threads as its parts checks do.
        let parts = !params["parts"].is_null();
        if method == "jobs.check" || (method == "jobs.start" && parts) {
            return super::media_sources::forward_with_store(host, reply, method, params);
        }
        forward_media(host, reply, method, params)
    }
}
