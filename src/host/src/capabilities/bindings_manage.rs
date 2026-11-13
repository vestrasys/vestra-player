//! `bindings.manage`: reading and changing every shortcut
//! (binding_manager.rs); capability `player`. The `bindings.changed` event
//! is open to every plugin (events.rs).
use super::{Group, Handled, Info, Method, PLAYER, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct BindingsManage;
static INFO: Info = Info {
    name: "bindings.manage",
    summary: "Read and change all shortcuts",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::needs("bindings.list", PLAYER),
    Method::needs("bindings.set", PLAYER),
    Method::needs("bindings.reset", PLAYER),
];
impl Group for BindingsManage {
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
        host.bindings_dispatch(method, params).map(Some)
    }
}
