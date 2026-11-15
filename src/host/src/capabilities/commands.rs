//! `commands`: menu commands with keys and state (crate::commands, the
//! binding table), and running the player's own actions (`host.run`).
use super::{Call, Capability, Handled, Info, Recorded, plugin};
use crate::{
    commands,
    conditions::Conditions,
    core_actions,
    host::{Host, Origin, Reply, RpcError, string},
    i18n, input_trace,
};
use serde_json::{Value, json};

pub struct Commands;
static INFO: Info = Info {
    name: "commands",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "commands.register",
    "commands.update",
    "commands.unregister",
    "host.run",
    // Player 0.7.6: the key labels of the plugin's own commands.
    "commands.keys",
];
impl Capability for Commands {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Each command as its plugin registered it, with its last state (never
    /// busy: a busy command keeps its plugin running, `Host::lazy_idle`).
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.core.commands.unsent(owner) {
            return Recorded::Live("a command without its registration");
        }
        let calls = host
            .core.commands
            .sent_of(owner)
            .into_iter()
            .map(|(id, revision, sent)| {
                let mut state = sent.state.clone();
                if let Some(state) = state.as_object_mut() {
                    state.insert("busy".into(), false.into());
                }
                let mut params = json!({"id":id,"revision":revision,"declaration":sent.declaration,"state":state});
                if !sent.when.is_null() {
                    params["when"] = sent.when.clone();
                }
                Call::new("commands.register", params)
            })
            .collect();
        Recorded::Calls(calls)
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        if let Err(error) = host.core.commands.rekey(from, to) {
            diag!("lazy: commands of {from}: {error}");
        }
        host.input.borrow_mut().table.rekey(from, to);
    }
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            host.core.commands.mark_recorded(owner);
        } else {
            let _ = host.core.commands.remove_owner(owner);
            host.input.borrow_mut().table.remove_owner(owner);
        }
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        match host.core.commands.remove_recorded(owner) {
            Ok(gone) => {
                let mut input = host.input.borrow_mut();
                for id in gone {
                    input.table.remove(owner, &id);
                }
            }
            Err(error) => diag!("lazy: commands of {owner}: {error}"),
        }
    }
    fn replay(&self, host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
        match call.method.as_str() {
            "commands.register" => register(host, owner, origin, &call.params).map(|()| None),
            _ => Err(RpcError::new("unsupported", "not a recorded method")),
        }
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
            // Player 0.7.6 (`plugin.commands.keys`): the caller's own
            // commands and the player's (`host/<id>`, SDK 1.6.16), never
            // another plugin's; changes come as the `commands.keys` event.
            "commands.keys" => Ok(Some(host.command_keys(reply.owner))),
            "commands.unregister" => {
                let id = string(params, "id")?;
                host.core
                    .commands
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
    let origin = plugin(host, reply)?.origin();
    if method == "commands.register" {
        register(host, reply.owner, &origin, params)?;
    } else {
        let (id, revision, state) = parsed(&origin, params)?;
        let conditions = conditions(params)?;
        host.core
            .commands
            .update(reply.owner, id, revision, state)
            .map_err(|e| RpcError::new("command", e.to_string()))?;
        refresh_conditions(host);
        host.core
            .commands
            .set_conditions(reply.owner, id, conditions)
            .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
        host.core.commands.note_sent(
            reply.owner,
            id,
            None,
            params["state"].clone(),
            params["when"].clone(),
        );
    }
    Ok(json!({"revision":host.core.commands.revision()}))
}

/// The id, revision and state of a register or update; the disabled reason
/// is one of the plugin's sentences: source text translated as a whole, or
/// a text value.
fn parsed<'a>(
    origin: &Origin,
    params: &'a Value,
) -> std::result::Result<(&'a str, u64, commands::CommandState), RpcError> {
    let id = string(params, "id")?;
    let revision = params["revision"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| RpcError::invalid("positive revision required"))?;
    let mut state = params["state"].clone();
    if let Some(reason) = state.get_mut("reason").filter(|r| !r.is_null()) {
        *reason = origin
            .texts()
            .declared(reason)
            .map_err(RpcError::invalid)?
            .into();
    }
    let state: commands::CommandState =
        serde_json::from_value(state).map_err(|e| RpcError::invalid(e.to_string()))?;
    Ok((id, revision, state))
}

