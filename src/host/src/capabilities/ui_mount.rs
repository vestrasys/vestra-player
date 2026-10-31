//! `ui.mount`: panels in the bottom bar, the sidebar and the backdrop, their
//! menus and interaction locks, and the layout of those slots.
use super::{Capability, Handled, Info, Method, Recorded, Residency, plugin};
use crate::{
    host::{
        Host, LayoutChange, MenuMode, Mount, MountChange, Reply, RpcError, debug_keys,
        layout_change, policy_slot, refuse_initial, string,
    },
    i18n, process,
    ui::composition,
    ui::slot_claims,
    ui::ui_patch,
    ui::window_state,
};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::collections::BTreeMap;

pub struct UiMount;
static INFO: Info = Info {
    name: "ui.mount",
    label: "Show panels in the bottom bar, the sidebar or behind the video",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[
    Method::this("ui.mount"),
    Method::this("ui.setMount"),
    Method::this("ui.unmount"),
    Method::this("ui.interaction"),
    Method::this("ui.menu"),
    Method::this("layout.state"),
    Method::this("layout.set"),
];
impl Capability for UiMount {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        let queued = host.mount_queue.iter().any(|c| c.reply.owner == owner)
            || host
                .preparing
                .as_ref()
                .is_some_and(|(_, c)| c.reply.owner == owner)
            || host.held_mounts.waiting(owner);
        if host.mounts.contains_key(&owner) || queued {
            return Recorded::Live("a panel");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "ui.mount" | "ui.unmount" => {
                mount(host, reply, method, params)?;
                Ok(None)
            }
            "ui.setMount" => set_mount(host, reply, params).map(Some),
            "ui.interaction" => interaction(host, reply, params).map(Some),
            "ui.menu" => menu(host, reply, params).map(Some),
            "layout.state" => host.layout_state().map(Some),
            _ => layout_set(host, params).map(Some),
        }
    }
}

