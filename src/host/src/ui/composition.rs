//! Slint composition of the shell's slots: the shell and the sidebar's
//! floating window, generated once, with a container per slot; the panels
//! are compiled each on their own and shown in the containers
//! (`ui/panels.rs`). Host-internal: the public contract is the SDK
//! window/panel API.
#![forbid(unsafe_code)]

use std::{path::PathBuf, sync::mpsc};

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
    /// Every slot, in the shell's order of its places.
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Sidebar, Self::Backdrop, Self::Adjust];
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
        Self::ALL.into_iter().find(|slot| slot.name() == name)
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

/// One panel: a component of a plugin's Slint file and its bridge global,
/// for a slot. `owner` is the panel's key (`host::panel_key`). The bottom
/// bar and the backdrop hold one panel each; the sidebar and the adjust
/// column any number, as tabs. Components inherit a native visual element,
/// not Window.
#[derive(Clone, Debug, PartialEq)]
pub struct CompileSpec {
    pub owner: u64,
    pub file: PathBuf,
    pub component: String,
    pub global: Option<String>,
    pub slot: Slot,
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

/// The shell property (and the floating window's, for the sidebar) with
/// the restart line of the panel a slot shows (`""`: live).
pub fn restarting_property(slot: Slot) -> String {
    format!("{}-restarting", slot.name())
}

/// Whether the restart line of the panel a slot shows has the keys (a
/// click on it gave them).
pub fn restarting_focus_property(slot: Slot) -> String {
    format!("{}-restarting-focused", slot.name())
}

/// A slot's restart line (`host/panel_restart.rs`): while the plugin of
/// the panel it shows restarts after a crash, the host dims the panel's
/// last content and draws one line over it, taking the pointer
/// (`ui/panel-restarting.slint`); a click on it gives it the keys, which
/// `<slot>-restarting-focused` reports. One per place of the slot: the
/// host writes the line of the panel shown there.
fn restarting_overlay(slot: Slot) -> String {
    let p = slot.name();
    format!(
        "if root.{p}-restarting != \"\": PanelRestarting {{ x: 0px; y: 0px; \
         width: parent.width; height: parent.height; text: root.{p}-restarting; \
         changed holds-keys => {{ root.{p}-restarting-focused = self.holds-keys; }} }}"
    )
}

/// The restart line's properties of `slots`: its text and whether it has
/// the keys.
fn restarting_properties(slots: &[Slot]) -> String {
    slots
        .iter()
        .map(|slot| {
            format!(
                "\n    in property <string> {0}-restarting;\
                 \n    in-out property <bool> {0}-restarting-focused;",
                slot.name()
            )
        })
        .collect()
}

/// The container of a slot's panel (`ui/panels.rs`): the host sets
/// `<slot>-factory` to the shown panel's factory. Placed (x, y), so its
/// panel's size constraints stay out of the layouts around it: they would
/// depend on the size the container gives the panel, and Slint reads them
/// while creating the panel inside that layout ("Recursion detected": the
/// visualizer's backdrop, whose scenes take its size).
fn container(slot: Slot) -> String {
    format!(
        "ComponentContainer {{ x: 0px; y: 0px; width: parent.width; height: parent.height; component-factory: root.{}-factory; }}",
        slot.name()
    )
}

/// The shell's slot properties: each slot's container factory, the tab
/// properties of its columns, and the bottom bar's (`bottom-mounted`: a
/// bar is mounted, else the player's strip has a band of its own;
/// `bottom-reserve`: the width the bar keeps for its own controls, px).
const SHELL_PROPERTIES: &str = "in property <bool> bottom-visible: true;\n\
    in property <bool> bottom-mounted: false;\n\
    in property <float> bottom-reserve: 0;\n\
    in property <component-factory> bottom-factory;\n\
    in property <component-factory> backdrop-factory;\n\
    in property <bool> sidebar-visible: false;\n\
    in property <bool> sidebar-overlay: false;\n\
    in property <bool> sidebar-popped: false;\n\
    in property <bool> sidebar-tabbed: false;\n\
    in property <component-factory> sidebar-factory;\n\
    in-out property <int> sidebar-active: 0;\n\
    in property <[string]> sidebar-titles;\n\
    callback sidebar-tab(int);\n\
    callback sidebar-overflow(float, float);\n\
    callback sidebar-detach();\n\
    in property <bool> adjust-visible: false;\n\
    in property <bool> adjust-overlay: false;\n\
    in property <bool> adjust-popped: false;\n\
    in property <component-factory> adjust-factory;\n\
    in-out property <int> adjust-active: 0;\n\
    in property <[string]> adjust-titles;\n\
    in property <[string]> adjust-keys;\n\
    callback adjust-tab(int);\n\
    callback adjust-overflow(float, float);\n\
    callback adjust-dismiss();";

/// The height of the host's sidebar tab strip (the playlist's).
pub const TAB_STRIP_HEIGHT: f64 = 31.0;

/// What fills a column of tabs (docked, overlay, the sidebar's floating
/// window): the tab strip (the sidebar: while it has two or more tabs,
/// `sidebar-tabbed`; the adjust column: always) over the container of the
/// current tab's panel, so switching between tabs is a property change. A
/// tab without a panel (a placeholder, one that does not compile) shows
/// nothing. A click on a tab gives the keys to `keys` (the column's
/// FocusScope): the panel that had them is gone. The adjust column's strip
/// has the keys under its titles and no detach button; `background`: the
/// panels' ground under the strip.
fn column_content(column: Column, keys: &str, detached: &str, background: &str) -> String {
    let p = column.prefix();
    let (tabbed, strip) = match column {
        Column::Sidebar => (
            "root.sidebar-tabbed",
            format!("detached: {detached}; detach => {{ root.sidebar-detach(); }}"),
        ),
        Column::Adjust => (
            "true",
            "keys: root.adjust-keys; detachable: false;".to_owned(),
        ),
    };
    let (container, restarting) = (container(column.slot()), restarting_overlay(column.slot()));
    // Placed by hand, not in a layout: a layout here would pass its size
    // constraints up to the window (it shrank to its minimum height). One
    // panel without a strip fills the column unclipped, as it always did.
    format!(
        "if {tabbed}: SidebarTabs {{ x: 0px; y: 0px; width: parent.width; height: {TAB_STRIP_HEIGHT}px; \
         titles: root.{p}-titles; active: root.{p}-active; {strip} \
         tab(index) => {{ root.{p}-active = index; {keys}.focus(); root.{p}-tab(index); }} \
         overflow(x, y) => {{ root.{p}-overflow(x, y); }} }} \
         Rectangle {{ x: 0px; y: {tabbed} ? {TAB_STRIP_HEIGHT}px : 0px; width: parent.width; \
         height: parent.height - self.y; background: {tabbed} ? {background} : transparent; \
         clip: {tabbed}; {container} {restarting} }}"
    )
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
/// its least width). The bar's own compilation reads whichever it has
/// (`ui/panels.rs`, `RESERVE`), and the host copies the number to the
/// shell's `bottom-reserve`.
pub const BAR_FULL_WIDTH: &str = "content-full-width";

/// The player's bottom-bar strip (bar-strip.slint) at the right end of its
/// parent, its cells on the parent's bottom edge; `min`: what the bar
/// keeps for its own controls.
fn bar_strip(min: &str) -> String {
    format!(
        "HostBarStrip {{ x: parent.width - self.width - 4px; y: parent.height - self.height; \
         room: max(0px, parent.width - {min} - 4px); more(x, y) => {{ root.bar-more(x, y); }} }}"
    )
}

/// The shell: ordinary Slint, generated once. It names no panel: every
/// slot is a container the host fills (`ui/panels.rs`), so the same
/// source serves whatever is mounted, and a mount change never compiles
/// it again.
pub fn shell_source() -> Result<String> {
    let mut source = String::from(
        "import { PanelRestarting } from \"panel-restarting.slint\";\n\
         import { SidebarTabs } from \"sidebar-tabs.slint\";\n",
    );
    let mut shell = include_str!("../../ui/host.slint").to_owned();
    // FS-BAR-1: the bottom bar's laid-out height, also while it is hidden
    // (the host's fullscreen band); 0 without a bottom bar.
    let properties = format!(
        "{SHELL_PROPERTIES}{}\n    out property <float> bottom-band-height: root.bottom-mounted ? bottom-keys.height / 1px : 0;",
        restarting_properties(&Slot::ALL),
    );
    replace_marker(&mut shell, "// VESTRA-MOUNT-PROPERTIES", &properties)?;
    let adjust = adjust_column();
    replace_marker(&mut shell, "// VESTRA-MOUNT-ADJUST", &adjust.docked)?;
    let sidebar = format!(
        "if root.sidebar-visible: Rectangle {{ width: 3px; horizontal-stretch: 0; background: Theme.line; \
         TouchArea {{ mouse-cursor: MouseCursor.ew-resize; \
         moved => {{ if self.pressed {{ root.sidebar-width = clamp(root.width / 1px - (self.absolute-position.x + self.mouse-x) / 1px - 1.5, 317, max(317, root.width / 1px - 321)); }} }} \
         pointer-event(event) => {{ if event.kind == PointerEventKind.up {{ root.sidebar-resized(root.sidebar-actual); }} }} }} }}\n\
         if root.sidebar-visible: sidebar-keys := FocusScope {{ width: root.sidebar-actual * 1px; horizontal-stretch: 0; {} {} }}",
        key_forward("panel:sidebar"),
        column_content(Column::Sidebar, "sidebar-keys", "false", "Theme.bg-list")
    );
    replace_marker(&mut shell, "// VESTRA-MOUNT-SIDEBAR", &sidebar)?;
    // G-01: in fullscreen the attached sidebar is an overlay over the right
    // of the video, full height; it slides out when popped and back when
    // collapsed. Its hover area reports to the host (the G-04 mechanism).
    let overlay = format!(
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
        column_content(Column::Sidebar, "overlay-keys", "false", "Theme.bg-list")
    );
    // The adjust column's overlay (fullscreen, narrow window) beside the
    // sidebar's: they never overlap, the sidebar outermost.
    let overlay = format!("{overlay}\n{}", adjust.overlay);
    replace_marker(&mut shell, "// VESTRA-MOUNT-OVERLAY", &overlay)?;
    // The bar's place always exists (its container is empty, and 0 high,
    // without a bar), so a bar keeps its instance while it is hidden.
    // Step 3a: without a bottom bar the player's strip still shows, in a
    // band of the bar's colour (hidden in fullscreen).
    let bottom = format!(
        "Rectangle {{ height: root.host-fullscreen || !root.bottom-visible || !root.bottom-mounted ? 0px : bottom-keys.height; vertical-stretch: 0; \
         bottom-keys := FocusScope {{ visible: root.bottom-visible && root.bottom-mounted; x: 0px; \
         width: parent.width - (root.host-fullscreen ? root.columns-popped-width * 1px : 0px); \
         height: min(bottom-content.max-height, max(bottom-content.min-height, bottom-content.preferred-height)); \
         y: root.host-fullscreen ? -self.height : 0px; {keys} \
         TouchArea {{ changed has-hover => {{ root.bottom-hover = self.has-hover; }} \
         changed mouse-x => {{ root.window-input(\"activity\"); }} \
         changed mouse-y => {{ root.window-input(\"activity\"); }} \
         bottom-content := ComponentContainer {{ x: 0px; y: 0px; width: parent.width; height: parent.height; component-factory: root.bottom-factory; }} \
         {restarting} if root.bottom-mounted: {strip} }} }} }}\n\
         if !root.bottom-mounted: Rectangle {{ height: root.host-fullscreen ? 0px : 40px; vertical-stretch: 0; \
         visible: !root.host-fullscreen; background: Theme.bg-bar; \
         Rectangle {{ x: 0px; y: 0px; width: parent.width; height: 1px; background: Theme.line; }} \
         {band} }}",
        keys = key_forward("panel:bottom"),
        restarting = restarting_overlay(Slot::Bottom),
        strip = bar_strip("root.bottom-reserve * 1px"),
        band = bar_strip("0px"),
    );
    replace_marker(&mut shell, "// VESTRA-MOUNT-BOTTOM", &bottom)?;
    // Under the video input layer, the OSD and the information blocks; it
    // never receives the pointer (the TouchArea above covers it) or focus.
    let backdrop = format!(
        "Rectangle {{ visible: root.no-video; {} {} }}",
        container(Slot::Backdrop),
        restarting_overlay(Slot::Backdrop)
    );
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

/// [`AdjustColumn`]: it always draws its strip (text tabs with the keys
/// under them, no detach) over its container.
fn adjust_column() -> AdjustColumn {
    let keys = key_forward("panel:adjust");
    // The splitter's right edge: the window's, less a docked sidebar.
    let docked = format!(
        "if root.adjust-visible: Rectangle {{ width: 3px; horizontal-stretch: 0; background: Theme.line; \
         TouchArea {{ mouse-cursor: MouseCursor.ew-resize; \
         moved => {{ if self.pressed {{ root.adjust-width = clamp(root.width / 1px - (root.sidebar-visible ? root.sidebar-actual + 3 : 0) - (self.absolute-position.x + self.mouse-x) / 1px - 1.5, 280, max(280, root.width / 1px * 0.5)); }} }} \
         pointer-event(event) => {{ if event.kind == PointerEventKind.up {{ root.adjust-resized(root.adjust-actual); }} }} }} }}\n\
         if root.adjust-visible: adjust-keys := FocusScope {{ width: root.adjust-actual * 1px; horizontal-stretch: 0; {keys} {} }}",
        column_content(Column::Adjust, "adjust-keys", "false", "Theme.bg-list")
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
            "adjust-overlay-keys",
            "false",
            "root.host-fullscreen ? Theme.bg-list : transparent"
        )
    );
    AdjustColumn { docked, overlay }
}

/// The detached sidebar (C14): a window of its own with the sidebar's
/// container, the shell's key forwarding, menu layer and (two or more
/// tabs, `sidebar-tabbed`) tab strip. The host sets `panel-title` to the
/// current tab's title.
pub fn floating_source() -> Result<String> {
    let content = column_content(Column::Sidebar, "panel-keys", "true", "Theme.bg-list");
    Ok(format!(
        r#"import {{ PanelRestarting }} from "panel-restarting.slint";
import {{ SidebarTabs }} from "sidebar-tabs.slint";
import {{ MenuView, MenuLevel }} from "menu-view.slint";
import {{ Theme }} from "@vestra/theme.slint";
export {{ VestraPalette }} from "palette.slint";
export {{ MenuChrome }} from "menu-view.slint";
export {{ BarButtonsHost }} from "@vestra/bar-buttons.slint";
export {{ VestraFocus }} from "@vestra/focus.slint";
export component FloatingPanel inherits Window {{
    in property <string> panel-title;
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
    in property <bool> sidebar-tabbed: false;
    in property <component-factory> sidebar-factory;
    {restarting}
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
        restarting = restarting_properties(&[Slot::Sidebar]),
    ))
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

/// Compile source the player generates (the Options window, which imports
/// plugins' settings pages) and take `component`, with the ordinary
/// compiler.
pub(crate) fn build(source: String, component: &str) -> Result<ComponentDefinition> {
    build_with(source, component, false)
}

/// [`build`] for the shell and the floating window, which import no
/// plugin file: Slint's experimental features (`ComponentContainer`,
/// `component-factory`) are enabled for them and only for them, on the
/// compiler configuration (no environment variable). A plugin's panel is
/// compiled on its own without them ([`build_panel`]).
pub(crate) fn build_shell(source: String, component: &str) -> Result<ComponentDefinition> {
    build_with(source, component, true)
}

fn build_with(source: String, component: &str, experimental: bool) -> Result<ComponentDefinition> {
    let base = crate::ui::view::ui_dir().join("host.slint");
    // The player's own source (shell, options window): its catalog.
    let source = crate::ui::view::translate(source, &base);
    let mut compiler = crate::ui::view::compiler();
    compiler
        .compiler_configuration(i_slint_core::InternalToken)
        .enable_experimental = experimental;
    let result = pollster::block_on(compiler.build_from_source(source, base));
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

/// Compile one panel's own source (`panels::panel_source`: a wrapper that
/// imports the plugin's file) with the ordinary compiler, experimental
/// features off, and take its component and every file the compile read.
pub(crate) fn build_panel(source: String) -> Result<(ComponentDefinition, Vec<PathBuf>)> {
    let base = crate::ui::view::ui_dir().join("vestra-panel.slint");
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
    let paths = result.watch_paths(i_slint_core::InternalToken).to_vec();
    let definition = result
        .component(crate::ui::panels::COMPONENT)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "composition compilation: {:?}",
                result.diagnostics().collect::<Vec<_>>()
            )
        })?;
    Ok((definition, paths))
}

thread_local! {
    /// The compiled shell and floating window: their source never changes
    /// in a run, so each compiles once (the floating window when it first
    /// opens).
    static SHELL: std::cell::RefCell<Option<ComponentDefinition>> = const { std::cell::RefCell::new(None) };
    static FLOATING: std::cell::RefCell<Option<ComponentDefinition>> = const { std::cell::RefCell::new(None) };
}

fn shell_definition() -> Result<ComponentDefinition> {
    if let Some(definition) = SHELL.with(|shell| shell.borrow().clone()) {
        return Ok(definition);
    }
    let definition = build_shell(shell_source()?, "HostWindow")?;
    SHELL.with(|shell| *shell.borrow_mut() = Some(definition.clone()));
    Ok(definition)
}

/// Call on the UI thread. A new, unshown shell; into `window` when given:
/// the new shell's component tree replaces the shown one inside the same
/// native window instead of opening a new one (`view::replace_shell`: no
/// hide/show, no move, no resize; maximized and fullscreen stay). Slint
/// attaches the new tree to the window as it creates it, so a caller that
/// fails afterwards puts the old tree back with
/// [`crate::ui::view::ShownTree::restore`]. The shell holds no panel: the
/// host shows them in its containers (`ui/panels.rs`).
pub fn shell_in(window: Option<&slint::Window>) -> Result<ComponentInstance> {
    let definition = shell_definition()?;
    Ok(match window {
        Some(window) => definition.create_with_existing_window(window)?,
        None => definition.create()?,
    })
}

/// The floating panel window; not shown.
pub fn floating() -> Result<ComponentInstance> {
    let definition = match FLOATING.with(|floating| floating.borrow().clone()) {
        Some(definition) => definition,
        None => {
            let definition = build_shell(floating_source()?, "FloatingPanel")?;
            FLOATING.with(|floating| *floating.borrow_mut() = Some(definition.clone()));
            definition
        }
    };
    Ok(definition.create()?)
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

/// Deliver a panel's bridge callback: bounded and nonblocking. A full
/// queue explicitly rejects the callback; it never waits for host policy
/// or backend work on the UI thread.
pub(crate) fn deliver_json(sender: &mpsc::SyncSender<JsonCallback>, event: JsonCallback) -> bool {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ui::panels::tests::Composed;
    use slint::ComponentHandle as _;

    /// A headless Slint platform on this test thread (each test runs on a
    /// thread of its own): windows are software windows nobody shows.
    pub(crate) fn headless() {
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
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mount")
            .join(name)
    }

    fn spec(owner: u64, file: &str, component: &str, slot: Slot) -> CompileSpec {
        CompileSpec {
            owner,
            file: fixture(file),
            component: component.into(),
            global: Some("Bridge".into()),
            slot,
        }
    }

    /// The shell names no panel: every slot is a container the host fills,
    /// in the docked column and in its fullscreen overlay alike; what the
    /// shell drew around its panels stays where it was.
    #[test]
    fn the_shell_holds_every_slot_in_a_container_and_names_no_panel() {
        let source = shell_source().unwrap();
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
            "Owner",
            "sidebar-shown",
            "adjust-shown",
        ] {
            assert!(!source.contains(removed), "still has {removed}");
        }
        let containers = |slot: &str| {
            source
                .matches(&format!("component-factory: root.{slot}-factory;"))
                .count()
        };
        assert_eq!(containers("bottom"), 1);
        assert_eq!(containers("backdrop"), 1);
        assert_eq!(containers("sidebar"), 2, "docked and overlay");
        assert_eq!(containers("adjust"), 2, "docked and overlay");
        // One restart line per place, over the panel it shows.
        for slot in Slot::ALL {
            let p = slot.name();
            assert!(source.contains(&format!("in property <string> {p}-restarting;")));
            assert!(source.contains(&format!("in-out property <bool> {p}-restarting-focused;")));
        }
        assert!(source.contains("root.no-video"));
        assert!(source.contains("key-input(\"panel:sidebar\""));
        assert!(source.contains("visible: root.bottom-visible && root.bottom-mounted"));
        assert!(!source.contains("// VESTRA-"));
        // G-04: the bar reports hover; FS-BAR-1: the bar's laid-out height
        // is exported for the band (0 without a bar).
        assert!(source.contains("root.bottom-hover = self.has-hover"));
        assert!(source.contains(
            "out property <float> bottom-band-height: root.bottom-mounted ? bottom-keys.height / 1px : 0;"
        ));
        // Step 3a: the strip is at the bar's right end, beyond the width the
        // bar keeps (`bottom-reserve`), or in a band of its own without one.
        assert_eq!(source.matches("HostBarStrip {").count(), 2);
        assert!(source.contains("room: max(0px, parent.width - root.bottom-reserve * 1px - 4px)"));
        assert!(source.contains("if !root.bottom-mounted: Rectangle"));
        assert!(source.contains("background: Theme.bg-bar"));
        // G-01: the fullscreen sidebar overlay is a second place above the
        // video and the docked layout.
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
        // Reference §6.3: bottom-bar content may draw above its slot
        // (negative y) over the video; nothing between it and the window
        // clips it.
        let bottom = &source[..source.find("bottom-content := ComponentContainer").unwrap()];
        let wrapper = &bottom[bottom.rfind("Rectangle {").unwrap()..];
        assert!(
            !wrapper.contains("clip"),
            "bottom bar wrapper clips: {wrapper}"
        );
        // The backdrop sits before the video input layer, so it is drawn
        // under it.
        assert!(
            source
                .find("component-factory: root.backdrop-factory")
                .unwrap()
                < source.find("mouse := TouchArea").unwrap()
        );
        // No `clip` property in the shell's own file (the video's
        // `source-clip-*` crops its picture only).
        assert!(
            !include_str!("../../ui/host.slint")
                .split(|c: char| !(c.is_alphanumeric() || c == '-'))
                .any(|word| word == "clip")
        );
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

    /// The shell and the floating window compile (experimental containers
    /// enabled for them), with the strip's global, the tab properties and
    /// callbacks the host binds, and no per-panel property; the compiler a
    /// plugin's file gets refuses the shell's containers.
    #[test]
    fn the_shell_and_the_floating_window_compile_with_their_containers() {
        let shell = build_shell(shell_source().unwrap(), "HostWindow").unwrap();
        let floating = build_shell(floating_source().unwrap(), "FloatingPanel").unwrap();
        // Player 0.5.10: both windows export the bar buttons' global.
        for window in [&shell, &floating] {
            let names: Vec<_> = window
                .global_properties(crate::ui::bar_buttons::GLOBAL)
                .expect("BarButtonsHost exported")
                .map(|(name, _)| name)
                .collect();
            for name in ["buttons", "toggle", "placed"] {
                assert!(names.iter().any(|n| n == name), "{name}");
            }
            for property in [
                "sidebar-active",
                "sidebar-titles",
                "sidebar-tabbed",
                "sidebar-factory",
                "sidebar-restarting",
            ] {
                assert!(
                    window.properties().any(|(name, _)| name == property),
                    "{property}"
                );
            }
            for callback in ["sidebar-tab", "sidebar-overflow", "sidebar-detach"] {
                assert!(
                    window.callbacks().any(|name| name == callback),
                    "{callback}"
                );
            }
            // No close button in the sidebar (user ruling 2026-11-07).
            assert!(!window.callbacks().any(|name| name == "sidebar-close"));
        }
        for property in [
            "bottom-factory",
            "backdrop-factory",
            "adjust-factory",
            "bottom-mounted",
            "bottom-reserve",
        ] {
            assert!(
                shell.properties().any(|(name, _)| name == property),
                "{property}"
            );
        }
        assert!(floating.properties().any(|(name, _)| name == "panel-title"));
        assert!(build(shell_source().unwrap(), "HostWindow").is_err());
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

    /// The sidebar draws its strip from two tabs on (`sidebar-tabbed`), in
    /// the docked column and the fullscreen overlay alike, and one panel
    /// fills the column unclipped. The panel key scopes keep Slint's
    /// click-to-focus, so `focus: "panel"` bindings work as before 0.7.6;
    /// only the window's outermost `keys` (host.slint) opts out.
    #[test]
    fn the_sidebar_strip_shows_from_two_tabs_on() {
        let source = shell_source().unwrap();
        assert_eq!(
            source
                .matches("if root.sidebar-tabbed: SidebarTabs {")
                .count(),
            2
        );
        assert!(source.contains(
            "sidebar-keys := FocusScope { width: root.sidebar-actual * 1px; horizontal-stretch: 0; key-pressed"
        ));
        assert!(source.contains("clip: root.sidebar-tabbed;"));
        assert!(source.contains("sidebar-keys.focus()") && source.contains("overlay-keys.focus()"));
        assert_eq!(source.matches("focus-on-click").count(), 1);
        let floating = floating_source().unwrap();
        assert!(!floating.contains("focus-on-click"));
        assert_eq!(
            floating
                .matches("if root.sidebar-tabbed: SidebarTabs {")
                .count(),
            1
        );
        assert!(floating.contains("panel-keys.focus()"));
    }

    /// Menu redesign step 3b: `[video][adjust][sidebar]`. The adjust column
    /// sits between the video column and the sidebar, docked with its own
    /// splitter, and has one overlay (over a narrow window's video, and the
    /// fullscreen slide-out left of the sidebar's); its strip always shows,
    /// with the keys under the titles and no detach button.
    #[test]
    fn the_adjust_column_sits_between_the_video_and_the_sidebar() {
        let source = shell_source().unwrap();
        let adjust = source
            .find("if root.adjust-visible: adjust-keys := FocusScope")
            .unwrap();
        let sidebar = source
            .find("if root.sidebar-visible: sidebar-keys := FocusScope")
            .unwrap();
        assert!(adjust < sidebar);
        assert!(source.find("bottom-content := ComponentContainer").unwrap() < adjust);
        assert!(source.contains("key-input(\"panel:adjust\""));
        // GUI check of 3b: a panel wider than its column is clipped to it
        // (docked and in the overlay), never drawn over the sidebar.
        assert_eq!(
            source
                .matches("clip: true; ComponentContainer { x: 0px; y: 0px; width: parent.width; height: parent.height; component-factory: root.adjust-factory; }")
                .count(),
            2
        );
        assert_eq!(source.matches("if true: SidebarTabs {").count(), 2);
        assert!(source.contains("keys: root.adjust-keys; detachable: false;"));
        assert!(!source.contains("root.adjust-detach"));
        // The overlay comes after the sidebar's (G-01), before the title
        // strip and the menu layer (G-19); Esc closes it in a window.
        let overlay = source.find("if root.adjust-overlay: Rectangle").unwrap();
        assert!(source.find("if root.sidebar-overlay:").unwrap() < overlay);
        assert!(overlay < source.find("title-strip := TitleStrip {").unwrap());
        assert!(source.contains("root.adjust-dismiss()"));
        assert!(source.contains("root.adjust-hover = self.has-hover"));
        // The fullscreen bar ends at both popped-out columns, and so does
        // the title strip, following their 150 ms slide (G-19).
        assert!(source.contains("root.columns-popped-width"));
        let strip = &source[source.find("title-strip := TitleStrip {").unwrap()..];
        let strip = &strip[..strip.find("enabled:").unwrap()];
        assert!(strip.contains("width: root.width - root.columns-popped-width * 1px;"));
        assert_eq!(strip.matches("animate width { duration: 150ms;").count(), 2);
        let definition = build_shell(source, "HostWindow").unwrap();
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
        assert_eq!(Slot::parse("adjust"), Some(Slot::Adjust));
        assert_eq!(Slot::parse("top"), None);
        assert!(Slot::Adjust.tabbed() && !Slot::Bottom.tabbed());
    }

    /// The adjust column's place in the shell: docked at 317 between a
    /// video of the rest and the sidebar; in a narrow window over the
    /// video's right side, 300 wide; in fullscreen slid out left of the
    /// sidebar's overlay (headless, laid out by Slint), with panels in the
    /// containers.
    #[test]
    fn the_adjust_column_lays_out_docked_over_the_video_and_in_fullscreen() {
        headless();
        let composed = Composed::new(&[
            spec(10, "bar.slint", "SourceBar", Slot::Bottom),
            spec(20, "sidebar.slint", "RowsSidebar", Slot::Sidebar),
            CompileSpec {
                global: Some("AdjustBridge".into()),
                ..spec(40, "adjust.slint", "LevelPanel", Slot::Adjust)
            },
        ]);
        let instance = &composed.shell;
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
        composed.settle();
        // 1400 - 317 - 3 - 317 - 3 = 760 of video.
        assert_eq!(number("adjust-actual"), 317.);
        assert_eq!(number("video-width"), 760.);
        // Narrow: the policy floats it over the video; the video gets the width back.
        set("adjust-visible", false);
        set("adjust-overlay", true);
        set("adjust-popped", true);
        composed.settle();
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
        composed.settle();
        assert_eq!(number("adjust-pop-width"), 317.);
        assert_eq!(number("columns-popped-width"), 317. + 317.);
        let pop = crate::ui::window_state::adjust_overlay_width(1400., 317., 317.);
        assert_eq!(number("adjust-pop-width"), pop);
    }

    /// What the per-panel compile costs on the UI thread: the shell alone,
    /// the start-up panels (the real playback bar, visualizer backdrop and
    /// playlist sidebar) and each of the four built-in adjust panels
    /// (Picture, Audio, Subtitles, Playback) as it is first shown, then a
    /// switch back to each. Release build, headless: `cargo test --release
    /// -- --ignored per_panel_compile_cost --nocapture --test-threads 1`.
    #[test]
    #[ignore = "measurement"]
    #[allow(clippy::disallowed_macros)] // the numbers for the terminal
    fn per_panel_compile_cost() {
        headless();
        let plugins = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let spec = |owner, file: &str, component: &str, global: &str, slot| CompileSpec {
            owner,
            file: plugins.join(file),
            component: component.into(),
            global: Some(global.into()),
            slot,
        };
        let base = [
            spec(
                1,
                "playback/ui/bar.slint",
                "PlaybackBar",
                "PlaybackBridge",
                Slot::Bottom,
            ),
            spec(
                2,
                "visualizer/ui/backdrop.slint",
                "Backdrop",
                "Art",
                Slot::Backdrop,
            ),
            spec(
                3,
                "playlist/ui/sidebar.slint",
                "PlaylistSidebar",
                "PlaylistBridge",
                Slot::Sidebar,
            ),
        ];
        let adjust = [
            spec(
                10,
                "video/picture.slint",
                "Picture",
                "PictureBridge",
                Slot::Adjust,
            ),
            spec(
                11,
                "audio/ui/workbench.slint",
                "Audio",
                "AudioBridge",
                Slot::Adjust,
            ),
            spec(
                12,
                "subtitles/ui/panel.slint",
                "SubtitlesPanel",
                "SubtitlesBridge",
                Slot::Adjust,
            ),
            spec(
                13,
                "playback/ui/panel.slint",
                "PlaybackPanel",
                "PlaybackPanelBridge",
                Slot::Adjust,
            ),
        ];
        let ms = |started: std::time::Instant| started.elapsed().as_secs_f64() * 1000.;
        let started = std::time::Instant::now();
        let shell = shell_in(None).unwrap();
        shell
            .window()
            .set_size(slint::LogicalSize::new(1400., 800.));
        crate::ui::view::settle_layout(shell.window());
        println!("shell (compiled once a run): {:.0} ms", ms(started));
        let started = std::time::Instant::now();
        let mut composed = Composed::in_shell(shell, &base);
        println!("bar, backdrop, playlist: {:.0} ms", ms(started));
        for spec in &adjust {
            composed.panels.insert(spec.clone());
        }
        composed
            .shell
            .set_property("adjust-visible", Value::Bool(true))
            .unwrap();
        for spec in &adjust {
            let started = std::time::Instant::now();
            composed.show(Slot::Adjust, Some(spec.owner)).unwrap();
            println!("first show of {}: {:.0} ms", spec.component, ms(started));
        }
        for spec in &adjust {
            let started = std::time::Instant::now();
            composed.show(Slot::Adjust, Some(spec.owner)).unwrap();
            println!("switch back to {}: {:.2} ms", spec.component, ms(started));
        }
    }

    /// A new video picture must not touch the
    /// window's layout. When the picture's size fed it, every frame redid the
    /// layout and everything that depends on it (with a UI Automation client
    /// and Slint's former accessibility support, about 30 ms a frame with
    /// VestraTube's 200 rows on the Radeon 780M laptop).
    #[test]
    fn a_new_video_picture_leaves_the_window_layout_alone() {
        headless();
        let composed = Composed::new(&[spec(20, "sidebar.slint", "RowsSidebar", Slot::Sidebar)]);
        let instance = &composed.shell;
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
