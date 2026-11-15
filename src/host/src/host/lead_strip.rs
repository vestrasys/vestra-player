//! The status strip (Ask Vestra UX step 2, H1/H2): the palette lead's
//! status while the palette is closed, drawn by the host where the palette
//! was (top centre, `ui/lead-strip.slint`), in the palette's style, without
//! the keyboard focus.
//!
//! What it shows, first match wins:
//!
//! 1. an end state the holder set (`status.state` done, failed, note or
//!    stopped, the last since SDK 1.8.0):
//!    once, for its time ([`end_ms`]), longer while the pointer is on it
//!    (the OSD's hold, `osd_hold`); never kept on the row or replayed;
//! 2. the step `commands.run` last ran for the lead's holder with its live
//!    ticket ("2/3 · Vestra Player: Mute", the line filled to 2/3): from the
//!    first step on, the holder's working text only changes the palette's
//!    status row, until an end state, `status: null` or the ticket's end;
//! 3. the holder's working status (the line sweeps; Stop): its `strip` text
//!    when it gave one (SDK 1.8.0), else its `text`.
//!
//! It yields to an open menu or palette (the palette's first row shows the
//! working status then) and comes back when it closes. Stop sends the
//! `palette.lead` event as Enter on the status row does; a click elsewhere
//! opens the palette, or dismisses an end state.
//! doc/architecture/palette-lead.md §3.
use super::*;
use crate::capabilities::{LeadStatus, StatusState};

/// How long a done status shows (then the 300 ms fade, as the OSD's).
pub(crate) const DONE_MS: u64 = 2000;
/// How long a failed status shows.
pub(crate) const FAILED_MS: u64 = 6000;
/// A note (a sentence for the user) shows this long, plus [`NOTE_CHAR_MS`]
/// a character, at most [`NOTE_MAX_MS`].
pub(crate) const NOTE_MS: u64 = 4000;
pub(crate) const NOTE_CHAR_MS: u64 = 50;
pub(crate) const NOTE_MAX_MS: u64 = 10_000;
/// How long a stopped status shows: the user stopped the work themselves,
/// so it only confirms the press.
pub(crate) const STOPPED_MS: u64 = 2000;
/// How long a reading of the system's animation setting is kept.
const ANIMATIONS_READ_MS: u64 = 1000;

/// How long an end state shows before it fades.
pub(crate) fn end_ms(state: StatusState, text: &str) -> u64 {
    match state {
        StatusState::Working => 0,
        StatusState::Done => DONE_MS,
        StatusState::Failed => FAILED_MS,
        StatusState::Note => {
            (NOTE_MS + NOTE_CHAR_MS * text.chars().count() as u64).min(NOTE_MAX_MS)
        }
        StatusState::Stopped => STOPPED_MS,
    }
}

/// What the strip draws (`LeadStrip` in ui/lead-strip.slint).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The holder's working text: the line sweeps.
    #[default]
    Working,
    /// A step `commands.run` ran: the line filled to `progress`.
    Running,
    Done,
    Failed,
    Note,
    /// The user stopped the work: plain, the icon dimmed, no line.
    Stopped,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Note => "note",
            Self::Stopped => "stopped",
        }
    }
    /// An end state: no Stop; a click dismisses; up to two lines.
    fn ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Note | Self::Stopped)
    }
}

/// A step `commands.run` showed for the lead's holder.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub(crate) ticket: String,
    pub(crate) text: String,
    /// step / of (1 for a lone command).
    pub(crate) progress: f32,
}

/// An end state on screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ended {
    pub(crate) text: String,
    pub(crate) state: StatusState,
    pub(crate) shown_at: u64,
    pub(crate) until: u64,
}

/// The strip's state: its sources and what the shell shows.
#[derive(Debug, Default)]
pub(crate) struct LeadStrip {
    pub(crate) step: Option<Step>,
    pub(crate) ended: Option<Ended>,
    /// As last written to the shell (again into a rebuilt one).
    pub(crate) shown: Shown,
    /// The system's animation setting (`platform::system_animations`) and
    /// when it was read: read again at most once a second, only while the
    /// strip would sweep.
    pub(crate) animations: Option<(u64, bool)>,
}

