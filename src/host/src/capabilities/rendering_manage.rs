//! `rendering.manage`: choosing GPU or software rendering for the next start
//! and restarting the player (host/rendering.rs; doc/architecture/README.md
//! §4.3); capability `player`. Reading the state, `rendering.state`, needs
//! none (general.rs).
use super::{Group, Handled, Info, Method, PLAYER, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct RenderingManage;
static INFO: Info = Info {
    name: "rendering.manage",
    summary: "Choose GPU or software rendering and restart the player",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::needs("rendering.setPreference", PLAYER),
    Method::needs("rendering.restart", PLAYER),
];
impl Group for RenderingManage {
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
    fn handle(&self, host: &mut Host, _reply: &Reply, method: &str, params: &Value) -> Handled {
        host.rendering_dispatch(method, params).map(Some)
    }
}
