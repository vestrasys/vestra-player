//! The manifest capabilities, one module each.
//!
//! A capability module owns its manifest name, its wording for the store and
//! the SDK (`label`, `risky`), the capabilities it implies (`requires`), a
//! residency hint, the RPC methods it serves and their handling. Nothing
//! else lists capabilities: the manifest check (`known`), the per-request
//! permission check (`required`) and the dispatch (`lookup`) are built from
//! [`ALL`], and the committed `src/host/capabilities.json` is generated from
//! it (`tests::json_is_current`), for the SDK's `CAPABILITY_INFO`, the
//! store's labels and src/tests/sdk/host-agreement.test.ts.
//!
//! Adding a capability: a new module implementing [`Capability`], one line in
//! [`ALL`] (the order is the manifest order the SDK repeats), then
//! `VESTRA_UPDATE_CAPABILITIES=1 cargo test capabilities` to rewrite the JSON.
use crate::{
    host::{Host, Origin, Reply, RpcError},
    playback::backend::Command,
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, sync::OnceLock};

mod bindings_manage;
mod clipboard;
mod commands;
mod commands_run;
mod events;
mod media_control;
mod media_read;
mod media_sources;
pub(crate) mod palette_lead;
mod playback_control;
mod playback_read;
mod plugins_manage;
mod processing_audio;
mod processing_video;
mod producer_av;
mod rendering_manage;
mod secrets;
mod source_resolve;
mod subtitles_control;
mod subtitles_read;
mod timeline_marks;
mod ui_dialog;
mod ui_drop;
pub(crate) mod ui_mount;
mod ui_osd;
mod ui_settings;
mod ui_theme;
mod ui_window;
mod uncapable;
mod video_upscaling;
mod window_control;

pub(crate) use commands::refresh_conditions;
pub(crate) use commands_run::{CommandRuns, answered as command_run_answered};
pub(crate) use events::WAKE_EVENTS;
pub(crate) use media_sources::locate_dialog;
pub(crate) use palette_lead::{EVENT as LEAD_EVENT, Lead, LeadStatus, StatusState};

/// What a capability is, as the manifest, the store and the SDK name it.
// Read by the JSON generator (`json`, a test) and the lazy activation to come.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Info {
    /// Exactly the manifest string: "commands", "ui.mount", ...
    pub name: &'static str,
    /// The store's and the SDK's wording (`CAPABILITY_INFO.label`, store `labels.ts`).
    pub label: &'static str,
    /// Drawn amber in the store.
    pub risky: bool,
    /// Capabilities its SDK members also need (`CAPABILITY_INFO.requires`).
    pub requires: &'static [&'static str],
    /// Whether what a plugin holds through it keeps the plugin running
    /// (doc/architecture/supervisor.md §2, "something live").
    /// A hint for the JSON; what decides is the capability's `record`.
    pub residency: Residency,
    /// Only built-in plugins may use it (`CAPABILITY_INFO.builtinOnly`):
    /// `Host::dispatch` and `replay` refuse it to any other plugin with
    /// `capability`, built-in by provenance (`Host::is_builtin`), never by
    /// a manifest field or a load mode; `packages.validate` refuses a
    /// package that declares it.
    pub builtin_only: bool,
}

/// The residency hint of a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Residency {
    /// Its requests leave nothing behind for the plugin.
    None,
    /// It registers entries the player can show and trigger without the
    /// plugin's process (menu commands, settings pages, resolvers, drop
    /// targets): recorded and replayed.
    Recordable,
    /// It holds something only the plugin's process can serve (a panel,
    /// a window, a listener, a filter chain, a stream, a lease).
    Live,
}
impl Residency {
    #[cfg_attr(not(test), allow(dead_code))]
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Recordable => "recordable",
            Self::Live => "live",
        }
    }
}

