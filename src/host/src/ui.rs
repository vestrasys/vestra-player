// 实验版本：只验证 Slint 原生界面组合；与最终成品 API 可能存在很大差异。
use crate::plugins::Manifest;
use slint_interpreter::{Compiler, ComponentDefinition, ComponentInstance, Value};
use std::path::PathBuf;

#[derive(Default)]
struct MenuNode {
    title: String,
    children: Vec<MenuNode>,
    actions: Vec<(String, String, String)>,
}

impl MenuNode {
    fn insert(&mut self, path: &[String], action: (String, String, String)) {
        if let Some((title, rest)) = path.split_first() {
            let index = self
                .children
                .iter()
                .position(|node| node.title == *title)
                .unwrap_or_else(|| {
                    self.children.push(MenuNode {
                        title: title.clone(),
                        ..Default::default()
                    });
                    self.children.len() - 1
                });
            self.children[index].insert(rest, action);
        } else {
            self.actions.push(action);
        }
    }

    fn slint(&self) -> Result<String, String> {
        let mut output = String::new();
        for child in &self.children {
            output.push_str(&format!(
                "Menu {{ title: {}; {} }}\n",
                literal(&child.title)?,
                child.slint()?
            ));
        }
        for (owner, id, title) in &self.actions {
            output.push_str(&format!(
                "MenuItem {{ title: {}; activated => {{ root.plugin-command({}, {}); }} }}\n",
                literal(title)?,
                literal(owner)?,
                literal(id)?,
            ));
        }
        Ok(output)
    }
}

fn literal(text: &str) -> Result<String, String> {
    if text.chars().any(char::is_control) {
        return Err("Slint 菜单和插件标识不能包含控制字符".into());
    }
    serde_json::to_string(text).map_err(|error| error.to_string())
}

pub fn compose(manifests: &[Manifest]) -> Result<ComponentInstance, String> {
    compile(manifests)?.create().map_err(|e| e.to_string())
}

