//! `ui.dialog`: native file pickers and confirmations.
use super::{Capability, Handled, Info, Method, Residency};
use crate::{
    host::{Event, Host, Reply, RpcError, string},
    i18n, platform,
    ui::confirm,
};
use serde_json::Value;

pub struct UiDialog;
static INFO: Info = Info {
    name: "ui.dialog",
    label: "Show file pickers and confirmations",
    risky: false,
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[
    Method::this("dialogs.open"),
    Method::this("dialogs.confirm"),
];
impl Capability for UiDialog {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "dialogs.confirm" => ask(host, reply, params)?,
            _ => open(host, reply, params)?,
        }
        Ok(None)
    }
}

fn ask(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    if host.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
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
    let events = host.events.clone();
    let answer = reply.clone();
    host.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    if let Err(error) = platform::confirm(&options, host.native_window, move |yes, checked| {
        let _ = events.send(Event::ConfirmDone(answer.clone(), yes, checked));
    }) {
        host.dialog_busy = false;
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
