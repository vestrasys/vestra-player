//! Interface state kept across launches (core-ui-design.md §8): window frame
//! and whether it was maximized, sidebar width, the sidebar slot (shown, floating, current tab, tab order,
//! floating frame), the user's binding overlay, and the plugins whose
//! commands a `commands.run` caller runs without asking (Ask Vestra UX step
//! 4, H4). One JSON file, written atomically and debounced.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const DEBOUNCE_MS: u64 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 100_000.)
            && self.width > 0.
            && self.height > 0.
    }
}

/// Before sidebar tabs: one sidebar panel's state, by manifest id. Read
/// only to fill [`SidebarSlot`] once, never written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelPrefs {
    pub visible: bool,
    #[serde(default)]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating: Option<Frame>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sidebar {
    pub width: f64,
}

/// At most this many plugin ids are kept in [`SidebarSlot::order`] (sidebar
/// tabs have no cap, sidebar-tabs.md; the oldest go first past it).
pub const ORDER_LIMIT: usize = 512;

/// The sidebar slot, whichever panels it holds (panel-slots.md step 1):
/// shown and floating as a whole, its current tab and the tab order (plugin
/// manifest ids, including plugins not running now), and the floating frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarSlot {
    pub visible: bool,
    #[serde(default)]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floating: Option<Frame>,
}

impl Default for SidebarSlot {
    fn default() -> Self {
        Self {
            visible: true,
            detached: false,
            active: None,
            order: Vec::new(),
            floating: None,
        }
    }
}

/// Remember `id` in a column's tab order (appended when new), evicting past
/// [`ORDER_LIMIT`] the oldest ids not in `current` (plugins with no tab now,
/// uninstalled ones among them) before any that has one.
fn remember_in(order: &mut Vec<String>, id: &str, current: &[&str]) {
    if order.iter().any(|known| known == id) {
        return;
    }
    order.push(id.into());
    while order.len() > ORDER_LIMIT {
        let gone = order
            .iter()
            .position(|known| !current.contains(&known.as_str()))
            .unwrap_or(0);
        order.remove(gone);
    }
}

/// The default and least width of the adjust column (menu redesign §3.3).
pub const ADJUST_WIDTH: f64 = 317.0;

/// The adjust column (menu redesign step 3b), whichever panels it holds:
/// shown or not, its width, its current tab and its tab order (plugin
/// manifest ids, including plugins not running now). The same shape as
/// [`SidebarSlot`], without floating (the adjust column does not detach);
/// closed until the user opens it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustSlot {
    #[serde(default)]
    pub visible: bool,
    #[serde(default = "adjust_width")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(default)]
    pub order: Vec<String>,
}

fn adjust_width() -> f64 {
    ADJUST_WIDTH
}

impl Default for AdjustSlot {
    fn default() -> Self {
        Self {
            visible: false,
            width: ADJUST_WIDTH,
            active: None,
            order: Vec::new(),
        }
    }
}

impl AdjustSlot {
    /// [`SidebarSlot::remember_among`] for the adjust column's order.
    pub fn remember_among(&mut self, id: &str, current: &[&str]) {
        remember_in(&mut self.order, id, current);
    }
}

impl SidebarSlot {
    /// Remember `id` in the tab order (appended when new).
    pub fn remember(&mut self, id: &str) {
        if !self.order.iter().any(|known| known == id) {
            self.order.push(id.into());
            let excess = self.order.len().saturating_sub(ORDER_LIMIT);
            self.order.drain(..excess);
        }
    }

    /// [`Self::remember`], evicting past [`ORDER_LIMIT`] the oldest ids not
    /// in `current` (plugins with no tab now, uninstalled ones among them)
    /// before any that has one.
    pub fn remember_among(&mut self, id: &str, current: &[&str]) {
        remember_in(&mut self.order, id, current);
    }

    /// Where `id` sorts among the tabs: its place in the order, new ones last.
    #[cfg(test)]
    pub fn position(&self, id: &str) -> usize {
        self.order
            .iter()
            .position(|known| known == id)
            .unwrap_or(usize::MAX)
    }

