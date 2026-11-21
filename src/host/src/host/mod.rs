//! The `Host` and its `Core`. The core ([`Core`], behind `Host::link`) is the
//! state and the code that never touch the interface: the lifecycle of
//! plugins, the dispatch of their requests (`crate::capabilities`), the
//! supervisor, triggers, packages; its turn takes the core alone (plan step
//! S2f(3): `Core::handle`, `Core::report`, `Core::STEPS`), and what it
//! asks of the interface waits in its outbox (`outbox.rs`). The `Host` is
//! the UI thread's state (one per player) around it: the window, menu,
//! mount and input handling, and the driver's side of the turn (routing
//! events, reporting the interface's facts, draining the outbox). Many
//! files hold both, in `impl Host` and `impl Core` blocks side by side.
mod actions;
mod core_turn;
pub(crate) use actions::migrate_fullscreen_escape;
mod diagnostics;
pub(crate) use diagnostics::{
    ShaderGuard, persist_shader_guard, register_shader_writer, session_end,
};
mod dispatch;
pub(crate) mod driver;
mod events;
mod handle;
mod input;
pub(crate) mod lead_strip;
mod lifecycle;
mod menus;
pub(crate) mod messages;
mod mounts;
pub(crate) mod origins;
pub(crate) mod outbox;
pub(crate) mod palette;
pub(crate) mod panel_restart;
mod rendering;
mod restart;
pub(crate) mod review_sheet;
pub(crate) mod revised;
pub(crate) use rendering::{Relaunch, Rendering, restarting, startup as rendering_startup};
mod rpc;
mod shell;
pub(crate) mod stall;
mod state;
pub(crate) mod supervisor;
mod tables;
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

