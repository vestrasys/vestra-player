//! The approval sheet (Ask Vestra UX step 3, H3): before a `commands.run`
//! plan with a command that asks first runs, the host asks once, in the
//! main window, where the palette and the status strip are (top centre),
//! in the palette's style, following the theme, without a system sound.
//! It is a menu-layer mode (`MenuMode::Review`, beside the palette), drawn
//! by `ui/review-sheet.slint` from the host's own data: the request the
//! ticket was minted for, each step's palette label and plugin, and why a
//! step asks when that is a fact (not built in; the player's Close and
//! Quit). The caller cannot answer, skip or word it.
//!
//! The same sheet asks a plugin's `dialogs.confirm` raised for the main
//! window (step 5, H5; `capabilities/ui_dialog.rs`): Ask Vestra's first-use
//! consent, clearing the playlist or the history, removing subtitles. A
//! confirmation is the main window's when the user last acted for the
//! caller there (`host/origins.rs`), or, without an origin, when no tool
//! window (a plugin's window, Options, the floating sidebar) has the focus
//! (`platform::tool_window_focused`); a tool window's stays a system
//! dialog owned by it. It shows
//! the caller's message, detail, checkbox and button texts; the request box
//! only when the caller holds the palette's first row and its request is
//! live (the consent asked while a request waits for it), and then that
//! request ending (120 s unused, `palette.lead.done`) answers Cancel. Its
//! checkbox is the caller's: its state is reported with either answer, and
//! Ctrl+Enter is a plain Enter. `destructive` draws nothing (user ruling 2
//! of 2026-11-12: no reason label; the approval sheet marks no destructive
//! step either).
//!
//! - It is modal: Enter or Run runs the plan, Esc, Don't Run or a press
//!   outside it runs nothing; keys and presses within [`GUARD_MS`] of it
//!   showing count for nothing (a carried-over Enter), and so do held-down
//!   repeats (the view).
//! - One at a time, a plan's or a confirmation's: while one waits, a run
//!   that would ask and a confirmation are refused `busy`
//!   ([`Host::sheet_waiting`]).
//! - It waits while a native dialog is open (`dialog_busy`), and shows again
//!   when something else closed the menu layer (a rebuilt shell, another
//!   menu): only an answer, the ticket's end (Don't Run, Cancel) or the
//!   caller's end removes it. It does not take the focus from another
//!   program.
//! - A plan's answer goes the usual way (`Event::CommandRunAnswered`); a
//!   confirmation's is the reply to `dialogs.confirm`.
//! - `"Don't ask again for <plugin>"` (step 4, H4) sits at the footer's left
//!   when the plan's steps that ask include some only because their plugin
//!   is not built in and no step is destructive (user ruling 3 of
//!   2026-11-12); Run with it checked, or Ctrl+Enter, runs and remembers.
//!   The host owns the box (`Review::checked`, unchecked for every sheet):
//!   a click asks it to toggle and counts for nothing within the guard, as
//!   every other input ([`decide`]). What is remembered is the host's list
//!   (`ConfirmedRun::remember`).
//!
//! doc/architecture/palette-lead.md §2.1.
use super::*;
use crate::ui::{command_palette, confirm};

/// Input within this long of the sheet showing is ignored (Chromium's
/// rule for security-sensitive UI; PROPOSAL §4.3).
pub(crate) const GUARD_MS: u64 = 500;
/// The sheet's step rows (`ui/review-sheet.slint`): every plan fits
/// (`commands_run::PLAN_STEPS`).
pub(crate) const SLOTS: usize = crate::capabilities::PLAN_STEPS;

/// One step of the plan as the sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    /// The command's palette label (`Category: Title`).
    pub(crate) label: String,
    /// Its plugin's shown name.
    pub(crate) plugin: String,
    /// Why it asks, when that is a fact; empty otherwise.
    pub(crate) reason: String,
    /// It asks first (drawn in `text`, the others in `text-list`).
    pub(crate) asks: bool,
    /// Its plugin is built in.
    pub(crate) builtin: bool,
}

