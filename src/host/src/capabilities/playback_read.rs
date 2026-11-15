//! `playback.read`: what is playing, the position, chapters, devices, stats
//! and timeline marks; the playback worker answers most of it.
use super::{Capability, Handled, Info, Recorded, forward};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct PlaybackRead;
static INFO: Info = Info {
    name: "playback.read",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "source.state",
    "processing.state",
    "playback.stats",
    "player.chapters",
    "player.audioDevices",
    "timeline.marks",
];
impl Capability for PlaybackRead {
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
        match method {
            "timeline.marks" => host
                .core
                .timeline_dispatch(reply.owner, method, params)
                .map(Some),
            _ => {
                if method == "source.state" {
                    host.core
                        .source_state_replies
                        .insert((reply.owner, reply.id));
                }
                forward(host, reply, method, params)
            }
        }
    }
}
