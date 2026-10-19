// 实验版本：本地可信插件的临时 stdio 协议，与最终成品 API 可能存在很大差异。
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::Sender,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub experimental: bool,
    pub entry: PathBuf,
    pub ui: UiManifest,
    #[serde(default)]
    pub commands: Vec<CommandManifest>,
    #[serde(skip)]
    pub ui_path: PathBuf,
    #[serde(skip)]
    pub entry_path: PathBuf,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandManifest {
    pub id: String,
    pub title: String,
    pub menu: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiManifest {
    pub file: PathBuf,
    pub component: String,
    pub global: String,
    // 实验挂载点约定；controls 唯一常驻，不是最终布局 API。
    #[serde(default)]
    pub region: Option<String>,
}

pub struct Incoming {
    pub plugin: String,
    pub message: Value,
}

pub struct Plugin {
    pub manifest: Manifest,
    pub route: String,
    pub managed_by: Option<String>,
    child: Child,
    stdin: ChildStdin,
    pub properties: HashMap<String, Value>,
    property_types: HashMap<String, slint_interpreter::ValueType>,
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn local_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute() {
        return Err("插件 entry 和 ui.file 必须是相对路径".into());
    }
    let file = root
        .join(relative)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !file.starts_with(root) || !file.is_file() {
        return Err("插件文件必须位于 manifest 所在目录内".into());
    }
    Ok(file)
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let root = path.parent().ok_or("manifest 没有父目录")?;
        let mut manifest: Self =
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if !manifest.experimental
            || manifest.id.is_empty()
            || manifest.name.is_empty()
            || manifest.id.len() > 100
            || !manifest
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("只接受带 experimental:true 的有名实验插件".into());
        }
        if !identifier(&manifest.ui.component) || !identifier(&manifest.ui.global) {
            return Err("Slint component/global 名称必须是 ASCII 标识符".into());
        }
        if manifest
            .ui
            .region
            .as_deref()
            .is_some_and(|region| !["sidebar", "controls"].contains(&region))
        {
            return Err("实验 ui.region 仅支持 sidebar/controls；省略表示工具面板".into());
        }
        let mut ids = std::collections::HashSet::new();
        for command in &manifest.commands {
            if command.id.is_empty()
                || command.title.is_empty()
                || command.menu.is_empty()
                || command.menu.iter().any(|p| p.is_empty())
                || !ids.insert(&command.id)
            {
                return Err("invalid/duplicate command".into());
            }
        }
        manifest.ui_path = local_file(root, &manifest.ui.file)?;
        manifest.entry_path = local_file(root, &manifest.entry)?;
        Ok(manifest)
    }
}

impl Plugin {
    pub fn start(manifest: Manifest, sender: Sender<Incoming>) -> Result<Self, String> {
        let route = manifest.id.clone();
        Self::start_routed(manifest, sender, route)
    }
    pub fn start_routed(
        manifest: Manifest,
        sender: Sender<Incoming>,
        route: String,
    ) -> Result<Self, String> {
        let definition = crate::ui::compile(std::slice::from_ref(&manifest))?;
        let property_types = definition
            .global_properties(&manifest.ui.global)
            .ok_or("missing exported UI global")?
            .collect();
        if !definition
            .global_callbacks(&manifest.ui.global)
            .is_some_and(|mut callbacks| callbacks.any(|name| name == "action"))
        {
            return Err("UI global requires action callback".into());
        }
        let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/plugin-host.ts");
        let mut child = Command::new("node")
            .arg(runtime)
            .arg(&manifest.entry_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("无法启动 Node 插件进程：{e}"))?;
        let stdin = child.stdin.take().ok_or("缺少插件 stdin")?;
        let stdout = child.stdout.take().ok_or("缺少插件 stdout")?;
        let id = route.clone();
        // 阻塞读取只在 worker 中；GUI timer 在主线程消费消息。
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let message = match line {
                    Ok(line) => match serde_json::from_str(&line) {
                        Ok(message) => message,
                        Err(error) => {
                            json!({"type":"fatal","error":format!("插件 stdout 非协议 JSON：{error}")})
                        }
                    },
                    Err(error) => json!({"type":"fatal","error":error.to_string()}),
                };
                if sender
                    .send(Incoming {
                        plugin: id.clone(),
                        message,
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = sender.send(Incoming {
                plugin: id,
                message: json!({"type":"fatal","error":"插件进程已结束"}),
            });
        });
        Ok(Self {
            manifest,
            route,
            managed_by: None,
            child,
            stdin,
            properties: HashMap::new(),
            property_types,
        })
    }

    pub fn validate_patch(
        &self,
        properties: &serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        use slint_interpreter::ValueType;
        for (name, value) in properties {
            let kind = self
                .property_types
                .get(name)
                .ok_or_else(|| format!("unknown Slint property {name}"))?;
            if !matches!(
                (kind, value),
                (ValueType::String, Value::String(_))
                    | (ValueType::Bool, Value::Bool(_))
                    | (ValueType::Number, Value::Number(_))
            ) {
                return Err(format!("invalid Slint property type for {name}"));
            }
        }
        Ok(())
    }
    pub fn send(&mut self, message: Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, &message).map_err(|e| e.to_string())?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| e.to_string())
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.send(json!({"type":"shutdown"}));
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn validate_unique<'a>(
    candidate: &Manifest,
    existing: impl Iterator<Item = &'a Manifest>,
    replacement: bool,
) -> Result<(), String> {
    for manifest in existing {
        if replacement && manifest.id == candidate.id {
            continue;
        }
        if candidate.ui.region.as_deref() == Some("controls")
            && manifest.ui.region.as_deref() == Some("controls")
        {
            return Err("only one controls region plugin may be installed".into());
        }
        if manifest.id == candidate.id
            || manifest.ui.component == candidate.ui.component
            || manifest.ui.global == candidate.ui.global
        {
            return Err("plugin id, Slint component and global must be unique".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Manifest, UiManifest, validate_unique};
    use std::path::PathBuf;

    fn manifest(id: &str, region: Option<&str>) -> Manifest {
        Manifest {
            id: id.into(),
            name: id.into(),
            experimental: true,
            entry: PathBuf::from("index.ts"),
            entry_path: PathBuf::new(),
            ui_path: PathBuf::new(),
            ui: UiManifest {
                file: PathBuf::from("ui.slint"),
                component: format!("{id}Panel"),
                global: format!("{id}State"),
                region: region.map(str::to_owned),
            },
            commands: vec![],
        }
    }

    #[test]
    fn controls_uniqueness_allows_same_id_replacement_but_no_other_control() {
        let old = manifest("Existing", Some("controls"));
        let updated = manifest("Existing", Some("controls"));
        let different = manifest("Other", Some("controls"));
        let tool = manifest("Tool", None);
        assert!(validate_unique(&updated, [&old, &tool].into_iter(), true).is_ok());
        assert!(validate_unique(&different, [&old, &tool].into_iter(), true).is_err());
        assert!(validate_unique(&updated, [&old, &different].into_iter(), true).is_err());
        assert!(validate_unique(&tool, [&old].into_iter(), false).is_ok());
    }
}
