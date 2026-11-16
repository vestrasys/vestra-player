//! The core's outbox (plan step S2f(3); design §5.3, "After S2f"): what
//! the core asks of the interface, in the core's order, and the core work
//! that must follow the interface's part of those requests.
//!
//! The core pushes; it never carries a request out itself. The driver
//! drains the outbox after every handled event, after every report the
//! interface sends, and after every core step ([`Host::drain_outbox`]):
//! each request is carried out by the interface, whose reports the core
//! handles at once (their own requests drained right after, before the
//! next entry), and each [`Then`] is run by the core in its place. So on
//! one thread every request is carried out in the turn that issued it,
//! after the rest of the handler that issued it, and before the next event.
//!
//! Before plan step S2f(3) a request was carried out in place, in the
//! middle of the handler. Where today's wire showed a plugin line the
//! interface's part wrote (through its report) *before* a later line of the
//! same handler, that later line rides behind the request as a [`Then`]
//! ("the core's later line rides with the command", plan S2f(3)); the
//! sites are listed at [`Then`]. S3a decides each of them again (its
//! wire-trace class 3), and moves this queue between the threads.
use super::messages::{SettingsOp, UiCommand};
use super::*;

/// One entry of the outbox.
pub(crate) enum Out {
    /// A request of the core to the interface (rule R3).
    Ui(UiCommand),
    /// Core work that follows the interface's part of the requests before
    /// it.
    Then(Then),
}

/// Core work kept behind the requests issued before it (one thread only:
/// plan step S2f(3)). Each use keeps a wire order of today's:
pub(crate) enum Then {
    /// A reply after the lines the interface's part of a request before it
    /// writes: `commands.run` that ran a host command whose interface part
    /// lays the window out (`LayoutOp::Toggle` runs a window tick, whose
    /// `window.state` and video-area replies went out before the run's
    /// reply), at once or after the sheet's Run.
    Respond {
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
    },
    /// `settings.register` of a recorded page again (`SettingsOp::Updated`):
    /// the open window's `ui.error` lines come first (design §5.1 note 11),
    /// then the refused initial values' `ui.error` lines and the reply.
    Registered {
        reply: Reply,
        refused: Vec<RpcError>,
    },
    /// What a process said while its spawn was not answered yet
    /// (`spawned`): each event handled after the requests of the ones
    /// before it were carried out, as when each was handled alone.
    Event(Event),
    /// The supervisor's pending jobs, after the held events (`spawned`).
    SupervisorRun,
}

impl Core {
    /// A request to the interface (rule R3): into the outbox, carried out
    /// when the driver drains it (plan step S2f(3)).
    pub(crate) fn ui_command(&mut self, command: UiCommand) {
        self.outbox.push_back(Out::Ui(command));
    }
    /// Core work kept behind the requests issued so far ([`Then`]).
    pub(crate) fn then(&mut self, then: Then) {
        self.outbox.push_back(Out::Then(then));
    }
    /// A request waits in the outbox whose interface part writes a plugin
    /// line through its report ([`writes_lines`]): a line of the core's
    /// that followed it on today's wire rides behind it ([`Then`]).
    pub(crate) fn lines_pending(&self) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(command) if writes_lines(command)))
    }
    /// Debug builds: where core code reads its copy of the interface's
    /// facts `read`, no request that changes them waits in the outbox. The
    /// copy is the interface's as of the last drain, so it is then the
    /// interface's now, as when every request was carried out in place
    /// (plan step S2f(3)).
    #[cfg(debug_assertions)]
    pub(crate) fn check_settled(&self, read: &[Fact]) {
        for &fact in read {
            debug_assert!(
                !self.outbox.iter().any(|out| match out {
                    // The core's copy dropped that confirmation already
                    // (`UiDialog::revoke`): a drop that finds none changes
                    // nothing.
                    Out::Ui(UiCommand::DropConfirm { owner }) => {
                        matches!(fact, Fact::Sheet)
                            && self
                                .ui_facts
                                .sheet
                                .is_some_and(|sheet| sheet.owner == *owner && sheet.confirm)
                    }
                    Out::Ui(command) => changes(command, fact),
                    Out::Then(_) => false,
                }),
                "the core reads {fact:?} while a request that changes it waits"
            );
        }
    }
    /// Debug builds: a `DropConfirm` of `owner` waits in the outbox (the
    /// core's copy of the sheet is ahead of the interface's until it is
    /// carried out, `UiDialog::revoke`).
    #[cfg(debug_assertions)]
    pub(crate) fn drop_pending(&self, owner: u64) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(UiCommand::DropConfirm { owner: o }) if *o == owner))
    }
    /// Debug builds: a cancel of the batching window's change waits in the
    /// outbox (the core cleared its record already; the window drops it
    /// when the request is carried out).
    #[cfg(debug_assertions)]
    pub(crate) fn cancel_pending(&self) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(UiCommand::CancelMount(_))))
    }
}

