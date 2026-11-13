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
    /// `status.strip` (SDK 1.8.0): what the status strip shows instead of
    /// the status's `text` while working, for a row that says more than the
    /// strip has room for. Only with a working status.
    pub strip: Option<String>,
}

impl LeadRow {
    /// What the status strip shows of a working status: its `strip` text
    /// when the holder gave one, else its `text`. `None` without a working
    /// status.
    pub fn working_strip(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|status| status.state == StatusState::Working)
            .map(|status| self.strip.as_deref().unwrap_or(&status.text))
    }
}

/// The longest texts of `palette.lead.set`, in characters as shown, each on
/// one line as sent. The SDK repeats them (`PALETTE_LEAD_LIMITS`);
/// src/tests/sdk/lead-status-agreement.test.ts checks they agree.
pub const TITLE_CHARS: usize = 40;
pub const PLACEHOLDER_CHARS: usize = 60;
pub const EMPTY_CHARS: usize = 80;
pub const NOTE_CHARS: usize = 40;
/// A working `status.text`: one line of the row and of the strip.
pub const STATUS_TEXT_CHARS: usize = 80;
/// An end state's `status.text` (SDK 1.8.0): it shows in the strip alone,
/// which wraps it to two lines.
pub const STATUS_END_TEXT_CHARS: usize = 160;
pub const STATUS_NOTE_CHARS: usize = 40;
/// `status.strip` (SDK 1.8.0): one line of the strip.
pub const STATUS_STRIP_CHARS: usize = 80;

/// `status` of `palette.lead.set`, as shown.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadStatus {
    /// What the palette's row shows while working, and the strip too unless
    /// `LeadRow::strip` is given: 1–80 characters. An end state's shows in
    /// the strip alone, on up to two lines: 1–160 (SDK 1.8.0).
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
    /// A sentence for the user.
    Note,
    /// The user stopped the work (SDK 1.8.0): shown plain, no answer.
    Stopped,
}

