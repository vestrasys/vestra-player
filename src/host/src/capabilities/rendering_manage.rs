//! `rendering.manage`: choosing GPU or software rendering for the next start
//! and restarting the player (host/rendering.rs; doc/architecture/README.md
//! §4.3). Reading the state, `rendering.state`, needs no
//! capability (uncapable.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct RenderingManage;
static INFO: Info = Info {
    name: "rendering.manage",
    builtin_only: false,
};
static METHODS: &[&str] = &["rendering.setPreference", "rendering.restart"];
impl Capability for RenderingManage {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
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
