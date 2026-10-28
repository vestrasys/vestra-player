//! `secrets`: sign-in details in the system keychain (system.rs,
//! secret_index.rs).
use super::{Capability, Handled, Info, Method, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct Secrets;
static INFO: Info = Info {
    name: "secrets",
    label: "Store sign-in details in the system keychain",
    risky: true,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("secrets.get"),
    Method::this("secrets.set"),
    Method::this("secrets.delete"),
];
impl Capability for Secrets {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.system_dispatch(reply, method, params)
    }
}
