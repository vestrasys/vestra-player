//! Theme colours set by the player (R-H6) and `plugin.appearance` (R-S2):
//! fixed palettes (`vestra`, the dark default, and `vestra-light`; both
//! Aurora, doc/product/ui-spec.md §2.1), delivered to every view instance
//! (shell with its panels, floating sidebar, tool windows, options window)
//! through the host-only `VestraPalette` global (`ui/palette.slint`). Plugin
//! Slint reads `Theme`, whose colours are `out` properties bound to the
//! palette. A palette also picks the std-widgets colour scheme (the fluent
//! controls: scroll bars, line edits, combo boxes, check boxes), so they are
//! light in a light palette although the host compiles them as fluent-dark.
#![forbid(unsafe_code)]
use crate::host::*;
use crate::ui::ui_state;
use anyhow::Result;
use serde_json::{Value, json};
use slint::ComponentHandle;
use slint_interpreter::Value as SValue;

/// The palettes, in menu order; the first is the default.
pub const THEMES: [&str; 2] = ["vestra", "vestra-light"];

/// The palette a theme id selects: an id the player does not have (one
/// retired in an earlier version, saved or sent by an older plugin) selects
/// the default, so neither a saved setting nor an older plugin fails.
pub fn known(theme: &str) -> &'static str {
    THEMES
        .iter()
        .find(|t| **t == theme)
        .copied()
        .unwrap_or(THEMES[0])
}

/// Colour tokens of `ui/palette.slint`; `vestra` is also its default.
const VESTRA: [(&str, u32); 22] = [
    ("bg-bar", 0x16141c),
    ("bg-video", 0x0a090e),
    ("bg-list", 0x1a1821),
    ("bg-panel", 0x201d28),
    ("line", 0x2b2734),
    ("hover-bg", 0x28232f),
    ("press-bg", 0x110f16),
    ("selected-bg", 0x2f2840),
    ("text", 0xf5f3fa),
    ("text-dim", 0x8e88a0),
    ("text-list", 0xbdb8cb),
    ("text-played", 0x6e6980),
    ("accent", 0xf72aa6),
    ("accent-press", 0xc41a83),
    ("accent-disabled", 0x5a1d48),
    ("icon-disabled", 0x3b3646),
    ("playing", 0x2bb6ff),
    ("button-bg", 0x2f2a3b),
    ("button-hover", 0x3a3448),
    ("button-press", 0x25202e),
    ("button-disabled", 0x1b1922),
    ("danger", 0xff5a5f),
];

/// Aurora Light: the same tokens on near-white; the video area stays dark.
const VESTRA_LIGHT: [(&str, u32); 22] = [
    ("bg-bar", 0xf7f6fa),
    ("bg-video", 0x0e0d12),
    ("bg-list", 0xffffff),
    ("bg-panel", 0xf1eff5),
    ("line", 0xdcd8e4),
    ("hover-bg", 0xe9e6f0),
    ("press-bg", 0xdcd8e6),
    ("selected-bg", 0xefe4fb),
    ("text", 0x1c1924),
    ("text-dim", 0x6f6a7f),
    ("text-list", 0x3b3748),
    ("text-played", 0x9a95a8),
    ("accent", 0xd4128a),
    ("accent-press", 0xa80d6c),
    ("accent-disabled", 0xecbcd9),
    ("icon-disabled", 0xc9c5d2),
    ("playing", 0x0773ba),
    ("button-bg", 0xe6e3ec),
    ("button-hover", 0xdcd8e4),
    ("button-press", 0xcfcadb),
    ("button-disabled", 0xefedf3),
    ("danger", 0xc42b2b),
];

/// One palette: its colours, the `VestraPalette` property holding its
/// gradient, whether the std-widgets draw light, and the context menu's drop
/// shadow (`MenuChrome.shadow`, RGBA; the same blur everywhere, lighter on a
/// light palette so the panel does not sit in a dark halo).
struct Table {
    colours: &'static [(&'static str, u32); 22],
    fill: &'static str,
    light: bool,
    menu_shadow: u32,
}

/// Each palette, in `THEMES` order.
const TABLES: [Table; THEMES.len()] = [
    Table {
        colours: &VESTRA,
        fill: "fill-vestra",
        light: false,
        menu_shadow: 0x0000_0080,
    },
    Table {
        colours: &VESTRA_LIGHT,
        fill: "fill-vestra-light",
        light: true,
        menu_shadow: 0x0000_0033,
    },
];

fn color(rgb: u32) -> SValue {
    SValue::Brush(slint::Brush::SolidColor(slint::Color::from_rgb_u8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    )))
}

