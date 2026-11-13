//! Requests any plugin may make, without a manifest capability.
use super::{Handled, plugin};
use crate::host::{Host, Reply};
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

pub fn handle(host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
    match method {
        "packages.list" => host.package_dispatch(reply.owner, method, params).map(Some),
        "window.state" => Ok(Some(host.window_state())),
        "ui.markdownScan" => host.markdown_scan(params).map(Some),
        "rendering.state" => host.rendering_dispatch(method, params).map(Some),
        method if method.starts_with("storage.session.") => {
            // Keyed by the plugin's data identity, not by `reply.owner`: the
            // values outlive this process (session_storage.rs).
            let key = plugin(host, reply)?.data_key.clone();
            host.session_storage
                .dispatch(&key, method, params)
                .map(Some)
        }
        _ => host.system_dispatch(reply, method, params),
    }
}
