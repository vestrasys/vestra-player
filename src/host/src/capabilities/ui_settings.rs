//! `ui.settings`: pages in the Options window (settings_window.rs, which
//! refuses the methods it does not know).
use super::{Call, Capability, Handled, Info, Recorded};
use crate::host::Origin;
use crate::host::{
    Core, Reply,
    messages::{SettingsOp, UiCommand},
};
use serde_json::Value;

pub struct UiSettings;
static INFO: Info = Info {
    name: "ui.settings",
    builtin_only: false,
};
static METHODS: &[&str] = &["settings.*"];
impl Capability for UiSettings {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Each page as its plugin registered it, with its current properties.
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        match core.settings.recording(owner) {
            Some(calls) => Recorded::Calls(
                calls
                    .into_iter()
                    .map(|params| Call::new("settings.register", params))
                    .collect(),
            ),
            None => Recorded::Live("a settings page without its registration"),
        }
    }
    /// The core's pages move to generation `to`; an open options window's
    /// callbacks follow (the interface's part: `SettingsOp::Rekeyed`, plan
    /// step S2d(3)).
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        let serials = core.settings.edit().rekey(from, to);
        core.ui_command(UiCommand::Settings(SettingsOp::Rekeyed { serials, to }));
    }
    /// The core's pages of `owner` are kept as recorded or go; an open
    /// options window is rebuilt when one went (the interface's part:
    /// `SettingsOp::OwnerGone`, plan step S2d(3)).
    fn revoke(&self, core: &mut Core, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            core.settings.edit().mark_recorded(owner);
        } else if core.settings.edit().remove_owner(owner) {
            core.ui_command(UiCommand::Settings(SettingsOp::OwnerGone { owner }));
        }
    }
    fn prune(&self, core: &mut Core, owner: u64) {
        if core.settings.edit().remove_recorded(owner) {
            core.ui_command(UiCommand::Settings(SettingsOp::OwnerGone { owner }));
        }
    }
    fn replay(&self, core: &mut Core, owner: u64, origin: &Origin, call: &Call) -> Handled {
        match call.method.as_str() {
            "settings.register" => core
                .settings_replay(owner, origin, &call.params)
                .map(|()| None),
            _ => Err(crate::host::RpcError::new(
                "unsupported",
                "not a recorded method",
            )),
        }
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        core.settings_dispatch(reply, method, params)
    }
}
