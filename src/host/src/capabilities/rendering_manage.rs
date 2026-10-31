//! `rendering.manage`: choosing GPU or software rendering for the next start
//! and restarting the player (host/rendering.rs; round record
//! 2026-10-render-modes). Reading the state, `rendering.state`, needs no
//! capability (uncapable.rs).
use super::{Capability, Handled, Info, Method, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct RenderingManage;
static INFO: Info = Info {
    name: "rendering.manage",
    label: "Choose GPU or software rendering and restart the player",
    risky: true,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("rendering.setPreference"),
    Method::this("rendering.restart"),
];
impl Capability for RenderingManage {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, _reply: &Reply, method: &str, params: &Value) -> Handled {
        host.rendering_dispatch(method, params).map(Some)
    }
}
