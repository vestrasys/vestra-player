//! `plugins.manage`: installing, enabling, reloading and uninstalling other
//! plugins; the RPC face of package_manager.rs, which refuses the methods it
//! does not know. `packages.list` needs no capability (uncapable.rs).
use super::{Capability, Handled, Info, Method, Residency};
use crate::{Host, Reply};
use serde_json::Value;

pub struct PluginsManage;
static INFO: Info = Info {
    name: "plugins.manage",
    label: "Install, uninstall and disable other plugins",
    risky: true,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[Method::this("packages.*")];
impl Capability for PluginsManage {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.package_dispatch(reply.owner, method, params).map(Some)
    }
}
