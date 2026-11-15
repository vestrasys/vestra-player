//! The plugin API's request handlers, one module per area.
//!
//! An area module owns the RPC methods it serves and their handling, and
//! what a plugin holds through them for standby (`record`, `rekey`,
//! `revoke`, `prune`, `replay`). The modules keep the names of the
//! manifest capabilities they once were (`commands`, `ui.mount`, ...);
//! since player 0.9.0 (SDK 1.9.0) a manifest declares nothing, and every
//! plugin may make every request (doc/plugins/README.md). The one refusal
//! left is by provenance: the built-in-only areas (`commands.run`,
//! `palette.lead`) answer only built-in plugins (`Route::check_caller`).
//! [`ALL`] and `uncapable.rs` are the only lists: the dispatch (`lookup`)
//! is built from them.
use crate::{
    host::{Host, Origin, Reply, RpcError},
    playback::backend::Command,
};
use serde_json::Value;
use std::{collections::HashMap, sync::OnceLock};

mod bindings_manage;
mod clipboard;
mod commands;
mod commands_run;
mod events;
mod media_control;
mod media_read;
mod media_sources;
mod mpv;
pub(crate) mod palette_lead;
mod playback_control;
mod playback_read;
mod plugins_manage;
mod processing_video;
mod producer_av;
mod rendering_manage;
mod secrets;
mod source_resolve;
mod subtitles_control;
mod subtitles_read;
mod timeline_marks;
mod triggers;
mod ui_dialog;
mod ui_drop;
pub(crate) mod ui_mount;
mod ui_osd;
mod ui_settings;
mod ui_theme;
mod ui_window;
mod uncapable;
mod video_motion;
mod video_upscaling;
mod window_control;

pub(crate) use commands::refresh_conditions;
pub(crate) use commands_run::{
    CommandRuns, PLAN_STEPS, answered as command_run_answered,
    ask_again as ask_again_before_plugin_commands, forget_trust as forget_command_trust,
    shown_name,
};
pub(crate) use events::{SOURCE_EVENTS, WAKE_EVENTS, wake_trigger};
pub(crate) use media_sources::locate_dialog;
#[cfg(test)]
pub(crate) use mpv::check as mpv_check;
pub(crate) use palette_lead::{EVENT as LEAD_EVENT, Lead, LeadStatus, StatusState};

/// What an area is.
pub struct Info {
    /// Its name, the manifest capability it once was: "commands", "ui.mount", ...
    pub name: &'static str,
    /// Only built-in plugins may use it: `Host::dispatch` and `replay`
    /// refuse its methods to any other plugin with `capability`, built-in
    /// by provenance (`Core::is_builtin`), never by a manifest field or a
    /// load mode.
    pub builtin_only: bool,
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

/// An area of the API: its methods and their handling.
pub trait Capability: Sync {
    fn info(&self) -> &'static Info;
    /// The RPC methods it owns; a method has exactly one owner (`tests`).
    fn methods(&self) -> &'static [&'static str];
    /// Serves one of `methods()`, after the built-in check and the text
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

/// Replays a recorded call in its area. A recording of a package that is
/// not built-in (the same id loaded for development or from another
/// directory) replays nothing built-in-only.
pub fn replay(host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
    let route = lookup(&call.method);
    let capability = route
        .and_then(|route| route.owner)
        .ok_or_else(|| RpcError::new("unsupported", "unknown method"))?;
    let builtin = host.core.is_builtin_package(origin);
    route.map_or(Ok(()), |route| route.check_caller(builtin))?;
    capability.replay(host, owner, origin, call)
}

/// Every area.
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
    // Built-in only (Ask Vestra, doc/architecture/palette-lead.md).
    &commands_run::CommandsRun,
    &palette_lead::PaletteLead,
    // The mpv pass-through (SDK 1.8.0).
    &mpv::Mpv,
    // Triggers (SDK 1.10.0).
    &triggers::Triggers,
];

