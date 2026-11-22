//! `ui.theme`: the player's palette and interface language (theme.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply};
use serde_json::Value;

pub struct UiTheme;
static INFO: Info = Info {
    name: "ui.theme",
    builtin_only: false,
};
static METHODS: &[&str] = &["appearance.set", "appearance.setLanguage"];
impl Capability for UiTheme {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, _: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "appearance.set" => core.appearance_set(params),
            _ => core.language_set(params),
        }
        .map(Some)
    }
}