/// What a sheet asks about.
pub(crate) enum Asks {
    /// A `commands.run` plan on `ticket`, minted for the request `query`.
    Run {
        ticket: String,
        query: String,
        steps: Vec<Step>,
    },
    /// A `dialogs.confirm` raised for the main window; `request`: the
    /// caller's live ticket and its request, when it holds the lead.
    Confirm {
        options: confirm::Options,
        request: Option<(String, String)>,
    },
}

/// A sheet waiting for the user's answer.
pub(crate) struct Review {
    /// The waiting call's reply (a run's: `CommandRuns::confirming`).
    pub(crate) reply: Reply,
    pub(crate) asks: Asks,
    /// The footer's checkbox text: "Don't ask again" (a plan) or the
    /// caller's (a confirmation); empty: no checkbox.
    pub(crate) checkbox: String,
    /// The box is checked (the host's state; unchecked for every sheet).
    pub(crate) checked: bool,
    /// When it last showed (`Core::now_ms`); `None` while it waits to show.
    pub(crate) shown_at: Option<u64>,
    /// It has shown once: the main window came into view for it
    /// (`Host::present_review`), which is not repeated when it shows again
    /// after another menu or a native dialog.
    pub(crate) hinted: bool,
}

impl Review {
    /// The ticket whose end ends the sheet.
    fn ticket(&self) -> Option<&str> {
        match &self.asks {
            Asks::Run { ticket, .. } => Some(ticket),
            Asks::Confirm { request, .. } => request.as_ref().map(|(ticket, _)| ticket.as_str()),
        }
    }
    /// The checkbox is the caller's (a confirmation): reported with either
    /// answer.
    fn reports_checkbox(&self) -> bool {
        matches!(self.asks, Asks::Confirm { .. })
    }
}

/// What an action on the sheet does.
#[derive(Debug, PartialEq)]
pub(crate) enum Decision {
    /// Within the guard, unknown, or a toggle with no checkbox.
    Ignore,
    /// The checkbox changes.
    Toggle,
    /// The answer: (yes, checked) — for a plan, (run, remember).
    Answer(bool, bool),
}

/// An action on a sheet shown at `shown_at`, at `now`, with a checkbox
/// (`offered`) that is `checked`: `run` (with the box's state),
/// `run-remember` (Ctrl+Enter: on a plan, remembering when offered),
/// `toggle-remember` (a click on the box) or `decline`. `reported`: the box
/// is a confirmation's (the caller's), so its state goes with either answer
/// and Ctrl+Enter is a plain Enter; a plan's box ("Don't ask again") counts
/// only with Run. Everything, the box included, counts for nothing within
/// [`GUARD_MS`].
pub(crate) fn decide(
    action: &str,
    shown_at: u64,
    now: u64,
    offered: bool,
    checked: bool,
    reported: bool,
) -> Decision {
    if !accepts(shown_at, now) {
        return Decision::Ignore;
    }
    let checked = offered && checked;
    match action {
        "run" => Decision::Answer(true, checked),
        "run-remember" => Decision::Answer(true, if reported { checked } else { offered }),
        "toggle-remember" if offered => Decision::Toggle,
        "decline" => Decision::Answer(false, reported && checked),
        _ => Decision::Ignore,
    }
}

/// Whether input at `now` counts on a sheet shown at `shown_at`.
pub(crate) fn accepts(shown_at: u64, now: u64) -> bool {
    now.saturating_sub(shown_at) >= GUARD_MS
}

/// The sheet's question: one command or `n`.
pub(crate) fn question(steps: usize) -> String {
    if steps == 1 {
        i18n::tr("Run this command?")
    } else {
        i18n::tr_args("Run these {n} commands?", &json!({ "n": steps }))
    }
}

/// The "Don't ask again" checkbox's text for the plugins it would trust,
/// by shown name (PROPOSAL §4.5): one or two by name, more by count (their
/// rows show which); none: empty (no checkbox).
pub(crate) fn remember_text(plugins: &[String]) -> String {
    match plugins {
        [] => String::new(),
        [one] => i18n::tr_args("Don't ask again for {plugin}", &json!({ "plugin": one })),
        [first, second] => {
            i18n::tr_args("Don't ask again for {0} and {1}", &json!([first, second]))
        }
        more => i18n::tr_args(
            "Don't ask again for these {n} plugins",
            &json!({ "n": more.len() }),
        ),
    }
}

