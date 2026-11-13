//! `subtitles.control`: loading, selecting, removing and styling subtitles
//! (subtitle_backend.rs, in the playback worker, which refuses the methods
//! it does not know).
use super::{Capability, Handled, Info, Recorded, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct SubtitlesControl;
static INFO: Info = Info {
    name: "subtitles.control",
    builtin_only: false,
};
static METHODS: &[&str] = &["subtitles.*"];
impl Capability for SubtitlesControl {
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
