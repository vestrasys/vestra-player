//! `ui.window`: a plugin's own Slint windows (ui/view.rs).
//!
//! Plan step S2d: the four requests (rows 36–39, UI-bound, R5) go through
//! the explicit pair. The core checks what it can (`open`, `change`) and
//! sends `UiCommand::View`; the interface carries it out
//! (`Host::view_command`) and answers with `UiReport::Answered`, from which
//! the core replies (`respond`: after an `ui.open` of a run still
//! activating, its readiness follows the reply, R12). Until plan step S3a
//! the command is delivered in place, inside the request, so the reply
//! leaves where it did.
use super::{Capability, Handled, Info, Recorded, plugin};
use crate::{
    host::{
        Core, Host, Reply, RpcError,
        messages::{UiCommand, UiReport, ViewKind, ViewOp},
        string,
    },
    platform, process,
    ui::theme,
    ui::ui_patch,
    ui::ui_values,
    ui::view,
};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::collections::BTreeMap;
use std::path::Path;

pub struct UiWindow;
static INFO: Info = Info {
    name: "ui.window",
    builtin_only: false,
};
static METHODS: &[&str] = &["ui.open", "ui.set", "ui.show", "ui.close"];
impl Capability for UiWindow {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        // The interface's fact (plan step S2f(3)).
        #[cfg(debug_assertions)]
        {
            core.check_settled(&[crate::host::outbox::Fact::Views(owner)]);
        }
        if core.ui_facts.views.contains(&owner) {
            return Recorded::Live("an open window");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        let (id, kind) = match method {
            "ui.open" => open_request(core, reply, params)?,
            _ => change_request(core, reply, method, params)?,
        };
        core.ui_command(UiCommand::View(ViewOp {
            reply: reply.clone(),
            id,
            kind,
        }));
        Ok(None)
    }
}

impl Host {
    /// The interface's side of `UiCommand::View` (plan step S2d): carry
    /// out the view request and report its result (`UiReport::Answered`).
    /// A view's write records are reset and released here, inside the
    /// request as before, so they stay in the one ordered stream with
    /// `ResetWrites`, `SetMount` and `SettingsOp::Set` (design K8).
    pub(crate) fn view_command(&mut self, op: ViewOp) {
        let ViewOp { reply, id, kind } = op;
        let result = match kind {
            ViewKind::Open {
                file,
                properties,
                callbacks,
            } => open(self, &reply, id, &file, &properties, &callbacks),
            ViewKind::Refused(error) => {
                if self.views.contains_key(&(reply.owner, id)) {
                    Err(conflict())
                } else {
                    Err(error)
                }
            }
            ViewKind::Set { params, prepared } => {
                change_view(self, &reply, "ui.set", id, &params, prepared)
            }
            ViewKind::Show { params } => change_view(self, &reply, "ui.show", id, &params, None),
            ViewKind::Close => change_view(self, &reply, "ui.close", id, &Value::Null, None),
        };
        self.ui_report(UiReport::Answered {
            reply,
            result: result.map(|()| Value::Null),
            handoff: None,
            confirm_failed: false,
        });
    }
}

fn conflict() -> RpcError {
    RpcError::new("conflict", "view already exists")
}

/// `ui.open` on the interface: the view's id must be free; the rest as the
/// core checked it.
fn open(
    host: &mut Host,
    reply: &Reply,
    id: String,
    path: &Path,
    properties: &BTreeMap<String, Value>,
    callbacks: &[String],
) -> std::result::Result<(), RpcError> {
    if host.views.contains_key(&(reply.owner, id.clone())) {
        return Err(conflict());
    }
    let view_key = format!("w:{id}");
    // Row patches apply on these initial properties (seq 0).
    host.images.reset_writes(reply.owner, &view_key);
    let mut values = Vec::new();
    for (name, value) in properties {
        match host.prop_value(reply.owner, &view_key, name, value) {
            Ok(value) => values.push((name.clone(), value)),
            Err(error) => {
                host.images.release_view(reply.owner, &view_key);
                return Err(error);
            }
        }
    }
    let theme = host.theme().to_owned();
    let missed = std::cell::Cell::new(false);
    let view = view::open(
        path,
        reply.owner,
        &id,
        &values,
        callbacks,
        host.core.events.clone(),
        |instance| {
            if theme != theme::THEMES[0] && theme::apply(instance, &theme).is_err()
                || theme::apply_fonts(instance).is_err()
            {
                missed.set(true);
            }
        },
    )
    .inspect_err(|_| host.images.release_view(reply.owner, &view_key))?;
    host.theme_pending |= missed.get();
    // The user looks at the window the plugin just showed: its
    // confirmations go there (`host/origins`).
    host.act(reply.owner, crate::host::origins::Origin::View(id.clone()));
    host.views.insert((reply.owner, id), view);
    Ok(())
}