/// Whether `theme` is a light palette (std-widgets scheme, title bars).
pub fn is_light(theme: &str) -> bool {
    let index = THEMES.iter().position(|t| *t == known(theme)).unwrap_or(0);
    TABLES[index].light
}

/// Set one view's palette: colours, the gradient (copied from the instance's
/// own constant, Slint's public API cannot build gradients) and the
/// std-widgets colour scheme.
pub fn apply(instance: &slint_interpreter::ComponentInstance, theme: &str) -> Result<()> {
    let index = THEMES.iter().position(|t| *t == known(theme)).unwrap_or(0);
    let table = &TABLES[index];
    for (name, rgb) in table.colours {
        instance.set_global_property(PALETTE, name, color(*rgb))?;
    }
    let fill = instance.get_global_property(PALETTE, table.fill)?;
    instance.set_global_property(PALETTE, "accent-fill", fill)?;
    let scheme = if table.light { "light" } else { "dark" };
    instance.set_global_property(
        PALETTE,
        "color-scheme",
        SValue::EnumerationValue("ColorScheme".into(), scheme.into()),
    )?;
    // Only windows that draw a MenuView (shell, floating sidebar) export it.
    if instance.definition().globals().any(|g| g == MENU_CHROME) {
        let [r, g, b, a] = table.menu_shadow.to_be_bytes();
        instance.set_global_property(
            MENU_CHROME,
            "shadow",
            SValue::Brush(slint::Brush::SolidColor(slint::Color::from_argb_u8(
                a, r, g, b,
            ))),
        )?;
    }
    Ok(())
}

pub const PALETTE: &str = "VestraPalette";
/// Host-only per-palette menu values (`ui/menu-view.slint`).
pub const MENU_CHROME: &str = "MenuChrome";

/// Set one view's per-system fonts (`Theme.font-mono`): every view, whatever
/// the palette, before it is shown where the caller can.
pub fn apply_fonts(instance: &slint_interpreter::ComponentInstance) -> Result<()> {
    instance.set_global_property(
        PALETTE,
        "font-mono",
        SValue::String(crate::platform::ui_monospace_font().into()),
    )?;
    Ok(())
}

/// The source line that makes the palette settable on an instance.
pub fn export_line() -> String {
    let path = crate::ui::view::ui_dir().join("palette.slint");
    let path = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("export {{ VestraPalette }} from \"{path}\";\n")
}

