//! `palette.lead` (built-in only): the command palette's first row, which
//! receives what is typed. One plugin holds it at a time; Enter, Shift+Enter
//! or a click on it sends that plugin the `palette.lead` event with the
//! query and a one-time ticket for `commands.run` (capabilities/
//! commands_run.rs). Recorded, so the row stays while its plugin is in
//! standby and the event wakes it (doc/architecture/palette-lead.md).
use super::{Call, Capability, Handled, Info, Method, Recorded, Residency};
use crate::{
    commands::{BUTTON_ICONS, ButtonIcon},
    host::{Host, Origin, Reply, RpcError, string},
    i18n,
};
use serde_json::{Value, json};

pub struct PaletteLead;
static INFO: Info = Info {
    name: "palette.lead",
    label: "Add the first row of the command palette",
    risky: false,
    requires: &[],
    residency: Residency::Recordable,
    builtin_only: true,
};
static METHODS: &[Method] = &[
    Method::this("palette.lead.set"),
    Method::this("palette.lead.clear"),
    Method::this("palette.lead.done"),
];

/// The event the holder gets when the user activates the row (a trigger:
/// it wakes a parked holder, host/supervisor `TRIGGERS`).
pub const EVENT: &str = "palette.lead";

/// What the row shows, in the interface language.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadRow {
    /// "title: <query>", or the title alone for an empty query.
    pub title: String,
    pub icon: ButtonIcon,
    /// The field's placeholder while the lead is set.
    pub placeholder: Option<String>,
    /// Dimmed after the title while the query is empty.
    pub empty: Option<String>,
    /// The row's right side.
    pub note: Option<String>,
    /// Replaces the row while the query is empty: (text, note).
    pub status: Option<(String, Option<String>)>,
}

/// The palette's lead: its holder and what it sent (recorded as sent).
#[derive(Clone, Debug)]
pub struct Lead {
    pub owner: u64,
    pub sent: Value,
    pub row: LeadRow,
    /// Replayed for a parked holder and not set again since it woke.
    pub recorded: bool,
}

/// A text field of `palette.lead.set`: a string (source text, translated
/// with the plugin's catalog) or a text value, `min..=max` characters as
/// shown.
fn text(
    texts: &i18n::text::Texts,
    value: &Value,
    key: &str,
    min: usize,
    max: usize,
) -> Result<Option<String>, RpcError> {
    if value.is_null() {
        return Ok(None);
    }
    if !value.is_string() && !i18n::text::is_text(value) {
        return Err(RpcError::invalid(format!("{key} is a string or a text")));
    }
    let shown = texts.declared(value).map_err(RpcError::invalid)?;
    let count = shown.chars().count();
    if !(min..=max).contains(&count) || shown.contains(['\0', '\n', '\r']) {
        return Err(RpcError::invalid(format!(
            "{key} must be {min}–{max} characters on one line"
        )));
    }
    Ok(Some(shown))
}

/// `palette.lead.set`'s parameters as the row shows them; an image icon is
/// checked against the plugin's directories (`roots`).
pub fn parse(
    texts: &i18n::text::Texts,
    roots: &[std::path::PathBuf],
    params: &Value,
) -> Result<LeadRow, RpcError> {
    const KEYS: [&str; 6] = ["title", "icon", "placeholder", "empty", "note", "status"];
    let object = params
        .as_object()
        .ok_or_else(|| RpcError::invalid("palette.lead.set takes an object"))?;
    if let Some(unknown) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(RpcError::invalid(format!("unknown field {unknown}")));
    }
    let title = text(texts, &params["title"], "title", 1, 40)?
        .ok_or_else(|| RpcError::invalid("title is required"))?;
    let icon = match &params["icon"] {
        Value::String(name) if BUTTON_ICONS.contains(&name.as_str()) => {
            ButtonIcon::Name(name.clone())
        }
        image if crate::ui::ui_image::reference(image).is_some() => {
            let (path, max_side) = crate::ui::ui_image::parts(image)?;
            crate::ui::ui_image::locate(path, roots)?;
            ButtonIcon::Image {
                path: path.to_owned(),
                max_side,
            }
        }
        _ => {
            return Err(RpcError::invalid(format!(
                "icon is one of {} or an image from plugin.ui.image",
                BUTTON_ICONS.join(", ")
            )));
        }
    };
    let status = match &params["status"] {
        Value::Null => None,
        Value::Object(status) => {
            if let Some(unknown) = status.keys().find(|key| *key != "text" && *key != "note") {
                return Err(RpcError::invalid(format!("unknown status field {unknown}")));
            }
            let shown = text(texts, &params["status"]["text"], "status.text", 1, 80)?
                .ok_or_else(|| RpcError::invalid("status.text is required"))?;
            Some((
                shown,
                text(texts, &params["status"]["note"], "status.note", 1, 40)?,
            ))
        }
        _ => return Err(RpcError::invalid("status is { text, note? } or null")),
    };
    Ok(LeadRow {
        title,
        icon,
        placeholder: text(texts, &params["placeholder"], "placeholder", 1, 60)?,
        empty: text(texts, &params["empty"], "empty", 1, 80)?,
        note: text(texts, &params["note"], "note", 1, 40)?,
        status,
    })
}

