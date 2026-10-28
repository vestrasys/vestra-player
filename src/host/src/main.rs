#![forbid(unsafe_code)]
// Windows: the release build is a GUI program (no console window when
// started from Explorer); debug builds keep their console for the logs. Where
// the logs go: doc/architecture/packaging-proposal.md §10.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `diag!`, used everywhere below (declared first for its textual scope).
#[macro_use]
mod diag;
mod backend;
mod bar_buttons;
mod binding_manager;
mod bindings;
mod capabilities;
mod clipboard;
mod commands;
pub mod composition;
mod confirm;
mod core_actions;
mod drops;
mod effects;
pub mod font_asset;
pub mod font_carrier;
mod frame_pacing;
mod i18n;
mod input_trace;
mod launch;
// Windows: libmpv reads leased local files through stream_cb (Unix: fd://).
#[cfg(any(windows, test))]
mod lease_stream;
mod manifest;
mod media_jobs;
mod media_tracks;
mod media_worker;
mod menu;
mod native_subtitle;
mod offline_audio;
mod offline_descriptor;
mod open_files;
pub mod package_activation;
mod package_archive;
mod package_manager;
pub mod package_stage;
pub mod package_store;
mod package_validate;
mod pip;
mod platform;
mod plugins;
mod power;
mod processing;
pub mod processing_operations;
#[cfg(feature = "producer-audio")]
mod producer;
#[cfg(feature = "producer-audio")]
mod producer_audio;
#[cfg(feature = "producer-audio")]
mod producer_controls;
#[cfg(feature = "producer-audio")]
pub mod producer_input;
#[cfg(feature = "producer-audio")]
mod producer_input_delivery;
mod runtime_paths;

#[cfg(feature = "producer-audio")]
pub mod producer_binary;
mod render_target;
mod renderer;
mod secret_index;
mod session;
mod session_storage;
mod settings_window;
mod shared_runner;
mod slot_claims;
mod source_backend;
mod source_geometry;
pub mod source_lease;
mod source_resolvers;
mod subtitle_backend;
pub mod subtitle_format;
pub mod subtitle_state;
pub mod subtitle_style;
pub mod subtitle_upload;
mod system;
mod theme;
mod timeline;
mod ui;
mod ui_fonts;
mod ui_image;
mod ui_markdown;
#[cfg(test)]
mod ui_shots;
mod ui_state;
#[cfg(test)]
mod ui_types;
mod ui_values;
mod video_click;
mod video_render;
mod video_view;
pub mod window_control;
pub mod window_state;
use anyhow::{Result, bail};
use backend::{Backend, Command};
use plugins::Plugin;
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