/// `ui.mount` and `ui.unmount`: queued for the next shell preparation.
fn mount(
    host: &mut Host,
    reply: &Reply,
    method: &str,
    params: &Value,
) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "mounts require a native shell",
        ));
    }
    if host
        .mount_queue
        .iter()
        .any(|c| c.reply.owner == reply.owner)
        || host
            .preparing
            .as_ref()
            .is_some_and(|(_, c)| c.reply.owner == reply.owner)
    {
        return Err(RpcError::new(
            "busy",
            "owner already has a pending UI preparation",
        ));
    }
    if host.mount_queue.len() >= 8 {
        return Err(RpcError::new("busy", "UI preparation queue full"));
    }
    let mut refused = Vec::new();
    let contribution = if method == "ui.mount" {
        if host.mounts.contains_key(&reply.owner) {
            return Err(RpcError::new("conflict", "owner already mounted"));
        }
        let slot = match string(params, "slot")? {
            "bottom" => composition::Slot::Bottom,
            "sidebar" => composition::Slot::Sidebar,
            "backdrop" => composition::Slot::Backdrop,
            _ => return Err(RpcError::invalid("unknown mount slot")),
        };
        // One component in the bottom bar and the backdrop (R-U3),
        // whoever mounted it: refused here, not when the shell
        // compiles. The sidebar takes tabs, up to a limit.
        let pending = host
            .mount_queue
            .iter()
            .chain(host.preparing.as_ref().map(|(_, change)| change))
            .map(|c| (c.reply.owner, c.contribution.as_ref().map(|m| m.spec.slot)))
            .collect::<Vec<_>>();
        let mounted = host.mounts.values().map(|m| (m.spec.owner, m.spec.slot));
        if slot == composition::Slot::Sidebar {
            if slot_claims::tabs(reply.owner, mounted, &pending) >= slot_claims::MAX_SIDEBAR_TABS {
                let error = RpcError::sidebar_full();
                let text = i18n::tr_args("Panel not shown: {}", &json!([error.message]));
                return Err(host.refuse_taken(reply.owner, error, text));
            }
        } else if let Some(holder) = slot_claims::occupant(slot, reply.owner, mounted, &pending) {
            let error = RpcError::slot_occupied(slot, &host.plugin_id(holder));
            let text = i18n::tr_args("Panel not shown: {}", &json!([error.message]));
            return Err(host.refuse_taken(reply.owner, error, text));
        }
        let plugin = plugin(host, reply)?;
        let file = process::package_file(&plugin.directory, string(params, "file")?)?;
        let mut properties: BTreeMap<String, Value> =
            serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                .map_err(|e| RpcError::invalid(e.to_string()))?;
        let title = params["title"].as_str().unwrap_or("");
        if title.len() > 128 || title.chars().any(char::is_control) {
            return Err(RpcError::invalid("title requires 0..=128 bytes"));
        }
        refused = refuse_initial(&mut properties);
        let manifest = plugin.manifest.id.clone();
        let name = plugin.manifest.name.clone().unwrap_or_default();
        let builtin = host.is_builtin(reply.owner);
        // C8: a sidebar panel joins the slot's remembered state
        // (`remember_sidebar`, applied when it attaches).
        Some(Mount {
            spec: composition::CompileSpec {
                owner: reply.owner,
                file,
                component: string(params, "component")?.into(),
                global: Some(string(params, "global")?.into()),
                slot,
            },
            properties,
            visible: true,
            detached: false,
            // The tab label and the floating window's title, in
            // the interface language (this plugin's catalog).
            title: if !title.is_empty() {
                plugin.text(title)
            } else if !name.is_empty() {
                plugin.text(&name)
            } else {
                "Vestra Player".into()
            },
            manifest,
            builtin,
        })
    } else {
        None
    };
    host.report_refused(reply.owner, refused);
    if contribution.is_some() {
        // Row patches apply on the initial properties again (seq 0).
        host.images.reset_writes(reply.owner, "$mount");
    }
    host.mount_queue.push_back(MountChange {
        reply: reply.clone(),
        contribution,
        announced: false,
    });
    Ok(())
}

fn set_mount(
    host: &mut Host,
    reply: &Reply,
    params: &Value,
) -> std::result::Result<Value, RpcError> {
    let spec = host
        .mounts
        .get(&reply.owner)
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?
        .spec
        .clone();
    let shell = host
        .shell
        .as_ref()
        .map(ComponentHandle::clone_strong)
        .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
    let property = string(params, "property")?;
    let floating = host
        .floating
        .as_ref()
        .filter(|f| f.owners.contains(&reply.owner))
        .map(|f| f.instance.clone_strong());
    let seq = if let Some(patch) = params.get("patch") {
        let (base, seq, ops) = ui_patch::request(patch)?;
        if host.images.seq(reply.owner, "$mount", property) != base {
            return Err(ui_patch::stale());
        }
        if patch_mount(host, reply.owner, &shell, floating.as_ref(), property, ops)? {
            host.images.set_seq(reply.owner, "$mount", property, seq);
            return Ok(Value::Null);
        }
        seq
    } else {
        params["seq"].as_u64().unwrap_or(0)
    };
    // A whole value, or a patched one that a live model could not take in place.
    let whole;
    let value = match params.get("patch") {
        Some(patch) => {
            whole = patched_mount(host, reply.owner, property, &patch["ops"])?;
            &whole
        }
        None => &params["value"],
    };
    let converted = host.prop_value(reply.owner, "$mount", property, value)?;
    composition::set_global_value(&shell, &spec, property, converted)?;
    if let Some(floating) = floating {
        let converted = host.prop_value(reply.owner, "$mount", property, value)?;
        composition::set_global_value(&floating, &spec, property, converted)?;
    }
    if let Some(mount) = host.mounts.get_mut(&reply.owner) {
        mount.properties.insert(property.into(), value.clone());
    }
    host.images.set_seq(reply.owner, "$mount", property, seq);
    Ok(Value::Null)
}

