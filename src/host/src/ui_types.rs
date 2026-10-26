//! Property values plugins send, type-checked against their compiled Slint.
//!
//! The SDK's test host keeps a plugin's window, panel and settings page
//! properties as plain JSON: it does not know the Slint declarations. A
//! value Slint refuses (a struct field the struct does not declare, a number
//! where a string is declared) therefore passes every plugin test and fails
//! only in the player, as `wrong type` (0.5.0: the AI window did not open).
//! Here each value goes through the player's own conversion (texts rendered,
//! Markdown documents, images) and is set on the real compiled component.
//!
//! `recorded_plugin_props_match_their_slint` checks every value the plugin
//! tests sent (recorded by `scripts/ui-record.mjs` when the tests run with
//! VESTRA_UI_RECORD; `npm run ci:local` does both).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value as Json;
use slint_interpreter::{ComponentInstance, Value};

/// One recorded value (a line of `scripts/ui-record.mjs`).
#[derive(Debug, Deserialize)]
struct Record {
    plugin: PathBuf,
    #[serde(default)]
    test: String,
    file: String,
    #[serde(default)]
    component: Option<String>,
    /// Panels and settings pages: their properties are this global's.
    #[serde(default)]
    global: Option<String>,
    property: String,
    value: Json,
}

/// The view a record goes to: a window file, or a component and its global.
type ViewKey = (PathBuf, String, Option<String>, Option<String>);

/// A window per component, never shown (no event loop on this thread).
struct Headless;
impl slint::platform::Platform for Headless {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
    {
        Ok(slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default()))
    }
}
fn headless() {
    thread_local! {
        static PLATFORM: () = slint::platform::set_platform(Box::new(Headless)).unwrap();
    }
    PLATFORM.with(|_| {});
}

fn view(
    plugin: &Path,
    file: &str,
    component: Option<&str>,
    global: Option<&str>,
) -> Result<ComponentInstance> {
    headless();
    let path = plugin.join(file);
    let instance = match (component, global) {
        (Some(component), Some(global)) => {
            let import = crate::composition::import_path(&path)?;
            let source = format!(
                "import {{ {component} }} from \"{import}\";\n\
                 export {{ {global} }} from \"{import}\";\n\
                 export component RecordedView inherits Window {{ {component} {{}} }}\n"
            );
            let result = pollster::block_on(
                crate::ui::compiler().build_from_source(source, plugin.join("recorded-view.slint")),
            );
            let errors: Vec<_> = result
                .diagnostics()
                .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
                .map(|d| d.to_string())
                .collect();
            if !errors.is_empty() {
                bail!("{}", errors.join("; "));
            }
            result
                .component("RecordedView")
                .context("no component")?
                .create()?
        }
        _ => crate::ui::compile(&path)?,
    };
    crate::ui_values::own_default_models(&instance, global)?;
    Ok(instance)
}

/// The value as the player converts it before setting it (`prop_value`).
fn convert(value: &Json) -> Result<Value> {
    let mut value = value.clone();
    crate::i18n::text::Texts::english()
        .resolve(&mut value)
        .map_err(|e| anyhow!(e))?;
    if crate::ui_image::reference(&value).is_some() {
        return Ok(Value::Image(Default::default()));
    }
    let mut cache = crate::ui_markdown::Cache::default();
    crate::ui_markdown::convert(&mut cache, (0, "recorded", "value"), &value, &mut |_| None)
        .map(|(value, _)| value)
        .map_err(|e| anyhow!(e.message))
}