/// What the strip shows, as written to the shell's `lead-strip`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    pub(crate) visible: bool,
    pub(crate) kind: Kind,
    pub(crate) text: String,
    /// The line's fill, 0–1 (`Running`; 1 for done and failed).
    pub(crate) progress: f32,
    /// The system lets nothing animate (Windows' Animation effects off,
    /// macOS' Reduce motion): a working line rests in the middle instead
    /// of sweeping (PROPOSAL §4.7).
    pub(crate) still: bool,
}

impl Shown {
    fn value(&self) -> Result<SValue> {
        ui_values::from_json(&json!({
            "visible": self.visible,
            "kind": self.kind.name(),
            "text": self.text,
            "progress": self.progress,
            "stop": !self.kind.ended(),
            "still": self.still,
        }))
    }
}

/// The strip for these sources (first match wins, see the module); hidden
/// without any, or while `covered` (a menu or the palette is open), keeping
/// the last content so it fades out as it was. `animations`: whether the
/// system lets the interface animate (asked only for a working strip, the
/// one that sweeps).
pub(crate) fn project(
    working: Option<&str>,
    step: Option<&Step>,
    ended: Option<&Ended>,
    covered: bool,
    last: &Shown,
    animations: impl FnOnce() -> bool,
) -> Shown {
    let content = if let Some(ended) = ended {
        let (kind, progress) = match ended.state {
            StatusState::Done => (Kind::Done, 1.),
            StatusState::Failed => (Kind::Failed, 1.),
            StatusState::Stopped => (Kind::Stopped, 0.),
            _ => (Kind::Note, 0.),
        };
        Some((kind, ended.text.clone(), progress))
    } else if let Some(step) = step {
        Some((Kind::Running, step.text.clone(), step.progress))
    } else {
        working.map(|text| (Kind::Working, text.to_owned(), 0.))
    };
    match content {
        Some((kind, text, progress)) => Shown {
            visible: !covered,
            kind,
            text,
            progress,
            still: kind == Kind::Working && !covered && !animations(),
        },
        None => Shown {
            visible: false,
            ..last.clone()
        },
    }
}

