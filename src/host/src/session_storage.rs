//! `plugin.storage.session` (SDK 1.5.25, player 0.5.5): JSON values that
//! live as long as the player process, not the plugin process. User ruling
//! 2026-10-27 (doc/process/rounds/history.md §2); the code map row in
//! doc/architecture/README.md §5.
//!
//! The store is keyed by the plugin's [`DataKey`](crate::process::DataKey) (its installed instance or
//! development id: the same identity as its data directory and secrets), never
//! by the process generation, so a value survives every restart of the
//! plugin's process within one player run: a reload, an upgrade, a crash, a
//! failed start. It is cleared when the user disables or uninstalls the
//! plugin, when a retired instance can no longer be reconnected
//! (`package_manager.rs`), and with the player process. Requests:
//! `storage.session.get` / `set` / `delete` / `keys`; no capability, like
//! `plugin.storage.json`.
//!
//! Values are kept as their compact JSON text, so the quota is what they
//! take: each entry counts its key, its text and [`ENTRY_BYTES`] for the
//! map's own bookkeeping. Object properties come back in the order
//! `serde_json` keeps them (sorted), not the order the plugin wrote.
#![forbid(unsafe_code)]
use crate::host::*;
use crate::process;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::collections::HashMap;

/// Bytes one plugin may hold: each key's UTF-8 bytes, its value's compact
/// JSON and [`ENTRY_BYTES`]. The SDK repeats these and [`MAX_KEY_BYTES`]
/// (`SESSION_STORAGE_LIMITS`); src/tests/sdk/host-agreement.test.ts checks them.
pub const MAX_SESSION_BYTES: usize = 1024 * 1024;
/// UTF-8 bytes of one key (at least 1; no NUL).
pub const MAX_KEY_BYTES: usize = 1024;
/// What one entry costs beyond its key and value text: two `String`s in a
/// `BTreeMap` node and their allocations. Counting it keeps the memory of
/// many tiny entries inside the quota too.
pub const ENTRY_BYTES: usize = 128;

/// One plugin's values (compact JSON text) and the bytes they count.
#[derive(Default)]
struct Entries {
    values: BTreeMap<String, String>,
    bytes: usize,
}

fn cost(key: &str, text: &str) -> usize {
    key.len() + text.len() + ENTRY_BYTES
}

#[derive(Default)]
pub struct SessionStorage {
    plugins: HashMap<String, Entries>,
}

