//! `producer.av`: streams a plugin generates itself (producer.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct ProducerAv;
static INFO: Info = Info {
    name: "producer.av",
    label: "Provide audio and video it generates itself",
    risky: false,
    requires: &["playback.control"],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[
    Method::this("producer.prepare"),
    Method::this("producer.state"),
    Method::this("producer.release"),
    Method::this("producer.ack"),
    Method::this("producer.declareInput"),
    Method::this("producer.ackInput"),
];
impl Capability for ProducerAv {
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
        forward(host, reply, method, params)
    }
}
