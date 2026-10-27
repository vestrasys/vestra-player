//! `processing.video`: the video filter chain, the video view and
//! interpolated rendering. Also serves the requests the two filter chains
//! share, which ask for `processing.video` or `processing.audio` by stage.
use super::{Capability, Handled, Info, Method, Needs, Residency, forward, forward_media};
use crate::{Host, Reply};
use serde_json::Value;

pub struct ProcessingVideo;
static INFO: Info = Info {
    name: "processing.video",
    label: "Process the picture (filters, shaders)",
    risky: false,
    requires: &["playback.read"],
    residency: Residency::Live,
};
const BY_STAGE: Needs = Needs::Param {
    key: "stage",
    cases: &[("video", Some("processing.video"))],
    otherwise: Some("processing.audio"),
};
static METHODS: &[Method] = &[
    Method {
        name: "processing.replace",
        needs: BY_STAGE,
    },
    Method {
        name: "processing.operation",
        needs: BY_STAGE,
    },
    Method::this("video.view.set"),
    Method::this("video.view.release"),
    Method::this("video.render.set"),
    Method::this("video.render.release"),
];
impl Capability for ProcessingVideo {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "processing.replace" | "processing.operation" => forward(host, reply, method, params),
            _ => forward_media(host, reply, method, params),
        }
    }
}
