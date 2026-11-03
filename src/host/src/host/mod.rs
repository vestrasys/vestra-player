//! The `Host`: the UI thread's state (one per player), its events, the
//! lifecycle of plugins, the dispatch of their requests and the window,
//! menu, mount and input handling. Capabilities (`crate::capabilities`) and
//! a few other modules add their own `impl Host` blocks.
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
mod lifecycle;
mod menus;
mod mounts;
mod rendering;
mod restart;
pub(crate) mod revised;
pub(crate) use rendering::{Relaunch, Rendering, startup as rendering_startup};
mod rpc;
mod shell;
mod state;
pub(crate) mod supervisor;
mod video;
mod window;

use crate::packages::{manifest, package_manager};
#[cfg(feature = "producer-audio")]
use crate::playback::producer_input;
use crate::playback::{
    backend::{Backend, Command},
    frame_pacing, render_target, video_click,
};
use crate::process::{Plugin, StopReason, shared_runner};
use crate::supervisor::{Pid, Supervisor, Trigger, activation};
use crate::ui::{
    bar_buttons, composition, drops, menu, pip, settings_window, slot_claims, ui_image, ui_state,
    ui_values, view, window_control, window_state,
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

pub(crate) use events::{Event, Injected, MenuAction};
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
pub(crate) use video::{FRAME_SINK, PIP_GPU_SINK};

#[derive(Clone)]
pub(crate) struct Mount {
    pub(crate) spec: composition::CompileSpec,
    pub(crate) properties: BTreeMap<String, Value>,
    /// The bottom bar's preference; a sidebar panel's come from the slot
    /// (`ui_state::SidebarSlot`, then the policy).
    pub(crate) visible: bool,
    pub(crate) detached: bool,
    /// The floating window's title and the sidebar tab's label: the mount's
    /// `title`, else the plugin's name.
    pub(crate) title: String,
    pub(crate) manifest: String,
    /// A built-in plugin's panel: its tab comes first.
    pub(crate) builtin: bool,
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
}
/// One wheel step per this many logical pixels of accumulated scroll.
pub(crate) const WHEEL_STEP: f64 = 40.;
pub(crate) const OSD_MS: u64 = 2000;
/// At start-up, how long a queued mount waits for the other built-ins'
/// mounts, so that the shell compiles once for all of them.
pub(crate) const STARTUP_MOUNT_GATHER: Duration = Duration::from_millis(400);
pub(crate) struct MountChange {
    pub(crate) reply: Reply,
    pub(crate) contribution: Option<Mount>,
    /// Its owner was already told `ui.preparing` (a change put back after
    /// its batch failed).
    pub(crate) announced: bool,
}
pub(crate) struct Host {
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

    pub(crate) views: BTreeMap<(u64, String), view::View>,
    pub(crate) events: mpsc::Sender<Event>,
    pub(crate) headless: bool,
    pub(crate) commands: commands::Registry,
    pub(crate) dialog_busy: bool,
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
    /// textures on (`attach_gpu`): it outlives a shell, since a new
    /// composition replaces the tree in the same window (`retarget_shell`).
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
    pub(crate) policy: window_state::Policy,
    pub(crate) clock: Instant,
    pub(crate) window_facts: Value,
    pub(crate) measured_shell: Option<u64>,
    pub(crate) menu_open: bool,
    /// G-01: the fullscreen sidebar overlay was popped out at the last tick.
    pub(crate) sidebar_popped: bool,
    /// The current sidebar tab at the last tick, and whether the user just
    /// clicked a tab (which gave the sidebar the keys already).
    pub(crate) sidebar_active: Option<u64>,
    pub(crate) tab_clicked: bool,
    /// The sidebar's panels in the current shell, in its tab order (its
    /// `sidebar-active` indexes this; the policy may already have dropped one).
    pub(crate) shell_tabs: Vec<u64>,
    /// The bottom bar was shown at the last tick.
    pub(crate) bottom_shown: bool,
    pub(crate) native_window: Option<platform::HostWindowId>,
    pub(crate) native_before: Option<platform::NativeSnapshot>,
    pub(crate) native_unavailable: Option<String>,
    pub(crate) input: Rc<RefCell<InputShared>>,
    pub(crate) ui_state: ui_state::Store,
    pub(crate) osd_until: Option<u64>,
    pub(crate) info_panels: BTreeMap<u64, String>,
    /// The sequence provider: owner, can-next, can-previous (C6).
    pub(crate) sequence: Option<(u64, bool, bool)>,
    pub(crate) last_source: Value,
    /// The rendering API's facts, preference and restart (`rendering.rs`).
    pub(crate) rendering: Rendering,
    pub(crate) source_state_replies: std::collections::BTreeSet<(u64, u64)>,
    pub(crate) floating: Option<Floating>,
    pub(crate) menu_mode: MenuMode,
    /// Window generation the open menu belongs to (shell or floating).
    pub(crate) menu_generation: u64,
    pub(crate) host_revision: u64,
    pub(crate) window_title: String,
    /// The plugin process supervisor's core (doc/architecture/supervisor.md):
    /// the unit table (identity: `by_id`, `by_pid`, owner
    /// generations) and each unit's activation; the Host adapter is
    /// `host/supervisor/`.
    pub(crate) supervisor: Supervisor<Value>,
    pub(crate) activation_receipts: std::collections::VecDeque<Value>,
    /// `resolvers.fileTypes` as last pushed by the backend (an array).
    pub(crate) file_types: Value,
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
    pub(crate) power: power::Power,
    /// When the shell last received a video frame (`power` keeps the display awake).
    pub(crate) last_frame: Option<Instant>,
    /// C-SIZE: the video area -> the renderer's target area (throttled).
    pub(crate) sizing: render_target::Sizing,
    /// `VESTRA_FRAME_LOG=1` diagnostics and the size last set on the UI.
    pub(crate) frame_log: Option<render_target::FrameLog>,
    pub(crate) shown_size: Option<render_target::Size>,
    /// Core actions in flight that drop a repeated trigger.
    pub(crate) actions_busy: std::collections::BTreeSet<String>,
    /// `player.setStepSeek` per owner: (write sequence, seconds, keyframes);
    /// the newest pair wins.
    pub(crate) step_seek: BTreeMap<u64, (u64, f64, bool)>,
    pub(crate) step_writes: u64,
    /// `window.setAlwaysOnTop` and `setCursorAutoHide("playing")` per plugin.
    pub(crate) on_top: window_control::LastWrite<bool>,
    pub(crate) cursor_playing: window_control::LastWrite<bool>,
    /// The level last applied: (floating, window count when applied).
    pub(crate) on_top_applied: Option<(bool, usize)>,
    /// `window.setVideoArea` answered once the video area shows the target.
    pub(crate) area_replies: Vec<AreaReply>,
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
    /// `plugin.timeline` layers (R-M3).
    pub(crate) timeline: timeline::Timeline,
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
    /// GPU rendering: PiP wants the renderer's frames (read by its frame
    /// callback on the render thread), and that callback is registered.
    pub(crate) pip_gpu: (Arc<AtomicBool>, bool),
    /// GPU rendering with host pacing: the frame callback that owes the main
    /// window a draw is registered (`gpu_pacing_hook`).
    pub(crate) gpu_paced: bool,
    /// File drops (R-U2).
    pub(crate) drops: drops::Drops,
    /// Files the system or another launch asked to open.
    pub(crate) opens: open_files::OpenFiles,
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
    /// On-demand start: the Host's side of standby, wake and
    /// classification (`supervisor/`, supervisor.md §4).
    pub(crate) on_demand: supervisor::OnDemand,
    /// The plugin shaders' crash guard (`diagnostics.rs`).
    pub(crate) shader_guard: ShaderGuard,
    /// When the memory line last went to the host log (`diagnostics.rs`).
    pub(crate) memory_logged: Option<Instant>,
}
pub(crate) struct AreaReply {
    pub(crate) reply: Reply,
    pub(crate) video: (f64, f64),
    pub(crate) result: Value,
    pub(crate) deadline: u64,
}
