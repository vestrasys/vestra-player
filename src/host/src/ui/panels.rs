//! Plugin panels, each compiled on its own and embedded in the shell (or
//! the sidebar's floating window) through Slint's `ComponentContainer`
//!.
//!
//! The shell is compiled once and names no panel: each slot (the bottom
//! bar, the backdrop, the sidebar, the adjust column) is a container with a
//! `<slot>-factory` property. The host sets it to the shown panel's factory
//! ([`Panels::show`]); Slint asks the factory for an instance when the
//! container exists and whenever the factory changes.
//!
//! - **Lazy**: a panel's file is compiled when the panel is first shown,
//!   not when it is mounted. A tab in the background costs nothing until
//!   the user switches to it.
//! - **Instances stay**: a panel keeps the instance it gave a container
//!   and hands the same one back when that container asks again, so a
//!   switch between tabs that were shown before creates nothing and each
//!   tab keeps its own state (scroll positions, open sections).
//! - **Own globals**: a panel compiled on its own has its own palette,
//!   focus global and bridge. What the shell used to give every panel at
//!   once is given to each instance as it is created ([`Panel::create`]):
//!   palette and fonts, the bridge's models and callbacks,
//!   `VestraFocus.release`, `tabbed`, and every value its plugin has set
//!   ([`Panel::write`] keeps them in Slint form; an array is the same model
//!   object in the store and in the instances, so a row patch applied to
//!   the stored value is what they show).
//! - **Bounded**: at most [`MAX_LOADED`] panels per column keep a compiled
//!   definition and instances (the least recently shown one is unloaded,
//!   [`Panels::evict`]); compiled files are shared through a cache that is
//!   checked against every file the compile read ([`Compiled::stamps`]) and
//!   keeps at most [`SPARE_DEFINITIONS`] definitions no panel uses.
//!
//! Experimental Slint API (1.18.1): `ComponentContainer` and
//! `component-factory` are enabled for the shell's own source only
//! (`composition::build_shell`), never for a plugin's file, which is compiled
//! with the ordinary compiler (`composition::build_panel`).
//! `slint::ComponentFactory` and `ComponentDefinition::create_embedded` are
//! hidden API: recheck on every Slint upgrade (the `ui::panels::tests` fail
//! if the behaviour changes).
#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use i_slint_core::component_factory::FactoryContext;
use i_slint_core::item_tree::ItemTreeWeak;
use slint::ComponentHandle as _;
use slint_interpreter::{ComponentDefinition, ComponentInstance, Value};

use crate::ui::composition::{self, CompileSpec, JsonCallback, Slot};

/// The name a panel's bridge global is exported under in its own
/// compilation.
pub const BRIDGE: &str = "PanelBridge";
/// The component of a panel's compilation: a `Window` around the plugin's
/// component (a container gives a window root its own size).
pub const COMPONENT: &str = "VestraPanel";
/// A bottom bar's wrapper: the width its row keeps for its own controls
/// (`composition::BAR_FULL_WIDTH`, else `BAR_MIN_WIDTH`, else 0), in px.
pub const RESERVE: &str = "vestra-reserve";
/// A bottom bar's wrapper: the bar places the deprecated `BarButtons`.
pub const PLACED: &str = "vestra-placed";
/// A bottom bar's wrapper: the strip's room beside the bar, which it
/// publishes to the bar's own `BarButtonsHost.room` when it changes; the
/// first value is the host's to copy ([`mirror_bar`]): an `init` runs while
/// Slint creates the instance, before its container gives the root its
/// size, and would publish the room of a bar 0 px wide.
pub const ROOM: &str = "vestra-room";
/// Panels of one column that keep a compiled definition and instances.
pub const MAX_LOADED: usize = 4;
/// Compiled files kept though no panel uses them (a plugin that unmounts
/// and mounts again, a reload): the most recent ones.
pub const SPARE_DEFINITIONS: usize = 2;

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.
}

/// One panel as a compilation of its own. A bottom bar (`reserve`: its
/// width property, if any) also exports the strip's global, which the host
/// fills as it fills the shell's, reports its reserved width and whether
/// it places the deprecated `BarButtons`, and publishes the strip's room
/// to its own `BarButtonsHost` (as the shell's strip does for the shell's).
pub fn panel_source(spec: &CompileSpec, reserve: Option<&str>) -> Result<String> {
    composition::identifier(&spec.component)?;
    let path = composition::import_path(&spec.file)?;
    let mut source = format!(
        "import {{ {} as PanelView }} from \"{path}\";\n",
        spec.component
    );
    if let Some(global) = &spec.global {
        composition::identifier(global)?;
        source.push_str(&format!(
            "import {{ {global} as {BRIDGE} }} from \"{path}\";\nexport {{ {BRIDGE} }}\n"
        ));
    }
    source.push_str(&crate::ui::theme::export_line());
    source.push_str("export { VestraFocus } from \"@vestra/focus.slint\";\n");
    let mut bar = String::new();
    if spec.slot == Slot::Bottom {
        source.push_str(
            "import { BarButtonsHost } from \"@vestra/bar-buttons.slint\";\nexport { BarButtonsHost }\n",
        );
        let reserved = reserve.map_or_else(|| "0px".to_owned(), |p| format!("view.{p}"));
        bar = format!(
            "out property <float> {RESERVE}: {reserved} / 1px; \
             out property <bool> {PLACED}: BarButtonsHost.placed; \
             out property <length> {ROOM}: max(0px, root.width - {reserved} - 4px); \
             changed {ROOM} => {{ BarButtonsHost.room = self.{ROOM}; }}              {BAR_CHECK} "
        );
    }
    source.push_str(&format!(
        "export component {COMPONENT} inherits Window {{ background: transparent; {bar}\
         view := PanelView {{ width: parent.width; height: parent.height; }} }}\n"
    ));
    Ok(source)
}

/// A bottom bar laid out as the shell lays it out (`composition`: the
/// slot is as high as the bar's own height constraints want), never
/// created (`if false`): a bar whose height constraints depend on its own
/// height is a binding loop the compiler reports here, so its mount fails
/// with `ui-prepare` as it did when the bar was compiled into the shell.
/// In the shell the same bar sits in a container, where the compiler
/// cannot see the loop: Slint would end the player with "Recursion
/// detected" at run time.
const BAR_CHECK: &str = "if false: Rectangle {     vestra-bar-check-keys := Rectangle {     height: min(vestra-bar-check.max-height, max(vestra-bar-check.min-height, vestra-bar-check.preferred-height));     vestra-bar-check := PanelView { x: 0px; y: 0px; width: parent.width; height: parent.height; } } }";

