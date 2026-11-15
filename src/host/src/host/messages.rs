//! The two kinds of message that cross between the core and the UI thread
//! (doc/architecture/core-thread.md, rule R3): [`UiCommand`] (core → UI) and
//! [`UiReport`] (UI → core). Plan step S2a added the types; S2c and S2d
//! route today's code through them while still on one thread (S2c(1):
//! [`UiReport::WindowFacts`], `Host::ui_report`; S2c(3): [`UiCommand::Mount`],
//! [`UiCommand::SetMount`], [`UiReport::MountPreparing`],
//! [`UiReport::MountApplied`] and the `ui.setMount` [`UiReport::Answered`],
//! host/mounts.rs), S3a moves the core to its own thread.
//!
//! Each variant names the rows of the design's method table (§5.1, "row
//! 36") and the rules it serves. A variant is one of two kinds:
//!
//! - **Snapshot**: a whole, immutable value; the receiver replaces what it
//!   had (last value wins, R3). Several in a row collapse to the newest.
//! - **Ordered**: a request (with the [`Reply`] it answers, for the 16
//!   UI-bound method names) or a one-way change; applied in arrival order,
//!   never collapsed (R4: the UI-bound stream keeps execution order).
//!
//! **How a later step coalesces snapshots** (S3a, where the queue between
//! the threads exists). The sender keeps its outgoing messages in one FIFO.
//! Before pushing a snapshot it looks for a pending message that the new
//! one [`replaces`](UiCommand::replaces) (the same snapshot kind) and, if
//! there is one, overwrites that message's value **in place** instead of
//! appending; otherwise it appends. In place, not at the end: an ordered
//! message queued after the old snapshot may rely on it (a panel menu on
//! the commands of a registry snapshot, a key on the binding table, R8),
//! and the newer value is still applied before that message. Ordered
//! messages are always appended. The receiver applies everything in queue
//! order. The same rule holds on the core's side for the UI's snapshots
//! ([`UiReport::replaces`]).
//!
//! **Window facts never go backwards.** Facts reach the core two ways: as
//! the [`UiReport::WindowFacts`] snapshot and inside the handoff of a reply
//! ([`UiReport::Answered`], [`UiReport::MountApplied`]), which is ordered
//! and never collapses. In-place coalescing can therefore put a newer
//! snapshot ahead of an older handoff (queue `[f1, Answered{h}]`, then `f3`
//! overwrites `f1`: the core sees `f3`, then `h`). So every
//! [`WindowFacts`] carries `seq`, from one counter on the UI side that
//! advances for every facts value it sends; the core applies a facts value
//! only when its `seq` is newer than the last one it applied, and
//! otherwise skips the facts (the reply of a handoff is still sent, R13).
//!
//! **What is not here, and why.** The answers of the dialogs that already
//! run on their own threads (`DialogDone`, `ConfirmDone`, `SourceLocated`)
//! reach the core as today's [`Event`]s, not through the UI. The UI's own
//! events (keys, clicks, callbacks: the 26 UI-originated [`Event`]
//! variants) reach the core inside [`UiReport::Event`], as the same
//! values. Not all of them are core work: some only change the interface
//! (`PipReturn` restores the window from picture-in-picture,
//! `SidebarResized` writes the policy's width and the interface state;
//! host/handle.rs:294-311). Which ones the UI handles itself and which it
//! forwards is decided per event in S2c/S2d; this type only carries them.

// S2a: the message types exist before most callers (plan S2c/S2d); the
// tests construct and match every variant but read no field.
#![allow(dead_code)]

use super::*;
use crate::ui::{settings_window::Page, ui_values::Prepared};

