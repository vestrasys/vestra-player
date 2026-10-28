//! `playback.read`: what is playing, the position, chapters, devices, stats
//! and timeline marks; the playback worker answers most of it.
use super::{Capability, Handled, Info, Method, Residency, forward};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct PlaybackRead;
static INFO: Info = Info {
    name: "playback.read",
    label: "Read what is playing and the position",
    risky: false,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("source.state"),
    Method::this("processing.state"),
    Method::this("playback.stats"),
    Method::this("player.chapters"),
    Method::this("player.audioDevices"),
    Method::this("timeline.marks"),
];
impl Capability for PlaybackRead {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "timeline.marks" => host
                .timeline_dispatch(reply.owner, method, params)
                .map(Some),
            _ => {
                if method == "source.state" {
                    host.source_state_replies.insert((reply.owner, reply.id));
                }
                forward(host, reply, method, params)
            }
        }
    }
}
