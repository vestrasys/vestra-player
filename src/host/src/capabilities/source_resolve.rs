//! `source.resolve`: resolvers for online addresses and file types
//! (source_resolvers.rs, in the playback worker).
use super::{Capability, Handled, Info, Method, Residency, forward};
use crate::{Host, Reply, RpcError};
use serde_json::Value;

pub struct SourceResolve;
static INFO: Info = Info {
    name: "source.resolve",
    label: "Resolve online video addresses",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
};
static METHODS: &[Method] = &[
    Method::this("sourceResolvers.register"),
    Method::this("sourceResolvers.unregister"),
    Method::this("sourceResolvers.respond"),
    Method::this("sourceResolvers.*"),
];
impl Capability for SourceResolve {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "sourceResolvers.register"
            | "sourceResolvers.unregister"
            | "sourceResolvers.respond" => forward(host, reply, method, params),
            _ => Err(RpcError::new("unsupported", "unknown method")),
        }
    }
}
