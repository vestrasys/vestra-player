//! The `Host`: the UI thread's state (one per player), its events, the
//! lifecycle of plugins, the dispatch of their requests and the window,
//! menu, mount and input handling. Capabilities (`crate::capabilities`) and
//! a few other modules add their own `impl Host` blocks. The state that
//! never touches the interface is `Host::core` ([`Core`], with its own
//! `impl Core` blocks beside them).
mod actions;
pub(crate) use actions::migrate_fullscreen_escape;
mod diagnostics;
pub(crate) use diagnostics::{
    ShaderGuard, persist_shader_guard, register_shader_writer, session_end,
};
mod dispatch;
mod events;
mod handle;
mod input;
pub(crate) mod lead_strip;
mod lifecycle;
mod menus;
mod messages;
mod mounts;
pub(crate) mod origins;
pub(crate) mod palette;
pub(crate) mod panel_restart;
mod rendering;
mod restart;
pub(crate) mod review_sheet;
pub(crate) mod revised;
pub(crate) use rendering::{Relaunch, Rendering, startup as rendering_startup};
mod rpc;
mod shell;
pub(crate) mod stall;
mod state;
pub(crate) mod supervisor;
mod triggers;
mod video;
mod window;
pub(crate) use triggers::{TriggerState, wall_ms};

