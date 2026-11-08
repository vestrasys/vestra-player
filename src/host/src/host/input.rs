//! Keys and pointer input of the windows: bindings, injected keys and
//! modifier releases.
use super::*;

/// Slint reports ⌘ as `control` and ⌃ as `meta` on Apple platforms.
pub(crate) fn slint_mods(control: bool, meta: bool) -> (bool, bool) {
    if platform::SLINT_CONTROL_IS_COMMAND {
        (meta, control)
    } else {
        (control, meta)
    }
}

/// Record a key press's physical key for the `key-input` callback that Slint
/// runs right after this hook. Returns the input of a dead key with Option
/// (no text, so Slint never reports it) and whether it is an auto-repeat.
pub(crate) fn observe_key(
    input: &Rc<RefCell<InputShared>>,
    modifiers: &std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
    event: &slint::winit_030::winit::event::WindowEvent,
) -> Option<(bindings::Input, bool)> {
    use slint::winit_030::winit::{
        event::{ElementState, WindowEvent},
        keyboard::{Key, PhysicalKey},
        platform::modifier_supplement::KeyEventExtModifierSupplement,
    };
    match event {
        WindowEvent::ModifiersChanged(state) => {
            // winit sends this from the key event's own flags too (before the
            // KeyboardInput), so it is right even without modifier key events.
            let (before, after) = (modifiers.get(), state.state());
            modifiers.set(after);
            // Slint follows modifiers only through their own key events; a
            // release it never saw (synthesized input) would leave Option or
            // Shift held for later pointer and key events. Release them.
            let mut shared = input.borrow_mut();
            for (was, now, keys) in [
                (before.alt_key(), after.alt_key(), ['\u{12}', '\u{13}']),
                (before.shift_key(), after.shift_key(), ['\u{10}', '\u{15}']),
                (
                    before.control_key(),
                    after.control_key(),
                    ['\u{11}', '\u{16}'],
                ),
                (before.super_key(), after.super_key(), ['\u{17}', '\u{18}']),
            ] {
                if was && !now {
                    shared.released.extend(keys);
                }
            }
            if debug_keys() {
                diag!("keys: winit modifiers {:?}", state.state());
            }
            None
        }
        WindowEvent::KeyboardInput {
            event,
            is_synthetic,
            ..
        } => {
            if debug_keys() {
                diag!(
                    "keys: winit {:?} synthetic={is_synthetic} physical={:?} logical={:?} text={:?} repeat={} modifiers={:?}",
                    event.state,
                    event.physical_key,
                    event.logical_key,
                    event.text,
                    event.repeat,
                    modifiers.get()
                );
            }
            if *is_synthetic || event.state != ElementState::Pressed {
                return None;
            }
            if !event.repeat {
                input_trace::begin(&format!("winit key {:?} down", event.physical_key));
            }
            let PhysicalKey::Code(code) = event.physical_key else {
                input.borrow_mut().physical = None;
                return None;
            };
            let base = match event.key_without_modifiers() {
                Key::Character(text) if text.chars().count() == 1 => text.chars().next(),
                _ => None,
            };
            let state = modifiers.get();
            let physical = bindings::Physical {
                code: format!("{code:?}"),
                base,
                mods: Some(bindings::Mods {
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    shift: state.shift_key(),
                    cmd: state.super_key(),
                }),
            };
            let dead = (matches!(event.logical_key, Key::Dead(_)) && state.alt_key())
                .then(|| bindings::from_key("", false, false, false, false, Some(&physical)))
                .flatten();
            if debug_keys() && matches!(event.logical_key, Key::Dead(_)) {
                diag!("keys: dead key → {dead:?}");
            }
            input.borrow_mut().physical = Some(physical);
            dead.map(|input| (input, event.repeat))
        }
        WindowEvent::MouseInput { state, button, .. } => {
            match state {
                ElementState::Pressed => {
                    input_trace::begin(&format!("winit mouse {button:?} down"))
                }
                ElementState::Released => input_trace::mark(&format!("winit mouse {button:?} up")),
            }
            None
        }
        _ => None,
    }
}

/// The side buttons (back = `Mouse:X1`, forward = `Mouse:X2`) are
/// window-level, as in browsers (host 0.6.0): a press resolves against the
/// binding table wherever the pointer is in the window (video, bottom bar,
/// sidebar, a floating panel), except over an open menu. Before, only the
/// video area's `TouchArea` reported them, so over the bar they did
/// nothing. No element handles them today; one that should would need its
/// own rule here. The event still goes on to Slint (focus and activity as
/// before); no `TouchArea` acts on these buttons, so they fire once.
pub(crate) fn observe_side_button(
    input: &Rc<RefCell<InputShared>>,
    modifiers: &std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
    events: &mpsc::Sender<Event>,
    generation: u64,
    event: &slint::winit_030::winit::event::WindowEvent,
    menu_open: impl FnOnce() -> bool,
) -> bool {
    use slint::winit_030::winit::event::WindowEvent;
    let WindowEvent::MouseInput { state, button, .. } = event else {
        return false;
    };
    side_button_press(*state, *button, modifiers.get(), menu_open)
        .is_some_and(|event| fire(input, events, generation, &event, false, None))
}

