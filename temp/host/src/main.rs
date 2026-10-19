// 实验版本：只验证 Rust/Slint 宿主与 TS 插件；与最终成品 API 可能存在很大差异。
#![forbid(unsafe_code)]

mod backend;
mod dialogs;
mod headless;
mod plugins;
mod retirement;
mod scenario;
mod services;
mod ui;

use backend::Backend;
use plugins::{Incoming, Manifest, Plugin};
use serde_json::{Value as Json, json};
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode};
use slint_interpreter::{ComponentInstance, Value};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

struct PendingInstall {
    caller: String,
    request_id: Json,
    previous: Option<String>,
    started: Instant,
}
struct Host {
    dialogs: dialogs::Dialogs,
    disposal_errors: Vec<String>,
    retiring: retirement::Retirement,
    dialog_fixtures: Option<std::collections::VecDeque<Option<PathBuf>>>,
    pending: std::collections::HashMap<String, PendingInstall>,
    install_serial: u64,
    window: ComponentInstance,
    backend: Backend,
    services: services::Services,
    plugins: Vec<Plugin>,
    sender: mpsc::Sender<Incoming>,
    frame: Image,
    last_broadcast: Instant,
    // 实验观测：首次空窗口计一次，隐藏/显示不递增。
    composition_count: usize,
    window_generation: usize,
    auto_media: Option<PathBuf>,
    received_frames: usize,
}

impl Host {
    fn report(&self, error: impl ToString) {
        let message = error.to_string();
        eprintln!("{message}");
        let _ = self
            .window
            .set_property("error", Value::String(message.as_str().into()));
    }

    fn state(&self) -> Json {
        let state = self.backend.snapshot();
        json!({
            "path":if self.services.source.is_some(){Json::Null}else{json!(state.path.as_ref().map(|p|p.to_string_lossy().to_string()))},
            "duration":if self.services.source.is_some(){Json::Null}else{json!(state.duration)}, "position":state.position, "paused":self.services.source.as_ref().map(|s|s.paused).unwrap_or(state.paused),
            "gaussianEnabled":state.gaussian_enabled, "sigma":state.sigma,
            "sessionId":self.services.session_id(), "sourceKind":if self.services.source.is_some(){"external"}else if state.path.is_some(){"file"}else{"none"},
        })
    }

