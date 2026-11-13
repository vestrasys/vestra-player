//! `playback.read`: what is playing, the position, chapters, devices, stats
//! and timeline marks; the playback worker answers most of it.
use super::{Group, Handled, Info, Method, Recorded, Residency, forward};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct PlaybackRead;
static INFO: Info = Info {
    name: "playback.read",
    summary: "Read what is playing and the position",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::free("source.state"),
    Method::free("processing.state"),
    Method::free("playback.stats"),
    Method::free("player.chapters"),
    Method::free("player.audioDevices"),
    Method::free("timeline.marks"),
];
impl Group for PlaybackRead {
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
