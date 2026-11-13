//! `palette.lead`: the command palette's first row, which receives what is
//! typed. No capability: the row is one slot and the host arbitrates it. One
//! plugin holds it at a time, the first to set it (`set` refuses another
//! with `conflict`, reason `lead-taken`), except that a built-in takes it
//! from a holder that is not built in; the slot is free again when the
//! holder clears it or stops for good. Enter, Shift+Enter or a click on it
//! sends the holder the `palette.lead` event with the query and a one-time
//! ticket for `commands.run` (capabilities/commands_run.rs). Recorded, so
//! the row stays while its plugin is in standby and the event wakes it
//! (doc/architecture/palette-lead.md).
use super::{Call, Group, Handled, Info, Method, Recorded, Residency};
use crate::{
    commands::{BUTTON_ICONS, ButtonIcon},
    host::{Host, Origin, Reply, RpcError, string},
    i18n,
};
use serde_json::{Value, json};

pub struct PaletteLead;
static INFO: Info = Info {
    name: "palette.lead",
    summary: "Add the first row of the command palette",
    residency: Residency::Recordable,
};
static METHODS: &[Method] = &[
    Method::free("palette.lead.set"),
    Method::free("palette.lead.clear"),
    Method::free("palette.lead.done"),
];

/// The event the holder gets when the user activates the row (a trigger:
/// it wakes a parked holder, host/supervisor `TRIGGERS`).
pub const EVENT: &str = "palette.lead";

/// What the row shows, in the interface language.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadRow {
    /// `title: <query>`, or the title alone for an empty query.
    pub title: String,
    pub icon: ButtonIcon,
    /// The field's placeholder while the lead is set.
    pub placeholder: Option<String>,
    /// Dimmed after the title while the query is empty.
    pub empty: Option<String>,
    /// The row's right side.
    pub note: Option<String>,
    /// What a request of its holder is doing, or how it ended. A working
    /// status replaces the row while the query is empty and shows in the
    /// status strip while the palette is closed (`host/lead_strip.rs`); an
    /// end state shows only in the strip, once.
    pub status: Option<LeadStatus>,
}

/// `status` of `palette.lead.set`, as shown.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadStatus {
    /// 1–80 characters.
    pub text: String,
    /// The palette row's right side, up to 40 characters.
    pub note: Option<String>,
    pub state: StatusState,
}

/// `status.state` (SDK 1.7.15): still working (absent: the default) or one
/// of the end states, which the host shows once in the strip, removes after
/// their time and never keeps or replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusState {
    Working,
    Done,
    Failed,
    Note,
}

