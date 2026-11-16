//! The two kinds of message that cross between the core and the UI thread
//! (doc/architecture/core-thread.md, rule R3): [`UiCommand`] (core → UI) and
//! [`UiReport`] (UI → core). Plan step S2a added the types; S2c and S2d
//! route today's code through them while still on one thread (S2c(1):
//! [`UiReport::WindowFacts`], `Host::ui_report`; S2c(3): [`UiCommand::Mount`],
//! [`UiCommand::SetMount`], [`UiReport::MountPreparing`],
//! [`UiReport::MountApplied`] and the `ui.setMount` [`UiReport::Answered`],
//! host/mounts.rs; S2c(4): [`UiCommand::Settings`] and the `settings.set` /
//! `.show` [`UiReport::Answered`], ui/settings_window.rs; S2c(5):
//! [`UiCommand::Layout`], [`UiCommand::Lead`], [`UiCommand::Dialog`],
//! [`UiCommand::Confirm`], [`UiCommand::Review`], [`UiReport::Acted`], the
//! `layout.set` and sheet-confirmation [`UiReport::Answered`] and the tab
//! order in [`WindowFacts`]; S2c(6): [`UiCommand::ResetWrites`]; S2c(7):
//! [`UiSnapshot::LastSource`], [`UiSnapshot::InfoPanels`],
//! [`UiSnapshot::Theme`], [`UiSnapshot::CursorAutoHide`],
//! [`WindowOp::OnTop`], [`UiReport::Remembered`] and [`UiReport::Drop`];
//! S2d(1): [`UiCommand::View`] with the view requests' [`UiReport::Answered`],
//! [`WindowOp::VideoArea`] with [`UiReport::AreaFitted`] and
//! [`UiReport::VideoArea`], [`WindowOp::Pip`]; S2d(2): [`UiReport::Batching`],
//! [`UiCommand::CancelMount`], [`UiSnapshot::StartupSettled`]; S2d(3):
//! [`UiCommand::RevokeOwner`], [`UiCommand::Panels`], [`UiCommand::DropConfirm`],
//! [`SettingsOp::OwnerGone`], [`SettingsOp::Rekeyed`]; S2d(4): [`UiCommand::Osd`],
//! [`UiSnapshot::Source`], [`UiCommand::PanelMenu`], [`WindowOp::Fullscreen`],
//! [`WindowOp::Raise`], [`WindowOp::Quit`], [`UiCommand::OpenPalette`]), S3a
//! moves the core to its own thread.
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
//! variants) are handled by the interface (`Host::ui_event`, plan step
//! S2f(1)); where the core has work, the interface reports it: the event
//! itself inside [`UiReport::Event`], or what it derived from it (the
//! command and its window, the lead request) as [`UiReport::Input`]. Some
//! are the interface's alone (`PipReturn` restores the window from
//! picture-in-picture, `SidebarResized` writes the policy's width and the
//! interface state; `InjectKey` and `ModifiersReleased` never leave it).

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
    /// A plugin stopped (K3; `stop_run`; plan step S2d(3)): the interface
    /// drops what it keeps of the run: the input slots naming it, its OSD
    /// buttons, its panel menu, its images, its picture-in-picture, its
    /// tool windows. `park`: it went to standby (its recorded tabs stay;
    /// the core leaves them out of the panels it sends as
    /// [`PanelsOp::Gone`]).
    RevokeOwner { owner: u64, park: bool },
    /// A woken or restarted plugin's interface moves to its new owner
    /// generation. Not sent as built in plan step S2d(3): the interface's
    /// rekeys carry the keys and pages they move ([`PanelsOp::Rekeyed`],
    /// [`SettingsOp::Rekeyed`]), sent where the core moved its own.
    RekeyOwner { from: u64, to: u64 },
    /// Panels leave or change owner (plan step S2d(3)): the interface's part
    /// of the core's removals and rekeys of mount records.
    Panels(PanelsOp),
    /// A plugin stopped while its `dialogs.confirm` waits on the sheet
    /// (`ui.dialog`'s revoke; plan step S2d(3)): the sheet ends unanswered.
    DropConfirm { owner: u64 },
    /// A view shows its initial properties again: `ui.mount` (row 40, at
    /// the request, before the batching window) and `settings.register`
    /// (row 83a). Row patches apply on them (`seq` 0), so the interface
    /// drops the view's write records: each property's last `seq`, the
    /// parts of a whole write held until its last part, and the rows'
    /// images and value costs (`ui_image::Images::reset_writes`). Ordered:
    /// a write of the view sent after it is judged against the reset
    /// records. The records stay the interface's, with the decoded images
    /// (plan step S2c(6); design §7, K8): the `stale` check and the write
    /// it guards are one step of the UI-bound stream.
    ResetWrites { owner: u64, view: String },
    /// The core cancels the mount change the interface's batching window
    /// holds (plan step S2d(2)): the host's own removal of a panel, given
    /// way to its plugin's mount, or every change of a stopped plugin. The
    /// interface drops it if its window holds it and reports
    /// [`UiReport::Batching`]; the core has cleared its record already.
    CancelMount(MountCancel),
    /// The command registry or the binding table changed under an open
    /// menu (`commands.unregister`, row 33; `bindings.set` / `.reset`, rows
    /// 76, 77; plan step S2f(1), design §5.3 group B): an open menu is
    /// written again from the interface's copies of the tables.
    RefreshMenu,
    /// A confirmation was refused `busy` because another waits (rows 48,
    /// 96; plan step S2f(1), group B): the one waiting comes into view
    /// (the sheet, or the system dialog).
    ShowWaiting,
    /// The user answered a native dialog (`ConfirmDone`, `DialogDone`,
    /// `SourceLocated`; plan step S2f(1), group B): it counts as activity
    /// for the panel policy (the pointer, the popped-out columns).
    Activity,
    /// `dialogs.confirm` as the core admitted it (row 48; plan step
    /// S2f(3)): the interface chooses where it shows from its live windows
    /// and focus (`origins::route`): the main window's sheet (carried out as
    /// [`Self::Review`]) or a system confirmation (reported as
    /// [`UiInput::SystemConfirm`], then carried out as [`Self::Confirm`]).
    Ask(AskRequest),
    /// `dialogs.open` (row 47) and `media.sources.locate` (row 65c) as the
    /// core admitted them (plan step S2f(3)): the interface chooses the
    /// window that owns the picker (`origins::dialog_place`) and carries it
    /// out as [`Self::Dialog`].
    OpenDialog {
        reply: Reply,
        purpose: DialogPurpose,
    },
    /// What the approval sheet offers to remember changed
    /// (`CommandRuns::forget_remembered`: Ask Again, an uninstall; plan
    /// step S2f(3)): an open sheet's checkbox is written again.
    ReviewOffer,
}

