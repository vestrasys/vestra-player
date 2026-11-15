//! `plugin.bindings` (`bindings.manage`): list every command's bindings, write
//! the user overlay, and tell managing plugins when anything changed.
#![forbid(unsafe_code)]
use crate::bindings;
use crate::host::*;
use serde_json::{Value, json};

/// A command as plugins name it: manifest id (`host` for the player) and command id.
fn reference(params: &Value) -> std::result::Result<(String, String), RpcError> {
    let plugin = string(params, "plugin")?;
    let command = string(params, "command")?;
    let valid = |text: &str| {
        !text.is_empty()
            && text.len() <= 128
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    };
    if !valid(plugin) || !valid(command) {
        return Err(RpcError::invalid("plugin and command must be identifiers"));
    }
    Ok((plugin.into(), command.into()))
}

fn json_ref(manifest: &str, id: &str) -> Value {
    json!({"plugin":manifest,"command":id})
}

/// One command of `bindings.list`: its bindings and how it is named.
fn command_row(info: &bindings::Info, declaration: &crate::commands::Declaration) -> Value {
    let texts = |inputs: &[bindings::Input]| inputs.iter().map(bindings::text).collect::<Vec<_>>();
    json!({
        "plugin":info.manifest,"command":info.id,
        // What the user sees: the interface language.
        "title":declaration.shown_title(),
        "path":(!declaration.path.is_empty()).then(|| declaration.shown_path()),
        // The keys menus merge by: the same in every language.
        "key":(!declaration.path.is_empty()).then(|| declaration.path.clone()),
        // Player 0.7.6: the category as shown, null without one.
        "category":declaration.shown_category(),
        "scope":if info.panel {"panel"} else {"global"},
        "defaults":texts(&info.defaults),"current":texts(&info.current),
        "overridden":info.overridden,
        "shadowed":info.shadowed.iter().map(|(input, manifest, id)| {
            json!({"input":bindings::text(input),"by":json_ref(manifest, id)})
        }).collect::<Vec<_>>(),
        "labels":info.current.iter().map(bindings::label).collect::<Vec<_>>(),
    })
}

/// `bindings.list`: every command the user can bind, and the overlay's
/// entries for commands nothing declares (`orphans`).
///
/// A `focus: "panel"` command fires only while its plugin's panel has the
/// keys, so one of a plugin that has no panel (in the bottom bar, the
/// sidebar, the adjust column or behind the video) can never fire from a
/// key: it is left out, so the Keyboard Shortcuts page offers no dead key
/// for it. It stays registered and runnable (`commands.run`, the plugin's
/// own controls), and a user binding stored for it stays in the overlay (it
/// is declared, so no orphan either). Decided by whether the plugin has
/// mounted a panel in this run (`Host::panel_plugins`, a replayed tab's
/// placeholder included; player 0.9.0, before: whether its manifest
/// declared `ui.mount`), so the list does not change as panels come and
/// go or as the plugin parks: it only grows, once per plugin.
fn list<'a>(
    table: &bindings::Table,
    declaration: impl Fn(&bindings::Info) -> Option<&'a crate::commands::Declaration>,
    mut has_panel: impl FnMut(&str) -> bool,
) -> Value {
    let commands = table
        .infos()
        .into_iter()
        .filter(|info| !info.panel || has_panel(&info.manifest))
        .filter_map(|info| Some(command_row(&info, declaration(&info)?)))
        .collect::<Vec<_>>();
    let orphans = table
        .orphans()
        .into_iter()
        .filter_map(|(key, inputs)| {
            let (manifest, id) = key.split_once('/')?;
            Some(json!({"ref":json_ref(manifest, id),"inputs":inputs}))
        })
        .collect::<Vec<_>>();
    json!({"commands":commands,"orphans":orphans})
}