impl Host {
    pub(crate) fn theme(&self) -> &str {
        self.ui_state
            .state
            .appearance
            .as_ref()
            .map_or(THEMES[0], |a| a.theme.as_str())
    }
    /// Deliver the palette to one new instance (failures retried on the next tick).
    pub(crate) fn theme_view(&mut self, instance: &slint_interpreter::ComponentInstance) {
        if let Err(error) = apply_fonts(instance) {
            diag!("host.error theme: {error:#}");
            self.theme_pending = true;
        }
        if self.theme() == THEMES[0] {
            // A fresh instance already shows the default palette.
            return;
        }
        if let Err(error) = apply(instance, self.theme()) {
            diag!("host.error theme: {error:#}");
            self.theme_pending = true;
        }
    }
    /// Every view, after a change or a failed delivery.
    pub(crate) fn theme_all(&mut self) {
        let theme = self.theme().to_owned();
        crate::platform::set_dark_title_bars(!is_light(&theme));
        let mut instances: Vec<_> = self
            .shell
            .iter()
            .chain(self.floating.iter().map(|f| &f.instance))
            .chain(self.views.values().map(|v| &v.instance))
            .chain(self.settings.window())
            .map(ComponentHandle::clone_strong)
            .collect();
        self.theme_pending = false;
        for instance in instances.drain(..) {
            if let Err(error) = apply(&instance, &theme).and_then(|()| apply_fonts(&instance)) {
                diag!("host.error theme: {error:#}");
                self.theme_pending = true;
            }
        }
    }
    pub(crate) fn appearance_set(
        &mut self,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        // An id the player does not have selects the default (`known`):
        // plugins built for an older SDK may still send a retired palette.
        let theme = known(string(params, "theme")?);
        if theme != self.theme() {
            let language = self.language_preference();
            self.ui_state.state.appearance = Some(ui_state::Appearance {
                theme: theme.into(),
                language,
            });
            let now = self.now_ms();
            self.ui_state.touch(now);
            self.theme_all();
            let language = self.language_preference();
            let language = language.as_deref().unwrap_or(crate::i18n::SYSTEM);
            // `changed` (B2c, an additive field): what this event changed, so a
            // plugin woken by it can tell a palette change from a language
            // one (doc/architecture/i18n.md §2).
            self.broadcast(
                "appearance.changed",
                json!({"theme":theme,"language":language,"changed":["theme"]}),
            );
        }
        Ok(Value::Null)
    }
    /// The saved interface language, `None` = follow the system.
    pub(crate) fn language_preference(&self) -> Option<String> {
        self.ui_state
            .state
            .appearance
            .as_ref()
            .and_then(|a| a.language.clone())
    }
    /// `appearance.setLanguage`: saved for the next start (the language of
    /// this run never changes); every plugin hears `appearance.changed`.
    pub(crate) fn language_set(&mut self, params: &Value) -> std::result::Result<Value, RpcError> {
        let value = string(params, "language")?;
        if !crate::i18n::valid_preference(value) {
            return Err(RpcError::invalid(format!(
                "language is \"system\" or one of {}",
                crate::i18n::LOCALES.join(", ")
            )));
        }
        let language = (value != crate::i18n::SYSTEM).then(|| value.to_owned());
        if language != self.language_preference() {
            self.ui_state.state.appearance = Some(ui_state::Appearance {
                theme: self.theme().into(),
                language,
            });
            let now = self.now_ms();
            self.ui_state.touch(now);
            self.broadcast(
                "appearance.changed",
                json!({"theme":self.theme(),"language":value,"changed":["language"]}),
            );
        }
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vestra_table_is_the_theme_file_default_and_palettes_match() {
        let file = include_str!("../../ui/palette.slint");
        for (name, rgb) in VESTRA {
            let line = format!("in property <color> {name}: #{rgb:06x};");
            assert!(file.contains(&line), "palette.slint lacks {line}");
        }
        let colours = file.matches("in property <color>").count();
        assert_eq!(colours, VESTRA.len(), "every colour token is in the tables");
        assert!(file.contains("in property <brush> accent-fill: self.fill-vestra;"));
        for table in &TABLES {
            // Every palette sets the same tokens, and has its gradient.
            let names: Vec<_> = table.colours.iter().map(|(n, _)| *n).collect();
            let default: Vec<_> = VESTRA.iter().map(|(n, _)| *n).collect();
            assert_eq!(names, default);
            assert!(file.contains(&format!(
                "out property <brush> {}: @linear-gradient(",
                table.fill
            )));
        }
        // The menu shadow default is the `vestra` palette's.
        let menu = include_str!("../../ui/menu-view.slint");
        let line = format!(
            "in property <color> shadow: #{:08x};",
            TABLES[0].menu_shadow
        );
        assert!(menu.contains(&line), "menu-view.slint lacks {line}");
        assert!(TABLES[1].menu_shadow & 0xff < TABLES[0].menu_shadow & 0xff);
        let theme = include_str!("../../ui/vestra/theme.slint");
        assert!(
            !theme.contains("in property"),
            "plugins cannot assign Theme"
        );
    }

    #[test]
    fn an_id_the_player_does_not_have_selects_the_default() {
        assert_eq!(known("vestra"), "vestra");
        assert_eq!(known("vestra-light"), "vestra-light");
        // Retired in an earlier version, or never a palette: the default, no error.
        assert_eq!(known("classic"), THEMES[0]);
        assert_eq!(known("light"), THEMES[0]);
        assert_eq!(known(""), "vestra");
        assert!(!is_light("vestra") && is_light("vestra-light") && !is_light("classic"));
        assert_eq!(TABLES.len(), THEMES.len());
    }

    #[test]
    fn plugin_slint_cannot_assign_theme_colours() {
        let source = r#"
import { Theme } from "@vestra/theme.slint";
export component W inherits Window {
    init => { Theme.accent = #ff0000; }
    background: Theme.bg-panel;
}"#;
        let dir = tempfile::tempdir().unwrap();
        let result = pollster::block_on(
            crate::ui::view::compiler()
                .build_from_source(source.into(), dir.path().join("w.slint")),
        );
        assert!(
            result
                .diagnostics()
                .any(|d| d.level() == slint_interpreter::DiagnosticLevel::Error),
            "assigning Theme.accent must not compile: {:?}",
            result
                .diagnostics()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
        );
        // Reading and the export the player adds compile.
        let ok = format!(
            "{}{}",
            source.replace("init => { Theme.accent = #ff0000; }", ""),
            export_line()
        );
        let result = pollster::block_on(
            crate::ui::view::compiler().build_from_source(ok, dir.path().join("w.slint")),
        );
        let errors: Vec<_> = result
            .diagnostics()
            .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
            .map(|d| d.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            result
                .component("W")
                .unwrap()
                .global_properties(PALETTE)
                .is_some()
        );
    }
}
