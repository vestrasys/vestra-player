//! `video.motion`: motion smoothing, the video's frame rate doubled by the
//! GPU's frame-rate converter (AMD AMF on Windows): whether it works here
//! and why not, and the mode this plugin asks for (`playback/motion.rs`,
//! SDK 1.7.19, player 0.8.1; doc/architecture/README.md §4.4a). The control
//! worker answers; the host adds its rendering state, for the advice in a
//! `rendering` reason.
//!
//! Not risky: it changes only how the picture moves while the plugin runs,
//! touches no data, and a stopped plugin's request is revoked (as
//! `video.upscaling`). The GPU's power use is what the user turns it on for.
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply, RpcError};
use crate::playback::backend::Command;
use serde_json::{Value, json};

pub struct VideoMotion;
static INFO: Info = Info {
    name: "video.motion",
    builtin_only: false,
};
static METHODS: &[&str] = &["video.motion.state", "video.motion.set"];
impl Capability for VideoMotion {
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
        let mut forwarded =
            json!({"rendering": host.rendering_state(), "engine": params["engine"].clone()});
        if method == "video.motion.set" {
            forwarded["mode"] = params["mode"].clone();
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
