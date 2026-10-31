//! `media.read`: the state of snapshots and export jobs.
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct MediaRead;
static INFO: Info = Info {
    name: "media.read",
    label: "Take snapshots and export clips",
    risky: false,
    requires: &["media.control"],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("snapshot.state"),
    Method::this("jobs.state"),
    Method::this("jobs.operation"),
];
impl Capability for MediaRead {
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
