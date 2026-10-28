//! `commands`: menu commands with keys and state (crate::commands, the
//! binding table), and running the player's own actions (`host.run`).
use super::{Capability, Handled, Info, Method, Residency, plugin};
use crate::{Host, Reply, RpcError, commands, core_actions, i18n, input_trace, string};
use serde_json::{Value, json};

pub struct Commands;
static INFO: Info = Info {
    name: "commands",
    label: "Add menu commands and shortcuts",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
};
static METHODS: &[Method] = &[
    Method::this("commands.register"),
    Method::this("commands.update"),
    Method::this("commands.unregister"),
    Method::this("host.run"),
];
impl Capability for Commands {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "host.run" => {
                let action = string(params, "action")?;
                input_trace::follow(&format!("host.run {action} from plugin {}", reply.owner));
                if !core_actions::ACTIONS.contains(&action) {
                    return Err(RpcError::invalid(format!("unknown host action {action}")));
                }
                // SDK 1.5.19: one step seek of another step (tiers 2–4).
                let step = core_actions::run_step(action, &params["seconds"])
                    .map_err(RpcError::invalid)?;
                if !host.run_core_action_with(action, Some(reply), step)? {
                    return Ok(None);
                }
                Ok(Some(Value::Null))
            }
            "commands.unregister" => {
                let id = string(params, "id")?;
                host.commands
                    .remove(reply.owner, id)
                    .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
                host.input.borrow_mut().table.remove(reply.owner, id);
                if host.menu_open {
                    let result = host.refresh_menu();
                    host.report_menu(result);
                }
                Ok(Some(Value::Null))
            }
            _ => register_or_update(host, reply, method, params).map(Some),
        }
    }
}

/// `commands.register` and `commands.update`.
fn register_or_update(
    host: &mut Host,
    reply: &Reply,
    method: &str,
    params: &Value,
) -> std::result::Result<Value, RpcError> {
    let plugin = plugin(host, reply)?;
    let id = string(params, "id")?;
    let revision = params["revision"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| RpcError::invalid("positive revision required"))?;
    let texts = plugin.texts();
    // The disabled reason is one of the plugin's sentences: source
    // text translated as a whole, or a text value.
    let mut state = params["state"].clone();
    if let Some(reason) = state.get_mut("reason").filter(|r| !r.is_null()) {
        *reason = texts.declared(reason).map_err(RpcError::invalid)?.into();
    }
    let state: commands::CommandState =
        serde_json::from_value(state).map_err(|e| RpcError::invalid(e.to_string()))?;
    if method == "commands.register" {
        // Text values in the title and path: the key is their
        // English rendering, the shown text their translation.
        let mut raw = params["declaration"].clone();
        // Player 0.5.10: the bottom-bar button, read apart (its image is
        // the plugin's file); `Declaration` still refuses unknown fields.
        let button = match raw.as_object_mut().and_then(|o| o.remove("button")) {
            None | Some(Value::Null) => None,
            Some(value) => Some(button(host, reply.owner, &value)?),
        };
        let mut shown_title = None;
        let mut shown_path = Vec::new();
        let menu_text = |value: &mut Value| {
            if !i18n::text::is_text(value) {
                return Ok(None);
            }
            if value.get(i18n::text::LIST_KEY).is_some() {
                return Err(RpcError::invalid(
                    "a menu text is a string or a text, not a list",
                ));
            }
            i18n::text::check(value).map_err(RpcError::invalid)?;
            let shown = texts.render(value);
            *value = i18n::text::Texts::english().render(value).into();
            Ok(Some(shown))
        };
        if let Some(title) = raw.get_mut("title") {
            shown_title = menu_text(title)?;
        }
        if let Some(path) = raw.get_mut("path").and_then(Value::as_array_mut) {
            for segment in path {
                shown_path.push(menu_text(segment)?);
            }
        }
        let mut declaration: commands::Declaration =
            serde_json::from_value(raw).map_err(|e| RpcError::invalid(e.to_string()))?;
        declaration.button = button;
        // Keys first (`@video`, old Chinese titles), then the
        // shown text from this plugin's catalog.
        let declared = declaration.path.clone();
        commands::normalize(&mut declaration.path)
            .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
        declaration.localize_declared(&declared, |text| plugin.text(text));
        declaration.show_texts(shown_title, &shown_path);
        let manifest = plugin.manifest.id.clone();
        let bindings = declaration.bindings.clone();
        let panel = declaration.focus == Some(commands::Focus::Panel);
        let repeat = declaration.repeat;
        host.commands
            .register(
                reply.owner,
                &manifest,
                id.into(),
                revision,
                declaration,
                state,
            )
            .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
        host.input
            .borrow_mut()
            .table
            .declare(reply.owner, id, &manifest, &bindings, panel, repeat)
            .map_err(RpcError::invalid)?;
    } else {
        host.commands
            .update(reply.owner, id, revision, state)
            .map_err(|e| RpcError::new("command", e.to_string()))?;
    }
    Ok(json!({"revision":host.commands.revision()}))
}

/// `declaration.button`: `{ icon, order }`, the icon a name of the icon
/// set (checked by the registry) or an image reference to one of the
/// plugin's files (checked here, like a property's image).
fn button(
    host: &Host,
    owner: u64,
    value: &Value,
) -> std::result::Result<commands::Button, RpcError> {
    let object = value
        .as_object()
        .filter(|o| o.keys().all(|key| key == "icon" || key == "order"))
        .ok_or_else(|| RpcError::invalid("button is { icon, order }"))?;
    let order = object
        .get("order")
        .and_then(Value::as_i64)
        .filter(|order| (0..=i64::from(commands::Registry::MAX_BUTTON_ORDER)).contains(order))
        .ok_or_else(|| {
            RpcError::invalid(format!(
                "button order must be an integer 0–{}",
                commands::Registry::MAX_BUTTON_ORDER
            ))
        })? as i32;
    let icon = match object.get("icon") {
        Some(Value::String(name)) => commands::ButtonIcon::Name(name.clone()),
        Some(image) if crate::ui_image::reference(image).is_some() => {
            let (path, max_side) = crate::ui_image::parts(image)?;
            crate::ui_image::locate(path, &host.image_roots(owner))?;
            commands::ButtonIcon::Image {
                path: path.to_owned(),
                max_side,
            }
        }
        _ => {
            return Err(RpcError::invalid(
                "button icon is an icon name or an image from plugin.ui.image",
            ));
        }
    };
    Ok(commands::Button { icon, order })
}