    fn bind(&self, window: &ComponentInstance, manifests: &[Manifest]) -> Result<(), String> {
        let sender = self.sender.clone();
        window
            .set_callback("add-plugin", move |_| {
                let _ = sender.send(Incoming {
                    plugin: String::new(),
                    message: json!({"type":"add-plugin"}),
                });
                Value::Void
            })
            .map_err(|e| e.to_string())?;
        let sender = self.sender.clone();
        let commands: Vec<_> = manifests
            .iter()
            .flat_map(|m| m.commands.iter().map(|c| (m.id.clone(), c.id.clone())))
            .collect();
        window
            .set_callback("plugin-command", move |args| {
                if let (Some(Value::String(owner)), Some(Value::String(command))) =
                    (args.first(), args.get(1))
                    && commands
                        .iter()
                        .any(|(o, c)| o == owner.as_str() && c == command.as_str())
                {
                    let _ = sender.send(Incoming {
                        plugin: owner.to_string(),
                        message: json!({"type":"dispatch","name":command.as_str(),"value":0.}),
                    });
                }
                Value::Void
            })
            .map_err(|e| e.to_string())?;
        let sender = self.sender.clone();
        window.set_callback("source-input",move |args|{
            if let(Some(Value::String(key)),Some(Value::Bool(pressed)))=(args.first(),args.get(1)){let _=sender.send(Incoming{plugin:String::new(),message:json!({"type":"source-input","key":key.as_str(),"pressed":pressed})});}Value::Void
        }).map_err(|e|e.to_string())?;
        for manifest in manifests {
            let sender = self.sender.clone();
            let id = manifest.id.clone();
            window
                .set_global_callback(&manifest.ui.global, "action", move |args| {
                    let name = match args.first() {
                        Some(Value::String(v)) => v.as_str(),
                        _ => "",
                    };
                    let value = match args.get(1) {
                        Some(Value::Number(v)) => *v,
                        _ => 0.0,
                    };
                    let _ = sender.send(Incoming {
                        plugin: id.clone(),
                        message: json!({"type":"dispatch","name":name,"value":value}),
                    });
                    Value::Void
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn add_plugin(&mut self) -> Result<(), String> {
        self.dialogs.begin(
            dialogs::Specification::add_plugin(),
            dialogs::Target::AddPlugin,
            self.sender.clone(),
        )
    }

    fn restore_tool_selection(
        &self,
        next: &ComponentInstance,
        manifests: &[Manifest],
    ) -> Result<(), String> {
        let Value::String(previous) = self
            .window
            .get_property("active-panel")
            .map_err(|error| error.to_string())?
        else {
            return Err("active-panel must be a string".into());
        };
        // A rebuild is not a request to open another tool. Preserve an existing
        // selection or the user's closed state; the first tool keeps UI defaults.
        if !previous.is_empty() || self.plugins.iter().any(|p| p.manifest.ui.region.is_none()) {
            let retained = manifests
                .iter()
                .any(|m| m.ui.region.is_none() && m.id == previous.as_str());
            next.set_property(
                "active-panel",
                Value::String(if retained { previous } else { "".into() }),
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn load_plugin(&mut self, path: &std::path::Path) -> Result<(), String> {
        let manifest = Manifest::read(path)?;
        plugins::validate_unique(&manifest, self.plugins.iter().map(|p| &p.manifest), false)?;
        let mut manifests: Vec<_> = self.plugins.iter().map(|p| p.manifest.clone()).collect();
        manifests.push(manifest.clone());
        self.composition_count += 1;
        let next = ui::compose(&manifests)?;
        self.bind(&next, &manifests)?;
        self.restore_tool_selection(&next, &manifests)?;
        for plugin in &self.plugins {
            for (name, value) in &plugin.properties {
                next.set_global_property(&plugin.manifest.ui.global, name, ui::primitive(value)?)
                    .map_err(|e| e.to_string())?;
            }
        }
        next.set_property("frame", Value::Image(self.frame.clone()))
            .map_err(|e| e.to_string())?;
        let logical_size = self
            .window
            .window()
            .size()
            .to_logical(self.window.window().scale_factor());
        next.window().set_position(self.window.window().position());
        let plugin = Plugin::start(manifest, self.sender.clone())?;
        // 先显示新窗口再隐藏旧窗口，防止最后一个窗口隐藏导致事件循环退出。
        next.show().map_err(|e| e.to_string())?;
        next.window().set_size(logical_size);
        self.window.hide().map_err(|e| e.to_string())?;
        self.window = next;
        self.window_generation += 1;
        self.plugins.push(plugin);
        Ok(())
    }

    fn recompose(&mut self, excluded: Option<&str>) -> Result<(), String> {
        let manifests: Vec<_> = self
            .plugins
            .iter()
            .filter(|p| Some(p.route.as_str()) != excluded)
            .map(|p| p.manifest.clone())
            .collect();
        let next = ui::compose(&manifests)?;
        self.bind(&next, &manifests)?;
        self.restore_tool_selection(&next, &manifests)?;
        for plugin in self
            .plugins
            .iter()
            .filter(|p| Some(p.route.as_str()) != excluded)
        {
            for (name, value) in &plugin.properties {
                next.set_global_property(&plugin.manifest.ui.global, name, ui::primitive(value)?)
                    .map_err(|e| e.to_string())?;
            }
        }
        next.set_property("frame", Value::Image(self.frame.clone()))
            .map_err(|e| e.to_string())?;
        let size = self
            .window
            .window()
            .size()
            .to_logical(self.window.window().scale_factor());
        next.window().set_position(self.window.window().position());
        next.show().map_err(|e| e.to_string())?;
        next.window().set_size(size);
        self.window.hide().map_err(|e| e.to_string())?;
        self.window = next;
        self.window_generation += 1;
        self.composition_count += 1;
        Ok(())
    }
    fn unload(&mut self, route: &str) {
        self.unload_wait(route, None);
    }
    fn unload_wait(&mut self, route: &str, waiter: Option<(String, Json)>) {
        self.dialogs.abandon(route);
        self.services.cleanup(route, &mut self.backend);
        if let Some(index) = self.plugins.iter().position(|p| p.route == route) {
            let plugin = self.plugins.remove(index);
            self.retiring.begin(plugin, waiter);
        }
    }
    fn creator(&self, route: &str) -> String {
        self.plugins
            .iter()
            .find(|p| p.route == route)
            .map(|p| p.manifest.id.clone())
            .unwrap_or_else(|| route.into())
    }
    fn begin_install(
        &mut self,
        caller: String,
        request_id: Json,
        path: &str,
    ) -> Result<(), String> {
        let manifest = services::validate(std::path::Path::new(path))?;
        if self
            .plugins
            .iter()
            .any(|p| p.manifest.id == manifest.id && self.pending.contains_key(&p.route))
        {
            return Err("plugin installation already pending".into());
        }
        plugins::validate_unique(&manifest, self.plugins.iter().map(|p| &p.manifest), true)?;
        let previous = self
            .plugins
            .iter()
            .find(|p| p.manifest.id == manifest.id)
            .map(|p| p.route.clone());
        self.install_serial += 1;
        let route = format!("{}#{}", manifest.id, self.install_serial);
        let creator = self.creator(&caller);
        if let Some(old) = self.plugins.iter().find(|p| p.manifest.id == manifest.id)
            && old.managed_by.as_deref() != Some(creator.as_str())
        {
            return Err("only caller-installed generated plugin can be replaced".into());
        }
        let mut plugin = Plugin::start_routed(manifest, self.sender.clone(), route.clone())?;
        plugin.managed_by = Some(creator);
        self.pending.insert(
            route,
            PendingInstall {
                caller,
                request_id,
                previous,
                started: Instant::now(),
            },
        );
        self.plugins.push(plugin);
        Ok(())
    }
    fn reply(&mut self, route: &str, id: Json, result: Result<Json, String>) -> Result<(), String> {
        let response = match result {
            Ok(result) => json!({"type":"response","id":id,"result":result}),
            Err(error) => json!({"type":"response","id":id,"error":error}),
        };
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.route == route) {
            plugin.send(response)?;
        } else {
            self.retiring.reply(route, response)?;
        }
        Ok(())
    }
    fn request(&mut self, index: usize, method: &str, params: &Json) -> Result<Json, String> {
        if let Some(result) = self.services.request(
            &self.plugins[index].route,
            method,
            params,
            &mut self.backend,
        )? {
            return Ok(result);
        }
        if ["dialogs.openFile", "dialogs.openMedia", "dialogs.saveFile"].contains(&method)
            && let Some(fixtures) = self.dialog_fixtures.as_mut()
        {
            return Ok(fixtures
                .pop_front()
                .flatten()
                .map(|p| json!(p.to_string_lossy()))
                .unwrap_or(Json::Null));
        }
        match method {
            "dialogs.openMedia" if self.auto_media.is_some() => {
                Ok(json!(self.auto_media.as_ref().unwrap().to_string_lossy()))
            }
            "dialogs.openMedia" | "dialogs.openFile" | "dialogs.saveFile" => {
                Err("native dialogs must use asynchronous RPC dispatch".into())
            }
            "player.load" => {
                let path = params["path"].as_str().ok_or("缺少 path")?;
                self.backend.load(PathBuf::from(path))?;
                self.services.file_loaded(&mut self.backend)?;
                Ok(Json::Null)
            }
            "player.togglePause" => {
                self.backend.toggle_pause()?;
                Ok(Json::Null)
            }
            "player.seek" => {
                self.backend
                    .seek(params["seconds"].as_f64().ok_or("缺少 seconds")?)?;
                Ok(Json::Null)
            }
            "player.filters.setGaussian" => {
                self.backend.set_gaussian(
                    params["enabled"].as_bool().ok_or("缺少 enabled")?,
                    params["sigma"].as_f64().ok_or("缺少 sigma")?,
                )?;
                Ok(Json::Null)
            }
            "player.snapshot" => Ok(self.state()),
            "plugins.remove" => {
                let id = params["id"].as_str().ok_or("missing id")?;
                let creator = self.creator(&self.plugins[index].route);
                if !self.plugins.iter().any(|p| {
                    p.manifest.id == id && p.managed_by.as_deref() == Some(creator.as_str())
                }) {
                    return Err("only caller-installed generated plugins may be removed".into());
                }
                let route = self
                    .plugins
                    .iter()
                    .find(|p| p.manifest.id == id)
                    .map(|p| p.route.clone())
                    .ok_or("unknown plugin")?;
                self.recompose(Some(&route))?;
                self.unload(&route);
                Ok(Json::Null)
            }
            "ui.patch" => {
                let properties = params["properties"].as_object().ok_or("缺少 properties")?;
                self.plugins[index].validate_patch(properties)?;
                for (name, value) in properties {
                    if !self.pending.contains_key(&self.plugins[index].route) {
                        self.window
                            .set_global_property(
                                &self.plugins[index].manifest.ui.global,
                                name,
                                ui::primitive(value)?,
                            )
                            .map_err(|e| e.to_string())?;
                    } else {
                        ui::primitive(value)?;
                    }
                    self.plugins[index]
                        .properties
                        .insert(name.clone(), value.clone());
                }
                Ok(Json::Null)
            }
            _ => Err(format!("实验没有实现方法 {method}")),
        }
    }

    fn incoming(&mut self, incoming: Incoming) -> Result<(), String> {
        let message = incoming.message;
        if incoming.plugin.is_empty() && message["type"] == "dialog-complete" {
            let ticket = message["ticket"]
                .as_u64()
                .ok_or("missing native dialog ticket")?;
            if let Some(target) = self.dialogs.complete(ticket) {
                let path = dialogs::selected_path(&message)?;
                match target {
                    dialogs::Target::Plugin { owner, request_id } => self.reply(
                        &owner,
                        request_id,
                        Ok(path
                            .map(|path| json!(path.to_string_lossy()))
                            .unwrap_or(Json::Null)),
                    )?,
                    dialogs::Target::AddPlugin => {
                        if let Some(path) = path {
                            self.load_plugin(&path)?;
                        }
                    }
                    dialogs::Target::Abandoned => {}
                }
            }
            return Ok(());
        }
        if incoming.plugin.is_empty() && message["type"] == "source-input" {
            if let Some(source) = self.services.source.as_ref() {
                let owner = source.owner.clone();
                if let Some(plugin) = self.plugins.iter_mut().find(|p| p.route == owner) {
                    plugin.send(json!({"type":"input","event":{"key":message["key"],"pressed":message["pressed"]}}))?;
                }
            }
            return Ok(());
        }
        if incoming.plugin.is_empty() && message["type"] == "add-plugin" {
            return self.add_plugin();
        }
        if self.retiring.contains(&incoming.plugin) {
            if message["type"] == "stopped"
                && let Some(error) = message["error"].as_str()
            {
                self.disposal_errors
                    .push(format!("{}: {error}", incoming.plugin));
            }
            if message["type"] == "request" {
                let result = self.retiring.cleanup_request(
                    &incoming.plugin,
                    message["method"].as_str().unwrap_or(""),
                    &message["params"],
                    &mut self.services,
                    &mut self.backend,
                );
                self.reply(&incoming.plugin, message["id"].clone(), result)?;
            } else if (message["type"] == "stopped" || message["type"] == "fatal")
                && let Some((caller, id)) = self.retiring.finish(&incoming.plugin)
            {
                let result = message["error"].as_str().map_or(Ok(Json::Null), |error| {
                    Err(format!("plugin disposal failed: {error}"))
                });
                self.reply(&caller, id, result)?;
            }
            return Ok(());
        }
        let index = self.plugins.iter().position(|p| {
            p.route == incoming.plugin
                || (p.manifest.id == incoming.plugin && !self.pending.contains_key(&p.route))
        });
        let Some(index) = index else {
            return Ok(());
        };
        match message["type"].as_str() {
            Some("dispatch") => self.plugins[index]
                .send(json!({"type":"action","name":message["name"],"value":message["value"]})),
            Some("request") => {
                let caller = self.plugins[index].route.clone();
                let method = message["method"].as_str().unwrap_or("");
                if ["dialogs.openFile", "dialogs.openMedia", "dialogs.saveFile"].contains(&method)
                    && self.dialog_fixtures.is_none()
                    && !(method == "dialogs.openMedia" && self.auto_media.is_some())
                {
                    let result = (|| {
                        let specification =
                            dialogs::Specification::from_request(method, &message["params"])?
                                .ok_or("invalid dialog request")?;
                        self.dialogs.begin(
                            specification,
                            dialogs::Target::Plugin {
                                owner: caller.clone(),
                                request_id: message["id"].clone(),
                            },
                            self.sender.clone(),
                        )
                    })();
                    if let Err(error) = result {
                        self.reply(&caller, message["id"].clone(), Err(error))?;
                    }
                    return Ok(());
                }

                if message["method"] == "plugins.remove" {
                    let result = (|| {
                        services::validate_params("plugins.remove", &message["params"])?;
                        let id = message["params"]["id"].as_str().ok_or("missing id")?;
                        let creator = self.creator(&caller);
                        let target = self
                            .plugins
                            .iter()
                            .find(|p| {
                                p.manifest.id == id
                                    && p.managed_by.as_deref() == Some(creator.as_str())
                            })
                            .ok_or("only caller-installed generated plugins may be removed")?;
                        let route = target.route.clone();
                        self.recompose(Some(&route))?;
                        Ok::<String, String>(route)
                    })();
                    match result {
                        Ok(route) => {
                            self.unload_wait(&route, Some((caller.clone(), message["id"].clone())))
                        }
                        Err(error) => self.reply(&caller, message["id"].clone(), Err(error))?,
                    }
                    return Ok(());
                }
                if message["method"] == "plugins.install" {
                    if let Err(error) =
                        services::validate_params("plugins.install", &message["params"])
                    {
                        self.reply(&caller, message["id"].clone(), Err(error))?;
                        return Ok(());
                    }
                    let result = self.begin_install(
                        caller.clone(),
                        message["id"].clone(),
                        message["params"]["manifest"].as_str().unwrap_or(""),
                    );
                    if let Err(error) = result {
                        self.reply(&caller, message["id"].clone(), Err(error))?;
                    }
                    return Ok(());
                }
                let result = self.request(
                    index,
                    message["method"].as_str().unwrap_or(""),
                    &message["params"],
                );
                self.reply(&caller, message["id"].clone(), result)
            }
            Some("ready") => {
                let route = self.plugins[index].route.clone();
                if let Some(pending) = self.pending.remove(&route) {
                    match self.recompose(pending.previous.as_deref()) {
                        Ok(()) => {
                            if let Some(old) = pending.previous {
                                self.unload(&old);
                            }
                            let id = self
                                .plugins
                                .iter()
                                .find(|p| p.route == route)
                                .map(|p| p.manifest.id.clone())
                                .unwrap_or_default();
                            self.reply(&pending.caller, pending.request_id, Ok(json!({"id":id})))?;
                        }
                        Err(error) => {
                            self.unload(&route);
                            self.reply(&pending.caller, pending.request_id, Err(error))?;
                            return Ok(());
                        }
                    }
                }
                let state = self.state();
                if let Some(plugin) = self.plugins.iter_mut().find(|p| p.route == route) {
                    plugin.send(json!({"type":"playback","state":state}))?;
                }
                Ok(())
            }
            Some("fatal") => {
                let route = self.plugins[index].route.clone();
                if let Some(pending) = self.pending.remove(&route) {
                    self.reply(
                        &pending.caller,
                        pending.request_id,
                        Err(message["error"].to_string()),
                    )?;
                }
                let _ = self.recompose(Some(&route));
                self.dialogs.abandon(&route);
                self.services.cleanup(&route, &mut self.backend);
                self.plugins.retain(|plugin| plugin.route != route);
                Err(format!("插件 {route}：{}", message["error"]))
            }
            _ => Err("未知插件消息".into()),
        }
    }

    fn tick(&mut self) -> Result<(), String> {
        for route in self.retiring.expired() {
            self.disposal_errors
                .push(format!("plugin {route} disposal timed out"));
            if let Some((caller, id)) = self.retiring.finish(&route) {
                self.reply(&caller, id, Err("plugin disposal timed out".into()))?;
            }
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.started.elapsed() > Duration::from_secs(25))
            .map(|(r, _)| r.clone())
            .collect();
        for route in expired {
            if let Some(p) = self.pending.remove(&route) {
                self.unload(&route);
                self.reply(
                    &p.caller,
                    p.request_id,
                    Err("plugin activation timeout".into()),
                )?;
            }
        }

        if let Some(source) = self.services.source.as_mut()
            && let Some(rgb) = source.frame.take()
        {
            let buffer =
                SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, source.width, source.height);
            self.frame = Image::from_rgb8(buffer);
            self.window
                .set_property("frame", Value::Image(self.frame.clone()))
                .map_err(|e| e.to_string())?;
        }
        if let Some(frame) = self.backend.poll()? {
            self.received_frames += 1;
            let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                &frame.rgb,
                backend::WIDTH,
                backend::HEIGHT,
            );
            self.frame = Image::from_rgb8(buffer);
            self.window
                .set_property("frame", Value::Image(self.frame.clone()))
                .map_err(|e| e.to_string())?;
        }
        if self.last_broadcast.elapsed() >= Duration::from_millis(100) {
            self.last_broadcast = Instant::now();
            let message = json!({"type":"playback","state":self.state()});
            for plugin in &mut self.plugins {
                plugin.send(message.clone())?;
            }
        }
        Ok(())
    }
}

// 仅实验验证：真实窗口、真实 TS RPC、真实 Slint global；不作为产品功能。
struct Verification {
    stage: u8,
    stage_started: Instant,
    deadline: Instant,
    observations: Vec<Json>,
    report: PathBuf,
    before_hide: Option<Json>,
    resume_requested: bool,
}

impl Verification {
    fn record(&mut self, host: &Host, label: &str) {
        self.observations.push(json!({"stage":label,"compositionCount":host.composition_count,
            "windowSize":{"width":host.window.window().size().width,"height":host.window.window().size().height,"scale":host.window.window().scale_factor()},
            "windowGeneration":host.window_generation,"frames":host.received_frames,"playback":host.state(),
            "playlist":host.plugins.iter().find(|p|p.manifest.id == "playlist").map(|p| &p.properties)}));
        self.stage += 1;
        self.stage_started = Instant::now();
        self.deadline = Instant::now() + Duration::from_secs(40);
    }

    // 实验截图使用 Slint 公开的窗口快照接口，不依赖动态嵌入内部 API。
    fn screenshot(&self, host: &Host, name: &str) -> Result<(), String> {
        let buffer = host
            .window
            .window()
            .take_snapshot()
            .map_err(|e| e.to_string())?;
        let directory = self
            .report
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        image::save_buffer(
            directory.join(format!("playlist-{name}.png")),
            buffer.as_bytes(),
            buffer.width(),
            buffer.height(),
            image::ColorType::Rgba8,
        )
        .map_err(|e| e.to_string())
    }

    fn action(host: &Host, global: &str, name: &str, number: f64) -> Result<(), String> {
        host.window
            .invoke_global(
                global,
                "action",
                &[Value::String(name.into()), Value::Number(number)],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn property(host: &Host, name: &str) -> Option<Json> {
        host.plugins
            .iter()
            .find(|p| p.manifest.id == "playlist")?
            .properties
            .get(name)
            .cloned()
    }

    fn visible(host: &Host, expected: bool) -> bool {
        Self::property(host, "visible") == Some(json!(expected))
            && host
                .window
                .get_global_property("PlaylistState", "visible")
                .ok()
                == Some(Value::Bool(expected))
    }

    fn step(&mut self, host: &mut Host) -> Result<bool, String> {
        if Instant::now() > self.deadline {
            return Err(format!("真实窗口验证 stage {} 超时", self.stage));
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        match self.stage {
            0 => {
                host.load_plugin(&root.join("file-playback/manifest.json"))?;
                self.record(host, "file-plugin-installed");
            }
            1 if host.plugins[0].properties.contains_key("status") => {
                Self::action(host, "FilePlaybackState", "open", 0.)?;
                self.record(host, "open-through-ts-action");
            }
            2 if host.backend.snapshot().position > 0.4 && host.received_frames > 0 => {
                Self::action(host, "FilePlaybackState", "toggle-pause", 0.)?;
                self.record(host, "frames-displayed");
            }
            3 if host.backend.snapshot().paused => {
                Self::action(host, "FilePlaybackState", "seek", 1.)?;
                self.record(host, "paused-through-ts");
            }
            4 if host.backend.snapshot().position >= 1. => {
                if host.backend.snapshot().paused {
                    if !self.resume_requested {
                        Self::action(host, "FilePlaybackState", "toggle-pause", 0.)?;
                        self.resume_requested = true;
                    }
                    return Ok(false);
                }
                host.load_plugin(&root.join("playlist/manifest.json"))?;
                self.record(host, "playlist-first-install-recomposed");
            }
            5 if Self::visible(host, true) => {
                Self::action(host, "PlaylistState", "add", 0.)?;
                self.record(host, "playlist-ready");
            }
            6 if Self::property(host, "count") == Some(json!(1)) => {
                self.before_hide = Self::property(host, "items-text");
                Self::action(host, "PlaylistState", "hide", 0.)?;
                self.record(host, "playlist-item-added-through-ts");
            }
            7 if Self::visible(host, false)
                && self.stage_started.elapsed() > Duration::from_millis(500) =>
            {
                if host.composition_count != 3 || host.window_generation != 3 {
                    return Err("隐藏侧栏意外重新编译或替换窗口".into());
                }
                if Self::property(host, "items-text") != self.before_hide {
                    return Err("隐藏侧栏丢失列表".into());
                }
                self.screenshot(host, "hidden")?;
                Self::action(host, "PlaylistState", "toggle", 0.)?;
                self.record(host, "hidden-without-recompose");
            }
            8 if Self::visible(host, true)
                && self.stage_started.elapsed() > Duration::from_millis(500) =>
            {
                if host.composition_count != 3 || host.window_generation != 3 {
                    return Err("重显侧栏意外重新编译或替换窗口".into());
                }
                self.screenshot(host, "shown")?;
                self.record(host, "shown-without-recompose");
                host.load_plugin(&root.join("gaussian-blur/manifest.json"))?;
            }
            9 if host
                .plugins
                .iter()
                .find(|p| p.manifest.id == "gaussian-blur")
                .is_some_and(|p| p.properties.contains_key("sigma")) =>
            {
                if !Self::visible(host, true)
                    || Self::property(host, "count") != Some(json!(1))
                    || Self::property(host, "items-text") != self.before_hide
                {
                    return Err("后续插件安装未恢复侧栏和列表状态".into());
                }
                Self::action(host, "GaussianBlurState", "enabled", 1.)?;
                self.record(host, "later-install-preserved-playlist");
            }
            10 if host.backend.snapshot().gaussian_enabled => {
                if host.backend.snapshot().position < 1.0 || host.backend.snapshot().paused {
                    return Err(format!(
                        "界面操作后播放位置或运行状态未保留：{}",
                        host.state()
                    ));
                }
                self.record(host, "gaussian-enabled-through-ts");
                let report = json!({"passed":true,"observations":self.observations});
                std::fs::write(
                    &self.report,
                    serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                println!("真实窗口实验通过：{}", self.report.display());
                return Ok(true);
            }
            _ => (),
        }
        Ok(false)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--headless") {
        return headless::run().map_err(Into::into);
    }
    if std::env::args().any(|arg| arg == "--check-ui") {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let supplied: Vec<_> = std::env::args()
            .skip_while(|a| a != "--check-ui")
            .skip(1)
            .collect();
        let paths: Vec<PathBuf> = if supplied.is_empty() {
            std::fs::read_dir(&root)?
                .filter_map(Result::ok)
                .map(|d| d.path().join("manifest.json"))
                .filter(|p| p.exists())
                .collect()
        } else {
            supplied.into_iter().map(PathBuf::from).collect()
        };
        let manifests = paths
            .iter()
            .map(|p| Manifest::read(p))
            .collect::<Result<Vec<_>, _>>()?;
        ui::compile(&[])?;
        ui::compile(&manifests)?;
        println!(
            "Slint 实验 UI 编译通过：空宿主与 {} 个插件组合（未创建窗口）",
            manifests.len()
        );
        return Ok(());
    }
    let arguments: Vec<_> = std::env::args().collect();
    let experiment = arguments
        .iter()
        .position(|arg| arg == "--experiment-playlist")
        .map(|index| {
            arguments
                .get(index + 1)
                .cloned()
                .ok_or("--experiment-playlist 需要 MP4 路径")
        })
        .transpose()?;
    let report = arguments
        .iter()
        .position(|arg| arg == "--report")
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("playlist-window-report.json"));
    let (sender, receiver) = mpsc::channel();
    let backend = if experiment.is_some() {
        Backend::new_silent()?
    } else {
        Backend::new()?
    };
    let window = ui::compose(&[])?;
    window
        .window()
        .set_size(slint::LogicalSize::new(960., 640.));
    let host = Rc::new(RefCell::new(Host {
        dialogs: dialogs::Dialogs::default(),
        disposal_errors: vec![],
        retiring: retirement::Retirement::default(),
        dialog_fixtures: if arguments.iter().any(|a| a == "--ui-scenario") {
            Some(std::collections::VecDeque::new())
        } else {
            None
        },
        pending: std::collections::HashMap::new(),
        install_serial: 0,
        window,
        backend,
        services: services::Services::new(),
        plugins: Vec::new(),
        sender,
        frame: Image::default(),
        last_broadcast: Instant::now(),
        composition_count: 1,
        window_generation: 1,
        auto_media: experiment.as_ref().map(PathBuf::from),
        received_frames: 0,
    }));
    host.borrow().bind(&host.borrow().window, &[])?;
    host.borrow().window.show()?;
    for pair in arguments.windows(2).filter(|pair| pair[0] == "--plugin") {
        host.borrow_mut()
            .load_plugin(std::path::Path::new(&pair[1]))?;
    }

    let mut scenario = arguments
        .iter()
        .position(|a| a == "--ui-scenario")
        .map(|i| {
            arguments
                .get(i + 1)
                .ok_or("--ui-scenario requires path")
                .and_then(|p| {
                    scenario::Driver::read(std::path::Path::new(p))
                        .map_err(|_| "failed reading UI scenario")
                })
        })
        .transpose()?;
    let receiver = Rc::new(RefCell::new(receiver));
    let receiver_live = receiver.clone();
    let timer = Timer::default();
    let live = host.clone();
    let mut verification = experiment.map(|_| Verification {
        stage: 0,
        stage_started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(40),
        observations: Vec::new(),
        report,
        before_hide: None,
        resume_requested: false,
    });
    let verification_error = Rc::new(RefCell::new(None::<String>));
    let verification_error_live = verification_error.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut host = live.borrow_mut();
        // 限制每帧处理消息数，避免插件消息洪水饿死绘制。
        for incoming in receiver_live.borrow().try_iter().take(64) {
            if let Err(error) = host.incoming(incoming) {
                host.report(error);
            }
        }
        if let Err(error) = host.tick() {
            host.report(error);
        }
        if let Some(driver) = scenario.as_mut() {
            match driver.step(&mut host) {
                Ok(true) => {
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    *verification_error_live.borrow_mut() = Some(error);
                    let _ = slint::quit_event_loop();
                }
                Ok(false) => {}
            }
        }
        if let Some(verification) = verification.as_mut() {
            match verification.step(&mut host) {
                Ok(true) => {
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    host.report(&error);
                    *verification_error_live.borrow_mut() = Some(error);
                    let _ = slint::quit_event_loop();
                }
                Ok(false) => (),
            }
        }
    });
    slint::run_event_loop()?;
    timer.stop();
    let routes: Vec<_> = host
        .borrow()
        .plugins
        .iter()
        .map(|p| p.route.clone())
        .collect();
    for route in routes {
        host.borrow_mut().unload(&route);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while !host.borrow().retiring.is_empty() && Instant::now() < deadline {
        if let Ok(incoming) = receiver.borrow().recv_timeout(Duration::from_millis(10)) {
            let _ = host.borrow_mut().incoming(incoming);
        }
    }

    if !host.borrow().retiring.is_empty() {
        return Err("GUI shutdown failed to drain plugin disposers".into());
    }
    if !host.borrow().disposal_errors.is_empty() {
        return Err(format!(
            "plugin disposal failed: {}",
            host.borrow().disposal_errors.join("; ")
        )
        .into());
    }
    if let Some(error) = verification_error.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}
