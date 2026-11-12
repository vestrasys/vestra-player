//! Behaviour of the `@vestra/widgets.slint` form controls (Choice, Check,
//! Radio, RadioGroup, Tabs, Tab) and the Slider's keyboard, driven by
//! pointer and key events on a windowless platform; and an ignored test that
//! draws every control state to PNG (`VESTRA_WIDGET_SHOTS=<dir>`).
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{LogicalPosition, SharedString};
use slint_interpreter::{ComponentHandle, ComponentInstance, Value};

struct Headless(Rc<MinimalSoftwareWindow>);
impl slint::platform::Platform for Headless {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// This thread's windowless platform and its one window (a new buffer per
/// frame, so a drawn picture is whole).
fn headless() -> Rc<MinimalSoftwareWindow> {
    thread_local! {
        static WINDOW: Rc<MinimalSoftwareWindow> = {
            let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
            window
        };
    }
    WINDOW.with(Rc::clone)
}

/// Compiles `W` from `source` as the player compiles a plugin window (the
/// library paths, fluent-dark, the palette export) and fails on any
/// diagnostic, warnings included.
fn build(source: &str, width: f32, height: f32) -> ComponentInstance {
    headless();
    let mut source = source.to_owned();
    source.push('\n');
    source.push_str(&crate::ui::theme::export_line());
    let path: PathBuf = crate::ui::view::ui_dir().join("widgets-test.slint");
    let result = pollster::block_on(crate::ui::view::compiler().build_from_source(source, path));
    let diagnostics: Vec<String> = result.diagnostics().map(|d| d.to_string()).collect();
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let instance = result.component("W").unwrap().create().unwrap();
    instance.show().unwrap();
    instance
        .window()
        .set_size(slint::LogicalSize::new(width, height));
    instance
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    settle();
    instance
}

fn settle() {
    slint::platform::update_timers_and_animations();
}

fn click(instance: &ComponentInstance, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let window = instance.window();
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    settle();
}

fn key(instance: &ComponentInstance, text: impl Into<SharedString>) {
    let text = text.into();
    let window = instance.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    settle();
}

fn get(instance: &ComponentInstance, name: &str) -> Value {
    instance
        .get_property(name)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}
fn number(instance: &ComponentInstance, name: &str) -> f64 {
    match get(instance, name) {
        Value::Number(n) => n,
        other => panic!("{name}: {other:?}"),
    }
}
fn boolean(instance: &ComponentInstance, name: &str) -> bool {
    match get(instance, name) {
        Value::Bool(b) => b,
        other => panic!("{name}: {other:?}"),
    }
}
fn string(instance: &ComponentInstance, name: &str) -> String {
    match get(instance, name) {
        Value::String(s) => s.into(),
        other => panic!("{name}: {other:?}"),
    }
}
fn call(instance: &ComponentInstance, name: &str) {
    instance
        .invoke(name, &[])
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    settle();
}

const SPACE: &str = " ";

/// The command palette's field (a plain `TextInput`, review of
/// 2026-11-08): a query longer than the field scrolls with the cursor and
/// is clipped to the field, as std-widgets' LineEdit; Home brings the
/// start back.
#[test]
fn the_palette_field_keeps_the_cursor_in_view() {
    let palette = build(
        r#"import { PaletteView } from "command-palette.slint";
export component W inherits Window {
    p := PaletteView { width: 100%; height: 100%; }
    out property <length> cursor: p.cursor-in-field;
    out property <length> room: p.field-width;
    out property <string> query: p.query;
}"#,
        420.,
        300.,
    );
    let room = number(&palette, "room");
    assert!(room > 100., "{room}");
    let inside = |what: &str| {
        let cursor = number(&palette, "cursor");
        assert!(
            (0. ..=room).contains(&cursor),
            "{what}: cursor at {cursor}, field {room}"
        );
        cursor
    };
    for _ in 0..120 {
        key(&palette, "w");
    }
    assert_eq!(
        get(&palette, "query"),
        Value::String("w".repeat(120).into())
    );
    // At the end of a query far wider than the field: near its right edge.
    let end = inside("after typing");
    assert!(end > room / 2., "{end} of {room}");
    // Home brings the start back where Home is the line start; on macOS it
    // is not (Slint follows the platform there: Cmd+Left is), so the step
    // is left out (CI on macOS, 2026-11-08).
    if !cfg!(target_os = "macos") {
        key(&palette, Key::Home);
        let start = inside("after Home");
        assert!(start < 4., "{start}");
    }
    key(&palette, Key::End);
    inside("after End");
}

/// Every control in every mode compiles without a warning, in a layout as
/// the plugins place them.
#[test]
fn form_controls_compile_without_warnings() {
    build(
        r#"
import { Theme } from "@vestra/theme.slint";
import { Button, Check, Choice, Radio, RadioGroup, RadioMark, Slider, Tab, Tabs } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <bool> on;
    in-out property <int> index;
    VerticalLayout {
        Tabs { model: ["Equalizer", "Effects", "Output"]; current-index <=> root.index; }
        HorizontalLayout { Tab { text: "Updates"; emphasis: true; } Tab { text: "All"; selected: true; } }
        Check { text: "Equalizer"; checked <=> root.on; toggled(on) => { root.index = on ? 1 : 0; } }
        Check { text: "Panel"; panel: true; enabled: false; }
        Radio { text: "None"; checked: root.index == 0; clicked => { root.index = 0; } }
        RadioGroup { model: ["Off", "All", "One"]; current-index <=> root.index; }
        RadioGroup { vertical: true; model: ["Off", "All", "One"]; }
        RadioMark { checked: true; hover: true; }
        Choice { width: 140px; model: ["Stereo", "Mono"]; current-index <=> root.index; max-visible-rows: 7; popup-min-width: 120px; }
        Slider { panel: true; step: 0.5; }
        Button { text: "OK"; }
    }
}"#,
        400.,
        400.,
    );
}

/// `KeyHint` (SDK 1.6.7) sizes to its label and takes no space with an
/// empty one (a cleared key); a `ToolTipArea` with `keys` and `detail`
/// compiles without a warning and still shows its tip on hover; changing
/// the keys while it shows keeps it showing.
#[test]
fn key_hints_size_to_their_label_and_vanish_when_empty() {
    let w = build(
        r#"
import { Theme } from "@vestra/theme.slint";
import { KeyHint, ToolTipArea, ToolTipKey } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <string> down: "W";
    in-out property <string> up: "Ctrl+Shift+E";
    in-out property <[ToolTipKey]> keys: [{ key: "W", text: "−" }, { key: "", text: "+" }];
    out property <float> down-width: d.width / 1px;
    out property <float> up-width: u.width / 1px;
    out property <float> down-height: d.height / 1px;
    out property <bool> tip-showing: tip.showing;
    background: Theme.bg-panel;
    HorizontalLayout {
        alignment: start;
        padding: 4px;
        spacing: 2px;
        VerticalLayout { alignment: start;
            HorizontalLayout { alignment: start; spacing: 2px;
                d := KeyHint { text: root.down; }
                u := KeyHint { text: root.up; } }
            tip := ToolTipArea {
                width: 100px; height: 30px;
                text: "Brightness"; keys: root.keys; detail: "· double-click: Default";
                Rectangle { background: Theme.bg-list; }
            }
        }
    }
}"#,
        300.,
        200.,
    );
    let down = number(&w, "down-width");
    assert!(
        down >= 16.,
        "a one-letter cap is at least 16px wide: {down}"
    );
    assert!(
        number(&w, "up-width") > down,
        "a longer label widens the cap"
    );
    assert_eq!(number(&w, "down-height"), 15.);
    w.set_property("down", Value::String("".into())).unwrap();
    settle();
    assert_eq!(number(&w, "down-width"), 0.);
    assert_eq!(number(&w, "down-height"), 0.);
    // Hover the tooltip area (below the caps): the tip shows after its delay.
    let position = LogicalPosition::new(40., 40.);
    w.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    settle();
    std::thread::sleep(std::time::Duration::from_millis(650));
    settle();
    assert!(boolean(&w, "tip-showing"));
    let keys: Vec<Value> = [("Shift+W", "−"), ("E", "+")]
        .into_iter()
        .map(|(key, text)| {
            let mut entry = slint_interpreter::Struct::default();
            entry.set_field("key".into(), Value::String(key.into()));
            entry.set_field("text".into(), Value::String(text.into()));
            Value::Struct(entry)
        })
        .collect();
    let model = slint::ModelRc::new(slint::VecModel::from(keys));
    w.set_property("keys", Value::Model(model)).unwrap();
    settle();
    assert!(boolean(&w, "tip-showing"));
}

