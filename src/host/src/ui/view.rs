//! Direct Slint interpreter integration, with primitive property and callback transport.
use crate::host::Event;
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
    crate::process::runtime_paths::get().ui.clone()
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
/// `ui::view::tests::geometry_*` tests fail if the behaviour changes).
pub fn settle_layout(window: &slint::Window) {
    slint::private_unstable_api::re_exports::WindowInner::from_pub(window)
        .ensure_tree_instantiated();
}

/// The component tree a window shows, kept while a replacement is built in
/// the same window (`composition::compile_json_in`, the Options window's
/// `settings_window::create_window`): Slint attaches the new tree as it
/// creates it, so a build that fails after that has to put the old tree
/// back, and a shown component cannot be attached again through the public
/// API. Uses Slint's private `WindowInner` like [`settle_layout`] (recheck
/// on every Slint upgrade; the `ui::view::tests::shell_swap_*` tests fail if
/// the behaviour changes).
pub struct ShownTree {
    tree: slint::private_unstable_api::re_exports::ItemTreeRc,
    /// The item that had the focus when the tree was taken.
    focus: slint::private_unstable_api::re_exports::ItemWeak,
}

impl ShownTree {
    /// The tree `window` shows now, if any, with its focused item.
    pub fn of(window: &slint::Window) -> Option<Self> {
        let inner = slint::private_unstable_api::re_exports::WindowInner::from_pub(window);
        inner.try_component().map(|tree| Self {
            tree,
            focus: inner.focus_item.borrow().clone(),
        })
    }

    /// Whether `window` shows this tree.
    pub fn is_shown_in(&self, window: &slint::Window) -> bool {
        slint::private_unstable_api::re_exports::WindowInner::from_pub(window)
            .try_component()
            .is_some_and(|shown| {
                slint::private_unstable_api::re_exports::VRc::ptr_eq(&shown, &self.tree)
            })
    }

    /// Show this tree in `window` again if a replacement took its place;
    /// whether it did (nothing otherwise: re-attaching would close its
    /// popups and reset its focus), and whether the item that had the focus
    /// has it again.
    ///
    /// Re-attaching clears the window's focus, and the tree's `init` (which
    /// gave it) does not run again: the item focused when the tree was taken
    /// gets it back; without one, the caller gives it. A replacement whose
    /// `show()` failed may hold the window's strong reference: showing the
    /// window again moves it back to this tree (the native window is already
    /// visible; winit returns at once).
    pub fn restore(&self, window: &slint::Window) -> Option<bool> {
        if self.is_shown_in(window) {
            return None;
        }
        let inner = slint::private_unstable_api::re_exports::WindowInner::from_pub(window);
        inner.set_component(&self.tree);
        if window.is_visible() {
            let _ = inner.show();
        }
        let focused = self.focus.upgrade().map(|item| {
            inner.set_focus_item(
                &item,
                true,
                slint::private_unstable_api::re_exports::FocusReason::Programmatic,
            );
        });
        Some(focused.is_some())
    }
}

/// Give the main window's shell the focus on its video area (its `init`
/// does), for [`replace_shell`] after a failed swap.
pub fn focus_video(shell: &ComponentInstance) {
    let _ = shell.invoke("focus-video", &[]);
}

/// Build the next component tree of a window that stays open (the main
/// window's shell, the Options window) and show it, the way browsers and
/// IDEs swap content without touching the top-level window.
///
/// With an `old` component, `build` gets its window and must build the new
/// one in it (`composition::compile_json_in`,
/// `create_with_existing_window`; one built elsewhere is refused); this
/// then shows it there, which only moves the window's strong reference to
/// the new tree: the native window stays visible where it was, at its size,
/// maximized or fullscreen, with its focus and z-order, so nothing hides,
/// fades, moves or resizes. The old component must not be hidden (it shares
/// the window): the caller drops it. If `build` fails, the window shows the
/// old tree again, the item that had the focus focused again; where none
/// had it, `refocus` gives the old component the focus (the main window:
/// its video area).
///
/// Without an old component, `build` gets `None` and shows its own new
/// window.
pub fn replace_shell(
    old: Option<&ComponentInstance>,
    refocus: impl FnOnce(&ComponentInstance),
    build: impl FnOnce(Option<&slint::Window>) -> Result<ComponentInstance>,
) -> Result<ComponentInstance> {
    let window = old.map(ComponentHandle::window);
    let kept = window.and_then(ShownTree::of);
    // Slint keeps these on the tree's root (the `Window` element), not on
    // the native window: a new tree starts without them, and the winit
    // backend would take the window out of fullscreen, unmaximize it or
    // restore it at the next property update. Read before the build (the
    // new tree takes the window as it is created) and set before `show()`
    // (which applies the window properties).
    let state = window.map(|w| (w.is_fullscreen(), w.is_maximized(), w.is_minimized()));
    let built = build(window).and_then(|shell| {
        if let (Some(window), Some(kept)) = (window, &kept)
            && kept.is_shown_in(window)
        {
            bail!("the new component was not built in the window it replaces");
        }
        if let Some((fullscreen, maximized, minimized)) = state {
            // All three at once on the new root: `Window::set_fullscreen` and
            // the like each apply the window properties, and an order with a
            // mixed state in between would unmaximize and maximize again.
            if let Some(item) =
                slint::private_unstable_api::re_exports::WindowInner::from_pub(shell.window())
                    .window_item()
            {
                let item = item.as_pin_ref();
                item.full_screen.set(fullscreen);
                item.maximized.set(maximized);
                item.minimized.set(minimized);
            }
            shell.show()?;
        }
        Ok(shell)
    });
    if built.is_err()
        && let (Some(old), Some(window), Some(kept)) = (old, window, &kept)
        && kept.restore(window) == Some(false)
    {
        refocus(old);
    }
    built
}

/// New windows whose frame goes where a saved or previous window's was
/// (`Window::position`, the frame's top-left corner, as `ui_state` saves it).
///
/// Slint creates the native window only after `show()`, from the event
/// loop, and hands a position set before that to winit as a creation
/// attribute, which macOS takes as the top-left corner of the *content* (the
/// title bar goes above it) and Windows as the frame's. Every window
/// restored that way on macOS opened one title bar higher than it was (the
/// floating sidebar and the main window at each launch). Once
/// the native window exists a position is the frame's everywhere, so
/// [`Placements::tick`] sets it again then.
///
/// The main window ([`Restore`]) also gets back its maximized state and, on
/// Windows, its frame in desktop pixels; a saved frame on a display that is
/// gone (or no longer reaches that far) moves onto the primary display
/// ([`reachable`]). It is never seen at the wrong frame: Windows keeps it
/// cloaked until it is placed (`platform::hold_next_shown`, `placed`), and
/// macOS zooms it at its creation, before it is visible (`main.rs`).
///
/// A placement follows the window, not its component tree: it is tracked
/// by Slint's window adapter, which a shell built in the same window by a
/// composition change shares (`replace_shell`).
#[derive(Default)]
pub struct Placements(Vec<Placement>);

struct Placement {
    window: std::rc::Weak<dyn slint::platform::WindowAdapter>,
    position: slint::WindowPosition,
    restore: Restore,
    /// Ticks waited for the native window.
    waited: u32,
    /// Placed at the last tick: kept one more, so a frame the system has
    /// not reported yet is not taken for the user's (`placing`).
    placed: bool,
}

/// What the main window gets back beyond its position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Restore {
    pub maximized: bool,
    /// The normal frame in desktop pixels (Windows: the window placement).
    pub physical: Option<Rect>,
}

/// Give up on a window that never gets a native one (about 2 s of ticks).
const PLACEMENT_TICKS: u32 = 120;

fn adapter_of(window: &slint::Window) -> std::rc::Weak<dyn slint::platform::WindowAdapter> {
    use slint::private_unstable_api::re_exports::WindowInner;
    std::rc::Rc::downgrade(&WindowInner::from_pub(window).window_adapter())
}

impl Placements {
    /// Show `component` with its frame at `position`.
    pub fn show_at(
        &mut self,
        component: &ComponentInstance,
        position: impl Into<slint::WindowPosition>,
    ) -> Result<(), slint::PlatformError> {
        self.show_restored(component, position, Restore::default())
    }

    /// [`Self::show_at`], with the main window's `restore`.
    pub fn show_restored(
        &mut self,
        component: &ComponentInstance,
        position: impl Into<slint::WindowPosition>,
        restore: Restore,
    ) -> Result<(), slint::PlatformError> {
        let position = position.into();
        component.window().set_position(position.clone());
        component.show()?;
        self.track(component.window(), position, restore);
        Ok(())
    }

    /// Put an already shown `window`'s frame at `position` once its native
    /// window exists.
    fn track(&mut self, window: &slint::Window, position: slint::WindowPosition, restore: Restore) {
        self.0.push(Placement {
            window: adapter_of(window),
            position,
            restore,
            waited: 0,
            placed: false,
        });
    }

    /// Whether `window` still waits for its frame to be placed (its size,
    /// position and maximized state are not the restored ones yet).
    pub fn placing(&self, window: &slint::Window) -> bool {
        self.0.iter().any(|placement| {
            placement
                .window
                .upgrade()
                .is_some_and(|adapter| std::ptr::eq(adapter.window(), window))
        })
    }