/// A plan's checkbox once what it would trust is down to the plugins
/// `names` (an uninstall, a replace, Ask Again): `None` when its text
/// (`current`) stays; else the new text and whether the box is still checked
/// (never when it goes: an empty text draws no checkbox).
pub(crate) fn offer_after(
    names: &[String],
    current: &str,
    checked: bool,
) -> Option<(String, bool)> {
    let text = remember_text(names);
    (text != current).then(|| {
        let checked = checked && !text.is_empty();
        (text, checked)
    })
}

/// The `ReviewSheet` value (ui/review-sheet.slint) of a plan, less its
/// image: `title` and `icon` of the request box, the request, the steps in
/// [`SLOTS`] rows, the note when a step's plugin is not built in, the
/// "Don't ask again" text (empty: no checkbox), and the buttons' texts and
/// keys.
pub(crate) fn sheet(
    title: &str,
    icon: &str,
    query: &str,
    steps: &[Step],
    remember: &str,
    keys: [&str; 2],
) -> Value {
    let rows: Vec<Value> = (0..SLOTS)
        .map(|at| match steps.get(at) {
            Some(step) => json!({"shown": true, "number": (at + 1).to_string(),
                "label": step.label, "plugin": step.plugin, "reason": step.reason,
                "asks": step.asks}),
            None => empty_row(),
        })
        .collect();
    let note = if steps.iter().any(|step| !step.builtin) {
        i18n::tr("The player cannot tell what commands of plugins that are not built in will do.")
    } else {
        String::new()
    };
    json!({
        "request": true,
        "icon": icon,
        "title": title,
        "query": query.trim(),
        "question": question(steps.len()),
        "message": "",
        "detail": "",
        "count": steps.len().min(SLOTS),
        "steps": rows,
        "note": note,
        "remember": remember,
        "run": i18n::tr("Run"),
        "cancel": i18n::tr("Don't Run"),
        "run-key": keys[0],
        "cancel-key": keys[1],
    })
}

fn empty_row() -> Value {
    json!({"shown": false, "number": "", "label": "", "plugin": "", "reason": "", "asks": false})
}

/// The `ReviewSheet` value of a confirmation, less its image: the request
/// box's (title, icon, request) when there is one, the caller's message,
/// detail, checkbox and buttons (as `confirm::parse` checked them), and
/// the buttons' keys. No question, no steps, no note.
pub(crate) fn confirm_sheet(
    request: Option<(&str, &str, &str)>,
    options: &confirm::Options,
    keys: [&str; 2],
) -> Value {
    let (title, icon, query) = request.unwrap_or_default();
    json!({
        "request": request.is_some(),
        "icon": icon,
        "title": title,
        "query": query.trim(),
        "question": "",
        "message": options.message,
        "detail": options.detail.as_deref().unwrap_or_default(),
        "count": 0,
        "steps": (0..SLOTS).map(|_| empty_row()).collect::<Vec<_>>(),
        "note": "",
        "remember": options.checkbox.as_deref().unwrap_or_default(),
        "run": options.confirm,
        "cancel": options.cancel,
        "run-key": keys[0],
        "cancel-key": keys[1],
    })
}

