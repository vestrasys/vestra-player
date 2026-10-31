//! `media.control`: taking snapshots and running export jobs (media_jobs.rs,
//! in the playback worker, which refuses the methods it does not know).
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
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
        forward_media(host, reply, method, params)
    }
}