    /// Place each window whose native window now exists (not in fullscreen,
    /// where it has no frame of its own); windows closed meanwhile are
    /// forgotten.
    pub fn tick(&mut self) {
        use slint::winit_030::WinitWindowAccessor;
        self.0.retain_mut(|placement| {
            let Some(adapter) = placement.window.upgrade() else {
                return false;
            };
            if placement.placed {
                return false;
            }
            let window = adapter.window();
            if window.has_winit_window() {
                if !window.is_fullscreen() {
                    place(window, &placement.position, placement.restore);
                }
                crate::platform::placed(window);
                placement.placed = true;
                return true;
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

/// Put `window` (its native window exists) at its saved frame.
fn place(window: &slint::Window, position: &slint::WindowPosition, restore: Restore) {
    if let Some(frame) = restore.physical {
        let (displays, primary) = displays(window);
        let target = reachable(frame, &displays, primary).unwrap_or(frame);
        if target != frame {
            diag!("ui: the saved window frame {frame:?} is off every display; moved to {target:?}");
        }
        if crate::platform::set_normal_frame(window, target, restore.maximized) {
            return;
        }
    }
    // macOS: zoomed at its creation (`main.rs`); moving it would end that.
    if restore.maximized && cfg!(target_os = "macos") {
        return;
    }
    window.set_position(position.clone());
    keep_reachable(window);
    if restore.maximized {
        window.set_maximized(true);
    }
}

/// A rectangle in desktop pixels (physical, as the system and winit report
/// frames and displays).
pub type Rect = crate::platform::DesktopRect;

/// The displays a frame must reach and the primary one: their work areas
/// where the system lists them (Windows: without the taskbar), else
/// winit's whole displays.
fn displays(window: &slint::Window) -> (Vec<Rect>, Option<Rect>) {
    use slint::winit_030::{WinitWindowAccessor, winit::monitor::MonitorHandle};
    let work = crate::platform::work_areas();
    if !work.is_empty() {
        let primary = work.iter().find(|(_, primary)| *primary).map(|(r, _)| *r);
        return (work.into_iter().map(|(r, _)| r).collect(), primary);
    }
    let rect = |display: &MonitorHandle| {
        let (position, size) = (display.position(), display.size());
        Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    };
    window
        .with_winit_window(|native| {
            (
                native.available_monitors().map(|d| rect(&d)).collect(),
                native.primary_monitor().map(|d| rect(&d)),
            )
        })
        .unwrap_or_default()
}

/// The part of a frame's top edge that must lie on a display for the frame
/// to be reachable: this many pixels of its height …
const GRIP_HEIGHT: i64 = 30;
/// … across this many of its width (all of a narrower frame).
const GRIP_WIDTH: i64 = 100;

/// Where a restored `frame` goes: `None` while it can be reached where it is
/// (a grip of its title bar lies on one of `displays`), else centred on
/// `fallback` (the primary display), shrunk to fit it. Chrome and Firefox
/// do the same with a window saved on a display that is gone. Without
/// displays (a system that does not list them) the frame stays.
pub fn reachable(frame: Rect, displays: &[Rect], fallback: Option<Rect>) -> Option<Rect> {
    let (width, height) = (i64::from(frame.width), i64::from(frame.height));
    let need_x = width.clamp(1, GRIP_WIDTH);
    let need_y = height.clamp(1, GRIP_HEIGHT);
    let (left, top) = (i64::from(frame.x), i64::from(frame.y));
    let grip_on = |display: &Rect| {
        let (dx, dy) = (i64::from(display.x), i64::from(display.y));
        let across = (left + width).min(dx + i64::from(display.width)) - left.max(dx);
        let down = (top + need_y).min(dy + i64::from(display.height)) - top.max(dy);
        across >= need_x && down >= need_y
    };
    if displays.is_empty() || displays.iter().any(grip_on) {
        return None;
    }
    let display = fallback.or_else(|| displays.first().copied())?;
    let (width, height) = (
        frame.width.min(display.width),
        frame.height.min(display.height),
    );
    let centred = |start: i32, room: u32, size: u32| start + ((room - size) / 2) as i32;
    Some(Rect {
        x: centred(display.x, display.width, width),
        y: centred(display.y, display.height, height),
        width,
        height,
    })
}

/// Move a window whose frame is out of reach ([`reachable`]) onto the primary
/// display.
fn keep_reachable(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    let (displays, primary) = displays(window);
    let moved = window
        .with_winit_window(|native| {
            let (position, size) = (native.outer_position().ok()?, native.outer_size());
            let frame = Rect {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            };
            let target = reachable(frame, &displays, primary)?;
            // The content shrinks by what the frame does.
            let inner = native.inner_size();
            let content = slint::PhysicalSize::new(
                inner
                    .width
                    .saturating_sub(frame.width - target.width)
                    .max(1),
                inner
                    .height
                    .saturating_sub(frame.height - target.height)
                    .max(1),
            );
            Some((frame, target, content))
        })
        .flatten();
    if let Some((frame, target, content)) = moved {
        diag!("ui: a window at {frame:?} is off every display; moved to {target:?}");
        if (target.width, target.height) != (frame.width, frame.height) {
            window.set_size(content);
        }
        window.set_position(slint::PhysicalPosition::new(target.x, target.y));
    }
}

/// The one Slint compiler configuration for the shell, panels, tool windows and
/// package validation: `@vestra/...` imports and the dark std-widgets style.
///
/// Every imported `.slint` file comes through the file loader, which puts
/// the interface language's translations into its `@tr` literals with the
/// catalog of the package that holds it (`ui/` files: the player's own;
/// doc/architecture/i18n.md §4). The file a compile starts from is given as
/// source: `build_from_source` callers pass it through [`translate`].
pub fn compiler() -> Compiler {
    let mut compiler = Compiler::default();
    compiler.set_library_paths(
        [("vestra".to_owned(), ui_dir().join("vestra"))]
            .into_iter()
            .collect(),
    );
    compiler.set_style("fluent-dark".into());
    compiler.set_file_loader(|path: &Path| {
        let path = path.to_owned();
        Box::pin(async move {
            // No catalog (English, or a package without this language): the
            // compiler reads the file itself.
            let catalog = crate::i18n::catalog_for(&path)?;
            // Not readable here: the compiler's own loader reports it.
            let source = std::fs::read_to_string(&path).ok()?;
            if !source.contains("@tr") {
                return None;
            }
            Some(Ok(catalog.rewrite(&source)))
        })
    });
    compiler
}

/// Source of the file at `path` (a package's, or the player's when it is
/// under `ui/`) in the interface language: see [`compiler`].
pub fn translate(source: String, path: &Path) -> String {
    crate::i18n::translate_slint(path, source)
}

/// Fonts: no window sets `default-font-family`, so every window uses the
/// system UI font and Slint's per-character fallback for scripts it lacks;
/// the CJK fallback chains are the host's (`ui_fonts`, architecture README
/// "fonts").
pub fn compile(path: &Path) -> Result<ComponentInstance> {
    compile_source(std::fs::read_to_string(path)?, path)
}
pub fn compile_source(source: String, path: &Path) -> Result<ComponentInstance> {
    definition_source(source, path)?
        .create()
        .map_err(Into::into)
}
/// The compiled component of [`compile_source`], to create several instances.
pub fn definition_source(
    mut source: String,
    path: &Path,
) -> Result<slint_interpreter::ComponentDefinition> {
    // R-H6: the player sets this window's palette (which `Theme` reads).
    if !source.contains(crate::ui::theme::PALETTE) {
        source.push('\n');
        source.push_str(&crate::ui::theme::export_line());
    }
    let source = translate(source, path);
    let result = pollster::block_on(compiler().build_from_source(source, path.to_owned()));
    if result.components().count() > 1 {
        bail!("Slint file must export exactly one component");
    }
    result.components().next().ok_or_else(|| {
        anyhow::anyhow!(
            "Slint compilation: {:?}",
            result.diagnostics().collect::<Vec<_>>()
        )
    })
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
/// What an explicit show did with a window that was open already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShownAgain {
    /// Un-minimized (if it was), in front and focused: the user works in
    /// the player.
    Front,
    /// Left where it is (minimized, or behind another app), its taskbar
    /// button flashing or the Dock icon bouncing once: the user works in
    /// another app, and an automatic show (the resume prompt on a file with
    /// a saved position, an online subtitle search) must not take the
    /// keyboard from it.
    Attention,
}

/// Show a window that is open already again, as an explicit "show" does in
/// other desktop apps (VS Code, Chrome): Slint's `show()` on a visible
/// window does nothing, so a minimized tool window or one behind the main
/// window stayed where it was. `foreground` (`platform::app_in_foreground`):
/// the user works in one of the player's windows, so the window comes back
/// un-minimized, in front, with the focus (a plain foreground change, which
/// the system allows the foreground process; no synthetic input). Otherwise
/// the protocol cannot tell a user's command from a plugin's own call, so
/// the window stays where it is and asks for attention instead.
pub fn show_again(instance: &ComponentInstance, foreground: bool) -> ShownAgain {
    let window = instance.window();
    if !foreground {
        use slint::winit_030::WinitWindowAccessor;
        window.with_winit_window(|native| {
            native.request_user_attention(Some(
                slint::winit_030::winit::window::UserAttentionType::Informational,
            ))
        });
        return ShownAgain::Attention;
    }
    let restored = unminimize(window);
    crate::platform::focus_own_window(window);
    if restored {
        focus_after_restore(instance, crate::platform::focus_own_window);
    }
    ShownAgain::Front
}

/// Bring the main window to the front from another app, for files the user
/// opened in Explorer or Finder (a second launch hands over its foreground
/// rights): winit's `focus_window`, which on Windows gets past the
/// foreground lock.
pub fn raise(instance: &ComponentInstance) {
    use slint::winit_030::WinitWindowAccessor;
    let focus = |window: &slint::Window| {
        window.with_winit_window(|native| native.focus_window());
    };
    if unminimize(instance.window()) {
        focus_after_restore(instance, focus);
    }
    focus(instance.window());
}

/// Un-minimize `window`; whether it was minimized. Slint's own flag goes
/// first (the winit backend keeps it equal to the system's on every state
/// change); winit's is checked as well in case a state event has not
/// arrived yet. Focusing a minimized window does nothing in winit, so this
/// comes first.
fn unminimize(window: &slint::Window) -> bool {
    let mut was = window.is_minimized();
    if was {
        window.set_minimized(false);
    }
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|native| {
        if native.is_minimized() == Some(true) {
            was = true;
            native.set_minimized(false);
        }
    });
    was
}

/// macOS restores a minimized window with an animation that ends after the
/// call returns, and winit does not focus a window it still sees minimized:
/// focus once more on the next turn of the event loop. Windows restores
/// synchronously.
fn focus_after_restore(instance: &ComponentInstance, focus: fn(&slint::Window)) {
    if cfg!(target_os = "macos") {
        let weak = instance.as_weak();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if let Some(instance) = weak.upgrade() {
                focus(instance.window());
            }
        });
    }
}
/// The main window's close request (its title bar's close button, Alt+F4;
/// ⌘W does nothing in the main window) quits the player, as in mpv, VLC and PotPlayer: `quit` starts the
/// player's Quit (the menu's), which ends the event loop, saves the state and
/// closes the tool windows. The window stays shown until then: left to
/// Slint's default (hide), the event loop kept running while a plugin's tool
/// window or the options window was still open, and the player with it.
/// A tool window's own close only closes that window (`open`).
pub fn on_main_close(window: &slint::Window, quit: impl Fn() + 'static) {
    window.on_close_requested(move || {
        quit();
        slint::CloseRequestResponse::KeepWindowShown
    });
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
    crate::ui::ui_values::own_default_models(&instance, None)?;
    for (name, value) in properties {
        crate::ui::ui_values::set(&instance, None, name, value.clone())?;
    }
    for callback in callbacks {
        let events = events.clone();
        let name = callback.clone();
        let id = id.to_owned();
        instance.set_callback(callback, move |args| {
            match args
                .iter()
                .map(crate::ui::ui_values::to_json)
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
    crate::platform::shown(&instance);
    Ok(View { instance })
}

/// For window tests in other modules (`capabilities::ui_window`).
#[cfg(test)]
pub(crate) use tests::recording_platform;

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

    /// Reported by a plugin author: the tip used to
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

    /// A close request on the main window (title bar, Alt+F4) starts the
    /// player's Quit and leaves the window shown for it; the default (hide)
    /// left the player running while a tool window was open.
    #[test]
    fn closing_the_main_window_quits() {
        headless();
        let instance = compile_source(
            "export component W inherits Window { width: 100px; height: 100px; }".into(),
            Path::new("w.slint"),
        )
        .unwrap();
        instance.show().unwrap();
        let quits = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = quits.clone();
        on_main_close(instance.window(), move || count.set(count.get() + 1));
        perform(instance.window(), crate::platform::WindowAction::Close);
        assert_eq!(quits.get(), 1);
        assert!(instance.window().is_visible());
        instance.hide().unwrap();
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
        crate::ui::theme::apply_fonts(&instance).unwrap();
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
        crate::ui::theme::apply(&instance, "vestra-light").unwrap();
        assert_eq!(get("bar"), brush(0xf7f6fa));
        // The video area stays dark.
        assert_eq!(get("video"), brush(0x0e0d12));
        assert_eq!(get("std-background"), brush(0xfafafa));
        assert_ne!(get("fill"), dark_fill);
        crate::ui::theme::apply(&instance, "vestra").unwrap();
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
        crate::ui::theme::apply(&instance, "vestra-light").unwrap();
        assert_eq!(alpha(), 0x33);
        crate::ui::theme::apply(&instance, "vestra").unwrap();
        assert_eq!(alpha(), 0x80);
    }

    /// i18n.md §4: a package's `.slint` files, the one opened and the ones
    /// it imports, show its catalog's translations; the player's library
    /// keeps the player's own (none in an English run).
    #[test]
    fn plugin_slint_files_show_their_packages_translations() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(
            root.join("ui/part.slint"),
            r#"export global Part { out property <string> label: @tr("Hello {}", "Ann"); }"#,
        )
        .unwrap();
        let main = root.join("ui/w.slint");
        std::fs::write(
            &main,
            r#"import { Part } from "part.slint";
import { UrlInputRow } from "@vestra/widgets.slint";
export component W inherits Window {
    in property <int> count: 2;
    out property <string> label: Part.label;
    out property <string> files: @tr("{n} file" | "{n} files" % count);
    out property <string> quote: @tr("Quote");
    out property <string> english: @tr("Untranslated");
    out property <string> library: row.confirm-text;
    row := UrlInputRow {}
}"#,
        )
        .unwrap();
        let po = r#"msgid "Hello {}"
msgstr "你好 {}"

msgid "{n} file"
msgid_plural "{n} files"
msgstr[0] "{n} 个文件"

msgid "Quote"
msgstr "引 \"号\" \\ 后"
"#;
        let catalog = std::sync::Arc::new(crate::i18n::Catalog::from_entries(
            &crate::i18n::po::parse(po).unwrap(),
        ));
        crate::i18n::register(&root, catalog.clone());
        let instance = compile(&main).unwrap();
        crate::i18n::unregister(&root, &catalog);
        let get = |name: &str| match instance.get_property(name).unwrap() {
            Value::String(text) => text.to_string(),
            other => panic!("{other:?}"),
        };
        assert_eq!(get("label"), "你好 Ann");
        assert_eq!(get("files"), "2 个文件");
        assert_eq!(get("quote"), "引 \"号\" \\ 后");
        assert_eq!(get("english"), "Untranslated");
        assert_eq!(get("library"), "Open");
        // Without the catalog the same files are English.
        let english = compile(&main).unwrap();
        assert_eq!(
            english.get_property("files").unwrap(),
            Value::String("2 files".into())
        );
    }

    fn headless() {
        headless_window();
    }

    /// The headless platform's one window (every component of this thread
    /// shows in it). It reuses its buffer, so a test that draws it repaints
    /// only the dirty regions, as the player's software renderer does.
    fn headless_window() -> std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow> {
        thread_local! {
            static WINDOW: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow> = {
                let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
                    slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
                );
                slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
                window
            };
        }
        WINDOW.with(std::rc::Rc::clone)
    }

    /// menu.rs sizes each panel from its own measurement of the row
    /// text; it must never come out narrower than Slint's layout of the same
    /// text at the menu font, or the row elides ("VestraNES Window…",
    /// coordinator report 2026-10-27). Checked on every string of the host's
    /// and the built-in plugins' catalogs in all languages (de and fr have
    /// the longest Latin labels, ja/ko/zh-Hans the CJK ones).
    #[allow(clippy::disallowed_macros)] // a skip note for the terminal
    #[test]
    fn menu_text_measurement_covers_slints_layout_in_every_language() {
        headless();
        crate::ui::ui_fonts::install("en");
        // A system without a Latin font (a bare container) has nothing to compare.
        if crate::ui::ui_fonts::text_width("x", 12., |_| 0.).is_none() {
            eprintln!("skipped: no system Latin font to compare with");
            return;
        }
        let source = r#"
export component W inherits Window {
    in property <string> t;
    out property <length> w: label.preferred-width;
    label := Text { text: root.t; font-size: 12px; }
}"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
        );
        let instance = result.component("W").unwrap().create().unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut directories = vec![src.join("host/locales")];
        for plugin in std::fs::read_dir(src.join("plugins")).unwrap() {
            directories.push(plugin.unwrap().path().join("locales"));
        }
        let (mut checked, mut short) = (0, Vec::new());
        for directory in directories {
            let Ok(files) = std::fs::read_dir(&directory) else {
                continue;
            };
            for file in files {
                let path = file.unwrap().path();
                if path.extension().is_none_or(|e| e != "po") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for entry in crate::i18n::po::parse(&text).unwrap() {
                    for string in entry.strings.iter().chain([&entry.id]) {
                        if string.is_empty() || string.contains('\n') {
                            continue;
                        }
                        instance
                            .set_property("t", Value::String(string.as_str().into()))
                            .unwrap();
                        let Value::Number(slint) = instance.get_property("w").unwrap() else {
                            panic!("w");
                        };
                        // Slint's width is whole pixels and up to 2px over the
                        // advances; `panel_width` keeps that much slack.
                        let ours = crate::ui::menu::text_width(string);
                        checked += 1;
                        if ours + 2. < slint {
                            short.push(format!(
                                "{}: {string:?} {ours:.1} < {slint:.1}",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "{checked}");
        assert!(short.is_empty(), "{}", short.join("\n"));
        // The longest menu rows of the built-in plugins' catalogs (the French
        // and Spanish A-B loop rows) and the reported VestraNES row fit below
        // the maximum panel width, so none of them elides.
        for (title, keys) in [
            (
                "Boucler le chapitre actuel ou la plage de signets actuelle",
                "Alt+\\",
            ),
            (
                "Repetir capítulo o intervalo de marcadores actual",
                "Alt+\\",
            ),
            ("VestraNES Window…", ""),
            ("VestraNES-Fenster…", ""),
        ] {
            let row = json!({"title": title, "keys": keys, "busy": false});
            let width = crate::ui::menu::panel_width(&[row]);
            assert!(width < crate::ui::menu::MAX_WIDTH, "{title}: {width}");
        }
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
            label: title.into(),
            group: "g".into(),
            children,
        };
        let roots = vec![group("视频", vec![group("比例", vec![])])];
        let none = |_: u64, _: &str| None;
        let set = |path: &[&str]| {
            let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
            let origin = crate::ui::menu::Point { x: 10., y: 10. };
            let window = crate::ui::menu::Point { x: 600., y: 400. };
            let value = crate::ui::menu::levels(&roots, &path, origin, window, &[], &none);
            instance
                .set_property("levels", crate::ui::ui_values::from_json(&value).unwrap())
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

    /// Coordinator report 2026-10-25: one pointer move straight to a far row,
    /// then a click there, must hover and activate that row, not the one
    /// hovered before. Each row has its own TouchArea, and Slint hit-tests a
    /// press at the press's own position (winit also reports the cursor at
    /// the press first), so even a press with no move before it lands on the
    /// row under it. Rows of a level that scrolls are drawn where `menu.rs`
    /// says they are (below the up arrow bar), so the host's submenu and
    /// tooltip positions match what the pointer hits.
    #[test]
    fn a_click_activates_the_row_under_it_after_a_single_jump() {
        use crate::commands::{CommandKey, CommandSnapshot, CommandState, Declaration, MenuNode};
        use crate::ui::menu::{ARROW, Point, levels};
        use std::cell::RefCell;
        use std::rc::Rc;
        headless();
        let leaf = |title: String, group: &str| {
            let mut declaration = Declaration::new(&title, &[title.as_str()]);
            declaration.group = group.into();
            MenuNode::Command {
                entry: CommandSnapshot {
                    key: CommandKey {
                        owner: 1,
                        local_id: title.clone(),
                    },
                    revision: 1,
                    declaration,
                    state: CommandState {
                        enabled: true,
                        checked: None,
                        busy: false,
                        reason: None,
                    },
                    manifest: "m".into(),
                    sequence: 1,
                },
            }
        };
        let none = |_: u64, _: &str| None;
        // The view, the host's last hovered token and the activated tokens.
        let view = |width: f32, height: f32| {
            let source = r#"
import { MenuView, MenuLevel } from "menu-view.slint";
export component W inherits Window {
    in property <[MenuLevel]> menu-levels;
    callback hover(int, string, bool);
    callback invoke(string);
    MenuView {
        width: root.width; height: root.height;
        levels: root.menu-levels;
        hovered(level, token, now) => { root.hover(level, token, now); }
        activated(token) => { root.invoke(token); }
    }
}"#;
            let result = pollster::block_on(
                compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
            );
            let instance = result.component("W").unwrap().create().unwrap();
            instance
                .window()
                .set_size(slint::LogicalSize::new(width, height));
            let log: Rc<RefCell<(String, Vec<String>)>> = Rc::default();
            let sink = log.clone();
            instance
                .set_callback("hover", move |args| {
                    if let Some(Value::String(token)) = args.get(1) {
                        sink.borrow_mut().0 = token.to_string();
                    }
                    Value::Void
                })
                .unwrap();
            let sink = log.clone();
            instance
                .set_callback("invoke", move |args| {
                    if let Some(Value::String(token)) = args.first() {
                        sink.borrow_mut().1.push(token.to_string());
                    }
                    Value::Void
                })
                .unwrap();
            (instance, log)
        };
        let settle = || {
            slint::platform::update_timers_and_animations();
        };
        let move_to = |instance: &slint_interpreter::ComponentInstance, (x, y): (f32, f32)| {
            instance
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(x, y),
                });
            settle();
        };
        let click = |instance: &slint_interpreter::ComponentInstance, (x, y): (f32, f32)| {
            let position = slint::LogicalPosition::new(x, y);
            let button = slint::platform::PointerEventButton::Left;
            let window = instance.window();
            window
                .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
            settle();
            window
                .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
            settle();
        };
        // The centre of a row's title, in window coordinates, from menu.rs.
        let centre = |level: &serde_json::Value, index: usize| {
            let shift = if level["scroll"] == true {
                ARROW - level["viewport-y"].as_f64().unwrap()
            } else {
                0.
            };
            let x = level["x"].as_f64().unwrap() + 40.;
            let y = level["y"].as_f64().unwrap()
                + shift
                + level["rows"][index]["y"].as_f64().unwrap()
                + 12.;
            (x as f32, y as f32)
        };
        let token = |level: &serde_json::Value, index: usize| {
            level["rows"][index]["token"].as_str().unwrap().to_owned()
        };

        // A top-level menu like the player's: groups of commands, then 设置 ▸.
        let mut roots: Vec<MenuNode> = (0..9)
            .map(|i| leaf(format!("命令{i}"), if i < 4 { "a" } else { "b" }))
            .collect();
        roots.push(leaf("插件商城…".into(), "c"));
        roots.push(MenuNode::Group {
            title: "设置".into(),
            label: "设置".into(),
            group: "d".into(),
            children: vec![leaf("偏好".into(), "d")],
        });
        let (instance, log) = view(900., 700.);
        let value = levels(
            &roots,
            &[],
            Point { x: 420., y: 100. },
            Point { x: 900., y: 700. },
            &[],
            &none,
        );
        crate::host::set_menu_model(&instance, crate::ui::ui_values::from_json(&value).unwrap())
            .unwrap();
        let level = &value[0];
        let (store, settings) = (9, 10);
        move_to(&instance, centre(level, 1));
        move_to(&instance, centre(level, store));
        assert_eq!(log.borrow().0, token(level, store));
        // One jump from 命令1 to 设置, no move in between.
        move_to(&instance, centre(level, 1));
        move_to(&instance, centre(level, settings));
        assert_eq!(
            log.borrow().0,
            token(level, settings),
            "hover follows the jump"
        );
        // A click on 命令7 with no move before it: the press's own position.
        click(&instance, centre(level, 7));
        assert_eq!(log.borrow().1, [token(level, 7)], "the row under the press");
        // A click on 插件商城… right after hovering 设置.
        move_to(&instance, centre(level, settings));
        move_to(&instance, centre(level, store));
        click(&instance, centre(level, store));
        assert_eq!(log.borrow().1.last().unwrap(), &token(level, store));

        // A level taller than the window, at the top and scrolled.
        let roots: Vec<MenuNode> = (0..25).map(|i| leaf(format!("行{i}"), "g")).collect();
        let (instance, log) = view(320., 182.);
        for offset in [0., 30., 1e6] {
            let value = levels(
                &roots,
                &[],
                Point { x: 10., y: 0. },
                Point { x: 320., y: 182. },
                &[offset],
                &none,
            );
            crate::host::set_menu_model(
                &instance,
                crate::ui::ui_values::from_json(&value).unwrap(),
            )
            .unwrap();
            let level = &value[0];
            let vp = level["viewport-y"].as_f64().unwrap();
            let visible = level["height"].as_f64().unwrap() - 2. * ARROW;
            for index in 0..25 {
                let top = level["rows"][index]["y"].as_f64().unwrap();
                if top < vp || top + 24. > vp + visible {
                    continue;
                }
                log.borrow_mut().1.clear();
                move_to(&instance, centre(level, index));
                click(&instance, centre(level, index));
                assert_eq!(
                    log.borrow().1,
                    [token(level, index)],
                    "offset {offset}: row {index} under its menu.rs position"
                );
            }
        }
    }

    /// Coordinator report 2026-10-24 (播放 ▸ 跳转 ▸ / 速度微调 ▸): drives the
    /// real view with pointer events and a host loop like `Host` (submenu
    /// aim, `set_menu_model`). Host 0.5.2 (G-15): hovering a group row opens
    /// it within one tick, walking down a submenu switches siblings as the
    /// pointer reaches them, a diagonal toward the open submenu across a
    /// sibling keeps it; a click opens at once even when an open level's
    /// state is published between press and release.
    #[test]
    fn menu_pointer_walk_switches_sibling_submenus_and_clicks_survive_state_changes() {
        use crate::commands::{CommandKey, CommandSnapshot, CommandState, Declaration, MenuNode};
        use crate::ui::menu::{HoverAim, Point, hover_target, levels, panels};
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        headless();
        let source = r#"
import { MenuView, MenuLevel } from "menu-view.slint";
export component W inherits Window {
    width: 900px; height: 600px;
    in property <[MenuLevel]> menu-levels;
    callback hover(int, string, bool);
    MenuView {
        width: root.width; height: root.height;
        levels: root.menu-levels;
        hovered(level, token, now) => { root.hover(level, token, now); }
    }
}"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
        );
        let instance = result.component("W").unwrap().create().unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(900., 600.));
        // The state a plugin republishes: 播放/暂停's check mark.
        let paused = Cell::new(false);
        let leaf = |title: &str| {
            let mut declaration = Declaration::new(title, &[title]);
            declaration.group = "g".into();
            MenuNode::Command {
                entry: CommandSnapshot {
                    key: CommandKey {
                        owner: 1,
                        local_id: title.into(),
                    },
                    revision: 1,
                    declaration,
                    state: CommandState {
                        enabled: true,
                        checked: (title == "播放/暂停").then(|| paused.get()),
                        busy: false,
                        reason: None,
                    },
                    manifest: "m".into(),
                    sequence: 1,
                },
            }
        };
        let group = |title: &str, children| MenuNode::Group {
            title: title.into(),
            label: title.into(),
            group: "g".into(),
            children,
        };
        let roots = || {
            vec![
                group("文件", vec![leaf("打开")]),
                group(
                    "播放",
                    vec![
                        leaf("播放/暂停"),
                        leaf("上一个"),
                        leaf("下一个"),
                        group(
                            "跳转",
                            vec![
                                leaf("上一帧"),
                                leaf("下一帧"),
                                leaf("后退5秒"),
                                leaf("前进5秒"),
                            ],
                        ),
                        group(
                            "速度微调",
                            vec![leaf("慢一点"), leaf("快一点"), leaf("复位")],
                        ),
                        leaf("循环"),
                    ],
                ),
            ]
        };
        let hovers: Rc<RefCell<Vec<(usize, String, bool)>>> = Rc::default();
        let sink = hovers.clone();
        instance
            .set_callback("hover", move |args| {
                if let [Value::Number(level), Value::String(token), Value::Bool(now)] = args {
                    sink.borrow_mut()
                        .push((*level as usize, token.to_string(), *now));
                }
                Value::Void
            })
            .unwrap();
        let none = |_: u64, _: &str| None;
        let origin = Point { x: 100., y: 100. };
        let size = Point { x: 900., y: 600. };
        // As the host writes it: only what changed (`apply_menu_levels`).
        let shown = RefCell::new(crate::host::ShownMenu::default());
        let show = |path: &[String]| {
            let value = levels(&roots(), path, origin, size, &[], &none);
            crate::host::apply_menu_levels(&instance, &mut shown.borrow_mut(), &value).unwrap();
        };
        let path = RefCell::new(Vec::<String>::new());
        let clock = Cell::new(0u64);
        let aim = RefCell::new(HoverAim::default());
        // Where the pointer is, as the host's winit hook records it.
        let at = Cell::new(None::<Point>);
        show(&path.borrow());
        // One host tick (16 ms): the view's hovers, then a held switch.
        let tick = || {
            slint::platform::update_timers_and_animations();
            let mut path = path.borrow_mut();
            for (level, token, now) in hovers.borrow_mut().drain(..) {
                let Some(target) = hover_target(&path, level, &token) else {
                    continue;
                };
                if now {
                    aim.borrow_mut().clear();
                    *path = target;
                    show(&path);
                } else {
                    let open = panels(&levels(&roots(), &path, origin, size, &[], &none));
                    let submenu = (path.len() > level)
                        .then(|| open.get(level + 1).copied())
                        .flatten();
                    let next =
                        aim.borrow_mut()
                            .hover(&path, target, at.get(), submenu, clock.get());
                    if let Some(next) = next {
                        aim.borrow_mut().clear();
                        *path = next;
                        show(&path);
                    }
                }
            }
            let next = aim.borrow_mut().tick(at.get(), clock.get());
            if let Some(next) = next {
                *path = next;
                show(&path);
            }
        };
        let wait = |ms: u64| {
            tick();
            for _ in 0..ms / 16 {
                clock.set(clock.get() + 16);
                tick();
            }
        };
        let pointer = |(x, y): (f32, f32)| {
            at.set(Some(Point {
                x: f64::from(x),
                y: f64::from(y),
            }));
            instance
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(x, y),
                });
        };
        // The centre of a row's title, in window coordinates.
        let row = |depth: usize, index: usize| {
            let level = levels(&roots(), &path.borrow(), origin, size, &[], &none)[depth].clone();
            let x = level["x"].as_f64().unwrap() + 40.;
            let y =
                level["y"].as_f64().unwrap() + level["rows"][index]["y"].as_f64().unwrap() + 12.;
            (x as f32, y as f32)
        };
        let open = |expected: &[&str], what: &str| assert_eq!(*path.borrow(), expected, "{what}");

        pointer(row(0, 1));
        wait(0);
        open(&["播放"], "hovering 播放 opens it within the tick");
        // Down 播放 ▸ in 20 px steps, one tick each: every group row opens
        // as the pointer reaches it.
        let (x, mut y) = row(1, 0);
        let (_, jump) = row(1, 3);
        let (_, speed) = row(1, 4);
        while y <= speed {
            pointer((x, y));
            wait(16);
            if (y - jump).abs() <= 12. {
                open(&["播放", "跳转"], "reaching 跳转 opens it at once");
            }
            y += 20.;
        }
        pointer((x, speed));
        wait(16);
        open(
            &["播放", "速度微调"],
            "going straight down switches to 速度微调 at once",
        );
        // Back on 跳转, then diagonally toward its submenu across 速度微调:
        // 跳转 stays open while the pointer heads for it.
        pointer((x, jump));
        wait(16);
        open(&["播放", "跳转"], "back on 跳转");
        let (sub_x, _) = row(2, 0);
        let start = (x, jump);
        for step in 1..=3 {
            let t = step as f32 / 4.;
            pointer((
                start.0 + (sub_x - 40. - start.0) * t,
                start.1 + (speed + 6. - start.1) * t,
            ));
            wait(16);
            open(&["播放", "跳转"], "a diagonal toward 跳转 ▸ keeps it");
        }
        // The pointer stops there, on 速度微调: it switches after the grace.
        wait(crate::ui::menu::AIM_GRACE_MS - 32);
        open(&["播放", "跳转"], "held within the grace");
        wait(48);
        open(&["播放", "速度微调"], "the pointer stopped: it switches");
        pointer((x, jump));
        wait(16);
        pointer((x, speed));
        wait(16);
        open(&["播放", "速度微调"], "straight down again: at once");
        // On 速度微调 with small wiggles: 跳转 closes, 速度微调 opens.
        for wiggle in 0..4 {
            pointer((x + if wiggle % 2 == 0 { 3. } else { -3. }, speed));
            wait(150);
        }
        open(
            &["播放", "速度微调"],
            "resting on a sibling switches submenus",
        );

        // Clicks, with 播放/暂停's state published between press and release.
        let click = |at: (f32, f32)| {
            pointer(at);
            wait(0);
            let position = slint::LogicalPosition::new(at.0, at.1);
            let button = slint::platform::PointerEventButton::Left;
            instance
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
            wait(32);
            paused.set(!paused.get());
            show(&path.borrow());
            wait(32);
            instance
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
            wait(0);
        };
        click(row(1, 3));
        open(&["播放", "跳转"], "a click on 跳转 opens it at once");
        click(row(1, 4));
        open(
            &["播放", "速度微调"],
            "a click on 速度微调 opens it at once",
        );
    }

    /// The shell with a bottom bar and a sidebar.
    fn bar_and_sidebar() -> [crate::ui::composition::CompileSpec; 2] {
        use crate::ui::composition::{CompileSpec, Slot};
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

    /// Host 0.6.0: the side buttons are window-level (the window's winit
    /// hook, `observe_side_button`, fires them once wherever the pointer
    /// is), so the shell reports them nowhere itself: over the video it no
    /// longer calls `mouse-input` with X1/X2 (which would fire them twice),
    /// over the bar it never did. Left presses on the video and a left
    /// click on a bar button act as before.
    #[test]
    fn side_buttons_are_not_reported_by_the_shell_and_left_clicks_are_unchanged() {
        use slint::platform::{PointerEventButton, WindowEvent};
        use std::cell::RefCell;
        use std::rc::Rc;
        headless();
        let specs = bar_and_sidebar();
        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::ui::composition::compile_json(&specs, sender, 1).unwrap();
        shell.set_property("bottom-visible", true.into()).unwrap();
        shell.set_property("sidebar-visible", false.into()).unwrap();
        crate::ui::composition::set_global_value(&shell, &specs[0], "can-pause", true.into())
            .unwrap();
        let mouse: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = mouse.clone();
        shell
            .set_callback("mouse-input", move |args| {
                if let Some(slint_interpreter::Value::String(name)) = args.first() {
                    sink.borrow_mut().push(name.to_string());
                }
                slint_interpreter::Value::Bool(false)
            })
            .unwrap();
        let presses = Rc::new(std::cell::Cell::new(0));
        let count = presses.clone();
        shell
            .set_callback("left-press", move |_| {
                count.set(count.get() + 1);
                slint_interpreter::Value::Void
            })
            .unwrap();
        shell.window().set_size(slint::LogicalSize::new(800., 600.));
        shell.show().unwrap();
        settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n as f32,
            other => panic!("{name}: {other:?}"),
        };
        let (x, y, w, h) = (
            number("video-x"),
            number("video-y"),
            number("video-width"),
            number("video-height"),
        );
        assert!(h < 600., "the bar is below the video");
        let video = (x + w / 2., y + h / 2.);
        // The fixture bar's Pause button: 8px padding, then the button.
        let bar_button = (x + 30., y + h + 30.);
        let click = |at: (f32, f32), button: PointerEventButton| {
            let position = slint::LogicalPosition::new(at.0, at.1);
            shell
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position });
            shell
                .window()
                .dispatch_event(WindowEvent::PointerPressed { position, button });
            shell
                .window()
                .dispatch_event(WindowEvent::PointerReleased { position, button });
            slint::platform::update_timers_and_animations();
        };
        for at in [video, bar_button] {
            for button in [PointerEventButton::Back, PointerEventButton::Forward] {
                click(at, button);
            }
        }
        assert!(
            mouse.borrow().is_empty(),
            "the shell reported {:?}",
            mouse.borrow()
        );
        // The bar's own callbacks (focus reports aside): none.
        let actions =
            |receiver: &std::sync::mpsc::Receiver<crate::ui::composition::JsonCallback>| {
                receiver
                    .try_iter()
                    .filter(|event| event.callback != "interaction")
                    .map(|event| (event.owner, event.callback))
                    .collect::<Vec<_>>()
            };
        assert_eq!(actions(&receiver), [], "no bar action");
        click(video, PointerEventButton::Left);
        assert_eq!(presses.get(), 1, "a left press on the video");
        click(bar_button, PointerEventButton::Left);
        assert_eq!(actions(&receiver), [(10, "set-paused".to_owned())]);
        assert_eq!(presses.get(), 1, "the bar is not the video");
        shell.hide().unwrap();
    }