    /// The state of the per-panel entries older players wrote: the playlist's
    /// (the panel that had the sidebar), else the first one's; every id joins
    /// the order, the playlist first.
    fn migrate(panels: &BTreeMap<String, PanelPrefs>) -> Option<Self> {
        const PLAYLIST: &str = "vestra.playlist";
        let (id, prefs) = panels
            .get_key_value(PLAYLIST)
            .or_else(|| panels.iter().next())?;
        let mut slot = Self {
            visible: prefs.visible,
            detached: prefs.detached,
            active: Some(id.clone()),
            order: Vec::new(),
            floating: prefs.floating,
        };
        slot.remember(id);
        for other in panels.keys() {
            slot.remember(other);
        }
        Some(slot)
    }
}

/// `plugin.appearance` (R-S2): the palette, kept when the choosing plugin
/// stops; and the interface language (`"system"` or a locale of
/// `i18n::LOCALES`, absent = system), read once at start-up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// The options window (R-H7): the page it showed last, `<manifest id>/<page id>`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_page: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    /// The main window's normal frame (logical px): never a maximized,
    /// minimized or fullscreen one (IA-CORE-03).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Frame>,
    /// The main window was maximized when last seen (under fullscreen: before
    /// it); it opens maximized again over [`Self::window`] (IA-CORE-03).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub window_maximized: bool,
    /// Windows: the normal frame in desktop pixels (the window placement),
    /// restored in preference to [`Self::window`]: logical pixels convert
    /// with the scale of whichever display the window is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_physical: Option<crate::platform::DesktopRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<Sidebar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_slot: Option<SidebarSlot>,
    /// The adjust column (menu redesign step 3b): `adjustSlot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjust_slot: Option<AdjustSlot>,
    /// `"<manifest id>/<command id>"` → bindings; read only, kept on save.
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
    /// One-time migrations of `bindings` already applied (host 0.6.0:
    /// `actions::FULLSCREEN_ESCAPE_MIGRATION`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings_migrated: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
    /// The command palette's recently run commands, newest first
    /// (`<manifest id>/<command id>`, at most `command_palette::RECENT_LIMIT`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette_recent: Vec<String>,
    /// "Don't ask again" on the approval sheet (Ask Vestra UX step 4, H4):
    /// by `commands.run` caller (its manifest id), the plugins (manifest
    /// ids, none built in) whose ordinary commands it runs without asking;
    /// destructive ones still ask. Cleared by the palette command Ask Again
    /// Before Plugin Commands; a plugin's entries go when it is uninstalled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trusted_plugins: BTreeMap<String, Vec<String>>,
}

/// The interface's part of [`UiState`] (plan step S2c(7) of
/// doc/process/core-thread-plan.md; design K8): what the interface writes
/// and reads itself. The core writes the file and owns the rest (the
/// binding overlay and its migrations, the appearance, the trusted
/// plugins); the interface keeps this part and gives it to the core whole
/// whenever it changes ([`UiState::set_interface`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interface {
    pub window: Option<Frame>,
    pub window_maximized: bool,
    pub window_physical: Option<crate::platform::DesktopRect>,
    pub sidebar: Option<Sidebar>,
    pub sidebar_slot: Option<SidebarSlot>,
    pub adjust_slot: Option<AdjustSlot>,
    pub settings: Option<Settings>,
    pub palette_recent: Vec<String>,
}

impl UiState {
    /// The interface's part, as the file holds it now.
    pub fn interface(&self) -> Interface {
        Interface {
            window: self.window,
            window_maximized: self.window_maximized,
            window_physical: self.window_physical,
            sidebar: self.sidebar.clone(),
            sidebar_slot: self.sidebar_slot.clone(),
            adjust_slot: self.adjust_slot.clone(),
            settings: self.settings.clone(),
            palette_recent: self.palette_recent.clone(),
        }
    }
    /// The interface's part replaced whole; the core's part stays.
    pub fn set_interface(&mut self, part: Interface) {
        let Interface {
            window,
            window_maximized,
            window_physical,
            sidebar,
            sidebar_slot,
            adjust_slot,
            settings,
            palette_recent,
        } = part;
        self.window = window;
        self.window_maximized = window_maximized;
        self.window_physical = window_physical;
        self.sidebar = sidebar;
        self.sidebar_slot = sidebar_slot;
        self.adjust_slot = adjust_slot;
        self.settings = settings;
        self.palette_recent = palette_recent;
    }
}