/// Core → UI: state snapshots, the UI-bound requests and the one-way
/// interface changes of the core's requests (design §5.1, column "UI work").
pub(crate) enum UiCommand {
    /// A whole value the UI shows; last value wins (R3).
    Snapshot(UiSnapshot),
    /// `ui.open`, `ui.set`, `ui.show`, `ui.close` (rows 36–39; stream B,
    /// R5). The UI answers with [`UiReport::Answered`]; after an
    /// `ui.open` the core runs `ui_finished` (readiness, R12, note 5).
    View(ViewOp),
    /// `ui.mount`, `ui.unmount` (rows 40, 42; stream B; K6). Today's
    /// queued change, unchanged: the UI batches (the 32 ms window) and
    /// compiles, and reports [`UiReport::MountPreparing`] and
    /// [`UiReport::MountApplied`]. The start-up gate (held mounts, note 6)
    /// stays on the core, before this is sent.
    Mount(MountChange),
    /// `ui.setMount` (row 41; stream B). Applies to the mount that is ready
    /// when it arrives at the UI, else `missing` (design §5.1, the
    /// `ui.setMount` note).
    SetMount(SetMount),
    /// `settings.*` (rows 83a–83d; K7).
    Settings(SettingsOp),
    /// Window operations (rows 34, 71, 72, 74, 90; `OpenFiles` with
    /// `front`).
    Window(WindowOp),
    /// A file dialog: `dialogs.open` (row 47) and `media.sources.locate`
    /// (row 65c, note 9); stream B. Its answer comes back on the dialog's
    /// thread as today (`Event::DialogDone`, `Event::SourceLocated`).
    Dialog(DialogRequest),
    /// The system confirmation of `dialogs.confirm` (row 48; stream B). Its
    /// answer comes back as today (`Event::ConfirmDone`).
    Confirm(ConfirmRequest),
    /// The approval sheet in the main window: `commands.run` with a plan
    /// that asks (row 96), and a main-window `dialogs.confirm` (row 48);
    /// stream B. The user's answer comes back as a UI event
    /// (`Event::ReviewSheet`, `Event::CommandRunAnswered`).
    Review(ReviewRequest),
    /// An OSD message or its end (row 66; the core's refusals of rows 23
    /// and 25, note 3; `host.run` results, note 4).
    Osd(OsdOp),
    /// `ui.menu`: a panel's context menu (row 44, note 7). Answered by the
    /// core at once; the UI drops it when its window generation is gone.
    PanelMenu(PanelMenu),
    /// `ui.interaction` (row 43, answered by the core at once, R5/K13),
    /// `layout.set` (row 46, stream B, answered after the UI applied it,
    /// R13), and the host's own column switches.
    Layout(LayoutOp),
    /// `palette.lead.set` / `.clear` (rows 97, 98).
    Lead(LeadOp),
    /// The host's Command Palette command (`host/palette`, reached by a
    /// key or a menu through `host_command`): open the palette in the main
    /// window.
    OpenPalette,
    /// A plugin stopped: its views, mounts, OSD buttons, floating window
    /// and shell properties go (K3; `stop_run`). `park`: it went to
    /// standby, its sidebar tabs stay as recorded placeholders.
    RevokeOwner { owner: u64, park: bool },
    /// A woken or restarted plugin's interface moves to its new owner
    /// generation (panels, views, settings pages).
    RekeyOwner { from: u64, to: u64 },
}

/// The core's tables and values the UI shows. Each is the whole value at
/// one moment; a newer one replaces an older one (R3).
pub(crate) enum UiSnapshot {
    /// The command registry as menus, the palette, the bar, the tab keys
    /// and the video click read it (rows 21, 31–33; R8): a copy of its
    /// commands and held owners at one revision, with the menu tree built
    /// from it once (`commands::Registry::view`). Plan step S2c(2) widened
    /// it from the menu tree alone: the bar, the panel menus and the
    /// palette read commands the tree leaves out. Copied once per revision
    /// and shared (K11; `host/tables.rs`).
    Registry(Arc<commands::View>),
    /// The binding table the key callbacks look up synchronously (rows 31,
    /// 33, 76, 77; R8). Copied once per change and shared.
    Bindings(Arc<bindings::Table>),
    /// The info panels of `osd.panel` (row 67), by owner.
    InfoPanels(Arc<BTreeMap<u64, String>>),
    /// The interface theme of `appearance.set` (row 84).
    Theme(String),
    /// What `source.state` shows (plan S2d): no video, the empty phase,
    /// the window title.
    Source(SourceShown),
    /// `window.setCursorAutoHide` (row 73): hide the pointer while playing
    /// (`true`) or only in fullscreen.
    CursorAutoHide(bool),
}

/// See [`UiSnapshot::Source`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourceShown {
    pub(crate) no_video: bool,
    pub(crate) empty: bool,
    pub(crate) title: String,
}

/// The four tool-window requests: the plugin's view `id` and what to do.
pub(crate) struct ViewOp {
    pub(crate) reply: Reply,
    pub(crate) id: String,
    pub(crate) kind: ViewKind,
}

pub(crate) enum ViewKind {
    /// `ui.open`: the package file, checked by the core, and the initial
    /// properties and callbacks.
    Open {
        file: PathBuf,
        properties: BTreeMap<String, Value>,
        callbacks: Vec<String>,
    },
    /// `ui.set`: the request's parameters and the large array its reader
    /// thread converted, if any (`Event::Prepared`).
    Set {
        params: Value,
        prepared: Option<Box<Prepared>>,
    },
    /// `ui.show`.
    Show { params: Value },
    /// `ui.close`.
    Close,
}

/// `ui.setMount`: the property of the plugin's panel `key` (`panel_key`).
pub(crate) struct SetMount {
    pub(crate) reply: Reply,
    pub(crate) key: u64,
    pub(crate) params: Value,
    pub(crate) prepared: Option<Box<Prepared>>,
}

pub(crate) enum SettingsOp {
    /// `settings.register` of a page whose compile check passed (row 83a,
    /// after the `settings-check` thread): an open options window shows it.
    Added(Box<Page>),
    /// `settings.register` again with the same files (row 83a): only what
    /// changed is written into an open window.
    Updated { before: Box<Page>, page: Box<Page> },
    /// `settings.unregister` (row 83b): an open window is rebuilt.
    Removed { owner: u64, id: String },
    /// `settings.set` (row 83c; stream B): answered after the UI wrote it.
    Set {
        reply: Reply,
        params: Value,
        prepared: Option<Box<Prepared>>,
    },
    /// `settings.show` (row 83d; stream B): answered with the window's
    /// open error.
    Show { reply: Reply, params: Value },
}

