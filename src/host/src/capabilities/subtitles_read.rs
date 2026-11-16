//! `subtitles.read`: the subtitle tracks and their state.
use super::{Capability, Handled, Info, Recorded, forward_media};
use crate::host::{Core, Reply};
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
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        forward_media(core, reply, method, params)
    }
}