impl Host {
    pub(super) fn bindings_dispatch(
        &mut self,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        match method {
            "bindings.list" => Ok(self.bindings_list()),
            "bindings.set" => {
                let (plugin, command) = reference(params)?;
                let inputs: Vec<String> = serde_json::from_value(params["inputs"].clone())
                    .map_err(|_| RpcError::invalid("inputs must be a list of bindings"))?;
                if inputs.len() > 16 {
                    return Err(RpcError::invalid("at most 16 bindings per command"));
                }
                let inputs = bindings::Table::canonical(&inputs).map_err(RpcError::invalid)?;
                let key = format!("{plugin}/{command}");
                if !self.core.bindings().declares(&plugin, &command)
                    && !self.ui_state.state.bindings.contains_key(&key)
                {
                    return Err(RpcError::new("missing", format!("no command {key}")));
                }
                self.ui_state.state.bindings.insert(key, inputs);
                self.overlay_changed();
                Ok(Value::Null)
            }
            "bindings.reset" => {
                if params["plugin"].is_null() && params["command"].is_null() {
                    self.ui_state.state.bindings.clear();
                } else {
                    let (plugin, command) = reference(params)?;
                    let key = format!("{plugin}/{command}");
                    if self.ui_state.state.bindings.remove(&key).is_none()
                        && !self.core.bindings().declares(&plugin, &command)
                    {
                        return Err(RpcError::new("missing", format!("no command {key}")));
                    }
                }
                self.overlay_changed();
                Ok(Value::Null)
            }
            _ => Err(RpcError::new("unsupported", "unknown bindings method")),
        }
    }
    /// The overlay is kept in the interface state file (debounced save).
    fn overlay_changed(&mut self) {
        let overlay = self.ui_state.state.bindings.clone();
        self.core.bindings_mut().set_overlay(overlay);
        let now = self.core.now_ms();
        self.ui_state.touch(now);
        if self.menu_open {
            let result = self.refresh_menu();
            self.report_menu(result);
        }
    }
    fn bindings_list(&self) -> Value {
        let table = self.core.bindings();
        list(
            table,
            |info| Some(&self.core.commands.get(info.owner, &info.id)?.declaration),
            |manifest| self.core.panel_plugins.contains(manifest),
        )
    }
    /// `bindings.changed` (declarations or overlay) and `packages.changed`
    /// (the plugin list) to the plugins that subscribed; at most every 200 ms.
    pub(super) fn notify_tick(&mut self) {
        let now = self.core.now_ms();
        if now < self.core.notified_at + 200 {
            return;
        }
        self.core.notified_at = now;
        let (revision, key) = {
            let table = self.core.bindings();
            (
                table.revision(),
                [
                    table.listed_revision(),
                    self.core.commands.listed_revision(),
                ],
            )
        };
        if revision != self.core.bindings_sent {
            self.core.bindings_sent = revision;
            // Only a change of the list: a table rebuilt to the same list
            // (a plugin parked or woken under a new owner, lazy activation
            // §6.9) is not one, so it wakes no parked listener.
            if !self.listened("bindings.changed") {
                self.core.bindings_list_sent = Value::Null;
            } else {
                // Built and compared only when what it shows may have
                // changed (supervisor.md): during a wake storm every
                // woken plugin's commands move to its new owner, and the
                // list took 3–6 ms to build each time.
                let moved =
                    key != self.core.bindings_seen || self.core.bindings_list_sent.is_null();
                if moved || cfg!(debug_assertions) {
                    let list = in_order(self.bindings_list());
                    let same = list == self.core.bindings_list_sent;
                    // Debug builds (every test with the real player) build
                    // it anyway: unchanged sources must give the same list.
                    assert!(
                        moved || same,
                        "bindings.list changed while its sources did not: {key:?}"
                    );
                    if !same {
                        // The first list of a run is no change for a parked plugin.
                        let first = self.core.bindings_list_sent.is_null();
                        self.core.bindings_list_sent = list;
                        self.announce("bindings.changed", Value::Null, first);
                    }
                    self.core.bindings_seen = key;
                }
            }
        }
        if revision != self.core.keys_checked {
            self.core.keys_checked = revision;
            self.push_command_keys();
        }
        if self.listened("packages.changed") {
            // Built and compared only once a source moved (supervisor.md
            // §10): a list of about 200 packages took 4–12 ms to build, every
            // 200 ms, also when nothing happened.
            // One clock reading for both: a start deadline passing between
            // two would change the list but not the key.
            let now = self.core.now_ms();
            let key = self.packages_key(now);
            let moved = key.is_none() || key != self.core.packages_seen;
            if moved || cfg!(debug_assertions) {
                let list = self.package_list_at(now);
                // A park or a wake alone changes only `process`: no event (§5.3).
                let same = same_but_process(&list, &self.core.packages_sent);
                // Debug builds (every test with the real player) build it
                // anyway: unchanged sources must give the same list.
                assert!(
                    moved || same,
                    "packages.list changed while its sources did not: {key:?}"
                );
                if !same {
                    let first = self.core.packages_sent.is_null();
                    self.core.packages_sent = list.clone();
                    self.announce("packages.changed", list, first);
                }
                self.core.packages_seen = key;
            }
        }
    }
    /// Everything `packages.list` (`package_list`) is built from, or
    /// `None` when that cannot be told: a development load's translations
    /// are read from its directory at each build, which may change on disk
    /// (built every time then, as before). Installed packages run from the
    /// library or the installation, whose files do not change while they
    /// are installed (a change is a new install: the store's revision);
    /// `process::data_root` and the interface language are fixed for the
    /// run.
    fn packages_key(&mut self, now: u64) -> Option<PackagesKey> {
        if !self.core.development.is_empty() {
            return None;
        }
        self.core.supervisor.set_now(now);
        let (store, operations) = self.core.packages.list_revision();
        Some(PackagesKey {
            revisions: [
                self.core.plugins_revision,
                self.core.warnings.revision(),
                self.core.failures.revision(),
                self.core.dev_disabled.revision(),
                self.core.development.revision(),
                self.core.on_demand.held_revision(),
                store,
                operations,
            ],
            units: self.core.supervisor.list_key(),
        })
    }
    /// `commands.keys` (`plugin.commands.keys`, player 0.7.6): the current
    /// key labels of each of `owner`'s own commands and of the player's own
    /// (`host/<id>`, SDK 1.6.16), as menus and the
    /// command palette show them (`bindings::label`: the user's keys, none
    /// when cleared, ⌘ on macOS). Noted as what it has, so `notify_tick`
    /// pushes only a later change.
    pub(crate) fn command_keys(&mut self, owner: u64) -> Value {
        let (manifest, keys) = self.keys_of(owner);
        if let Some(manifest) = manifest {
            self.core.keys_sent.insert(manifest, keys.clone());
        }
        self.keys_data(keys)
    }
    /// `owner`'s key labels now (`keys_of`): the state of its
    /// `commands.keys` (`Host::wake_missed`).
    pub(crate) fn keys_state(&self, owner: u64) -> Value {
        self.keys_of(owner).1
    }
    /// `owner` in standby is woken for its key labels `keys`
    /// (`Host::wake_missed`): noted as what it has, as `push_command_keys`
    /// notes a wake's; the event data.
    pub(crate) fn keys_woken(&mut self, owner: u64, keys: Value) -> Value {
        if let (Some(manifest), _) = self.keys_of(owner) {
            self.core.keys_sent.insert(manifest, keys.clone());
        }
        self.keys_data(keys)
    }
    /// The answer and event data: the labels by command id, and the table
    /// revision they are of (a larger one is newer: the SDK keeps the
    /// newest of an answer and an event that cross).
    fn keys_data(&self, keys: Value) -> Value {
        json!({"keys":keys,"revision":self.core.bindings().revision()})
    }
    /// `owner`'s manifest id (running or in standby) and its key labels
    /// (`labels_with_host`).
    fn keys_of(&self, owner: u64) -> (Option<String>, Value) {
        let (declared_by, labels) = labels_with_host(self.core.bindings(), owner);
        let manifest = self
            .core
            .plugins
            .get(&owner)
            .map(|plugin| plugin.manifest.id.clone())
            .or_else(|| self.held_id(owner).map(str::to_owned))
            .or_else(|| declared_by.map(str::to_owned));
        (manifest, json!(labels))
    }
    /// The binding table changed: each plugin that subscribed to
    /// `commands.keys` and whose labels are not what it last got receives
    /// its own (never another plugin's). One in standby that listens is
    /// woken for it (a key edit is a deliberate change, as for
    /// `bindings.changed`), unless it never got any in this run: it reads
    /// them when it starts. What is sent where is `plan_keys`'s.
    fn push_command_keys(&mut self) {
        let running = self
            .core
            .plugins
            .iter()
            .map(|(owner, plugin)| (*owner, plugin.subscriptions.contains_key(KEYS_EVENT)))
            .collect::<Vec<_>>();
        let running = running
            .into_iter()
            .filter_map(|(owner, subscribed)| {
                // Labels only for a subscriber; any running plugin keeps its
                // entry in `keys_sent`.
                let (manifest, keys) = if subscribed {
                    self.keys_of(owner)
                } else {
                    let manifest = self.core.plugins.get(&owner).map(|p| p.manifest.id.clone());
                    (manifest, Value::Null)
                };
                Some(KeysCandidate {
                    owner,
                    manifest: manifest?,
                    keys,
                    subscribed,
                    waking: self
                        .core
                        .supervisor
                        .waking(crate::supervisor::Pid(owner))
                        .is_some(),
                })
            })
            .collect::<Vec<_>>();
        let parked = self
            .core
            .on_demand
            .wakes
            .iter()
            .filter(|(_, events)| events.contains(KEYS_EVENT))
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        let parked = parked
            .into_iter()
            .filter_map(|owner| {
                let (manifest, keys) = self.keys_of(owner);
                Some(KeysCandidate {
                    owner,
                    manifest: manifest?,
                    keys,
                    subscribed: true,
                    waking: false,
                })
            })
            .collect::<Vec<_>>();
        for push in plan_keys(&running, &parked, &mut self.core.keys_sent) {
            match push {
                KeysPush::Send(owner, keys) => {
                    let data = self.keys_data(keys);
                    self.send(owner, json!({"event":KEYS_EVENT,"data":data}));
                }
                KeysPush::Queue(owner, keys) => {
                    // A plugin waking for a wake event gets the held one
                    // first; a newer one waits behind it (as `broadcast`).
                    let data = self.keys_data(keys);
                    let now = self.core.now_ms();
                    self.core.supervisor.set_now(now);
                    let trigger = crate::supervisor::Trigger::event(
                        KEYS_EVENT,
                        json!({"event":KEYS_EVENT,"data":data}),
                    );
                    self.core
                        .supervisor
                        .wake(crate::supervisor::Pid(owner), &trigger);
                }
                KeysPush::Wake(owner, keys) => {
                    let data = self.keys_data(keys);
                    self.wake_listener(owner, KEYS_EVENT, &data);
                }
            }
        }
    }
    /// Broadcast `event`; `first` (the first of a run) wakes no parked plugin.
    fn announce(&mut self, event: &str, data: Value, first: bool) {
        self.core.on_demand.mute = first;
        self.broadcast(event, data);
        self.core.on_demand.mute = false;
    }
    /// A running plugin subscribed to `event`, or a parked one is woken by it.
    fn listened(&self, event: &str) -> bool {
        self.core
            .plugins
            .values()
            .any(|plugin| plugin.subscriptions.contains_key(event))
            || self
                .core
                .on_demand
                .wakes
                .values()
                .any(|events| events.contains(event))
    }
}
impl Core {
    /// After the mounted panels changed: a plugin with a panel for the first
    /// time in this run has its panel commands listed from now on
    /// (`bindings.list`), so the list's revision moves.
    pub(crate) fn note_panel_plugins(&mut self) {
        let mut grew = false;
        for mount in self.mounts.values() {
            if !self.panel_plugins.contains(&mount.manifest) {
                self.panel_plugins.insert(mount.manifest.clone());
                grew = true;
            }
        }
        if grew {
            self.bindings_mut().listed_elsewhere();
        }
    }
}