#[test]
fn check_flips_on_click_and_space_never_takes_focus_by_click_and_wraps_its_label() {
    let w = build(
        r#"
import { Check } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <bool> on;
    in-out property <string> log;
    in-out property <bool> enabled: true;
    out property <bool> focused: c.has-focus;
    out property <length> wrapped-height: long.height;
    out property <length> short-height: c.height;
    public function focus-check() { c.focus(); }
    c := Check { x: 10px; y: 10px; width: 200px; height: 18px; text: "Loop"; enabled: root.enabled;
        checked <=> root.on; toggled(on) => { root.log += on ? "on;" : "off;"; } }
    VerticalLayout {
        x: 10px; y: 40px; width: 120px;
        alignment: start;
        long := Check { text: "Check for updates automatically, then every 24 hours"; }
    }
}"#,
        300.,
        200.,
    );
    click(&w, 15., 19.);
    assert!(boolean(&w, "on"));
    assert!(!boolean(&w, "focused"), "a click does not take the focus");
    click(&w, 100., 19.);
    assert!(!boolean(&w, "on"), "the label is part of the target");
    call(&w, "focus-check");
    assert!(boolean(&w, "focused"));
    key(&w, SPACE);
    assert!(boolean(&w, "on"));
    // Enter is the dialog's, not the check's.
    key(&w, Key::Return);
    assert!(boolean(&w, "on"));
    assert_eq!(string(&w, "log"), "on;off;on;");
    w.set_property("enabled", false.into()).unwrap();
    settle();
    click(&w, 15., 19.);
    assert!(boolean(&w, "on"), "disabled");
    assert_eq!(number(&w, "short-height"), 18.);
    assert!(
        number(&w, "wrapped-height") > 30.,
        "a long label wraps: {}",
        number(&w, "wrapped-height")
    );
}

#[test]
fn check_reaches_the_focus_by_tab() {
    let w = build(
        r#"
import { Check } from "@vestra/widgets.slint";
export component W inherits Window {
    out property <bool> focused: c.has-focus;
    c := Check { x: 10px; y: 10px; width: 100px; height: 18px; text: "Loop"; }
}"#,
        200.,
        100.,
    );
    key(&w, Key::Tab);
    assert!(boolean(&w, "focused"));
}

#[test]
fn radio_group_picks_by_click_and_by_arrow_keys_which_wrap() {
    let w = build(
        r#"
import { Radio, RadioGroup } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <int> index: -1;
    in-out property <string> log;
    in-out property <int> lone;
    public function focus-group() { g.focus(); }
    g := RadioGroup { x: 10px; y: 10px; width: 300px; height: 18px; model: ["Off", "All", "One"];
        current-index <=> root.index; selected(i) => { root.log += i + ";"; } }
    r := Radio { x: 10px; y: 50px; width: 100px; height: 18px; text: "Alone"; checked: root.lone == 1;
        clicked => { root.lone += 1; } }
}"#,
        400.,
        100.,
    );
    // "Off" is at x 10..~40; the radios are 12px apart.
    click(&w, 14., 19.);
    assert_eq!(number(&w, "index"), 0.);
    click(&w, 14., 19.);
    assert_eq!(
        string(&w, "log"),
        "0;",
        "picking the current radio is no change"
    );
    call(&w, "focus-group");
    key(&w, Key::RightArrow);
    key(&w, Key::DownArrow);
    assert_eq!(number(&w, "index"), 2.);
    key(&w, Key::RightArrow);
    assert_eq!(number(&w, "index"), 0., "wraps to the first");
    key(&w, Key::LeftArrow);
    assert_eq!(number(&w, "index"), 2., "wraps to the last");
    assert_eq!(string(&w, "log"), "0;1;2;0;2;");
    // A lone radio asks; the plugin decides.
    click(&w, 14., 59.);
    click(&w, 14., 59.);
    assert_eq!(number(&w, "lone"), 2.);
}

#[test]
fn tabs_pick_by_click_and_keys_and_size_to_their_labels() {
    let w = build(
        r#"
import { Tabs } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <int> index;
    in-out property <string> log;
    out property <length> strip-min: t.min-width;
    out property <length> strip-height: t.height;
    public function focus-tabs() { t.focus(); }
    t := Tabs { x: 0px; y: 0px; width: 400px; model: ["Equalizer", "Effects", "A rather long tab label"];
        current-index <=> root.index; selected(i) => { root.log += i + ";"; } }
}"#,
        400.,
        100.,
    );
    assert_eq!(number(&w, "strip-height"), 31.);
    // Two 74px tabs and one wider than 74.
    assert!(
        number(&w, "strip-min") > 3. * 74.,
        "{}",
        number(&w, "strip-min")
    );
    click(&w, 74. + 37., 15.);
    assert_eq!(number(&w, "index"), 1.);
    call(&w, "focus-tabs");
    key(&w, Key::RightArrow);
    key(&w, Key::RightArrow);
    assert_eq!(number(&w, "index"), 2., "stops at the last");
    key(&w, Key::Home);
    assert_eq!(number(&w, "index"), 0.);
    key(&w, Key::End);
    assert_eq!(string(&w, "log"), "1;2;0;2;");
}

const CHOICE: &str = r#"
import { Choice } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <[string]> model: ["Stereo", "Left Channel", "Right Channel", "Mono Mix", "Swap Left and Right"];
    in-out property <int> index: 1;
    in-out property <string> log;
    out property <bool> open: c.open;
    out property <string> shown: c.current-text;
    out property <bool> focused: c.has-focus;
    public function focus-choice() { c.focus(); }
    c := Choice { x: 10px; y: 10px; width: 140px; model: root.model; max-visible-rows: 7;
        current-index <=> root.index; selected(i) => { root.log += i + ";"; } }
}"#;

#[test]
fn choice_opens_its_list_and_picks_with_the_pointer() {
    let w = build(CHOICE, 300., 300.);
    assert_eq!(string(&w, "shown"), "Left Channel");
    click(&w, 50., 20.);
    assert!(boolean(&w, "open"));
    assert!(
        !boolean(&w, "focused"),
        "a click does not focus the closed field"
    );
    // Rows: the list starts at y 10 + 21, 1px border, 20px rows.
    click(&w, 50., 31. + 1. + 3. * 20. + 10.);
    assert!(!boolean(&w, "open"));
    assert_eq!(number(&w, "index"), 3.);
    assert_eq!(string(&w, "shown"), "Mono Mix");
    // A click outside closes without a change.
    click(&w, 50., 20.);
    assert!(boolean(&w, "open"));
    click(&w, 250., 280.);
    assert!(!boolean(&w, "open"));
    assert_eq!(string(&w, "log"), "3;");
}

#[test]
fn choice_keyboard_changes_closed_and_moves_the_lit_row_open() {
    let w = build(CHOICE, 300., 300.);
    call(&w, "focus-choice");
    assert!(boolean(&w, "focused"));
    key(&w, Key::DownArrow);
    assert_eq!(number(&w, "index"), 2.);
    key(&w, Key::UpArrow);
    key(&w, Key::UpArrow);
    key(&w, Key::UpArrow);
    assert_eq!(number(&w, "index"), 0., "stops at the first");
    key(&w, Key::End);
    assert_eq!(number(&w, "index"), 4.);
    // Open: the arrows light rows; Escape keeps the choice.
    key(&w, Key::Return);
    assert!(boolean(&w, "open"));
    key(&w, Key::UpArrow);
    key(&w, Key::UpArrow);
    key(&w, Key::Escape);
    assert!(!boolean(&w, "open"));
    assert_eq!(number(&w, "index"), 4.);
    assert!(boolean(&w, "focused"), "the focus comes back from the list");
    // Enter picks the lit row.
    key(&w, SPACE);
    assert!(boolean(&w, "open"));
    key(&w, Key::UpArrow);
    key(&w, Key::Return);
    assert!(!boolean(&w, "open"));
    assert_eq!(number(&w, "index"), 3.);
    key(&w, Key::F4);
    assert!(boolean(&w, "open"));
    key(&w, Key::Home);
    key(&w, SPACE);
    assert_eq!(number(&w, "index"), 0.);
    assert_eq!(string(&w, "log"), "2;1;0;4;3;0;");
}

/// Near the window's bottom the list moves up to end at the window's
/// edge (Slint keeps a popup inside its window), instead of being cut off.
#[test]
fn choice_list_stays_inside_the_window() {
    let w = build(&CHOICE.replace("y: 10px;", "y: 250px;"), 300., 300.);
    click(&w, 50., 260.);
    assert!(boolean(&w, "open"));
    // 5 rows: 102px high, so it spans y 198..300; its first row is at 199.
    click(&w, 50., 209.);
    assert!(!boolean(&w, "open"));
    assert_eq!(number(&w, "index"), 0.);
}