/// The stored array of a panel's property with `ops` applied.
fn patched_mount(
    host: &Host,
    owner: u64,
    property: &str,
    ops: &Value,
) -> std::result::Result<Value, RpcError> {
    let Some(Value::Array(stored)) = host
        .mounts
        .get(&owner)
        .and_then(|mount| mount.properties.get(property))
    else {
        return Err(ui_patch::stale());
    };
    let ops = ui_patch::parse(ops, stored.len())?;
    Ok(ui_patch::patched(stored, &ops))
}

/// A row patch of a panel's property, in place in the shell and the
/// floating pop-out and in the stored properties. False: a live model
/// cannot take it in place (nothing changed; the caller writes the patched
/// whole value).
fn patch_mount(
    host: &mut Host,
    owner: u64,
    shell: &slint_interpreter::ComponentInstance,
    floating: Option<&slint_interpreter::ComponentInstance>,
    property: &str,
    ops: &Value,
) -> std::result::Result<bool, RpcError> {
    let Some(Value::Array(stored)) = host
        .mounts
        .get(&owner)
        .and_then(|mount| mount.properties.get(property))
    else {
        return Err(ui_patch::stale());
    };
    let ops = ui_patch::parse(ops, stored.len())?;
    let alias = composition::global_alias(owner);
    let mut instances = vec![shell];
    instances.extend(floating);
    let mut converted = Vec::new();
    for instance in &instances {
        let current = instance
            .get_global_property(&alias, property)
            .map_err(anyhow::Error::from)?;
        let Some(rows) = host.patch_rows(owner, "$mount", property, &ops)? else {
            return Ok(false);
        };
        if !ui_patch::applicable(&current, ops.before, &ops.shapes, &rows.values) {
            return Ok(false);
        }
        converted.push((current, rows));
    }
    let mut written = None;
    for (current, rows) in converted {
        ui_patch::apply(&current, &ops.shapes, rows.values);
        written = Some(rows.written);
    }
    if let Some(written) = written {
        host.images.patched_rows(owner, "$mount", property, written);
    }
    let rows: Vec<Value> = ops.rows.iter().map(|row| (*row).clone()).collect();
    let shapes = ops.shapes;
    if let Some(Value::Array(stored)) = host
        .mounts
        .get_mut(&owner)
        .and_then(|mount| mount.properties.get_mut(property))
    {
        ui_patch::apply_vec(stored, &shapes, rows);
    }
    Ok(true)
}

fn interaction(
    host: &mut Host,
    reply: &Reply,
    params: &Value,
) -> std::result::Result<Value, RpcError> {
    let mount = host
        .mounts
        .get(&reply.owner)
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
    let boolean = |key: &str| {
        params[key]
            .as_bool()
            .ok_or_else(|| RpcError::invalid(format!("{key} boolean required")))
    };
    let locks = window_state::Locks {
        focused: boolean("focused")?,
        dragging: boolean("dragging")?,
        popup_open: boolean("popupOpen")?,
    };
    let slot = mount.spec.slot;
    host.policy.activity(host.now_ms());
    // The backdrop never auto-hides: nothing to lock.
    if let Some(slot) = policy_slot(slot) {
        host.policy
            .set_locks(slot, reply.owner, locks)
            .map_err(anyhow::Error::from)?;
    }
    Ok(Value::Null)
}