pub(crate) enum WindowOp {
    /// `window.setVideoArea` (row 71; stream B). The UI fits the window and
    /// reports [`UiReport::AreaFitted`] (or an error as
    /// [`UiReport::Answered`]); the core answers when the window facts show
    /// the size or after its 1 s deadline (R13).
    VideoArea {
        reply: Reply,
        width: f64,
        height: f64,
    },
    /// `window.setPictureInPicture { on }` (row 74; stream B). Answered from
    /// the PiP state machine through [`UiReport::Answered`], whose handoff
    /// carries the facts with the new `active` flag (R13).
    Pip { reply: Reply, on: bool },
    /// `window.setAlwaysOnTop` (row 72): every ordinary window at the
    /// floating level, or not. Answered by the core at once.
    OnTop(bool),
    /// The shell window in or out of fullscreen: `host.run fullscreen`
    /// (row 34, note 4) and the host's `fullscreen`, `enter-fullscreen`,
    /// `exit-fullscreen` commands (`window_command`).
    Fullscreen(FullscreenChange),
    /// `OpenFiles` from another launch: the main window to the front.
    Raise,
    /// The player ends its event loop: `rendering.restart` (row 90, after
    /// the reply) and the host's `quit` command (`host_command`: the GPU
    /// waits released, handoffs stopped, the interface state flushed).
    Quit,
}

/// See [`WindowOp::Fullscreen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FullscreenChange {
    Enter,
    Exit,
    /// In when windowed, out when fullscreen.
    Toggle,
}

pub(crate) enum DialogPurpose {
    /// `dialogs.open`: its parameters.
    Open(Value),
    /// `media.sources.locate`: `{ store, token, name }` of the reference.
    Locate(Value),
}

pub(crate) struct DialogRequest {
    pub(crate) reply: Reply,
    pub(crate) purpose: DialogPurpose,
}

/// `dialogs.confirm` as a system confirmation. Admission (headless,
/// `dialog_busy`, the rate checks) stays on the core (note 8).
pub(crate) struct ConfirmRequest {
    pub(crate) reply: Reply,
    pub(crate) params: Value,
}

pub(crate) enum ReviewKind {
    /// `dialogs.confirm` shown in the main window.
    Confirm(Value),
    /// `commands.run` whose plan asks: the request's parameters.
    CommandRun(Value),
}

pub(crate) struct ReviewRequest {
    pub(crate) reply: Reply,
    pub(crate) kind: ReviewKind,
}

pub(crate) enum OsdOp {
    /// Show one line, from a plugin (`None`: the player), with the buttons
    /// of `actions`.
    Show {
        text: String,
        from: Option<u64>,
        actions: Option<OsdButtons>,
    },
    /// The message fades out (`reason` for `VESTRA_DEBUG_OSD`).
    Hide { reason: String },
}

/// An OSD message's buttons: the plugin, its command ids and their titles.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OsdButtons {
    pub(crate) owner: u64,
    pub(crate) commands: Vec<String>,
    pub(crate) titles: Vec<String>,
}

/// A panel menu: the panel `key`, its command ids (`None`: a separator),
/// and the request's parameters (the origin).
pub(crate) struct PanelMenu {
    pub(crate) owner: u64,
    pub(crate) key: u64,
    pub(crate) ids: Vec<Option<String>>,
    pub(crate) params: Value,
}

/// A change to the panel layout. Ordered, not a snapshot: the panel policy
/// is written by the UI as well (pointer bands, activity, tabs clicked), so
/// a whole policy sent by the core would overwrite the UI's own writes. The
/// UI reports the resulting tab order with its window facts (R9).
pub(crate) enum LayoutOp {
    /// `ui.interaction` (row 43): the locks of panel `key`. The UI also
    /// marks activity on the policy (`policy.activity`, ui_mount.rs:724).
    Locks {
        key: u64,
        locks: window_state::Locks,
    },
    /// `layout.set` (row 46, stream B): the request's parameters, checked
    /// by the core. Today it runs a layout pass before answering so that a
    /// following `layout.state` sees the change (ui_mount.rs:888-889; the
    /// SDK reads right after it): the UI answers with
    /// [`UiReport::Answered`] whose handoff carries the facts after the
    /// pass (R13).
    Set { reply: Reply, params: Value },
    /// The host's Sidebar and Adjustments commands (`host_command`,
    /// `toggle_sidebar` / `toggle_adjust`): switch the column's current
    /// panel on or off.
    Toggle(crate::ui::sidebar_tabs::Column),
}

pub(crate) enum LeadOp {
    /// `palette.lead.set`: the plugin's parameters.
    Set { owner: u64, params: Value },
    /// `palette.lead.clear`.
    Clear { owner: u64 },
}

