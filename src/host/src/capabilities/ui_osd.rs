//! `ui.osd`: messages on the video and the information panel.
use super::{Capability, Handled, Info, Recorded};
use crate::{
    host::{Host, OsdActions, Reply, RpcError},
    i18n,
};
use serde_json::Value;

/// On-screen display limits: a message's characters, the information
/// block's lines and bytes, its `maxColumns` range, and a message's action
/// buttons and their titles' characters. The SDK repeats them
/// (`OSD_LIMITS`); src/tests/sdk/host-agreement.test.ts checks they agree.
pub const OSD_MESSAGE_CHARS: usize = 200;
pub const OSD_PANEL_LINES: usize = 32;
pub const OSD_PANEL_BYTES: usize = 4096;
pub const OSD_MIN_COLUMNS: u64 = 8;
pub const OSD_MAX_COLUMNS: u64 = 1000;
pub const OSD_ACTIONS: usize = 2;
pub const OSD_ACTION_CHARS: usize = 40;

pub struct UiOsd;
static INFO: Info = Info {
    name: "ui.osd",
    builtin_only: false,
};
static METHODS: &[&str] = &["osd.show", "osd.panel"];
impl Capability for UiOsd {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.info_panels.contains_key(&owner) {
            return Recorded::Live("an OSD panel");
        }
        // Its buttons run its commands while the message shows.
        if host.osd_actions.as_ref().is_some_and(|a| a.owner == owner) {
            return Recorded::Live("an OSD message with actions");
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
    let owner = reply.owner;
    let actions = parse_actions(
        &params["actions"],
        |title| host.plugin_sentence(owner, title, &Value::Null),
        |id| host.commands.get(owner, id).is_some(),
    )?;
    if host.shell.is_none() {
        return Err(RpcError::new("unavailable", "native shell unavailable"));
    }
    let actions = (!actions.is_empty()).then(|| {
        let (titles, commands) = actions.into_iter().unzip();
        (OsdActions { owner, commands }, titles)
    });
    host.put_osd(&text, Some(owner), actions);
    Ok(())
}

/// `osd.show`'s `actions` (player 0.7.6): absent or a list of at most
/// [`OSD_ACTIONS`] `{ title, command }`, each title 1–[`OSD_ACTION_CHARS`]
/// characters once translated (line breaks shown as spaces), each command
/// one of the plugin's own (`known`). Returns (title, command) pairs.
fn parse_actions(
    actions: &Value,
    title: impl Fn(&Value) -> std::result::Result<String, RpcError>,
    known: impl Fn(&str) -> bool,
) -> std::result::Result<Vec<(String, String)>, RpcError> {
    let list = match actions {
        Value::Null => return Ok(Vec::new()),
        Value::Array(list) if list.len() <= OSD_ACTIONS => list,
        _ => {
            return Err(RpcError::invalid(format!(
                "actions is a list of at most {OSD_ACTIONS}"
            )));
        }
    };
    list.iter()
        .map(|action| {
            let shown = title(&action["title"])?.replace(['\n', '\r'], " ");
            if shown.trim().is_empty() || shown.chars().count() > OSD_ACTION_CHARS {
                return Err(RpcError::invalid(format!(
                    "an action title needs 1..={OSD_ACTION_CHARS} characters"
                )));
            }
            let command = action["command"]
                .as_str()
                .ok_or_else(|| RpcError::invalid("an action's command is a command id"))?;
            if !known(command) {
                return Err(RpcError::invalid(format!(
                    "{command} is not a command of this plugin"
                )));
            }
            Ok((shown, command.to_string()))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn title(value: &Value) -> std::result::Result<String, RpcError> {
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RpcError::invalid("title is text"))
    }

    fn parse(actions: Value) -> std::result::Result<Vec<(String, String)>, RpcError> {
        parse_actions(&actions, title, |id| id == "reveal" || id == "copy")
    }

    #[test]
    fn actions_are_at_most_two_known_commands_with_short_titles() {
        assert!(parse(Value::Null).unwrap().is_empty());
        assert!(parse(json!([])).unwrap().is_empty());
        assert_eq!(
            parse(json!([
                {"title":"Show in\nFolder","command":"reveal"},
                {"title":"Copy","command":"copy"},
            ]))
            .unwrap(),
            [
                ("Show in Folder".to_string(), "reveal".to_string()),
                ("Copy".to_string(), "copy".to_string()),
            ]
        );
        let long = "x".repeat(OSD_ACTION_CHARS + 1);
        let most = "字".repeat(OSD_ACTION_CHARS);
        assert!(parse(json!([{"title":most,"command":"copy"}])).is_ok());
        for bad in [
            json!([
                {"title":"A","command":"copy"},
                {"title":"B","command":"copy"},
                {"title":"C","command":"copy"},
            ]),
            json!({"title":"Copy","command":"copy"}),
            json!([{"title":"Copy","command":"vestra.other/copy"}]),
            json!([{"title":"Copy"}]),
            json!([{"title":" ","command":"copy"}]),
            json!([{"title":long,"command":"copy"}]),
            json!([{"command":"copy"}]),
        ] {
            let error = parse(bad.clone()).unwrap_err();
            assert_eq!(error.code, "invalid", "{bad}");
        }
    }
}