use crate::packages::{manifest, package_manager};
#[cfg(feature = "producer-audio")]
use crate::playback::producer_input;
use crate::playback::{
    backend::{Backend, Command},
    frame_pacing, render_target, video_click,
};
use crate::process::{Plugin, StopReason, shared_runner};
use crate::supervisor::{Pid, Supervisor, activation};
use crate::ui::{
    bar_buttons, composition, drops, menu, pip, settings_window, sidebar_tabs, slot_claims,
    ui_image, ui_state, ui_values, view, window_control, window_state,
};
use crate::{
    bindings, capabilities, commands, core_actions, i18n, input_trace, open_files, platform, power,
    process, session_storage, timeline,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slint::ComponentHandle;
use slint_interpreter::Value as SValue;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(crate) use events::{Event, Injected, MenuAction, drain as drain_events};
#[cfg(feature = "producer-audio")]
pub(crate) use input::host_key;
pub(crate) use input::{
    debug_keys, double_click_interval, fire, inject_option_key, observe_key, observe_side_button,
    release_modifiers, slint_mods,
};
pub(crate) use lifecycle::activation_outcome;
#[cfg(test)]
pub(crate) use menus::{MenuWrite, ShownMenu, set_menu_model};
pub(crate) use menus::{
    apply_menu_levels, debug_menu, debug_menu_pointer, menu_levels, track_pointer,
};
pub(crate) use mounts::refuse_initial;
pub(crate) use restart::{
    LAST_SOURCE, LostDevice, event_loop_ended, restart_log, restart_logged, restart_player,
    run_through_loss, teardown_step,
};
pub(crate) use rpc::{Reply, Request, RpcError, string, ui_error};
pub(crate) use state::{LayoutChange, layout_change, policy_slot};
pub(crate) use supervisor::Origin;
pub(crate) use supervisor::shutdown::{DEVICE_LOSS_BUDGET, QUIT_BUDGET};
pub(crate) use video::{FRAME_SINK, PIP_GPU_SINK, RESIZE_SINK};

/// The key of a plugin's panel in one slot (SDK 1.6.15: a plugin mounts
/// one panel per slot): what the shell, the policy and the compile sets
/// know a panel by (`CompileSpec.owner`, `Owner<key>Bridge`). The plugin
/// is [`panel_plugin`]; everything addressed to the plugin (messages,
/// commands, images, its process) uses that.
///
/// Owner generations count up from 1, one per plugin start
/// (`alloc_pid`, kept below the wire's 2^53), so far below `u64::MAX / 4`; a
/// larger one would wrap into another panel's key, which debug builds catch.
pub(crate) fn panel_key(owner: u64, slot: composition::Slot) -> u64 {
    debug_assert!(owner <= u64::MAX / PANEL_SLOTS, "owner {owner} too large");
    owner.wrapping_mul(PANEL_SLOTS) + slot as u64
}
/// Panel `key` under run `to` (its slot kept): a plugin's panels moving to
/// its next run (a woken plugin's tabs, a restarted plugin's kept panels).
pub(crate) fn rekey_panel(key: u64, to: u64) -> u64 {
    to.wrapping_mul(PANEL_SLOTS) + key % PANEL_SLOTS
}
/// The plugin a panel key belongs to.
pub(crate) fn panel_plugin(key: u64) -> u64 {
    key / PANEL_SLOTS
}
/// Slots per plugin in a panel key (bottom, sidebar, backdrop, adjust).
const PANEL_SLOTS: u64 = 4;
/// The view a panel's images and row patches are kept under, per slot
/// (the bottom bar keeps the `"$mount"` of one-panel plugins).
pub(crate) fn mount_view(slot: composition::Slot) -> &'static str {
    match slot {
        composition::Slot::Bottom => "$mount",
        composition::Slot::Sidebar => "$mount:sidebar",
        composition::Slot::Backdrop => "$mount:backdrop",
        composition::Slot::Adjust => "$mount:adjust",
    }
}
#[derive(Clone)]
pub(crate) struct Mount {
    /// `spec.owner` is the panel's key (`panel_key`), not the plugin.
    pub(crate) spec: composition::CompileSpec,
    /// The plugin that mounted it (its owner generation).
    pub(crate) plugin: u64,
    pub(crate) properties: BTreeMap<String, Value>,
    /// The bottom bar's preference; a sidebar panel's come from the slot
    /// (`ui_state::SidebarSlot`, then the policy).
    pub(crate) visible: bool,
    /// The floating window's title and the sidebar tab's label: the mount's
    /// `title`, else the plugin's name.
    pub(crate) title: String,
    pub(crate) manifest: String,
    /// A built-in plugin's panel: its tab comes first.
    pub(crate) builtin: bool,
    /// A sidebar tab no process holds (sidebar-tabs.md): replayed from its
    /// plugin's recording, or kept when its plugin stood by. Showing it
    /// wakes the plugin, whose `ui.mount` takes it over.
    pub(crate) recorded: bool,
    /// The `ui.mount` call a recording keeps for it
    /// (`sidebar_tabs::recorded_call`).
    pub(crate) call: Value,
}
/// What the host keeps of one column of tabs (the sidebar, the adjust
/// column) between ticks and shells (sidebar-tabs.md, per column).
#[derive(Default)]
pub(crate) struct ColumnState {
    /// The current tab at the last tick, and whether the user just clicked
    /// a tab (which gave the column the keys already).
    pub(crate) active: Option<u64>,
    pub(crate) tab_clicked: bool,
    /// Its panels in the current shell, in its tab order (its
    /// `<column>-active` indexes this; the policy may already have dropped one).
    pub(crate) tabs: Vec<u64>,
    /// The column draws its tab strip.
    pub(crate) tabbed: bool,
    /// The placeholder tab whose plugin was woken for being shown.
    pub(crate) woken: Option<u64>,
    /// G-01: its fullscreen overlay was popped out at the last tick (the
    /// adjust column: also its overlay over a narrow window's video).
    pub(crate) popped: bool,
    /// It was on screen at the last tick (the adjust column).
    pub(crate) shown: bool,
    /// What the keys under its tabs were last computed from: the binding
    /// table's and the command registry's revisions, the shell generation
    /// and the tabs (the adjust column).
    pub(crate) keys_from: Option<(u64, u64, u64, Vec<u64>)>,
}
/// The detached sidebar's own window (C14).
pub(crate) struct Floating {
    pub(crate) instance: slint_interpreter::ComponentInstance,
    /// The sidebar's panels when it opened, in tab order.
    pub(crate) owners: Vec<u64>,
    pub(crate) generation: u64,
    /// Its native window, for the pointer at a file drop.
    pub(crate) native: Option<platform::HostWindowId>,
}
/// Read synchronously by the shell's key/mouse callbacks, without borrowing Host.
#[derive(Default)]
pub(crate) struct InputShared {
    pub(crate) table: bindings::Table,
    pub(crate) bottom: Option<u64>,
    pub(crate) sidebar: Option<u64>,
    /// The adjust column's current tab (`panel:adjust` keys).
    pub(crate) adjust: Option<u64>,
    pub(crate) blocked: bool,
    pub(crate) wheel: (f64, f64),
    /// The key press being dispatched, recorded by the window's winit hook.
    pub(crate) physical: Option<bindings::Physical>,
    /// Slint modifier key codes winit saw released, to hand to Slint.
    pub(crate) released: Vec<char>,
    /// A left press on the video waiting to become `Mouse:LeftClick` (R-H11).
    pub(crate) click: Option<video_click::Press>,
    /// The pointer in a window (its generation, logical px), from its winit
    /// hook; G-15 aims submenus with it.
    pub(crate) pointer: Option<(u64, menu::Point)>,
}
/// Which window shows the open menu, and what it lists.
#[derive(Clone, PartialEq)]
pub(crate) enum MenuMode {
    Main,
    /// A panel menu (C11): owner and its command ids, `None` = separator.
    Panel(u64, Vec<Option<String>>),
    /// A column's overflow menu (`»`, sidebar-tabs.md): every tab.
    Tabs(sidebar_tabs::Column),
    /// The command palette (`host/palette`), in the main window.
    Palette,
    /// The approval sheet of `commands.run`, or a main-window
    /// `dialogs.confirm` (`host/review_sheet`), in the main window.
    Review,
    /// The bottom-bar strip's `»` (bar_buttons.rs): the plugin buttons not
    /// in the strip, in strip order (owner, command id).
    Bar(Vec<(u64, String)>),
}
/// One wheel step per this many logical pixels of accumulated scroll.
pub(crate) const WHEEL_STEP: f64 = 40.;
pub(crate) const OSD_MS: u64 = 2000;
/// A message with actions (`osd.show` with `actions`) stays this long, and
/// longer while the pointer is on it.
pub(crate) const OSD_ACTION_MS: u64 = 4000;
/// The longest a message with actions is held under the pointer, from when
/// it showed: Slint's `has-hover` changes only on pointer events, so a
/// message that moved under a still pointer (a fullscreen toggle by key,
/// the title strip sliding) would otherwise read as hovered for ever.
pub(crate) const OSD_HOLD_MS: u64 = 30_000;
/// After a message hides, its button titles are cleared once the 300 ms
/// fade is over (and 300 ms more), so no invisible, disabled buttons stay
/// in the accessibility tree.
pub(crate) const OSD_CLEAR_MS: u64 = 600;

