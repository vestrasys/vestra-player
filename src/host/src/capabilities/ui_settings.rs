//! `ui.settings`: pages in the Options window (settings_window.rs, which
//! refuses the methods it does not know).
use super::{Call, Capability, Handled, Info, Recorded};
use crate::host::Origin;
use crate::host::{Host, Reply};
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
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        match host.core.settings.recording(owner) {
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
    /// callbacks follow (the interface's part, called in place:
    /// a later step routes it as `UiCommand::RekeyOwner`).
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        let moved = host.core.settings.rekey(from, to);
        host.settings.rekey(&moved, to);
    }
    /// The core's pages of `owner` are kept as recorded or go; an open
    /// options window is rebuilt when one went (the interface's part,
    /// called in place: plan step S2d routes it as
    /// `UiCommand::RevokeOwner`).
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            host.core.settings.mark_recorded(owner);
        } else if host.core.settings.remove_owner(owner) {
            host.settings.changed();
        }
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        if host.core.settings.remove_recorded(owner) {
            host.settings.changed();
        }
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