/// A `commands.register` of `owner`'s, from its package `origin`: sent by
/// its running plugin, or replayed from its recording (lazy activation).
fn register(
    host: &mut Host,
    owner: u64,
    origin: &Origin,
    params: &Value,
) -> std::result::Result<(), RpcError> {
    let (id, revision, state) = parsed(origin, params)?;
    let conditions = conditions(params)?;
    let texts = origin.texts();
    // Text values in the title and path: the key is their
    // English rendering, the shown text their translation.
    let mut raw = params["declaration"].clone();
    // Player 0.5.10: the bottom-bar button, read apart (its image is
    // the plugin's file); `Declaration` still refuses unknown fields.
    let button = match raw.as_object_mut().and_then(|o| o.remove("button")) {
        None | Some(Value::Null) => None,
        Some(value) => Some(button(origin, &value)?),
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
    // Player 0.7.6 (SDK 1.6.4): the category, a menu text too.
    let mut shown_category = None;
    if let Some(category) = raw.get_mut("category") {
        shown_category = menu_text(category)?;
    }
    let mut declaration: commands::Declaration =
        serde_json::from_value(raw).map_err(|e| RpcError::invalid(e.to_string()))?;
    declaration.button = button;
    // Keys first (`@video`, old Chinese titles), then the
    // shown text from this plugin's catalog.
    let declared = declaration.path.clone();
    commands::normalize(&mut declaration.path)
        .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
    declaration.localize_declared(&declared, |text| origin.text(text));
    declaration.show_texts(shown_title, &shown_path);
    declaration.show_category(shown_category);
    let bindings = declaration.bindings.clone();
    let panel = declaration.focus == Some(commands::Focus::Panel);
    let repeat = declaration.repeat;
    // Everything that can refuse the registration (bindings parsed and not
    // repeated, the id, path, conflict and limits) is checked by the
    // registry before it changes; what follows only records it.
    host.core
        .commands
        .register(owner, &origin.id, id.into(), revision, declaration, state)
        .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
    refresh_conditions(host);
    host.core
        .commands
        .set_conditions(owner, id, conditions)
        .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
    host.core.commands.note_sent(
        owner,
        id,
        Some(params["declaration"].clone()),
        params["state"].clone(),
        params["when"].clone(),
    );
    host.input
        .borrow_mut()
        .table
        .declare(owner, id, &origin.id, &bindings, panel, repeat)
        .map_err(RpcError::invalid)?;
    Ok(())
}

/// A register's or update's `when` (SDK 1.5.40, `plugin.when`): its
/// `enabled` and `checked` conditions, evaluated by the player.
fn conditions(params: &Value) -> std::result::Result<Option<Conditions>, RpcError> {
    Conditions::parse(&params["when"]).map_err(RpcError::invalid)
}

/// The player's state that conditions read, as it is now (the same facts
/// as a trigger's, `Host::condition_facts`, the primary subtitle track
/// included): the registry evaluates every condition again when it changed
/// (`Host::broadcast` of `source.state`, `window.state` and
/// `appearance.changed`, and the playback worker's subtitle track,
/// `Host::triggers_subtitle`), and before a command's conditions are first
/// evaluated.
pub(crate) fn refresh_conditions(host: &mut Host) {
    let facts = host.condition_facts();
    host.core.commands.set_facts(facts);
}

/// `declaration.button`: `{ icon, order }`, the icon a name of the icon
/// set (checked by the registry) or an image reference to one of the
/// plugin's files (checked here, like a property's image).
fn button(origin: &Origin, value: &Value) -> std::result::Result<commands::Button, RpcError> {
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
        Some(image) if crate::ui::ui_image::reference(image).is_some() => {
            let (path, max_side) = crate::ui::ui_image::parts(image)?;
            crate::ui::ui_image::locate(path, &origin.image_roots())?;
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