/// UI → core: results of UI-bound requests, the UI's facts (snapshots) and
/// the UI's own events.
pub(crate) enum UiReport {
    /// The window facts (R9, K5): last value wins. The core evaluates
    /// conditions and triggers from them and broadcasts `window.state`;
    /// `layout.state`, `window.state` and `ui.menu`'s refusals read them
    /// (rows 44, 45, 109; notes 1, 7). It also answers the pending
    /// `window.setVideoArea` whose size they show (row 71, R13).
    WindowFacts(Box<WindowFacts>),
    /// The displays `app.system` reports (row 113, note 10): last value
    /// wins.
    Displays(Value),
    /// A video frame was shown at this time (`power_tick`, `power` on the
    /// core, K10): last value wins.
    FrameShown(Instant),
    /// The UI's result of an ordered request with a reply: the view ops
    /// (rows 36–39), `ui.setMount` (41), `settings.set` / `.show` (83c,
    /// 83d), picture-in-picture (74), `layout.set` (46), and the errors
    /// of `window.setVideoArea` (71). The core handles it in R13's order:
    /// apply the `handoff` (only if its `seq` is newer than the facts it
    /// has, see the module doc), send the reply, broadcast what changed.
    Answered {
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
        handoff: Option<Box<WindowFacts>>,
    },
    /// `window.setVideoArea` was fitted: the video size it waits for and
    /// the reply it will carry. The core keeps it with its deadline
    /// (request time + 1000 ms) and answers from [`Self::WindowFacts`].
    /// On receipt it first checks the facts it already has: they may show
    /// the size already (a fact that arrived before this report), and then
    /// it answers at once instead of waiting out the deadline.
    AreaFitted {
        reply: Reply,
        video: (f64, f64),
        result: Value,
    },
    /// Mount changes left the batching queue: the core sends their
    /// `ui.preparing` (`phase: preparing`) lines, in this order. Only
    /// changes not announced before (a failed batch's changes are put back
    /// announced).
    MountPreparing(Vec<(Reply, u64)>),
    /// A batch of mount changes was applied (rows 40, 42; K6, R12, R13).
    MountApplied(MountApplied),
    /// The user acted for a plugin in this window (`origins`: where its
    /// `dialogs.confirm` goes), e.g. the tool window an `ui.open` showed.
    Acted { owner: u64, origin: origins::Origin },
    /// One of the 26 UI-originated events, unchanged.
    Event(UiEvent),
}

/// What the UI knows of its windows, whole (R9). The R13 handoff data of
/// [`UiReport::Answered`] and [`UiReport::MountApplied`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WindowFacts {
    /// From one counter on the UI side, advanced for every facts value it
    /// sends (snapshot or handoff): the core applies only a newer one.
    pub(crate) seq: u64,
    /// Today's `window_facts` value, whole (size, fullscreen, focus, video
    /// area, slots, menu state).
    pub(crate) facts: Value,
    /// The shell's generation; `None` without a shell (`host.run
    /// fullscreen` answers from it, note 4).
    pub(crate) shell: Option<u64>,
    /// The shell's compiled definition has writable `menu-origin-x`/`-y`:
    /// `ui.menu`'s property error is decided from it (note 7).
    pub(crate) menu_origin: bool,
    /// `window.state.pictureInPicture`: offered here, and the system window
    /// showing (R13).
    pub(crate) pip_supported: bool,
    pub(crate) pip_active: bool,
    /// The panel policy's tab order of the sidebar and the adjust column
    /// (`layout.state`; the layout part of R13's handoff).
    pub(crate) sidebar_panels: Vec<u64>,
    pub(crate) adjust_panels: Vec<u64>,
}

/// See [`UiReport::MountApplied`]: the panel keys of the batch, its one
/// result, how long it took, and the facts after it (R13: the core updates
/// its mount registry and layout, then answers, then broadcasts). Plan
/// step S2c(3) sends one per change of a batch, each right after that
/// change's restart line, so the interleaving with the interface's work is
/// today's (`Host::prepare_mount_tick`).
pub(crate) struct MountApplied {
    pub(crate) batch: Vec<(Reply, u64)>,
    pub(crate) result: std::result::Result<Value, RpcError>,
    pub(crate) elapsed_ms: f64,
    pub(crate) handoff: Option<Box<WindowFacts>>,
    /// The keys of `batch` that are placeholder tabs replayed from a
    /// recording: their plugin, in standby, is told nothing (no
    /// `ui.preparing`); a failure is still announced as `host.error`.
    pub(crate) placeholders: Vec<u64>,
}

/// One of today's UI-originated [`Event`]s (study §2.1: keys, clicks,
/// window callbacks, view callbacks). Only those can be wrapped.
pub(crate) struct UiEvent(Event);

impl UiEvent {
    /// The event, if the UI thread is where it comes from; else it back.
    pub(crate) fn new(event: Event) -> std::result::Result<Self, Event> {
        if from_ui(&event) {
            Ok(Self(event))
        } else {
            Err(event)
        }
    }
    pub(crate) fn into_event(self) -> Event {
        self.0
    }
}