impl Host {
    /// The holder's working status as the strip shows it, if it has one:
    /// its `strip` text when it gave one (SDK 1.8.0), else its `text` (the
    /// palette's row always shows `text`).
    fn lead_working(&self) -> Option<&str> {
        self.core.lead.as_ref()?.row.working_strip()
    }
    /// The holder set an end state (`palette.lead.set`): the strip shows it
    /// once, in place of any step.
    pub(crate) fn lead_ended(&mut self, status: &LeadStatus) {
        // The state only: the text may quote the user's request.
        diag!("lead strip: {}", status.state.name());
        let now = self.core.now_ms();
        self.lead_strip.step = None;
        self.lead_strip.ended = Some(Ended {
            text: status.text.clone(),
            state: status.state,
            shown_at: now,
            until: now + end_ms(status.state, &status.text),
        });
    }
    /// `commands.run` ran a step for the lead's holder with `ticket`.
    pub(crate) fn lead_step(&mut self, ticket: &str, text: &str, progress: f32) {
        self.lead_strip.ended = None;
        self.lead_strip.step = Some(Step {
            ticket: ticket.to_owned(),
            text: text.to_owned(),
            progress: progress.clamp(0., 1.),
        });
        self.refresh_strip();
    }
    /// The lead was set, changed or cleared: a working status replaces an
    /// end state still on screen (a new request); no status or no lead ends
    /// the steps.
    pub(crate) fn lead_strip_follow(&mut self) {
        let status = self
            .core
            .lead
            .as_ref()
            .and_then(|lead| lead.row.status.as_ref());
        match status {
            Some(status) if status.state == StatusState::Working => {
                if self.lead_strip.step.is_none() {
                    self.lead_strip.ended = None;
                }
            }
            _ => self.lead_strip.step = None,
        }
        self.refresh_strip();
    }
    /// Write the strip into the shell if it changed.
    pub(crate) fn refresh_strip(&mut self) {
        let covered = self.menu_open && self.menu_generation == self.shell_generation;
        let now = self.core.now_ms();
        let cached = self.lead_strip.animations;
        let mut read = None;
        let next = project(
            self.lead_working(),
            self.lead_strip.step.as_ref(),
            self.lead_strip.ended.as_ref(),
            covered,
            &self.lead_strip.shown,
            || match cached {
                Some((at, on)) if now.saturating_sub(at) < ANIMATIONS_READ_MS => on,
                _ => *read.insert(crate::platform::system_animations()),
            },
        );
        if let Some(on) = read {
            self.lead_strip.animations = Some((now, on));
        }
        if next != self.lead_strip.shown {
            self.lead_strip.shown = next;
            self.write_strip();
        }
    }
    /// Write the whole strip into the shell, after a change.
    pub(crate) fn write_strip(&self) {
        let Some(shell) = &self.shell else {
            return;
        };
        let result = self
            .lead_strip
            .shown
            .value()
            .and_then(|value| Ok(shell.set_property("lead-strip", value)?));
        if let Err(error) = result {
            diag!("host.error lead strip: {error:#}");
        }
    }
    /// Every window tick: a step whose ticket ended goes; an end state whose
    /// time is up fades (held while the active pointer is on it, at most
    /// `OSD_HOLD_MS`); the strip yields to a menu.
    pub(crate) fn lead_strip_tick(&mut self, now: u64, hover: bool) {
        if self
            .lead_strip
            .step
            .as_ref()
            .is_some_and(|step| !self.core.command_runs.is_live(&step.ticket, now))
        {
            self.lead_strip.step = None;
        }
        if let Some(ended) = &self.lead_strip.ended
            && now >= ended.until
        {
            let shown_at = ended.shown_at;
            match osd_hold(now, shown_at, true, hover, self.policy.idle(now)) {
                Some(until) => {
                    if let Some(ended) = self.lead_strip.ended.as_mut() {
                        ended.until = until;
                    }
                }
                None => self.lead_strip.ended = None,
            }
        }
        self.refresh_strip();
    }
    /// A press on the strip (`lead-strip-action`): `stop` (its Stop
    /// button), `open` (elsewhere on a working strip), `dismiss` (an end
    /// state).
    pub(crate) fn lead_strip_action(&mut self, action: &str) {
        // A press during the fade-out is no press.
        if !self.lead_strip.shown.visible {
            return;
        }
        match action {
            "stop" if !self.lead_strip.shown.kind.ended() => {
                input_trace::follow("lead strip: stop");
                self.lead_event(String::new(), false, true);
            }
            "open" => self.open_palette(),
            "dismiss" => {
                self.lead_strip.ended = None;
                self.refresh_strip();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(text: &str, progress: f32) -> Step {
        Step {
            ticket: "t".into(),
            text: text.into(),
            progress,
        }
    }

    fn ended(text: &str, state: StatusState) -> Ended {
        Ended {
            text: text.into(),
            state,
            shown_at: 0,
            until: end_ms(state, text),
        }
    }

    #[test]
    fn an_end_state_wins_over_a_step_and_a_step_over_the_working_text() {
        let none = Shown::default();
        let working = project(Some("Planning…"), None, None, false, &none, || true);
        assert_eq!(
            (working.visible, working.kind, working.text.as_str()),
            (true, Kind::Working, "Planning…")
        );
        let running = project(
            Some("Planning…"),
            Some(&step("2/3 · Mute", 2. / 3.)),
            None,
            false,
            &working,
            || true,
        );
        assert_eq!(
            (running.kind, running.text.as_str()),
            (Kind::Running, "2/3 · Mute")
        );
        assert!((running.progress - 2. / 3.).abs() < 1e-6);
        let done = project(
            Some("Planning…"),
            Some(&step("2/3 · Mute", 0.6)),
            Some(&ended("Done · Mute", StatusState::Done)),
            false,
            &running,
            || true,
        );
        assert_eq!((done.kind, done.progress), (Kind::Done, 1.));
        let failed = project(
            None,
            None,
            Some(&ended("Step 2 failed", StatusState::Failed)),
            false,
            &none,
            || true,
        );
        assert_eq!((failed.kind, failed.progress), (Kind::Failed, 1.));
        let note = project(
            None,
            None,
            Some(&ended("No command sets it", StatusState::Note)),
            false,
            &none,
            || true,
        );
        assert_eq!((note.kind, note.progress), (Kind::Note, 0.));
    }

    #[test]
    fn it_hides_without_a_source_or_under_a_menu_keeping_its_content_for_the_fade() {
        let shown = project(
            Some("Planning…"),
            None,
            None,
            false,
            &Shown::default(),
            || true,
        );
        let covered = project(Some("Planning…"), None, None, true, &shown, || true);
        assert!(!covered.visible);
        assert_eq!(covered.text, "Planning…");
        let gone = project(None, None, None, false, &shown, || true);
        assert_eq!(
            gone,
            Shown {
                visible: false,
                ..shown
            }
        );
    }

    /// T5 (step 2 review follow-up; PROPOSAL §4.7): with the system's
    /// animations off the working line rests in the middle; the setting is
    /// asked only for a working strip that shows.
    #[test]
    fn a_working_strip_is_still_when_the_system_animates_nothing() {
        let none = Shown::default();
        let still = project(Some("Planning…"), None, None, false, &none, || false);
        assert!(still.visible && still.still);
        let moving = project(Some("Planning…"), None, None, false, &none, || true);
        assert!(!moving.still);
        let asked = std::cell::Cell::new(false);
        let ask = || {
            asked.set(true);
            false
        };
        let running = project(
            None,
            Some(&step("1/2 · Mute", 0.5)),
            None,
            false,
            &none,
            ask,
        );
        assert!(!running.still && !asked.get());
        let covered = project(Some("Planning…"), None, None, true, &still, || {
            asked.set(true);
            false
        });
        assert!(!covered.still && !asked.get());
        let SValue::Struct(value) = still.value().unwrap() else {
            panic!("a struct");
        };
        assert_eq!(value.get_field("still"), Some(&SValue::Bool(true)));
    }

    #[test]
    fn end_states_show_for_their_time() {
        assert_eq!(end_ms(StatusState::Done, "Done · Mute"), 2000);
        assert_eq!(end_ms(StatusState::Failed, "x"), 6000);
        assert_eq!(end_ms(StatusState::Note, ""), 4000);
        assert_eq!(end_ms(StatusState::Note, &"a".repeat(40)), 6000);
        // Characters, not bytes; at most 10 s.
        assert_eq!(end_ms(StatusState::Note, &"问".repeat(40)), 6000);
        assert_eq!(end_ms(StatusState::Note, &"a".repeat(200)), 10_000);
    }

    #[test]
    fn the_shell_value_has_stop_only_while_working_or_running() {
        for (kind, stop) in [
            (Kind::Working, true),
            (Kind::Running, true),
            (Kind::Done, false),
            (Kind::Failed, false),
            (Kind::Note, false),
        ] {
            let shown = Shown {
                visible: true,
                kind,
                text: "t".into(),
                progress: 0.5,
                still: false,
            };
            let SValue::Struct(value) = shown.value().unwrap() else {
                panic!("a struct");
            };
            assert_eq!(value.get_field("stop"), Some(&SValue::Bool(stop)));
            assert_eq!(
                value.get_field("kind"),
                Some(&SValue::String(kind.name().into()))
            );
        }
    }
}

/// SDK 1.8.0: the end state `stopped`.
#[cfg(test)]
mod tests_1_8 {
    use super::*;

    #[test]
    fn a_stopped_status_shows_plain_for_two_seconds_in_place_of_a_step() {
        let step = Step {
            ticket: "t".into(),
            text: "1/2 · Mute".into(),
            progress: 0.5,
        };
        let text = "Stopped: nothing was run";
        let ended = Ended {
            text: text.into(),
            state: StatusState::Stopped,
            shown_at: 0,
            until: end_ms(StatusState::Stopped, text),
        };
        let stopped = project(
            Some("Planning…"),
            Some(&step),
            Some(&ended),
            false,
            &Shown::default(),
            || true,
        );
        // Its own kind, no line, not sweeping.
        assert_eq!(
            (stopped.kind, stopped.progress, stopped.text.as_str()),
            (Kind::Stopped, 0., text)
        );
        assert!(stopped.visible && !stopped.still);
        // An end state: no Stop button.
        assert!(Kind::Stopped.ended());
        assert_eq!(Kind::Stopped.name(), "stopped");
        assert_eq!(StatusState::Stopped.name(), "stopped");
        // The user did it: 2 s whatever its length.
        assert_eq!(STOPPED_MS, 2000);
        assert_eq!(end_ms(StatusState::Stopped, "Stopped"), 2000);
        assert_eq!(end_ms(StatusState::Stopped, &"a".repeat(160)), 2000);
        // The other end states keep their kinds.
        for (state, kind) in [
            (StatusState::Done, Kind::Done),
            (StatusState::Failed, Kind::Failed),
            (StatusState::Note, Kind::Note),
        ] {
            let other = Ended {
                state,
                ..ended.clone()
            };
            let shown = project(None, None, Some(&other), false, &Shown::default(), || true);
            assert_eq!(shown.kind, kind);
        }
    }
}
