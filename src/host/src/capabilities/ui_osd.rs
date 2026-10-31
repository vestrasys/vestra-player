//! `ui.osd`: messages on the video and the information panel.
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::{
    host::{Host, OSD_MS, Reply, RpcError},
    i18n,
};
use serde_json::Value;
use slint_interpreter::Value as SValue;

/// On-screen display limits: a message's characters, the information
/// block's lines and bytes, and its `maxColumns` range. The SDK repeats them
/// (`OSD_LIMITS`); src/tests/sdk/host-agreement.test.ts checks they agree.
pub const OSD_MESSAGE_CHARS: usize = 200;
pub const OSD_PANEL_LINES: usize = 32;
pub const OSD_PANEL_BYTES: usize = 4096;
pub const OSD_MIN_COLUMNS: u64 = 8;
pub const OSD_MAX_COLUMNS: u64 = 1000;

pub struct UiOsd;
static INFO: Info = Info {
    name: "ui.osd",
    label: "Show messages on the video",
    risky: false,
    requires: &[],
    residency: Residency::Live,
};
static METHODS: &[Method] = &[Method::this("osd.show"), Method::this("osd.panel")];
impl Capability for UiOsd {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.info_panels.contains_key(&owner) {
            return Recorded::Live("an OSD panel");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "osd.show" => show(host, reply, params)?,
            _ => panel(host, reply, params)?,
        }
        Ok(Some(Value::Null))
    }
}

fn show(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    // A template in the plugin's language: translated with its
    // catalog, then formatted with `args` (i18n.md §5).
    i18n::valid_args(&params["args"]).map_err(RpcError::invalid)?;
    let text = host
        .plugin_sentence(reply.owner, &params["text"], &params["args"])?
        .replace(['\n', '\r'], " ");
    if text.trim().is_empty() || text.chars().count() > OSD_MESSAGE_CHARS {
        return Err(RpcError::invalid(format!(
            "osd text needs 1..={OSD_MESSAGE_CHARS} characters"
        )));
    }
    let shell = host
        .shell
        .as_ref()
        .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
    shell
        .set_property("osd-text", SValue::String(text.as_str().into()))
        .map_err(anyhow::Error::from)?;
    shell
        .set_property("osd-visible", true.into())
        .map_err(anyhow::Error::from)?;
    host.osd_until = Some(host.now_ms() + OSD_MS);
    Ok(())
}

fn panel(host: &mut Host, reply: &Reply, params: &Value) -> std::result::Result<(), RpcError> {
    if host.shell.is_none() {
        return Err(RpcError::new("unavailable", "native shell unavailable"));
    }
    match &params["text"] {
        Value::Null => {
            host.info_panels.remove(&reply.owner);
        }
        text if text.is_string() || i18n::text::is_text(text) => {
            let mut text = host.plugin_sentence(reply.owner, text, &params["args"])?;
            // Cut after translation: the plugin cannot measure
            // the translated lines.
            match &params["maxColumns"] {
                Value::Null => {}
                max => {
                    let max = max
                        .as_u64()
                        .filter(|m| (OSD_MIN_COLUMNS..=OSD_MAX_COLUMNS).contains(m))
                        .ok_or_else(|| {
                            RpcError::invalid(format!(
                                "maxColumns is an integer from {OSD_MIN_COLUMNS} to {OSD_MAX_COLUMNS}"
                            ))
                        })?;
                    text = i18n::text::fit_columns(&text, max as usize);
                }
            }
            if text.len() > OSD_PANEL_BYTES || text.lines().count() > OSD_PANEL_LINES {
                return Err(RpcError::invalid(format!(
                    "osd panel allows {OSD_PANEL_LINES} lines, {OSD_PANEL_BYTES} bytes"
                )));
            }
            host.info_panels.insert(reply.owner, text);
        }
        _ => return Err(RpcError::invalid("text requires string or null")),
    }
    host.show_info_panels();
    Ok(())
}
