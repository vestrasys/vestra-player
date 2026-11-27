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
//! the interface applies at its wake ([`Host::apply_queue`]).
//!
//! Plan step S3a(4), the thread: the core runs on its own thread
//! (`vestra-core`, [`run_core`]); since plan step S3b it sleeps on its
//! queue and handles each event when it arrives, its periodic steps on a
//! 16 ms deadline. It wakes the interface with `invoke_from_event_loop`,
//! at most one wake pending at a time ([`Waker`]); at quit the interface
//! asks it to end and joins it with a bound ([`Host::join_core`]), and it
//! ends on its own too (the run's end, its own quit, a lost device).
//! Start-up and headless runs keep the core here (the headless driver has
//! no interface and runs the same turn on the main thread).
use super::messages::{UiEvent, UiReport};
use super::outbox::{Copies, ToUi};
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// The core as the interface holds it: here (start-up, headless runs, and
/// back again after its thread ended at quit), or on its own thread (plan
/// step S3a(4)).
pub(crate) struct CoreLink {
    core: Option<Core>,
    thread: Option<CoreThread>,
}

/// The core's thread: its handle, and where it hands the core back when it
/// ends ([`run_core`]).
struct CoreThread {
    handle: std::thread::JoinHandle<()>,
    done: mpsc::Receiver<Core>,
}

impl CoreLink {
    pub(crate) fn new(core: Core) -> Self {
        Self {
            core: Some(core),
            thread: None,
        }
    }
    /// The core, here. Only where the core is here: the start-up and quit
    /// calls, headless runs.
    fn here(&mut self) -> &mut Core {
        self.core
            .as_mut()
            .expect("the core runs on its own thread (plan step S3a(4))")
    }
    fn here_ref(&self) -> &Core {
        self.core
            .as_ref()
            .expect("the core runs on its own thread (plan step S3a(4))")
    }
}

/// The period of the core's steps (R10): their fixed deadline. Since plan
/// step S3b an arriving event wakes the core at once (it sleeps on its
/// queue until the next deadline); only the steps wait for it.
pub(crate) const CORE_PERIOD: Duration = Duration::from_millis(16);

/// How long the core waits for the interface's quit after it ended the run
/// itself (its `quit` command, the run's end, a lost device), before it
/// writes the state file and ends without it (the S3a brief's risk 5: also
/// when the interface never asks). The interface's last report (its part of
/// the state file) arrives before its quit in the same queue.
const UI_QUIT_WAIT: Duration = Duration::from_secs(5);

/// The margin of the interface's bounded join over the plugins' shutdown
/// budget (the S3a brief's risk 5; the implementer names it): the core
/// writes the state file and ends its turn after the shutdown, well inside
/// it.
pub(crate) const JOIN_MARGIN: Duration = Duration::from_secs(2);
/// The exit code when the core's thread panicked: what a panic that
/// unwound out of the UI thread's loop gave before plan step S3a(4).
pub(crate) const CORE_PANIC_EXIT: u32 = 101;
/// How long the unwind of a panicked core may take before the process is
/// ended anyway (`PanicWatchdog`): dropping the core stops its plugins and
/// joins its playback worker, either of which can block.
const PANIC_EXIT_WAIT: Duration = Duration::from_secs(2);

/// Ends the process [`PANIC_EXIT_WAIT`] after the core's thread began to
/// unwind from a panic (S3a(4) re-review): its drop runs before the core's
/// own (a local of `run_core`, dropped before its `core` argument), so a
/// `Plugin::drop` or `Backend::drop` that blocks cannot keep the player
/// alive with a dead core. On a normal return it does nothing.
struct PanicWatchdog;

impl Drop for PanicWatchdog {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = std::thread::Builder::new()
                .name("core-panic-watchdog".into())
                .spawn(|| {
                    std::thread::sleep(PANIC_EXIT_WAIT);
                    logged!("exit: the core's unwind did not end; ending the player");
                    platform::exit_now(CORE_PANIC_EXIT);
                });
        }
    }
}

