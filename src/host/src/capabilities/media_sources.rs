//! `media.sources`: persistent references to local files the user opened
//! (`playback/media_sources.rs`, in the playback worker). Requests are
//! forwarded with the plugin's own store key, so a token only means
//! something to the plugin that got it; `media.sources.locate` comes back
//! here for the file dialog (`Event::SourceLocate`). Exports of their parts
//! are `jobs.start` / `jobs.check` with `parts` (`media_control.rs`).
use super::{Capability, Handled, Info, Recorded, forward_media, plugin};
use crate::host::{
    Host, Reply, RpcError,
    messages::{DialogPurpose, DialogRequest, UiCommand},
};
use serde_json::{Value, json};

pub struct MediaSources;
static INFO: Info = Info {
    name: "media.sources",
    builtin_only: false,
};
static METHODS: &[&str] = &["media.sources.open", "media.sources.*"];
/// The methods the prefix covers.
const SOURCES: &[&str] = &[
    "media.sources.remember",
    "media.sources.get",
    "media.sources.locate",
    "media.sources.forget",
    "media.sources.list",
    "media.sources.open",
];
impl Capability for MediaSources {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
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

/// The shortest time between two Locate dialogs of one plugin.
const LOCATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
static LAST_LOCATE: std::sync::Mutex<std::collections::BTreeMap<u64, std::time::Instant>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// `media.sources.locate`: the native file dialog for a referenced file
/// (`fields`: `store`, `token`, `name`, `directory`); its choice goes back to
/// the playback worker (`Event::SourceLocated`).
pub(crate) fn locate_dialog(
    host: &mut Host,
    reply: &Reply,
    fields: Value,
) -> std::result::Result<(), RpcError> {
    if host.core.headless {
        return Err(RpcError::new(
            "unavailable",
            "native dialogs require a running native window",
        ));
    }
    if host.core.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    // One dialog per plugin every few seconds: a plugin cannot put up one
    // file dialog after another (each dialog still needs the previous one
    // closed, as `dialogs.open`).
    {
        let mut last = LAST_LOCATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        if last
            .get(&reply.owner)
            .is_some_and(|at| now.duration_since(*at) < LOCATE_INTERVAL)
        {
            return Err(RpcError::new(
                "busy",
                "a Locate dialog was shown a moment ago; try again",
            ));
        }
        last.retain(|_, at| now.duration_since(*at) < LOCATE_INTERVAL);
        last.insert(reply.owner, now);
    }
    // Busy before it shows (`dialogs.open`). The interface creates the
    // dialog (`UiCommand::Dialog`, plan step S2c(5)), owned like
    // `dialogs.open`'s (`origins::dialog_place`); its choice comes back as
    // `Event::SourceLocated`.
    host.core.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.core
        .physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    let place = host.dialog_place(reply.owner);
    host.ui_command(UiCommand::Dialog(DialogRequest {
        reply: reply.clone(),
        purpose: DialogPurpose::Locate(fields),
        place,
    }));
    Ok(())
}