#[derive(Serialize, Debug, Clone)]
struct RpcError {
    code: String,
    message: String,
    /// Extra fields beside `code` and `message` (a slot refusal's `reason`, `slot`, `owner`).
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    detail: Option<serde_json::Map<String, Value>>,
}
impl RpcError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }
    /// Refused because another plugin holds a single-owner resource:
    /// `conflict` with a `reason` (and `owner`, the holder's id). Such a
    /// refusal does not fail activation (`activation_outcome`, the runner).
    fn taken(reason: &str, message: String, detail: Value) -> Self {
        let mut error = Self::new("conflict", message);
        if let Value::Object(mut detail) = detail {
            detail.insert("reason".into(), reason.into());
            error.detail = Some(detail);
        }
        error
    }
    /// `ui.mount`: the slot shows another plugin's panel, or will.
    fn slot_occupied(slot: composition::Slot, holder: &str) -> Self {
        Self::taken(
            "slot-occupied",
            slot_claims::refusal(slot, holder),
            json!({"slot":slot.name(),"owner":holder}),
        )
    }
    /// `sequence.set`: another plugin provides next/previous.
    /// The sidebar has all the tabs it takes (`slot_claims::MAX_SIDEBAR_TABS`);
    /// like a taken slot, it does not fail activation.
    fn sidebar_full() -> Self {
        Self::taken(
            "slot-occupied",
            i18n::tr_args(
                "The sidebar holds at most {} tabs",
                &json!([slot_claims::MAX_SIDEBAR_TABS]),
            ),
            json!({"slot":"sidebar"}),
        )
    }
    fn sequence_taken(holder: &str) -> Self {
        Self::taken(
            "sequence-taken",
            i18n::tr_args("Previous/Next is provided by {}", &json!([holder])),
            json!({"owner":holder}),
        )
    }
    fn is_taken(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| {
            matches!(
                d.get("reason").and_then(Value::as_str),
                Some("slot-occupied" | "sequence-taken" | "resolver-taken")
            )
        })
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
    /// The same failure, naming the property it is about.
    fn for_property(mut self, property: &str) -> Self {
        self.message = format!("{property}: {}", self.message);
        self
    }
    /// A property value the player stores now and converts later, checked
    /// against the value limits now (`ui_values::check`).
    fn check_property(property: &str, value: &Value) -> std::result::Result<(), Self> {
        ui_values::check(value).map_err(|error| Self::from(error).for_property(property))
    }
}
impl From<anyhow::Error> for RpcError {
    fn from(value: anyhow::Error) -> Self {
        if value.downcast_ref::<manifest::NeedsPlayer>().is_some() {
            return Self::new("versionMismatch", value.to_string());
        }
        if value.downcast_ref::<ui_values::OverLimit>().is_some() {
            return Self::new("limit", value.to_string());
        }
        Self::new("host", value.to_string())
    }
}
#[derive(Clone)]
struct Reply {
    owner: u64,
    id: u64,
}
enum MenuAction {
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
enum Injected {
    Pressed,
    Repeated,
    Released,
}
enum Event {
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
    Reply(Reply, std::result::Result<Value, RpcError>),
    /// A core action finished on the control worker: (requester, action, `{ osd }`).
    ActionDone(Reply, String, std::result::Result<Value, RpcError>),
    Broadcast(String, Value),
    UiCallback(u64, Value),
    /// The user closed a plugin tool window: (owner, view id).
    ViewClosed(u64, String),
    PluginMessage(u64, String, Value),
    Exited(u64),
    /// A control line of the shared runner (`g` 0): (runner, message).
    Runner(shared_runner::RunnerId, Value),
    /// The shared runner's stdout closed (it exited or was killed).
    RunnerExited(shared_runner::RunnerId),
    Reaped(u64),
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
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    method: String,
    #[serde(default)]
    params: Value,
}
#[derive(Clone)]
struct Mount {
    spec: composition::CompileSpec,
    properties: BTreeMap<String, Value>,
    /// The bottom bar's preference; a sidebar panel's come from the slot
    /// (`ui_state::SidebarSlot`, then the policy).
    visible: bool,
    detached: bool,
    /// The floating window's title and the sidebar tab's label: the mount's
    /// `title`, else the plugin's name.
    title: String,
    manifest: String,
    /// A built-in plugin's panel: its tab comes first.
    builtin: bool,
}
/// The detached sidebar's own window (C14).
struct Floating {
    instance: slint_interpreter::ComponentInstance,
    /// The sidebar's panels when it opened, in tab order.
    owners: Vec<u64>,
    generation: u64,
    /// Its native window, for the pointer at a file drop.
    native: Option<platform::HostWindowId>,
}
/// Read synchronously by the shell's key/mouse callbacks, without borrowing Host.
#[derive(Default)]
struct InputShared {
    table: bindings::Table,
    bottom: Option<u64>,
    sidebar: Option<u64>,
    blocked: bool,
    wheel: (f64, f64),
    /// The key press being dispatched, recorded by the window's winit hook.
    physical: Option<bindings::Physical>,
    /// Slint modifier key codes winit saw released, to hand to Slint.
    released: Vec<char>,
    /// A left press on the video waiting to become `Mouse:LeftClick` (R-H11).
    click: Option<video_click::Press>,
    /// The pointer in a window (its generation, logical px), from its winit
    /// hook; G-15 aims submenus with it.
    pointer: Option<(u64, menu::Point)>,
}
/// Which window shows the open menu, and what it lists.
#[derive(Clone, PartialEq)]
enum MenuMode {
    Main,
    /// A panel menu (C11): owner and its command ids, `None` = separator.
    Panel(u64, Vec<Option<String>>),
}
/// One wheel step per this many logical pixels of accumulated scroll.
const WHEEL_STEP: f64 = 40.;
const OSD_MS: u64 = 2000;
/// At start-up, how long a queued mount waits for the other built-ins'
/// mounts, so that the shell compiles once for all of them.
const STARTUP_MOUNT_GATHER: Duration = Duration::from_millis(400);
struct MountChange {
    reply: Reply,
    contribution: Option<Mount>,
    /// Its owner was already told `ui.preparing` (a change put back after
    /// its batch failed).
    announced: bool,
}
struct Host {
    backend: Backend,
    /// The shared runner of the trusted plugins, while one runs. Declared
    /// before `plugins`: at quit it stops (all members at once) before the
    /// members are dropped.
    shared: Option<shared_runner::SharedRunner>,
    /// Restarts, quarantine and fail-open of the shared runner for this run.
    shared_policy: shared_runner::Policy,
    plugins: BTreeMap<u64, Plugin>,
    views: BTreeMap<(u64, String), ui::View>,
    events: mpsc::Sender<Event>,
    next_generation: u64,
    headless: bool,
    commands: commands::Registry,
    dialog_busy: bool,
    shell: Option<slint_interpreter::ComponentInstance>,
    mounts: BTreeMap<u64, Mount>,
    mount_queue: std::collections::VecDeque<MountChange>,
    preparing: Option<(Instant, MountChange)>,
    /// Shell preparations still to run one change each, after a batch of
    /// changes failed to compile (`prepare_mount_tick`).
    mount_singly: usize,
    mount_events: mpsc::SyncSender<composition::JsonCallback>,
    mount_input: mpsc::Receiver<composition::JsonCallback>,
    last_image: Option<slint::Image>,
    /// Software rendering in an OpenGL window: the main window's reused
    /// video texture (`vestra-video-texture`), `None` without one.
    video_texture: RefCell<Option<vestra_video_texture::Feed>>,
    shell_generation: u64,
    next_shell_generation: u64,
    menu_path: Vec<String>,
    /// Requested scroll offset of each open menu level (G-03).
    menu_scroll: Vec<f64>,
    /// G-15: submenu aim, and the open panels it aims at (by depth).
    menu_aim: menu::HoverAim,
    menu_panels: Vec<menu::Rect>,
    policy: window_state::Policy,
    clock: Instant,
    window_facts: Value,
    measured_shell: Option<u64>,
    menu_open: bool,
    /// G-01: the fullscreen sidebar overlay was popped out at the last tick.
    sidebar_popped: bool,
    /// The current sidebar tab at the last tick, and whether the user just
    /// clicked a tab (which gave the sidebar the keys already).
    sidebar_active: Option<u64>,
    tab_clicked: bool,
    /// The sidebar's panels in the current shell, in its tab order (its
    /// `sidebar-active` indexes this; the policy may already have dropped one).
    shell_tabs: Vec<u64>,
    /// The bottom bar was shown at the last tick.
    bottom_shown: bool,
    native_window: Option<platform::HostWindowId>,
    native_before: Option<platform::NativeSnapshot>,
    native_unavailable: Option<String>,
    input: Rc<RefCell<InputShared>>,
    ui_state: ui_state::Store,
    osd_until: Option<u64>,
    info_panels: BTreeMap<u64, String>,
    /// The sequence provider: owner, can-next, can-previous (C6).
    sequence: Option<(u64, bool, bool)>,
    last_source: Value,
    source_state_replies: std::collections::BTreeSet<(u64, u64)>,
    floating: Option<Floating>,
    menu_mode: MenuMode,
    /// Window generation the open menu belongs to (shell or floating).
    menu_generation: u64,
    host_revision: u64,
    window_title: String,
    activations: package_activation::Book,
    activation_ui: std::collections::BTreeSet<(u64, u64)>,
    activation_receipts: std::collections::VecDeque<Value>,
    /// `resolvers.fileTypes` as last pushed by the backend (an array).
    file_types: Value,
    /// Mount requests of other plugins waiting for the built-ins to start: reply, method, params.
    held_mounts: slot_claims::Gate<(Reply, String, Value)>,
    /// What a running plugin was refused because another holds it: the
    /// `packages.list` `warnings` entry and its text, per owner.
    warnings: BTreeMap<u64, Vec<(Value, String)>>,
    packages: package_manager::Manager,
    cleanup: BTreeMap<u64, (bool, bool)>,
    power: power::Power,
    /// When the shell last received a video frame (`power` keeps the display awake).
    last_frame: Option<Instant>,
    /// C-SIZE: the video area -> the renderer's target area (throttled).
    sizing: render_target::Sizing,
    /// `VESTRA_FRAME_LOG=1` diagnostics and the size last set on the UI.
    frame_log: Option<render_target::FrameLog>,
    shown_size: Option<render_target::Size>,
    /// Core actions in flight that drop a repeated trigger.
    actions_busy: std::collections::BTreeSet<String>,
    /// `player.setStepSeek` per owner: (write sequence, seconds, keyframes);
    /// the newest pair wins.
    step_seek: BTreeMap<u64, (u64, f64, bool)>,
    step_writes: u64,
    /// `window.setAlwaysOnTop` and `setCursorAutoHide("playing")` per plugin.
    on_top: window_control::LastWrite<bool>,
    cursor_playing: window_control::LastWrite<bool>,
    /// The level last applied: (floating, window count when applied).
    on_top_applied: Option<(bool, usize)>,
    /// `window.setVideoArea` answered once the video area shows the target.
    area_replies: Vec<AreaReply>,
    /// Development loads of this run (`--plugin`, or re-enabled): directory, manifest.
    development: BTreeMap<String, (PathBuf, plugins::Manifest)>,
    /// Development loads the user disabled (this run only).
    dev_disabled: std::collections::BTreeSet<String>,
    /// Why a plugin last stopped on its own, by id.
    failures: BTreeMap<String, String>,
    /// `uninstall({ removeData })`: data directories to delete once reaped.
    data_removals: BTreeMap<u64, PathBuf>,
    /// `plugin.storage.session`: per plugin identity, for this player run.
    session_storage: session_storage::SessionStorage,
    /// What `bindings.changed` / `packages.changed` last announced.
    bindings_sent: u64,
    packages_sent: Value,
    notified_at: u64,
    /// `plugin.timeline` layers (R-M3).
    timeline: timeline::Timeline,
    /// Decoded `plugin.ui.image` files (R-U1).
    images: ui_image::Images,
    /// No video frame to show (R-U3): the backdrop is effective.
    no_video: bool,
    /// The source has no video, loading keeping the previous answer (PiP).
    video_absent: bool,
    /// GPU rendering: the video area was cleared for this empty phase.
    video_cleared: bool,
    /// A backdrop is mounted, so mpv is told not to show cover art (R-U3).
    cover_art_off: bool,
    /// `window.setPictureInPicture` (R-W5, R-H8).
    pip: pip::PictureInPicture<platform::PipWindow>,
    /// The last `VESTRA_PIP_LOG=1` line (printed when it changes).
    pip_log: Option<String>,
    /// The physical video area (C-SIZE) and the size last sent to the
    /// renderers (the area, or the picture's shape while PiP wants frames).
    render_area: Option<render_target::Size>,
    render_sent: Option<render_target::Size>,
    /// GPU rendering: PiP wants the renderer's frames (read by its frame
    /// callback on the render thread), and that callback is registered.
    pip_gpu: (Arc<AtomicBool>, bool),
    /// File drops (R-U2).
    drops: drops::Drops,
    /// Files the system or another launch asked to open.
    opens: open_files::OpenFiles,
    /// Settings pages and the options window (R-S1, R-H7).
    settings: settings_window::Settings,
    /// The category of the page the options window last showed.
    settings_category: Option<usize>,
    /// A view missed the palette: deliver again on the next tick.
    theme_pending: bool,
    /// New windows to put where the previous ones were (`ui::Placements`).
    placements: ui::Placements,
    /// Bottom-bar command buttons (`bar_buttons.rs`).
    bar: bar_buttons::Strip,
}
struct AreaReply {
    reply: Reply,
    video: (f64, f64),
    result: Value,
    deadline: u64,
}
impl Host {
    fn activation(&mut self, owner: u64) -> Result<Value> {
        if let Some(plugin) = self.plugins.get(&owner) {
            let id = plugin.manifest.id.clone();
            let now = self.now_ms();
            let status = self.activations.status(owner, now)?;
            return Ok(json!({"owner":owner,"id":id,"running":true,"activation":status}));
        }
        Ok(self
            .activation_receipts
            .iter()
            .find(|receipt| receipt["owner"] == owner)
            .cloned()
            .unwrap_or_else(
                || json!({"owner":owner,"running":false,"activation":{"status":"unknown"}}),
            ))
    }
    fn archive_activation(&mut self, owner: u64, message: &str) {
        let now = self.now_ms();
        let _ = self.activations.runtime_failed(owner, message.into(), now);
        if let Ok(mut receipt) = self.activation(owner) {
            receipt["running"] = false.into();
            self.activation_receipts
                .retain(|receipt| receipt["owner"] != owner);
            self.activation_receipts.push_back(receipt);
            if self.activation_receipts.len() > 128 {
                self.activation_receipts.pop_front();
            }
        }
        self.activations.remove(owner);
        self.activation_ui
            .retain(|(candidate, _)| *candidate != owner);
    }
    fn activation_tick(&mut self) {
        let owners = self.plugins.keys().copied().collect::<Vec<_>>();
        let now = self.now_ms();
        for owner in owners {
            let status = match self.activations.status(owner, now) {
                Ok(status) => status,
                Err(error) => {
                    diag!("activation status: {error}");
                    continue;
                }
            };
            let terminal_failure = matches!(
                status,
                package_activation::Status::Failed { .. } | package_activation::Status::TimedOut
            );
            if terminal_failure {
                let message = match status {
                    package_activation::Status::Failed { message } => message,
                    _ => "activation timed out".into(),
                };
                self.record_failure(owner, &message);
                self.disable(owner);
            }
        }
    }
    #[cfg(feature = "producer-audio")]
    fn physical_input(&self, params: Value) {
        if self
            .backend
            .commands
            .send(Command {
                reply: Reply { owner: 0, id: 0 },
                method: "host.physicalInput".into(),
                params,
            })
            .is_err()
        {
            diag!("host.error physical-input backend unavailable");
        }
    }
    #[cfg(feature = "producer-audio")]
    fn physical_revoke(&self, reason: producer_input::RevokeReason) {
        self.physical_input(json!({"kind":"revoke","reason":reason}));
    }
    /// Load a development package (`--plugin DIR`).
    fn load(&mut self, directory: &Path, timeout_ms: u64) -> Result<()> {
        if !(1_000..=120_000).contains(&timeout_ms) {
            bail!("activation timeout must be 1000..120000ms");
        }
        // Validate before spawning, preventing duplicate plugin activation side effects.
        let candidate = manifest::read_development(directory)?;
        if self.plugins.values().any(|p| p.manifest.id == candidate.id) {
            bail!("plugin already running: {}", candidate.id);
        }
        // Built-in plugins are developed with `--plugin`, so a reserved id
        // loads here; a development load is never installed (data under
        // plugin-data/development/), so it cannot become a built-in.
        if manifest::reserved_id(&candidate.id) {
            diag!(
                "host.warning development load {}: vestra. ids are reserved for built-in plugins; \
                 this load is for built-in development only and cannot be installed",
                candidate.id
            );
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|owner| *owner <= 9_007_199_254_740_991)
            .ok_or_else(|| anyhow::anyhow!("plugin owner identities exhausted"))?;
        self.activations.begin(
            self.next_generation,
            self.now_ms()
                .checked_add(timeout_ms)
                .ok_or_else(|| anyhow::anyhow!("activation deadline overflow"))?,
        )?;
        let plugin = match Plugin::load(directory, self.next_generation, self.events.clone()) {
            Ok(plugin) => plugin,
            Err(error) => {
                self.activations.remove(self.next_generation);
                return Err(error);
            }
        };
        let manifest_id = plugin.manifest.id.clone();
        // `packages.list` keeps this run's development loads after they stop.
        self.development.insert(
            manifest_id.clone(),
            (plugin.directory.clone(), plugin.manifest.clone()),
        );
        self.failures.remove(&manifest_id);
        self.plugins.insert(self.next_generation, plugin);
        self.backend.commands.send(Command {
            reply: Reply {
                owner: self.next_generation,
                id: 0,
            },
            method: "owner.registered".into(),
            params: json!({"manifestId":manifest_id}),
        })?;
        Ok(())
    }
    fn disable(&mut self, owner: u64) {
        // Process exit commonly follows an explicit disable. The first revoke
        // owns cleanup: a duplicate notification must not cancel its id=0
        // rebuild, clear another instance, or repeat backend/resource cleanup.
        if !self.plugins.contains_key(&owner) {
            return;
        }
        self.archive_activation(owner, "plugin disabled before activation completed");
        self.held_mounts.forget(owner);
        self.warnings.remove(&owner);
        let _ = self.commands.remove_owner(owner);
        {
            let mut input = self.input.borrow_mut();
            input.table.remove_owner(owner);
            if input.bottom == Some(owner) {
                input.bottom = None;
            }
            if input.sidebar == Some(owner) {
                input.sidebar = None;
            }
        }
        let tabs_before = self.policy.sidebar_panels();
        if self
            .sequence
            .is_some_and(|(provider, ..)| provider == owner)
        {
            self.sequence = None;
            self.rebroadcast_source();
        }
        if self.info_panels.remove(&owner).is_some() {
            self.show_info_panels();
        }
        // The floating window opens again without this panel (`sync_floating`).
        if self
            .floating
            .as_ref()
            .is_some_and(|f| f.owners.contains(&owner))
        {
            self.close_floating();
        }
        if matches!(&self.menu_mode, MenuMode::Panel(menu_owner, _) if *menu_owner == owner) {
            self.close_menu();
        }
        self.policy.remove_owner(owner);
        self.power.remove_owner(owner);
        self.timeline.remove_owner(owner);
        self.images.release_owner(owner);
        self.drops.remove_owner(owner);
        self.settings.remove_owner(owner);
        self.on_top.remove(owner);
        self.cursor_playing.remove(owner);
        self.pip.remove_owner(owner);
        let step = self.step_seconds();
        if self.step_seek.remove(&owner).is_some() {
            self.step_seek_changed(step);
        }
        self.mount_queue.retain(|c| c.reply.owner != owner);
        if self
            .preparing
            .as_ref()
            .is_some_and(|(_, c)| c.reply.owner == owner)
        {
            self.preparing = None;
        }
        if let Some(mount) = self.mounts.remove(&owner) {
            // Another sidebar tab stays shown until the shell is rebuilt.
            let last_tab = tabs_before.iter().all(|&o| o == owner);
            if let Some(shell) = &self.shell
                && let Some(property) = mount.spec.slot.visible_property()
                && (mount.spec.slot != composition::Slot::Sidebar || last_tab)
            {
                let _ = shell.set_property(property, false.into());
            }
            self.mount_queue.push_front(MountChange {
                reply: Reply { owner, id: 0 },
                contribution: None,
                announced: false,
            });
        }
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner, id: 0 },
            method: "owner.disabled".into(),
            params: Value::Null,
        });
        let keys: Vec<_> = self
            .views
            .keys()
            .filter(|(key, _)| *key == owner)
            .cloned()
            .collect();
        for key in keys {
            if let Some(view) = self.views.remove(&key) {
                let _ = view.instance.hide();
            }
        }
        self.cleanup.insert(owner, (false, false));
        // Waiting for graceful disposal/killing belongs off the Slint event loop.
        if let Some(plugin) = self.plugins.remove(&owner) {
            let events = self.events.clone();
            std::thread::spawn(move || {
                drop(plugin);
                let _ = events.send(Event::Reaped(owner));
            });
        }
    }
    fn send(&mut self, owner: u64, message: Value) {
        let failed = self
            .plugins
            .get_mut(&owner)
            .and_then(|plugin| plugin.send(message).err());
        if let Some(error) = failed {
            self.transport_failed(owner, &error);
        }
    }
    /// A message for `owner` could not be queued. An own process: its queue
    /// is full or its pipe closed, and it stops, as before. A member of the
    /// shared runner: its queue fills only while the runner reads none of
    /// its input (one reader for every member), so the runner is stuck;
    /// every member restarts with it (`restart_shared`), this one included,
    /// instead of this one stopping for good.
    fn transport_failed(&mut self, owner: u64, error: &anyhow::Error) {
        let Some(plugin) = self.plugins.get(&owner) else {
            return;
        };
        diag!("plugin {owner} ({}) transport: {error}", plugin.manifest.id);
        let runner = plugin.shared_runner();
        if runner.is_some() && self.shared.as_ref().map(|s| s.link.id) == runner {
            self.restart_shared(&format!(
                "{}'s queue is full: the runner reads no input",
                plugin.manifest.id
            ));
        } else {
            self.disable(owner);
        }
    }
    fn broadcast(&mut self, event: &str, data: Value) {
        if event == "host.error" {
            diag!("host.error {data}");
        }
        let mut failed = Vec::new();
        for (owner, plugin) in self
            .plugins
            .iter_mut()
            .filter(|(_, p)| p.subscriptions.contains(event))
        {
            if let Err(error) = plugin.send(json!({"event":event,"data":data})) {
                failed.push((*owner, error));
            }
        }
        for (owner, error) in failed {
            self.transport_failed(owner, &error);
        }
    }
    fn handle(&mut self, event: Event) {
        match event {
            Event::Request(owner, message) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                if let Some(event) = message["event"].as_str() {
                    diag!("runtime {owner} {event}: {}", message["data"]);
                    let now = self.now_ms();
                    let result = match event {
                        "runtime.ready" => self.activations.runtime_ready(owner, now),
                        "runtime.failed" => self.activations.runtime_failed(
                            owner,
                            message["data"].to_string().chars().take(512).collect(),
                            now,
                        ),
                        _ => Ok(()),
                    };
                    if let Err(error) = result {
                        diag!("activation runtime event: {error}");
                    }
                    self.activation_tick();
                    return;
                }
                match serde_json::from_value::<Request>(message) {
                    Ok(request) => {
                        let reply = Reply {
                            owner,
                            id: request.id,
                        };
                        if matches!(request.method.as_str(), "ui.open" | "ui.mount") {
                            let now = self.now_ms();
                            if matches!(
                                self.activations.status(owner, now),
                                Ok(package_activation::Status::Pending { .. })
                            ) {
                                if let Err(error) = self.activations.ui_begin(owner, reply.id, now)
                                {
                                    self.respond(
                                        reply,
                                        Err(RpcError::new("activation", error.to_string())),
                                    );
                                    return;
                                }
                                self.activation_ui.insert((owner, reply.id));
                            }
                        }
                        // Built-in plugins claim panel slots, the sequence and resolvers first (slot_claims).
                        if matches!(
                            request.method.as_str(),
                            "ui.mount"
                                | "ui.unmount"
                                | "sequence.set"
                                | "sourceResolvers.register"
                                | "sourceResolvers.unregister"
                        ) && !self.headless
                        {
                            let builtin = self.is_builtin(owner);
                            let starting = self.builtins_starting();
                            if self.held_mounts.holds(owner, builtin, starting) {
                                self.held_mounts
                                    .hold(owner, (reply, request.method, request.params));
                                return;
                            }
                        }
                        self.answer(reply, &request.method, &request.params);
                    }
                    Err(error) => {
                        diag!("malformed request: {error}");
                        self.disable(owner);
                    }
                }
            }
            #[cfg(feature = "producer-audio")]
            Event::PhysicalInput(generation, params) => {
                if generation == self.shell_generation {
                    self.physical_input(params);
                }
            }
            Event::WindowInput(generation, kind) => {
                if generation != self.shell_generation {
                    return;
                }
                let _ = kind;
                // Activity keeps the pointer and the popped-out sidebar; it
                // never reveals the fullscreen bar (FS-BAR-1).
                self.policy.activity(self.now_ms());
            }
            Event::MenuAction(generation, action) => {
                if !self.window_generation_live(generation)
                    || (!matches!(action, MenuAction::Reset) && generation != self.menu_generation)
                {
                    return;
                }
                self.policy.activity(self.now_ms());
                if debug_menu()
                    && let MenuAction::Invoke(token) = &action
                {
                    diag!("menu: invoke {token} open={:?}", self.menu_path);
                }
                match action {
                    MenuAction::Reset => {
                        #[cfg(feature = "producer-audio")]
                        self.physical_revoke(producer_input::RevokeReason::Menu);
                        self.open_menu(generation, MenuMode::Main);
                    }
                    MenuAction::Back => {
                        let mut path = self.menu_path.clone();
                        path.pop();
                        self.set_menu_path(path);
                    }
                    MenuAction::Dismiss => self.close_menu(),
                    MenuAction::Hover(level, token, now) => {
                        if debug_menu() {
                            diag!(
                                "menu: hover level={level} token={token} now={now} open={:?} t={}",
                                self.menu_path,
                                self.now_ms()
                            );
                        }
                        let Some(path) = menu::hover_target(&self.menu_path, level, &token) else {
                            return;
                        };
                        if now {
                            self.set_menu_path(path);
                        } else {
                            // The open submenu beside the hovered level.
                            let submenu = (self.menu_path.len() > level)
                                .then(|| self.menu_panels.get(level + 1).copied())
                                .flatten();
                            let pointer = self.menu_pointer();
                            let now = self.now_ms();
                            if let Some(path) =
                                self.menu_aim
                                    .hover(&self.menu_path, path, pointer, submenu, now)
                            {
                                self.set_menu_path(path);
                            }
                        }
                    }
                    MenuAction::Scroll(level, offset) => {
                        if level <= self.menu_path.len() && offset.is_finite() {
                            self.menu_scroll.resize(level + 1, 0.);
                            self.menu_scroll[level] = offset;
                            let result = self.refresh_menu();
                            self.report_menu(result);
                        }
                    }
                    MenuAction::Invoke(token) => match menu::parse_token(&token) {
                        Some(menu::Token::Group(path)) => self.set_menu_path(path),
                        Some(menu::Token::Command { owner, id }) => {
                            self.close_menu();
                            if let Err(error) = self.execute_command(owner, &id) {
                                diag!("menu execute: {:?}", error);
                            }
                        }
                        None => (),
                    },
                }
            }
            Event::ActionDone(reply, action, result) => {
                input_trace::mark(&format!("host loop: {action} done"));
                self.action_done(reply, &action, result)
            }
            Event::Reply(reply, mut result) => {
                if self.source_state_replies.remove(&(reply.owner, reply.id))
                    && let Ok(state) = &mut result
                {
                    self.patch_source(state);
                }
                self.respond(reply, result)
            }
            Event::Broadcast(event, mut data) => {
                if event == "source.state" {
                    self.patch_source(&mut data);
                    let previous = std::mem::replace(&mut self.last_source, data.clone());
                    if previous["phase"] != data["phase"] {
                        input_trace::mark(&format!(
                            "host loop: source.state phase={}",
                            data["phase"]
                        ));
                    }
                    if previous["paused"] != data["paused"] {
                        input_trace::mark(&format!(
                            "host loop: source.state paused={}",
                            data["paused"]
                        ));
                    }
                    self.timeline_source(&previous, &data);
                    self.backdrop_source();
                    self.update_title();
                    self.core_action_states();
                }
                if event == "resolvers.fileTypes" {
                    self.file_types = data["fileTypes"].clone();
                }
                self.broadcast(&event, data)
            }
            Event::Binding(generation, owner, id) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                input_trace::mark(&format!("host loop: binding → {owner}/{id}"));
                self.policy.activity(self.now_ms());
                match self.commands.resolve_execution(owner, &id) {
                    // A key never fails silently: the command's reason shows.
                    Err(commands::CommandError::Disabled {
                        reason: Some(reason),
                    }) => self.show_osd(&reason),
                    _ => {
                        if let Err(error) = self.execute_command(owner, &id) {
                            diag!("binding execute: {error:?}");
                        }
                    }
                }
            }
            Event::SidebarResized(generation, width) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.set_sidebar_width(width);
                self.ui_state.state.sidebar = Some(ui_state::Sidebar { width });
                self.ui_state.touch(self.now_ms());
            }
            Event::BarButton(generation, token) => self.bar_click(generation, &token),
            Event::PipReturn(generation) => {
                if generation == self.shell_generation {
                    self.pip.restore();
                }
            }
            Event::FloatingClosed(generation) => {
                if self
                    .floating
                    .as_ref()
                    .is_some_and(|f| f.generation == generation)
                {
                    self.close_sidebar();
                    self.close_floating();
                }
            }
            Event::SidebarTab(generation, index) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                // The index is in that window's tab order.
                let tabs = match &self.floating {
                    Some(floating) if floating.generation == generation => &floating.owners,
                    _ => &self.shell_tabs,
                };
                if let Some(&owner) = tabs.get(index)
                    && self.policy.set_active(owner).is_ok()
                {
                    self.tab_clicked = true;
                    self.remember_sidebar(true);
                    self.window_tick();
                }
            }
            Event::SidebarDetach(generation) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                if let (Some(owner), Some((_, detached))) =
                    (self.policy.sidebar_active(), self.policy.sidebar_state())
                    && self.policy.set_detached(owner, !detached).is_ok()
                {
                    self.remember_sidebar(false);
                    self.window_tick();
                }
            }
            Event::SidebarClose(generation) => {
                if self.window_generation_live(generation) {
                    self.close_sidebar();
                    self.window_tick();
                }
            }
            Event::PluginMessage(owner, event, data) => {
                self.send(owner, json!({"event":event,"data":data}))
            }
            Event::ViewClosed(owner, id) => {
                if let Some(view) = self.views.remove(&(owner, id.clone())) {
                    let _ = view.instance.hide();
                    self.images.release_view(owner, &format!("w:{id}"));
                    self.send(owner, json!({"event":"ui.closed","data":{"view":id}}));
                }
            }
            Event::UiCallback(owner, data) => {
                self.send(owner, json!({"event":"ui.callback","data":data}))
            }
            Event::InjectKey(generation, key, kind, physical) => {
                if debug_keys() {
                    diag!("keys: inject to slint {key:?} {kind:?} window={generation}");
                }
                if let Some(window) = self.window_for(generation) {
                    let window = window.clone_strong();
                    if kind != Injected::Released {
                        self.input.borrow_mut().physical = physical;
                    }
                    let text = key.into();
                    window.window().dispatch_event(match kind {
                        Injected::Pressed => slint::platform::WindowEvent::KeyPressed { text },
                        Injected::Repeated => {
                            slint::platform::WindowEvent::KeyPressRepeated { text }
                        }
                        Injected::Released => slint::platform::WindowEvent::KeyReleased { text },
                    });
                    // The press was resolved (or not) by now.
                    self.input.borrow_mut().physical = None;
                }
            }
            Event::ModifiersReleased(generation, keys) => {
                if debug_keys() {
                    diag!(
                        "keys: release to slint {keys:?} window={generation} live={} shell={}",
                        self.window_for(generation).is_some(),
                        self.shell_generation
                    );
                }
                if let Some(window) = self.window_for(generation) {
                    for key in keys {
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                                text: key.into(),
                            });
                    }
                }
            }
            Event::Dropped(generation, path) => self.drops.arrived(generation, path),
            Event::OpenFiles(files, front) => self.open_files(files, front),
            Event::Settings(generation, index) => self.settings_event(generation, index),
            Event::SettingsChecked(serial, result) => self.settings_checked(serial, result),
            Event::ConfirmDone(reply, confirmed, checked) => {
                self.policy.activity(self.now_ms());
                self.dialog_busy = false;
                self.respond(reply, Ok(json!({"confirmed":confirmed,"checked":checked})));
            }
            Event::DialogDone(reply, paths) => {
                self.policy.activity(self.now_ms());
                self.dialog_busy = false;
                self.respond(
                    reply,
                    Ok(json!({"cancelled":paths.is_none(),"paths":paths.unwrap_or_default()})),
                );
            }
            Event::Exited(owner) => {
                // Still running: the process ended without being stopped.
                self.record_failure(owner, "plugin process exited");
                self.disable(owner)
            }
            Event::Runner(runner, message) => self.runner_event(runner, &message),
            Event::RunnerExited(runner) => {
                if self.shared.as_ref().is_some_and(|s| s.link.id == runner) {
                    self.restart_shared("the shared runner exited");
                }
            }
            Event::Reaped(owner) => {
                if let Some(cleanup) = self.cleanup.get_mut(&owner) {
                    cleanup.0 = true;
                }
                if let Some(directory) = self.data_removals.remove(&owner) {
                    let _ = std::fs::remove_dir_all(directory);
                }
            }
            Event::BackendCleaned(owner) => {
                if let Some(cleanup) = self.cleanup.get_mut(&owner) {
                    cleanup.1 = true;
                }
            }
            Event::RuntimeFault(owner, error) => {
                diag!("plugin {owner}: {error}");
                let now = self.now_ms();
                let _ =
                    self.activations
                        .runtime_failed(owner, error.chars().take(512).collect(), now);
                self.record_failure(owner, &error);
                self.disable(owner);
            }
        }
    }
    /// Why a plugin stopped on its own (`packages.list` state `failed`).
    fn record_failure(&mut self, owner: u64, message: &str) {
        if let Some(plugin) = self.plugins.get(&owner) {
            self.failures.insert(
                plugin.manifest.id.clone(),
                message.chars().take(512).collect(),
            );
        }
    }
    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    /// GPU rendering: show the renderer's texture in this window (before
    /// `show()`, so the rendering notifier sees the setup). A window that
    /// cannot show it switches the player to software rendering. Either way
    /// the window's draws are counted (`stats().presentedFps`). Returns the
    /// window's reused video texture, if any, for `video_texture` once the
    /// window is shown.
    #[must_use]
    fn attach_gpu(
        &self,
        shell: &slint_interpreter::ComponentInstance,
    ) -> Option<vestra_video_texture::Feed> {
        let Some(gpu) = self.backend.render.gpu() else {
            return self.count_draws(shell);
        };
        let render = self.backend.render.clone();
        let drawn = move |draw: platform::GpuDraw| match draw {
            platform::GpuDraw::Begin => render.presentation.draw_begin(Instant::now()),
            platform::GpuDraw::End { copied } => {
                render.presentation.draw_end(Instant::now(), copied);
            }
        };
        self.backend.render.presentation.start_counting();
        let render = self.backend.render.clone();
        let show = move |shell: &slint_interpreter::ComponentInstance, image: slint::Image| {
            // A generated stream's picture stays (its frames come as pixels).
            if !render.producer() {
                let _ = shell.set_property("video-frame", image.into());
            }
        };
        let render = self.backend.render.clone();
        let failed = move |reason: String| render.fall_back(&reason);
        if let Err(error) = gpu.attach(shell, show, failed, drawn) {
            self.backend
                .render
                .fall_back(&format!("the window cannot show GPU frames ({error})"));
        }
        None
    }
    /// Software rendering: count the window's draws with its rendering
    /// notifier (femtovg); Slint's software renderer has none, so there each
    /// winit redraw counts (without draw times). Where enabled
    /// (`platform::VIDEO_TEXTURE`), the notifier also shows the pictures
    /// through one reused texture (`show_frame`), returned for `video_texture`.
    fn count_draws(
        &self,
        shell: &slint_interpreter::ComponentInstance,
    ) -> Option<vestra_video_texture::Feed> {
        if platform::VIDEO_TEXTURE {
            let render = self.backend.render.clone();
            let drawn = move |draw| match draw {
                vestra_video_texture::Draw::Begin => {
                    let now = Instant::now();
                    frame_pacing::draw_began();
                    render.presentation.draw_begin(now);
                }
                vestra_video_texture::Draw::End => {
                    render.presentation.draw_end(Instant::now(), false);
                }
            };
            let show = |shell: &slint_interpreter::ComponentInstance, image: slint::Image| {
                let _ = shell.set_property("video-frame", image.into());
            };
            if let Ok(feed) = vestra_video_texture::attach(shell, show, drawn) {
                self.backend.render.presentation.start_counting();
                return Some(feed);
            }
        }
        let render = self.backend.render.clone();
        let notifier = move |state, _: &slint::GraphicsAPI| match state {
            slint::RenderingState::BeforeRendering => {
                let now = Instant::now();
                frame_pacing::draw_began();
                render.presentation.draw_begin(now);
            }
            slint::RenderingState::AfterRendering => {
                render.presentation.draw_end(Instant::now(), false);
            }
            _ => (),
        };
        if shell.window().set_rendering_notifier(notifier).is_err() {
            use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
            let render = self.backend.render.clone();
            shell.window().on_winit_window_event(move |_, event| {
                if matches!(event, WindowEvent::RedrawRequested) {
                    render.presentation.draw_end(Instant::now(), false);
                }
                EventResult::Propagate
            });
        }
        self.backend.render.presentation.start_counting();
        None
    }
    /// Set the newest rendered frame on the video area (no copy: the renderer
    /// drew into this buffer).
    fn show_frame(&mut self) {
        // Paced (Windows): one new picture per draw; a second one within the
        // same refresh comes again right after the draw.
        if !frame_pacing::accept_picture() {
            self.backend.frame.miss();
            return;
        }
        let Some(frame) = self.backend.frame.take() else {
            return;
        };
        let Some(window) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        self.shown_size = Some((frame.buffer.width(), frame.buffer.height()));
        let (width, height) = (frame.buffer.width(), frame.buffer.height());
        if self.pip.feeding() && self.pip_shape((width, height)) {
            self.pip
                .push(width as usize, height as usize, frame.buffer.as_bytes());
        }
        let set = Instant::now();
        // Through the reused texture when the window has one (uploaded at
        // the next draw); otherwise as a new image (femtovg then makes a new
        // texture for it, Slint's software renderer composes it).
        let texture = self
            .video_texture
            .borrow()
            .as_ref()
            .and_then(|feed| feed.put(frame.buffer.clone()));
        let image = slint::Image::from_rgb8(frame.buffer);
        self.last_image = Some(image.clone());
        self.last_frame = Some(set);
        match texture {
            Some(texture) => {
                if let Some(texture) = texture {
                    let _ = window.set_property("video-frame", texture.into());
                }
                window.window().request_redraw();
            }
            None => {
                let _ = window.set_property("video-frame", image.into());
            }
        }
        self.backend.render.presentation.picture_set(set.elapsed());
        frame_pacing::picture_set();
        // With GPU rendering these are a generated stream's frames (or its
        // cleared canvas): the next GPU frame sets its texture again.
        if let Some(gpu) = self.backend.render.gpu() {
            gpu.reshow();
        }
    }
    /// What `window.setPictureInPicture` and the PiP window depend on.
    fn pip_facts(&self) -> pip::Facts {
        pip::Facts {
            window: !self.headless && self.native_window.is_some(),
            supported: !self.headless && platform::pip_supported(),
            fullscreen: self
                .shell
                .as_ref()
                .is_some_and(|s| s.window().is_fullscreen()),
            video: !self.video_absent,
        }
    }
    /// `window.setPictureInPicture { on }` (R-W5): answered once the system
    /// window has opened or closed. From fullscreen the player leaves
    /// fullscreen first.
    fn set_picture_in_picture(
        &mut self,
        reply: &Reply,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let on = params["on"]
            .as_bool()
            .ok_or_else(|| RpcError::invalid("on requires a boolean"))?;
        let facts = self.pip_facts();
        let now = self.now_ms();
        let answer = self
            .pip
            .request(reply.clone(), on, &facts, now, platform::PipWindow::new)?;
        if on
            && answer.is_none()
            && facts.fullscreen
            && let Some(shell) = &self.shell
        {
            shell.window().set_fullscreen(false);
        }
        Ok(answer)
    }
    /// Each UI tick (R-H8): keep the PiP source layer on the video area,
    /// report playback to the PiP window, run what its buttons ask, and
    /// show the main window's "playing in picture-in-picture" note.
    fn pip_tick(&mut self) {
        // Switch the renderers to the picture's shape before any frame goes
        // to PiP (frames of the old shape are not pushed).
        self.apply_render_size();
        self.pip_gpu_hook();
        if self.pip.needs_frame() {
            if self.pip_gpu_frames() {
                // Paused playback renders nothing new: start from the newest.
                self.pip_gpu_frame();
            } else if let Some(still) = self.last_image.as_ref().and_then(slint::Image::to_rgb8)
                && self.pip_shape((still.width(), still.height()))
            {
                let (width, height) = (still.width() as usize, still.height() as usize);
                self.pip.push(width, height, still.as_bytes());
            }
        }
        let place = self
            .shell
            .as_ref()
            .zip(self.native_window)
            .map(|(shell, id)| {
                ui::settle_layout(shell.window());
                let number = |name: &str| match shell.get_property(name) {
                    Ok(SValue::Number(n)) => n,
                    _ => 0.,
                };
                let area = platform::Rect {
                    x: number("video-x"),
                    y: number("video-y"),
                    width: number("video-width"),
                    height: number("video-height"),
                };
                (id, area)
            });
        let source = &self.last_source;
        let seconds = |key: &str| source[key].as_f64().filter(|n| n.is_finite());
        let playback = pip::Playback {
            paused: source["paused"] == true,
            position: seconds("position"),
            duration: seconds("duration").filter(|_| source["capabilities"]["seek"] == true),
            rate: seconds("rate").unwrap_or(1.),
        };
        let facts = self.pip_facts();
        let now = self.now_ms();
        if std::env::var_os("VESTRA_PIP_LOG").is_some()
            && let Some((id, area)) = place
            && let Some(line) = self.pip.describe(id)
        {
            let scale = self
                .shell
                .as_ref()
                .map_or(0., |s| s.window().scale_factor());
            let line = format!(
                "pip: video area ({:.1},{:.1} {:.1}x{:.1}) pt, slint scale {scale} | render area {:?} sent {:?} picture {:?} | {line}",
                area.x,
                area.y,
                area.width,
                area.height,
                self.render_area,
                self.render_sent,
                self.backend
                    .target
                    .picture()
                    .map(|(w, h)| (w.round(), h.round())),
            );
            if self.pip_log.as_deref() != Some(line.as_str()) {
                diag!("{line}");
                self.pip_log = Some(line);
            }
        }
        for effect in self.pip.tick(&facts, place, &playback, now) {
            match effect {
                pip::Effect::Respond(reply, result) => self.respond(reply, result),
                pip::Effect::Action(action, step) => {
                    if let Err(error) = self.run_core_action_with(action, None, step) {
                        diag!("pip: {action}: {}", error.message);
                    }
                }
            }
        }
        if let Some(shell) = &self.shell {
            let active = self.pip.active();
            if shell.get_property("picture-in-picture").ok() != Some(active.into()) {
                let _ = shell.set_property("picture-in-picture", active.into());
            }
        }
    }
    /// GPU rendering: once, have the render thread's frame callback wake the
    /// UI thread for each new frame while PiP wants frames (coalesced; a
    /// wake-up that finds the host busy is dropped, the next frame comes).
    fn pip_gpu_hook(&mut self) {
        let (wanted, hooked) = &mut self.pip_gpu;
        wanted.store(self.pip.feeding(), Ordering::Release);
        let Some(gpu) = self.backend.render.gpu().filter(|_| !*hooked) else {
            return;
        };
        *hooked = true;
        let wanted = wanted.clone();
        let queued = Arc::new(AtomicBool::new(false));
        gpu.on_frame(move || {
            if !wanted.load(Ordering::Acquire) || queued.swap(true, Ordering::AcqRel) {
                return;
            }
            let queued = queued.clone();
            let _ = slint::invoke_from_event_loop(move || {
                queued.store(false, Ordering::Release);
                PIP_GPU_SINK.with(|sink| {
                    if let Some(deliver) = &*sink.borrow() {
                        deliver();
                    }
                });
            });
        });
    }
    /// PiP shows the GPU renderer's frames: GPU rendering, and no generated
    /// stream (whose pixel frames go through `show_frame` like software ones).
    fn pip_gpu_frames(&self) -> bool {
        self.backend.render.gpu().is_some() && !self.backend.render.producer()
    }
    /// The GPU renderer's newest frame to PiP, without a copy (each frame
    /// once). The renderer does not draw into a surface the system still
    /// shows, so the PiP window never sees a frame being redrawn.
    fn pip_gpu_frame(&mut self) {
        if !self.pip.feeding() || !self.pip_gpu_frames() {
            return;
        }
        let Some(gpu) = self.backend.render.gpu() else {
            return;
        };
        if let Some((sequence, surface)) = gpu.latest()
            && gpu.size().is_some_and(|size| self.pip_shape(size))
        {
            self.pip.push_surface(sequence, &surface);
        }
    }
    /// The size both renderers draw at: the video area, or while PiP wants
    /// mpv's frames the picture's own shape inside it (no bars, R-H8; the
    /// PiP window takes the frame's shape). Sent when it changes.
    fn apply_render_size(&mut self) {
        let Some(area) = self.render_area else {
            return;
        };
        let picture = self.backend.target.picture();
        let size = match picture {
            Some(picture) if self.pip.feeding() && !self.backend.render.producer() => {
                render_target::fit(area, picture)
            }
            _ => area,
        };
        if self.render_sent != Some(size) {
            self.render_sent = Some(size);
            self.backend.target.set_area(size);
            if let Some(gpu) = self.backend.render.gpu() {
                gpu.set_size(size);
            }
        }
    }
    /// A frame of `size` pixels may go to PiP: it has the shape the renderer
    /// was last asked for (a frame from before the switch to the picture's
    /// shape would size the PiP window wrongly). Generated streams' frames
    /// always may.
    fn pip_shape(&self, size: (u32, u32)) -> bool {
        self.backend.render.producer()
            || self
                .render_sent
                .is_none_or(|sent| render_target::same_shape(size, sent))
    }
    /// R-H11: a pending left press on the video becomes `Mouse:LeftClick`
    /// on release when nothing makes it wait (`video_click`), else once the
    /// double-click interval passes without a second press. It runs here,
    /// not through the event queue, so it costs no further loop turn.
    fn click_tick(&mut self, pointer: (f64, f64)) {
        let Some(click) = self.input.borrow().click else {
            return;
        };
        let input = bindings::Input {
            mods: click.mods,
            name: "Mouse:LeftClick".into(),
        };
        let immediate = video_click::immediate(&self.input.borrow().table, &self.commands, &input);
        let verdict = click.verdict(Instant::now(), pointer, double_click_interval(), immediate);
        if verdict == video_click::Verdict::Wait {
            return;
        }
        self.input.borrow_mut().click = None;
        let target = {
            let shared = self.input.borrow();
            (!shared.blocked)
                .then(|| shared.table.resolve(&input, None).cloned())
                .flatten()
        };
        if verdict == video_click::Verdict::Fire
            && click.generation == self.shell_generation
            && let Some(target) = target
        {
            // A click also focuses the video and starts a window drag, so a
            // click whose command cannot run now (play/pause with nothing
            // open) does nothing, without the reason a key would show.
            if matches!(
                self.commands.resolve_execution(target.owner, &target.id),
                Err(commands::CommandError::Disabled { .. })
            ) {
                input_trace::mark(&format!(
                    "click dropped: {}/{} is disabled",
                    target.owner, target.id
                ));
                return;
            }
            input_trace::mark(&format!(
                "click fired ({}) → {}/{}",
                if immediate {
                    "on release"
                } else {
                    "after the double-click interval"
                },
                target.owner,
                target.id
            ));
            self.handle(Event::Binding(click.generation, target.owner, target.id));
        }
    }
    fn window_tick(&mut self) {
        self.pip_tick();
        self.menu_tick();
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        let now = self.now_ms();
        let fullscreen = shell.window().is_fullscreen();
        if self.native_window.is_none()
            && let Some(before) = &self.native_before
        {
            match platform::associate_new(before) {
                Ok(id) => {
                    self.native_window = Some(id);
                    self.native_before = None;
                    self.native_unavailable = None;
                }
                Err(error) => self.native_unavailable = Some(error.to_string()),
            }
        }
        let native = self
            .native_window
            .and_then(|id| platform::screen_for(id).ok());
        if native.is_some_and(|facts| !facts.active)
            && self.menu_open
            && self.menu_generation == self.shell_generation
        {
            self.close_menu();
            self.policy.activity(now);
        }
        self.policy.set_fullscreen(fullscreen, now);
        self.policy
            .set_obstructions(self.menu_open, self.dialog_busy);
        #[cfg(feature = "producer-audio")]
        let captured = self
            .backend
            .input
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|routing| routing.captured);
        #[cfg(not(feature = "producer-audio"))]
        let captured = false;
        self.input.borrow_mut().blocked = self.menu_open || self.dialog_busy || captured;
        if let Err(error) =
            shell.set_property("input-blocked", (self.menu_open || self.dialog_busy).into())
        {
            diag!("host.error physical-input bridge: {error}");
        }
        if self.osd_until.is_some_and(|until| now >= until) {
            self.osd_until = None;
            let _ = shell.set_property("osd-visible", false.into());
        }
        self.host_command_states(fullscreen);
        // The sidebar and plugin panels show and hide `if` elements.
        ui::settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name) {
            Ok(slint_interpreter::Value::Number(n)) => n,
            _ => 0.,
        };
        let viewport = window_state::Viewport {
            width: number("logical-width"),
            height: number("logical-height"),
        };
        let video = (number("video-width"), number("video-height"));
        self.backdrop_cover_art();
        let scale = f64::from(shell.window().scale_factor());
        if let Some(area) = self
            .sizing
            .observe(video, scale, fullscreen, Instant::now())
        {
            self.render_area = Some(area);
        }
        self.apply_render_size();
        let presentation = &self.backend.render.presentation;
        presentation.sample(Instant::now());
        if let Some(log) = self.frame_log.as_mut() {
            let ((mut produced, mut shown), mut size) =
                (self.backend.frame.counts(), self.shown_size);
            if let Some(gpu) = self.backend.render.gpu() {
                (produced, shown) = (produced + gpu.frames(), shown + gpu.shown());
                size = gpu.size().or(size);
            }
            let (draws, presented) = presentation.counts();
            let counts = render_target::LogCounts {
                produced,
                shown,
                draws,
                presented,
            };
            let refresh = || platform::window_refresh(shell.window());
            if let Some(line) = log.line(counts, presentation, refresh, size, Instant::now()) {
                diag!(
                    "{line}{}{}",
                    self.backend.frame.stages.text(),
                    frame_pacing::log_text()
                );
            }
        }
        self.apply_on_top();
        self.click_tick((number("pointer-x"), number("pointer-y")));
        if self.measured_shell != Some(self.shell_generation)
            && (viewport.width <= 0. || viewport.height <= 0.)
        {
            return;
        }
        // IA-CORE-03: only the normal size is remembered (not fullscreen, not
        // maximized), and the player always starts at it (`RestoreLastState=0`).
        if !fullscreen && !shell.window().is_maximized() && !self.headless {
            self.remember_window(&shell);
        }
        // G-04 / G-01: where the pointer is. A hover counts only while its
        // panel is showing (Slint keeps `has-hover` until the pointer moves).
        let flag = |name: &str| matches!(shell.get_property(name), Ok(SValue::Bool(true)));
        let bottom_hover = flag("bottom-hover") && flag("bottom-visible");
        self.policy
            .set_hover(window_state::Slot::Bottom, bottom_hover);
        self.policy.set_hover(
            window_state::Slot::Sidebar,
            flag("sidebar-hover") && self.sidebar_popped,
        );
        // FS-BAR-1: the fullscreen bar shows only while the pointer is in
        // its band (the bar's laid-out height, also while it is hidden).
        let over_video = flag("video-hover").then(|| (number("pointer-x"), number("pointer-y")));
        self.policy.bottom_band(
            bottom_hover,
            over_video,
            viewport,
            number("bottom-band-height"),
            now,
        );
        if let Some((x, _)) = over_video {
            self.policy.pointer(x, viewport.width, now);
        } else if bottom_hover {
            // The bar lies left of the popped-out sidebar.
            self.policy.pointer(0., viewport.width, now);
        }
        self.policy.tick(now);
        self.sync_tabs(&shell);
        let mut sidebar_seen = None;
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            sidebar_seen = snapshot.sidebar;
            shell.set_property("host-fullscreen", fullscreen.into())?;
            shell.set_property(
                "menu-open",
                (self.menu_open && self.menu_generation == self.shell_generation).into(),
            )?;
            let bottom = snapshot.bottom.is_some_and(|s| s.effective);
            shell.set_property("bottom-visible", bottom.into())?;
            // Slint 1.18 drops the focus of an element that becomes invisible
            // (#11079), and then no key reaches the bindings: hiding the bar
            // gives the focus back to the video, as collapsing the sidebar does.
            if self.bottom_shown && !bottom && !flag("canvas-focused") {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.bottom_shown = bottom;
            let sidebar = snapshot.sidebar.filter(|s| !s.detached);
            shell.set_property(
                "sidebar-visible",
                sidebar.is_some_and(|s| s.effective && !s.overlay).into(),
            )?;
            // G-01: the fullscreen overlay exists while attached in
            // fullscreen, and slides out while effective.
            shell.set_property("sidebar-overlay", sidebar.is_some_and(|s| s.overlay).into())?;
            let popped = sidebar.is_some_and(|s| s.overlay && s.effective);
            shell.set_property("sidebar-popped", popped.into())?;
            if self.sidebar_popped && !popped {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.sidebar_popped = popped;
            let playing =
                self.last_source["phase"] == "ready" && self.last_source["paused"] == false;
            // UI-SHELL-09: in fullscreen the pointer hides while playing and
            // idle (3 s without motion or keys, not over the bar or in its
            // band, no menu, dialog or bar lock); FS-BAR-1 decoupled it from
            // the bar. `setCursorAutoHide("playing")`: in a window too, while
            // it has focus.
            let idle = self.policy.idle(now);
            let windowed =
                self.cursor_playing.get() && idle && native.is_some_and(|facts| facts.active);
            shell.set_property(
                "cursor-hidden",
                (playing && ((fullscreen && idle) || windowed)).into(),
            )?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"videoArea":{"width":video.0,"height":video.1},"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot,"backdrop":self.backdrop_state(),"pictureInPicture":self.pip_state()}),
            )
        })();
        self.answer_video_area(video, now);
        self.bar_tick(&shell, sidebar_seen);
        match result {
            Ok(facts) => {
                let detached = facts["slots"]["sidebar"]["effective"] == true
                    && facts["slots"]["sidebar"]["detached"] == true;
                self.sync_floating(detached);
                if facts != self.window_facts {
                    self.measured_shell = Some(self.shell_generation);
                    self.window_facts = facts.clone();
                    self.broadcast("window.state", facts);
                }
            }
            Err(error) => self.broadcast(
                "host.error",
                json!({"code":"window-policy","message":error.to_string()}),
            ),
        }
        self.ui_state.save_if_due(now);
    }
    /// Record the main window frame (logical px) for the next launch.
    fn remember_window(&mut self, shell: &slint_interpreter::ComponentInstance) {
        let window = shell.window();
        let scale = window.scale_factor() as f64;
        let (size, position) = (window.size(), window.position());
        if size.width == 0 || size.height == 0 || scale <= 0. {
            return;
        }
        let frame = ui_state::Frame {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        };
        if self.ui_state.state.window != Some(frame) {
            self.ui_state.state.window = Some(frame);
            self.ui_state.touch(self.now_ms());
        }
    }
    /// The slot's remembered state (defaults before anything was saved).
    fn sidebar_slot(&self) -> ui_state::SidebarSlot {
        self.ui_state.state.sidebar_slot.clone().unwrap_or_default()
    }
    /// Keep the sidebar slot's persisted state current: shown, floating, the
    /// current tab and the tab order.
    /// `chosen`: the current tab was just chosen (a click, `layout.set`
    /// `active`), so it is remembered even over one not mounted yet.
    fn remember_sidebar(&mut self, chosen: bool) {
        let Some((visible, detached)) = self.policy.sidebar_state() else {
            return;
        };
        let mut slot = self.sidebar_slot();
        slot.visible = visible;
        slot.detached = detached;
        // A remembered tab whose plugin has not mounted (yet: plugins start
        // one after another) stays remembered, so it is current again once
        // it does.
        let manifests = self
            .policy
            .sidebar_panels()
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.manifest.clone()))
            .collect::<Vec<_>>();
        let waiting = slot
            .active
            .as_ref()
            .is_some_and(|active| !manifests.contains(active));
        if chosen || !waiting {
            slot.active = self
                .policy
                .sidebar_active()
                .and_then(|owner| self.mounts.get(&owner))
                .map(|m| m.manifest.clone());
        }
        for owner in self.policy.sidebar_panels() {
            if let Some(mount) = self.mounts.get(&owner) {
                slot.remember(&mount.manifest);
            }
        }
        if self.ui_state.state.sidebar_slot.as_ref() != Some(&slot) {
            self.ui_state.state.sidebar_slot = Some(slot);
            self.ui_state.touch(self.now_ms());
        }
    }
    /// Hide the whole sidebar (its close button, the floating window closed).
    fn close_sidebar(&mut self) {
        if let Some(owner) = self.policy.sidebar_active() {
            let now = self.now_ms();
            let _ = self
                .policy
                .set_preferred(window_state::Slot::Sidebar, owner, false, now);
            self.remember_sidebar(false);
        }
    }
    /// The sidebar's panels in tab order: the playlist's, other built-in plugins', then as
    /// remembered (`SidebarSlot::order`), new ones last in mount order.
    fn tab_order(&self, mounts: &BTreeMap<u64, Mount>) -> Vec<u64> {
        let slot = self.sidebar_slot();
        let mut tabs = mounts
            .values()
            .filter(|m| m.spec.slot == composition::Slot::Sidebar)
            .map(|m| {
                // The playlist is always first (also as a development load).
                let rank = if m.manifest == "vestra.playlist" {
                    0
                } else if m.builtin {
                    1
                } else {
                    2
                };
                (
                    (rank, slot.position(&m.manifest), m.spec.owner),
                    m.spec.owner,
                )
            })
            .collect::<Vec<_>>();
        tabs.sort();
        tabs.into_iter().map(|(_, owner)| owner).collect()
    }
    /// The current tab everywhere it matters: the panel keys' owner, the
    /// shell's and the floating window's `sidebar-active` (and the floating
    /// window's title). A tab that changed without a click on the strip
    /// (a plugin, the current panel removed) hands the keys back to the
    /// video if they were in the sidebar: the panel that had them is gone.
    fn sync_tabs(&mut self, shell: &slint_interpreter::ComponentInstance) {
        let tabs = self.policy.sidebar_panels();
        let active = self.policy.sidebar_active();
        self.input.borrow_mut().sidebar = active;
        // Each window indexes its own tab order; a tab it does not have yet
        // (or any more) leaves it as it is until it is rebuilt.
        let set_index = |window: &slint_interpreter::ComponentInstance, order: &[u64]| {
            let Some(index) = active.and_then(|owner| order.iter().position(|&o| o == owner))
            else {
                return;
            };
            let index = index as f64;
            if !matches!(window.get_property("sidebar-active"), Ok(SValue::Number(n)) if n == index)
            {
                let _ = window.set_property("sidebar-active", SValue::Number(index));
            }
        };
        set_index(shell, &self.shell_tabs);
        if let Some(floating) = &self.floating {
            set_index(&floating.instance, &floating.owners);
            if let Some(mount) = active.and_then(|owner| self.mounts.get(&owner)) {
                let title = SValue::String(mount.title.as_str().into());
                if floating.instance.get_property("panel-title").ok().as_ref() != Some(&title) {
                    let _ = floating.instance.set_property("panel-title", title);
                }
            }
        }
        if self.sidebar_active != active {
            let canvas = matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)));
            if self.sidebar_active.is_some() && !self.tab_clicked && !canvas && tabs.len() > 1 {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.sidebar_active = active;
        }
        self.tab_clicked = false;
    }
    /// The titles of the sidebar's tabs, in tab order.
    fn tab_titles(&self, owners: &[u64]) -> Result<SValue> {
        let titles = owners
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.title.clone()))
            .collect::<Vec<_>>();
        ui_values::from_json(&json!(titles))
    }
    /// The host's tab strip callbacks of a shell or floating window.
    fn bind_tabs(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let events = self.events.clone();
        window.set_callback("sidebar-tab", move |args| {
            if let Some(SValue::Number(index)) = args.first()
                && *index >= 0.
            {
                let _ = events.send(Event::SidebarTab(generation, *index as usize));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-detach", move |_| {
            let _ = events.send(Event::SidebarDetach(generation));
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-close", move |_| {
            let _ = events.send(Event::SidebarClose(generation));
            SValue::Void
        })?;
        Ok(())
    }
    /// Open or close the floating window so it matches the policy; a window
    /// with other panels than the sidebar's now opens again.
    fn sync_floating(&mut self, want: bool) {
        let owners = self.policy.sidebar_panels();
        match (&self.floating, want) {
            (Some(floating), true) if floating.owners == owners => {
                self.remember_floating();
            }
            (None, true) if !owners.is_empty() => {
                if let Err(error) = self.open_floating(&owners) {
                    diag!("floating panel: {error:#}");
                    if let Some(&owner) = owners.first() {
                        for owner in &owners {
                            self.send(*owner, json!({"event":"ui.error","data":{"code":"ui-prepare","message":format!("ui-prepare: {error:#}")}}));
                        }
                        let _ = self.policy.set_detached(owner, false);
                        self.remember_sidebar(false);
                    }
                }
            }
            (Some(_), _) => self.close_floating(),
            _ => {}
        }
    }
    fn open_floating(&mut self, owners: &[u64]) -> Result<()> {
        let mounts = owners
            .iter()
            .map(|owner| {
                self.mounts
                    .get(owner)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no sidebar mount"))
            })
            .collect::<Result<Vec<_>>>()?;
        let specs = mounts.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
        let active = self.policy.sidebar_active();
        let title = mounts
            .iter()
            .find(|m| Some(m.spec.owner) == active)
            .unwrap_or(&mounts[0])
            .title
            .clone();
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        let instance =
            composition::compile_floating(&specs, &title, self.mount_events.clone(), generation)?;
        for mount in &mounts {
            for (name, value) in self.mount_values(mount) {
                composition::set_global_value(&instance, &mount.spec, &name, value)?;
            }
            composition::set_tabbed(&instance, &mount.spec, mounts.len() > 1)?;
        }
        instance.set_property("sidebar-titles", self.tab_titles(owners)?)?;
        self.bind_menu(&instance, generation, false)?;
        self.observe_keys(&instance, generation);
        self.bind_keys(&instance, generation)?;
        self.bind_tabs(&instance, generation)?;
        let events = self.events.clone();
        instance.window().on_close_requested(move || {
            let _ = events.send(Event::FloatingClosed(generation));
            slint::CloseRequestResponse::KeepWindowShown
        });
        let frame = self.sidebar_slot().floating.unwrap_or(ui_state::Frame {
            x: 438.,
            y: 100.,
            width: 300.,
            height: 408.,
        });
        instance.window().set_size(slint::LogicalSize::new(
            frame.width.max(300.) as f32,
            frame.height.max(250.) as f32,
        ));
        self.theme_view(&instance);
        let before = platform::capture_windows().ok();
        self.placements.show_at(
            &instance,
            slint::LogicalPosition::new(frame.x as f32, frame.y as f32),
        )?;
        platform::shown(&instance);
        let native = before.and_then(|before| platform::associate_new(&before).ok());
        self.floating = Some(Floating {
            instance,
            owners: owners.to_vec(),
            generation,
            native,
        });
        Ok(())
    }
    fn remember_floating(&mut self) {
        let Some(floating) = &self.floating else {
            return;
        };
        let window = floating.instance.window();
        let scale = window.scale_factor() as f64;
        let (size, position) = (window.size(), window.position());
        if size.width == 0 || scale <= 0. {
            return;
        }
        let frame = ui_state::Frame {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        };
        let mut slot = self.sidebar_slot();
        if slot.floating != Some(frame) {
            slot.floating = Some(frame);
            self.ui_state.state.sidebar_slot = Some(slot);
            self.ui_state.touch(self.now_ms());
        }
    }
    fn close_floating(&mut self) {
        if let Some(floating) = self.floating.take() {
            if self.menu_open && self.menu_generation == floating.generation {
                self.menu_open = false;
                self.menu_path.clear();
                self.menu_scroll.clear();
                self.menu_aim.clear();
                self.menu_mode = MenuMode::Main;
            }
            let _ = floating.instance.hide();
        }
    }
    /// Shell or floating window generation still alive.
    fn window_generation_live(&self, generation: u64) -> bool {
        generation == self.shell_generation
            || self
                .floating
                .as_ref()
                .is_some_and(|f| f.generation == generation)
    }
    fn window_for(&self, generation: u64) -> Option<&slint_interpreter::ComponentInstance> {
        if generation == self.shell_generation {
            self.shell.as_ref()
        } else {
            self.floating
                .as_ref()
                .filter(|f| f.generation == generation)
                .map(|f| &f.instance)
        }
    }
    fn report_menu(&mut self, result: Result<()>) {
        if let Err(error) = result {
            self.broadcast(
                "host.error",
                json!({"code":"menu-bridge","message":error.to_string()}),
            );
        }
    }
    fn bind_shell(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        {
            use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
            let events = self.events.clone();
            #[cfg(feature = "producer-audio")]
            let routing = self.backend.input.clone();
            let input = self.input.clone();
            let weak = window.as_weak();
            let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
            let menu_cursor = std::cell::Cell::new((f64::NAN, f64::NAN));
            window.window().on_winit_window_event(move |slint_window, event| {
                if debug_menu()
                    && weak.upgrade().is_some_and(|shell| {
                        matches!(shell.get_property("menu-open"), Ok(SValue::Bool(true)))
                    })
                {
                    debug_menu_pointer(slint_window, &menu_cursor, event);
                }
                // Before Slint lays out and draws at the new size.
                if matches!(event, winit::event::WindowEvent::Resized(_)) {
                    platform::fullscreen_resized(slint_window);
                }
                track_pointer(&input, generation, slint_window, event);
                let dead = observe_key(&input, &modifiers, event);
                release_modifiers(&input, &events, generation);
                if let winit::event::WindowEvent::DroppedFile(path) = event {
                    let _ = events.send(Event::Dropped(generation, path.clone()));
                }
                if inject_option_key(&input, &modifiers, &events, generation, event) {
                    return EventResult::PreventDefault;
                }
                if let Some((dead, repeat)) = dead {
                    // Slint drops a dead key (it has no text), so its Option
                    // binding resolves here, only while the video area has
                    // focus: a focused text field composes with the dead key.
                    let focused = weak.upgrade().is_some_and(|shell| {
                        matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)))
                            && !matches!(shell.get_property("menu-open"), Ok(SValue::Bool(true)))
                    });
                    if focused && fire(&input, &events, generation, &dead, repeat, None) {
                        return EventResult::PreventDefault;
                    }
                }
                #[cfg(feature = "producer-audio")]
                match event {
                    winit::event::WindowEvent::Focused(false) | winit::event::WindowEvent::Occluded(true) | winit::event::WindowEvent::Destroyed => {
                        let _=events.send(Event::PhysicalInput(generation,json!({"kind":"revoke","reason":"focusLost"})));
                    }
                    winit::event::WindowEvent::KeyboardInput {event,is_synthetic:false,..} => {
                        let Some(shell) = weak.upgrade() else { return EventResult::Propagate; };
                        if !matches!(shell.get_property("canvas-focused"),Ok(slint_interpreter::Value::Bool(true)))
                            || matches!(shell.get_property("menu-open"),Ok(slint_interpreter::Value::Bool(true)))
                            || matches!(shell.get_property("input-blocked"),Ok(slint_interpreter::Value::Bool(true))) { return EventResult::Propagate; }
                        let routing = routing.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                        let Some(routing) = routing.filter(|routing| routing.captured) else { return EventResult::Propagate; };
                        if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
                            let code = format!("{code:?}");
                            // Keys bound to the host's own commands (fullscreen, exit fullscreen) pass.
                            if host_key(&input, &code) {return EventResult::Propagate;}
                            if routing.keys.contains(&code) {
                                if !event.repeat {
                                    let _=events.send(Event::PhysicalInput(generation,json!({"kind":"key","scope":routing.scope,"code":code,"down":event.state==winit::event::ElementState::Pressed})));
                                }
                                return EventResult::PreventDefault;
                            }
                        }
                    }
                    _=>{}
                }
                EventResult::Propagate
            });
        }
        let events = self.events.clone();
        #[cfg(feature = "producer-audio")]
        let routing = self.backend.input.clone();
        window.set_callback("window-input", move |args| {
            if let Some(slint_interpreter::Value::String(kind)) = args.first() {
                #[cfg(feature = "producer-audio")]
                match kind.as_str() {
                    "capture" => {
                        if let Some(routing) = routing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                        {
                            let _ = events.send(Event::PhysicalInput(
                                generation,
                                json!({"kind":"capture","scope":routing.scope}),
                            ));
                        }
                    }
                    "canvasFocusLost" => {
                        let _ = events.send(Event::PhysicalInput(
                            generation,
                            json!({"kind":"revoke","reason":"focusLost"}),
                        ));
                    }
                    _ => {}
                }
                let _ = events.send(Event::WindowInput(generation, kind.to_string()));
            }
            slint_interpreter::Value::Void
        })?;
        self.bind_menu(window, generation, true)?;
        self.bind_keys(window, generation)?;
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("mouse-input", move |args| {
            let [SValue::String(name), mods @ ..] = args else {
                return SValue::Bool(false);
            };
            let flag = |i: usize| matches!(mods.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(0), flag(3));
            let event = bindings::Input {
                mods: bindings::Mods {
                    ctrl,
                    alt: flag(1),
                    shift: flag(2),
                    cmd,
                },
                name: format!("Mouse:{name}"),
            };
            SValue::Bool(fire(&input, &events, generation, &event, false, None))
        })?;
        // G-05: dragging the video moves the window; the press is no click.
        let input = self.input.clone();
        let weak = window.as_weak();
        window.set_callback("drag-window", move |_| {
            input.borrow_mut().click = None;
            if let Some(window) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                window.window().with_winit_window(|window| {
                    // Systems without it (some Linux) refuse; the window stays.
                    let _ = window.drag_window();
                });
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("pip-return", move |_| {
            let _ = events.send(Event::PipReturn(generation));
            SValue::Void
        })?;
        let input = self.input.clone();
        window.set_callback("left-press", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let flag = |i: usize| matches!(args.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(2), flag(5));
            let mut shared = input.borrow_mut();
            if !shared.table.has_left_click() {
                shared.click = None;
                return SValue::Void;
            }
            let (x, y) = (number(0), number(1));
            let now = Instant::now();
            // A second press within the double-click interval is a double
            // click; a first click still waiting for it is no click.
            let second = shared
                .click
                .take()
                .is_some_and(|first| first.doubled_by(x, y, now, double_click_interval()));
            input_trace::mark(if second {
                "video left press: a double click"
            } else {
                "video left press: a click pending"
            });
            if !second {
                let mods = bindings::Mods {
                    ctrl,
                    alt: flag(3),
                    shift: flag(4),
                    cmd,
                };
                shared.click = Some(video_click::Press::new(now, x, y, mods, generation));
            }
            SValue::Void
        })?;
        let input = self.input.clone();
        window.set_callback("left-release", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let mut shared = input.borrow_mut();
            if matches!(args.get(2), Some(SValue::Bool(true))) {
                shared.click = None;
            } else if let Some(click) = shared.click.as_mut().filter(|c| c.released.is_none()) {
                click.released = Some((number(0), number(1)));
                input_trace::mark("video left release");
            }
            SValue::Void
        })?;
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("wheel-input", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let flag = |i: usize| matches!(args.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(2), flag(5));
            let mods = bindings::Mods {
                ctrl,
                alt: flag(3),
                shift: flag(4),
                cmd,
            };
            let (dx, dy) = (number(0), number(1));
            let (vertical, horizontal) = {
                let shared = input.borrow();
                (shared.table.has_wheel(true), shared.table.has_wheel(false))
            };
            if !vertical && !horizontal {
                return SValue::Bool(false);
            }
            let mut steps = Vec::new();
            {
                let mut shared = input.borrow_mut();
                shared.wheel.0 += dx;
                shared.wheel.1 += dy;
                while shared.wheel.1.abs() >= WHEEL_STEP {
                    let up = shared.wheel.1 > 0.;
                    shared.wheel.1 -= WHEEL_STEP.copysign(shared.wheel.1);
                    steps.push(if up { "WheelUp" } else { "WheelDown" });
                }
                while shared.wheel.0.abs() >= WHEEL_STEP {
                    let left = shared.wheel.0 > 0.;
                    shared.wheel.0 -= WHEEL_STEP.copysign(shared.wheel.0);
                    steps.push(if left { "WheelLeft" } else { "WheelRight" });
                }
            }
            for step in steps {
                let event = bindings::Input {
                    mods,
                    name: format!("Mouse:{step}"),
                };
                fire(&input, &events, generation, &event, false, None);
            }
            SValue::Bool(true)
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-resized", move |args| {
            if let Some(SValue::Number(width)) = args.first() {
                let _ = events.send(Event::SidebarResized(generation, *width));
            }
            SValue::Void
        })?;
        window.set_property("sidebar-width", self.policy.sidebar_user_width().into())?;
        window.set_property(
            "window-title",
            SValue::String(self.window_title.as_str().into()),
        )?;
        window.set_property(
            "info-panels",
            ui_values::from_json(&json!(self.info_panels.values().collect::<Vec<_>>()))?,
        )?;
        let roots = self.commands.snapshot().roots;
        let labels = self.labels();
        set_menu_model(
            window,
            menu_levels(
                window,
                &roots,
                &mut self.menu_path.clone(),
                &self.menu_scroll,
                &labels,
            )?
            .0,
        )?;
        if let Some(image) = &self.last_image {
            window.set_property("video-frame", image.clone().into())?;
        }
        window.set_property("no-video", self.no_video.into())?;
        Ok(())
    }
    /// Menu callbacks shared by the shell and the floating window.
    fn bind_menu(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
        reset: bool,
    ) -> Result<()> {
        let events = self.events.clone();
        window.set_callback("menu-hover", move |args| {
            if let [
                SValue::Number(level),
                SValue::String(token),
                SValue::Bool(now),
            ] = args
                && *level >= 0.
            {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Hover(*level as usize, token.to_string(), *now),
                ));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("menu-scroll", move |args| {
            if let [SValue::Number(level), SValue::Number(offset)] = args
                && *level >= 0.
            {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Scroll(*level as usize, *offset),
                ));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("menu-invoke", move |args| {
            if let Some(SValue::String(token)) = args.first() {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Invoke(token.to_string()),
                ));
            }
            SValue::Void
        })?;
        let mut callbacks = vec!["menu-back", "menu-dismiss"];
        if reset {
            callbacks.push("menu-reset");
        }
        for callback in callbacks {
            let events = self.events.clone();
            window.set_callback(callback, move |_| {
                let action = match callback {
                    "menu-reset" => MenuAction::Reset,
                    "menu-back" => MenuAction::Back,
                    _ => MenuAction::Dismiss,
                };
                let _ = events.send(Event::MenuAction(generation, action));
                SValue::Void
            })?;
        }
        Ok(())
    }
    /// A floating panel forwards keys to the binding table and records each
    /// press's physical key first (see `observe_key`). Its standard window
    /// shortcuts (⌘W…) come before that and never reach the table; the main
    /// window has its own hook (`bind_shell`) without them.
    fn observe_keys(&self, window: &slint_interpreter::ComponentInstance, generation: u64) {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
        let input = self.input.clone();
        let events = self.events.clone();
        let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
        let shortcuts = ui::WindowShortcuts::default();
        window.window().on_winit_window_event(move |window, event| {
            if shortcuts.filter(window, event) {
                return EventResult::PreventDefault;
            }
            track_pointer(&input, generation, window, event);
            let _ = observe_key(&input, &modifiers, event);
            release_modifiers(&input, &events, generation);
            if let winit::event::WindowEvent::DroppedFile(path) = event {
                let _ = events.send(Event::Dropped(generation, path.clone()));
            }
            if inject_option_key(&input, &modifiers, &events, generation, event) {
                return EventResult::PreventDefault;
            }
            EventResult::Propagate
        });
    }
    /// Key forwarding: `key-input(scope, text, control, alt, shift, meta, repeat)`.
    fn bind_keys(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("key-input", move |args| {
            let [SValue::String(scope), SValue::String(text), rest @ ..] = args else {
                return SValue::Bool(false);
            };
            let flag = |i: usize| matches!(rest.get(i), Some(SValue::Bool(true)));
            // The press this callback reports was recorded just before by
            // the window's winit hook, in the same event dispatch.
            let physical = input.borrow_mut().physical.take();
            let resolved =
                bindings::from_key(text, flag(0), flag(1), flag(2), flag(3), physical.as_ref());
            // A symbol typed with Option (German ⌥5 = `[`) is tried first:
            // the first candidate with a binding fires.
            let typed = bindings::typed_with_option(text, flag(0), flag(3), physical.as_ref());
            if debug_keys() {
                diag!(
                    "keys: slint scope={:?} text={:?} control={} alt={} shift={} meta={} repeat={} physical={physical:?} → {resolved:?} typed={typed:?} bound={}",
                    scope.as_str(),
                    text.as_str(),
                    flag(0),
                    flag(1),
                    flag(2),
                    flag(3),
                    flag(4),
                    resolved
                        .as_ref()
                        .is_some_and(|event| input.borrow().table.resolve(event, None).is_some())
                );
            }
            let Some(mut event) = resolved else {
                return SValue::Bool(false);
            };
            let panel = {
                let shared = input.borrow();
                match scope.as_str() {
                    "panel:bottom" => shared.bottom,
                    "panel:sidebar" => shared.sidebar,
                    _ => None,
                }
            };
            if let Some(typed) = typed
                && input.borrow().table.lookup(&typed, panel) != bindings::Lookup::Unbound
            {
                event = typed;
            }
            SValue::Bool(fire(&input, &events, generation, &event, flag(4), panel))
        })?;
        Ok(())
    }
    fn labels(&self) -> impl Fn(u64, &str) -> Option<String> + use<> {
        let input = self.input.clone();
        move |owner, id| input.borrow().table.label(owner, id)
    }
    /// Show the menu in the window of `generation` (shell or floating).
    fn open_menu(&mut self, generation: u64, mode: MenuMode) {
        if self.menu_open && self.menu_generation != generation {
            self.close_menu();
        }
        self.menu_open = true;
        self.menu_generation = generation;
        self.menu_mode = mode;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        if let Some(window) = self.window_for(generation) {
            let _ = window.set_property("menu-open", true.into());
        }
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// Open this chain of submenus now. Levels that close or change forget
    /// their scroll offset; a waiting delayed hover is dropped.
    fn set_menu_path(&mut self, path: Vec<String>) {
        self.menu_aim.clear();
        if path == self.menu_path {
            return;
        }
        let common = path
            .iter()
            .zip(&self.menu_path)
            .take_while(|(a, b)| a == b)
            .count();
        self.menu_scroll.truncate(common + 1);
        self.menu_path = path;
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// G-15: apply a submenu switch held while the pointer headed for the
    /// open submenu, once it turned away or stopped.
    fn menu_tick(&mut self) {
        if !self.menu_open {
            return;
        }
        let now = self.now_ms();
        let pointer = self.menu_pointer();
        if let Some(path) = self.menu_aim.tick(pointer, now) {
            self.set_menu_path(path);
        }
    }
    /// The pointer in the window the menu is open in (logical px).
    fn menu_pointer(&self) -> Option<menu::Point> {
        self.input
            .borrow()
            .pointer
            .filter(|(generation, _)| *generation == self.menu_generation)
            .map(|(_, point)| point)
    }
    fn refresh_menu(&mut self) -> Result<()> {
        let labels = self.labels();
        let Some(window) = self
            .window_for(self.menu_generation)
            .or(self.shell.as_ref())
            .map(ComponentHandle::clone_strong)
        else {
            return Ok(());
        };
        let window = &window;
        let model = match &self.menu_mode {
            MenuMode::Main => {
                let roots = self.commands.snapshot().roots;
                let (model, panels) = menu_levels(
                    window,
                    &roots,
                    &mut self.menu_path,
                    &self.menu_scroll,
                    &labels,
                )?;
                self.menu_panels = panels;
                model
            }
            MenuMode::Panel(owner, items) => {
                // A command removed while its panel menu is open drops out.
                let entries: Vec<_> = items
                    .iter()
                    .filter_map(|id| match id {
                        None => Some(None),
                        Some(id) => self.commands.get(*owner, id).map(Some),
                    })
                    .collect();
                let (origin, size) = menu_frame(window);
                let value = menu::context_level(
                    &entries,
                    origin,
                    size,
                    self.menu_scroll.first().copied().unwrap_or(0.),
                    &labels,
                );
                self.menu_panels = menu::panels(&value);
                ui_values::from_json(&value)?
            }
        };
        set_menu_model(window, model)
    }
    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        self.menu_mode = MenuMode::Main;
        for window in [
            self.shell.as_ref(),
            self.floating.as_ref().map(|f| &f.instance),
        ]
        .into_iter()
        .flatten()
        {
            let _ = window.set_property("menu-open", false.into());
        }
        if let Some(shell) = &self.shell {
            let result = shell
                .invoke("close-menu", &[])
                .map(|_| ())
                .map_err(anyhow::Error::from);
            self.report_menu(result);
        }
    }
    fn prepare_mount_tick(&mut self) {
        self.release_held_mounts();
        if self.preparing.is_none() {
            if let Some(change) = self.mount_queue.pop_front() {
                if !change.announced {
                    self.send(
                        change.reply.owner,
                        json!({"event":"ui.preparing","data":{"phase":"preparing"}}),
                    );
                }
                self.preparing = Some((Instant::now(), change));
            }
            return;
        }
        let waited = self.preparing.as_ref().unwrap().0.elapsed();
        if waited < Duration::from_millis(32) {
            return;
        }
        // At start-up, a built-in that will mount its panel and has not
        // asked yet is waited for (briefly), to compile the shell once.
        if waited < STARTUP_MOUNT_GATHER && self.builtin_mount_expected() {
            return;
        }
        let (_, change) = self.preparing.take().unwrap();
        let started = Instant::now();
        // Every change queued meanwhile joins this one: one compile of the
        // shell instead of one per mount (start-up mounts arrive together).
        // After a failed batch its changes go one at a time again, so that
        // one bad component fails only its own mount.
        let mut changes = vec![change];
        if self.mount_singly > 0 {
            self.mount_singly -= 1;
        } else {
            for change in self.mount_queue.drain(..).collect::<Vec<_>>() {
                if !change.announced {
                    self.send(
                        change.reply.owner,
                        json!({"event":"ui.preparing","data":{"phase":"preparing"}}),
                    );
                }
                changes.push(change);
            }
        }
        // A mount of a plugin stopped since is dropped, as before.
        changes.retain(|change| {
            change.contribution.is_none() || self.plugins.contains_key(&change.reply.owner)
        });
        if changes.is_empty() {
            return;
        }
        let mut next = self.mounts.clone();
        for change in &changes {
            let owner = change.reply.owner;
            if let Some(mount) = &change.contribution {
                // Mounted after all: an earlier refusal no longer applies.
                self.clear_warning(owner, "slot-occupied");
                next.insert(owner, mount.clone());
            } else {
                next.remove(&owner);
            }
        }
        self.next_shell_generation += 1;
        let candidate_generation = self.next_shell_generation;
        let native_before = platform::capture_windows().ok();
        // The sidebar's panels in tab order (after the other slots' panels).
        let order = self.tab_order(&next);
        let shown = next
            .values()
            .filter(|m| m.spec.slot != composition::Slot::Sidebar)
            .chain(order.iter().filter_map(|owner| next.get(owner)))
            .cloned()
            .collect::<Vec<_>>();
        let values: Vec<_> = shown.iter().map(|mount| self.mount_values(mount)).collect();
        let slot = self.sidebar_slot();
        let sidebar = self
            .policy
            .sidebar_state()
            .unwrap_or((slot.visible, slot.detached));
        let titles = ui_values::from_json(&json!(
            order
                .iter()
                .filter_map(|owner| next.get(owner).map(|m| m.title.clone()))
                .collect::<Vec<_>>()
        ));
        let build = || -> Result<slint_interpreter::ComponentInstance> {
            let specs = shown.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
            let shell =
                composition::compile_json(&specs, self.mount_events.clone(), candidate_generation)?;
            for (mount, values) in shown.iter().zip(values) {
                for (name, value) in values {
                    composition::set_global_value(&shell, &mount.spec, &name, value)?;
                }
                let visible = match mount.spec.slot {
                    composition::Slot::Sidebar => {
                        composition::set_tabbed(&shell, &mount.spec, order.len() > 1)?;
                        sidebar.0 && !sidebar.1
                    }
                    _ => mount.visible && !mount.detached,
                };
                if let Some(property) = mount.spec.slot.visible_property() {
                    shell.set_property(property, visible.into())?;
                }
            }
            shell.set_property("sidebar-titles", titles?)?;
            // The tab that will be current, so the first frame shows it.
            let active = order
                .iter()
                .position(|&o| Some(o) == self.policy.sidebar_active())
                .or_else(|| {
                    order.iter().position(|o| {
                        next.get(o).map(|m| m.manifest.as_str()) == slot.active.as_deref()
                    })
                })
                .unwrap_or(0);
            shell.set_property("sidebar-active", SValue::Number(active as f64))?;
            self.bind_shell(&shell, candidate_generation)?;
            self.bind_tabs(&shell, candidate_generation)?;
            if let Some(old) = &self.shell {
                let size = old.window().size();
                let scale = old.window().scale_factor();
                shell.window().set_size(slint::LogicalSize::new(
                    size.width as f32 / scale,
                    size.height as f32 / scale,
                ));
                shell.window().set_position(old.window().position());
                shell.window().set_fullscreen(old.window().is_fullscreen());
            }
            let texture = self.attach_gpu(&shell);
            shell.show()?;
            // Only a shown candidate replaces the main window's texture.
            self.video_texture.replace(texture);
            platform::shown(&shell);
            frame_pacing::follow(&shell);
            Ok(shell)
        };
        // Where the old shell's frame is (`ui::Placements`).
        let position = self.shell.as_ref().map(|old| old.window().position());
        let result = match build() {
            Ok(shell) => {
                if let Some(position) = position {
                    self.placements.track(&shell, position.into());
                }
                self.theme_view(&shell);
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::WindowHidden);
                self.native_before = native_before.clone();
                self.native_window = native_before
                    .as_ref()
                    .and_then(|before| platform::associate_new(before).ok());
                if let Some(old) = self.shell.replace(shell) {
                    let _ = old.hide();
                }
                let old_owners = self.mounts.keys().copied().collect::<Vec<_>>();
                for old in old_owners {
                    if !next.contains_key(&old) {
                        self.policy.remove_owner(old);
                        self.images.release_view(old, "$mount");
                    }
                }
                for mount in &shown {
                    let id = mount.spec.owner;
                    if self.mounts.contains_key(&id) {
                        continue;
                    }
                    let Some(policy) = policy_slot(mount.spec.slot) else {
                        continue;
                    };
                    let (visible, detached) = match policy {
                        window_state::Slot::Sidebar => (slot.visible, slot.detached),
                        window_state::Slot::Bottom => (mount.visible, false),
                    };
                    if let Err(error) = self.policy.attach(policy, id, visible, detached) {
                        diag!("mount policy attach: {error}");
                    }
                    // The tab that was current at the last run is current again.
                    if policy == window_state::Slot::Sidebar
                        && slot.active.as_deref() == Some(mount.manifest.as_str())
                    {
                        let _ = self.policy.set_active(id);
                    }
                }
                self.policy.order_sidebar(&order);
                for (id, mount) in &next {
                    let Some(slot) = policy_slot(mount.spec.slot) else {
                        continue;
                    };
                    if let Err(error) =
                        self.policy
                            .set_locks(slot, *id, window_state::Locks::default())
                    {
                        diag!("mount policy reset: {error}");
                    }
                }
                self.policy.activity(self.now_ms());
                {
                    let mut input = self.input.borrow_mut();
                    input.bottom = next
                        .values()
                        .find(|m| m.spec.slot == composition::Slot::Bottom)
                        .map(|m| m.spec.owner);
                    input.sidebar = self.policy.sidebar_active();
                }
                if self.menu_generation == self.shell_generation {
                    self.menu_open = false;
                    self.menu_mode = MenuMode::Main;
                }
                self.mounts = next;
                self.shell_generation = candidate_generation;
                self.shell_tabs = order;
                self.remember_sidebar(false);
                Ok(
                    json!({"status":"ready","shellGeneration":candidate_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
                )
            }
            Err(error) => Err(RpcError::new("ui-prepare", error.to_string())),
        };
        if self.frame_log.is_some() {
            diag!(
                "vestra shell: {} change(s), {} panel(s), prepared in {:.1} ms ({})",
                changes.len(),
                self.mounts.len(),
                started.elapsed().as_secs_f64() * 1000.,
                if result.is_ok() { "ready" } else { "failed" }
            );
        }
        if result.is_err() && changes.len() > 1 {
            // Again one at a time, from the next tick on (each still one
            // compile per tick): the first, then the rest in their order.
            self.mount_singly = changes.len();
            let mut changes = changes.into_iter();
            let first = changes.next().expect("more than one change");
            for mut change in changes.rev() {
                change.announced = true;
                self.mount_queue.push_front(change);
            }
            let due = Instant::now()
                .checked_sub(Duration::from_millis(32))
                .unwrap_or_else(Instant::now);
            self.preparing = Some((due, first));
            return;
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        for change in changes {
            let owner = change.reply.owner;
            self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed}}));
            if change.reply.id == 0 {
                if let Err(error) = &result {
                    self.broadcast(
                        "host.error",
                        json!({"code":"ui-owner-cleanup","owner":owner,"message":error.message}),
                    );
                }
            } else {
                self.respond(change.reply, result.clone());
            }
        }
    }
    /// Initial properties `refuse_initial` left out: logged and reported
    /// to the plugin (`ui.error`, its `onError`).
    fn report_refused(&mut self, owner: u64, refused: Vec<RpcError>) {
        for error in refused {
            diag!("initial property {owner}: {}", error.message);
            self.send(owner, ui_error(&error));
        }
    }
    /// A mount's properties converted for a new instance. They were checked
    /// when written, so a value that no longer converts is an image file
    /// removed since (or a document over the text total): it is left out (the
    /// property keeps its default), logged and reported to the plugin
    /// (`ui.error`, its `onError`).
    fn mount_values(&mut self, mount: &Mount) -> Vec<(String, SValue)> {
        let owner = mount.spec.owner;
        let mut values = Vec::new();
        for (name, value) in &mount.properties {
            match self.prop_value(owner, "$mount", name, value) {
                Ok(value) => values.push((name.clone(), value)),
                Err(error) => {
                    diag!("mount property {owner}/{name}: {}", error.message);
                    self.send(owner, ui_error(&error));
                }
            }
        }
        values
    }
    fn mount_callbacks(&mut self) {
        for event in self.mount_input.try_iter().take(64).collect::<Vec<_>>() {
            if !self.window_generation_live(event.shell_generation)
                || !self.mounts.contains_key(&event.owner)
            {
                continue;
            }
            match event.args {
                Ok(args)=>self.send(event.owner,json!({"event":"ui.callback","data":{"view":"$mount","callback":event.callback,"args":args}})),
                Err(error)=>self.send(event.owner,json!({"event":"ui.error","data":{"code":"ui","message":format!("callback {}: {error}",event.callback)}})),
            }
        }
    }
    fn execute_command(&mut self, owner: u64, id: &str) -> std::result::Result<(), RpcError> {
        let execution = self
            .commands
            .resolve_execution(owner, id)
            .map_err(|e| RpcError::new("command", e.to_string()))?;
        if owner == bindings::HOST {
            self.host_command(id);
            return Ok(());
        }
        self.send(execution.owner,json!({"event":"commands.execute","data":{"id":execution.local_id,"revision":execution.revision}}));
        Ok(())
    }
    /// The host's own commands (owner 0): fullscreen, exit fullscreen, quit.
    fn register_host_commands(&mut self) -> Result<()> {
        let state = |enabled: bool, checked: Option<bool>| commands::CommandState {
            enabled,
            checked,
            busy: false,
            reason: None,
        };
        let mut fullscreen =
            commands::Declaration::new("Toggle Fullscreen", &["Toggle Fullscreen"]);
        fullscreen.group = "screen".into();
        fullscreen.order = 700;
        // IA-CORE-04: Enter and Alt+Enter toggle full screen; F11 too. No
        // mouse default (user ruling 2026-10-26): a click on the video is
        // play/pause, and a double click does nothing unless the user binds it.
        fullscreen.bindings = vec!["Enter".into(), "Alt+Enter".into(), "F11".into()];
        fullscreen.localize(|_| i18n::tr("Toggle Fullscreen"));
        let mut exit = commands::Declaration::new("Leave Fullscreen", &[]);
        exit.bindings = vec!["Escape".into()];
        exit.localize(|_| i18n::tr("Leave Fullscreen"));
        let mut quit = commands::Declaration::new("Quit", &["Quit"]);
        quit.localize(|_| i18n::tr("Quit"));
        quit.group = "exit".into();
        quit.order = 990;
        // Windows and Linux menus show no `Cmd` binding (`label_shown`).
        quit.bindings = vec!["Cmd+Q".into()];
        // Player 0.5.10: the bottom bar's sidebar toggle (the rightmost cell
        // of `BarButtons`, bar_buttons.rs); outside the menu, no default key
        // (F6 is the playlist's), bindable in Keyboard Shortcuts.
        let mut sidebar = commands::Declaration::new("Toggle Sidebar", &[]);
        sidebar.localize(|_| i18n::tr("Toggle Sidebar"));
        for (id, declaration, state) in [
            ("fullscreen", fullscreen, state(true, Some(false))),
            ("exit-fullscreen", exit, state(false, None)),
            ("quit", quit, state(true, None)),
            // `bar_buttons::SIDEBAR` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("sidebar", sidebar, state(false, Some(false))),
        ] {
            self.input
                .borrow_mut()
                .table
                .declare(
                    bindings::HOST,
                    id,
                    "host",
                    &declaration.bindings,
                    false,
                    false,
                )
                .map_err(anyhow::Error::msg)?;
            self.commands
                .register(bindings::HOST, "host", id.into(), 1, declaration, state)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        self.host_revision = 1;
        {
            for (id, declaration) in core_actions::declarations(self.step_seconds()) {
                self.register_core_action(id, declaration)?;
            }
            self.core_action_states();
        }
        Ok(())
    }
    fn register_core_action(&mut self, id: &str, declaration: commands::Declaration) -> Result<()> {
        self.input
            .borrow_mut()
            .table
            .declare(
                bindings::HOST,
                id,
                "host",
                &declaration.bindings,
                false,
                declaration.repeat,
            )
            .map_err(anyhow::Error::msg)?;
        let state = core_actions::state(id, &core_actions::Facts::default(), false);
        self.commands
            .register(
                bindings::HOST,
                "host",
                id.into(),
                self.host_revision.max(1),
                declaration,
                state,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    /// The first-tier relative seek: the newest `setStepSeek`, else 5 seconds.
    fn step_seconds(&self) -> f64 {
        self.step_seek
            .values()
            .max_by_key(|(sequence, ..)| *sequence)
            .map_or(core_actions::DEFAULT_STEP, |(_, seconds, _)| *seconds)
    }
    /// Whether step seeks land on keyframes: the newest `setStepSeek`'s
    /// `keyframes` (written as a pair with the seconds), else exact.
    fn step_keyframes(&self) -> bool {
        self.step_seek
            .values()
            .max_by_key(|(sequence, ..)| *sequence)
            .is_some_and(|(.., keyframes)| *keyframes)
    }
    /// The seek menu titles carry the step: re-register them when it changes.
    fn step_seek_changed(&mut self, before: f64) {
        let after = self.step_seconds();
        if after == before {
            return;
        }
        for (id, declaration) in core_actions::declarations(after) {
            if id == "seek-backward" || id == "seek-forward" {
                let _ = self.commands.remove(bindings::HOST, id);
                if let Err(error) = self.register_core_action(id, declaration) {
                    diag!("host.error core action {id}: {error}");
                }
            }
        }
        self.core_action_states();
    }
    /// The last source state with the sequence flags merged in.
    fn core_facts(&self) -> core_actions::Facts {
        let mut state = self.last_source.clone();
        self.patch_source(&mut state);
        core_actions::Facts::from_state(&state)
    }
    /// Publish changed enabled/checked/busy states of the registered actions.
    fn core_action_states(&mut self) {
        let facts = self.core_facts();
        for id in core_actions::ACTIONS {
            if core_actions::WINDOW_ACTIONS.contains(&id) {
                continue;
            }
            let state = core_actions::state(id, &facts, self.actions_busy.contains(id));
            if self
                .commands
                .get(bindings::HOST, id)
                .is_some_and(|entry| entry.state != state)
            {
                self.host_revision += 1;
                let _ = self
                    .commands
                    .update(bindings::HOST, id, self.host_revision, state);
            }
        }
    }
    /// Run a core action for a binding, the menu or `host.run`. `reply` (a
    /// plugin's `host.run`) is answered when it finished. `Ok(true)` means
    /// the action is done and the caller answers now (a sequence step
    /// handed to its provider, a repeat dropped while the same action runs,
    /// a window action); `Ok(false)` means the backend answers `reply`
    /// itself later (`host.run` then returns no reply). A disabled action
    /// is `unavailable`.
    fn run_core_action(
        &mut self,
        action: &str,
        reply: Option<&Reply>,
    ) -> std::result::Result<bool, RpcError> {
        self.run_core_action_with(action, reply, None)
    }
    /// `run_core_action` with a seek step other than the user's (the PiP
    /// window's skip interval).
    fn run_core_action_with(
        &mut self,
        action: &str,
        reply: Option<&Reply>,
        step: Option<f64>,
    ) -> std::result::Result<bool, RpcError> {
        if core_actions::WINDOW_ACTIONS.contains(&action) {
            // `host.run("fullscreen")` (host 0.5.2): what the Toggle
            // Fullscreen key does, answered at once.
            return if self.window_command(action) {
                Ok(true)
            } else {
                Err(RpcError::new("unavailable", "the player has no window"))
            };
        }
        let facts = self.core_facts();
        let state = core_actions::state(action, &facts, false);
        if !state.enabled {
            return Err(RpcError::new(
                "unavailable",
                state.reason.unwrap_or_else(|| "unavailable".into()),
            ));
        }
        if matches!(action, "previous" | "next") {
            let (provider, ..) = self
                .sequence
                .ok_or_else(|| RpcError::new("unavailable", "no sequence provider"))?;
            self.send(
                provider,
                json!({"event":"sequence.step","data":{"direction":action}}),
            );
            input_trace::mark(&format!(
                "host loop: sequence.step {action} sent to plugin {provider}"
            ));
            return Ok(true);
        }
        if core_actions::exclusive(action) && !self.actions_busy.insert(action.to_owned()) {
            // The same action is still running: this trigger is dropped.
            return Ok(true);
        }
        self.core_action_states();
        let reply = reply.cloned().unwrap_or(Reply {
            owner: bindings::HOST,
            id: 0,
        });
        self.backend
            .commands
            .send(Command {
                reply,
                method: "host.action".into(),
                params: json!({"action":action,"step":step.unwrap_or_else(|| self.step_seconds()),
                    "keyframes":self.step_keyframes(),
                    "sequence":{"next":facts.next,"previous":facts.previous}}),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        input_trace::mark(&format!("host loop: {action} sent to the playback worker"));
        Ok(false)
    }
    fn action_done(
        &mut self,
        reply: Reply,
        action: &str,
        result: std::result::Result<Value, RpcError>,
    ) {
        self.actions_busy.remove(action);
        match &result {
            Ok(value) => {
                if let Some(text) = value["osd"].as_str() {
                    self.show_osd(text);
                }
            }
            // Failures show on the video, like the playback bar did.
            Err(error) if reply.owner == bindings::HOST => self.show_osd(&error.message),
            Err(_) => {}
        }
        self.core_action_states();
        if reply.owner != bindings::HOST {
            self.respond(reply, result.map(|_| Value::Null));
        }
    }
    /// One OSD line; nothing without a window.
    fn show_osd(&mut self, text: &str) {
        let text: String = text.replace(['\n', '\r'], " ").chars().take(200).collect();
        if let Some(shell) = &self.shell
            && !text.trim().is_empty()
        {
            let _ = shell.set_property("osd-text", SValue::String(text.as_str().into()));
            let _ = shell.set_property("osd-visible", true.into());
            self.osd_until = Some(self.now_ms() + OSD_MS);
        }
    }
    fn host_command_states(&mut self, fullscreen: bool) {
        let checked = self
            .commands
            .get(bindings::HOST, "fullscreen")
            .and_then(|entry| entry.state.checked);
        if checked == Some(fullscreen) {
            return;
        }
        self.host_revision += 1;
        let revision = self.host_revision;
        let _ = self.commands.update(
            bindings::HOST,
            "fullscreen",
            revision,
            commands::CommandState {
                enabled: true,
                checked: Some(fullscreen),
                busy: false,
                reason: None,
            },
        );
        let _ = self.commands.update(
            bindings::HOST,
            "exit-fullscreen",
            revision,
            commands::CommandState {
                enabled: fullscreen,
                checked: None,
                busy: false,
                reason: None,
            },
        );
    }
    fn host_command(&mut self, id: &str) {
        if core_actions::ACTIONS.contains(&id) && !core_actions::WINDOW_ACTIONS.contains(&id) {
            if let Err(error) = self.run_core_action(id, None) {
                diag!("core action {id}: {}", error.message);
            }
            return;
        }
        if id == bar_buttons::SIDEBAR {
            self.toggle_sidebar();
            return;
        }
        if id == "quit" {
            stop_handoffs();
            self.ui_state.flush();
            let _ = slint::quit_event_loop();
            return;
        }
        self.window_command(id);
    }
    /// The main window's commands, for a key, the menu and `host.run`:
    /// `fullscreen` toggles (in when windowed, out when fullscreen),
    /// `exit-fullscreen` leaves. False without a window.
    fn window_command(&self, id: &str) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let window = shell.window();
        match id {
            "fullscreen" => window.set_fullscreen(!window.is_fullscreen()),
            "exit-fullscreen" => window.set_fullscreen(false),
            _ => {}
        }
        true
    }
    /// Merge the sequence provider's flags into a source state (C6).
    fn patch_source(&self, state: &mut Value) {
        if state.is_object() {
            let (next, previous) = self
                .sequence
                .map(|(_, next, previous)| (next, previous))
                .unwrap_or((false, false));
            state["sequence"] = json!({"next":next,"previous":previous});
        }
    }
    /// Re-send the last source state with the current sequence flags.
    fn rebroadcast_source(&mut self) {
        if self.last_source.is_object() {
            let mut state = self.last_source.clone();
            self.patch_source(&mut state);
            self.last_source = state.clone();
            self.broadcast("source.state", state);
            self.core_action_states();
        }
    }
    /// R-U3: follow the source into `no-video`; an empty source also drops
    /// the last video frame, so the video area shows the backdrop or plain colour.
    /// A mounted backdrop shows audio files itself: mpv stops picking cover
    /// art as a video track while one is mounted (from the next file opened).
    fn backdrop_cover_art(&mut self) {
        let mounted = self
            .mounts
            .values()
            .any(|m| m.spec.slot == composition::Slot::Backdrop);
        if mounted != self.cover_art_off {
            self.cover_art_off = mounted;
            let _ = self.backend.commands.send(Command {
                reply: Reply {
                    owner: bindings::HOST,
                    id: 0,
                },
                method: "host.coverArt".into(),
                params: json!({ "off": mounted }),
            });
        }
    }
    fn backdrop_source(&mut self) {
        self.video_absent = composition::video_absent(self.video_absent, &self.last_source);
        let next = composition::no_video(self.no_video, &self.last_source);
        let empty = self.last_source["phase"] == "empty";
        let Some(shell) = &self.shell else {
            self.no_video = next;
            return;
        };
        // GPU frames are not kept in `last_image`: clear once per empty phase.
        let gpu = self
            .backend
            .render
            .gpu()
            .filter(|_| empty && !self.video_cleared);
        self.video_cleared = empty;
        if empty && self.last_image.take().is_some() || gpu.is_some() {
            let _ = shell.set_property("video-frame", SValue::Image(Default::default()));
            // The next source starts from its own first frame, not this one's last.
            if let Some(gpu) = gpu {
                gpu.reshow();
            }
        }
        if next != self.no_video {
            self.no_video = next;
            let _ = shell.set_property("no-video", next.into());
        }
    }
    /// C9: "<source name> - Vestra Player", or "Vestra Player" without a source.
    fn update_title(&mut self) {
        let source = &self.last_source;
        let name = match source["source"].as_str() {
            None => None,
            Some(_) if source["kind"] == "producer" => None,
            Some(text) if text.contains("://") => Some(
                source["title"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(text)
                    .to_owned(),
            ),
            Some(text) => Some(
                Path::new(text)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| text.to_owned()),
            ),
        };
        let title = name.map_or_else(
            || "Vestra Player".to_owned(),
            |name| format!("{name} - Vestra Player"),
        );
        if title != self.window_title {
            self.window_title = title;
            if let Some(shell) = &self.shell {
                let _ = shell.set_property(
                    "window-title",
                    SValue::String(self.window_title.as_str().into()),
                );
            }
        }
    }
    /// Keep the sleep assertion in line with what is playing: ready and not
    /// paused (media or stream), video when a frame arrived within a second.
    fn power_tick(&mut self) {
        let source = &self.last_source;
        let playback = power::Playback {
            playing: source["phase"] == "ready" && source["paused"] == false,
            video: self
                .last_frame
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1)),
        };
        if let Some(error) = self.power.update(playback) {
            diag!("host.error power {error}");
        }
    }
    /// `window.setAlwaysOnTop`: every ordinary window of the application at
    /// the floating level, applied again when windows open or close.
    fn apply_on_top(&mut self) {
        if self.headless {
            return;
        }
        let want = self.on_top.get();
        let count = platform::window_count();
        if self.on_top_applied == Some((want, count)) {
            return;
        }
        match platform::set_floating(want) {
            Ok(count) => self.on_top_applied = Some((want, count)),
            Err(error) => {
                diag!("host.error window level: {error}");
                self.on_top_applied = Some((want, count));
            }
        }
    }
    /// `{ fullscreen, videoArea, pictureInPicture }` of the main window
    /// (zero without one), with the palette and the file resolvers'
    /// extensions (`plugin.resolvers.fileTypes`).
    fn window_state(&self) -> Value {
        let facts = &self.window_facts;
        json!({"fullscreen":facts["fullscreen"] == true,
            "videoArea":{"width":facts["videoArea"]["width"].as_f64().unwrap_or(0.),
                "height":facts["videoArea"]["height"].as_f64().unwrap_or(0.)},
            "pictureInPicture":self.pip_state(),
            "appearance":self.theme(),"fileTypes":self.file_types,
            "language":self.language_preference().unwrap_or_else(|| i18n::SYSTEM.into()),
            "locale":i18n::locale()})
    }
    /// `WindowState.pictureInPicture`: offered here, and the window showing.
    fn pip_state(&self) -> Value {
        let facts = self.pip_facts();
        json!({"supported":facts.window && facts.supported,"active":self.pip.active()})
    }
    /// `window.setVideoArea { width, height }`: resize the window so the video
    /// column gets that size (bars added, fitted to the screen). Answered when
    /// the layout shows it, or after a second.
    fn set_video_area(
        &mut self,
        reply: &Reply,
        params: &Value,
    ) -> std::result::Result<(), RpcError> {
        let (width, height) = (
            params["width"].as_f64().unwrap_or(f64::NAN),
            params["height"].as_f64().unwrap_or(f64::NAN),
        );
        if !window_control::valid_size(width, height) {
            return Err(RpcError::invalid("width and height must be 1..16384"));
        }
        let unavailable = |message: &str| RpcError::new("unavailable", message);
        let shell = self
            .shell
            .as_ref()
            .ok_or_else(|| unavailable("the video area needs a native window"))?;
        if shell.window().is_fullscreen() {
            return Err(unavailable(&i18n::tr("Not available in fullscreen")));
        }
        ui::settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name) {
            Ok(SValue::Number(n)) if n.is_finite() => n,
            _ => 0.,
        };
        let id = self
            .native_window
            .ok_or_else(|| unavailable("the native window is not known"))?;
        let screen = platform::screen_for(id).map_err(|e| unavailable(e.0))?;
        let sidebar = &self.window_facts["slots"]["sidebar"];
        let docked = sidebar["preferred"] == true && sidebar["detached"] == false;
        let frame = window_control::Frame {
            extra_height: (number("logical-height") - number("video-height")).max(0.),
            extra_width: if docked {
                self.policy.sidebar_user_width() + window_state::SPLITTER_WIDTH
            } else {
                0.
            },
            max_content: platform::max_content(&screen),
        };
        let fit = window_control::fit(width, height, frame);
        platform::resize_content(id, fit.content.0, fit.content.1).map_err(|e| unavailable(e.0))?;
        self.area_replies.push(AreaReply {
            reply: reply.clone(),
            video: fit.video,
            result: json!({"width":fit.video.0,"height":fit.video.1,"clamped":fit.clamped}),
            deadline: self.now_ms() + 1000,
        });
        Ok(())
    }
    fn answer_video_area(&mut self, video: (f64, f64), now: u64) {
        let (done, waiting): (Vec<_>, Vec<_>) = self.area_replies.drain(..).partition(|pending| {
            now >= pending.deadline
                || ((video.0 - pending.video.0).abs() <= 1.
                    && (video.1 - pending.video.1).abs() <= 1.)
        });
        self.area_replies = waiting;
        for pending in done {
            self.respond(pending.reply, Ok(pending.result));
        }
    }
    fn show_info_panels(&self) {
        if let Some(shell) = &self.shell
            && let Ok(value) =
                ui_values::from_json(&json!(self.info_panels.values().collect::<Vec<_>>()))
        {
            let _ = shell.set_property("info-panels", value);
        }
    }
    /// Current layout of both positions, or `pending` until the shell is measured.
    fn layout_state(&self) -> std::result::Result<Value, RpcError> {
        if self.window_facts["shellGeneration"].as_u64() != Some(self.shell_generation) {
            return Err(RpcError::new(
                "pending",
                "current shell layout is not yet confirmed",
            ));
        }
        // `panels`: the slot's panels in tab order (sidebar tabs); `owner`
        // is the current one's.
        let bottom = self
            .mounts
            .values()
            .filter(|m| m.spec.slot == composition::Slot::Bottom)
            .map(|m| m.spec.owner)
            .collect::<Vec<_>>();
        let sidebar = self.policy.sidebar_panels();
        let slot = |name: &str| {
            let slot = &self.window_facts["slots"][name];
            if slot.is_null() {
                return Value::Null;
            }
            let owners = if name == "sidebar" { &sidebar } else { &bottom };
            let panels = owners
                .iter()
                .filter_map(|owner| self.mounts.get(owner))
                .map(|m| json!({"owner":m.spec.owner,"plugin":m.manifest,"title":m.title}))
                .collect::<Vec<_>>();
            json!({"owner":slot["owner"],"preferred":slot["preferred"],"effective":slot["effective"],
                "reason":slot["reason"],"detached":slot["detached"],"panels":panels})
        };
        Ok(
            json!({"bottom":slot("bottom"),"sidebar":slot("sidebar"),"backdrop":self.backdrop_state()}),
        )
    }
    /// `panels.backdrop.state`: `{ owner, effective }`, null without a backdrop.
    fn backdrop_state(&self) -> Value {
        self.mounts
            .values()
            .find(|m| m.spec.slot == composition::Slot::Backdrop)
            .map_or(
                Value::Null,
                |m| json!({"owner":m.spec.owner,"effective":self.no_video}),
            )
    }
    fn answer(&mut self, reply: Reply, method: &str, params: &Value) {
        match self.dispatch(&reply, method, params) {
            Ok(None) => (),
            Ok(Some(value)) => self.respond(reply, Ok(value)),
            Err(error) => {
                if debug_keys() && method == "ui.menu" {
                    diag!("keys: ui.menu refused {}: {}", error.code, error.message);
                }
                self.respond(reply, Err(error))
            }
        }
    }
    /// Mount requests held while the built-ins start, answered in order once they have.
    fn release_held_mounts(&mut self) {
        if self.held_mounts.is_empty() {
            return;
        }
        let starting = self.builtins_starting();
        for (reply, method, params) in self.held_mounts.release(starting) {
            if self.plugins.contains_key(&reply.owner) {
                self.answer(reply, &method, &params);
            }
        }
    }
    /// The manifest id of a running plugin (its owner number otherwise).
    fn plugin_id(&self, owner: u64) -> String {
        self.plugins
            .get(&owner)
            .map_or_else(|| owner.to_string(), |p| p.manifest.id.clone())
    }
    /// Refuse what another plugin holds (`RpcError::taken`): recorded for
    /// A sentence of a plugin's (`osd.show`, dialogs) in the interface
    /// language: a template translated with its catalog and formatted with
    /// `args`, or a text value (`i18n::text`).
    fn plugin_sentence(
        &self,
        owner: u64,
        text: &Value,
        args: &Value,
    ) -> std::result::Result<String, RpcError> {
        let texts = self.plugins.get(&owner).map_or(
            i18n::text::Texts {
                catalog: None,
                locale: i18n::locale(),
            },
            plugins::Plugin::texts,
        );
        texts.sentence(text, args).map_err(RpcError::invalid)
    }
    /// `packages.list`, shown on the OSD and announced as `host.error`.
    fn refuse_taken(&mut self, owner: u64, error: RpcError, text: String) -> RpcError {
        let id = self.plugin_id(owner);
        let mut warning = Value::Object(error.detail.clone().unwrap_or_default());
        if let Some(reason) = warning.as_object_mut().and_then(|w| w.remove("reason")) {
            warning["code"] = reason;
        }
        let warnings = self.warnings.entry(owner).or_default();
        warnings.retain(|(w, _)| w["code"] != warning["code"] || w["slot"] != warning["slot"]);
        warnings.push((warning.clone(), text.clone()));
        diag!("{id}: {}", error.message);
        self.show_osd(&i18n::tr_args("{}: {}", &json!([id, text])));
        warning["plugin"] = id.into();
        warning["message"] = error.message.clone().into();
        self.broadcast("host.error", warning);
        error
    }
    fn clear_warning(&mut self, owner: u64, code: &str) {
        if let Some(warnings) = self.warnings.get_mut(&owner) {
            warnings.retain(|(w, _)| w["code"] != code);
        }
    }
    /// `packages.list` `warnings` of a running plugin.
    fn warnings_of(&self, owner: u64) -> Vec<Value> {
        self.warnings
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|(warning, _)| warning.clone())
            .collect()
    }
    /// The same warnings as text: 「面板未显示：侧栏已被 vestra.playlist 使用」.
    fn warning_text(&self, owner: u64) -> Option<String> {
        let text = self
            .warnings
            .get(&owner)?
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        (!text.is_empty()).then(|| text.join(&i18n::tr("; ")))
    }
    fn respond(&mut self, reply: Reply, mut result: std::result::Result<Value, RpcError>) {
        // The backend names the holder of a taken resolver by owner number.
        if let Err(error) = &mut result
            && let Some(detail) = error.detail.as_mut()
            && detail.get("reason").and_then(Value::as_str) == Some("resolver-taken")
            && let Some(holder) = detail.remove("holder").and_then(|h| h.as_u64())
        {
            let target = detail
                .remove("target")
                .and_then(|t| t.as_str().map(str::to_owned))
                .unwrap_or_default();
            let id = self.plugin_id(holder);
            detail.insert("owner".into(), id.clone().into());
            detail.insert("target".into(), target.clone().into());
            error.message = i18n::tr_args("{} is resolved by {}", &json!([target, id]));
            let text = i18n::tr_args("{} not taken over: resolved by {}", &json!([target, id]));
            let refusal = error.clone();
            result = Err(self.refuse_taken(reply.owner, refusal, text));
        }
        if self.activation_ui.remove(&(reply.owner, reply.id)) {
            let outcome = activation_outcome(&result);
            if let Err(error) =
                self.activations
                    .ui_finish(reply.owner, reply.id, outcome, self.now_ms())
            {
                diag!("activation UI completion: {error}");
            }
        }
        let message = match result {
            Ok(value) => json!({"id":reply.id,"result":value}),
            Err(error) => json!({"id":reply.id,"error":error}),
        };
        self.send(reply.owner, message);
    }
    /// One request of a plugin's: its route (`capabilities::lookup`), the
    /// permission check, text resolution, then the capability's `handle`.
    fn dispatch(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let plugin = self
            .plugins
            .get(&reply.owner)
            .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
        let route = capabilities::lookup(method)
            .ok_or_else(|| RpcError::new("unsupported", "unknown method"))?;
        let capability = route.needs(params, |name| plugin.allows(name));
        if capability.is_some_and(|capability| !plugin.allows(capability)) {
            return Err(RpcError::new(
                "capability",
                "capability must be declared in manifest",
            ));
        }
        // Text values (i18n.md §7) in property values, timeline titles,
        // subtitle track titles and resolver failures become the strings
        // they show, in this plugin's catalog, before anything reads them (so
        // the value and length limits count what is shown).
        let resolved;
        let params = if (matches!(
            method,
            "ui.open"
                | "ui.set"
                | "ui.mount"
                | "ui.setMount"
                | "settings.register"
                | "settings.set"
                | "timeline.set"
                | "subtitles.commit"
        ) && i18n::text::contains(params))
            || (method == "sourceResolvers.respond" && i18n::text::is_text(&params["error"]))
        {
            let mut copy = params.clone();
            plugin
                .texts()
                .resolve(&mut copy)
                .map_err(RpcError::invalid)?;
            resolved = copy;
            &resolved
        } else {
            params
        };
        route.handle(self, reply, method, params)
    }
}
/// Initial properties of a panel or settings page are converted when the
/// view is built. A value over the limits is left out now (the property
/// keeps its default) and returned, to be reported (`report_refused`),
/// rather than failing the mount: a mount failing during activation would
/// fail the whole plugin.
fn refuse_initial(properties: &mut BTreeMap<String, Value>) -> Vec<RpcError> {
    let mut refused = Vec::new();
    properties.retain(|name, value| match RpcError::check_property(name, value) {
        Ok(()) => true,
        Err(error) => {
            refused.push(error);
            false
        }
    });
    refused
}
/// A `ui.error` event: a failure the plugin did not ask about (a property
/// the player could not show when it rebuilt a view, a callback whose
/// arguments it could not convert). The SDK reports it to `onError`.
fn ui_error(error: &RpcError) -> Value {
    json!({"event":"ui.error","data":{"code":error.code,"message":error.message}})
}
/// The window policy's slot (the backdrop has no preference or auto-hide).
/// What an initial UI request means for activation. A slot another plugin
/// holds does not fail it: the plugin runs without its panel, with a warning
/// (`packages.list`).
fn activation_outcome(
    result: &std::result::Result<Value, RpcError>,
) -> std::result::Result<(), String> {
    match result {
        Err(error) if !error.is_taken() => Err(format!("{}: {}", error.code, error.message)
            .chars()
            .take(512)
            .collect()),
        _ => Ok(()),
    }
}
/// What a `layout.set` changes: the preference, floating, the current tab.
#[derive(Debug, PartialEq)]
struct LayoutChange {
    visible: Option<bool>,
    detached: Option<bool>,
    /// The owner of the panel whose tab becomes current.
    active: Option<u64>,
}

