//! Interface state kept across launches (core-ui-design.md §8): window frame,
//! sidebar width, per-plugin sidebar preferences and floating frame, and the
//! user's binding overlay. One JSON file, written atomically and debounced.
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

/// `plugin.appearance` (R-S2): the palette, kept when the choosing plugin stops.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub theme: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<Sidebar>,
    #[serde(default)]
    pub panels: BTreeMap<String, PanelPrefs>,
    /// "<manifest id>/<command id>" → bindings; read only this round, kept on save.
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
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
        let sidebar = serde_json::from_value::<Sidebar>(field("sidebar"))
            .ok()
            .filter(|s| s.width.is_finite() && s.width > 0.);
        let panels = field("panels")
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
        let appearance = serde_json::from_value::<Appearance>(field("appearance"))
            .ok()
            .filter(|a| ["vestra", "potplayer"].contains(&a.theme.as_str()));
        let settings = serde_json::from_value::<Settings>(field("settings")).ok();
        Self {
            window,
            sidebar,
            panels,
            bindings,
            appearance,
            settings,
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
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/Application Support/Vestra/ui-state.json")
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
                    eprintln!(
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
            Err(error) => eprintln!("ui state {}: {error}", path.display()),
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
        store.state.sidebar = Some(Sidebar { width: 400. });
        store.state.panels.insert(
            "vestra.playlist".into(),
            PanelPrefs {
                visible: false,
                detached: true,
                floating: Some(Frame {
                    x: 438.,
                    y: 100.,
                    width: 300.,
                    height: 408.,
                }),
            },
        );
        store
            .state
            .bindings
            .insert("vestra.playback/toggle-pause".into(), vec!["Space".into()]);
        store.state.appearance = Some(Appearance {
            theme: "potplayer".into(),
        });
        store.state.settings = Some(Settings {
            last_page: Some("vestra.preferences/appearance".into()),
        });
        store.touch(0);
        store.save_if_due(999);
        assert!(!path.exists(), "debounced");
        store.save_if_due(1000);
        let reopened = Store::open(&path);
        assert_eq!(reopened.state, store.state);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
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
            r#"{"version":1,"appearance":{"theme":"pink"},"window":{"x":"left"},"sidebar":{"width":350},"panels":{"a":{"visible":true},"b":7},"bindings":{"x/y":["F6"],"z":3}}"#,
        )
        .unwrap();
        let state = Store::open(&path).state;
        assert_eq!(state.window, None);
        assert_eq!(state.appearance, None);
        assert_eq!(state.sidebar, Some(Sidebar { width: 350. }));
        assert_eq!(state.panels.len(), 1);
        assert_eq!(state.bindings.len(), 1);
        std::fs::write(&path, r#"{"version":9}"#).unwrap();
        assert_eq!(Store::open(&path).state, UiState::default());
        let mut disabled = Store::disabled();
        disabled.state.sidebar = Some(Sidebar { width: 1. });
        disabled.touch(0);
        disabled.flush();
    }
}