/// `ui.set`, `ui.show` and `ui.close` on the interface, of the view `id`;
/// `prepared`: `ui.set`'s large array, taken by the core.
fn change_view(
    host: &mut Host,
    reply: &Reply,
    method: &str,
    id: String,
    params: &Value,
    prepared: Option<Box<ui_values::Prepared>>,
) -> std::result::Result<(), RpcError> {
    let key = (reply.owner, id);
    if !host.views.contains_key(&key) {
        return Err(RpcError::new("missing", "view not found"));
    }
    let view_key = format!("w:{}", key.1);
    // A large array converted on the reader thread (P3): a whole value was
    // taken out of `params`. A part of a whole value sent in parts is held
    // until the last.
    let mut done = None;
    if method == "ui.set" {
        let property = string(params, "property")?;
        match host.parts(
            reply.owner,
            &view_key,
            property,
            params,
            prepared.as_deref(),
        )? {
            ui_patch::Parted::Held => return Ok(()),
            ui_patch::Parted::Done(value, rows, seq) => done = Some((value, rows, seq)),
            ui_patch::Parted::Not => {}
        }
    }
    if method == "ui.set"
        && done.is_none()
        && let Some(patch) = params.get("patch")
    {
        let property = string(params, "property")?;
        let (base, seq, ops) = ui_patch::request(patch)?;
        if host.images.seq(reply.owner, &view_key, property) != base {
            return Err(ui_patch::stale());
        }
        let current = host.views[&key]
            .instance
            .get_property(property)
            .map_err(|e| RpcError::new("ui", e.to_string()))?;
        if let ui_patch::InWindow::Whole(whole) = host.patch_window(
            reply.owner,
            &view_key,
            property,
            &current,
            ops,
            prepared.as_ref().map(|prepared| &prepared.rows),
        )? {
            // Written whole, as a panel writes its patched stored value.
            let next = host.prop_value(reply.owner, &view_key, property, &whole)?;
            let view = &host.views[&key];
            if !ui_values::reconcile_in(view.instance.window(), &current, &next)? {
                ui_values::set(&view.instance, None, property, next)
                    .map_err(|e| RpcError::new("ui", e.to_string()))?;
            }
        }
        host.images.set_seq(reply.owner, &view_key, property, seq);
        return Ok(());
    }
    let mut seq = params["seq"].as_u64().unwrap_or(0);
    let next = match method {
        "ui.set" => {
            let (value, rows) = match (done, prepared.map(|prepared| *prepared)) {
                (Some((value, rows, last)), _) => {
                    seq = last;
                    (std::borrow::Cow::Owned(value), Some(rows))
                }
                (None, Some(prepared)) => (
                    std::borrow::Cow::Owned(prepared.whole.unwrap_or_default()),
                    Some(prepared.rows),
                ),
                (None, None) => (std::borrow::Cow::Borrowed(&params["value"]), None),
            };
            Some(host.prop_value_with(
                reply.owner,
                &format!("w:{}", key.1),
                string(params, "property")?,
                &value,
                rows.as_ref(),
            )?)
        }
        _ => None,
    };
    let view = &host.views[&key];
    match method {
        "ui.set" => {
            let name = string(params, "property")?;
            let next = next.unwrap_or_default();
            let current = view
                .instance
                .get_property(name)
                .map_err(|e| RpcError::new("ui", e.to_string()))?;
            if !ui_values::reconcile_in(view.instance.window(), &current, &next)? {
                ui_values::set(&view.instance, None, name, next)
                    .map_err(|e| RpcError::new("ui", e.to_string()))?;
            }
            host.images.set_seq(reply.owner, &view_key, name, seq);
        }
        "ui.show" => {
            if params["visible"]
                .as_bool()
                .ok_or_else(|| RpcError::invalid("visible requires boolean"))?
            {
                show(&view.instance, platform::app_in_foreground())?;
                host.act(key.0, crate::host::origins::Origin::View(key.1.clone()));
            } else {
                view.instance.hide().map_err(anyhow::Error::from)?;
            }
        }
        _ => {
            view.instance.hide().map_err(anyhow::Error::from)?;
            host.images.release_view(key.0, &format!("w:{}", key.1));
            host.views.remove(&key);
        }
    }
    Ok(())
}