/// Sets `owner`'s lead from `params`, refused while another plugin holds it.
fn set(host: &mut Host, owner: u64, origin: &Origin, params: &Value) -> Result<(), RpcError> {
    if let Some(lead) = &host.lead
        && lead.owner != owner
    {
        let holder = host
            .plugins
            .get(&lead.owner)
            .map(|p| p.manifest.id.clone())
            .or_else(|| {
                host.on_demand
                    .held(lead.owner)
                    .map(|held| held.origin.id.clone())
            })
            .unwrap_or_else(|| lead.owner.to_string());
        return Err(RpcError::taken(
            "lead-taken",
            i18n::tr_args("The command palette's first row is {}'s", &json!([holder])),
            json!({"owner": holder}),
        ));
    }
    let row = parse(&origin.texts(), &origin.image_roots(), params)?;
    let appeared = host.lead.is_none();
    host.lead = Some(Lead {
        owner,
        sent: params.clone(),
        row,
        recorded: false,
    });
    host.lead_changed(appeared);
    Ok(())
}

fn clear(host: &mut Host, owner: u64) {
    if host.lead.as_ref().is_some_and(|lead| lead.owner == owner) {
        host.lead = None;
        host.lead_changed(true);
    }
}

impl Capability for PaletteLead {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// The row as its holder set it: shown without its process.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        match &host.lead {
            Some(lead) if lead.owner == owner => {
                Recorded::Calls(vec![Call::new("palette.lead.set", lead.sent.clone())])
            }
            _ => Recorded::Nothing,
        }
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        if let Some(lead) = host.lead.as_mut().filter(|lead| lead.owner == from) {
            lead.owner = to;
        }
    }
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if keep_recorded {
            if let Some(lead) = host.lead.as_mut().filter(|lead| lead.owner == owner) {
                lead.recorded = true;
            }
        } else {
            clear(host, owner);
        }
    }
    fn prune(&self, host: &mut Host, owner: u64) {
        if host
            .lead
            .as_ref()
            .is_some_and(|lead| lead.owner == owner && lead.recorded)
        {
            clear(host, owner);
        }
    }
    fn replay(&self, host: &mut Host, owner: u64, origin: &Origin, call: &Call) -> Handled {
        match call.method.as_str() {
            "palette.lead.set" => set(host, owner, origin, &call.params).map(|()| None),
            _ => Err(RpcError::new("unsupported", "not a recorded method")),
        }
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "palette.lead.set" => {
                let origin = super::plugin(host, reply)?.origin();
                set(host, reply.owner, &origin, params)?;
            }
            "palette.lead.clear" => clear(host, reply.owner),
            _ => {
                let ticket = string(params, "ticket")?;
                host.command_runs.done(reply.owner, ticket);
            }
        }
        Ok(Some(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> i18n::text::Texts<'static> {
        i18n::text::Texts::english()
    }

    #[test]
    fn a_lead_has_a_title_an_icon_and_bounded_texts() {
        let row = parse(
            &english(),
            &[],
            &json!({"title":"Ask Vestra","icon":"ai","placeholder":"Type a command or ask Vestra",
                "empty":"Type a request or a question","note":"Set up a model first",
                "status":{"text":"Working · Step 2 of 2","note":"Enter stops"}}),
        )
        .unwrap();
        assert_eq!(row.title, "Ask Vestra");
        assert_eq!(row.icon, ButtonIcon::Name("ai".into()));
        assert_eq!(row.note.as_deref(), Some("Set up a model first"));
        assert_eq!(
            row.status,
            Some(("Working · Step 2 of 2".into(), Some("Enter stops".into())))
        );
        let minimal = parse(&english(), &[], &json!({"title":"Calc","icon":"search"})).unwrap();
        assert_eq!((minimal.placeholder, minimal.status), (None, None));
        // A text value is shown rendered.
        let text = parse(
            &english(),
            &[],
            &json!({"title":{"$vestraText":"Go to {}","args":["12:30"]},"icon":"ai"}),
        )
        .unwrap();
        assert_eq!(text.title, "Go to 12:30");
        for bad in [
            json!({"icon":"ai"}),
            json!({"title":"","icon":"ai"}),
            json!({"title":"x".repeat(41),"icon":"ai"}),
            json!({"title":"Ask","icon":"rocket"}),
            json!({"title":"Ask","icon":"ai","placeholder":"x".repeat(61)}),
            json!({"title":"Ask","icon":"ai","empty":"x".repeat(81)}),
            json!({"title":"Ask","icon":"ai","note":"x".repeat(41)}),
            json!({"title":"Ask","icon":"ai","status":{"note":"n"}}),
            json!({"title":"Ask","icon":"ai","status":{"text":"x".repeat(81)}}),
            json!({"title":"Ask","icon":"ai","status":{"text":"t","other":1}}),
            json!({"title":"Ask","icon":"ai","status":"busy"}),
            json!({"title":"Ask","icon":"ai","extra":true}),
            json!({"title":"two\nlines","icon":"ai"}),
            json!({"title":3,"icon":"ai"}),
        ] {
            assert_eq!(
                parse(&english(), &[], &bad).unwrap_err().code,
                "invalid",
                "{bad}"
            );
        }
        // 40 CJK characters are 40 characters.
        assert!(
            parse(
                &english(),
                &[],
                &json!({"title":"问".repeat(40),"icon":"ai"})
            )
            .is_ok()
        );
    }

    #[test]
    fn its_capability_is_built_in_only_and_recordable() {
        assert!(INFO.builtin_only && !INFO.risky);
        assert_eq!(INFO.residency, Residency::Recordable);
        assert!(super::super::builtin_only("palette.lead"));
        assert!(super::super::builtin_only("commands.run"));
        assert!(!super::super::builtin_only("commands"));
    }
}
