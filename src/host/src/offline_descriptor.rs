//! Experimental finite offline video mapper; final public API may differ.
//! A graph is not font selection, backend acceptance, or pixel parity evidence.
//! Caller retains immutable source/subtitle/font leases and runs source 0 from
//! its original timeline with FFmpeg default autorotate. Do not input-seek away
//! the timeline before this graph. This module does not parse subtitle text.
#![forbid(unsafe_code)]

use std::path::{Component, Path};

use anyhow::{Result, ensure};

use crate::{
    effects::{self, Descriptors, Resource, VideoNode},
    subtitle_style::{self, Color, Style},
};

pub struct Subtitle<'a> {
    /// Caller-owned derived carrier, never an arbitrary user subtitle path.
    pub carrier: &'a Path,
    /// Caller-owned directory containing the frozen font asset.
    pub fonts_dir: &'a Path,
    pub family: &'a str,
    pub style: &'a Style,
    /// Actual post-autorotate, square-pixel display dimensions.
    pub display_width: u32,
    pub display_height: u32,
}

const SQUARE_PIXELS: &str = "scale=w='max(1,round(iw*sar))':h=ih,setsar=1";

// Deliberately reject filter metacharacters rather than implement an arbitrary
// path-expression escaping language. Ownership/existence remain caller checks.
fn owned_path(path: &Path) -> Result<&str> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 carrier/font path"))?;
    ensure!(
        text.starts_with('/')
            && text.len() <= 4096
            && text
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_.-".contains(&c))
            && path
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
        "carrier/font paths must be absolute ASCII paths using only letters, digits, /_.- without parent traversal"
    );
    Ok(text)
}

fn ass_color(color: Color) -> &'static str {
    // ASS is inverted-alpha BBGGRR, unlike mpv's opaque ARGB strings.
    match color {
        Color::White => "&H00FFFFFF",
        Color::Yellow => "&H0000FFFF",
        Color::Cyan => "&H00FFFF00",
    }
}

fn subtitle_filter(subtitle: &Subtitle<'_>) -> Result<String> {
    let carrier = owned_path(subtitle.carrier)?;
    let fonts = owned_path(subtitle.fonts_dir)?;
    let family = subtitle.family;
    ensure!(
        !family.is_empty()
            && family.len() <= 128
            && family.trim() == family
            && family
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b" _.-".contains(&c)),
        "font family must be bounded ASCII letters, digits, spaces or _.-"
    );
    // Reuse the canonical typed-field validation. Caller must ALSO validate
    // font_id against its real available-font table and resolve this family;
    // constructing property arguments does not prove either fact.
    subtitle_style::mpv_properties(subtitle.style, family)?;
    let (width, height) = (subtitle.display_width, subtitle.display_height);
    ensure!(
        (1..=16384).contains(&width)
            && (1..=16384).contains(&height)
            && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024,
        "subtitle display dimensions exceed the finite canvas limits"
    );
    let height = f64::from(height);
    let size = f64::from(subtitle.style.font_size) * height / 1080.0;
    let outline = f64::from(subtitle.style.outline) * height / 1080.0;
    let margin_v = f64::from(subtitle.style.margin_percent) * height / 100.0;
    // Fixed shared candidate: mpv 0.41 sub/osd.c's 19 at reference height720,
    // scaled as sub/ass_mp.c does. The realtime bridge must explicitly set19.
    // These integer ASS fields truncate as mpv's ASS_Style integer assignment.
    let margin_x = (19.0 * height / 720.0) as u32;
    let margin_v = margin_v as u32;
    // Same shadow/box rule as the realtime mapping. ASS alpha is inverted
    // (00 opaque); BorderStyle=4 is libass's background box.
    let (border_style, back_colour, shadow) = match subtitle.style.box_opacity {
        Some(opacity) => (
            4,
            format!("&H{:02X}000000", 255 - subtitle_style::box_alpha(opacity)),
            subtitle_style::BOX_MARGIN,
        ),
        None => (
            1,
            "&H80000000".into(),
            f64::from(subtitle.style.shadow.unwrap_or(0)),
        ),
    };
    let shadow = shadow * height / 1080.0;
    let bold = if subtitle.style.bold == Some(true) {
        -1
    } else {
        0
    };
    let force_style = format!(
        "FontName={family},FontSize={size},PrimaryColour={},SecondaryColour={},OutlineColour=&H00000000,BackColour={back_colour},BorderStyle={border_style},Outline={outline},Shadow={shadow},Alignment=2,MarginL={margin_x},MarginR={margin_x},MarginV={margin_v},Bold={bold},Italic=0,Spacing=0,ScaleX=1,ScaleY=1,PlayResX={width},PlayResY={},ScaledBorderAndShadow=yes",
        ass_color(subtitle.style.color),
        ass_color(subtitle.style.color),
        subtitle.display_height
    );
    // force_style ScaleX/Y are libass internal multipliers (1), not ASS style
    // row percentages (100); see this round's style-mapping.md counterexample.
    Ok(format!(
        "subtitles=filename='{carrier}':fontsdir='{fonts}':force_style='{force_style}'"
    ))
}

