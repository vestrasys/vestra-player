//! `ui.mount`: panels in the bottom bar, the sidebar and the backdrop, their
//! menus and interaction locks, and the layout of those slots.
use super::{Call, Capability, Handled, Info, Method, Recorded, Residency, plugin};
use crate::{
    host::{
        Host, LayoutChange, MenuMode, Mount, MountChange, Origin, Reply, RpcError, debug_keys,
        layout_change, mount_view, panel_key, policy_slot, refuse_initial, string,
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
    builtin_only: false,
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
    /// (Resident). A tab (the sidebar, the adjust column) is recorded,
    /// current or not (sidebar-tabs.md, user ruling 2026-11-03): its plugin
    /// is Normal, the current tab keeps it running as declared work
    /// (`Busy::ShownTab`), and a background tab stays as a placeholder while
    /// the plugin stands by. A plugin may hold one panel per slot (SDK
    /// 1.6.15): any live one makes it Resident; its tabs are recorded, one
    /// call each.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        // What each of its panels will be once the waiting changes apply:
        // the mounted ones, then the waiting changes in order (a waiting
        // bottom bar or backdrop is live, `sidebar_tabs::pending_live`).
        let mut panels: BTreeMap<u64, (composition::Slot, Value)> = host
            .mounts
            .values()
            .filter(|m| m.plugin == owner)
            .map(|m| (m.spec.owner, (m.spec.slot, m.call.clone())))
            .collect();
        for (key, slot, call) in waiting_mounts(host, owner) {
            match slot {
                Some(slot) => panels.insert(key, (slot, call)),
                None => panels.remove(&key),
            };
        }
        if panels.values().any(|(slot, _)| sidebar_tabs::live(*slot)) {
            return Recorded::Live("a panel");
        }
        if panels.is_empty() {
            return Recorded::Nothing;
        }
        Recorded::Calls(
            panels
                .into_values()
                .map(|(_, call)| Call::new("ui.mount", call))
                .collect(),
        )
    }
    /// A parked plugin woke as `to`: its tabs move with it (still
    /// placeholders until its new mounts take them over). A branch compiled
    /// under the old key stays drawn for the tab meanwhile (`shell_compiled`).
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        rekey_panels(host, from, to);
    }
    /// Parked (`keep_recorded`): its tabs stay, as placeholders. Gone for
    /// good (disabled, uninstalled, a placeholder its plugin did not mount
    /// again): the placeholders go. A running plugin's mounts are removed by
    /// `Host::stop_run` itself.
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        let keys = host
            .mounts
            .values()
            .filter(|m| m.plugin == owner)
            .map(|m| m.spec.owner)
            .collect::<Vec<_>>();
        if !keep_recorded {
            // Its mounts not applied yet (replayed tabs among them) go too,
            // also when another of its panels is applied: a queued
            // placeholder would otherwise stay without its plugin.
            forget_queued(&mut host.mount_queue, owner);
        }
        for key in keys {
            revoke_panel(host, key, keep_recorded);
        }
    }
    /// At a woken run's ready: a placeholder it did not mount again goes.
    fn prune(&self, host: &mut Host, owner: u64) {
        let stale = host
            .mounts
            .values()
            .filter(|m| m.plugin == owner && m.recorded)
            .map(|m| m.spec.owner)
            .filter(|&key| !pending(host, owner, key))
            .collect::<Vec<_>>();
        for key in stale {
            revoke_panel(host, key, false);
        }
    }
    /// A recorded tab: a placeholder in its column, compiled into nothing
    /// (sidebar-tabs.md).
    fn replay(&self, host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
        if call.method != "ui.mount" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        if host.headless {
            return Ok(None);
        }
        let params = &call.params;
        if !params["slot"]
            .as_str()
            .and_then(composition::Slot::parse)
            .is_some_and(composition::Slot::tabbed)
        {
            return Err(RpcError::invalid(
                "only a tab of the sidebar or the adjust column is recorded",
            ));
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
        let key = mount.spec.owner;
        host.mount_queue.retain(|c| c.key != key);
        host.mount_queue.push_back(MountChange {
            reply: Reply { owner, id: 0 },
            key,
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

/// The panels of run `from` move to run `to` (a parked plugin woke as
/// `to`; a crashed plugin's kept panels go to its restart,
/// `panel_restart.rs`): its tabs stay placeholders until its new mounts
/// take them over, and a branch compiled under the old key stays drawn
/// meanwhile (`shell_compiled`).
pub(crate) fn rekey_panels(host: &mut Host, from: u64, to: u64) {
    let keys = host
        .mounts
        .values()
        .filter(|m| m.plugin == from)
        .map(|m| (m.spec.owner, m.spec.slot))
        .collect::<Vec<_>>();
    for (old, slot) in keys {
        let new = panel_key(to, slot);
        let Some(mut mount) = host.mounts.remove(&old) else {
            continue;
        };
        mount.spec.owner = new;
        mount.plugin = to;
        host.mounts.insert(new, mount);
        host.policy.rekey(old, new);
        if let Some(alias) = host.shell_compiled.remove(&old) {
            host.shell_compiled.insert(new, alias);
        }
        let swap = |key: &mut u64| {
            if *key == old {
                *key = new;
            }
        };
        if let Some(floating) = &mut host.floating {
            floating.owners.iter_mut().for_each(swap);
        }
        for state in &mut host.columns {
            state.tabs.iter_mut().for_each(swap);
            for key in [&mut state.active, &mut state.woken].into_iter().flatten() {
                swap(key);
            }
        }
    }
    // The panel keys' owners (plugins).
    let mut shared = host.input.borrow_mut();
    let input = &mut *shared;
    for owner in [&mut input.sidebar, &mut input.adjust]
        .into_iter()
        .flatten()
    {
        if *owner == from {
            *owner = to;
        }
    }
}

/// Drop the mounts of plugin `owner` that wait in `queue` (its unmounts
/// stay: they remove what is applied).
fn forget_queued(queue: &mut std::collections::VecDeque<MountChange>, owner: u64) {
    queue.retain(|c| c.reply.owner != owner || c.contribution.is_none());
}

/// [`UiMount::revoke`] for one panel.
fn revoke_panel(host: &mut Host, key: u64, keep_recorded: bool) {
    let Some(mount) = host.mounts.get_mut(&key) else {
        return;
    };
    if keep_recorded {
        if mount.spec.slot.tabbed() {
            mount.recorded = true;
        }
    } else if mount.recorded {
        let owner = mount.plugin;
        host.mounts.remove(&key);
        host.policy.remove_owner(key);
        host.mount_queue.retain(|c| c.key != key);
        host.mount_queue.push_front(MountChange {
            reply: Reply { owner, id: 0 },
            key,
            contribution: None,
            announced: true,
        });
    }
}

/// The panel a `ui.setMount`, `ui.interaction`, `ui.menu` or `ui.unmount`
/// is about (SDK 1.6.15: `slot`): the plugin's panel in `slot`, or, without
/// one (SDKs before 1.6.15, one panel each), its only panel. `None`: it
/// has none.
fn panel_of(host: &Host, owner: u64, params: &Value) -> std::result::Result<Option<u64>, RpcError> {
    if let Some(slot) = params.get("slot").filter(|v| !v.is_null()) {
        let slot = slot
            .as_str()
            .and_then(composition::Slot::parse)
            .ok_or_else(|| RpcError::invalid("unknown mount slot"))?;
        return Ok(Some(panel_key(owner, slot)));
    }
    let mut keys = host
        .mounts
        .values()
        .filter(|m| m.plugin == owner && !m.recorded && !host.restarts.waiting(m.spec.owner))
        .map(|m| m.spec.owner);
    match (keys.next(), keys.next()) {
        (key, None) => Ok(key),
        _ => Err(RpcError::invalid(
            "this plugin has several panels: name the slot",
        )),
    }
}

/// `ui.mount` and `ui.unmount`: queued for the next shell preparation. A
/// plugin mounts one panel per slot (SDK 1.6.15); `ui.unmount` takes the
/// slot's panel, or without `slot` every panel of the plugin.
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
    // Panels kept over this plugin's restart belong to this run from now
    // on (normally done at its spawn already, `panel_restart.rs`).
    let id = host.plugin_id(reply.owner);
    host.adopt_panels(&id, reply.owner);
    if method == "ui.unmount" {
        let keys = match params.get("slot").filter(|v| !v.is_null()) {
            Some(_) => panel_of(host, reply.owner, params)?.into_iter().collect(),
            // A panel kept over this plugin's restart is its too: removed
            // now, not left for `REMOUNT_LIMIT` (with `slot` it is the
            // slot's panel either way).
            None => host
                .mounts
                .values()
                .filter(|m| m.plugin == reply.owner && !m.recorded)
                .map(|m| m.spec.owner)
                .collect::<Vec<_>>(),
        };
        for key in keys.iter().copied() {
            if pending_change(host, key) {
                return Err(RpcError::new(
                    "busy",
                    "this panel already has a pending UI preparation",
                ));
            }
        }
        if keys.is_empty() {
            // Answered at the next preparation, as before: nothing to remove.
            host.mount_queue.push_back(MountChange {
                reply: reply.clone(),
                key: panel_key(reply.owner, composition::Slot::Bottom),
                contribution: None,
                announced: false,
            });
        }
        for (index, key) in keys.iter().copied().enumerate() {
            // One answer for the call: the last change carries the reply.
            let last = index + 1 == keys.len();
            host.mount_queue.push_back(MountChange {
                reply: if last {
                    reply.clone()
                } else {
                    Reply {
                        owner: reply.owner,
                        id: 0,
                    }
                },
                key,
                contribution: None,
                announced: !last,
            });
        }
        return Ok(());
    }
    let slot = composition::Slot::parse(string(params, "slot")?)
        .ok_or_else(|| RpcError::invalid("unknown mount slot"))?;
    let key = panel_key(reply.owner, slot);
    // The host's removal of this panel's placeholder (`prune` at its
    // plugin's ready, an id 0 unmount) gives way to the plugin's own mount:
    // a plugin that mounts a little after its ready keeps its tab.
    let removal = |c: &MountChange| c.key == key && c.reply.id == 0 && c.contribution.is_none();
    host.mount_queue.retain(|c| !removal(c));
    if host.preparing.as_ref().is_some_and(|(_, c)| removal(c)) {
        host.preparing = None;
    }
    if pending_change(host, key) {
        return Err(RpcError::new(
            "busy",
            "this panel already has a pending UI preparation",
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
    // One panel per slot and plugin; a placeholder tab of this plugin
    // (sidebar-tabs.md) is taken over by its woken run's mount, a panel
    // kept over its restart by its restarted run's (`panel_restart.rs`).
    if host.mounts.get(&key).is_some_and(|m| !m.recorded) && !host.restarts.waiting(key) {
        return Err(RpcError::new(
            "conflict",
            "this plugin already has a panel in this slot",
        ));
    }
    // One component in the bottom bar and the backdrop (R-U3), whoever
    // mounted it: refused here, not when the shell compiles. The sidebar
    // and the adjust column take any number of tabs (sidebar-tabs.md).
    if !slot.tabbed() {
        let pending = host
            .mount_queue
            .iter()
            .chain(host.preparing.as_ref().map(|(_, change)| change))
            .map(|c| (c.reply.owner, c.contribution.as_ref().map(|m| m.spec.slot)))
            .collect::<Vec<_>>();
        let mounted = host.mounts.values().map(|m| (m.plugin, m.spec.slot));
        if let Some(holder) = slot_claims::occupant(slot, reply.owner, mounted, &pending) {
            let error = RpcError::slot_occupied(slot, &host.plugin_id(holder));
            let text = i18n::tr_args("Panel not shown: {}", &json!([error.message]));
            return Err(host.refuse_taken(reply.owner, error, text));
        }
    }
    let builtin = host.is_builtin(reply.owner);
    let plugin = plugin(host, reply)?;
    let name = plugin.manifest.name.clone().unwrap_or_default();
    let (mut mount, refused) = new_mount(
        reply.owner,
        &plugin.directory,
        &plugin.manifest.id,
        &name,
        |text| plugin.text(text),
        builtin,
        params,
    )?;
    mount.call = sidebar_tabs::recorded_call(params, &name);
    host.report_refused(reply.owner, refused);
    // Row patches apply on the initial properties again (seq 0).
    host.images.reset_writes(reply.owner, mount_view(slot));
    host.mount_queue.push_back(MountChange {
        reply: reply.clone(),
        key,
        contribution: Some(mount),
        announced: false,
    });
    Ok(())
}

/// A change of panel `key` waits to be applied.
fn pending_change(host: &Host, key: u64) -> bool {
    host.mount_queue.iter().any(|c| c.key == key)
        || host.preparing.as_ref().is_some_and(|(_, c)| c.key == key)
}

/// A mount or unmount of panel `key` of plugin `owner` waits to be applied
/// (a held request of the plugin counts for all its panels).
fn pending(host: &Host, owner: u64, key: u64) -> bool {
    pending_change(host, key) || host.held_mounts.waiting(owner)
}

/// The mount changes of `owner`'s panels not applied yet, oldest first: the
/// panel key, its slot (`None`: an unmount) and, for a mount, the call a
/// recording keeps. Held requests (non-built-ins while built-ins start) are
/// read from their parameters; an unknown slot counts as the bottom bar
/// (live, the safe side), and a held unmount without a slot as all panels.
fn waiting_mounts(host: &Host, owner: u64) -> Vec<(u64, Option<composition::Slot>, Value)> {
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
            Some(mount) => (change.key, Some(mount.spec.slot), mount.call.clone()),
            None => (change.key, None, Value::Null),
        })
        .collect::<Vec<_>>();
    let mounted = host
        .mounts
        .values()
        .filter(|m| m.plugin == owner)
        .map(|m| m.spec.owner)
        .collect::<Vec<_>>();
    for (_, method, params) in host.held_mounts.held_of(owner) {
        let slot = params["slot"].as_str().and_then(composition::Slot::parse);
        if method == "ui.mount" {
            let slot = slot.unwrap_or(composition::Slot::Bottom);
            waiting.push((
                panel_key(owner, slot),
                Some(slot),
                sidebar_tabs::recorded_call(params, &name),
            ));
        } else if let Some(slot) = slot {
            waiting.push((panel_key(owner, slot), None, Value::Null));
        } else {
            waiting.extend(mounted.iter().map(|&key| (key, None, Value::Null)));
        }
    }
    waiting
}

/// A panel from `ui.mount`'s `params` (or a recorded call's, which has no
/// properties), in the package `directory` of plugin `manifest` (`owner`):
/// its initial properties over the limits are left out and returned. Its
/// key is `panel_key(owner, slot)`.
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
    let slot = composition::Slot::parse(string(params, "slot")?)
        .ok_or_else(|| RpcError::invalid("unknown mount slot"))?;
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
            owner: panel_key(owner, slot),
            file,
            component: string(params, "component")?.into(),
            global: Some(string(params, "global")?.into()),
            slot,
        },
        plugin: owner,
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
    let key = panel_of(host, reply.owner, params)?
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
    let mut spec = host
        .mounts
        .get(&key)
        .filter(|mount| !mount.recorded && !host.restarts.waiting(key))
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?
        .spec
        .clone();
    let view = mount_view(spec.slot);
    let shell = host
        .shell
        .as_ref()
        .map(ComponentHandle::clone_strong)
        .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
    // A tab beyond the front tabs is not in the shell (sidebar-tabs.md):
    // its values are only stored, and written when it is compiled. A panel
    // kept over its plugin's restart is written under the key its branch
    // was compiled under (`panel_restart.rs`).
    let branch = host.shell_branch(key, spec.slot.tabbed());
    spec.owner = branch.unwrap_or(key);
    let shell = branch.is_some().then_some(shell);
    let property = string(params, "property")?;
    let floating = host
        .floating
        .as_ref()
        .filter(|f| branch.is_some_and(|branch| f.compiled.contains(&branch)))
        .map(|f| f.instance.clone_strong());
    // A part of a whole value sent in parts: held until the last (P3).
    let parted = host.parts(reply.owner, view, property, params, prepared.as_deref())?;
    let (seq, value, rows) = match parted {
        ui_patch::Parted::Held => return Ok(Value::Null),
        ui_patch::Parted::Done(value, rows, seq) => (seq, value, Some(rows)),
        ui_patch::Parted::Not => {
            let seq = if let Some(patch) = params.get("patch") {
                let (base, seq, ops) = ui_patch::request(patch)?;
                if host.images.seq(reply.owner, view, property) != base {
                    return Err(ui_patch::stale());
                }
                if patch_mount(
                    host,
                    reply.owner,
                    key,
                    shell.as_ref(),
                    floating.as_ref(),
                    property,
                    ops,
                    prepared.take(),
                )? {
                    host.images.set_seq(reply.owner, view, property, seq);
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
                    patched_mount(host, key, property, &patch["ops"])?,
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
    let converted = host.prop_value_with(reply.owner, view, property, &value, rows.as_ref())?;
    if let Some(shell) = &shell {
        composition::set_global_value(shell, &spec, property, converted)?;
    }
    if let Some(floating) = floating {
        let converted = host.prop_value_with(reply.owner, view, property, &value, rows.as_ref())?;
        composition::set_global_value(&floating, &spec, property, converted)?;
    }
    if let Some(mount) = host.mounts.get_mut(&key) {
        mount.properties.insert(property.into(), value);
    }
    host.images.set_seq(reply.owner, view, property, seq);
    Ok(Value::Null)
}

/// The stored array of a panel's property with `ops` applied.
fn patched_mount(
    host: &Host,
    key: u64,
    property: &str,
    ops: &Value,
) -> std::result::Result<Value, RpcError> {
    let Some(Value::Array(stored)) = host
        .mounts
        .get(&key)
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
#[allow(clippy::too_many_arguments)]
fn patch_mount(
    host: &mut Host,
    owner: u64,
    key: u64,
    shell: Option<&slint_interpreter::ComponentInstance>,
    floating: Option<&slint_interpreter::ComponentInstance>,
    property: &str,
    ops: &Value,
    prepared: Option<Box<crate::ui::ui_values::Prepared>>,
) -> std::result::Result<bool, RpcError> {
    let Some(mount) = host.mounts.get(&key) else {
        return Err(ui_patch::stale());
    };
    let view = mount_view(mount.spec.slot);
    let Some(Value::Array(stored)) = mount.properties.get(property) else {
        return Err(ui_patch::stale());
    };
    let ops = ui_patch::parse(ops, stored.len())?;
    let alias = composition::global_alias(host.restarts.compiled_key(key));
    let mut instances = shell.into_iter().collect::<Vec<_>>();
    instances.extend(floating);
    // Converted once (on the reader thread when large), written into each
    // instance's model.
    let Some(rows) = host.patch_rows(
        owner,
        view,
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
    host.images.patched_rows(owner, view, property, written);
    // The stored rows: copied on the reader thread when large.
    let carried: Vec<Value> = match prepared {
        Some(prepared) if prepared.carried.len() == ops.rows.len() => prepared.carried,
        _ => ops.rows.iter().map(|row| (*row).clone()).collect(),
    };
    let shapes = ops.shapes;
    if let Some(Value::Array(stored)) = host
        .mounts
        .get_mut(&key)
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
    let key = panel_of(host, reply.owner, params)?
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
    let mount = host
        .mounts
        .get(&key)
        .filter(|_| !host.restarts.waiting(key))
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
            .set_locks(slot, key, locks)
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
    let key = panel_of(host, reply.owner, params)?
        .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
    let mount = host
        .mounts
        .get(&key)
        .filter(|_| !host.restarts.waiting(key))
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
    let mount_slot = mount.spec.slot;
    if mount_slot == composition::Slot::Backdrop {
        return Err(RpcError::new(
            "unavailable",
            "the backdrop takes no pointer input",
        ));
    }
    let slot = mount_slot.name();
    // A panel in a background tab is not shown (the columns' tabs).
    if let Some(policy) = policy_slot(mount_slot).filter(|_| mount_slot.tabbed())
        && host.policy.active(policy) != Some(key)
    {
        return Err(RpcError::new(
            "unavailable",
            "the panel's tab is not the current one",
        ));
    }
    let generation = if mount_slot == composition::Slot::Sidebar
        && host
            .floating
            .as_ref()
            .is_some_and(|f| f.owners.contains(&key))
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
    let slot = composition::Slot::parse(string(params, "slot")?)
        .filter(|slot| *slot != composition::Slot::Backdrop)
        .ok_or_else(|| RpcError::invalid("unknown slot"))?;
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
    let owner = slot
        .tabbed()
        .then(|| host.policy.active(policy_slot))
        .flatten()
        .unwrap_or(panels[0].0);
    if let Some(active) = active
        && slot.tabbed()
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
        // `reveal: false` (SDK 1.6.13 `toggle()`): a switch, which in
        // fullscreen pops nothing out (user ruling 2026-11-08); absent or
        // true (`show()`, `setVisible`, older SDKs): reveal, as before.
        let reveal = params["reveal"].as_bool().unwrap_or(true);
        if reveal {
            host.policy.set_preferred(policy_slot, owner, visible, now)
        } else {
            host.policy.switch(policy_slot, owner, visible, now)
        }
        .map_err(anyhow::Error::from)?;
        if let Some(mount) = host.mounts.get_mut(&owner) {
            mount.visible = visible;
        }
    }
    if let Some(column) = sidebar_tabs::Column::of(slot) {
        host.remember_column(column, active.is_some());
    }
    // Answer after the next layout pass so a following layout.state sees it.
    host.window_tick();
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(owner: u64, slot: composition::Slot, mount: bool) -> MountChange {
        let key = panel_key(owner, slot);
        MountChange {
            reply: Reply { owner, id: 0 },
            key,
            contribution: mount.then(|| Mount {
                spec: composition::CompileSpec {
                    owner: key,
                    file: std::path::PathBuf::from("tab.slint"),
                    component: "Tab".into(),
                    global: Some("Bridge".into()),
                    slot,
                },
                plugin: owner,
                properties: BTreeMap::new(),
                visible: true,
                detached: false,
                title: "Tab".into(),
                manifest: "a.b".into(),
                builtin: false,
                recorded: true,
                call: Value::Null,
            }),
            announced: true,
        }
    }

    /// Review of the multi-panel change: a plugin gone for good (disabled,
    /// uninstalled) with one tab applied and another replayed but still
    /// queued loses the queued one too (`UiMount::revoke` always forgets its
    /// queued mounts), so no placeholder is left without its plugin; its
    /// unmounts and other plugins' changes stay.
    #[test]
    fn a_plugin_gone_for_good_leaves_no_queued_placeholder() {
        use composition::Slot;
        let mut queue = std::collections::VecDeque::from([
            // Its adjust tab replayed, not applied yet (its sidebar tab is).
            change(7, Slot::Adjust, true),
            change(9, Slot::Sidebar, true),
            change(7, Slot::Bottom, false),
        ]);
        forget_queued(&mut queue, 7);
        let left = queue
            .iter()
            .map(|c| (c.reply.owner, c.key, c.contribution.is_some()))
            .collect::<Vec<_>>();
        assert_eq!(
            left,
            [
                (9, panel_key(9, Slot::Sidebar), true),
                (7, panel_key(7, Slot::Bottom), false),
            ]
        );
    }
}
