// 实验版本：只验证 Slint 原生界面组合；与最终成品 API 可能存在很大差异。
use crate::plugins::Manifest;
use slint_interpreter::{Compiler, ComponentDefinition, ComponentInstance, Value};
use std::path::PathBuf;

pub fn compose(manifests: &[Manifest]) -> Result<ComponentInstance, String> {
    compile(manifests)?.create().map_err(|e| e.to_string())
}

// 编译验证不创建窗口、不启动播放后端，适用于 --check-ui。
pub fn compile(manifests: &[Manifest]) -> Result<ComponentDefinition, String> {
    let mut imports = String::new();
    let mut panels = String::new();
    let mut sidebar = String::new();
    let mut sidebar_menu = String::new();
    for manifest in manifests {
        let path = manifest.ui_path.to_str().ok_or("界面路径必须是 UTF-8")?;
        // Slint import 路径不解析 JSON 转义；拒绝不能直接写入路径字面量的字符。
        if path.contains('"') || path.chars().any(char::is_control) {
            return Err("实验 Slint 文件路径不能包含引号或控制字符".into());
        }
        let literal = format!("\"{}\"", path.replace('\\', "/"));
        imports.push_str(&format!(
            "import {{ {}, {} }} from {};\nexport {{ {} }} from {};\n",
            manifest.ui.component, manifest.ui.global, literal, manifest.ui.global, literal
        ));
        if manifest.ui.region.as_deref() == Some("sidebar") {
            if !sidebar.is_empty() { return Err("实验只允许一个 sidebar 插件".into()); }
            sidebar = format!("if {}.visible : {} {{ }}", manifest.ui.global, manifest.ui.component);
            sidebar_menu = format!(
                "MenuItem {{ title: {}.visible ? \"隐藏播放列表\" : \"显示播放列表\"; activated => {{ {}.action(\"toggle\", 0); }} }}",
                manifest.ui.global, manifest.ui.global,
            );
        } else {
            panels.push_str(&format!("{} {{ }}\n", manifest.ui.component));
        }
    }
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
    callback add-plugin();
    ContextMenuArea {{
        width: 100%; height: 100%;
        Menu {{
            MenuItem {{ title: "添加实验插件…"; activated => {{ root.add-plugin(); }} }}
            {sidebar_menu}
        }}
        HorizontalLayout {{
            spacing: 0px;
            VerticalLayout {{
                spacing: 0px;
                horizontal-stretch: 1;
                Rectangle {{
                    vertical-stretch: 1;
                    background: black;
                    Image {{ width: 100%; height: 100%; source: root.frame; image-fit: contain; }}
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
}}
"#
    );
    let compiler = Compiler::default();
    let result = spin_on::spin_on(compiler.build_from_source(source, PathBuf::from("experimental-host.slint")));
    if result.has_errors() {
        return Err(result.diagnostics().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"));
    }
    result.component("ExperimentalHost").ok_or("缺少主窗口组件".into())
}

pub fn primitive(value: &serde_json::Value) -> Result<Value, String> {
    match value {
        serde_json::Value::Bool(v) => Ok(Value::Bool(*v)),
        serde_json::Value::String(v) => Ok(Value::String(v.as_str().into())),
        serde_json::Value::Number(v) => v.as_f64().map(Value::Number).ok_or("无效数值".into()),
        _ => Err("ui.patch 只支持 string、number、boolean；实验不包装 Slint 全部类型".into()),
    }
}