impl StatusState {
    /// The wire names (`PaletteLeadStatus.state`).
    pub const NAMES: [&'static str; 4] = ["working", "done", "failed", "note"];
    fn parse(value: &Value) -> Result<Self, RpcError> {
        match value.as_str() {
            _ if value.is_null() => Ok(Self::Working),
            Some("working") => Ok(Self::Working),
            Some("done") => Ok(Self::Done),
            Some("failed") => Ok(Self::Failed),
            Some("note") => Ok(Self::Note),
            _ => Err(RpcError::invalid(format!(
                "status.state is one of {}",
                Self::NAMES.join(", ")
            ))),
        }
    }
    /// Done, failed or a note: shown once, then gone.
    pub fn ended(self) -> bool {
        self != Self::Working
    }
}

/// The palette's lead: its holder and what it sent (recorded as sent).
#[derive(Clone, Debug)]
pub struct Lead {
    pub owner: u64,
    /// What it sent, less an end-state status (never replayed).
    pub sent: Value,
    pub row: LeadRow,
    /// Replayed for a parked holder and not set again since it woke.
    pub recorded: bool,
    /// The end-state status of the last `set`, already shown: the same one
    /// sent again (an update of the other fields) does not show again.
    pub ended: Option<LeadStatus>,
}

impl Lead {
    /// Its holder is parked (or restarts): the row stays, recorded, but a
    /// working status goes from the row, from the status strip (which would
    /// sweep with nothing working: the holder's tickets end with its
    /// process, `CommandRuns::revoke`) and from the recorded call, so the
    /// woken holder starts without it. Whether a status went.
    pub fn park(&mut self) -> bool {
        self.recorded = true;
        if self.row.status.take().is_none() {
            return false;
        }
        if let Some(object) = self.sent.as_object_mut() {
            object.remove("status");
        }
        true
    }
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
            if let Some(unknown) = status
                .keys()
                .find(|key| !["text", "note", "state"].contains(&key.as_str()))
            {
                return Err(RpcError::invalid(format!("unknown status field {unknown}")));
            }
            let shown = text(texts, &params["status"]["text"], "status.text", 1, 80)?
                .ok_or_else(|| RpcError::invalid("status.text is required"))?;
            Some(LeadStatus {
                text: shown,
                note: text(texts, &params["status"]["note"], "status.note", 1, 40)?,
                state: StatusState::parse(&params["status"]["state"])?,
            })
        }
        _ => {
            return Err(RpcError::invalid(
                "status is { text, note?, state? } or null",
            ));
        }
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

/// Sets `owner`'s lead from `params` (`origin`: its package). The slot's
/// one rule: the first plugin to set it holds it and another's `set` is
/// refused, except that the player's own plugins come first, as they do for
/// panel slots: a built-in's `set` takes the row from a holder that is not
/// built in (by provenance, never by the manifest). So which of the two
/// started first does not matter, and no plugin can keep the row from the
/// player's own. The holder that lost it is not told: its next `set` is
/// refused, it gets no more tickets, and a ticket it still has runs out.
fn set(host: &mut Host, owner: u64, origin: &Origin, params: &Value) -> Result<(), RpcError> {
    let takes = host.lead.as_ref().is_some_and(|lead| {
        lead.owner != owner
            && host.is_builtin_package(origin)
            && !super::commands_run::builtin_owner(host, lead.owner)
    });
    if let Some(lead) = &host.lead
        && lead.owner != owner
        && !takes
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
    let mut row = parse(&origin.texts(), &origin.image_roots(), params)?;
    if takes && let Some(lost) = host.lead.take() {
        diag!(
            "palette lead: {} takes the row from {}",
            origin.id,
            host.plugin_id(lost.owner)
        );
    }
    let appeared = host.lead.is_none();
    // An end state goes to the strip once and is neither kept on the row
    // nor recorded: the row goes back to its usual form.
    let mut sent = params.clone();
    let ended = row.status.take_if(|status| status.state.ended());
    if ended.is_some()
        && let Some(object) = sent.as_object_mut()
    {
        object.remove("status");
    }
    let before = host.lead.as_ref().and_then(|lead| lead.ended.clone());
    host.lead = Some(Lead {
        owner,
        sent,
        row,
        recorded: false,
        ended: ended.clone(),
    });
    if let Some(status) = ended.filter(|status| before.as_ref() != Some(status)) {
        host.lead_ended(&status);
    }
    host.lead_changed(appeared);
    Ok(())
}

fn clear(host: &mut Host, owner: u64) {
    if host.lead.as_ref().is_some_and(|lead| lead.owner == owner) {
        host.lead = None;
        host.lead_changed(true);
    }
}

impl Group for PaletteLead {
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
            let Some(lead) = host.lead.as_mut().filter(|lead| lead.owner == owner) else {
                return;
            };
            if lead.park() {
                // Off the palette's row and the strip at once.
                host.lead_changed(false);
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
            Some(LeadStatus {
                text: "Working · Step 2 of 2".into(),
                note: Some("Enter stops".into()),
                state: StatusState::Working,
            })
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
            json!({"title":"Ask","icon":"ai","status":{"text":"t","state":"stopped"}}),
            json!({"title":"Ask","icon":"ai","status":{"text":"t","state":"Done"}}),
            json!({"title":"Ask","icon":"ai","status":{"text":"t","state":1}}),
            json!({"title":"Ask","icon":"ai","status":{"text":"t","state":""}}),
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
    fn a_status_is_working_unless_its_state_says_it_ended() {
        let state = |status: Value| {
            parse(
                &english(),
                &[],
                &json!({"title":"Ask","icon":"ai","status":status}),
            )
            .unwrap()
            .status
            .unwrap()
            .state
        };
        assert_eq!(state(json!({"text":"Planning…"})), StatusState::Working);
        assert_eq!(
            state(json!({"text":"Planning…","state":null})),
            StatusState::Working
        );
        for (name, expected) in StatusState::NAMES.iter().zip([
            StatusState::Working,
            StatusState::Done,
            StatusState::Failed,
            StatusState::Note,
        ]) {
            assert_eq!(state(json!({"text":"t","state":name})), expected);
            assert_eq!(expected.ended(), *name != "working");
        }
    }

    /// T5 (step 2 review follow-up): a holder parked while its status says
    /// it works keeps its row but not the status: the strip stops sweeping,
    /// the row is the plain row again, and the recorded call (replayed when
    /// it wakes) has no status.
    #[test]
    fn a_parked_holder_keeps_its_row_without_a_working_status() {
        let sent = json!({"title":"Ask Vestra","icon":"ai",
            "status":{"text":"Planning…","note":"Enter stops"}});
        let mut lead = Lead {
            owner: 3,
            row: parse(&english(), &[], &sent).unwrap(),
            sent,
            recorded: false,
            ended: None,
        };
        assert!(lead.park());
        assert!(lead.recorded);
        assert_eq!(lead.row.status, None);
        assert_eq!(lead.row.title, "Ask Vestra");
        assert_eq!(lead.sent, json!({"title":"Ask Vestra","icon":"ai"}));
        // Nothing to take the second time; a row without a status is kept.
        assert!(!lead.park());
        assert_eq!(lead.sent, json!({"title":"Ask Vestra","icon":"ai"}));
    }

    #[test]
    fn it_is_recordable() {
        assert_eq!(INFO.residency, Residency::Recordable);
    }
}
