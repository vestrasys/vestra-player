//! Requests any plugin may make, without a manifest capability.
use super::{Handled, Method, plugin};
use crate::host::{Host, Reply};
use serde_json::Value;

pub static METHODS: &[Method] = &[
    Method::free("packages.list"),
    Method::free("window.state"),
    Method::free("system.reveal"),
    Method::free("system.openPath"),
    Method::free("system.openUrl"),
    Method::free("app.system"),
    Method::free("ui.markdownScan"),
    // SDK 1.5.25, player 0.5.5: no capability, like `plugin.storage.json`.
    Method::free("storage.session.get"),
    Method::free("storage.session.set"),
    Method::free("storage.session.delete"),
    Method::free("storage.session.keys"),
    // SDK 1.5.35, player 0.5.13: reading the rendering state (host/rendering.rs).
    Method::free("rendering.state"),
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
