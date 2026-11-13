//! `clipboard`: reading and writing the system clipboard (crate::clipboard).
use super::{CLIPBOARD, Group, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply, RpcError};
use serde_json::Value;

pub struct Clipboard;
static INFO: Info = Info {
    name: "clipboard",
    summary: "Read and write the clipboard",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::needs("clipboard.readText", CLIPBOARD),
    Method::needs("clipboard.writeText", CLIPBOARD),
    Method::needs("clipboard.writeImage", CLIPBOARD),
];
impl Group for Clipboard {
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