/// The list is a ListView: a long list opens at once and scrolls with the
/// keys to its end.
#[test]
fn choice_long_list_scrolls_to_the_lit_row() {
    let w = build(CHOICE, 300., 300.);
    let model: Vec<Value> = (0..5000)
        .map(|i| Value::String(format!("Device {i}").into()))
        .collect();
    w.set_property(
        "model",
        Value::Model(slint::ModelRc::new(slint::VecModel::from(model))),
    )
    .unwrap();
    w.set_property("index", 2500.into()).unwrap();
    settle();
    assert_eq!(string(&w, "shown"), "Device 2500");
    let started = std::time::Instant::now();
    call(&w, "focus-choice");
    key(&w, Key::Return);
    assert!(boolean(&w, "open"));
    // The current row is lit and visible: Enter keeps it.
    key(&w, Key::Return);
    assert_eq!(number(&w, "index"), 2500.);
    key(&w, Key::Return);
    key(&w, Key::End);
    key(&w, Key::PageUp);
    key(&w, Key::Return);
    assert_eq!(number(&w, "index"), 4999. - 7.);
    // Click the last visible row after scrolling to the end: the view
    // follows the lit row, so the bottom row is the last item.
    key(&w, Key::Return);
    key(&w, Key::End);
    click(&w, 50., 31. + 1. + 6. * 20. + 10.);
    assert_eq!(number(&w, "index"), 4999.);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn slider_keys_step_and_report_release() {
    let w = build(
        r#"
import { Slider } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <float> value: 50;
    in-out property <int> changes;
    in-out property <int> releases;
    in-out property <bool> enabled: true;
    out property <bool> focused: s.has-focus;
    public function focus-slider() { s.focus(); }
    s := Slider { x: 10px; y: 10px; width: 100px; height: 10px; minimum: 0; maximum: 100;
        enabled: root.enabled; value <=> root.value;
        changed => { root.changes += 1; } released => { root.releases += 1; } }
}"#,
        200.,
        100.,
    );
    click(&w, 60., 15.);
    assert!(
        !boolean(&w, "focused"),
        "a click leaves the focus where it was"
    );
    w.set_property("value", Value::Number(50.)).unwrap();
    w.set_property("changes", 0.into()).unwrap();
    w.set_property("releases", 0.into()).unwrap();
    call(&w, "focus-slider");
    key(&w, Key::RightArrow);
    assert_eq!(number(&w, "value"), 55., "a twentieth of the range");
    key(&w, Key::DownArrow);
    key(&w, Key::DownArrow);
    assert_eq!(number(&w, "value"), 45.);
    key(&w, Key::PageUp);
    assert_eq!(number(&w, "value"), 95.);
    key(&w, Key::PageUp);
    assert_eq!(number(&w, "value"), 100., "clamped");
    key(&w, Key::Home);
    assert_eq!(number(&w, "value"), 0.);
    key(&w, Key::LeftArrow);
    assert_eq!(number(&w, "value"), 0.);
    assert_eq!(number(&w, "changes"), 6., "no change at the end");
    assert_eq!(number(&w, "releases"), 7.);
    w.set_property("enabled", false.into()).unwrap();
    settle();
    key(&w, Key::End);
    assert_eq!(number(&w, "value"), 0.);
}

/// Plugin-decided (`self-toggle: false`): the box does not flip, `toggled`
/// asks for the other state, and the one-way binding stays (the hotkeys
/// plugin confirms "Ignore default shortcuts" before it turns on).
#[test]
fn check_without_self_toggle_asks_and_keeps_its_binding() {
    let w = build(
        r#"
import { Check } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <bool> on;
    in-out property <string> log;
    out property <bool> shown: c.checked;
    public function focus-check() { c.focus(); }
    c := Check { x: 10px; y: 10px; width: 200px; height: 18px; text: "Ignore"; self-toggle: false;
        checked: root.on; toggled(want) => { root.log += want ? "want-on;" : "want-off;"; }
        clicked => { root.log += "clicked;"; } }
}"#,
        300.,
        100.,
    );
    click(&w, 15., 19.);
    assert!(!boolean(&w, "shown"), "the box waits for the plugin");
    w.set_property("on", true.into()).unwrap();
    settle();
    assert!(boolean(&w, "shown"), "the binding is still there");
    call(&w, "focus-check");
    key(&w, SPACE);
    assert!(boolean(&w, "shown"));
    assert_eq!(string(&w, "log"), "want-on;clicked;want-off;clicked;");
}

/// Tabs passes `emphasis` per tab: the store's "Updates (2)" label is drawn
/// in the accent colour, the others are not.
#[test]
fn tabs_emphasise_single_tabs() {
    let w = build(
        r#"
import { Theme } from "@vestra/theme.slint";
import { Tabs } from "@vestra/widgets.slint";
export component W inherits Window {
    out property <color> accent: Theme.accent;
    Tabs { x: 0px; y: 0px; width: 300px; model: ["All", "Installed", "Updates (2)"]; emphasis: [false, false, true]; }
}"#,
        300.,
        31.,
    );
    let Value::Brush(accent) = get(&w, "accent") else {
        panic!("accent")
    };
    let accent = accent.color();
    let picture = render(&w, 1., 300., 31.);
    let accent_pixels = |from: u32, to: u32| {
        picture
            .enumerate_pixels()
            .filter(|(x, _, p)| {
                (from..to).contains(x)
                    && [p[0], p[1], p[2]]
                        .iter()
                        .zip([accent.red(), accent.green(), accent.blue()])
                        .all(|(a, b)| a.abs_diff(b) < 40)
            })
            .count()
    };
    // Tabs are 74px or wider; "Updates (2)" is the third.
    assert_eq!(accent_pixels(74, 148), 0, "Installed");
    assert!(accent_pixels(148, 300) > 5, "Updates (2)");
}

/// `focusable: false` keeps a slider out of the Tab chain (the bottom bar),
/// `focus()` still reaches it.
#[test]
fn slider_out_of_the_tab_chain() {
    let w = build(
        r#"
import { Check, Slider } from "@vestra/widgets.slint";
export component W inherits Window {
    out property <bool> slider-focused: s.has-focus;
    out property <bool> check-focused: c.has-focus;
    public function focus-slider() { s.focus(); }
    s := Slider { x: 10px; y: 10px; width: 100px; height: 10px; focusable: false; }
    c := Check { x: 10px; y: 40px; width: 100px; height: 18px; text: "After"; }
}"#,
        200.,
        100.,
    );
    key(&w, Key::Tab);
    assert!(!boolean(&w, "slider-focused"));
    assert!(boolean(&w, "check-focused"));
    call(&w, "focus-slider");
    assert!(boolean(&w, "slider-focused"));
}

/// In a window shorter than the list, `window-height` cuts the list to the
/// window and the last row can still be reached and picked.
#[test]
fn choice_list_fits_a_short_window() {
    let w = build(
        r#"
import { Choice } from "@vestra/widgets.slint";
export component W inherits Window {
    in-out property <int> index;
    public function focus-choice() { c.focus(); }
    c := Choice { x: 10px; y: 10px; width: 140px; window-height: root.height; current-index <=> root.index;
        model: ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]; }
}"#,
        300.,
        100.,
    );
    call(&w, "focus-choice");
    key(&w, Key::Return);
    key(&w, Key::End);
    // The list is 100px high, moved up to y 0; scrolled to the end, row 9
    // ends at the list's bottom border.
    click(&w, 50., 90.);
    assert_eq!(number(&w, "index"), 9.);
}

/// Draws the controls in their states, both palettes, to
/// `VESTRA_WIDGET_SHOTS` (PNG, one pixel per logical pixel, scale 2 copies
/// beside them): `controls-<palette>.png`, `choice-open-<palette>.png`.
#[test]
#[ignore = "draws the widget gallery: VESTRA_WIDGET_SHOTS=<dir>"]
fn render_widget_gallery() {
    let Some(out) = std::env::var_os("VESTRA_WIDGET_SHOTS") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    for palette in ["vestra", "vestra-light"] {
        for scale in [1f32, 2.] {
            let suffix = if scale == 1. {
                palette.to_owned()
            } else {
                format!("{palette}@2x")
            };
            {
                let w = build(GALLERY, 560., 420.);
                crate::ui::theme::apply(&w, palette).unwrap();
                call(&w, "focus-rings");
                shoot(
                    &w,
                    scale,
                    560.,
                    420.,
                    &out.join(format!("controls-{suffix}.png")),
                );
                w.hide().unwrap();
            }
            {
                let w = build(GALLERY_OPEN, 300., 260.);
                crate::ui::theme::apply(&w, palette).unwrap();
                call(&w, "open-lists");
                key(&w, Key::DownArrow);
                key(&w, Key::DownArrow);
                let png = out.join(format!("choice-open-{suffix}.png"));
                shoot(&w, scale, 300., 260., &png);
                w.hide().unwrap();
            }
        }
    }
}

fn shoot(w: &ComponentInstance, scale: f32, width: f32, height: f32, png: &std::path::Path) {
    render(w, scale, width, height).save(png).unwrap();
}