/// What a mouse button event over a window binds, if it is a side button's
/// press: `Mouse:X1` (back) or `Mouse:X2` (forward) with the window's
/// modifiers (winit's control is Ctrl, its super is ⌘ / the Windows key,
/// as for keys). `None` for other buttons, for releases (a side button
/// fires on its press) and while a menu is open (`menu_open` is asked only
/// for a side button's press).
pub(crate) fn side_button_press(
    state: slint::winit_030::winit::event::ElementState,
    button: slint::winit_030::winit::event::MouseButton,
    modifiers: slint::winit_030::winit::keyboard::ModifiersState,
    menu_open: impl FnOnce() -> bool,
) -> Option<bindings::Input> {
    use slint::winit_030::winit::event::{ElementState, MouseButton};
    if state != ElementState::Pressed {
        return None;
    }
    let name = match button {
        MouseButton::Back => "X1",
        MouseButton::Forward => "X2",
        _ => return None,
    };
    if menu_open() {
        return None;
    }
    Some(bindings::Input {
        mods: bindings::Mods {
            ctrl: modifiers.control_key(),
            alt: modifiers.alt_key(),
            shift: modifiers.shift_key(),
            cmd: modifiers.super_key(),
        },
        name: format!("Mouse:{name}"),
    })
}

/// Option + a named key (⌥⌦, ⌥←): macOS delivers it in a form Slint does not
/// route to the focused item, and its input handling then stalls until the
/// next ordinary key. Such a press and release are kept from Slint and handed
/// to it again as plain key events from the host loop, with the press's
/// physical key and window modifiers recorded, so the binding resolves in the
/// focused scope (`Alt+Delete` in the playlist panel) like any other key.
pub(crate) fn inject_option_key(
    input: &Rc<RefCell<InputShared>>,
    modifiers: &std::cell::Cell<slint::winit_030::winit::keyboard::ModifiersState>,
    events: &mpsc::Sender<Event>,
    generation: u64,
    event: &slint::winit_030::winit::event::WindowEvent,
) -> bool {
    use slint::winit_030::winit::{
        event::{ElementState, WindowEvent},
        keyboard::PhysicalKey,
    };
    let WindowEvent::KeyboardInput {
        event,
        is_synthetic: false,
        ..
    } = event
    else {
        return false;
    };
    let PhysicalKey::Code(code) = event.physical_key else {
        return false;
    };
    // ⌥Space types a character (no-break space): Slint gets it as it is.
    let code = format!("{code:?}");
    let Some(key) = bindings::slint_key(&code).filter(|_| code != "Space") else {
        return false;
    };
    if !modifiers.get().alt_key() {
        return false;
    }
    let pressed = event.state == ElementState::Pressed;
    let physical = if pressed {
        input.borrow_mut().physical.take()
    } else {
        None
    };
    let kind = match (pressed, event.repeat) {
        (false, _) => Injected::Released,
        (true, false) => Injected::Pressed,
        (true, true) => Injected::Repeated,
    };
    let _ = events.send(Event::InjectKey(generation, key, kind, physical));
    true
}

/// Hand modifier releases Slint missed to the window, outside winit's dispatch.
pub(crate) fn release_modifiers(
    input: &Rc<RefCell<InputShared>>,
    events: &mpsc::Sender<Event>,
    generation: u64,
) {
    let released = std::mem::take(&mut input.borrow_mut().released);
    if !released.is_empty() {
        let _ = events.send(Event::ModifiersReleased(generation, released));
    }
}

/// The system double-click interval (0.5 s where the system has none).
pub(crate) fn double_click_interval() -> Duration {
    let seconds = platform::double_click_seconds();
    Duration::from_secs_f64(if (0.05..=5.).contains(&seconds) {
        seconds
    } else {
        0.5
    })
}

pub(crate) fn debug_keys() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_KEYS").is_some_and(|v| v == "1"))
}