/// The capability a method asks for.
pub enum Needs {
    /// The capability that owns the method.
    This,
    /// None: any plugin may call it.
    Nothing,
    /// Chosen by a string parameter: the capability of the first matching
    /// case, `otherwise` when none matches (`None`: no capability).
    Param {
        key: &'static str,
        cases: &'static [(&'static str, Option<&'static str>)],
        otherwise: Option<&'static str>,
    },
    /// `then` when the plugin declared `declared`, `otherwise` if not.
    IfDeclared {
        declared: &'static str,
        then: &'static str,
        otherwise: &'static str,
    },
}

/// An RPC method a capability serves. A name ending in `.*` covers every
/// method with that prefix that no exact name covers (the prefix's handler
/// refuses the ones it does not know).
pub struct Method {
    pub name: &'static str,
    pub needs: Needs,
}
impl Method {
    pub const fn this(name: &'static str) -> Self {
        Self {
            name,
            needs: Needs::This,
        }
    }
    pub const fn free(name: &'static str) -> Self {
        Self {
            name,
            needs: Needs::Nothing,
        }
    }
}

pub type Handled = std::result::Result<Option<Value>, RpcError>;

/// A request that recreates a registration without the plugin's process,
/// in the form the plugin sent it (lazy activation, `host/supervisor/checkpoint.rs`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Call {
    pub method: String,
    pub params: Value,
}
impl Call {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

/// What an owner holds through a capability, for standby
/// (doc/architecture/supervisor.md §2, "something live").
#[derive(Debug, PartialEq)]
pub enum Recorded {
    /// Nothing the host keeps for it here.
    Nothing,
    /// Registrations the host shows and triggers without the process.
    Calls(Vec<Call>),
    /// Something only its process can serve (why): it stays running.
    Live(&'static str),
}

/// A manifest capability: its metadata, its methods and their handling.
pub trait Capability: Sync {
    fn info(&self) -> &'static Info;
    /// The RPC methods it owns; a method has exactly one owner (`tests`).
    fn methods(&self) -> &'static [Method];
    /// Serves one of `methods()`, after the permission check and the text
    /// resolution of `Host::dispatch`. `Ok(None)`: answered later.
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled;
    /// What `owner` holds through this capability: calls that recreate its
    /// registrations, or `Live`. `backend` is the playback worker's answer
    /// to `owner.resources` (`{ live, resolvers }`); what the worker holds
    /// (chains, streams, tracks, jobs, upscaling) is judged from its `live`
    /// list as a whole (`Host::lazy_answer`), so capabilities served there
    /// record `Nothing` here.
    fn record(&self, host: &Host, owner: u64, backend: &Value) -> Recorded;
    /// A parked owner woke as generation `to`: its entries move. Only
    /// recordable capabilities hold anything for a parked owner; the others
    /// do nothing (an owner holding them is never parked).
    fn rekey(&self, host: &mut Host, from: u64, to: u64);
    /// The registry half of stopping `owner` (`Host::stop`): remove its
    /// entries, or keep them marked recorded when it is parked. What the
    /// other capabilities hold is still revoked by `Host::stop` itself.
    fn revoke(&self, _host: &mut Host, _owner: u64, _keep_recorded: bool) {}
    /// At `runtime.ready` of a woken owner: its recorded entries that it
    /// did not register again go.
    fn prune(&self, _host: &mut Host, _owner: u64) {}
    /// Replays one recorded call for a parked `owner` of the package
    /// `origin`, at start-up (no process, no reply).
    fn replay(&self, _host: &mut Host, _owner: u64, _origin: &Origin, _call: &Call) -> Handled {
        Err(RpcError::new("unsupported", "not a recorded method"))
    }
}

/// Every capability's registry half of stopping `owner` (`Capability::revoke`).
pub fn revoke(host: &mut Host, owner: u64, keep_recorded: bool) {
    for capability in ALL {
        capability.revoke(host, owner, keep_recorded);
    }
}

/// Every capability's entries of `from` move to `to` (`Capability::rekey`).
pub fn rekey(host: &mut Host, from: u64, to: u64) {
    for capability in ALL {
        capability.rekey(host, from, to);
    }
}

/// Every capability's recorded entries of `owner` not registered again go.
pub fn prune(host: &mut Host, owner: u64) {
    for capability in ALL {
        capability.prune(host, owner);
    }
}

/// What `owner` holds: the calls of every capability, or the first `Live`.
pub fn record(host: &Host, owner: u64, backend: &Value) -> Result<Vec<Call>, &'static str> {
    let mut calls = Vec::new();
    for capability in ALL {
        match capability.record(host, owner, backend) {
            Recorded::Nothing => (),
            Recorded::Calls(more) => calls.extend(more),
            Recorded::Live(reason) => return Err(reason),
        }
    }
    Ok(calls)
}

/// What `owner` registered that a recording can hold, whatever else it
/// holds live: every capability's calls, a `Live` one's left out (the
/// events capability's listeners of wake events kept). Only for
/// `VESTRA_TEST_PARK=all` (host/supervisor/probe.rs), which parks every plugin to
/// diff its registries; a real park needs [`record`].
pub fn recordable(host: &Host, owner: u64, backend: &Value) -> Vec<Call> {
    let mut calls = Vec::new();
    for capability in ALL {
        match capability.record(host, owner, backend) {
            Recorded::Calls(more) => calls.extend(more),
            Recorded::Live(_) if capability.info().name == "events" => {
                calls.extend(events::wake_calls(host, owner));
            }
            Recorded::Nothing | Recorded::Live(_) => (),
        }
    }
    calls
}

/// Replays a recorded call: its capability, which the package must declare.
pub fn replay(
    host: &mut Host,
    owner: u64,
    origin: &Origin,
    declared: &[String],
    call: &Call,
) -> Handled {
    let route = lookup(&call.method);
    let capability = route
        .and_then(|route| route.owner)
        .ok_or_else(|| RpcError::new("unsupported", "unknown method"))?;
    // What the request asks for, as for a live one (`packages.changed`
    // needs no capability, `bindings.changed` needs `bindings.manage`).
    let allows = |name: &str| declared.iter().any(|c| c == name);
    let needs = route.and_then(|route| route.needs(&call.params, allows));
    if needs.is_some_and(|needed| !allows(needed)) {
        return Err(RpcError::new(
            "capability",
            "capability must be declared in manifest",
        ));
    }
    // A recording of a package that is not built-in (the same id loaded for
    // development or from another directory) replays nothing built-in-only.
    let builtin = host.is_builtin_package(origin);
    builtin_only_check(needs, builtin)?;
    capability.replay(host, owner, origin, call)
}

/// The refusal of a built-in-only capability (`Info::builtin_only`) to a
/// plugin that is not built-in: `needs` is the capability a request needs,
/// `builtin` whether its plugin is built-in by provenance.
pub fn builtin_only_check(needs: Option<&str>, builtin: bool) -> std::result::Result<(), RpcError> {
    if !builtin && needs.is_some_and(builtin_only) {
        return Err(RpcError::new(
            "capability",
            "this capability is for built-in plugins only",
        ));
    }
    Ok(())
}

/// Whether the capability `name` is for built-in plugins only.
pub fn builtin_only(name: &str) -> bool {
    ALL.iter()
        .any(|c| c.info().name == name && c.info().builtin_only)
}

/// The first built-in-only capability a manifest declares: what a package
/// that is not built-in may not declare (`packages.validate`).
pub fn first_builtin_only(declared: &[String]) -> Option<&str> {
    declared
        .iter()
        .map(String::as_str)
        .find(|name| builtin_only(name))
}

/// Every manifest capability, in the manifest order the SDK's
/// `CAPABILITIES` repeats.
pub static ALL: &[&dyn Capability] = &[
    &producer_av::ProducerAv,
    &playback_read::PlaybackRead,
    &playback_control::PlaybackControl,
    &source_resolve::SourceResolve,
    &events::Events,
    &commands::Commands,
    &ui_window::UiWindow,
    &ui_mount::UiMount,
    &ui_dialog::UiDialog,
    &plugins_manage::PluginsManage,
    &processing_video::ProcessingVideo,
    &processing_audio::ProcessingAudio,
    &subtitles_read::SubtitlesRead,
    &subtitles_control::SubtitlesControl,
    &media_read::MediaRead,
    &media_control::MediaControl,
    &media_sources::MediaSources,
    &ui_osd::UiOsd,
    &clipboard::Clipboard,
    &window_control::WindowControl,
    &bindings_manage::BindingsManage,
    &timeline_marks::TimelineMarks,
    &ui_drop::UiDrop,
    &ui_settings::UiSettings,
    &ui_theme::UiTheme,
    &secrets::Secrets,
    &rendering_manage::RenderingManage,
    &video_upscaling::VideoUpscaling,
    // Built-in only (Ask Vestra, doc/architecture/palette-lead.md).
    &commands_run::CommandsRun,
    &palette_lead::PaletteLead,
];

/// Whether `name` is a manifest capability.
pub fn known(name: &str) -> bool {
    ALL.iter().any(|c| c.info().name == name)
}

/// How many manifest capabilities there are.
pub fn count() -> usize {
    ALL.len()
}

/// Where a method goes: its owner (`None`: `uncapable`) and its entry.
#[derive(Clone, Copy)]
pub struct Route {
    owner: Option<&'static dyn Capability>,
    method: &'static Method,
}
impl Route {
    /// The capability the request needs (`None`: none); `allows` tells
    /// which capabilities the plugin declared.
    pub fn needs(&self, params: &Value, allows: impl Fn(&str) -> bool) -> Option<&'static str> {
        match &self.method.needs {
            Needs::This => self.owner.map(|c| c.info().name),
            Needs::Nothing => None,
            Needs::Param {
                key,
                cases,
                otherwise,
            } => {
                let value = params[*key].as_str();
                cases
                    .iter()
                    .find(|(case, _)| value == Some(*case))
                    .map_or(*otherwise, |(_, needs)| *needs)
            }
            Needs::IfDeclared {
                declared,
                then,
                otherwise,
            } => Some(if allows(declared) { then } else { otherwise }),
        }
    }
    pub fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match self.owner {
            Some(capability) => capability.handle(host, reply, method, params),
            None => uncapable::handle(host, reply, method, params),
        }
    }
}