/// The window drawn at `width` × `height` logical pixels and `scale`.
fn render(w: &ComponentInstance, scale: f32, width: f32, height: f32) -> image::RgbaImage {
    let window = headless();
    w.window().dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    w.window().set_size(slint::LogicalSize::new(width, height));
    settle();
    crate::ui::view::settle_layout(w.window());
    let size = w.window().size();
    let (pw, ph) = (size.width, size.height);
    let mut buffer = vec![PremultipliedRgbaColor::default(); pw as usize * ph as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, pw as usize);
    });
    let mut pixels = Vec::with_capacity(buffer.len() * 4);
    for p in buffer {
        let unmultiply = |c: u8| {
            if p.alpha == 0 {
                0
            } else {
                ((c as u32 * 255 + p.alpha as u32 / 2) / p.alpha as u32).min(255) as u8
            }
        };
        pixels.extend_from_slice(&[
            unmultiply(p.red),
            unmultiply(p.green),
            unmultiply(p.blue),
            p.alpha,
        ]);
    }
    image::RgbaImage::from_raw(pw, ph, pixels).unwrap()
}

const GALLERY: &str = r#"
import { Theme } from "@vestra/theme.slint";
import { Check, Choice, KeyHint, Radio, RadioGroup, Slider, Tab, Tabs } from "@vestra/widgets.slint";
component Caption inherits Text { width: 110px; color: Theme.text-dim; font-size: Theme.font-xs; vertical-alignment: center; }
export component W inherits Window {
    background: Theme.bg-panel;
    public function focus-rings() { focused-check.focus(); }
    VerticalLayout {
        Tabs { model: ["Equalizer", "Effects", "Output"]; current-index: 1; }
        VerticalLayout {
            padding: 10px;
            spacing: 6px;
            HorizontalLayout { spacing: 12px; Caption { text: "Check"; }
                Check { text: "Off"; } Check { text: "On"; checked: true; }
                Check { text: "Disabled"; enabled: false; } Check { text: "Disabled on"; checked: true; enabled: false; } }
            HorizontalLayout { spacing: 12px; Caption { text: "Check panel"; }
                Check { panel: true; text: "Off"; } Check { panel: true; text: "On"; checked: true; }
                Check { panel: true; text: "Disabled"; enabled: false; checked: true; } }
            HorizontalLayout { spacing: 12px; Caption { text: "Check focused"; }
                focused-check := Check { text: "Focus ring"; checked: true; }
                Check { width: 200px; text: "A long label wraps onto a second line beside its box"; } }
            HorizontalLayout { spacing: 12px; Caption { text: "Radio"; }
                Radio { text: "Off"; } Radio { text: "On"; checked: true; }
                Radio { text: "Disabled"; enabled: false; } Radio { text: "Focused"; focused: true; } }
            HorizontalLayout { spacing: 12px; Caption { text: "RadioGroup"; }
                RadioGroup { model: ["Off", "All", "One"]; current-index: 1; }
                RadioGroup { vertical: true; model: ["Ask", "Always", "Never"]; current-index: 0; } }
            HorizontalLayout { spacing: 12px; Caption { text: "Choice"; }
                Choice { width: 140px; model: ["Stereo", "Mono Mix"]; current-index: 0; }
                Choice { width: 140px; model: ["Disabled"]; current-index: 0; enabled: false; }
                Choice { width: 100px; model: ["A label too long for the field"]; current-index: 0; } }
            HorizontalLayout { spacing: 12px; Caption { text: "Slider"; }
                Slider { width: 120px; value: 0.4; } Slider { width: 120px; panel: true; value: 0.7; }
                Slider { width: 80px; panel: true; value: 0.3; enabled: false; } }
            HorizontalLayout { spacing: 0px; Caption { text: "Tab"; }
                Tab { text: "All"; selected: true; } Tab { text: "Installed"; }
                Tab { text: "Updates (2)"; emphasis: true; } Tab { text: "Focused"; focused: true; }
                Rectangle { horizontal-stretch: 1; } }
            HorizontalLayout { spacing: 4px; alignment: start; Caption { text: "KeyHint"; }
                KeyHint { text: "W"; } KeyHint { text: "E"; } KeyHint { text: "Ctrl+B"; } KeyHint { text: "⇧⌘S"; } KeyHint { text: ""; } }
            Rectangle { vertical-stretch: 1; }
        }
    }
}"#;

const GALLERY_OPEN: &str = r#"
import { Theme } from "@vestra/theme.slint";
import { Choice } from "@vestra/widgets.slint";
export component W inherits Window {
    background: Theme.bg-panel;
    public function open-lists() { c.focus(); c.show-list(); }
    c := Choice { x: 10px; y: 10px; width: 160px; max-visible-rows: 7; current-index: 3;
        model: ["System Default", "Speakers (Realtek(R) Audio)", "Headphones (USB Audio)", "LG HDR 4K (NVIDIA High Definition Audio)",
            "Digital Output (S/PDIF)", "Microphone Array", "Virtual Cable", "Bluetooth Earbuds", "Monitor Speakers", "Dock Audio"]; }
}"#;

/// The status strip (ui/lead-strip.slint, host/lead_strip.rs) in a window
/// `room` + 16px wide: its size, presses and fade.
const STRIP: &str = r#"
import { LeadStripView, LeadStrip } from "lead-strip.slint";
export component W inherits Window {
    in property <LeadStrip> s;
    in property <bool> still;
    in-out property <string> last;
    out property <length> w: v.width;
    out property <length> h: v.height;
    out property <bool> shown: v.visible;
    out property <length> right: v.x + v.width;
    v := LeadStripView {
        x: 8px; y: 8px;
        room: root.width - 16px;
        strip: root.s;
        still: root.still;
        action(name) => { root.last = name; }
    }
}"#;