/// Whether the interface's part of `command` writes a plugin line through
/// its report outside a failure of its own (a Slint call that fails): the
/// UI-bound replies (`UiReport::Answered`, with `layout.set`'s
/// `window.state`), the window tick a column toggle runs, the open options
/// window's `ui.error` lines of a page registered again.
pub(crate) fn writes_lines(command: &UiCommand) -> bool {
    use super::messages::{LayoutOp, WindowOp};
    matches!(
        command,
        UiCommand::View(_)
            | UiCommand::SetMount(_)
            | UiCommand::Settings(
                SettingsOp::Updated { .. } | SettingsOp::Set { .. } | SettingsOp::Show { .. }
            )
            | UiCommand::Layout(LayoutOp::Set { .. } | LayoutOp::Toggle(_))
            | UiCommand::Window(WindowOp::VideoArea { .. } | WindowOp::Pip { .. })
            | UiCommand::Confirm(_)
            | UiCommand::Ask(_)
    )
}

/// What core code reads of the interface's facts (`UiFacts`) and tabs
/// (`WindowFacts`), for [`Core::check_settled`].
#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fact {
    /// The approval sheet (`UiFacts::sheet`).
    Sheet,
    /// The options window open (`UiFacts::options_open`).
    Options,
    /// The columns' policy facts (`UiFacts::columns`).
    Columns,
    /// No video (`UiFacts::no_video`).
    NoVideo,
    /// Whether the OSD message's buttons are this plugin's
    /// (`UiFacts::osd_buttons`).
    OsdButtons(u64),
    /// The panel policy's tabs (`Core::panel_tabs`).
    Tabs,
    /// The floating window's panels (`UiFacts::floating`).
    Floating,
    /// Whether this plugin has a window of its own (`UiFacts::views`).
    Views(u64),
    /// Picture-in-picture offered and shown (`UiFacts::pip`).
    Pip,
    /// Whether this plugin's picture-in-picture request is in effect
    /// (`PipFact::holder`).
    PipHolder(u64),
}

/// Whether carrying out `command` can change `fact`.
#[cfg(debug_assertions)]
fn changes(command: &UiCommand, fact: Fact) -> bool {
    use super::messages::{UiSnapshot, WindowOp};
    let revokes =
        |plugin: u64| matches!(command, UiCommand::RevokeOwner { owner, .. } if *owner == plugin);
    match fact {
        Fact::Sheet => matches!(
            command,
            UiCommand::Review(_) | UiCommand::Ask(_) | UiCommand::DropConfirm { .. }
        ),
        Fact::Options => matches!(command, UiCommand::Settings(_)),
        Fact::Columns | Fact::Tabs | Fact::Floating => {
            matches!(command, UiCommand::Layout(_) | UiCommand::Panels(_))
        }
        Fact::NoVideo => matches!(command, UiCommand::Snapshot(UiSnapshot::Source(_))),
        Fact::OsdButtons(plugin) => matches!(command, UiCommand::Osd(_)) || revokes(plugin),
        Fact::Views(plugin) => {
            matches!(command, UiCommand::View(op) if op.reply.owner == plugin) || revokes(plugin)
        }
        Fact::Pip => matches!(
            command,
            UiCommand::Window(WindowOp::Pip { .. }) | UiCommand::RevokeOwner { .. }
        ),
        Fact::PipHolder(plugin) => {
            matches!(command, UiCommand::Window(WindowOp::Pip { reply, .. }) if reply.owner == plugin)
                || revokes(plugin)
        }
    }
}

impl Host {
    /// The driver's side of the outbox (plan step S2f(3)): every entry in
    /// the core's order. A request is carried out by the interface, then
    /// its facts go to the core (`report_ui_facts`, as after every request
    /// since S2f(1)); a report it sends on the way is handled by the core
    /// at once and drains what that issued (`ui_report`). A [`Then`] is the
    /// core's, run here, its own requests drained before the next entry.
    /// Each pass takes the entries issued so far, so what an entry's
    /// handling issues is carried out before the entries after it, as when
    /// each request was carried out in place.
    pub(crate) fn drain_outbox(&mut self) {
        loop {
            let entries = std::mem::take(&mut self.core.outbox);
            if entries.is_empty() {
                return;
            }
            for entry in entries {
                match entry {
                    Out::Ui(command) => {
                        self.apply_ui_command(command);
                        self.report_ui_facts();
                    }
                    Out::Then(then) => {
                        self.report_ui_facts();
                        self.run_then(then);
                        self.drain_outbox();
                    }
                }
                // Every report drained what it issued: nothing waits
                // behind the entries still to come.
                debug_assert!(
                    self.core.outbox.is_empty(),
                    "a request was issued and not drained"
                );
            }
        }
    }
    /// `reply` with `result`, now or, while a request whose interface part
    /// writes lines waits ([`Core::lines_pending`]), behind it
    /// ([`Then::Respond`]).
    pub(crate) fn respond_in_order(
        &mut self,
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
    ) {
        if self.core.lines_pending() {
            self.core.then(Then::Respond { reply, result });
        } else {
            self.respond(reply, result);
        }
    }
    /// The core's [`Then`].
    fn run_then(&mut self, then: Then) {
        match then {
            Then::Respond { reply, result } => self.respond(reply, result),
            Then::Registered { reply, refused } => {
                self.report_refused(reply.owner, refused);
                self.respond(reply, Ok(Value::Null));
            }
            Then::Event(event) => self.core_event(event),
            Then::SupervisorRun => self.supervisor_run(),
        }
    }
}