impl Host {
    /// The interface's side of `UiCommand::Review` (plan step S2c(5)): the
    /// core admitted the request and chose the main window's sheet; its
    /// answer goes back as `Event::CommandRunAnswered` (a plan) or
    /// `UiReport::Answered` (a confirmation).
    pub(crate) fn review_command(&mut self, request: messages::ReviewRequest) {
        let messages::ReviewRequest { reply, kind } = request;
        match kind {
            messages::ReviewKind::Confirm(messages::ConfirmSheet { options, request }) => {
                self.open_confirm(reply, options, request)
            }
            messages::ReviewKind::CommandRun(messages::RunSheet {
                ticket,
                query,
                steps,
                remember,
            }) => self.open_review(reply, ticket, query, steps, remember),
        }
    }
    /// `commands.run` asks the user about `steps` (the run's own first) on
    /// `ticket`, offering "Don't ask again" with the text `remember` (empty:
    /// not offered): the sheet shows now, or once nothing is in its way. One
    /// at a time: `CommandRuns::admit` refuses (`busy`) a run that would ask
    /// while any sheet waits, so this never replaces a waiting one.
    pub(crate) fn open_review(
        &mut self,
        reply: Reply,
        ticket: String,
        query: String,
        steps: Vec<Step>,
        remember: String,
    ) {
        // The count only: the labels may name the user's file.
        diag!("review sheet: asks about {} steps", steps.len());
        self.open_sheet(Review {
            reply,
            asks: Asks::Run {
                ticket,
                query,
                steps,
            },
            checkbox: remember,
            checked: false,
            shown_at: None,
            hinted: false,
        });
    }
    /// `dialogs.confirm` raised for the main window: the sheet asks it, with
    /// the request box when `request` (the caller's live ticket and its
    /// request) is given. One at a time: `dialogs.confirm` refuses (`busy`)
    /// while any sheet waits.
    pub(crate) fn open_confirm(
        &mut self,
        reply: Reply,
        options: confirm::Options,
        request: Option<(String, String)>,
    ) {
        // Nothing of the texts: they may name the user's file.
        diag!(
            "review sheet: {} asks to confirm",
            self.core.plugin_id(reply.owner)
        );
        self.open_sheet(Review {
            reply,
            checkbox: options.checkbox.clone().unwrap_or_default(),
            asks: Asks::Confirm { options, request },
            checked: false,
            shown_at: None,
            hinted: false,
        });
    }
    fn open_sheet(&mut self, review: Review) {
        // A sheet left by a caller that stopped (gone at the next tick) is
        // replaced; a waiting one never is.
        debug_assert!(!self.sheet_waiting(), "one sheet at a time");
        self.review = Some(review);
        self.present_review();
    }
    /// A sheet waits for the user's answer (its caller did not stop): no
    /// other one may ask now.
    pub(crate) fn sheet_waiting(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(|review| match review.asks {
                Asks::Run { .. } => self.core.command_runs.waiting(&review.reply),
                Asks::Confirm { .. } => true,
            })
    }
    /// The sheet as the core reads it (`UiFacts::sheet`, plan step
    /// S2f(1)): its request, and whether it is a confirmation.
    pub(crate) fn sheet_fact(&self) -> Option<messages::SheetFact> {
        self.review.as_ref().map(|review| messages::SheetFact {
            owner: review.reply.owner,
            id: review.reply.id,
            confirm: matches!(review.asks, Asks::Confirm { .. }),
        })
    }
    /// `owner` stopped: its confirmation goes unanswered (a plan's goes
    /// with `CommandRuns::revoke`, at the next tick).
    pub(crate) fn drop_confirm_of(&mut self, owner: u64) {
        if self
            .review
            .as_ref()
            .is_some_and(|review| review.reply.owner == owner && review.reports_checkbox())
        {
            self.end_review(None);
        }
    }
    /// The sheet is on screen in the main window.
    pub(crate) fn review_presented(&self) -> bool {
        self.review.as_ref().is_some_and(|r| r.shown_at.is_some())
            && self.menu_open
            && self.menu_mode == MenuMode::Review
            && self.menu_generation == self.shell_generation
    }
    /// Show the waiting sheet on the main window's menu layer, replacing
    /// any open menu or palette, unless a native dialog is open or there is
    /// no window yet. Its guard starts now.
    pub(crate) fn present_review(&mut self) {
        if self.review.is_none() || self.core.dialog_busy || self.review_presented() {
            return;
        }
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        if self.menu_open {
            self.close_menu();
        }
        #[cfg(feature = "producer-audio")]
        self.core
            .physical_revoke(producer_input::RevokeReason::Menu);
        let Some(value) = self.review_value() else {
            return;
        };
        let checked = self.review.as_ref().is_some_and(|review| review.checked);
        self.menu_open = true;
        self.menu_generation = self.shell_generation;
        self.menu_mode = MenuMode::Review;
        let result = (|| -> Result<()> {
            shell.set_property("review-sheet", value)?;
            // Set with every value: a new sheet starts unchecked.
            shell.set_property("review-remember-checked", checked.into())?;
            shell.set_property("palette-open", false.into())?;
            shell.set_property("review-open", true.into())?;
            shell.set_property("menu-open", true.into())?;
            Ok(())
        })();
        self.report_menu(result);
        let now = self.core.now_ms();
        let first = self.review.as_mut().is_some_and(|review| {
            review.shown_at = Some(now);
            !std::mem::replace(&mut review.hinted, true)
        });
        input_trace::follow("review sheet: shown");
        // The first time it shows, the main window comes into view: to the
        // front, un-minimized, while the player is in the foreground, and
        // asking for attention (the taskbar button flashes, the Dock icon
        // bounces) otherwise. The sheet is in the main window only; a
        // minimized or covered one would wait unseen.
        if first {
            crate::ui::view::show_again(&shell, crate::platform::app_in_foreground());
        }
        // The status strip yields to it.
        self.refresh_strip();
    }
    /// The waiting sheet's `ReviewSheet` value, with the request box's
    /// image.
    fn review_value(&mut self) -> Option<SValue> {
        let review = self.review.as_ref()?;
        let owner = review.reply.owner;
        // The request box: the lead's icon and title when the caller holds
        // it (always for a plan, today: only the lead mints tickets), else
        // its name.
        let (title, icon) = match self.core.lead.as_ref().filter(|lead| lead.owner == owner) {
            Some(lead) => (lead.row.title.clone(), lead.row.icon.clone()),
            None => (
                crate::capabilities::shown_name(&self.core, owner),
                commands::ButtonIcon::Name("ai".into()),
            ),
        };
        let icon_name = match &icon {
            commands::ButtonIcon::Name(name) => name.as_str(),
            commands::ButtonIcon::Image { .. } => "",
        };
        let keys = command_palette::footer_keys();
        let keys = [keys[0].as_str(), keys[2].as_str()];
        let (value, request) = match &review.asks {
            Asks::Run { query, steps, .. } => (
                sheet(&title, icon_name, query, steps, &review.checkbox, keys),
                true,
            ),
            Asks::Confirm { options, request } => (
                confirm_sheet(
                    request
                        .as_ref()
                        .map(|(_, query)| (title.as_str(), icon_name, query.as_str())),
                    options,
                    keys,
                ),
                request.is_some(),
            ),
        };
        let image = if request {
            self.lead_image(owner, &icon)
        } else {
            None
        };
        let mut value = match ui_values::from_json(&value) {
            Ok(value) => value,
            Err(error) => {
                diag!("host.error review sheet: {error:#}");
                return None;
            }
        };
        if let SValue::Struct(fields) = &mut value {
            fields.set_field("image".into(), SValue::Image(image.unwrap_or_default()));
        }
        Some(value)
    }
    /// What a waiting plan's "Don't ask again" would trust lost a plugin
    /// (uninstalled, replaced from another source, Ask Again Before Plugin
    /// Commands): its checkbox names only the plugins left, and goes,
    /// unchecked, when none is. A sheet on screen is written again, and
    /// its guard starts again (what it offers changed under the pointer).
    pub(crate) fn review_offer_changed(&mut self) {
        let Some(review) = &self.review else {
            return;
        };
        if !matches!(review.asks, Asks::Run { .. }) {
            return;
        }
        let Some((text, checked)) = self
            .core
            .command_runs
            .offered(&review.reply)
            .and_then(|names| offer_after(names, &review.checkbox, review.checked))
        else {
            return;
        };
        let presented = self.review_presented();
        if let Some(review) = self.review.as_mut() {
            review.checkbox = text;
            review.checked = checked;
        }
        if !presented {
            return;
        }
        let (Some(shell), Some(value)) = (
            self.shell.as_ref().map(ComponentHandle::clone_strong),
            self.review_value(),
        ) else {
            return;
        };
        let checked = self.review.as_ref().is_some_and(|review| review.checked);
        let result = (|| -> Result<()> {
            shell.set_property("review-sheet", value)?;
            shell.set_property("review-remember-checked", checked.into())?;
            Ok(())
        })();
        self.report_menu(result);
        let now = self.core.now_ms();
        if let Some(review) = self.review.as_mut() {
            review.shown_at = Some(now);
        }
    }
    /// The menu layer closed or changed under the sheet (`close_menu`,
    /// `open_menu`): it shows again at the next tick.
    pub(crate) fn review_hidden(&mut self) {
        if let Some(review) = self.review.as_mut() {
            review.shown_at = None;
        }
    }
    /// Every window tick: the sheet goes when its caller stopped, answers
    /// Don't Run (Cancel) when its ticket ended (120 s unused,
    /// `palette.lead.done`), steps aside while a native dialog is open and
    /// shows again otherwise.
    pub(crate) fn review_tick(&mut self, now: u64) {
        let Some(review) = &self.review else {
            return;
        };
        if !self.sheet_waiting() {
            self.end_review(None);
            return;
        }
        if review
            .ticket()
            .is_some_and(|ticket| !self.core.command_runs.is_live(ticket, now))
        {
            diag!("review sheet: the ticket ended: Don't Run");
            let checked = review.reports_checkbox() && review.checked;
            self.end_review(Some((false, checked)));
            return;
        }
        if self.core.dialog_busy {
            if self.review_presented() {
                self.close_menu();
            }
            return;
        }
        self.present_review();
    }
    /// A key or press on the sheet (`review-sheet-action`, a press outside
    /// it), as [`decide`] takes it.
    pub(crate) fn review_action(&mut self, action: &str) {
        if !self.review_presented() {
            return;
        }
        let Some(review) = self.review.as_ref() else {
            return;
        };
        let decision = decide(
            action,
            review.shown_at.unwrap_or(0),
            self.core.now_ms(),
            !review.checkbox.is_empty(),
            review.checked,
            review.reports_checkbox(),
        );
        match decision {
            Decision::Ignore => {
                input_trace::follow(&format!("review sheet: {action} ignored"));
            }
            Decision::Toggle => {
                let checked = !review.checked;
                if let Some(review) = self.review.as_mut() {
                    review.checked = checked;
                }
                if let Some(shell) = &self.shell {
                    let _ = shell.set_property("review-remember-checked", checked.into());
                }
            }
            Decision::Answer(yes, checked) => self.end_review(Some((yes, checked))),
        }
    }
    /// The sheet goes, with the user's answer, (yes, checked) (`None`:
    /// nobody waits for it).
    fn end_review(&mut self, answer: Option<(bool, bool)>) {
        let Some(review) = self.review.take() else {
            return;
        };
        if self.menu_open && self.menu_mode == MenuMode::Review {
            self.close_menu();
        }
        let Some((yes, checked)) = answer else {
            return;
        };
        input_trace::follow(&format!("review sheet: answered {yes} (checked {checked})"));
        match review.asks {
            Asks::Run { .. } => {
                let _ =
                    self.core
                        .events
                        .send(Event::CommandRunAnswered(review.reply, yes, checked));
            }
            Asks::Confirm { .. } => {
                // As `Event::ConfirmDone` (the system dialog's answer) and
                // `Event::CommandRunAnswered` (a plan's) do.
                self.policy.activity(self.core.now_ms());
                self.ui_report(messages::UiReport::Answered {
                    reply: review.reply,
                    result: Ok(json!({"confirmed": yes, "checked": checked})),
                    handoff: None,
                    confirm_failed: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(label: &str, plugin: &str, reason: &str, asks: bool, builtin: bool) -> Step {
        Step {
            label: label.into(),
            plugin: plugin.into(),
            reason: reason.into(),
            asks,
            builtin,
        }
    }

    #[test]
    fn input_counts_half_a_second_after_the_sheet_shows() {
        assert!(!accepts(1_000, 1_000));
        assert!(!accepts(1_000, 1_499));
        assert!(accepts(1_000, 1_500));
        // A clock that went back is no reason to accept.
        assert!(!accepts(1_000, 900));
    }

    #[test]
    fn the_question_counts_the_commands() {
        assert_eq!(question(1), "Run this command?");
        assert_eq!(question(3), "Run these 3 commands?");
    }

    /// Every slot is there (hidden ones empty); the note only with a step
    /// of a plugin that is not built in.
    #[test]
    fn the_sheet_lists_every_step_in_fixed_slots() {
        let steps = [
            step("Reset: Picture", "Video", "", true, true),
            step("Vestra Player: Mute", "Vestra Player", "", false, true),
            step(
                "Export: Export Last 30 Seconds",
                "Clip Export",
                "Not built in",
                true,
                false,
            ),
        ];
        let value = sheet(
            "Ask Vestra",
            "ai",
            "  reset, mute, export ",
            &steps,
            "",
            ["Enter", "Esc"],
        );
        assert_eq!(value["question"], "Run these 3 commands?");
        assert_eq!(value["remember"], "");
        assert_eq!(value["query"], "reset, mute, export");
        assert_eq!(value["count"], 3);
        let rows = value["steps"].as_array().unwrap();
        assert_eq!(rows.len(), SLOTS);
        assert_eq!(rows[0]["number"], "1");
        assert_eq!(rows[0]["reason"], "");
        assert_eq!(rows[1]["asks"], false);
        assert_eq!(rows[2]["reason"], "Not built in");
        assert!(rows[3..].iter().all(|row| row["shown"] == false));
        assert_eq!(
            value["note"],
            "The player cannot tell what commands of plugins that are not built in will do."
        );
        assert_eq!(
            (&value["run"], &value["cancel"]),
            (&json!("Run"), &json!("Don't Run"))
        );
        assert_eq!(
            (&value["run-key"], &value["cancel-key"]),
            (&json!("Enter"), &json!("Esc"))
        );
        let one = sheet("Ask Vestra", "ai", "", &steps[..1], "", ["Enter", "Esc"]);
        assert_eq!(one["question"], "Run this command?");
        assert_eq!(one["note"], "");
        let export = sheet(
            "Ask Vestra",
            "ai",
            "",
            &steps[2..],
            &remember_text(&["Clip Export".into()]),
            ["Enter", "Esc"],
        );
        assert_eq!(export["remember"], "Don't ask again for Clip Export");
    }

    /// Review fix 1: the box too counts for nothing within the guard; Run
    /// remembers only when it is offered and checked, Ctrl+Enter when it
    /// is offered; a toggle without a box is nothing.
    #[test]
    fn the_guard_covers_the_checkbox_and_run_follows_the_hosts_box() {
        use Decision::*;
        assert_eq!(plan("toggle-remember", 1_000, 1_200, true, false), Ignore);
        assert_eq!(plan("run-remember", 1_000, 1_499, true, false), Ignore);
        assert_eq!(plan("toggle-remember", 1_000, 1_500, true, false), Toggle);
        assert_eq!(plan("toggle-remember", 1_000, 1_500, false, false), Ignore);
        assert_eq!(plan("run", 1_000, 2_000, true, false), Answer(true, false));
        assert_eq!(plan("run", 1_000, 2_000, true, true), Answer(true, true));
        assert_eq!(plan("run", 1_000, 2_000, false, true), Answer(true, false));
        assert_eq!(
            plan("run-remember", 1_000, 2_000, true, false),
            Answer(true, true)
        );
        assert_eq!(
            plan("run-remember", 1_000, 2_000, false, false),
            Answer(true, false)
        );
        assert_eq!(
            plan("decline", 1_000, 2_000, true, true),
            Answer(false, false)
        );
        assert_eq!(plan("other", 1_000, 2_000, true, true), Ignore);
    }

    /// PROPOSAL §4.5: one or two plugins by name, more by count.
    #[test]
    fn dont_ask_again_names_one_or_two_plugins_and_counts_more() {
        let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(remember_text(&[]), "");
        assert_eq!(
            remember_text(&names(&["Clip Export"])),
            "Don't ask again for Clip Export"
        );
        assert_eq!(
            remember_text(&names(&["Clip Export", "Auto Frame"])),
            "Don't ask again for Clip Export and Auto Frame"
        );
        assert_eq!(
            remember_text(&names(&["Clip Export", "Auto Frame", "VestraTube"])),
            "Don't ask again for these 3 plugins"
        );
    }

    /// T5 (step 4 review follow-up): a sheet whose "Don't ask again" list
    /// lost a plugin (uninstalled while it waited) names only the rest, and
    /// one whose list was emptied shows no checkbox (and nothing checked).
    #[test]
    fn the_checkbox_follows_what_is_left_to_trust() {
        let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let two = remember_text(&names(&["Clip Export", "Auto Frame"]));
        assert_eq!(
            offer_after(&names(&["Clip Export", "Auto Frame"]), &two, true),
            None
        );
        assert_eq!(
            offer_after(&names(&["Auto Frame"]), &two, true),
            Some(("Don't ask again for Auto Frame".into(), true))
        );
        assert_eq!(offer_after(&[], &two, true), Some((String::new(), false)));
        assert_eq!(offer_after(&[], "", false), None);
    }

    /// A plan's decision ([`decide`] with its own "Don't ask again").
    fn plan(action: &str, shown_at: u64, now: u64, offered: bool, checked: bool) -> Decision {
        decide(action, shown_at, now, offered, checked, false)
    }

    /// H5: a confirmation's box is the caller's: its state goes with
    /// either answer, Ctrl+Enter is a plain Enter, and the guard holds.
    #[test]
    fn a_confirmations_checkbox_is_reported_with_either_answer() {
        use Decision::*;
        let confirm =
            |action, now, offered, checked| decide(action, 1_000, now, offered, checked, true);
        assert_eq!(confirm("run", 1_499, true, true), Ignore);
        assert_eq!(confirm("toggle-remember", 1_200, true, false), Ignore);
        assert_eq!(confirm("toggle-remember", 1_500, true, false), Toggle);
        assert_eq!(confirm("toggle-remember", 1_500, false, false), Ignore);
        assert_eq!(confirm("run", 2_000, true, true), Answer(true, true));
        assert_eq!(confirm("run", 2_000, true, false), Answer(true, false));
        assert_eq!(
            confirm("run-remember", 2_000, true, false),
            Answer(true, false)
        );
        assert_eq!(
            confirm("run-remember", 2_000, true, true),
            Answer(true, true)
        );
        assert_eq!(confirm("decline", 2_000, true, true), Answer(false, true));
        assert_eq!(confirm("decline", 2_000, false, true), Answer(false, false));
    }

    /// H5: a confirmation shows the caller's texts and nothing of a plan;
    /// the request box only with a request.
    #[test]
    fn a_confirmation_sheet_carries_the_callers_texts() {
        let options = confirm::Options {
            message: "Clear the playlist?".into(),
            detail: Some("All 12 entries are removed.".into()),
            confirm: "Clear".into(),
            cancel: "Cancel".into(),
            destructive: true,
            checkbox: Some("Also clear the history".into()),
        };
        let plain = confirm_sheet(None, &options, ["Enter", "Esc"]);
        assert_eq!(plain["request"], false);
        assert_eq!(plain["title"], "");
        assert_eq!(plain["message"], "Clear the playlist?");
        assert_eq!(plain["detail"], "All 12 entries are removed.");
        assert_eq!(plain["remember"], "Also clear the history");
        assert_eq!(
            (&plain["run"], &plain["cancel"]),
            (&json!("Clear"), &json!("Cancel"))
        );
        assert_eq!(
            (&plain["question"], &plain["note"], &plain["count"]),
            (&json!(""), &json!(""), &json!(0))
        );
        let rows = plain["steps"].as_array().unwrap();
        assert_eq!(rows.len(), SLOTS);
        assert!(rows.iter().all(|row| row["shown"] == false));
        let consent = confirm_sheet(
            Some(("Ask Vestra", "ai", " mute ")),
            &confirm::Options {
                detail: None,
                checkbox: None,
                ..options
            },
            ["Enter", "Esc"],
        );
        assert_eq!(consent["request"], true);
        assert_eq!(
            (&consent["title"], &consent["query"]),
            (&json!("Ask Vestra"), &json!("mute"))
        );
        assert_eq!(
            (&consent["detail"], &consent["remember"]),
            (&json!(""), &json!(""))
        );
        // A plan's sheet has no message: the view draws its question.
        let plan = sheet("Ask Vestra", "ai", "x", &[], "", ["Enter", "Esc"]);
        assert_eq!(
            (&plan["request"], &plan["message"]),
            (&json!(true), &json!(""))
        );
    }
}
