//! `source.resolve`: resolvers for online addresses and file types
//! (source_resolvers.rs, in the playback worker).
use super::{Call, Capability, Handled, Info, Recorded, forward};
use crate::host::Origin;
use crate::host::{Core, Reply, RpcError};
use crate::playback::backend::Command;
use serde_json::Value;

pub struct SourceResolve;
static INFO: Info = Info {
    name: "source.resolve",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "sourceResolvers.register",
    "sourceResolvers.unregister",
    "sourceResolvers.respond",
    "sourceResolvers.*",
];
impl Capability for SourceResolve {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Its resolvers, as the playback worker answered `owner.resources`.
    fn record(&self, _: &Core, _: u64, backend: &Value) -> Recorded {
        Recorded::Calls(
            backend["resolvers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|params| Call::new("sourceResolvers.register", params.clone()))
                .collect(),
        )
    }
    /// The worker moves its resolvers and their waiting resolutions.
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        let _ = core.backend.commands.send(Command {
            reply: Reply { owner: from, id: 0 },
            method: "owner.rekey".into(),
            params: serde_json::json!({ "to": to }),
        });
    }
    fn prune(&self, core: &mut Core, owner: u64) {
        let _ = core.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.prune".into(),
            params: Value::Null,
        });
    }
    fn replay(&self, core: &mut Core, owner: u64, _: &Origin, call: &Call) -> Handled {
        if call.method != "sourceResolvers.register" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        core.backend
            .commands
            .send(Command {
                reply: Reply { owner, id: 0 },
                method: "owner.replay".into(),
                params: call.params.clone(),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        Ok(None)
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "sourceResolvers.respond" => {
                // The request it answers is no longer in flight.
                core.resolution_answered(reply.owner, params["requestId"].as_u64());
                forward(core, reply, method, params)
            }
            "sourceResolvers.register" | "sourceResolvers.unregister" => {
                forward(core, reply, method, params)
            }
            _ => Err(RpcError::new("unsupported", "unknown method")),
        }
    }
}
