//! `producer.av`: streams a plugin generates itself (producer.rs).
use super::{Group, Handled, Info, Method, Recorded, Residency, forward};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct ProducerAv;
static INFO: Info = Info {
    name: "producer.av",
    summary: "Provide audio and video it generates itself",
    residency: Residency::Live,
};
static METHODS: &[Method] = &[
    Method::free("producer.prepare"),
    Method::free("producer.state"),
    Method::free("producer.release"),
    Method::free("producer.ack"),
    Method::free("producer.declareInput"),
    Method::free("producer.ackInput"),
];
impl Group for ProducerAv {
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