/// The bottom bar's own compilation reports the width it keeps for its own
/// controls and whether it places the deprecated `BarButtons` ([`RESERVE`],
/// [`PLACED`]): copied to the shell's strip (`bottom-reserve`, its
/// `BarButtonsHost.placed`) when they changed; no bar: 0 and `false`. The
/// bar's own `BarButtonsHost.room` gets the strip's room ([`ROOM`]).
pub(crate) fn mirror_bar(shell: &ComponentInstance, bar: Option<&ComponentInstance>) {
    use crate::ui::bar_buttons::GLOBAL;
    let read = |name: &str| bar.and_then(|bar| bar.get_property(name).ok());
    let reserve = read(RESERVE).unwrap_or(Value::Number(0.));
    if shell.get_property("bottom-reserve").ok().as_ref() != Some(&reserve) {
        let _ = shell.set_property("bottom-reserve", reserve);
    }
    let placed = read(PLACED).unwrap_or(Value::Bool(false));
    if shell.get_global_property(GLOBAL, "placed").ok().as_ref() != Some(&placed) {
        let _ = shell.set_global_property(GLOBAL, "placed", placed);
    }
    // The bar's own strip global gets its first room (later ones come with
    // the wrapper's `changed`).
    if let (Some(bar), Some(room)) = (bar, read(ROOM))
        && bar.get_global_property(GLOBAL, "room").ok().as_ref() != Some(&room)
    {
        let _ = bar.set_global_property(GLOBAL, "room", room);
    }
}

/// A compiled panel file.
pub(crate) struct Compiled {
    file: PathBuf,
    component: String,
    global: Option<String>,
    /// Compiled as a bottom bar (its wrapper differs).
    bar: bool,
    /// Every file the compile read, with its modification time then: a
    /// change to any of them (the panel's file, a file it imports) makes
    /// the next mount compile again.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    pub(crate) definition: ComponentDefinition,
}