/// Read `layout.set`'s `visible`, `detached` and `active` (a plugin id among
/// the slot's `panels`: owner, manifest id) for `slot`.
fn layout_change(
    params: &Value,
    slot: composition::Slot,
    panels: &[(u64, String)],
) -> std::result::Result<LayoutChange, RpcError> {
    let flag = |key: &str| -> std::result::Result<Option<bool>, RpcError> {
        match params.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(value) => value
                .as_bool()
                .map(Some)
                .ok_or_else(|| RpcError::invalid(format!("{key} boolean required"))),
        }
    };
    let (visible, detached) = (flag("visible")?, flag("detached")?);
    if detached.is_some() && slot == composition::Slot::Bottom {
        return Err(RpcError::invalid("the bottom bar cannot be detached"));
    }
    let active = match params.get("active").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::String(id)) => Some(
            panels
                .iter()
                .find(|(_, manifest)| manifest == id)
                .map(|(owner, _)| *owner)
                .ok_or_else(|| {
                    RpcError::new("missing", format!("{id} has no panel at this position"))
                })?,
        ),
        Some(_) => return Err(RpcError::invalid("active requires a plugin id")),
    };
    Ok(LayoutChange {
        visible,
        detached,
        active,
    })
}

fn policy_slot(slot: composition::Slot) -> Option<window_state::Slot> {
    match slot {
        composition::Slot::Bottom => Some(window_state::Slot::Bottom),
        composition::Slot::Sidebar => Some(window_state::Slot::Sidebar),
        composition::Slot::Backdrop => None,
    }
}
/// Projects the open chain at the origin the view recorded for this opening.
fn menu_levels(
    window: &slint_interpreter::ComponentInstance,
    roots: &[commands::MenuNode],
    path: &mut Vec<String>,
    scroll: &[f64],
    labels: &dyn Fn(u64, &str) -> Option<String>,
) -> Result<(slint_interpreter::Value, Vec<menu::Rect>)> {
    *path = menu::valid_path(roots, path);
    let (origin, size) = menu_frame(window);
    let value = menu::levels(roots, path, origin, size, scroll, labels);
    if debug_menu() {
        let panels: Vec<String> = value
            .as_array()
            .into_iter()
            .flatten()
            .map(|level| {
                let number = |key: &str| level[key].as_f64().unwrap_or(f64::NAN);
                format!(
                    "x {:.0}..{:.0} y {:.0}..{:.0}",
                    number("x"),
                    number("x") + number("width"),
                    number("y"),
                    number("y") + number("height")
                )
            })
            .collect();
        diag!(
            "menu: open={path:?} window={:.0}x{:.0} panels=[{}]",
            size.x,
            size.y,
            panels.join("; ")
        );
    }
    Ok((ui_values::from_json(&value)?, menu::panels(&value)))
}
/// Where the menu opens and the window it must fit in, in logical px.
fn menu_frame(window: &slint_interpreter::ComponentInstance) -> (menu::Point, menu::Point) {
    let number = |name: &str| match window.get_property(name) {
        Ok(slint_interpreter::Value::Number(value)) if value.is_finite() => value,
        _ => 0.,
    };
    (
        menu::Point {
            x: number("menu-origin-x"),
            y: number("menu-origin-y"),
        },
        menu::Point {
            x: number("logical-width"),
            y: number("logical-height"),
        },
    )
}
/// Slint reports ⌘ as `control` and ⌃ as `meta` on Apple platforms.
fn slint_mods(control: bool, meta: bool) -> (bool, bool) {
    if platform::SLINT_CONTROL_IS_COMMAND {
        (meta, control)
    } else {
        (control, meta)
    }
}
/// Record a key press's physical key for the `key-input` callback that Slint
/// runs right after this hook. Returns the input of a dead key with Option
/// (no text, so Slint never reports it) and whether it is an auto-repeat.
fn observe_key(
    input: &Rc<RefCell<InputShared>>,
    modifiers: &std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
    event: &slint::winit_030::winit::event::WindowEvent,
) -> Option<(bindings::Input, bool)> {
    use slint::winit_030::winit::{
        event::{ElementState, WindowEvent},
        keyboard::{Key, PhysicalKey},
        platform::modifier_supplement::KeyEventExtModifierSupplement,
    };
    match event {
        WindowEvent::ModifiersChanged(state) => {
            // winit sends this from the key event's own flags too (before the
            // KeyboardInput), so it is right even without modifier key events.
            let (before, after) = (modifiers.get(), state.state());
            modifiers.set(after);
            // Slint follows modifiers only through their own key events; a
            // release it never saw (synthesized input) would leave Option or
            // Shift held for later pointer and key events. Release them.
            let mut shared = input.borrow_mut();
            for (was, now, keys) in [
                (before.alt_key(), after.alt_key(), ['\u{12}', '\u{13}']),
                (before.shift_key(), after.shift_key(), ['\u{10}', '\u{15}']),
                (
                    before.control_key(),
                    after.control_key(),
                    ['\u{11}', '\u{16}'],
                ),
                (before.super_key(), after.super_key(), ['\u{17}', '\u{18}']),
            ] {
                if was && !now {
                    shared.released.extend(keys);
                }
            }
            if debug_keys() {
                diag!("keys: winit modifiers {:?}", state.state());
            }
            None
        }
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => {
            if debug_keys() {
                diag!(
                    "keys: winit {:?} synthetic={is_synthetic} physical={:?} logical={:?} text={:?} repeat={} modifiers={:?}",
                    event.state,
                    event.physical_key,
                    event.logical_key,
                    event.text,
                    event.repeat,
                    modifiers.get()
                );
            }
            if *is_synthetic || event.state != ElementState::Pressed {
                return None;
            }
            if !event.repeat {
                input_trace::begin(&format!("winit key {:?} down", event.physical_key));
            }
            let PhysicalKey::Code(code) = event.physical_key else {
                input.borrow_mut().physical = None;
                return None;
            };
            let base = match event.key_without_modifiers() {
                Key::Character(text) if text.chars().count() == 1 => text.chars().next(),
                _ => None,
            };
            let state = modifiers.get();
            let physical = bindings::Physical {
                code: format!("{code:?}"),
                base,
                mods: Some(bindings::Mods {
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    shift: state.shift_key(),
                    cmd: state.super_key(),
                }),
            };
            let dead = (matches!(event.logical_key, Key::Dead(_)) && state.alt_key())
                .then(|| bindings::from_key("", false, false, false, false, Some(&physical)))
                .flatten();
            if debug_keys() && matches!(event.logical_key, Key::Dead(_)) {
                diag!("keys: dead key → {dead:?}");
            }
            input.borrow_mut().physical = Some(physical);
            dead.map(|input| (input, event.repeat))
        }
        WindowEvent::MouseInput { state, button, .. } => {
            match state {
                ElementState::Pressed => {
                    input_trace::begin(&format!("winit mouse {button:?} down"))
                }
                ElementState::Released => input_trace::mark(&format!("winit mouse {button:?} up")),
            }
            None
        }
        _ => None,
    }
}
/// Option + a named key (⌥⌦, ⌥←): macOS delivers it in a form Slint does not
/// route to the focused item, and its input handling then stalls until the
/// next ordinary key. Such a press and release are kept from Slint and handed
/// to it again as plain key events from the host loop, with the press's
/// physical key and window modifiers recorded, so the binding resolves in the
/// focused scope (`Alt+Delete` in the playlist panel) like any other key.
fn inject_option_key(
    input: &Rc<RefCell<InputShared>>,
    modifiers: &std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
    events: &mpsc::Sender<Event>,
    generation: u64,
    event: &slint::winit_030::winit::event::WindowEvent,
) -> bool {
    use slint::winit_030::winit::{
        event::{ElementState, WindowEvent},
        keyboard::PhysicalKey,
    };
    let WindowEvent::KeyboardInput {
        event,
        is_synthetic: false,
        ..
    } = event
    else {
        return false;
    };
    let PhysicalKey::Code(code) = event.physical_key else {
        return false;
    };
    // ⌥Space types a character (no-break space): Slint gets it as it is.
    let code = format!("{code:?}");
    let Some(key) = bindings::slint_key(&code).filter(|_| code != "Space") else {
        return false;
    };
    if !modifiers.get().alt_key() {
        return false;
    }
    let pressed = event.state == ElementState::Pressed;
    let physical = if pressed {
        input.borrow_mut().physical.take()
    } else {
        None
    };
    let kind = match (pressed, event.repeat) {
        (false, _) => Injected::Released,
        (true, false) => Injected::Pressed,
        (true, true) => Injected::Repeated,
    };
    let _ = events.send(Event::InjectKey(generation, key, kind, physical));
    true
}
/// Hand modifier releases Slint missed to the window, outside winit's dispatch.
fn release_modifiers(
    input: &Rc<RefCell<InputShared>>,
    events: &mpsc::Sender<Event>,
    generation: u64,
) {
    let released = std::mem::take(&mut input.borrow_mut().released);
    if !released.is_empty() {
        let _ = events.send(Event::ModifiersReleased(generation, released));
    }
}
/// The system double-click interval (0.5 s where the system has none).
fn double_click_interval() -> Duration {
    let seconds = platform::double_click_seconds();
    Duration::from_secs_f64(if (0.05..=5.).contains(&seconds) {
        seconds
    } else {
        0.5
    })
}
/// `VESTRA_DEBUG_KEYS=1`: log every key event and how it resolved to stderr.
/// `VESTRA_DEBUG_MENU=1`: the context menu's hovers, invocations and panel
/// geometry on stderr.
fn debug_menu() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_MENU").is_some_and(|v| v == "1"))
}
/// `VESTRA_DEBUG_MENU=1`: the pointer as Slint receives it (logical px in
/// the window's content), so a log shows which row a move or press hits.
/// Record where the pointer is in this window (`InputShared::pointer`).
fn track_pointer(
    input: &Rc<RefCell<InputShared>>,
    generation: u64,
    window: &slint::Window,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::WindowEvent;
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let scale = f64::from(window.scale_factor());
            let point = menu::Point {
                x: position.x / scale,
                y: position.y / scale,
            };
            input.borrow_mut().pointer = Some((generation, point));
        }
        WindowEvent::CursorLeft { .. } => {
            let mut shared = input.borrow_mut();
            if shared.pointer.is_some_and(|(owner, _)| owner == generation) {
                shared.pointer = None;
            }
        }
        _ => {}
    }
}
fn debug_menu_pointer(
    window: &slint::Window,
    cursor: &std::cell::Cell<(f64, f64)>,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::WindowEvent;
    let scale = f64::from(window.scale_factor());
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let (x, y) = (position.x / scale, position.y / scale);
            cursor.set((x, y));
            diag!("menu: pointer moved x={x:.1} y={y:.1}");
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let (x, y) = cursor.get();
            diag!("menu: pointer {state:?} {button:?} at x={x:.1} y={y:.1}");
        }
        _ => {}
    }
}
fn debug_keys() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_KEYS").is_some_and(|v| v == "1"))
}
/// Resolve a bound input on the UI thread; true = bound (Slint stops propagating).
/// Auto-repeats fire only commands that declared `repeat`.
fn fire(
    input: &Rc<RefCell<InputShared>>,
    events: &mpsc::Sender<Event>,
    generation: u64,
    event: &bindings::Input,
    repeat: bool,
    panel: Option<u64>,
) -> bool {
    let shared = input.borrow();
    if shared.blocked {
        return false;
    }
    let target = match shared.table.lookup(event, panel) {
        bindings::Lookup::Fire(target) => target,
        // Only disabled commands hold it: one with a reason says why on the
        // OSD (the host loop's execution reports it); otherwise the key passes.
        bindings::Lookup::Disabled {
            target,
            reason: Some(_),
        } => {
            if !repeat {
                input_trace::mark(&format!(
                    "disabled {} → {}/{}",
                    bindings::text(event),
                    target.owner,
                    target.id
                ));
                let _ = events.send(Event::Binding(generation, target.owner, target.id.clone()));
            }
            return true;
        }
        bindings::Lookup::Disabled { reason: None, .. } | bindings::Lookup::Unbound => {
            return false;
        }
    };
    if !repeat {
        input_trace::mark(&format!(
            "fire {} → {}/{}",
            bindings::text(event),
            target.owner,
            target.id
        ));
    }
    if !repeat || target.repeat {
        let _ = events.send(Event::Binding(generation, target.owner, target.id.clone()));
    }
    true
}
/// The producer capture lets keys bound to host commands through (winit key code names).
#[cfg(feature = "producer-audio")]
fn host_key(input: &Rc<RefCell<InputShared>>, code: &str) -> bool {
    let name = match code {
        "Escape" | "Enter" | "Tab" | "Space" => code.to_owned(),
        "NumpadEnter" => "Enter".into(),
        code if code.starts_with('F') && code[1..].parse::<u8>().is_ok() => code.to_owned(),
        _ => return false,
    };
    input.borrow().table.host_key(&bindings::Input {
        mods: bindings::Mods::default(),
        name,
    })
}
/// Registry revisions include changes outside the open levels and identical
/// state publications. Reconcile in place at every depth so the rows keep
/// their live elements: a state published while a row is pressed must not
/// cancel the click, nor a switched submenu the hover of its parent's rows.
fn set_menu_model(
    window: &slint_interpreter::ComponentInstance,
    next: slint_interpreter::Value,
) -> Result<()> {
    let current = window.get_property("menu-levels")?;
    if !ui_values::reconcile_nested(&current, &next)? {
        window.set_property("menu-levels", next)?;
    }
    Ok(())
}
fn string<'a>(params: &'a Value, key: &str) -> std::result::Result<&'a str, RpcError> {
    params[key]
        .as_str()
        .ok_or_else(|| RpcError::invalid(format!("{key} requires string")))
}
thread_local! {
    /// The UI thread's frame delivery, run by the renderer's wake-up.
    static FRAME_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// The UI thread's delivery of GPU frames to picture-in-picture.
    static PIP_GPU_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}
