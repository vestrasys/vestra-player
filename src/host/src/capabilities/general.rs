//! `general`: requests no other group serves (the plugin list, the main
//! window's state, the system's file manager and browser, session storage,
//! the rendering state). None needs a capability.
use super::{Group, Handled, Info, Method, Recorded, Residency, plugin};
use crate::host::{Host, Reply};
use serde_json::Value;

pub struct General;
static INFO: Info = Info {
    name: "general",
    summary: "Read the plugin list, the window and rendering state; open files and links with the system; keep values for this run",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::free("packages.list"),
    Method::free("window.state"),
    Method::free("system.reveal"),
    Method::free("system.openPath"),
    Method::free("system.openUrl"),
    Method::free("app.system"),
    Method::free("ui.markdownScan"),
    // SDK 1.5.25, player 0.5.5: like `plugin.storage.json`, for this run.
    Method::free("storage.session.get"),
    Method::free("storage.session.set"),
    Method::free("storage.session.delete"),
    Method::free("storage.session.keys"),
    // SDK 1.5.35, player 0.5.13: reading the rendering state (host/rendering.rs).
    Method::free("rendering.state"),
];

impl Group for General {
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
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "packages.list" => host.package_dispatch(reply.owner, method, params).map(Some),
            "window.state" => Ok(Some(host.window_state())),
            "ui.markdownScan" => host.markdown_scan(params).map(Some),
            "rendering.state" => host.rendering_dispatch(method, params).map(Some),
            method if method.starts_with("storage.session.") => {
                // Keyed by the plugin's data identity, not by `reply.owner`:
                // the values outlive this process (session_storage.rs).
                let key = plugin(host, reply)?.data_key.clone();
                host.session_storage
                    .dispatch(&key, method, params)
                    .map(Some)
            }
            _ => host.system_dispatch(reply, method, params),
        }
    }
}