impl StatusState {
    /// The wire names (`PaletteLeadStatus.state`).
    pub const NAMES: [&'static str; 5] = ["working", "done", "failed", "note", "stopped"];
    fn parse(value: &Value) -> Result<Self, RpcError> {
        match value.as_str() {
            _ if value.is_null() => Ok(Self::Working),
            Some("working") => Ok(Self::Working),
            Some("done") => Ok(Self::Done),
            Some("failed") => Ok(Self::Failed),
            Some("note") => Ok(Self::Note),
            Some("stopped") => Ok(Self::Stopped),
            _ => Err(RpcError::invalid(format!(
                "status.state is one of {}",
                Self::NAMES.join(", ")
            ))),
        }
    }
    /// The wire name.
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    /// Done, failed, a note or stopped: shown once, then gone.
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
        // The strip's own text belongs to the working status.
        self.row.strip = None;
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
///
/// The checks keep the order they had before SDK 1.8.0 (the title, the
/// icon, the status: its fields' names, `text`, `note`, `state`; then
/// `placeholder`, `empty`, `note`), so a request with several faults is
/// answered as before. What 1.8.0 added is checked where it belongs: an end
/// state's `text` may be twice as long, and `strip` comes after `state`
/// (first that it is for a working status, then its length).
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
    let title = text(texts, &params["title"], "title", 1, TITLE_CHARS)?
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
    let mut strip = None;
    let status = match &params["status"] {
        Value::Null => None,
        Value::Object(status) => {
            if let Some(unknown) = status
                .keys()
                .find(|key| !["text", "note", "state", "strip"].contains(&key.as_str()))
            {
                return Err(RpcError::invalid(format!("unknown status field {unknown}")));
            }
            // An end state's text shows in the strip alone, on two lines.
            // Any other state (working, none, one that is refused below)
            // keeps the one line.
            let ends = StatusState::parse(&params["status"]["state"]).is_ok_and(StatusState::ended);
            let longest = if ends {
                STATUS_END_TEXT_CHARS
            } else {
                STATUS_TEXT_CHARS
            };
            let shown = text(texts, &params["status"]["text"], "status.text", 1, longest)?
                .ok_or_else(|| RpcError::invalid("status.text is required"))?;
            let status = LeadStatus {
                text: shown,
                note: text(
                    texts,
                    &params["status"]["note"],
                    "status.note",
                    1,
                    STATUS_NOTE_CHARS,
                )?,
                state: StatusState::parse(&params["status"]["state"])?,
            };
            // An end state's text is already the strip's.
            if status.state.ended() && !params["status"]["strip"].is_null() {
                return Err(RpcError::invalid("status.strip is for a working status"));
            }
            strip = text(
                texts,
                &params["status"]["strip"],
                "status.strip",
                1,
                STATUS_STRIP_CHARS,
            )?;
            Some(status)
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
        placeholder: text(
            texts,
            &params["placeholder"],
            "placeholder",
            1,
            PLACEHOLDER_CHARS,
        )?,
        empty: text(texts, &params["empty"], "empty", 1, EMPTY_CHARS)?,
        note: text(texts, &params["note"], "note", 1, NOTE_CHARS)?,
        status,
        strip,
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
    let mut row = parse(&origin.texts(), &origin.image_roots(), params)?;
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
            json!({"title":"Ask","icon":"ai","status":{"text":"t","state":"halted"}}),
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
    fn its_capability_is_built_in_only_and_recordable() {
        assert!(INFO.builtin_only && !INFO.risky);
        assert_eq!(INFO.residency, Residency::Recordable);
        assert!(super::super::builtin_only("palette.lead"));
        assert!(super::super::builtin_only("commands.run"));
        assert!(!super::super::builtin_only("commands"));
    }
}

/// SDK 1.8.0's additions to the status: an end state's longer text and the
/// strip's own text. What was there before is in `tests`, unchanged.
#[cfg(test)]
mod tests_1_8 {
    use super::*;

    fn english() -> i18n::text::Texts<'static> {
        i18n::text::Texts::english()
    }

    fn row(status: Value) -> Result<LeadRow, RpcError> {
        parse(
            &english(),
            &[],
            &json!({"title":"Ask","icon":"ai","status":status}),
        )
    }

    fn message(params: Value) -> String {
        let error = parse(&english(), &[], &params).unwrap_err();
        assert_eq!(error.code, "invalid", "{params}");
        error.message
    }

    /// A working text is one line of the row and of the strip (80
    /// characters); an end state's shows in the strip alone, on up to two
    /// lines (160), for every end state.
    #[test]
    fn an_end_states_text_may_be_twice_as_long_as_a_working_one() {
        let refused = |status: Value| message(json!({"title":"Ask","icon":"ai","status":status}));
        for working in [json!(null), json!("working")] {
            assert!(row(json!({"text":"x".repeat(80),"state":working})).is_ok());
            assert_eq!(
                refused(json!({"text":"x".repeat(81),"state":working})),
                "status.text must be 1–80 characters on one line"
            );
        }
        for end in ["done", "failed", "note", "stopped"] {
            let long = row(json!({"text":"问".repeat(160),"state":end})).unwrap();
            let status = long.status.unwrap();
            assert_eq!(status.text.chars().count(), 160);
            assert!(status.state.ended());
            assert_eq!(
                refused(json!({"text":"x".repeat(161),"state":end})),
                "status.text must be 1–160 characters on one line",
                "{end}"
            );
            // Still one line as sent: the strip wraps it.
            assert_eq!(
                refused(json!({"text":"two\nlines","state":end})),
                "status.text must be 1–160 characters on one line"
            );
            assert_eq!(
                refused(json!({"text":"","state":end})),
                "status.text must be 1–160 characters on one line"
            );
        }
        // A state that is refused gives no longer text: the text's fault is
        // named first, as before, then the state's.
        assert_eq!(
            refused(json!({"text":"x".repeat(81),"state":"halted"})),
            "status.text must be 1–80 characters on one line"
        );
        assert_eq!(
            refused(json!({"text":"x".repeat(80),"state":"halted"})),
            format!("status.state is one of {}", StatusState::NAMES.join(", "))
        );
        // The limits as the SDK repeats them.
        assert_eq!(
            [
                TITLE_CHARS,
                PLACEHOLDER_CHARS,
                EMPTY_CHARS,
                NOTE_CHARS,
                STATUS_TEXT_CHARS,
                STATUS_END_TEXT_CHARS,
                STATUS_NOTE_CHARS,
                STATUS_STRIP_CHARS
            ],
            [40, 60, 80, 40, 80, 160, 40, 80]
        );
    }

    /// `status.strip` is what the strip shows instead of `text` while
    /// working; the row keeps `text`. An end state has none.
    #[test]
    fn a_working_status_may_give_the_strip_its_own_text() {
        let both =
            row(json!({"text":"Vestra is working · Planning…","strip":"Planning…"})).unwrap();
        assert_eq!(
            both.status.as_ref().unwrap().text,
            "Vestra is working · Planning…"
        );
        assert_eq!(both.strip.as_deref(), Some("Planning…"));
        assert_eq!(both.working_strip(), Some("Planning…"));
        // Without one (absent or null) the strip shows the text.
        for plain in [
            json!({"text":"Planning…"}),
            json!({"text":"Planning…","strip":null}),
            json!({"text":"Planning…","state":"working"}),
        ] {
            let plain = row(plain).unwrap();
            assert_eq!(
                (plain.strip.as_deref(), plain.working_strip()),
                (None, Some("Planning…"))
            );
        }
        let explicit = row(json!({"text":"t","state":"working","strip":"s".repeat(80)})).unwrap();
        assert_eq!(explicit.working_strip(), Some("s".repeat(80).as_str()));
        // A text value is shown rendered, as the other texts.
        let value =
            row(json!({"text":"t","strip":{"$vestraText":"Step {}","args":["2"]}})).unwrap();
        assert_eq!(value.working_strip(), Some("Step 2"));
        let refused = |status: Value| message(json!({"title":"Ask","icon":"ai","status":status}));
        for bad in [json!("s".repeat(81)), json!(""), json!("a\nb")] {
            assert_eq!(
                refused(json!({"text":"t","strip":bad})),
                "status.strip must be 1–80 characters on one line"
            );
        }
        assert_eq!(
            refused(json!({"text":"t","strip":3})),
            "status.strip is a string or a text"
        );
        for end in ["done", "failed", "note", "stopped"] {
            assert_eq!(
                refused(json!({"text":"t","state":end,"strip":"s"})),
                "status.strip is for a working status",
                "{end}"
            );
            // A null strip is no strip; an end state shows its text.
            let ended = row(json!({"text":"t","state":end,"strip":null})).unwrap();
            assert_eq!(
                (ended.strip.as_deref(), ended.working_strip()),
                (None, None)
            );
        }
        // No status, no strip text.
        let none = parse(&english(), &[], &json!({"title":"Ask","icon":"ai"})).unwrap();
        assert_eq!((none.strip, none.status), (None, None));
    }

    /// The order of the checks is the one from before SDK 1.8.0, with the
    /// strip's two checks after the state's: a request with several faults
    /// is answered as it was.
    #[test]
    fn a_request_with_several_faults_names_the_same_one_as_before() {
        // The title, then the icon, before anything of the status.
        assert_eq!(
            message(
                json!({"title":"","icon":"rocket","placeholder":"p".repeat(61),
                "status":{"text":"x".repeat(200),"state":"halted","strip":"s".repeat(99)}})
            ),
            "title must be 1–40 characters on one line"
        );
        assert!(
            message(
                json!({"title":"Ask","icon":"rocket","placeholder":"p".repeat(61),
                "status":{"text":"x".repeat(200),"state":"halted","strip":"s".repeat(99)}})
            )
            .starts_with("icon is one of ")
        );
        // Of the status: an unknown field, its text, its note, its state.
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai",
                "status":{"text":"x".repeat(200),"other":1,"state":"halted"}})),
            "unknown status field other"
        );
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai",
                "status":{"text":"x".repeat(81),"note":"n".repeat(41),"state":"halted"}})),
            "status.text must be 1–80 characters on one line"
        );
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai","status":{"note":"n","state":"halted"}})),
            "status.text is required"
        );
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai",
                "status":{"text":"t","note":"n".repeat(41),"state":"halted"}})),
            "status.note must be 1–40 characters on one line"
        );
        assert_eq!(
            message(
                json!({"title":"Ask","icon":"ai","placeholder":"p".repeat(61),
                "status":{"text":"t","state":"halted","strip":"s".repeat(99)}})
            ),
            format!("status.state is one of {}", StatusState::NAMES.join(", "))
        );
        // Then the strip: that it is for a working status, then its length.
        assert_eq!(
            message(
                json!({"title":"Ask","icon":"ai","placeholder":"p".repeat(61),
                "status":{"text":"t","state":"done","strip":"s".repeat(99)}})
            ),
            "status.strip is for a working status"
        );
        assert_eq!(
            message(
                json!({"title":"Ask","icon":"ai","placeholder":"p".repeat(61),
                "status":{"text":"t","strip":"s".repeat(81)}})
            ),
            "status.strip must be 1–80 characters on one line"
        );
        // The row's other texts last.
        assert_eq!(
            message(
                json!({"title":"Ask","icon":"ai","placeholder":"p".repeat(61),
                "empty":"e".repeat(81),"note":"n".repeat(41),"status":{"text":"t","strip":"s"}})
            ),
            "placeholder must be 1–60 characters on one line"
        );
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai","empty":"e".repeat(81),
                "note":"n".repeat(41)})),
            "empty must be 1–80 characters on one line"
        );
        assert_eq!(
            message(json!({"title":"Ask","icon":"ai","note":"n".repeat(41)})),
            "note must be 1–40 characters on one line"
        );
    }

    /// A parked holder keeps its row without its working status: the
    /// strip's own text goes with it, from the row and the recorded call.
    #[test]
    fn a_parked_holder_loses_the_strips_text_with_its_working_status() {
        let sent = json!({"title":"Ask Vestra","icon":"ai",
            "status":{"text":"Vestra is working · Planning…","strip":"Planning…"}});
        let mut lead = Lead {
            owner: 3,
            row: parse(&english(), &[], &sent).unwrap(),
            sent,
            recorded: false,
            ended: None,
        };
        assert_eq!(lead.row.working_strip(), Some("Planning…"));
        assert!(lead.park());
        assert_eq!(
            (lead.row.strip.as_deref(), lead.row.working_strip()),
            (None, None)
        );
        assert_eq!(lead.sent, json!({"title":"Ask Vestra","icon":"ai"}));
    }
}
