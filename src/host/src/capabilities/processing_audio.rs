//! `processing.audio`: the audio filter chain. Its requests
//! (`processing.replace` and `processing.operation` for the audio stage) are
//! served by processing_video.rs, which asks for this capability by stage.
use super::{Capability, Handled, Info, Method, Residency};
use crate::{Host, Reply, RpcError};
use serde_json::Value;

pub struct ProcessingAudio;
static INFO: Info = Info {
    name: "processing.audio",
    label: "Process the sound",
    risky: false,
    requires: &["playback.read", "events"],
    residency: Residency::Live,
};
impl Capability for ProcessingAudio {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        &[]
    }
    fn handle(&self, _: &mut Host, _: &Reply, _: &str, _: &Value) -> Handled {
        // Never routed here: this capability owns no method.
        Err(RpcError::new("unsupported", "unknown method"))
    }
}
