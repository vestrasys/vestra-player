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

/// Default UI font. Without a forced family Slint 1.16 renders CJK text in
/// plugin windows as blank (no per-script fallback here, GUI-verified
/// 2026-10-20), so a system CJK font file (`platform::ui_font`: Hiragino
/// Sans GB on macOS, Microsoft YaHei on Windows) is imported and made the
/// default family whenever it exists (§18 Q12). `VESTRA_FONT_FALLBACK=system`
/// opts out, for checking a future Slint's own fallback.
pub fn font() -> Option<(&'static str, &'static str)> {
    if std::env::var("VESTRA_FONT_FALLBACK").as_deref() == Ok("system") {
        return None;
    }
    crate::platform::ui_font()
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
    // R-H6: the player sets this window's palette (which `Theme` reads).
    if !source.contains(crate::theme::PALETTE) {
        source.push('\n');
        source.push_str(&crate::theme::export_line());
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
    // Before the window appears (the player's theme colours).
    prepare: impl FnOnce(&ComponentInstance),
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
    prepare(&instance);
    instance.show()?;
    crate::platform::shown(instance.window());
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

    /// A windowless platform, so a widget can take pointer events in a test.
    struct Headless(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<
            std::rc::Rc<dyn slint::platform::WindowAdapter>,
            slint::PlatformError,
        > {
            Ok(self.0.clone())
        }
    }

    /// Reported by a plugin author (interaction round): the tip used to
    /// assign its timer's `running`, dropping the binding, so it showed once.
    #[test]
    fn tool_tip_shows_again_on_every_hover_and_hides_when_the_pointer_leaves() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let source = r#"
import { ToolTipArea } from "@vestra/widgets.slint";
export component W inherits Window {
    width: 200px; height: 100px;
    out property <bool> showing: tip.showing;
    tip := ToolTipArea { x: 0px; y: 0px; width: 100px; height: 100px; text: "提示"; }
}"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), dir.path().join("w.slint")),
        );
        let instance = result.component("W").unwrap().create().unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(200., 100.));
        let showing = || instance.get_property("showing").unwrap() == true.into();
        let at = |x: f32| slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(x, 50.),
        };
        let wait = |ms: u64| {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
        };
        for round in 0..2 {
            instance.window().dispatch_event(at(50.));
            wait(200);
            assert!(!showing(), "round {round}: before 600 ms");
            wait(500);
            assert!(showing(), "round {round}: shown after 600 ms");
            instance.window().dispatch_event(at(150.));
            wait(0);
            assert!(!showing(), "round {round}: hidden when the pointer left");
        }
        // A press hides it for the rest of this hover.
        instance.window().dispatch_event(at(50.));
        wait(700);
        assert!(showing());
        instance
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed {
                position: slint::LogicalPosition::new(50., 50.),
                button: slint::platform::PointerEventButton::Left,
            });
        wait(700);
        assert!(!showing());
    }

    fn headless() {
        thread_local! {
            static PLATFORM: () = {
                let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
                    Default::default(),
                );
                slint::platform::set_platform(Box::new(Headless(window))).unwrap();
            };
        }
        PLATFORM.with(|_| {});
    }

    /// IA-CORE-01: Escape closes the deepest level first; on the top level it
    /// dismisses the menu.
    #[test]
    fn escape_in_the_menu_closes_one_level_at_a_time() {
        headless();
        let source = r#"
import { MenuView, MenuLevel } from "menu-view.slint";
export component W inherits Window {
    width: 600px; height: 400px;
    in property <[MenuLevel]> levels;
    in-out property <int> backs;
    in-out property <int> dismissals;
    MenuView {
        width: root.width; height: root.height;
        levels: root.levels;
        back => { root.backs += 1; }
        dismissed => { root.dismissals += 1; }
    }
}"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
        );
        let errors: Vec<_> = result
            .diagnostics()
            .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
            .map(|d| d.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let instance = result.component("W").unwrap().create().unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(600., 400.));
        let group = |title: &str, children| crate::commands::MenuNode::Group {
            title: title.into(),
            group: "g".into(),
            children,
        };
        let roots = vec![group("视频", vec![group("比例", vec![])])];
        let none = |_: u64, _: &str| None;
        let set = |path: &[&str]| {
            let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
            let origin = crate::menu::Point { x: 10., y: 10. };
            let window = crate::menu::Point { x: 600., y: 400. };
            let value = crate::menu::levels(&roots, &path, origin, window, &[], &none);
            instance
                .set_property("levels", crate::ui_values::from_json(&value).unwrap())
                .unwrap();
        };
        let escape = || {
            instance
                .window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::Escape.into(),
                });
        };
        let count = |name: &str| match instance.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{other:?}"),
        };
        // 视频 ▶ 比例 ▶ open: three levels. The host pops a level on `back`.
        set(&["视频", "比例"]);
        escape();
        assert_eq!((count("backs"), count("dismissals")), (1., 0.));
        set(&["视频"]);
        escape();
        assert_eq!((count("backs"), count("dismissals")), (2., 0.));
        set(&[]);
        escape();
        assert_eq!((count("backs"), count("dismissals")), (2., 1.));
    }

    /// G-01: with the fullscreen sidebar popped out, the bottom bar spans from
    /// the left edge to the overlay's left edge. (It used to be centred: a
    /// narrower element without `x` in a plain parent is centred by Slint.)
    #[test]
    fn popped_sidebar_narrows_the_fullscreen_bar_from_the_left_edge() {
        use crate::composition::{CompileSpec, Slot};
        headless();
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
        let source = crate::composition::shell_source(&specs).unwrap();
        let marker = "    callback focus-video();\n";
        assert!(source.contains(marker));
        let probes = "    callback focus-video();\n\
            out property <length> t-bar-x: bottom-keys.absolute-position.x;\n\
            out property <length> t-bar-w: bottom-keys.width;\n";
        let source = source.replacen(marker, probes, 1);
        let shell = crate::composition::build(source, "HostWindow")
            .unwrap()
            .create()
            .unwrap();
        shell
            .window()
            .set_size(slint::LogicalSize::new(1470., 956.));
        let set = |name: &str, value: slint_interpreter::Value| {
            shell.set_property(name, value).unwrap();
        };
        let read = |name: &str| match shell.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        set("host-fullscreen", true.into());
        set("sidebar-visible", false.into());
        set("sidebar-overlay", true.into());
        set("sidebar-width", 317.0.into());
        set("sidebar-popped", false.into());
        assert_eq!((read("t-bar-x"), read("t-bar-w")), (0., 1470.));
        set("sidebar-popped", true.into());
        assert_eq!(read("sidebar-pop-width"), 317.);
        assert_eq!((read("t-bar-x"), read("t-bar-w")), (0., 1470. - 317.));
    }

    /// G-10: the OSD and information block sizes follow the video area.
    #[test]
    fn osd_font_follows_the_video_area_height() {
        headless();
        let source = crate::composition::shell_source(&[]).unwrap();
        let shell = crate::composition::build(source, "HostWindow")
            .unwrap()
            .create()
            .unwrap();
        let size = |name: &str| match shell.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{other:?}"),
        };
        for (width, height, osd, info) in [
            (780., 609., 18., 12.375),
            (1920., 1080., 32., 22.),
            (320., 182., 12., 9.),
            (3840., 2160., 48., 28.),
        ] {
            shell
                .window()
                .set_size(slint::LogicalSize::new(width, height));
            assert_eq!(size("osd-size"), osd, "{width}x{height}");
            assert_eq!(size("info-size"), info, "{width}x{height}");
        }
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