/// At most this many plugins are trusted per caller (the oldest go first).
pub const TRUST_LIMIT: usize = 256;

impl UiState {
    /// Whether `caller` runs the ordinary commands of `plugin` without
    /// asking (manifest ids). (`commands_run` reads the caller's list once
    /// per call instead.)
    #[cfg(test)]
    pub fn trusts(&self, caller: &str, plugin: &str) -> bool {
        self.trusted_plugins
            .get(caller)
            .is_some_and(|list| list.iter().any(|id| id == plugin))
    }
    /// `caller` runs the ordinary commands of `plugins` without asking from
    /// now on; whether anything changed.
    pub fn trust(&mut self, caller: &str, plugins: &[String]) -> bool {
        let list = self.trusted_plugins.entry(caller.to_owned()).or_default();
        let before = list.len();
        for plugin in plugins {
            if !plugin.is_empty() && !list.contains(plugin) {
                list.push(plugin.clone());
            }
        }
        let added = list.len() != before;
        let excess = list.len().saturating_sub(TRUST_LIMIT);
        list.drain(..excess);
        if list.is_empty() {
            self.trusted_plugins.remove(caller);
        }
        added
    }
    /// `plugin` was uninstalled: no caller trusts it any more, and what it
    /// trusted as a caller goes; whether anything changed.
    pub fn forget_trust(&mut self, plugin: &str) -> bool {
        let mut changed = self.trusted_plugins.remove(plugin).is_some();
        self.trusted_plugins.retain(|_, list| {
            let before = list.len();
            list.retain(|id| id != plugin);
            changed |= list.len() != before;
            !list.is_empty()
        });
        changed
    }
    /// Ask Again Before Plugin Commands: every caller asks again; whether
    /// anything was trusted.
    pub fn clear_trust(&mut self) -> bool {
        let had = !self.trusted_plugins.is_empty();
        self.trusted_plugins.clear();
        had
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(flatten)]
    state: serde_json::Value,
}

