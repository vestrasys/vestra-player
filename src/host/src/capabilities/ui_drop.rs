//! `ui.drop`: files dropped on the player (drops.rs). Drop targets are
//! recordable; a panel's `drops.listen` lives with its mount.
use super::{Call, Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply};
use crate::host::{Origin, RpcError, string};
use crate::ui::drops::{Provider, parse_extensions};
use serde_json::Value;

pub struct UiDrop;
static INFO: Info = Info {
    name: "ui.drop",
    label: "Receive files dropped on the player",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
    builtin_only: false,
};
static METHODS: &[Method] = &[
    Method::this("drops.provide"),
    Method::this("drops.withdraw"),
    Method::this("drops.listen"),
];
impl Capability for UiDrop {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// Its drop targets; a panel listening for drops is live.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.drops.panels.contains(&owner) {
            return Recorded::Live("a panel listening for drops");
        }
        Recorded::Calls(
            host.drops
                .recording(owner)
                .into_iter()
                .map(|params| Call::new("drops.provide", params))
                .collect(),
        )
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.drops.rekey(from, to);
    }
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            host.drops.mark_recorded(owner);
        } else {
            host.drops.remove_owner(owner);
        }
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        host.drops.remove_recorded(owner);
    }
    fn replay(&self, host: &mut Host, owner: u64, _: &Origin, call: &Call) -> Handled {
        if call.method != "drops.provide" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        let extensions = parse_extensions(&call.params["extensions"]).map_err(RpcError::invalid)?;
        host.drops.provide_recorded(Provider {
            owner,
            id: string(&call.params, "id")?.into(),
            extensions,
        });
        Ok(None)
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.drops_dispatch(reply.owner, method, params).map(Some)
    }
}
