//! `ui.theme`: the player's palette and interface language (theme.rs).
use super::{Capability, Handled, Info, Method, Residency};
use crate::{Host, Reply};
use serde_json::Value;

pub struct UiTheme;
static INFO: Info = Info {
    name: "ui.theme",
    label: "Change the player's palette",
    risky: false,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("appearance.set"),
    Method::this("appearance.setLanguage"),
];
impl Capability for UiTheme {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, _: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "appearance.set" => host.appearance_set(params),
            _ => host.language_set(params),
        }
        .map(Some)
    }
}