    /// Slint 1.17+ (see `settle_layout`): hiding the docked sidebar (an `if`
    /// in the shell's layout) and reading the video area in the same turn,
    /// as the window tick and `window.setVideoArea` do.
    #[test]
    fn geometry_is_current_after_a_condition_changes() {
        headless();
        let source = crate::ui::composition::shell_source(&bar_and_sidebar()).unwrap();
        let shell = crate::ui::composition::build(source, "HostWindow")
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
            diag!("sidebar-visible {visible}: {before} before settle_layout");
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
        let source = crate::ui::composition::shell_source(&bar_and_sidebar()).unwrap();
        let marker = "    callback focus-video();\n";
        assert!(source.contains(marker));
        let probes = "    callback focus-video();\n\
            out property <length> t-bar-x: bottom-keys.absolute-position.x;\n\
            out property <length> t-bar-w: bottom-keys.width;\n";
        let source = source.replacen(marker, probes, 1);
        let shell = crate::ui::composition::build(source, "HostWindow")
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
        let source = crate::ui::composition::shell_source(&[]).unwrap();
        let shell = crate::ui::composition::build(source, "HostWindow")
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
        specs.push(crate::ui::composition::CompileSpec {
            owner: 30,
            file: base.join("tab.slint"),
            component: "NoteTab".into(),
            global: Some("Bridge".into()),
            slot: crate::ui::composition::Slot::Sidebar,
        });
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::ui::composition::compile_json(&specs, sender, 1).unwrap();
        crate::ui::composition::set_tabbed(&shell, &specs[2], true).unwrap();
        // The fixture without the property is left alone.
        crate::ui::composition::set_tabbed(&shell, &specs[1], true).unwrap();
        assert_eq!(
            shell
                .get_global_property("Owner30Bridge", "tabbed")
                .unwrap(),
            slint_interpreter::Value::Bool(true)
        );
        shell
            .set_property(
                "sidebar-titles",
                crate::ui::ui_values::from_json(&serde_json::json!(["播放列表", "Example list"]))
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
        for (active, owner) in [(0., 20.), (1., 30.), (0., 20.)] {
            shell.set_property("sidebar-active", active.into()).unwrap();
            shell.set_property("sidebar-shown", owner.into()).unwrap();
            settle_layout(shell.window());
            assert_eq!(read("video-width"), 1470. - 320., "tab {active}");
        }
        shell.hide().unwrap();
    }

    /// sidebar-tabs.md: twenty sidebar tabs, four panels compiled (the
    /// front tabs and the current one, `sidebar_tabs::compiled`); a switch
    /// between compiled tabs is a property change, a placeholder shows no
    /// panel, and the strip moves the current tab into its last place with
    /// the `»` button beside it.
    #[test]
    fn twenty_tabs_compile_four_panels_and_the_strip_overflows() {
        use crate::ui::composition::{CompileSpec, Slot};
        use crate::ui::sidebar_tabs;
        headless();
        let dir = tempfile::tempdir().unwrap();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let order = (1..=20).map(|n| 100 + n).collect::<Vec<u64>>();
        let active = Some(117);
        let compiled =
            sidebar_tabs::compiled(&order, active, |_| false, |_| false, &Default::default());
        assert_eq!(compiled, [101, 102, 103, 117]);
        let mut specs = vec![bar_and_sidebar()[0].clone()];
        for owner in &compiled {
            // Each owner its own file (independent packages).
            let file = dir.path().join(format!("tab{owner}.slint"));
            std::fs::copy(base.join("tab.slint"), &file).unwrap();
            specs.push(CompileSpec {
                owner: *owner,
                file,
                component: "NoteTab".into(),
                global: Some("Bridge".into()),
                slot: Slot::Sidebar,
            });
        }
        let source = crate::ui::composition::shell_source_tabbed(&specs, true).unwrap();
        // Two columns (docked and fullscreen overlay), four branches each.
        assert_eq!(source.matches(": Owner1").count(), 2 * compiled.len());
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::ui::composition::compile_json_in(&specs, true, sender, 1, None).unwrap();
        let titles = order
            .iter()
            .map(|owner| format!("Tab {owner}"))
            .collect::<Vec<_>>();
        shell
            .set_property(
                "sidebar-titles",
                crate::ui::ui_values::from_json(&serde_json::json!(titles)).unwrap(),
            )
            .unwrap();
        shell
            .window()
            .set_size(slint::LogicalSize::new(1470., 956.));
        shell.show().unwrap();
        for (index, owner) in [(16., 117.), (0., 101.), (19., -1.)] {
            shell.set_property("sidebar-active", index.into()).unwrap();
            shell.set_property("sidebar-shown", owner.into()).unwrap();
            settle_layout(shell.window());
        }
        shell.hide().unwrap();
    }

    /// The strip's overflow (sidebar-tabs.md): tabs stay at least 72 px; the
    /// rest go into `»`, and the current tab takes the last place when it
    /// would not fit.
    #[test]
    fn the_tab_strip_overflows_and_keeps_the_current_tab_visible() {
        headless();
        let source = r#"import { SidebarTabs } from "sidebar-tabs.slint";
export component T inherits Window {
    in property <length> strip-width: 317px;
    in property <int> count: 2;
    in property <int> active: 0;
    width: 900px; height: 40px;
    strip := SidebarTabs {
        x: 0px; y: 0px; width: root.strip-width;
        titles: [ "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20" ];
        active: root.active; detached: false;
    }
    narrow := SidebarTabs {
        x: 0px; y: 0px; width: root.strip-width;
        titles: [ "1", "2" ];
        active: root.active; detached: false;
    }
    out property <bool> overflowing: strip.overflowing;
    out property <int> shown: strip.shown;
    out property <int> last: strip.last;
    out property <bool> two-overflowing: narrow.overflowing;
    out property <int> two-shown: narrow.shown;
}"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
        );
        let window = result
            .component("T")
            .unwrap_or_else(|| panic!("{:?}", result.diagnostics().collect::<Vec<_>>()))
            .create()
            .unwrap();
        let number = |name: &str| match window.get_property(name).unwrap() {
            Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        let flag = |name: &str| window.get_property(name).unwrap() == Value::Bool(true);
        // The default sidebar width (317 px, 289 px of room): 3 tabs, `»`.
        window.set_property("active", 16.0.into()).unwrap();
        assert!(flag("overflowing"));
        assert_eq!(number("shown"), 3.);
        // Tabs 1 and 2, then the current 17th in the last place.
        assert_eq!(number("last"), 16.);
        window.set_property("active", 1.0.into()).unwrap();
        assert_eq!(number("last"), 2.);
        // Two tabs fit at the default width, as before (no `»`).
        assert!(!flag("two-overflowing"));
        assert_eq!(number("two-shown"), 2.);
        // A narrow sidebar: one place, the current tab in it.
        window.set_property("strip-width", 150.0.into()).unwrap();
        window.set_property("active", 7.0.into()).unwrap();
        assert_eq!(number("shown"), 1.);
        assert_eq!(number("last"), 7.);
        // A wide one: four places at most, the panels the host compiles
        // (`sidebar_tabs::front`): the first three and the current tab.
        window.set_property("strip-width", 600.0.into()).unwrap();
        assert_eq!(number("shown"), 4.);
        assert_eq!(number("last"), 7.);
        window.set_property("active", 1.0.into()).unwrap();
        assert_eq!(number("last"), 3.);
    }

    thread_local! {
        /// What the recording platform's windows were asked to do.
        static NATIVE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    fn native(call: impl Into<String>) {
        NATIVE.with_borrow_mut(|log| log.push(call.into()));
    }
    fn native_calls() -> Vec<String> {
        NATIVE.with_borrow_mut(std::mem::take)
    }

    /// A native window that records what Slint asks of the windowing
    /// system (the seam the winit backend sits behind in the player).
    struct Recorded {
        window: slint::Window,
        renderer: slint::platform::software_renderer::SoftwareRenderer,
        size: std::cell::Cell<slint::PhysicalSize>,
        position: std::cell::Cell<slint::PhysicalPosition>,
        /// Fullscreen, maximized, minimized as the system has them: like
        /// winit, a property update that differs changes the window.
        flags: std::cell::Cell<(bool, bool, bool)>,
    }
    impl slint::platform::WindowAdapter for Recorded {
        fn window(&self) -> &slint::Window {
            &self.window
        }
        fn size(&self) -> slint::PhysicalSize {
            self.size.get()
        }
        fn set_size(&self, size: slint::WindowSize) {
            native("set_size");
            let scale = self.window.scale_factor();
            self.size.set(size.to_physical(scale));
            self.window
                .dispatch_event(slint::platform::WindowEvent::Resized {
                    size: size.to_logical(scale),
                });
        }
        fn position(&self) -> Option<slint::PhysicalPosition> {
            Some(self.position.get())
        }
        fn set_position(&self, position: slint::WindowPosition) {
            native("set_position");
            self.position
                .set(position.to_physical(self.window.scale_factor()));
        }
        fn set_visible(&self, visible: bool) -> Result<(), slint::PlatformError> {
            native(format!("set_visible {visible}"));
            Ok(())
        }
        fn update_window_properties(&self, properties: slint::platform::WindowProperties<'_>) {
            let flags = (
                properties.is_fullscreen(),
                properties.is_maximized(),
                properties.is_minimized(),
            );
            let old = self.flags.replace(flags);
            for (name, was, now) in [
                ("fullscreen", old.0, flags.0),
                ("maximized", old.1, flags.1),
                ("minimized", old.2, flags.2),
            ] {
                if was != now {
                    native(format!("{name} {now}"));
                }
            }
        }
        fn renderer(&self) -> &dyn slint::platform::Renderer {
            &self.renderer
        }
    }
    struct Recording;
    impl slint::platform::Platform for Recording {
        fn create_window_adapter(
            &self,
        ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            native("new window");
            Ok(std::rc::Rc::new_cyclic(|weak: &std::rc::Weak<Recorded>| {
                Recorded {
                    window: slint::Window::new(weak.clone()),
                    renderer: slint::platform::software_renderer::SoftwareRenderer::new(),
                    size: Default::default(),
                    position: Default::default(),
                    flags: Default::default(),
                }
            }))
        }
    }

    /// The main window as the defect saw it: 1.75 scale, maximized, at a
    /// position, showing a shell without panels; the native calls so far
    /// are dropped.
    fn recorded_main_window() -> (
        ComponentInstance,
        std::sync::mpsc::SyncSender<crate::ui::composition::JsonCallback>,
    ) {
        slint::platform::set_platform(Box::new(Recording)).unwrap();
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let old = crate::ui::composition::compile_json(&[], sender.clone(), 1).unwrap();
        let window = old.window();
        window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
            scale_factor: 1.75,
        });
        window.set_size(slint::PhysicalSize::new(2572, 1673));
        window.set_position(slint::PhysicalPosition::new(120, 64));
        old.show().unwrap();
        window.set_maximized(true);
        native_calls();
        (old, sender)
    }

