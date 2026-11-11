//! `plugins.manage`: installing, enabling, reloading and uninstalling other
//! plugins; the RPC face of package_manager.rs, which refuses the methods it
//! does not know. `packages.list` needs no capability (uncapable.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct PluginsManage;
static INFO: Info = Info {
    name: "plugins.manage",
    label: "Install, uninstall and disable other plugins",
    risky: true,
    requires: &[],
    residency: Residency::None,
    builtin_only: false,
};
static METHODS: &[Method] = &[Method::this("packages.*")];
impl Capability for PluginsManage {
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
        match method {
            // Answered later, off the UI thread (package_validate.rs).
            "packages.validate" | "packages.validateArchive" => {
                host.validate_package(reply, method, params).map(|()| None)
            }
            _ => host.package_dispatch(reply.owner, method, params).map(Some),
        }
    }
}
