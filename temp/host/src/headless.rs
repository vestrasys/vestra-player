// 实验用公共宿主 CLI：真实 Rust 后端与真实 Node 插件，无 Slint 窗口。
use crate::{
    backend::Backend,
    plugins::{Incoming, Manifest, Plugin},
    services::{Services, validate},
};
use serde_json::{Value as Json, json};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
struct Pending {
    caller: String,
    id: Json,
    replaced: Option<String>,
    started: Instant,
}
struct Headless {
    disposal_errors: Vec<String>,
    retiring: crate::retirement::Retirement,
    dialogs: std::collections::VecDeque<Option<String>>,
    backend: Backend,
    services: Services,
    plugins: Vec<Plugin>,
    sender: mpsc::Sender<Incoming>,
    pending: HashMap<String, Pending>,
    serial: u64,
}
impl Headless {
    fn state(&self) -> Json {
        let s = self.backend.snapshot();
        json!({"path":if self.services.source.is_some(){Json::Null}else{json!(s.path.map(|p|p.to_string_lossy().to_string()))},"duration":if self.services.source.is_some(){Json::Null}else{json!(s.duration)},"position":s.position,"paused":self.services.source.as_ref().map(|s|s.paused).unwrap_or(s.paused),"gaussianEnabled":s.gaussian_enabled,"sigma":s.sigma,"sessionId":self.services.session_id(),"sourceKind":if self.services.source.is_some(){"external"}else if s.duration>0.{"file"}else{"none"}})
    }
    fn response(&mut self, owner: &str, id: Json, result: Result<Json, String>) {
        let message = match result {
            Ok(result) => json!({"type":"response","id":id,"result":result}),
            Err(error) => json!({"type":"response","id":id,"error":error}),
        };
        if owner == "$cli" {
            println!("{message}");
            let _ = std::io::stdout().flush();
        } else if let Some(p) = self.plugins.iter_mut().find(|p| p.route == owner) {
            let _ = p.send(message);
        } else {
            let _ = self.retiring.reply(owner, message);
        }
    }
    fn unload(&mut self, route: &str) {
        self.unload_wait(route, None);
    }
    fn unload_wait(&mut self, route: &str, waiter: Option<(String, Json)>) {
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
    fn request(&mut self, owner: &str, method: &str, p: &Json) -> Result<Json, String> {
        if let Some(result) = self.services.request(owner, method, p, &mut self.backend)? {
            return Ok(result);
        }
        match method {
            "player.snapshot" => Ok(self.state()),
            "player.load" => {
                let path = p["path"].as_str().ok_or("missing path")?;
                self.backend.load(path.into())?;
                self.services.file_loaded(&mut self.backend)?;
                Ok(Json::Null)
            }
            "player.togglePause" => {
                self.backend.toggle_pause()?;
                Ok(Json::Null)
            }
            "player.seek" => {
                self.backend
                    .seek(p["seconds"].as_f64().ok_or("missing seconds")?)?;
                Ok(Json::Null)
            }
            "player.filters.setGaussian" => {
                self.backend.set_gaussian(
                    p["enabled"].as_bool().ok_or("missing enabled")?,
                    p["sigma"].as_f64().ok_or("missing sigma")?,
                )?;
                Ok(Json::Null)
            }
            "ui.patch" => {
                let properties = p["properties"].as_object().ok_or("missing properties")?;
                for v in properties.values() {
                    crate::ui::primitive(v)?;
                }
                let plugin = self
                    .plugins
                    .iter_mut()
                    .find(|p| p.route == owner)
                    .ok_or("unknown plugin")?;
                plugin.validate_patch(properties)?;
                plugin
                    .properties
                    .extend(properties.iter().map(|(k, v)| (k.clone(), v.clone())));
                Ok(Json::Null)
            }
            "dialogs.openMedia" | "dialogs.openFile" | "dialogs.saveFile" => Ok(self
                .dialogs
                .pop_front()
                .flatten()
                .map(|p| json!(p))
                .unwrap_or(Json::Null)),
            "host.dialogs" => {
                self.dialogs = p["paths"]
                    .as_array()
                    .ok_or("missing paths")?
                    .iter()
                    .map(|p| {
                        if p.is_null() {
                            Ok(None)
                        } else {
                            p.as_str()
                                .map(|s| Some(s.to_string()))
                                .ok_or("path not string")
                        }
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Json::Null)
            }
            "host.inspect" => Ok(
                json!({"playback":self.state(),"resources":self.services.diagnostics(),"plugins":self.plugins.iter().map(|p|json!({"id":p.manifest.id,"route":p.route,"properties":p.properties})).collect::<Vec<_>>(),"source":self.services.source.as_ref().map(|s|s.diagnostics())}),
            ),
            "host.load" => {
                let manifest =
                    Manifest::read(Path::new(p["manifest"].as_str().ok_or("missing manifest")?))?;
                crate::plugins::validate_unique(
                    &manifest,
                    self.plugins.iter().map(|p| &p.manifest),
                    false,
                )?;
                let id = manifest.id.clone();
                self.plugins
                    .push(Plugin::start(manifest, self.sender.clone())?);
                Ok(json!({"id":id}))
            }
            "host.action" => {
                let id = p["plugin"].as_str().ok_or("missing plugin")?;
                let plugin = self
                    .plugins
                    .iter_mut()
                    .find(|x| x.manifest.id == id)
                    .ok_or("unknown plugin")?;
                plugin.send(json!({"type":"action","name":p["name"],"value":p["value"].as_f64().unwrap_or(0.)}))?;
                Ok(Json::Null)
            }
            "host.input" => {
                let id = p["plugin"].as_str().ok_or("missing plugin")?;
                self.plugins
                    .iter_mut()
                    .find(|x| x.manifest.id == id)
                    .ok_or("unknown plugin")?
                    .send(
                        json!({"type":"input","event":{"key":p["key"],"pressed":p["pressed"]}}),
                    )?;
                Ok(Json::Null)
            }
            "host.unload" | "plugins.remove" => {
                let id = p["plugin"]
                    .as_str()
                    .or(p["id"].as_str())
                    .ok_or("missing plugin")?;
                let route = self
                    .plugins
                    .iter()
                    .find(|x| x.manifest.id == id)
                    .map(|x| x.route.clone())
                    .ok_or("unknown plugin")?;
                self.unload(&route);
                Ok(Json::Null)
            }
            _ => Err(format!("unknown method {method}")),
        }
    }
    fn incoming(&mut self, incoming: Incoming) {
        let m = incoming.message;
        let owner = incoming.plugin;
        if self.retiring.contains(&owner) {
            if m["type"] == "stopped"
                && let Some(error) = m["error"].as_str()
            {
                self.disposal_errors.push(format!("{owner}: {error}"));
            }
            if m["type"] == "request" {
                let result = self.retiring.cleanup_request(
                    &owner,
                    m["method"].as_str().unwrap_or(""),
                    &m["params"],
                    &mut self.services,
                    &mut self.backend,
                );
                self.response(&owner, m["id"].clone(), result);
            } else if (m["type"] == "stopped" || m["type"] == "fatal")
                && let Some((caller, id)) = self.retiring.finish(&owner)
            {
                let result = m["error"].as_str().map_or(Ok(Json::Null), |error| {
                    Err(format!("plugin disposal failed: {error}"))
                });
                self.response(&caller, id, result);
            }
            return;
        }
        match m["type"].as_str() {
            Some("request") => {
                let method = m["method"].as_str().unwrap_or("");
                if method == "host.unload" || method == "plugins.remove" {
                    let result = (|| {
                        crate::services::validate_params(method, &m["params"])?;
                        let id = m["params"]["plugin"]
                            .as_str()
                            .or(m["params"]["id"].as_str())
                            .ok_or("missing plugin")?;
                        let plugin = self
                            .plugins
                            .iter()
                            .find(|p| p.manifest.id == id)
                            .ok_or("unknown plugin")?;
                        if method == "plugins.remove"
                            && plugin.managed_by.as_deref() != Some(self.creator(&owner).as_str())
                        {
                            return Err(
                                "only caller-installed generated plugins may be removed".into()
                            );
                        }
                        Ok::<String, String>(plugin.route.clone())
                    })();
                    match result {
                        Ok(route) => {
                            self.unload_wait(&route, Some((owner.clone(), m["id"].clone())))
                        }
                        Err(error) => self.response(&owner, m["id"].clone(), Err(error)),
                    }
                    return;
                }
                if method == "plugins.install" {
                    let result = (|| {
                        crate::services::validate_params(method, &m["params"])?;
                        let manifest = validate(Path::new(
                            m["params"]["manifest"].as_str().ok_or("missing manifest")?,
                        ))?;
                        if self.plugins.iter().any(|p| {
                            p.manifest.id == manifest.id && self.pending.contains_key(&p.route)
                        }) {
                            return Err("plugin installation already pending".into());
                        }
                        crate::plugins::validate_unique(
                            &manifest,
                            self.plugins.iter().map(|p| &p.manifest),
                            true,
                        )?;
                        self.serial += 1;
                        let route = format!("{}#{}", manifest.id, self.serial);
                        let old = self
                            .plugins
                            .iter()
                            .find(|p| p.manifest.id == manifest.id)
                            .map(|p| p.route.clone());
                        let creator = self.creator(&owner);
                        if let Some(existing) =
                            self.plugins.iter().find(|p| p.manifest.id == manifest.id)
                            && existing.managed_by.as_deref() != Some(creator.as_str())
                        {
                            return Err(
                                "only caller-installed generated plugin can be replaced".into()
                            );
                        }
                        let mut plugin =
                            Plugin::start_routed(manifest, self.sender.clone(), route.clone())?;
                        plugin.managed_by = Some(creator);
                        self.plugins.push(plugin);
                        self.pending.insert(
                            route,
                            Pending {
                                caller: owner.clone(),
                                id: m["id"].clone(),
                                replaced: old,
                                started: Instant::now(),
                            },
                        );
                        Ok::<(), String>(())
                    })();
                    if let Err(e) = result {
                        self.response(&owner, m["id"].clone(), Err(e));
                    }
                } else {
                    let result = self.request(&owner, method, &m["params"]);
                    self.response(&owner, m["id"].clone(), result);
                }
            }
            Some("ready") => {
                if let Some(pending) = self.pending.remove(&owner) {
                    if let Some(old) = pending.replaced {
                        self.unload(&old);
                    }
                    let id = self
                        .plugins
                        .iter()
                        .find(|p| p.route == owner)
                        .map(|p| p.manifest.id.clone())
                        .unwrap_or_default();
                    self.response(&pending.caller, pending.id, Ok(json!({"id":id})));
                }
                let state = self.state();
                if let Some(p) = self.plugins.iter_mut().find(|p| p.route == owner) {
                    let _ = p.send(json!({"type":"playback","state":state}));
                }
            }
            Some("fatal") => {
                if let Some(pending) = self.pending.remove(&owner) {
                    self.response(
                        &pending.caller,
                        pending.id,
                        Err(m["error"].as_str().unwrap_or("activation failed").into()),
                    );
                }
                self.services.cleanup(&owner, &mut self.backend);
                self.plugins.retain(|plugin| plugin.route != owner);
            }
            _ => {}
        }
    }
}
pub fn run() -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let input = sender.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    let message=serde_json::from_str(&line).unwrap_or_else(|e|json!({"type":"request","id":null,"method":"invalid","params":{"error":e.to_string()}}));
                    if input.send(Incoming{plugin:"$cli".into(),message:json!({"type":"request","id":message["id"],"method":message["method"],"params":message["params"]})}).is_err(){break;}
                }
                Err(_) => break,
            }
        }
        let _ = input.send(Incoming {
            plugin: "$cli".into(),
            message: json!({"type":"eof"}),
        });
    });
    let mut host = Headless {
        disposal_errors: vec![],
        retiring: crate::retirement::Retirement::default(),
        dialogs: std::collections::VecDeque::new(),
        backend: Backend::new_silent()?,
        services: Services::new(),
        plugins: vec![],
        sender,
        pending: HashMap::new(),
        serial: 0,
    };
    let mut last = Instant::now();
    let mut closing = false;
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(m) if m.message["type"] == "eof" => {
                closing = true;
                let routes: Vec<_> = host.plugins.iter().map(|p| p.route.clone()).collect();
                for route in routes {
                    host.unload(&route);
                }
            }
            Ok(m) => host.incoming(m),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        for route in host.retiring.expired() {
            host.disposal_errors
                .push(format!("{route}: disposal timed out"));
            if let Some((caller, id)) = host.retiring.finish(&route) {
                host.response(&caller, id, Err("plugin disposal timed out".into()));
            }
        }
        if closing && host.plugins.is_empty() && host.retiring.is_empty() {
            break;
        }
        let expired: Vec<_> = host
            .pending
            .iter()
            .filter(|(_, p)| p.started.elapsed() > Duration::from_secs(25))
            .map(|(id, _)| id.clone())
            .collect();
        for route in expired {
            if let Some(pending) = host.pending.remove(&route) {
                host.unload(&route);
                host.response(
                    &pending.caller,
                    pending.id,
                    Err("plugin activation timeout".into()),
                );
            }
        }
        let _ = host.backend.poll();
        if last.elapsed() > Duration::from_millis(100) {
            last = Instant::now();
            let state = host.state();
            for p in &mut host.plugins {
                let _ = p.send(json!({"type":"playback","state":state}));
            }
        }
    }
    if !host.disposal_errors.is_empty() {
        return Err(host.disposal_errors.join("; "));
    }
    Ok(())
}
