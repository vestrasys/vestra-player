//! Experimental finite subtitle presentation descriptors, not a final SDK API.
//! Validation and deterministic arbitration are not backend application or
//! visible-frame confirmation. The finite mpv option mapping below is a candidate
//! for actual backend/pixel validation; no offline ASS mapping is supplied.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Color {
    White,
    Yellow,
    Cyan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Style {
    pub font_id: String,
    pub font_size: u16,
    pub color: Color,
    pub outline: u8,
    pub margin_percent: u8,
    pub offset_ms: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    UnsupportedFont,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid subtitle style or layer metadata",
            Self::UnsupportedFont => "subtitle font is not in the available font table",
        })
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

/// Caller supplies a real available font ID; constructing this value does not
/// discover fonts or imply that the supplied ID is installed or distributable.
pub fn default_for(font_id: String) -> Style {
    Style {
        font_id,
        font_size: 36,
        color: Color::White,
        outline: 2,
        margin_percent: 5,
        offset_ms: 0,
    }
}

fn validate_fields(style: &Style) -> Result<()> {
    if style.font_id.is_empty()
        || style.font_id.len() > 128
        || !(16..=72).contains(&style.font_size)
        || style.outline > 4
        || !(2..=15).contains(&style.margin_percent)
        || !(-10_000..=10_000).contains(&style.offset_ms)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

pub fn validate(style: &Style, available_fonts: &BTreeSet<String>) -> Result<()> {
    validate_fields(style)?;
    if !available_fonts.contains(&style.font_id) {
        return Err(Error::UnsupportedFont);
    }
    Ok(())
}

/// Map the 1920x1080 reference descriptor to mpv 0.41's public properties.
/// Font/outline/margin units are scaled pixels at height 720. Eight-digit hex
/// colors are ARGB, not RGBA. These properties target the video-sized reference,
/// rather than the whole application window or its black borders.
///
/// Caller must first validate against the finite font table and resolve its ID
/// to this family. This function checks numeric/metadata bounds and family shape,
/// but cannot confirm actual selected font, backend support, or visible pixels.
/// Returned values are property arguments, never shell/filter expressions.
pub fn mpv_properties(style: &Style, resolved_family: &str) -> Result<Vec<(&'static str, String)>> {
    validate_fields(style)?;
    if resolved_family.is_empty()
        || resolved_family.len() > 256
        || resolved_family.trim() != resolved_family
        || resolved_family.chars().any(char::is_control)
    {
        return Err(Error::Invalid);
    }
    let color = match style.color {
        Color::White => "#FFFFFFFF",
        Color::Yellow => "#FFFFFF00",
        Color::Cyan => "#FF00FFFF",
    };
    Ok(vec![
        ("sub-ass-override", "force".into()),
        ("sub-scale-by-window", "yes".into()),
        ("sub-scale-with-window", "no".into()),
        ("sub-use-margins", "no".into()),
        ("sub-align-x", "center".into()),
        ("sub-align-y", "bottom".into()),
        ("sub-margin-x", "19".into()),
        ("sub-font", resolved_family.into()),
        (
            "sub-font-size",
            (f64::from(style.font_size) * 720.0 / 1080.0).to_string(),
        ),
        (
            "sub-border-size",
            (f64::from(style.outline) * 720.0 / 1080.0).to_string(),
        ),
        (
            "sub-margin-y",
            (f64::from(style.margin_percent) * 720.0 / 100.0).to_string(),
        ),
        (
            "sub-delay",
            (f64::from(style.offset_ms) / 1000.0).to_string(),
        ),
        ("sub-color", color.into()),
        ("sub-border-color", "#FF000000".into()),
        ("sub-shadow-offset", "0".into()),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub owner: u64,
    pub manifest_id: String,
    pub revision: u64,
    pub style: Style,
}

/// This candidate uses the lexically LAST manifest ID as the effective layer.
/// Input/activation order never affects it. One layer per owner and manifest ID;
/// caller validates each complete Style against its finite font table first.
/// This does not register resources, change selection, or apply backend state.
pub fn effective(layers: &[Layer]) -> Result<Option<&Layer>> {
    if layers.len() > 32 {
        return Err(Error::Invalid);
    }
    let mut owners = BTreeSet::new();
    let mut manifests = BTreeSet::new();
    let mut effective: Option<&Layer> = None;
    for layer in layers {
        if layer.manifest_id.is_empty()
            || layer.manifest_id.len() > 128
            || layer.revision == 0
            || layer.revision > (1_u64 << 53) - 1
            || !owners.insert(layer.owner)
            || !manifests.insert(layer.manifest_id.as_str())
        {
            return Err(Error::Invalid);
        }
        if effective.is_none_or(|current| layer.manifest_id > current.manifest_id) {
            effective = Some(layer);
        }
    }
    Ok(effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fonts() -> BTreeSet<String> {
        BTreeSet::from(["verified-font".into()])
    }
    fn style() -> Style {
        default_for("verified-font".into())
    }
    fn layer(owner: u64, manifest: &str) -> Layer {
        Layer {
            owner,
            manifest_id: manifest.into(),
            revision: 1,
            style: style(),
        }
    }

    #[test]
    fn all_numeric_endpoints_and_exact_font_membership_are_enforced() {
        let mut candidate = style();
        for (size, outline, margin, offset) in [(16, 0, 2, -10_000), (72, 4, 15, 10_000)] {
            candidate.font_size = size;
            candidate.outline = outline;
            candidate.margin_percent = margin;
            candidate.offset_ms = offset;
            assert_eq!(validate(&candidate, &fonts()), Ok(()));
        }
        for (size, outline, margin, offset) in [
            (15, 2, 5, 0),
            (73, 2, 5, 0),
            (36, 5, 5, 0),
            (36, 2, 1, 0),
            (36, 2, 16, 0),
            (36, 2, 5, -10_001),
            (36, 2, 5, 10_001),
        ] {
            candidate = style();
            candidate.font_size = size;
            candidate.outline = outline;
            candidate.margin_percent = margin;
            candidate.offset_ms = offset;
            assert_eq!(validate(&candidate, &fonts()), Err(Error::Invalid));
        }
        assert_eq!(
            validate(&default_for("not-installed".into()), &fonts()),
            Err(Error::UnsupportedFont)
        );
        assert_eq!(
            validate(&default_for(String::new()), &fonts()),
            Err(Error::Invalid)
        );
        assert_eq!(
            validate(&default_for("x".repeat(129)), &fonts()),
            Err(Error::Invalid)
        );
    }

    #[test]
    fn wire_descriptor_uses_camel_case_and_rejects_unknown_fields_or_color() {
        let encoded = serde_json::to_value(style()).unwrap();
        assert_eq!(
            encoded,
            json!({"fontId":"verified-font","fontSize":36,"color":"white","outline":2,"marginPercent":5,"offsetMs":0})
        );
        assert_eq!(
            serde_json::from_value::<Style>(encoded.clone()).unwrap(),
            style()
        );
        let mut unknown = encoded.clone();
        unknown["arbitraryFilter"] = json!("untrusted");
        assert!(serde_json::from_value::<Style>(unknown).is_err());
        let mut unknown = encoded;
        unknown["color"] = json!("custom");
        assert!(serde_json::from_value::<Style>(unknown).is_err());
        assert_eq!(
            serde_json::to_value(Color::Yellow).unwrap(),
            json!("yellow")
        );
        assert_eq!(serde_json::to_value(Color::Cyan).unwrap(), json!("cyan"));
    }

    #[test]
    fn effective_layer_is_stable_and_conflicting_owner_or_manifest_is_rejected() {
        let a = layer(1, "a.owner");
        let z = layer(2, "z.owner");
        let first = [a.clone(), z.clone()];
        let second = [z.clone(), a.clone()];
        assert_eq!(effective(&first).unwrap().unwrap().owner, 2);
        assert_eq!(effective(&second).unwrap().unwrap().owner, 2);
        assert!(effective(&[]).unwrap().is_none());
        assert_eq!(
            effective(&[a.clone(), layer(1, "other")]),
            Err(Error::Invalid)
        );
        assert_eq!(effective(&[a, layer(3, "a.owner")]), Err(Error::Invalid));
        let mut invalid = z;
        invalid.revision = 1_u64 << 53;
        assert_eq!(effective(&[invalid]), Err(Error::Invalid));
    }

    #[test]
    fn mpv_mapping_preserves_1080_reference_sizes_and_uses_video_scaling() {
        let mut style = style();
        style.offset_ms = -1250;
        let pairs = mpv_properties(&style, "Verified Family").unwrap();
        let properties: std::collections::BTreeMap<_, _> = pairs.iter().cloned().collect();
        assert_eq!(pairs.len(), properties.len());
        let number = |key: &str| properties[key].parse::<f64>().unwrap();
        assert!((number("sub-font-size") * 1080.0 / 720.0 - 36.0).abs() < 1e-10);
        assert!((number("sub-border-size") * 1080.0 / 720.0 - 2.0).abs() < 1e-10);
        assert_eq!(number("sub-margin-y") / 720.0, 0.05);
        assert_eq!(number("sub-delay"), -1.25);
        assert_eq!(properties["sub-font"], "Verified Family");
        assert_eq!(properties["sub-ass-override"], "force");
        assert_eq!(properties["sub-scale-by-window"], "yes");
        assert_eq!(properties["sub-scale-with-window"], "no");
        assert_eq!(properties["sub-use-margins"], "no");
        assert_eq!(properties["sub-align-x"], "center");
        assert_eq!(properties["sub-align-y"], "bottom");
        assert_eq!(number("sub-shadow-offset"), 0.0);
    }

    #[test]
    fn mpv_colors_are_opaque_argb_and_family_input_is_bounded() {
        for (color, expected_rgb) in [
            (Color::White, 0xffffff),
            (Color::Yellow, 0xffff00),
            (Color::Cyan, 0x00ffff),
        ] {
            let mut style = style();
            style.color = color;
            let properties: std::collections::BTreeMap<_, _> =
                mpv_properties(&style, "Verified Family")
                    .unwrap()
                    .into_iter()
                    .collect();
            let encoded = u32::from_str_radix(&properties["sub-color"][1..], 16).unwrap();
            assert_eq!(encoded >> 24, 255);
            assert_eq!(encoded & 0x00ffffff, expected_rgb);
            assert_eq!(properties["sub-border-color"], "#FF000000");
        }
        for family in [
            String::new(),
            "x".repeat(257),
            "name\0extra".into(),
            "name\nextra".into(),
        ] {
            assert_eq!(mpv_properties(&style(), &family), Err(Error::Invalid));
        }
        let encoded = serde_json::to_value(layer(7, "example")).unwrap();
        assert_eq!(encoded["manifestId"], json!("example"));
        assert_eq!(encoded["owner"], json!(7));
    }

    #[test]
    fn layer_limit_is_finite_and_structure_check_does_not_claim_font_validation() {
        let layers: Vec<_> = (0..32)
            .map(|owner| layer(owner, &format!("owner{owner:02}")))
            .collect();
        assert_eq!(effective(&layers).unwrap().unwrap().owner, 31);
        let mut over = layers;
        over.push(layer(32, "owner32"));
        assert_eq!(effective(&over), Err(Error::Invalid));
        let mut structurally_valid = layer(1, "a");
        structurally_valid.style.font_id = "missing".into();
        assert!(effective(&[structurally_valid.clone()]).is_ok());
        assert_eq!(
            validate(&structurally_valid.style, &fonts()),
            Err(Error::UnsupportedFont)
        );
    }
}
