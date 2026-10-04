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
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub experimental: bool,
    pub entry: PathBuf,
    pub ui: UiManifest,
    #[serde(skip)]
    pub ui_path: PathBuf,
    #[serde(skip)]
    pub entry_path: PathBuf,
}

#[derive(Clone, Deserialize)]
pub struct UiManifest {
    pub file: PathBuf,
    pub component: String,
    pub global: String,
    // 实验只有下方面板和单个侧栏，不是最终布局 API。
    #[serde(default)]
    pub region: Option<String>,
}

pub struct Incoming {
    pub plugin: String,
    pub message: Value,
}

pub struct Plugin {
    pub manifest: Manifest,
    child: Child,
    stdin: ChildStdin,
    pub properties: HashMap<String, Value>,
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn local_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute() {
        return Err("插件 entry 和 ui.file 必须是相对路径".into());
    }
    let file = root.join(relative).canonicalize().map_err(|e| e.to_string())?;
    if !file.starts_with(root) || !file.is_file() {
        return Err("插件文件必须位于 manifest 所在目录内".into());
    }
    Ok(file)
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let root = path.parent().ok_or("manifest 没有父目录")?;
        let mut manifest: Self = serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if !manifest.experimental || manifest.id.is_empty() || manifest.name.is_empty() {
            return Err("只接受带 experimental:true 的有名实验插件".into());
        }
        if !identifier(&manifest.ui.component) || !identifier(&manifest.ui.global) {
            return Err("Slint component/global 名称必须是 ASCII 标识符".into());
        }
        if manifest.ui.region.as_deref().is_some_and(|region| region != "sidebar") {
            return Err("实验 ui.region 仅支持 sidebar；省略表示下方面板".into());
        }
        manifest.ui_path = local_file(root, &manifest.ui.file)?;
        manifest.entry_path = local_file(root, &manifest.entry)?;
        Ok(manifest)
    }
}

impl Plugin {
    pub fn start(manifest: Manifest, sender: Sender<Incoming>) -> Result<Self, String> {
        let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/plugin-host.ts");
        let mut child = Command::new("node")
            .arg(runtime).arg(&manifest.entry_path)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().map_err(|e| format!("无法启动 Node 插件进程：{e}"))?;
        let stdin = child.stdin.take().ok_or("缺少插件 stdin")?;
        let stdout = child.stdout.take().ok_or("缺少插件 stdout")?;
        let id = manifest.id.clone();
        // 阻塞读取只在 worker 中；GUI timer 在主线程消费消息。
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let message = match line {
                    Ok(line) => match serde_json::from_str(&line) {
                        Ok(message) => message,
                        Err(error) => json!({"type":"fatal","error":format!("插件 stdout 非协议 JSON：{error}")}),
                    },
                    Err(error) => json!({"type":"fatal","error":error.to_string()}),
                };
                if sender.send(Incoming { plugin: id.clone(), message }).is_err() { return; }
            }
            let _ = sender.send(Incoming { plugin: id, message: json!({"type":"fatal","error":"插件进程已结束"}) });
        });
        Ok(Self { manifest, child, stdin, properties: HashMap::new() })
    }

    pub fn send(&mut self, message: Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, &message).map_err(|e| e.to_string())?;
        self.stdin.write_all(b"\n").and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.send(json!({"type":"shutdown"}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
