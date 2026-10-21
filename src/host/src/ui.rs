//! Direct Slint interpreter integration, with primitive property and callback transport.
use crate::Event;
use anyhow::{Result, bail};
use serde_json::json;
use slint_interpreter::{Compiler, ComponentHandle, ComponentInstance, Value};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

/// Host-shipped Slint directory: `Resources/ui` in a bundle, else the source tree.
pub fn ui_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources/ui")))
        .filter(|dir| dir.join("host.slint").is_file())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"))
}

/// The one Slint compiler configuration for the shell, panels, tool windows and
/// package validation: `@vestra/...` imports and the dark std-widgets style.
pub fn compiler() -> Compiler {
    let mut compiler = Compiler::default();
    compiler.set_library_paths(
        [("vestra".to_owned(), ui_dir().join("vestra"))]
            .into_iter()
            .collect(),
    );
    compiler.set_style("fluent-dark".into());
    compiler
}

const HIRAGINO: &str = "/System/Library/Fonts/Hiragino Sans GB.ttc";

/// Default UI font. Without a forced family Slint 1.16 renders CJK text in
/// plugin windows as blank (no per-script fallback here, GUI-verified
/// 2026-10-20), so the system Hiragino Sans GB file is imported and made the
/// default family whenever it exists (§18 Q12). `VESTRA_FONT_FALLBACK=system`
/// opts out, for checking a future Slint's own fallback.
pub fn font() -> Option<(&'static str, &'static str)> {
    (std::env::var("VESTRA_FONT_FALLBACK").as_deref() != Ok("system")
        && Path::new(HIRAGINO).is_file())
    .then_some(("Hiragino Sans GB", HIRAGINO))
}

/// Source prefix that makes the forced font available (empty by default).
pub fn font_import() -> String {
    font()
        .map(|(_, path)| format!("import \"{path}\";\n"))
        .unwrap_or_default()
}

pub fn compile(path: &Path) -> Result<ComponentInstance> {
    compile_source(std::fs::read_to_string(path)?, path)
}
pub fn compile_source(mut source: String, path: &Path) -> Result<ComponentInstance> {
    if let Some((family, _)) = font()
        && !source.contains("default-font-family")
    {
        source = format!("{}{}", font_import(), with_window_font(&source, family));
    }
    let result = pollster::block_on(compiler().build_from_source(source, path.to_owned()));
    if result.components().count() > 1 {
        bail!("Slint file must export exactly one component");
    }
    let definition = result.components().next().ok_or_else(|| {
        anyhow::anyhow!(
            "Slint compilation: {:?}",
            result.diagnostics().collect::<Vec<_>>()
        )
    })?;
    definition.create().map_err(Into::into)
}
/// Insert a default font family into the exported root Window, if there is one.
fn with_window_font(source: &str, family: &str) -> String {
    let Some(start) = source.find("export component ") else {
        return source.to_owned();
    };
    let Some(brace) = source[start..].find('{').map(|i| start + i) else {
        return source.to_owned();
    };
    if !source[start..brace].contains("inherits Window") {
        return source.to_owned();
    }
    format!(
        "{}{{\n    default-font-family: \"{family}\";{}",
        &source[..brace],
        &source[brace + 1..]
    )
}
pub struct View {
    pub instance: ComponentInstance,
}
pub fn open(
    path: &Path,
    owner: u64,
    id: &str,
    properties: &[(String, Value)],
    callbacks: &[String],
    events: mpsc::Sender<Event>,
) -> Result<View> {
    let instance = compile(path)?;
    crate::ui_values::own_default_models(&instance, None)?;
    for (name, value) in properties {
        instance.set_property(name, value.clone())?;
    }
    for callback in callbacks {
        let events = events.clone();
        let name = callback.clone();
        let id = id.to_owned();
        instance.set_callback(callback, move |args| {
            match args
                .iter()
                .map(crate::ui_values::to_json)
                .collect::<Result<Vec<_>>>()
            {
                Ok(args) => {
                    let _ = events.send(Event::UiCallback(
                        owner,
                        json!({"view":id,"callback":name,"args":args}),
                    ));
                }
                Err(error) => {
                    let _ = events.send(Event::Broadcast(
                        "host.error".into(),
                        json!({"owner":owner,"code":"ui-value","message":error.to_string()}),
                    ));
                }
            }
            Value::Void
        })?;
    }
    // The title bar's close button: the view goes away like `ui.close`, and
    // its plugin learns that the user closed it.
    let closed = id.to_owned();
    instance.window().on_close_requested(move || {
        let _ = events.send(Event::ViewClosed(owner, closed.clone()));
        slint::CloseRequestResponse::HideWindow
    });
    instance.show()?;
    Ok(View { instance })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_files_import_the_vestra_library_and_compile_dark() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("w.slint");
        let source = r#"
import { Button as StdButton } from "std-widgets.slint";
import { Theme } from "@vestra/theme.slint";
import { Button, Slider, ToolTipArea, UrlInputRow } from "@vestra/widgets.slint";
export component W inherits Window {
    background: Theme.bg-panel;
    in-out property <string> url;
    callback open(string);
    init => { link.focus-input(); }
    VerticalLayout {
        link := UrlInputRow {
            text <=> root.url;
            placeholder: "https://…";
            accepted(text) => { root.open(text); }
            cancelled => { root.url = ""; }
        }
        ToolTipArea { text: "tip"; Text { text: "亮度"; color: Theme.text; } }
        Slider { panel: true; value: 0.5; }
        Slider { value: 0.2; }
        Button { text: "默认"; small: true; }
        Button { icon: "M 3 2.5 L 11 10.5 M 11 2.5 L 3 10.5"; stroked: true; checked: true; width: 25px; }
        Button { icon: "M 0 0 L 12 7 L 0 14 Z"; icon-width: 12px; icon-height: 14px; }
        StdButton { text: "std"; }
    }
}"#;
        let result = pollster::block_on(compiler().build_from_source(source.into(), file));
        let errors: Vec<_> = result
            .diagnostics()
            .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(result.component("W").is_some());
    }

    /// Every plugin and example Slint file compiles with the host's configuration.
    #[test]
    fn all_plugin_and_example_slint_files_compile() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if path.is_dir() {
                    if !["dist", "node_modules", "vendor", "tests"]
                        .contains(&name.to_str().unwrap_or(""))
                    {
                        walk(&path, out);
                    }
                } else if path.extension().is_some_and(|e| e == "slint") {
                    out.push(path);
                }
            }
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        walk(&root.join("plugins"), &mut files);
        walk(&root.join("examples"), &mut files);
        assert!(files.len() >= 15, "found {files:?}");
        for file in files {
            let result = pollster::block_on(compiler().build_from_path(&file));
            let errors: Vec<_> = result
                .diagnostics()
                .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
                .map(|d| d.to_string())
                .collect();
            assert!(errors.is_empty(), "{}: {errors:?}", file.display());
        }
    }
}
