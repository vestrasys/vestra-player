//! The manifest capabilities and the plugin protocol's methods.
//!
//! **Capabilities** are a closed list of eight names ([`CAPABILITIES`]),
//! decided by whose things a plugin touches, never by feature
//! (doc/plugins/README.md, "Capabilities"): `player` (the user's own player
//! settings), `plugins` (other plugins), and the user's system: `clipboard`,
//! `credentials`, and the reserved `camera`, `microphone`, `screen` and
//! `notifications`, which no method asks for yet. What a plugin does with its
//! own things and with the playback session needs no declaration. No
//! capability is added to this list; a new host API for a resource the
//! operating system protects takes the matching reserved name.
//!
//! **Methods** are served by groups, one module each ([`Group`], [`ALL`]): a
//! group owns its RPC methods, their handling and what they leave behind
//! for standby (`record`, `rekey`, `revoke`, `prune`, `replay`). A method
//! needs one capability or none ([`Method`]); most need none. A group's name
//! is only a name: several were capability names before the list was closed
//! ([`RETIRED`]).
//!
//! Nothing else lists either: the manifest check ([`check_declared`]), the
//! per-request permission check and the dispatch ([`lookup`]) are built from
//! the two tables, and the committed `src/host/capabilities.json` is
//! generated from them (`tests::json_is_current`), for the SDK's tables, the
//! store's labels and src/tests/sdk/host-agreement.test.ts.
//!
//! Adding a method: a line in its group's `METHODS` (a new group: a module
//! implementing [`Group`] and one line in [`ALL`]), then
//! `VESTRA_UPDATE_CAPABILITIES=1 cargo test capabilities` to rewrite the JSON.
use crate::{
    host::{Host, Origin, Reply, RpcError},
    playback::backend::Command,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::OnceLock};

mod bindings_manage;
mod clipboard;
mod commands;
mod commands_run;
mod events;
mod general;
mod media_control;
mod media_read;
mod media_sources;
pub(crate) mod palette_lead;
mod playback_control;
mod playback_read;
mod plugins_manage;
mod processing;
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
mod video_motion;
mod video_upscaling;
mod window_control;

pub(crate) use commands::refresh_conditions;
pub(crate) use commands_run::{
    CommandRuns, PLAN_STEPS, answered as command_run_answered,
    ask_again as ask_again_before_plugin_commands, forget_trust as forget_command_trust,
    shown_name,
};
pub(crate) use events::WAKE_EVENTS;
pub(crate) use media_sources::locate_dialog;
pub(crate) use palette_lead::{EVENT as LEAD_EVENT, Lead, LeadStatus, StatusState};

/// A manifest capability: a boundary a plugin declares that it crosses.
pub struct Capability {
    /// Exactly the manifest string.
    pub name: &'static str,
    /// The store's and the SDK's wording.
    // Read by the JSON generator (`json`, a test).
    #[cfg_attr(not(test), allow(dead_code))]
    pub label: &'static str,
    /// A known name no method asks for yet: a manifest may declare it and
    /// gets nothing by it. The others are active: methods ask for them.
    #[cfg_attr(not(test), allow(dead_code))]
    pub reserved: bool,
}

/// The active capabilities, as the groups' method tables name them.
pub(crate) const PLAYER: &str = "player";
pub(crate) const PLUGINS: &str = "plugins";
pub(crate) const CLIPBOARD: &str = "clipboard";
pub(crate) const CREDENTIALS: &str = "credentials";

/// Every manifest capability, in the order the SDK and the store repeat.
/// The list is closed (module documentation).
pub static CAPABILITIES: &[Capability] = &[
    // The player: effects that outlive the plugin.
    Capability {
        name: PLAYER,
        label: "Change your player settings (shortcuts, theme, language, rendering) and restart the player",
        reserved: false,
    },
    // Other plugins. Running a command acts through whichever plugin owns
    // it, so it is disclosed together with managing them.
    Capability {
        name: PLUGINS,
        label: "Install, uninstall and disable other plugins, and run any command, including theirs",
        reserved: false,
    },
    // The user's system, by the operating systems' own privacy categories.
    Capability {
        name: CLIPBOARD,
        label: "Read and write the clipboard",
        reserved: false,
    },
    Capability {
        name: CREDENTIALS,
        label: "Keep sign-in details in the system keychain",
        reserved: false,
    },
    Capability {
        name: "camera",
        label: "Use the camera",
        reserved: true,
    },
    Capability {
        name: "microphone",
        label: "Use the microphone",
        reserved: true,
    },
    Capability {
        name: "screen",
        label: "Record the screen",
        reserved: true,
    },
    Capability {
        name: "notifications",
        label: "Send system notifications",
        reserved: true,
    },
];