/// A `LeadStrip` value as the host writes it.
fn strip_value(kind: &str, text: &str, progress: f64, visible: bool) -> Value {
    let stop = matches!(kind, "working" | "running");
    Value::Struct(
        [
            ("visible", Value::Bool(visible)),
            ("kind", Value::String(kind.into())),
            ("text", Value::String(text.into())),
            ("progress", Value::Number(progress)),
            ("stop", Value::Bool(stop)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
    )
}

fn set_strip(w: &ComponentInstance, value: Value) {
    w.set_property("s", value).unwrap();
    settle();
}

/// §4.2: one line 32px high, 168–420px wide (an end state two lines, 48px,
/// up to 520px), never wider than the window allows.
#[test]
fn the_lead_strip_sizes_to_its_text() {
    let w = build(STRIP, 600., 120.);
    set_strip(&w, strip_value("working", "Planning…", 0., true));
    assert_eq!((number(&w, "h"), number(&w, "w")), (32., 168.));
    set_strip(
        &w,
        strip_value("running", &"Vestra Player: Mute ".repeat(8), 0.5, true),
    );
    assert_eq!((number(&w, "h"), number(&w, "w")), (32., 420.));
    let long = "No command sets an exact volume. Ask for Volume Up or Volume Down, or drag the volume slider in the bar.";
    set_strip(&w, strip_value("note", long, 0., true));
    assert_eq!((number(&w, "h"), number(&w, "w")), (48., 520.));
    // Two lines at most.
    set_strip(&w, strip_value("note", &long.repeat(4), 0., true));
    assert_eq!(number(&w, "h"), 48.);
    set_strip(
        &w,
        strip_value("done", "Done · Vestra Player: Mute", 0., true),
    );
    let one = number(&w, "w");
    assert!(number(&w, "h") == 32. && one > 168. && one < 420., "{one}");
    // A narrow window: the window's width less 16px.
    let narrow = build(STRIP, 200., 120.);
    set_strip(
        &narrow,
        strip_value("working", &"Planning ".repeat(10), 0., true),
    );
    assert_eq!(number(&narrow, "w"), 184.);
}

/// Stop, a press elsewhere (opens the palette; an end state goes), and no
/// press once it faded out.
#[test]
fn the_lead_strip_reports_presses_and_takes_none_once_gone() {
    let w = build(STRIP, 600., 120.);
    set_strip(&w, strip_value("working", "Planning…", 0., true));
    let right = number(&w, "right") as f32;
    click(&w, right - 20., 24.);
    assert_eq!(string(&w, "last"), "stop");
    click(&w, 20., 24.);
    assert_eq!(string(&w, "last"), "open");
    set_strip(
        &w,
        strip_value("done", "Done · Vestra Player: Mute", 0., true),
    );
    let right = number(&w, "right") as f32;
    click(&w, right - 20., 24.);
    assert_eq!(string(&w, "last"), "dismiss");
    w.set_property("last", Value::String("".into())).unwrap();
    set_strip(
        &w,
        strip_value("done", "Done · Vestra Player: Mute", 0., false),
    );
    // The 300 ms fade-out, then nothing of it is there.
    std::thread::sleep(std::time::Duration::from_millis(400));
    settle();
    assert!(!boolean(&w, "shown"));
    click(&w, 20., 24.);
    assert_eq!(string(&w, "last"), "");
}

/// Only a working strip that shows animates (the sweep reads
/// `animation-tick()`): a step, an end state, a hidden or a still strip
/// asks for no frame.
#[test]
fn the_lead_strip_animates_only_while_it_sweeps() {
    let w = build(STRIP, 600., 120.);
    let animating = |value: Value, still: bool| {
        w.set_property("still", Value::Bool(still)).unwrap();
        w.set_property("s", value).unwrap();
        // Past any fade and fill transition.
        std::thread::sleep(std::time::Duration::from_millis(400));
        settle();
        // Drawing evaluates the line; only the sweep reads the tick.
        let _ = render(&w, 1., 600., 120.);
        w.window().has_active_animations()
    };
    assert!(animating(
        strip_value("working", "Planning…", 0., true),
        false
    ));
    assert!(!animating(
        strip_value("working", "Planning…", 0., true),
        true
    ));
    assert!(!animating(
        strip_value("running", "1/2 · Mute", 0.5, true),
        false
    ));
    assert!(!animating(
        strip_value("done", "Done · Mute", 1., true),
        false
    ));
    assert!(!animating(
        strip_value("working", "Planning…", 0., false),
        false
    ));
}

/// The strip's states as the spec's renders show them (PROPOSAL §4.1,
/// proposed/detail-*.png), over the spec's video still, at the top of a
/// 1100px window at 2×, both palettes: `VESTRA_STRIP_SHOTS=<dir>`,
/// `VESTRA_STRIP_LANG=en|zh-Hans` (one language per process: run this test
/// alone), `VESTRA_STRIP_BACKDROP=<png>` (optional).
#[test]
#[ignore = "draws the status strip's states: VESTRA_STRIP_SHOTS=<dir>"]
fn render_lead_strip_states() {
    let Some(out) = std::env::var_os("VESTRA_STRIP_SHOTS") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let lang = std::env::var("VESTRA_STRIP_LANG").unwrap_or_else(|_| "en".into());
    let locale = crate::i18n::LOCALES
        .iter()
        .copied()
        .find(|l| *l == lang)
        .expect("a player language");
    headless();
    crate::i18n::init(locale);
    crate::ui::ui_fonts::install(locale);
    let zh = locale == "zh-Hans";
    let pick = |en: &'static str, zh_text: &'static str| if zh { zh_text } else { en };
    let states: Vec<(&str, &str, &str, f64)> = vec![
        ("r02-working", "working", pick("Planning…", "规划中…"), 0.),
        (
            "r04-running",
            "running",
            pick("2/3 · Vestra Player: Mute", "2/3 · Vestra Player: 静音"),
            2. / 3.,
        ),
        (
            "r04-single",
            "running",
            pick("Vestra Player: Mute", "Vestra Player: 静音"),
            1.,
        ),
        (
            "r05-done",
            "done",
            pick(
                "Done · Reset: Picture, Vestra Player: Mute, Export: Export Last 30 Seconds",
                "完成 · 重置: 画面、Vestra Player: 静音、导出: 导出最后 30 秒",
            ),
            1.,
        ),
        (
            "r05-done-one",
            "done",
            pick("Done · Vestra Player: Mute", "完成 · Vestra Player: 静音"),
            1.,
        ),
        (
            "r06-failed",
            "failed",
            pick(
                "Step 2 failed: Nothing to capture",
                "第 2 步失败：没有可截取的画面",
            ),
            1.,
        ),
        (
            "r09-stopped",
            "note",
            pick("Stopped: nothing was run", "已停止：没有运行任何命令"),
            0.,
        ),
        (
            "r10-sentence",
            "note",
            pick(
                "No command sets an exact volume. Ask for Volume Up or Volume Down, or drag the volume slider in the bar.",
                "没有命令可以把音量设为某个具体数值。可以让我执行“增大音量”或“减小音量”，或者拖动底栏的音量滑块。",
            ),
            0.,
        ),
    ];
    let source = r#"
import { LeadStripView, LeadStrip } from "lead-strip.slint";
import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    in property <image> backdrop;
    in property <LeadStrip> s;
    background: Theme.bg-video;
    Image {
        x: 0px; y: 0px;
        width: root.width;
        height: root.width * root.backdrop.height / max(1, root.backdrop.width);
        source: root.backdrop;
        image-fit: fill;
    }
    LeadStripView {
        x: (root.width - self.width) / 2; y: 16px;
        room: root.width - 16px;
        strip: root.s;
        still: true;
    }
}"#;
    let backdrop = std::env::var_os("VESTRA_STRIP_BACKDROP")
        .map(|path| slint::Image::load_from_path(std::path::Path::new(&path)).unwrap());
    let (width, height, scale) = (1100., 80., 2.);
    let prefix = if zh { "zh-" } else { "" };
    for palette in ["vestra", "vestra-light"] {
        let theme = if palette == "vestra" { "dark" } else { "light" };
        let mut sheet: Vec<image::RgbaImage> = Vec::new();
        for (name, kind, text, progress) in &states {
            let w = build(source, width, height);
            crate::ui::theme::apply_fonts(&w).unwrap();
            crate::ui::theme::apply(&w, palette).unwrap();
            if let Some(image) = &backdrop {
                w.set_property("backdrop", Value::Image(image.clone()))
                    .unwrap();
            }
            set_strip(&w, strip_value(kind, text, *progress, true));
            let picture = render(&w, scale, width, height);
            picture
                .save(out.join(format!("{prefix}{name}-{theme}.png")))
                .unwrap();
            sheet.push(picture);
            w.hide().unwrap();
        }
        let (pw, ph) = (sheet[0].width(), sheet[0].height());
        let mut all = image::RgbaImage::new(pw, ph * sheet.len() as u32);
        for (i, picture) in sheet.iter().enumerate() {
            image::imageops::replace(&mut all, picture, 0, i64::from(ph) * i as i64);
        }
        all.save(out.join(format!("{prefix}sheet-{theme}.png")))
            .unwrap();
    }
}

/// What the strip's sweep costs a CPU-drawn window (the frame-rate rule: the
/// interface keeps the display rate on CPU rendering too): the window over
/// the spec's video still at 1100×700, 2560×1600 and 3840×2160, drawn by
/// Slint's software renderer at 60 Hz pacing for 4 s, every frame whole
/// (as a renderer without partial repaint) and only the changed region.
/// Prints draw times; `VESTRA_STRIP_BENCH=1`, `VESTRA_STRIP_BACKDROP=<png>`.
#[test]
#[ignore = "measures the status strip's sweep: VESTRA_STRIP_BENCH=1"]
fn lead_strip_sweep_frame_cost() {
    if std::env::var_os("VESTRA_STRIP_BENCH").is_none() {
        return;
    }
    let backdrop = std::env::var_os("VESTRA_STRIP_BACKDROP").map(PathBuf::from);
    for (width, height) in [(1100u32, 700u32), (2560, 1600), (3840, 2160)] {
        for reused in [false, true] {
            let backdrop = backdrop.clone();
            let line = std::thread::spawn(move || sweep_cost(width, height, reused, backdrop))
                .join()
                .unwrap();
            diag!("lead strip sweep: {line}");
        }
    }
}

/// One measurement on a thread of its own (its own platform and window).
fn sweep_cost(width: u32, height: u32, reused: bool, backdrop: Option<PathBuf>) -> String {
    use std::time::{Duration, Instant};
    let window = MinimalSoftwareWindow::new(if reused {
        RepaintBufferType::ReusedBuffer
    } else {
        RepaintBufferType::NewBuffer
    });
    slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
    let mut source = r#"
import { LeadStripView, LeadStrip } from "lead-strip.slint";
import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    in property <image> backdrop;
    in property <LeadStrip> s;
    background: Theme.bg-video;
    Image { x: 0px; y: 0px; width: root.width; height: root.height; source: root.backdrop; image-fit: cover; }
    LeadStripView { x: (root.width - self.width) / 2; y: 16px; room: root.width - 16px; strip: root.s; }
}"#
    .to_owned();
    source.push('\n');
    source.push_str(&crate::ui::theme::export_line());
    let path: PathBuf = crate::ui::view::ui_dir().join("widgets-test.slint");
    let result = pollster::block_on(crate::ui::view::compiler().build_from_source(source, path));
    let w = result.component("W").unwrap().create().unwrap();
    w.show().unwrap();
    w.window().set_size(slint::PhysicalSize::new(width, height));
    if let Some(path) = backdrop {
        w.set_property(
            "backdrop",
            Value::Image(slint::Image::load_from_path(&path).unwrap()),
        )
        .unwrap();
    }
    let mut buffer = vec![PremultipliedRgbaColor::default(); width as usize * height as usize];
    let mut frames = |kind: &str, seconds: f64| {
        w.set_property("s", strip_value(kind, "Planning…", 0.5, true))
            .unwrap();
        let period = Duration::from_micros(16_667);
        let start = Instant::now();
        let mut times = Vec::new();
        let mut ticks = 0;
        while start.elapsed().as_secs_f64() < seconds {
            let due = start + period * ticks;
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            ticks += 1;
            let began = Instant::now();
            slint::platform::update_timers_and_animations();
            let drawn = window.draw_if_needed(|renderer| {
                renderer.render(&mut buffer, width as usize);
            });
            if drawn {
                times.push(began.elapsed().as_secs_f64() * 1000.);
            }
        }
        // The first draws of a state settle it (the fade, the first layout).
        let steady: Vec<f64> = times.iter().copied().skip(3).collect();
        (ticks, times.len(), steady)
    };
    // Warm up (glyphs, the image), then the sweep, then a step (static).
    let _ = frames("working", 0.5);
    let (ticks, draws, mut steady) = frames("working", 4.);
    let (_, still_draws, _) = frames("running", 1.);
    steady.sort_by(f64::total_cmp);
    let avg = steady.iter().sum::<f64>() / steady.len().max(1) as f64;
    let at = |q: f64| {
        steady
            .get(((steady.len() as f64 - 1.) * q).round() as usize)
            .copied()
            .unwrap_or(0.)
    };
    w.hide().unwrap();
    format!(
        "{width}x{height} {}: {draws} draws in {ticks} 60 Hz ticks while sweeping, draw avg {avg:.2} ms, p95 {:.2} ms, max {:.2} ms; a step (static) drew {still_draws} times in 1 s",
        if reused {
            "changed region"
        } else {
            "whole frame"
        },
        at(0.95),
        at(1.),
    )
}

