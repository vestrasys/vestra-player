//! `ui.settings`: pages in the Options window (settings_window.rs, which
//! refuses the methods it does not know).
use super::{Capability, Handled, Info, Method, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct UiSettings;
static INFO: Info = Info {
    name: "ui.settings",
    label: "Add pages to the Options window",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
};
static METHODS: &[Method] = &[Method::this("settings.*")];
impl Capability for UiSettings {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.settings_dispatch(reply, method, params)
    }
}
