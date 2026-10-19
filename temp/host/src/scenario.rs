// 实验 GUI 场景驱动：只使用公开窗口/回调能力，不代表成品 API。
use crate::{Host, ui};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use slint::ComponentHandle;
use slint_interpreter::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    output: PathBuf,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
enum Step {
    Load {
        manifest: PathBuf,
    },
    Dialogs {
        paths: Vec<Option<PathBuf>>,
    },
    Request {
        plugin: String,
        method: String,
        params: Json,
    },
    Action {
        plugin: String,
        name: String,
        #[serde(default)]
        value: f64,
    },
    Menu {
        plugin: String,
        command: String,
    },
    Input {
        key: String,
        pressed: bool,
    },
    Panel {
        plugin: String,
    },
    WaitProperty {
        plugin: String,
        name: String,
        value: Json,
    },
    WaitFrames {
        minimum: u64,
    },
    Delay {
        milliseconds: u64,
    },
    Snapshot {
        name: String,
    },
}

pub struct Driver {
    scenario: Scenario,
    root: PathBuf,
    index: usize,
    started: Instant,
    observations: Vec<Json>,
}

impl Driver {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let root = path
            .parent()
            .ok_or("scenario has no directory")?
            .to_path_buf();
        let mut scenario: Scenario =
            serde_json::from_slice(&std::fs::read(&path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if scenario.steps.is_empty() {
            return Err("scenario requires steps".into());
        }
        if !scenario.output.is_absolute() {
            scenario.output = root.join(&scenario.output);
        }
        std::fs::create_dir_all(&scenario.output).map_err(|error| error.to_string())?;
        Ok(Self {
            scenario,
            root,
            index: 0,
            started: Instant::now(),
            observations: vec![],
        })
    }

    fn action(&self, host: &Host, plugin: &str, name: &str, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("non-finite action value".into());
        }
        let plugin = host
            .plugins
            .iter()
            .find(|p| p.manifest.id == plugin)
            .ok_or("plugin not loaded")?;
        host.window
            .invoke_global(
                &plugin.manifest.ui.global,
                "action",
                &[Value::String(name.into()), Value::Number(value)],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn write_report(&self, passed: bool, error: Option<&str>) -> Result<(), String> {
        std::fs::write(self.scenario.output.join("report.json"), serde_json::to_vec_pretty(&json!({
            "experimental": true, "passed": passed, "error": error, "step": self.index,
            "observations": self.observations,
            "scope": "real Slint window, global callbacks and menu callbacks; not physical mouse/keyboard automation",
        })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())
    }

    pub fn step(&mut self, host: &mut Host) -> Result<bool, String> {
        match self.perform(host) {
            Ok(done) => Ok(done),
            Err(error) => {
                let message = format!("GUI scenario step {}: {error}", self.index);
                self.write_report(false, Some(&message))?;
                Err(message)
            }
        }
    }

    fn perform(&mut self, host: &mut Host) -> Result<bool, String> {
        let Some(step) = self.scenario.steps.get(self.index) else {
            self.write_report(true, None)?;
            return Ok(true);
        };
        if self.started.elapsed() > Duration::from_secs(40) {
            return Err("timed out waiting for observable state".into());
        }
        let label = match step {
            Step::Dialogs { paths } => {
                let fixtures = host
                    .dialog_fixtures
                    .as_mut()
                    .ok_or("dialog fixtures require --ui-scenario")?;
                *fixtures = paths
                    .iter()
                    .map(|path| {
                        path.as_ref().map(|path| {
                            if path.is_absolute() {
                                path.clone()
                            } else {
                                self.root.join(path)
                            }
                        })
                    })
                    .collect();
                "dialogs"
            }
            Step::Load { manifest } => {
                let manifest = if manifest.is_absolute() {
                    manifest.clone()
                } else {
                    self.root.join(manifest)
                };
                host.load_plugin(&manifest)?;
                "load"
            }
            Step::Request {
                plugin,
                method,
                params,
            } => {
                let index = host
                    .plugins
                    .iter()
                    .position(|p| &p.manifest.id == plugin)
                    .ok_or("plugin not loaded")?;
                host.request(index, method, params)?;
                "request"
            }
            Step::Action {
                plugin,
                name,
                value,
            } => {
                self.action(host, plugin, name, *value)?;
                "action"
            }
            Step::Menu { plugin, command } => {
                let owner = host
                    .plugins
                    .iter()
                    .find(|p| &p.manifest.id == plugin)
                    .ok_or("plugin not loaded")?;
                if !owner.manifest.commands.iter().any(|c| &c.id == command) {
                    return Err("undeclared menu command".into());
                }
                host.window
                    .invoke(
                        "plugin-command",
                        &[
                            Value::String(plugin.as_str().into()),
                            Value::String(command.as_str().into()),
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                "menu"
            }
            Step::Input { key, pressed } => {
                host.window
                    .invoke(
                        "source-input",
                        &[Value::String(key.as_str().into()), Value::Bool(*pressed)],
                    )
                    .map_err(|error| error.to_string())?;
                "input"
            }
            Step::Panel { plugin } => {
                if !host.plugins.iter().any(|p| &p.manifest.id == plugin) {
                    return Err("plugin not loaded".into());
                }
                host.window
                    .set_property("active-panel", Value::String(plugin.as_str().into()))
                    .map_err(|error| error.to_string())?;
                "panel"
            }
            Step::WaitProperty {
                plugin,
                name,
                value,
            } => {
                let Some(plugin) = host.plugins.iter().find(|p| &p.manifest.id == plugin) else {
                    return Ok(false);
                };
                let expected = ui::primitive(value)?;
                if host
                    .window
                    .get_global_property(&plugin.manifest.ui.global, name)
                    .ok()
                    != Some(expected)
                {
                    return Ok(false);
                }
                "wait-property"
            }
            Step::WaitFrames { minimum } => {
                if (host.received_frames as u64) < *minimum {
                    return Ok(false);
                }
                "wait-frames"
            }
            Step::Delay { milliseconds } => {
                if *milliseconds > 30_000 {
                    return Err("scenario delay exceeds 30 seconds".into());
                }
                if self.started.elapsed() < Duration::from_millis(*milliseconds) {
                    return Ok(false);
                }
                "delay"
            }
            Step::Snapshot { name } => {
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err("invalid screenshot name".into());
                }
                let image = host
                    .window
                    .window()
                    .take_snapshot()
                    .map_err(|error| error.to_string())?;
                image::save_buffer(
                    self.scenario.output.join(format!("{name}.png")),
                    image.as_bytes(),
                    image.width(),
                    image.height(),
                    image::ColorType::Rgba8,
                )
                .map_err(|error| error.to_string())?;
                "snapshot"
            }
        };
        let properties: Vec<Json> = host
            .plugins
            .iter()
            .map(|p| json!({"id":p.manifest.id,"properties":p.properties}))
            .collect();
        self.observations.push(json!({"step":self.index,"op":label,"elapsedMs":self.started.elapsed().as_millis(),
            "playback":host.state(),"plugins":properties,"frames":host.received_frames,
            "compositionCount":host.composition_count,"windowGeneration":host.window_generation,
            "activePanel":host.window.get_property("active-panel").ok().and_then(|v|if let Value::String(s)=v {Some(s.to_string())}else{None}),
        }));
        self.index += 1;
        self.started = Instant::now();
        Ok(false)
    }
}
