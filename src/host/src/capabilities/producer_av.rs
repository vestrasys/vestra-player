//! `producer.av`: streams a plugin generates itself (producer.rs).
use super::{Capability, Handled, Info, Recorded, forward};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct ProducerAv;
static INFO: Info = Info {
    name: "producer.av",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "producer.prepare",
    "producer.state",
    "producer.release",
    "producer.ack",
    "producer.declareInput",
    "producer.ackInput",
];
impl Capability for ProducerAv {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        forward(core, reply, method, params)
    }
}
