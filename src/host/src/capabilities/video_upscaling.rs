//! `video.upscaling`: the GPU's video upscaler (NVIDIA RTX Video Super
//! Resolution on Windows): whether it works here and why not, and turning
//! it on or off for this plugin (`playback/upscaling.rs`, SDK 1.5.37, player
//! 0.5.16; doc/architecture/README.md §4.4). The control worker answers;
//! the host adds its rendering state, for the advice in a `rendering`
//! reason.
//!
//! Not risky: it changes only how the picture is scaled while the plugin
//! runs, touches no data, and a stopped plugin's request is revoked (as a
//! `processing.video` filter, which is not risky either). The GPU's power
//! use is what the user turns it on for.
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply, RpcError};
use crate::playback::backend::Command;
use serde_json::{Value, json};

pub struct VideoUpscaling;
static INFO: Info = Info {
    name: "video.upscaling",
    builtin_only: false,
};
static METHODS: &[&str] = &["video.upscaling.state", "video.upscaling.set"];
impl Capability for VideoUpscaling {
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
        // Only the fields the worker reads; the rendering state is the
        // host's, never the plugin's.
        let mut forwarded = json!({"rendering": host.rendering_state()});
        if method == "video.upscaling.set" {
            forwarded["enabled"] = params["enabled"].clone();
        }
        host.backend
            .commands
            .send(Command {
                reply: reply.clone(),
                method: method.into(),
                params: forwarded,
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        host.worker_requested(reply.owner);
        Ok(None)
    }
}
