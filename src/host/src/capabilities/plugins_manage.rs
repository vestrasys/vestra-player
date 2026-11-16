//! `plugins.manage`: installing, enabling, reloading and uninstalling other
//! plugins; the RPC face of package_manager.rs, which refuses the methods it
//! does not know. `packages.list` needs no capability (uncapable.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct PluginsManage;
static INFO: Info = Info {
    name: "plugins.manage",
    builtin_only: false,
};
static METHODS: &[&str] = &["packages.*"];
impl Capability for PluginsManage {
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
        match method {
            // Answered later, off the UI thread (package_validate.rs).
            "packages.validate" | "packages.validateArchive" => {
                core.validate_package(reply, method, params).map(|()| None)
            }
            _ => core.package_dispatch(reply.owner, method, params).map(Some),
        }
    }
}
