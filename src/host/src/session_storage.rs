//! `plugin.storage.session` (SDK 1.5.25, player 0.5.5): JSON values that
//! live as long as the player process, not the plugin process. User ruling
//! 2026-10-27, after Chrome's `chrome.storage.session`; design record
//! doc/process/rounds/2026-10-session-storage.md.
//!
//! The store is keyed by the plugin's [`DataKey`] (its installed instance or
//! development id: the same identity as its data directory and secrets), never
//! by the process generation, so a value survives every restart of the
//! plugin's process within one player run: a reload, an upgrade, a crash, a
//! restart of the shared runner, a later idle stop. It is cleared when the
//! user disables or uninstalls the plugin (`package_manager.rs`), and with the
//! player process. Requests: `storage.session.get` / `set` / `delete` /
//! `keys`; no capability, like `plugin.storage.json`.
#![forbid(unsafe_code)]
use super::*;
use std::collections::HashMap;

/// UTF-8 bytes one plugin may hold: each key's bytes plus its value's
/// compact JSON. The SDK repeats this and [`MAX_KEY_BYTES`]
/// (`SESSION_STORAGE_LIMITS`); src/tests/sdk/host-agreement.test.ts checks them.
pub const MAX_SESSION_BYTES: usize = 1024 * 1024;
/// UTF-8 bytes of one key (at least 1; no NUL).
pub const MAX_KEY_BYTES: usize = 1024;

/// One plugin's values and the bytes they count.
#[derive(Default)]
struct Entries {
    values: BTreeMap<String, (Value, usize)>,
    bytes: usize,
}

#[derive(Default)]
pub struct SessionStorage {
    plugins: HashMap<String, Entries>,
}

/// The store's own key for a plugin: one per data identity.
fn identity(key: &plugins::DataKey) -> String {
    match key {
        plugins::DataKey::Instance(guid) => format!("instance:{guid}"),
        plugins::DataKey::Development(id) => format!("development:{id}"),
    }
}

fn key_of(params: &Value) -> std::result::Result<&str, RpcError> {
    params["key"]
        .as_str()
        .filter(|key| (1..=MAX_KEY_BYTES).contains(&key.len()) && !key.contains('\0'))
        .ok_or_else(|| {
            RpcError::invalid("session storage keys are 1–1024 bytes of text without NUL")
        })
}

