//! `subtitles.read`: the subtitle tracks and their state.
use super::{Capability, Handled, Info, Method, Residency, forward_media};
use crate::{Host, Reply};
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
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        forward_media(host, reply, method, params)
    }
}