    /// The recording platform for this thread's windows (other modules'
    /// tests of window behaviour); returns a function that takes the native
    /// calls made so far.
    pub(crate) fn recording_platform() -> fn() -> Vec<String> {
        slint::platform::set_platform(Box::new(Recording)).unwrap();
        native_calls
    }

    fn video_width(shell: &ComponentInstance) -> f32 {
        settle_layout(shell.window());
        match shell.get_property("video-width").unwrap() {
            slint_interpreter::Value::Number(n) => n as f32,
            other => panic!("video-width: {other:?}"),
        }
    }

    /// A new composition (here: a bar and a sidebar panel mounted) replaces
    /// the tree inside the same native window: no new window, nothing
    /// hidden, moved or resized; size, position, scale and maximized state
    /// stay, and the new tree is laid out at the window's size.
    #[test]
    fn shell_swap_keeps_the_native_window() {
        let (old, sender) = recorded_main_window();
        let logical = 2572. / 1.75;
        assert!((video_width(&old) - logical).abs() < 1.);
        let old_tree = ShownTree::of(old.window()).unwrap();
        let new = replace_shell(Some(&old), focus_video, |window| {
            let shell = crate::ui::composition::compile_json_in(
                &bar_and_sidebar(),
                false,
                sender.clone(),
                2,
                window,
            )?;
            shell.set_property("sidebar-visible", true.into())?;
            Ok(shell)
        })
        .unwrap();
        drop(old);
        // The one call is Slint's `show()` on a window already visible (the
        // winit backend returns at once: `WinitWindowAdapter::set_visibility`).
        assert_eq!(native_calls(), ["set_visible true"]);
        let window = new.window();
        assert!(!old_tree.is_shown_in(window));
        assert!(ShownTree::of(window).is_some());
        assert!(window.is_visible());
        assert!(window.is_maximized());
        assert!(canvas_focused(&new));
        assert_eq!(window.scale_factor(), 1.75);
        assert_eq!(window.size(), slint::PhysicalSize::new(2572, 1673));
        assert_eq!(window.position(), slint::PhysicalPosition::new(120, 64));
        // The sidebar (317px) and its 3px splitter.
        assert!((video_width(&new) - (logical - 320.)).abs() < 1.);
        // In fullscreen, the panels unmounted again.
        new.window().set_fullscreen(true);
        native_calls();
        let last = replace_shell(Some(&new), focus_video, |window| {
            crate::ui::composition::compile_json_in(&[], false, sender.clone(), 3, window)
        })
        .unwrap();
        drop(new);
        assert_eq!(native_calls(), ["set_visible true"]);
        let window = last.window();
        assert!(window.is_fullscreen());
        assert!(window.is_maximized());
        assert_eq!(window.size(), slint::PhysicalSize::new(2572, 1673));
        assert!((video_width(&last) - logical).abs() < 1.);
        last.hide().unwrap();
    }

