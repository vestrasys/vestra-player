//! `ui.dialog`: native file pickers owned by the window where the user
//! last acted for the caller, else by the main window
//! (`origins::dialog_place`), and confirmations in the window where
//! the user last acted for the caller (`host/origins.rs`; without one, the
//! focus decides: `platform::tool_window_focused`): a tool window's as a
//! system dialog owned by it, the main window's on the sheet drawn in the
//! player (`host/review_sheet.rs`, Ask Vestra UX H5).
use super::{Capability, Handled, Info, Recorded};
use crate::{
    host::{
        Event, Host, Reply, RpcError,
        messages::{
            ConfirmRequest, ConfirmSheet, DialogPurpose, DialogRequest, ReviewKind, ReviewRequest,
            UiCommand, UiReport,
        },
        origins::{Origin, Route},
        string,
    },
    i18n, platform,
    ui::{confirm, view},
};
use serde_json::{Value, json};
use slint::ComponentHandle;

pub struct UiDialog;
static INFO: Info = Info {
    name: "ui.dialog",
    builtin_only: false,
};
static METHODS: &[&str] = &["dialogs.open", "dialogs.confirm"];
impl Capability for UiDialog {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    /// Where the user last acted for the plugin stays its origin when it
    /// wakes (`host/origins`): the click that woke it is recorded first.
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.core.origins.rekey(from, to);
    }
    /// A confirmation of a plugin that stopped leaves the sheet unanswered;
    /// its origin goes unless it only stands by.
    /// The sheet is the interface's (`UiCommand::DropConfirm`, plan step
    /// S2d(3)).
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        host.ui_command(UiCommand::DropConfirm { owner });
        if !keep_recorded {
            host.core.origins.forget(owner);
        }
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "dialogs.confirm" => ask(host, reply, params)?,
            _ => open(host, reply, params)?,
        }
        Ok(None)
    }
}