/// The capability names of API `vestra/1` before the list was closed (SDK
/// 1.8.0), each with the capability that took over its methods (`None`:
/// they need none now). Not aliases: a manifest that names one is refused
/// ([`check_declared`]); the table only words the refusal.
pub static RETIRED: &[(&str, Option<&str>)] = &[
    ("producer.av", None),
    ("playback.read", None),
    ("playback.control", None),
    ("source.resolve", None),
    ("events", None),
    ("commands", None),
    ("ui.window", None),
    ("ui.mount", None),
    ("ui.dialog", None),
    ("plugins.manage", Some(PLUGINS)),
    ("processing.video", None),
    ("processing.audio", None),
    ("subtitles.read", None),
    ("subtitles.control", None),
    ("media.read", None),
    ("media.control", None),
    ("media.sources", None),
    ("ui.osd", None),
    ("window.control", None),
    ("bindings.manage", Some(PLAYER)),
    ("timeline.marks", None),
    ("ui.drop", None),
    ("ui.settings", None),
    ("ui.theme", Some(PLAYER)),
    ("secrets", Some(CREDENTIALS)),
    ("rendering.manage", Some(PLAYER)),
    ("video.upscaling", None),
    ("video.motion", None),
    ("commands.run", Some(PLUGINS)),
    ("palette.lead", None),
];

/// Whether `name` is a manifest capability (active or reserved).
pub fn known(name: &str) -> bool {
    CAPABILITIES.iter().any(|c| c.name == name)
}

/// How many manifest capabilities there are.
pub fn count() -> usize {
    CAPABILITIES.len()
}

