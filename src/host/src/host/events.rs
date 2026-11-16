//! What the UI thread's event loop receives (`Event`).
use super::*;

pub(crate) enum MenuAction {
    Invoke(String),
    /// A row entered: level, token, and whether it acts at once (a click or
    /// a key) or after the submenu delay (the pointer, G-15).
    Hover(usize, String, bool),
    /// G-03: scroll a level to this offset (clamped when projected).
    Scroll(usize, f64),
    Reset,
    Back,
    Dismiss,
    /// The command palette's field, cursor, wheel and runs.
    Palette(super::palette::PaletteAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Injected {
    Pressed,
    Repeated,
    Released,
}

pub(crate) enum Event {
    #[cfg(feature = "producer-audio")]
    PhysicalInput(u64, Value),
    WindowInput(u64, String),
    /// A bound input fired: (window generation, owner, command id).
    Binding(u64, u64, String),
    SidebarResized(u64, f64),
    /// The adjust column's splitter was released at this width.
    AdjustResized(u64, f64),
    /// A column's tab strip (shell or floating window generation): a tab
    /// clicked (its index in that window's tab order).
    Tab(u64, crate::ui::sidebar_tabs::Column, usize),
    /// A strip's `»` (sidebar-tabs.md): the overflow menu's origin, logical px.
    TabOverflow(u64, crate::ui::sidebar_tabs::Column, f64, f64),
    /// The sidebar strip's detach/attach button.
    SidebarDetach(u64),
    /// Esc inside the adjust column floating over a narrow window's video.
    AdjustDismiss(u64),
    /// IA-CORE-12: a double click on the video during picture-in-picture.
    PipReturn(u64),
    FloatingClosed(u64),
    MenuAction(u64, MenuAction),
    Request(u64, Value),
    /// A request whose large array its reader thread converted already
    /// (`ui_values::prepare_request`; a whole value is taken out of the
    /// message): answered as a `Request` with `Host::prepared` set.
    Prepared(u64, Value, Box<crate::ui::ui_values::Prepared>),
    Reply(Reply, std::result::Result<Value, RpcError>),
    /// A core action finished on the control worker: (requester, action, `{ osd }`).
    ActionDone(Reply, String, std::result::Result<Value, RpcError>),
    Broadcast(String, Value),
    UiCallback(u64, Value),
    /// The user closed a plugin tool window: (owner, view id).
    ViewClosed(u64, String),
    /// The user closed the main window: the player quits (`view::on_main_close`).
    MainClosed,
    /// A button of the fullscreen title strip: (shell generation,
    /// `minimize`, `exit-fullscreen` or `close`).
    TitleStrip(u64, String),
    /// A button of the OSD message: (shell generation, its index).
    OsdAction(u64, usize),
    /// A press on the status strip: (shell generation, `stop`, `open` or
    /// `dismiss`; `host/lead_strip.rs`).
    LeadStrip(u64, String),
    /// A key or press on the approval sheet: (shell generation, `run` or
    /// `decline`; `host/review_sheet.rs`).
    ReviewSheet(u64, String),
    PluginMessage(u64, String, Value),
    Exited(u64),
    /// A control line of the shared runner (`g` 0): (runner, message).
    Runner(shared_runner::RunnerId, Value),
    /// The shared runner's stdout closed (it exited or was killed).
    RunnerExited(shared_runner::RunnerId),
    /// A stopped plugin's process is gone (the plugin reaper): (owner, its
    /// exit code when it had a process of its own).
    Reaped(u64, Option<i32>),
    /// V8 reported a heap allocation failure on the plugin's stderr (it
    /// aborts next): (owner).
    OutOfMemory(u64),
    /// The spawn thread created a plugin's own process, or could not
    /// (`host/supervisor/worker.rs`): (owner, the error); the process waits
    /// in the spawner for the Host to take it.
    Spawned(u64, std::result::Result<(), String>),
    /// The recorder thread wrote a standby's recording, or could not
    /// (`host/supervisor/checkpoint.rs` `Writer`): (owner, the write's
    /// serial, the error).
    Recorded(u64, u64, std::result::Result<(), String>),
    BackendCleaned(u64),
    DialogDone(Reply, Option<Vec<PathBuf>>),
    /// `media.sources.locate`: the playback worker found the reference;
    /// show the file dialog (requester, `{ store, token, name }`).
    SourceLocate(Reply, Value),
    /// That dialog closed: (requester, the same fields, the chosen file).
    SourceLocated(Reply, Value, Option<PathBuf>),
    /// An Option + named key handed to Slint again: (window generation, Slint
    /// key code, pressed, the press's physical key).
    InjectKey(u64, char, Injected, Option<bindings::Physical>),
    /// winit saw these Slint modifier keys released: (window generation, key codes).
    ModifiersReleased(u64, Vec<char>),
    /// `dialogs.confirm` answered: (requester, confirmed, checked).
    ConfirmDone(Reply, bool, bool),
    /// The approval sheet before a `commands.run` answered: (caller, run,
    /// "Don't ask again" checked).
    CommandRunAnswered(Reply, bool, bool),
    /// A file dropped on a window: (window generation, path).
    Dropped(u64, PathBuf),
    /// Files to open (`open_files.rs`); `true`: from another launch, which
    /// also wants the window in front.
    OpenFiles(Vec<PathBuf>, bool),
    /// The options window: (generation, tree index to show, or `None` = close).
    Settings(u64, Option<usize>),
    /// A settings page compiled on the `settings-check` thread: (page
    /// serial, the compile's error).
    SettingsChecked(u64, std::result::Result<(), String>),
    RuntimeFault(u64, String),
    /// A bottom-bar button clicked: (window generation, its token).
    BarButton(u64, String),
    /// The bottom-bar strip's `»`: (window generation, the cell's top-left
    /// corner, logical px).
    BarMore(u64, f64, f64),
    /// The playback worker's answer to `owner.resources` (lazy activation):
    /// (owner, `{ live, resolvers }`).
    OwnerResources(u64, Value),
    /// The position model's report (`playback/position_model.rs`): a
    /// boundary a trigger waits for was reached, or the position jumped.
    Position(crate::playback::position_model::Report),
    /// The primary subtitle track changed (the `subtitleTrack` condition).
    SubtitleTrack(Option<String>),
}

impl Event {
    /// Its kind, and the plugin it came from where it names one (the slow
    /// turn line, `stall.rs`).
    pub(crate) fn label(&self) -> super::stall::Label {
        match self {
            #[cfg(feature = "producer-audio")]
            Self::PhysicalInput(..) => ("PhysicalInput", None),
            Self::WindowInput(..) => ("WindowInput", None),
            Self::Binding(..) => ("Binding", None),
            Self::SidebarResized(..) => ("SidebarResized", None),
            Self::AdjustResized(..) => ("AdjustResized", None),
            Self::Tab(..) => ("Tab", None),
            Self::TabOverflow(..) => ("TabOverflow", None),
            Self::SidebarDetach(..) => ("SidebarDetach", None),
            Self::AdjustDismiss(..) => ("AdjustDismiss", None),
            Self::PipReturn(..) => ("PipReturn", None),
            Self::FloatingClosed(..) => ("FloatingClosed", None),
            Self::MenuAction(..) => ("MenuAction", None),
            Self::Request(owner, _) => ("Request", Some(*owner)),
            Self::Prepared(owner, ..) => ("Request", Some(*owner)),
            Self::Reply(..) => ("Reply", None),
            Self::ActionDone(..) => ("ActionDone", None),
            Self::Broadcast(..) => ("Broadcast", None),
            Self::UiCallback(owner, _) => ("UiCallback", Some(*owner)),
            Self::ViewClosed(owner, _) => ("ViewClosed", Some(*owner)),
            Self::MainClosed => ("MainClosed", None),
            Self::TitleStrip(..) => ("TitleStrip", None),
            Self::OsdAction(..) => ("OsdAction", None),
            Self::LeadStrip(..) => ("LeadStrip", None),
            Self::ReviewSheet(..) => ("ReviewSheet", None),
            Self::PluginMessage(owner, ..) => ("PluginMessage", Some(*owner)),
            Self::Exited(owner) => ("Exited", Some(*owner)),
            Self::Runner(..) => ("Runner", None),
            Self::RunnerExited(..) => ("RunnerExited", None),
            Self::Reaped(owner, _) => ("Reaped", Some(*owner)),
            Self::OutOfMemory(owner) => ("OutOfMemory", Some(*owner)),
            Self::Spawned(owner, _) => ("Spawned", Some(*owner)),
            Self::Recorded(owner, ..) => ("Recorded", Some(*owner)),
            Self::BackendCleaned(owner) => ("BackendCleaned", Some(*owner)),
            Self::DialogDone(..) => ("DialogDone", None),
            Self::SourceLocate(..) => ("SourceLocate", None),
            Self::SourceLocated(..) => ("SourceLocated", None),
            Self::InjectKey(..) => ("InjectKey", None),
            Self::ModifiersReleased(..) => ("ModifiersReleased", None),
            Self::ConfirmDone(..) => ("ConfirmDone", None),
            Self::CommandRunAnswered(..) => ("CommandRunAnswered", None),
            Self::Dropped(..) => ("Dropped", None),
            Self::OpenFiles(..) => ("OpenFiles", None),
            Self::Settings(..) => ("Settings", None),
            Self::SettingsChecked(..) => ("SettingsChecked", None),
            Self::RuntimeFault(owner, _) => ("RuntimeFault", Some(*owner)),
            Self::BarButton(..) => ("BarButton", None),
            Self::BarMore(..) => ("BarMore", None),
            Self::OwnerResources(owner, _) => ("OwnerResources", Some(*owner)),
            Self::Position(_) => ("Position", None),
            Self::SubtitleTrack(_) => ("SubtitleTrack", None),
        }
    }
}

/// Events one tick of the UI thread's loop handles at least, when that many
/// are queued.
pub(crate) const EVENTS_PER_TICK: usize = 128;
/// Past [`EVENTS_PER_TICK`], a tick goes on handling queued events while
/// its handling has taken less than this. One queue carries every event
/// (keys and clicks, playback replies and state, the shared runner's
/// heartbeat, every plugin's requests), so a fixed count capped it at
/// 128 a tick (about 4 600 events/s at the measured 36 ticks/s under load):
/// a plugin's burst of 100 000 cheap requests held everything behind it
/// for 21 s and the shared runner was restarted for a missed heartbeat
/// (stress measurement 2026-11-11). A tick with nothing more queued, or
/// with costly events, is as before.
pub(crate) const DRAIN_BUDGET: Duration = Duration::from_millis(4);

/// The events of one tick that began at `began`, `handled` of them handled
/// already: each queued one, until [`EVENTS_PER_TICK`] are handled and the
/// tick has taken [`DRAIN_BUDGET`]. Returns how many it handled in all.
pub(crate) fn drain(
    receiver: &mpsc::Receiver<Event>,
    began: Instant,
    mut handled: usize,
    mut handle: impl FnMut(Event),
) -> usize {
    while handled < EVENTS_PER_TICK || began.elapsed() < DRAIN_BUDGET {
        let Ok(event) = receiver.try_recv() else {
            break;
        };
        handle(event);
        handled += 1;
    }
    handled
}

// The core's turn (plan step S2e), the same for the windowed and the
// headless driver, in two halves: `Host::core_events` (the events) and
// `Host::core_steps` (the periodic steps), in the driver (`driver.rs`, plan
// step S3a(1)). The windowed driver runs the interface's steps between and
// after them, as before; the headless driver has none. S3a(2) runs both
// halves on the core thread.

/// One periodic step of the core ([`Core::STEPS`]).
pub(crate) struct Step {
    pub(crate) run: fn(&mut Core),
    /// The windowed tick's record names the steps up to here so.
    pub(crate) ends: Option<&'static str>,
    /// Only with a window (the headless driver never ran it).
    pub(crate) windowed: bool,
}

impl Core {
    /// The second half of the core's turn (plan step S2e; S2f(3): `Core`
    /// only): activation, on-demand start, power, diagnostics, packages,
    /// notifications, the timeline, files to open; with a window also the
    /// two steps the windowed tick ran elsewhere before (`window_steps`).
    pub(crate) const STEPS: [Step; 10] = [
        Step {
            run: Core::activation_tick,
            ends: None,
            windowed: false,
        },
        Step {
            run: Core::on_demand_tick,
            ends: None,
            windowed: false,
        },
        Step {
            run: Core::on_demand_check,
            ends: Some("plugin activation"),
            windowed: false,
        },
        Step {
            run: Core::power_tick,
            ends: None,
            windowed: false,
        },
        Step {
            run: Core::diagnostics_tick,
            ends: Some("power and diagnostics"),
            windowed: false,
        },
        Step {
            run: Core::package_tick,
            ends: Some("packages"),
            windowed: false,
        },
        Step {
            run: Core::notify_tick,
            ends: None,
            windowed: false,
        },
        Step {
            run: Core::timeline_tick,
            ends: None,
            windowed: false,
        },
        Step {
            run: Core::open_files_tick,
            ends: Some("notifications, timeline, open files"),
            windowed: false,
        },
        Step {
            run: Core::window_steps,
            ends: Some("bindings and state file"),
            windowed: true,
        },
    ];
    /// The two windowed steps: the binding table follows the registry's
    /// disabled commands (it followed the theme step), and the state file's
    /// debounced write (at the end of the window tick before).
    fn window_steps(&mut self) {
        // The binding table follows the registry's disabled commands (keys
        // bound to them pass on). Only a change is a write: the interface's
        // copy of the table is made again only then (`host/tables.rs`).
        if self.commands.revision() != self.binding_revision {
            self.binding_revision = self.commands.revision();
            let disabled = self.commands.disabled();
            if *self.bindings().disabled() != disabled {
                self.bindings_mut().set_disabled(disabled);
            }
        }
        // The state file is the core's to write (plan step S2c(7)), once a
        // second after its last change.
        let now = self.now_ms();
        self.ui_state.save_if_due(now);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_handles_its_count_then_what_its_time_budget_allows() {
        let (events, receiver) = mpsc::channel();
        for _ in 0..1000 {
            events.send(Event::MainClosed).unwrap();
        }
        // Over the budget already: the count, as before.
        let late = Instant::now() - Duration::from_secs(1);
        assert_eq!(drain(&receiver, late, 0, |_| ()), EVENTS_PER_TICK);
        // One handled before (the headless loop's first): one fewer here.
        let mut here = 0;
        assert_eq!(drain(&receiver, late, 1, |_| here += 1), EVENTS_PER_TICK);
        assert_eq!(here, EVENTS_PER_TICK - 1);
        // Within it: cheap events go on being handled, here all the rest.
        let rest = 1000 - 2 * EVENTS_PER_TICK + 1;
        assert_eq!(drain(&receiver, Instant::now(), 0, |_| ()), rest);
        // Nothing queued: nothing to do.
        assert_eq!(drain(&receiver, Instant::now(), 0, |_| ()), 0);
    }
}