// 编译验证不创建窗口、不启动播放后端，适用于 --check-ui。
pub fn compile(manifests: &[Manifest]) -> Result<ComponentDefinition, String> {
    let mut imports = String::new();
    let mut panels = String::new();
    let mut sidebar = String::new();
    let mut sidebar_menu = String::new();
    let mut panel_menu = String::new();
    let mut active_panel = String::new();
    let mut commands = MenuNode::default();
    for manifest in manifests {
        for command in &manifest.commands {
            if command.menu.len() > 8 || command.menu.iter().any(|part| part.trim().is_empty()) {
                return Err("实验菜单路径必须非空且深度不超过 8".into());
            }
            commands.insert(
                &command.menu,
                (
                    manifest.id.clone(),
                    command.id.clone(),
                    command.title.clone(),
                ),
            );
        }
        let path = manifest.ui_path.to_str().ok_or("界面路径必须是 UTF-8")?;
        // Slint import 路径不解析 JSON 转义；拒绝不能直接写入路径字面量的字符。
        if path.contains('"') || path.chars().any(char::is_control) {
            return Err("实验 Slint 文件路径不能包含引号或控制字符".into());
        }
        let import_literal = format!("\"{}\"", path.replace('\\', "/"));
        imports.push_str(&format!(
            "import {{ {}, {} }} from {};\nexport {{ {} }} from {};\n",
            manifest.ui.component,
            manifest.ui.global,
            import_literal,
            manifest.ui.global,
            import_literal
        ));
        if manifest.ui.region.as_deref() == Some("sidebar") {
            if !sidebar.is_empty() {
                return Err("实验只允许一个 sidebar 插件".into());
            }
            sidebar = format!(
                "if {}.visible : {} {{ }}",
                manifest.ui.global, manifest.ui.component
            );
            sidebar_menu = format!(
                "MenuItem {{ title: {}.visible ? {} : {}; activated => {{ {}.action(\"toggle\", 0); }} }}",
                manifest.ui.global,
                literal(&format!("隐藏{}", manifest.name))?,
                literal(&format!("显示{}", manifest.name))?,
                manifest.ui.global,
            );
        } else {
            active_panel = manifest.id.clone();
            panels.push_str(&format!(
                "if root.active-panel == {} : {} {{ }}\n",
                literal(&manifest.id)?,
                manifest.ui.component
            ));
            panel_menu.push_str(&format!(
                "MenuItem {{ title: {}; activated => {{ root.active-panel = {}; }} }}\n",
                literal(&manifest.name)?,
                literal(&manifest.id)?,
            ));
        }
    }
    let command_menu = commands.slint()?;
    let active_panel_literal = literal(&active_panel)?;
    // 这是普通 Slint 源码组合，不是自建 GUI 描述语言。
    // 添加插件会重新编译并替换窗口；不等同运行中任意插入组件。
    let source = format!(
        r#"// 实验版本，与最终成品 API 可能存在很大差异。
{imports}
export component ExperimentalHost inherits Window {{
    title: "Vestra · Slint 实验";
    preferred-width: 960px; preferred-height: 640px;
    min-width: 800px; min-height: 400px;
    background: #121820;
    in property <image> frame;
    in property <string> error;
    in-out property <string> active-panel: {active_panel_literal};
    callback add-plugin();
    callback plugin-command(string, string);
    callback source-input(string, bool);
    function input-key(code: string) -> string {{
        if code == Key.LeftArrow {{ return "ArrowLeft"; }}
        if code == Key.RightArrow {{ return "ArrowRight"; }}
        if code == Key.UpArrow {{ return "ArrowUp"; }}
        if code == Key.DownArrow {{ return "ArrowDown"; }}
        if code == Key.Return {{ return "Enter"; }}
        if code == Key.Shift || code == Key.ShiftR {{ return "Shift"; }}
        return code;
    }}
    forward-focus: source-focus;
        source-focus := FocusScope {{
            width: 100%; height: 100%;
            key-pressed(event) => {{ root.source-input(root.input-key(event.text), true); return reject; }}
            key-released(event) => {{ root.source-input(root.input-key(event.text), false); return reject; }}
        HorizontalLayout {{
            spacing: 0px;
            VerticalLayout {{
                spacing: 0px;
                horizontal-stretch: 1;
                Rectangle {{
                    vertical-stretch: 1;
                    background: black;
                    Image {{ width: 100%; height: 100%; source: root.frame; image-fit: contain; }}
                    TouchArea {{ clicked => {{ source-focus.focus(); }} }}
                }}
                {panels}
                if root.error != "" : Text {{
                    text: root.error; color: #ffb5b5;
                    wrap: word-wrap;
                }}
            }}
            {sidebar}
        }}
        }}
    // TouchArea 会抓取右键；将菜单区域放在内容上层，左键仍交给下方控件。
    ContextMenuArea {{
        width: 100%; height: 100%;
        Menu {{
            MenuItem {{ title: "添加实验插件…"; activated => {{ root.add-plugin(); }} }}
            {sidebar_menu}
            Menu {{ title: "插件界面"; {panel_menu} }}
            {command_menu}
        }}
    }}
}}
"#
    );
    let compiler = Compiler::default();
    let result = spin_on::spin_on(
        compiler.build_from_source(source, PathBuf::from("experimental-host.slint")),
    );
    if result.has_errors() {
        return Err(result
            .diagnostics()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n"));
    }
    result
        .component("ExperimentalHost")
        .ok_or("缺少主窗口组件".into())
}

pub fn primitive(value: &serde_json::Value) -> Result<Value, String> {
    match value {
        serde_json::Value::Bool(v) => Ok(Value::Bool(*v)),
        serde_json::Value::String(v) => Ok(Value::String(v.as_str().into())),
        serde_json::Value::Number(v) => v.as_f64().map(Value::Number).ok_or("无效数值".into()),
        _ => Err("ui.patch 只支持 string、number、boolean；实验不包装 Slint 全部类型".into()),
    }
}