/// A manifest's `capabilities`: every name is one of [`CAPABILITIES`]. The
/// error names the first that is not and, for a retired name, what to write
/// instead. The manifest is refused whole: no plugin runs with a list its
/// author and the player read differently.
pub fn check_declared(declared: &[String]) -> std::result::Result<(), String> {
    let Some(name) = declared.iter().find(|name| !known(name)) else {
        return Ok(());
    };
    Err(match RETIRED.iter().find(|(retired, _)| retired == name) {
        Some((_, Some(replacement))) => {
            format!("capability \"{name}\" was retired: declare \"{replacement}\" instead")
        }
        Some((_, None)) => format!(
            "capability \"{name}\" was retired: what it allowed needs no capability now, remove it"
        ),
        None => format!(
            "unknown capability \"{name}\" (the capabilities are {})",
            CAPABILITIES
                .iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

/// The refusal of a request whose capability the manifest does not declare.
pub fn undeclared(capability: &str) -> RpcError {
    RpcError::new(
        "capability",
        format!("{capability} must be declared in manifest"),
    )
}

/// What a method group is, for the generated JSON and the SDK's documents.
pub struct Info {
    /// The group's name: "commands", "ui.mount", ...
    pub name: &'static str,
    /// What its methods do, in one line.
    // Read by the JSON generator (`json`, a test).
    #[cfg_attr(not(test), allow(dead_code))]
    pub summary: &'static str,
    /// Whether what a plugin holds through it keeps the plugin running
    /// (doc/architecture/supervisor.md §2, "something live").
    /// A hint for the JSON; what decides is the group's `record`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub residency: Residency,
}

/// The residency hint of a group.
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

/// An RPC method a group serves. A name ending in `.*` covers every method
/// with that prefix that no exact name covers (the prefix's handler refuses
/// the ones it does not know).
pub struct Method {
    pub name: &'static str,
    /// The capability a plugin must declare to call it (`None`: none).
    /// Fixed per method: no parameter and no other declaration changes it.
    pub needs: Option<&'static str>,
}
impl Method {
    /// A method any plugin may call.
    pub const fn free(name: &'static str) -> Self {
        Self { name, needs: None }
    }
    /// A method that needs `capability`, an active one of [`CAPABILITIES`].
    pub const fn needs(name: &'static str, capability: &'static str) -> Self {
        Self {
            name,
            needs: Some(capability),
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

/// What an owner holds through a group, for standby
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

/// A method group: its name, its methods and their handling.
pub trait Group: Sync {
    fn info(&self) -> &'static Info;
    /// The RPC methods it owns; a method has exactly one owner (`tests`).
    fn methods(&self) -> &'static [Method];
    /// Serves one of `methods()`, after the permission check and the text
    /// resolution of `Host::dispatch`. `Ok(None)`: answered later.
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled;
    /// What `owner` holds through this group: calls that recreate its
    /// registrations, or `Live`. `backend` is the playback worker's answer
    /// to `owner.resources` (`{ live, resolvers }`); what the worker holds
    /// (chains, streams, tracks, jobs, upscaling) is judged from its `live`
    /// list as a whole (`Host::lazy_answer`), so groups served there record
    /// `Nothing` here.
    fn record(&self, host: &Host, owner: u64, backend: &Value) -> Recorded;
    /// A parked owner woke as generation `to`: its entries move. Only
    /// recordable groups hold anything for a parked owner; the others do
    /// nothing (an owner holding them is never parked).
    fn rekey(&self, host: &mut Host, from: u64, to: u64);
    /// The registry half of stopping `owner` (`Host::stop`): remove its
    /// entries, or keep them marked recorded when it is parked. What the
    /// other groups hold is still revoked by `Host::stop` itself.
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

/// Every group's registry half of stopping `owner` (`Group::revoke`).
pub fn revoke(host: &mut Host, owner: u64, keep_recorded: bool) {
    for group in ALL {
        group.revoke(host, owner, keep_recorded);
    }
}

/// Every group's entries of `from` move to `to` (`Group::rekey`).
pub fn rekey(host: &mut Host, from: u64, to: u64) {
    for group in ALL {
        group.rekey(host, from, to);
    }
}

/// Every group's recorded entries of `owner` not registered again go.
pub fn prune(host: &mut Host, owner: u64) {
    for group in ALL {
        group.prune(host, owner);
    }
}

/// What `owner` holds: the calls of every group, or the first `Live`.
pub fn record(host: &Host, owner: u64, backend: &Value) -> Result<Vec<Call>, &'static str> {
    let mut calls = Vec::new();
    for group in ALL {
        match group.record(host, owner, backend) {
            Recorded::Nothing => (),
            Recorded::Calls(more) => calls.extend(more),
            Recorded::Live(reason) => return Err(reason),
        }
    }
    Ok(calls)
}

/// What `owner` registered that a recording can hold, whatever else it
/// holds live: every group's calls, a `Live` one's left out (the events
/// group's listeners of wake events kept). Only for `VESTRA_TEST_PARK=all`
/// (host/supervisor/probe.rs), which parks every plugin to diff its
/// registries; a real park needs [`record`].
pub fn recordable(host: &Host, owner: u64, backend: &Value) -> Vec<Call> {
    let mut calls = Vec::new();
    for group in ALL {
        match group.record(host, owner, backend) {
            Recorded::Calls(more) => calls.extend(more),
            Recorded::Live(_) if group.info().name == "events" => {
                calls.extend(events::wake_calls(host, owner));
            }
            Recorded::Nothing | Recorded::Live(_) => (),
        }
    }
    calls
}

/// Replays a recorded call through its group; the package must declare the
/// capability the method needs, as for a live request.
pub fn replay(
    host: &mut Host,
    owner: u64,
    origin: &Origin,
    declared: &[String],
    call: &Call,
) -> Handled {
    let route =
        lookup(&call.method).ok_or_else(|| RpcError::new("unsupported", "unknown method"))?;
    if let Some(needed) = route.needs()
        && !declared.iter().any(|c| c == needed)
    {
        return Err(undeclared(needed));
    }
    route.group.replay(host, owner, origin, call)
}

/// Every method group. The order is the order of `capabilities.json`.
pub static ALL: &[&dyn Group] = &[
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
    &processing::Processing,
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
    &video_motion::VideoMotion,
    &commands_run::CommandsRun,
    &palette_lead::PaletteLead,
    &general::General,
];

/// Where a method goes: its group and its entry.
#[derive(Clone, Copy)]
pub struct Route {
    group: &'static dyn Group,
    method: &'static Method,
}
impl Route {
    /// The capability the request needs (`None`: none).
    pub fn needs(&self) -> Option<&'static str> {
        self.method.needs
    }
    pub fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        self.group.handle(host, reply, method, params)
    }
}

/// Exact method names, and `prefix.*` entries as (prefix with its dot, route).
type Routes = (HashMap<&'static str, Route>, Vec<(&'static str, Route)>);

fn routes() -> &'static Routes {
    static ROUTES: OnceLock<Routes> = OnceLock::new();
    ROUTES.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut prefixes = Vec::new();
        for group in ALL {
            for method in group.methods() {
                let route = Route {
                    group: *group,
                    method,
                };
                match method.name.strip_suffix('*') {
                    Some(prefix) => prefixes.push((prefix, route)),
                    None => {
                        exact.insert(method.name, route);
                    }
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

/// `src/host/capabilities.json`: [`CAPABILITIES`], [`RETIRED`] and [`ALL`]
/// as data.
#[cfg_attr(not(test), allow(dead_code))]
pub fn json() -> Value {
    json!({
        "about": "Generated from src/host/src/capabilities/ (VESTRA_UPDATE_CAPABILITIES=1 cargo test capabilities); do not edit. \
            capabilities: the closed list a manifest may declare; a reserved one is known and no method needs it yet. \
            retired: names a manifest may no longer declare (the player refuses it), with the capability to declare instead (null: none). \
            groups: every protocol method by the group that serves it; a method's needs is null (any plugin) or one capability name. \
            A method name ending in .* covers the prefix's other methods.",
        "apiVersion": crate::process::API,
        "capabilities": CAPABILITIES.iter().map(|c| json!({
            "name": c.name,
            "label": c.label,
            "reserved": c.reserved,
        })).collect::<Vec<_>>(),
        "retired": RETIRED.iter().map(|(name, replaced_by)| json!({
            "name": name,
            "replacedBy": replaced_by,
        })).collect::<Vec<_>>(),
        "groups": ALL.iter().map(|group| {
            let info = group.info();
            json!({
                "name": info.name,
                "summary": info.summary,
                "residency": info.residency.name(),
                "methods": group.methods().iter().map(|m| json!({
                    "name": m.name,
                    "needs": m.needs,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The capability a request needs; unknown methods fail `unsupported`.
    fn required(method: &str) -> std::result::Result<Option<&'static str>, RpcError> {
        lookup(method)
            .map(|route| route.needs())
            .ok_or_else(|| RpcError::new("unsupported", "unknown method"))
    }

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

    /// The closed list: eight names, four active and four reserved. A ninth
    /// is not added (doc/plugins/README.md, "Capabilities").
    #[test]
    fn the_capabilities_are_the_closed_list_of_eight() {
        let names = |reserved| {
            CAPABILITIES
                .iter()
                .filter(|c| c.reserved == reserved)
                .map(|c| c.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(false),
            ["player", "plugins", "clipboard", "credentials"]
        );
        assert_eq!(
            names(true),
            ["camera", "microphone", "screen", "notifications"]
        );
        assert_eq!(count(), 8);
        for capability in CAPABILITIES {
            assert!(known(capability.name));
            assert!(!capability.label.is_empty(), "{}", capability.name);
        }
    }

    #[test]
    fn every_method_has_one_owner_and_names_are_unique() {
        let mut seen = BTreeSet::new();
        for method in ALL.iter().flat_map(|group| group.methods()) {
            assert!(seen.insert(method.name), "{} has two owners", method.name);
        }
        let mut groups = BTreeSet::new();
        for group in ALL {
            assert!(
                groups.insert(group.info().name),
                "{} listed twice",
                group.info().name
            );
            assert!(!group.info().summary.is_empty(), "{}", group.info().name);
        }
        let mut capabilities = BTreeSet::new();
        for capability in CAPABILITIES {
            assert!(
                capabilities.insert(capability.name),
                "{} listed twice",
                capability.name
            );
        }
    }

    /// The whole permission table: these methods need a capability, every
    /// other method needs none. A reserved capability gates nothing.
    #[test]
    fn only_these_methods_need_a_capability() {
        let gated = ALL
            .iter()
            .flat_map(|group| group.methods())
            .filter_map(|method| Some((method.name, method.needs?)))
            .collect::<BTreeSet<_>>();
        let expected = [
            ("appearance.set", "player"),
            ("appearance.setLanguage", "player"),
            ("bindings.list", "player"),
            ("bindings.reset", "player"),
            ("bindings.set", "player"),
            ("rendering.restart", "player"),
            ("rendering.setPreference", "player"),
            ("commands.list", "plugins"),
            ("commands.run", "plugins"),
            ("packages.*", "plugins"),
            ("clipboard.readText", "clipboard"),
            ("clipboard.writeImage", "clipboard"),
            ("clipboard.writeText", "clipboard"),
            ("secrets.delete", "credentials"),
            ("secrets.get", "credentials"),
            ("secrets.set", "credentials"),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(gated, expected);
        let active = CAPABILITIES
            .iter()
            .filter(|c| !c.reserved)
            .map(|c| c.name)
            .collect::<BTreeSet<_>>();
        let needed = gated.iter().map(|(_, c)| *c).collect::<BTreeSet<_>>();
        assert_eq!(needed, active);
    }

    /// A retired name is no alias: the manifest is refused, and the error
    /// says what to write instead.
    #[test]
    fn a_manifest_that_names_a_retired_capability_is_refused_with_its_replacement() {
        let check = |names: &[&str]| {
            check_declared(&names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(check(&[]), Ok(()));
        assert_eq!(
            check(&["player", "plugins", "clipboard", "credentials"]),
            Ok(())
        );
        // Reserved names are known: declaring one is allowed and grants nothing.
        assert_eq!(
            check(&["camera", "microphone", "screen", "notifications"]),
            Ok(())
        );
        assert_eq!(
            check(&["clipboard", "secrets"]).unwrap_err(),
            "capability \"secrets\" was retired: declare \"credentials\" instead"
        );
        assert_eq!(
            check(&["ui.osd"]).unwrap_err(),
            "capability \"ui.osd\" was retired: what it allowed needs no capability now, remove it"
        );
        assert_eq!(
            check(&["player", "telepathy"]).unwrap_err(),
            "unknown capability \"telepathy\" (the capabilities are player, plugins, clipboard, \
             credentials, camera, microphone, screen, notifications)"
        );
        // The 31 names of SDK 1.7 less `clipboard`, which stays.
        assert_eq!(RETIRED.len(), 30);
        let mut names = BTreeSet::new();
        for (name, replaced_by) in RETIRED {
            assert!(names.insert(*name), "{name} listed twice");
            assert!(!known(name), "{name} is still a capability");
            assert!(check(&[name]).is_err(), "{name}");
            if let Some(replacement) = replaced_by {
                let active = CAPABILITIES
                    .iter()
                    .any(|c| c.name == *replacement && !c.reserved);
                assert!(active, "{name} is replaced by {replacement}");
            }
        }
        let replaced = |capability: &str| {
            RETIRED
                .iter()
                .filter(|(_, replaced_by)| *replaced_by == Some(capability))
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            replaced("player"),
            ["bindings.manage", "ui.theme", "rendering.manage"]
        );
        assert_eq!(replaced("plugins"), ["plugins.manage", "commands.run"]);
        assert_eq!(replaced("credentials"), ["secrets"]);
        assert_eq!(replaced("clipboard"), [""; 0]);
    }

    /// What needed a capability of its own, a second one, or one chosen by a
    /// parameter before the list was closed needs none.
    #[test]
    fn what_the_retired_capabilities_gated_is_open_to_every_plugin() {
        for method in [
            "events.subscribe",
            "events.unsubscribe",
            "source.state",
            "source.operation",
            "playback.set",
            "power.preventSleep",
            "processing.replace",
            "processing.operation",
            "video.shaders.set",
            "media.sources.open",
            "media.sources.remember",
            "snapshot.state",
            "snapshot.capture",
            "jobs.start",
            "jobs.check",
            "subtitles.state",
            "subtitles.select",
            "settings.anything",
            "commands.register",
            "commands.keys",
            "host.run",
            "ui.mount",
            "ui.open",
            "dialogs.confirm",
            "osd.show",
            "window.setAlwaysOnTop",
            "timeline.set",
            "drops.provide",
            "sourceResolvers.register",
            "producer.prepare",
            "video.upscaling.set",
            "video.motion.set",
            "palette.lead.set",
            "palette.lead.clear",
            "palette.lead.done",
            "packages.list",
            "window.state",
            "storage.session.get",
            "rendering.state",
        ] {
            assert_eq!(required(method).unwrap(), None, "{method}");
        }
    }

    #[test]
    fn exact_names_come_before_prefixes_and_unknown_methods_are_unsupported() {
        // An exact name before its prefix; the prefix covers the rest.
        assert_eq!(required("packages.list").unwrap(), None);
        assert_eq!(required("packages.install").unwrap(), Some("plugins"));
        assert_eq!(required("packages.anything").unwrap(), Some("plugins"));
        // The host's own source messages are no plugin method, and names
        // without a prefix entry are not covered by a neighbour's.
        for unknown in [
            "owner.sourceLocated",
            "owner.sourcesRevoked",
            "source.nothing",
            "bindings.other",
            "system.other",
            "storage.session.other",
            "ui.other",
        ] {
            assert_eq!(
                required(unknown).unwrap_err().code,
                "unsupported",
                "{unknown}"
            );
        }
    }
}