/// Opt-in bounded fixture diagnostics, not a plugin frame API or source/PTS fence.
struct FrameEvidence {
    directory: PathBuf,
    index: std::fs::File,
    count: usize,
    last: Instant,
}
impl FrameEvidence {
    fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let index = std::fs::File::create(directory.join("frames.jsonl"))?;
        Ok(Self {
            directory,
            index,
            count: 0,
            last: Instant::now() - Duration::from_secs(1),
        })
    }
    fn sample(&mut self, backend: &Backend, source: &Value, start: Instant) -> Result<()> {
        use std::io::Write;
        if self.count >= 64 || self.last.elapsed() < Duration::from_millis(200) {
            return Ok(());
        }
        let frame = backend.frame.peek(|f| {
            (
                f.sequence,
                f.buffer.as_bytes().to_vec(),
                f.buffer.width(),
                f.buffer.height(),
            )
        });
        if let Some((sequence, pixels, width, height)) = frame {
            let name = format!("frame-{:03}.ppm", self.count);
            let mut file = std::fs::File::create(self.directory.join(&name))?;
            write!(file, "P6\n{width} {height}\n255\n")?;
            file.write_all(&pixels)?;
            writeln!(
                self.index,
                "{}",
                json!({"file":name,"softwareSequence":sequence,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"observedSource":source,"frameSourceAssociation":"none"})
            )?;
            self.index.flush()?;
            self.count += 1;
            self.last = Instant::now();
        }
        Ok(())
    }
}
/// Whether this player still takes a later launch's files (`Primary::serve`).
/// Cleared when quitting starts (Quit, or the event loop ending after the last
/// window closed): a launch during the player's last moments is refused, keeps
/// trying, and becomes the next player once this process exits and frees the
/// lock. If the teardown takes longer than the launch waits (5 s), the launch
/// runs on its own without the lock (`single_instance`'s error path).
static HANDOFFS: AtomicBool = AtomicBool::new(true);

