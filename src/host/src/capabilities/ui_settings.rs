//! `ui.settings`: pages in the Options window (settings_window.rs, which
//! refuses the methods it does not know).
use super::{Call, Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::Origin;
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
    /// Each page as its plugin registered it, with its current properties.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        match host.settings.recording(owner) {
            Some(calls) => Recorded::Calls(
                calls
                    .into_iter()
                    .map(|params| Call::new("settings.register", params))
                    .collect(),
            ),
            None => Recorded::Live("a settings page without its registration"),
        }
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.settings.rekey(from, to);
    }
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            host.settings.mark_recorded(owner);
        } else {
            host.settings.remove_owner(owner);
        }
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        host.settings.remove_recorded(owner);
    }
    fn replay(&self, host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
        match call.method.as_str() {
            "settings.register" => host
                .settings_replay(owner, origin, &call.params)
                .map(|()| None),
            _ => Err(crate::host::RpcError::new(
                "unsupported",
                "not a recorded method",
            )),
        }
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.settings_dispatch(reply, method, params)
    }
}
