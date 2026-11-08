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
    key(&palette, Key::Home);
    let start = inside("after Home");
    assert!(start < 4., "{start}");
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