/// The tick for a message whose time is up: `Some(until)` keeps it that
/// long. Only a message with actions is held, while the pointer is on it
/// and active (not idle), and never past [`OSD_HOLD_MS`] after `shown_at`.
pub(crate) fn osd_hold(
    now: u64,
    shown_at: u64,
    actions: bool,
    hover: bool,
    idle: bool,
) -> Option<u64> {
    let cap = shown_at.saturating_add(OSD_HOLD_MS);
    (actions && hover && !idle && now < cap).then(|| (now + OSD_MS).min(cap))
}

/// Why a message whose time is up goes (`VESTRA_DEBUG_OSD`), when
/// [`osd_hold`] does not keep it.
pub(crate) fn osd_hide_reason(
    now: u64,
    shown_at: u64,
    actions: bool,
    hover: bool,
    idle: bool,
) -> &'static str {
    if !actions || !hover {
        "time"
    } else if now >= shown_at.saturating_add(OSD_HOLD_MS) {
        "hold cap"
    } else if idle {
        "pointer idle"
    } else {
        "time"
    }
}

/// What the OSD message shows, as the host last wrote it to the shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OsdShown {
    pub(crate) text: String,
    /// The button titles; `""` for none.
    pub(crate) titles: [String; 2],
    pub(crate) visible: bool,
}

