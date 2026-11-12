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
//! - It is modal: Enter or Run runs the plan, Esc, Don't Run or a press
//!   outside it runs nothing; keys and presses within [`GUARD_MS`] of it
//!   showing count for nothing (a carried-over Enter), and so do held-down
//!   repeats (the view).
//! - It waits while a native dialog is open (`dialog_busy`), and shows again
//!   when something else closed the menu layer (a rebuilt shell, another
//!   menu): only an answer, the ticket's end (Don't Run) or the caller's end
//!   removes it. It does not take the focus from another program.
//! - The answer goes the usual way (`Event::CommandRunAnswered`).
//! - `"Don't ask again for <plugin>"` (step 4, H4) sits at the footer's left
//!   when the plan's steps that ask include some only because their plugin
//!   is not built in and no step is destructive (user ruling 3 of
//!   2026-11-12); Run with it checked, or Ctrl+Enter, runs and remembers.
//!   What is remembered is the host's list (`ConfirmedRun::remember`); the
//!   view only says whether the box was checked.
//!
//! doc/architecture/palette-lead.md §2.1.
use super::*;
use crate::ui::command_palette;

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

/// A sheet waiting for the user's answer.
pub(crate) struct Review {
    /// The waiting `commands.run`'s reply (`CommandRuns::confirming`).
    pub(crate) reply: Reply,
    pub(crate) ticket: String,
    /// The request the ticket was minted for.
    pub(crate) query: String,
    pub(crate) steps: Vec<Step>,
    /// The "Don't ask again" checkbox's text; empty: no checkbox.
    pub(crate) remember: String,
    /// When it last showed (`Host::now_ms`); `None` while it waits to show.
    pub(crate) shown_at: Option<u64>,
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

/// The `ReviewSheet` value (ui/review-sheet.slint), less its image: `title`
/// and `icon` of the request box, the request, the steps in [`SLOTS`] rows,
/// the note when a step's plugin is not built in, the "Don't ask again"
/// text (empty: no checkbox), and the buttons' texts and keys.
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
            None => json!({"shown": false, "number": "", "label": "", "plugin": "",
                "reason": "", "asks": false}),
        })
        .collect();
    let note = if steps.iter().any(|step| !step.builtin) {
        i18n::tr("The player cannot tell what commands of plugins that are not built in will do.")
    } else {
        String::new()
    };
    json!({
        "icon": icon,
        "title": title,
        "query": query.trim(),
        "question": question(steps.len()),
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

impl Host {
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
        // A sheet left by a caller that stopped (gone at the next tick) is
        // replaced; a waiting one never is.
        debug_assert!(
            self.review
                .as_ref()
                .is_none_or(|old| !self.command_runs.waiting(&old.reply)),
            "one approval sheet at a time"
        );
        self.review = Some(Review {
            reply,
            ticket,
            query,
            steps,
            remember,
            shown_at: None,
        });
        self.present_review();
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
        if self.review.is_none() || self.dialog_busy || self.review_presented() {
            return;
        }
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        if self.menu_open {
            self.close_menu();
        }
        #[cfg(feature = "producer-audio")]
        self.physical_revoke(producer_input::RevokeReason::Menu);
        let Some(review) = &self.review else {
            return;
        };
        let owner = review.reply.owner;
        // The request box: the lead's icon and title when the caller holds
        // it (always, today: only the lead mints tickets), else its name.
        let (title, icon) = match self.lead.as_ref().filter(|lead| lead.owner == owner) {
            Some(lead) => (lead.row.title.clone(), lead.row.icon.clone()),
            None => (
                crate::capabilities::shown_name(self, owner),
                commands::ButtonIcon::Name("ai".into()),
            ),
        };
        let keys = command_palette::footer_keys();
        let value = sheet(
            &title,
            match &icon {
                commands::ButtonIcon::Name(name) => name,
                commands::ButtonIcon::Image { .. } => "",
            },
            &review.query,
            &review.steps,
            &review.remember,
            [&keys[0], &keys[2]],
        );
        let image = self.lead_image(owner, &icon);
        self.menu_open = true;
        self.menu_generation = self.shell_generation;
        self.menu_mode = MenuMode::Review;
        let result = (|| -> Result<()> {
            let mut value = ui_values::from_json(&value)?;
            if let SValue::Struct(fields) = &mut value {
                fields.set_field("image".into(), SValue::Image(image.unwrap_or_default()));
            }
            shell.set_property("review-sheet", value)?;
            shell.set_property("palette-open", false.into())?;
            shell.set_property("review-open", true.into())?;
            shell.set_property("menu-open", true.into())?;
            Ok(())
        })();
        self.report_menu(result);
        let now = self.now_ms();
        if let Some(review) = self.review.as_mut() {
            review.shown_at = Some(now);
        }
        input_trace::follow("review sheet: shown");
        // The status strip yields to it.
        self.refresh_strip();
    }
    /// The menu layer closed or changed under the sheet (`close_menu`,
    /// `open_menu`): it shows again at the next tick.
    pub(crate) fn review_hidden(&mut self) {
        if let Some(review) = self.review.as_mut() {
            review.shown_at = None;
        }
    }
    /// Every window tick: the sheet goes when its caller stopped, answers
    /// Don't Run when its ticket ended (120 s unused, `palette.lead.done`),
    /// steps aside while a native dialog is open and shows again otherwise.
    pub(crate) fn review_tick(&mut self, now: u64) {
        let Some(review) = &self.review else {
            return;
        };
        if !self.command_runs.waiting(&review.reply) {
            self.end_review(None);
            return;
        }
        if !self.command_runs.is_live(&review.ticket, now) {
            diag!("review sheet: the ticket ended: Don't Run");
            self.end_review(Some((false, false)));
            return;
        }
        if self.dialog_busy {
            if self.review_presented() {
                self.close_menu();
            }
            return;
        }
        self.present_review();
    }
    /// A key or press on the sheet (`review-sheet-action`, a press outside
    /// it): `run`, `run-remember` (Run with "Don't ask again" checked, or
    /// Ctrl+Enter; plain Run when the sheet offers no checkbox) or
    /// `decline`, ignored within the guard.
    pub(crate) fn review_action(&mut self, action: &str) {
        if !self.review_presented() {
            return;
        }
        let shown_at = self.review.as_ref().and_then(|r| r.shown_at).unwrap_or(0);
        if !accepts(shown_at, self.now_ms()) {
            input_trace::follow(&format!("review sheet: {action} within the guard, ignored"));
            return;
        }
        match action {
            "run" => self.end_review(Some((true, false))),
            "run-remember" => {
                let offered = self.review.as_ref().is_some_and(|r| !r.remember.is_empty());
                self.end_review(Some((true, offered)));
            }
            "decline" => self.end_review(Some((false, false))),
            _ => {}
        }
    }
    /// The sheet goes, with the user's answer, (run, remember) (`None`:
    /// nobody waits for it).
    fn end_review(&mut self, answer: Option<(bool, bool)>) {
        let Some(review) = self.review.take() else {
            return;
        };
        if self.menu_open && self.menu_mode == MenuMode::Review {
            self.close_menu();
        }
        if let Some((yes, remember)) = answer {
            input_trace::follow(&format!(
                "review sheet: answered {yes} (remember {remember})"
            ));
            let _ = self
                .events
                .send(Event::CommandRunAnswered(review.reply, yes, remember));
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
}