pub(crate) use events::{CoreQuit, Event, Injected, MenuAction};
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
/// The slot of a panel key (`panel_key`).
pub(crate) fn panel_slot(key: u64) -> composition::Slot {
    [
        composition::Slot::Bottom,
        composition::Slot::Sidebar,
        composition::Slot::Backdrop,
        composition::Slot::Adjust,
    ]
    .into_iter()
    .find(|&slot| slot as u64 == key % PANEL_SLOTS)
    .unwrap_or(composition::Slot::Bottom)
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
impl Mount {
    /// The core's record of this mount (plan step S3a(1)): everything but
    /// the values its panel was given (`properties`), which only the
    /// interface reads; `visible` is the interface's too, kept as it is.
    pub(crate) fn record(&self) -> Mount {
        Mount {
            spec: self.spec.clone(),
            plugin: self.plugin,
            properties: BTreeMap::new(),
            visible: self.visible,
            title: self.title.clone(),
            manifest: self.manifest.clone(),
            builtin: self.builtin,
            recorded: self.recorded,
            call: self.call.clone(),
        }
    }
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
    /// The interface's copy of the core's binding table (rule R8,
    /// `host/tables.rs`): read here by the key and mouse callbacks.
    pub(crate) table: bindings::Snapshot,
    pub(crate) bottom: Option<u64>,
    pub(crate) sidebar: Option<u64>,
    /// The adjust column's current tab (`panel:adjust` keys).
    pub(crate) adjust: Option<u64>,
    pub(crate) blocked: bool,
    /// A system dialog (file dialog or confirmation) is open (`dialog_busy`):
    /// an input that comes while `blocked` brings it to the front.
    pub(crate) dialog: bool,
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
#[derive(Clone)]
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
/// of doc/process/research/core-loop-study.md §1.1). `Host` holds it in
/// its `link`, reached only through the driver (`driver.rs`, plan step
/// S3a(1)), still on the UI thread until plan step S3a(2).
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
    /// The command registry (rule R8): the interface reads its copy,
    /// `Host::registry` (`host/tables.rs`).
    pub(crate) commands: commands::Registry,
    /// The binding table (rule R8), written through `bindings_mut`, and
    /// what the interface was last given of both tables
    /// (`host/tables.rs`).
    pub(crate) tables: tables::Tables,
    /// The plugins (manifest ids) that have mounted a panel in this run,
    /// a replayed tab's placeholder included: `bindings.list` lists their
    /// `focus: "panel"` commands for the rest of the run
    /// (`Core::note_panel_plugins`).
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
    /// The events the core took from its queue behind the interface's
    /// reports and handles after them, in their order, within the turn's
    /// budget (`core_events`; S3a(3) verification).
    pub(crate) held_events: std::collections::VecDeque<Event>,
    /// The facts `window.state` last broadcast (`Core::window_state_out`).
    pub(crate) window_state_sent: Value,
    /// The interface's facts as last reported (`UiReport::UiFacts`; design
    /// §5.3 group A, plan step S2f(1)).
    pub(crate) ui_facts: messages::UiFacts,
    /// Panels kept while their plugin restarts after a crash
    /// (`panel_restart.rs`): the core's record (plan step S2f(2); design
    /// §5.3 group C), and the waiting lines last sent to the interface.
    pub(crate) restarts: panel_restart::Restarts,
    pub(crate) restart_lines_sent: BTreeMap<u64, String>,
    /// `power.preventSleep` (K10; plan step S2f(2): its launcher and
    /// assertion are `Send`, by the ruled change of its test fake).
    pub(crate) power: power::Power,
    /// When the shell last received a video frame (`power` keeps the
    /// display awake): `UiReport::FrameShown`, last value wins.
    pub(crate) last_frame: Option<Instant>,
    /// Mount requests of other plugins waiting for the built-ins to start: reply, method, params.
    pub(crate) held_mounts: slot_claims::Gate<(Reply, String, Value)>,
    /// The mount registry (K6; plan step S2c(3)): every plugin's panel by
    /// its key (`panel_key`), a placeholder tab replayed from a recording
    /// included, with its spec, values and the `ui.mount` call a recording
    /// keeps. Standby recording, `rekey`, `revoke`, `prune` and `replay`
    /// read and write it here. A batch of changes is applied by the
    /// interface, whose copy (`Host::mounts`) the registry takes without
    /// the panels' values (`UiInput::Mounts`, plan step S3a(1)); the
    /// core's own changes reach that copy as `outbox::MountOp`s.
    pub(crate) mounts: BTreeMap<u64, Mount>,
    /// Mount changes the core accepted (`ui.mount`, `ui.unmount`, a replayed
    /// tab, a removal) that the interface has not taken into its batching
    /// window yet (`UiCommand::Mount`, `Core::mount_command`).
    pub(crate) mount_queue: std::collections::VecDeque<MountChange>,
    /// The registry of settings pages (K7; plan step S2c(4)): pages,
    /// registrations waiting for the `settings-check` thread, recorded
    /// pages and their recordings. The open options window is the
    /// interface's (`Host::settings`), told of changes as
    /// `UiCommand::Settings`.
    pub(crate) settings: settings_window::Settings,
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
    /// A native dialog is open, a file picker or a system confirmation
    /// (plan step S2c(5)): `dialogs.open`, `dialogs.confirm` and
    /// `media.sources.locate` are refused `busy` while it is, and it counts
    /// as `Busy::Dialog`. The interface reads it for its window policy
    /// (input blocked, `window.state.dialogOpen`).
    pub(crate) dialog_busy: bool,
    /// A system confirmation is open (`dialog_busy` is also a file
    /// picker): it comes into view when another request is refused
    /// `busy`. A confirmation that failed to show (`UiReport::Answered`
    /// marked `confirm_failed`) clears both.
    pub(crate) confirm_open: bool,
    /// Where the user last acted for each plugin: the window its
    /// `dialogs.confirm` belongs to (`origins`, palette-lead.md §2.3). The
    /// interface reports each act as `UiReport::Acted` (`Core::act`).
    pub(crate) origins: origins::Origins,
    /// The palette's first row and its holder (`palette.lead`; plan step
    /// S2c(5)): admission, the recording and the holder's lifecycle are
    /// the core's; the interface is told of each change as
    /// `UiCommand::Lead` and draws the row, the strip and the bar's icon.
    pub(crate) lead: Option<crate::capabilities::Lead>,
    /// The panel policy's tabs as the interface last reported them with
    /// the window facts (rule R9; plan step S2c(5)). The policy stays the
    /// interface's; the core judges `ui.interaction`'s and `layout.set`'s
    /// refusals and answers `layout.state`'s tab order from this copy.
    pub(crate) panel_tabs: state::PanelTabs,
    /// The info panels of `osd.panel`, by owner (plan step S2c(7)): the
    /// interface shows its copy (`Host::info_panels`), given whole as
    /// `UiSnapshot::InfoPanels` at every change.
    pub(crate) info_panels: BTreeMap<u64, String>,
    /// `window.setAlwaysOnTop` and `setCursorAutoHide("playing")` per
    /// plugin (plan step S2c(7)): the interface reads the values in effect
    /// from its copies (`Host::on_top`, `Host::cursor_playing`;
    /// `Core::window_settings`).
    pub(crate) on_top: window_control::LastWrite<bool>,
    pub(crate) cursor_playing: window_control::LastWrite<bool>,
    /// What the interface was last given of `on_top` and `cursor_playing`
    /// (`None`: nothing yet).
    pub(crate) window_settings_sent: (Option<bool>, Option<bool>),
    /// A backdrop is mounted, so mpv is told not to show cover art (R-U3;
    /// `Core::backdrop_cover_art`).
    pub(crate) cover_art_off: bool,
    /// The registry of file drops (R-U2; plan step S2c(7)): the providers,
    /// the panels that listen and the recorded providers. The drop being
    /// collected and where it lands are the interface's (`Host::drops`);
    /// a complete drop reaches the core as `UiReport::Drop`.
    pub(crate) drops: drops::Drops,
    /// The interface state file (`ui-state.json`; plan step S2c(7), design
    /// K8): the core is its one writer. Its own parts (the binding
    /// overlay, the appearance, the trusted plugins) it writes here; the
    /// interface's parts (window frame, columns, palette recents, the
    /// options window's last page) the interface reports whole as
    /// `UiReport::Remembered` when they change.
    pub(crate) ui_state: ui_state::Store,
    /// `window.setVideoArea` requests the interface fitted
    /// (`UiReport::AreaFitted`; plan step S2d), answered once the layout
    /// shows their size (`UiReport::VideoArea`) or a second after.
    pub(crate) area_replies: Vec<AreaReply>,
    /// The mount change the interface's batching window holds (plan step
    /// S2d(2)): set when the core hands one over (`mount_command`),
    /// replaced by the interface's `UiReport::Batching`, cleared when the
    /// core cancels it (`UiCommand::CancelMount`). The core's checks of
    /// pending mount changes read it, not the window.
    pub(crate) handed: Option<mounts::HandedMount>,
    /// The plugins' start-up panels are in place: start-up settled with no
    /// mount left to apply (`open_files_tick`, a core step; a file given at
    /// launch opens only then, so no panel's compile lands on early
    /// playback). The interface keeps a copy for its columns
    /// (`Host::startup_panels`, `UiSnapshot::StartupSettled`; plan step
    /// S2d(2)).
    pub(crate) startup_panels: bool,
    /// What `source.state` showed last (plan step S2d(4)): no video, the
    /// empty phase, the window title, sent to the interface as
    /// `UiSnapshot::Source` where a new state arrives
    /// (`source_display_next`).
    pub(crate) source_display: messages::SourceShown,
    /// The command registry's revision the binding table's disabled set
    /// last followed (`core_steps`; plan step S2e moved it from the
    /// windowed tick).
    pub(crate) binding_revision: u64,
    /// What the core asks of the interface, in order, and the core work
    /// kept behind it (plan step S2f(3), `outbox.rs`); plan step S3a(3):
    /// flushed into the core→UI queue after every event, report and step.
    pub(crate) outbox: outbox::Outbox,
    /// The number of the interface's facts last applied (`UiReport::UiFacts`;
    /// plan step S3a(3)).
    pub(crate) ui_facts_seq: u64,
    /// The batching window's hand-overs (`UiCommand::HandOver`; plan step
    /// S3a(3)): the last number given, and the cancelled ones whose late
    /// report is dropped until the window reports itself closed
    /// (`Core::batching`).
    pub(crate) handover_seq: u64,
    pub(crate) cancelled_handovers: Vec<u64>,
    /// The batching window as the core keeps it (plan step S3a(3); review
    /// items 6, B1, B2): the change it holds and since when (the core closes
    /// it 32 ms later, `Core::close_window`); the batch pushed and not yet
    /// reported applied (pending, `Core::handed_all`); after a failed batch,
    /// how many closes take one change each, and the next window is due at
    /// once.
    pub(crate) window: Option<(MountChange, Instant)>,
    pub(crate) batch_in_flight: Vec<mounts::HandedMount>,
    pub(crate) mount_singly: usize,
    pub(crate) retry_due: bool,
    /// The changes a failed batch gave back (owner, request id, panel):
    /// once none of them waits any more (applied, or dropped with their
    /// plugin), the single-change retry ends (S3a(3) re-review).
    pub(crate) retry_changes: Vec<(u64, u64, u64)>,
    /// The serial clipboard thread (`clipboard.*`; plan step S3a(3), the
    /// S3a brief's item 7), started at its first call.
    pub(crate) clipboard: crate::clipboard::Serial,
    /// Plan step S3a(4): the interface asked the core on its thread to end
    /// (`Event::CoreQuit`), and the core ran its own `quit` command (it then
    /// ends the run itself if the interface does not ask in time;
    /// `driver::run_core`).
    pub(crate) quit_asked: Option<CoreQuit>,
    pub(crate) quitting: bool,
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
    /// The core part (`Core`), reached only through the driver (plan step
    /// S3a(1), `driver.rs`). Declared first, so it is dropped first, as
    /// `backend`, `shared` and `plugins` (its first fields) were before the
    /// split.
    pub(crate) link: driver::CoreLink,
    /// The interface's own event sender (`driver::UiEvents`; plan step
    /// S3a(1)): its callbacks send there. Plan step S3a(3): its own queue
    /// (the S3a brief's item 1), drained at the interface's wake after the
    /// core→UI queue (R8), its receiver here.
    pub(crate) events: driver::UiEvents,
    pub(crate) ui_events: mpsc::Receiver<Event>,
    /// The core's one event queue, where the interface's reports go (plan
    /// step S3a(3)).
    pub(crate) to_core: mpsc::Sender<Event>,
    /// The core→UI queue (plan step S3a(3)), and the number of the last
    /// command applied from it.
    pub(crate) queue: outbox::UiQueue,
    pub(crate) applied_seq: u64,
    /// The core's wake of the interface, at most one pending (plan step
    /// S3a(4), `driver::Waker`): taken before the queue is drained.
    pub(crate) waker: driver::Waker,
    /// The most entries the interface found waiting at one wake (logged at
    /// quit: the S3a row's measurement of the queue's peak length).
    pub(crate) queue_peak: usize,
    /// The interface's clock, on the core's epoch (`Host::now_ms` gives
    /// the core's millisecond values; plan S3a brief item 10).
    pub(crate) clock: Instant,
    /// A headless run (no window; the core's `headless`).
    pub(crate) headless: bool,
    /// The playback handles the interface draws with, cloned at start.
    pub(crate) playback: driver::Playback,
    /// The interface's copy of the mount registry (plan step S3a(1); K6):
    /// the record it applies is the core's, the core's own changes reach
    /// it as `outbox::MountOp`s, and the values its panels were given
    /// (`properties`, `visible`) are its own.
    pub(crate) mounts: BTreeMap<u64, Mount>,
    /// The interface's copy of the settings pages (K7; plan step S3a(1)),
    /// given whole when the core's registry changed; the values the open
    /// window was given flow back to the core (`UiInput::Stored`).
    pub(crate) pages: Vec<settings_window::Page>,
    /// The interface's writes of `pages` the core has not handled yet,
    /// written again into each copy it is given (plan step S3a(3)).
    pub(crate) stored_in_flight: settings_window::StoredInFlight,
    /// The interface's copies of the core's small values (design §5.3 group
    /// D: the dialog flag, the lead, the run tickets, the origins).
    pub(crate) view: outbox::CoreFacts,
    /// The interface's copy of what it reads of the plugins (ids, names,
    /// packages).
    pub(crate) plugins: outbox::PluginsView,
    /// What makes its per-tick reports change nothing in the core
    /// (`outbox::TickFacts`; plan step S3a(2)): they are sent only otherwise.
    pub(crate) tick_facts: outbox::TickFacts,
    /// The displays last read (`Host::displays_tick`) and when.
    pub(crate) displays: Arc<Value>,
    pub(crate) displays_read: Option<Instant>,
    pub(crate) views: BTreeMap<(u64, String), view::View>,
    pub(crate) shell: Option<slint_interpreter::ComponentInstance>,
    /// The interface's mount batching window (K6): the change that opened
    /// it and when. The changes the core hands over during the next 32 ms
    /// join it (`prepare_mount_tick`).
    pub(crate) preparing: Option<(Instant, MountChange)>,
    /// Plan step S3a(3): the number of the hand-over the window holds.
    pub(crate) handover_seq: u64,
    /// The interface fitted a video area and reported it (`AreaFitted`);
    /// the next window tick sends the size (plan step S3a(3)).
    pub(crate) area_fitted_sent: bool,
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
    /// A facts value or a handoff went out since the last
    /// `UiReport::FactsSettled` (`Host::facts_settled`; S3a(4) review F2).
    pub(crate) facts_unsettled: bool,
    /// The window tick updates `window_facts` without a report of its own
    /// (a batch's layout pass, `apply_batch`; plan step S3b).
    pub(crate) facts_quiet: bool,
    /// The panel tabs the UI last sent the core with its facts
    /// (`report_tabs`; plan step S2c(5)).
    pub(crate) tabs_sent: state::PanelTabs,
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
    /// The interface's copy of the panels waiting for their plugin's
    /// restart, with the line drawn over each (`Core::restarts`;
    /// `PanelsOp::Restarting`, plan step S2f(2)).
    pub(crate) restart_lines: BTreeMap<u64, String>,
    /// The interface's copy of `Core::startup_panels` (plan step S2d(2);
    /// `UiSnapshot::StartupSettled`): its columns read it (`current_in`).
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
    /// The interface's part of the interface state file (plan step
    /// S2c(7)): window frame, columns, palette recents, the options
    /// window's last page. The interface reads and writes it here and
    /// reports it whole to the core, the file's writer, at every change
    /// (`Host::remember`).
    pub(crate) remembered: ui_state::Interface,
    /// The palette the interface shows: the core's appearance
    /// (`Core::theme`), given as `UiSnapshot::Theme` (plan step S2c(7)).
    pub(crate) shown_theme: String,
    /// The interface's copy of the core's last `source.state`
    /// (`Core::last_source`; plan step S2c(7)), replaced whole by
    /// `UiSnapshot::LastSource` wherever the core replaces its own.
    pub(crate) source: Arc<Value>,
    pub(crate) osd_until: Option<u64>,
    pub(crate) osd_actions: Option<OsdActions>,
    /// The facts last reported to the core (`UiReport::UiFacts`, plan step
    /// S2f(1)): a value is sent only when it differs.
    pub(crate) ui_facts_sent: Option<messages::UiFacts>,
    /// When the message on screen showed (`osd_hold`).
    pub(crate) osd_shown_at: u64,
    /// When the hidden message's button titles are cleared (`OSD_CLEAR_MS`).
    pub(crate) osd_clear_at: Option<u64>,
    /// What the OSD shows.
    pub(crate) osd_shown: OsdShown,
    /// The plugin that showed the message on screen; `None`: the player.
    pub(crate) osd_from: Option<u64>,
    /// The interface's copy of the core's info panels (`osd.panel`,
    /// `Core::info_panels`; `UiSnapshot::InfoPanels`).
    pub(crate) info_panels: Arc<BTreeMap<u64, String>>,
    pub(crate) floating: Option<Floating>,
    pub(crate) menu_mode: MenuMode,
    /// Window generation the open menu belongs to (shell or floating).
    pub(crate) menu_generation: u64,
    pub(crate) window_title: String,
    /// C-SIZE: the video area -> the renderer's target area (throttled).
    pub(crate) sizing: render_target::Sizing,
    /// `VESTRA_FRAME_LOG=1` diagnostics and the size last set on the UI.
    pub(crate) frame_log: Option<render_target::FrameLog>,
    pub(crate) shown_size: Option<render_target::Size>,
    /// The values of `window.setAlwaysOnTop` and
    /// `setCursorAutoHide("playing")` in effect, as the core last gave them
    /// (`Core::window_settings`; plan step S2c(7)).
    pub(crate) on_top: bool,
    pub(crate) cursor_playing: bool,
    /// The level last applied: (floating, window count when applied).
    pub(crate) on_top_applied: Option<(bool, usize)>,
    /// Decoded `plugin.ui.image` files (R-U1).
    pub(crate) images: ui_image::Images,
    /// No video frame to show (R-U3): the backdrop is effective.
    pub(crate) no_video: bool,
    /// The source has no video, loading keeping the previous answer (PiP).
    pub(crate) video_absent: bool,
    /// GPU rendering: the video area was cleared for this empty phase.
    pub(crate) video_cleared: bool,
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
    /// File drops (R-U2): the drop being collected (`Drops::arrived`,
    /// `due`). Only that part of this value is used: the registry of
    /// providers and listening panels is the core's (`Core::drops`; plan
    /// step S2c(7)).
    pub(crate) drops: drops::Drops,
    /// The options window (R-H7; K7): the pages it shows are the core's
    /// (`Core::settings`).
    pub(crate) settings: settings_window::Options,
    /// The category of the page the options window last showed.
    pub(crate) settings_category: Option<usize>,
    /// A view missed the palette: deliver again on the next tick.
    pub(crate) theme_pending: bool,
    /// New windows to put where the previous ones were (`view::Placements`).
    pub(crate) placements: view::Placements,
    /// Bottom-bar command buttons (`bar_buttons.rs`).
    pub(crate) bar: bar_buttons::Strip,
    /// The interface's copy of the core's command registry (rule R8,
    /// `host/tables.rs`): menus, the palette, the bar, the tab keys and
    /// the video click read it.
    pub(crate) registry: Arc<commands::View>,
}
pub(crate) struct AreaReply {
    pub(crate) reply: Reply,
    pub(crate) video: (f64, f64),
    pub(crate) result: Value,
    pub(crate) deadline: u64,
}
