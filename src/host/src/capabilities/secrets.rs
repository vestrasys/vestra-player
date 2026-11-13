//! `secrets`: sign-in details in the system keychain (system.rs,
//! secret_index.rs); capability `credentials`.
use super::{CREDENTIALS, Group, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct Secrets;
static INFO: Info = Info {
    name: "secrets",
    summary: "Store sign-in details in the system keychain",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::needs("secrets.get", CREDENTIALS),
    Method::needs("secrets.set", CREDENTIALS),
    Method::needs("secrets.delete", CREDENTIALS),
];
impl Group for Secrets {
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
        host.system_dispatch(reply, method, params)
    }
}
