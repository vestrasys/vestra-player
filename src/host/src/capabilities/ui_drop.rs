//! `ui.drop`: files dropped on the player (drops.rs). Drop targets are
//! recordable; a panel's `drops.listen` lives with its mount.
use super::{Capability, Handled, Info, Method, Residency};
use crate::{Host, Reply};
use serde_json::Value;

pub struct UiDrop;
static INFO: Info = Info {
    name: "ui.drop",
    label: "Receive files dropped on the player",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
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
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        host.drops_dispatch(reply.owner, method, params).map(Some)
    }
}
