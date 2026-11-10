//! `media.sources`: persistent references to local files the user opened
//! (`playback/media_sources.rs`, in the playback worker). Requests are
//! forwarded with the plugin's own store key, so a token only means
//! something to the plugin that got it; `media.sources.locate` comes back
//! here for the file dialog (`Event::SourceLocate`). Exports of their parts
//! are `jobs.start` / `jobs.check` with `parts` (`media_control.rs`).
use super::{Capability, Handled, Info, Method, Recorded, Residency, forward_media, plugin};
use crate::{
    host::{Event, Host, Reply, RpcError},
    i18n,
};
use serde_json::{Value, json};

pub struct MediaSources;
static INFO: Info = Info {
    name: "media.sources",
    label: "Remember videos you add and export from them later",
    risky: false,
    // Its own members need nothing else; an export of the references'
    // parts is `jobs.start`, which needs `media.control` as any export.
    requires: &[],
    residency: Residency::None,
};
static METHODS: &[Method] = &[Method::this("media.sources.*")];
/// The methods the prefix covers.
const SOURCES: &[&str] = &[
    "media.sources.reference",
    "media.sources.resolve",
    "media.sources.locate",
    "media.sources.forget",
    "media.sources.list",
    "media.sources.open",
];
impl Capability for MediaSources {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        if !SOURCES.contains(&method) {
            return Err(RpcError::new("unsupported", "unknown method"));
        }
        forward_with_store(host, reply, method, params)
    }
}

/// Forward to the playback worker with the plugin's own store key as
/// `store` (overwriting whatever the plugin sent).
pub(super) fn forward_with_store(
    host: &mut Host,
    reply: &Reply,
    method: &str,
    params: &Value,
) -> Handled {
    let key = crate::playback::media_sources::store_key(&plugin(host, reply)?.data_key);
    let mut params = match params {
        Value::Null => json!({}),
        Value::Object(_) => params.clone(),
        _ => return Err(RpcError::invalid("object required")),
    };
    params["store"] = json!(key);
    forward_media(host, reply, method, &params)
}

/// `media.sources.locate`: the native file dialog for a referenced file
/// (`fields`: `store`, `token`, `name`, `directory`); its choice goes back to
/// the playback worker (`Event::SourceLocated`).
pub(crate) fn locate_dialog(
    host: &mut Host,
    reply: &Reply,
    fields: Value,
) -> std::result::Result<(), RpcError> {
    if host.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    if host.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    let name = fields["name"].as_str().unwrap_or_default();
    let mut dialog =
        rfd::AsyncFileDialog::new().set_title(i18n::tr_args("Locate {}", &json!([name])));
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
    // RFD creates the native operation on this Slint main thread; only
    // awaiting its completion belongs to the worker.
    let task = dialog.pick_file();
    let events = host.events.clone();
    let reply = reply.clone();
    std::thread::spawn(move || {
        let path = pollster::block_on(task).map(|file| file.path().to_path_buf());
        let _ = events.send(Event::SourceLocated(reply, fields, path));
    });
    host.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    Ok(())
}
