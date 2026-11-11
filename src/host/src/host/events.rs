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
    /// The host's alert before a `commands.run` answered: (caller, run).
    CommandRunAnswered(Reply, bool),
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