impl UiState {
    /// Parse leniently: a field that does not fit is dropped, not the whole file.
    fn from_value(value: serde_json::Value) -> Self {
        let field = |name: &str| value.get(name).cloned().unwrap_or(serde_json::Value::Null);
        let window = serde_json::from_value::<Frame>(field("window"))
            .ok()
            .filter(Frame::valid);
        let window_maximized = field("windowMaximized").as_bool().unwrap_or(false);
        let window_physical =
            serde_json::from_value::<crate::platform::DesktopRect>(field("windowPhysical"))
                .ok()
                .filter(|r| {
                    r.width > 0
                        && r.height > 0
                        && r.width < 100_000
                        && r.height < 100_000
                        && r.x.abs() < 100_000
                        && r.y.abs() < 100_000
                });
        let sidebar = serde_json::from_value::<Sidebar>(field("sidebar"))
            .ok()
            .filter(|s| s.width.is_finite() && s.width > 0.);
        let panels: BTreeMap<String, PanelPrefs> = field("panels")
            .as_object()
            .map(|panels| {
                panels
                    .iter()
                    .filter_map(|(id, prefs)| {
                        let mut prefs = serde_json::from_value::<PanelPrefs>(prefs.clone()).ok()?;
                        prefs.floating = prefs.floating.filter(Frame::valid);
                        Some((id.clone(), prefs))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let bindings = field("bindings")
            .as_object()
            .map(|bindings| {
                bindings
                    .iter()
                    .filter_map(|(key, list)| {
                        Some((key.clone(), serde_json::from_value(list.clone()).ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let bindings_migrated =
            serde_json::from_value(field("bindingsMigrated")).unwrap_or_default();
        // A palette this player does not have (one retired in an earlier
        // version) becomes the default, silently.
        let appearance = serde_json::from_value::<Appearance>(field("appearance"))
            .ok()
            .map(|a| Appearance {
                theme: crate::ui::theme::known(&a.theme).into(),
                // A language this player does not have follows the system.
                language: a
                    .language
                    .filter(|l| crate::i18n::valid_preference(l) && l != crate::i18n::SYSTEM),
            });
        let settings = serde_json::from_value::<Settings>(field("settings")).ok();
        let mut palette_recent: Vec<String> =
            serde_json::from_value(field("paletteRecent")).unwrap_or_default();
        palette_recent.truncate(crate::ui::command_palette::RECENT_LIMIT);
        // Lists of non-empty ids, each kept once and bounded.
        let trusted_plugins = field("trustedPlugins")
            .as_object()
            .map(|callers| {
                callers
                    .iter()
                    .filter_map(|(caller, list)| {
                        let mut kept: Vec<String> = Vec::new();
                        for id in list.as_array()?.iter().filter_map(|id| id.as_str()) {
                            if !id.is_empty() && !kept.iter().any(|known| known == id) {
                                kept.push(id.to_owned());
                            }
                        }
                        let excess = kept.len().saturating_sub(TRUST_LIMIT);
                        kept.drain(..excess);
                        (!caller.is_empty() && !kept.is_empty()).then(|| (caller.clone(), kept))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The per-panel entries of older players become the slot's state;
        // they are not written again.
        let sidebar_slot = serde_json::from_value::<SidebarSlot>(field("sidebarSlot"))
            .ok()
            .map(|mut slot| {
                slot.floating = slot.floating.filter(Frame::valid);
                slot.order.truncate(ORDER_LIMIT);
                slot
            })
            .or_else(|| SidebarSlot::migrate(&panels));
        let adjust_slot = serde_json::from_value::<AdjustSlot>(field("adjustSlot"))
            .ok()
            .map(|mut slot| {
                if !slot.width.is_finite() || slot.width <= 0. || slot.width > 100_000. {
                    slot.width = ADJUST_WIDTH;
                }
                slot.order.truncate(ORDER_LIMIT);
                slot
            });
        Self {
            window,
            window_maximized,
            window_physical,
            sidebar,
            sidebar_slot,
            adjust_slot,
            bindings,
            bindings_migrated,
            appearance,
            settings,
            palette_recent,
            trusted_plugins,
        }
    }
}

/// The file plus pending-write bookkeeping. `None` path = never read or write.
pub struct Store {
    path: Option<PathBuf>,
    pub state: UiState,
    saved: UiState,
    due_ms: Option<u64>,
}

pub fn default_path() -> PathBuf {
    std::env::var_os("VESTRA_UI_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::platform::app_data_dir()
                .unwrap_or_default()
                .join("ui-state.json")
        })
}

impl Store {
    pub fn disabled() -> Self {
        Self {
            path: None,
            state: UiState::default(),
            saved: UiState::default(),
            due_ms: None,
        }
    }

    /// Unreadable content is moved aside to `<name>.bad` and defaults are used.
    pub fn open(path: &Path) -> Self {
        let state = match std::fs::read(path) {
            Err(_) => UiState::default(),
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(file) if file.version == VERSION => UiState::from_value(file.state),
                other => {
                    let reason = match other {
                        Ok(file) => format!("unknown version {}", file.version),
                        Err(error) => error.to_string(),
                    };
                    let bad = path.with_extension("json.bad");
                    diag!(
                        "ui state {}: {reason}; moved to {}",
                        path.display(),
                        bad.display()
                    );
                    let _ = std::fs::rename(path, bad);
                    UiState::default()
                }
            },
        };
        Self {
            path: Some(path.to_owned()),
            saved: state.clone(),
            state,
            due_ms: None,
        }
    }

    /// Call after changing `state`; the write happens one second later.
    pub fn touch(&mut self, now_ms: u64) {
        if self.path.is_some() && self.state != self.saved && self.due_ms.is_none() {
            self.due_ms = Some(now_ms + DEBOUNCE_MS);
        }
    }

    pub fn save_if_due(&mut self, now_ms: u64) {
        if self.due_ms.is_some_and(|due| now_ms >= due) {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        self.due_ms = None;
        let Some(path) = &self.path else { return };
        if self.state == self.saved {
            return;
        }
        let write = || -> anyhow::Result<()> {
            let mut value = serde_json::to_value(&self.state)?;
            value["version"] = VERSION.into();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("json.tmp");
            std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
            std::fs::rename(&temporary, path)?;
            Ok(())
        };
        match write() {
            Ok(()) => self.saved = self.state.clone(),
            Err(error) => diag!("ui state {}: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_every_field_and_debounces_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Vestra/ui-state.json");
        let mut store = Store::open(&path);
        assert_eq!(store.state, UiState::default());
        store.state.window = Some(Frame {
            x: 10.,
            y: 20.,
            width: 780.,
            height: 671.,
        });
        store.state.window_maximized = true;
        store.state.window_physical = Some(crate::platform::DesktopRect {
            x: -1920,
            y: 40,
            width: 1365,
            height: 1174,
        });
        store.state.sidebar = Some(Sidebar { width: 400. });
        store.state.sidebar_slot = Some(SidebarSlot {
            visible: false,
            detached: true,
            active: Some("examples.panel-menu".into()),
            order: vec!["vestra.playlist".into(), "examples.panel-menu".into()],
            floating: Some(Frame {
                x: 438.,
                y: 100.,
                width: 300.,
                height: 408.,
            }),
        });
        store
            .state
            .bindings
            .insert("vestra.playback/toggle-pause".into(), vec!["Space".into()]);
        store.state.bindings_migrated = vec!["fullscreen-escape-0.6.0".into()];
        // A palette other than the default survives a restart.
        store.state.appearance = Some(Appearance {
            theme: "vestra-light".into(),
            language: Some("ja".into()),
        });
        store.state.settings = Some(Settings {
            last_page: Some("vestra.preferences/appearance".into()),
        });
        store.state.palette_recent = vec!["host/mute".into(), "vestra.video/video.reset".into()];
        // Ask Vestra UX step 4: the trusted plugins survive a restart.
        store
            .state
            .trust("vestra.workshop", &["zysilm.clipexport".into()]);
        // Menu redesign step 3b: the adjust column's own state.
        store.state.adjust_slot = Some(AdjustSlot {
            visible: true,
            width: 340.,
            active: Some("vestra.audio".into()),
            order: vec!["examples.adjust-panel".into()],
        });
        store.touch(0);
        store.save_if_due(999);
        assert!(!path.exists(), "debounced");
        store.save_if_due(1000);
        let reopened = Store::open(&path);
        assert_eq!(reopened.state, store.state);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"adjustSlot\""), "{text}");
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("\"bindingsMigrated\""), "{text}");
        assert!(text.contains("\"windowMaximized\": true"), "{text}");
        assert!(text.contains("\"paletteRecent\""), "{text}");
        assert!(text.contains("\"trustedPlugins\""), "{text}");
    }

    /// H4: trust is per caller, kept once, bounded; an uninstalled plugin's
    /// entries go (as trusted and as a caller); the reset clears all.
    #[test]
    fn trusted_plugins_are_per_caller_and_go_with_an_uninstall_or_the_reset() {
        let mut state = UiState::default();
        assert!(!state.trusts("vestra.workshop", "zysilm.clipexport"));
        assert!(state.trust(
            "vestra.workshop",
            &["zysilm.clipexport".into(), "zysilm.autoframe".into()]
        ));
        assert!(!state.trust("vestra.workshop", &["zysilm.clipexport".into()]));
        assert!(!state.trust("vestra.other", &[]));
        assert!(!state.trusted_plugins.contains_key("vestra.other"));
        assert!(state.trusts("vestra.workshop", "zysilm.clipexport"));
        // Per caller: another caller still asks.
        assert!(!state.trusts("vestra.other", "zysilm.clipexport"));
        state.trust("zysilm.autoframe", &["zysilm.clipexport".into()]);
        assert!(state.forget_trust("zysilm.autoframe"));
        assert_eq!(
            state.trusted_plugins,
            BTreeMap::from([(
                "vestra.workshop".to_owned(),
                vec!["zysilm.clipexport".to_owned()]
            )])
        );
        assert!(!state.forget_trust("zysilm.autoframe"));
        assert!(state.forget_trust("zysilm.clipexport"));
        assert!(state.trusted_plugins.is_empty());
        state.trust("vestra.workshop", &["a.b".into()]);
        assert!(state.clear_trust());
        assert!(!state.clear_trust());
        // Bounded: the oldest go first.
        let many: Vec<String> = (0..TRUST_LIMIT + 3).map(|n| format!("p.{n}")).collect();
        state.trust("vestra.workshop", &many);
        let list = &state.trusted_plugins["vestra.workshop"];
        assert_eq!(list.len(), TRUST_LIMIT);
        assert_eq!(list[0], "p.3");
    }

    #[test]
    fn trusted_plugins_read_leniently() {
        let read =
            |text: &str| UiState::from_value(serde_json::from_str(text).unwrap()).trusted_plugins;
        assert_eq!(
            read(
                r#"{"trustedPlugins":{"a.caller":["x.y",7,"","x.y","z.w"],"b.caller":"x.y","c.caller":[],"":["x.y"]}}"#
            ),
            BTreeMap::from([(
                "a.caller".to_owned(),
                vec!["x.y".to_owned(), "z.w".to_owned()]
            )])
        );
        assert!(read(r#"{"trustedPlugins":[1]}"#).is_empty());
        assert!(read("{}").is_empty());
    }

    /// The adjust column starts closed at the default width; a bad width
    /// falls back to it, missing fields take their defaults.
    #[test]
    fn the_adjust_slot_reads_leniently_and_starts_closed() {
        let default = AdjustSlot::default();
        assert!(!default.visible && default.width == ADJUST_WIDTH);
        let read =
            |text: &str| UiState::from_value(serde_json::from_str(text).unwrap()).adjust_slot;
        assert_eq!(
            read(r#"{"adjustSlot":{"visible":true}}"#),
            Some(AdjustSlot {
                visible: true,
                ..AdjustSlot::default()
            })
        );
        assert_eq!(
            read(r#"{"adjustSlot":{"width":-4,"active":"a.b"}}"#),
            Some(AdjustSlot {
                active: Some("a.b".into()),
                ..AdjustSlot::default()
            })
        );
        assert_eq!(read(r#"{"adjustSlot":7}"#), None);
        assert_eq!(read("{}"), None);
    }

    #[test]
    fn a_saved_palette_the_player_no_longer_has_becomes_vestra() {
        // An earlier version offered a second palette; its id (any id not in
        // `theme::THEMES`) loads as the default and is saved as such.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"appearance":{"theme":"classic"},"sidebar":{"width":350}}"#,
        )
        .unwrap();
        let mut store = Store::open(&path);
        assert_eq!(store.state.appearance.as_ref().unwrap().theme, "vestra");
        assert_eq!(store.state.sidebar, Some(Sidebar { width: 350. }));
        // The next write (after any other change) stores the default.
        store.state.sidebar = Some(Sidebar { width: 360. });
        store.touch(0);
        store.save_if_due(1000);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""theme": "vestra""#), "{text}");
        assert!(!text.contains("classic"));
    }

    #[test]
    fn corrupt_files_move_aside_and_bad_fields_are_dropped_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Store::open(&path).state, UiState::default());
        assert!(dir.path().join("ui-state.json.bad").exists());
        std::fs::write(
            &path,
            r#"{"version":1,"appearance":{"theme":"pink"},"window":{"x":"left"},"windowMaximized":"yes","windowPhysical":{"x":0,"y":0,"width":0,"height":5},"sidebar":{"width":350},"panels":{"a":{"visible":true},"b":7},"bindings":{"x/y":["F6"],"z":3}}"#,
        )
        .unwrap();
        let state = Store::open(&path).state;
        assert_eq!(state.window, None);
        assert!(!state.window_maximized);
        assert_eq!(state.window_physical, None, "an empty frame");
        assert_eq!(state.appearance.unwrap().theme, "vestra");
        assert_eq!(state.sidebar, Some(Sidebar { width: 350. }));
        // The one readable panel entry ("a") became the slot's state.
        assert_eq!(state.sidebar_slot.unwrap().order, ["a"]);
        assert_eq!(state.bindings.len(), 1);
        std::fs::write(&path, r#"{"version":9}"#).unwrap();
        assert_eq!(Store::open(&path).state, UiState::default());
        let mut disabled = Store::disabled();
        disabled.state.sidebar = Some(Sidebar { width: 1. });
        disabled.touch(0);
        disabled.flush();
    }

    #[test]
    fn per_panel_entries_of_older_players_become_the_sidebar_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"panels":{
                "a.list":{"visible":true},
                "vestra.playlist":{"visible":false,"detached":true,"floating":{"x":5,"y":6,"width":300,"height":400}}}}"#,
        )
        .unwrap();
        let mut store = Store::open(&path);
        let slot = store.state.sidebar_slot.clone().unwrap();
        // The playlist had the sidebar: its state is the slot's.
        assert_eq!(
            slot,
            SidebarSlot {
                visible: false,
                detached: true,
                active: Some("vestra.playlist".into()),
                order: vec!["vestra.playlist".into(), "a.list".into()],
                floating: Some(Frame {
                    x: 5.,
                    y: 6.,
                    width: 300.,
                    height: 400.
                }),
            }
        );
        // Written back as `sidebarSlot`, without the old entries.
        store.state.sidebar = Some(Sidebar { width: 320. });
        store.touch(0);
        store.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("sidebarSlot") && !text.contains("\"panels\""),
            "{text}"
        );
        assert_eq!(Store::open(&path).state.sidebar_slot, Some(slot));
        // Without the playlist, the first entry; nothing to migrate: none.
        let other = BTreeMap::from([("b.x".to_string(), PanelPrefs::default())]);
        assert_eq!(
            SidebarSlot::migrate(&other).unwrap().active.as_deref(),
            Some("b.x")
        );
        assert_eq!(SidebarSlot::migrate(&BTreeMap::new()), None);
    }

    #[test]
    fn a_full_tab_order_evicts_ids_without_a_tab_first() {
        let mut slot = SidebarSlot::default();
        for n in 0..ORDER_LIMIT {
            slot.remember(&format!("p{n}"));
        }
        // p0 and p1 have tabs now; p2 is the oldest without one.
        slot.remember_among("new", &["p0", "p1", "new"]);
        assert_eq!(slot.order.len(), ORDER_LIMIT);
        assert_eq!(slot.order[..2], ["p0", "p1"]);
        assert_eq!(slot.position("p2"), usize::MAX);
        assert_eq!(slot.order.last().unwrap(), "new");
    }

    #[test]
    fn the_tab_order_remembers_new_ids_last_and_stays_bounded() {
        let mut slot = SidebarSlot::default();
        assert!(slot.visible && !slot.detached);
        slot.remember("vestra.playlist");
        slot.remember("examples.panel-menu");
        slot.remember("vestra.playlist");
        assert_eq!(slot.order, ["vestra.playlist", "examples.panel-menu"]);
        assert_eq!(slot.position("examples.panel-menu"), 1);
        assert_eq!(slot.position("new.one"), usize::MAX);
        for n in 0..ORDER_LIMIT + 10 {
            slot.remember(&format!("p{n}"));
        }
        assert_eq!(slot.order.len(), ORDER_LIMIT);
        assert_eq!(slot.order.last().unwrap(), &format!("p{}", ORDER_LIMIT + 9));
    }
}
