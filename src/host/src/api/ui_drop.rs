//! `ui.drop`: files dropped on the player (drops.rs). Drop targets are
//! recordable; a panel's `drops.listen` lives with its mount.
use super::{Call, Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply};
use crate::host::{Origin, RpcError, string};
use crate::ui::drops::{Provider, parse_extensions};
use serde_json::Value;

pub struct UiDrop;
static INFO: Info = Info {
    name: "ui.drop",
    builtin_only: false,
};
static METHODS: &[&str] = &["drops.provide", "drops.withdraw", "drops.listen"];
impl Capability for UiDrop {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Its drop targets; a panel listening for drops is live.
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        if core.drops.panels.contains(&owner) {
            return Recorded::Live("a panel listening for drops");
        }
        Recorded::Calls(
            core.drops
                .recording(owner)
                .into_iter()
                .map(|params| Call::new("drops.provide", params))
                .collect(),
        )
    }
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        core.drops.rekey(from, to);
    }
    fn revoke(&self, core: &mut Core, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            core.drops.mark_recorded(owner);
        } else {
            core.drops.remove_owner(owner);
        }
    }
    fn prune(&self, core: &mut Core, owner: u64) {
        core.drops.remove_recorded(owner);
    }
    fn replay(&self, core: &mut Core, owner: u64, _: &Origin, call: &Call) -> Handled {
        if call.method != "drops.provide" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        let extensions = parse_extensions(&call.params["extensions"]).map_err(RpcError::invalid)?;
        core.drops.provide_recorded(Provider {
            owner,
            id: string(&call.params, "id")?.into(),
            extensions,
        });
        Ok(None)
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        core.drops_dispatch(reply.owner, method, params).map(Some)
    }
}
