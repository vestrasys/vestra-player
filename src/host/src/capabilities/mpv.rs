//! `mpv`: the mpv pass-through (SDK 1.8.0; doc/architecture/README.md
//! §4.9). A plugin sends mpv commands and reads, sets and observes its
//! properties; the channel is the player's contract, the names and what
//! they do are mpv's. Each request's shape and the closed refusal rules are
//! checked here (`playback/mpv_rules.rs`: what crosses another boundary,
//! or changes what the player manages itself, is `refused`); the control
//! worker does the rest (`playback/mpv_passthrough.rs`: filter labels and
//! their owners, observers). What a plugin holds there (a labelled filter,
//! an observer) keeps it running; the worker reports it
//! (`owner.resources`).
//!
//! Risky: what a plugin sets stays the player's setting after the plugin
//! stopped, until the player quits, and mpv's options reach far (decoding,
//! network, caches). The rules keep the channel off other boundaries
//! (programs, scripts, files, the clipboard, input) and off the player's
//! own state; they are no sandbox.
use super::{Capability, Handled, Info, Recorded};
use crate::{
    host::{Host, Reply, RpcError},
    playback::{backend::Command, mpv_rules},
};
use serde_json::Value;

pub struct Mpv;
static INFO: Info = Info {
    name: "mpv",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "mpv.command",
    "mpv.get",
    "mpv.set",
    "mpv.observe",
    "mpv.unobserve",
];

/// The refusal of a request, before anything reaches mpv: `invalid` for
/// its shape, then `refused` by the rules.
pub(crate) fn check(method: &str, params: &Value) -> Result<(), RpcError> {
    let refused = |message: String| RpcError::new("refused", message);
    if method == "mpv.command" {
        if let Some(problem) = mpv_rules::command_problem(&params["args"]) {
            return Err(RpcError::invalid(problem));
        }
        let args = params["args"].as_array().map_or(&[][..], Vec::as_slice);
        return mpv_rules::command_refusal(args).map_or(Ok(()), |message| Err(refused(message)));
    }
    if let Some(problem) = mpv_rules::name_problem(&params["name"]) {
        return Err(RpcError::invalid(problem));
    }
    let name = params["name"].as_str().unwrap_or_default();
    let write = method == "mpv.set";
    if let Some(message) = mpv_rules::property_refusal(name, write) {
        return Err(refused(message));
    }
    if write && params.get("value").is_none() {
        return Err(RpcError::invalid("set takes a value"));
    }
    Ok(())
}

impl Capability for Mpv {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// What it holds is in the playback worker (`owner.resources`).
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        check(method, params)?;
        // Only the fields the worker reads, and whose request it is: the
        // worker names a filter label's holder by its manifest id.
        let mut forwarded = serde_json::Map::new();
        for key in ["args", "name", "value"] {
            if let Some(value) = params.get(key) {
                forwarded.insert(key.into(), value.clone());
            }
        }
        forwarded.insert("plugin".into(), host.core.plugin_id(reply.owner).into());
        host.core
            .backend
            .commands
            .send(Command {
                reply: reply.clone(),
                method: method.into(),
                params: forwarded.into(),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        host.worker_requested(reply.owner);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refusal(method: &str, params: Value) -> (String, String) {
        let error = check(method, &params).unwrap_err();
        (error.code, error.message)
    }

    #[test]
    fn its_methods_are_open_to_every_plugin() {
        assert!(!INFO.builtin_only);
        // Every method of the group goes here, whatever the parameters.
        for method in METHODS {
            let route = super::super::lookup(method).unwrap();
            assert!(route.check_caller(false).is_ok(), "{method}");
        }
        // No prefix: a name beside them is no method.
        assert!(super::super::lookup("mpv.other").is_none());
    }

    #[test]
    fn a_request_is_checked_for_its_shape_then_for_the_rules() {
        assert!(check("mpv.command", &json!({"args":["frame-drop-count"]})).is_ok());
        assert!(
            check(
                "mpv.command",
                &json!({"args":["vf","add","@mine:lavfi=[hflip]"]})
            )
            .is_ok()
        );
        assert!(check("mpv.get", &json!({"name":"pause"})).is_ok());
        assert!(check("mpv.observe", &json!({"name":"time-pos"})).is_ok());
        assert!(check("mpv.unobserve", &json!({"name":"time-pos"})).is_ok());
        assert!(check("mpv.set", &json!({"name":"brightness","value":5})).is_ok());
        assert!(check("mpv.set", &json!({"name":"brightness","value":null})).is_ok());
        assert_eq!(
            refusal("mpv.command", json!({})),
            (
                "invalid".into(),
                "a command is an array of 1–32 elements, its name first".into()
            )
        );
        assert_eq!(
            refusal("mpv.command", json!({"args":["quit"]})),
            (
                "refused".into(),
                "mpv command \"quit\" is refused: it ends the player".into()
            )
        );
        // The shape first, also of a refused command.
        assert_eq!(
            refusal("mpv.command", json!({"args":["quit", null]})).0,
            "invalid"
        );
        for method in ["mpv.get", "mpv.set", "mpv.observe", "mpv.unobserve"] {
            assert_eq!(refusal(method, json!({"value":1})).0, "invalid", "{method}");
            assert_eq!(
                refusal(method, json!({"name":"a b","value":1})).0,
                "invalid"
            );
            // The clipboard may not even be read.
            assert_eq!(
                refusal(method, json!({"name":"clipboard/text","value":1})).0,
                "refused",
                "{method}"
            );
        }
        // A write of what the player manages; the missing value comes after.
        assert_eq!(
            refusal("mpv.set", json!({"name":"pause"})),
            (
                "refused".into(),
                "mpv property \"pause\" is refused: the player manages it and keeps its own state of it (use the plugin API's member for it)".into()
            )
        );
        assert_eq!(
            refusal("mpv.set", json!({"name":"brightness"})),
            ("invalid".into(), "set takes a value".into())
        );
    }
}
