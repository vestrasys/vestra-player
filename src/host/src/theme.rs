//! Theme colours set by the player (R-H6) and `plugin.appearance` (R-S2):
//! fixed palettes (one today, `vestra`; the table form leaves room for a
//! light palette), delivered to every view instance (shell with its
//! panels, floating sidebar, tool windows, options window) through the
//! host-only `VestraPalette` global (`ui/palette.slint`). Plugin Slint reads
//! `Theme`, whose colours are `out` properties bound to the palette.
#![forbid(unsafe_code)]
use super::*;

/// The palettes, in menu order; the first is the default.
pub const THEMES: [&str; 1] = ["vestra"];

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

/// Colour tokens of `ui/palette.slint`; Vestra is also its default.
const VESTRA: [(&str, u32); 22] = [
    ("bg-bar", 0x17141c),
    ("bg-video", 0x0b0a0f),
    ("bg-list", 0x1c1922),
    ("bg-panel", 0x221e29),
    ("line", 0x07060a),
    ("hover-bg", 0x2a2533),
    ("press-bg", 0x120f16),
    ("selected-bg", 0x2f2938),
    ("text", 0xf2eff7),
    ("text-dim", 0x8a8496),
    ("text-list", 0xb8b2c4),
    ("text-played", 0x6f6a7a),
    ("accent", 0xfa139c),
    ("accent-press", 0xc40f7a),
    ("accent-disabled", 0x5c1a42),
    ("icon-disabled", 0x3a3542),
    ("playing", 0xa24bff),
    ("button-bg", 0x322c3b),
    ("button-hover", 0x3d3648),
    ("button-press", 0x26212e),
    ("button-disabled", 0x1d1a23),
    ("danger", 0xff5a4a),
];

/// Each palette's colours, in `THEMES` order.
const TABLES: [&[(&str, u32); 22]; THEMES.len()] = [&VESTRA];

fn color(rgb: u32) -> SValue {
    SValue::Brush(slint::Brush::SolidColor(slint::Color::from_rgb_u8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    )))
}

/// Set one view's palette. `vestra_fill` is the default gradient brush, read
/// once from a fresh instance (Slint's public API cannot build gradients).
pub fn apply(
    instance: &slint_interpreter::ComponentInstance,
    theme: &str,
    vestra_fill: Option<&SValue>,
) -> Result<()> {
    let index = THEMES.iter().position(|t| *t == known(theme)).unwrap_or(0);
    let table = TABLES[index];
    // Vestra fills with its gradient; another palette with its accent.
    let fill = if index == 0 {
        vestra_fill.cloned()
    } else {
        table
            .iter()
            .find(|(n, _)| *n == "accent")
            .map(|(_, rgb)| color(*rgb))
    };
    for (name, rgb) in table {
        instance.set_global_property(PALETTE, name, color(*rgb))?;
    }
    if let Some(fill) = fill {
        instance.set_global_property(PALETTE, "accent-fill", fill)?;
    }
    Ok(())
}

pub const PALETTE: &str = "VestraPalette";

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
    let path = crate::ui::ui_dir().join("palette.slint");
    let path = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("export {{ VestraPalette }} from \"{path}\";\n")
}

impl Host {
    pub(super) fn theme(&self) -> &str {
        self.ui_state
            .state
            .appearance
            .as_ref()
            .map_or("vestra", |a| a.theme.as_str())
    }
    /// Deliver the palette to one new instance (failures retried on the next tick).
    pub(super) fn theme_view(&mut self, instance: &slint_interpreter::ComponentInstance) {
        if self.vestra_fill.is_none() {
            self.vestra_fill = instance.get_global_property(PALETTE, "accent-fill").ok();
        }
        if let Err(error) = apply_fonts(instance) {
            eprintln!("host.error theme: {error:#}");
            self.theme_pending = true;
        }
        if self.theme() == "vestra" && self.vestra_fill.is_some() {
            // A fresh instance already shows Vestra.
            return;
        }
        if let Err(error) = apply(instance, self.theme(), self.vestra_fill.as_ref()) {
            eprintln!("host.error theme: {error:#}");
            self.theme_pending = true;
        }
    }
    /// Every view, after a change or a failed delivery.
    pub(super) fn theme_all(&mut self) {
        let theme = self.theme().to_owned();
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
            if let Err(error) = apply(&instance, &theme, self.vestra_fill.as_ref())
                .and_then(|()| apply_fonts(&instance))
            {
                eprintln!("host.error theme: {error:#}");
                self.theme_pending = true;
            }
        }
    }
    pub(super) fn appearance_set(
        &mut self,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        // An id the player does not have selects the default (`known`):
        // plugins built for an older SDK may still send a retired palette.
        let theme = known(string(params, "theme")?);
        if theme != self.theme() {
            self.ui_state.state.appearance = Some(ui_state::Appearance {
                theme: theme.into(),
            });
            let now = self.now_ms();
            self.ui_state.touch(now);
            self.theme_all();
            self.broadcast("appearance.changed", json!({"theme":theme}));
        }
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vestra_table_is_the_theme_file_default_and_palettes_match() {
        let file = include_str!("../ui/palette.slint");
        for (name, rgb) in VESTRA {
            let line = format!("in property <color> {name}: #{rgb:06x};");
            assert!(file.contains(&line), "theme.slint lacks {line}");
        }
        let colours = file.matches("in property <color>").count();
        assert_eq!(colours, VESTRA.len(), "every colour token is in the tables");
        assert!(file.contains("in property <brush> accent-fill"));
        let theme = include_str!("../ui/vestra/theme.slint");
        assert!(
            !theme.contains("in property"),
            "plugins cannot assign Theme"
        );
    }

    #[test]
    fn an_id_the_player_does_not_have_selects_the_default() {
        assert_eq!(known("vestra"), "vestra");
        // Retired in an earlier version, or never a palette: the default, no error.
        assert_eq!(known("classic"), THEMES[0]);
        assert_eq!(known(""), "vestra");
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
            crate::ui::compiler().build_from_source(source.into(), dir.path().join("w.slint")),
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
            crate::ui::compiler().build_from_source(ok, dir.path().join("w.slint")),
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