impl OsdShown {
    /// The shell properties that show this state.
    pub(crate) fn values(&self) -> [(&'static str, SValue); 4] {
        let text = |text: &str| SValue::String(text.into());
        [
            ("osd-text", text(&self.text)),
            ("osd-action-1", text(&self.titles[0])),
            ("osd-action-2", text(&self.titles[1])),
            ("osd-visible", self.visible.into()),
        ]
    }
}

/// `VESTRA_DEBUG_OSD=1`: a stderr line for every OSD message shown or
/// hidden, with the plugin that showed it, its buttons and why it went.
pub(crate) fn debug_osd() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_OSD").is_some_and(|v| v == "1"))
}

/// One `VESTRA_DEBUG_OSD` event.
pub(crate) enum OsdEvent<'a> {
    Show(&'a str),
    Hide(&'a str),
}

/// The `VESTRA_DEBUG_OSD` line of `event` for a message from `from` (a
/// plugin id, or "the player") with `actions` buttons.
pub(crate) fn osd_line(event: &OsdEvent<'_>, from: &str, actions: usize) -> String {
    let buttons = match actions {
        0 => "no actions".to_owned(),
        1 => "1 action".to_owned(),
        n => format!("{n} actions"),
    };
    match event {
        OsdEvent::Show(text) => format!("osd: show from {from}, {buttons}: {text:?}"),
        OsdEvent::Hide(reason) => format!("osd: hide ({reason}) from {from}, {buttons}"),
    }
}
/// The action buttons of the message on screen: the plugin that showed it
/// and the local ids of its commands, one per button.
pub(crate) struct OsdActions {
    pub(crate) owner: u64,
    pub(crate) commands: Vec<String>,
}
pub(crate) struct MountChange {
    pub(crate) reply: Reply,
    /// The panel it changes (`panel_key`); `reply.owner` is the plugin.
    pub(crate) key: u64,
    pub(crate) contribution: Option<Mount>,
    /// Its owner was already told `ui.preparing` (a change put back after
    /// its batch failed).
    pub(crate) announced: bool,
}
/// The host's core: the state that never touches a Slint object or a
/// window (doc/architecture/core-thread.md, plan step S2b; the CORE rows
/// of doc/process/research/core-loop-study.md §1.1). `Host` holds it as
/// `core`, still on the UI thread until plan step S3a.
pub(crate) struct Core {
    pub(crate) backend: Backend,
    /// The shared runner of the trusted plugins, while one runs. Declared
    /// before `plugins`: at quit it stops (all members at once) before the
    /// members are dropped.
    pub(crate) shared: Option<shared_runner::SharedRunner>,
    /// Restarts, quarantine and fail-open of the shared runner for this run.
    pub(crate) shared_policy: shared_runner::Policy,
    pub(crate) plugins: BTreeMap<u64, Plugin>,
    /// Moves when a plugin comes or goes in `plugins` (`add_plugin`, `stop`,
    /// `quit_stop`): a source of `packages.list` (`packages_key`).
    pub(crate) plugins_revision: u64,
    pub(crate) events: mpsc::Sender<Event>,
    pub(crate) headless: bool,
    pub(crate) commands: commands::Registry,
    /// The plugins (manifest ids) that have mounted a panel in this run,
    /// a replayed tab's placeholder included: `bindings.list` lists their
    /// `focus: "panel"` commands for the rest of the run
    /// (`Host::note_panel_plugins`).
    pub(crate) panel_plugins: std::collections::BTreeSet<String>,
    /// `commands.run` tickets and rates.
    pub(crate) command_runs: crate::capabilities::CommandRuns,
    pub(crate) clock: Instant,
    /// The sequence provider: owner, can-next, can-previous (C6).
    pub(crate) sequence: Option<(u64, bool, bool)>,
    pub(crate) last_source: Value,
    /// The rendering API's facts, preference and restart (`rendering.rs`).
    pub(crate) rendering: Rendering,
    pub(crate) source_state_replies: std::collections::BTreeSet<(u64, u64)>,
    pub(crate) host_revision: u64,
    /// The plugin process supervisor's core (doc/architecture/supervisor.md):
    /// the unit table (identity: `by_id`, `by_pid`, owner
    /// generations) and each unit's activation; the Host adapter is
    /// `host/supervisor/`.
    pub(crate) supervisor: Supervisor<Value>,
    pub(crate) activation_receipts: std::collections::VecDeque<Value>,
    /// `resolvers.fileTypes` as last pushed by the backend (an array).
    pub(crate) file_types: Value,
    /// The core's copy of the window facts (rule R9; plan step S2c): set
    /// only from a `UiReport::WindowFacts` newer than `window_facts_seq`.
    /// Conditions and triggers, `window.state`, `layout.state` and
    /// `ui.menu`'s refusals read it.
    pub(crate) window_facts: Value,
    /// The `seq` of the facts last applied (`None`: none yet).
    pub(crate) window_facts_seq: Option<u64>,
    /// Mount requests of other plugins waiting for the built-ins to start: reply, method, params.
    pub(crate) held_mounts: slot_claims::Gate<(Reply, String, Value)>,
    /// The large array of the request being answered, converted on its
    /// plugin's reader thread (`Event::Prepared`, `take_prepared`): owner,
    /// request id, rows.
    pub(crate) prepared: Option<(u64, u64, Box<crate::ui::ui_values::Prepared>)>,
    /// What a running plugin was refused because another holds it: the
    /// `packages.list` `warnings` entry and its text, per owner.
    pub(crate) warnings: revised::Revised<BTreeMap<u64, Vec<(Value, String)>>>,
    pub(crate) packages: package_manager::Manager,
    /// The `plugin-reaper` thread: every plugin the host stops is waited
    /// for (and killed after its grace) there (supervisor.md §1).
    pub(crate) reaper: process::reaper::Reaper,
    /// Core actions in flight that drop a repeated trigger.
    pub(crate) actions_busy: std::collections::BTreeSet<String>,
    /// `player.setStepSeek` per owner: (write sequence, seconds, keyframes);
    /// the newest pair wins.
    pub(crate) step_seek: BTreeMap<u64, (u64, f64, bool)>,
    pub(crate) step_writes: u64,
    /// Development loads of this run (`--plugin`, or re-enabled): directory, manifest.
    pub(crate) development: revised::Revised<BTreeMap<String, (PathBuf, process::Manifest)>>,
    /// Development loads the user disabled (this run only).
    pub(crate) dev_disabled: revised::Revised<std::collections::BTreeSet<String>>,
    /// Why a plugin last stopped on its own, by id.
    pub(crate) failures: revised::Revised<BTreeMap<String, String>>,
    /// A package RPC that starts a plugin and answers with its start's
    /// error (`setEnabled`, `reload`, a development load): the id, and
    /// the error of a spawn that failed while the RPC ran (`spawn_unit`).
    pub(crate) start_error: Option<(String, Option<RpcError>)>,
    /// An uninstall's data directories, resolved at each request, by unit:
    /// deleted at the core's `RemoveData` (supervisor.md).
    pub(crate) data_removals: BTreeMap<crate::supervisor::UnitId, Vec<PathBuf>>,
    /// The directory of a development load being started (`load`), for its
    /// spawn (`load_development_as`).
    pub(crate) dev_loading: Option<PathBuf>,
    /// `plugin.storage.session`: per plugin identity, for this player run.
    pub(crate) session_storage: session_storage::SessionStorage,
    /// What `bindings.changed` / `packages.changed` last announced.
    pub(crate) bindings_sent: u64,
    /// The binding list `bindings.changed` last announced: a table rebuilt
    /// to the same list (a plugin woken or parked) announces nothing.
    pub(crate) bindings_list_sent: Value,
    /// What the list behind `bindings_list_sent` showed was built from:
    /// the table's and the command registry's listed revisions. It is
    /// built again only once they moved (`notify_tick`).
    pub(crate) bindings_seen: [u64; 2],
    pub(crate) packages_sent: Value,
    /// The sources of the `packages.list` behind `packages_sent`: the list
    /// is built again only once they moved (`packages_key`).
    pub(crate) packages_seen: Option<crate::binding_manager::PackagesKey>,
    pub(crate) notified_at: u64,
    /// `commands.keys` (player 0.7.6): the binding table revision it was
    /// last checked at, and what each plugin (by manifest id) last got.
    pub(crate) keys_checked: u64,
    pub(crate) keys_sent: BTreeMap<String, Value>,
    /// `plugin.timeline` layers (R-M3).
    pub(crate) timeline: timeline::Timeline,
    /// Files the system or another launch asked to open.
    pub(crate) opens: open_files::OpenFiles,
    /// Trigger conditions (`crate::triggers`, SDK 1.10.0) and what the
    /// Host keeps beside them (`triggers.rs`).
    pub(crate) triggers: crate::triggers::Triggers,
    pub(crate) trigger_state: TriggerState,
    /// The plugin shaders' crash guard (`diagnostics.rs`).
    pub(crate) shader_guard: ShaderGuard,
    /// When the memory line last went to the host log (`diagnostics.rs`).
    pub(crate) memory_logged: Option<Instant>,
    /// On-demand start: the Host's side of standby, wake and
    /// classification (`supervisor/`, supervisor.md §4). Moved from `Host`
    /// in plan step S2c as its last field, so it drops after every other
    /// core field (after the backend, the shared runner and the plugins, as
    /// before); like every core field it now drops before the UI fields,
    /// none of which its drop touches (it closes the low-memory handle and
    /// the recorder's queue).
    pub(crate) on_demand: supervisor::OnDemand,
}
// `Core` may move to its own thread (plan step S3a): every field is `Send`.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Core>();
};
pub(crate) struct Host {
    /// The core part (`Core`). Declared first, so it is dropped first, as
    /// `backend`, `shared` and `plugins` (its first fields) were before the
    /// split.
    pub(crate) core: Core,
    pub(crate) views: BTreeMap<(u64, String), view::View>,
    pub(crate) dialog_busy: bool,
    /// Where the user last acted for each plugin: the window its
    /// `dialogs.confirm` belongs to (`origins`, palette-lead.md §2.3).
    pub(crate) origins: origins::Origins,
    /// A system confirmation is open (`dialog_busy` is also a file
    /// picker): it comes into view when another request is refused `busy`.
    pub(crate) confirm_open: bool,
    pub(crate) shell: Option<slint_interpreter::ComponentInstance>,
    pub(crate) mounts: BTreeMap<u64, Mount>,
    pub(crate) mount_queue: std::collections::VecDeque<MountChange>,
    pub(crate) preparing: Option<(Instant, MountChange)>,
    /// Shell preparations still to run one change each, after a batch of
    /// changes failed to compile (`prepare_mount_tick`).
    pub(crate) mount_singly: usize,
    pub(crate) mount_events: mpsc::SyncSender<composition::JsonCallback>,
    pub(crate) mount_input: mpsc::Receiver<composition::JsonCallback>,
    pub(crate) last_image: Option<slint::Image>,
    /// Software rendering in an OpenGL window: the main window's reused
    /// video texture (`vestra-video-texture`), `None` without one.
    pub(crate) video_texture: RefCell<Option<vestra_video_texture::Feed>>,
    /// The main shell the window's rendering notifier shows new video
    /// textures on (`attach_gpu`).
    pub(crate) video_target: Rc<RefCell<slint::Weak<slint_interpreter::ComponentInstance>>>,
    /// The main window has neither a rendering notifier nor the pacing hook's
    /// draw count (Slint's software renderer): its one winit event hook
    /// counts its draws (`count_draws`, `bind_window_events`). Outlives a
    /// shell, like that hook.
    pub(crate) winit_draws: Rc<std::cell::Cell<bool>>,
    pub(crate) shell_generation: u64,
    pub(crate) next_shell_generation: u64,
    pub(crate) menu_path: Vec<String>,
    /// Requested scroll offset of each open menu level (G-03).
    pub(crate) menu_scroll: Vec<f64>,
    /// G-15: submenu aim, and the open panels it aims at (by depth).
    pub(crate) menu_aim: menu::HoverAim,
    pub(crate) menu_panels: Vec<menu::Rect>,
    /// What the menu's window shows (`apply_menu_levels`); a cell, as
    /// `bind_shell` (which takes `&self`) writes it too.
    pub(crate) menu_shown: RefCell<menus::ShownMenu>,
    /// The open command palette (`MenuMode::Palette`).
    pub(crate) palette: palette::PaletteState,
    /// The palette's first row and its holder (`palette.lead`).
    pub(crate) lead: Option<crate::capabilities::Lead>,
    /// The status strip: the lead's status while the palette is closed.
    pub(crate) lead_strip: lead_strip::LeadStrip,
    /// The sheet waiting for the user (`MenuMode::Review`): a plan's or a
    /// confirmation's.
    pub(crate) review: Option<review_sheet::Review>,
    pub(crate) policy: window_state::Policy,
    /// The window facts the UI last sent the core (`UiReport::WindowFacts`,
    /// rule R9): `window_tick` sends a value only when it differs from
    /// this one. The UI's own readers (drops, `window.setVideoArea`) read
    /// it; the core reads its copy, `Core::window_facts`.
    pub(crate) window_facts: Value,
    /// The UI's one counter of facts values sent (`WindowFacts.seq`).
    pub(crate) facts_seq: u64,
    pub(crate) measured_shell: Option<u64>,
    pub(crate) menu_open: bool,
    /// The fullscreen title strip (`title-strip.slint`).
    pub(crate) title_strip: window_state::TitleStrip,
    /// The columns of tabs (by `sidebar_tabs::Column::index`): the sidebar
    /// and the adjust column.
    pub(crate) columns: [ColumnState; 2],
    /// The mounted panels that can be drawn, each compiled on its own when
    /// first shown and embedded in its slot's container (`ui/panels.rs`).
    pub(crate) panels: crate::ui::panels::Panels,
    /// The panels of the mount batch being applied whose file was compiled
    /// for it (for the log line of a panel kept over its plugin's restart).
    pub(crate) compiled_now: std::collections::BTreeSet<u64>,
    /// Panels kept while their plugin restarts after a crash
    /// (`panel_restart.rs`).
    pub(crate) restarts: panel_restart::Restarts,
    /// The plugins' start-up panels are in place: start-up settled with no
    /// mount left to apply (`open_files_tick`; a file given at launch opens
    /// only then, so no panel's compile lands on early playback).
    pub(crate) startup_panels: bool,
    /// Panels whose file did not compile when they were first shown
    /// (reported to their plugin): left out until they mount again.
    pub(crate) tabs_failed: std::collections::BTreeSet<u64>,
    /// The bottom bar was shown at the last tick.
    pub(crate) bottom_shown: bool,
    pub(crate) native_window: Option<platform::HostWindowId>,
    pub(crate) native_before: Option<platform::NativeSnapshot>,
    pub(crate) native_unavailable: Option<String>,
    pub(crate) input: Rc<RefCell<InputShared>>,
    pub(crate) ui_state: ui_state::Store,
    pub(crate) osd_until: Option<u64>,
    pub(crate) osd_actions: Option<OsdActions>,
    /// When the message on screen showed (`osd_hold`).
    pub(crate) osd_shown_at: u64,
    /// When the hidden message's button titles are cleared (`OSD_CLEAR_MS`).
    pub(crate) osd_clear_at: Option<u64>,
    /// What the OSD shows.
    pub(crate) osd_shown: OsdShown,
    /// The plugin that showed the message on screen; `None`: the player.
    pub(crate) osd_from: Option<u64>,
    pub(crate) info_panels: BTreeMap<u64, String>,
    pub(crate) floating: Option<Floating>,
    pub(crate) menu_mode: MenuMode,
    /// Window generation the open menu belongs to (shell or floating).
    pub(crate) menu_generation: u64,
    pub(crate) window_title: String,
    pub(crate) power: power::Power,
    /// When the shell last received a video frame (`power` keeps the display awake).
    pub(crate) last_frame: Option<Instant>,
    /// C-SIZE: the video area -> the renderer's target area (throttled).
    pub(crate) sizing: render_target::Sizing,
    /// `VESTRA_FRAME_LOG=1` diagnostics and the size last set on the UI.
    pub(crate) frame_log: Option<render_target::FrameLog>,
    pub(crate) shown_size: Option<render_target::Size>,
    /// `window.setAlwaysOnTop` and `setCursorAutoHide("playing")` per plugin.
    pub(crate) on_top: window_control::LastWrite<bool>,
    pub(crate) cursor_playing: window_control::LastWrite<bool>,
    /// The level last applied: (floating, window count when applied).
    pub(crate) on_top_applied: Option<(bool, usize)>,
    /// `window.setVideoArea` answered once the video area shows the target.
    pub(crate) area_replies: Vec<AreaReply>,
    /// Decoded `plugin.ui.image` files (R-U1).
    pub(crate) images: ui_image::Images,
    /// No video frame to show (R-U3): the backdrop is effective.
    pub(crate) no_video: bool,
    /// The source has no video, loading keeping the previous answer (PiP).
    pub(crate) video_absent: bool,
    /// GPU rendering: the video area was cleared for this empty phase.
    pub(crate) video_cleared: bool,
    /// A backdrop is mounted, so mpv is told not to show cover art (R-U3).
    pub(crate) cover_art_off: bool,
    /// `window.setPictureInPicture` (R-W5, R-H8).
    pub(crate) pip: pip::PictureInPicture<platform::PipWindow>,
    /// The last `VESTRA_PIP_LOG=1` line (printed when it changes).
    pub(crate) pip_log: Option<String>,
    /// The physical video area (C-SIZE) and the size last sent to the
    /// renderers (the area, or the picture's shape while PiP wants frames).
    pub(crate) render_area: Option<render_target::Size>,
    pub(crate) render_sent: Option<render_target::Size>,
    /// The area last applied belongs to a run of changes (a border drag),
    /// not a single step (`render_target::Applied`).
    pub(crate) render_continuous: bool,
    /// GPU rendering: PiP wants the renderer's frames (read by its frame
    /// callback on the render thread), and that callback is registered.
    pub(crate) pip_gpu: (Arc<AtomicBool>, bool),
    /// GPU rendering with host pacing: the frame callback that owes the main
    /// window a draw is registered (`gpu_pacing_hook`).
    pub(crate) gpu_paced: bool,
    /// File drops (R-U2).
    pub(crate) drops: drops::Drops,
    /// Settings pages and the options window (R-S1, R-H7).
    pub(crate) settings: settings_window::Settings,
    /// The category of the page the options window last showed.
    pub(crate) settings_category: Option<usize>,
    /// A view missed the palette: deliver again on the next tick.
    pub(crate) theme_pending: bool,
    /// New windows to put where the previous ones were (`view::Placements`).
    pub(crate) placements: view::Placements,
    /// Bottom-bar command buttons (`bar_buttons.rs`).
    pub(crate) bar: bar_buttons::Strip,
}
pub(crate) struct AreaReply {
    pub(crate) reply: Reply,
    pub(crate) video: (f64, f64),
    pub(crate) result: Value,
    pub(crate) deadline: u64,
}