/// See [`UiCommand::Panels`].
pub(crate) enum PanelsOp {
    /// Panels of a stopped run, or kept panels whose restart never came
    /// (`stop_run`, `drop_panels`), with their slots, in the core's order:
    /// the floating window closes if it shows one, they leave the panel
    /// policy (the tabs are reported), and a slot whose last panel left is
    /// hidden in the shell.
    Gone(Vec<(u64, composition::Slot)>),
    /// A placeholder tab goes (`ui.mount`'s revoke and prune): it leaves the
    /// panel policy (the tabs are reported).
    PlaceholderGone(u64),
    /// Run `from`'s panels are run `to`'s now, under the keys `keys`
    /// (old, new), in the core's order (`rekey_panels`): the policy, the
    /// drawn panels, the floating window, the columns and the input slots
    /// follow, and the tabs are reported.
    Rekeyed {
        from: u64,
        to: u64,
        keys: Vec<(u64, u64)>,
    },
    /// The panels waiting for their plugin's restart and the line over each
    /// changed (`Core::restarts`, plan step S2f(2)): the interface keeps
    /// them whole. `back`: that panel was mounted again (or went) and its
    /// line went, so the keys it held go back to the video; `redraw`: the
    /// lines are written now (else at the next `sync_panels`, as before).
    Restarting {
        lines: BTreeMap<u64, String>,
        back: Option<u64>,
        redraw: bool,
    },
    /// Kept panels a restarted run did not mount again: the interface lets
    /// their images go (each panel's plugin and view), before they go
    /// (plan step S2f(2)).
    ImagesReleased(Vec<(u64, &'static str)>),
}

/// See [`UiCommand::CancelMount`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MountCancel {
    /// The host's own removal (an unmount with id 0) of panel `key`.
    Removal { key: u64 },
    /// Any change of a stopped plugin (its owner generation).
    Owner(u64),
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
    /// The core's last `source.state` (`Core::last_source`; rule R1:
    /// playback control is the core's), whole (plan step S2c(7)). The
    /// interface reads its copy for what it shows from the source: no
    /// video and the empty phase, the window title, the pointer hidden
    /// while playing, picture-in-picture's playback, the first draws'
    /// crash guard.
    LastSource(Arc<Value>),
    /// The plugins' start-up panels are in place (plan step S2d(2);
    /// `Core::startup_panels`, decided in `open_files_tick`): the
    /// interface's columns stop waiting for their remembered tabs.
    StartupSettled(bool),
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
    /// `ui.open` refused by the core's checks (the package file, the
    /// properties, the callbacks; plan step S2d). Not answered by the core:
    /// a view of that id already open is refused `conflict` before them,
    /// as before, and the open views are the interface's. The interface
    /// answers `conflict` or this error.
    Refused(RpcError),
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
    /// Pages of `owner` left the registry (its stop for good, or the
    /// recorded pages its woken run did not register again; plan step
    /// S2d(3)): an open window is rebuilt.
    OwnerGone { owner: u64 },
    /// A woken plugin's pages `serials` belong to run `to` now (plan step
    /// S2d(3)): an open window's callbacks follow.
    Rekeyed { serials: Vec<u64>, to: u64 },
}

pub(crate) enum WindowOp {
    /// `window.setVideoArea` (row 71; stream B). The core checks the size;
    /// the UI fits the window and reports [`UiReport::AreaFitted`] (or an
    /// error as [`UiReport::Answered`]); the core answers when the layout
    /// shows the size ([`UiReport::VideoArea`]) or after its 1 s deadline
    /// (R13). As built in plan step S2d.
    VideoArea {
        reply: Reply,
        width: f64,
        height: f64,
    },
    /// `window.setPictureInPicture { on }` (row 74; stream B). Answered from
    /// the PiP state machine through [`UiReport::Answered`] (at once, or
    /// when the system window has opened or closed). Plan step S2d sends no
    /// handoff with it: `window.state` reads the interface's live PiP state
    /// on this thread, and a handoff with the new `active` flag would
    /// broadcast `window.state` before today's window tick does. R13's
    /// handoff of the flag is plan step S3a's (the core's own copy).
    Pip { reply: Reply, on: bool },
    /// `window.setAlwaysOnTop` (row 72): every ordinary window at the
    /// floating level, or not. Answered by the core at once. Plan step
    /// S2c(7): the value in effect (the newest write of a running or
    /// standby plugin), sent when it changed (`Core::window_settings`); the
    /// UI keeps it and applies it at `apply_on_top`, right after the
    /// request and at every window tick.
    OnTop(bool),
    /// `window.setAlwaysOnTop` (row 72) was taken: the interface applies
    /// the value in effect now (`apply_on_top`), as the core did in place
    /// before plan step S2f(1) (design §5.3, group B).
    ApplyOnTop,
    /// The shell window in or out of fullscreen: `host.run fullscreen`
    /// (row 34, note 4) and the host's `fullscreen`, `enter-fullscreen`,
    /// `exit-fullscreen` commands (`window_command`).
    Fullscreen(FullscreenChange),
    /// `OpenFiles` from another launch: the main window to the front.
    Raise,
    /// The player ends its event loop: the host's `quit` command
    /// (`host_command`; plan step S2d(4)). The core stops the handoffs and
    /// writes the state file (the core's since plan step S2c(7)) before it
    /// sends this; the interface releases the GPU waits and ends the loop.
    /// `rendering.restart` (row 90) keeps its own quit after its reply
    /// (`slint::invoke_from_event_loop`, safe from any thread).
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
    /// `dialogs.open`: its parameters as the core checked them, the title
    /// and file name in the plugin's language (`{ kind, title, fileName }`;
    /// capabilities/ui_dialog.rs).
    Open(Value),
    /// `media.sources.locate`: `{ store, token, name }` of the reference.
    Locate(Value),
}

/// A file dialog (`dialogs.open`, `media.sources.locate`) and the tool
/// window that owns it (`None`: the main window; `origins::dialog_place`).
pub(crate) struct DialogRequest {
    pub(crate) reply: Reply,
    pub(crate) purpose: DialogPurpose,
    pub(crate) place: Option<origins::Place>,
}

/// `dialogs.confirm` as a system confirmation. Admission (headless,
/// `dialog_busy`, the rate checks) and the choice of the window stay on
/// the core (note 8): `options` in the caller's language, and the tool
/// window that owns the dialog (`None`: the focused one's, else the main
/// window's, `platform::confirm`). A confirmation that fails to show is
/// answered with [`UiReport::Answered`]'s error.
pub(crate) struct ConfirmRequest {
    pub(crate) reply: Reply,
    pub(crate) options: crate::ui::confirm::Options,
    pub(crate) place: Option<origins::Place>,
}

/// See [`UiCommand::Ask`]: the caller's texts, and its live ticket and
/// request when it holds the lead (shown in the sheet's request box).
pub(crate) struct AskRequest {
    pub(crate) reply: Reply,
    pub(crate) options: crate::ui::confirm::Options,
    pub(crate) request: Option<(String, String)>,
}

pub(crate) enum ReviewKind {
    /// `dialogs.confirm` shown in the main window.
    Confirm(ConfirmSheet),
    /// `commands.run` whose plan asks.
    CommandRun(RunSheet),
}

/// What the sheet asks of a `dialogs.confirm` (`Host::open_confirm`): the
/// caller's texts, and its live ticket and request when it holds the lead.
pub(crate) struct ConfirmSheet {
    pub(crate) options: crate::ui::confirm::Options,
    pub(crate) request: Option<(String, String)>,
}

/// What the sheet asks of a `commands.run` plan (`Host::open_review`): the
/// ticket, its request, the plan's steps and the "Don't ask again" text
/// (empty: not offered).
pub(crate) struct RunSheet {
    pub(crate) ticket: String,
    pub(crate) query: String,
    pub(crate) steps: Vec<review_sheet::Step>,
    pub(crate) remember: String,
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
    /// `ui.interaction` (row 43): the locks of panel `key` (its slot is the
    /// key's, `panel_slot`). The UI also marks activity on the policy
    /// (`policy.activity`). The core answers from its own judgement of the
    /// locks (`PanelTabs::holds`), not from the UI's.
    Locks {
        key: u64,
        locks: window_state::Locks,
    },
    /// `layout.set` (row 46, stream B), as the core checked it: the slot,
    /// the change, the slot's first panel (the panel the change goes
    /// through when the column has no current tab) and `reveal`. The core
    /// refuses at once what is refused before any step takes effect
    /// (`PanelTabs::judge_layout`). The UI applies it, runs a layout pass
    /// so that a following `layout.state` sees the change (the SDK reads
    /// right after it), and answers with [`UiReport::Answered`] whose
    /// handoff carries the facts and tab order after the pass (R13).
    Set {
        reply: Reply,
        slot: composition::Slot,
        change: LayoutChange,
        first: u64,
        reveal: bool,
    },
    /// The host's Sidebar and Adjustments commands (`host_command`,
    /// `toggle_sidebar` / `toggle_adjust`): switch the column's current
    /// panel on or off.
    Toggle(crate::ui::sidebar_tabs::Column),
}

pub(crate) enum LeadOp {
    /// `palette.lead.set` (or its replay), as the core took it into
    /// `Core::lead`: whether the row appeared (none was set before), and
    /// the end-state status the strip shows once (one not shown before).
    /// Also a parked holder's row losing its working status (`appeared`
    /// false, no end state).
    Set {
        owner: u64,
        appeared: bool,
        ended: Option<crate::capabilities::LeadStatus>,
    },
    /// `palette.lead.clear`, or its holder gone.
    Clear { owner: u64 },
    /// `commands.run` ran a step of the holder's request (plan step
    /// S2f(3)): the status strip shows `line` and `progress` for
    /// `ticket` (`LeadStrip::step`).
    Step {
        ticket: String,
        line: String,
        progress: f32,
    },
    /// A request went to the holder (the `VESTRA_TEST_EXECUTE` hook's
    /// `lead:`; plan step S2f(3)): a menu not `keep` open closes, as
    /// after the palette's first row.
    Sent { keep: bool },
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
    /// 83d), picture-in-picture (74), `layout.set` (46), a sheet's
    /// `dialogs.confirm` (48), a system confirmation that failed to show
    /// (48), and the errors of `window.setVideoArea` (71). The core
    /// handles it in R13's order: apply the `handoff` (only if its `seq`
    /// is newer than the facts it has, see the module doc), send the reply,
    /// broadcast what changed.
    Answered {
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
        handoff: Option<Box<WindowFacts>>,
        /// The answer is a system confirmation's failure to show
        /// ([`ConfirmRequest`]): no native dialog is open any more, so the
        /// core clears `dialog_busy` and `confirm_open`. Marked, not
        /// matched by reply: another request of the same plugin may carry
        /// the same id.
        confirm_failed: bool,
    },
    /// `window.setVideoArea` was fitted: the video size it waits for and
    /// the reply it will carry. The core keeps it with its deadline
    /// (receipt + 1000 ms, `Core::area_replies`) and answers from
    /// [`Self::VideoArea`]. As built in plan step S2d it is not checked on
    /// receipt: today's reply waits for the next window tick's layout even
    /// when the size is shown already, and the wire trace keeps that.
    /// Answering at once when the core's copy already shows the size is
    /// plan step S3a's choice (design §5, the 1 s timeout).
    AreaFitted {
        reply: Reply,
        video: (f64, f64),
        result: Value,
    },
    /// The size of the video area the window tick's layout shows, at that
    /// tick's time (`now`, the core clock's ms), after the bar, the columns
    /// and fullscreen changed it (plan step S2d): the core answers each
    /// pending `window.setVideoArea` the size matches, or whose deadline
    /// passed. Last value wins: the newest layout is what the window shows.
    /// The core's window facts are measured earlier in the same tick, so
    /// they cannot stand in for it without moving the reply a tick later.
    VideoArea { size: (f64, f64), now: u64 },
    /// The mount change the interface's batching window holds now, or none
    /// (plan step S2d(2)): sent where the window opens, closes, takes a
    /// failed batch's first change again, or drops one the core
    /// cancelled. The core replaces its record (`Core::handed`), which it
    /// also sets itself when it hands a change over. Last value wins.
    Batching(Option<super::mounts::HandedMount>),
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
    /// The interface's part of the interface state file changed (plan step
    /// S2c(7); design K8): the core, the file's one writer, takes it whole
    /// and writes the file a second after `at` (the core clock's ms of the
    /// change, `ui_state::Store::touch`). Last value wins.
    Remembered {
        state: Box<crate::ui::ui_state::Interface>,
        at: u64,
    },
    /// A complete file drop (R-U2; plan step S2c(7)): the interface
    /// collected its paths and found the panel under the pointer; the core
    /// delivers it from its registry (`drops.panel` to a listening panel,
    /// else `drops.drop` to the providers).
    Drop(FileDrop),
    /// One of the 26 UI-originated events, unchanged, whose core part needs
    /// nothing but the event (plan step S2f(1)): `PhysicalInput`,
    /// `MainClosed`, `UiCallback` (the core writes `ui.callback`),
    /// `ViewClosed` (the core writes `ui.closed`, sent only for a view the
    /// interface had open), `CommandRunAnswered`. The interface sends it at
    /// the point of its own part where the core's work ran before.
    Event(UiEvent),
    /// What the interface derived from a UI event for the core's part
    /// (plan step S2f(1)): a command the user ran, with the window it came
    /// from; a host command of the title strip; a request to the palette
    /// lead's holder; a parked plugin's page or tab shown; the shown tab's
    /// work. Ordered.
    Input(UiInput),
    /// The interface's facts core code reads (design §5.3, group A; plan
    /// step S2f(1)): last value wins.
    UiFacts(Box<UiFacts>),
}

/// The interface's own state that core code reads (design §5.3, group A;
/// plan step S2f(1)), whole: reported as [`UiReport::UiFacts`] whenever it
/// differs from the value sent last, checked at every point where the
/// interface hands the turn to the core (an event of the core, a report,
/// the end of a command, the core's steps), so the core's copy is the
/// interface's whenever core code runs. Values the core can derive are not
/// here: the core judges them from these and its own data (whether a
/// plan's sheet still waits, a column's current tab).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct UiFacts {
    /// The approval sheet: its request and whether it is a
    /// `dialogs.confirm` (else a `commands.run` plan) (`host_busy`'s
    /// `Busy::Dialog`, the `busy` refusals of rows 48 and 96).
    pub(crate) sheet: Option<SheetFact>,
    /// The options window is open (`Busy::Settings`).
    pub(crate) options_open: bool,
    /// The sidebar's and the adjust column's (in `Column::ALL` order)
    /// policy facts for `Busy::ShownTab`.
    pub(crate) columns: [ColumnFacts; 2],
    /// The shell's generation while a shell exists (`window_command`,
    /// `diagnostics_tick`, `dialogs.confirm`'s route).
    pub(crate) shell: Option<u64>,
    /// The shell generation counter (`layout.state` is pending until the
    /// window facts show it).
    pub(crate) shell_generation: u64,
    /// What the video area shows nothing (`layout.state`'s backdrop
    /// `effective`).
    pub(crate) no_video: bool,
    /// The plugin whose OSD message has buttons now (`UiOsd::record`).
    pub(crate) osd_buttons: Option<u64>,
    /// A menu is open (`ui.menu`'s `VESTRA_DEBUG_KEYS` line; plan step
    /// S2f(3)).
    pub(crate) menu_open: bool,
    /// The panels the floating sidebar window shows (`ui.menu`: a panel
    /// there is shown; plan step S2f(3)).
    pub(crate) floating: Vec<u64>,
    /// The plugins with a window of their own open (`UiWindow::record`;
    /// plan step S2f(3)).
    pub(crate) views: std::collections::BTreeSet<u64>,
    /// Picture-in-picture: offered here (a native window and system
    /// support), shown now, and the plugin whose request is in effect
    /// (`window.state`, `WindowControl::record`; plan step S2f(3)).
    pub(crate) pip: PipFact,
}

