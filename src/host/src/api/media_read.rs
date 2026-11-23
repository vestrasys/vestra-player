//! `media.read`: the state of snapshots and export jobs.
use super::{Capability, Handled, Info, Recorded, forward_media};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct MediaRead;
static INFO: Info = Info {
    name: "media.read",
    builtin_only: false,
};
static METHODS: &[&str] = &["snapshot.state", "jobs.state", "jobs.operation"];
impl Capability for MediaRead {
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
        forward_media(core, reply, method, params)
    }
}
