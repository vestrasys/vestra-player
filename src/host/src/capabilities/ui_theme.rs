//! `ui.theme`: the player's palette and interface language (theme.rs);
//! capability `player`.
use super::{Group, Handled, Info, Method, PLAYER, Recorded, Residency};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct UiTheme;
static INFO: Info = Info {
    name: "ui.theme",
    summary: "Change the player's palette and interface language",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::needs("appearance.set", PLAYER),
    Method::needs("appearance.setLanguage", PLAYER),
];
impl Group for UiTheme {
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
    fn handle(&self, host: &mut Host, _: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "appearance.set" => host.appearance_set(params),
            _ => host.language_set(params),
        }
        .map(Some)
    }
}
