//! `subtitles.read`: the subtitle tracks and their state.
use super::{Group, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct SubtitlesRead;
static INFO: Info = Info {
    name: "subtitles.read",
    summary: "Read subtitles",
    residency: Residency::None,
};
static METHODS: &[Method] = &[Method::free("subtitles.state")];
impl Group for SubtitlesRead {
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