/// The approval sheet (ui/review-sheet.slint, host/review_sheet.rs) in a
/// window: its answers and its panel's geometry.
const SHEET: &str = r#"
import { ReviewSheetView, ReviewSheet } from "review-sheet.slint";
import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    in property <image> backdrop;
    in property <ReviewSheet> s;
    in-out property <string> last;
    out property <length> px: v.panel-x;
    out property <length> py: v.panel-y;
    out property <length> pw: v.panel-width;
    out property <length> ph: v.panel-height;
    out property <bool> keys: v.has-keys;
    out property <length> run-x: v.run-x;
    out property <length> cancel-x: v.cancel-x;
    out property <length> buttons-y: v.buttons-y;
    out property <bool> remember-shown: v.remember-shown;
    in-out property <bool> remember-checked;
    out property <length> remember-x: v.remember-x;
    out property <length> remember-y: v.remember-y;
    background: Theme.bg-video;
    Image {
        x: 0px; y: 0px;
        width: root.width;
        height: root.width * root.backdrop.height / max(1, root.backdrop.width);
        source: root.backdrop;
        image-fit: fill;
    }
    v := ReviewSheetView {
        width: root.width; height: root.height;
        sheet: root.s;
        remember-checked: root.remember-checked;
        action(name) => { root.last = name; }
    }
}"#;

/// One step of a sheet: label, plugin, reason, asks, built in.
fn sheet_step(
    label: &str,
    plugin: &str,
    reason: &str,
    asks: bool,
    builtin: bool,
) -> crate::host::review_sheet::Step {
    crate::host::review_sheet::Step {
        label: label.into(),
        plugin: plugin.into(),
        reason: reason.into(),
        asks,
        builtin,
    }
}

/// A `ReviewSheet` value as the host writes it.
fn sheet_value(title: &str, query: &str, steps: &[crate::host::review_sheet::Step]) -> Value {
    sheet_value_remembering(title, query, steps, &[])
}

/// [`sheet_value`] offering "Don't ask again" for the plugins `remember`.
fn sheet_value_remembering(
    title: &str,
    query: &str,
    steps: &[crate::host::review_sheet::Step],
    remember: &[&str],
) -> Value {
    let remember: Vec<String> = remember.iter().map(|s| (*s).to_owned()).collect();
    crate::ui::ui_values::from_json(&crate::host::review_sheet::sheet(
        title,
        "ai",
        query,
        steps,
        &crate::host::review_sheet::remember_text(&remember),
        ["Enter", "Esc"],
    ))
    .unwrap()
}

/// §4.3: the palette's place and width (520px, at most the window less
/// 16px); 72px of request box and question, 28px a step, the note when a
/// step is not built in, the 44px footer.
#[test]
fn the_review_sheet_sizes_to_its_steps() {
    let w = build(SHEET, 1100., 600.);
    let reset = sheet_step("Reset: Picture", "Video", "", true, true);
    w.set_property(
        "s",
        sheet_value(
            "Ask Vestra",
            "reset the picture",
            std::slice::from_ref(&reset),
        ),
    )
    .unwrap();
    settle();
    assert_eq!(
        (number(&w, "px"), number(&w, "py"), number(&w, "pw")),
        (290., 16., 520.)
    );
    assert_eq!(number(&w, "ph"), 72. + 28. + 44.);
    let export = sheet_step(
        "Export: Export Last 30 Seconds",
        "Clip Export",
        "Not built in",
        true,
        false,
    );
    let mute = sheet_step("Vestra Player: Mute", "Vestra Player", "", false, true);
    w.set_property(
        "s",
        sheet_value("Ask Vestra", "reset, mute, export", &[reset, mute, export]),
    )
    .unwrap();
    settle();
    let height = number(&w, "ph");
    // One line of note in 520px, and 8px around it.
    assert!(
        height > 72. + 84. + 44. + 8. && height < 72. + 84. + 44. + 30.,
        "{height}"
    );
    let narrow = build(SHEET, 300., 600.);
    narrow
        .set_property(
            "s",
            sheet_value(
                "Ask Vestra",
                "x",
                &[sheet_step("A: B", "C", "", true, true)],
            ),
        )
        .unwrap();
    settle();
    assert_eq!(number(&narrow, "pw"), 284.);
}

/// It takes the keys: Enter runs, Esc does not, a held-down key's repeats
/// are nothing (the Enter that sent the request); its buttons answer too.
#[test]
fn the_review_sheet_answers_enter_esc_and_its_buttons_but_not_a_repeat() {
    let w = build(SHEET, 1100., 600.);
    w.set_property(
        "s",
        sheet_value(
            "Ask Vestra",
            "reset the picture",
            &[sheet_step("Reset: Picture", "Video", "", true, true)],
        ),
    )
    .unwrap();
    settle();
    assert!(boolean(&w, "keys"));
    w.window().dispatch_event(WindowEvent::KeyPressRepeated {
        text: Key::Return.into(),
    });
    settle();
    assert_eq!(string(&w, "last"), "");
    key(&w, Key::Return);
    assert_eq!(string(&w, "last"), "run");
    key(&w, Key::Escape);
    assert_eq!(string(&w, "last"), "decline");
    let (x, y, width, height) = (
        number(&w, "px") as f32,
        number(&w, "py") as f32,
        number(&w, "pw") as f32,
        number(&w, "ph") as f32,
    );
    let (run, cancel, buttons) = (
        number(&w, "run-x") as f32,
        number(&w, "cancel-x") as f32,
        number(&w, "buttons-y") as f32,
    );
    // Confirm first, both at the footer's right, inside the panel.
    assert!(x + width / 2. < run && run < cancel && cancel < x + width);
    assert!(y + height - 38. < buttons && buttons < y + height);
    click(&w, run, buttons);
    assert_eq!(string(&w, "last"), "run");
    click(&w, cancel, buttons);
    assert_eq!(string(&w, "last"), "decline");
    // A press on the panel elsewhere answers nothing.
    w.set_property("last", Value::String("".into())).unwrap();
    click(&w, x + 40., y + 60.);
    assert_eq!(string(&w, "last"), "");
    // No checkbox here: none shown, and Ctrl+Enter is a plain Run.
    assert!(!boolean(&w, "remember-shown"));
    ctrl_enter(&w);
    assert_eq!(string(&w, "last"), "run");
    assert_eq!(number(&w, "ph"), 72. + 28. + 44.);
}

/// Ctrl+Enter: Control held around an Enter.
fn ctrl_enter(w: &ComponentInstance) {
    let window = w.window();
    window.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(w, Key::Return);
    window.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    settle();
}

