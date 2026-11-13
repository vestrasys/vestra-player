//! `ui.dialog`: native file pickers, and confirmations in the window where
//! the user last acted for the caller (`host/origins.rs`; without one, the
//! focus decides: `platform::tool_window_focused`): a tool window's as a
//! system dialog owned by it, the main window's on the sheet drawn in the
//! player (`host/review_sheet.rs`, Ask Vestra UX H5).
use super::{Group, Handled, Info, Method, Recorded, Residency};
use crate::{
    host::{
        Event, Host, Reply, RpcError,
        origins::{Origin, Route},
        string,
    },
    i18n, platform,
    ui::{confirm, view},
};
use serde_json::Value;
use slint::ComponentHandle;

pub struct UiDialog;
static INFO: Info = Info {
    name: "ui.dialog",
    summary: "Show file pickers and confirmations",
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::free("dialogs.open"),
    Method::free("dialogs.confirm"),
];
impl Group for UiDialog {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    /// Where the user last acted for the plugin stays its origin when it
    /// wakes (`host/origins`): the click that woke it is recorded first.
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.origins.rekey(from, to);
    }
    /// A confirmation of a plugin that stopped leaves the sheet unanswered;
    /// its origin goes unless it only stands by.
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        host.drop_confirm_of(owner);
        if !keep_recorded {
            host.origins.forget(owner);
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
/// a time, a system dialog's or a sheet's (an approval sheet's too).
fn ask(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    // Refused while another waits; that one comes into view, so the wait
    // is never somewhere the user cannot see.
    if host.dialog_busy {
        host.show_waiting_confirmation();
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    if host.sheet_waiting() {
        host.show_waiting_confirmation();
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
        shown[field] = host.plugin_sentence(reply.owner, text, args)?.into();
    }
    let options = confirm::parse(&shown)?;
    // The window where the user last acted for the caller (`host/origins`):
    // a tool window owns a system dialog, the main window gets the sheet.
    // Without an origin or a window of the caller's shown, the focus at
    // arrival decides, as before (a tool window's: the owner
    // `platform::confirm` gives its dialog).
    // Systems without system alerts draw every one on the sheet.
    let route = match host.confirm_route(reply.owner) {
        _ if !platform::SYSTEM_ALERTS && host.shell.is_some() => Route::Sheet,
        Route::Focus if host.shell.is_some() && !platform::tool_window_focused() => Route::Sheet,
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
            let now = host.now_ms();
            let request = host
                .lead
                .as_ref()
                .filter(|lead| lead.owner == reply.owner)
                .filter(|_| host.origins.get(reply.owner) == Some(&Origin::Lead))
                .and_then(|_| host.command_runs.live_request(reply.owner, now));
            host.open_confirm(reply.clone(), options, request);
            return Ok(());
        }
        Route::Owned(place) => Some(place),
        Route::Focus => None,
    };
    // A system dialog: owned by the origin's tool window, brought to the
    // front first (with the focus while the player is in the foreground,
    // asking for attention otherwise); else the focused window's, else
    // the main window's (`platform::confirm`).
    let owner = place
        .as_ref()
        .and_then(|place| host.place_window(reply.owner, place))
        .map(ComponentHandle::clone_strong);
    if let Some(instance) = &owner {
        view::show_again(instance, platform::app_in_foreground());
    }
    let events = host.events.clone();
    let answer = reply.clone();
    // Busy before it shows: macOS without a window runs it modally here.
    host.dialog_busy = true;
    host.confirm_open = true;
    #[cfg(feature = "producer-audio")]
    host.physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    if let Err(error) = platform::confirm(
        &options,
        owner.as_ref().map(|instance| instance.window()),
        host.native_window,
        move |yes, checked| {
            let _ = events.send(Event::ConfirmDone(answer.clone(), yes, checked));
        },
    ) {
        host.dialog_busy = false;
        host.confirm_open = false;
        return Err(error);
    }
    Ok(())
}

/// `dialogs.open`: answered when the picker closes (`Event::DialogDone`).
fn open(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    if host.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    let kind = string(params, "kind")?;
    if !["openFile", "openFiles", "openDirectory", "saveFile"].contains(&kind) {
        return Err(RpcError::invalid("unsupported dialog kind"));
    }
    let title = match &params["title"] {
        Value::Null => i18n::tr("Select a path"),
        title => host.plugin_sentence(reply.owner, title, &Value::Null)?,
    };
    if title.len() > 256 {
        return Err(RpcError::invalid("dialog title too long"));
    }
    let title = title.as_str();
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    // A string file name is data; a text value is shown
    // translated, a path separator in the translation replaced.
    let name = match &params["fileName"] {
        name if i18n::text::is_text(name) => Some(
            host.plugin_sentence(reply.owner, name, &Value::Null)?
                .replace(['/', '\\'], "-"),
        ),
        name => name.as_str().map(str::to_owned),
    };
    if let Some(name) = name.as_deref() {
        if name.len() > 256 || name.contains(['/', '\\']) {
            return Err(RpcError::invalid("fileName must be a simple name"));
        }
        dialog = dialog.set_file_name(name);
    }
    let events = host.events.clone();
    let reply = reply.clone();
    // RFD creates the native operation on this Slint main thread.
    // Only awaiting its completion belongs to the worker.
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
                let paths = pollster::block_on(task).map(|f| vec![f.path().to_path_buf()]);
                let _ = events.send(Event::DialogDone(reply, paths));
            });
        }
    }
    host.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    Ok(())
}