impl SessionStorage {
    /// Answers one `storage.session.*` request of the plugin `key`.
    pub fn dispatch(
        &mut self,
        key: &plugins::DataKey,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        let id = identity(key);
        match method {
            "storage.session.get" => {
                let name = key_of(params)?;
                Ok(self
                    .plugins
                    .get(&id)
                    .and_then(|entries| entries.values.get(name))
                    .map_or_else(|| json!({}), |(value, _)| json!({ "value": value })))
            }
            "storage.session.set" => {
                let name = key_of(params)?;
                let value = params
                    .get("value")
                    .ok_or_else(|| RpcError::invalid("set needs a JSON value"))?;
                let size = name.len() + value.to_string().len();
                let entries = self.plugins.entry(id).or_default();
                let replaced = entries.values.get(name).map_or(0, |(_, size)| *size);
                let total = entries.bytes - replaced + size;
                if total > MAX_SESSION_BYTES {
                    return Err(RpcError::new(
                        "limit",
                        format!(
                            "session storage holds at most {MAX_SESSION_BYTES} bytes per plugin \
                             (keys plus JSON values); this value would make {total}"
                        ),
                    ));
                }
                entries.bytes = total;
                entries
                    .values
                    .insert(name.to_owned(), (value.clone(), size));
                Ok(Value::Null)
            }
            "storage.session.delete" => {
                let name = key_of(params)?;
                if let Some(entries) = self.plugins.get_mut(&id) {
                    if let Some((_, size)) = entries.values.remove(name) {
                        entries.bytes -= size;
                    }
                    if entries.values.is_empty() {
                        self.plugins.remove(&id);
                    }
                }
                Ok(Value::Null)
            }
            "storage.session.keys" => Ok(self
                .plugins
                .get(&id)
                .map(|entries| entries.values.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
                .into()),
            _ => Err(RpcError::new("unsupported", "unknown method")),
        }
    }
    /// Forget everything the plugin `key` stored: the user disabled or
    /// uninstalled it. A restart of its process does not call this.
    pub fn clear(&mut self, key: &plugins::DataKey) {
        self.plugins.remove(&identity(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugins::DataKey;

    fn call(
        store: &mut SessionStorage,
        key: &DataKey,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, RpcError> {
        store.dispatch(key, &format!("storage.session.{method}"), &params)
    }

    #[test]
    fn values_round_trip_as_json_and_missing_keys_have_no_value() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        assert_eq!(
            call(&mut store, &a, "get", json!({"key":"x"})).unwrap(),
            json!({})
        );
        assert_eq!(call(&mut store, &a, "keys", json!({})).unwrap(), json!([]));
        for value in [
            json!(null),
            json!(0),
            json!("s"),
            json!({"n":[1,2,{"b":true}]}),
        ] {
            call(&mut store, &a, "set", json!({"key":"x","value":value})).unwrap();
            assert_eq!(
                call(&mut store, &a, "get", json!({"key":"x"})).unwrap(),
                json!({ "value": value })
            );
        }
        call(
            &mut store,
            &a,
            "set",
            json!({"key":"https://example.com/a?b","value":1}),
        )
        .unwrap();
        assert_eq!(
            call(&mut store, &a, "keys", json!({})).unwrap(),
            json!(["https://example.com/a?b", "x"])
        );
        call(&mut store, &a, "delete", json!({"key":"x"})).unwrap();
        call(&mut store, &a, "delete", json!({"key":"never"})).unwrap();
        assert_eq!(
            call(&mut store, &a, "keys", json!({})).unwrap(),
            json!(["https://example.com/a?b"])
        );
    }

    #[test]
    fn bad_keys_and_a_missing_value_are_invalid() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        let long = "k".repeat(MAX_KEY_BYTES + 1);
        for params in [
            json!({}),
            json!({"key":""}),
            json!({"key":7}),
            json!({"key":"a\0b"}),
            json!({ "key": long }),
        ] {
            for method in ["get", "delete"] {
                let error = call(&mut store, &a, method, params.clone()).unwrap_err();
                assert_eq!(error.code, "invalid", "{method} {params}");
            }
        }
        let error = call(&mut store, &a, "set", json!({"key":"x"})).unwrap_err();
        assert_eq!(error.code, "invalid");
        let at_limit = "k".repeat(MAX_KEY_BYTES);
        call(&mut store, &a, "set", json!({"key":at_limit,"value":1})).unwrap();
    }

    #[test]
    fn plugins_see_only_their_own_values_and_development_loads_are_apart() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        let b = DataKey::Instance("b".into());
        let dev_a = DataKey::Development("a".into());
        call(&mut store, &a, "set", json!({"key":"x","value":"a"})).unwrap();
        call(&mut store, &b, "set", json!({"key":"x","value":"b"})).unwrap();
        assert_eq!(
            call(&mut store, &a, "get", json!({"key":"x"})).unwrap(),
            json!({"value":"a"})
        );
        assert_eq!(
            call(&mut store, &dev_a, "keys", json!({})).unwrap(),
            json!([])
        );
        store.clear(&a);
        assert_eq!(
            call(&mut store, &a, "get", json!({"key":"x"})).unwrap(),
            json!({})
        );
        assert_eq!(
            call(&mut store, &b, "get", json!({"key":"x"})).unwrap(),
            json!({"value":"b"})
        );
    }

    #[test]
    fn the_quota_counts_keys_and_compact_json_and_a_refused_set_changes_nothing() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        // "k" + "\"…\"": 1 + 2 + n bytes.
        let fill = "x".repeat(MAX_SESSION_BYTES - 3);
        call(&mut store, &a, "set", json!({"key":"k","value":fill})).unwrap();
        let error = call(&mut store, &a, "set", json!({"key":"j","value":0})).unwrap_err();
        assert_eq!(error.code, "limit");
        assert_eq!(
            call(&mut store, &a, "keys", json!({})).unwrap(),
            json!(["k"])
        );
        // Replacing counts the new value instead of the old one.
        let over = "x".repeat(MAX_SESSION_BYTES - 2);
        let error = call(&mut store, &a, "set", json!({"key":"k","value":over})).unwrap_err();
        assert_eq!(error.code, "limit");
        assert_eq!(
            call(&mut store, &a, "get", json!({"key":"k"})).unwrap()["value"]
                .as_str()
                .unwrap()
                .len(),
            MAX_SESSION_BYTES - 3
        );
        call(&mut store, &a, "set", json!({"key":"k","value":1})).unwrap();
        call(&mut store, &a, "set", json!({"key":"j","value":0})).unwrap();
        // Another plugin has its own quota.
        let b = DataKey::Instance("b".into());
        call(&mut store, &b, "set", json!({"key":"k","value":fill})).unwrap();
        // Deleting gives the bytes back.
        call(&mut store, &b, "delete", json!({"key":"k"})).unwrap();
        call(&mut store, &b, "set", json!({"key":"k","value":fill})).unwrap();
    }
}