/// `ui.menu`: a panel's own context menu of its commands.
fn menu(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<Value, RpcError> {
    if debug_keys() {
        diag!(
            "keys: ui.menu owner={} menu_open={} dialog={} sidebar_effective={}",
            reply.owner,
            host.menu_open,
            host.dialog_busy,
            host.window_facts["slots"]["sidebar"]["effective"]
        );
    }
    let mount = host
        .mounts
        .get(&reply.owner)
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
    let items = params["items"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 32)
        .ok_or_else(|| RpcError::invalid("items requires 1..=32 entries"))?;
    let mut ids = Vec::new();
    for item in items {
        match item {
            Value::Null => ids.push(None),
            Value::String(id) if host.commands.get(reply.owner, id).is_some() => {
                ids.push(Some(id.clone()))
            }
            _ => {
                return Err(RpcError::new(
                    "missing",
                    format!("{item} is not a command of this plugin"),
                ));
            }
        }
    }
    let number = |key: &str| {
        params[key]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| RpcError::invalid(format!("{key} requires a finite number")))
    };
    let (x, y) = (number("x")?, number("y")?);
    let slot = match mount.spec.slot {
        composition::Slot::Bottom => "bottom",
        composition::Slot::Sidebar => "sidebar",
        composition::Slot::Backdrop => {
            return Err(RpcError::new(
                "unavailable",
                "the backdrop takes no pointer input",
            ));
        }
    };
    // A panel in a background tab is not shown (sidebar tabs).
    if slot == "sidebar" && host.policy.sidebar_active() != Some(reply.owner) {
        return Err(RpcError::new(
            "unavailable",
            "the panel's tab is not the current one",
        ));
    }
    let generation = if host
        .floating
        .as_ref()
        .is_some_and(|f| f.owners.contains(&reply.owner))
    {
        host.floating.as_ref().unwrap().generation
    } else if host.window_facts["slots"][slot]["effective"] == true {
        host.shell_generation
    } else {
        return Err(RpcError::new("unavailable", "the panel is not shown"));
    };
    if let Some(window) = host.window_for(generation) {
        window
            .set_property("menu-origin-x", x.into())
            .map_err(anyhow::Error::from)?;
        window
            .set_property("menu-origin-y", y.into())
            .map_err(anyhow::Error::from)?;
    }
    host.open_menu(generation, MenuMode::Panel(reply.owner, ids));
    Ok(Value::Null)
}

fn layout_set(host: &mut Host, params: &Value) -> std::result::Result<Value, RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "layout requires a native shell",
        ));
    }
    let slot = match string(params, "slot")? {
        "bottom" => composition::Slot::Bottom,
        "sidebar" => composition::Slot::Sidebar,
        _ => return Err(RpcError::invalid("unknown slot")),
    };
    let panels = host
        .mounts
        .values()
        .filter(|m| m.spec.slot == slot)
        .map(|m| (m.spec.owner, m.manifest.clone()))
        .collect::<Vec<_>>();
    if panels.is_empty() {
        return Err(RpcError::new("missing", "no panel at this position"));
    }
    let policy_slot =
        policy_slot(slot).ok_or_else(|| RpcError::invalid("the backdrop has no preference"))?;
    let LayoutChange {
        visible,
        detached,
        active,
    } = layout_change(params, slot, &panels)?;
    // The slot's state goes through its current panel (any tab's would do).
    let owner = match slot {
        composition::Slot::Sidebar => host.policy.sidebar_active(),
        _ => None,
    }
    .unwrap_or(panels[0].0);
    if let Some(active) = active
        && slot == composition::Slot::Sidebar
    {
        host.policy
            .set_active(active)
            .map_err(anyhow::Error::from)?;
    }
    if let Some(detached) = detached {
        host.policy
            .set_detached(owner, detached)
            .map_err(anyhow::Error::from)?;
    }
    if let Some(visible) = visible {
        let now = host.now_ms();
        host.policy
            .set_preferred(policy_slot, owner, visible, now)
            .map_err(anyhow::Error::from)?;
        if let Some(mount) = host.mounts.get_mut(&owner) {
            mount.visible = visible;
        }
    }
    host.remember_sidebar(active.is_some());
    // Answer after the next layout pass so a following layout.state sees it.
    host.window_tick();
    Ok(Value::Null)
}