fn stamp(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Compiled {
    fn is(&self, file: &std::path::Path, spec: &CompileSpec) -> bool {
        self.file == file
            && self.component == spec.component
            && self.global == spec.global
            && self.bar == (spec.slot == Slot::Bottom)
    }
    fn current(&self) -> bool {
        self.stamps.iter().all(|(path, at)| stamp(path) == *at)
    }
    /// Compile `spec`'s panel. A bottom bar's width property is whichever
    /// its component has (`composition::BAR_FULL_WIDTH`, else
    /// `BAR_MIN_WIDTH`, else none): the wrapper that reads it compiles
    /// exactly when it exists, is readable and is a length, so the
    /// compiler answers (also for a property of a base component in
    /// another file), in the compile that is needed anyway.
    fn compile(file: PathBuf, spec: &CompileSpec) -> Result<Self> {
        let bar = spec.slot == Slot::Bottom;
        let variants: &[Option<&str>] = if bar {
            &[
                Some(composition::BAR_FULL_WIDTH),
                Some(composition::BAR_MIN_WIDTH),
                None,
            ]
        } else {
            &[None]
        };
        let mut last = None;
        for reserve in variants {
            match composition::build_panel(panel_source(spec, *reserve)?) {
                Ok((definition, paths)) => {
                    return Ok(Self {
                        file,
                        component: spec.component.clone(),
                        global: spec.global.clone(),
                        bar,
                        stamps: paths
                            .into_iter()
                            .map(|path| {
                                let at = stamp(&path);
                                (path, at)
                            })
                            .collect(),
                        definition,
                    });
                }
                // The check of `BAR_CHECK` failed: say what it means.
                Err(error) if bar && format!("{error:#}").contains("vestra-bar-check") => {
                    last = Some(error.context(
                        "the bottom bar's height constraints depend on its own height (a binding loop \
                         with the bar's place, which is as high as the bar wants)",
                    ));
                }
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("panel compilation")))
    }
}

/// An instance a panel gave a container, kept for when it asks again.
struct Kept {
    /// The container: the item tree that holds it and its place there.
    parent: ItemTreeWeak,
    index: u32,
    generation: u64,
    /// Unique among all instances of the run (`Panels::bar_instance`: a
    /// new bar instance gets the strip's global filled).
    serial: u64,
    instance: ComponentInstance,
}

impl Kept {
    /// Its container still exists (a column that closed, a fullscreen
    /// overlay that went, a closed floating window take theirs along).
    fn live(&self) -> bool {
        self.parent.upgrade().is_some()
    }
}

thread_local! {
    static SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// One mounted panel: its compiled definition (once shown), what its
/// plugin has set, and its instances.
pub(crate) struct Panel {
    /// As mounted; `spec.owner` is the key its callbacks carry (the key it
    /// was mounted under: after its plugin woke or restarted under another
    /// owner, until its new run mounts it again, its callbacks name a
    /// panel that is no longer mounted and are dropped, as before).
    pub(crate) spec: CompileSpec,
    compiled: RefCell<Option<Rc<Compiled>>>,
    values: RefCell<BTreeMap<String, Value>>,
    tabbed: Cell<bool>,
    kept: RefCell<Vec<Kept>>,
    /// One factory per window generation (a factory is compared by
    /// identity: setting the same one again creates nothing).
    #[allow(deprecated)]
    factories: RefCell<BTreeMap<u64, slint::ComponentFactory>>,
    /// Values a new instance could not take (`Panels::take_refused`).
    refused: RefCell<Vec<String>>,
    /// Instances created so far.
    pub(crate) created: Cell<u32>,
    /// When it was last shown (`Panels::clock`).
    used: Cell<u64>,
}

impl Panel {
    fn new(spec: CompileSpec) -> Self {
        Self {
            spec,
            compiled: RefCell::default(),
            values: RefCell::default(),
            tabbed: Cell::new(false),
            kept: RefCell::default(),
            factories: RefCell::default(),
            refused: RefCell::default(),
            created: Cell::new(0),
            used: Cell::new(0),
        }
    }

    /// Its instances whose container exists (shown, or a tab in the
    /// background of a column that is open).
    pub(crate) fn instances(&self) -> Vec<ComponentInstance> {
        self.kept
            .borrow()
            .iter()
            .filter(|kept| kept.live())
            .map(|kept| kept.instance.clone_strong())
            .collect()
    }

    /// Its instance in the window of `generation`, with its serial.
    pub(crate) fn instance_in(&self, generation: u64) -> Option<(u64, ComponentInstance)> {
        self.kept
            .borrow()
            .iter()
            .rev()
            .find(|kept| kept.generation == generation && kept.live())
            .map(|kept| (kept.serial, kept.instance.clone_strong()))
    }

    /// Whether its file is compiled (it was shown and not unloaded since).
    pub(crate) fn loaded(&self) -> bool {
        self.compiled.borrow().is_some()
    }

    fn has_bridge(&self) -> bool {
        self.spec.global.is_some()
    }

    /// Whether the bridge declares `property`; not compiled yet: checked
    /// when its first instance takes the value (`refused`).
    fn declares(&self, property: &str) -> bool {
        self.compiled.borrow().as_ref().is_none_or(|compiled| {
            compiled
                .definition
                .global_properties(BRIDGE)
                .is_some_and(|mut all| all.any(|(name, _)| name == property))
        })
    }

    /// The stored value of `property`: the instances' own (the same model
    /// object), for row patches.
    pub(crate) fn current(&self, property: &str) -> Option<Value> {
        self.values.borrow().get(property).cloned()
    }

    /// `ui.setMount` (and a mount's initial properties): write `value` to
    /// the instances and keep it for the next one.
    pub(crate) fn write(&self, property: &str, value: Value) -> Result<()> {
        if !self.has_bridge() {
            bail!("composition contribution has no exported bridge global");
        }
        let instances = self.instances();
        if instances.is_empty() {
            if !self.declares(property) {
                bail!("{property}: no such property");
            }
            let mut values = self.values.borrow_mut();
            let kept = match values.get(property) {
                Some(current) => crate::ui::ui_values::reconcile(current, &value)?,
                None => false,
            };
            if !kept {
                values.insert(property.to_owned(), value);
            }
            return Ok(());
        }
        for instance in &instances {
            composition::set_alias_value(instance, BRIDGE, property, value.clone())?;
        }
        // What the instances hold now (the model they kept, or the new one).
        let now = instances[0].get_global_property(BRIDGE, property)?;
        self.values.borrow_mut().insert(property.to_owned(), now);
        Ok(())
    }

    /// The host-owned `tabbed` property: `true` while the host draws its
    /// tab strip above the panel, so a panel with a strip of its own hides
    /// it. Set only on bridges that declare it.
    pub(crate) fn set_tabbed(&self, tabbed: bool) {
        if self.tabbed.replace(tabbed) == tabbed {
            return;
        }
        for instance in self.instances() {
            if instance.get_global_property(BRIDGE, "tabbed").is_ok() {
                let _ = instance.set_global_property(BRIDGE, "tabbed", Value::Bool(tabbed));
            }
        }
    }

    /// Unload: its instances, factories and definition go; what its plugin
    /// set stays for when it is shown again.
    fn unload(&self) {
        self.kept.borrow_mut().clear();
        self.factories.borrow_mut().clear();
        self.compiled.borrow_mut().take();
    }

    /// The instance for the container `ctx` of the window of `generation`:
    /// the one it had before, or a new one set up as the shell used to set
    /// up its panels.
    fn instance_for(
        self: &Rc<Self>,
        ctx: FactoryContext,
        generation: u64,
        window: &slint_interpreter::Weak<ComponentInstance>,
        events: &mpsc::SyncSender<JsonCallback>,
        theme: &str,
    ) -> Result<ComponentInstance> {
        if let Some(kept) = self.kept.borrow().iter().find(|kept| {
            kept.index == ctx.parent_item_tree_index
                && ItemTreeWeak::ptr_eq(&kept.parent, &ctx.parent_item_tree)
        }) {
            return Ok(kept.instance.clone_strong());
        }
        let (parent, index) = (ctx.parent_item_tree.clone(), ctx.parent_item_tree_index);
        let instance = self.create(ctx, generation, window, events, theme)?;
        let mut kept = self.kept.borrow_mut();
        // What the bridge held in the last instance beyond what the plugin
        // set (state the panel's own code wrote) moves over, so a column
        // closed and opened again, or docked after fullscreen, shows the
        // panel as it was (the bridge was the shell's before, and stayed).
        if let Some(previous) = kept.last() {
            self.carry(&previous.instance, &instance);
        }
        // Containers that are gone took their instances' places along.
        kept.retain(Kept::live);
        let serial = SERIAL.with(|serial| {
            serial.set(serial.get() + 1);
            serial.get()
        });
        kept.push(Kept {
            parent,
            index,
            generation,
            serial,
            instance: instance.clone_strong(),
        });
        Ok(instance)
    }

    /// Bridge properties whose value in `from` differs from the fresh
    /// `to`'s and that the plugin did not set: written by the panel's own
    /// code. A property that still equals its default or its binding's
    /// value is left alone (a write would replace the binding); an output
    /// property refuses the write.
    fn carry(&self, from: &ComponentInstance, to: &ComponentInstance) {
        let Some(compiled) = self.compiled.borrow().clone() else {
            return;
        };
        let Some(properties) = compiled.definition.global_properties(BRIDGE) else {
            return;
        };
        let values = self.values.borrow();
        for (name, _) in properties {
            if name == "tabbed" || values.contains_key(&name) {
                continue;
            }
            let (Ok(old), Ok(new)) = (
                from.get_global_property(BRIDGE, &name),
                to.get_global_property(BRIDGE, &name),
            ) else {
                continue;
            };
            if old != new {
                let _ = to.set_global_property(BRIDGE, &name, old);
            }
        }
    }

    fn create(
        self: &Rc<Self>,
        ctx: FactoryContext,
        generation: u64,
        window: &slint_interpreter::Weak<ComponentInstance>,
        events: &mpsc::SyncSender<JsonCallback>,
        theme: &str,
    ) -> Result<ComponentInstance> {
        let definition = self
            .compiled
            .borrow()
            .as_ref()
            .map(|compiled| compiled.definition.clone())
            .context("panel not compiled")?;
        let instance = definition.create_embedded(ctx)?;
        crate::ui::theme::apply_fonts(&instance)?;
        if theme != crate::ui::theme::THEMES[0] {
            crate::ui::theme::apply(&instance, theme)?;
        }
        // `VestraFocus.release()` gives the keys to the window's video area
        // (the floating window: its key scope), as `view::bind_focus` does
        // for the window's own global.
        let weak = window.clone();
        instance.set_global_callback(crate::ui::view::FOCUS_GLOBAL, "release", move |_| {
            if let Some(window) = weak.upgrade() {
                crate::ui::view::focus_video(&window);
            }
            Value::Void
        })?;
        if self.has_bridge() {
            crate::ui::ui_values::own_default_models(&instance, Some(BRIDGE))?;
            let callbacks = definition
                .global_callbacks(BRIDGE)
                .context("composition global unavailable")?;
            for callback in callbacks {
                let sender = events.clone();
                let owner = self.spec.owner;
                let name = callback.clone();
                instance.set_global_callback(BRIDGE, &callback, move |args| {
                    composition::deliver_json(
                        &sender,
                        JsonCallback {
                            shell_generation: generation,
                            owner,
                            callback: name.clone(),
                            args: composition::json_args(args),
                        },
                    );
                    Value::Void
                })?;
            }
            // Everything its plugin has set so far. A value it cannot take
            // (a property it does not declare, a wrong type) is reported
            // and left out, never a failure of the panel.
            let mut values = self.values.borrow_mut();
            let mut refused = Vec::new();
            for (name, value) in values.iter_mut() {
                match crate::ui::ui_values::set(&instance, Some(BRIDGE), name, value.clone()) {
                    Ok(()) => {
                        if let Ok(now) = instance.get_global_property(BRIDGE, name) {
                            *value = now;
                        }
                    }
                    Err(error) => refused.push((name.clone(), format!("{name}: {error:#}"))),
                }
            }
            for (name, message) in refused {
                values.remove(&name);
                self.refused.borrow_mut().push(message);
            }
            if instance.get_global_property(BRIDGE, "tabbed").is_ok() {
                let _ =
                    instance.set_global_property(BRIDGE, "tabbed", Value::Bool(self.tabbed.get()));
            }
        }
        self.created.set(self.created.get() + 1);
        Ok(instance)
    }

    /// The factory of this panel for the window of `generation`.
    #[allow(deprecated)]
    fn factory(
        self: &Rc<Self>,
        generation: u64,
        window: &ComponentInstance,
        events: &mpsc::SyncSender<JsonCallback>,
        theme: &Rc<RefCell<String>>,
    ) -> slint::ComponentFactory {
        if let Some(factory) = self.factories.borrow().get(&generation) {
            return factory.clone();
        }
        let panel = Rc::downgrade(self);
        let window = window.as_weak();
        let events = events.clone();
        let theme = theme.clone();
        let factory = slint::ComponentFactory::new(move |ctx| {
            let panel = panel.upgrade()?;
            let theme = theme.borrow().clone();
            match panel.instance_for(ctx, generation, &window, &events, &theme) {
                Ok(instance) => Some(instance),
                Err(error) => {
                    diag!("panel {}: instance: {error:#}", panel.spec.owner);
                    panel
                        .refused
                        .borrow_mut()
                        .push(format!("ui-prepare: {error:#}"));
                    None
                }
            }
        });
        self.factories
            .borrow_mut()
            .insert(generation, factory.clone());
        factory
    }
}

/// What [`Panels::show`] did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Shown {
    /// The container shows what it showed.
    Same,
    /// The container's factory changed: an instance is created (or the
    /// kept one embedded again) at the next layout pass. `compiled_ms`:
    /// the panel's file was compiled for it, in this time.
    Changed { compiled_ms: Option<f64> },
}

/// The mounted panels that can be drawn (`Host::panels`), by panel key.
#[derive(Default)]
pub(crate) struct Panels {
    panels: BTreeMap<u64, Rc<Panel>>,
    /// Compiled files, least recently used first.
    cache: Vec<Rc<Compiled>>,
    /// Compiled for mounts being applied ([`Panels::precompile`]): held
    /// until their panels are inserted, so the cache's trim cannot drop them
    /// in between.
    pending: Vec<Rc<Compiled>>,
    /// The palette new instances get (`Host::theme_all` keeps it).
    theme: Rc<RefCell<String>>,
    /// What each container shows: (window generation, slot) → panel key.
    shown: BTreeMap<(u64, Slot), Option<u64>>,
    clock: u64,
}

impl Panels {
    pub(crate) fn panel(&self, key: u64) -> Option<Rc<Panel>> {
        self.panels.get(&key).cloned()
    }

    /// Panel `key` can be drawn (a placeholder tab replayed from a
    /// recording cannot: its plugin never mounted it in this run).
    pub(crate) fn contains(&self, key: u64) -> bool {
        self.panels.contains_key(&key)
    }

    pub(crate) fn set_theme(&self, theme: &str) {
        if *self.theme.borrow() != theme {
            *self.theme.borrow_mut() = theme.to_owned();
        }
    }

    /// Every instance whose container exists (for a palette change).
    pub(crate) fn instances(&self) -> Vec<ComponentInstance> {
        self.panels
            .values()
            .flat_map(|panel| panel.instances())
            .collect()
    }

    /// Compiled files kept (for the log and tests).
    pub(crate) fn definitions(&self) -> usize {
        self.cache.len()
    }

    /// A new panel for `spec` under its key (a `ui.mount`): nothing is
    /// compiled; a panel that was under that key goes, with its instances.
    pub(crate) fn insert(&mut self, spec: CompileSpec) -> Rc<Panel> {
        let key = spec.owner;
        let panel = Rc::new(Panel::new(spec));
        // Compiled for this mount: the panel has it from the start.
        if let Ok(file) = dunce::canonicalize(&panel.spec.file)
            && let Some(at) = self
                .pending
                .iter()
                .position(|compiled| compiled.is(&file, &panel.spec))
        {
            *panel.compiled.borrow_mut() = Some(self.pending.swap_remove(at));
        }
        self.panels.insert(key, panel.clone());
        self.forget_shown(key);
        panel
    }

    /// Keep only the panels `mounted` says are.
    pub(crate) fn retain(&mut self, mounted: impl Fn(u64) -> bool) {
        let gone = self
            .panels
            .keys()
            .copied()
            .filter(|&key| !mounted(key))
            .collect::<Vec<_>>();
        for key in gone {
            self.panels.remove(&key);
            self.forget_shown(key);
        }
        self.trim();
    }

    /// The panel under `old` is now under `new` (its plugin woke or
    /// restarted under another owner): drawn as it was until its new run
    /// mounts it again.
    pub(crate) fn rekey(&mut self, old: u64, new: u64) {
        if let Some(panel) = self.panels.remove(&old) {
            self.panels.insert(new, panel);
        }
        for shown in self.shown.values_mut() {
            if *shown == Some(old) {
                *shown = Some(new);
            }
        }
    }

    /// A container showing `key` must be given its factory again.
    fn forget_shown(&mut self, key: u64) {
        self.shown.retain(|_, shown| *shown != Some(key));
    }

    /// What the container of `slot` in the window of `generation` shows.
    pub(crate) fn shown(&self, generation: u64, slot: Slot) -> Option<u64> {
        self.shown.get(&(generation, slot)).copied().flatten()
    }

    /// The bottom bar's instance in the window of `generation`, with its
    /// serial (a new instance has a new one).
    pub(crate) fn bar_instance(&self, generation: u64) -> Option<(u64, ComponentInstance)> {
        self.panel(self.shown(generation, Slot::Bottom)?)?
            .instance_in(generation)
    }

    /// Windows of other generations are gone: forget what they showed.
    pub(crate) fn prune(&mut self, live: &[u64]) {
        self.shown
            .retain(|(generation, _), _| live.contains(generation));
        for panel in self.panels.values() {
            panel
                .factories
                .borrow_mut()
                .retain(|generation, _| live.contains(generation));
            panel
                .kept
                .borrow_mut()
                .retain(|kept| live.contains(&kept.generation));
        }
    }

    /// The compiled file of `spec`: from the cache when none of the files
    /// it was compiled from changed, else compiled now (in the returned
    /// milliseconds).
    fn compiled(&mut self, spec: &CompileSpec) -> Result<(Rc<Compiled>, Option<f64>)> {
        let file = dunce::canonicalize(&spec.file).context("composition file unavailable")?;
        if let Some(at) = self
            .cache
            .iter()
            .position(|compiled| compiled.is(&file, spec))
        {
            let found = self.cache.remove(at);
            if found.current() {
                self.cache.push(found.clone());
                return Ok((found, None));
            }
        }
        let started = Instant::now();
        let compiled = Rc::new(Compiled::compile(file, spec)?);
        self.cache.push(compiled.clone());
        Ok((compiled, Some(ms(started))))
    }

    /// Compile `spec`'s file into the cache before its panel exists: a
    /// mount that shows at once fails here, before anything changed. The
    /// panel [`Panels::insert`] makes for `spec` next takes it.
    pub(crate) fn precompile(&mut self, spec: &CompileSpec) -> Result<Option<f64>> {
        let (compiled, took) = self.compiled(spec)?;
        self.pending.push(compiled);
        Ok(took)
    }

    /// Drop what [`Panels::precompile`] holds for panels never inserted (a
    /// batch that failed).
    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Show panel `key` (`None`: nothing) in the container of `slot` of
    /// `window` (the shell or the floating window, of `generation`). Its
    /// file is compiled now if it was not: an error is that compile's, and
    /// the container is left as it was.
    #[allow(deprecated)]
    pub(crate) fn show(
        &mut self,
        window: &ComponentInstance,
        generation: u64,
        slot: Slot,
        key: Option<u64>,
        events: &mpsc::SyncSender<JsonCallback>,
    ) -> Result<Shown> {
        let place = (generation, slot);
        if self.shown.get(&place) == Some(&key) {
            return Ok(Shown::Same);
        }
        let panel = key.and_then(|key| self.panel(key));
        let mut compiled_ms = None;
        if let Some(panel) = &panel {
            if !panel.loaded() {
                let (compiled, took) = self
                    .compiled(&panel.spec)
                    .with_context(|| format!("panel {}", panel.spec.file.display()))?;
                *panel.compiled.borrow_mut() = Some(compiled);
                compiled_ms = took;
            }
            self.clock += 1;
            panel.used.set(self.clock);
        }
        let factory = match &panel {
            Some(panel) => panel.factory(generation, window, events, &self.theme),
            None => slint::ComponentFactory::default(),
        };
        let name = format!("{}-factory", slot.name());
        window
            .set_property(&name, Value::ComponentFactory(factory))
            .map_err(|error| anyhow!("{name}: {error}"))?;
        self.shown.insert(place, key.filter(|_| panel.is_some()));
        Ok(Shown::Changed { compiled_ms })
    }

    /// Compile panel `key`'s file now if it is not loaded, without showing
    /// it (the adjust column's front tabs at its opening,
    /// `Host::preload_front`): its first showing then only creates an
    /// instance. It counts as shown now for [`Panels::evict`]. `Ok(None)`:
    /// already loaded (or no such panel); else the compile's milliseconds
    /// (`None` inside: taken from the cache).
    pub(crate) fn load(&mut self, key: u64) -> Result<Option<Option<f64>>> {
        let Some(panel) = self.panel(key) else {
            return Ok(None);
        };
        self.clock += 1;
        panel.used.set(self.clock);
        if panel.loaded() {
            return Ok(None);
        }
        let (compiled, took) = self
            .compiled(&panel.spec)
            .with_context(|| format!("panel {}", panel.spec.file.display()))?;
        *panel.compiled.borrow_mut() = Some(compiled);
        Ok(Some(took))
    }

    /// Bound what `slot`'s panels hold: beyond [`MAX_LOADED`] loaded ones,
    /// the least recently shown that no container shows is unloaded. The
    /// keys unloaded.
    pub(crate) fn evict(&mut self, slot: Slot) -> Vec<u64> {
        let showing = self.shown.values().flatten().copied().collect::<Vec<_>>();
        let mut loaded = self
            .panels
            .iter()
            .filter(|(_, panel)| panel.spec.slot == slot && panel.loaded())
            .map(|(&key, panel)| (panel.used.get(), key))
            .collect::<Vec<_>>();
        loaded.sort_unstable();
        let mut over = loaded.len().saturating_sub(MAX_LOADED);
        let mut gone = Vec::new();
        for (_, key) in loaded {
            if over == 0 {
                break;
            }
            if showing.contains(&key) {
                continue;
            }
            self.panels[&key].unload();
            gone.push(key);
            over -= 1;
        }
        if !gone.is_empty() {
            self.trim();
        }
        gone
    }

    /// Drop compiled files no panel uses, but the most recent
    /// [`SPARE_DEFINITIONS`].
    fn trim(&mut self) {
        let unused = self
            .cache
            .iter()
            .filter(|compiled| Rc::strong_count(compiled) == 1)
            .count();
        let mut drop = unused.saturating_sub(SPARE_DEFINITIONS);
        self.cache.retain(|compiled| {
            if drop > 0 && Rc::strong_count(compiled) == 1 {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }

    /// The values new instances could not take since the last call, by
    /// panel key (`Host::report_value_failures`).
    pub(crate) fn take_refused(&self) -> Vec<(u64, String)> {
        self.panels
            .iter()
            .flat_map(|(&key, panel)| {
                std::mem::take(&mut *panel.refused.borrow_mut())
                    .into_iter()
                    .map(move |message| (key, message))
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ui::composition::tests::headless;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mount")
            .join(name)
    }

    fn write(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn spec(owner: u64, file: PathBuf, component: &str, slot: Slot) -> CompileSpec {
        CompileSpec {
            owner,
            file,
            component: component.into(),
            global: Some("Bridge".into()),
            slot,
        }
    }

    const COUNTER: &str = "export global Bridge { in property <string> label; \
        in-out property <int> count; in-out property <int> twice: self.count * 2; \
        in property <bool> tabbed; callback hit(int); }\n\
        export component Panel inherits Rectangle { \
        Text { text: Bridge.label + Bridge.count; } \
        TouchArea { clicked => { Bridge.count += 1; Bridge.hit(Bridge.count); } } }\n";

    /// A shell with panels shown in it, for tests of the composed window:
    /// `specs`' panels (each with its bridge under [`BRIDGE`]), the first
    /// of each slot shown, laid out.
    pub(crate) struct Composed {
        pub(crate) shell: ComponentInstance,
        pub(crate) panels: Panels,
        pub(crate) events: mpsc::Receiver<JsonCallback>,
        sender: mpsc::SyncSender<JsonCallback>,
    }

    impl Composed {
        pub(crate) fn new(specs: &[CompileSpec]) -> Self {
            let shell = composition::shell_in(None).unwrap();
            Self::in_shell(shell, specs)
        }
        pub(crate) fn in_shell(shell: ComponentInstance, specs: &[CompileSpec]) -> Self {
            let (sender, events) = mpsc::sync_channel(64);
            let mut composed = Self {
                shell,
                panels: Panels::default(),
                events,
                sender,
            };
            for spec in specs {
                composed.panels.insert(spec.clone());
            }
            for slot in [Slot::Bottom, Slot::Sidebar, Slot::Backdrop, Slot::Adjust] {
                if let Some(spec) = specs.iter().find(|spec| spec.slot == slot) {
                    composed.show(slot, Some(spec.owner)).unwrap();
                }
            }
            // As the host sets them (a floating window has the sidebar's
            // only).
            let tabs = |slot| specs.iter().filter(|spec| spec.slot == slot).count();
            let shell = &composed.shell;
            let _ = shell.set_property("sidebar-tabbed", (tabs(Slot::Sidebar) > 1).into());
            let _ = shell.set_property("bottom-mounted", (tabs(Slot::Bottom) > 0).into());
            let _ = shell.set_property("adjust-visible", (tabs(Slot::Adjust) > 0).into());
            let _ = shell.set_property("sidebar-visible", (tabs(Slot::Sidebar) > 0).into());
            composed.settle();
            composed.mirror_bar();
            composed
        }
        /// The bar's reserved width and `placed` to the shell, as the
        /// host's tick does.
        pub(crate) fn mirror_bar(&self) {
            let bar = self.panels.bar_instance(1).map(|(_, bar)| bar);
            mirror_bar(&self.shell, bar.as_ref());
        }
        /// The windows' and the bottom bar's `BarButtonsHost` (the host
        /// fills each alike, `bar_buttons.rs`).
        pub(crate) fn strip_globals(&self) -> Vec<ComponentInstance> {
            let mut all = vec![self.shell.clone_strong()];
            all.extend(self.panels.bar_instance(1).map(|(_, bar)| bar));
            all
        }
        pub(crate) fn show(&mut self, slot: Slot, key: Option<u64>) -> Result<Shown> {
            let shown = self.panels.show(&self.shell, 1, slot, key, &self.sender);
            self.settle();
            shown
        }
        pub(crate) fn settle(&self) {
            crate::ui::view::settle_layout(self.shell.window());
        }
        /// Panel `key`'s instance in the shell.
        pub(crate) fn instance(&self, key: u64) -> ComponentInstance {
            self.panels
                .panel(key)
                .and_then(|panel| panel.instance_in(1))
                .unwrap_or_else(|| panic!("panel {key} has no instance in the shell"))
                .1
        }
        pub(crate) fn bridge(&self, key: u64, property: &str) -> Value {
            self.instance(key)
                .get_global_property(BRIDGE, property)
                .unwrap()
        }
    }

    /// A panel compiles when it is first shown, not when it is mounted; a
    /// switch away and back hands the container the same instance, with
    /// the state its own code wrote; values set while it was in the
    /// background show when it is back.
    #[test]
    fn panels_compile_when_first_shown_and_keep_their_instances() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let a = spec(
            21,
            write(dir.path(), "a.slint", COUNTER),
            "Panel",
            Slot::Sidebar,
        );
        let b = spec(
            25,
            write(dir.path(), "b.slint", COUNTER),
            "Panel",
            Slot::Sidebar,
        );
        let mut composed = Composed::new(&[]);
        let first = composed.panels.insert(a.clone());
        let second = composed.panels.insert(b.clone());
        first.write("label", Value::String("a".into())).unwrap();
        assert!(!first.loaded() && !second.loaded(), "mounted: no compile");
        assert_eq!(composed.panels.definitions(), 0);
        composed
            .shell
            .set_property("sidebar-visible", true.into())
            .unwrap();
        let shown = composed.show(Slot::Sidebar, Some(21)).unwrap();
        assert!(matches!(
            shown,
            Shown::Changed {
                compiled_ms: Some(_)
            }
        ));
        assert!(first.loaded() && !second.loaded());
        assert_eq!(first.created.get(), 1);
        assert_eq!(composed.bridge(21, "label"), Value::String("a".into()));
        // The same panel again: nothing.
        assert_eq!(composed.show(Slot::Sidebar, Some(21)).unwrap(), Shown::Same);
        // State its own code wrote.
        composed
            .instance(21)
            .set_global_property(BRIDGE, "count", Value::Number(3.))
            .unwrap();
        // The other tab: compiled now; the first one keeps its instance.
        let shown = composed.show(Slot::Sidebar, Some(25)).unwrap();
        assert!(matches!(
            shown,
            Shown::Changed {
                compiled_ms: Some(_)
            }
        ));
        first.write("label", Value::String("later".into())).unwrap();
        // And back: no compile, no new instance, its state and the value
        // set meanwhile.
        let shown = composed.show(Slot::Sidebar, Some(21)).unwrap();
        assert_eq!(shown, Shown::Changed { compiled_ms: None });
        assert_eq!(first.created.get(), 1, "the kept instance, not a new one");
        assert_eq!(composed.bridge(21, "count"), Value::Number(3.));
        assert_eq!(composed.bridge(21, "label"), Value::String("later".into()));
        assert_eq!(second.created.get(), 1);
    }

    /// A column closed and opened again makes a new container: the panel
    /// gets a new instance there, with what its plugin set and what its
    /// own code wrote to the bridge, and a property still driven by its
    /// binding keeps the binding.
    #[test]
    fn a_new_container_gets_an_instance_as_the_last_one_was() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let a = spec(
            21,
            write(dir.path(), "a.slint", COUNTER),
            "Panel",
            Slot::Sidebar,
        );
        let composed = Composed::new(std::slice::from_ref(&a));
        let panel = composed.panels.panel(21).unwrap();
        panel.write("label", Value::String("a".into())).unwrap();
        composed
            .instance(21)
            .set_global_property(BRIDGE, "count", Value::Number(4.))
            .unwrap();
        assert_eq!(composed.bridge(21, "twice"), Value::Number(8.));
        for visible in [false, true] {
            composed
                .shell
                .set_property("sidebar-visible", visible.into())
                .unwrap();
            composed.settle();
        }
        assert_eq!(panel.created.get(), 2, "a new container, a new instance");
        assert_eq!(composed.bridge(21, "label"), Value::String("a".into()));
        assert_eq!(composed.bridge(21, "count"), Value::Number(4.));
        // Still the binding: it follows `count`.
        composed
            .instance(21)
            .set_global_property(BRIDGE, "count", Value::Number(5.))
            .unwrap();
        assert_eq!(composed.bridge(21, "twice"), Value::Number(10.));
        assert_eq!(panel.instances().len(), 1, "the closed column's is gone");
    }

    /// Callbacks carry the panel's key and the window's generation; a
    /// value the bridge does not declare is refused at once once the
    /// panel is compiled, and reported when its first instance takes it
    /// otherwise.
    #[test]
    fn callbacks_and_refused_values_name_their_panel() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let a = spec(
            21,
            write(dir.path(), "a.slint", COUNTER),
            "Panel",
            Slot::Sidebar,
        );
        let mut composed = Composed::new(&[]);
        let panel = composed.panels.insert(a);
        panel.write("nope", Value::Number(1.)).unwrap();
        composed
            .shell
            .set_property("sidebar-visible", true.into())
            .unwrap();
        composed.show(Slot::Sidebar, Some(21)).unwrap();
        let refused = composed.panels.take_refused();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(refused[0].0 == 21 && refused[0].1.starts_with("nope:"));
        assert!(composed.panels.take_refused().is_empty());
        assert!(panel.write("nope", Value::Number(1.)).is_err());
        composed
            .instance(21)
            .invoke_global(BRIDGE, "hit", &[Value::Number(2.)])
            .unwrap();
        let event = composed.events.try_recv().unwrap();
        assert_eq!(
            (event.shell_generation, event.owner, event.callback.as_str()),
            (1, 21, "hit")
        );
    }

    /// A panel that does not compile fails alone, when it is shown: the
    /// container keeps what it showed, and the other panels are untouched.
    #[test]
    fn a_panel_that_does_not_compile_fails_alone() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let good = spec(
            21,
            write(dir.path(), "a.slint", COUNTER),
            "Panel",
            Slot::Sidebar,
        );
        let bad = spec(
            25,
            write(
                dir.path(),
                "bad.slint",
                "export global Bridge {}\nexport component Panel inherits Rectangle { nope: 1; }\n",
            ),
            "Panel",
            Slot::Sidebar,
        );
        let mut composed = Composed::new(&[good, bad.clone()]);
        assert!(composed.panels.precompile(&bad).is_err());
        let error = composed.show(Slot::Sidebar, Some(25)).unwrap_err();
        assert!(format!("{error:#}").contains("bad.slint"), "{error:#}");
        assert_eq!(composed.panels.shown(1, Slot::Sidebar), Some(21));
        assert_eq!(composed.panels.panel(21).unwrap().instances().len(), 1);
    }

    /// The compiled file is kept for the next mount of the same panel and
    /// compiled again once a file it was compiled from changed (an
    /// imported one too); beyond `MAX_LOADED` shown panels of a column the
    /// least recently shown is unloaded, and compiled files no panel uses
    /// are dropped beyond `SPARE_DEFINITIONS`.
    #[test]
    fn definitions_are_cached_checked_against_their_files_and_bounded() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "part.slint",
            "export component Part inherits Text { text: \"one\"; }\n",
        );
        let file = write(
            dir.path(),
            "a.slint",
            "import { Part } from \"part.slint\";\nexport global Bridge {}\n\
             export component Panel inherits Rectangle { Part {} }\n",
        );
        let a = spec(21, file, "Panel", Slot::Sidebar);
        let mut panels = Panels::default();
        assert!(panels.precompile(&a).unwrap().is_some());
        assert!(panels.precompile(&a).unwrap().is_none(), "cached");
        // The imported file changes (its time moved on, as an update does).
        let part = dir.path().join("part.slint");
        std::fs::write(
            &part,
            "export component Part inherits Text { text: \"two\"; }\n",
        )
        .unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&part)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(panels.precompile(&a).unwrap().is_some(), "compiled again");
        assert_eq!(panels.definitions(), 1, "the stale one is gone");

        // Six tabs shown one after another: four stay loaded.
        let mut composed = Composed::new(&[]);
        composed
            .shell
            .set_property("sidebar-visible", true.into())
            .unwrap();
        let keys = (0..6).map(|i| 21 + 4 * i).collect::<Vec<u64>>();
        for &key in &keys {
            let file = write(dir.path(), &format!("tab{key}.slint"), COUNTER);
            composed
                .panels
                .insert(spec(key, file, "Panel", Slot::Sidebar));
        }
        for &key in &keys {
            composed.show(Slot::Sidebar, Some(key)).unwrap();
            composed.panels.evict(Slot::Sidebar);
        }
        let loaded = keys
            .iter()
            .filter(|&&key| composed.panels.panel(key).unwrap().loaded())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(loaded, keys[2..], "the two shown first were unloaded");
        // Four in use and the two spare ones.
        assert_eq!(
            composed.panels.definitions(),
            MAX_LOADED + SPARE_DEFINITIONS
        );
        // An unloaded tab shown again comes back (here from a spare one).
        let shown = composed.show(Slot::Sidebar, Some(keys[0])).unwrap();
        assert!(matches!(shown, Shown::Changed { .. }));
        assert_eq!(composed.panels.panel(keys[0]).unwrap().created.get(), 2);
        // Unmounted: their compiled files go, but the spare ones.
        composed.show(Slot::Sidebar, None).unwrap();
        composed.panels.retain(|_| false);
        assert_eq!(composed.panels.definitions(), SPARE_DEFINITIONS);
    }

    /// The files a mount batch compiles before its panels exist are the
    /// panels' when they are inserted, however many: the cache's bound on
    /// unused files never drops them in between (it did: the bar compiled
    /// twice at start-up).
    #[test]
    fn a_batch_keeps_what_it_compiled_for_its_panels() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let specs = (0..SPARE_DEFINITIONS as u64 + 2)
            .map(|i| {
                let file = write(dir.path(), &format!("p{i}.slint"), COUNTER);
                spec(21 + 4 * i, file, "Panel", Slot::Sidebar)
            })
            .collect::<Vec<_>>();
        let mut panels = Panels::default();
        for spec in &specs {
            assert!(panels.precompile(spec).unwrap().is_some());
        }
        for spec in &specs {
            panels.insert(spec.clone());
        }
        panels.clear_pending();
        panels.retain(|_| true);
        for spec in &specs {
            assert!(panels.panel(spec.owner).unwrap().loaded(), "{}", spec.owner);
        }
        assert_eq!(panels.definitions(), specs.len());
    }

    /// A bottom bar's wrapper reads the bar's width property where it has
    /// one (its own or a base component's), publishes the strip's room to
    /// the bar's own `BarButtonsHost`, and passes the bar's height through
    /// the container.
    #[test]
    fn a_bottom_bar_reports_its_reserved_width_and_reads_the_strips_width() {
        headless();
        for (file, component, global, reserve) in [
            ("min-bar.slint", "MinBar", "MinBridge", true),
            ("min-derived.slint", "DerivedBar", "DerivedBridge", true),
            ("strip-bar.slint", "StripBar", "StripBridge", false),
        ] {
            let bar = CompileSpec {
                global: Some(global.into()),
                ..spec(20, fixture(file), component, Slot::Bottom)
            };
            let composed = Composed::new(std::slice::from_ref(&bar));
            let instance = composed.instance(20);
            let reserved = instance.get_property(RESERVE).unwrap();
            assert_eq!(
                matches!(reserved, Value::Number(n) if n > 0.),
                reserve,
                "{file}: {reserved:?}"
            );
            // The strip bar places the deprecated `BarButtons`.
            assert_eq!(
                instance.get_property(PLACED).unwrap(),
                Value::Bool(!reserve)
            );
            assert!(
                instance
                    .get_global_property(crate::ui::bar_buttons::GLOBAL, "room")
                    .is_ok(),
                "{file}: the strip's global is the bar's own"
            );
        }
        // Not a bar: no strip global, no reserve.
        let source = panel_source(
            &spec(21, fixture("min-bar.slint"), "MinBar", Slot::Sidebar),
            None,
        )
        .unwrap();
        assert!(!source.contains("BarButtonsHost") && !source.contains(RESERVE));
    }

    /// Step 3a: a bar's width property counts only where its component
    /// really has it, readable and a length (the compiler answers, not the
    /// text): in a comment, on another component of the file, of another
    /// type or private, it does not, and the bar keeps no width; a bar that
    /// does not compile fails its own panel.
    #[test]
    fn a_bar_keeps_a_width_only_where_its_component_declares_one() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [
            (
                "comment.slint",
                "// out property <length> content-min-width: 3px;\nexport component B inherits Rectangle {}\n",
            ),
            (
                "other.slint",
                "component Part inherits Rectangle { out property <length> content-min-width: 3px; }\n\
                 export component B inherits Rectangle { Part {} }\n",
            ),
            (
                "typed.slint",
                "export component B inherits Rectangle { out property <int> content-min-width: 3; }\n",
            ),
            (
                "private.slint",
                "export component B inherits Rectangle { property <length> content-min-width: 3px; }\n",
            ),
        ] {
            let bar = CompileSpec {
                global: None,
                ..spec(20, write(dir.path(), name, text), "B", Slot::Bottom)
            };
            let composed = Composed::new(std::slice::from_ref(&bar));
            let instance = composed.instance(20);
            assert_eq!(
                instance.get_property(RESERVE).unwrap(),
                Value::Number(0.),
                "{name}"
            );
        }
        let broken = spec(
            20,
            write(
                dir.path(),
                "broken.slint",
                "export component B inherits Rectangle { nonsense: 3px; }\n",
            ),
            "B",
            Slot::Bottom,
        );
        assert!(Panels::default().precompile(&broken).is_err());
    }

    /// Experimental Slint syntax is the shell's alone: a plugin file that
    /// uses `ComponentContainer` does not compile.
    #[test]
    fn plugin_files_cannot_use_experimental_slint() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "exp.slint",
            "export global Bridge {}\nexport component Panel inherits Rectangle { \
             in property <component-factory> f; ComponentContainer { component-factory: root.f; } }\n",
        );
        let error = Panels::default()
            .precompile(&spec(21, file, "Panel", Slot::Sidebar))
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("ComponentContainer") || text.contains("component-factory"),
            "{text}"
        );
    }

    /// A bottom bar whose height constraints depend on its own height is
    /// refused when it is compiled (its mount fails with `ui-prepare`, as
    /// before per-panel compilation), instead of ending the player with
    /// Slint's "Recursion detected" once the shell lays it out. A bar with
    /// fixed constraints, or none, still compiles and shows.
    #[test]
    fn a_bar_whose_height_constraints_follow_its_height_is_refused() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let bar = |name: &str, body: &str| {
            let text = format!(
                "export global Bridge {{ in property <bool> tabbed; }}\n\
                 export component Bar inherits Rectangle {{ {body} }}\n"
            );
            spec(31, write(dir.path(), name, &text), "Bar", Slot::Bottom)
        };
        for (name, body) in [
            ("preferred.slint", "preferred-height: self.height;"),
            ("min.slint", "min-height: self.height;"),
            ("max.slint", "max-height: self.height + 1px;"),
            (
                "layout.slint",
                "VerticalLayout { Rectangle { min-height: root.height / 2; } }",
            ),
        ] {
            let looping = bar(name, body);
            let error = Panels::default().precompile(&looping).unwrap_err();
            let text = format!("{error:#}");
            assert!(
                text.contains("the bottom bar's height constraints depend on its own height")
                    && text.contains("binding loop"),
                "{name}: {text}"
            );
            // Shown anyway (a path that did not compile it first): refused,
            // the shell lays out without it and the player goes on.
            let mut composed = Composed::new(&[]);
            composed.panels.insert(looping);
            assert!(composed.show(Slot::Bottom, Some(31)).is_err(), "{name}");
            assert_eq!(composed.panels.shown(1, Slot::Bottom), None, "{name}");
            composed.settle();
        }
        for (name, body) in [
            ("fixed.slint", "preferred-height: 40px;"),
            ("none.slint", ""),
            (
                "width.slint",
                "VerticalLayout { Rectangle { min-height: 30px; min-width: root.width / 4; } }",
            ),
        ] {
            let fine = bar(name, body);
            assert!(Panels::default().precompile(&fine).is_ok(), "{name}");
            let composed = Composed::new(&[fine]);
            assert_eq!(composed.panels.shown(1, Slot::Bottom), Some(31), "{name}");
        }
    }

    /// The other slots need no check like the bar's: their containers are
    /// placed by hand (`composition::container`), never in a layout, so a
    /// panel whose size constraints depend on its own size (in either
    /// direction, its own or a child layout's) is no binding loop there.
    /// Each such panel compiles, shows in the sidebar (docked, overlay,
    /// floating window), the adjust column (docked, overlay) and the
    /// backdrop, and the windows lay out and draw, also after a resize,
    /// without Slint's "Recursion detected" (a panic, which fails the test).
    #[test]
    fn panels_whose_constraints_follow_their_size_run_in_the_other_slots() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let bodies = [
            ("preferred-h", "preferred-height: self.height;"),
            ("min-h", "min-height: self.height;"),
            ("max-h", "max-height: self.height + 1px;"),
            ("preferred-w", "preferred-width: self.width;"),
            ("min-w", "min-width: self.width;"),
            ("max-w", "max-width: self.width + 1px;"),
            (
                "layout",
                "VerticalLayout { Rectangle { min-height: root.height / 2; min-width: root.width / 2; } }",
            ),
            (
                "both",
                "min-width: self.width; min-height: self.height; \
                 preferred-width: self.width * 2; preferred-height: self.height * 2;",
            ),
        ];
        let draw = |shell: &ComponentInstance, what: &str| {
            for (width, height) in [(1400., 800.), (700., 420.), (1600., 900.)] {
                shell
                    .window()
                    .set_size(slint::LogicalSize::new(width, height));
                crate::ui::view::settle_layout(shell.window());
                shell
                    .window()
                    .take_snapshot()
                    .unwrap_or_else(|error| panic!("{what}: {error}"));
            }
        };
        for (name, body) in bodies {
            let text = format!(
                "export global Bridge {{ in property <bool> tabbed; }}\n\
                 export component Panel inherits Rectangle {{ background: #123; {body} }}\n"
            );
            let file = write(dir.path(), &format!("{name}.slint"), &text);
            for slot in [Slot::Sidebar, Slot::Adjust, Slot::Backdrop] {
                let what = format!("{name} in {}", slot.name());
                let panel = spec(41, file.clone(), "Panel", slot);
                assert!(Panels::default().precompile(&panel).is_ok(), "{what}");
                let mut composed = Composed::new(std::slice::from_ref(&panel));
                assert_eq!(composed.panels.shown(1, slot), Some(41), "{what}");
                draw(&composed.shell, &what);
                assert!(
                    composed.panels.panel(41).unwrap().created.get() >= 1,
                    "{what}: no instance"
                );
                // The columns' overlays (fullscreen, a narrow window), popped
                // out: another container, a new instance.
                if let Some(column) = crate::ui::sidebar_tabs::Column::of(slot) {
                    let p = column.prefix();
                    for (property, value) in [
                        ("host-fullscreen", true),
                        (&format!("{p}-overlay") as &str, true),
                        (&format!("{p}-popped"), true),
                    ] {
                        let _ = composed.shell.set_property(property, value.into());
                    }
                    draw(&composed.shell, &format!("{what}, overlay"));
                }
                // The sidebar's floating window.
                if slot == Slot::Sidebar {
                    let floating = composition::floating().unwrap();
                    let shown = composed
                        .panels
                        .show(&floating, 2, slot, Some(41), &composed.sender)
                        .unwrap();
                    assert!(matches!(shown, Shown::Changed { .. }), "{what}");
                    draw(&floating, &format!("{what}, floating"));
                    assert!(
                        composed.panels.panel(41).unwrap().instance_in(2).is_some(),
                        "{what}: floating instance"
                    );
                }
                assert!(composed.panels.take_refused().is_empty(), "{what}");
            }
        }
    }

    /// The adjust column's front tabs at its opening (`Host::preload_front`):
    /// a panel loaded without being shown compiles now and creates nothing;
    /// its first showing then compiles nothing. Loading again does nothing;
    /// a file that does not compile is an error and leaves it unloaded.
    #[test]
    fn a_loaded_panel_shows_without_a_compile() {
        headless();
        let dir = tempfile::tempdir().unwrap();
        let a = spec(
            51,
            write(dir.path(), "a.slint", COUNTER),
            "Panel",
            Slot::Adjust,
        );
        let b = spec(
            55,
            write(dir.path(), "b.slint", COUNTER),
            "Panel",
            Slot::Adjust,
        );
        let broken = spec(
            59,
            write(dir.path(), "broken.slint", "export component Panel {"),
            "Panel",
            Slot::Adjust,
        );
        let mut composed = Composed::new(std::slice::from_ref(&a));
        let second = composed.panels.insert(b);
        assert!(!second.loaded());
        assert!(matches!(composed.panels.load(55), Ok(Some(Some(_)))));
        assert!(second.loaded());
        assert_eq!(second.created.get(), 0, "loaded, not created");
        assert!(matches!(composed.panels.load(55), Ok(None)));
        assert!(
            matches!(composed.panels.load(99), Ok(None)),
            "no such panel"
        );
        let shown = composed.show(Slot::Adjust, Some(55)).unwrap();
        assert_eq!(shown, Shown::Changed { compiled_ms: None });
        assert_eq!(second.created.get(), 1);
        let third = composed.panels.insert(broken);
        assert!(composed.panels.load(59).is_err());
        assert!(!third.loaded());
    }
}