/// Build only a bounded typed single video chain. Source decoding/autorotate,
/// actual canvas facts, leased assets and process execution belong to caller.
pub fn video(
    resources: &[Resource<VideoNode>],
    subtitle: Option<Subtitle<'_>>,
    time: f64,
) -> Result<String> {
    video_range(resources, subtitle, time, None)
}

/// Keep clip bounds on the source timeline; resetting at the first accepted
/// frame before the absolute end test would admit frames outside [start,end).
pub fn video_range(
    resources: &[Resource<VideoNode>],
    subtitle: Option<Subtitle<'_>>,
    time: f64,
    end: Option<f64>,
) -> Result<String> {
    ensure!(
        end.is_none_or(|end| end.is_finite() && time < end && end <= 86400.),
        "clip end must be finite after start within 24 hours"
    );
    ensure!(
        time.is_finite() && (0.0..=86400.0).contains(&time),
        "time must be finite within 0..=24 hours"
    );
    let effects = effects::render_lavfi(&Descriptors::Video(resources.to_vec()))?;
    let mut graph = format!("{effects},{SQUARE_PIXELS},settb=AVTB");
    if let Some(subtitle) = subtitle {
        let filter = subtitle_filter(&subtitle)?;
        let offset = f64::from(subtitle.style.offset_ms) / 1000.0;
        // Positive delay moves the subtitle evaluation clock backwards, then
        // restores video timestamps before trim. Negative offsets remain signed.
        graph.push_str(&format!(
            ",setpts=PTS-({offset})/TB,{filter},setpts=PTS+({offset})/TB"
        ));
    }
    if let Some(end) = end {
        graph.push_str(&format!(",trim=start={time}:end={end},setpts=PTS-STARTPTS"));
    } else {
        graph.push_str(&format!(",trim=start={time},setpts=PTS-STARTPTS"));
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(style: &Style) -> Subtitle<'_> {
        Subtitle {
            carrier: Path::new("/owned/cue.vtt"),
            fonts_dir: Path::new("/owned/fonts"),
            family: "Noto Sans CJK SC",
            style,
            display_width: 1920,
            display_height: 1080,
        }
    }

    #[test]
    fn reference_units_colors_and_scale_factor_are_explicit() {
        let mut style = subtitle_style::default_for("font".into());
        for (color, expected) in [
            (Color::White, "&H00FFFFFF"),
            (Color::Yellow, "&H0000FFFF"),
            (Color::Cyan, "&H00FFFF00"),
        ] {
            style.color = color;
            let graph = video(&[], Some(descriptor(&style)), 2.5).unwrap();
            assert!(graph.contains(&format!("PrimaryColour={expected}")));
            assert!(graph.contains("FontSize=36,"));
            assert!(
                graph.contains("Outline=2,Shadow=0,Alignment=2,MarginL=28,MarginR=28,MarginV=54,")
            );
            assert!(graph.contains(
                "ScaleX=1,ScaleY=1,PlayResX=1920,PlayResY=1080,ScaledBorderAndShadow=yes"
            ));
            assert!(graph.ends_with("trim=start=2.5,setpts=PTS-STARTPTS"));
        }
        let mut subtitle = descriptor(&style);
        subtitle.display_width = 640;
        subtitle.display_height = 360;
        let graph = video(&[], Some(subtitle), 0.0).unwrap();
        assert!(graph.contains("FontSize=12,"));
        assert!(graph.contains("MarginL=9,MarginR=9,MarginV=18,"));
        style.bold = Some(true);
        style.shadow = Some(2);
        let graph = video(&[], Some(descriptor(&style)), 0.0).unwrap();
        assert!(graph.contains("BackColour=&H80000000,BorderStyle=1,Outline=2,Shadow=2,"));
        assert!(graph.contains("Bold=-1,"));
        style.box_opacity = Some(100);
        let graph = video(&[], Some(descriptor(&style)), 0.0).unwrap();
        assert!(graph.contains("BackColour=&H00000000,BorderStyle=4,Outline=2,Shadow=6,"));
    }

    #[test]
    fn signed_delay_surrounds_subtitle_and_trim_follows_restore() {
        for offset_ms in [-1500, 1500] {
            let mut style = subtitle_style::default_for("font".into());
            style.offset_ms = offset_ms;
            let graph = video(&[], Some(descriptor(&style)), 4.0).unwrap();
            let offset = f64::from(offset_ms) / 1000.0;
            assert!(graph.contains(&format!("setpts=PTS-({offset})/TB,subtitles=")));
            assert!(graph.ends_with(&format!(
                "setpts=PTS+({offset})/TB,trim=start=4,setpts=PTS-STARTPTS"
            )));
        }
    }

    #[test]
    fn typed_effects_precede_sar_like_realtime_and_keep_stable_resource_order() {
        let resource = |id: &str, sigma| Resource {
            manifest_id: id.into(),
            resource_id: "blur".into(),
            nodes: vec![VideoNode::Gblur { sigma, steps: 3 }],
        };
        let graph = video(&[resource("z", 2.0), resource("a", 1.0)], None, 0.0).unwrap();
        assert!(graph.starts_with("gblur@n0=sigma=1:sigmaV=1:steps=3,gblur@n1=sigma=2:"));
        assert!(graph.find("gblur@n1").unwrap() < graph.find(SQUARE_PIXELS).unwrap());
        assert!(!graph.contains("subtitles="));
        assert!(video(&[resource("a", f64::NAN)], None, 0.0).is_err());
    }

    #[test]
    fn frame_blend_changes_the_exported_frame_rate_and_the_view_never_enters() {
        let resources = [Resource {
            manifest_id: "video".into(),
            resource_id: "blend".into(),
            nodes: vec![VideoNode::FrameBlend { fps: 48.0 }],
        }];
        let graph = video_range(&resources, None, 1.0, Some(6.0)).unwrap();
        assert!(graph.starts_with("framerate@n0=fps=48,"), "{graph}");
        assert!(graph.find("framerate@n0").unwrap() < graph.find("trim=").unwrap());
        // Display-layer settings (vo properties) are not part of the offline chain.
        for property in [
            "video-zoom",
            "video-pan",
            "video-aspect",
            "panscan",
            "keepaspect",
        ] {
            assert!(!graph.contains(property));
        }
    }

    #[test]
    fn clip_end_is_absolute_before_timestamp_reset() {
        let graph = video_range(&[], None, 4.31, Some(4.32)).unwrap();
        assert!(graph.ends_with("trim=start=4.31:end=4.32,setpts=PTS-STARTPTS"));
        assert!(graph.find("settb=AVTB").unwrap() < graph.find("trim=start=").unwrap());
        assert!(!graph.contains("trim=duration"));
        assert!(video_range(&[], None, 4.31, Some(4.31)).is_err());
        assert!(video_range(&[], None, 4.31, Some(f64::NAN)).is_err());
    }

    #[test]
    fn rejects_expression_paths_families_and_unbounded_inputs() {
        let style = subtitle_style::default_for("font".into());
        for path in [
            "relative.vtt",
            "/owned/a b.vtt",
            "/owned/a'b.vtt",
            "/owned/a:b.vtt",
            "/owned/../a.vtt",
            "/owned/字.vtt",
        ] {
            let mut subtitle = descriptor(&style);
            subtitle.carrier = Path::new(path);
            assert!(video(&[], Some(subtitle), 0.0).is_err());
        }
        for family in [
            "",
            " Font",
            "Font,Outline=0",
            "Font'",
            "Font:other",
            "Font\n",
        ] {
            let mut subtitle = descriptor(&style);
            subtitle.family = family;
            assert!(video(&[], Some(subtitle), 0.0).is_err());
        }
        for time in [f64::NAN, f64::INFINITY, -1.0, 86400.1] {
            assert!(video(&[], None, time).is_err());
        }
        for (width, height) in [(0, 360), (640, 0), (16385, 1), (16384, 16384)] {
            let mut subtitle = descriptor(&style);
            subtitle.display_width = width;
            subtitle.display_height = height;
            assert!(video(&[], Some(subtitle), 0.0).is_err());
        }
        assert!(video(&[], None, 86400.0).is_ok());
    }
}