/// The store's own key for a plugin: one per data identity.
fn identity(key: &process::DataKey) -> String {
    match key {
        process::DataKey::Instance(guid) => format!("instance:{guid}"),
        process::DataKey::Development(id) => format!("development:{id}"),
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
        key: &process::DataKey,
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
                    .map_or_else(
                        || json!({}),
                        |text| {
                            // Written by `set` from a `Value`: always parses.
                            let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
                            json!({ "value": value })
                        },
                    ))
            }
            "storage.session.set" => {
                let name = key_of(params)?;
                let text = params
                    .get("value")
                    .ok_or_else(|| RpcError::invalid("set needs a JSON value"))?
                    .to_string();
                let (held, replaced) = self.plugins.get(&id).map_or((0, 0), |entries| {
                    let replaced = entries.values.get(name).map_or(0, |old| cost(name, old));
                    (entries.bytes, replaced)
                });
                let total = held - replaced + cost(name, &text);
                if total > MAX_SESSION_BYTES {
                    // Refused before touching the store: nothing changes.
                    return Err(RpcError::new(
                        "limit",
                        format!(
                            "session storage holds at most {MAX_SESSION_BYTES} bytes per plugin \
                             (keys, JSON values and {ENTRY_BYTES} per key); this value would make \
                             {total}"
                        ),
                    ));
                }
                let entries = self.plugins.entry(id).or_default();
                entries.bytes = total;
                entries.values.insert(name.to_owned(), text);
                Ok(Value::Null)
            }
            "storage.session.delete" => {
                let name = key_of(params)?;
                if let Some(entries) = self.plugins.get_mut(&id) {
                    if let Some(text) = entries.values.remove(name) {
                        entries.bytes -= cost(name, &text);
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
    pub fn clear(&mut self, key: &process::DataKey) {
        self.plugins.remove(&identity(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use process::DataKey;

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

    /// Values of a realistic size: one request is at most 1 MiB on the wire
    /// (`process::MAX_MESSAGE_BYTES`), so a single value stays below that.
    #[test]
    fn the_quota_counts_keys_compact_json_and_entries_and_a_refused_set_changes_nothing() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        // "kN" + "\"…\"" + ENTRY_BYTES each: four of them fill the quota.
        let quarter = MAX_SESSION_BYTES / 4 - 2 - 2 - ENTRY_BYTES;
        let fill = "x".repeat(quarter);
        for key in ["k0", "k1", "k2", "k3"] {
            call(&mut store, &a, "set", json!({"key":key,"value":fill})).unwrap();
        }
        assert_eq!(store.plugins["instance:a"].bytes, MAX_SESSION_BYTES);
        let error = call(&mut store, &a, "set", json!({"key":"j","value":0})).unwrap_err();
        assert_eq!(error.code, "limit");
        assert_eq!(
            call(&mut store, &a, "keys", json!({})).unwrap(),
            json!(["k0", "k1", "k2", "k3"])
        );
        // Replacing counts the new value instead of the old one.
        let over = format!("{fill}x");
        let error = call(&mut store, &a, "set", json!({"key":"k0","value":over})).unwrap_err();
        assert_eq!(error.code, "limit");
        assert_eq!(
            call(&mut store, &a, "get", json!({"key":"k0"})).unwrap()["value"]
                .as_str()
                .unwrap()
                .len(),
            quarter
        );
        call(&mut store, &a, "set", json!({"key":"k0","value":1})).unwrap();
        call(&mut store, &a, "set", json!({"key":"j","value":0})).unwrap();
        // Another plugin has its own quota; deleting gives the bytes back.
        let b = DataKey::Instance("b".into());
        for key in ["k0", "k1", "k2", "k3"] {
            call(&mut store, &b, "set", json!({"key":key,"value":fill})).unwrap();
        }
        call(&mut store, &b, "delete", json!({"key":"k0"})).unwrap();
        call(&mut store, &b, "set", json!({"key":"k0","value":fill})).unwrap();
    }

    #[test]
    fn many_tiny_entries_count_their_bookkeeping() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        let mut stored = 0;
        while call(
            &mut store,
            &a,
            "set",
            json!({"key":stored.to_string(),"value":0}),
        )
        .is_ok()
        {
            stored += 1;
        }
        assert!(stored <= MAX_SESSION_BYTES / ENTRY_BYTES, "{stored}");
        assert!(store.plugins["instance:a"].bytes <= MAX_SESSION_BYTES);
    }

    #[test]
    fn a_refused_first_set_leaves_no_entry() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        let huge = "x".repeat(MAX_SESSION_BYTES);
        let error = call(&mut store, &a, "set", json!({"key":"k","value":huge})).unwrap_err();
        assert_eq!(error.code, "limit");
        assert!(store.plugins.is_empty());
    }

    #[test]
    fn values_are_kept_as_compact_text_and_objects_come_back_sorted() {
        let mut store = SessionStorage::default();
        let a = DataKey::Instance("a".into());
        let value: Value =
            serde_json::from_str(r#"{"b": [0, 0], "a": {"d": 1, "c": null}}"#).unwrap();
        call(&mut store, &a, "set", json!({"key":"k","value":value})).unwrap();
        assert_eq!(
            store.plugins["instance:a"].values["k"],
            r#"{"a":{"c":null,"d":1},"b":[0,0]}"#
        );
        let back = call(&mut store, &a, "get", json!({"key":"k"})).unwrap();
        assert_eq!(
            back.to_string(),
            r#"{"value":{"a":{"c":null,"d":1},"b":[0,0]}}"#
        );
    }
}
