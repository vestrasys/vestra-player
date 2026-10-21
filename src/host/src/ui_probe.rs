//! A plugin UI must not take the player down when the plugin sets one of its
//! global properties. Slint 1.16.1 has a compiler bug that does exactly that:
//! a std-widgets property with a `changed` handler inside the widget, aliased
//! to a global through the widget's own property (e.g.
//! `ComboBox { current-index <=> Bridge.index; }`), gets its handler moved onto
//! the global (`i-slint-compiler` `passes/remove_aliases.rs`, `add_link` only
//! checks the two ends of each link for handlers; fixed upstream in 1.18). The
//! handler then runs without its element and the interpreter panics in the
//! event loop: "accessing deleted parent (issue #6426)".
//!
//! The player cannot catch that panic in its own event loop, so it tries the UI
//! first, on a scratch thread with its own Slint context and no window: every
//! scalar global property is changed once and the change handlers run. A UI
//! that panics there fails its own registration (`ui-prepare`) instead.
#![forbid(unsafe_code)]
use anyhow::{Result, anyhow, bail};
use slint_interpreter::Value;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

/// A probe that takes longer than this is not waited for (and counts as passed).
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the plugin author is told.
pub const HAZARD: &str = "this UI crashes Slint 1.16 when a global property changes: \
    a std-widgets property with a change handler is bound with <=> to a global \
    (e.g. ComboBox current-index <=> Bridge.index); bind one way \
    (current-index: Bridge.index) and set it again in a `changed` handler";

/// Sources already probed successfully (by content), so reopening a window
/// does not compile it twice.
static PASSED: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

fn key(source: &str, path: &std::path::Path, component: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (source, path, component).hash(&mut hasher);
    hasher.finish()
}

/// Build `source` (as if at `path`) on a scratch thread, create `component`
/// (or the only exported one), change each global property and run the
/// change handlers.
pub fn check(source: String, path: PathBuf, component: Option<String>) -> Result<()> {
    let id = key(&source, &path, component.as_deref());
    if PASSED
        .lock()
        .map(|set| set.as_ref().is_some_and(|s| s.contains(&id)))
        .unwrap_or(false)
    {
        return Ok(());
    }
    let (done, wait) = mpsc::channel();
    std::thread::Builder::new()
        .name("ui-probe".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(|| probe(source, path, component.as_deref()));
            let _ = done.send(match result {
                Ok(result) => result.map_err(|e| format!("{e:#}")),
                Err(_) => Err(HAZARD.to_owned()),
            });
        })?;
    match wait.recv_timeout(TIMEOUT) {
        Ok(Ok(())) => {
            if let Ok(mut set) = PASSED.lock() {
                set.get_or_insert_with(HashSet::new).insert(id);
            }
            Ok(())
        }
        Ok(Err(message)) => Err(anyhow!(message)),
        // A panic outside catch_unwind ends the thread without an answer.
        Err(mpsc::RecvTimeoutError::Disconnected) => bail!(HAZARD),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!("ui probe: no answer within {TIMEOUT:?}, not checked");
            Ok(())
        }
    }
}

struct Windowless(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
impl slint::platform::Platform for Windowless {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
    {
        Ok(self.0.clone())
    }
}

fn probe(source: String, path: PathBuf, component: Option<&str>) -> Result<()> {
    // Slint keeps its platform per thread: this one never touches the player's.
    let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default());
    slint::platform::set_platform(Box::new(Windowless(window)))
        .map_err(|e| anyhow!("ui probe platform: {e}"))?;
    let result = pollster::block_on(crate::ui::compiler().build_from_source(source, path));
    let definition = match component {
        Some(name) => result.component(name),
        None => result.components().next(),
    };
    let Some(definition) = definition else {
        bail!(
            "Slint compilation: {:?}",
            result.diagnostics().collect::<Vec<_>>()
        );
    };
    let instance = definition.create()?;
    slint::platform::update_timers_and_animations();
    for global in definition.globals() {
        let Some(properties) = definition.global_properties(&global) else {
            continue;
        };
        for (name, _) in properties.collect::<Vec<_>>() {
            let Ok(current) = instance.get_global_property(&global, &name) else {
                continue;
            };
            let changed = match current {
                Value::Bool(b) => Value::Bool(!b),
                Value::Number(n) => Value::Number(n + 1.),
                Value::String(s) => Value::String(format!("{s}x").into()),
                _ => continue,
            };
            // Output properties refuse the write; that is fine.
            let _ = instance.set_global_property(&global, &name, changed);
        }
    }
    slint::platform::update_timers_and_animations();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_ui(body: &str) -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let source = format!(
            "import {{ ComboBox, CheckBox, LineEdit }} from \"std-widgets.slint\";\n\
             export global Bridge {{ in-out property <int> index; in-out property <bool> on; in-out property <string> text; }}\n\
             export component W inherits Window {{ {body} }}"
        );
        check(source, dir.path().join("w.slint"), Some("W".into()))
    }

    /// The Slint 1.16.1 bug behind the 选项 / 插件管理 crash (2026-10-21):
    /// the std ComboBox's own `changed current-index` handler lands on the
    /// global. If this starts passing, the Slint in use has the upstream fix.
    #[test]
    fn a_combo_box_two_way_bound_to_a_global_is_refused() {
        let error = try_ui("ComboBox { model: [\"a\", \"b\"]; current-index <=> Bridge.index; }")
            .unwrap_err();
        assert!(error.to_string().contains("<=>"), "{error}");
    }

    #[test]
    fn one_way_bindings_and_other_two_way_bindings_pass() {
        try_ui(
            "ComboBox { model: [\"a\", \"b\"]; current-index: Bridge.index; }\n\
             CheckBox { checked <=> Bridge.on; }\n\
             LineEdit { text <=> Bridge.text; }",
        )
        .unwrap();
    }

    #[test]
    fn a_broken_ui_reports_its_diagnostics() {
        let error = try_ui("Nope { }").unwrap_err();
        assert!(error.to_string().contains("Slint compilation"), "{error}");
    }
}