/// The core's thread returned (it gave its core back, `run_core`): from
/// then on the interface ends its event loop, also when the quit it was
/// asked for never reached the loop (`core_ended`).
static CORE_ENDED: AtomicBool = AtomicBool::new(false);

/// Whether the core's thread has ended. The interface asks for the quit
/// only after its event loop ended, so a loop still running then lost its
/// quit (Windows: winit's hidden thread window was destroyed): it ends on
/// its own, without the event loop's proxy (`frame_pacing::Handler`), so
/// the player never outlives its core.
pub(crate) fn core_ended() -> bool {
    CORE_ENDED.load(Ordering::Acquire)
}

/// The interface's wake (plan step S3a(4), the S3a brief's "Queues"): at
/// most one `invoke_from_event_loop` pending at a time. The core sets the
/// flag when it posts; the interface clears it before it drains.
#[derive(Clone, Default)]
pub(crate) struct Waker(Arc<AtomicBool>);

impl Waker {
    /// Posts the interface's wake unless one is pending.
    pub(crate) fn wake(&self) {
        if !self.0.swap(true, Ordering::AcqRel) && slint::invoke_from_event_loop(wake_ui).is_err() {
            // No event loop (it ended): nothing to wake.
            self.0.store(false, Ordering::Release);
        }
    }
    /// The interface takes the wake (before it drains).
    pub(crate) fn taken(&self) {
        self.0.store(false, Ordering::Release);
    }
    /// A wake is posted and not taken yet: also one the interface missed
    /// while a callback held it (review N1), which its next tick takes.
    pub(crate) fn pending(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

thread_local! {
    /// The interface's side of the core's wake (UI thread; set in `main.rs`).
    static CORE_WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The interface's wake handler, set once its loop exists (`main.rs`).
pub(crate) fn set_core_wake(wake: Option<Box<dyn Fn()>>) {
    CORE_WAKE.with(|sink| *sink.borrow_mut() = wake);
}

/// Runs on the UI thread, posted by [`Waker::wake`].
fn wake_ui() {
    CORE_WAKE.with(|sink| {
        if let Some(wake) = &*sink.borrow() {
            wake();
        }
    });
}

/// The first half of the core's turn, which began at `began` (plan step
/// S2e): `first` (an event the headless driver already waited for), then
/// the queued ones ([`super::events::drain`]), each handled and followed by
/// what it issued (`Core::flush`). Plan step S3a(4): on whichever thread
/// runs the core; an interface event that reaches the core's queue goes on
/// to the interface's own queue (`to_ui`). `observe` sees each event before
/// it is handled (the headless evidence); `turn`, the turn's record, gets
/// each event's handling time (`host/stall.rs`).
pub(crate) fn core_events(
    core: &mut Core,
    receiver: &mpsc::Receiver<Event>,
    began: Instant,
    first: Option<Event>,
    mut turn: Option<&mut super::stall::Turn>,
    mut observe: impl FnMut(&Event),
    to_ui: &mpsc::Sender<Event>,
) {
    let mut handle = |core: &mut Core, event: Event| {
        observe(&event);
        let (label, at) = (event.label(), Instant::now());
        match UiEvent::new(event) {
            Ok(event) => {
                let _ = to_ui.send(event.into_event());
            }
            Err(event) => {
                core.handle(event);
                core.flush();
            }
        }
        if let Some(turn) = turn.as_deref_mut() {
            turn.event(label, at.elapsed());
        }
    };
    // The interface's reports first (`window.state` goes at its
    // `FactsSettled` among them), then the other events in their order
    // (S3a(3) verification, re-review B2: `events::turn_events`); a
    // subscription that came in meanwhile is answered after that state,
    // as on main.
    super::events::turn_events(receiver, first, began, |step| {
        use super::events::TurnStep;
        match step {
            TurnStep::Event(event) => handle(core, event),
            TurnStep::Hold(event) => core.held_events.push_back(event),
            TurnStep::ReportsDone => core.flush(),
            TurnStep::NextHeld => return core.held_events.pop_front(),
        }
        None
    });
}

/// The second half of the core's turn: plan step S3a(3) runs the
/// panel-restart step first, in the core turn right after the event drain
/// (the S3a brief's item 3), then the periodic steps ([`Core::STEPS`]), in
/// their order, each followed by what it issued (`Core::flush`), then the
/// end of the turn's pushes (`Core::turn_end`). A headless run never ran
/// the windowed steps and still does not. `turn` names the steps for the
/// turn's record.
pub(crate) fn core_steps(core: &mut Core, mut turn: Option<&mut super::stall::Turn>) {
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

/// What the core's thread is started with ([`run_core`]).
pub(crate) struct CoreStart {
    /// The core's one event queue.
    pub(crate) receiver: mpsc::Receiver<Event>,
    /// The interface's own queue, for an interface event that reaches the
    /// core's.
    pub(crate) to_ui: mpsc::Sender<Event>,
    pub(crate) waker: Waker,
    /// The run's end (`--seconds`): a quit the core acts on itself (K9).
    pub(crate) ends_at: Option<Instant>,
    /// What a restart after a lost window device needs: the core starts it
    /// (the S3a brief's risk 5).
    pub(crate) lost: Option<LostDevice>,
}

/// The core's thread (plan step S3a(4); plan step S3b, R10's second step):
/// it sleeps on its queue until an event arrives or the steps' 16 ms
/// deadline comes. An event is handled when it arrives, in the turn's
/// order (`core_events`: the interface's reports first, then the rest, at
/// least 128, then the time budget), followed by the end-of-turn pushes
/// (`Core::turn_end`). At the deadline (a late one moves the next a full
/// period after it began, without a burst; review N3) what arrived is
/// taken in (reports handled, the rest held: `Busy::Request`) and the
/// panel-restart step and the periodic steps run. After each, the
/// interface is woken if the core→UI queue holds anything. Its quit (the
/// S3a brief's risk 5): it ends when the interface asks
/// (`Event::CoreQuit`: the plugins stop within the budget asked, the
/// state file is written, and after a lost device the player is restarted
/// from here); it also ends the run on its own at the run's end, after its
/// own `quit` command and on a lost device: the plugins stop at once, and
/// it waits for the interface's quit at most [`UI_QUIT_WAIT`] before it
/// writes the state file and ends. The core goes back to the interface,
/// which drops it before its windows, as before.
pub(crate) fn run_core(mut core: Core, start: CoreStart, done: mpsc::Sender<Core>) {
    let _watchdog = PanicWatchdog;
    let CoreStart {
        receiver,
        to_ui,
        waker,
        ends_at,
        lost,
    } = start;
    let mut next = Instant::now();
    // `VESTRA_TEST_CORE_PANIC=<ms>`, a test switch: the core panics that
    // long after its thread started (S3a(4) review F1's test).
    let panic_at = std::env::var("VESTRA_TEST_CORE_PANIC")
        .ok()
        .filter(|_| std::env::var_os("VESTRA_TEST_OFFSCREEN").is_some_and(|v| v == "1"))
        .and_then(|ms| ms.trim().parse::<u64>().ok())
        .map(|ms| next + Duration::from_millis(ms));
    // The core ended the run itself, since then; the plugins were stopped.
    let mut own_quit: Option<Instant> = None;
    let mut stopped = false;
    loop {
        // Plan step S3b (R10's second step): the core sleeps on its queue
        // until an event arrives or its periodic steps are due, whichever
        // is first; events it holds are handled without a wait.
        let now = Instant::now();
        let first = if core.held_events.is_empty() && next > now {
            match receiver.recv_timeout(next - now) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // No sender left (the interface is gone): the steps
                    // keep their deadline.
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                    None
                }
            }
        } else {
            None
        };
        let began = Instant::now();
        if panic_at.is_some_and(|at| began >= at) {
            panic!("test: the core panics (VESTRA_TEST_CORE_PANIC)");
        }
        let mut turn = super::stall::Turn::core(began);
        // The events, in the turn's order (`core_events`: the interface's
        // reports first, then the rest within the budget).
        core_events(
            &mut core,
            &receiver,
            began,
            first,
            Some(&mut turn),
            |_| (),
            &to_ui,
        );
        turn.step("events");
        if Instant::now() >= next {
            // The periodic steps, on their 16 ms deadline (R10).
            next += CORE_PERIOD;
            // A late turn: the next one a whole period after this one
            // began (review N3), not at once.
            if next < began {
                next = began + CORE_PERIOD;
            }
            // What came in during the events (S3a(4) verification): the
            // interface's reports are handled now, the rest waits in
            // `Core::held_events`, where the supervisor sees a plugin's
            // request still unanswered (`Busy::Request`) and does not stop
            // it for idleness meanwhile.
            while let Ok(event) = receiver.try_recv() {
                if matches!(event, Event::Report(_)) {
                    core.handle(event);
                    core.flush();
                } else {
                    core.held_events.push_back(event);
                }
            }
            core_steps(&mut core, Some(&mut turn));
        } else {
            // Between the steps (plan step S3b): what the events changed of
            // the end-of-turn pushes goes now (the batching window's
            // hand-over and close on its own clock, the tables and window
            // settings), so a reply or a table does not wait for the next
            // deadline.
            core.turn_end();
            core.flush();
        }
        super::stall::turn_ended(&turn, |owner| core.plugin_id(owner));
        let asked = core.quit_asked.take();
        let device_lost = lost.is_some() && platform::window_device_lost();
        if own_quit.is_none()
            && (core.quitting || device_lost || ends_at.is_some_and(|end| Instant::now() >= end))
        {
            own_quit = Some(Instant::now());
            // The interface applies what is queued before the plugins stop
            // (review N5).
            waker.wake();
            let budget = if device_lost {
                DEVICE_LOSS_BUDGET
            } else {
                QUIT_BUDGET
            };
            core.shutdown_plugins(budget);
            core.flush();
            stopped = true;
            if device_lost {
                restart_from_core(&mut core, lost.as_ref());
            }
        }
        if let Some(quit) = asked {
            if !stopped {
                core.shutdown_plugins(quit.budget);
                core.flush();
            }
            if quit.device_lost {
                restart_from_core(&mut core, lost.as_ref());
            }
            core.ui_state.flush();
            waker.wake();
            let _ = done.send(core);
            return;
        }
        if own_quit.is_some_and(|at| at.elapsed() >= UI_QUIT_WAIT) {
            diag!(
                "core: the interface did not ask for the quit within {} s; ending without it",
                UI_QUIT_WAIT.as_secs()
            );
            core.ui_state.flush();
            waker.wake();
            let _ = done.send(core);
            return;
        }
        if !core.outbox.to_ui.is_empty() {
            waker.wake();
        }
    }
}

/// After a lost window device, from the core (the S3a brief's risk 5): the
/// state file first, then the restart on what was playing
/// (`restart::restart_player`, once per process).
fn restart_from_core(core: &mut Core, lost: Option<&LostDevice>) {
    core.ui_state.flush();
    if let Some(lost) = lost {
        let source = core.last_source.clone();
        super::restart::restart_player(lost, &source);
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
        let core = self.link.here();
        core.handle(event);
        core.flush();
    }
    /// Moves the core to its own thread (plan step S3a(4)): once the
    /// interface's windows exist, before its timer starts; the start-up
    /// calls ran here before (the S3a brief's item 9).
    pub(crate) fn start_core_thread(&mut self, start: CoreStart) -> std::io::Result<()> {
        let core = self
            .link
            .core
            .take()
            .expect("the core is here until its thread starts");
        let (done, back) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("vestra-core".into())
            .spawn(move || {
                // A panic of the core ends the player, as it did on the UI
                // thread before (S3a(4) review F1): the panic hook has
                // logged it; a live window with a dead core answers nothing.
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_core(core, start, done)
                }));
                if ran.is_err() {
                    logged!("exit: the core's thread panicked; ending the player");
                    platform::exit_now(CORE_PANIC_EXIT);
                }
                CORE_ENDED.store(true, Ordering::Release);
            })?;
        self.link.thread = Some(CoreThread { handle, done: back });
        Ok(())
    }
    /// The interface's side of the quit (plan step S3a(4), the S3a brief's
    /// risk 5): asks the core's thread to end (`Event::CoreQuit`, behind
    /// the interface's last reports), and joins it with a bound, the
    /// shutdown `budget` plus [`JOIN_MARGIN`], before any window is
    /// dropped; the core comes back here. On a timeout the process is ended
    /// at once (`platform::exit_now`), as a stuck teardown after a device
    /// loss is. Nothing to join where the core is already here.
    pub(crate) fn join_core(&mut self, budget: Duration, device_lost: bool) {
        let Some(thread) = self.link.thread.take() else {
            return;
        };
        let _ = self.to_core.send(Event::CoreQuit(CoreQuit {
            budget,
            device_lost,
        }));
        let bound = budget + JOIN_MARGIN;
        match thread.done.recv_timeout(bound) {
            Ok(core) => {
                let _ = thread.handle.join();
                self.link.core = Some(core);
            }
            // The thread ended without giving the core back: a panic its
            // wrapper did not get to end the process for (S3a(4) review F1).
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                logged!(
                    "exit: the core's thread ended without its core (a panic); ending the player"
                );
                platform::exit_now(CORE_PANIC_EXIT);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                logged!(
                    "exit: the core did not end within {} ms of the quit; ending the player",
                    bound.as_millis()
                );
                platform::exit_now(3);
            }
        }
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
        self.link.here().flush();
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
    /// The core's turn's first half, here (the headless driver, which
    /// keeps no turn record; [`core_events`]).
    pub(crate) fn core_events(
        &mut self,
        receiver: &mpsc::Receiver<Event>,
        began: Instant,
        first: Option<Event>,
        observe: impl FnMut(&Event),
    ) {
        let to_ui = self.events.sender();
        core_events(
            self.link.here(),
            receiver,
            began,
            first,
            None,
            observe,
            &to_ui,
        );
    }
    /// Events the core's last turn left behind the reports
    /// (`Core::held_events`): the headless driver does not wait for more.
    pub(crate) fn events_held(&mut self) -> bool {
        !self.link.here().held_events.is_empty()
    }
    /// The core's turn's second half, here (the headless driver;
    /// [`core_steps`]).
    pub(crate) fn core_steps(&mut self) {
        core_steps(self.link.here(), None);
    }
    /// At quit, before the core's quit calls, with the core here (S3a(3)
    /// review item 9): the interface's reports waiting in the core's queue
    /// are handled (its last facts, a final `Remembered` for the state
    /// file), as each was handled in place before; the other events waiting
    /// are left, as they were at quit before. (On its thread the core
    /// handles them before the interface's quit, which follows them in the
    /// same queue.)
    pub(crate) fn quit_reports(&mut self, receiver: &mpsc::Receiver<Event>) {
        while let Ok(event) = receiver.try_recv() {
            if matches!(event, Event::Report(_)) {
                self.deliver_event(event);
            }
        }
        let core = self.link.here();
        core.window_state_out();
        core.flush();
    }
    /// The core's start-up calls (plan S3a brief, item 9): they run on the
    /// main thread, on the core before the interface's loop starts, at
    /// today's points; nothing of them runs while the interface does. What
    /// they ask of the interface is applied where the start drained before
    /// ([`Host::sync`]).
    pub(crate) fn starting<R>(&mut self, start: impl FnOnce(&mut Core) -> R) -> R {
        start(self.link.here())
    }
    /// The core's calls at quit (design §5.3 group F), after the interface's
    /// loop ended, on the core here (plan step S3a(4): back from its thread,
    /// [`Host::join_core`]).
    pub(crate) fn quitting<R>(&mut self, quit: impl FnOnce(&mut Core) -> R) -> R {
        quit(self.link.here())
    }
    /// The playback backend, for the headless driver's evidence.
    pub(crate) fn backend(&self) -> &Backend {
        &self.link.here_ref().backend
    }
}
