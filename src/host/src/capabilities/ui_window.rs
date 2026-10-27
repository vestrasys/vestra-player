//! `ui.window`: a plugin's own Slint windows (ui.rs).
use super::{Capability, Handled, Info, Method, Residency, plugin};
use crate::{Host, Reply, RpcError, platform, plugins, string, theme, ui, ui_values};
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
    let path = plugins::package_file(&plugin(host, reply)?.directory, string(params, "file")?)?;
    let properties: BTreeMap<String, Value> =
        serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
    let callbacks = serde_json::from_value::<Vec<String>>(
        params.get("callbacks").cloned().unwrap_or(json!([])),
    )
    .map_err(|e| RpcError::invalid(e.to_string()))?;
    let view_key = format!("w:{id}");
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
    let view = ui::open(
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
            if !ui_values::reconcile(&current, &next)? {
                ui_values::set(&view.instance, None, name, next)
                    .map_err(|e| RpcError::new("ui", e.to_string()))?;
            }
        }
        "ui.show" => {
            if params["visible"]
                .as_bool()
                .ok_or_else(|| RpcError::invalid("visible requires boolean"))?
            {
                view.instance.show().map_err(anyhow::Error::from)?;
                platform::shown(view.instance.window());
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
