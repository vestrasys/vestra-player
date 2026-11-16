//! The driver: the one place where the interface reaches the core (plan
//! step S3a(1); design §5, rules R1–R3). `Host` holds the core inside a
//! [`CoreLink`] whose field is private to this module, so the compiler
//! keeps every other `impl Host` off the core's data: the interface gives
//! the core an event, a report or a turn, takes what the core answers to
//! its requests (`Take`), and reads its own copies of the core's data
//! (`Host::mounts`, `Host::pages`, `Host::view`, `Host::plugins`), given
//! at the start of every drain ([`Host::drain_outbox`]). On one thread
//! everything here is still delivered in place, at today's points (zero
//! difference); plan step S3a(2) moves the core behind this link to its
//! own thread.
use super::messages::{Take, UiCommand, UiReport};
use super::outbox::Out;
use super::*;

/// The core, as the interface holds it until plan step S3a(2).
pub(crate) struct CoreLink {
    core: Core,
}

impl CoreLink {
    pub(crate) fn new(core: Core) -> Self {
        Self { core }
    }
}

/// The sender the interface's callbacks give their events to (plan step
/// S3a(1), brief item 1). On one thread it is the core's event queue, so
/// the interleaving of the interface's events with the plugins' is today's;
/// plan step S3a(2) gives it a queue of its own on the UI side.
#[derive(Clone)]
pub(crate) struct UiEvents(mpsc::Sender<Event>);

impl UiEvents {
    pub(crate) fn new(events: mpsc::Sender<Event>) -> Self {
        Self(events)
    }
    pub(crate) fn send(&self, event: Event) -> Result<(), mpsc::SendError<Event>> {
        self.0.send(event)
    }
    /// A sender of its own, for a callback that keeps one (a view's, a
    /// settings page's).
    pub(crate) fn sender(&self) -> mpsc::Sender<Event> {
        self.0.clone()
    }
}

