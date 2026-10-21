#![forbid(unsafe_code)]
mod backend;
mod binding_manager;
mod bindings;
mod clipboard;
mod commands;
pub mod composition;
mod confirm;
mod core_actions;
mod drops;
mod effects;
pub mod font_asset;
pub mod font_carrier;
mod media_jobs;
mod media_tracks;
mod media_worker;
mod menu;
mod native_subtitle;
mod offline_audio;
mod offline_descriptor;
pub mod package_activation;
mod package_manager;
pub mod package_stage;
pub mod package_store;
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

#[cfg(feature = "producer-audio")]
pub mod producer_binary;
mod render_target;
mod renderer;
mod session;
mod settings_window;
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
mod ui_image;
mod ui_state;
mod ui_values;
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

#[derive(Serialize, Debug)]
struct RpcError {
    code: String,
    message: String,
}
impl RpcError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
}
impl From<anyhow::Error> for RpcError {
    fn from(value: anyhow::Error) -> Self {
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
    /// The options window: (generation, tree index to show, or `None` = close).
    Settings(u64, Option<usize>),
    RuntimeFault(u64, String),
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
    visible: bool,
    detached: bool,
    title: String,
    manifest: String,
}
/// The detached sidebar's own window (C14).
struct Floating {
    instance: slint_interpreter::ComponentInstance,
    owner: u64,
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
    click: Option<PendingClick>,
}
#[derive(Clone, Copy)]
struct PendingClick {
    at: Instant,
    x: f64,
    y: f64,
    mods: bindings::Mods,
    generation: u64,
}
/// A click moves at most this far (logical px) or it is a drag.
const CLICK_SLOP: f64 = 4.;
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
struct MountChange {
    reply: Reply,
    contribution: Option<Mount>,
}
struct Host {
    backend: Backend,
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
    mount_events: mpsc::SyncSender<composition::JsonCallback>,
    mount_input: mpsc::Receiver<composition::JsonCallback>,
    last_image: Option<slint::Image>,
    shell_generation: u64,
    next_shell_generation: u64,
    menu_path: Vec<String>,
    /// Requested scroll offset of each open menu level (G-03).
    menu_scroll: Vec<f64>,
    /// The submenu delay (G-15) and its length.
    menu_delay: menu::HoverDelay,
    menu_show_delay_ms: u64,
    policy: window_state::Policy,
    clock: Instant,
    window_facts: Value,
    measured_shell: Option<u64>,
    menu_open: bool,
    /// G-01: the fullscreen sidebar overlay was popped out at the last tick.
    sidebar_popped: bool,
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
    /// `player.setStepSeek` per owner: (write sequence, seconds); the newest wins.
    step_seek: BTreeMap<u64, (u64, f64)>,
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
    /// Settings pages and the options window (R-S1, R-H7).
    settings: settings_window::Settings,
    /// The category of the page the options window last showed.
    settings_category: Option<usize>,
    /// Vestra's `accent-fill` gradient, read from the first instance (R-H6).
    vestra_fill: Option<SValue>,
    /// A view missed the palette: deliver again on the next tick.
    theme_pending: bool,
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
                    eprintln!("activation status: {error}");
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
            eprintln!("host.error physical-input backend unavailable");
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
        let candidate: plugins::Manifest =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json"))?)?;
        if self.plugins.values().any(|p| p.manifest.id == candidate.id) {
            bail!("plugin already running: {}", candidate.id);
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
        if self.floating.as_ref().is_some_and(|f| f.owner == owner) {
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
            if let Some(shell) = &self.shell
                && let Some(property) = mount.spec.slot.visible_property()
            {
                let _ = shell.set_property(property, false.into());
            }
            self.mount_queue.push_front(MountChange {
                reply: Reply { owner, id: 0 },
                contribution: None,
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
        self.packages.owner_gone(owner);
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
            eprintln!("plugin transport: {error}");
            self.disable(owner);
        }
    }
    fn broadcast(&mut self, event: &str, data: Value) {
        if event == "host.error" {
            eprintln!("host.error {data}");
        }
        let mut failed = Vec::new();
        for (owner, plugin) in self
            .plugins
            .iter_mut()
            .filter(|(_, p)| p.subscriptions.contains(event))
        {
            if plugin.send(json!({"event":event,"data":data})).is_err() {
                failed.push(*owner);
            }
        }
        for owner in failed {
            self.disable(owner);
        }
    }
    fn handle(&mut self, event: Event) {
        match event {
            Event::Request(owner, message) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                if let Some(event) = message["event"].as_str() {
                    eprintln!("runtime {owner} {event}: {}", message["data"]);
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
                        eprintln!("activation runtime event: {error}");
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
                        match self.dispatch(&reply, &request.method, &request.params) {
                            Ok(None) => (),
                            Ok(Some(value)) => self.respond(reply, Ok(value)),
                            Err(error) => {
                                if debug_keys() && request.method == "ui.menu" {
                                    eprintln!(
                                        "keys: ui.menu refused {}: {}",
                                        error.code, error.message
                                    );
                                }
                                self.respond(reply, Err(error))
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("malformed request: {error}");
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
                self.policy.activity(self.now_ms());
            }
            Event::MenuAction(generation, action) => {
                if !self.window_generation_live(generation)
                    || (!matches!(action, MenuAction::Reset) && generation != self.menu_generation)
                {
                    return;
                }
                self.policy.activity(self.now_ms());
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
                        let Some(path) = menu::hover_target(&self.menu_path, level, &token) else {
                            return;
                        };
                        if now {
                            self.set_menu_path(path);
                        } else {
                            self.menu_delay.hover(&self.menu_path, path, self.now_ms());
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
                                eprintln!("menu execute: {:?}", error);
                            }
                        }
                        None => (),
                    },
                }
            }
            Event::ActionDone(reply, action, result) => self.action_done(reply, &action, result),
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
                    self.timeline_source(&previous, &data);
                    self.backdrop_source();
                    self.update_title();
                    self.core_action_states();
                }
                self.broadcast(&event, data)
            }
            Event::Binding(generation, owner, id) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                self.policy.activity(self.now_ms());
                if let Err(error) = self.execute_command(owner, &id) {
                    eprintln!("binding execute: {error:?}");
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
            Event::PipReturn(generation) => {
                if generation == self.shell_generation {
                    self.pip.restore();
                }
            }
            Event::FloatingClosed(generation) => {
                if let Some(owner) = self
                    .floating
                    .as_ref()
                    .filter(|f| f.generation == generation)
                    .map(|f| f.owner)
                {
                    let now = self.now_ms();
                    let _ =
                        self.policy
                            .set_preferred(window_state::Slot::Sidebar, owner, false, now);
                    if let Some(mount) = self.mounts.get_mut(&owner) {
                        mount.visible = false;
                    }
                    self.remember_panel(owner);
                    self.close_floating();
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
                    eprintln!("keys: inject to slint {key:?} {kind:?} window={generation}");
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
                    eprintln!(
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
            Event::Settings(generation, index) => self.settings_event(generation, index),
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
                eprintln!("plugin {owner}: {error}");
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
        if self.packages.is_trial(owner) {
            return;
        }
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
    /// cannot show it switches the player to software rendering.
    fn attach_gpu(&self, shell: &slint_interpreter::ComponentInstance) {
        let Some(gpu) = self.backend.render.gpu() else {
            return;
        };
        let render = self.backend.render.clone();
        let show = move |shell: &slint_interpreter::ComponentInstance, image: slint::Image| {
            // A generated stream's picture stays (its frames come as pixels).
            if !render.producer() {
                let _ = shell.set_property("video-frame", image.into());
            }
        };
        let render = self.backend.render.clone();
        let failed = move |reason: String| render.fall_back(&reason);
        if let Err(error) = gpu.attach(shell, show, failed) {
            self.backend
                .render
                .fall_back(&format!("the window cannot show GPU frames ({error})"));
        }
    }
    /// Set the newest rendered frame on the video area (no copy: the renderer
    /// drew into this buffer).
    fn show_frame(&mut self) {
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
        let image = slint::Image::from_rgb8(frame.buffer);
        self.last_image = Some(image.clone());
        self.last_frame = Some(Instant::now());
        let _ = window.set_property("video-frame", image.into());
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
                eprintln!("{line}");
                self.pip_log = Some(line);
            }
        }
        for effect in self.pip.tick(&facts, place, &playback, now) {
            match effect {
                pip::Effect::Respond(reply, result) => self.respond(reply, result),
                pip::Effect::Action(action, step) => {
                    if let Err(error) = self.run_core_action_with(action, None, step) {
                        eprintln!("pip: {action}: {}", error.message);
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
            eprintln!("host.error physical-input bridge: {error}");
        }
        if self.osd_until.is_some_and(|until| now >= until) {
            self.osd_until = None;
            let _ = shell.set_property("osd-visible", false.into());
        }
        self.host_command_states(fullscreen);
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
        if let Some(log) = self.frame_log.as_mut() {
            let (mut counts, mut size) = (self.backend.frame.counts(), self.shown_size);
            if let Some(gpu) = self.backend.render.gpu() {
                counts = (counts.0 + gpu.frames(), counts.1 + gpu.shown());
                size = gpu.size().or(size);
            }
            log.tick(counts, size, Instant::now());
        }
        self.apply_on_top();
        // R-H11: a left press with no second press within the double-click
        // interval, and the pointer still within 4 px, is a single click.
        let due = self
            .input
            .borrow()
            .click
            .filter(|click| click.at.elapsed() >= double_click_interval());
        if let Some(click) = due {
            self.input.borrow_mut().click = None;
            let moved = (number("pointer-x") - click.x).hypot(number("pointer-y") - click.y);
            if click.generation == self.shell_generation && moved <= CLICK_SLOP {
                let input = bindings::Input {
                    mods: click.mods,
                    name: "Mouse:LeftClick".into(),
                };
                fire(
                    &self.input,
                    &self.events,
                    click.generation,
                    &input,
                    false,
                    None,
                );
            }
        }
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
        if flag("video-hover") {
            self.policy
                .pointer(number("pointer-x"), viewport.width, now);
        } else if bottom_hover {
            // The bar lies left of the popped-out sidebar.
            self.policy.pointer(0., viewport.width, now);
        }
        self.policy.tick(now);
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            shell.set_property("host-fullscreen", fullscreen.into())?;
            shell.set_property(
                "menu-open",
                (self.menu_open && self.menu_generation == self.shell_generation).into(),
            )?;
            let bottom = snapshot.bottom.is_some_and(|s| s.effective);
            shell.set_property("bottom-visible", bottom.into())?;
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
            // UI-SHELL-09: the pointer hides with the idle fullscreen bar while playing.
            let idle = snapshot
                .bottom
                .is_some_and(|s| s.reason == Some(window_state::Reason::Idle));
            // `setCursorAutoHide("playing")`: in a window too, while it has focus.
            let windowed = self.cursor_playing.get()
                && self.policy.idle(now)
                && (fullscreen || native.is_some_and(|facts| facts.active));
            shell.set_property(
                "cursor-hidden",
                (playing && ((fullscreen && idle) || windowed)).into(),
            )?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"videoArea":{"width":video.0,"height":video.1},"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot,"backdrop":self.backdrop_state(),"pictureInPicture":self.pip_state()}),
            )
        })();
        self.answer_video_area(video, now);
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
    /// Keep the sidebar owner's persisted preferences current.
    fn remember_panel(&mut self, owner: u64) {
        let Some(mount) = self.mounts.get(&owner) else {
            return;
        };
        if mount.spec.slot != composition::Slot::Sidebar {
            return;
        }
        let floating = self
            .ui_state
            .state
            .panels
            .get(&mount.manifest)
            .and_then(|p| p.floating);
        let prefs = ui_state::PanelPrefs {
            visible: mount.visible,
            detached: mount.detached,
            floating,
        };
        self.ui_state
            .state
            .panels
            .insert(mount.manifest.clone(), prefs);
        self.ui_state.touch(self.now_ms());
    }
    /// Open or close the floating window so it matches the policy.
    fn sync_floating(&mut self, want: bool) {
        let owner = self
            .mounts
            .values()
            .find(|m| m.spec.slot == composition::Slot::Sidebar)
            .map(|m| m.spec.owner);
        match (&self.floating, want, owner) {
            (Some(floating), true, Some(owner)) if floating.owner == owner => {
                self.remember_floating();
            }
            (None, true, Some(owner)) => {
                if let Err(error) = self.open_floating(owner) {
                    eprintln!("floating panel: {error:#}");
                    self.send(owner, json!({"event":"ui.error","data":{"message":format!("ui-prepare: {error:#}")}}));
                    let _ = self.policy.set_detached(owner, false);
                    if let Some(mount) = self.mounts.get_mut(&owner) {
                        mount.detached = false;
                    }
                }
            }
            (Some(_), _, _) => self.close_floating(),
            _ => {}
        }
    }
    fn open_floating(&mut self, owner: u64) -> Result<()> {
        let mount = self
            .mounts
            .get(&owner)
            .ok_or_else(|| anyhow::anyhow!("no sidebar mount"))?
            .clone();
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        let instance = composition::compile_floating(
            &mount.spec,
            &mount.title,
            self.mount_events.clone(),
            generation,
        )?;
        for (name, value) in self.mount_values(&mount) {
            composition::set_global_value(&instance, &mount.spec, &name, value)?;
        }
        self.bind_menu(&instance, generation, false)?;
        self.observe_keys(&instance, generation);
        self.bind_keys(&instance, generation)?;
        let events = self.events.clone();
        instance.window().on_close_requested(move || {
            let _ = events.send(Event::FloatingClosed(generation));
            slint::CloseRequestResponse::KeepWindowShown
        });
        let frame = self
            .ui_state
            .state
            .panels
            .get(&mount.manifest)
            .and_then(|p| p.floating)
            .unwrap_or(ui_state::Frame {
                x: 438.,
                y: 100.,
                width: 300.,
                height: 408.,
            });
        instance.window().set_size(slint::LogicalSize::new(
            frame.width.max(300.) as f32,
            frame.height.max(250.) as f32,
        ));
        instance
            .window()
            .set_position(slint::LogicalPosition::new(frame.x as f32, frame.y as f32));
        self.theme_view(&instance);
        let before = platform::capture_windows().ok();
        instance.show()?;
        platform::shown(instance.window());
        let native = before.and_then(|before| platform::associate_new(&before).ok());
        self.floating = Some(Floating {
            instance,
            owner,
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
        let Some(manifest) = self.mounts.get(&floating.owner).map(|m| m.manifest.clone()) else {
            return;
        };
        let prefs = self
            .ui_state
            .state
            .panels
            .entry(manifest)
            .or_insert(ui_state::PanelPrefs {
                visible: true,
                detached: true,
                floating: None,
            });
        if prefs.floating != Some(frame) {
            prefs.floating = Some(frame);
            self.ui_state.touch(self.now_ms());
        }
    }
    fn close_floating(&mut self) {
        if let Some(floating) = self.floating.take() {
            if self.menu_open && self.menu_generation == floating.generation {
                self.menu_open = false;
                self.menu_path.clear();
                self.menu_scroll.clear();
                self.menu_delay.clear();
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
            window.window().on_winit_window_event(move |_, event| {
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
            // A second press within the double-click interval is a double click.
            let second = shared.click.take().is_some_and(|first| {
                first.at.elapsed() < double_click_interval()
                    && (first.x - x).hypot(first.y - y) <= CLICK_SLOP
            });
            if !second {
                shared.click = Some(PendingClick {
                    at: Instant::now(),
                    x,
                    y,
                    mods: bindings::Mods {
                        ctrl,
                        alt: flag(3),
                        shift: flag(4),
                        cmd,
                    },
                    generation,
                });
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
            )?,
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
    /// A window that forwards keys to the binding table records each press's
    /// physical key first (see `observe_key`).
    fn observe_keys(&self, window: &slint_interpreter::ComponentInstance, generation: u64) {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
        let input = self.input.clone();
        let events = self.events.clone();
        let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
        window.window().on_winit_window_event(move |_, event| {
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
                eprintln!(
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
                && input.borrow().table.resolve(&typed, panel).is_some()
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
        self.menu_delay.clear();
        if let Some(window) = self.window_for(generation) {
            let _ = window.set_property("menu-open", true.into());
        }
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// Open this chain of submenus now. Levels that close or change forget
    /// their scroll offset; a waiting delayed hover is dropped.
    fn set_menu_path(&mut self, path: Vec<String>) {
        self.menu_delay.clear();
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
    /// G-15: apply a hover whose submenu delay has passed.
    fn menu_tick(&mut self) {
        if !self.menu_open {
            return;
        }
        let now = self.now_ms();
        if let Some(path) = self.menu_delay.due(now, self.menu_show_delay_ms) {
            self.set_menu_path(path);
        }
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
                menu_levels(
                    window,
                    &roots,
                    &mut self.menu_path,
                    &self.menu_scroll,
                    &labels,
                )?
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
                ui_values::from_json(&menu::context_level(
                    &entries,
                    origin,
                    size,
                    self.menu_scroll.first().copied().unwrap_or(0.),
                    &labels,
                ))?
            }
        };
        set_menu_model(window, model)
    }
    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_delay.clear();
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
        if self.preparing.is_none() {
            if let Some(change) = self.mount_queue.pop_front() {
                self.send(
                    change.reply.owner,
                    json!({"event":"ui.preparing","data":{"phase":"preparing"}}),
                );
                self.preparing = Some((Instant::now(), change));
            }
            return;
        }
        if self.preparing.as_ref().unwrap().0.elapsed() < Duration::from_millis(32) {
            return;
        }
        let (_, change) = self.preparing.take().unwrap();
        let started = Instant::now();
        let mut next = self.mounts.clone();
        let owner = change.reply.owner;
        if let Some(mount) = change.contribution {
            if !self.plugins.contains_key(&owner) {
                return;
            }
            next.insert(owner, mount);
        } else {
            next.remove(&owner);
        }
        self.next_shell_generation += 1;
        let candidate_generation = self.next_shell_generation;
        let native_before = platform::capture_windows().ok();
        let values: Vec<_> = next
            .values()
            .map(|mount| self.mount_values(mount))
            .collect();
        let build = || -> Result<slint_interpreter::ComponentInstance> {
            let specs = next.values().map(|m| m.spec.clone()).collect::<Vec<_>>();
            let shell =
                composition::compile_json(&specs, self.mount_events.clone(), candidate_generation)?;
            for (mount, values) in next.values().zip(values) {
                for (name, value) in values {
                    composition::set_global_value(&shell, &mount.spec, &name, value)?;
                }
                if let Some(property) = mount.spec.slot.visible_property() {
                    shell.set_property(property, (mount.visible && !mount.detached).into())?;
                }
            }
            self.bind_shell(&shell, candidate_generation)?;
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
            self.attach_gpu(&shell);
            shell.show()?;
            platform::shown(shell.window());
            Ok(shell)
        };
        let result = match build() {
            Ok(shell) => {
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
                for (id, mount) in &next {
                    if !self.mounts.contains_key(id) {
                        let Some(slot) = policy_slot(mount.spec.slot) else {
                            continue;
                        };
                        if let Err(error) =
                            self.policy.attach(slot, *id, mount.visible, mount.detached)
                        {
                            eprintln!("mount policy attach: {error}");
                        }
                    }
                }
                for (id, mount) in &next {
                    let Some(slot) = policy_slot(mount.spec.slot) else {
                        continue;
                    };
                    if let Err(error) =
                        self.policy
                            .set_locks(slot, *id, window_state::Locks::default())
                    {
                        eprintln!("mount policy reset: {error}");
                    }
                }
                self.policy.activity(self.now_ms());
                {
                    let mut input = self.input.borrow_mut();
                    let owner = |slot| {
                        next.values()
                            .find(|m| m.spec.slot == slot)
                            .map(|m| m.spec.owner)
                    };
                    input.bottom = owner(composition::Slot::Bottom);
                    input.sidebar = owner(composition::Slot::Sidebar);
                }
                if self.menu_generation == self.shell_generation {
                    self.menu_open = false;
                    self.menu_mode = MenuMode::Main;
                }
                self.mounts = next;
                self.shell_generation = candidate_generation;
                Ok(
                    json!({"status":"ready","shellGeneration":candidate_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
                )
            }
            Err(error) => Err(RpcError::new("ui-prepare", error.to_string())),
        };
        self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":started.elapsed().as_secs_f64()*1000.}}));
        if change.reply.id == 0 {
            if let Err(error) = result {
                self.broadcast(
                    "host.error",
                    json!({"code":"ui-owner-cleanup","owner":owner,"message":error.message}),
                );
            }
        } else {
            self.respond(change.reply, result);
        }
    }
    /// A mount's properties converted for a new instance; a value that no
    /// longer converts (an image file removed since) is left out and logged.
    fn mount_values(&mut self, mount: &Mount) -> Vec<(String, SValue)> {
        let owner = mount.spec.owner;
        mount
            .properties
            .iter()
            .filter_map(
                |(name, value)| match self.prop_value(owner, "$mount", name, value) {
                    Ok(value) => Some((name.clone(), value)),
                    Err(error) => {
                        eprintln!("mount property {owner}/{name}: {}", error.message);
                        None
                    }
                },
            )
            .collect()
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
                Err(error)=>self.send(event.owner,json!({"event":"ui.error","data":{"message":error}})),
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
        let mut fullscreen = commands::Declaration::new("全屏", &["全屏"]);
        fullscreen.group = "screen".into();
        fullscreen.order = 700;
        // IA-CORE-04: Enter and Alt+Enter as in PotPlayer; F11 is a Vestra extra.
        fullscreen.bindings = vec![
            "Enter".into(),
            "Alt+Enter".into(),
            "F11".into(),
            "Mouse:LeftDoubleClick".into(),
        ];
        let mut exit = commands::Declaration::new("退出全屏", &[]);
        exit.bindings = vec!["Escape".into()];
        let mut quit = commands::Declaration::new("退出", &["退出"]);
        quit.group = "exit".into();
        quit.order = 990;
        quit.bindings = vec!["Cmd+Q".into()];
        for (id, declaration, state) in [
            ("fullscreen", fullscreen, state(true, Some(false))),
            ("exit-fullscreen", exit, state(false, None)),
            ("quit", quit, state(true, None)),
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
            .max_by_key(|(sequence, _)| *sequence)
            .map_or(core_actions::DEFAULT_STEP, |(_, seconds)| *seconds)
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
                    eprintln!("host.error core action {id}: {error}");
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
    /// plugin's `host.run`) is answered when it finished; `Ok(true)` means it
    /// was answered already. A disabled action is `unavailable`.
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
                    "sequence":{"next":facts.next,"previous":facts.previous}}),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
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
        if core_actions::ACTIONS.contains(&id) {
            if let Err(error) = self.run_core_action(id, None) {
                eprintln!("core action {id}: {}", error.message);
            }
            return;
        }
        let Some(shell) = &self.shell else { return };
        let window = shell.window();
        match id {
            "fullscreen" => window.set_fullscreen(!window.is_fullscreen()),
            "exit-fullscreen" => window.set_fullscreen(false),
            "quit" => {
                self.ui_state.flush();
                let _ = slint::quit_event_loop();
            }
            _ => {}
        }
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
    /// C9: "<source name> - Vestra", or "Vestra" without a source.
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
        let title = name.map_or_else(|| "Vestra".to_owned(), |name| format!("{name} - Vestra"));
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
            eprintln!("host.error power {error}");
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
                eprintln!("host.error window level: {error}");
                self.on_top_applied = Some((want, count));
            }
        }
    }
    /// `{ fullscreen, videoArea, pictureInPicture }` of the main window
    /// (zero without one).
    fn window_state(&self) -> Value {
        let facts = &self.window_facts;
        json!({"fullscreen":facts["fullscreen"] == true,
            "videoArea":{"width":facts["videoArea"]["width"].as_f64().unwrap_or(0.),
                "height":facts["videoArea"]["height"].as_f64().unwrap_or(0.)},
            "pictureInPicture":self.pip_state(),
            "appearance":self.theme()})
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
            return Err(unavailable("全屏时不可用"));
        }
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
        let slot = |name: &str| {
            let slot = &self.window_facts["slots"][name];
            if slot.is_null() {
                return Value::Null;
            }
            json!({"owner":slot["owner"],"preferred":slot["preferred"],"effective":slot["effective"],
                "reason":slot["reason"],"detached":slot["detached"]})
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
    fn respond(&mut self, reply: Reply, result: std::result::Result<Value, RpcError>) {
        if self.activation_ui.remove(&(reply.owner, reply.id)) {
            let outcome = result.as_ref().map(|_| ()).map_err(|error| {
                format!("{}: {}", error.code, error.message)
                    .chars()
                    .take(512)
                    .collect()
            });
            if let Err(error) =
                self.activations
                    .ui_finish(reply.owner, reply.id, outcome, self.now_ms())
            {
                eprintln!("activation UI completion: {error}");
            }
        }
        let message = match result {
            Ok(value) => json!({"id":reply.id,"result":value}),
            Err(error) => json!({"id":reply.id,"error":error}),
        };
        self.send(reply.owner, message);
    }
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
        let capability = match method {
            "packages.list" => None,
            "bindings.list" | "bindings.set" | "bindings.reset" => Some("bindings.manage"),
            // Plugin and binding changes have their own (or no) capability.
            "events.subscribe" | "events.unsubscribe" => match params["event"].as_str() {
                Some("packages.changed") => None,
                Some("bindings.changed") => Some("bindings.manage"),
                _ => Some("events"),
            },
            method if method.starts_with("packages.") => Some("plugins.manage"),
            method if method.starts_with("sourceResolvers.") => Some("source.resolve"),
            "dialogs.open" | "dialogs.confirm" => Some("ui.dialog"),
            "window.state" => None,
            "window.setVideoArea"
            | "window.setAlwaysOnTop"
            | "window.setCursorAutoHide"
            | "window.setPictureInPicture" => Some("window.control"),
            "player.chapters" | "player.audioDevices" | "timeline.marks" => Some("playback.read"),
            "timeline.set" => Some("timeline.marks"),
            "drops.provide" | "drops.withdraw" | "drops.listen" => Some("ui.drop"),
            method if method.starts_with("settings.") => Some("ui.settings"),
            "appearance.set" => Some("ui.theme"),
            "player.setLoop" | "player.setAudioDevice" => Some("playback.control"),
            "ui.mount" | "ui.setMount" | "ui.unmount" | "ui.interaction" | "ui.menu"
            | "layout.state" | "layout.set" => Some("ui.mount"),
            "osd.show" | "osd.panel" => Some("ui.osd"),
            "clipboard.readText" | "clipboard.writeText" | "clipboard.writeImage" => {
                Some("clipboard")
            }
            "power.preventSleep" | "player.setStepSeek" | "player.step" => Some("playback.control"),
            "system.reveal" | "system.openPath" | "system.openUrl" | "app.system" => None,
            "secrets.get" | "secrets.set" | "secrets.delete" => Some("secrets"),
            "host.run" => Some("commands"),
            "playback.stats" => Some("playback.read"),
            "sequence.set" | "sequence.step" => Some("playback.control"),
            "commands.register" | "commands.update" | "commands.unregister" => Some("commands"),
            "source.state" | "source.operation" | "processing.state" => Some("playback.read"),
            "playback.set" | "source.submit" | "audio.addTrack" | "audio.selectTrack" => {
                Some("playback.control")
            }
            "producer.prepare"
            | "producer.state"
            | "producer.release"
            | "producer.ack"
            | "producer.declareInput"
            | "producer.ackInput" => Some("producer.av"),
            "snapshot.state" | "jobs.state" | "jobs.operation" => Some("media.read"),
            method if method.starts_with("snapshot.") || method.starts_with("jobs.") => {
                Some("media.control")
            }
            "subtitles.state" => Some("subtitles.read"),
            method if method.starts_with("subtitles.") => Some("subtitles.control"),
            "processing.replace" | "processing.operation" => Some(if params["stage"] == "video" {
                "processing.video"
            } else {
                "processing.audio"
            }),
            "ui.open" | "ui.set" | "ui.show" | "ui.close" => Some("ui.window"),
            "video.view.set" | "video.view.release" => Some("processing.video"),
            "video.render.set" | "video.render.release" => Some("processing.video"),
            _ => return Err(RpcError::new("unsupported", "unknown method")),
        };
        if capability.is_some_and(|capability| !plugin.allows(capability)) {
            return Err(RpcError::new(
                "capability",
                "capability must be declared in manifest",
            ));
        }
        if method.starts_with("packages.") {
            return self.package_dispatch(reply.owner, method, params).map(Some);
        }
        if method.starts_with("bindings.") {
            return self.bindings_dispatch(method, params).map(Some);
        }
        if method.starts_with("system.") || method.starts_with("secrets.") || method == "app.system"
        {
            return self.system_dispatch(reply, method, params);
        }
        if method.starts_with("subtitles.")
            || method.starts_with("snapshot.")
            || method.starts_with("jobs.")
            || method.starts_with("video.view.")
            || method.starts_with("video.render.")
        {
            self.backend
                .commands
                .send(Command {
                    reply: reply.clone(),
                    method: method.into(),
                    params: params.clone(),
                })
                .map_err(anyhow::Error::from)?;
            return Ok(None);
        }
        let value = match method {
            "clipboard.readText" | "clipboard.writeText" | "clipboard.writeImage" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "the clipboard is unavailable in headless mode",
                    ));
                }
                clipboard::dispatch(method, params)?
            }
            "host.run" => {
                let action = string(params, "action")?;
                if !core_actions::ACTIONS.contains(&action) {
                    return Err(RpcError::invalid(format!("unknown host action {action}")));
                }
                if !self.run_core_action(action, Some(reply))? {
                    return Ok(None);
                }
                Value::Null
            }
            "player.setStepSeek" => {
                let seconds = params["seconds"]
                    .as_f64()
                    .filter(|s| core_actions::valid_step(*s))
                    .ok_or_else(|| RpcError::invalid("seconds must be 0.1..3600 in tenths"))?;
                let before = self.step_seconds();
                self.step_writes += 1;
                self.step_seek
                    .insert(reply.owner, (self.step_writes, seconds));
                self.step_seek_changed(before);
                Value::Null
            }
            "power.preventSleep" => {
                self.power
                    .request(reply.owner, params["mode"].as_str().unwrap_or(""))
                    .map_err(RpcError::invalid)?;
                self.power_tick();
                Value::Null
            }
            "layout.state" => self.layout_state()?,
            "timeline.marks" | "timeline.set" => {
                self.timeline_dispatch(reply.owner, method, params)?
            }
            "drops.provide" | "drops.withdraw" | "drops.listen" => {
                self.drops_dispatch(reply.owner, method, params)?
            }
            "appearance.set" => self.appearance_set(params)?,
            method if method.starts_with("settings.") => {
                return self.settings_dispatch(reply, method, params);
            }
            "window.state" => self.window_state(),
            "window.setVideoArea" => {
                self.set_video_area(reply, params)?;
                return Ok(None);
            }
            "window.setAlwaysOnTop" => {
                let on = params["onTop"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("onTop requires a boolean"))?;
                if self.headless {
                    return Err(RpcError::new("unavailable", "no native window"));
                }
                self.on_top.set(reply.owner, on);
                self.apply_on_top();
                Value::Null
            }
            "window.setPictureInPicture" => match self.set_picture_in_picture(reply, params)? {
                Some(answer) => answer,
                None => return Ok(None),
            },
            "window.setCursorAutoHide" => {
                let playing = match string(params, "mode")? {
                    "fullscreen" => false,
                    "playing" => true,
                    _ => return Err(RpcError::invalid("mode is fullscreen or playing")),
                };
                self.cursor_playing.set(reply.owner, playing);
                Value::Null
            }
            "dialogs.confirm" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "native dialogs require a running native window",
                    ));
                }
                if self.dialog_busy {
                    return Err(RpcError::new("busy", "a native dialog is already open"));
                }
                let options = confirm::parse(params)?;
                let events = self.events.clone();
                let answer = reply.clone();
                self.dialog_busy = true;
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::Dialog);
                if let Err(error) =
                    platform::confirm(&options, self.native_window, move |yes, checked| {
                        let _ = events.send(Event::ConfirmDone(answer.clone(), yes, checked));
                    })
                {
                    self.dialog_busy = false;
                    return Err(error);
                }
                return Ok(None);
            }
            "layout.set" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "layout requires a native shell",
                    ));
                }
                let slot = match string(params, "slot")? {
                    "bottom" => composition::Slot::Bottom,
                    "sidebar" => composition::Slot::Sidebar,
                    _ => return Err(RpcError::invalid("unknown slot")),
                };
                let owner = self
                    .mounts
                    .values()
                    .find(|m| m.spec.slot == slot)
                    .map(|m| m.spec.owner)
                    .ok_or_else(|| RpcError::new("missing", "no panel at this position"))?;
                let policy_slot = policy_slot(slot)
                    .ok_or_else(|| RpcError::invalid("the backdrop has no preference"))?;
                if let Some(detached) = params.get("detached").filter(|v| !v.is_null()) {
                    let detached = detached
                        .as_bool()
                        .ok_or_else(|| RpcError::invalid("detached boolean required"))?;
                    if slot == composition::Slot::Bottom {
                        return Err(RpcError::invalid("the bottom bar cannot be detached"));
                    }
                    self.policy
                        .set_detached(owner, detached)
                        .map_err(anyhow::Error::from)?;
                    if let Some(mount) = self.mounts.get_mut(&owner) {
                        mount.detached = detached;
                    }
                }
                if let Some(visible) = params.get("visible").filter(|v| !v.is_null()) {
                    let visible = visible
                        .as_bool()
                        .ok_or_else(|| RpcError::invalid("visible boolean required"))?;
                    let now = self.now_ms();
                    self.policy
                        .set_preferred(policy_slot, owner, visible, now)
                        .map_err(anyhow::Error::from)?;
                    if let Some(mount) = self.mounts.get_mut(&owner) {
                        mount.visible = visible;
                    }
                }
                self.remember_panel(owner);
                // Answer after the next layout pass so a following layout.state sees it.
                self.window_tick();
                Value::Null
            }
            "ui.menu" => {
                if debug_keys() {
                    eprintln!(
                        "keys: ui.menu owner={} menu_open={} dialog={} sidebar_effective={}",
                        reply.owner,
                        self.menu_open,
                        self.dialog_busy,
                        self.window_facts["slots"]["sidebar"]["effective"]
                    );
                }
                let mount = self
                    .mounts
                    .get(&reply.owner)
                    .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
                let items = params["items"]
                    .as_array()
                    .filter(|items| !items.is_empty() && items.len() <= 32)
                    .ok_or_else(|| RpcError::invalid("items requires 1..=32 entries"))?;
                let mut ids = Vec::new();
                for item in items {
                    match item {
                        Value::Null => ids.push(None),
                        Value::String(id) if self.commands.get(reply.owner, id).is_some() => {
                            ids.push(Some(id.clone()))
                        }
                        _ => {
                            return Err(RpcError::new(
                                "missing",
                                format!("{item} is not a command of this plugin"),
                            ));
                        }
                    }
                }
                let number = |key: &str| {
                    params[key]
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| RpcError::invalid(format!("{key} requires a finite number")))
                };
                let (x, y) = (number("x")?, number("y")?);
                let slot = match mount.spec.slot {
                    composition::Slot::Bottom => "bottom",
                    composition::Slot::Sidebar => "sidebar",
                    composition::Slot::Backdrop => {
                        return Err(RpcError::new(
                            "unavailable",
                            "the backdrop takes no pointer input",
                        ));
                    }
                };
                let generation = if self
                    .floating
                    .as_ref()
                    .is_some_and(|f| f.owner == reply.owner)
                {
                    self.floating.as_ref().unwrap().generation
                } else if self.window_facts["slots"][slot]["effective"] == true {
                    self.shell_generation
                } else {
                    return Err(RpcError::new("unavailable", "the panel is not shown"));
                };
                if let Some(window) = self.window_for(generation) {
                    window
                        .set_property("menu-origin-x", x.into())
                        .map_err(anyhow::Error::from)?;
                    window
                        .set_property("menu-origin-y", y.into())
                        .map_err(anyhow::Error::from)?;
                }
                self.open_menu(generation, MenuMode::Panel(reply.owner, ids));
                Value::Null
            }
            "osd.show" => {
                let text = string(params, "text")?.replace(['\n', '\r'], " ");
                if text.trim().is_empty() || text.chars().count() > 200 {
                    return Err(RpcError::invalid("osd text needs 1..=200 characters"));
                }
                let shell = self
                    .shell
                    .as_ref()
                    .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
                shell
                    .set_property("osd-text", SValue::String(text.as_str().into()))
                    .map_err(anyhow::Error::from)?;
                shell
                    .set_property("osd-visible", true.into())
                    .map_err(anyhow::Error::from)?;
                self.osd_until = Some(self.now_ms() + OSD_MS);
                Value::Null
            }
            "osd.panel" => {
                if self.shell.is_none() {
                    return Err(RpcError::new("unavailable", "native shell unavailable"));
                }
                match &params["text"] {
                    Value::Null => {
                        self.info_panels.remove(&reply.owner);
                    }
                    Value::String(text) => {
                        if text.len() > 4096 || text.lines().count() > 32 {
                            return Err(RpcError::invalid("osd panel allows 32 lines, 4096 bytes"));
                        }
                        self.info_panels.insert(reply.owner, text.clone());
                    }
                    _ => return Err(RpcError::invalid("text requires string or null")),
                }
                self.show_info_panels();
                Value::Null
            }
            "sequence.set" => {
                if let Some((provider, ..)) = self.sequence
                    && provider != reply.owner
                {
                    return Err(RpcError::new(
                        "conflict",
                        "another plugin provides the sequence",
                    ));
                }
                let flags = &params["flags"];
                self.sequence = if flags.is_null() {
                    None
                } else {
                    let flag = |key: &str| {
                        flags[key]
                            .as_bool()
                            .ok_or_else(|| RpcError::invalid(format!("{key} boolean required")))
                    };
                    Some((reply.owner, flag("canNext")?, flag("canPrevious")?))
                };
                self.rebroadcast_source();
                Value::Null
            }
            "sequence.step" => {
                let direction = string(params, "direction")?;
                let (provider, next, previous) = self
                    .sequence
                    .ok_or_else(|| RpcError::new("unavailable", "no sequence provider"))?;
                let allowed = match direction {
                    "next" => next,
                    "previous" => previous,
                    _ => return Err(RpcError::invalid("direction is next or previous")),
                };
                if !allowed {
                    return Err(RpcError::new(
                        "unavailable",
                        format!("cannot go {direction}"),
                    ));
                }
                self.send(
                    provider,
                    json!({"event":"sequence.step","data":{"direction":direction}}),
                );
                Value::Null
            }
            "ui.interaction" => {
                let mount = self
                    .mounts
                    .get(&reply.owner)
                    .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
                let boolean = |key: &str| {
                    params[key]
                        .as_bool()
                        .ok_or_else(|| RpcError::invalid(format!("{key} boolean required")))
                };
                let locks = window_state::Locks {
                    focused: boolean("focused")?,
                    dragging: boolean("dragging")?,
                    popup_open: boolean("popupOpen")?,
                };
                self.policy.activity(self.now_ms());
                // The backdrop never auto-hides: nothing to lock.
                if let Some(slot) = policy_slot(mount.spec.slot) {
                    self.policy
                        .set_locks(slot, reply.owner, locks)
                        .map_err(anyhow::Error::from)?;
                }
                Value::Null
            }
            "ui.mount" | "ui.unmount" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "mounts require a native shell",
                    ));
                }
                if self
                    .mount_queue
                    .iter()
                    .any(|c| c.reply.owner == reply.owner)
                    || self
                        .preparing
                        .as_ref()
                        .is_some_and(|(_, c)| c.reply.owner == reply.owner)
                {
                    return Err(RpcError::new(
                        "busy",
                        "owner already has a pending UI preparation",
                    ));
                }
                if self.mount_queue.len() >= 8 {
                    return Err(RpcError::new("busy", "UI preparation queue full"));
                }
                let contribution = if method == "ui.mount" {
                    if self.mounts.contains_key(&reply.owner) {
                        return Err(RpcError::new("conflict", "owner already mounted"));
                    }
                    let slot = match string(params, "slot")? {
                        "bottom" => composition::Slot::Bottom,
                        "sidebar" => composition::Slot::Sidebar,
                        "backdrop" => composition::Slot::Backdrop,
                        _ => return Err(RpcError::invalid("unknown mount slot")),
                    };
                    // R-U3: one backdrop at a time, whoever mounted it.
                    let backdrop =
                        |spec: &composition::CompileSpec| spec.slot == composition::Slot::Backdrop;
                    let queued = self
                        .mount_queue
                        .iter()
                        .chain(self.preparing.as_ref().map(|(_, change)| change))
                        .any(|c| c.contribution.as_ref().is_some_and(|m| backdrop(&m.spec)));
                    if slot == composition::Slot::Backdrop
                        && (queued || self.mounts.values().any(|m| backdrop(&m.spec)))
                    {
                        return Err(RpcError::new(
                            "conflict",
                            "another plugin shows the backdrop",
                        ));
                    }
                    let file = plugins::package_file(&plugin.directory, string(params, "file")?)?;
                    let properties = serde_json::from_value(
                        params.get("properties").cloned().unwrap_or(json!({})),
                    )
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                    let title = params["title"].as_str().unwrap_or("Vestra");
                    if title.len() > 128 || title.chars().any(char::is_control) {
                        return Err(RpcError::invalid("title requires 0..=128 bytes"));
                    }
                    let manifest = plugin.manifest.id.clone();
                    // C8: a sidebar panel resumes its last visibility and floating state.
                    let prefs = (slot == composition::Slot::Sidebar)
                        .then(|| self.ui_state.state.panels.get(&manifest))
                        .flatten();
                    Some(Mount {
                        spec: composition::CompileSpec {
                            owner: reply.owner,
                            file,
                            component: string(params, "component")?.into(),
                            global: Some(string(params, "global")?.into()),
                            slot,
                        },
                        properties,
                        visible: prefs.is_none_or(|p| p.visible),
                        detached: prefs.is_some_and(|p| p.detached),
                        title: if title.is_empty() {
                            "Vestra".into()
                        } else {
                            title.into()
                        },
                        manifest,
                    })
                } else {
                    None
                };
                self.mount_queue.push_back(MountChange {
                    reply: reply.clone(),
                    contribution,
                });
                return Ok(None);
            }
            "ui.setMount" => {
                let spec = self
                    .mounts
                    .get(&reply.owner)
                    .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?
                    .spec
                    .clone();
                let shell = self
                    .shell
                    .as_ref()
                    .map(ComponentHandle::clone_strong)
                    .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
                let property = string(params, "property")?;
                let value = self.prop_value(reply.owner, "$mount", property, &params["value"])?;
                composition::set_global_value(&shell, &spec, property, value)?;
                if let Some(floating) = self
                    .floating
                    .as_ref()
                    .filter(|f| f.owner == reply.owner)
                    .map(|f| f.instance.clone_strong())
                {
                    let value =
                        self.prop_value(reply.owner, "$mount", property, &params["value"])?;
                    composition::set_global_value(&floating, &spec, property, value)?;
                }
                if let Some(mount) = self.mounts.get_mut(&reply.owner) {
                    mount
                        .properties
                        .insert(property.into(), params["value"].clone());
                }
                Value::Null
            }
            "dialogs.open" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "native dialogs require a running native window",
                    ));
                }
                if self.dialog_busy {
                    return Err(RpcError::new("busy", "a native dialog is already open"));
                }
                let kind = string(params, "kind")?;
                if !["openFile", "openFiles", "openDirectory", "saveFile"].contains(&kind) {
                    return Err(RpcError::invalid("unsupported dialog kind"));
                }
                let title = params["title"].as_str().unwrap_or("Select a path");
                if title.len() > 256 {
                    return Err(RpcError::invalid("dialog title too long"));
                }
                let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
                if let Some(name) = params["fileName"].as_str() {
                    if name.len() > 256 || name.contains(['/', '\\']) {
                        return Err(RpcError::invalid("fileName must be a simple name"));
                    }
                    dialog = dialog.set_file_name(name);
                }
                let events = self.events.clone();
                let reply = reply.clone();
                // RFD creates the native operation on this Slint main thread.
                // Only awaiting its completion belongs to the worker.
                match kind {
                    "openFiles" => {
                        let task = dialog.pick_files();
                        std::thread::spawn(move || {
                            let paths = pollster::block_on(task)
                                .map(|v| v.into_iter().map(|f| f.path().to_path_buf()).collect());
                            let _ = events.send(Event::DialogDone(reply, paths));
                        });
                    }
                    _ => {
                        let task: std::pin::Pin<
                            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
                        > = match kind {
                            "openDirectory" => Box::pin(dialog.pick_folder()),
                            "saveFile" => Box::pin(dialog.save_file()),
                            _ => Box::pin(dialog.pick_file()),
                        };
                        std::thread::spawn(move || {
                            let paths =
                                pollster::block_on(task).map(|f| vec![f.path().to_path_buf()]);
                            let _ = events.send(Event::DialogDone(reply, paths));
                        });
                    }
                }
                self.dialog_busy = true;
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::Dialog);
                return Ok(None);
            }
            "commands.unregister" => {
                let id = string(params, "id")?;
                self.commands
                    .remove(reply.owner, id)
                    .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
                self.input.borrow_mut().table.remove(reply.owner, id);
                if self.menu_open {
                    let result = self.refresh_menu();
                    self.report_menu(result);
                }
                Value::Null
            }
            "commands.register" | "commands.update" => {
                let id = string(params, "id")?;
                let revision = params["revision"]
                    .as_u64()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| RpcError::invalid("positive revision required"))?;
                let state = serde_json::from_value(params["state"].clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                if method == "commands.register" {
                    let mut declaration: commands::Declaration =
                        serde_json::from_value(params["declaration"].clone())
                            .map_err(|e| RpcError::invalid(e.to_string()))?;
                    // Trial packages: a marked leaf title and no bindings.
                    if self.packages.is_trial(reply.owner) {
                        if let Some(leaf) = declaration.path.last_mut() {
                            *leaf = format!("试载 {} · {leaf}", reply.owner);
                        }
                        declaration.bindings.clear();
                    }
                    let manifest = plugin.manifest.id.clone();
                    let bindings = declaration.bindings.clone();
                    let panel = declaration.focus == Some(commands::Focus::Panel);
                    let repeat = declaration.repeat;
                    self.commands
                        .register(
                            reply.owner,
                            &manifest,
                            id.into(),
                            revision,
                            declaration,
                            state,
                        )
                        .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
                    self.input
                        .borrow_mut()
                        .table
                        .declare(reply.owner, id, &manifest, &bindings, panel, repeat)
                        .map_err(RpcError::invalid)?;
                } else {
                    self.commands
                        .update(reply.owner, id, revision, state)
                        .map_err(|e| RpcError::new("command", e.to_string()))?;
                }
                json!({"revision":self.commands.revision()})
            }
            "sourceResolvers.register"
            | "sourceResolvers.unregister"
            | "sourceResolvers.respond"
            | "playback.set"
            | "audio.addTrack"
            | "audio.selectTrack"
            | "player.step"
            | "player.chapters"
            | "player.setLoop"
            | "player.audioDevices"
            | "player.setAudioDevice"
            | "source.state"
            | "source.operation"
            | "source.submit"
            | "producer.prepare"
            | "producer.state"
            | "producer.release"
            | "producer.ack"
            | "producer.declareInput"
            | "producer.ackInput"
            | "processing.replace"
            | "processing.operation"
            | "processing.state"
            | "playback.stats" => {
                if method == "source.state" {
                    self.source_state_replies.insert((reply.owner, reply.id));
                }
                self.backend
                    .commands
                    .send(Command {
                        reply: reply.clone(),
                        method: method.into(),
                        params: params.clone(),
                    })
                    .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
                return Ok(None);
            }
            "events.subscribe" | "events.unsubscribe" => {
                let event = string(params, "event")?;
                if ![
                    "playback.backend",
                    "source.state",
                    "window.state",
                    "packages.changed",
                    "bindings.changed",
                    "timeline.changed",
                    "appearance.changed",
                ]
                .contains(&event)
                {
                    return Err(RpcError::new("unsupported", "unknown event"));
                }
                let plugin = self.plugins.get_mut(&reply.owner).unwrap();
                if method == "events.subscribe" {
                    plugin.subscriptions.insert(event.into());
                } else {
                    plugin.subscriptions.remove(event);
                }
                Value::Null
            }
            "ui.open" => {
                if self.headless {
                    return Err(RpcError::new(
                        "unavailable",
                        "UI is unavailable in headless mode",
                    ));
                }
                let id = string(params, "id")?.to_owned();
                if self.views.contains_key(&(reply.owner, id.clone())) {
                    return Err(RpcError::new("conflict", "view already exists"));
                }
                let path = plugins::package_file(&plugin.directory, string(params, "file")?)?;
                let properties: BTreeMap<String, Value> =
                    serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                let callbacks = serde_json::from_value::<Vec<String>>(
                    params.get("callbacks").cloned().unwrap_or(json!([])),
                )
                .map_err(|e| RpcError::invalid(e.to_string()))?;
                let view_key = format!("w:{id}");
                let mut values = Vec::new();
                for (name, value) in &properties {
                    match self.prop_value(reply.owner, &view_key, name, value) {
                        Ok(value) => values.push((name.clone(), value)),
                        Err(error) => {
                            self.images.release_view(reply.owner, &view_key);
                            return Err(error);
                        }
                    }
                }
                let (theme, fill) = (self.theme().to_owned(), self.vestra_fill.clone());
                let missed = std::cell::Cell::new(false);
                let view = ui::open(
                    &path,
                    reply.owner,
                    &id,
                    &values,
                    &callbacks,
                    self.events.clone(),
                    |instance| {
                        if theme != "vestra"
                            && theme::apply(instance, &theme, fill.as_ref()).is_err()
                        {
                            missed.set(true);
                        }
                    },
                )
                .inspect_err(|_| self.images.release_view(reply.owner, &view_key))?;
                self.theme_pending |= missed.get();
                self.views.insert((reply.owner, id), view);
                Value::Null
            }
            "ui.set" | "ui.show" | "ui.close" => {
                let key = (reply.owner, string(params, "id")?.to_owned());
                if !self.views.contains_key(&key) {
                    return Err(RpcError::new("missing", "view not found"));
                }
                let next = match method {
                    "ui.set" => Some(self.prop_value(
                        reply.owner,
                        &format!("w:{}", key.1),
                        string(params, "property")?,
                        &params["value"],
                    )?),
                    _ => None,
                };
                let view = &self.views[&key];
                match method {
                    "ui.set" => {
                        let name = string(params, "property")?;
                        let next = next.unwrap_or_default();
                        let current = view
                            .instance
                            .get_property(name)
                            .map_err(|e| RpcError::new("ui", e.to_string()))?;
                        if !ui_values::reconcile(&current, &next)? {
                            view.instance
                                .set_property(name, next)
                                .map_err(|e| RpcError::new("ui", e.to_string()))?;
                        }
                    }
                    "ui.show" => {
                        if params["visible"]
                            .as_bool()
                            .ok_or_else(|| RpcError::invalid("visible requires boolean"))?
                        {
                            view.instance.show().map_err(anyhow::Error::from)?;
                            platform::shown(view.instance.window());
                        } else {
                            view.instance.hide().map_err(anyhow::Error::from)?;
                        }
                    }
                    _ => {
                        view.instance.hide().map_err(anyhow::Error::from)?;
                        self.images.release_view(key.0, &format!("w:{}", key.1));
                        self.views.remove(&key);
                    }
                }
                Value::Null
            }
            _ => unreachable!(),
        };
        Ok(Some(value))
    }
}
/// The window policy's slot (the backdrop has no preference or auto-hide).
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
) -> Result<slint_interpreter::Value> {
    *path = menu::valid_path(roots, path);
    let (origin, size) = menu_frame(window);
    ui_values::from_json(&menu::levels(roots, path, origin, size, scroll, labels))
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
                eprintln!("keys: winit modifiers {:?}", state.state());
            }
            None
        }
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => {
            if debug_keys() {
                eprintln!(
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
                eprintln!("keys: dead key → {dead:?}");
            }
            input.borrow_mut().physical = Some(physical);
            dead.map(|input| (input, event.repeat))
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
    let Some(target) = shared.table.resolve(event, panel) else {
        return false;
    };
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
/// state publications. Reconcile in place so unchanged panels keep their live
/// elements; a changed level replaces only that level's row.
fn set_menu_model(
    window: &slint_interpreter::ComponentInstance,
    next: slint_interpreter::Value,
) -> Result<()> {
    let current = window.get_property("menu-levels")?;
    if !ui_values::reconcile(&current, &next)? {
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
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut directories = Vec::<PathBuf>::new();
    let mut headless = false;
    let mut silent = false;
    let mut seconds = None;
    let mut snapshot = None;
    let mut frame_evidence = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frame-evidence-dir" => {
                frame_evidence =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--frame-evidence-dir requires path")
                    })?));
            }
            "--snapshot" => {
                snapshot =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--snapshot requires PNG path")
                    })?))
            }
            "--headless" => headless = true,
            "--silent" => silent = true,
            "--plugin" => directories
                .push(PathBuf::from(args.next().ok_or_else(|| {
                    anyhow::anyhow!("--plugin requires directory")
                })?)),
            "--seconds" => {
                seconds = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--seconds requires number"))?
                        .parse::<f64>()?,
                )
            }
            _ => bail!(
                "unknown argument {arg}; use --plugin DIR [--headless] [--silent] [--seconds N]"
            ),
        }
    }
    if frame_evidence.is_some() && !headless {
        bail!("--frame-evidence-dir is headless diagnostic only");
    }
    let mut evidence = frame_evidence.map(FrameEvidence::new).transpose()?;
    // GPU rendering (reference §5.4) needs Slint's OpenGL renderer (femtovg);
    // headless, `VESTRA_SOFTWARE_RENDER=1` and other systems render in software.
    let mut gpu = renderer::wanted(
        headless,
        std::env::var_os("VESTRA_SOFTWARE_RENDER").as_deref(),
    );
    if !headless {
        let winit = || slint::BackendSelector::new().backend_name("winit".into());
        if gpu && let Err(error) = winit().renderer_name("femtovg".into()).select() {
            eprintln!(
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
        mount_events,
        mount_input,
        last_image: None,
        shell_generation: 1,
        next_shell_generation: 1,
        menu_path: Vec::new(),
        menu_scroll: Vec::new(),
        menu_delay: menu::HoverDelay::default(),
        menu_show_delay_ms: platform::menu_show_delay_ms(),
        policy: window_state::Policy::new(0),
        clock: Instant::now(),
        window_facts: Value::Null,
        measured_shell: None,
        menu_open: false,
        sidebar_popped: false,
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
        window_title: "Vestra".into(),
        activations: package_activation::Book::default(),
        activation_ui: Default::default(),
        activation_receipts: Default::default(),
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
        settings: Default::default(),
        settings_category: None,
        vestra_fill: None,
        theme_pending: false,
        commands: commands::Registry::new(),
    };
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
    let start = Instant::now();
    if headless {
        loop {
            if seconds.is_some_and(|seconds| start.elapsed().as_secs_f64() >= seconds) {
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
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
            if let Some(evidence) = evidence.as_mut() {
                evidence.sample(&host.backend, &observed_source, start)?;
            }
        }
    } else {
        let native_before = platform::capture_windows().ok();
        let window = composition::compile_json(&[], host.mount_events.clone(), 1)?;
        if let Some(frame) = host.ui_state.state.window {
            window.window().set_size(slint::LogicalSize::new(
                frame.width.max(320.) as f32,
                frame.height.max(182.) as f32,
            ));
            window
                .window()
                .set_position(slint::LogicalPosition::new(frame.x as f32, frame.y as f32));
        }
        host.attach_gpu(&window);
        window.show()?;
        platform::shown(window.window());
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
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let mut host = tick_host.borrow_mut();
                for event in receiver.try_iter().take(128) {
                    host.handle(event);
                }
                host.mount_callbacks();
                host.prepare_mount_tick();
                host.window_tick();
                host.activation_tick();
                host.power_tick();
                host.package_tick();
                host.notify_tick();
                host.timeline_tick();
                host.drops_tick();
                host.settings_tick();
                if host.theme_pending {
                    host.theme_all();
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
                            eprintln!("snapshot failed: {error}");
                        }
                    }
                    let _ = slint::quit_event_loop();
                }
            },
        );
        window.run()?;
        timer.stop();
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
