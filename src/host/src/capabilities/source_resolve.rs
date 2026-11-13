//! `source.resolve`: resolvers for online addresses and file types
//! (source_resolvers.rs, in the playback worker).
use super::{Call, Group, Handled, Info, Method, Recorded, Residency, forward};
use crate::host::Origin;
use crate::host::{Host, Reply, RpcError};
use crate::playback::backend::Command;
use serde_json::Value;

pub struct SourceResolve;
static INFO: Info = Info {
    name: "source.resolve",
    summary: "Resolve online video addresses",
    residency: Residency::Recordable,
};
static METHODS: &[Method] = &[
    Method::free("sourceResolvers.register"),
    Method::free("sourceResolvers.unregister"),
    Method::free("sourceResolvers.respond"),
    Method::free("sourceResolvers.*"),
];
impl Group for SourceResolve {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// Its resolvers, as the playback worker answered `owner.resources`.
    fn record(&self, _: &Host, _: u64, backend: &Value) -> Recorded {
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
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        let _ = host.backend.commands.send(Command {
            reply: Reply { owner: from, id: 0 },
            method: "owner.rekey".into(),
            params: serde_json::json!({ "to": to }),
        });
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        let _ = host.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.prune".into(),
            params: Value::Null,
        });
    }
    fn replay(&self, host: &mut Host, owner: u64, _: &Origin, call: &Call) -> Handled {
        if call.method != "sourceResolvers.register" {
            return Err(RpcError::new("unsupported", "not a recorded method"));
        }
        host.backend
            .commands
            .send(Command {
                reply: Reply { owner, id: 0 },
                method: "owner.replay".into(),
                params: call.params.clone(),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        Ok(None)
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "sourceResolvers.respond" => {
                // The request it answers is no longer in flight.
                host.resolution_answered(reply.owner, params["requestId"].as_u64());
                forward(host, reply, method, params)
            }
            "sourceResolvers.register" | "sourceResolvers.unregister" => {
                forward(host, reply, method, params)
            }
            _ => Err(RpcError::new("unsupported", "unknown method")),
        }
    }
}