/// The input helpers take the sender by reference (`host/input.rs`).
impl std::ops::Deref for UiEvents {
    type Target = mpsc::Sender<Event>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The playback handles the interface draws with (the frame slot, the
/// renderer, its target and the input routing): the backend's, shared
/// (they are thread-safe), cloned once at start (design §5.3 group D).
pub(crate) struct Playback {
    pub(crate) frame: Arc<render_target::FrameSlot>,
    pub(crate) target: Arc<render_target::Shared>,
    pub(crate) render: Arc<crate::playback::renderer::Render>,
    #[cfg(feature = "producer-audio")]
    pub(crate) input: Arc<std::sync::Mutex<Option<producer_input::Routing>>>,
}

impl Playback {
    pub(crate) fn of(backend: &Backend) -> Self {
        Self {
            frame: backend.frame.clone(),
            target: backend.target.clone(),
            render: backend.render.clone(),
            #[cfg(feature = "producer-audio")]
            input: backend.input.clone(),
        }
    }
}

impl Host {
    /// The interface's clock (plan S3a brief, item 10): milliseconds since
    /// the core's epoch, so the values are the core's (`Core::now_ms`).
    pub(crate) fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
}

impl Host {
    /// An event of the core (`Host::handle`): the interface's facts first
    /// (plan step S2f(1)), then the core's handling, then what it issued
    /// (plan step S2f(3)).
    pub(crate) fn deliver_event(&mut self, event: Event) {
        self.report_ui_facts();
        #[cfg(debug_assertions)]
        self.check_copies();
        self.link.core.handle(event);
        self.drain_outbox();
    }
    /// The core's side of a report (`Host::ui_report`), then what it issued.
    pub(crate) fn deliver_report(&mut self, report: UiReport) {
        self.link.core.report(report);
        self.drain_outbox();
    }
    /// What the core answers to a request of the interface's (plan step
    /// S3a(1): the interface's pulls of core data are requests now). On one
    /// thread it is answered in place, where the interface pulled before;
    /// it reads none of the interface's facts, so none are reported first.
    pub(crate) fn take(&mut self, take: Take) -> Vec<UiCommand> {
        let answer = self.link.core.take(take);
        self.drain_outbox();
        answer
    }
    /// The driver's side of the outbox (plan step S2f(3)): every entry in
    /// the core's order. The interface's copies of the core's data come
    /// first (plan step S3a(1): `Core::copies`), so what an entry's
    /// interface part reads is the core's data as the handler left it. A
    /// request is carried out by the interface, then its facts go to the
    /// core (`report_ui_facts`, as after every request since S2f(1)); a
    /// report it sends on the way is handled by the core at once and drains
    /// what that issued (`ui_report`). A [`super::outbox::Then`] is the
    /// core's, run here, its own requests drained before the next entry.
    /// Each pass takes the entries issued so far, so what an entry's
    /// handling issues is carried out before the entries after it, as when
    /// each request was carried out in place.
    pub(crate) fn drain_outbox(&mut self) {
        loop {
            self.take_copies();
            let entries = std::mem::take(&mut *self.link.core.outbox);
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
                        self.link.core.run_then(then);
                        self.drain_outbox();
                    }
                }
                // Every report drained what it issued: nothing waits
                // behind the entries still to come.
                debug_assert!(
                    self.link.core.outbox.is_empty(),
                    "a request was issued and not drained"
                );
            }
        }
    }
    /// The interface's copies of the core's data follow the core (plan step
    /// S3a(1)).
    fn take_copies(&mut self) {
        let copies = self.link.core.copies();
        for op in copies.mount_ops {
            self.mount_op(op);
        }
        if let Some(facts) = copies.facts {
            self.view = facts;
        }
        if let Some(tick) = copies.tick {
            self.tick_facts = tick;
        }
        if let Some(plugins) = copies.plugins {
            self.plugins = plugins;
        }
        if let Some(pages) = copies.pages {
            self.pages = pages;
        }
    }
    /// The first half of the core's turn, which began at `began` (plan step
    /// S2e): `first` (an event the headless driver already waited for),
    /// then the queued ones ([`super::events::drain`]), each handled.
    /// `observe` sees each event before it is handled (the headless
    /// evidence); `turn`, the windowed tick's record, gets each event's
    /// handling time (`host/stall.rs`).
    pub(crate) fn core_events(
        &mut self,
        receiver: &mpsc::Receiver<Event>,
        began: Instant,
        first: Option<Event>,
        mut turn: Option<&mut super::stall::Turn>,
        mut observe: impl FnMut(&Event),
    ) {
        let mut handle = |host: &mut Host, event: Event| {
            observe(&event);
            let (label, at) = (event.label(), Instant::now());
            host.handle(event);
            if let Some(turn) = turn.as_deref_mut() {
                turn.event(label, at.elapsed());
            }
        };
        let handled = match first {
            Some(event) => {
                handle(self, event);
                1
            }
            None => 0,
        };
        super::events::drain(receiver, began, handled, |event| handle(self, event));
    }
    /// The second half of the core's turn: its periodic steps
    /// ([`Core::STEPS`]), in their order, each followed by what it asked of
    /// the interface (plan step S2f(3), `outbox.rs`). A headless run never
    /// ran the windowed steps and still does not. `turn` names the steps
    /// for the windowed tick's record.
    pub(crate) fn core_steps(&mut self, mut turn: Option<&mut super::stall::Turn>) {
        // The interface's facts as the core's steps read them (plan step
        // S2f(1)).
        self.report_ui_facts();
        #[cfg(debug_assertions)]
        self.check_copies();
        for step in Core::STEPS {
            if step.windowed && self.headless {
                break;
            }
            (step.run)(&mut self.link.core);
            self.drain_outbox();
            if let (Some(what), Some(turn)) = (step.ends, turn.as_deref_mut()) {
                turn.step(what);
            }
        }
    }
    /// The core's panel-restart step at its point of the windowed tick
    /// (between the two halves of the core's turn): a hand-over to the core
    /// (plan step S2f(1)), then what it asked of the interface (plan step
    /// S2f(3)).
    pub(crate) fn panel_restart_tick(&mut self) {
        if self.link.core.restarts.plugins.is_empty() {
            return;
        }
        self.report_ui_facts();
        #[cfg(debug_assertions)]
        self.check_copies();
        self.link.core.panel_restart_tick();
        self.drain_outbox();
    }
    /// The core's start-up calls (plan S3a brief, item 9): they run on the
    /// main thread, on the core before the interface's loop starts, at
    /// today's points; nothing of them runs while the interface does. What
    /// they ask of the interface is drained where the start drained before.
    pub(crate) fn starting<R>(&mut self, start: impl FnOnce(&mut Core) -> R) -> R {
        start(&mut self.link.core)
    }
    /// The core's calls at quit (design §5.3 group F), after the interface's
    /// loop ended; plan step S3a(2) gives the core its own deadline and exit.
    pub(crate) fn quitting<R>(&mut self, quit: impl FnOnce(&mut Core) -> R) -> R {
        quit(&mut self.link.core)
    }
    /// The playback backend, for the headless driver's evidence.
    pub(crate) fn backend(&self) -> &Backend {
        &self.link.core.backend
    }
    /// Debug builds: where the driver hands a turn to the core (an event of
    /// the core, the core's steps, the panel-restart step), the core's
    /// copies of the interface's state are the interface's: its facts, its
    /// tabs and its record of the batching window (plan step S2f(3)). Core
    /// code takes no `Host`, so the checks run here, not at the reads: the
    /// interface does not run inside a core turn, and a read inside it
    /// checks that no command that would change what it reads waits in the
    /// outbox (`Core::check_settled`). They compare the two sides in place
    /// and are removed with the in-place delivery (plan step S3a(2)).
    #[cfg(debug_assertions)]
    pub(crate) fn check_copies(&self) {
        self.check_ui_facts();
        self.check_panel_tabs();
        self.check_handed();
        self.check_mounts();
        self.check_core_copies();
    }
    /// Debug builds: the core's copy of the interface's facts is the
    /// interface's (plan step S2f(1)), and the current tabs the core derives
    /// from it are the interface's (`check_copies`).
    #[cfg(debug_assertions)]
    fn check_ui_facts(&self) {
        let core = &self.link.core;
        let mut now = self.ui_facts_now();
        // A stopped plugin's confirmation that a waiting `DropConfirm`
        // drops: the core's copy is ahead (plan step S2f(3),
        // `UiDialog::revoke`).
        if now
            .sheet
            .is_some_and(|sheet| sheet.confirm && core.drop_pending(sheet.owner))
        {
            now.sheet = None;
        }
        debug_assert_eq!(
            core.ui_facts, now,
            "the core's interface facts are behind the interface"
        );
        for column in sidebar_tabs::Column::ALL {
            debug_assert_eq!(
                core.current_tab(column),
                self.current_tab(column),
                "the core's current tab of {column:?} differs"
            );
        }
    }
    /// Debug builds: the core's copy of the tabs is the policy's wherever
    /// the core reads it (plan step S2c(5): `report_tabs` after every
    /// change).
    #[cfg(debug_assertions)]
    fn check_panel_tabs(&self) {
        debug_assert_eq!(
            self.link.core.panel_tabs,
            self.policy_tabs(),
            "the core's panel tabs are behind the policy"
        );
    }
    /// Debug builds: the core's record of the change the batching window
    /// holds is the window's at the core's reads of it (plan step S2d(2)),
    /// except while a change is being handed over (`handing_over`: the
    /// record is set, the window not yet).
    #[cfg(debug_assertions)]
    fn check_handed(&self) {
        let core = &self.link.core;
        // Nor while a cancel of the core's waits in its outbox (plan step
        // S2f(3)): the core cleared its record, the window drops the
        // change when the cancel is carried out.
        if self.handing_over || core.cancel_pending() {
            return;
        }
        let window = self
            .preparing
            .as_ref()
            .map(|(_, change)| super::mounts::HandedMount::of(change));
        let same = match (&core.handed, &window) {
            (None, None) => true,
            (Some(core), Some(window)) => core.same(window),
            _ => false,
        };
        debug_assert!(
            same,
            "the core's record of the batching window is behind it"
        );
    }
    /// Debug builds (plan step S3a(1)): the interface's copy of the mount
    /// registry has the core's panels, slots, plugins and placeholders.
    #[cfg(debug_assertions)]
    fn check_mounts(&self) {
        let shape = |mounts: &BTreeMap<u64, Mount>| {
            mounts
                .iter()
                .map(|(key, m)| (*key, m.spec.owner, m.spec.slot as u8, m.plugin, m.recorded))
                .collect::<Vec<_>>()
        };
        debug_assert_eq!(
            shape(&self.mounts),
            shape(&self.link.core.mounts),
            "the interface's copy of the mount registry is behind the core's"
        );
    }
    /// Debug builds (plan step S3a(1), from the static review): the
    /// interface's copies of the settings pages, the core's small values
    /// and the plugins are the core's.
    #[cfg(debug_assertions)]
    fn check_core_copies(&self) {
        let core = &self.link.core;
        debug_assert!(
            self.pages == core.settings.pages(),
            "the interface's copy of the settings pages is behind the core's"
        );
        debug_assert!(
            self.view == core.facts_now(),
            "the interface's copy of the core's facts is behind the core's"
        );
        debug_assert!(
            self.tick_facts == core.tick_facts(),
            "the interface's copy of the core's tick facts is behind the core's"
        );
        debug_assert!(
            self.plugins.shape() == core.plugins_view().shape(),
            "the interface's copy of the plugins is behind the core's"
        );
    }
    /// Debug builds: the interface's restart lines are the core's record's
    /// (plan step S2f(2); `Host::restart_waiting`).
    #[cfg(debug_assertions)]
    pub(crate) fn check_restart_lines(&self) {
        debug_assert_eq!(
            self.restart_lines,
            self.link.core.restarts.lines(),
            "the interface's restart lines are behind the core's record"
        );
    }
}
