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
    host::{Host, Reply, RpcError},
    playback::backend::Command,
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, sync::OnceLock};

mod bindings_manage;
mod clipboard;
mod commands;
mod events;
mod media_control;
mod media_read;
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
mod ui_mount;
mod ui_osd;
mod ui_settings;
mod ui_theme;
mod ui_window;
mod uncapable;
mod window_control;

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
    /// (lazy activation, doc/process/rounds/2026-10-lazy-activation.md §3.2).
    /// A hint only: nothing reads it yet.
    pub residency: Residency,
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

/// A manifest capability: its metadata, its methods and their handling.
pub trait Capability: Sync {
    fn info(&self) -> &'static Info;
    /// The RPC methods it owns; a method has exactly one owner (`tests`).
    fn methods(&self) -> &'static [Method];
    /// Serves one of `methods()`, after the permission check and the text
    /// resolution of `Host::dispatch`. `Ok(None)`: answered later.
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled;
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
fn forward(host: &Host, reply: &Reply, method: &str, params: &Value) -> Handled {
    host.backend
        .commands
        .send(Command {
            reply: reply.clone(),
            method: method.into(),
            params: params.clone(),
        })
        .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
    Ok(None)
}

/// Hands a request to the playback worker; a stopped worker is `host`
/// (subtitles, snapshots, jobs, video view and render).
fn forward_media(host: &Host, reply: &Reply, method: &str, params: &Value) -> Handled {
    host.backend
        .commands
        .send(Command {
            reply: reply.clone(),
            method: method.into(),
            params: params.clone(),
        })
        .map_err(anyhow::Error::from)?;
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
