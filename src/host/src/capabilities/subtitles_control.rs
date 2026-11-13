//! `subtitles.control`: loading, selecting, removing and styling subtitles
//! (subtitle_backend.rs, in the playback worker, which refuses the methods
//! it does not know).
use super::{Group, Handled, Info, Method, Recorded, Residency, forward_media};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct SubtitlesControl;
static INFO: Info = Info {
    name: "subtitles.control",
    summary: "Load and change subtitles",
    residency: Residency::Live,
};
static METHODS: &[Method] = &[Method::free("subtitles.*")];
impl Group for SubtitlesControl {
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