/// Whether the UI thread produces `event` (Slint callbacks, winit hooks).
/// Every variant is listed, so a new one must be classified here.
fn from_ui(event: &Event) -> bool {
    match event {
        #[cfg(feature = "producer-audio")]
        Event::PhysicalInput(..) => true,
        Event::WindowInput(..)
        | Event::Binding(..)
        | Event::SidebarResized(..)
        | Event::AdjustResized(..)
        | Event::Tab(..)
        | Event::TabOverflow(..)
        | Event::SidebarDetach(..)
        | Event::AdjustDismiss(..)
        | Event::PipReturn(..)
        | Event::FloatingClosed(..)
        | Event::MenuAction(..)
        | Event::UiCallback(..)
        | Event::ViewClosed(..)
        | Event::MainClosed
        | Event::TitleStrip(..)
        | Event::OsdAction(..)
        | Event::LeadStrip(..)
        | Event::ReviewSheet(..)
        | Event::InjectKey(..)
        | Event::ModifiersReleased(..)
        | Event::Dropped(..)
        | Event::Settings(..)
        | Event::BarButton(..)
        | Event::BarMore(..)
        | Event::CommandRunAnswered(..) => true,
        Event::Request(..)
        | Event::Prepared(..)
        | Event::Reply(..)
        | Event::ActionDone(..)
        | Event::Broadcast(..)
        | Event::PluginMessage(..)
        | Event::Exited(..)
        | Event::Runner(..)
        | Event::RunnerExited(..)
        | Event::Reaped(..)
        | Event::OutOfMemory(..)
        | Event::Spawned(..)
        | Event::Recorded(..)
        | Event::BackendCleaned(..)
        | Event::DialogDone(..)
        | Event::SourceLocate(..)
        | Event::SourceLocated(..)
        | Event::ConfirmDone(..)
        | Event::OpenFiles(..)
        | Event::SettingsChecked(..)
        | Event::RuntimeFault(..)
        | Event::OwnerResources(..)
        | Event::Position(..)
        | Event::SubtitleTrack(..) => false,
    }
}

impl UiCommand {
    /// Whether this replaces `older` in a queue: both are snapshots of the
    /// same kind (see the module's coalescing rule).
    pub(crate) fn replaces(&self, older: &Self) -> bool {
        match (self, older) {
            (Self::Snapshot(new), Self::Snapshot(old)) => {
                std::mem::discriminant(new) == std::mem::discriminant(old)
            }
            _ => false,
        }
    }
}

impl WindowFacts {
    /// Whether the core applies these facts, having applied `applied`
    /// last (`None`: none yet): only a newer value (module doc).
    pub(crate) fn newer_than(&self, applied: Option<u64>) -> bool {
        applied.is_none_or(|seq| self.seq > seq)
    }
}

impl UiReport {
    /// Whether this replaces `older` in a queue: both are UI facts of the
    /// same kind. `Answered`'s and `MountApplied`'s handoff facts are
    /// ordered with their reply and never collapse.
    pub(crate) fn replaces(&self, older: &Self) -> bool {
        matches!(
            (self, older),
            (Self::WindowFacts(_), Self::WindowFacts(_))
                | (Self::Displays(_), Self::Displays(_))
                | (Self::FrameShown(_), Self::FrameShown(_))
        )
    }
}

