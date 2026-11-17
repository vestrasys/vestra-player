//! The driver: the one place where the interface reaches the core (plan
//! step S3a(1); design §5, rules R1–R3). `Host` holds the core inside a
//! [`CoreLink`] whose field is private to this module, so the compiler
//! keeps every other `impl Host` off the core's data: the interface gives
//! the core an event, a report or a turn, takes what the core answers to
//! its requests (`Take`), and reads its own copies of the core's data
//! (`Host::mounts`, `Host::pages`, `Host::view`, `Host::plugins`).
//!
//! Plan step S3a(3), the queues on one thread: nothing is delivered in
//! place any more. The interface's reports go into the core's one event
//! queue ([`Host::send_report`]) and are handled in the core's turn; what
//! the core issues goes into the one core→UI queue (`Core::flush`), which
//! the interface applies at its wake ([`Host::apply_queue`]). The core's
//! turn is still called from the UI thread's timer (the arrangement
//! `VESTRA_CORE_ON_UI=1` keeps from plan step S3a(4) on, when the core
//! gets its own thread).
use super::messages::UiReport;
use super::outbox::{Copies, ToUi};
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
    /// An event of the core's queue (`Host::handle`): the core handles it,
    /// then pushes what it issued into the core→UI queue (plan step
    /// S3a(3), `Core::flush`). The interface's facts are not reported here
    /// any more: they reach the core's queue with each report, numbered.
    pub(crate) fn deliver_event(&mut self, event: Event) {
        self.link.core.handle(event);
        self.link.core.flush();
    }
    /// A report of the interface, into the core's one event queue (plan
    /// step S3a(3), the S3a brief's "Queues"): handled in the core's next
    /// turn.
    pub(crate) fn send_report(&mut self, report: UiReport) {
        let _ = self.to_core.send(Event::Report(Box::new(report)));
    }
    /// The interface's wake (plan step S3a(3)): every entry the core pushed
    /// since, in order, applied: the copies of the core's data that changed
    /// (as at the start of every drain before), then each command, followed
    /// by the interface's facts. The commands arrive numbered without gaps
    /// (checked; the drain's "nothing left behind" assert before). The
    /// interface then tells the core how far it applied
    /// (`UiReport::Applied`), after the reports those commands' parts sent.
    pub(crate) fn apply_queue(&mut self) {
        let entries = self.queue.take_all();
        // The queue's peak length (the S3a row's measurement), logged at
        // the end of the run.
        self.queue_peak = self.queue_peak.max(entries.len());
        let mut last = None;
        for entry in entries {
            match entry {
                ToUi::Copies(copies) => self.apply_copies(*copies),
                ToUi::Command(seq, command) => {
                    debug_assert_eq!(
                        seq,
                        self.applied_seq + 1,
                        "the core→UI queue skipped or repeated a command"
                    );
                    self.applied_seq = seq;
                    self.apply_ui_command(command);
                    self.report_ui_facts();
                    last = Some(seq);
                }
            }
        }
        if let Some(seq) = last {
            self.send_report(UiReport::Applied(seq));
        }
    }
    /// Both sides at once where nothing else runs (plan S3a brief item 9:
    /// the start-up calls before the interface's loop; the quit calls
    /// after it): the core pushes what it issued, the interface applies it.
    /// Reports the interface's part sends are handled in the core's next
    /// turn (at start-up: the first turn of the loop).
    pub(crate) fn sync(&mut self) {
        self.link.core.flush();
        self.apply_queue();
    }
    /// The interface's copies of the core's data follow the core (plan step
    /// S3a(1)); S3a(3): from the core→UI queue.
    fn apply_copies(&mut self, copies: Copies) {
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
        self.stored_in_flight
            .take(&mut self.pages, copies.stored, copies.pages);
    }
    /// The first half of the core's turn, which began at `began` (plan step
    /// S2e): `first` (an event the headless driver already waited for),
    /// then the queued ones ([`super::events::drain`]), each handled.
    /// `observe` sees each event before it is handled (the headless
    /// evidence); `turn`, the windowed tick's record, gets each event's
    /// handling time (`host/stall.rs`). Plan step S3a(3): the interface's
    /// reports are events of this queue too.
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
        let mut handled = match first {
            Some(event) => {
                handle(self, event);
                1
            }
            None => 0,
        };
        // The interface's reports first (S3a(3) verification): on one thread
        // they come from its last wake, whose work main did in place, before
        // the core read any line that came in meanwhile; the other events
        // wait in `Core::held_events`, in their order. Then `window.state`
        // from the facts the reports left, before the requests that came in
        // meanwhile (a subscription answered now does not get it, as on
        // main).
        while let Ok(event) = receiver.try_recv() {
            if matches!(event, Event::Report(_)) {
                handle(self, event);
                handled += 1;
            } else {
                self.link.core.held_events.push_back(event);
            }
        }
        self.link.core.window_state_out();
        self.link.core.flush();
        let mut held = std::mem::take(&mut self.link.core.held_events);
        super::events::drain(
            || held.pop_front(),
            began,
            handled,
            |event| handle(self, event),
        );
        self.link.core.held_events = held;
    }
    /// Events the core's last turn left behind the reports
    /// (`Core::held_events`): the headless driver does not wait for more.
    pub(crate) fn events_held(&self) -> bool {
        !self.link.core.held_events.is_empty()
    }
    /// The second half of the core's turn: plan step S3a(3) runs the
    /// panel-restart step first, in the core turn right after the event
    /// drain (the S3a brief's item 3; it ran from the windowed tick between
    /// the interface's mount step and window tick before), then its
    /// periodic steps ([`Core::STEPS`]), in their order, each followed by
    /// what it issued (`Core::flush`), then the end of the turn's pushes
    /// (the tables and window settings that changed, `Core::turn_end`). A
    /// headless run never ran the windowed steps and still does not. `turn`
    /// names the steps for the windowed tick's record.
    pub(crate) fn core_steps(&mut self, mut turn: Option<&mut super::stall::Turn>) {
        let core = &mut self.link.core;
        if !core.headless && !core.restarts.plugins.is_empty() {
            core.panel_restart_tick();
            core.flush();
            if let Some(turn) = turn.as_deref_mut() {
                turn.step("panel restarts");
            }
        }
        for step in Core::STEPS {
            if step.windowed && core.headless {
                break;
            }
            (step.run)(core);
            core.flush();
            if let (Some(what), Some(turn)) = (step.ends, turn.as_deref_mut()) {
                turn.step(what);
            }
        }
        core.turn_end();
        core.flush();
    }
    /// At quit, before the core's quit calls (S3a(3) review item 9): the
    /// interface's reports waiting in the core's queue are handled (its
    /// last facts, a final `Remembered` for the state file), as each was
    /// handled in place before; the other events waiting are left, as they
    /// were at quit before.
    pub(crate) fn quit_reports(&mut self, receiver: &mpsc::Receiver<Event>) {
        while let Ok(event) = receiver.try_recv() {
            if matches!(event, Event::Report(_)) {
                self.deliver_event(event);
            }
        }
        self.link.core.window_state_out();
        self.link.core.flush();
    }
    /// The core's start-up calls (plan S3a brief, item 9): they run on the
    /// main thread, on the core before the interface's loop starts, at
    /// today's points; nothing of them runs while the interface does. What
    /// they ask of the interface is applied where the start drained before
    /// ([`Host::sync`]).
    pub(crate) fn starting<R>(&mut self, start: impl FnOnce(&mut Core) -> R) -> R {
        start(&mut self.link.core)
    }
    /// The core's calls at quit (design §5.3 group F), after the interface's
    /// loop ended; plan step S3a(4) gives the core its own deadline and exit.
    pub(crate) fn quitting<R>(&mut self, quit: impl FnOnce(&mut Core) -> R) -> R {
        quit(&mut self.link.core)
    }
    /// The playback backend, for the headless driver's evidence.
    pub(crate) fn backend(&self) -> &Backend {
        &self.link.core.backend
    }
}
