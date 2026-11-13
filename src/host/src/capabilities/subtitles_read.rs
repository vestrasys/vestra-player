//! `subtitles.read`: the subtitle tracks and their state.
use super::{Capability, Handled, Info, Recorded, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct SubtitlesRead;
static INFO: Info = Info {
    name: "subtitles.read",
    builtin_only: false,
};
static METHODS: &[&str] = &["subtitles.state"];
impl Capability for SubtitlesRead {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
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