    fn canvas_focused(shell: &ComponentInstance) -> bool {
        matches!(
            shell.get_property("canvas-focused"),
            Ok(slint_interpreter::Value::Bool(true))
        )
    }

    /// A shell that fails after Slint attached it to the window (a binding,
    /// a panel's property, `show()`) leaves the old tree shown and working,
    /// with the window's strong reference and the video area's focus back
    /// and no change of the window's state; one that fails before
    /// (compilation) leaves it untouched.
    #[test]
    fn shell_swap_failure_shows_the_old_tree_again() {
        let (old, sender) = recorded_main_window();
        let old_tree = ShownTree::of(old.window()).unwrap();
        let width = video_width(&old);
        assert!(canvas_focused(&old));
        let failed = replace_shell(Some(&old), focus_video, |window| {
            let _shell = crate::ui::composition::compile_json_in(
                &bar_and_sidebar(),
                false,
                sender.clone(),
                2,
                window,
            )?;
            anyhow::bail!("binding failed")
        });
        assert!(failed.is_err());
        assert!(old_tree.is_shown_in(old.window()));
        assert!(canvas_focused(&old));
        // Only the re-show that moves the strong reference back.
        assert_eq!(native_calls(), ["set_visible true"]);
        let failed = replace_shell(Some(&old), focus_video, |_| {
            anyhow::bail!("compilation failed")
        });
        assert!(failed.is_err());
        assert!(old_tree.is_shown_in(old.window()));
        assert!(native_calls().is_empty());
        assert!(old.window().is_visible());
        assert!(old.window().is_maximized());
        old.set_property("sidebar-visible", false.into()).unwrap();
        assert_eq!(video_width(&old), width);
        old.hide().unwrap();
    }

    /// A shell built in a window of its own (`compile_json` instead of
    /// `compile_json_in` with the main window) is refused, and the main
    /// window keeps its tree: the swap never opens a second window.
    #[test]
    fn shell_swap_refuses_a_shell_built_in_another_window() {
        let (old, sender) = recorded_main_window();
        let old_tree = ShownTree::of(old.window()).unwrap();
        let failed = replace_shell(Some(&old), focus_video, |_| {
            crate::ui::composition::compile_json(&bar_and_sidebar(), sender.clone(), 2)
        });
        let error = failed.err().unwrap().to_string();
        assert!(
            error.contains("not built in the window it replaces"),
            "{error}"
        );
        assert!(old_tree.is_shown_in(old.window()));
        // The stray window was created but never shown.
        assert_eq!(native_calls(), ["new window"]);
        old.hide().unwrap();
    }

