//! `triggers` (SDK 1.10.0, events-and-triggers.md §4.10): a plugin hands
//! over, replaces and removes the conditions of its triggers
//! (`triggers.set`, `triggers.off`) and says which triggers its
//! `activate()` declared (`triggers.declare`). The conditions are kept by
//! the Host (`crate::triggers`), outside the standby recording: holding one
//! keeps no plugin running.
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply, RpcError};
use crate::triggers::{MAX_NAME, MAX_TRIGGERS, TriggerId};
use serde_json::{Value, json};

pub struct Triggers;
static INFO: Info = Info {
    name: "triggers",
    builtin_only: false,
};
static METHODS: &[&str] = &["triggers.set", "triggers.off", "triggers.declare"];

fn refused(refusal: crate::conditions::Refusal) -> RpcError {
    RpcError::new(refusal.code, refusal.message)
}

impl Capability for Triggers {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Nothing: the conditions are kept apart from the recording.
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        let plugin = host
            .core
            .triggers_plugin(reply.owner)
            .ok_or_else(|| RpcError::new("unavailable", "the plugin does not run"))?;
        match method {
            "triggers.set" => {
                let trigger = TriggerId::parse(&params["trigger"]).map_err(refused)?;
                // Facts are not followed while nothing is kept: now.
                host.triggers_facts_now();
                let (seq, out) = host
                    .core
                    .triggers
                    .set(
                        &plugin,
                        reply.owner,
                        trigger,
                        &params["when"],
                        crate::host::wall_ms(),
                    )
                    .map_err(refused)?;
                host.deliver_fired(out);
                Ok(Some(json!({ "seq": seq })))
            }
            "triggers.off" => {
                let trigger = TriggerId::parse(&params["trigger"]).map_err(refused)?;
                let seq = if params["unhandled"] == true {
                    host.core.triggers.unhandled(&plugin, &trigger)
                } else {
                    host.core.triggers.off(&plugin, &trigger)
                };
                host.core.sync_trigger_watch();
                Ok(Some(json!({ "seq": seq })))
            }
            _ => {
                let names = params["names"]
                    .as_array()
                    .filter(|names| names.len() <= MAX_TRIGGERS)
                    .and_then(|names| {
                        names
                            .iter()
                            .map(|name| {
                                name.as_str()
                                    .filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME)
                                    .map(str::to_owned)
                            })
                            .collect::<Option<Vec<_>>>()
                    })
                    .ok_or_else(|| {
                        RpcError::invalid(format!(
                            "names is a list of at most {MAX_TRIGGERS} trigger names"
                        ))
                    })?;
                let unnamed = params["unnamed"]
                    .as_u64()
                    .filter(|n| (*n as usize) <= MAX_TRIGGERS)
                    .ok_or_else(|| {
                        RpcError::invalid(format!("unnamed is a count 0–{MAX_TRIGGERS}"))
                    })?;
                host.core
                    .triggers
                    .declare(&plugin, reply.owner, &names, unnamed as u32);
                // What could not be delivered before fires again now that
                // its triggers are declared; the removed conditions'
                // boundaries go from the position model (`deliver_fired`).
                let out = host
                    .core
                    .triggers
                    .redeliver(&plugin, crate::host::wall_ms());
                host.deliver_fired(out);
                Ok(Some(Value::Null))
            }
        }
    }
}