pub(crate) const KEYS_EVENT: &str = "commands.keys";

/// A plugin `push_command_keys` considers: running (`subscribed`: it
/// subscribed to `commands.keys`; `waking`: its start for a wake event is
/// not ready yet) or in standby with a recorded listener.
#[derive(Debug)]
struct KeysCandidate {
    owner: u64,
    manifest: String,
    keys: Value,
    subscribed: bool,
    waking: bool,
}

/// What `push_command_keys` does for one plugin.
#[derive(Debug, PartialEq)]
enum KeysPush {
    /// Send the event now.
    Send(u64, Value),
    /// Queue it behind the wake event the plugin is starting for.
    Queue(u64, Value),
    /// Wake the plugin in standby to receive it.
    Wake(u64, Value),
}

/// Who gets `commands.keys` now, given what each plugin (by manifest id)
/// last got in `sent`, which this updates: a subscriber whose labels
/// changed (or never got any), queued while it wakes; a listener in
/// standby whose labels changed since it last got them, woken; a plugin in
/// standby never seen in this run is only noted (it reads its keys when it
/// starts, and waking every one at the first table change would be a
/// storm). Entries of plugins neither running nor listening in standby go
/// (uninstalled, disabled or parked without a listener: a new process asks
/// again at its start).
fn plan_keys(
    running: &[KeysCandidate],
    parked: &[KeysCandidate],
    sent: &mut std::collections::BTreeMap<String, Value>,
) -> Vec<KeysPush> {
    let mut pushes = Vec::new();
    for each in running.iter().filter(|each| each.subscribed) {
        if sent.get(&each.manifest) == Some(&each.keys) {
            continue;
        }
        sent.insert(each.manifest.clone(), each.keys.clone());
        let keys = each.keys.clone();
        pushes.push(if each.waking {
            KeysPush::Queue(each.owner, keys)
        } else {
            KeysPush::Send(each.owner, keys)
        });
    }
    for each in parked {
        match sent.get(&each.manifest) {
            None => {
                sent.insert(each.manifest.clone(), each.keys.clone());
            }
            Some(last) if *last == each.keys => {}
            Some(_) => {
                sent.insert(each.manifest.clone(), each.keys.clone());
                pushes.push(KeysPush::Wake(each.owner, each.keys.clone()));
            }
        }
    }
    let known = running
        .iter()
        .chain(parked)
        .map(|each| each.manifest.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    sent.retain(|manifest, _| known.contains(manifest.as_str()));
    pushes
}

/// The sources of `packages.list` (`Host::packages_key`): the revisions of
/// the Host's (running plugins, their warnings, failures, development
/// loads and those disabled, units held in standby) and the package
/// manager's (the library's catalog, operations), and what the unit table
/// answers now (`Supervisor::list_key`).
#[derive(Debug, PartialEq)]
pub(crate) struct PackagesKey {
    revisions: [u64; 8],
    units: crate::supervisor::ListKey,
}

/// `bindings.list` with its commands by plugin and command id: the
/// table's order follows owner generations, which a wake changes.
fn in_order(mut list: Value) -> Value {
    if let Some(commands) = list["commands"].as_array_mut() {
        commands.sort_by(|a, b| {
            (a["plugin"].as_str(), a["command"].as_str())
                .cmp(&(b["plugin"].as_str(), b["command"].as_str()))
        });
    }
    list
}

/// Two `packages.list`s are the same but for each plugin's `process`
/// (compared in place: no copy of either).
fn same_but_process(a: &Value, b: &Value) -> bool {
    let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
        return a == b;
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
                return a == b;
            };
            let fields = |plugin: &serde_json::Map<String, Value>| {
                plugin.keys().filter(|key| *key != "process").count()
            };
            fields(a) == fields(b)
                && a.iter()
                    .filter(|(key, _)| *key != "process")
                    .all(|(key, value)| b.get(key) == Some(value))
        })
}

