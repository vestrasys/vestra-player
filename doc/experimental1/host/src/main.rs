// 实验版本：只验证 Rust/Slint 宿主与 TS 插件；与最终成品 API 可能存在很大差异。
#![forbid(unsafe_code)]

mod backend;
mod plugins;
mod ui;

use backend::Backend;
use plugins::{Incoming, Manifest, Plugin};
use serde_json::{Value as Json, json};
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode};
use slint_interpreter::{ComponentInstance, Value};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::mpsc, time::{Duration, Instant}};

struct Host {
    window: ComponentInstance,
    backend: Backend,
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
        let _ = self.window.set_property("error", Value::String(message.as_str().into()));
    }

    fn state(&self) -> Json {
        let state = self.backend.snapshot();
        json!({
            "path":state.path.as_ref().map(|p|p.to_string_lossy().to_string()),
            "duration":state.duration, "position":state.position, "paused":state.paused,
            "gaussianEnabled":state.gaussian_enabled, "sigma":state.sigma,
        })
    }

    fn bind(&self, window: &ComponentInstance, manifests: &[Manifest]) -> Result<(), String> {
        let sender = self.sender.clone();
        window.set_callback("add-plugin", move |_| {
            let _ = sender.send(Incoming { plugin: String::new(), message: json!({"type":"add-plugin"}) });
            Value::Void
        }).map_err(|e| e.to_string())?;
        for manifest in manifests {
            let sender = self.sender.clone();
            let id = manifest.id.clone();
            window.set_global_callback(&manifest.ui.global, "action", move |args| {
                let name = match args.first() { Some(Value::String(v)) => v.as_str(), _ => "" };
                let value = match args.get(1) { Some(Value::Number(v)) => *v, _ => 0.0 };
                let _ = sender.send(Incoming { plugin: id.clone(), message: json!({"type":"dispatch","name":name,"value":value}) });
                Value::Void
            }).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn add_plugin(&mut self) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new().set_title("选择实验插件 manifest.json")
            .add_filter("插件 manifest", &["json"]).pick_file() else { return Ok(()); };
        self.load_plugin(&path)
    }

    fn load_plugin(&mut self, path: &std::path::Path) -> Result<(), String> {
        let manifest = Manifest::read(&path)?;
        if self.plugins.iter().any(|p| p.manifest.id == manifest.id
            || p.manifest.ui.component == manifest.ui.component || p.manifest.ui.global == manifest.ui.global) {
            return Err("插件 id、component 和 global 必须唯一".into());
        }
        let mut manifests: Vec<_> = self.plugins.iter().map(|p| p.manifest.clone()).collect();
        manifests.push(manifest.clone());
        self.composition_count += 1;
        let next = ui::compose(&manifests)?;
        self.bind(&next, &manifests)?;
        for plugin in &self.plugins {
            for (name, value) in &plugin.properties {
                next.set_global_property(&plugin.manifest.ui.global, name, ui::primitive(value)?)
                    .map_err(|e| e.to_string())?;
            }
        }
        next.set_property("frame", Value::Image(self.frame.clone())).map_err(|e| e.to_string())?;
        let logical_size = self.window.window().size().to_logical(self.window.window().scale_factor());
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

    fn request(&mut self, index: usize, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "dialogs.openMedia" if self.auto_media.is_some() => Ok(json!(self.auto_media.as_ref().unwrap().to_string_lossy())),
            "dialogs.openMedia" => Ok(rfd::FileDialog::new().set_title("打开 MP4")
                .add_filter("MP4", &["mp4"]).pick_file()
                .map(|p| json!(p.to_string_lossy())).unwrap_or(Json::Null)),
            "player.load" => {
                let path = params["path"].as_str().ok_or("缺少 path")?;
                self.backend.load(PathBuf::from(path))?;
                Ok(Json::Null)
            }
            "player.togglePause" => { self.backend.toggle_pause()?; Ok(Json::Null) }
            "player.seek" => {
                self.backend.seek(params["seconds"].as_f64().ok_or("缺少 seconds")?)?;
                Ok(Json::Null)
            }
            "player.filters.setGaussian" => {
                self.backend.set_gaussian(params["enabled"].as_bool().ok_or("缺少 enabled")?,
                    params["sigma"].as_f64().ok_or("缺少 sigma")?)?;
                Ok(Json::Null)
            }
            "player.snapshot" => Ok(self.state()),
            "ui.patch" => {
                let properties = params["properties"].as_object().ok_or("缺少 properties")?;
                for (name, value) in properties {
                    self.window.set_global_property(&self.plugins[index].manifest.ui.global,
                        name, ui::primitive(value)?).map_err(|e| e.to_string())?;
                    self.plugins[index].properties.insert(name.clone(), value.clone());
                }
                Ok(Json::Null)
            }
            _ => Err(format!("实验没有实现方法 {method}")),
        }
    }

    fn incoming(&mut self, incoming: Incoming) -> Result<(), String> {
        let message = incoming.message;
        if message["type"] == "add-plugin" { return self.add_plugin(); }
        let index = self.plugins.iter().position(|p| p.manifest.id == incoming.plugin)
            .ok_or("消息来源插件不存在")?;
        match message["type"].as_str() {
            Some("dispatch") => self.plugins[index].send(json!({"type":"action","name":message["name"],"value":message["value"]})),
            Some("request") => {
                let result = self.request(index, message["method"].as_str().unwrap_or(""), &message["params"]);
                let response = match result {
                    Ok(result) => json!({"type":"response","id":message["id"],"result":result}),
                    Err(error) => { self.report(&error); json!({"type":"response","id":message["id"],"error":error}) }
                };
                self.plugins[index].send(response)
            }
            Some("ready") => {
                let state = self.state();
                self.plugins[index].send(json!({"type":"playback","state":state}))
            }
            Some("fatal") => Err(format!("插件 {}：{}", incoming.plugin, message["error"])),
            _ => Err("未知插件消息".into()),
        }
    }

    fn tick(&mut self) -> Result<(), String> {
        if let Some(frame) = self.backend.poll()? {
            self.received_frames += 1;
            let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&frame.rgb, backend::WIDTH, backend::HEIGHT);
            self.frame = Image::from_rgb8(buffer);
            self.window.set_property("frame", Value::Image(self.frame.clone())).map_err(|e| e.to_string())?;
        }
        if self.last_broadcast.elapsed() >= Duration::from_millis(100) {
            self.last_broadcast = Instant::now();
            let message = json!({"type":"playback","state":self.state()});
            for plugin in &mut self.plugins { plugin.send(message.clone())?; }
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
        let buffer = host.window.window().take_snapshot().map_err(|e|e.to_string())?;
        let directory = self.report.parent().unwrap_or_else(||std::path::Path::new("."));
        image::save_buffer(directory.join(format!("playlist-{name}.png")), buffer.as_bytes(),
            buffer.width(), buffer.height(), image::ColorType::Rgba8).map_err(|e|e.to_string())
    }

    fn action(host: &Host, global: &str, name: &str, number: f64) -> Result<(), String> {
        host.window.invoke_global(global, "action", &[Value::String(name.into()), Value::Number(number)])
            .map(|_|()).map_err(|e|e.to_string())
    }

    fn property(host: &Host, name: &str) -> Option<Json> {
        host.plugins.iter().find(|p|p.manifest.id == "playlist")?.properties.get(name).cloned()
    }

    fn visible(host: &Host, expected: bool) -> bool {
        Self::property(host, "visible") == Some(json!(expected))
            && host.window.get_global_property("PlaylistState", "visible").ok() == Some(Value::Bool(expected))
    }

    fn step(&mut self, host: &mut Host) -> Result<bool, String> {
        if Instant::now() > self.deadline { return Err(format!("真实窗口验证 stage {} 超时", self.stage)); }
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
            7 if Self::visible(host, false) && self.stage_started.elapsed() > Duration::from_millis(500) => {
                if host.composition_count != 3 || host.window_generation != 3 {
                    return Err("隐藏侧栏意外重新编译或替换窗口".into());
                }
                if Self::property(host, "items-text") != self.before_hide { return Err("隐藏侧栏丢失列表".into()); }
                self.screenshot(host, "hidden")?;
                Self::action(host, "PlaylistState", "toggle", 0.)?;
                self.record(host, "hidden-without-recompose");
            }
            8 if Self::visible(host, true) && self.stage_started.elapsed() > Duration::from_millis(500) => {
                if host.composition_count != 3 || host.window_generation != 3 {
                    return Err("重显侧栏意外重新编译或替换窗口".into());
                }
                self.screenshot(host, "shown")?;
                self.record(host, "shown-without-recompose");
                host.load_plugin(&root.join("gaussian-blur/manifest.json"))?;
            }
            9 if host.plugins.iter().find(|p|p.manifest.id == "gaussian-blur")
                .is_some_and(|p|p.properties.contains_key("sigma")) => {
                if !Self::visible(host, true) || Self::property(host, "count") != Some(json!(1))
                    || Self::property(host, "items-text") != self.before_hide {
                    return Err("后续插件安装未恢复侧栏和列表状态".into());
                }
                Self::action(host, "GaussianBlurState", "enabled", 1.)?;
                self.record(host, "later-install-preserved-playlist");
            }
            10 if host.backend.snapshot().gaussian_enabled => {
                if host.backend.snapshot().position < 1.0 || host.backend.snapshot().paused {
                    return Err(format!("界面操作后播放位置或运行状态未保留：{}", host.state()));
                }
                self.record(host, "gaussian-enabled-through-ts");
                let report = json!({"passed":true,"observations":self.observations});
                std::fs::write(&self.report, serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?)
                    .map_err(|e|e.to_string())?;
                println!("真实窗口实验通过：{}", self.report.display());
                return Ok(true);
            }
            _ => (),
        }
        Ok(false)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--check-ui") {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let manifests = vec![
            Manifest::read(&root.join("file-playback/manifest.json"))?,
            Manifest::read(&root.join("gaussian-blur/manifest.json"))?,
            Manifest::read(&root.join("playlist/manifest.json"))?,
        ];
        ui::compile(&[])?;
        ui::compile(&manifests)?;
        println!("Slint 实验 UI 编译通过：空宿主与三个插件组合（未创建窗口）");
        return Ok(());
    }
    let arguments: Vec<_> = std::env::args().collect();
    let experiment = arguments.iter().position(|arg|arg == "--experiment-playlist")
        .map(|index|arguments.get(index+1).cloned().ok_or("--experiment-playlist 需要 MP4 路径"))
        .transpose()?;
    let report = arguments.iter().position(|arg|arg == "--report")
        .and_then(|index|arguments.get(index+1)).map(PathBuf::from)
        .unwrap_or_else(||PathBuf::from("playlist-window-report.json"));
    let (sender, receiver) = mpsc::channel();
    let backend = if experiment.is_some() { Backend::new_silent()? } else { Backend::new()? };
    let window = ui::compose(&[])?;
    window.window().set_size(slint::LogicalSize::new(960., 640.));
    let host = Rc::new(RefCell::new(Host { window, backend, plugins: Vec::new(), sender,
        frame: Image::default(), last_broadcast: Instant::now(), composition_count: 1, window_generation: 1,
        auto_media: experiment.as_ref().map(PathBuf::from), received_frames: 0 }));
    host.borrow().bind(&host.borrow().window, &[])?;
    host.borrow().window.show()?;
    let timer = Timer::default();
    let live = host.clone();
    let mut verification = experiment.map(|_| Verification { stage:0,
        stage_started:Instant::now(),
        deadline:Instant::now()+Duration::from_secs(40), observations:Vec::new(), report,
        before_hide:None, resume_requested:false });
    let verification_error = Rc::new(RefCell::new(None::<String>));
    let verification_error_live = verification_error.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut host = live.borrow_mut();
        // 限制每帧处理消息数，避免插件消息洪水饿死绘制。
        for incoming in receiver.try_iter().take(64) {
            if let Err(error) = host.incoming(incoming) { host.report(error); }
        }
        if let Err(error) = host.tick() { host.report(error); }
        if let Some(verification) = verification.as_mut() {
            match verification.step(&mut host) {
                Ok(true) => { let _ = slint::quit_event_loop(); }
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
    if let Some(error) = verification_error.borrow_mut().take() { return Err(error.into()); }
    Ok(())
}
