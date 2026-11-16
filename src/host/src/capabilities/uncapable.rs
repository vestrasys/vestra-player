//! Requests any plugin may make, without a manifest capability.
use super::{Handled, plugin};
use crate::host::{Core, Reply};
use serde_json::Value;

pub static METHODS: &[&str] = &[
    "packages.list",
    "window.state",
    "system.reveal",
    "system.openPath",
    "system.openUrl",
    "app.system",
    "ui.markdownScan",
    // SDK 1.5.25, player 0.5.5: no capability, like `plugin.storage.json`.
    "storage.session.get",
    "storage.session.set",
    "storage.session.delete",
    "storage.session.keys",
    // SDK 1.5.35, player 0.5.13: reading the rendering state (host/rendering.rs).
    "rendering.state",
];

pub fn handle(core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
    match method {
        "packages.list" => core.package_dispatch(reply.owner, method, params).map(Some),
        "window.state" => Ok(Some(core.window_state())),
        "ui.markdownScan" => core.markdown_scan(params).map(Some),
        "rendering.state" => core.rendering_dispatch(method, params).map(Some),
        method if method.starts_with("storage.session.") => {
            // Keyed by the plugin's data identity, not by `reply.owner`: the
            // values outlive this process (session_storage.rs).
            let key = plugin(core, reply)?.data_key.clone();
            core.session_storage
                .dispatch(&key, method, params)
                .map(Some)
        }
        _ => core.system_dispatch(reply, method, params),
    }
}