/// See [`UiFacts::pip`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PipFact {
    pub(crate) supported: bool,
    pub(crate) active: bool,
    pub(crate) holder: Option<u64>,
}

/// See [`UiFacts::sheet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SheetFact {
    pub(crate) owner: u64,
    pub(crate) id: u64,
    pub(crate) confirm: bool,
}

/// See [`UiFacts::columns`]: the column's current tab in the panel policy,
/// whether the column is wanted shown, and its remembered tab (the
/// interface state's, by manifest).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ColumnFacts {
    pub(crate) active: Option<u64>,
    pub(crate) wanted: bool,
    pub(crate) remembered: Option<String>,
}

/// See [`UiReport::Input`].
pub(crate) enum UiInput {
    /// The user ran command `id` of `owner` from `from`. `origin`: the
    /// window it came from (`origins`), as the interface knows its windows
    /// (`None`: no live window of that generation, nothing recorded). The
    /// core records it and executes the command; a key whose command is
    /// disabled with a reason shows the reason instead.
    Command {
        from: CommandFrom,
        owner: u64,
        id: String,
        origin: Option<origins::Origin>,
    },
    /// One of the host's own commands from the interface's own buttons
    /// (the fullscreen title strip: `exit-fullscreen`, `quit`).
    HostCommand(&'static str),
    /// The palette's first row or the status strip's Stop: the
    /// `palette.lead` event to the lead's holder (`Host::lead_event`): the
    /// core records the origin, mints the ticket and writes the line.
    Lead {
        query: String,
        keep: bool,
        status: bool,
    },
    /// The user reset the main menu (`MenuAction::Reset`): the core
    /// revokes the physical input (`producer-audio`).
    MenuReset,
    /// A settings page of `owner` became the shown one (`settings_select`):
    /// a parked plugin is woken (`Host::page_shown`).
    PageShown(u64),
    /// A column's tab of `owner`, a plugin in standby, became the current
    /// one (`tab_demand`): it is woken (`Host::panel_shown`).
    TabShown(u64),
    /// The current tab's running plugin `owner` (every window tick,
    /// `tab_demand`): its declared work `Busy::ShownTab` (`Host::tab_work`).
    TabWork(u64),
    /// The interface's work failed or a panel called back, for a plugin
    /// (design §5.3, group E): the core writes the line (R6: one writer)
    /// where the interface wrote it before.
    Line(PluginLine),
    /// The window tick saw the shell in or out of fullscreen (group E): the
    /// core keeps the host's fullscreen commands' states
    /// (`Core::host_command_states`) and the backdrop's cover art
    /// (`Core::backdrop_cover_art`), at the tick's point where the
    /// interface called them before.
    WindowTick { fullscreen: bool },
    /// The bottom bar's column toggles (`sidebar`, `adjust`): their states
    /// from the panel policy (group E); the core writes the registry
    /// (`Core::column_command_states`).
    ColumnCommands([(&'static str, commands::CommandState); 2]),
    /// The interface's mount step begins (`prepare_mount_tick`, group E):
    /// the core first answers the requests it held while the built-ins
    /// started (`Host::release_held_mounts`).
    MountTick,
    /// A batch of mount changes failed (`prepare_mount_tick`, group E):
    /// its changes after the first, in batch order and marked announced,
    /// go back to the front of the core's queue (`Core::mount_queue`),
    /// to be applied one at a time; the first stays in the batching
    /// window.
    MountRetry(Vec<MountChange>),
    /// The interface shows a `dialogs.confirm` as a system confirmation
    /// ([`UiCommand::Ask`], plan step S2f(3)): the core marks a native
    /// dialog open (`dialog_busy`, `confirm_open`) and revokes the
    /// physical input, before it shows.
    SystemConfirm,
    /// A batch applied the change of panel `key`, which waited for its
    /// plugin's restart (`prepare_mount_tick`, plan step S2f(2)): the core
    /// ends the wait (`Host::panel_back`).
    PanelBack(u64),
}

/// See [`UiInput::Line`]: what the interface tells, and to whom.
pub(crate) enum PluginLine {
    /// `ui.error` to `owner` with `data` (`{code, message}`, a slot).
    UiError { owner: u64, data: Value },
    /// `ui.callback` of a docked panel to `owner` with `data`.
    UiCallback { owner: u64, data: Value },
    /// `host.error` to every plugin with `data` (`{code, message}`).
    HostError(Value),
}

/// Where the user ran a command ([`UiInput::Command`]): each path keeps its
/// own log line on a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandFrom {
    /// The main or a panel's menu (`MenuAction::Invoke`).
    Menu,
    /// A key binding (`Event::Binding`).
    Key,
    /// A bottom-bar cell (`Event::BarButton`).
    Bar,
    /// An OSD message's button (`Event::OsdAction`).
    Osd,
    /// A palette row.
    Palette,
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
    /// (`layout.state`; the layout part of R13's handoff), and the bottom
    /// bar's panel: the core's `PanelTabs`, from which it judges
    /// `ui.interaction`'s and `layout.set`'s refusals (plan step S2c(5)).
    /// The UI also sends a value when only these changed (`report_tabs`).
    pub(crate) sidebar_panels: Vec<u64>,
    pub(crate) adjust_panels: Vec<u64>,
    pub(crate) bottom_panel: Option<u64>,
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

/// See [`UiReport::Drop`]: the paths in the order they came, the panel
/// under the pointer by its plugin with the point in the panel's
/// coordinates (`None`: no panel there), and the modifier keys
/// (`{alt, shift, cmd}`) when the drop closed.
pub(crate) struct FileDrop {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) panel: Option<(u64, (f64, f64))>,
    pub(crate) modifiers: Value,
}

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
                | (Self::Remembered { .. }, Self::Remembered { .. })
                | (Self::VideoArea { .. }, Self::VideoArea { .. })
                | (Self::Batching(_), Self::Batching(_))
                | (Self::UiFacts(_), Self::UiFacts(_))
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
    send::<UiInput>();
    send::<UiFacts>();
    send::<WindowFacts>();
    send::<MountApplied>();
    send::<FileDrop>();
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
            bottom_panel: None,
        })
    }

    fn options() -> crate::ui::confirm::Options {
        crate::ui::confirm::Options {
            message: "Sure?".into(),
            detail: None,
            confirm: "OK".into(),
            cancel: "Cancel".into(),
            destructive: false,
            checkbox: None,
        }
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
                place: None,
            }),
            UiCommand::Dialog(DialogRequest {
                reply: reply(11),
                purpose: DialogPurpose::Locate(json!({"name":"a.mkv"})),
                place: Some(origins::Place::Floating),
            }),
            UiCommand::Confirm(ConfirmRequest {
                reply: reply(12),
                options: options(),
                place: None,
            }),
            UiCommand::Review(ReviewRequest {
                reply: reply(13),
                kind: ReviewKind::Confirm(ConfirmSheet {
                    options: options(),
                    request: None,
                }),
            }),
            UiCommand::Review(ReviewRequest {
                reply: reply(14),
                kind: ReviewKind::CommandRun(RunSheet {
                    ticket: "t".into(),
                    query: "q".into(),
                    steps: vec![],
                    remember: String::new(),
                }),
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
                slot: composition::Slot::Sidebar,
                change: LayoutChange {
                    visible: Some(true),
                    detached: None,
                    active: None,
                },
                first: 29,
                reveal: true,
            }),
            UiCommand::Layout(LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Sidebar)),
            UiCommand::Layout(LayoutOp::Toggle(crate::ui::sidebar_tabs::Column::Adjust)),
            UiCommand::OpenPalette,
            UiCommand::Lead(LeadOp::Set {
                owner: 7,
                appeared: true,
                ended: None,
            }),
            UiCommand::Lead(LeadOp::Clear { owner: 7 }),
            UiCommand::RevokeOwner {
                owner: 7,
                park: true,
            },
            UiCommand::RekeyOwner { from: 7, to: 8 },
            UiCommand::Ask(AskRequest {
                reply: reply(9),
                options: options(),
                request: None,
            }),
            UiCommand::OpenDialog {
                reply: reply(9),
                purpose: DialogPurpose::Open(json!({"kind":"openFile"})),
            },
            UiCommand::ReviewOffer,
            UiCommand::Lead(LeadOp::Step {
                ticket: "t".into(),
                line: "1/2".into(),
                progress: 0.5,
            }),
            UiCommand::Lead(LeadOp::Sent { keep: false }),
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
                confirm_failed: false,
            },
            UiReport::Answered {
                reply: reply(9),
                result: Err(RpcError::new("unavailable", "no window")),
                handoff: Some(facts()),
                confirm_failed: false,
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
                UiSnapshot::LastSource(_) => "LastSource",
                UiSnapshot::StartupSettled(_) => "StartupSettled",
            },
            UiCommand::View(op) => match op.kind {
                ViewKind::Open { .. } => "View.Open",
                ViewKind::Set { .. } => "View.Set",
                ViewKind::Show { .. } => "View.Show",
                ViewKind::Close => "View.Close",
                ViewKind::Refused(_) => "View.Refused",
            },
            UiCommand::Mount(_) => "Mount",
            UiCommand::SetMount(_) => "SetMount",
            UiCommand::Settings(op) => match op {
                SettingsOp::Added(_) => "Settings.Added",
                SettingsOp::Updated { .. } => "Settings.Updated",
                SettingsOp::Removed { .. } => "Settings.Removed",
                SettingsOp::Set { .. } => "Settings.Set",
                SettingsOp::Show { .. } => "Settings.Show",
                SettingsOp::OwnerGone { .. } => "Settings.OwnerGone",
                SettingsOp::Rekeyed { .. } => "Settings.Rekeyed",
            },
            UiCommand::Window(op) => match op {
                WindowOp::VideoArea { .. } => "Window.VideoArea",
                WindowOp::Pip { .. } => "Window.Pip",
                WindowOp::OnTop(_) => "Window.OnTop",
                WindowOp::ApplyOnTop => "Window.ApplyOnTop",
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
                LeadOp::Step { .. } => "Lead.Step",
                LeadOp::Sent { .. } => "Lead.Sent",
            },
            UiCommand::OpenPalette => "OpenPalette",
            UiCommand::RevokeOwner { .. } => "RevokeOwner",
            UiCommand::RekeyOwner { .. } => "RekeyOwner",
            UiCommand::ResetWrites { .. } => "ResetWrites",
            UiCommand::CancelMount(_) => "CancelMount",
            UiCommand::Panels(op) => match op {
                PanelsOp::Gone(_) => "Panels.Gone",
                PanelsOp::PlaceholderGone(_) => "Panels.PlaceholderGone",
                PanelsOp::Rekeyed { .. } => "Panels.Rekeyed",
                PanelsOp::Restarting { .. } => "Panels.Restarting",
                PanelsOp::ImagesReleased(_) => "Panels.ImagesReleased",
            },
            UiCommand::DropConfirm { .. } => "DropConfirm",
            UiCommand::RefreshMenu => "RefreshMenu",
            UiCommand::ShowWaiting => "ShowWaiting",
            UiCommand::Activity => "Activity",
            UiCommand::Ask(_) => "Ask",
            UiCommand::OpenDialog { .. } => "OpenDialog",
            UiCommand::ReviewOffer => "ReviewOffer",
        }
    }

    /// What plan step S2f(1) added (design §5.3, group B): one-way
    /// commands, ordered, never collapsed.
    #[test]
    fn the_group_b_commands_are_ordered() {
        let added = || {
            vec![
                UiCommand::RefreshMenu,
                UiCommand::ShowWaiting,
                UiCommand::Activity,
                UiCommand::Window(WindowOp::ApplyOnTop),
            ]
        };
        let names: Vec<_> = added().iter().map(command_name).collect();
        assert_eq!(
            names,
            [
                "RefreshMenu",
                "ShowWaiting",
                "Activity",
                "Window.ApplyOnTop"
            ]
        );
        for command in added() {
            for other in added().iter().chain(every_command().iter()) {
                assert!(!command.replaces(other) && !other.replaces(&command));
            }
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
            UiReport::Remembered { .. } => "Remembered",
            UiReport::Drop(_) => "Drop",
            UiReport::VideoArea { .. } => "VideoArea",
            UiReport::Batching(_) => "Batching",
            UiReport::Event(event) => event.0.label().0,
            UiReport::Input(_) => "Input",
            UiReport::UiFacts(_) => "UiFacts",
        }
    }

    /// What plan step S2f(2) added: the restart record's interface parts
    /// are ordered and never collapse.
    #[test]
    fn the_restart_commands_are_ordered() {
        let added = || {
            vec![
                UiCommand::Panels(PanelsOp::Restarting {
                    lines: BTreeMap::from([(28, "A is restarting…".into())]),
                    back: Some(29),
                    redraw: true,
                }),
                UiCommand::Panels(PanelsOp::ImagesReleased(vec![(7, "$mount")])),
            ]
        };
        let names: Vec<_> = added().iter().map(command_name).collect();
        assert_eq!(names, ["Panels.Restarting", "Panels.ImagesReleased"]);
        for command in added() {
            for other in added().iter().chain(every_command().iter()) {
                assert!(!command.replaces(other) && !other.replaces(&command));
            }
        }
    }

    /// What plan step S2f(1) added (group A): the interface's facts replace
    /// only an older value of their kind.
    #[test]
    fn the_ui_facts_coalesce() {
        let facts = |open| {
            UiReport::UiFacts(Box::new(UiFacts {
                options_open: open,
                ..UiFacts::default()
            }))
        };
        assert_eq!(report_name(&facts(true)), "UiFacts");
        assert!(facts(true).replaces(&facts(false)));
        for other in every_report() {
            assert!(!facts(true).replaces(&other) && !other.replaces(&facts(true)));
        }
    }

    /// What plan step S2f(1) added: the core's part of a UI event is
    /// ordered and never collapses.
    #[test]
    fn the_inputs_are_ordered() {
        let inputs = || {
            vec![
                UiReport::Input(UiInput::Command {
                    from: CommandFrom::Key,
                    owner: 7,
                    id: "play".into(),
                    origin: Some(origins::Origin::Main),
                }),
                UiReport::Input(UiInput::HostCommand("quit")),
                UiReport::Input(UiInput::Lead {
                    query: "q".into(),
                    keep: false,
                    status: false,
                }),
                UiReport::Input(UiInput::MenuReset),
                UiReport::Input(UiInput::PageShown(7)),
                UiReport::Input(UiInput::TabShown(7)),
                UiReport::Input(UiInput::TabWork(7)),
                UiReport::Input(UiInput::Line(PluginLine::UiError {
                    owner: 7,
                    data: json!({"code":"ui","message":"m"}),
                })),
                UiReport::Input(UiInput::Line(PluginLine::UiCallback {
                    owner: 7,
                    data: json!({"view":"$mount"}),
                })),
                UiReport::Input(UiInput::Line(PluginLine::HostError(
                    json!({"code":"menu-bridge","message":"m"}),
                ))),
                UiReport::Input(UiInput::WindowTick { fullscreen: true }),
                UiReport::Input(UiInput::ColumnCommands(["sidebar", "adjust"].map(|id| {
                    (
                        id,
                        commands::CommandState {
                            enabled: true,
                            checked: Some(false),
                            busy: false,
                            reason: None,
                        },
                    )
                }))),
                UiReport::Input(UiInput::MountTick),
                UiReport::Input(UiInput::MountRetry(vec![mount_change()])),
                UiReport::Input(UiInput::PanelBack(28)),
            ]
        };
        for input in inputs() {
            assert_eq!(report_name(&input), "Input");
            for other in inputs().iter().chain(every_report().iter()) {
                assert!(!input.replaces(other) && !other.replaces(&input));
            }
        }
    }

    #[test]
    fn every_command_is_constructed_and_matched_once() {
        let names: Vec<_> = every_command().iter().map(command_name).collect();
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        assert_eq!(names.len(), 6 + 41);
    }

    /// The variant plan step S2c(6) added, constructed and matched on its
    /// own (`every_command` keeps the S2a set its count names): ordered,
    /// never collapsed.
    #[test]
    fn reset_writes_is_constructed_matched_and_ordered() {
        let reset = UiCommand::ResetWrites {
            owner: 7,
            view: "$mount".into(),
        };
        assert_eq!(command_name(&reset), "ResetWrites");
        assert!(!reset.replaces(&reset));
        for other in every_command() {
            assert!(!reset.replaces(&other) && !other.replaces(&reset));
            assert_ne!(command_name(&other), "ResetWrites");
        }
    }

    /// The snapshot plan step S2c(7) added: it replaces only an older
    /// value of its own kind (`every_command` keeps the S2a set).
    #[test]
    fn the_last_source_snapshot_is_constructed_matched_and_coalesced() {
        let older = UiCommand::Snapshot(UiSnapshot::LastSource(Arc::new(json!({"phase":"empty"}))));
        let newer = UiCommand::Snapshot(UiSnapshot::LastSource(Arc::new(json!({"phase":"ready"}))));
        assert_eq!(command_name(&newer), "LastSource");
        assert!(newer.replaces(&older));
        for other in every_command() {
            assert!(!newer.replaces(&other) && !other.replaces(&newer));
        }
    }

    /// The reports plan step S2c(7) added: the interface's part of the
    /// state file is a fact (last value wins), a drop is ordered.
    #[test]
    fn remembered_coalesces_and_a_drop_is_ordered() {
        let remembered = |at| UiReport::Remembered {
            state: Box::default(),
            at,
        };
        let drop = || {
            UiReport::Drop(FileDrop {
                paths: vec![PathBuf::from("a.mkv")],
                panel: Some((7, (1., 2.))),
                modifiers: json!({"alt":false,"shift":false,"cmd":false}),
            })
        };
        assert_eq!(report_name(&remembered(1)), "Remembered");
        assert_eq!(report_name(&drop()), "Drop");
        assert!(remembered(2).replaces(&remembered(1)));
        assert!(!drop().replaces(&drop()));
        assert!(!remembered(1).replaces(&drop()) && !drop().replaces(&remembered(1)));
        for other in every_report() {
            for new in [remembered(1), drop()] {
                assert!(!new.replaces(&other) && !other.replaces(&new));
            }
        }
    }

    /// What plan step S2d added (`every_command` and `every_report` keep
    /// the S2a set): a refused `ui.open` is ordered like the other view
    /// requests; the layout's video area is a fact that replaces only an
    /// older one.
    #[test]
    fn a_refused_open_is_ordered_and_the_video_area_coalesces() {
        let refused = || {
            UiCommand::View(ViewOp {
                reply: reply(16),
                id: "w".into(),
                kind: ViewKind::Refused(RpcError::invalid("bad file")),
            })
        };
        assert_eq!(command_name(&refused()), "View.Refused");
        assert!(!refused().replaces(&refused()));
        for other in every_command() {
            assert!(!refused().replaces(&other) && !other.replaces(&refused()));
            assert_ne!(command_name(&other), "View.Refused");
        }
        let area = |width| UiReport::VideoArea {
            size: (width, 360.),
            now: 5,
        };
        assert_eq!(report_name(&area(640.)), "VideoArea");
        assert!(area(641.).replaces(&area(640.)));
        for other in every_report() {
            assert!(!area(640.).replaces(&other) && !other.replaces(&area(640.)));
        }
    }

    /// What plan step S2d(2) added: a cancellation is ordered, start-up
    /// settled and the batching window's change are values that replace
    /// only an older one of their kind.
    #[test]
    fn a_cancel_is_ordered_and_the_settled_and_batching_values_coalesce() {
        let cancels = [
            UiCommand::CancelMount(MountCancel::Removal { key: 28 }),
            UiCommand::CancelMount(MountCancel::Owner(7)),
        ];
        for cancel in &cancels {
            assert_eq!(command_name(cancel), "CancelMount");
            for other in cancels.iter().chain(every_command().iter()) {
                assert!(!cancel.replaces(other) && !other.replaces(cancel));
            }
        }
        let settled = |on| UiCommand::Snapshot(UiSnapshot::StartupSettled(on));
        assert_eq!(command_name(&settled(true)), "StartupSettled");
        assert!(settled(true).replaces(&settled(false)));
        for other in every_command() {
            assert!(!settled(true).replaces(&other) && !other.replaces(&settled(true)));
        }
        let held = |id| {
            UiReport::Batching(Some(super::super::mounts::HandedMount {
                reply: reply(id),
                key: 28,
                slot: Some(composition::Slot::Sidebar),
                call: json!({}),
            }))
        };
        assert_eq!(report_name(&held(3)), "Batching");
        assert!(UiReport::Batching(None).replaces(&held(3)));
        assert!(held(4).replaces(&UiReport::Batching(None)));
        for other in every_report() {
            assert!(!held(3).replaces(&other) && !other.replaces(&held(3)));
        }
    }

    /// What plan step S2d(3) added: the interface's parts of a stop, a
    /// revoke and a rekey are ordered and never collapse.
    #[test]
    fn the_owner_and_panel_commands_are_ordered() {
        let added = [
            UiCommand::Panels(PanelsOp::Gone(vec![(28, composition::Slot::Sidebar)])),
            UiCommand::Panels(PanelsOp::PlaceholderGone(28)),
            UiCommand::Panels(PanelsOp::Rekeyed {
                from: 7,
                to: 8,
                keys: vec![(28, 32)],
            }),
            UiCommand::DropConfirm { owner: 7 },
            UiCommand::Settings(SettingsOp::OwnerGone { owner: 7 }),
            UiCommand::Settings(SettingsOp::Rekeyed {
                serials: vec![1],
                to: 8,
            }),
        ];
        let names: Vec<_> = added.iter().map(command_name).collect();
        assert_eq!(
            names,
            [
                "Panels.Gone",
                "Panels.PlaceholderGone",
                "Panels.Rekeyed",
                "DropConfirm",
                "Settings.OwnerGone",
                "Settings.Rekeyed"
            ]
        );
        for command in &added {
            for other in added.iter().chain(every_command().iter()) {
                assert!(!command.replaces(other) && !other.replaces(command));
            }
        }
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