/// Every refusal, one line each (`plugin/file: property: why (test)`).
fn check(records: &[Record]) -> Vec<String> {
    let mut views: BTreeMap<ViewKey, Vec<&Record>> = BTreeMap::new();
    for record in records {
        let key = (
            record.plugin.clone(),
            record.file.clone(),
            record.component.clone(),
            record.global.clone(),
        );
        views.entry(key).or_default().push(record);
    }
    let mut failures = Vec::new();
    for ((plugin, file, component, global), records) in views {
        let name = format!(
            "{}/{file}{}",
            plugin.file_name().unwrap_or_default().to_string_lossy(),
            component
                .as_deref()
                .map(|c| format!(" {c}"))
                .unwrap_or_default()
        );
        let instance = match view(&plugin, &file, component.as_deref(), global.as_deref()) {
            Ok(instance) => instance,
            Err(error) => {
                failures.push(format!("{name}: does not compile: {error:#}"));
                continue;
            }
        };
        let mut seen = std::collections::BTreeSet::new();
        for record in records {
            let result = convert(&record.value).and_then(|value| {
                crate::ui_values::set(&instance, global.as_deref(), &record.property, value)
            });
            if let Err(error) = result {
                let line = format!("{name}: {error:#}");
                if seen.insert(line.clone()) {
                    let test = Path::new(&record.test)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    failures.push(if test.is_empty() {
                        line
                    } else {
                        format!("{line} (sent in {test})")
                    });
                }
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plugin(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../plugins")
            .join(name)
    }

    #[test]
    fn a_refused_value_names_the_property_and_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("w.slint");
        std::fs::write(
            &file,
            "export struct Say { code: string, args: [string] }\n\
             export struct Row { id: string, n: int }\n\
             export component W inherits Window {\n\
                 in property <Say> banner;\n\
                 in property <[Row]> rows: [{ id: \"a\", n: 1 }];\n\
                 in property <string> heading;\n\
                 out property <bool> busy;\n\
             }\n",
        )
        .unwrap();
        let instance = view(dir.path(), "w.slint", None, None).unwrap();
        let set = |name: &str, value: Json| {
            crate::ui_values::set(&instance, None, name, convert(&value).unwrap())
                .map_err(|e| e.to_string())
        };
        assert_eq!(
            set("banner", json!({"code": "x", "args": [], "n": 0})).unwrap_err(),
            "banner: wrong type: banner.n is not a field of the declared struct (args, code)"
        );
        assert_eq!(
            set("rows", json!([{"id": "b", "n": "2"}])).unwrap_err(),
            "rows: wrong type: rows[0].n is a string where the declaration has a number"
        );
        assert_eq!(
            set("heading", json!(3)).unwrap_err(),
            "heading: wrong type: heading is a number where the declaration has a string"
        );
        assert_eq!(
            set("busy", json!(true)).unwrap_err(),
            "busy: not writable (an out property)"
        );
        assert_eq!(
            set("nothing", json!(1)).unwrap_err(),
            "nothing: no such property"
        );
        // A text value is rendered first, like in the player.
        set("heading", json!({"$vestraText": "Hello {}", "args": [1]})).unwrap();
        set("banner", json!({"code": "x", "args": ["a"]})).unwrap();
    }

    /// 0.5.0: "Vestra Player AI…" opened nothing (`HostError: wrong type`):
    /// the window's banner is the plugin's `Say` (code, args, n), declared
    /// as `CardSay` (code, args). These are the properties the plugin sends
    /// when it opens the window (captured on the SDK test host).
    #[test]
    fn the_ai_window_takes_what_the_workshop_sends_when_it_opens() {
        let properties = json!({
            "has-project": false, "projects": [], "project-index": 0, "project-title": "",
            "card-status": {"code": "", "args": []}, "card-keys": [], "card-busy": false,
            "sessions": [], "rows": [], "configured": false,
            "banner": {"code": "reading", "args": [], "n": 0}, "busy": false, "blocked": false,
            "session-title": "", "provider-id": "anthropic", "provider-name": "Anthropic",
            "model-text": "", "has-folder": false, "can-retry": false, "draft": ""
        });
        let records: Vec<Record> = properties
            .as_object()
            .unwrap()
            .iter()
            .map(|(property, value)| Record {
                plugin: plugin("workshop"),
                test: String::new(),
                file: "ai.slint".into(),
                component: None,
                global: None,
                property: property.clone(),
                value: value.clone(),
            })
            .collect();
        let failures = check(&records);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    #[ignore = "checks the values plugin tests recorded in VESTRA_UI_RECORD (npm run ci:local)"]
    fn recorded_plugin_props_match_their_slint() {
        let Some(dir) = std::env::var_os("VESTRA_UI_RECORD") else {
            diag!("VESTRA_UI_RECORD is not set: nothing recorded to check");
            return;
        };
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            if entry.path().extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            for line in std::fs::read_to_string(entry.path()).unwrap().lines() {
                records.push(serde_json::from_str::<Record>(line).unwrap());
            }
        }
        assert!(
            !records.is_empty(),
            "nothing recorded in {}: run the plugin tests with VESTRA_UI_RECORD set first \
             (`npm run plugin -- test --all`)",
            Path::new(&dir).display()
        );
        let plugins: std::collections::BTreeSet<_> =
            records.iter().map(|r| r.plugin.clone()).collect();
        diag!(
            "{} recorded values from {} plugins",
            records.len(),
            plugins.len()
        );
        let failures = check(&records);
        assert!(
            failures.is_empty(),
            "values Slint refuses:\n{}",
            failures.join("\n")
        );
    }
}
