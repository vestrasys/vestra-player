//! Direct Slint interpreter integration, with primitive property and callback transport.
use crate::Event;
use anyhow::{Result, bail};
use serde_json::{Value as Json, json};
use slint_interpreter::{Compiler, ComponentHandle, ComponentInstance, Value};
use std::{collections::BTreeMap, path::Path, sync::mpsc};
pub fn compile(path: &Path) -> Result<ComponentInstance> {
    compile_source(std::fs::read_to_string(path)?, path)
}
pub fn compile_source(mut source: String, path: &Path) -> Result<ComponentInstance> {
    let compiler = Compiler::default();
    // Slint's documented font import is stable; the runtime Fontique adapter is
    // explicitly unstable in 1.16 and is deliberately not used here.
    #[cfg(target_os = "macos")]
    if Path::new("/System/Library/Fonts/Supplemental/Arial Unicode.ttf").is_file() {
        // Plugin tool windows do not inherit the main window's family; without it the
        // renderer's default face has no CJK glyphs and Chinese labels render blank.
        if !source.contains("default-font-family") {
            source = with_window_font(&source, "Arial Unicode MS");
        }
        source =
            format!("import \"/System/Library/Fonts/Supplemental/Arial Unicode.ttf\";\n{source}");
    }
    let result = pollster::block_on(compiler.build_from_source(source, path.to_owned()));
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
pub use crate::ui_values::from_json;
pub struct View {
    pub instance: ComponentInstance,
}
pub fn open(
    path: &Path,
    owner: u64,
    id: &str,
    properties: &BTreeMap<String, Json>,
    callbacks: &[String],
    events: mpsc::Sender<Event>,
) -> Result<View> {
    let instance = compile(path)?;
    crate::ui_values::own_default_models(&instance, None)?;
    for (name, value) in properties {
        instance.set_property(name, from_json(value)?)?;
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
    instance.show()?;
    Ok(View { instance })
}