/// `ui.show` with `visible: true`: show the window, and when it is open
/// already (minimized, or behind other windows) bring it back
/// (`view::show_again`: to the front with the focus while the user works
/// in the player, `foreground`; a taskbar flash or Dock bounce otherwise).
fn show(
    instance: &slint_interpreter::ComponentInstance,
    foreground: bool,
) -> std::result::Result<view::ShownAgain, RpcError> {
    instance.show().map_err(anyhow::Error::from)?;
    platform::shown(instance);
    Ok(view::show_again(instance, foreground))
}

// The two core parts stay below the interface's functions: the source test
// `host::origins::tests::every_input_path_records_an_origin` reads the first
// `fn open(` and `"ui.show" =>` of this file, which must be the interface's.

/// The core's part of `ui.open` (row 36): no interface without a window,
/// the view's id, then the package file, the initial properties and the
/// callbacks (`ViewKind::Open`). A refusal of those three is not answered
/// here: a view of that id already open is refused `conflict` first, as
/// before, and the open views are the interface's, so it goes as
/// `ViewKind::Refused` and the interface answers.
fn open_request(
    core: &mut Core,
    reply: &Reply,
    params: &Value,
) -> std::result::Result<(String, ViewKind), RpcError> {
    if core.headless {
        return Err(RpcError::new(
            "unavailable",
            "UI is unavailable in headless mode",
        ));
    }
    let id = string(params, "id")?.to_owned();
    let checked = (|| -> std::result::Result<ViewKind, RpcError> {
        let file = process::package_file(&plugin(core, reply)?.directory, string(params, "file")?)?;
        let properties: BTreeMap<String, Value> =
            serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                .map_err(|e| RpcError::invalid(e.to_string()))?;
        let callbacks = serde_json::from_value::<Vec<String>>(
            params.get("callbacks").cloned().unwrap_or(json!([])),
        )
        .map_err(|e| RpcError::invalid(e.to_string()))?;
        Ok(ViewKind::Open {
            file,
            properties,
            callbacks,
        })
    })();
    Ok((id, checked.unwrap_or_else(ViewKind::Refused)))
}

/// The core's part of `ui.set`, `ui.show` and `ui.close` (rows 37–39): the
/// view's id, and for `ui.set` the large array its reader thread
/// converted, if any (`Event::Prepared`). Everything else reads the open
/// view, the interface's.
fn change_request(
    core: &mut Core,
    reply: &Reply,
    method: &str,
    params: &Value,
) -> std::result::Result<(String, ViewKind), RpcError> {
    let id = string(params, "id")?.to_owned();
    let kind = match method {
        "ui.set" => ViewKind::Set {
            params: params.clone(),
            prepared: core.take_prepared(reply),
        },
        "ui.show" => ViewKind::Show {
            params: params.clone(),
        },
        _ => ViewKind::Close,
    };
    Ok((id, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ui.show` of an open tool window that the user minimized (Workshop,
    /// Plugin Store: its menu item chosen again) un-minimizes it while the
    /// player is in the foreground, and leaves it minimized otherwise (an
    /// automatic show, such as the resume prompt, takes nothing from the
    /// app the user works in). Slint's `show()` alone left it minimized.
    /// The winit half (raise and focus, `SetForegroundWindow`, the taskbar
    /// flash) has no test platform here: checked by hand in the player.
    #[test]
    fn showing_an_open_minimized_window_restores_it_only_in_the_foreground() {
        let calls = view::recording_platform();
        let instance = view::compile_source(
            "export component W inherits Window { width: 100px; height: 100px; }".into(),
            std::path::Path::new("w.slint"),
        )
        .unwrap();
        instance.show().unwrap();
        let minimized = || {
            // The user minimizes it (the winit backend mirrors the system's
            // state into the window like this).
            instance.window().set_minimized(true);
            calls();
        };
        // Only the minimized state is checked: `show()` on a shown window
        // may repeat `set_visible`, which changes no state.
        let states = || -> Vec<String> {
            calls()
                .into_iter()
                .filter(|call| call.starts_with("minimized"))
                .collect()
        };
        minimized();
        assert_eq!(show(&instance, false).unwrap(), view::ShownAgain::Attention);
        assert_eq!(states(), Vec::<String>::new());
        assert!(instance.window().is_minimized());
        assert_eq!(show(&instance, true).unwrap(), view::ShownAgain::Front);
        assert_eq!(states(), ["minimized false"]);
        assert!(!instance.window().is_minimized());
        // Shown again while already restored: no state change.
        assert_eq!(show(&instance, true).unwrap(), view::ShownAgain::Front);
        assert_eq!(states(), Vec::<String>::new());
    }
}