/// Both kinds cross threads from S3a on (R3): checked now, at compile time
/// (design §8, the `Send` check).
const _: () = {
    const fn send<T: Send>() {}
    send::<UiCommand>();
    send::<UiSnapshot>();
    send::<UiReport>();
    send::<UiEvent>();
    send::<WindowFacts>();
    send::<MountApplied>();
};

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: u64) -> Reply {
        Reply { owner: 7, id }
    }

    fn facts() -> Box<WindowFacts> {
        Box::new(WindowFacts {
            seq: 1,
            facts: json!({"fullscreen":false}),
            shell: Some(1),
            menu_origin: true,
            pip_supported: true,
            pip_active: false,
            sidebar_panels: vec![28],
            adjust_panels: vec![],
        })
    }

    fn mount_change() -> MountChange {
        MountChange {
            reply: reply(3),
            key: panel_key(7, composition::Slot::Bottom),
            contribution: None,
            announced: false,
        }
    }

    fn page() -> Box<Page> {
        Box::new(Page {
            owner: 7,
            manifest: "vestra.example".into(),
            id: "main".into(),
            category: 0,
            title: "Example".into(),
            order: 0,
            serial: 1,
            file: PathBuf::from("settings.slint"),
            component: "Page".into(),
            global: "Settings".into(),
            properties: BTreeMap::new(),
        })
    }

    /// One of each, in declaration order.
    fn every_snapshot() -> Vec<UiSnapshot> {
        vec![
            UiSnapshot::Registry(Arc::new(commands::Registry::new().view())),
            UiSnapshot::Bindings(Arc::new(bindings::Table::default())),
            UiSnapshot::InfoPanels(Arc::new(BTreeMap::from([(7, "info".into())]))),
            UiSnapshot::Theme("dark".into()),
            UiSnapshot::Source(SourceShown {
                no_video: true,
                empty: false,
                title: "Vestra Player".into(),
            }),
            UiSnapshot::CursorAutoHide(true),
        ]
    }

    fn every_command() -> Vec<UiCommand> {
        let mut all: Vec<UiCommand> = every_snapshot()
            .into_iter()
            .map(UiCommand::Snapshot)
            .collect();
        all.extend([
            UiCommand::View(ViewOp {
                reply: reply(1),
                id: "w".into(),
                kind: ViewKind::Open {
                    file: PathBuf::from("w.slint"),
                    properties: BTreeMap::new(),
                    callbacks: vec!["clicked".into()],
                },
            }),
            UiCommand::View(ViewOp {
                reply: reply(2),
                id: "w".into(),
                kind: ViewKind::Set {
                    params: json!({"property":"a","value":1}),
                    prepared: None,
                },
            }),
            UiCommand::View(ViewOp {
                reply: reply(3),
                id: "w".into(),
                kind: ViewKind::Show { params: json!({}) },
            }),
            UiCommand::View(ViewOp {
                reply: reply(4),
                id: "w".into(),
                kind: ViewKind::Close,
            }),
            UiCommand::Mount(mount_change()),
            UiCommand::SetMount(SetMount {
                reply: reply(5),
                key: 28,
                params: json!({"property":"a","value":1}),
                prepared: None,
            }),
            UiCommand::Settings(SettingsOp::Added(page())),
            UiCommand::Settings(SettingsOp::Updated {
                before: page(),
                page: page(),
            }),
            UiCommand::Settings(SettingsOp::Removed {
                owner: 7,
                id: "main".into(),
            }),
            UiCommand::Settings(SettingsOp::Set {
                reply: reply(6),
                params: json!({}),
                prepared: None,
            }),
            UiCommand::Settings(SettingsOp::Show {
                reply: reply(7),
                params: json!({}),
            }),
            UiCommand::Window(WindowOp::VideoArea {
                reply: reply(8),
                width: 640.,
                height: 360.,
            }),
            UiCommand::Window(WindowOp::Pip {
                reply: reply(9),
                on: true,
            }),
            UiCommand::Window(WindowOp::OnTop(true)),
            UiCommand::Window(WindowOp::Fullscreen(FullscreenChange::Enter)),
            UiCommand::Window(WindowOp::Fullscreen(FullscreenChange::Exit)),
            UiCommand::Window(WindowOp::Fullscreen(FullscreenChange::Toggle)),
            UiCommand::Window(WindowOp::Raise),
            UiCommand::Window(WindowOp::Quit),
            UiCommand::Dialog(DialogRequest {
                reply: reply(10),
                purpose: DialogPurpose::Open(json!({})),
            }),
            UiCommand::Dialog(DialogRequest {
                reply: reply(11),
                purpose: DialogPurpose::Locate(json!({"name":"a.mkv"})),
            }),
            UiCommand::Confirm(ConfirmRequest {
                reply: reply(12),
                params: json!({}),
            }),
            UiCommand::Review(ReviewRequest {
                reply: reply(13),
                kind: ReviewKind::Confirm(json!({})),
            }),
            UiCommand::Review(ReviewRequest {
                reply: reply(14),
                kind: ReviewKind::CommandRun(json!({})),
            }),
            UiCommand::Osd(OsdOp::Show {
                text: "hello".into(),
                from: Some(7),
                actions: Some(OsdButtons {
                    owner: 7,
                    commands: vec!["undo".into()],
                    titles: vec!["Undo".into()],
                }),
            }),
            UiCommand::Osd(OsdOp::Hide {
                reason: "replaced".into(),
            }),
            UiCommand::PanelMenu(PanelMenu {
                owner: 7,
                key: 28,
                ids: vec![Some("a".into()), None],
                params: json!({}),
            }),
            UiCommand::Layout(LayoutOp::Locks {
                key: 28,
                locks: window_state::Locks {
                    focused: true,
                    dragging: false,
                    popup_open: false,
                },
            }),
            UiCommand::Layout(LayoutOp::Set {
                reply: reply(15),
                params: json!({"slot":"sidebar"}),
            }),
            UiCommand::Layout(LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Sidebar)),
            UiCommand::Layout(LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Adjust)),
            UiCommand::OpenPalette,
            UiCommand::Lead(LeadOp::Set {
                owner: 7,
                params: json!({}),
            }),
            UiCommand::Lead(LeadOp::Clear { owner: 7 }),
            UiCommand::RevokeOwner {
                owner: 7,
                park: true,
            },
            UiCommand::RekeyOwner { from: 7, to: 8 },
        ]);
        all
    }

    /// The 26 UI-originated events.
    fn ui_events() -> Vec<Event> {
        let column = crate::ui::sidebar_tabs::Column::Sidebar;
        let mut all = vec![
            Event::WindowInput(1, "Mouse:LeftClick".into()),
            Event::Binding(1, 7, "play".into()),
            Event::SidebarResized(1, 300.),
            Event::AdjustResized(1, 300.),
            Event::Tab(1, column, 0),
            Event::TabOverflow(1, column, 0., 0.),
            Event::SidebarDetach(1),
            Event::AdjustDismiss(1),
            Event::PipReturn(1),
            Event::FloatingClosed(1),
            Event::MenuAction(1, MenuAction::Reset),
            Event::UiCallback(7, json!({})),
            Event::ViewClosed(7, "w".into()),
            Event::MainClosed,
            Event::TitleStrip(1, "close".into()),
            Event::OsdAction(1, 0),
            Event::LeadStrip(1, "stop".into()),
            Event::ReviewSheet(1, "run".into()),
            Event::InjectKey(1, 'a', Injected::Pressed, None),
            Event::ModifiersReleased(1, vec!['a']),
            Event::Dropped(1, PathBuf::from("a.mkv")),
            Event::Settings(1, None),
            Event::BarButton(1, "a".into()),
            Event::BarMore(1, 0., 0.),
            Event::CommandRunAnswered(reply(1), true, false),
        ];
        // The 26th, behind its feature.
        #[cfg(feature = "producer-audio")]
        all.push(Event::PhysicalInput(1, json!({})));
        let expected = if cfg!(feature = "producer-audio") {
            26
        } else {
            25
        };
        assert_eq!(all.len(), expected);
        all
    }

    fn every_report() -> Vec<UiReport> {
        let mut all = vec![
            UiReport::WindowFacts(facts()),
            UiReport::Displays(json!([])),
            UiReport::FrameShown(Instant::now()),
            UiReport::Answered {
                reply: reply(1),
                result: Ok(Value::Null),
                handoff: None,
            },
            UiReport::Answered {
                reply: reply(9),
                result: Err(RpcError::new("unavailable", "no window")),
                handoff: Some(facts()),
            },
            UiReport::AreaFitted {
                reply: reply(8),
                video: (640., 360.),
                result: json!({"width":640,"height":360,"clamped":false}),
            },
            UiReport::MountPreparing(vec![(reply(3), 28)]),
            UiReport::MountApplied(MountApplied {
                batch: vec![(reply(3), 28)],
                result: Ok(Value::Null),
                elapsed_ms: 1.5,
                handoff: Some(facts()),
                placeholders: vec![],
            }),
            UiReport::Acted {
                owner: 7,
                origin: origins::Origin::View("w".into()),
            },
        ];
        for event in ui_events() {
            all.push(UiReport::Event(
                UiEvent::new(event).ok().expect("a UI event"),
            ));
        }
        all
    }

    /// The name of each variant, by an exhaustive match (a new variant
    /// fails to compile until it is named here and constructed above).
    fn command_name(command: &UiCommand) -> &'static str {
        match command {
            UiCommand::Snapshot(snapshot) => match snapshot {
                UiSnapshot::Registry(_) => "Registry",
                UiSnapshot::Bindings(_) => "Bindings",
                UiSnapshot::InfoPanels(_) => "InfoPanels",
                UiSnapshot::Theme(_) => "Theme",
                UiSnapshot::Source(_) => "Source",
                UiSnapshot::CursorAutoHide(_) => "CursorAutoHide",
            },
            UiCommand::View(op) => match op.kind {
                ViewKind::Open { .. } => "View.Open",
                ViewKind::Set { .. } => "View.Set",
                ViewKind::Show { .. } => "View.Show",
                ViewKind::Close => "View.Close",
            },
            UiCommand::Mount(_) => "Mount",
            UiCommand::SetMount(_) => "SetMount",
            UiCommand::Settings(op) => match op {
                SettingsOp::Added(_) => "Settings.Added",
                SettingsOp::Updated { .. } => "Settings.Updated",
                SettingsOp::Removed { .. } => "Settings.Removed",
                SettingsOp::Set { .. } => "Settings.Set",
                SettingsOp::Show { .. } => "Settings.Show",
            },
            UiCommand::Window(op) => match op {
                WindowOp::VideoArea { .. } => "Window.VideoArea",
                WindowOp::Pip { .. } => "Window.Pip",
                WindowOp::OnTop(_) => "Window.OnTop",
                WindowOp::Fullscreen(FullscreenChange::Enter) => "Window.Fullscreen.Enter",
                WindowOp::Fullscreen(FullscreenChange::Exit) => "Window.Fullscreen.Exit",
                WindowOp::Fullscreen(FullscreenChange::Toggle) => "Window.Fullscreen.Toggle",
                WindowOp::Raise => "Window.Raise",
                WindowOp::Quit => "Window.Quit",
            },
            UiCommand::Dialog(request) => match request.purpose {
                DialogPurpose::Open(_) => "Dialog.Open",
                DialogPurpose::Locate(_) => "Dialog.Locate",
            },
            UiCommand::Confirm(_) => "Confirm",
            UiCommand::Review(request) => match request.kind {
                ReviewKind::Confirm(_) => "Review.Confirm",
                ReviewKind::CommandRun(_) => "Review.CommandRun",
            },
            UiCommand::Osd(op) => match op {
                OsdOp::Show { .. } => "Osd.Show",
                OsdOp::Hide { .. } => "Osd.Hide",
            },
            UiCommand::PanelMenu(_) => "PanelMenu",
            UiCommand::Layout(op) => match op {
                LayoutOp::Locks { .. } => "Layout.Locks",
                LayoutOp::Set { .. } => "Layout.Set",
                LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Sidebar) => {
                    "Layout.Toggle.Sidebar"
                }
                LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Adjust) => "Layout.Toggle.Adjust",
            },
            UiCommand::Lead(op) => match op {
                LeadOp::Set { .. } => "Lead.Set",
                LeadOp::Clear { .. } => "Lead.Clear",
            },
            UiCommand::OpenPalette => "OpenPalette",
            UiCommand::RevokeOwner { .. } => "RevokeOwner",
            UiCommand::RekeyOwner { .. } => "RekeyOwner",
        }
    }

    fn report_name(report: &UiReport) -> &'static str {
        match report {
            UiReport::WindowFacts(_) => "WindowFacts",
            UiReport::Displays(_) => "Displays",
            UiReport::FrameShown(_) => "FrameShown",
            UiReport::Answered { .. } => "Answered",
            UiReport::AreaFitted { .. } => "AreaFitted",
            UiReport::MountPreparing(_) => "MountPreparing",
            UiReport::MountApplied(_) => "MountApplied",
            UiReport::Acted { .. } => "Acted",
            UiReport::Event(event) => event.0.label().0,
        }
    }

    #[test]
    fn every_command_is_constructed_and_matched_once() {
        let names: Vec<_> = every_command().iter().map(command_name).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        assert_eq!(names.len(), 6 + 36);
    }

    #[test]
    fn every_report_is_constructed_and_matched() {
        let names: Vec<_> = every_report().iter().map(report_name).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        // `Answered` twice (a result and an error with its handoff).
        assert_eq!(unique.len(), names.len() - 1, "{names:?}");
        assert_eq!(names.len(), 9 + ui_events().len());
    }

    #[test]
    fn only_ui_originated_events_are_wrapped_and_come_back_unchanged() {
        for event in ui_events() {
            let label = event.label();
            let wrapped = UiEvent::new(event).ok().expect("a UI event");
            assert_eq!(wrapped.into_event().label(), label);
        }
        let core = [
            Event::Exited(7),
            Event::Reply(reply(1), Ok(Value::Null)),
            Event::DialogDone(reply(1), None),
            Event::ConfirmDone(reply(1), true, false),
            Event::SourceLocated(reply(1), json!({}), None),
            Event::OpenFiles(vec![], true),
            Event::SettingsChecked(1, Ok(())),
            Event::SubtitleTrack(None),
        ];
        for event in core {
            let label = event.label();
            let refused = UiEvent::new(event).err().expect("not a UI event");
            assert_eq!(refused.label(), label);
        }
    }

    #[test]
    fn a_snapshot_replaces_only_a_snapshot_of_its_own_kind() {
        let snapshots = every_command();
        for (i, new) in snapshots.iter().enumerate() {
            for (j, old) in snapshots.iter().enumerate() {
                let both_snapshots =
                    matches!((new, old), (UiCommand::Snapshot(_), UiCommand::Snapshot(_)));
                assert_eq!(
                    new.replaces(old),
                    both_snapshots && i == j,
                    "{} over {}",
                    command_name(new),
                    command_name(old)
                );
            }
        }
        // A newer value of the same kind replaces the older.
        let older = UiCommand::Snapshot(UiSnapshot::Theme("light".into()));
        let newer = UiCommand::Snapshot(UiSnapshot::Theme("dark".into()));
        assert!(newer.replaces(&older));
    }

    #[test]
    fn facts_are_applied_only_when_newer() {
        // [f1, Answered{h = 2}] with f3 coalesced over f1: f3, then h.
        let (h, f3) = (
            WindowFacts { seq: 2, ..*facts() },
            WindowFacts { seq: 3, ..*facts() },
        );
        let mut applied = None;
        assert!(f3.newer_than(applied));
        applied = Some(f3.seq);
        assert!(!h.newer_than(applied), "the older handoff is skipped");
        assert!(!f3.newer_than(applied), "the same value again too");
        assert!(WindowFacts { seq: 4, ..*facts() }.newer_than(applied));
    }

    #[test]
    fn only_the_ui_facts_replace_each_other() {
        let reports = every_report();
        for (i, new) in reports.iter().enumerate() {
            for (j, old) in reports.iter().enumerate() {
                let facts = i < 3 && i == j;
                assert_eq!(
                    new.replaces(old),
                    facts,
                    "{} over {}",
                    report_name(new),
                    report_name(old)
                );
            }
        }
    }
}
