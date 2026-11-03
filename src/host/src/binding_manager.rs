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
        let texts =
            |inputs: &[bindings::Input]| inputs.iter().map(bindings::text).collect::<Vec<_>>();
        let commands = table
            .infos()
            .into_iter()
            .filter_map(|info| {
                let declaration = &self.commands.get(info.owner, &info.id)?.declaration;
                Some(json!({
                    "plugin":info.manifest,"command":info.id,
                    // What the user sees: the interface language.
                    "title":declaration.shown_title(),
                    "path":(!declaration.path.is_empty()).then(|| declaration.shown_path()),
                    // The keys menus merge by: the same in every language.
                    "key":(!declaration.path.is_empty()).then(|| declaration.path.clone()),
                    "scope":if info.panel {"panel"} else {"global"},
                    "defaults":texts(&info.defaults),"current":texts(&info.current),
                    "overridden":info.overridden,
                    "shadowed":info.shadowed.iter().map(|(input, manifest, id)| {
                        json!({"input":bindings::text(input),"by":json_ref(manifest, id)})
                    }).collect::<Vec<_>>(),
                    "labels":info.current.iter().map(bindings::label).collect::<Vec<_>>(),
                }))
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
        let revision = self.input.borrow().table.revision();
        if revision != self.bindings_sent {
            self.bindings_sent = revision;
            // Only a change of the list: a table rebuilt to the same list
            // (a plugin parked or woken under a new owner, lazy activation
            // §6.9) is not one, so it wakes no parked listener.
            let list = if self.listened("bindings.changed") {
                in_order(self.bindings_list())
            } else {
                Value::Null
            };
            if list.is_null() {
                self.bindings_list_sent = list;
            } else if list != self.bindings_list_sent {
                // The first list of a run is no change for a parked plugin.
                let first = self.bindings_list_sent.is_null();
                self.bindings_list_sent = list;
                self.announce("bindings.changed", Value::Null, first);
            }
        }
        if self.listened("packages.changed") {
            let list = self.package_list();
            // A park or a wake alone changes only `process`: no event (§5.3).
            let compared = without_process(&list);
            if compared != self.packages_sent {
                let first = self.packages_sent.is_null();
                self.packages_sent = compared;
                self.announce("packages.changed", list, first);
            }
        }
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

/// `packages.list` without each plugin's `process`, to compare.
fn without_process(list: &Value) -> Value {
    let mut list = list.clone();
    if let Some(plugins) = list.as_array_mut() {
        for plugin in plugins.iter_mut().filter_map(Value::as_object_mut) {
            plugin.remove("process");
        }
    }
    list
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

    /// B0 (supervisor.md §9): `packages.changed` compares the list without
    /// `process`: a park (`running` to `idle`) or a wake alone announces
    /// nothing; any other field does.
    #[test]
    fn b0_packages_changed_ignores_only_process() {
        let running = json!([{"id":"a.b","state":"running","process":"running","error":null}]);
        let parked = json!([{"id":"a.b","state":"running","process":"idle","error":null}]);
        assert_eq!(without_process(&running), without_process(&parked));
        assert_eq!(
            without_process(&parked),
            json!([{"id":"a.b","state":"running","error":null}])
        );
        let disabled = json!([{"id":"a.b","state":"disabled","process":null,"error":null}]);
        assert_ne!(without_process(&running), without_process(&disabled));
        let starting = json!([{"id":"a.b","state":"starting","process":null,"error":null}]);
        assert_ne!(without_process(&running), without_process(&starting));
    }
}