fn stop_handoffs() {
    HANDOFFS.store(false, Ordering::Release);
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let launch::Launch {
        directories,
        headless,
        silent,
        seconds,
        snapshot,
        frame_evidence,
        files,
        urls,
    } = launch::parse(&args, &std::env::current_dir()?)?;
    for url in &urls {
        diag!("host.warning open: {url} is not a file; open links from a plugin's command");
    }
    // One player per user (`platform::single_instance`): a later launch
    // hands its files to the running one and exits. Development runs
    // (`--plugin`) and headless ones stay on their own unless
    // `VESTRA_INSTANCE_DIR` names the directory to coordinate in (tests).
    let instance_dir = std::env::var_os("VESTRA_INSTANCE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            (!headless && directories.is_empty())
                .then(platform::app_data_dir)
                .flatten()
        });
    let mut primary = None;
    if let Some(directory) = instance_dir {
        match platform::single_instance(&directory, &files, Duration::from_secs(5)) {
            Ok(platform::Launch::HandedOff) => return Ok(()),
            Ok(platform::Launch::Primary(claimed)) => primary = Some(claimed),
            Err(error) => diag!("single instance: {error}; running on its own"),
        }
    }
    let mut evidence = frame_evidence.map(FrameEvidence::new).transpose()?;
    // A packaged player missing its own files (the bundled Node above all)
    // stops here, not with every plugin failing on its own.
    let missing = runtime_paths::get().missing(std::path::Path::is_file);
    if !missing.is_empty() {
        let text = format!(
            "This installation of Vestra Player is incomplete; reinstall it.\n\n{}",
            missing.join("\n")
        );
        if !headless {
            rfd::MessageDialog::new()
                .set_title("Vestra Player")
                .set_description(&text)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        bail!("{text}");
    }
    // GPU rendering (SDK site, Concepts: render options) needs Slint's OpenGL renderer (femtovg);
    // headless, `VESTRA_SOFTWARE_RENDER=1` and other systems render in software.
    let mut gpu = renderer::wanted(
        headless,
        std::env::var_os("VESTRA_SOFTWARE_RENDER").as_deref(),
    );
    if !headless {
        // Windows: the main window draws at the display's vertical blanks
        // (`frame_pacing.rs`), not on Slint's timer.
        let winit = || {
            let selector = slint::BackendSelector::new().backend_name("winit".into());
            if frame_pacing::wanted() {
                selector.with_winit_custom_application_handler(frame_pacing::Handler)
            } else {
                selector
            }
        };
        if gpu && let Err(error) = winit().renderer_name("femtovg".into()).select() {
            diag!(
                "render: the OpenGL window renderer is unavailable ({error}); software rendering"
            );
            gpu = false;
            winit().select()?;
        } else if !gpu {
            winit().select()?;
        }
    }
    let mut observed_source = Value::Null;
    let (events, receiver) = mpsc::channel();
    if !headless {
        // Before the event loop: the launch's own open-documents event (macOS).
        let events = events.clone();
        platform::on_open_files(move |files| {
            let _ = events.send(Event::OpenFiles(files, false));
        });
    }
    if let Some(primary) = primary {
        let events = events.clone();
        primary.serve(move |files| {
            HANDOFFS.load(Ordering::Acquire) && events.send(Event::OpenFiles(files, true)).is_ok()
        });
    }
    let backend = Backend::start(events.clone(), silent, gpu)?;
    // `plugin.app`, handed to every plugin process when it starts.
    let ffmpeg = &backend.versions["ffmpeg"];
    plugins::set_app_info(&json!({"version":env!("CARGO_PKG_VERSION"),
        "build":option_env!("VESTRA_BUILD"),
        "components":{"mpv":backend.versions["mpv"],
            "libav":if ffmpeg.is_string() { json!({"ffmpeg":ffmpeg}) } else { json!({}) },
            "slint":env!("VESTRA_SLINT_VERSION")}}));
    let (mount_events, mount_input) = mpsc::sync_channel(64);
    let mut host = Host {
        backend,
        shared: None,
        shared_policy: Default::default(),
        plugins: BTreeMap::new(),
        views: BTreeMap::new(),
        events,
        next_generation: 0,
        headless,
        dialog_busy: false,
        shell: None,
        mounts: BTreeMap::new(),
        mount_queue: std::collections::VecDeque::new(),
        preparing: None,
        mount_singly: 0,
        mount_events,
        mount_input,
        last_image: None,
        video_texture: RefCell::new(None),
        shell_generation: 1,
        next_shell_generation: 1,
        menu_path: Vec::new(),
        menu_scroll: Vec::new(),
        menu_aim: menu::HoverAim::default(),
        menu_panels: Vec::new(),
        policy: window_state::Policy::new(0),
        clock: Instant::now(),
        window_facts: Value::Null,
        measured_shell: None,
        menu_open: false,
        sidebar_popped: false,
        sidebar_active: None,
        tab_clicked: false,
        shell_tabs: Vec::new(),
        bottom_shown: true,
        native_window: None,
        native_before: None,
        native_unavailable: None,
        input: Rc::new(RefCell::new(InputShared::default())),
        ui_state: if headless {
            ui_state::Store::disabled()
        } else {
            ui_state::Store::open(&ui_state::default_path())
        },
        osd_until: None,
        info_panels: BTreeMap::new(),
        sequence: None,
        last_source: Value::Null,
        source_state_replies: Default::default(),
        floating: None,
        menu_mode: MenuMode::Main,
        menu_generation: 1,
        host_revision: 0,
        window_title: "Vestra Player".into(),
        activations: package_activation::Book::default(),
        activation_ui: Default::default(),
        activation_receipts: Default::default(),
        file_types: json!([]),
        held_mounts: Default::default(),
        warnings: BTreeMap::new(),
        packages: package_manager::Manager::open()?,
        cleanup: Default::default(),
        power: power::Power::system(),
        last_frame: None,
        sizing: Default::default(),
        frame_log: render_target::FrameLog::from_env(),
        shown_size: None,
        actions_busy: Default::default(),
        step_seek: BTreeMap::new(),
        step_writes: 0,
        on_top: window_control::LastWrite::new(false),
        cursor_playing: window_control::LastWrite::new(false),
        on_top_applied: None,
        area_replies: Vec::new(),
        development: BTreeMap::new(),
        dev_disabled: Default::default(),
        failures: BTreeMap::new(),
        data_removals: BTreeMap::new(),
        session_storage: Default::default(),
        bindings_sent: 0,
        packages_sent: Value::Null,
        notified_at: 0,
        timeline: Default::default(),
        images: Default::default(),
        no_video: true,
        video_absent: true,
        video_cleared: false,
        cover_art_off: false,
        pip: Default::default(),
        pip_log: None,
        render_area: None,
        render_sent: None,
        pip_gpu: Default::default(),
        drops: Default::default(),
        opens: Default::default(),
        settings: Default::default(),
        settings_category: None,
        theme_pending: false,
        placements: ui::Placements::default(),
        bar: bar_buttons::Strip::default(),
        commands: commands::Registry::new(),
    };
    // The interface language of this run (doc/architecture/i18n.md §2):
    // `VESTRA_LANGUAGE` (development and tests), else the saved choice, else
    // the system's languages. Headless runs ignore the system: English
    // unless asked, so diagnostics read the same on every machine.
    let preference = std::env::var("VESTRA_LANGUAGE")
        .ok()
        .or_else(|| host.language_preference());
    let system = if headless {
        Vec::new()
    } else {
        platform::preferred_languages()
    };
    i18n::init(i18n::resolve(preference.as_deref(), &system));
    if !headless {
        // CJK fallback fonts for every window, before the first one exists
        // (`ui_fonts`), ordered for the interface language (Japanese fonts
        // first in `ja`, Korean in `ko`, Simplified Chinese otherwise).
        ui_fonts::install(i18n::locale());
    }
    host.register_host_commands()?;
    {
        if core_actions::migrate_overlay(&mut host.ui_state.state.bindings) {
            // Old playback-plugin keys now name the core actions; saved once.
            let now = host.now_ms();
            host.ui_state.touch(now);
            host.ui_state.flush();
        }
        let overlay = host.ui_state.state.bindings.clone();
        host.input.borrow_mut().table.set_overlay(overlay);
        if let Some(sidebar) = &host.ui_state.state.sidebar {
            host.policy.set_sidebar_width(sidebar.width);
        }
    }
    for directory in directories {
        host.load(&directory, 120_000)?;
    }
    host.open_files(files, false);
    let start = Instant::now();
    if headless {
        loop {
            if seconds.is_some_and(|seconds| start.elapsed().as_secs_f64() >= seconds) {
                break;
            }
            let received = receiver.recv_timeout(Duration::from_millis(10));
            // A tick is the work after the wait (`VESTRA_FRAME_LOG=1`).
            let began = Instant::now();
            match received {
                Ok(event) => {
                    if let Event::Broadcast(name, data) = &event
                        && name == "source.state"
                    {
                        observed_source = data.clone();
                    }
                    host.handle(event)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => break,
            }
            host.activation_tick();
            host.power_tick();
            host.package_tick();
            host.notify_tick();
            host.timeline_tick();
            host.open_files_tick();
            if let Some(log) = host.frame_log.as_mut() {
                log.tick_took(began.elapsed());
                let presentation = &host.backend.render.presentation;
                let (produced, shown) = host.backend.frame.counts();
                let (draws, presented) = presentation.counts();
                let counts = render_target::LogCounts {
                    produced,
                    shown,
                    draws,
                    presented,
                };
                if let Some(line) = log.line(counts, presentation, || None, None, Instant::now()) {
                    diag!("{line}{}", host.backend.frame.stages.text());
                }
            }
            if let Some(evidence) = evidence.as_mut() {
                evidence.sample(&host.backend, &observed_source, start)?;
            }
        }
        stop_handoffs();
    } else {
        let native_before = platform::capture_windows().ok();
        let window = composition::compile_json(&[], host.mount_events.clone(), 1)?;
        let position = host.ui_state.state.window.map(|frame| {
            window.window().set_size(slint::LogicalSize::new(
                frame.width.max(320.) as f32,
                frame.height.max(182.) as f32,
            ));
            slint::LogicalPosition::new(frame.x as f32, frame.y as f32)
        });
        let texture = host.attach_gpu(&window);
        // Title bars follow the palette from the first window on.
        platform::set_dark_title_bars(!theme::is_light(host.theme()));
        match position {
            Some(position) => host.placements.show_at(&window, position)?,
            None => window.show()?,
        }
        host.video_texture.replace(texture);
        platform::shown(&window);
        frame_pacing::follow(&window);
        // Interpolation (`video.render()`) resamples to the main display's rate.
        let displays = platform::displays();
        let main_display = displays.iter().find(|d| d.primary).or(displays.first());
        host.backend
            .render
            .set_refresh(main_display.and_then(|d| d.refresh_rate));
        host.native_before = native_before.clone();
        host.native_window = native_before
            .as_ref()
            .and_then(|before| platform::associate_new(before).ok());
        let host = Rc::new(RefCell::new(host));
        host.borrow().bind_shell(&window, 1)?;
        host.borrow_mut().theme_view(&window);
        host.borrow_mut().shell = Some(window.clone_strong());
        // Render cadence: each new frame wakes the event loop (coalesced)
        // instead of waiting for the 16 ms tick.
        let frames = host.borrow().backend.frame.clone();
        let frame_host = host.clone();
        let busy = frames.clone();
        FRAME_SINK.with(|sink| {
            *sink.borrow_mut() = Some(Box::new(move || match frame_host.try_borrow_mut() {
                Ok(mut host) => host.show_frame(),
                Err(_) => busy.miss(),
            }))
        });
        frame_pacing::on_deliver(Some(Box::new(|| {
            FRAME_SINK.with(|sink| {
                if let Some(deliver) = &*sink.borrow() {
                    deliver();
                }
            })
        })));
        let pip_host = host.clone();
        PIP_GPU_SINK.with(|sink| {
            *sink.borrow_mut() = Some(Box::new(move || {
                if let Ok(mut host) = pip_host.try_borrow_mut() {
                    host.pip_gpu_frame();
                }
            }))
        });
        frames.set_waker(Box::new(|| {
            let _ = slint::invoke_from_event_loop(|| {
                FRAME_SINK.with(|sink| {
                    if let Some(deliver) = &*sink.borrow() {
                        deliver();
                    }
                })
            });
        }));
        let tick_host = host.clone();
        let timer = slint::Timer::default();
        let mut menu_revision = u64::MAX;
        let mut binding_revision = u64::MAX;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let began = Instant::now();
                let mut host = tick_host.borrow_mut();
                for event in receiver.try_iter().take(128) {
                    host.handle(event);
                }
                host.placements.tick();
                host.mount_callbacks();
                host.prepare_mount_tick();
                host.window_tick();
                host.activation_tick();
                host.power_tick();
                host.package_tick();
                host.notify_tick();
                host.timeline_tick();
                host.drops_tick();
                host.open_files_tick();
                host.settings_tick();
                if host.theme_pending {
                    host.theme_all();
                }
                if host.commands.revision() != binding_revision {
                    binding_revision = host.commands.revision();
                    let disabled = host.commands.disabled();
                    host.input.borrow_mut().table.set_disabled(disabled);
                }
                if host.shell.is_some() {
                    let revision = host.commands.snapshot().revision;
                    if revision != menu_revision {
                        menu_revision = revision;
                        if let Err(error) = host.refresh_menu() {
                            host.broadcast(
                                "host.error",
                                json!({"code":"menu-projection","message":error.to_string()}),
                            );
                        }
                    }
                }
                // The renderer's wake-up delivers frames; this only catches
                // one whose wake-up found the host busy.
                if host.backend.frame.missed() {
                    host.show_frame();
                }
                frame_pacing::tick();
                if let Some(log) = host.frame_log.as_mut() {
                    log.tick_took(began.elapsed());
                }
                if seconds.is_some_and(|seconds| start.elapsed().as_secs_f64() >= seconds) {
                    if let Some(path) = snapshot.as_ref()
                        && let Some(window) = host.shell.as_ref()
                    {
                        let save = || -> Result<()> {
                            let pixels = window.window().take_snapshot()?;
                            let mut encoder = png::Encoder::new(
                                std::fs::File::create(path)?,
                                pixels.width(),
                                pixels.height(),
                            );
                            encoder.set_color(png::ColorType::Rgba);
                            encoder.set_depth(png::BitDepth::Eight);
                            encoder
                                .write_header()?
                                .write_image_data(pixels.as_bytes())?;
                            Ok(())
                        };
                        if let Err(error) = save() {
                            diag!("snapshot failed: {error}");
                        }
                    }
                    let _ = slint::quit_event_loop();
                }
            },
        );
        let ran = window.run();
        stop_handoffs();
        ran?;
        timer.stop();
        frame_pacing::stop();
        frame_pacing::on_deliver(None);
        FRAME_SINK.with(|sink| sink.borrow_mut().take());
        PIP_GPU_SINK.with(|sink| sink.borrow_mut().take());
        host.borrow_mut().ui_state.flush();
        // Native windows/resources must be released before process teardown.
        host.borrow_mut().close_floating();
        host.borrow_mut().views.clear();
        host.borrow_mut().shell = None;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_over_the_limits_is_limit_naming_the_property() {
        // D1: 3000 playlist rows of 8 fields; refused when stored, not later.
        let row = json!({"a":"1","b":"","c":"","d":"","e":false,"f":false,"g":false,"h":false});
        let rows = Value::Array(vec![row; 3000]);
        let error = RpcError::check_property("rows", &rows).unwrap_err();
        assert_eq!(error.code, "limit");
        assert!(error.message.starts_with("rows: UI "), "{}", error.message);
        assert!(RpcError::check_property("rows", &json!([1, 2])).is_ok());
        // Other conversion failures keep their code.
        let error = RpcError::check_property("x", &json!({"a_b": 1})).unwrap_err();
        assert_eq!(error.code, "host");
        assert_eq!(
            ui_error(&error),
            json!({"event":"ui.error","data":{"code":"host","message":error.message}})
        );
    }

    #[test]
    fn a_slot_refusal_is_a_conflict_naming_the_slot_and_its_holder() {
        let refusal = RpcError::slot_occupied(composition::Slot::Sidebar, "vestra.playlist");
        assert_eq!(
            serde_json::to_value(&refusal).unwrap(),
            json!({"code":"conflict","message":"The sidebar is used by vestra.playlist",
                "reason":"slot-occupied","slot":"sidebar","owner":"vestra.playlist"})
        );
        let plain = RpcError::new("conflict", "owner already mounted");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({"code":"conflict","message":"owner already mounted"})
        );
        let sequence = RpcError::sequence_taken("vestra.playlist");
        assert_eq!(
            serde_json::to_value(&sequence).unwrap(),
            json!({"code":"conflict","message":"Previous/Next is provided by vestra.playlist",
                "reason":"sequence-taken","owner":"vestra.playlist"})
        );
        assert!(refusal.is_taken() && sequence.is_taken() && !plain.is_taken());
    }

    #[test]
    fn only_a_slot_refusal_leaves_activation_to_succeed() {
        let refusal = RpcError::slot_occupied(composition::Slot::Bottom, "a.b");
        assert_eq!(activation_outcome(&Err(refusal)), Ok(()));
        let sequence = RpcError::sequence_taken("a.b");
        assert_eq!(activation_outcome(&Err(sequence)), Ok(()));
        // A file type or site another plugin resolves (source_resolvers).
        let resolver = RpcError::taken("resolver-taken", "taken".into(), json!({"holder":1}));
        assert_eq!(activation_outcome(&Err(resolver)), Ok(()));
        assert_eq!(activation_outcome(&Ok(Value::Null)), Ok(()));
        assert_eq!(
            activation_outcome(&Err(RpcError::new("ui-prepare", "syntax error"))),
            Err("ui-prepare: syntax error".into())
        );
        assert_eq!(
            activation_outcome(&Err(RpcError::new("conflict", "owner already mounted"))),
            Err("conflict: owner already mounted".into())
        );
    }

    #[test]
    fn layout_set_reads_the_current_tab_by_plugin_id() {
        use composition::Slot;
        let panels = [
            (4, "vestra.playlist".to_string()),
            (9, "examples.panel-menu".into()),
        ];
        assert_eq!(
            layout_change(
                &json!({"slot":"sidebar","active":"examples.panel-menu","visible":true}),
                Slot::Sidebar,
                &panels
            )
            .unwrap(),
            LayoutChange {
                visible: Some(true),
                detached: None,
                active: Some(9)
            }
        );
        assert_eq!(
            layout_change(
                &json!({"active":null,"detached":false}),
                Slot::Sidebar,
                &panels
            )
            .unwrap(),
            LayoutChange {
                visible: None,
                detached: Some(false),
                active: None
            }
        );
        let code = |params: Value, slot| layout_change(&params, slot, &panels).unwrap_err().code;
        assert_eq!(
            code(json!({"active":"other.plugin"}), Slot::Sidebar),
            "missing"
        );
        assert_eq!(code(json!({"active":7}), Slot::Sidebar), "invalid");
        assert_eq!(code(json!({"visible":"yes"}), Slot::Sidebar), "invalid");
        assert_eq!(code(json!({"detached":true}), Slot::Bottom), "invalid");
    }

    #[test]
    fn a_full_sidebar_is_refused_like_a_taken_slot() {
        let full = RpcError::sidebar_full();
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            json!({"code":"conflict","message":"The sidebar holds at most 8 tabs","reason":"slot-occupied","slot":"sidebar"})
        );
        assert_eq!(activation_outcome(&Err(full)), Ok(()));
    }
}
