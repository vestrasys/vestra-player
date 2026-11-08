//! Slint composition of the shell's slots, using the public interpreter API.
//! Host-internal: the public contract is the SDK window/panel API.
#![forbid(unsafe_code)]

use std::{collections::BTreeSet, path::PathBuf, sync::mpsc};

use anyhow::{Context, Result, bail};
use serde_json::Value as Json;
use slint_interpreter::{ComponentDefinition, ComponentInstance, Value};

use crate::ui::sidebar_tabs::Column;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    Bottom,
    Sidebar,
    /// Behind the host's video input layer, shown while there is no video (R-U3).
    Backdrop,
    /// The adjust column (menu redesign step 3b): controls for what is
    /// playing, between the video and the sidebar.
    Adjust,
}

impl Slot {
    /// The shell property showing this slot; the backdrop follows `no-video`.
    pub fn visible_property(self) -> Option<&'static str> {
        match self {
            Self::Bottom => Some("bottom-visible"),
            Self::Sidebar => Some("sidebar-visible"),
            Self::Adjust => Some("adjust-visible"),
            Self::Backdrop => None,
        }
    }
    /// The wire name (`ui.mount`'s `slot`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Sidebar => "sidebar",
            Self::Backdrop => "backdrop",
            Self::Adjust => "adjust",
        }
    }
    /// The slot of a wire name (`ui.mount`'s `slot`).
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Bottom, Self::Sidebar, Self::Backdrop, Self::Adjust]
            .into_iter()
            .find(|slot| slot.name() == name)
    }
    /// What the user calls it, in host messages.
    pub fn label(self) -> String {
        match self {
            Self::Bottom => crate::i18n::tr("bottom bar"),
            Self::Sidebar => crate::i18n::tr("sidebar"),
            Self::Backdrop => crate::i18n::tr("video backdrop"),
            Self::Adjust => crate::i18n::tr("adjust column"),
        }
    }
    /// A column of tabs (the sidebar, the adjust column): any number of
    /// plugins mount there, one tab each.
    pub fn tabbed(self) -> bool {
        matches!(self, Self::Sidebar | Self::Adjust)
    }
}

/// One independent package per owner. The bottom bar and the backdrop hold
/// one component each; the sidebar holds one or more, as tabs in the order of
/// the specs. Components inherit a native visual element, not Window.
#[derive(Clone, Debug, PartialEq)]
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

