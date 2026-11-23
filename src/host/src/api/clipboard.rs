//! `clipboard`: reading and writing the system clipboard (crate::clipboard).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply, RpcError};
use serde_json::Value;

pub struct Clipboard;
static INFO: Info = Info {
    name: "clipboard",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "clipboard.readText",
    "clipboard.writeText",
    "clipboard.writeImage",
];
impl Capability for Clipboard {
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
        if core.headless {
            return Err(RpcError::new(
                "unavailable",
                "the clipboard is unavailable in headless mode",
            ));
        }
        // On the serial clipboard thread (plan step S3a(3)); answered from
        // its reply event.
        let events = core.events.clone();
        core.clipboard
            .call(&events, reply.clone(), method, params.clone())?;
        Ok(None)
    }
}