/// H4 (§4.3, §4.6): "Don't ask again" at the footer's left, drawn as the
/// host says (it owns the box and its guard): a click asks it to toggle,
/// Run and Enter stay `run` (the host remembers when checked); Space does
/// nothing; Ctrl+Enter runs and remembers; the footer keeps the palette's
/// 44px for one line and grows when the text wraps.
#[test]
fn the_review_sheet_offers_dont_ask_again_by_click_or_ctrl_enter() {
    let w = build(SHEET, 1100., 600.);
    let export = sheet_step(
        "Export: Export Last 30 Seconds",
        "Clip Export",
        "Not built in",
        true,
        false,
    );
    w.set_property(
        "s",
        sheet_value_remembering(
            "Ask Vestra",
            "export the last 30 s",
            std::slice::from_ref(&export),
            &["Clip Export"],
        ),
    )
    .unwrap();
    settle();
    assert!(boolean(&w, "remember-shown"));
    assert!(!boolean(&w, "remember-checked"));
    let one_line = number(&w, "ph");
    let (x, y) = (number(&w, "px"), number(&w, "py"));
    let (bx, by) = (number(&w, "remember-x"), number(&w, "remember-y"));
    // At the footer's left, level with the buttons.
    assert!(bx > x && bx < x + 40., "{bx}");
    assert!((by - number(&w, "buttons-y")).abs() < 2., "{by}");
    assert!(by > y + one_line - 38.);
    // Enter with the box unchecked: a plain Run; Ctrl+Enter remembers.
    key(&w, Key::Return);
    assert_eq!(string(&w, "last"), "run");
    ctrl_enter(&w);
    assert_eq!(string(&w, "last"), "run-remember");
    // Space is not the checkbox's (the keys stay on the sheet).
    w.set_property("last", Value::String("".into())).unwrap();
    key(&w, Key::Space);
    assert_eq!(string(&w, "last"), "");
    assert!(boolean(&w, "keys"));
    // A click asks the host (which owns the box and its guard); the view
    // does not check it by itself.
    click(&w, bx as f32, by as f32);
    assert_eq!(string(&w, "last"), "toggle-remember");
    assert!(!boolean(&w, "remember-checked"));
    assert!(
        boolean(&w, "keys"),
        "the click leaves the keys on the sheet"
    );
    // Checked by the host: Enter and Run stay "run" (the host remembers).
    w.set_property("remember-checked", true.into()).unwrap();
    settle();
    key(&w, Key::Return);
    assert_eq!(string(&w, "last"), "run");
    click(&w, bx as f32, by as f32);
    assert_eq!(string(&w, "last"), "toggle-remember");
    click(
        &w,
        number(&w, "run-x") as f32,
        number(&w, "buttons-y") as f32,
    );
    assert_eq!(string(&w, "last"), "run");
    key(&w, Key::Escape);
    assert_eq!(string(&w, "last"), "decline");
    // Two long names wrap: the footer grows, the buttons stay inside.
    let long = "A Plugin With A Very Long Shown Name Indeed";
    w.set_property(
        "s",
        sheet_value_remembering(
            "Ask Vestra",
            "x",
            std::slice::from_ref(&export),
            &[long, long],
        ),
    )
    .unwrap();
    settle();
    let wrapped = number(&w, "ph");
    assert!(wrapped > one_line, "{wrapped} > {one_line}");
    let (y, buttons) = (number(&w, "py"), number(&w, "buttons-y"));
    assert!(buttons < y + wrapped - 8., "{buttons}");
}

/// The sheet as the spec's renders show it (PROPOSAL §4.3,
/// proposed/r03-approve-*.png and detail-*.png): the top 1100×400 of the
/// window over the spec's video still at 2×, both palettes; one step (a
/// destructive one; one of a plugin not built in), several steps with one
/// not built in, Close, and eight steps: `VESTRA_SHEET_SHOTS=<dir>`,
/// `VESTRA_SHEET_LANG=en|zh-Hans` (one language per process: run this test
/// alone), `VESTRA_SHEET_BACKDROP=<png>` (optional).
#[test]
#[ignore = "draws the approval sheet: VESTRA_SHEET_SHOTS=<dir>"]
fn render_review_sheet_states() {
    let Some(out) = std::env::var_os("VESTRA_SHEET_SHOTS") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let lang = std::env::var("VESTRA_SHEET_LANG").unwrap_or_else(|_| "en".into());
    let locale = crate::i18n::LOCALES
        .iter()
        .copied()
        .find(|l| *l == lang)
        .expect("a player language");
    headless();
    crate::i18n::init(locale);
    crate::ui::ui_fonts::install(locale);
    let zh = locale == "zh-Hans";
    let pick = |en: &'static str, zh_text: &'static str| if zh { zh_text } else { en };
    let not_built_in = crate::i18n::tr("Not built in");
    let reset = sheet_step(
        pick("Reset: Picture", "重置: 画面"),
        pick("Video", "视频"),
        "",
        true,
        true,
    );
    let mute = sheet_step(
        pick("Vestra Player: Mute", "Vestra Player: 静音"),
        "Vestra Player",
        "",
        false,
        true,
    );
    let export = sheet_step(
        pick("Export: Export Last 30 Seconds", "导出: 导出最后 30 秒"),
        "Clip Export",
        &not_built_in,
        true,
        false,
    );
    let close = sheet_step(
        pick("Vestra Player: Close", "Vestra Player: 关闭"),
        "Vestra Player",
        &crate::i18n::tr("Closes the video"),
        true,
        true,
    );
    let fullscreen = sheet_step(
        pick("Vestra Player: Fullscreen", "Vestra Player: 全屏"),
        "Vestra Player",
        "",
        false,
        true,
    );
    let request = pick(
        "reset the picture, mute, export the last 30 s",
        "重置画面，静音，导出最后 30 秒",
    );
    let frame = sheet_step(
        pick("Auto Frame: Frame the Speaker", "Auto Frame: 跟随说话人"),
        "Auto Frame",
        &not_built_in,
        true,
        false,
    );
    let eight: Vec<_> = [
        reset.clone(),
        mute.clone(),
        fullscreen.clone(),
        sheet_step(
            pick("Audio: Volume Up", "音频: 增大音量"),
            pick("Audio", "音频"),
            "",
            false,
            true,
        ),
        frame.clone(),
        export.clone(),
        sheet_step(
            pick("Playlist: Remove All", "播放列表: 全部移除"),
            pick("Playlist", "播放列表"),
            "",
            true,
            true,
        ),
        close.clone(),
    ]
    .into();
    // (name, request, steps, "Don't ask again" plugins as the host offers
    // them: none with a destructive step; checked as the host sets it).
    type State<'a> = (
        &'a str,
        &'a str,
        Vec<crate::host::review_sheet::Step>,
        Vec<&'a str>,
        bool,
    );
    let states: Vec<State> = vec![
        (
            "r03-plan",
            request,
            vec![reset.clone(), mute.clone(), export.clone()],
            vec![],
            false,
        ),
        (
            "r03-one-destructive",
            pick("reset the picture", "重置画面"),
            vec![reset.clone()],
            vec![],
            false,
        ),
        (
            "r03-one-not-built-in",
            pick("export the last 30 s", "导出最后 30 秒"),
            vec![export.clone()],
            vec!["Clip Export"],
            false,
        ),
        (
            "r04-remember-one-checked",
            pick("mute and export the last 30 s", "静音并导出最后 30 秒"),
            vec![mute.clone(), export.clone()],
            vec!["Clip Export"],
            true,
        ),
        (
            "r04-remember-two",
            pick(
                "mute, follow the speaker and export the last 30 s",
                "静音，跟随说话人，导出最后 30 秒",
            ),
            vec![mute.clone(), frame.clone(), export.clone()],
            vec!["Auto Frame", "Clip Export"],
            false,
        ),
        (
            "r03-close",
            pick("mute and close the video", "静音并关闭视频"),
            vec![mute.clone(), close.clone()],
            vec![],
            false,
        ),
        (
            "r03-eight",
            pick(
                "reset everything, louder, follow the speaker, export, clear the playlist and close",
                "全部重置，大声点，跟随说话人，导出，清空播放列表然后关闭",
            ),
            eight,
            vec![],
            false,
        ),
    ];
    let backdrop = std::env::var_os("VESTRA_SHEET_BACKDROP")
        .map(|path| slint::Image::load_from_path(std::path::Path::new(&path)).unwrap());
    let (width, height, scale) = (1100., 400., 2.);
    let prefix = if zh { "zh-" } else { "" };
    let title = pick("Ask Vestra", "问 Vestra");
    for palette in ["vestra", "vestra-light"] {
        let theme = if palette == "vestra" { "dark" } else { "light" };
        for (name, query, steps, remember, checked) in &states {
            let w = build(SHEET, width, height);
            crate::ui::theme::apply_fonts(&w).unwrap();
            crate::ui::theme::apply(&w, palette).unwrap();
            if let Some(image) = &backdrop {
                w.set_property("backdrop", Value::Image(image.clone()))
                    .unwrap();
            }
            w.set_property("s", sheet_value_remembering(title, query, steps, remember))
                .unwrap();
            settle();
            if *checked {
                // As the host sets it after a click.
                w.set_property("remember-checked", true.into()).unwrap();
                settle();
            }
            render(&w, scale, width, height)
                .save(out.join(format!("{prefix}{name}-{theme}.png")))
                .unwrap();
            w.hide().unwrap();
        }
    }
}