/// Resolve a bound input on the UI thread; true = bound (Slint stops propagating).
/// Auto-repeats fire only commands that declared `repeat`.
pub(crate) fn fire(
    input: &Rc<RefCell<InputShared>>,
    events: &mpsc::Sender<Event>,
    generation: u64,
    event: &bindings::Input,
    repeat: bool,
    panel: Option<u64>,
) -> bool {
    let shared = input.borrow();
    if shared.blocked {
        return false;
    }
    let target = match shared.table.lookup(event, panel) {
        bindings::Lookup::Fire(target) => target,
        // Only disabled commands hold it: one with a reason says why on the
        // OSD (the host loop's execution reports it); otherwise the key passes.
        bindings::Lookup::Disabled {
            target,
            reason: Some(_),
        } => {
            if !repeat {
                input_trace::mark(&format!(
                    "disabled {} → {}/{}",
                    bindings::text(event),
                    target.owner,
                    target.id
                ));
                let _ = events.send(Event::Binding(generation, target.owner, target.id.clone()));
            }
            return true;
        }
        bindings::Lookup::Disabled { reason: None, .. } | bindings::Lookup::Unbound => {
            return false;
        }
    };
    if !repeat {
        input_trace::mark(&format!(
            "fire {} → {}/{}",
            bindings::text(event),
            target.owner,
            target.id
        ));
    }
    if !repeat || target.repeat {
        let _ = events.send(Event::Binding(generation, target.owner, target.id.clone()));
    }
    true
}

/// The producer capture lets keys bound to host commands through (winit key code names).
#[cfg(feature = "producer-audio")]
pub(crate) fn host_key(input: &Rc<RefCell<InputShared>>, code: &str) -> bool {
    let name = match code {
        "Escape" | "Enter" | "Tab" | "Space" => code.to_owned(),
        "NumpadEnter" => "Enter".into(),
        code if code.starts_with('F') && code[1..].parse::<u8>().is_ok() => code.to_owned(),
        _ => return false,
    };
    input.borrow().table.host_key(&bindings::Input {
        mods: bindings::Mods::default(),
        name,
    })
}

