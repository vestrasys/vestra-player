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
                if !self.input.borrow().table.declares(&plugin, &command)
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
                        && !self.input.borrow().table.declares(&plugin, &command)
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
        self.input.borrow_mut().table.set_overlay(overlay);
        let now = self.now_ms();
        self.ui_state.touch(now);
        if self.menu_open {
            let result = self.refresh_menu();
            self.report_menu(result);
        }
    }
    fn bindings_list(&self) -> Value {
        let table = &self.input.borrow().table;
        let commands = table
            .infos()
            .into_iter()
            .filter_map(|info| {
                let declaration = &self.commands.get(info.owner, &info.id)?.declaration;
                Some(command_row(&info, declaration))
            })
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
    /// `bindings.changed` (declarations or overlay) and `packages.changed`
    /// (the plugin list) to the plugins that subscribed; at most every 200 ms.
    pub(super) fn notify_tick(&mut self) {
        let now = self.now_ms();
        if now < self.notified_at + 200 {
            return;
        }
        self.notified_at = now;
        let (revision, key) = {
            let table = &self.input.borrow().table;
            (
                table.revision(),
                [table.listed_revision(), self.commands.listed_revision()],
            )
        };
        if revision != self.bindings_sent {
            self.bindings_sent = revision;
            // Only a change of the list: a table rebuilt to the same list
            // (a plugin parked or woken under a new owner, lazy activation
            // §6.9) is not one, so it wakes no parked listener.
            if !self.listened("bindings.changed") {
                self.bindings_list_sent = Value::Null;
            } else {
                // Built and compared only when what it shows may have
                // changed (supervisor.md): during a wake storm every
                // woken plugin's commands move to its new owner, and the
                // list took 3–6 ms to build each time.
                let moved = key != self.bindings_seen || self.bindings_list_sent.is_null();
                if moved || cfg!(debug_assertions) {
                    let list = in_order(self.bindings_list());
                    let same = list == self.bindings_list_sent;
                    // Debug builds (every test with the real player) build
                    // it anyway: unchanged sources must give the same list.
                    assert!(
                        moved || same,
                        "bindings.list changed while its sources did not: {key:?}"
                    );
                    if !same {
                        // The first list of a run is no change for a parked plugin.
                        let first = self.bindings_list_sent.is_null();
                        self.bindings_list_sent = list;
                        self.announce("bindings.changed", Value::Null, first);
                    }
                    self.bindings_seen = key;
                }
            }
        }
        if self.listened("packages.changed") {
            // Built and compared only once a source moved (supervisor.md
            // §10): a list of about 200 packages took 4–12 ms to build, every
            // 200 ms, also when nothing happened.
            // One clock reading for both: a start deadline passing between
            // two would change the list but not the key.
            let now = self.now_ms();
            let key = self.packages_key(now);
            let moved = key.is_none() || key != self.packages_seen;
            if moved || cfg!(debug_assertions) {
                let list = self.package_list_at(now);
                // A park or a wake alone changes only `process`: no event (§5.3).
                let same = same_but_process(&list, &self.packages_sent);
                // Debug builds (every test with the real player) build it
                // anyway: unchanged sources must give the same list.
                assert!(
                    moved || same,
                    "packages.list changed while its sources did not: {key:?}"
                );
                if !same {
                    let first = self.packages_sent.is_null();
                    self.packages_sent = list.clone();
                    self.announce("packages.changed", list, first);
                }
                self.packages_seen = key;
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
        if !self.development.is_empty() {
            return None;
        }
        self.supervisor.set_now(now);
        let (store, operations) = self.packages.list_revision();
        Some(PackagesKey {
            revisions: [
                self.plugins_revision,
                self.warnings.revision(),
                self.failures.revision(),
                self.dev_disabled.revision(),
                self.development.revision(),
                self.on_demand.held_revision(),
                store,
                operations,
            ],
            units: self.supervisor.list_key(),
        })
    }
    /// Broadcast `event`; `first` (the first of a run) wakes no parked plugin.
    fn announce(&mut self, event: &str, data: Value, first: bool) {
        self.on_demand.mute = first;
        self.broadcast(event, data);
        self.on_demand.mute = false;
    }
    /// A running plugin subscribed to `event`, or a parked one is woken by it.
    fn listened(&self, event: &str) -> bool {
        self.plugins
            .values()
            .any(|plugin| plugin.subscriptions.contains_key(event))
            || self
                .on_demand
                .wakes
                .values()
                .any(|events| events.contains(event))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // Titles, keys and menu paths stay as they were.
        assert_eq!(
            rows["seek-backward"]["key"],
            json!(["Playback", "Seek", "Back 5 s"])
        );
        assert_eq!(rows["palette"]["key"], Value::Null);
        assert_eq!(rows["palette"]["defaults"], json!(["Ctrl+Shift+P"]));
    }
}
