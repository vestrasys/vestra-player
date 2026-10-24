//! Direct Slint interpreter integration, with primitive property and callback transport.
use crate::Event;
use anyhow::{Result, bail};
use serde_json::json;
use slint_interpreter::{Compiler, ComponentHandle, ComponentInstance, Value};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

/// Host-shipped Slint directory: the installation's `ui`, else the source
/// tree (`runtime_paths.rs`).
pub fn ui_dir() -> PathBuf {
    crate::runtime_paths::get().ui.clone()
}

/// Brings `if` and `for` elements up to date before the host reads geometry
/// (`width`, `absolute-position`, layout results) off a window.
///
/// Since Slint 1.17 the conditional and repeated elements are created and
/// removed in a pass that runs before each frame and each input event, not
/// when a layout is read: geometry read right after a condition changed
/// (`sidebar-visible`, or a plugin panel's own `if`) is the old layout until
/// the next frame. This runs that pass now. It is the only Slint private
/// interface outside `ui_markdown`: recheck on every Slint upgrade (the
/// `ui::tests::geometry_*` tests fail if the behaviour changes).
pub fn settle_layout(window: &slint::Window) {
    slint::private_unstable_api::re_exports::WindowInner::from_pub(window)
        .ensure_tree_instantiated();
}

/// New windows whose frame goes where a saved or previous window's was
/// (`Window::position`, the frame's top-left corner, as `ui_state` saves it).
///
/// Slint creates the native window only after `show()`, from the event
/// loop, and hands a position set before that to winit as a creation
/// attribute, which macOS takes as the top-left corner of the *content* (the
/// title bar goes above it) and Windows as the frame's. Every window
/// restored that way on macOS opened one title bar higher than it was (the
/// floating sidebar at each launch, the main window at each rebuild). Once
/// the native window exists a position is the frame's everywhere, so
/// [`Placements::tick`] sets it again then.
#[derive(Default)]
pub struct Placements(Vec<Placement>);

struct Placement {
    window: slint::Weak<ComponentInstance>,
    position: slint::WindowPosition,
    /// Ticks waited for the native window.
    waited: u32,
}

/// Give up on a window that never gets a native one (about 2 s of ticks).
const PLACEMENT_TICKS: u32 = 120;

impl Placements {
    /// Show `component` with its frame at `position`.
    pub fn show_at(
        &mut self,
        component: &ComponentInstance,
        position: impl Into<slint::WindowPosition>,
    ) -> Result<(), slint::PlatformError> {
        let position = position.into();
        component.window().set_position(position.clone());
        component.show()?;
        self.track(component, position);
        Ok(())
    }

    /// Put an already shown `component`'s frame at `position` once its
    /// native window exists.
    pub fn track(&mut self, component: &ComponentInstance, position: slint::WindowPosition) {
        self.0.push(Placement {
            window: component.as_weak(),
            position,
            waited: 0,
        });
    }

    /// Set the frame position of each window whose native window now exists
    /// (not in fullscreen, where it has no frame of its own); windows closed
    /// meanwhile are forgotten.
    pub fn tick(&mut self) {
        use slint::winit_030::WinitWindowAccessor;
        self.0.retain_mut(|placement| {
            let Some(component) = placement.window.upgrade() else {
                return false;
            };
            let window = component.window();
            if window.has_winit_window() {
                if !window.is_fullscreen() {
                    window.set_position(placement.position.clone());
                }
                return false;
            }
            placement.waited += 1;
            placement.waited < PLACEMENT_TICKS
        });
    }

