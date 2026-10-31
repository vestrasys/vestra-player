//! `subtitles.read`: the subtitle tracks and their state.
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct SubtitlesRead;
static INFO: Info = Info {
    name: "subtitles.read",
    label: "Read subtitles",
    risky: false,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[Method::this("subtitles.state")];
impl Capability for SubtitlesRead {
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