/// The key labels `commands.keys` gives `owner`: its own commands', by
/// local id, and the player's own commands' as `host/<id>` (SDK 1.6.16: a
/// panel shows the keys of Volume + and Mute; a local id has no `/`, so
/// the two never meet). Never another plugin's. With the manifest id that
/// declared `owner`'s commands.
fn labels_with_host(
    table: &bindings::Table,
    owner: u64,
) -> (
    Option<&str>,
    std::collections::BTreeMap<String, Vec<String>>,
) {
    let (declared_by, mut labels) = table.labels_of(owner);
    let (_, host) = table.labels_of(bindings::HOST);
    labels.extend(
        host.into_iter()
            .map(|(id, keys)| (format!("host/{id}"), keys)),
    );
    (declared_by, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `plugin.commands.keys` reads the player's own commands' keys as
    /// `host/<id>` beside the plugin's own, never another plugin's.
    #[test]
    fn a_plugin_reads_its_own_keys_and_the_players() {
        let mut table = bindings::Table::default();
        table
            .declare(bindings::HOST, "mute", "host", &["M".into()], false, false)
            .unwrap();
        table
            .declare(bindings::HOST, "quit", "host", &[], false, false)
            .unwrap();
        table
            .declare(4, "eq", "p.audio", &["E".into()], false, false)
            .unwrap();
        table
            .declare(5, "other", "p.other", &["K".into()], false, false)
            .unwrap();
        let (manifest, labels) = labels_with_host(&table, 4);
        assert_eq!(manifest, Some("p.audio"));
        assert_eq!(
            json!(labels),
            json!({"eq": ["E"], "host/mute": ["M"], "host/quit": []})
        );
    }

    #[test]
    fn references_are_identifiers() {
        assert_eq!(
            reference(&json!({"plugin":"host","command":"play-pause"})).unwrap(),
            ("host".into(), "play-pause".into())
        );
        for bad in [
            json!({"plugin":"a/b","command":"x"}),
            json!({"plugin":"","command":"x"}),
            json!({"plugin":"a"}),
        ] {
            assert_eq!(reference(&bad).unwrap_err().code, "invalid");
        }
    }

    fn candidate(owner: u64, manifest: &str, key: &str) -> KeysCandidate {
        KeysCandidate {
            owner,
            manifest: manifest.into(),
            keys: json!({"up":[key]}),
            subscribed: true,
            waking: false,
        }
    }

    /// `commands.keys` goes only to subscribers, only when their labels
    /// changed, and each gets its own.
    #[test]
    fn keys_go_to_subscribers_only_and_only_when_changed() {
        let mut sent = std::collections::BTreeMap::new();
        let quiet = KeysCandidate {
            subscribed: false,
            ..candidate(3, "c.quiet", "Q")
        };
        let running = [
            candidate(1, "a.video", "E"),
            candidate(2, "b.audio", "U"),
            quiet,
        ];
        let pushes = plan_keys(&running, &[], &mut sent);
        assert_eq!(
            pushes,
            [
                KeysPush::Send(1, json!({"up":["E"]})),
                KeysPush::Send(2, json!({"up":["U"]}))
            ]
        );
        // The same labels again: nothing.
        assert!(plan_keys(&running, &[], &mut sent).is_empty());
        // One plugin's key changed: only it hears.
        let running = [
            candidate(1, "a.video", "Shift+E"),
            candidate(2, "b.audio", "U"),
        ];
        assert_eq!(
            plan_keys(&running, &[], &mut sent),
            [KeysPush::Send(1, json!({"up":["Shift+E"]}))]
        );
    }

    /// A plugin starting for a wake event gets the push queued behind it.
    #[test]
    fn a_waking_plugin_gets_the_keys_queued() {
        let mut sent = std::collections::BTreeMap::new();
        let waking = KeysCandidate {
            waking: true,
            ..candidate(4, "a.video", "E")
        };
        assert_eq!(
            plan_keys(&[waking], &[], &mut sent),
            [KeysPush::Queue(4, json!({"up":["E"]}))]
        );
    }

    /// A listener in standby is woken for a change of its keys; one never
    /// seen in this run is only noted (no wake storm at the first table
    /// change), and woken by a later change.
    #[test]
    fn parked_listeners_wake_for_a_change_but_not_when_first_seen() {
        let mut sent = std::collections::BTreeMap::new();
        let parked = [candidate(7, "a.video", "E")];
        assert!(plan_keys(&[], &parked, &mut sent).is_empty());
        assert_eq!(sent["a.video"], json!({"up":["E"]}));
        assert!(plan_keys(&[], &parked, &mut sent).is_empty());
        let parked = [candidate(7, "a.video", "R")];
        assert_eq!(
            plan_keys(&[], &parked, &mut sent),
            [KeysPush::Wake(7, json!({"up":["R"]}))]
        );
        // What it got while running counts: a change since wakes it.
        let mut sent = std::collections::BTreeMap::new();
        plan_keys(&[candidate(5, "b.audio", "U")], &[], &mut sent);
        assert_eq!(
            plan_keys(&[], &[candidate(8, "b.audio", "I")], &mut sent),
            [KeysPush::Wake(8, json!({"up":["I"]}))]
        );
    }

    /// A plugin neither running nor listening in standby (uninstalled,
    /// disabled) loses its entry.
    #[test]
    fn keys_of_gone_plugins_are_forgotten() {
        let mut sent = std::collections::BTreeMap::new();
        plan_keys(
            &[candidate(1, "a.video", "E"), candidate(2, "b.gone", "G")],
            &[],
            &mut sent,
        );
        assert_eq!(sent.len(), 2);
        plan_keys(&[candidate(1, "a.video", "E")], &[], &mut sent);
        assert_eq!(sent.keys().collect::<Vec<_>>(), ["a.video"]);
    }

    /// supervisor.md §10: `packages.changed` compares the list without
    /// `process`: a park (`running` to `idle`) or a wake alone announces
    /// nothing; any other field does.
    #[test]
    fn b0_packages_changed_ignores_only_process() {
        let running = json!([{"id":"a.b","state":"running","process":"running","error":null}]);
        let parked = json!([{"id":"a.b","state":"running","process":"idle","error":null}]);
        assert!(same_but_process(&running, &parked));
        // An entry without `process` (a quarantined record) against one
        // with it: the other fields decide.
        let bare = json!([{"id":"a.b","state":"running","error":null}]);
        assert!(same_but_process(&parked, &bare) && same_but_process(&bare, &parked));
        let disabled = json!([{"id":"a.b","state":"disabled","process":null,"error":null}]);
        assert!(!same_but_process(&running, &disabled));
        let starting = json!([{"id":"a.b","state":"starting","process":null,"error":null}]);
        assert!(!same_but_process(&running, &starting));
        // Another field, a field more, another plugin, another count.
        let error = json!([{"id":"a.b","state":"running","process":"running","error":"x"}]);
        assert!(!same_but_process(&running, &error));
        let more = json!([{"id":"a.b","state":"running","process":"running","error":null,"x":1}]);
        assert!(!same_but_process(&running, &more) && !same_but_process(&more, &running));
        let other = json!([{"id":"c.d","state":"running","process":"running","error":null}]);
        assert!(!same_but_process(&running, &other));
        let two = json!([running[0], other[0]]);
        assert!(!same_but_process(&running, &two));
        // The first list of a run (nothing sent yet) is a change.
        assert!(!same_but_process(&running, &Value::Null));
        assert!(same_but_process(&json!([]), &json!([])));
    }

    /// A panel-scoped command of a plugin without panels can never fire from
    /// a key: `bindings.list` leaves it out (and its stored binding is kept,
    /// not an orphan). One of a plugin with a panel and every global command
    /// stay.
    #[test]
    fn panel_commands_of_a_plugin_without_panels_are_not_listed() {
        let mut table = bindings::Table::default();
        let mut declarations = std::collections::BTreeMap::new();
        let mut declare = |owner, manifest: &str, id: &str, panel| {
            let mut declaration = crate::commands::Declaration::new(id, &[]);
            if panel {
                declaration.focus = Some(crate::commands::Focus::Panel);
            }
            table
                .declare(owner, id, manifest, &["B".into()], panel, false)
                .unwrap();
            declarations.insert((owner, id.to_owned()), declaration);
        };
        declare(bindings::HOST, "host", "quit", false);
        declare(4, "p.ask", "busy", true);
        declare(4, "p.ask", "ask", false);
        declare(5, "p.list", "remove", true);
        let overlay = [("p.ask/busy".to_owned(), vec!["Ctrl+K".to_owned()])];
        table.set_overlay(overlay.into_iter().collect());
        let mut asked = Vec::new();
        let listed = list(
            &table,
            |info| declarations.get(&(info.owner, info.id.clone())),
            |manifest| {
                asked.push(manifest.to_owned());
                manifest == "p.list"
            },
        );
        let rows = listed["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{}/{}",
                    row["plugin"].as_str().unwrap(),
                    row["command"].as_str().unwrap()
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            rows,
            ["host/quit", "p.ask/ask", "p.list/remove"]
                .map(String::from)
                .into()
        );
        // Asked only for panel-scoped commands.
        asked.sort();
        assert_eq!(asked, ["p.ask", "p.list"]);
        // The hidden command's binding is stored, not offered as an orphan.
        assert_eq!(listed["orphans"], json!([]));
        assert!(table.declares("p.ask", "busy"));
    }

    /// `bindings.list` names the player's own commands and the core actions
    /// by their categories, as the command palette does ("Seek: Back 5 s").
    #[test]
    fn bindings_list_gives_the_player_commands_their_categories() {
        let mut table = bindings::Table::default();
        let mut declarations = std::collections::BTreeMap::new();
        let own = Host::host_declarations()
            .into_iter()
            .map(|(id, declaration, _)| (id, declaration));
        for (id, declaration) in own.chain(crate::core_actions::declarations(5.)) {
            table
                .declare(
                    bindings::HOST,
                    id,
                    "host",
                    &declaration.bindings,
                    false,
                    false,
                )
                .unwrap();
            declarations.insert(id.to_owned(), declaration);
        }
        let rows: std::collections::BTreeMap<String, Value> = table
            .infos()
            .iter()
            .map(|info| (info.id.clone(), command_row(info, &declarations[&info.id])))
            .collect();
        assert_eq!(rows.len(), declarations.len());
        for (id, row) in &rows {
            assert!(row["category"].is_string(), "{id}: {row}");
        }
        let named = |id: &str| {
            let row = &rows[id];
            format!(
                "{}: {}",
                row["category"].as_str().unwrap(),
                row["title"].as_str().unwrap()
            )
        };
        assert_eq!(named("seek-backward"), "Seek: Back 5 s");
        assert_eq!(named("stop"), "Playback: Close");
        assert_eq!(named("mute"), "Audio: Mute");
        assert_eq!(named("exit-fullscreen"), "Window: Leave Fullscreen");
        assert_eq!(named("quit"), "File: Quit");
        // Titles and keys stay; seek, frame and speed left the menu (menu
        // redesign step 4), Previous, Next, the volume steps and Mute too
        // (the slim menu, B5), so they have no menu path; Play/Pause is a
        // top-level item, the command palette is in Tools ▸ (the short
        // menu, user ruling 2026-11-11).
        assert_eq!(rows["seek-backward"]["key"], Value::Null);
        assert_eq!(rows["mute"]["key"], Value::Null);
        assert_eq!(rows["previous"]["key"], Value::Null);
        assert_eq!(rows["stop"]["key"], json!(["Close"]));
        assert_eq!(rows["play-pause"]["key"], json!(["Play/Pause"]));
        assert_eq!(rows["palette"]["key"], json!(["Tools", "Command Palette…"]));
        assert_eq!(rows["palette"]["defaults"], json!(["Ctrl+Shift+P"]));
        // The column toggles left the menu (the short menu, user ruling
        // 2026-11-11), still without a default key.
        assert_eq!(named("sidebar"), "Window: Sidebar");
        assert_eq!(rows["sidebar"]["key"], Value::Null);
        assert_eq!(named("adjust"), "Window: Adjustments and Effects");
        assert_eq!(rows["adjust"]["key"], Value::Null);
        assert_eq!(rows["adjust"]["defaults"], json!([]));
    }
}