    #[cfg(test)]
    fn pending(&self) -> usize {
        self.0.len()
    }
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

/// Fonts: no window sets `default-font-family`, so every window uses the
/// system UI font and Slint's per-character fallback for scripts it lacks
/// (CJK included since Slint 1.18; architecture README "fonts" has how to
/// add a fallback font for one platform only).
pub fn compile(path: &Path) -> Result<ComponentInstance> {
    compile_source(std::fs::read_to_string(path)?, path)
}
pub fn compile_source(mut source: String, path: &Path) -> Result<ComponentInstance> {
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
/// The standard window shortcuts (`platform::standard_window_shortcut`) of
/// a secondary window: tool windows, floating panels, the options window;
/// never the main window. Its winit hook calls `filter` first, before Slint
/// sees the key, so the shortcut works with a text field focused and never
/// reaches the window's Slint handlers, the binding table or a plugin. (On
/// macOS Slint reports ⌘ as `control`, which is why the options window's
/// own Slint `meta`+W handler never fired for ⌘W.)
#[derive(Default)]
pub struct WindowShortcuts {
    modifiers: std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
}
impl WindowShortcuts {
    /// True: the event was a standard shortcut, done; the hook returns
    /// `EventResult::PreventDefault`. An auto-repeat is kept from Slint too.
    pub fn filter(
        &self,
        window: &slint::Window,
        event: &slint::winit_030::winit::event::WindowEvent,
    ) -> bool {
        use slint::winit_030::winit::{
            event::{ElementState, WindowEvent},
            keyboard::{Key, PhysicalKey},
            platform::modifier_supplement::KeyEventExtModifierSupplement,
        };
        match event {
            WindowEvent::ModifiersChanged(state) => {
                self.modifiers.set(state.state());
                false
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if event.state == ElementState::Pressed => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return false;
                };
                let state = self.modifiers.get();
                let press = crate::platform::KeyPress {
                    code: format!("{code:?}"),
                    base: match event.key_without_modifiers() {
                        Key::Character(text) if text.chars().count() == 1 => text.chars().next(),
                        _ => None,
                    },
                    mods: crate::platform::ShortcutMods {
                        ctrl: state.control_key(),
                        alt: state.alt_key(),
                        shift: state.shift_key(),
                        cmd: state.super_key(),
                    },
                };
                let Some(action) = crate::platform::standard_window_shortcut(&press) else {
                    return false;
                };
                if !event.repeat {
                    perform(window, action);
                }
                true
            }
            _ => false,
        }
    }
}
/// Do a standard window action. Close is the title bar's close button: the
/// window's own close-requested handler runs (a plugin's view sends
/// `ui.closed`, the options window and a floating panel close as usual).
pub fn perform(window: &slint::Window, action: crate::platform::WindowAction) {
    match action {
        crate::platform::WindowAction::Close => {
            window.dispatch_event(slint::platform::WindowEvent::CloseRequested)
        }
    }
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
    let shortcuts = WindowShortcuts::default();
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor};
        instance
            .window()
            .on_winit_window_event(move |window, event| {
                if shortcuts.filter(window, event) {
                    EventResult::PreventDefault
                } else {
                    EventResult::Propagate
                }
            });
    }
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

    /// Ruling 2026-10-22 item 2: a standard close shortcut on a plugin's
    /// tool window is its title bar's close button: the window hides and
    /// the plugin learns the view closed (`ui.closed`). The focused text
    /// field does not matter: the winit hook takes the key before Slint.
    #[test]
    fn standard_close_on_a_tool_window_is_its_close_button() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("w.slint");
        std::fs::write(
            &file,
            r#"import { LineEdit } from "std-widgets.slint";
export component W inherits Window {
    width: 200px; height: 100px;
    field := LineEdit {}
    init => { field.focus(); }
}"#,
        )
        .unwrap();
        let (events, received) = mpsc::channel();
        let view = open(&file, 7, "find", &[], &[], events, |_| {}).unwrap();
        let window = view.instance.window();
        assert!(window.is_visible());
        perform(window, crate::platform::WindowAction::Close);
        assert!(!window.is_visible());
        assert!(matches!(
            received.try_recv(),
            Ok(Event::ViewClosed(7, id)) if id == "find"
        ));
    }

    /// Floating panels and the options window answer close requests with
    /// `KeepWindowShown` and close through their own event; the shortcut
    /// runs that same handler and leaves the hiding to it.
    #[test]
    fn standard_close_runs_the_windows_own_close_handler() {
        headless();
        let instance = compile_source(
            "export component W inherits Window { width: 100px; height: 100px; }".into(),
            Path::new("w.slint"),
        )
        .unwrap();
        instance.show().unwrap();
        let asked = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = asked.clone();
        instance.window().on_close_requested(move || {
            count.set(count.get() + 1);
            slint::CloseRequestResponse::KeepWindowShown
        });
        perform(instance.window(), crate::platform::WindowAction::Close);
        assert_eq!(asked.get(), 1);
        assert!(instance.window().is_visible());
    }

    /// `Theme.font-mono` in a plugin's own window (and so in its Markdown
    /// code blocks) is the running system's monospace family once the
    /// player delivers the fonts (`theme::apply_fonts`).
    #[test]
    fn plugin_windows_read_the_systems_monospace_font() {
        headless();
        let instance = compile_source(
            r#"import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    out property <string> mono: Theme.font-mono;
    Text { text: "代码"; font-family: Theme.font-mono; }
}"#
            .into(),
            &ui_dir().join("w.slint"),
        )
        .unwrap();
        assert_eq!(
            instance.get_property("mono").unwrap(),
            Value::from(slint::SharedString::from("Menlo"))
        );
        crate::theme::apply_fonts(&instance).unwrap();
        assert_eq!(
            instance.get_property("mono").unwrap(),
            Value::from(slint::SharedString::from(
                crate::platform::ui_monospace_font()
            ))
        );
    }

    /// R-H6: a light palette also turns the std-widgets light (the host
    /// compiles them as fluent-dark), switches the gradient, and the default
    /// palette brings both back.
    #[test]
    fn palettes_switch_colours_gradient_and_the_std_widgets_scheme() {
        headless();
        let instance = compile_source(
            r#"import { Palette, CheckBox } from "std-widgets.slint";
import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    out property <brush> std-background: Palette.background;
    out property <brush> fill: Theme.accent-fill;
    out property <color> bar: Theme.bg-bar;
    out property <color> video: Theme.bg-video;
    CheckBox { text: "x"; }
}"#
            .into(),
            &ui_dir().join("w.slint"),
        )
        .unwrap();
        let get = |name: &str| instance.get_property(name).unwrap();
        let brush = |rgb: u32| {
            Value::Brush(slint::Brush::SolidColor(slint::Color::from_rgb_u8(
                (rgb >> 16) as u8,
                (rgb >> 8) as u8,
                rgb as u8,
            )))
        };
        let dark_fill = get("fill");
        assert_eq!(get("bar"), brush(0x16141c));
        assert_eq!(get("std-background"), brush(0x1c1c1c));
        crate::theme::apply(&instance, "vestra-light").unwrap();
        assert_eq!(get("bar"), brush(0xf7f6fa));
        // The video area stays dark.
        assert_eq!(get("video"), brush(0x0e0d12));
        assert_eq!(get("std-background"), brush(0xfafafa));
        assert_ne!(get("fill"), dark_fill);
        crate::theme::apply(&instance, "vestra").unwrap();
        assert_eq!(get("bar"), brush(0x16141c));
        assert_eq!(get("std-background"), brush(0x1c1c1c));
        assert_eq!(get("fill"), dark_fill);
    }

    /// The menu shadow follows the palette in windows that export MenuChrome,
    /// and a window without a menu still takes a palette.
    #[test]
    fn the_menu_shadow_follows_the_palette() {
        headless();
        let instance = compile_source(
            r#"import { MenuChrome } from "menu-view.slint";
export { MenuChrome } from "menu-view.slint";
export component W inherits Window {
    out property <color> shadow: MenuChrome.shadow;
}"#
            .into(),
            &ui_dir().join("w.slint"),
        )
        .unwrap();
        let alpha = || match instance.get_property("shadow").unwrap() {
            Value::Brush(brush) => brush.color().alpha(),
            other => panic!("{other:?}"),
        };
        assert_eq!(alpha(), 0x80);
        crate::theme::apply(&instance, "vestra-light").unwrap();
        assert_eq!(alpha(), 0x33);
        crate::theme::apply(&instance, "vestra").unwrap();
        assert_eq!(alpha(), 0x80);
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

    /// The shell with a bottom bar and a sidebar.
    fn bar_and_sidebar() -> [crate::composition::CompileSpec; 2] {
        use crate::composition::{CompileSpec, Slot};
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        [
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
        ]
    }

    /// Slint 1.17+ (see `settle_layout`): hiding the docked sidebar (an `if`
    /// in the shell's layout) and reading the video area in the same turn,
    /// as the window tick and `window.setVideoArea` do.
    #[test]
    fn geometry_is_current_after_a_condition_changes() {
        headless();
        let source = crate::composition::shell_source(&bar_and_sidebar()).unwrap();
        let shell = crate::composition::build(source, "HostWindow")
            .unwrap()
            .create()
            .unwrap();
        shell
            .window()
            .set_size(slint::LogicalSize::new(1470., 956.));
        shell.show().unwrap();
        let read = |name: &str| match shell.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        settle_layout(shell.window());
        // The sidebar (317px) and its 3px splitter.
        assert_eq!(read("video-width"), 1470. - 320.);
        let unsettled = |visible: bool| {
            shell
                .set_property("sidebar-visible", visible.into())
                .unwrap();
            let before = read("video-width");
            settle_layout(shell.window());
            eprintln!("sidebar-visible {visible}: {before} before settle_layout");
            read("video-width")
        };
        assert_eq!(unsettled(false), 1470.);
        assert_eq!(read("video-x"), 0.);
        assert_eq!(unsettled(true), 1470. - 320.);
        shell.hide().unwrap();
    }

    /// G-01: with the fullscreen sidebar popped out, the bottom bar spans from
    /// the left edge to the overlay's left edge. (It used to be centred: a
    /// narrower element without `x` in a plain parent is centred by Slint.)
    #[test]
    fn popped_sidebar_narrows_the_fullscreen_bar_from_the_left_edge() {
        headless();
        let source = crate::composition::shell_source(&bar_and_sidebar()).unwrap();
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
        // Hiding the docked sidebar removes an `if` from the layout.
        settle_layout(shell.window());
        assert_eq!((read("t-bar-x"), read("t-bar-w")), (0., 1470.));
        set("sidebar-popped", true.into());
        settle_layout(shell.window());
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

    /// Sidebar tabs: the docked column is as wide as with one panel, the
    /// tab strip takes the top 31px, and switching tabs (a property) keeps
    /// the layout; the host-owned `tabbed` reaches a global declaring it.
    #[test]
    fn sidebar_tabs_switch_by_property_without_changing_the_layout() {
        headless();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let mut specs = bar_and_sidebar().to_vec();
        specs.push(crate::composition::CompileSpec {
            owner: 30,
            file: base.join("tab.slint"),
            component: "NoteTab".into(),
            global: Some("Bridge".into()),
            slot: crate::composition::Slot::Sidebar,
        });
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::composition::compile_json(&specs, sender, 1).unwrap();
        crate::composition::set_tabbed(&shell, &specs[2], true).unwrap();
        // The fixture without the property is left alone.
        crate::composition::set_tabbed(&shell, &specs[1], true).unwrap();
        assert_eq!(
            shell
                .get_global_property("Owner30Bridge", "tabbed")
                .unwrap(),
            slint_interpreter::Value::Bool(true)
        );
        shell
            .set_property(
                "sidebar-titles",
                crate::ui_values::from_json(&serde_json::json!(["播放列表", "Example list"]))
                    .unwrap(),
            )
            .unwrap();
        shell
            .window()
            .set_size(slint::LogicalSize::new(1470., 956.));
        shell.show().unwrap();
        let read = |name: &str| match shell.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        for active in [0., 1., 0.] {
            shell.set_property("sidebar-active", active.into()).unwrap();
            settle_layout(shell.window());
            assert_eq!(read("video-width"), 1470. - 320., "tab {active}");
        }
        shell.hide().unwrap();
    }

    /// A window waits for its native window (none on the headless
    /// platform) to be placed, at most `PLACEMENT_TICKS`; a closed one is
    /// forgotten at once.
    #[test]
    fn placements_wait_for_the_native_window_and_forget_closed_ones() {
        headless();
        let make = || {
            let source = r#"export component W inherits Window { width: 100px; height: 80px; }"#;
            let result = pollster::block_on(
                compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
            );
            result.component("W").unwrap().create().unwrap()
        };
        let mut placements = Placements::default();
        let kept = make();
        placements
            .show_at(&kept, slint::LogicalPosition::new(40., 65.))
            .unwrap();
        let closed = make();
        placements.track(&closed, slint::LogicalPosition::new(1., 2.).into());
        drop(closed);
        placements.tick();
        assert_eq!(placements.pending(), 1);
        for _ in 1..PLACEMENT_TICKS - 1 {
            placements.tick();
        }
        assert_eq!(placements.pending(), 1);
        placements.tick();
        assert_eq!(placements.pending(), 0);
        kept.hide().unwrap();
    }

    /// The sources the host generates (main window with panels, floating
    /// panel, options window) compile without warnings, so a Slint upgrade
    /// that deprecates something they use shows up here.
    #[test]
    fn host_generated_sources_compile_without_warnings() {
        let specs = bar_and_sidebar();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let page = crate::settings_window::Page {
            owner: 1,
            manifest: "a.plugin".into(),
            id: "look".into(),
            category: 6,
            title: "look".into(),
            order: 0,
            serial: 1,
            file: base.join("backdrop.slint"),
            component: "Backdrop".into(),
            global: "Bridge".into(),
            properties: Default::default(),
        };
        let mut tabs = specs.to_vec();
        tabs.push(crate::composition::CompileSpec {
            owner: 30,
            file: base.join("tab.slint"),
            component: "NoteTab".into(),
            global: Some("Bridge".into()),
            slot: crate::composition::Slot::Sidebar,
        });
        for source in [
            crate::composition::shell_source(&specs).unwrap(),
            crate::composition::shell_source(&tabs).unwrap(),
            crate::composition::floating_source(&tabs[1..], "列表").unwrap(),
            crate::composition::shell_source(&[]).unwrap(),
            crate::composition::floating_source(&specs[1..], "列表").unwrap(),
            crate::settings_window::source(&[&page]).unwrap(),
        ] {
            let result = pollster::block_on(
                compiler().build_from_source(source, ui_dir().join("host.slint")),
            );
            // Known since Slint 1.16 (not new in 1.18): `sidebar-actual`
            // reads the window width that the sidebar's fixed width feeds.
            let diagnostics: Vec<_> = result
                .diagnostics()
                .map(|d| d.to_string())
                .filter(|d| {
                    !d.contains("part of a binding loop (root.layoutinfo-h -> root.sidebar-actual")
                })
                .collect();
            assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