pub(crate) fn identifier(name: &str) -> Result<()> {
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
/// The panel key scopes it goes into (the docked sidebar, its fullscreen
/// overlay, the bottom bar, the adjust column, the detached window's) keep
/// Slint's default `focus-on-click: true`, so plugins' `focus: "panel"`
/// bindings work as they did before 0.7.6: a press that no control of the
/// panel takes (an empty part of it) gives the panel the keys. A press a
/// control takes (a button, a row's `TouchArea`, a drag) never reaches the
/// scope (Slint 1.18 offers a press to the children first), so it leaves
/// the focus where it was; a panel that wants an empty press to leave it
/// too puts a `TouchArea {}` under its content (the playlist does).
/// `VestraFocus.release` gives the keys back to the video.
fn key_forward(scope: &str) -> String {
    format!(
        "key-pressed(event) => {{ if root.key-input(\"{scope}\", event.text, event.modifiers.control, event.modifiers.alt, event.modifiers.shift, event.modifiers.meta, event.repeat) {{ return accept; }} reject }}"
    )
}

pub(crate) fn import_path(file: &std::path::Path) -> Result<String> {
    let path = dunce::canonicalize(file).context("composition file unavailable")?;
    let path = path.to_str().context("Slint import path must be UTF-8")?;
    if path.chars().any(char::is_control) {
        bail!("control character in Slint import path");
    }
    // dunce keeps the Windows verbatim form only where no plain path names
    // the file (too long, reserved names); Slint does not resolve it.
    if path.starts_with(r"\\?\") {
        bail!("Slint cannot import this path (Windows verbatim path): {path}");
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

/// The shell's slot properties and the tab properties of its columns (also
/// in a shell without tabs, so the host binds them alike).
const SHELL_PROPERTIES: &str = "in property <bool> bottom-visible: true;\n\
    in property <bool> sidebar-visible: true;\n\
    in property <bool> sidebar-overlay: false;\n\
    in property <bool> sidebar-popped: false;\n\
    in-out property <int> sidebar-active: 0;\n\
    in property <int> sidebar-shown: -1;\n\
    in property <[string]> sidebar-titles;\n\
    callback sidebar-tab(int);\n\
    callback sidebar-overflow(float, float);\n\
    callback sidebar-detach();\n\
    in property <bool> adjust-visible: false;\n\
    in property <bool> adjust-overlay: false;\n\
    in property <bool> adjust-popped: false;\n\
    in-out property <int> adjust-active: 0;\n\
    in property <int> adjust-shown: -1;\n\
    in property <[string]> adjust-titles;\n\
    in property <[string]> adjust-keys;\n\
    callback adjust-tab(int);\n\
    callback adjust-overflow(float, float);\n\
    callback adjust-dismiss();";

/// The height of the host's sidebar tab strip (the playlist's, ui-spec §5.2).
pub const TAB_STRIP_HEIGHT: f64 = 31.0;

/// What fills a column of tabs (docked, overlay, the sidebar's floating
/// window): the one panel as before (the sidebar with one tab), or the tab
/// strip over one branch per compiled panel (sidebar-tabs.md: the front
/// tabs, not every tab), the one whose owner is `<column>-shown` drawn, so
/// switching between compiled tabs only changes a property. A tab without a
/// compiled panel (a placeholder) shows nothing until the host compiles it.
/// A click on a tab gives the keys to `keys` (the column's FocusScope): the
/// panel that had them is gone. The adjust column's strip has the keys
/// under its titles and no detach button; `background`: the panels' ground.
fn column_content(
    column: Column,
    tabs: &[&CompileSpec],
    tabbed: bool,
    keys: &str,
    detached: &str,
    background: &str,
) -> String {
    match tabs {
        [] if !tabbed => return String::new(),
        [spec] if !tabbed => {
            return format!(
                "Owner{}View {{ width: parent.width; height: parent.height; }}",
                spec.owner
            );
        }
        _ => {}
    }
    let p = column.prefix();
    let views = tabs
        .iter()
        .map(|spec| {
            format!(
                "if root.{p}-shown == {0}: Owner{0}View {{ width: parent.width; height: parent.height; }}",
                spec.owner
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let strip = match column {
        Column::Sidebar => format!("detached: {detached}; detach => {{ root.sidebar-detach(); }}"),
        Column::Adjust => "keys: root.adjust-keys; detachable: false;".to_owned(),
    };
    // Placed by hand, not in a layout: a layout here would pass its size
    // constraints up to the window (it shrank to its minimum height).
    format!(
        "SidebarTabs {{ x: 0px; y: 0px; width: parent.width; height: {TAB_STRIP_HEIGHT}px; \
         titles: root.{p}-titles; active: root.{p}-active; {strip} \
         tab(index) => {{ root.{p}-active = index; {keys}.focus(); root.{p}-tab(index); }} \
         overflow(x, y) => {{ root.{p}-overflow(x, y); }} }} \
         Rectangle {{ x: 0px; y: {TAB_STRIP_HEIGHT}px; width: parent.width; \
         height: parent.height - {TAB_STRIP_HEIGHT}px; background: {background}; clip: true; {views} }}"
    )
}

/// [`column_content`] for the sidebar.
fn sidebar_content(tabs: &[&CompileSpec], tabbed: bool, keys: &str, detached: &str) -> String {
    column_content(
        Column::Sidebar,
        tabs,
        tabbed,
        keys,
        detached,
        "Theme.bg-list",
    )
}

/// Whether the columns of these compiled panels draw their tab strips (by
/// `Column::index`): the sidebar with two or more tabs, the adjust column
/// with any.
fn default_tabbed(specs: &[CompileSpec]) -> [bool; 2] {
    let count = |slot| specs.iter().filter(|spec| spec.slot == slot).count();
    [count(Slot::Sidebar) > 1, count(Slot::Adjust) > 0]
}

/// Generated source is ordinary Slint, not a layout protocol. Imports use
/// canonical files; package containment is the package loader's responsibility.
/// Aliases prevent exported-name collisions, but are not a sandbox. Slint globals
/// imported from the same underlying file remain shared, so independent owners
/// must not import a shared stateful global (including through dependencies).
pub fn shell_source(specs: &[CompileSpec]) -> Result<String> {
    shell_source_columns(specs, default_tabbed(specs))
}

/// [`shell_source`] with the sidebar's tab strip drawn when `tabbed`, also
/// over one compiled panel (the other tabs are placeholders).
pub fn shell_source_tabbed(specs: &[CompileSpec], tabbed: bool) -> Result<String> {
    let [_, adjust] = default_tabbed(specs);
    shell_source_columns(specs, [tabbed, adjust])
}

/// [`shell_source`] with each column (by `Column::index`) drawn with its
/// tab strip when `tabbed`, also with no compiled panel (every tab a
/// placeholder).
pub fn shell_source_columns(specs: &[CompileSpec], tabbed: [bool; 2]) -> Result<String> {
    shell_source_with(specs, tabbed, bar_reports_min(specs))
}

/// The property a bottom bar reports the least width of its own controls
/// in (menu redesign step 3a): the player's button strip gets the rest.
pub const BAR_MIN_WIDTH: &str = "content-min-width";

/// The property a bottom bar reports the width of its row with every one
/// of its controls shown in (coordinator ruling after the GUI check of
/// step 3a): plugin buttons get only the room beyond it, so the bar drops
/// its own controls only after every plugin button is in `»`, and the
/// strip's overflow follows the window's width monotonically. A bar that
/// reports only [`BAR_MIN_WIDTH`] keeps the 3a behaviour (the room beyond
/// its least width).
pub const BAR_FULL_WIDTH: &str = "content-full-width";

/// The mounted bottom bar's width property the strip's room is measured
/// beyond: [`BAR_FULL_WIDTH`] when its component has that public `length`
/// property, else [`BAR_MIN_WIDTH`], else none (the whole bar); its own or
/// one it inherits from a base component in another file. The Slint compiler answers, not a text scan:
/// `ComponentDefinition::properties()` lists only the properties a
/// component declares itself (Slint 1.18.1 lowers `public_properties` from
/// the root element's own declarations), so an inherited one would be
/// missed. Instead a probe component that reads the property from an
/// instance of the bar into a `length` property is compiled: it compiles
/// exactly when the property exists, is readable from outside and is a
/// length. A bar that does not compile has no property here; the shell's
/// compile then reports its real error.
///
/// The answer is kept for the bar's run: its owner generation, file and
/// component. Owner generations are never reused (`Supervisor::alloc_pid`),
/// and the files of a plugin change only between runs: an install, update
/// or reload restarts it (`JobKind::Restart`), a woken plugin is a new
/// run too. So the shell's later compiles in the same run (a tab switch, a
/// mount elsewhere) do not repeat the probe, and a changed file anywhere in
/// the bar's import closure is probed again at its next run. A
/// modification time would miss a base component or an import in another
/// file; hashing the closure would need the import resolution the compiler
/// already does.
fn bar_reports_min(specs: &[CompileSpec]) -> Option<&'static str> {
    let spec = specs.iter().find(|spec| spec.slot == Slot::Bottom)?;
    [BAR_FULL_WIDTH, BAR_MIN_WIDTH]
        .into_iter()
        .find(|property| declares_length(spec.owner, &spec.file, &spec.component, property))
}

/// [`bar_reports_min`] for any run (`owner`), file, component and `length`
/// property. One bottom bar is mounted at a time, so only its answers are
/// kept (both properties).
fn declares_length(owner: u64, file: &std::path::Path, component: &str, property: &str) -> bool {
    type Key = (u64, PathBuf, String, String);
    thread_local! {
        static LAST: std::cell::RefCell<Vec<(Key, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = (
        owner,
        file.to_path_buf(),
        component.to_owned(),
        property.to_owned(),
    );
    if let Some(known) = LAST.with(|last| {
        last.borrow()
            .iter()
            .find(|(seen, _)| *seen == key)
            .map(|(_, found)| *found)
    }) {
        return known;
    }
    let found = probe_length(file, component, property);
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        // Another bar or another run: its answers go.
        last.retain(|(seen, _)| seen.0 == key.0 && seen.1 == key.1 && seen.2 == key.2);
        last.push((key, found));
    });
    found
}

/// The probe of [`bar_reports_min`], uncached.
fn probe_length(file: &std::path::Path, component: &str, property: &str) -> bool {
    (|| {
        identifier(component).ok()?;
        identifier(property).ok()?;
        let path = import_path(file).ok()?;
        let probe = format!(
            "import {{ {component} as ProbedBar }} from \"{path}\";\n\
             export component VestraProbe {{ bar := ProbedBar {{}} \
             out property <length> probed: bar.{property}; }}\n"
        );
        let base = file.with_file_name("vestra-property-probe.slint");
        let result = pollster::block_on(crate::ui::view::compiler().build_from_source(probe, base));
        let failed = result
            .diagnostics()
            .any(|d| d.level() == slint_interpreter::DiagnosticLevel::Error);
        Some(!failed && result.component("VestraProbe").is_some())
    })()
    .unwrap_or(false)
}

/// The player's bottom-bar strip (bar-strip.slint) at the right end of its
/// parent, its cells on the parent's bottom edge; `min`: what the bar
/// keeps for its own controls.
fn bar_strip(min: &str) -> String {
    format!(
        "bar-strip := HostBarStrip {{ x: parent.width - self.width - 4px; y: parent.height - self.height; \
         room: max(0px, parent.width - {min} - 4px); more(x, y) => {{ root.bar-more(x, y); }} }}"
    )
}

/// [`shell_source_tabbed`]; `bar_room`: the bar's property the strip's room
/// is measured beyond ([`BAR_FULL_WIDTH`], else [`BAR_MIN_WIDTH`]), `None`:
/// the whole bar.
fn shell_source_with(
    specs: &[CompileSpec],
    tabbed: [bool; 2],
    bar_room: Option<&str>,
) -> Result<String> {
    let mut owners = BTreeSet::new();
    let mut slots = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut source = String::new();
    for spec in specs {
        if !owners.insert(spec.owner) || (!spec.slot.tabbed() && !slots.insert(spec.slot)) {
            bail!("this shell requires unique owners and at most one bottom bar and one backdrop");
        }
        if !files.insert(dunce::canonicalize(&spec.file).context("composition file unavailable")?) {
            bail!("independent composition owners require distinct Slint files");
        }
        let path = import_path(&spec.file)?;
        import_spec(&mut source, spec, &path)?;
    }
    let mut shell = include_str!("../../ui/host.slint").to_owned();
    // FS-BAR-1: the bottom bar's laid-out height, also while it is hidden
    // (the host's fullscreen band); 0 without a bottom bar.
    let has_bottom = specs.iter().any(|spec| spec.slot == Slot::Bottom);
    let properties = format!(
        "{SHELL_PROPERTIES}\n    out property <float> bottom-band-height: {};",
        if has_bottom {
            "bottom-keys.height / 1px"
        } else {
            "0"
        }
    );
    replace_marker(&mut shell, "// VESTRA-MOUNT-PROPERTIES", &properties)?;
    let tabs = specs
        .iter()
        .filter(|spec| spec.slot == Slot::Sidebar)
        .collect::<Vec<_>>();
    let adjust_tabs = specs
        .iter()
        .filter(|spec| spec.slot == Slot::Adjust)
        .collect::<Vec<_>>();
    // With tabs (two or more, compiled or not) the column and its strip
    // exist even while no panel is compiled (every front tab a placeholder).
    let tabbed_adjust = tabbed[1] || !adjust_tabs.is_empty();
    let tabbed = tabbed[0] || tabs.len() > 1;
    if tabbed || tabbed_adjust {
        source.push_str("import { SidebarTabs } from \"sidebar-tabs.slint\";\n");
    }
    let adjust = adjust_column(&adjust_tabs, tabbed_adjust);
    replace_marker(&mut shell, "// VESTRA-MOUNT-ADJUST", &adjust.docked)?;
    let column = tabbed || !tabs.is_empty();
    let sidebar = if !column {
        String::new()
    } else {
        format!(
            "if root.sidebar-visible: Rectangle {{ width: 3px; horizontal-stretch: 0; background: Theme.line; \
             TouchArea {{ mouse-cursor: MouseCursor.ew-resize; \
             moved => {{ if self.pressed {{ root.sidebar-width = clamp(root.width / 1px - (self.absolute-position.x + self.mouse-x) / 1px - 1.5, 317, max(317, root.width / 1px - 321)); }} }} \
             pointer-event(event) => {{ if event.kind == PointerEventKind.up {{ root.sidebar-resized(root.sidebar-actual); }} }} }} }}\n\
             if root.sidebar-visible: sidebar-keys := FocusScope {{ width: root.sidebar-actual * 1px; horizontal-stretch: 0; {} {} }}",
            key_forward("panel:sidebar"),
            sidebar_content(&tabs, tabbed, "sidebar-keys", "false")
        )
    };
    replace_marker(&mut shell, "// VESTRA-MOUNT-SIDEBAR", &sidebar)?;
    // G-01: in fullscreen the attached sidebar is an overlay over the right
    // of the video, full height; it slides out when popped and back when
    // collapsed. Its hover area reports to the host (the G-04 mechanism).
    let overlay = if !column {
        String::new()
    } else {
        format!(
            "if root.sidebar-overlay: Rectangle {{ x: root.width; y: 0px; \
             width: root.sidebar-pop-width * 1px; height: root.height; \
             states [ popped when root.sidebar-popped: {{ x: root.width - self.width; \
             in {{ animate x {{ duration: 150ms; easing: ease-out; }} }} \
             out {{ animate x {{ duration: 150ms; easing: ease-in; }} }} }} ] \
             TouchArea {{ changed has-hover => {{ root.sidebar-hover = self.has-hover; }} \
             changed mouse-x => {{ root.window-input(\"activity\"); }} \
             changed mouse-y => {{ root.window-input(\"activity\"); }} \
             overlay-keys := FocusScope {{ {} {} }} }} }}",
            key_forward("panel:sidebar"),
            sidebar_content(&tabs, tabbed, "overlay-keys", "false")
        )
    };
    // The adjust column's overlay (fullscreen, narrow window) beside the
    // sidebar's: they never overlap, the sidebar outermost.
    let overlay = format!("{overlay}\n{}", adjust.overlay);
    replace_marker(&mut shell, "// VESTRA-MOUNT-OVERLAY", &overlay)?;
    let bottom = specs
        .iter()
        .find(|spec| spec.slot == Slot::Bottom)
        .map(|spec| {
            format!(
                "Rectangle {{ height: root.host-fullscreen || !root.bottom-visible ? 0px : bottom-keys.height; vertical-stretch: 0; \
                 bottom-keys := FocusScope {{ visible: root.bottom-visible; x: 0px; \
                 width: parent.width - (root.host-fullscreen ? root.columns-popped-width * 1px : 0px); \
                 height: min(bottom-content.max-height, max(bottom-content.min-height, bottom-content.preferred-height)); \
                 y: root.host-fullscreen ? -self.height : 0px; {} \
                 TouchArea {{ changed has-hover => {{ root.bottom-hover = self.has-hover; }} \
                 changed mouse-x => {{ root.window-input(\"activity\"); }} \
                 changed mouse-y => {{ root.window-input(\"activity\"); }} \
                 bottom-content := Owner{}View {{ width: parent.width; height: parent.height; }} {} }} }} }}",
                key_forward("panel:bottom"),
                spec.owner,
                bar_strip(
                    &bar_room.map_or_else(|| "0px".to_owned(), |p| format!("bottom-content.{p}"))
                )
            )
        })
        // Step 3a: without a bottom bar the player's strip still shows, in a
        // band of the bar's colour (hidden in fullscreen).
        .unwrap_or_else(|| {
            format!(
                "Rectangle {{ height: root.host-fullscreen ? 0px : 40px; vertical-stretch: 0; \
                 visible: !root.host-fullscreen; background: Theme.bg-bar; \
                 Rectangle {{ x: 0px; y: 0px; width: parent.width; height: 1px; background: Theme.line; }} \
                 {} }}",
                bar_strip("0px")
            )
        });
    replace_marker(&mut shell, "// VESTRA-MOUNT-BOTTOM", &bottom)?;
    // Under the video input layer, the OSD and the information blocks; it
    // never receives the pointer (the TouchArea above covers it) or focus.
    let backdrop = specs
        .iter()
        .find(|spec| spec.slot == Slot::Backdrop)
        .map(|spec| {
            format!(
                "Rectangle {{ visible: root.no-video; \
                 Owner{}View {{ width: parent.width; height: parent.height; }} }}",
                spec.owner
            )
        })
        .unwrap_or_default();
    replace_marker(&mut shell, "// VESTRA-MOUNT-BACKDROP", &backdrop)?;
    source.push_str(&shell);
    Ok(source)
}

/// The adjust column's two places in the shell (menu redesign §3.3).
struct AdjustColumn {
    /// Docked between the video column and the sidebar, with its splitter.
    docked: String,
    /// Over the video's right side while the window is too narrow to dock
    /// it (300 px, the list colour at 90 %, a shadow on its left), or, in
    /// fullscreen, sliding out from the right edge left of the sidebar's
    /// overlay (G-01). Esc inside it closes it in a window (fullscreen keeps
    /// Esc for Toggle Fullscreen). Its hover area reports to the host.
    overlay: String,
}

/// [`AdjustColumn`] for the adjust panels `tabs` (the compiled ones); the
/// column exists while `tabbed` (it has tabs, compiled or not). It always
/// draws its strip: text tabs with the keys under them, no detach.
fn adjust_column(tabs: &[&CompileSpec], tabbed: bool) -> AdjustColumn {
    if !tabbed {
        return AdjustColumn {
            docked: String::new(),
            overlay: String::new(),
        };
    }
    let keys = key_forward("panel:adjust");
    // The splitter's right edge: the window's, less a docked sidebar.
    let docked = format!(
        "if root.adjust-visible: Rectangle {{ width: 3px; horizontal-stretch: 0; background: Theme.line; \
         TouchArea {{ mouse-cursor: MouseCursor.ew-resize; \
         moved => {{ if self.pressed {{ root.adjust-width = clamp(root.width / 1px - (root.sidebar-visible ? root.sidebar-actual + 3 : 0) - (self.absolute-position.x + self.mouse-x) / 1px - 1.5, 280, max(280, root.width / 1px * 0.5)); }} }} \
         pointer-event(event) => {{ if event.kind == PointerEventKind.up {{ root.adjust-resized(root.adjust-actual); }} }} }} }}\n\
         if root.adjust-visible: adjust-keys := FocusScope {{ width: root.adjust-actual * 1px; horizontal-stretch: 0; {keys} {} }}",
        column_content(
            Column::Adjust,
            tabs,
            true,
            "adjust-keys",
            "false",
            "Theme.bg-list"
        )
    );
    let overlay = format!(
        "if root.adjust-overlay: Rectangle {{ \
         x: root.host-fullscreen ? root.width : (root.video-x + root.video-width) * 1px - self.width; \
         y: root.host-fullscreen ? 0px : root.video-y * 1px; \
         width: (root.host-fullscreen ? root.adjust-pop-width : min(300, root.video-width)) * 1px; \
         height: root.host-fullscreen ? root.height : root.video-height * 1px; \
         drop-shadow-blur: root.host-fullscreen ? 0px : 12px; drop-shadow-offset-x: -2px; \
         drop-shadow-color: root.host-fullscreen ? transparent : #00000080; \
         background: root.host-fullscreen ? transparent : Theme.bg-list.with-alpha(0.9); \
         states [ popped when root.host-fullscreen && root.adjust-popped: {{ \
         x: root.width - self.width - (root.sidebar-popped ? root.sidebar-pop-width * 1px : 0px); \
         in {{ animate x {{ duration: 150ms; easing: ease-out; }} }} \
         out {{ animate x {{ duration: 150ms; easing: ease-in; }} }} }} ] \
         TouchArea {{ changed has-hover => {{ root.adjust-hover = self.has-hover; }} \
         changed mouse-x => {{ root.window-input(\"activity\"); }} \
         changed mouse-y => {{ root.window-input(\"activity\"); }} \
         adjust-overlay-keys := FocusScope {{ \
         key-pressed(event) => {{ if !root.host-fullscreen && event.text == Key.Escape {{ root.adjust-dismiss(); return accept; }} \
         if root.key-input(\"panel:adjust\", event.text, event.modifiers.control, event.modifiers.alt, event.modifiers.shift, event.modifiers.meta, event.repeat) {{ return accept; }} reject }} \
         {} }} }} }}",
        column_content(
            Column::Adjust,
            tabs,
            true,
            "adjust-overlay-keys",
            "false",
            "root.host-fullscreen ? Theme.bg-list : transparent"
        )
    );
    AdjustColumn { docked, overlay }
}

/// The detached sidebar (C14): the same components and bridge globals in a
/// window of its own, with the shell's key forwarding, menu layer and (two
/// or more panels) tab strip. `title` is the first window title; the host
/// sets `panel-title` to the current tab's.
pub fn floating_source(specs: &[CompileSpec], title: &str) -> Result<String> {
    floating_source_tabbed(specs, title, specs.len() > 1)
}

/// [`floating_source`] with the tab strip drawn when `tabbed`.
pub fn floating_source_tabbed(specs: &[CompileSpec], title: &str, tabbed: bool) -> Result<String> {
    let mut source = String::new();
    for spec in specs {
        let path = import_path(&spec.file)?;
        import_spec(&mut source, spec, &path)?;
    }
    let tabbed = tabbed || specs.len() > 1;
    if tabbed {
        source.push_str("import { SidebarTabs } from \"sidebar-tabs.slint\";\n");
    }
    let tabs = specs.iter().collect::<Vec<_>>();
    let content = sidebar_content(&tabs, tabbed, "panel-keys", "true");
    let title = serde_json::to_string(title)?;
    source.push_str(&format!(
        r#"import {{ MenuView, MenuLevel }} from "menu-view.slint";
import {{ Theme }} from "@vestra/theme.slint";
export {{ VestraPalette }} from "palette.slint";
export {{ MenuChrome }} from "menu-view.slint";
export {{ BarButtonsHost }} from "@vestra/bar-buttons.slint";
export {{ VestraFocus }} from "@vestra/focus.slint";
export component FloatingPanel inherits Window {{
    in property <string> panel-title: {title};
    title: root.panel-title;
    min-width: 300px; min-height: 250px;
    preferred-width: 300px; preferred-height: 408px;
    background: Theme.bg-list;
    in property <[MenuLevel]> menu-levels;
    in property <bool> menu-open: false;
    in-out property <float> menu-origin-x;
    in-out property <float> menu-origin-y;
    out property <float> logical-width: root.width / 1px;
    out property <float> logical-height: root.height / 1px;
    callback menu-invoke(string);
    callback menu-hover(int, string, bool);
    callback menu-scroll(int, float);
    callback menu-back();
    callback menu-dismiss();
    callback key-input(string, string, bool, bool, bool, bool, bool) -> bool;
    in-out property <int> sidebar-active: 0;
    in property <int> sidebar-shown: -1;
    in property <[string]> sidebar-titles;
    callback sidebar-tab(int);
    callback sidebar-overflow(float, float);
    callback sidebar-detach();
    // `VestraFocus.release` (view::bind_focus): no video here, so the
    // window's key scope takes the keys from the panel's control.
    callback focus-video();
    focus-video => {{ panel-keys.focus(); }}
    panel-keys := FocusScope {{
        {keys}
        {content}
    }}
    if root.menu-open: Rectangle {{
        width: root.width; height: root.height;
        TouchArea {{ pointer-event(event) => {{ if event.kind == PointerEventKind.down {{ root.menu-dismiss(); }} }} }}
        MenuView {{
            width: root.width; height: root.height;
            levels: root.menu-levels;
            hovered(level, token, now) => {{ root.menu-hover(level, token, now); }}
            scrolled(level, offset) => {{ root.menu-scroll(level, offset); }}
            activated(token) => {{ root.menu-invoke(token); }}
            back => {{ root.menu-back(); }}
            dismissed => {{ root.menu-dismiss(); }}
        }}
    }}
}}
"#,
        keys = key_forward("panel:sidebar"),
    ));
    Ok(source)
}

/// R-U3: whether the video area has no video frame to show. Empty: none;
/// ready or ended: none unless mpv shows a picture (cover art counts) or a
/// stream produces frames; loading: none (G-16a, the backdrop shows the
/// loading screen instead of the previous source's last frame); error keeps
/// the previous answer.
pub fn no_video(previous: bool, source: &Json) -> bool {
    match source["phase"].as_str() {
        Some("loading") => true,
        _ => video_absent(previous, source),
    }
}

/// Whether the source has no video, with loading and error keeping the
/// previous answer: picture-in-picture stays open while the next source loads.
pub fn video_absent(previous: bool, source: &Json) -> bool {
    match source["phase"].as_str() {
        Some("empty") | None => true,
        Some("ready" | "ended") => !(source["kind"] == "producer" || source["videoOutput"] == true),
        _ => previous,
    }
}

fn replace_marker(source: &mut String, marker: &str, replacement: &str) -> Result<()> {
    if source.matches(marker).count() != 1 {
        bail!("host shell must contain exactly one {marker} insertion marker");
    }
    *source = source.replacen(marker, replacement, 1);
    Ok(())
}

pub(crate) fn build(source: String, component: &str) -> Result<ComponentDefinition> {
    let base = crate::ui::view::ui_dir().join("host.slint");
    // The player's own source (shell, options window): its catalog.
    let source = crate::ui::view::translate(source, &base);
    let result = pollster::block_on(crate::ui::view::compiler().build_from_source(source, base));
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
    compile_definition_tabbed(specs, default_tabbed(specs))
}

fn compile_definition_tabbed(
    specs: &[CompileSpec],
    tabbed: [bool; 2],
) -> Result<ComponentDefinition> {
    build(shell_source_columns(specs, tabbed)?, "HostWindow")
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
        crate::ui::ui_values::own_default_models(&instance, Some(&global_alias(spec.owner)))?;
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

pub(crate) fn json_args(args: &[Value]) -> Result<Vec<Json>, String> {
    if args.len() > crate::ui::ui_values::MAX_ARRAY {
        return Err("composition callback has too many arguments".into());
    }
    // Treat all arguments as one value, so depth/node/text budgets also bound
    // the aggregate callback rather than resetting for every argument.
    let model = Value::Model(std::rc::Rc::new(slint::VecModel::from(args.to_vec())).into());
    match crate::ui::ui_values::to_json(&model) {
        Ok(Json::Array(args)) => Ok(args),
        Ok(_) => Err("composition callback arguments did not form an array".into()),
        Err(error) => Err(format!("{error:#}")),
    }
}

fn deliver_json(sender: &mpsc::SyncSender<JsonCallback>, event: JsonCallback) -> bool {
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(event)) => {
            diag!(
                "composition callback queue full: owner={} callback={}; callback rejected",
                event.owner,
                event.callback
            );
            false
        }
        Err(mpsc::TrySendError::Disconnected(event)) => {
            diag!(
                "composition callback receiver closed: owner={} callback={}",
                event.owner,
                event.callback
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
    compile_json_in(specs, default_tabbed(specs), events, shell_generation, None)
}

/// [`compile_json`] into `window` (the main window's), when given: the new
/// shell's component tree replaces the shown one inside the same native
/// window instead of opening a new window, the way browsers and IDEs swap
/// content (no hide/show, no move, no resize; maximized and fullscreen stay).
///
/// Slint attaches the new tree to the window as it creates it (its
/// `init` code then sets the initial focus), so the window shows the new
/// tree from its next draw on: the caller finishes setting it up before
/// returning to the event loop, calls `show()` on it (which only moves the
/// window's strong reference to it; the native window is already visible),
/// and must not `hide()` the old shell, which shares the window. A caller
/// that fails after this returned puts the old tree back with
/// [`crate::ui::view::ShownTree::restore`]. `tabbed`: each column (by
/// `Column::index`) draws its tab strip (the sidebar with two or more tabs,
/// the adjust column with any, of which only `specs`' are compiled).
pub fn compile_json_in(
    specs: &[CompileSpec],
    tabbed: [bool; 2],
    events: mpsc::SyncSender<JsonCallback>,
    shell_generation: u64,
    window: Option<&slint::Window>,
) -> Result<ComponentInstance> {
    let definition = compile_definition_tabbed(specs, tabbed)?;
    let instance = match window {
        Some(window) => definition.create_with_existing_window(window)?,
        None => definition.create()?,
    };
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
    specs: &[CompileSpec],
    title: &str,
    tabbed: bool,
    events: mpsc::SyncSender<JsonCallback>,
    generation: u64,
) -> Result<ComponentInstance> {
    let definition = build(
        floating_source_tabbed(specs, title, tabbed)?,
        "FloatingPanel",
    )?;
    let instance = definition.create()?;
    for spec in specs {
        let alias = global_alias(spec.owner);
        crate::ui::ui_values::own_default_models(&instance, Some(&alias))?;
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
    }
    Ok(instance)
}

/// The host-owned `tabbed` property: `true` while the host draws its tab
/// strip above the panel (the sidebar holds two or more), so a panel with a
/// tab strip of its own hides it. Set only on globals that declare it.
pub fn set_tabbed(instance: &ComponentInstance, spec: &CompileSpec, tabbed: bool) -> Result<()> {
    let alias = global_alias(spec.owner);
    if spec.global.is_some() && instance.get_global_property(&alias, "tabbed").is_ok() {
        instance.set_global_property(&alias, "tabbed", Value::Bool(tabbed))?;
    }
    Ok(())
}

pub fn set_json_global(
    instance: &ComponentInstance,
    spec: &CompileSpec,
    property: &str,
    value: &Json,
) -> Result<()> {
    set_global_value(
        instance,
        spec,
        property,
        crate::ui::ui_values::from_json(value)?,
    )
}

/// Write a converted value to a global by alias, keeping equal live values.
pub fn set_alias_value(
    instance: &ComponentInstance,
    alias: &str,
    property: &str,
    value: Value,
) -> Result<()> {
    let current = instance.get_global_property(alias, property)?;
    if !crate::ui::ui_values::reconcile_in(
        slint::ComponentHandle::window(instance),
        &current,
        &value,
    )? {
        crate::ui::ui_values::set(instance, Some(alias), property, value)?;
    }
    Ok(())
}

/// Write an already converted value (JSON or a runtime image) to the owner's global.
pub fn set_global_value(
    instance: &ComponentInstance,
    spec: &CompileSpec,
    property: &str,
    value: Value,
) -> Result<()> {
    if spec.global.is_none() {
        bail!("composition contribution has no exported bridge global");
    }
    let alias = global_alias(spec.owner);
    let current = instance.get_global_property(&alias, property)?;
    if !crate::ui::ui_values::reconcile_in(
        slint::ComponentHandle::window(instance),
        &current,
        &value,
    )? {
        crate::ui::ui_values::set(instance, Some(&alias), property, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Step 3a: a bar's `content-min-width` is read only where its file
    /// declares it (comments do not count), and a shell without a bottom
    /// bar holds the player's strip in a band of its own.
    #[test]
    fn the_bar_minimum_is_read_where_declared_and_the_band_holds_the_strip() {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let has =
            |file: &std::path::Path, component: &str| probe_length(file, component, BAR_MIN_WIDTH);
        // Declared on the component, or inherited from a base component in
        // another file (the interpreter's view, not the text).
        assert!(has(&fixtures.join("min-bar.slint"), "MinBar"));
        assert!(has(&fixtures.join("min-derived.slint"), "DerivedBar"));
        assert!(!has(&fixtures.join("strip-bar.slint"), "StripBar"));
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            path
        };
        // Only in a comment, on another component of the file, of another
        // type, a private property, or a file that does not compile: no.
        let comment = write(
            "comment.slint",
            "// out property <length> content-min-width: 3px;\nexport component B inherits Rectangle {}\n",
        );
        assert!(!has(&comment, "B"));
        let other = write(
            "other.slint",
            "component Part inherits Rectangle { out property <length> content-min-width: 3px; }\n\
             export component B inherits Rectangle { Part {} }\n",
        );
        assert!(!has(&other, "B"));
        let typed = write(
            "typed.slint",
            "export component B inherits Rectangle { out property <int> content-min-width: 3; }\n",
        );
        assert!(!has(&typed, "B"));
        let private = write(
            "private.slint",
            "export component B inherits Rectangle { property <length> content-min-width: 3px; }\n",
        );
        assert!(!has(&private, "B"));
        let broken = write(
            "broken.slint",
            "export component B inherits Rectangle { nonsense: 3px; }\n",
        );
        assert!(!has(&broken, "B"));
        // The shell reads it from the inherited declaration too.
        let derived = shell_source(&[CompileSpec {
            owner: 2,
            file: fixtures.join("min-derived.slint"),
            component: "DerivedBar".into(),
            global: Some("DerivedBridge".into()),
            slot: Slot::Bottom,
        }])
        .unwrap();
        assert!(derived.contains("bottom-content.content-min-width"));
        build(derived, "HostWindow").unwrap();
    }

    /// The probe's answer lasts one run of the bar (its owner generation):
    /// a base component in another file that changes (an update, a reload:
    /// a new run) is probed again, though the bar's own file did not change.
    #[test]
    fn the_bar_minimum_is_probed_again_for_each_run_of_the_bar() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base.slint");
        let bar = dir.path().join("bar.slint");
        std::fs::write(
            &base,
            "export component Base inherits Rectangle { out property <length> content-min-width: 9px; }\n",
        )
        .unwrap();
        std::fs::write(
            &bar,
            "import { Base } from \"base.slint\";\nexport component Bar inherits Base {}\n",
        )
        .unwrap();
        assert!(declares_length(41, &bar, "Bar", BAR_MIN_WIDTH));
        // The next version's base drops the property; bar.slint is untouched.
        std::fs::write(&base, "export component Base inherits Rectangle {}\n").unwrap();
        assert!(
            declares_length(41, &bar, "Bar", BAR_MIN_WIDTH),
            "the same run keeps its answer"
        );
        assert!(
            !declares_length(42, &bar, "Bar", BAR_MIN_WIDTH),
            "the next run (new owner generation) probes again"
        );
        std::fs::write(
            &base,
            "export component Base inherits Rectangle { out property <length> content-min-width: 9px; }\n",
        )
        .unwrap();
        assert!(declares_length(43, &bar, "Bar", BAR_MIN_WIDTH));
        let empty = shell_source(&[]).unwrap();
        assert_eq!(empty.matches("HostBarStrip {").count(), 1, "{empty}");
        assert!(empty.contains("background: Theme.bg-bar"));
        let old = shell_source(&[CompileSpec {
            owner: 1,
            file: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/mount/strip-bar.slint"),
            component: "StripBar".into(),
            global: Some("StripBridge".into()),
            slot: Slot::Bottom,
        }])
        .unwrap();
        assert_eq!(old.matches("HostBarStrip {").count(), 1);
        assert!(!old.contains("content-min-width"));
        assert!(old.contains("room: max(0px, parent.width - 0px - 4px)"));
    }

    #[test]
    fn backdrop_follows_the_source_without_flicker() {
        use serde_json::json;
        let state =
            |phase: &str, video: bool| json!({"phase":phase,"kind":"media","videoOutput":video});
        assert!(no_video(false, &state("empty", false)));
        assert!(no_video(false, &state("ready", false)));
        assert!(!no_video(true, &state("ready", true)));
        assert!(!no_video(true, &state("ended", true)));
        // G-16a: loading shows the backdrop; error keeps what is shown.
        assert!(no_video(true, &state("loading", false)));
        assert!(no_video(false, &state("loading", false)));
        assert!(!no_video(false, &state("error", false)));
        assert!(no_video(true, &state("error", false)));
        // For picture-in-picture, loading keeps the previous answer.
        assert!(!video_absent(false, &state("loading", false)));
        assert!(video_absent(true, &state("loading", false)));
        assert!(!video_absent(false, &state("error", false)));
        assert!(video_absent(false, &state("ready", false)));
        assert!(!no_video(true, &json!({"phase":"ready","kind":"producer"})));
        assert!(no_video(false, &Json::Null));
    }

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
        assert!(source.contains("root.no-video"));
        assert!(source.contains("key-input(\"panel:sidebar\""));
        assert!(source.contains("bottom-content := Owner10View"));
        assert!(source.contains("visible: root.bottom-visible"));
        assert!(!source.contains("// VESTRA-"));
        // G-04: the bar reports hover; G-01: the fullscreen sidebar overlay is
        // a second instance above the video and the docked layout.
        assert!(source.contains("root.bottom-hover = self.has-hover"));
        // FS-BAR-1: the bar's laid-out height is exported for the band.
        assert!(
            source.contains("out property <float> bottom-band-height: bottom-keys.height / 1px;")
        );
        assert!(source.contains("root.sidebar-hover = self.has-hover"));
        let overlay = source.find("if root.sidebar-overlay:").unwrap();
        assert!(
            overlay
                > source
                    .find("if root.sidebar-visible: sidebar-keys := FocusScope")
                    .unwrap()
        );
        assert!(overlay < source.find("if root.menu-open: Rectangle").unwrap());
        // The fullscreen title strip: above the video and the sidebar
        // overlay, below the menu layer; one instance, not an `if`.
        let strip = source.find("title-strip := TitleStrip {").unwrap();
        assert!(overlay < strip && strip < source.find("if root.menu-open: Rectangle").unwrap());
        assert_eq!(source.matches("Owner20View {").count(), 2);
        // Reference §6.3: bottom-bar content may draw above its slot (negative
        // y) over the video; nothing between it and the window clips it.
        let bottom = &source[..source.find("bottom-content := Owner10View").unwrap()];
        let wrapper = &bottom[bottom.rfind("Rectangle {").unwrap()..];
        assert!(
            !wrapper.contains("clip"),
            "bottom bar wrapper clips: {wrapper}"
        );
        assert!(!include_str!("../../ui/host.slint").contains("clip"));
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
        let mut with_backdrop = specs.to_vec();
        with_backdrop.push(CompileSpec {
            owner: 30,
            file: base.join("backdrop.slint"),
            component: "Backdrop".into(),
            global: Some("Bridge".into()),
            slot: Slot::Backdrop,
        });
        let definition = compile_definition(&with_backdrop).unwrap();
        // The backdrop sits before the video input layer, so it is drawn under it.
        let source = shell_source(&with_backdrop).unwrap();
        assert!(source.find("Owner30View").unwrap() < source.find("mouse := TouchArea").unwrap());
        assert!(definition.global_properties("Owner30Bridge").is_some());
        let floating = build(
            floating_source(&specs[1..], "播放列表").unwrap(),
            "FloatingPanel",
        )
        .unwrap();
        // Player 0.5.10: both windows export the bar buttons' global (bar_buttons.rs).
        for window in [&definition, &floating] {
            let names: Vec<_> = window
                .global_properties(crate::ui::bar_buttons::GLOBAL)
                .expect("BarButtonsHost exported")
                .map(|(name, _)| name)
                .collect();
            for name in ["buttons", "toggle", "placed"] {
                assert!(names.iter().any(|n| n == name), "{name}");
            }
        }
    }

    #[test]
    fn bounded_json_callbacks_preserve_structs_and_reject_overload() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let row = serde_json::json!({"rowId": "row-a", "label": "A"});
        let value = crate::ui::ui_values::from_json(&row).unwrap();
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

    fn tab_specs() -> Vec<CompileSpec> {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        vec![
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
            CompileSpec {
                owner: 30,
                file: base.join("tab.slint"),
                component: "NoteTab".into(),
                global: Some("Bridge".into()),
                slot: Slot::Sidebar,
            },
        ]
    }

    #[test]
    fn one_sidebar_panel_has_no_tab_strip_and_two_have_one_branch_each() {
        let specs = tab_specs();
        // One panel: the column is the panel itself, as before tabs.
        let single = shell_source(&specs[..2]).unwrap();
        assert!(!single.contains("SidebarTabs") && !single.contains("sidebar-shown =="));
        assert!(single.contains(
            "sidebar-keys := FocusScope { width: root.sidebar-actual * 1px; horizontal-stretch: 0; key-pressed"
        ));
        // The panel key scopes (docked, overlay, bottom bar, detached) keep
        // Slint's click-to-focus, so `focus: "panel"` bindings work as
        // before 0.7.6; only the window's outermost `keys` (host.slint)
        // opts out.
        assert_eq!(single.matches("focus-on-click").count(), 1);
        assert!(
            !floating_source(&specs[1..2], "List")
                .unwrap()
                .contains("focus-on-click")
        );
        // Two: the strip and one `if` per compiled tab, keyed by its owner
        // (`sidebar-shown`), in the docked column and the fullscreen overlay alike.
        let tabbed = shell_source(&specs).unwrap();
        assert_eq!(tabbed.matches("SidebarTabs {").count(), 2);
        assert_eq!(
            tabbed
                .matches("if root.sidebar-shown == 20: Owner20View")
                .count(),
            2
        );
        assert_eq!(
            tabbed
                .matches("if root.sidebar-shown == 30: Owner30View")
                .count(),
            2
        );
        assert!(tabbed.contains("sidebar-keys.focus()") && tabbed.contains("overlay-keys.focus()"));
        // sidebar-tabs.md: one compiled panel among several tabs still has
        // the strip; none compiled (every front tab a placeholder) keeps the
        // column and the strip.
        let one = shell_source_tabbed(&specs[..2], true).unwrap();
        assert_eq!(one.matches("SidebarTabs {").count(), 2);
        assert_eq!(
            one.matches("if root.sidebar-shown == 20: Owner20View")
                .count(),
            2
        );
        let none = shell_source_tabbed(&specs[..1], true).unwrap();
        assert_eq!(none.matches("SidebarTabs {").count(), 2);
        assert!(none.contains("sidebar-keys := FocusScope"));
        assert!(
            !shell_source_tabbed(&specs[..1], false)
                .unwrap()
                .contains("sidebar-keys")
        );
        build(one, "HostWindow").unwrap();
        build(none, "HostWindow").unwrap();
        // Bottom bar and backdrop stay one each.
        let mut two_bars = specs.clone();
        two_bars[2].slot = Slot::Bottom;
        assert!(shell_source(&two_bars).is_err());
    }

    #[test]
    fn tabbed_shell_and_floating_window_compile_and_switch_by_property() {
        let specs = tab_specs();
        let definition = compile_definition(&specs).unwrap();
        for property in ["sidebar-active", "sidebar-titles"] {
            assert!(definition.properties().any(|(name, _)| name == property));
        }
        for callback in ["sidebar-tab", "sidebar-overflow", "sidebar-detach"] {
            assert!(definition.callbacks().any(|name| name == callback));
        }
        // No close button in the sidebar (user ruling 2026-11-07).
        assert!(!definition.callbacks().any(|name| name == "sidebar-close"));
        assert!(definition.global_properties("Owner30Bridge").is_some());
        let floating = build(
            floating_source(&specs[1..], "播放列表").unwrap(),
            "FloatingPanel",
        )
        .unwrap();
        assert!(floating.global_properties("Owner20Bridge").is_some());
        assert!(floating.global_properties("Owner30Bridge").is_some());
        assert!(floating.properties().any(|(name, _)| name == "panel-title"));
        assert!(
            floating
                .properties()
                .any(|(name, _)| name == "sidebar-shown")
        );
        // The floating window with one compiled panel among several tabs.
        build(
            floating_source_tabbed(&specs[1..2], "播放列表", true).unwrap(),
            "FloatingPanel",
        )
        .unwrap();
    }

    fn adjust_spec(owner: u64) -> CompileSpec {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        CompileSpec {
            owner,
            file: base.join("adjust.slint"),
            component: "LevelPanel".into(),
            global: Some("AdjustBridge".into()),
            slot: Slot::Adjust,
        }
    }

    /// Menu redesign step 3b: `[video][adjust][sidebar]`. The adjust column
    /// sits between the video column and the sidebar, docked with its own
    /// splitter, and has one overlay (over a narrow window's video, and the
    /// fullscreen slide-out left of the sidebar's); its strip always shows,
    /// with the keys under the titles and no detach button.
    #[test]
    fn the_adjust_column_sits_between_the_video_and_the_sidebar() {
        let mut specs = tab_specs();
        specs.push(adjust_spec(40));
        let source = shell_source(&specs).unwrap();
        // Both columns: the adjust column docked before the sidebar.
        let adjust = source
            .find("if root.adjust-visible: adjust-keys := FocusScope")
            .unwrap();
        let sidebar = source
            .find("if root.sidebar-visible: sidebar-keys := FocusScope")
            .unwrap();
        assert!(adjust < sidebar);
        assert!(source.find("bottom-content := Owner10View").unwrap() < adjust);
        // The docked column, the narrow window's overlay and the sidebar
        // all keep Slint's click-to-focus (`focus: "panel"` bindings);
        // only the window's outermost `keys` (host.slint) opts out.
        assert_eq!(source.matches("focus-on-click").count(), 1);
        // One branch docked, one in the overlay, keyed by its owner.
        assert_eq!(
            source
                .matches("if root.adjust-shown == 40: Owner40View")
                .count(),
            2
        );
        assert!(source.contains("key-input(\"panel:adjust\""));
        // GUI check of 3b: a panel wider than its column is clipped to it
        // (docked and in the overlay), never drawn over the sidebar.
        assert_eq!(
            source
                .matches("clip: true; if root.adjust-shown == 40")
                .count(),
            2
        );
        assert!(source.contains("keys: root.adjust-keys; detachable: false;"));
        assert!(!source.contains("root.adjust-detach"));
        // The overlay comes after the sidebar's (G-01), before the title
        // strip and the menu layer (G-19); Esc closes it in a window.
        let overlay = source.find("if root.adjust-overlay: Rectangle").unwrap();
        assert!(source.find("if root.sidebar-overlay:").unwrap() < overlay);
        assert!(overlay < source.find("title-strip := TitleStrip {").unwrap());
        assert!(source.contains("root.adjust-dismiss()"));
        assert!(source.contains("root.adjust-hover = self.has-hover"));
        // The fullscreen bar ends at both popped-out columns.
        assert!(source.contains("root.columns-popped-width"));
        let definition = build(source, "HostWindow").unwrap();
        for callback in [
            "adjust-tab",
            "adjust-overflow",
            "adjust-dismiss",
            "adjust-resized",
        ] {
            assert!(
                definition.callbacks().any(|name| name == callback),
                "{callback}"
            );
        }
        for property in [
            "adjust-keys",
            "adjust-width",
            "adjust-actual",
            "adjust-pop-width",
        ] {
            assert!(
                definition.properties().any(|(name, _)| name == property),
                "{property}"
            );
        }
        assert!(definition.global_properties("Owner40Bridge").is_some());
        // The adjust column alone: one tab still draws the strip.
        let alone = shell_source(&[specs[0].clone(), adjust_spec(40)]).unwrap();
        assert_eq!(alone.matches("SidebarTabs {").count(), 2);
        assert!(!alone.contains("sidebar-keys"));
        build(alone, "HostWindow").unwrap();
        // Every adjust tab a placeholder (none compiled yet, the column not
        // opened): the column and its strip exist, with no branch.
        let placeholders = shell_source_columns(&specs[..1], [false, true]).unwrap();
        assert!(placeholders.contains("adjust-keys := FocusScope"));
        assert!(!placeholders.contains("adjust-shown =="));
        build(placeholders, "HostWindow").unwrap();
        // No adjust tab at all: no column.
        assert!(
            !shell_source(&specs[..2])
                .unwrap()
                .contains("adjust-keys := FocusScope")
        );
        // The bare shell keeps the adjust properties, so the host binds them alike.
        build(shell_source(&[]).unwrap(), "HostWindow").unwrap();
        assert_eq!(Slot::parse("adjust"), Some(Slot::Adjust));
        assert_eq!(Slot::parse("top"), None);
        assert!(Slot::Adjust.tabbed() && !Slot::Bottom.tabbed());
    }

    /// The adjust column's place in the shell: docked at 317 between a
    /// video of the rest and the sidebar; in a narrow window over the
    /// video's right side, 300 wide; in fullscreen slid out left of the
    /// sidebar's overlay (headless, laid out by Slint).
    #[test]
    fn the_adjust_column_lays_out_docked_over_the_video_and_in_fullscreen() {
        use slint::ComponentHandle as _;
        struct Headless;
        impl slint::platform::Platform for Headless {
            fn create_window_adapter(
                &self,
            ) -> std::result::Result<
                std::rc::Rc<dyn slint::platform::WindowAdapter>,
                slint::PlatformError,
            > {
                Ok(
                    slint::platform::software_renderer::MinimalSoftwareWindow::new(
                        Default::default(),
                    ),
                )
            }
        }
        thread_local! {
            static PLATFORM: () = { let _ = slint::platform::set_platform(Box::new(Headless)); };
        }
        PLATFORM.with(|_| {});
        let mut specs = tab_specs();
        specs.push(adjust_spec(40));
        let instance = compile_definition(&specs).unwrap().create().unwrap();
        let number = |name: &str| match instance.get_property(name).unwrap() {
            Value::Number(n) => n,
            other => panic!("{name}: {other:?}"),
        };
        let set = |name: &str, on: bool| instance.set_property(name, Value::Bool(on)).unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(1400., 800.));
        set("sidebar-visible", true);
        set("adjust-visible", true);
        crate::ui::view::settle_layout(instance.window());
        // 1400 - 317 - 3 - 317 - 3 = 760 of video.
        assert_eq!(number("adjust-actual"), 317.);
        assert_eq!(number("video-width"), 760.);
        // Narrow: the policy floats it over the video; the video gets the width back.
        set("adjust-visible", false);
        set("adjust-overlay", true);
        set("adjust-popped", true);
        crate::ui::view::settle_layout(instance.window());
        assert_eq!(number("video-width"), 1080.);
        // Half the window at most, 280 at least.
        instance
            .set_property("adjust-width", Value::Number(1000.))
            .unwrap();
        assert_eq!(number("adjust-actual"), 700.);
        instance
            .set_property("adjust-width", Value::Number(10.))
            .unwrap();
        assert_eq!(number("adjust-actual"), 280.);
        // Fullscreen: both popped out cover their widths at the right edge.
        instance
            .set_property("adjust-width", Value::Number(317.))
            .unwrap();
        set("host-fullscreen", true);
        set("sidebar-overlay", true);
        set("sidebar-popped", true);
        set("sidebar-visible", false);
        crate::ui::view::settle_layout(instance.window());
        assert_eq!(number("adjust-pop-width"), 317.);
        assert_eq!(number("columns-popped-width"), 317. + 317.);
        let pop = crate::ui::window_state::adjust_overlay_width(1400., 317., 317.);
        assert_eq!(number("adjust-pop-width"), pop);
    }

    /// Menu redesign, open question 1: what the adjust column's first
    /// opening costs (its panels compile on demand). The shell holds the
    /// real playback bar, visualizer backdrop and playlist sidebar; the
    /// adjust panels are the four built-in ones of step 4 (Picture, Audio,
    /// Subtitles, Playback), as their plugins mount them.
    /// Release build, headless: `cargo test --release --locked -- --ignored
    /// the_adjust_columns_first_opening_cost --nocapture`.
    #[test]
    #[ignore = "measurement"]
    #[allow(clippy::disallowed_macros)] // the numbers for the terminal
    fn the_adjust_columns_first_opening_cost() {
        use slint::ComponentHandle as _;
        struct Headless;
        impl slint::platform::Platform for Headless {
            fn create_window_adapter(
                &self,
            ) -> std::result::Result<
                std::rc::Rc<dyn slint::platform::WindowAdapter>,
                slint::PlatformError,
            > {
                Ok(
                    slint::platform::software_renderer::MinimalSoftwareWindow::new(
                        Default::default(),
                    ),
                )
            }
        }
        let _ = slint::platform::set_platform(Box::new(Headless));
        let plugins = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let spec = |owner, file: PathBuf, component: &str, global: &str, slot| CompileSpec {
            owner,
            file,
            component: component.into(),
            global: Some(global.into()),
            slot,
        };
        let base = vec![
            spec(
                1,
                plugins.join("playback/ui/bar.slint"),
                "PlaybackBar",
                "PlaybackBridge",
                Slot::Bottom,
            ),
            spec(
                2,
                plugins.join("visualizer/ui/backdrop.slint"),
                "Backdrop",
                "Art",
                Slot::Backdrop,
            ),
            spec(
                3,
                plugins.join("playlist/ui/sidebar.slint"),
                "PlaylistSidebar",
                "PlaylistBridge",
                Slot::Sidebar,
            ),
        ];
        // The four built-in adjust panels (menu redesign step 4), as their
        // plugins mount them.
        let adjust = [
            spec(
                10,
                plugins.join("video/picture.slint"),
                "Picture",
                "PictureBridge",
                Slot::Adjust,
            ),
            spec(
                11,
                plugins.join("audio/ui/workbench.slint"),
                "Audio",
                "AudioBridge",
                Slot::Adjust,
            ),
            spec(
                12,
                plugins.join("subtitles/ui/panel.slint"),
                "SubtitlesPanel",
                "SubtitlesBridge",
                Slot::Adjust,
            ),
            spec(
                13,
                plugins.join("playback/ui/panel.slint"),
                "PlaybackPanel",
                "PlaybackPanelBridge",
                Slot::Adjust,
            ),
        ];
        let median = |mut runs: Vec<f64>| {
            runs.sort_by(f64::total_cmp);
            runs[runs.len() / 2]
        };
        let time = |specs: &[CompileSpec], tabbed: [bool; 2]| {
            let mut compile = Vec::new();
            let mut whole = Vec::new();
            for _ in 0..7 {
                let started = std::time::Instant::now();
                let definition = compile_definition_tabbed(specs, tabbed).unwrap();
                compile.push(started.elapsed().as_secs_f64() * 1000.);
                let instance = definition.create().unwrap();
                instance
                    .window()
                    .set_size(slint::LogicalSize::new(1400., 800.));
                instance
                    .set_property("adjust-visible", Value::Bool(true))
                    .unwrap();
                crate::ui::view::settle_layout(instance.window());
                whole.push(started.elapsed().as_secs_f64() * 1000.);
            }
            (median(compile), median(whole))
        };
        let (closed, closed_whole) = time(&base, [false, true]);
        println!(
            "adjust column closed (placeholders): compile {closed:.0} ms, whole {closed_whole:.0} ms"
        );
        for count in 1..=adjust.len() {
            let mut specs = base.clone();
            specs.extend(adjust[..count].iter().cloned());
            let (compile, whole) = time(&specs, [false, true]);
            println!(
                "{count} adjust panel(s): compile {compile:.0} ms, whole {whole:.0} ms (+{:.0} ms over closed)",
                whole - closed_whole
            );
        }
    }

    /// A new video picture must not touch the
    /// window's layout. When the picture's size fed it, every frame redid the
    /// layout and everything that depends on it (with a UI Automation client
    /// and Slint's former accessibility support, about 30 ms a frame with
    /// VestraTube's 200 rows on the Radeon 780M laptop).
    #[test]
    fn a_new_video_picture_leaves_the_window_layout_alone() {
        use slint::ComponentHandle as _;
        struct Headless;
        impl slint::platform::Platform for Headless {
            fn create_window_adapter(
                &self,
            ) -> std::result::Result<
                std::rc::Rc<dyn slint::platform::WindowAdapter>,
                slint::PlatformError,
            > {
                Ok(
                    slint::platform::software_renderer::MinimalSoftwareWindow::new(
                        Default::default(),
                    ),
                )
            }
        }
        thread_local! {
            static PLATFORM: () = slint::platform::set_platform(Box::new(Headless)).unwrap();
        }
        PLATFORM.with(|_| {});
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        let specs = [CompileSpec {
            owner: 20,
            file: base.join("sidebar.slint"),
            component: "RowsSidebar".into(),
            global: Some("Bridge".into()),
            slot: Slot::Sidebar,
        }];
        let instance = compile_definition(&specs).unwrap().create().unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(1000., 600.));
        instance
            .set_property("no-video", Value::Bool(false))
            .unwrap();
        let picture = |width, height| {
            Value::Image(slint::Image::from_rgb8(slint::SharedPixelBuffer::new(
                width, height,
            )))
        };
        instance
            .set_property("video-frame", picture(64, 36))
            .unwrap();
        // What depends on the layout: the video area's size and place.
        let tracker: std::pin::Pin<Box<slint::private_unstable_api::re_exports::PropertyTracker>> =
            Box::pin(Default::default());
        let probe = || {
            ["video-width", "video-height", "video-x", "sidebar-actual"]
                .map(|name| instance.get_property(name).unwrap())
        };
        let before = tracker.as_ref().evaluate(probe);
        assert!(!tracker.is_dirty());
        instance
            .set_property("video-frame", picture(1920, 1080))
            .unwrap();
        assert!(
            !tracker.is_dirty(),
            "a new video picture marked the window layout dirty"
        );
        assert_eq!(tracker.as_ref().evaluate(probe), before);
        // The probe does see the layout: a resize marks it.
        instance
            .window()
            .set_size(slint::LogicalSize::new(1200., 600.));
        assert!(tracker.is_dirty());
    }
}
