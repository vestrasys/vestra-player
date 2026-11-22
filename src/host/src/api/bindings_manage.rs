//! `bindings.manage`: reading and changing every shortcut
//! (binding_manager.rs). `events.subscribe` of `bindings.changed` asks for
//! it too (events.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct BindingsManage;
static INFO: Info = Info {
    name: "bindings.manage",
    builtin_only: false,
};
static METHODS: &[&str] = &["bindings.list", "bindings.set", "bindings.reset"];
impl Capability for BindingsManage {
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
    fn handle(&self, core: &mut Core, _: &Reply, method: &str, params: &Value) -> Handled {
        core.bindings_dispatch(method, params).map(Some)
    }
}
