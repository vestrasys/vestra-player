//! `ui.window`: a plugin's own Slint windows (ui/view.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency, plugin};
use crate::{
    host::{Host, Reply, RpcError, string},
    platform, process,
    ui::theme,
    ui::ui_patch,
    ui::ui_values,
    ui::view,
};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::collections::BTreeMap;

pub struct UiWindow;
static INFO: Info = Info {
    name: "ui.window",
    label: "Open tool windows",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[
    Method::this("ui.open"),
    Method::this("ui.set"),
    Method::this("ui.show"),
    Method::this("ui.close"),
];
impl Capability for UiWindow {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.views.keys().any(|(each, _)| *each == owner) {
            return Recorded::Live("an open window");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "ui.open" => open(host, reply, params),
            _ => change(host, reply, method, params),
        }
        .map(|()| Some(Value::Null))
    }
}

fn open(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "UI is unavailable in headless mode",
        ));
    }
    let id = string(params, "id")?.to_owned();
    if host.views.contains_key(&(reply.owner, id.clone())) {
        return Err(RpcError::new("conflict", "view already exists"));
    }
    let path = process::package_file(&plugin(host, reply)?.directory, string(params, "file")?)?;
    let properties: BTreeMap<String, Value> =
        serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
    let callbacks = serde_json::from_value::<Vec<String>>(
        params.get("callbacks").cloned().unwrap_or(json!([])),
    )
    .map_err(|e| RpcError::invalid(e.to_string()))?;
    let view_key = format!("w:{id}");
    // Row patches apply on these initial properties (seq 0).
    host.images.reset_writes(reply.owner, &view_key);
    let mut values = Vec::new();
    for (name, value) in &properties {
        match host.prop_value(reply.owner, &view_key, name, value) {
            Ok(value) => values.push((name.clone(), value)),
            Err(error) => {
                host.images.release_view(reply.owner, &view_key);
                return Err(error);
            }
        }
    }
    let theme = host.theme().to_owned();
    let missed = std::cell::Cell::new(false);
    let view = view::open(
        &path,
        reply.owner,
        &id,
        &values,
        &callbacks,
        host.events.clone(),
        |instance| {
            if theme != theme::THEMES[0] && theme::apply(instance, &theme).is_err()
                || theme::apply_fonts(instance).is_err()
            {
                missed.set(true);
            }
        },
    )
    .inspect_err(|_| host.images.release_view(reply.owner, &view_key))?;
    host.theme_pending |= missed.get();
    host.views.insert((reply.owner, id), view);
    Ok(())
}

/// `ui.set`, `ui.show` and `ui.close`.
fn change(
    host: &mut Host,
    reply: &Reply,
    method: &str,
    params: &Value,
) -> std::result::Result<(), RpcError> {
    let key = (reply.owner, string(params, "id")?.to_owned());
    if !host.views.contains_key(&key) {
        return Err(RpcError::new("missing", "view not found"));
    }
    let view_key = format!("w:{}", key.1);
    if method == "ui.set"
        && let Some(patch) = params.get("patch")
    {
        let property = string(params, "property")?;
        let (base, seq, ops) = ui_patch::request(patch)?;
        if host.images.seq(reply.owner, &view_key, property) != base {
            return Err(ui_patch::stale());
        }
        let current = host.views[&key]
            .instance
            .get_property(property)
            .map_err(|e| RpcError::new("ui", e.to_string()))?;
        let slint_interpreter::Value::Model(model) = &current else {
            return Err(ui_patch::stale());
        };
        let ops = ui_patch::parse(ops, slint::Model::row_count(model))?;
        // A window keeps no stored value: what its model cannot take in
        // place is `stale`, and the SDK sends the whole value.
        let rows = host
            .patch_rows(reply.owner, &view_key, property, &ops)?
            .filter(|rows| ui_patch::applicable(&current, ops.before, &ops.shapes, &rows.values))
            .ok_or_else(ui_patch::stale)?;
        ui_patch::apply(&current, &ops.shapes, rows.values);
        host.images
            .patched_rows(reply.owner, &view_key, property, rows.keys);
        host.images.set_seq(reply.owner, &view_key, property, seq);
        return Ok(());
    }
    let next = match method {
        "ui.set" => Some(host.prop_value(
            reply.owner,
            &format!("w:{}", key.1),
            string(params, "property")?,
            &params["value"],
        )?),
        _ => None,
    };
    let view = &host.views[&key];
    match method {
        "ui.set" => {
            let name = string(params, "property")?;
            let next = next.unwrap_or_default();
            let current = view
                .instance
                .get_property(name)
                .map_err(|e| RpcError::new("ui", e.to_string()))?;
            if !ui_values::reconcile_in(view.instance.window(), &current, &next)? {
                ui_values::set(&view.instance, None, name, next)
                    .map_err(|e| RpcError::new("ui", e.to_string()))?;
            }
            let seq = params["seq"].as_u64().unwrap_or(0);
            host.images.set_seq(reply.owner, &view_key, name, seq);
        }
        "ui.show" => {
            if params["visible"]
                .as_bool()
                .ok_or_else(|| RpcError::invalid("visible requires boolean"))?
            {
                view.instance.show().map_err(anyhow::Error::from)?;
                platform::shown(&view.instance);
            } else {
                view.instance.hide().map_err(anyhow::Error::from)?;
            }
        }
        _ => {
            view.instance.hide().map_err(anyhow::Error::from)?;
            host.images.release_view(key.0, &format!("w:{}", key.1));
            host.views.remove(&key);
        }
    }
    Ok(())
}