/// Exact method names, and `prefix.*` entries as (prefix with its dot, route).
type Routes = (HashMap<&'static str, Route>, Vec<(&'static str, Route)>);

fn routes() -> &'static Routes {
    static ROUTES: OnceLock<Routes> = OnceLock::new();
    ROUTES.get_or_init(|| {
        let owned = ALL
            .iter()
            .flat_map(|c| c.methods().iter().map(move |m| (Some(*c), m)))
            .chain(uncapable::METHODS.iter().map(|m| (None, m)));
        let mut exact = HashMap::new();
        let mut prefixes = Vec::new();
        for (owner, method) in owned {
            let route = Route { owner, method };
            match method.name.strip_suffix('*') {
                Some(prefix) => prefixes.push((prefix, route)),
                None => {
                    exact.insert(method.name, route);
                }
            }
        }
        (exact, prefixes)
    })
}

/// The route of a method: its exact name first, then its prefix.
pub fn lookup(method: &str) -> Option<Route> {
    let (exact, prefixes) = routes();
    exact.get(method).copied().or_else(|| {
        prefixes
            .iter()
            .find(|(prefix, _)| method.starts_with(prefix))
            .map(|(_, route)| *route)
    })
}

/// The manifest capability a request needs (`None`: none); `allows` tells
/// which capabilities the plugin declared. Unknown methods fail
/// `unsupported`. (`Host::dispatch` uses the route itself.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn required(
    method: &str,
    params: &Value,
    allows: impl Fn(&str) -> bool,
) -> std::result::Result<Option<&'static str>, RpcError> {
    lookup(method)
        .map(|route| route.needs(params, allows))
        .ok_or_else(|| RpcError::new("unsupported", "unknown method"))
}

