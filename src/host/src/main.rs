#![forbid(unsafe_code)]
mod backend;
mod commands;
pub mod composition;
mod effects;
pub mod font_asset;
pub mod font_carrier;
mod media_jobs;
mod media_tracks;
mod media_worker;
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
#[cfg(feature = "producer-audio-probe")]
mod producer;
#[cfg(feature = "producer-audio-probe")]
mod producer_audio;
#[cfg(feature = "producer-audio-probe")]
mod producer_controls;
#[cfg(feature = "producer-audio-probe")]
pub mod producer_input;
#[cfg(feature = "producer-audio-probe")]
mod producer_input_delivery;

#[cfg(feature = "producer-audio-probe")]
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
mod ui_values;
pub mod window_state;
use anyhow::{Result, bail};
use backend::{Backend, Command};
use plugins::Plugin;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slint::ComponentHandle;
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
    Reset,
    Back,
    Dismiss,
}
enum Event {
    #[cfg(feature = "producer-audio-probe")]
    PhysicalInput(u64, Value),
    WindowInput(u64, String),
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
}
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
    sidebar_resize_requested: bool,
    sidebar_resize: Value,
    activations: package_activation::Book,
    activation_ui: std::collections::BTreeSet<(u64, u64)>,
    activation_receipts: std::collections::VecDeque<Value>,
    activation_last: BTreeMap<u64, package_activation::Status>,
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
        self.activation_last.remove(&owner);
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
            if self.activation_last.get(&owner) != Some(&status) {
                self.activation_last.insert(owner, status.clone());
                if let Ok(receipt) = self.activation(owner) {
                    self.broadcast("plugins.activation", receipt);
                }
            }
            if terminal_failure {
                self.disable(owner);
            }
        }
    }
    #[cfg(feature = "producer-audio-probe")]
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
    #[cfg(feature = "producer-audio-probe")]
    fn physical_revoke(&self, reason: producer_input::RevokeReason) {
        self.physical_input(json!({"kind":"revoke","reason":reason}));
    }
    fn load(&mut self, directory: &Path, timeout_ms: u64) -> Result<Value> {
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
        let mut manifest = serde_json::to_value(&plugin.manifest)?;
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
        self.broadcast("plugins.changed", self.plugin_list());
        manifest["owner"] = self.next_generation.into();
        manifest["accepted"] = true.into();
        manifest["activation"] = json!({"status":"pending","runtimeReady":false,"pendingUi":0});
        Ok(manifest)
    }
    fn plugin_list(&self) -> Value {
        json!(
            self.plugins
                .values()
                .map(|p| &p.manifest)
                .collect::<Vec<_>>()
        )
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
        self.broadcast("plugins.changed", self.plugin_list());
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
            #[cfg(feature = "producer-audio-probe")]
            Event::PhysicalInput(generation, params) => {
                if generation == self.shell_generation {
                    self.physical_input(params);
                }
            }
            Event::WindowInput(generation, kind) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.activity(self.now_ms());
                if let Some(shortcut) = kind.strip_prefix("shortcut:") {
                    self.canvas_shortcut(shortcut);
                    return;
                }
                if (kind == "toggleFullscreen" || kind == "escape")
                    && let Some(shell) = &self.shell
                {
                    if kind == "toggleFullscreen" {
                        shell
                            .window()
                            .set_fullscreen(!shell.window().is_fullscreen());
                    } else if self.menu_open {
                        let result = shell
                            .invoke("close-menu", &[])
                            .map(|_| ())
                            .map_err(anyhow::Error::from);
                        self.report_menu(result);
                        self.menu_open = false;
                    } else {
                        shell.window().set_fullscreen(false);
                    }
                }
            }
            Event::MenuAction(generation, action) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.activity(self.now_ms());
                match action {
                    MenuAction::Reset => {
                        #[cfg(feature = "producer-audio-probe")]
                        self.physical_revoke(producer_input::RevokeReason::Menu);
                        self.menu_open = true;
                        self.menu_path.clear();
                        let result = self.refresh_menu();
                        self.report_menu(result);
                    }
                    MenuAction::Back => {
                        self.menu_path.pop();
                        let result = self.refresh_menu();
                        self.report_menu(result);
                    }
                    MenuAction::Dismiss => {
                        self.menu_open = false;
                        if let Some(shell) = &self.shell {
                            let result = shell
                                .invoke("close-menu", &[])
                                .map(|_| ())
                                .map_err(anyhow::Error::from);
                            self.report_menu(result);
                        }
                    }
                    MenuAction::Invoke(token) => {
                        if let Some(group) = token.strip_prefix("g:") {
                            if let Ok(path) = serde_json::from_str::<Vec<String>>(group) {
                                self.menu_path = path;
                                let result = self.refresh_menu();
                                self.report_menu(result);
                            }
                        } else if let Some(command) = token.strip_prefix("c:")
                            && let Some((owner, id)) = command.split_once(':')
                            && let Ok(owner) = owner.parse::<u64>()
                        {
                            self.menu_open = false;
                            if let Some(shell) = &self.shell {
                                let result = shell
                                    .invoke("close-menu", &[])
                                    .map(|_| ())
                                    .map_err(anyhow::Error::from);
                                self.report_menu(result);
                            }
                            if let Err(error) = self.execute_command(owner, id) {
                                eprintln!("menu execute: {:?}", error);
                            }
                        }
                    }
                }
            }
            Event::Reply(reply, result) => self.respond(reply, result),
            Event::Broadcast(event, data) => self.broadcast(&event, data),
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
        if native.is_some_and(|facts| !facts.active) && self.menu_open {
            self.menu_open = false;
            let result = shell
                .invoke("close-menu", &[])
                .map(|_| ())
                .map_err(Into::into);
            self.report_menu(result);
            self.policy.activity(now);
        }
        self.policy.set_fullscreen(fullscreen, now);
        self.policy
            .set_obstructions(self.menu_open, self.dialog_busy);
        if let Err(error) =
            shell.set_property("input-blocked", (self.menu_open || self.dialog_busy).into())
        {
            eprintln!("host.error physical-input bridge: {error}");
        }
        let number = |name: &str| match shell.get_property(name) {
            Ok(slint_interpreter::Value::Number(n)) => n,
            _ => 0.,
        };
        let viewport = window_state::Viewport {
            canvas_available_width: number("canvas-space-width"),
            canvas_available_height: number("canvas-space-height"),
        };
        if self.sidebar_resize_requested {
            self.sidebar_resize_requested = false;
            let size = shell.window().size();
            let scale = shell.window().scale_factor() as f64;
            let width = size.width as f64 / scale;
            let height = size.height as f64 / scale;
            if viewport.canvas_available_width >= 880. && viewport.canvas_available_height >= 360. {
                self.sidebar_resize = json!({"status":"alreadyFits"});
            } else if fullscreen {
                self.sidebar_resize = json!({"status":"screenLimit"});
            } else if let Some(facts) = native {
                if (scale - facts.backing_scale_factor).abs() > 0.01 {
                    self.sidebar_resize = json!({"status":"screenLimitUnavailable","reason":"native/Slint scale mismatch"});
                } else {
                    let chrome_w = facts.native_frame.width - facts.content_rect.width;
                    let chrome_h = facts.native_frame.height - facts.content_rect.height;
                    let desired_w = width.max(880. + width - viewport.canvas_available_width);
                    let desired_h = height.max(360. + height - viewport.canvas_available_height);
                    let target_w = desired_w.min(facts.visible_frame.width - chrome_w);
                    let target_h = desired_h.min(facts.visible_frame.height - chrome_h);
                    if target_w > 0. && target_h > 0. {
                        let outer_w = target_w + chrome_w;
                        let outer_h = target_h + chrome_h;
                        let x = facts.native_frame.x.clamp(
                            facts.visible_frame.x,
                            facts.visible_frame.x + facts.visible_frame.width - outer_w,
                        );
                        let y = facts.native_frame.y.clamp(
                            facts.visible_frame.y,
                            facts.visible_frame.y + facts.visible_frame.height - outer_h,
                        );
                        let old = shell.window().position();
                        let dx = (x - facts.native_frame.x) * scale;
                        let dy = (facts.native_frame.y + facts.native_frame.height - y - outer_h)
                            * scale;
                        shell.window().set_size(slint::PhysicalSize::new(
                            (target_w * scale).round() as u32,
                            (target_h * scale).round() as u32,
                        ));
                        shell.window().set_position(slint::PhysicalPosition::new(
                            old.x.saturating_add(dx.round() as i32),
                            old.y.saturating_add(dy.round() as i32),
                        ));
                        self.sidebar_resize = json!({"status":"requested","limited":target_w<desired_w || target_h<desired_h,"targetLogicalWidth":target_w,"targetLogicalHeight":target_h});
                    } else {
                        self.sidebar_resize = json!({"status":"screenLimit"});
                    }
                }
            } else {
                self.sidebar_resize = json!({"status":"screenLimitUnavailable","reason":"host native screen association unavailable"});
            }
        }
        if self.measured_shell != Some(self.shell_generation)
            && (viewport.canvas_available_width <= 0. || viewport.canvas_available_height <= 0.)
        {
            return;
        }
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            shell.set_property("host-fullscreen", fullscreen.into())?;
            shell.set_property("menu-open", self.menu_open.into())?;
            shell.set_property(
                "bottom-visible",
                snapshot.bottom.is_some_and(|s| s.effective).into(),
            )?;
            shell.set_property(
                "sidebar-visible",
                snapshot.sidebar.is_some_and(|s| s.effective).into(),
            )?;
            shell.set_property("sidebar-width", snapshot.sidebar_width.into())?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"logicalWidth":number("logical-width"),"logicalHeight":number("logical-height"),"canvasAvailableWidth":viewport.canvas_available_width,"canvasAvailableHeight":viewport.canvas_available_height,"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"sidebarResize":self.sidebar_resize,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot}),
            )
        })();
        match result {
            Ok(facts) => {
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
        #[cfg(feature = "producer-audio-probe")]
        {
            use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
            let events = self.events.clone();
            let routing = self.backend.input.clone();
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
                            if matches!(code.as_str(), "F11" | "Escape") {return EventResult::Propagate;}
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
        #[cfg(feature = "producer-audio-probe")]
        let routing = self.backend.input.clone();
        window.set_callback("window-input", move |args| {
            if let Some(slint_interpreter::Value::String(kind)) = args.first() {
                #[cfg(feature = "producer-audio-probe")]
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
        let events = self.events.clone();
        window.set_callback("menu-invoke", move |args| {
            if let Some(slint_interpreter::Value::String(token)) = args.first() {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Invoke(token.to_string()),
                ));
            }
            slint_interpreter::Value::Void
        })?;
        for callback in ["menu-reset", "menu-back", "menu-dismiss"] {
            let events = self.events.clone();
            window.set_callback(callback, move |_| {
                let action = match callback {
                    "menu-reset" => MenuAction::Reset,
                    "menu-back" => MenuAction::Back,
                    _ => MenuAction::Dismiss,
                };
                let _ = events.send(Event::MenuAction(generation, action));
                slint_interpreter::Value::Void
            })?;
        }
        window.set_property(
            "menu-entries",
            menu_values(&self.commands.snapshot().roots, &self.menu_path)?,
        )?;
        window.set_property("menu-can-back", (!self.menu_path.is_empty()).into())?;
        if let Some(image) = &self.last_image {
            window.set_property("video-frame", image.clone().into())?;
        }
        Ok(())
    }
    fn refresh_menu(&self) -> Result<()> {
        if let Some(shell) = &self.shell {
            set_menu_model(
                shell,
                menu_values(&self.commands.snapshot().roots, &self.menu_path)?,
            )?;
            shell.set_property("menu-can-back", (!self.menu_path.is_empty()).into())?;
        }
        Ok(())
    }
    fn prepare_mount_tick(&mut self) {
        if self.preparing.is_none() {
            if let Some(change) = self.mount_queue.pop_front() {
                if let Some(shell) = &self.shell {
                    let _ = shell.set_property(
                        "diagnostic",
                        slint_interpreter::Value::String("Preparing plugin UI…".into()),
                    );
                }
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
                shell.set_property(property, mount.visible.into())?;
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
                #[cfg(feature = "producer-audio-probe")]
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
                        if let Err(error) = self.policy.attach(slot, *id, mount.visible) {
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
                self.mounts = next;
                self.shell_generation = candidate_generation;
                self.menu_open = false;
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
            if event.shell_generation != self.shell_generation
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
    fn canvas_shortcut(&mut self, shortcut: &str) {
        if self.menu_open
            || self.dialog_busy
            || !self.shell.as_ref().is_some_and(|shell| {
                matches!(
                    shell.get_property("canvas-focused"),
                    Ok(slint_interpreter::Value::Bool(true))
                )
            })
        {
            return;
        }
        #[cfg(feature = "producer-audio-probe")]
        if self
            .backend
            .input
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|routing| routing.captured)
        {
            return;
        }
        if let Some(execution) = self.commands.resolve_shortcut(shortcut) {
            let _ = self.execute_command(execution.owner, &execution.local_id);
        }
    }
    fn execute_command(&mut self, owner: u64, id: &str) -> std::result::Result<(), RpcError> {
        let execution = self
            .commands
            .resolve_execution(owner, id)
            .map_err(|e| RpcError::new("command", e.to_string()))?;
        self.send(execution.owner,json!({"event":"commands.execute","data":{"id":execution.local_id,"revision":execution.revision}}));
        Ok(())
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
            "host.capabilities" => None,
            method if method.starts_with("sourceResolvers.") => Some("source.resolve"),
            "dialogs.open" => Some("ui.dialog"),
            "window.state" | "window.setFullscreen" => Some("ui.window"),
            "ui.mount" | "ui.setMount" | "ui.showMount" | "ui.unmount" | "ui.mountState"
            | "ui.interaction" => Some("ui.mount"),
            "commands.register" | "commands.update" | "commands.snapshot" | "commands.execute" => {
                Some("commands")
            }
            "playback.state" | "source.state" | "source.operation" | "processing.state" => {
                Some("playback.read")
            }
            "playback.open" | "playback.seek" | "playback.set" | "source.submit" => {
                Some("playback.control")
            }
            "producer.prepare"
            | "producer.state"
            | "producer.release"
            | "producer.ack"
            | "producer.declareInput"
            | "producer.ackInput" => Some("producer.av"),
            "snapshot.state" | "jobs.state" | "jobs.list" | "jobs.operation" => Some("media.read"),
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
            "plugins.list" | "plugins.load" | "plugins.disable" | "plugins.activation" => {
                Some("plugins.manage")
            }
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
            "host.capabilities" => {
                json!({"apiVersion":plugins::API,"experimentalCapabilities":["source.intent/0.1","processing.resources/0.1","processing.operations/0.1","subtitles.resources/0.1","media.raw-jobs/0.1","media.processed-png/0.1","media.clip-mp4/0.1","media.sample-time/0.1","media.native-plain-srt/0.1"],"capabilities":plugins::CAPABILITIES,"backend":"libmpv-software","ui":!self.headless})
            }
            "window.state" => self.window_facts.clone(),
            "window.setFullscreen" => {
                let fullscreen = params["fullscreen"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("fullscreen boolean required"))?;
                let shell = self
                    .shell
                    .as_ref()
                    .ok_or_else(|| RpcError::new("unavailable", "native shell unavailable"))?;
                shell.window().set_fullscreen(fullscreen);
                self.policy.activity(self.now_ms());
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
                    Some(Mount {
                        spec: composition::CompileSpec {
                            owner: reply.owner,
                            file,
                            component: string(params, "component")?.into(),
                            global: Some(string(params, "global")?.into()),
                            slot,
                        },
                        properties,
                        visible: true,
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
                        if visible && slot == window_state::Slot::Sidebar {
                            self.sidebar_resize_requested = true;
                        }
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
                #[cfg(feature = "producer-audio-probe")]
                self.physical_revoke(producer_input::RevokeReason::Dialog);
                return Ok(None);
            }
            "commands.snapshot" => json!(self.commands.snapshot()),
            "commands.register" | "commands.update" => {
                let id = string(params, "id")?;
                let revision = params["revision"]
                    .as_u64()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| RpcError::invalid("positive revision required"))?;
                let state = serde_json::from_value(params["state"].clone())
                    .map_err(|e| RpcError::invalid(e.to_string()))?;
                if method == "commands.register" {
                    let mut declaration_json = params["declaration"].clone();
                    if self.packages.is_trial(reply.owner)
                        && let Some(path) = declaration_json["path"].as_array_mut()
                        && path.len() > 1
                    {
                        path[1] =
                            format!("试载 {} · {}", reply.owner, path[1].as_str().unwrap_or(""))
                                .into();
                    }
                    if self.packages.is_trial(reply.owner) {
                        declaration_json["shortcut"] = Value::Null;
                    }
                    let declaration = serde_json::from_value(declaration_json)
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                    self.commands
                        .register(reply.owner, id.into(), revision, declaration, state)
                } else {
                    self.commands.update(reply.owner, id, revision, state)
                }
                .map_err(|e| RpcError::new("command", e.to_string()))?;
                json!({"revision":self.commands.snapshot().revision})
            }
            "commands.execute" => {
                self.execute_command(reply.owner, string(params, "id")?)?;
                Value::Null
            }
            "playback.state" => self.backend.state.lock().unwrap().clone(),
            "sourceResolvers.register"
            | "sourceResolvers.unregister"
            | "sourceResolvers.respond"
            | "playback.open"
            | "playback.seek"
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
            | "processing.state" => {
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
                    "playback.state",
                    "playback.backend",
                    "source.state",
                    "plugins.changed",
                    "host.error",
                    "window.state",
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
            "plugins.list" => self.plugin_list(),
            "plugins.activation" => self.activation(
                params["owner"]
                    .as_u64()
                    .filter(|owner| *owner > 0 && *owner <= 9_007_199_254_740_991)
                    .ok_or_else(|| RpcError::invalid("positive safe owner required"))?,
            )?,
            "plugins.load" => self.load(
                Path::new(string(params, "directory")?),
                params
                    .get("activationTimeoutMs")
                    .map_or(Ok(120_000), |value| {
                        value
                            .as_u64()
                            .ok_or_else(|| {
                                RpcError::invalid("activationTimeoutMs requires integer")
                            })
                            .and_then(|timeout| {
                                if (1_000..=120_000).contains(&timeout) {
                                    Ok(timeout)
                                } else {
                                    Err(RpcError::invalid(
                                        "activationTimeoutMs must be 1000..120000",
                                    ))
                                }
                            })
                    })?,
            )?,
            "plugins.disable" => {
                let id = string(params, "id")?;
                let owner = self
                    .plugins
                    .iter()
                    .find(|(_, plugin)| plugin.manifest.id == id)
                    .map(|(key, _)| *key)
                    .ok_or_else(|| RpcError::new("missing", "plugin not running"))?;
                // Reply to self-disable before severing its pipe.
                if owner == reply.owner {
                    self.respond(reply.clone(), Ok(Value::Null));
                    self.disable(owner);
                    return Ok(None);
                }
                self.disable(owner);
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
/// Registry revisions include changes outside this visible layer and identical
/// state publications. Keep the live model on equal projection, so pointer and
/// keyboard focus are not discarded on every playback state event. A genuinely
/// changed menu layer still replaces the model and resets its cursor normally.
fn set_menu_model(
    window: &slint_interpreter::ComponentInstance,
    next: slint_interpreter::Value,
) -> Result<()> {
    let current = window.get_property("menu-entries")?;
    if ui_values::to_json(&current)? != ui_values::to_json(&next)? {
        window.set_property("menu-entries", next)?;
    }
    Ok(())
}
fn menu_values(roots: &[commands::MenuNode], path: &[String]) -> Result<slint_interpreter::Value> {
    let mut nodes = roots;
    for title in path {
        nodes = nodes
            .iter()
            .find_map(|n| match n {
                commands::MenuNode::Group { title: t, children } if t == title => {
                    Some(children.as_slice())
                }
                _ => None,
            })
            .unwrap_or(&[]);
    }
    let output=nodes.iter().map(|node|match node {
        commands::MenuNode::Group{title,..}=>{
            let mut next=path.to_vec(); next.push(title.clone());
            json!({"token":format!("g:{}",json!(next)),"title":title,"enabled":true,"checked":false,"busy":false,"reason":"","has-children":true})
        }
        commands::MenuNode::Command{entry}=>json!({"token":format!("c:{}:{}",entry.key.owner,entry.key.local_id),"title":entry.declaration.title,"enabled":entry.state.enabled,"checked":entry.state.checked.unwrap_or(false),"busy":entry.state.busy,"reason":entry.state.reason.clone().unwrap_or_default(),"has-children":false})
    }).collect::<Vec<_>>();
    ui_values::from_json(&json!(output))
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
        sidebar_resize_requested: false,
        sidebar_resize: Value::Null,
        activations: package_activation::Book::default(),
        activation_ui: Default::default(),
        activation_receipts: Default::default(),
        activation_last: Default::default(),
        packages: package_manager::Manager::open()?,
        cleanup: Default::default(),
        commands: commands::Registry::new(
            [
                "文件",
                "播放",
                "视频",
                "音频",
                "字幕",
                "视图",
                "模拟器",
                "插件",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        )?,
    };
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
                if let Some(window) = host.shell.as_ref() {
                    let snapshot = host.commands.snapshot();
                    if snapshot.revision != menu_revision {
                        menu_revision = snapshot.revision;
                        if let Err(error) = menu_values(&snapshot.roots, &host.menu_path)
                            .and_then(|value| set_menu_model(window, value))
                        {
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
                    let state = host.backend.state.lock().unwrap();
                    let diagnostic = if host.preparing.is_some() {
                        "Preparing plugin UI…".into()
                    } else {
                        format!(
                            "libmpv · session {} · {:.2}s · embedded frame {}",
                            state["session"],
                            state["position"].as_f64().unwrap_or(0.0),
                            frame.sequence
                        )
                    };
                    let _ = window.set_property(
                        "diagnostic",
                        slint_interpreter::Value::String(diagnostic.as_str().into()),
                    );
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
        // Native windows/resources must be released before process teardown.
        host.borrow_mut().views.clear();
        host.borrow_mut().shell = None;
    }
    Ok(())
}