impl Host {
    /// A floating panel forwards keys to the binding table and records each
    /// press's physical key first (see `observe_key`). Its standard window
    /// shortcuts (⌘W…) come before that and never reach the table; the main
    /// window has its own hook (`bind_window_events`) without them.
    pub(crate) fn observe_keys(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
        let input = self.input.clone();
        let events = self.events.clone();
        let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
        let shortcuts = view::WindowShortcuts::default();
        let floating = window.as_weak();
        window.window().on_winit_window_event(move |window, event| {
            if shortcuts.filter(window, event) {
                return EventResult::PreventDefault;
            }
            track_pointer(&input, generation, window, event);
            let _ = observe_key(&input, &modifiers, event);
            release_modifiers(&input, &events, generation);
            // The floating panel window too: its keys reach the global
            // bindings, so do its side buttons (`observe_side_button`).
            observe_side_button(&input, &modifiers, &events, generation, event, || {
                floating.upgrade().is_some_and(|instance| {
                    matches!(instance.get_property("menu-open"), Ok(SValue::Bool(true)))
                })
            });
            if let winit::event::WindowEvent::DroppedFile(path) = event {
                let _ = events.send(Event::Dropped(generation, path.clone()));
            }
            if inject_option_key(&input, &modifiers, &events, generation, event) {
                return EventResult::PreventDefault;
            }
            EventResult::Propagate
        });
    }
    /// Key forwarding: `key-input(scope, text, control, alt, shift, meta, repeat)`.
    pub(crate) fn bind_keys(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("key-input", move |args| {
            let [SValue::String(scope), SValue::String(text), rest @ ..] = args else {
                return SValue::Bool(false);
            };
            let flag = |i: usize| matches!(rest.get(i), Some(SValue::Bool(true)));
            // The press this callback reports was recorded just before by
            // the window's winit hook, in the same event dispatch.
            let physical = input.borrow_mut().physical.take();
            let resolved =
                bindings::from_key(text, flag(0), flag(1), flag(2), flag(3), physical.as_ref());
            // A symbol typed with Option (German ⌥5 = `[`) is tried first:
            // the first candidate with a binding fires.
            let typed = bindings::typed_with_option(text, flag(0), flag(3), physical.as_ref());
            if debug_keys() {
                diag!(
                    "keys: slint scope={:?} text={:?} control={} alt={} shift={} meta={} repeat={} physical={physical:?} → {resolved:?} typed={typed:?} bound={}",
                    scope.as_str(),
                    text.as_str(),
                    flag(0),
                    flag(1),
                    flag(2),
                    flag(3),
                    flag(4),
                    resolved
                        .as_ref()
                        .is_some_and(|event| input.borrow().table.resolve(event, None).is_some())
                );
            }
            let Some(mut event) = resolved else {
                return SValue::Bool(false);
            };
            let panel = {
                let shared = input.borrow();
                match scope.as_str() {
                    "panel:bottom" => shared.bottom,
                    "panel:sidebar" => shared.sidebar,
                    "panel:adjust" => shared.adjust,
                    _ => None,
                }
            };
            if let Some(typed) = typed
                && input.borrow().table.lookup(&typed, panel) != bindings::Lookup::Unbound
            {
                event = typed;
            }
            SValue::Bool(fire(&input, &events, generation, &event, flag(4), panel))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::event::{ElementState, MouseButton};
    use slint::winit_030::winit::keyboard::ModifiersState;

    fn closed() -> bool {
        false
    }

    /// Back is `Mouse:X1`, forward `Mouse:X2`, on their press only; other
    /// buttons and releases are none of them.
    #[test]
    fn side_button_presses_name_x1_and_x2() {
        let none = ModifiersState::empty();
        let press = |button| side_button_press(ElementState::Pressed, button, none, closed);
        assert_eq!(press(MouseButton::Back).unwrap().name, "Mouse:X1");
        assert_eq!(press(MouseButton::Forward).unwrap().name, "Mouse:X2");
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Other(9),
        ] {
            assert_eq!(press(button), None, "{button:?}");
        }
        for button in [MouseButton::Back, MouseButton::Forward] {
            assert_eq!(
                side_button_press(ElementState::Released, button, none, closed),
                None
            );
        }
    }

    /// An open menu keeps them (and is asked only for a side button).
    #[test]
    fn side_buttons_do_nothing_while_a_menu_is_open() {
        let none = ModifiersState::empty();
        for button in [MouseButton::Back, MouseButton::Forward] {
            assert_eq!(
                side_button_press(ElementState::Pressed, button, none, || true),
                None
            );
        }
        let asked = std::cell::Cell::new(false);
        let ask = || {
            asked.set(true);
            true
        };
        assert_eq!(
            side_button_press(ElementState::Pressed, MouseButton::Left, none, ask),
            None
        );
        assert!(!asked.get(), "not asked for the left button");
    }

    /// The window's modifiers: winit's control is Ctrl and its super ⌘ (or
    /// the Windows key), as for keys.
    #[test]
    fn side_buttons_carry_the_window_modifiers() {
        let mods = |modifiers| {
            side_button_press(ElementState::Pressed, MouseButton::Back, modifiers, closed)
                .unwrap()
                .mods
        };
        assert_eq!(mods(ModifiersState::empty()), bindings::Mods::default());
        assert_eq!(
            mods(ModifiersState::CONTROL),
            bindings::Mods {
                ctrl: true,
                ..Default::default()
            }
        );
        assert_eq!(
            mods(ModifiersState::SUPER),
            bindings::Mods {
                cmd: true,
                ..Default::default()
            }
        );
        assert_eq!(
            mods(ModifiersState::SHIFT | ModifiersState::ALT),
            bindings::Mods {
                shift: true,
                alt: true,
                ..Default::default()
            }
        );
        assert_eq!(
            bindings::text(
                &side_button_press(
                    ElementState::Pressed,
                    MouseButton::Forward,
                    ModifiersState::CONTROL,
                    closed
                )
                .unwrap()
            ),
            "Ctrl+Mouse:X2"
        );
    }

    /// With the defaults of core_actions.rs (user setting 2026-11-01: back
    /// is Next, forward Previous), each press fires its command once.
    #[test]
    fn side_buttons_fire_their_binding_once_per_press() {
        let input = Rc::new(RefCell::new(InputShared::default()));
        for (id, declaration) in crate::core_actions::declarations(5.) {
            input
                .borrow_mut()
                .table
                .declare(
                    bindings::HOST,
                    id,
                    "host",
                    &declaration.bindings,
                    false,
                    false,
                )
                .unwrap();
        }
        let (events, received) = mpsc::channel();
        let fired = |received: &mpsc::Receiver<Event>| -> Vec<String> {
            received
                .try_iter()
                .map(|event| match event {
                    Event::Binding(7, bindings::HOST, id) => id,
                    _ => panic!("not a binding of window 7"),
                })
                .collect()
        };
        let press = |button| {
            side_button_press(
                ElementState::Pressed,
                button,
                ModifiersState::empty(),
                closed,
            )
            .is_some_and(|event| fire(&input, &events, 7, &event, false, None))
        };
        assert!(press(MouseButton::Back));
        assert_eq!(fired(&received), ["next"]);
        assert!(press(MouseButton::Forward));
        assert_eq!(fired(&received), ["previous"]);
        // An input block (a modal) holds them like keys.
        input.borrow_mut().blocked = true;
        assert!(!press(MouseButton::Back));
        assert!(fired(&received).is_empty());
    }
}