/// `dialogs.confirm`: answered when the user answers (`Event::ConfirmDone`
/// for a system dialog, the sheet's answer otherwise). One confirmation at
/// a time, a system dialog's or a sheet's (an approval sheet's too). The
/// core admits it and chooses the window (plan step S2c(5)); the interface
/// shows it (`UiCommand::Review` for the sheet, `UiCommand::Confirm` for a
/// system dialog).
fn ask(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.core.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    // The interface's facts from the core's copy (plan step S2f(1)).
    #[cfg(debug_assertions)]
    host.check_ui_facts();
    // Refused while another waits; that one comes into view, so the wait
    // is never somewhere the user cannot see.
    if host.core.dialog_busy {
        host.ui_command(crate::host::messages::UiCommand::ShowWaiting);
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    if host.core.sheet_waiting() {
        host.ui_command(crate::host::messages::UiCommand::ShowWaiting);
        return Err(RpcError::new(
            "busy",
            "another confirmation is waiting for the user's answer",
        ));
    }
    // Message and detail are templates (`args`); the button and
    // checkbox texts are translated as they are.
    // A text value carries its own arguments.
    i18n::valid_args(&params["args"]).map_err(RpcError::invalid)?;
    let mut shown = params.clone();
    for field in ["message", "detail", "confirm", "cancel", "checkbox"] {
        let text = &params[field];
        if text.is_null() {
            continue;
        }
        let args = if matches!(field, "message" | "detail") && text.is_string() {
            &params["args"]
        } else {
            &Value::Null
        };
        shown[field] = host.core.plugin_sentence(reply.owner, text, args)?.into();
    }
    let options = confirm::parse(&shown)?;
    // The window where the user last acted for the caller (`host/origins`):
    // a tool window owns a system dialog, the main window gets the sheet.
    // Without an origin or a window of the caller's shown, the focus at
    // arrival decides, as before (a tool window's: the owner
    // `platform::confirm` gives its dialog).
    // Systems without system alerts draw every one on the sheet.
    let route = match host.confirm_route(reply.owner) {
        _ if !platform::SYSTEM_ALERTS && host.core.ui_facts.shell.is_some() => Route::Sheet,
        Route::Focus if host.core.ui_facts.shell.is_some() && !platform::tool_window_focused() => {
            Route::Sheet
        }
        route => route,
    };
    let place = match route {
        Route::Sheet => {
            // The main window's: drawn in the player. The main window comes
            // forward when the sheet first shows (`Host::present_review`:
            // with the focus while the player is in the foreground, asking
            // for attention otherwise); a tool window over the sheet's place
            // stays above it (owned windows, the floating level on macOS):
            // the sheet is only for requests made in the main window, where
            // the user is looking. The request box only for a request typed
            // in the palette's first row: the caller holds the lead, the
            // user last acted for it there (not in its panel, say, while an
            // older request's ticket is still live) and that request is live.
            let now = host.core.now_ms();
            let request = host
                .core
                .lead
                .as_ref()
                .filter(|lead| lead.owner == reply.owner)
                .filter(|_| host.core.origins.get(reply.owner) == Some(&Origin::Lead))
                .and_then(|_| host.core.command_runs.live_request(reply.owner, now));
            host.ui_command(UiCommand::Review(ReviewRequest {
                reply: reply.clone(),
                kind: ReviewKind::Confirm(ConfirmSheet { options, request }),
            }));
            return Ok(());
        }
        Route::Owned(place) => Some(place),
        Route::Focus => None,
    };
    // A system dialog. Busy before it shows: macOS without a window runs
    // it modally (`Host::confirm_command`).
    host.core.dialog_busy = true;
    host.core.confirm_open = true;
    #[cfg(feature = "producer-audio")]
    host.core
        .physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    host.ui_command(UiCommand::Confirm(ConfirmRequest {
        reply: reply.clone(),
        options,
        place,
    }));
    Ok(())
}

/// `dialogs.open`: answered when the picker closes (`Event::DialogDone`).
/// The core checks it and translates its texts; the interface creates the
/// picker (`UiCommand::Dialog`, plan step S2c(5)).
fn open(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.core.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    if host.core.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    let kind = string(params, "kind")?;
    if !["openFile", "openFiles", "openDirectory", "saveFile"].contains(&kind) {
        return Err(RpcError::invalid("unsupported dialog kind"));
    }
    let title = match &params["title"] {
        Value::Null => i18n::tr("Select a path"),
        title => host
            .core
            .plugin_sentence(reply.owner, title, &Value::Null)?,
    };
    if title.len() > 256 {
        return Err(RpcError::invalid("dialog title too long"));
    }
    // A string file name is data; a text value is shown
    // translated, a path separator in the translation replaced.
    let name = match &params["fileName"] {
        name if i18n::text::is_text(name) => Some(
            host.core
                .plugin_sentence(reply.owner, name, &Value::Null)?
                .replace(['/', '\\'], "-"),
        ),
        name => name.as_str().map(str::to_owned),
    };
    if let Some(name) = name.as_deref()
        && (name.len() > 256 || name.contains(['/', '\\']))
    {
        return Err(RpcError::invalid("fileName must be a simple name"));
    }
    // Busy before it shows, as a confirmation: the interface then takes the
    // player off the floating level first (`Host::apply_on_top`).
    host.core.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.core
        .physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    // Owned like a confirmation: the tool window the user acted in, else
    // the main window (`origins::dialog_place`).
    let place = host.dialog_place(reply.owner);
    host.ui_command(UiCommand::Dialog(DialogRequest {
        reply: reply.clone(),
        purpose: DialogPurpose::Open(json!({"kind":kind,"title":title,"fileName":name})),
        place,
    }));
    Ok(())
}

impl Host {
    /// The interface's side of `UiCommand::Dialog` (plan step S2c(5)):
    /// RFD creates the native operation on this Slint main thread; only
    /// awaiting its completion belongs to a worker, which sends the choice
    /// to the core (`Event::DialogDone`, `Event::SourceLocated`).
    pub(crate) fn dialog_command(&mut self, request: DialogRequest) {
        let DialogRequest {
            reply,
            purpose,
            place,
        } = request;
        // Off the floating level before it shows (`dialog_busy` is set).
        self.apply_on_top();
        let owner = place
            .as_ref()
            .and_then(|place| self.place_window(reply.owner, place))
            .or(self.shell.as_ref())
            .map(ComponentHandle::clone_strong);
        let owned = |dialog: rfd::AsyncFileDialog| {
            platform::own_file_dialog(dialog, owner.as_ref().map(|owner| owner.window()))
        };
        let events = self.core.events.clone();
        match purpose {
            DialogPurpose::Open(params) => {
                let kind = params["kind"].as_str().unwrap_or_default();
                let title = params["title"].as_str().unwrap_or_default();
                let mut dialog = owned(rfd::AsyncFileDialog::new()).set_title(title);
                if let Some(name) = params["fileName"].as_str() {
                    dialog = dialog.set_file_name(name);
                }
                match kind {
                    "openFiles" => {
                        let task = dialog.pick_files();
                        std::thread::spawn(move || {
                            let paths = pollster::block_on(task)
                                .map(|v| v.into_iter().map(|f| f.path().to_path_buf()).collect());
                            let _ = events.send(Event::DialogDone(reply, paths));
                        });
                    }
                    _ => {
                        let task: std::pin::Pin<
                            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
                        > = match kind {
                            "openDirectory" => Box::pin(dialog.pick_folder()),
                            "saveFile" => Box::pin(dialog.save_file()),
                            _ => Box::pin(dialog.pick_file()),
                        };
                        std::thread::spawn(move || {
                            let paths =
                                pollster::block_on(task).map(|f| vec![f.path().to_path_buf()]);
                            let _ = events.send(Event::DialogDone(reply, paths));
                        });
                    }
                }
            }
            DialogPurpose::Locate(fields) => {
                let name = fields["name"].as_str().unwrap_or_default();
                let mut dialog = owned(rfd::AsyncFileDialog::new())
                    .set_title(i18n::tr_args("Locate {}", &json!([name])));
                if let Some(directory) = fields["directory"]
                    .as_str()
                    .map(std::path::Path::new)
                    .filter(|d| d.is_dir())
                {
                    dialog = dialog.set_directory(directory);
                }
                if !name.is_empty() {
                    dialog = dialog.set_file_name(name);
                }
                let task = dialog.pick_file();
                std::thread::spawn(move || {
                    let path = pollster::block_on(task).map(|file| file.path().to_path_buf());
                    let _ = events.send(Event::SourceLocated(reply, fields, path));
                });
            }
        }
    }
    /// The interface's side of `UiCommand::Confirm` (plan step S2c(5)): a
    /// system dialog owned by the origin's tool window, brought to the
    /// front first (with the focus while the player is in the foreground,
    /// asking for attention otherwise); else the focused window's, else the
    /// main window's (`platform::confirm`). The answer comes back as
    /// `Event::ConfirmDone`; a dialog that fails to show is answered with
    /// the error (`UiReport::Answered`, marked `confirm_failed`).
    pub(crate) fn confirm_command(&mut self, request: ConfirmRequest) {
        let ConfirmRequest {
            reply,
            options,
            place,
        } = request;
        // Off the floating level while it is open, as a file dialog.
        self.apply_on_top();
        let owner = place
            .as_ref()
            .and_then(|place| self.place_window(reply.owner, place))
            .map(ComponentHandle::clone_strong);
        if let Some(instance) = &owner {
            view::show_again(instance, platform::app_in_foreground());
        }
        let events = self.core.events.clone();
        let answer = reply.clone();
        if let Err(error) = platform::confirm(
            &options,
            owner.as_ref().map(|instance| instance.window()),
            self.native_window,
            move |yes, checked| {
                let _ = events.send(Event::ConfirmDone(answer.clone(), yes, checked));
            },
        ) {
            self.ui_report(UiReport::Answered {
                reply,
                result: Err(error),
                handoff: None,
                confirm_failed: true,
            });
        }
    }
}
