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
    /// The host's sidebar tab strip (shell or floating window generation): a
    /// tab clicked (its index), the detach/attach button, the close button.
    SidebarTab(u64, usize),
    SidebarDetach(u64),
    SidebarClose(u64),
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
    /// An Option + named key handed to Slint again: (window generation, Slint
    /// key code, pressed, the press's physical key).
    InjectKey(u64, char, Injected, Option<bindings::Physical>),
    /// winit saw these Slint modifier keys released: (window generation, key codes).
    ModifiersReleased(u64, Vec<char>),
    /// `dialogs.confirm` answered: (requester, confirmed, checked).
    ConfirmDone(Reply, bool, bool),
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
    /// The playback worker's answer to `owner.resources` (lazy activation):
    /// (owner, `{ live, resolvers }`).
    OwnerResources(u64, Value),
}