/// A main-window `dialogs.confirm` on the sheet (H5) as the host writes it:
/// with the request box when `request` is (title, request).
fn confirm_value(
    request: Option<(&str, &str)>,
    message: &str,
    detail: Option<&str>,
    confirm: &str,
    cancel: &str,
    checkbox: Option<&str>,
) -> Value {
    let options = crate::ui::confirm::Options {
        message: message.into(),
        detail: detail.map(str::to_owned),
        confirm: confirm.into(),
        cancel: cancel.into(),
        destructive: checkbox.is_some(),
        checkbox: checkbox.map(str::to_owned),
    };
    crate::ui::ui_values::from_json(&crate::host::review_sheet::confirm_sheet(
        request.map(|(title, query)| (title, "ai", query)),
        &options,
        ["Enter", "Esc"],
    ))
    .unwrap()
}

/// Ask Vestra's consent text (workshop `ask.ts`), English and Chinese.
const CONSENT_DETAIL: [&str; 2] = [
    "What you type in the command palette, the player's command list (names, menus, keys and whether each is on or available) and whether something is open and playing go to this provider with your API key, as in the chat. File names, playlists, history, track and chapter titles and subtitles are not sent. The provider charges your account.",
    "你在命令面板中输入的内容、播放器的命令列表（名称、菜单、快捷键，以及各命令是否开启或可用）以及是否有内容打开并在播放，会和对话一样，用你的 API 密钥发送给该服务商。文件名、播放列表、历史记录、轨道和章节标题以及字幕不会发送。费用由服务商向你的账户收取。",
];

/// H5 (§4.4): a confirmation at the palette's place: without the request
/// box, the 28px message and the 44px footer; with it, 38px more, and the
/// detail grows the panel; the caller's checkbox at the footer's left;
/// Enter and Esc answer, its buttons too, a repeat does not.
#[test]
fn a_confirmation_sheet_sizes_to_its_texts_and_answers() {
    let w = build(SHEET, 1100., 600.);
    w.set_property(
        "s",
        confirm_value(
            None,
            "Replace the current list with Films.m3u?",
            None,
            "Replace",
            "Cancel",
            None,
        ),
    )
    .unwrap();
    settle();
    assert_eq!(
        (number(&w, "px"), number(&w, "py"), number(&w, "pw")),
        (290., 16., 520.)
    );
    assert_eq!(number(&w, "ph"), 6. + 28. + 44.);
    assert!(!boolean(&w, "remember-shown"));
    assert!(boolean(&w, "keys"));
    w.window().dispatch_event(WindowEvent::KeyPressRepeated {
        text: Key::Return.into(),
    });
    settle();
    assert_eq!(string(&w, "last"), "");
    key(&w, Key::Return);
    assert_eq!(string(&w, "last"), "run");
    key(&w, Key::Escape);
    assert_eq!(string(&w, "last"), "decline");
    click(
        &w,
        number(&w, "run-x") as f32,
        number(&w, "buttons-y") as f32,
    );
    assert_eq!(string(&w, "last"), "run");
    click(
        &w,
        number(&w, "cancel-x") as f32,
        number(&w, "buttons-y") as f32,
    );
    assert_eq!(string(&w, "last"), "decline");
    // The request box and a four-line detail.
    w.set_property(
        "s",
        confirm_value(
            Some(("Ask Vestra", "mute")),
            "Ask Vestra sends your request to MiniMax",
            Some(CONSENT_DETAIL[0]),
            "Continue",
            "Cancel",
            None,
        ),
    )
    .unwrap();
    settle();
    let height = number(&w, "ph");
    assert!(
        height > 44. + 28. + 44. + 40. && height < 44. + 28. + 44. + 100.,
        "{height}"
    );
    // The caller's checkbox at the footer's left; a click asks the host.
    w.set_property(
        "s",
        confirm_value(
            None,
            "Remove all 12 items from the playlist?",
            Some("The media that is playing continues to play."),
            "Remove All",
            "Cancel",
            Some("Don't ask again"),
        ),
    )
    .unwrap();
    settle();
    assert!(boolean(&w, "remember-shown"));
    let (bx, by) = (number(&w, "remember-x"), number(&w, "remember-y"));
    assert!(bx > number(&w, "px") && bx < number(&w, "px") + 40., "{bx}");
    click(&w, bx as f32, by as f32);
    assert_eq!(string(&w, "last"), "toggle-remember");
    assert!(!boolean(&w, "remember-checked"));
}

/// The main window's confirmations on the sheet (H5; PROPOSAL §4.4,
/// proposed/r07-consent-*.png): Ask Vestra's first-use consent (with the
/// request box), a plain one (Open List's Replace) and a destructive one
/// with a checkbox (the playlist's Remove All), both palettes, at 2×:
/// `VESTRA_SHEET_SHOTS=<dir>`, `VESTRA_SHEET_LANG=en|zh-Hans` (one language
/// per process: run this test alone), `VESTRA_SHEET_BACKDROP=<png>`.
#[test]
#[ignore = "draws the confirmation sheet: VESTRA_SHEET_SHOTS=<dir>"]
fn render_confirm_sheet_states() {
    let Some(out) = std::env::var_os("VESTRA_SHEET_SHOTS") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let lang = std::env::var("VESTRA_SHEET_LANG").unwrap_or_else(|_| "en".into());
    let locale = crate::i18n::LOCALES
        .iter()
        .copied()
        .find(|l| *l == lang)
        .expect("a player language");
    headless();
    crate::i18n::init(locale);
    crate::ui::ui_fonts::install(locale);
    let zh = locale == "zh-Hans";
    let pick = |en: &'static str, zh_text: &'static str| if zh { zh_text } else { en };
    let cancel = crate::i18n::tr("Cancel");
    let remove = (
        pick(
            "Remove all 12 items from the playlist?",
            "移除播放列表中的全部 12 项？",
        ),
        Some(pick(
            "The media that is playing continues to play.",
            "正在播放的媒体继续播放。",
        )),
        pick("Remove All", "移除全部"),
        Some(pick("Don't ask again", "不再询问")),
    );
    // (name, request, (message, detail, confirm, checkbox), checked)
    type Texts<'a> = (&'a str, Option<&'a str>, &'a str, Option<&'a str>);
    type State<'a> = (&'a str, Option<(&'a str, &'a str)>, Texts<'a>, bool);
    let states: Vec<State> = vec![
        (
            "r07-consent",
            Some((
                pick("Ask Vestra", "问 Vestra"),
                pick(
                    "reset the picture, mute, export the last 30 s",
                    "重置画面，静音，导出最后 30 秒",
                ),
            )),
            (
                pick(
                    "Ask Vestra sends your request to MiniMax",
                    "“问 Vestra”会把你的请求发送给 MiniMax",
                ),
                Some(CONSENT_DETAIL[usize::from(zh)]),
                pick("Continue", "继续"),
                None,
            ),
            false,
        ),
        (
            "h5-plain-replace-list",
            None,
            (
                pick(
                    "Replace the current list with Films.m3u?",
                    "用 Films.m3u 替换当前列表？",
                ),
                None,
                pick("Replace", "替换"),
                None,
            ),
            false,
        ),
        ("h5-destructive-checkbox", None, remove, false),
        ("h5-destructive-checked", None, remove, true),
    ];
    let backdrop = std::env::var_os("VESTRA_SHEET_BACKDROP")
        .map(|path| slint::Image::load_from_path(std::path::Path::new(&path)).unwrap());
    let (width, height, scale) = (1100., 400., 2.);
    let prefix = if zh { "zh-" } else { "" };
    for palette in ["vestra", "vestra-light"] {
        let theme = if palette == "vestra" { "dark" } else { "light" };
        for (name, request, (message, detail, confirm, checkbox), checked) in &states {
            let w = build(SHEET, width, height);
            crate::ui::theme::apply_fonts(&w).unwrap();
            crate::ui::theme::apply(&w, palette).unwrap();
            if let Some(image) = &backdrop {
                w.set_property("backdrop", Value::Image(image.clone()))
                    .unwrap();
            }
            w.set_property(
                "s",
                confirm_value(*request, message, *detail, confirm, &cancel, *checkbox),
            )
            .unwrap();
            settle();
            if *checked {
                w.set_property("remember-checked", true.into()).unwrap();
                settle();
            }
            render(&w, scale, width, height)
                .save(out.join(format!("{prefix}{name}-{theme}.png")))
                .unwrap();
            w.hide().unwrap();
        }
    }
}
