#![forbid(unsafe_code)]
mod backend;
mod bindings;
mod commands;
pub mod composition;
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
pub mod platform_window;
mod plugins;
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
mod session;
mod source_backend;
mod source_geometry;
pub mod source_lease;
mod source_resolvers;
mod subtitle_backend;
pub mod subtitle_format;
pub mod subtitle_state;
pub mod subtitle_style;
pub mod subtitle_upload;
mod ui;
mod ui_state;
mod ui_values;
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
    sync::mpsc,
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
    Hover(usize, String),
    Reset,
    Back,
    Dismiss,
}
enum Event {
    #[cfg(feature = "producer-audio")]
    PhysicalInput(u64, Value),
    WindowInput(u64, String),
    /// A bound input fired: (window generation, owner, command id).
    Binding(u64, u64, String),
    SidebarResized(u64, f64),
    FloatingClosed(u64),
    MenuAction(u64, MenuAction),
    Request(u64, Value),
    Reply(Reply, std::result::Result<Value, RpcError>),
    Broadcast(String, Value),
    UiCallback(u64, Value),
    PluginMessage(u64, String, Value),
    Exited(u64),
    Reaped(u64),
    BackendCleaned(u64),
    DialogDone(Reply, Option<Vec<PathBuf>>),
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
}
/// Read synchronously by the shell's key/mouse callbacks, without borrowing Host.
#[derive(Default)]
struct InputShared {
    table: bindings::Table,
    bottom: Option<u64>,
    sidebar: Option<u64>,
    blocked: bool,
    wheel: (f64, f64),
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
    policy: window_state::Policy,
    clock: Instant,
    window_facts: Value,
    measured_shell: Option<u64>,
    menu_open: bool,
    native_window: Option<platform_window::HostWindowId>,
    native_before: Option<platform_window::NativeSnapshot>,
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
        self.mount_queue.retain(|c| c.reply.owner != owner);
        if self
            .preparing
            .as_ref()
            .is_some_and(|(_, c)| c.reply.owner == owner)
        {
            self.preparing = None;
        }
        if let Some(mount) = self.mounts.remove(&owner) {
            if let Some(shell) = &self.shell {
                let _ = shell.set_property(
                    match mount.spec.slot {
                        composition::Slot::Bottom => "bottom-visible",
                        composition::Slot::Sidebar => "sidebar-visible",
                    },
                    false.into(),
                );
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
                            Err(error) => self.respond(reply, Err(error)),
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
                        self.menu_path.pop();
                        let result = self.refresh_menu();
                        self.report_menu(result);
                    }
                    MenuAction::Dismiss => self.close_menu(),
                    MenuAction::Hover(level, token) => {
                        let mut path = match menu::parse_token(&token) {
                            Some(menu::Token::Group(path)) => path,
                            Some(menu::Token::Command { .. }) => {
                                self.menu_path.iter().take(level).cloned().collect()
                            }
                            None => return,
                        };
                        path.truncate(level + 1);
                        if path != self.menu_path {
                            self.menu_path = path;
                            let result = self.refresh_menu();
                            self.report_menu(result);
                        }
                    }
                    MenuAction::Invoke(token) => match menu::parse_token(&token) {
                        Some(menu::Token::Group(path)) => {
                            self.menu_path = path;
                            let result = self.refresh_menu();
                            self.report_menu(result);
                        }
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
                    self.last_source = data.clone();
                    self.update_title();
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
            Event::FloatingClosed(generation) => {
                if let Some(owner) = self
                    .floating
                    .as_ref()
                    .filter(|f| f.generation == generation)
                    .map(|f| f.owner)
                {
                    let _ = self
                        .policy
                        .set_preferred(window_state::Slot::Sidebar, owner, false);
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
            Event::UiCallback(owner, data) => {
                self.send(owner, json!({"event":"ui.callback","data":data}))
            }
            Event::DialogDone(reply, paths) => {
                self.policy.activity(self.now_ms());
                self.dialog_busy = false;
                self.respond(
                    reply,
                    Ok(json!({"cancelled":paths.is_none(),"paths":paths.unwrap_or_default()})),
                );
            }
            Event::Exited(owner) => self.disable(owner),
            Event::Reaped(owner) => {
                if let Some(cleanup) = self.cleanup.get_mut(&owner) {
                    cleanup.0 = true;
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
                self.disable(owner);
            }
        }
    }
    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn window_tick(&mut self) {
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        let now = self.now_ms();
        let fullscreen = shell.window().is_fullscreen();
        if self.native_window.is_none()
            && let Some(before) = &self.native_before
        {
            match platform_window::associate_new(before) {
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
            .and_then(|id| platform_window::screen_for(id).ok());
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
        if self.measured_shell != Some(self.shell_generation)
            && (viewport.width <= 0. || viewport.height <= 0.)
        {
            return;
        }
        if !fullscreen && !self.headless {
            self.remember_window(&shell);
        }
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            shell.set_property("host-fullscreen", fullscreen.into())?;
            shell.set_property(
                "menu-open",
                (self.menu_open && self.menu_generation == self.shell_generation).into(),
            )?;
            let bottom = snapshot.bottom.is_some_and(|s| s.effective);
            shell.set_property("bottom-visible", bottom.into())?;
            shell.set_property(
                "sidebar-visible",
                snapshot
                    .sidebar
                    .is_some_and(|s| s.effective && !s.detached)
                    .into(),
            )?;
            let playing =
                self.last_source["phase"] == "ready" && self.last_source["paused"] == false;
            // UI-SHELL-09: the pointer hides with the idle fullscreen bar while playing.
            let idle = snapshot
                .bottom
                .is_some_and(|s| s.reason == Some(window_state::Reason::Idle));
            shell.set_property("cursor-hidden", (fullscreen && idle && playing).into())?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot}),
            )
        })();
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
        for (name, value) in &mount.properties {
            composition::set_json_global(&instance, &mount.spec, name, value)?;
        }
        self.bind_menu(&instance, generation, false)?;
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
        instance.show()?;
        self.floating = Some(Floating {
            instance,
            owner,
            generation,
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
        #[cfg(feature = "producer-audio")]
        {
            use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
            let events = self.events.clone();
            let routing = self.backend.input.clone();
            let input = self.input.clone();
            let weak = window.as_weak();
            window.window().on_winit_window_event(move |_, event| {
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
            menu_levels(window, &roots, &mut self.menu_path.clone(), &labels)?,
        )?;
        if let Some(image) = &self.last_image {
            window.set_property("video-frame", image.clone().into())?;
        }
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
            if let [SValue::Number(level), SValue::String(token)] = args
                && *level >= 0.
            {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Hover(*level as usize, token.to_string()),
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
            let Some(event) = bindings::from_slint_key(text, flag(0), flag(1), flag(2), flag(3))
            else {
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
        if let Some(window) = self.window_for(generation) {
            let _ = window.set_property("menu-open", true.into());
        }
        let result = self.refresh_menu();
        self.report_menu(result);
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
                menu_levels(window, &roots, &mut self.menu_path, &labels)?
            }
            MenuMode::Panel(owner, items) => {
                let entries: Vec<_> = items
                    .iter()
                    .map(|id| id.as_deref().and_then(|id| self.commands.get(*owner, id)))
                    .collect();
                let (origin, size) = menu_frame(window);
                ui_values::from_json(&menu::context_level(&entries, origin, size, &labels))?
            }
        };
        set_menu_model(window, model)
    }
    fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_path.clear();
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
        let native_before = platform_window::capture_windows().ok();
        let build = || -> Result<slint_interpreter::ComponentInstance> {
            let specs = next.values().map(|m| m.spec.clone()).collect::<Vec<_>>();
            let shell =
                composition::compile_json(&specs, self.mount_events.clone(), candidate_generation)?;
            for mount in next.values() {
                for (name, value) in &mount.properties {
                    composition::set_json_global(&shell, &mount.spec, name, value)?;
                }
                let property = match mount.spec.slot {
                    composition::Slot::Bottom => "bottom-visible",
                    composition::Slot::Sidebar => "sidebar-visible",
                };
                shell.set_property(property, (mount.visible && !mount.detached).into())?;
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
            shell.show()?;
            Ok(shell)
        };
        let result = match build() {
            Ok(shell) => {
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::WindowHidden);
                self.native_before = native_before.clone();
                self.native_window = native_before
                    .as_ref()
                    .and_then(|before| platform_window::associate_new(before).ok());
                if let Some(old) = self.shell.replace(shell) {
                    let _ = old.hide();
                }
                let old_owners = self.mounts.keys().copied().collect::<Vec<_>>();
                for old in old_owners {
                    if !next.contains_key(&old) {
                        self.policy.remove_owner(old);
                    }
                }
                for (id, mount) in &next {
                    if !self.mounts.contains_key(id) {
                        let slot = match mount.spec.slot {
                            composition::Slot::Bottom => window_state::Slot::Bottom,
                            composition::Slot::Sidebar => window_state::Slot::Sidebar,
                        };
                        if let Err(error) =
                            self.policy.attach(slot, *id, mount.visible, mount.detached)
                        {
                            eprintln!("mount policy attach: {error}");
                        }
                    }
                }
                for (id, mount) in &next {
                    let slot = match mount.spec.slot {
                        composition::Slot::Bottom => window_state::Slot::Bottom,
                        composition::Slot::Sidebar => window_state::Slot::Sidebar,
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
        fullscreen.bindings = vec!["Enter".into(), "F11".into(), "Mouse:LeftDoubleClick".into()];
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
        Ok(())
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
        Ok(json!({"bottom":slot("bottom"),"sidebar":slot("sidebar")}))
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
            method if method.starts_with("packages.") => Some("plugins.manage"),
            method if method.starts_with("sourceResolvers.") => Some("source.resolve"),
            "dialogs.open" => Some("ui.dialog"),
            "ui.mount" | "ui.setMount" | "ui.showMount" | "ui.unmount" | "ui.mountState"
            | "ui.interaction" | "ui.menu" | "layout.state" | "layout.set" => Some("ui.mount"),
            "osd.show" | "osd.panel" => Some("ui.osd"),
            "playback.stats" => Some("playback.read"),
            "sequence.set" | "sequence.step" => Some("playback.control"),
            "commands.register" | "commands.update" => Some("commands"),
            "source.state" | "source.operation" | "processing.state" => Some("playback.read"),
            "playback.set" | "source.submit" => Some("playback.control"),
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
            "events.subscribe" | "events.unsubscribe" => Some("events"),
            "ui.open" | "ui.set" | "ui.show" | "ui.close" => Some("ui.window"),
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
        if method.starts_with("subtitles.")
            || method.starts_with("snapshot.")
            || method.starts_with("jobs.")
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
            "layout.state" => self.layout_state()?,
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
                let policy_slot = match slot {
                    composition::Slot::Bottom => window_state::Slot::Bottom,
                    composition::Slot::Sidebar => window_state::Slot::Sidebar,
                };
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
                    self.policy
                        .set_preferred(policy_slot, owner, visible)
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
            "ui.mountState" => {
                let mount = self
                    .mounts
                    .get(&reply.owner)
                    .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
                let slot = match mount.spec.slot {
                    composition::Slot::Bottom => "bottom",
                    composition::Slot::Sidebar => "sidebar",
                };
                if self.window_facts["shellGeneration"].as_u64() != Some(self.shell_generation)
                    || self.window_facts["slots"][slot]["owner"].as_u64() != Some(reply.owner)
                {
                    return Err(RpcError::new(
                        "pending",
                        "current shell layout is not yet confirmed",
                    ));
                }
                self.window_facts["slots"][slot].clone()
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
                self.policy
                    .set_locks(
                        match mount.spec.slot {
                            composition::Slot::Bottom => window_state::Slot::Bottom,
                            composition::Slot::Sidebar => window_state::Slot::Sidebar,
                        },
                        reply.owner,
                        locks,
                    )
                    .map_err(anyhow::Error::from)?;
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
                        _ => return Err(RpcError::invalid("unknown mount slot")),
                    };
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
            "ui.setMount" | "ui.showMount" => {
                let mount = self
                    .mounts
                    .get_mut(&reply.owner)
                    .ok_or_else(|| RpcError::new("missing", "owner has no ready mount"))?;
                let shell = self
                    .shell
                    .as_ref()
                    .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
                if method == "ui.setMount" {
                    let property = string(params, "property")?;
                    composition::set_json_global(shell, &mount.spec, property, &params["value"])?;
                    if let Some(floating) =
                        self.floating.as_ref().filter(|f| f.owner == reply.owner)
                    {
                        composition::set_json_global(
                            &floating.instance,
                            &mount.spec,
                            property,
                            &params["value"],
                        )?;
                    }
                    mount
                        .properties
                        .insert(property.into(), params["value"].clone());
                } else {
                    let visible = params["visible"]
                        .as_bool()
                        .ok_or_else(|| RpcError::invalid("visible boolean required"))?;
                    let slot = match mount.spec.slot {
                        composition::Slot::Bottom => window_state::Slot::Bottom,
                        composition::Slot::Sidebar => window_state::Slot::Sidebar,
                    };
                    if params["scope"] == "presentation" {
                        self.policy
                            .reveal_sidebar_in_fullscreen(reply.owner, visible)
                            .map_err(anyhow::Error::from)?;
                    } else {
                        self.policy
                            .set_preferred(slot, reply.owner, visible)
                            .map_err(anyhow::Error::from)?;
                        mount.visible = visible;
                        self.remember_panel(reply.owner);
                    }
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
                        declaration.shortcut = None;
                    }
                    let manifest = plugin.manifest.id.clone();
                    let bindings = declaration.all_bindings();
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
                if !["playback.backend", "source.state", "window.state"].contains(&event) {
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
                let properties =
                    serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                let callbacks = serde_json::from_value::<Vec<String>>(
                    params.get("callbacks").cloned().unwrap_or(json!([])),
                )
                .map_err(|e| RpcError::invalid(e.to_string()))?;
                let view = ui::open(
                    &path,
                    reply.owner,
                    &id,
                    &properties,
                    &callbacks,
                    self.events.clone(),
                )?;
                self.views.insert((reply.owner, id), view);
                Value::Null
            }
            "ui.set" | "ui.show" | "ui.close" => {
                let key = (reply.owner, string(params, "id")?.to_owned());
                let view = self
                    .views
                    .get(&key)
                    .ok_or_else(|| RpcError::new("missing", "view not found"))?;
                match method {
                    "ui.set" => {
                        let name = string(params, "property")?;
                        let next = ui::from_json(&params["value"])?;
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
                        } else {
                            view.instance.hide().map_err(anyhow::Error::from)?;
                        }
                    }
                    _ => {
                        view.instance.hide().map_err(anyhow::Error::from)?;
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
/// Projects the open chain at the origin the view recorded for this opening.
fn menu_levels(
    window: &slint_interpreter::ComponentInstance,
    roots: &[commands::MenuNode],
    path: &mut Vec<String>,
    labels: &dyn Fn(u64, &str) -> Option<String>,
) -> Result<slint_interpreter::Value> {
    *path = menu::valid_path(roots, path);
    let (origin, size) = menu_frame(window);
    ui_values::from_json(&menu::levels(roots, path, origin, size, labels))
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
    if cfg!(target_os = "macos") {
        (meta, control)
    } else {
        (control, meta)
    }
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
        let frame = backend
            .frame
            .lock()
            .unwrap()
            .as_ref()
            .map(|f| (f.sequence, f.pixels.clone(), f.width, f.height));
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
    if !headless {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .select()?;
    }
    let mut observed_source = Value::Null;
    let (events, receiver) = mpsc::channel();
    let backend = Backend::start(events.clone(), silent)?;
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
        policy: window_state::Policy::new(0),
        clock: Instant::now(),
        window_facts: Value::Null,
        measured_shell: None,
        menu_open: false,
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
        commands: commands::Registry::new(),
    };
    host.register_host_commands()?;
    {
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
            host.package_tick();
            if let Some(evidence) = evidence.as_mut() {
                evidence.sample(&host.backend, &observed_source, start)?;
            }
        }
    } else {
        let native_before = platform_window::capture_windows().ok();
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
        window.show()?;
        host.native_before = native_before.clone();
        host.native_window = native_before
            .as_ref()
            .and_then(|before| platform_window::associate_new(before).ok());
        let host = Rc::new(RefCell::new(host));
        host.borrow().bind_shell(&window, 1)?;
        host.borrow_mut().shell = Some(window.clone_strong());
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
                host.package_tick();
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
                let frame = host.backend.frame.lock().unwrap().take();
                if let Some(frame) = frame {
                    let window = host.shell.as_ref().unwrap().clone_strong();
                    let buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(
                        &frame.pixels,
                        frame.width,
                        frame.height,
                    );
                    let image = slint::Image::from_rgb8(buffer);
                    host.last_image = Some(image.clone());
                    let _ = window.set_property("video-frame", image.into());
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
        host.borrow_mut().ui_state.flush();
        // Native windows/resources must be released before process teardown.
        host.borrow_mut().close_floating();
        host.borrow_mut().views.clear();
        host.borrow_mut().shell = None;
    }
    Ok(())
}