/// Hands a request to the playback worker; a stopped worker is `backend`.
/// What the plugin holds there may change with it: a run that may still go
/// to standby has the worker asked again (`Host::worker_requested`).
fn forward(host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
    host.backend
        .commands
        .send(Command {
            reply: reply.clone(),
            method: method.into(),
            params: params.clone(),
        })
        .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
    host.worker_requested(reply.owner);
    Ok(None)
}

/// Hands a request to the playback worker; a stopped worker is `host`
/// (subtitles, snapshots, jobs, video view and render). As `forward`, what
/// the plugin holds there is asked again.
fn forward_media(host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
    host.backend
        .commands
        .send(Command {
            reply: reply.clone(),
            method: method.into(),
            params: params.clone(),
        })
        .map_err(anyhow::Error::from)?;
    host.worker_requested(reply.owner);
    Ok(None)
}

/// The running plugin a request came from (`Host::dispatch` checked it).
fn plugin<'a>(host: &'a Host, reply: &Reply) -> std::result::Result<&'a crate::Plugin, RpcError> {
    host.plugins
        .get(&reply.owner)
        .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn needs_json(needs: &Needs, owner: Option<&Info>) -> Value {
    let name = |n: Option<&str>| n.map_or(Value::Null, |n| n.into());
    match needs {
        Needs::This => name(owner.map(|o| o.name)),
        Needs::Nothing => Value::Null,
        Needs::Param {
            key,
            cases,
            otherwise,
        } => json!({
            "param": key,
            "cases": cases.iter().map(|(case, needs)| ((*case).to_owned(), name(*needs))).collect::<Map<_, _>>(),
            "otherwise": name(*otherwise),
        }),
        Needs::IfDeclared {
            declared,
            then,
            otherwise,
        } => json!({"ifDeclared": declared, "then": then, "otherwise": otherwise}),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn methods_json(methods: &[Method], owner: Option<&Info>) -> Value {
    methods
        .iter()
        .map(|m| json!({"name": m.name, "needs": needs_json(&m.needs, owner)}))
        .collect()
}

/// `src/host/capabilities.json`: [`ALL`] as data.
#[cfg_attr(not(test), allow(dead_code))]
pub fn json() -> Value {
    json!({
        "about": "Generated from src/host/src/capabilities/ (VESTRA_UPDATE_CAPABILITIES=1 cargo test capabilities); do not edit. \
            A method's needs: null (none), a capability name, \
            {param, cases, otherwise} (by a string parameter) or {ifDeclared, then, otherwise}. \
            A method name ending in .* covers the prefix's other methods.",
        "apiVersion": crate::process::API,
        "capabilities": ALL.iter().map(|c| {
            let info = c.info();
            json!({
                "name": info.name,
                "label": info.label,
                "risky": info.risky,
                "requires": info.requires,
                "residency": info.residency.name(),
                "builtinOnly": info.builtin_only,
                "methods": methods_json(c.methods(), Some(info)),
            })
        }).collect::<Vec<_>>(),
        "uncapable": methods_json(uncapable::METHODS, None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn json_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/capabilities.json");
        let generated = serde_json::to_string_pretty(&json()).unwrap() + "\n";
        if std::env::var("VESTRA_UPDATE_CAPABILITIES").as_deref() == Ok("1") {
            std::fs::write(path, &generated).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if committed != generated {
            let first = committed
                .lines()
                .zip(generated.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(committed.lines().count().min(generated.lines().count()));
            panic!(
                "src/host/capabilities.json is not what src/host/src/capabilities/ generates \
                 (first difference at line {}: committed {:?}, generated {:?}); \
                 rerun with VESTRA_UPDATE_CAPABILITIES=1 and commit the file",
                first + 1,
                committed.lines().nth(first),
                generated.lines().nth(first),
            );
        }
    }

    #[test]
    fn every_method_has_one_owner_and_names_are_unique() {
        let mut seen = BTreeSet::new();
        let all = ALL
            .iter()
            .flat_map(|c| c.methods())
            .chain(uncapable::METHODS);
        for method in all {
            assert!(seen.insert(method.name), "{} has two owners", method.name);
        }
        let mut names = BTreeSet::new();
        for capability in ALL {
            let info = capability.info();
            assert!(names.insert(info.name), "{} listed twice", info.name);
        }
        for capability in ALL {
            for required in capability.info().requires {
                assert!(
                    known(required),
                    "{} requires unknown {required}",
                    capability.info().name
                );
            }
        }
    }

    #[test]
    fn every_capability_a_method_names_exists_and_uncapable_needs_nothing() {
        let named = |needs: &Needs| -> Vec<&'static str> {
            match needs {
                Needs::This | Needs::Nothing => Vec::new(),
                Needs::Param {
                    cases, otherwise, ..
                } => cases
                    .iter()
                    .filter_map(|(_, n)| *n)
                    .chain(*otherwise)
                    .collect(),
                Needs::IfDeclared {
                    declared,
                    then,
                    otherwise,
                } => vec![declared, then, otherwise],
            }
        };
        for method in ALL.iter().flat_map(|c| c.methods()) {
            for name in named(&method.needs) {
                assert!(known(name), "{} names unknown {name}", method.name);
            }
        }
        for method in uncapable::METHODS {
            assert!(matches!(method.needs, Needs::Nothing), "{}", method.name);
        }
    }

    /// `commands.run` and `palette.lead` are refused to a plugin that is not
    /// built-in, live (`Host::dispatch`) and replayed (`replay`), whatever it
    /// declared; every other capability is unaffected.
    #[test]
    fn built_in_only_capabilities_are_refused_to_other_plugins() {
        let only: Vec<_> = ALL
            .iter()
            .map(|c| c.info())
            .filter(|info| info.builtin_only)
            .map(|info| info.name)
            .collect();
        assert_eq!(only, ["commands.run", "palette.lead"]);
        for method in [
            "commands.list",
            "commands.run",
            "palette.lead.set",
            "palette.lead.clear",
            "palette.lead.done",
        ] {
            let needs = required(method, &json!({}), |_| true).unwrap();
            assert!(needs.is_some_and(builtin_only), "{method}");
            let refused = builtin_only_check(needs, false).unwrap_err();
            assert_eq!(refused.code, "capability", "{method}");
            assert_eq!(
                refused.message,
                "this capability is for built-in plugins only"
            );
            assert!(builtin_only_check(needs, true).is_ok(), "{method}");
        }
        // The `commands` capability's own methods stay open to everyone.
        for method in ["commands.register", "commands.keys", "host.run"] {
            let needs = required(method, &json!({}), |_| true).unwrap();
            assert!(builtin_only_check(needs, false).is_ok(), "{method}");
        }
        assert!(builtin_only_check(None, false).is_ok());
        let declared = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            first_builtin_only(&declared(&["commands", "palette.lead"])),
            Some("palette.lead")
        );
        assert_eq!(first_builtin_only(&declared(&["commands", "events"])), None);
        // Exported for the SDK.
        let exported = json();
        let flag = |name: &str| {
            exported["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == name)
                .unwrap()["builtinOnly"]
                .clone()
        };
        assert_eq!(flag("commands.run"), true);
        assert_eq!(flag("palette.lead"), true);
        assert_eq!(flag("commands"), false);
    }

    #[test]
    fn opening_a_remembered_file_needs_media_sources_and_playback_control() {
        // What `Host::dispatch` lets through: the needed capability declared.
        let allowed = |method: &str, declared: &[&str]| {
            let allows = |name: &str| declared.contains(&name);
            required(method, &json!({}), allows)
                .unwrap()
                .is_none_or(allows)
        };
        let both = ["media.sources", "playback.control"];
        assert!(allowed("media.sources.open", &both));
        assert!(!allowed("media.sources.open", &["media.sources"]));
        assert!(!allowed("media.sources.open", &["playback.control"]));
        assert!(!allowed("media.sources.open", &[]));
        // The other members need `media.sources` alone.
        for method in [
            "media.sources.remember",
            "media.sources.get",
            "media.sources.locate",
            "media.sources.forget",
            "media.sources.list",
        ] {
            assert!(allowed(method, &["media.sources"]), "{method}");
            assert!(!allowed(method, &["playback.control"]), "{method}");
        }
    }

    #[test]
    fn a_plugin_follows_up_its_own_source_operation_with_playback_control() {
        let query = |declared: &[&str]| {
            required("source.operation", &json!({"operationId": 1}), |name| {
                declared.contains(&name)
            })
            .unwrap()
        };
        assert_eq!(query(&["playback.control"]), Some("playback.control"));
        assert_eq!(query(&["playback.read"]), Some("playback.read"));
        assert_eq!(
            query(&["playback.read", "playback.control"]),
            Some("playback.control")
        );
        // Neither declared: the check asks for playback.read, which fails.
        assert_eq!(query(&[]), Some("playback.read"));
        let state = required("source.state", &json!({}), |_| true).unwrap();
        assert_eq!(state, Some("playback.read"));
        let unknown = required("source.nothing", &json!({}), |_| true).unwrap_err();
        assert_eq!(unknown.code, "unsupported");
    }

    #[test]
    fn parameters_and_prefixes_choose_the_capability() {
        let needs = |method: &str, params: Value| required(method, &params, |_| false).unwrap();
        let event = |name: &str| needs("events.subscribe", json!({"event": name}));
        assert_eq!(event("packages.changed"), None);
        assert_eq!(event("bindings.changed"), Some("bindings.manage"));
        // A plugin's own key labels need only `commands`.
        assert_eq!(event("commands.keys"), Some("commands"));
        assert_eq!(needs("commands.keys", json!({})), Some("commands"));
        assert_eq!(event("source.state"), Some("events"));
        assert_eq!(needs("events.unsubscribe", json!({})), Some("events"));
        let stage = |stage: Value| needs("processing.replace", json!({"stage": stage}));
        assert_eq!(stage("video".into()), Some("processing.video"));
        assert_eq!(stage("audio".into()), Some("processing.audio"));
        assert_eq!(stage(Value::Null), Some("processing.audio"));
        // An exact name before its prefix; the prefix covers the rest.
        assert_eq!(needs("packages.list", json!({})), None);
        assert_eq!(needs("packages.install", json!({})), Some("plugins.manage"));
        assert_eq!(
            needs("packages.anything", json!({})),
            Some("plugins.manage")
        );
        assert_eq!(needs("snapshot.state", json!({})), Some("media.read"));
        assert_eq!(needs("snapshot.capture", json!({})), Some("media.control"));
        // An export of parts is an export (media.sources checked as well,
        // capabilities/media_control.rs); the check too.
        assert_eq!(
            needs("jobs.start", json!({"parts": []})),
            Some("media.control")
        );
        assert_eq!(needs("jobs.check", json!({})), Some("media.control"));
        assert_eq!(
            needs("media.sources.remember", json!({})),
            Some("media.sources")
        );
        // The host's own source messages are no plugin method.
        for internal in ["owner.sourceLocated", "owner.sourcesRevoked"] {
            assert_eq!(
                required(internal, &json!({}), |_| true).unwrap_err().code,
                "unsupported"
            );
        }
        assert_eq!(needs("subtitles.state", json!({})), Some("subtitles.read"));
        assert_eq!(
            needs("subtitles.select", json!({})),
            Some("subtitles.control")
        );
        assert_eq!(needs("settings.anything", json!({})), Some("ui.settings"));
        assert_eq!(needs("window.state", json!({})), None);
        assert_eq!(needs("storage.session.get", json!({})), None);
        // Names without a prefix entry are not covered by a neighbour's.
        for unknown in [
            "bindings.other",
            "system.other",
            "storage.session.other",
            "ui.other",
        ] {
            assert_eq!(
                required(unknown, &json!({}), |_| true).unwrap_err().code,
                "unsupported",
                "{unknown}"
            );
        }
    }
}
