//! `ui.mount`: panels in the bottom bar, the sidebar and the backdrop, their
//! menus and interaction locks, and the layout of those slots.
use super::{Call, Capability, Handled, Info, Method, Recorded, Residency, plugin};
use crate::{
    host::{
        Host, LayoutChange, MenuMode, Mount, MountChange, Origin, Reply, RpcError, debug_keys,
        layout_change, policy_slot, refuse_initial, string,
    },
    i18n, process,
    ui::composition,
    ui::sidebar_tabs,
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
    /// A panel in the bottom bar or the backdrop keeps its plugin running
    /// (Resident). A sidebar tab is recorded, current or not (sidebar-tabs.md,
    /// user ruling 2026-11-03): its plugin is Normal, the current tab keeps
    /// it running as declared work (`Busy::ShownTab`), and a background tab
    /// stays as a placeholder while the plugin stands by.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        // A waiting change: live only for the bottom bar or the backdrop
        // (`sidebar_tabs::pending_live`); a waiting sidebar mount is what the
        // recording keeps.
        let waiting = waiting_mounts(host, owner);
        if sidebar_tabs::pending_live(waiting.iter().map(|(slot, _)| *slot)) {
            return Recorded::Live("a panel");
        }
        if let Some((_, call)) = waiting.iter().rev().find(|(slot, _)| slot.is_some()) {
            return Recorded::Calls(vec![Call::new("ui.mount", call.clone())]);
        }
        if !waiting.is_empty() {
            // Its last waiting change is an unmount: nothing to keep.
            return Recorded::Nothing;
        }
        match host.mounts.get(&owner) {
            None => Recorded::Nothing,
            Some(mount) if !sidebar_tabs::live(mount.spec.slot) => {
                Recorded::Calls(vec![Call::new("ui.mount", mount.call.clone())])
            }
            Some(_) => Recorded::Live("a panel"),
        }
    }
    /// A parked plugin woke as `to`: its tab moves with it (still a
    /// placeholder until its new mount takes it over). A branch compiled
    /// under `from` stays drawn for the tab meanwhile (`shell_compiled`).
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        let Some(mut mount) = host.mounts.remove(&from) else {
            return;
        };
        mount.spec.owner = to;
        host.mounts.insert(to, mount);
        host.policy.rekey(from, to);
        if let Some(alias) = host.shell_compiled.remove(&from) {
            host.shell_compiled.insert(to, alias);
        }
        let swap = |owner: &mut u64| {
            if *owner == from {
                *owner = to;
            }
        };
        host.shell_tabs.iter_mut().for_each(swap);
        if let Some(floating) = &mut host.floating {
            floating.owners.iter_mut().for_each(swap);
        }
        for owner in [
            &mut host.sidebar_active,
            &mut host.tab_woken,
            &mut host.input.borrow_mut().sidebar,
        ]
        .into_iter()
        .flatten()
        {
            swap(owner);
        }
    }
    /// Parked (`keep_recorded`): a sidebar tab stays, as a placeholder. Gone
    /// for good (disabled, uninstalled, a placeholder its plugin did not
    /// mount again): the placeholder goes. A running plugin's mount is
    /// removed by `Host::stop_run` itself.
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        let Some(mount) = host.mounts.get_mut(&owner) else {
            if !keep_recorded {
                // A replayed tab not applied yet.
                host.mount_queue
                    .retain(|c| c.reply.owner != owner || c.contribution.is_none());
            }
            return;
        };
        if keep_recorded {
            if mount.spec.slot == composition::Slot::Sidebar {
                mount.recorded = true;
            }
        } else if mount.recorded {
            host.mounts.remove(&owner);
            host.policy.remove_owner(owner);
            host.mount_queue.retain(|c| c.reply.owner != owner);
            host.mount_queue.push_front(MountChange {
                reply: Reply { owner, id: 0 },
                contribution: None,
                announced: true,
            });
        }
    }
    /// At a woken run's ready: a placeholder it did not mount again goes.
    fn prune(&self, host: &mut Host, owner: u64) {
        if host.mounts.get(&owner).is_some_and(|m| m.recorded) && !pending(host, owner) {
            self.revoke(host, owner, false);
        }
    }
    /// A recorded background tab: a placeholder in the sidebar, compiled
    /// into nothing (sidebar-tabs.md).
    fn replay(&self, host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
        if call.method != "ui.mount" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        if host.headless {
            return Ok(None);
        }
        let params = &call.params;
        if params["slot"] != "sidebar" {
            return Err(RpcError::invalid("only a sidebar tab is recorded"));
        }
        let builtin = host.is_builtin_package(&origin.id, &origin.directory);
        let (mut mount, _) = new_mount(
            owner,
            &origin.directory,
            &origin.id,
            params["name"].as_str().unwrap_or(""),
            |text| origin.text(text),
            builtin,
            params,
        )?;
        mount.recorded = true;
        mount.call = params.clone();
        host.mount_queue.retain(|c| c.reply.owner != owner);
        host.mount_queue.push_back(MountChange {
            reply: Reply { owner, id: 0 },
            contribution: Some(mount),
            announced: true,
        });
        Ok(None)
    }
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
    // The host's removal of this plugin's placeholder (`prune` at its
    // ready, an id 0 unmount) gives way to the plugin's own mount: a plugin
    // that mounts a little after its ready keeps its tab.
    if method == "ui.mount" {
        let removal = |c: &MountChange| {
            c.reply.owner == reply.owner && c.reply.id == 0 && c.contribution.is_none()
        };
        host.mount_queue.retain(|c| !removal(c));
        if host.preparing.as_ref().is_some_and(|(_, c)| removal(c)) {
            host.preparing = None;
        }
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
    // Placeholders replayed from recordings do not count (any number).
    if host
        .mount_queue
        .iter()
        .filter(|c| !c.contribution.as_ref().is_some_and(|m| m.recorded))
        .count()
        >= 8
    {
        return Err(RpcError::new("busy", "UI preparation queue full"));
    }
    let mut refused = Vec::new();
    let contribution = if method == "ui.mount" {
        // A placeholder tab of this plugin (sidebar-tabs.md): its woken run
        // mounts the panel again, which takes the tab over.
        if host.mounts.get(&reply.owner).is_some_and(|m| !m.recorded) {
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
        // compiles. The sidebar takes any number of tabs (sidebar-tabs.md).
        if slot != composition::Slot::Sidebar {
            let pending = host
                .mount_queue
                .iter()
                .chain(host.preparing.as_ref().map(|(_, change)| change))
                .map(|c| (c.reply.owner, c.contribution.as_ref().map(|m| m.spec.slot)))
                .collect::<Vec<_>>();
            let mounted = host.mounts.values().map(|m| (m.spec.owner, m.spec.slot));
            if let Some(holder) = slot_claims::occupant(slot, reply.owner, mounted, &pending) {
                let error = RpcError::slot_occupied(slot, &host.plugin_id(holder));
                let text = i18n::tr_args("Panel not shown: {}", &json!([error.message]));
                return Err(host.refuse_taken(reply.owner, error, text));
            }
        }
        let builtin = host.is_builtin(reply.owner);
        let plugin = plugin(host, reply)?;
        let name = plugin.manifest.name.clone().unwrap_or_default();
        let (mut mount, refusals) = new_mount(
            reply.owner,
            &plugin.directory,
            &plugin.manifest.id,
            &name,
            |text| plugin.text(text),
            builtin,
            params,
        )?;
        refused = refusals;
        mount.call = sidebar_tabs::recorded_call(params, &name);
        Some(mount)
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

/// A mount or unmount of `owner` waits to be applied.
fn pending(host: &Host, owner: u64) -> bool {
    host.mount_queue.iter().any(|c| c.reply.owner == owner)
        || host
            .preparing
            .as_ref()
            .is_some_and(|(_, c)| c.reply.owner == owner)
        || host.held_mounts.waiting(owner)
}

/// The mount changes of `owner` not applied yet, oldest first: its slot
/// (`None`: an unmount) and, for a mount, the call a recording keeps. Held
/// requests (non-built-ins while built-ins start) are read from their
/// parameters; an unknown slot counts as the bottom bar (live, the safe
/// side).
fn waiting_mounts(host: &Host, owner: u64) -> Vec<(Option<composition::Slot>, Value)> {
    let name = host
        .plugins
        .get(&owner)
        .and_then(|plugin| plugin.manifest.name.clone())
        .unwrap_or_default();
    let mut waiting = host
        .preparing
        .as_ref()
        .map(|(_, change)| change)
        .into_iter()
        .chain(host.mount_queue.iter())
        .filter(|change| change.reply.owner == owner)
        .map(|change| match &change.contribution {
            Some(mount) => (Some(mount.spec.slot), mount.call.clone()),
            None => (None, Value::Null),
        })
        .collect::<Vec<_>>();
    for (_, method, params) in host.held_mounts.held_of(owner) {
        waiting.push(if method == "ui.mount" {
            let slot = match params["slot"].as_str() {
                Some("sidebar") => composition::Slot::Sidebar,
                Some("backdrop") => composition::Slot::Backdrop,
                _ => composition::Slot::Bottom,
            };
            (Some(slot), sidebar_tabs::recorded_call(params, &name))
        } else {
            (None, Value::Null)
        });
    }
    waiting
}

/// A panel from `ui.mount`'s `params` (or a recorded call's, which has no
/// properties), in the package `directory` of plugin `manifest`: its
/// initial properties over the limits are left out and returned.
#[allow(clippy::too_many_arguments)]
fn new_mount(
    owner: u64,
    directory: &std::path::Path,
    manifest: &str,
    name: &str,
    text: impl Fn(&str) -> String,
    builtin: bool,
    params: &Value,
) -> std::result::Result<(Mount, Vec<RpcError>), RpcError> {
    let slot = match string(params, "slot")? {
        "bottom" => composition::Slot::Bottom,
        "sidebar" => composition::Slot::Sidebar,
        "backdrop" => composition::Slot::Backdrop,
        _ => return Err(RpcError::invalid("unknown mount slot")),
    };
    let file = process::package_file(directory, string(params, "file")?)?;
    let mut properties: BTreeMap<String, Value> =
        serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
            .map_err(|e| RpcError::invalid(e.to_string()))?;
    let title = params["title"].as_str().unwrap_or("");
    if title.len() > 128 || title.chars().any(char::is_control) {
        return Err(RpcError::invalid("title requires 0..=128 bytes"));
    }
    let refused = refuse_initial(&mut properties);
    // C8: a sidebar panel joins the slot's remembered state
    // (`remember_sidebar`, applied when it attaches).
    let mount = Mount {
        spec: composition::CompileSpec {
            owner,
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
            text(title)
        } else if !name.is_empty() {
            text(name)
        } else {
            "Vestra Player".into()
        },
        manifest: manifest.to_owned(),
        builtin,
        recorded: false,
        call: Value::Null,
    };
    Ok((mount, refused))
}

fn set_mount(
    host: &mut Host,
    reply: &Reply,
    params: &Value,
) -> std::result::Result<Value, RpcError> {
    // A large array converted on the plugin's reader thread: its whole
    // value was taken out of `params` (P3).
    let mut prepared = host.take_prepared(reply);
    let spec = host
        .mounts
        .get(&reply.owner)
        .filter(|mount| !mount.recorded)
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?
        .spec
        .clone();
    let shell = host
        .shell
        .as_ref()
        .map(ComponentHandle::clone_strong)
        .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
    // A sidebar tab beyond the front tabs is not in the shell (sidebar-tabs.md):
    // its values are only stored, and written when it is compiled.
    let shell = (spec.slot != composition::Slot::Sidebar
        || host.shell_compiled.get(&reply.owner) == Some(&reply.owner))
    .then_some(shell);
    let property = string(params, "property")?;
    let floating = host
        .floating
        .as_ref()
        .filter(|f| {
            f.compiled.contains(&reply.owner)
                && (spec.slot != composition::Slot::Sidebar
                    || host.shell_compiled.get(&reply.owner) == Some(&reply.owner))
        })
        .map(|f| f.instance.clone_strong());
    // A part of a whole value sent in parts: held until the last (P3).
    let parted = host.parts(reply.owner, "$mount", property, params, prepared.as_deref())?;
    let (seq, value, rows) = match parted {
        ui_patch::Parted::Held => return Ok(Value::Null),
        ui_patch::Parted::Done(value, rows, seq) => (seq, value, Some(rows)),
        ui_patch::Parted::Not => {
            let seq = if let Some(patch) = params.get("patch") {
                let (base, seq, ops) = ui_patch::request(patch)?;
                if host.images.seq(reply.owner, "$mount", property) != base {
                    return Err(ui_patch::stale());
                }
                if patch_mount(
                    host,
                    reply.owner,
                    shell.as_ref(),
                    floating.as_ref(),
                    property,
                    ops,
                    prepared.take(),
                )? {
                    host.images.set_seq(reply.owner, "$mount", property, seq);
                    return Ok(Value::Null);
                }
                seq
            } else {
                params["seq"].as_u64().unwrap_or(0)
            };
            // A whole value, or a patched one that a live model could not
            // take in place.
            match (params.get("patch"), prepared) {
                (Some(patch), _) => (
                    seq,
                    patched_mount(host, reply.owner, property, &patch["ops"])?,
                    None,
                ),
                (None, Some(prepared)) => {
                    let prepared = *prepared;
                    (seq, prepared.whole.unwrap_or_default(), Some(prepared.rows))
                }
                (None, None) => (seq, params["value"].clone(), None),
            }
        }
    };
    let converted = host.prop_value_with(reply.owner, "$mount", property, &value, rows.as_ref())?;
    if let Some(shell) = &shell {
        composition::set_global_value(shell, &spec, property, converted)?;
    }
    if let Some(floating) = floating {
        let converted =
            host.prop_value_with(reply.owner, "$mount", property, &value, rows.as_ref())?;
        composition::set_global_value(&floating, &spec, property, converted)?;
    }
    if let Some(mount) = host.mounts.get_mut(&reply.owner) {
        mount.properties.insert(property.into(), value);
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
    shell: Option<&slint_interpreter::ComponentInstance>,
    floating: Option<&slint_interpreter::ComponentInstance>,
    property: &str,
    ops: &Value,
    prepared: Option<Box<crate::ui::ui_values::Prepared>>,
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
    let mut instances = shell.into_iter().collect::<Vec<_>>();
    instances.extend(floating);
    // Converted once (on the reader thread when large), written into each
    // instance's model.
    let Some(rows) = host.patch_rows(
        owner,
        "$mount",
        property,
        &ops,
        prepared.as_ref().map(|prepared| &prepared.rows),
    )?
    else {
        return Ok(false);
    };
    let mut currents = Vec::new();
    for instance in &instances {
        let current = instance
            .get_global_property(&alias, property)
            .map_err(anyhow::Error::from)?;
        if !ui_patch::applicable(&current, ops.before, &ops.shapes, &rows) {
            return Ok(false);
        }
        currents.push(current);
    }
    let ui_patch::Rows {
        values,
        images,
        written,
    } = rows;
    for current in currents {
        ui_patch::apply(&current, &ops.shapes, values.clone(), &images);
    }
    host.images.patched_rows(owner, "$mount", property, written);
    // The stored rows: copied on the reader thread when large.
    let carried: Vec<Value> = match prepared {
        Some(prepared) if prepared.carried.len() == ops.rows.len() => prepared.carried,
        _ => ops.rows.iter().map(|row| (*row).clone()).collect(),
    };
    let shapes = ops.shapes;
    if let Some(Value::Array(stored)) = host
        .mounts
        .get_mut(&owner)
        .and_then(|mount| mount.properties.get_mut(property))
    {
        ui_patch::apply_vec(stored, &shapes, carried);
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