    /// `prepare_mount_tick` (which needs a whole `Host`) swaps through
    /// `replace_shell` and builds in the window it is given; a change back
    /// to building a new window fails here.
    #[test]
    fn prepare_mount_tick_builds_in_the_main_window() {
        let mounts = include_str!("../host/mounts.rs");
        assert!(
            mounts.contains("view::replace_shell(self.shell.as_ref(), view::focus_video, build)")
        );
        assert!(mounts.contains("composition::compile_json_in("));
        assert!(!mounts.contains("composition::compile_json("));
        assert!(!mounts.contains(".hide()"));
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
        // A window of its own (the headless platform shows every component
        // in one), closed: its adapter is gone.
        use slint::platform::WindowAdapter as _;
        let closed = slint::platform::software_renderer::MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );
        let restore = Restore {
            maximized: true,
            physical: None,
        };
        placements.track(
            closed.window(),
            slint::LogicalPosition::new(1., 2.).into(),
            restore,
        );
        assert!(placements.placing(closed.window()));
        drop(closed);
        placements.tick();
        assert_eq!(placements.pending(), 1);
        assert!(placements.placing(kept.window()));
        for _ in 1..PLACEMENT_TICKS - 1 {
            placements.tick();
        }
        assert_eq!(placements.pending(), 1);
        placements.tick();
        assert_eq!(placements.pending(), 0);
        assert!(!placements.placing(kept.window()));
        kept.hide().unwrap();
    }

    /// A placement follows the window, not its tree: the shell a composition
    /// change builds in the same window (`replace_shell`) before the native
    /// window exists still gets the restored frame.
    #[test]
    fn a_placement_follows_the_window_into_a_new_tree() {
        headless();
        let source = r#"export component W inherits Window { width: 100px; height: 80px; }"#;
        let result = pollster::block_on(
            compiler().build_from_source(source.into(), ui_dir().join("t.slint")),
        );
        let definition = result.component("W").unwrap();
        let first = definition.create().unwrap();
        let mut placements = Placements::default();
        placements
            .show_at(&first, slint::LogicalPosition::new(40., 65.))
            .unwrap();
        let second = definition
            .create_with_existing_window(first.window())
            .unwrap();
        second.show().unwrap();
        drop(first);
        placements.tick();
        assert!(placements.placing(second.window()));
        second.hide().unwrap();
    }

    /// A restored frame stays while a grip of its title bar is on a display;
    /// one on a display that is gone, above the top or too far off an edge
    /// moves onto the primary display, centred and shrunk to fit it.
    #[test]
    fn a_frame_off_every_display_moves_onto_the_primary_one() {
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        let primary = rect(0, 0, 2560, 1600);
        // A second display left of the primary one.
        let left = rect(-1920, 200, 1920, 1080);
        let displays = [primary, left];
        let at = |x, y| rect(x, y, 1200, 700);
        assert_eq!(reachable(at(100, 100), &displays, Some(primary)), None);
        assert_eq!(
            reachable(at(-1500, 300), &displays, Some(primary)),
            None,
            "on the left display"
        );
        assert_eq!(
            reachable(at(-600, 50), &displays, Some(primary)),
            None,
            "across both"
        );
        // Partly off the right edge, 100 px of the title bar still on.
        assert_eq!(reachable(at(2460, 100), &displays, Some(primary)), None);
        assert_eq!(reachable(at(100, 1570), &displays, Some(primary)), None);
        let centred = Some(rect(680, 450, 1200, 700));
        // The display it was on is gone (only the primary one now).
        assert_eq!(
            reachable(at(-1500, 300), &[primary], Some(primary)),
            centred
        );
        // 99 px of the title bar on: not enough.
        assert_eq!(reachable(at(2461, 100), &displays, Some(primary)), centred);
        // The title bar above every display, or below it.
        assert_eq!(reachable(at(100, -40), &displays, Some(primary)), centred);
        assert_eq!(reachable(at(100, 1571), &displays, Some(primary)), centred);
        // In the gap above the left display.
        assert_eq!(reachable(at(-1500, 0), &displays, Some(primary)), centred);
        // The minimized position Windows reports.
        assert_eq!(
            reachable(at(-32000, -32000), &displays, Some(primary)),
            centred
        );
        // Larger than the display it lands on: shrunk to it.
        assert_eq!(
            reachable(rect(5000, 0, 3000, 2000), &displays, Some(primary)),
            Some(primary)
        );
        // No primary display known: the first one.
        assert_eq!(
            reachable(at(9000, 0), &[left], None),
            Some(rect(-1560, 390, 1200, 700))
        );
        // A narrow frame needs only its own width on a display.
        assert_eq!(reachable(rect(2500, 0, 60, 20), &displays, None), None);
        // Displays not listed: the frame stays.
        assert_eq!(reachable(at(-32000, -32000), &[], None), None);
    }

    /// The sources the host generates (main window with panels, floating
    /// panel, options window) compile without warnings, so a Slint upgrade
    /// that deprecates something they use shows up here.
    #[test]
    fn host_generated_sources_compile_without_warnings() {
        let specs = bar_and_sidebar();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let page = crate::ui::settings_window::Page {
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
        tabs.push(crate::ui::composition::CompileSpec {
            owner: 30,
            file: base.join("tab.slint"),
            component: "NoteTab".into(),
            global: Some("Bridge".into()),
            slot: crate::ui::composition::Slot::Sidebar,
        });
        for source in [
            crate::ui::composition::shell_source(&specs).unwrap(),
            crate::ui::composition::shell_source(&tabs).unwrap(),
            crate::ui::composition::floating_source(&tabs[1..], "列表").unwrap(),
            crate::ui::composition::shell_source(&[]).unwrap(),
            crate::ui::composition::floating_source(&specs[1..], "列表").unwrap(),
            crate::ui::settings_window::source(&[&page]).unwrap(),
        ] {
            let result = pollster::block_on(
                compiler().build_from_source(source, ui_dir().join("host.slint")),
            );
            // Known since Slint 1.16 (not new in 1.18): `sidebar-actual`
            // reads the window width that the sidebar's fixed width feeds.
            // Slint names the loop from any of its members (0.5.24's video
            // image change moved its first one), so match the members.
            let diagnostics: Vec<_> = result
                .diagnostics()
                .map(|d| d.to_string())
                .filter(|d| {
                    !(d.contains("part of a binding loop (")
                        && d.contains(
                            "root.layoutinfo-h -> root.sidebar-actual -> sidebar-keys.width",
                        ))
                })
                .collect();
            assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        }
    }

    /// Every plugin and example Slint file compiles with the host's configuration.
    /// A translated label longer than the minimum widens its button.
    #[test]
    fn button_width_follows_its_text_above_the_minimum() {
        headless();
        let source = r#"
import { Button } from "vestra/widgets.slint";
export component W inherits Window {
    HorizontalLayout {
        alignment: start;
        short := Button { text: "OK"; }
        long := Button { text: "Ignore Default Shortcuts and Restart Now"; }
        small := Button { text: "Ignore Default Shortcuts and Restart Now"; small: true; }
        icon := Button { icon: "M 0 0 L 14 6 L 0 12 Z"; }
    }
    out property <length> short-width: short.width;
    out property <length> long-width: long.width;
    out property <length> small-width: small.width;
    out property <length> icon-width: icon.width;
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
        let length = |name: &str| match instance.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        instance
            .window()
            .set_size(slint::LogicalSize::new(900., 100.));
        // The minimum stays; a longer label is not cut off.
        assert_eq!(length("short-width"), 66.);
        assert!(length("long-width") > 150., "{}", length("long-width"));
        assert!(length("small-width") > 150., "{}", length("small-width"));
        assert_eq!(length("icon-width"), 14.);
        // Squeezed, it can still shrink to the old minimum width.
        instance
            .window()
            .set_size(slint::LogicalSize::new(200., 100.));
        assert!(length("long-width") < 100., "{}", length("long-width"));
        assert!(length("long-width") >= 66.);
    }

    #[test]
    fn all_plugin_slint_files_compile() {
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
        assert!(files.len() >= 15, "found {files:?}");
        for file in files {
            let result = pollster::block_on(compiler().build_from_path(&file));
            let errors: Vec<_> = result
                .diagnostics()
                .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
                .map(|d| d.to_string())
                // A library file (only globals or exported components for other
                // files to import) has no component of its own; its importers
                // compile it, so any real error in it still fails here.
                .filter(|e| e != "No component found")
                .collect();
            assert!(errors.is_empty(), "{}: {errors:?}", file.display());
        }
    }

    /// Player 0.5.10: a bottom bar placing `BarButtons` shares the shell's
    /// `BarButtonsHost` (the bar imports the same library file): the strip
    /// reports itself placed and shows the cells that fit in its room, the
    /// sidebar toggle longest and the plugins' lowest order first gone.
    #[test]
    fn bar_buttons_share_the_shell_global_and_drop_the_lowest_order_first() {
        use crate::commands::ButtonIcon;
        use crate::ui::bar_buttons::{GLOBAL, Item, cell};
        use slint_interpreter::Value;
        headless();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let specs = [crate::ui::composition::CompileSpec {
            owner: 10,
            file: base.join("strip-bar.slint"),
            component: "StripBar".into(),
            global: Some("StripBridge".into()),
            slot: crate::ui::composition::Slot::Bottom,
        }];
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::ui::composition::compile_json(&specs, sender.clone(), 1).unwrap();
        assert_eq!(
            shell.get_global_property(GLOBAL, "placed").unwrap(),
            Value::Bool(true)
        );
        // A bar without the strip: the player logs the hidden buttons.
        let plain = crate::ui::composition::compile_json(&bar_and_sidebar(), sender, 2).unwrap();
        assert_eq!(
            plain.get_global_property(GLOBAL, "placed").unwrap(),
            Value::Bool(false)
        );
        let item = |id: &str, icon: &str| Item {
            owner: 3,
            id: id.into(),
            icon: ButtonIcon::Name(icon.into()),
            label: id.into(),
            tip: id.into(),
            enabled: true,
            checkable: false,
            checked: false,
            dimmed: false,
        };
        let cells = vec![
            cell(Some(&item("ai", "ai")), None),
            cell(Some(&item("store", "store")), None),
        ];
        shell
            .set_global_property(
                GLOBAL,
                "buttons",
                Value::Model(slint::ModelRc::new(slint::VecModel::from(cells))),
            )
            .unwrap();
        shell
            .set_global_property(
                GLOBAL,
                "toggle",
                cell(Some(&item("sidebar", "playlist")), None),
            )
            .unwrap();
        shell.window().set_size(slint::LogicalSize::new(800., 600.));
        shell.show().unwrap();
        // The strip's width and the plugin buttons it shows (by id; "toggle"
        // when the sidebar toggle shows too) at this room.
        let strip = |room: f64, toggle: bool| {
            shell
                .set_global_property("Owner10Bridge", "room", Value::Number(room))
                .unwrap();
            settle_layout(shell.window());
            slint::platform::update_timers_and_animations();
            let number = |name| match shell.get_global_property("Owner10Bridge", name).unwrap() {
                Value::Number(n) => n,
                other => panic!("{other:?}"),
            };
            let (width, first) = (number("shown"), number("first") as usize);
            let cells = (width / 41.) as usize;
            let mut shown: Vec<&str> = ["ai", "store"][first..].to_vec();
            shown.truncate(cells - usize::from(toggle && cells > 0));
            if toggle && cells > 0 {
                shown.push("toggle");
            }
            assert_eq!(shown.len(), cells, "room {room}");
            (width, shown)
        };
        // AI (order 100) and Store (200): cells go from the left end inward,
        // the lowest order first, the toggle last.
        assert_eq!(strip(1000., true), (123., vec!["ai", "store", "toggle"]));
        assert_eq!(strip(123., true), (123., vec!["ai", "store", "toggle"]));
        assert_eq!(strip(100., true), (82., vec!["store", "toggle"]));
        assert_eq!(strip(41., true), (41., vec!["toggle"]));
        assert_eq!(strip(40., true), (0., vec![]));
        // Without a toggle the buttons take the room, the lowest order going first.
        shell
            .set_global_property(GLOBAL, "toggle", cell(None, None))
            .unwrap();
        assert_eq!(strip(1000., false), (82., vec!["ai", "store"]));
        assert_eq!(strip(60., false), (41., vec!["store"]));
        assert_eq!(strip(0., false), (0., vec![]));
        shell.hide().unwrap();
    }

    /// A menu driven like the player drives it: the real MenuView, the
    /// host's hover rules (`menu::hover_target`, `menu::HoverAim`, a tick
    /// every 16 ms) and its model updates (`set_menu_model`), drawn by the
    /// software renderer into a reused buffer, as on Windows.
    struct MenuSim {
        window: slint_interpreter::ComponentInstance,
        roots: Vec<crate::commands::MenuNode>,
        path: Vec<String>,
        aim: crate::ui::menu::HoverAim,
        panels: Vec<crate::ui::menu::Rect>,
        levels: serde_json::Value,
        shown: crate::host::ShownMenu,
        /// Each write's counts (`apply_menu_levels`).
        writes: Vec<crate::host::MenuWrite>,
        hovers: std::rc::Rc<std::cell::RefCell<Vec<(usize, String, bool)>>>,
        pointer: crate::ui::menu::Point,
        now: u64,
        buffer: Vec<slint::platform::software_renderer::PremultipliedRgbaColor>,
    }

    const SIM_SIZE: (u32, u32) = (800, 600);

    const SIM_SOURCE: &str = r#"
import { MenuView, MenuLevel } from "menu-view.slint";
export { MenuChrome } from "menu-view.slint";
export { VestraPalette } from "palette.slint";
export component MenuWindow inherits Window {
    width: 800px; height: 600px;
    in property <[MenuLevel]> menu-levels;
    callback menu-hover(int, string, bool);
    MenuView {
        width: root.width; height: root.height;
        levels: root.menu-levels;
        hovered(level, token, now) => { root.menu-hover(level, token, now); }
    }
}"#;

    impl MenuSim {
        fn new() -> Self {
            use crate::commands::{CommandState, Declaration, Registry};
            headless();
            let mut registry = Registry::new();
            let state = CommandState {
                enabled: true,
                checked: None,
                busy: false,
                reason: None,
            };
            let paths: &[&[&str]] = &[
                &["Open File…"],
                &["Video", "Pixel Shaders", "Use Shaders"],
                &["Video", "Deinterlace", "Auto"],
                &["Video", "Aspect", "16:9"],
                &["Video", "Framing", "Zoom In"],
                &["Video", "Image Processing", "Sharpen"],
                &["Video", "Image Processing", "Deband"],
                &["Video", "Rotation", "90°"],
                &["Video", "Snapshot", "Save Current Frame"],
                &["Video", "Picture Adjustments…"],
                &["Video", "Reset"],
                &["Video", "Brightness -1%"],
                &["Video", "Brightness +1%"],
                &["Video", "Contrast -1%"],
                &["Quit"],
            ];
            for (index, path) in paths.iter().enumerate() {
                registry
                    .register(
                        1,
                        "test.menu",
                        format!("c{index}"),
                        1,
                        Declaration::new(path.last().unwrap(), path),
                        state.clone(),
                    )
                    .unwrap();
            }
            let window = crate::ui::composition::build(SIM_SOURCE.into(), "MenuWindow")
                .unwrap()
                .create()
                .unwrap();
            let hovers = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let sink = hovers.clone();
            window
                .set_callback("menu-hover", move |args| {
                    if let [
                        slint_interpreter::Value::Number(level),
                        slint_interpreter::Value::String(token),
                        slint_interpreter::Value::Bool(now),
                    ] = args
                    {
                        sink.borrow_mut()
                            .push((*level as usize, token.to_string(), *now));
                    }
                    slint_interpreter::Value::Void
                })
                .unwrap();
            window
                .window()
                .set_size(slint::PhysicalSize::new(SIM_SIZE.0, SIM_SIZE.1));
            window.show().unwrap();
            let mut sim = Self {
                window,
                roots: registry.snapshot().roots.clone(),
                path: Vec::new(),
                aim: crate::ui::menu::HoverAim::default(),
                panels: Vec::new(),
                levels: serde_json::Value::Null,
                shown: Default::default(),
                writes: Vec::new(),
                hovers,
                pointer: crate::ui::menu::Point { x: 0., y: 0. },
                now: 1000,
                buffer: vec![Default::default(); (SIM_SIZE.0 * SIM_SIZE.1) as usize],
            };
            sim.refresh();
            sim.draw();
            sim
        }
        fn refresh(&mut self) {
            self.path = crate::ui::menu::valid_path(&self.roots, &self.path);
            self.levels = crate::ui::menu::levels(
                &self.roots,
                &self.path,
                crate::ui::menu::Point { x: 20., y: 20. },
                crate::ui::menu::Point {
                    x: f64::from(SIM_SIZE.0),
                    y: f64::from(SIM_SIZE.1),
                },
                &[],
                &|_, _| None,
            );
            self.panels = crate::ui::menu::panels(&self.levels);
            let write = crate::host::apply_menu_levels(&self.window, &mut self.shown, &self.levels)
                .unwrap();
            self.writes.push(write);
        }
        fn set_path(&mut self, path: Vec<String>) {
            self.aim.clear();
            if path != self.path {
                self.path = path;
                self.refresh();
            }
        }
        /// The host loop: the hovers the view reported, as host/handle.rs handles them.
        fn host(&mut self) {
            let hovers: Vec<_> = self.hovers.borrow_mut().drain(..).collect();
            for (level, token, now) in hovers {
                let Some(path) = crate::ui::menu::hover_target(&self.path, level, &token) else {
                    continue;
                };
                if now {
                    self.set_path(path);
                    continue;
                }
                let submenu = (self.path.len() > level)
                    .then(|| self.panels.get(level + 1).copied())
                    .flatten();
                if let Some(path) =
                    self.aim
                        .hover(&self.path, path, Some(self.pointer), submenu, self.now)
                {
                    self.set_path(path);
                }
            }
        }
        /// A host tick (main.rs: the events of the last 16 ms, then
        /// `window_tick`'s `menu_tick`) and a frame.
        fn tick(&mut self) {
            self.host();
            if let Some(path) = self.aim.tick(Some(self.pointer), self.now) {
                self.set_path(path);
            }
            self.draw();
        }
        fn draw(&mut self) {
            slint::platform::update_timers_and_animations();
            let buffer = &mut self.buffer;
            headless_window().draw_if_needed(|renderer| {
                renderer.render(buffer, SIM_SIZE.0 as usize);
            });
        }
        /// A key pressed and released, then a host tick and a frame.
        fn key(&mut self, key: slint::platform::Key) {
            for event in [
                slint::platform::WindowEvent::KeyPressed { text: key.into() },
                slint::platform::WindowEvent::KeyReleased { text: key.into() },
            ] {
                self.window.window().dispatch_event(event);
            }
            self.tick();
        }
        /// One pointer step, 1 ms after the last; a host tick every 16 ms.
        fn move_to(&mut self, x: f64, y: f64) {
            self.pointer = crate::ui::menu::Point { x, y };
            self.window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(x as f32, y as f32),
                });
            slint::platform::update_timers_and_animations();
            self.now += 1;
            if self.now.is_multiple_of(16) {
                self.tick();
            }
        }
        /// Where a row of a level is: its panel's x and the row's middle.
        fn row(&self, level: usize, title: &str) -> (f64, f64) {
            let panel = &self.levels[level];
            let row = panel["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["title"] == title)
                .unwrap_or_else(|| panic!("no row {title}"));
            (
                panel["x"].as_f64().unwrap(),
                panel["y"].as_f64().unwrap() + row["y"].as_f64().unwrap() + 12.,
            )
        }
        /// The rows of a level whose pixel at `dx` from the panel's left
        /// edge differs from the pixel at `against` on the row's middle line.
        fn rows_where(&self, level: usize, dx: f64, against: f64) -> Vec<String> {
            let Some(panel) = self.levels.get(level) else {
                return Vec::new();
            };
            let pixel = |x: f64, y: f64| {
                let p = self.buffer[y as usize * SIM_SIZE.0 as usize + x as usize];
                (p.red, p.green, p.blue)
            };
            panel["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| {
                    let x = panel["x"].as_f64().unwrap();
                    let y = panel["y"].as_f64().unwrap() + row["y"].as_f64().unwrap() + 12.;
                    pixel(x + dx, y) != pixel(x + against, y)
                })
                .map(|row| row["title"].as_str().unwrap().to_owned())
                .collect()
        }
        /// The rows of a level drawn with the 2px accent bar (lit, or with
        /// their submenu open): the bar differs from the row beside it.
        fn marked(&self, level: usize) -> Vec<String> {
            self.rows_where(level, 3., 14.)
        }
        /// The rows of a level lit in full: their background differs from
        /// the panel's (1px inside its border, left of the row).
        fn lit(&self, level: usize) -> Vec<String> {
            self.rows_where(level, 14., 1.)
        }
        /// The level and row under the pointer (deepest panel first), if
        /// the pointer is on a row.
        fn under(&self) -> Option<(usize, String)> {
            let levels = self.levels.as_array()?;
            levels.iter().enumerate().rev().find_map(|(level, panel)| {
                let number = |key: &str| panel[key].as_f64().unwrap();
                let (x, y) = (self.pointer.x, self.pointer.y);
                if x < number("x") + 2. || x >= number("x") + number("width") - 2. {
                    return None;
                }
                panel["rows"].as_array()?.iter().find_map(|row| {
                    let top = number("y") + row["y"].as_f64().unwrap();
                    (y >= top && y < top + 24.)
                        .then(|| (level, row["title"].as_str().unwrap().to_owned()))
                })
            })
        }
        /// After a frame: at most one row lit in full per level; with the
        /// pointer on a row, that row lit at once and exactly one row lit
        /// on each level above it (the open row leading to it); the bar
        /// only on the lit row and the row whose submenu is open.
        fn check(&self, context: &str) {
            let under = self.under();
            for level in 0..self.levels.as_array().map_or(0, Vec::len) {
                let lit = self.lit(level);
                assert!(lit.len() <= 1, "{context}: level {level} lit {lit:?}");
                if let Some((cursor, _)) = &under
                    && level < *cursor
                {
                    assert_eq!(
                        lit.iter().collect::<Vec<_>>(),
                        self.path.get(level).into_iter().collect::<Vec<_>>(),
                        "{context}: level {level} above the pointer's keeps its open row lit"
                    );
                }
                let mut marked: Vec<&String> = lit.iter().chain(self.path.get(level)).collect();
                marked.sort();
                marked.dedup();
                let mut drawn = self.marked(level);
                drawn.sort();
                assert_eq!(
                    drawn.iter().collect::<Vec<_>>(),
                    marked,
                    "{context}: level {level} bars (path {:?})",
                    self.path
                );
            }
            if let Some((level, row)) = under {
                assert_eq!(self.lit(level), [row], "{context}: the pointer's row");
            }
        }
    }

    /// Open Video, sweep its submenu up and down three times at `step` px
    /// per 1 ms (starting `phase` ms into a 16 ms host tick) and rest on
    /// Reset; the open path, the rows with a bar in both levels and the
    /// rows lit in full in both levels.
    #[allow(clippy::type_complexity)]
    fn sweep(step: f64, phase: u64) -> (Vec<String>, [Vec<String>; 2], [Vec<String>; 2]) {
        let mut sim = MenuSim::new();
        sim.now += phase;
        let (x, y) = sim.row(0, "Video");
        for _ in 0..20 {
            sim.move_to(x + 31., y);
            sim.move_to(x + 30., y);
        }
        assert_eq!(sim.path, ["Video"]);
        let (sub_x, top) = sim.row(1, "Pixel Shaders");
        let (_, bottom) = sim.row(1, "Contrast -1%");
        let (_, reset) = sim.row(1, "Reset");
        let x = sub_x + 40.;
        for _ in 0..3 {
            let mut y = top;
            while y < bottom {
                sim.move_to(x, y);
                y += step;
            }
            while y > top {
                sim.move_to(x, y);
                y -= step;
            }
        }
        let mut y = top;
        while y < reset {
            sim.move_to(x, y);
            y += step;
        }
        // Rest there: a held switch (G-15) settles, frames are drawn.
        for _ in 0..400 {
            sim.move_to(x, reset);
        }
        sim.draw();
        let result = (
            sim.path.clone(),
            [sim.marked(0), sim.marked(1)],
            [sim.lit(0), sim.lit(1)],
        );
        sim.window.hide().unwrap();
        result
    }

    /// User report (0.5.9, Windows, software rendering): sweeping a submenu
    /// up and down at about 6000 px/s left rows that open a submenu
    /// ("Image Processing") drawn lit beside the row under the pointer, with
    /// their submenus closed. Like native menus, the row under the pointer
    /// and the open parents leading to it are lit, one per level.
    /// Before the fix every combination below left one to five such rows.
    #[test]
    fn a_fast_sweep_leaves_only_the_pointer_row_and_open_parents_lit() {
        let mut failures = Vec::new();
        for step in [2., 6., 14.] {
            for phase in [0, 5, 11] {
                let (path, marked, lit) = sweep(step, phase);
                if path != ["Video"]
                    || marked != [vec!["Video".to_owned()], vec!["Reset".to_owned()]]
                    || lit != [vec!["Video".to_owned()], vec!["Reset".to_owned()]]
                {
                    failures.push(format!(
                        "step {step} phase {phase}: path {path:?} bars {marked:?} lit {lit:?}"
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Open Video ▸ Image Processing, then move the pointer from its row
    /// toward its submenu across Rotation (a held switch, G-15), checking
    /// every frame (`MenuSim::check`). Returns the sim and where the
    /// pointer stopped, on the Rotation row.
    fn hold_across_rotation() -> (MenuSim, (f64, f64)) {
        let mut sim = MenuSim::new();
        let (x, y) = sim.row(0, "Video");
        for _ in 0..20 {
            sim.move_to(x + 30., y);
        }
        let (sub_x, ip) = sim.row(1, "Image Processing");
        let (_, rotation) = sim.row(1, "Rotation");
        assert_eq!(rotation - ip, 24., "Rotation is the row below");
        let width = sim.levels[1]["width"].as_f64().unwrap();
        for _ in 0..20 {
            sim.move_to(sub_x + 40., ip);
        }
        sim.draw();
        assert_eq!(sim.path, ["Video", "Image Processing"]);
        assert_eq!(sim.lit(1), ["Image Processing"]);
        // Toward the submenu's near edge, 1 px down per 2 px right, into
        // the Rotation row (the submenu spans Image Processing's height
        // and the next row: the move heads for it).
        let target = (sub_x + width - 4., rotation + 4.);
        let steps = 40;
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            sim.move_to(
                sub_x + 40. + (target.0 - sub_x - 40.) * t,
                ip + (target.1 - ip) * t,
            );
            sim.draw();
            sim.check(&format!("step {step}"));
        }
        (sim, target)
    }

    /// 0.6.0 user ruling (Windows menus): the row under the pointer is lit
    /// at once, even while a switch away from the open submenu is held
    /// (G-15); the open row of its level keeps only the accent bar, the
    /// open row of the level above stays lit. Before, the lit row waited
    /// for the held switch (up to `AIM_GRACE_MS` and a tick).
    #[test]
    fn during_a_held_switch_the_pointer_row_is_lit_and_the_open_parent_marked() {
        let (mut sim, _) = hold_across_rotation();
        assert_eq!(
            sim.path,
            ["Video", "Image Processing"],
            "the switch is held"
        );
        assert_eq!(sim.lit(0), ["Video"]);
        assert_eq!(sim.lit(1), ["Rotation"]);
        assert_eq!(sim.marked(1), ["Image Processing", "Rotation"]);
        // Back on the open row: the hold is cancelled, one row lit.
        let (sub_x, ip) = sim.row(1, "Image Processing");
        for step in 0..40 {
            sim.move_to(sub_x + 40., ip);
            sim.draw();
            sim.check(&format!("back {step}"));
        }
        assert_eq!(sim.path, ["Video", "Image Processing"]);
        assert_eq!(sim.lit(1), ["Image Processing"]);
        assert_eq!(sim.marked(1), ["Image Processing"]);
        assert_eq!(sim.lit(0), ["Video"]);
        assert_eq!(sim.marked(0), ["Video"]);
    }

    /// A held switch released (the pointer rests on Rotation): Rotation
    /// stays the lit row, its submenu opens and the bar leaves Image
    /// Processing.
    #[test]
    fn a_released_switch_opens_the_lit_row() {
        let (mut sim, target) = hold_across_rotation();
        for step in 0..(crate::ui::menu::AIM_GRACE_MS * 3) {
            sim.move_to(target.0, target.1);
            sim.draw();
            sim.check(&format!("rest {step}"));
        }
        assert_eq!(sim.path, ["Video", "Rotation"], "the held switch happened");
        assert_eq!(sim.lit(1), ["Rotation"]);
        assert_eq!(sim.marked(1), ["Rotation"]);
        assert_eq!(sim.lit(2), Vec::<String>::new());
    }

    /// The pointer reaches the held submenu: the switch is cancelled, the
    /// submenu lights the row under the pointer and the parent chain
    /// (Video, Image Processing) stays lit, as in Windows and macOS menus.
    #[test]
    fn reaching_the_submenu_lights_its_row_and_keeps_the_parents_lit() {
        let (mut sim, target) = hold_across_rotation();
        let (sub2_x, deband) = sim.row(2, "Deband");
        let steps = 12;
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            sim.move_to(
                target.0 + (sub2_x + 30. - target.0) * t,
                target.1 + (deband - target.1) * t,
            );
            sim.draw();
            sim.check(&format!("step {step}"));
        }
        for step in 0..(crate::ui::menu::AIM_GRACE_MS * 2) {
            sim.move_to(sub2_x + 30., deband);
            sim.draw();
            sim.check(&format!("rest {step}"));
        }
        assert_eq!(sim.path, ["Video", "Image Processing"]);
        assert_eq!(sim.marked(0), ["Video"]);
        assert_eq!(sim.lit(0), ["Video"]);
        assert_eq!(sim.lit(1), ["Image Processing"]);
        assert_eq!(sim.marked(1), ["Image Processing"]);
        assert_eq!(sim.lit(2), ["Deband"]);
        assert_eq!(sim.marked(2), ["Deband"]);
    }

    /// The keyboard: Right into a submenu keeps its parent lit; Down on
    /// that level moves the one lit row; Left back and Down past the open
    /// row leaves one lit row and the bar on the open one.
    #[test]
    fn keys_keep_one_lit_row_per_level_and_the_parent_lit() {
        let mut sim = MenuSim::new();
        let title = |sim: &MenuSim, level: usize, index: usize| {
            sim.levels[level]["rows"][index]["title"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        for _ in 0..10 {
            if sim.lit(0) == ["Video"] {
                break;
            }
            sim.key(slint::platform::Key::DownArrow);
        }
        assert_eq!(sim.lit(0), ["Video"]);
        sim.key(slint::platform::Key::RightArrow);
        assert_eq!(sim.path, ["Video"]);
        let first = title(&sim, 1, 0);
        assert_eq!(sim.lit(0), ["Video"], "Right keeps the parent lit");
        assert_eq!(sim.lit(1), std::slice::from_ref(&first));
        sim.key(slint::platform::Key::DownArrow);
        let second = title(&sim, 1, 1);
        assert_eq!(
            sim.levels[1]["rows"][1]["has-children"], true,
            "{second} is a group"
        );
        assert_eq!(sim.lit(0), ["Video"]);
        assert_eq!(
            sim.lit(1),
            std::slice::from_ref(&second),
            "Down moves the one lit row"
        );
        // Into the second row's submenu and back: it stays open (the sim
        // does not close it on Left), the cursor on it.
        sim.key(slint::platform::Key::RightArrow);
        assert_eq!(sim.path, ["Video".to_owned(), second.clone()]);
        assert_eq!(sim.lit(1), std::slice::from_ref(&second));
        assert_eq!(sim.lit(2), [title(&sim, 2, 0)]);
        sim.key(slint::platform::Key::LeftArrow);
        assert_eq!(sim.lit(1), std::slice::from_ref(&second));
        assert_eq!(sim.lit(2), Vec::<String>::new());
        // Down past the open row: one lit row, the bar stays on the open one.
        sim.key(slint::platform::Key::DownArrow);
        let third = title(&sim, 1, 2);
        assert_eq!(sim.path, ["Video".to_owned(), second.clone()]);
        assert_eq!(sim.lit(0), ["Video"]);
        assert_eq!(sim.lit(1), std::slice::from_ref(&third));
        let mut marked = vec![second, third];
        marked.sort();
        let mut drawn = sim.marked(1);
        drawn.sort();
        assert_eq!(drawn, marked);
    }

    /// Host 0.5.26: a hover writes only the levels it changes. Before, every
    /// submenu switch converted, compared and wrote the whole menu and
    /// settled the window (about 3 ms a hover on the user's machine).
    #[test]
    fn a_menu_hover_writes_only_what_it_changes() {
        use crate::host::MenuWrite;
        let mut sim = MenuSim::new();
        assert!(sim.writes[0].full, "the first write installs the model");
        sim.set_path(vec!["Video".into()]);
        let opened = *sim.writes.last().unwrap();
        assert!(!opened.full && opened.settled, "{opened:?}");
        // Root: its Video row and its `open`; the Video level is added.
        assert_eq!((opened.rows, opened.levels), (1, 2), "{opened:?}");
        sim.set_path(vec!["Video".into(), "Pixel Shaders".into()]);
        sim.draw();
        sim.set_path(vec!["Video".into(), "Deinterlace".into()]);
        let switched = *sim.writes.last().unwrap();
        // The Video level: two rows' `open` and its own; the submenu: its
        // one row and its position. The root level is not touched.
        assert_eq!(
            switched,
            MenuWrite {
                full: false,
                rows: 3,
                levels: 2,
                settled: true
            }
        );
        // A republished state that changes nothing writes nothing.
        let count = sim.writes.len();
        sim.refresh();
        assert_eq!(sim.writes[count], MenuWrite::default());
        // A changed command state writes its row, without a settle.
        let mut levels = sim.levels.clone();
        levels[2]["rows"][0]["checked"] = true.into();
        let checked = crate::host::apply_menu_levels(&sim.window, &mut sim.shown, &levels).unwrap();
        assert_eq!(
            checked,
            MenuWrite {
                full: false,
                rows: 1,
                levels: 0,
                settled: false
            }
        );
        sim.draw();
        // Closing the submenu removes its level.
        sim.set_path(vec!["Video".into()]);
        let closed = *sim.writes.last().unwrap();
        assert_eq!((closed.full, closed.levels), (false, 2), "{closed:?}");
        // What is drawn is what a whole write draws.
        sim.draw();
        let incremental = sim.buffer.clone();
        crate::host::set_menu_model(
            &sim.window,
            crate::ui::ui_values::from_json(&sim.levels).unwrap(),
        )
        .unwrap();
        headless_window().request_redraw();
        sim.draw();
        let rgba = |buffer: &[slint::platform::software_renderer::PremultipliedRgbaColor]| {
            buffer
                .iter()
                .map(|p| (p.red, p.green, p.blue, p.alpha))
                .collect::<Vec<_>>()
        };
        assert!(
            rgba(&incremental) == rgba(&sim.buffer),
            "the same pixels as a whole write"
        );
        sim.window.hide().unwrap();
    }

    /// Perf probe (0.5.26): hovers that switch submenus in the real shell,
    /// the whole write (before) against `apply_menu_levels` (after). Run with
    /// `VESTRA_MENU_PROBE=1 cargo test --release menu_hover_probe -- --ignored --nocapture`.
    #[test]
    #[ignore = "perf probe"]
    fn menu_hover_probe() {
        use crate::commands::{CommandState, Declaration, Registry};
        use crate::ui::menu::{Point, levels};
        use std::time::Instant;
        if std::env::var_os("VESTRA_MENU_PROBE").is_none() {
            return;
        }
        headless();
        // A menu the size of the player's: 9 groups of 12 commands and 2
        // submenus of 8.
        let mut registry = Registry::new();
        let state = CommandState {
            enabled: true,
            checked: None,
            busy: false,
            reason: None,
        };
        let mut index = 0;
        let mut add = |path: Vec<String>| {
            let title = path.last().unwrap().clone();
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            registry
                .register(
                    1 + index / 100,
                    "test.menu",
                    format!("c{index}"),
                    1,
                    Declaration::new(&title, &path),
                    state.clone(),
                )
                .unwrap();
            index += 1;
        };
        for g in 0..9 {
            for c in 0..12 {
                add(vec![format!("Group {g}"), format!("Command {g}.{c}")]);
            }
            for s in 0..2 {
                for c in 0..8 {
                    add(vec![
                        format!("Group {g}"),
                        format!("Sub {g}.{s}"),
                        format!("Item {g}.{s}.{c}"),
                    ]);
                }
            }
        }
        let roots = registry.snapshot().roots.clone();
        let (sender, _receiver) = std::sync::mpsc::sync_channel(8);
        let shell = crate::ui::composition::compile_json(&bar_and_sidebar(), sender, 1).unwrap();
        let (w, h) = (1280u32, 720u32);
        shell.window().set_size(slint::PhysicalSize::new(w, h));
        shell.set_property("menu-open", true.into()).unwrap();
        shell.show().unwrap();
        let mut buffer = vec![
            slint::platform::software_renderer::PremultipliedRgbaColor::default();
            (w * h) as usize
        ];
        let mut draw = || {
            slint::platform::update_timers_and_animations();
            headless_window().draw_if_needed(|renderer| {
                renderer.render(&mut buffer, w as usize);
            });
        };
        // Per group: switch to it (the open group changes), open a
        // submenu, switch to its sibling, republish unchanged, close it.
        const KINDS: [&str; 5] = [
            "switch group",
            "open submenu",
            "switch submenu",
            "republish",
            "close submenu",
        ];
        let paths: Vec<Vec<String>> = (0..300)
            .map(|i| {
                let g = (i / 5) % 9;
                match i % 5 {
                    0 | 4 => vec![format!("Group {g}")],
                    1 => vec![format!("Group {g}"), format!("Sub {g}.0")],
                    _ => vec![format!("Group {g}"), format!("Sub {g}.1")],
                }
            })
            .collect();
        let origin = Point { x: 100., y: 50. };
        let size = Point {
            x: f64::from(w),
            y: f64::from(h),
        };
        let none = |_: u64, _: &str| None;
        let ms = |since: Instant| since.elapsed().as_secs_f64() * 1000.;
        let report = |name: &str, samples: &mut Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            let at = |p: f64| samples[((samples.len() - 1) as f64 * p).round() as usize];
            diag!(
                "{name}: p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms",
                at(0.5),
                at(0.95),
                at(1.)
            );
        };
        let mut project = Vec::new();
        // [round][kind]: the write, and the write with the next frame.
        let mut write = vec![vec![Vec::new(); KINDS.len()]; 2];
        let mut frame = vec![vec![Vec::new(); KINDS.len()]; 2];
        for round in 0..2 {
            shell
                .set_property(
                    "menu-levels",
                    Value::Model(slint::ModelRc::new(slint::VecModel::<Value>::default())),
                )
                .unwrap();
            let mut shown = crate::host::ShownMenu::default();
            for (i, path) in paths.iter().enumerate() {
                let start = Instant::now();
                let value = levels(&roots, path, origin, size, &[], &none);
                project.push(ms(start));
                let start = Instant::now();
                if round == 0 {
                    crate::host::set_menu_model(
                        &shell,
                        crate::ui::ui_values::from_json(&value).unwrap(),
                    )
                    .unwrap();
                } else {
                    crate::host::apply_menu_levels(&shell, &mut shown, &value).unwrap();
                }
                write[round][i % 5].push(ms(start));
                draw();
                frame[round][i % 5].push(ms(start));
            }
        }
        report("menu::levels", &mut project);
        for (kind, name) in KINDS.iter().enumerate() {
            for (round, label) in ["before", "after"].iter().enumerate() {
                report(&format!("{name} write ({label})"), &mut write[round][kind]);
                report(
                    &format!("{name} write+frame ({label})"),
                    &mut frame[round][kind],
                );
            }
        }
        shell.hide().unwrap();
    }
}
