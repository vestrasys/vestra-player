//! Experimental two-slot Slint composition, using the public 1.16.1 interpreter.
//! This is a source integration candidate, not a validated public SDK contract.
#![forbid(unsafe_code)]

use std::{collections::BTreeSet, path::PathBuf, sync::mpsc};

use anyhow::{Context, Result, bail};
use serde_json::Value as Json;
use slint_interpreter::{ComponentDefinition, ComponentInstance, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    Bottom,
    Sidebar,
}

/// One independent package per owner; this minimal shell allows one component
/// in each slot. Components inherit a native visual element, not Window.
#[derive(Clone, Debug)]
pub struct CompileSpec {
    pub owner: u64,
    pub file: PathBuf,
    pub component: String,
    pub global: Option<String>,
    pub slot: Slot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    Bool(bool),
    Number(f64),
    String(String),
}

impl Primitive {
    fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::Number(value) if value.is_finite() => Ok(Self::Number(*value)),
            Value::String(value) => Ok(Self::String(value.to_string())),
            _ => {
                Err("composition callback requires finite number, bool or string arguments".into())
            }
        }
    }

    fn into_value(self) -> Result<Value> {
        Ok(match self {
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) if value.is_finite() => Value::Number(value),
            Self::Number(_) => bail!("non-finite composition property"),
            Self::String(value) => Value::String(value.into()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct CallbackEvent {
    pub owner: u64,
    pub callback: String,
    /// Unsupported arguments are reported explicitly, never silently nullified.
    pub args: Result<Vec<Primitive>, String>,
}

fn identifier(name: &str) -> Result<()> {
    let mut chars = name.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid Slint exported identifier: {name}");
    }
    Ok(())
}

pub fn global_alias(owner: u64) -> String {
    format!("Owner{owner}Bridge")
}

/// Slint handler body that forwards a key event to the host binding table.
fn key_forward(scope: &str) -> String {
    format!(
        "key-pressed(event) => {{ if root.key-input(\"{scope}\", event.text, event.modifiers.control, event.modifiers.alt, event.modifiers.shift, event.modifiers.meta, event.repeat) {{ return accept; }} reject }}"
    )
}

fn import_path(spec: &CompileSpec) -> Result<String> {
    let path = spec
        .file
        .canonicalize()
        .context("composition file unavailable")?;
    let path = path.to_str().context("Slint import path must be UTF-8")?;
    if path.chars().any(char::is_control) {
        bail!("control character in Slint import path");
    }
    Ok(path.replace('\\', "\\\\").replace('"', "\\\""))
}

fn import_spec(source: &mut String, spec: &CompileSpec, path: &str) -> Result<()> {
    identifier(&spec.component)?;
    source.push_str(&format!(
        "import {{ {} as Owner{}View }} from \"{}\";\n",
        spec.component, spec.owner, path
    ));
    if let Some(global) = &spec.global {
        identifier(global)?;
        let alias = global_alias(spec.owner);
        source.push_str(&format!(
            "import {{ {global} as {alias} }} from \"{path}\";\nexport {{ {alias} }}\n"
        ));
    }
    Ok(())
}

fn font_property() -> String {
    crate::ui::font()
        .map(|(family, _)| format!("default-font-family: \"{family}\";"))
        .unwrap_or_default()
}

/// Generated source is ordinary Slint, not a layout protocol. Imports use
/// canonical files; package containment is the package loader's responsibility.
/// Aliases prevent exported-name collisions, but are not a sandbox. Slint globals
/// imported from the same underlying file remain shared, so independent owners
/// must not import a shared stateful global (including through dependencies).
pub fn shell_source(specs: &[CompileSpec]) -> Result<String> {
    let mut owners = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut source = crate::ui::font_import();
    for spec in specs {
        if !owners.insert(spec.owner) || !slots.insert(spec.slot) {
            bail!("this shell requires unique owners and at most one component per slot");
        }
        if !files.insert(
            spec.file
                .canonicalize()
                .context("composition file unavailable")?,
        ) {
            bail!("independent composition owners require distinct Slint files");
        }
        let path = import_path(spec)?;
        import_spec(&mut source, spec, &path)?;
    }
    let mut shell = include_str!("../ui/host.slint").to_owned();
    replace_marker(&mut shell, "// VESTRA-FONT", &font_property())?;
    let properties = "in property <bool> bottom-visible: true;\n\
                      in property <bool> sidebar-visible: true;";
    replace_marker(&mut shell, "// VESTRA-MOUNT-PROPERTIES", properties)?;
    let sidebar = specs
        .iter()
        .find(|spec| spec.slot == Slot::Sidebar)
        .map(|spec| {
            format!(
                "if root.sidebar-visible: Rectangle {{ width: 3px; horizontal-stretch: 0; background: Theme.line; \
                 TouchArea {{ mouse-cursor: MouseCursor.ew-resize; \
                 moved => {{ if self.pressed {{ root.sidebar-width = clamp(root.width / 1px - (self.absolute-position.x + self.mouse-x) / 1px - 1.5, 317, max(317, root.width / 1px - 321)); }} }} \
                 pointer-event(event) => {{ if event.kind == PointerEventKind.up {{ root.sidebar-resized(root.sidebar-actual); }} }} }} }}\n\
                 if root.sidebar-visible: FocusScope {{ width: root.sidebar-actual * 1px; horizontal-stretch: 0; {} \
                 Owner{}View {{ width: parent.width; height: parent.height; }} }}",
                key_forward("panel:sidebar"),
                spec.owner
            )
        })
        .unwrap_or_default();
    replace_marker(&mut shell, "// VESTRA-MOUNT-SIDEBAR", &sidebar)?;
    let bottom = specs
        .iter()
        .find(|spec| spec.slot == Slot::Bottom)
        .map(|spec| {
            format!(
                "Rectangle {{ height: root.host-fullscreen || !root.bottom-visible ? 0px : bottom-keys.height; vertical-stretch: 0; \
                 bottom-keys := FocusScope {{ visible: root.bottom-visible; width: parent.width; \
                 height: min(bottom-content.max-height, max(bottom-content.min-height, bottom-content.preferred-height)); \
                 y: root.host-fullscreen ? -self.height : 0px; {} \
                 bottom-content := Owner{}View {{ width: parent.width; height: parent.height; }} }} }}",
                key_forward("panel:bottom"),
                spec.owner
            )
        })
        .unwrap_or_default();
    replace_marker(&mut shell, "// VESTRA-MOUNT-BOTTOM", &bottom)?;
    source.push_str(&shell);
    Ok(source)
}

/// The detached sidebar (C14): the same component and bridge global in a
/// window of its own, with the shell's key forwarding and menu layer.
pub fn floating_source(spec: &CompileSpec, title: &str) -> Result<String> {
    let mut source = crate::ui::font_import();
    let path = import_path(spec)?;
    import_spec(&mut source, spec, &path)?;
    let title = serde_json::to_string(title)?;
    source.push_str(&format!(
        r#"import {{ MenuView, MenuLevel }} from "menu-view.slint";
import {{ Theme }} from "@vestra/theme.slint";
export component FloatingPanel inherits Window {{
    title: {title};
    min-width: 300px; min-height: 250px;
    preferred-width: 300px; preferred-height: 408px;
    background: Theme.bg-list;
    {font}
    in property <[MenuLevel]> menu-levels;
    in property <bool> menu-open: false;
    in-out property <float> menu-origin-x;
    in-out property <float> menu-origin-y;
    out property <float> logical-width: root.width / 1px;
    out property <float> logical-height: root.height / 1px;
    callback menu-invoke(string);
    callback menu-hover(int, string);
    callback menu-back();
    callback menu-dismiss();
    callback key-input(string, string, bool, bool, bool, bool, bool) -> bool;
    FocusScope {{
        {keys}
        Owner{owner}View {{ width: parent.width; height: parent.height; }}
    }}
    if root.menu-open: Rectangle {{
        width: root.width; height: root.height;
        TouchArea {{ pointer-event(event) => {{ if event.kind == PointerEventKind.down {{ root.menu-dismiss(); }} }} }}
        MenuView {{
            width: root.width; height: root.height;
            levels: root.menu-levels;
            hovered(level, token) => {{ root.menu-hover(level, token); }}
            activated(token) => {{ root.menu-invoke(token); }}
            back => {{ root.menu-back(); }}
            dismissed => {{ root.menu-dismiss(); }}
        }}
    }}
}}
"#,
        font = font_property(),
        keys = key_forward("panel:sidebar"),
        owner = spec.owner,
    ));
    Ok(source)
}

fn replace_marker(source: &mut String, marker: &str, replacement: &str) -> Result<()> {
    if source.matches(marker).count() != 1 {
        bail!("host shell must contain exactly one {marker} insertion marker");
    }
    *source = source.replacen(marker, replacement, 1);
    Ok(())
}

fn build(source: String, component: &str) -> Result<ComponentDefinition> {
    let base = crate::ui::ui_dir().join("host.slint");
    let result = pollster::block_on(crate::ui::compiler().build_from_source(source, base));
    if result
        .diagnostics()
        .any(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
    {
        bail!(
            "composition compilation: {:?}",
            result.diagnostics().collect::<Vec<_>>()
        );
    }
    result.component(component).ok_or_else(|| {
        anyhow::anyhow!(
            "composition compilation: {:?}",
            result.diagnostics().collect::<Vec<_>>()
        )
    })
}

fn compile_definition(specs: &[CompileSpec]) -> Result<ComponentDefinition> {
    build(shell_source(specs)?, "HostWindow")
}

/// Call on the UI thread. Returns an unshown instance, letting the caller keep
/// its old shell on any compilation/binding failure. This does not run the event
/// loop or transfer backend ownership. Global callbacks must be void-returning;
/// return-valued callbacks are outside this finite asynchronous bridge.
pub fn compile(
    specs: &[CompileSpec],
    events: mpsc::Sender<CallbackEvent>,
) -> Result<ComponentInstance> {
    let definition = compile_definition(specs)?;
    let instance = definition.create()?;
    for spec in specs.iter().filter(|spec| spec.global.is_some()) {
        crate::ui_values::own_default_models(&instance, Some(&global_alias(spec.owner)))?;
    }
    for spec in specs.iter().filter(|spec| spec.global.is_some()) {
        let alias = global_alias(spec.owner);
        let callbacks = definition
            .global_callbacks(&alias)
            .with_context(|| format!("composition global unavailable: {alias}"))?;
        for callback in callbacks {
            let sender = events.clone();
            let owner = spec.owner;
            let name = callback.clone();
            instance.set_global_callback(&alias, &callback, move |args| {
                let _ = sender.send(CallbackEvent {
                    owner,
                    callback: name.clone(),
                    args: args.iter().map(Primitive::from_value).collect(),
                });
                Value::Void
            })?;
        }
    }
    Ok(instance)
}

/// Caller routes only to a current owner's own CompileSpec. Use interpreter
/// set_property("video-frame", Value::Image(...)) for the host-owned video frame.
pub fn set_global(
    instance: &ComponentInstance,
    spec: &CompileSpec,
    property: &str,
    value: Primitive,
) -> Result<()> {
    if spec.global.is_none() {
        bail!("composition contribution has no exported bridge global");
    }
    instance.set_global_property(&global_alias(spec.owner), property, value.into_value()?)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct JsonCallback {
    pub shell_generation: u64,
    pub owner: u64,
    pub callback: String,
    pub args: Result<Vec<Json>, String>,
}

fn json_args(args: &[Value]) -> Result<Vec<Json>, String> {
    if args.len() > crate::ui_values::MAX_ARRAY {
        return Err("composition callback has too many arguments".into());
    }
    // Treat all arguments as one value, so depth/node/text budgets also bound
    // the aggregate callback rather than resetting for every argument.
    let model = Value::Model(std::rc::Rc::new(slint::VecModel::from(args.to_vec())).into());
    match crate::ui_values::to_json(&model) {
        Ok(Json::Array(args)) => Ok(args),
        Ok(_) => Err("composition callback arguments did not form an array".into()),
        Err(error) => Err(format!("{error:#}")),
    }
}

fn deliver_json(sender: &mpsc::SyncSender<JsonCallback>, event: JsonCallback) -> bool {
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(event)) => {
            eprintln!(
                "composition callback queue full: owner={} callback={}; callback rejected",
                event.owner, event.callback
            );
            false
        }
        Err(mpsc::TrySendError::Disconnected(event)) => {
            eprintln!(
                "composition callback receiver closed: owner={} callback={}",
                event.owner, event.callback
            );
            false
        }
    }
}

/// Like compile(), but bridges bounded JSON structs/models and uses a bounded
/// nonblocking sender. Full queues explicitly reject callbacks; they never wait
/// for host policy or backend work on the UI thread. The caller keeps the old
/// shell until this complete candidate (including callback bindings) succeeds.
pub fn compile_json(
    specs: &[CompileSpec],
    events: mpsc::SyncSender<JsonCallback>,
    shell_generation: u64,
) -> Result<ComponentInstance> {
    let definition = compile_definition(specs)?;
    let instance = definition.create()?;
    for spec in specs.iter().filter(|spec| spec.global.is_some()) {
        let alias = global_alias(spec.owner);
        let callbacks = definition
            .global_callbacks(&alias)
            .with_context(|| format!("composition global unavailable: {alias}"))?;
        for callback in callbacks {
            let sender = events.clone();
            let owner = spec.owner;
            let name = callback.clone();
            instance.set_global_callback(&alias, &callback, move |args| {
                deliver_json(
                    &sender,
                    JsonCallback {
                        shell_generation,
                        owner,
                        callback: name.clone(),
                        args: json_args(args),
                    },
                );
                Value::Void
            })?;
        }
    }
    Ok(instance)
}

/// Compile and bind the floating panel window; not shown. Bridge callbacks
/// arrive tagged with `generation`, like a shell generation.
pub fn compile_floating(
    spec: &CompileSpec,
    title: &str,
    events: mpsc::SyncSender<JsonCallback>,
    generation: u64,
) -> Result<ComponentInstance> {
    let definition = build(floating_source(spec, title)?, "FloatingPanel")?;
    let instance = definition.create()?;
    let alias = global_alias(spec.owner);
    crate::ui_values::own_default_models(&instance, Some(&alias))?;
    let callbacks = definition
        .global_callbacks(&alias)
        .with_context(|| format!("composition global unavailable: {alias}"))?;
    for callback in callbacks {
        let sender = events.clone();
        let owner = spec.owner;
        let name = callback.clone();
        instance.set_global_callback(&alias, &callback, move |args| {
            deliver_json(
                &sender,
                JsonCallback {
                    shell_generation: generation,
                    owner,
                    callback: name.clone(),
                    args: json_args(args),
                },
            );
            Value::Void
        })?;
    }
    Ok(instance)
}

pub fn set_json_global(
    instance: &ComponentInstance,
    spec: &CompileSpec,
    property: &str,
    value: &Json,
) -> Result<()> {
    if spec.global.is_none() {
        bail!("composition contribution has no exported bridge global");
    }
    let value = crate::ui_values::from_json(value)?;
    let alias = global_alias(spec.owner);
    let current = instance.get_global_property(&alias, property)?;
    if !crate::ui_values::reconcile(&current, &value)? {
        instance.set_global_property(&alias, property, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_duplicate_shell_markers_fail_instead_of_guessing_layout() {
        let mut missing = "Window {}".to_owned();
        assert!(replace_marker(&mut missing, "// SLOT", "injected").is_err());
        let mut duplicate = "// SLOT\n// SLOT".to_owned();
        assert!(replace_marker(&mut duplicate, "// SLOT", "injected").is_err());
    }

    #[test]
    fn generated_shell_preserves_shared_menu_video_and_two_owner_globals() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let specs = [
            CompileSpec {
                owner: 10,
                file: base.join("bar.slint"),
                component: "SourceBar".into(),
                global: Some("Bridge".into()),
                slot: Slot::Bottom,
            },
            CompileSpec {
                owner: 20,
                file: base.join("sidebar.slint"),
                component: "RowsSidebar".into(),
                global: Some("Bridge".into()),
                slot: Slot::Sidebar,
            },
        ];
        let source = shell_source(&specs).unwrap();
        for preserved in [
            "video-frame",
            "menu-levels",
            "menu-invoke",
            "key-input",
            "if root.menu-open: Rectangle",
            "export component HostWindow",
        ] {
            assert!(source.contains(preserved), "missing {preserved}");
        }
        for removed in [
            "diagnostic",
            "Commands",
            "Arial Unicode",
            "padding: root.host-fullscreen ? 0px : 8px",
        ] {
            assert!(!source.contains(removed), "still has {removed}");
        }
        assert!(source.contains("Bridge as Owner10Bridge"));
        assert!(source.contains("Bridge as Owner20Bridge"));
        assert!(source.contains("Owner20View { width: parent.width; height: parent.height; }"));
        assert!(source.contains("key-input(\"panel:sidebar\""));
        assert!(source.contains("bottom-content := Owner10View"));
        assert!(source.contains("visible: root.bottom-visible"));
        assert!(!source.contains("// VESTRA-"));
    }

    #[test]
    fn actual_compiler_accepts_shell_imports_models_and_both_mounts_without_a_window() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let specs = [
            CompileSpec {
                owner: 10,
                file: base.join("bar.slint"),
                component: "SourceBar".into(),
                global: Some("Bridge".into()),
                slot: Slot::Bottom,
            },
            CompileSpec {
                owner: 20,
                file: base.join("sidebar.slint"),
                component: "RowsSidebar".into(),
                global: Some("Bridge".into()),
                slot: Slot::Sidebar,
            },
        ];
        // ComponentDefinition only: no create(), Window, event loop or GUI.
        compile_definition(&[]).unwrap();
        compile_definition(&specs).unwrap();
        build(
            floating_source(&specs[1], "播放列表").unwrap(),
            "FloatingPanel",
        )
        .unwrap();
    }

    #[test]
    fn bounded_json_callbacks_preserve_structs_and_reject_overload() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let row = serde_json::json!({"rowId": "row-a", "label": "A"});
        let value = crate::ui_values::from_json(&row).unwrap();
        let args = json_args(&[value]).unwrap();
        assert_eq!(args, [row]);
        let event = || JsonCallback {
            shell_generation: 0,
            owner: 7,
            callback: "select-row".into(),
            args: Ok(args.clone()),
        };
        assert!(deliver_json(&sender, event()));
        assert!(!deliver_json(&sender, event()));
        assert_eq!(receiver.try_recv().unwrap().owner, 7);
        assert!(receiver.try_recv().is_err());
        assert!(json_args(&[Value::Number(f64::NAN)]).is_err());
        drop(receiver);
        assert!(!deliver_json(&sender, event()));
    }
}