/// Where a method goes: its owner (`None`: `uncapable`).
#[derive(Clone, Copy)]
pub struct Route {
    owner: Option<&'static dyn Capability>,
}
impl Route {
    /// Only built-in plugins may call it (`Info::builtin_only` of its area).
    pub fn builtin_only(&self) -> bool {
        self.owner.is_some_and(|c| c.info().builtin_only)
    }
    /// The refusal of a built-in-only method to a plugin that is not
    /// built-in (`builtin`: by provenance).
    pub fn check_caller(&self, builtin: bool) -> std::result::Result<(), RpcError> {
        if self.builtin_only() && !builtin {
            return Err(RpcError::new(
                "capability",
                "this capability is for built-in plugins only",
            ));
        }
        Ok(())
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
            .flat_map(|c| c.methods().iter().map(move |m| (Some(*c), *m)))
            .chain(uncapable::METHODS.iter().map(|m| (None, *m)));
        let mut exact = HashMap::new();
        let mut prefixes = Vec::new();
        for (owner, name) in owned {
            let route = Route { owner };
            match name.strip_suffix('*') {
                Some(prefix) => prefixes.push((prefix, route)),
                None => {
                    exact.insert(name, route);
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
    host.core
        .backend
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
    host.core
        .backend
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
    host.core
        .plugins
        .get(&reply.owner)
        .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The area a method goes to (`None`: `uncapable`), or `Err` for
    /// no method.
    fn area(method: &str) -> Result<Option<&'static str>, ()> {
        lookup(method)
            .map(|route| route.owner.map(|c| c.info().name))
            .ok_or(())
    }

    #[test]
    fn every_method_has_one_owner_and_names_are_unique() {
        let mut seen = BTreeSet::new();
        let all = ALL
            .iter()
            .flat_map(|c| c.methods())
            .chain(uncapable::METHODS);
        for method in all {
            assert!(seen.insert(*method), "{method} has two owners");
        }
        let mut names = BTreeSet::new();
        for capability in ALL {
            let info = capability.info();
            assert!(names.insert(info.name), "{} listed twice", info.name);
        }
    }

    /// `commands.run` and `palette.lead` are refused to a plugin that is not
    /// built-in, live (`Host::dispatch`) and replayed (`replay`); every
    /// other area is open to every plugin.
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
            let route = lookup(method).unwrap();
            assert!(route.builtin_only(), "{method}");
            let refused = route.check_caller(false).unwrap_err();
            assert_eq!(refused.code, "capability", "{method}");
            assert_eq!(
                refused.message,
                "this capability is for built-in plugins only"
            );
            assert!(route.check_caller(true).is_ok(), "{method}");
        }
        // The `commands` area's own methods stay open to everyone, as does
        // every other method.
        for method in ["commands.register", "commands.keys", "host.run"] {
            assert!(
                lookup(method).unwrap().check_caller(false).is_ok(),
                "{method}"
            );
        }
        for capability in ALL.iter().filter(|c| !c.info().builtin_only) {
            for method in capability.methods() {
                let route = lookup(method.trim_end_matches('*')).unwrap();
                assert!(route.check_caller(false).is_ok(), "{method}");
            }
        }
        for method in uncapable::METHODS {
            assert!(
                lookup(method).unwrap().check_caller(false).is_ok(),
                "{method}"
            );
        }
    }

    #[test]
    fn an_unknown_source_method_is_no_method() {
        assert_eq!(area("source.operation"), Ok(Some("playback.control")));
        assert_eq!(area("source.state"), Ok(Some("playback.read")));
        assert_eq!(area("source.nothing"), Err(()));
    }

    #[test]
    fn exact_names_come_before_prefixes() {
        assert_eq!(area("events.subscribe"), Ok(Some("events")));
        assert_eq!(area("events.unsubscribe"), Ok(Some("events")));
        assert_eq!(area("processing.replace"), Ok(Some("processing.video")));
        // An exact name before its prefix; the prefix covers the rest.
        assert_eq!(area("packages.list"), Ok(None));
        assert_eq!(area("packages.install"), Ok(Some("plugins.manage")));
        assert_eq!(area("packages.anything"), Ok(Some("plugins.manage")));
        assert_eq!(area("snapshot.state"), Ok(Some("media.read")));
        assert_eq!(area("snapshot.capture"), Ok(Some("media.control")));
        assert_eq!(area("jobs.start"), Ok(Some("media.control")));
        assert_eq!(area("jobs.check"), Ok(Some("media.control")));
        assert_eq!(area("media.sources.open"), Ok(Some("media.sources")));
        assert_eq!(area("media.sources.remember"), Ok(Some("media.sources")));
        // The host's own source messages are no plugin method.
        for internal in ["owner.sourceLocated", "owner.sourcesRevoked"] {
            assert_eq!(area(internal), Err(()), "{internal}");
        }
        assert_eq!(area("subtitles.state"), Ok(Some("subtitles.read")));
        assert_eq!(area("subtitles.select"), Ok(Some("subtitles.control")));
        assert_eq!(area("settings.anything"), Ok(Some("ui.settings")));
        assert_eq!(area("window.state"), Ok(None));
        assert_eq!(area("storage.session.get"), Ok(None));
        // Names without a prefix entry are not covered by a neighbour's.
        for unknown in [
            "bindings.other",
            "system.other",
            "storage.session.other",
            "ui.other",
            "mpv.other",
        ] {
            assert_eq!(area(unknown), Err(()), "{unknown}");
        }
    }
}
