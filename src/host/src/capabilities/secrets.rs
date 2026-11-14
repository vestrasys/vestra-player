//! `secrets`: sign-in details in the system keychain (system.rs,
//! secret_index.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct Secrets;
static INFO: Info = Info {
    name: "secrets",
    builtin_only: false,
};
static METHODS: &[&str] = &["secrets.get", "secrets.set", "secrets.delete"];
impl Capability for Secrets {
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
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.system_dispatch(reply, method, params)
    }
}
