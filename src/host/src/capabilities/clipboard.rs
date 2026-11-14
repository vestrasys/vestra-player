//! `clipboard`: reading and writing the system clipboard (crate::clipboard).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply, RpcError};
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
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, _: &Reply, method: &str, params: &Value) -> Handled {
        if host.headless {
            return Err(RpcError::new(
                "unavailable",
                "the clipboard is unavailable in headless mode",
            ));
        }
        crate::clipboard::dispatch(method, params).map(Some)
    }
}
