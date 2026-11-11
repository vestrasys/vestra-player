//! `bindings.manage`: reading and changing every shortcut
//! (binding_manager.rs). `events.subscribe` of `bindings.changed` asks for
//! it too (events.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct BindingsManage;
static INFO: Info = Info {
    name: "bindings.manage",
    label: "Change all shortcuts",
    risky: true,
    requires: &[],
    residency: Residency::None,
    builtin_only: false,
};
static METHODS: &[Method] = &[
    Method::this("bindings.list"),
    Method::this("bindings.set"),
    Method::this("bindings.reset"),
];
impl Capability for BindingsManage {
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
