//! Finite offline video mapper (host-internal).
//! A graph is not font selection, backend acceptance, or pixel parity evidence.
//! Caller retains immutable source/subtitle/font leases and runs source 0 from
//! its original timeline with FFmpeg default autorotate. Do not input-seek away
//! the timeline before this graph. This module does not parse subtitle text.
#![forbid(unsafe_code)]

use std::path::{Component, Path, Prefix};

use anyhow::{Result, ensure};

use crate::{
    playback::effects::{self, Descriptors, Resource, VideoNode},
    playback::subtitle_style::{self, Color, Style},
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

/// One literal option value of a `-vf` filter, whatever its characters.
///
/// FFmpeg tokenizes a filter graph twice before a value reaches the filter:
/// the graph parser cuts the filter's arguments at `[],;`, then the filter's
/// option parser cuts `key=value` pairs at `:`. Both levels use
/// `av_get_token`, where `\` makes the next character literal, `'` quotes,
/// and leading/trailing whitespace is trimmed. Escaping each level in turn
/// (FFmpeg's filters documentation, "Notes on filtergraph escaping") keeps
/// Windows drive colons and backslashes, quotes, spaces, graph separators and
/// non-ASCII names inside the value instead of letting them end it or start
/// another option or filter.
fn filter_value(text: &str) -> String {
    fn escape(text: &str, special: &[char]) -> String {
        let mut escaped = String::with_capacity(text.len() * 2);
        for c in text.chars() {
            if special.contains(&c) || c.is_whitespace() {
                escaped.push('\\');
            }
            escaped.push(c);
        }
        escaped
    }
    let option = escape(text, &['\\', '\'', ':']);
    escape(&option, &['\\', '\'', '[', ']', ',', ';'])
}

/// The caller's own carrier/font path as an escaped filter value: absolute
/// (no protocol prefix or working-directory lookup), no parent traversal or
/// control characters, and on Windows only a drive or UNC root (not the
/// `\\?\` / `\\.\` device namespaces, in any mix of `/` and `\`: Rust parses
/// `//?/C:/x` or `//./pipe/x` as a UNC share on server `?` / `.`, yet Windows
/// opens them in the device namespace). Ownership/existence remain caller checks.
fn owned_path(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 carrier/font path"))?;
    let separator = |c: Option<&u8>| matches!(c, Some(b'/' | b'\\'));
    let bytes = text.as_bytes();
    let device_namespace = separator(bytes.first())
        && separator(bytes.get(1))
        && matches!(bytes.get(2), Some(b'?' | b'.'))
        && (bytes.len() == 3 || separator(bytes.get(3)));
    ensure!(
        path.is_absolute()
            && !device_namespace
            && text.len() <= 4096
            && !text.chars().any(char::is_control)
            && path.components().all(|c| match c {
                Component::Prefix(prefix) => match prefix.kind() {
                    Prefix::Disk(_) => true,
                    Prefix::UNC(server, _) => server != "?" && server != ".",
                    _ => false,
                },
                Component::RootDir | Component::Normal(_) => true,
                Component::CurDir | Component::ParentDir => false,
            }),
        "carrier/font paths must be absolute local paths without parent traversal or control characters"
    );
    Ok(filter_value(text))
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
    // row percentages (100).
    Ok(format!(
        "subtitles=filename={carrier}:fontsdir={fonts}:force_style='{force_style}'"
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

    /// Absolute paths on the test's own platform.
    const CARRIER: &str = if cfg!(windows) {
        r"C:\owned\cue.vtt"
    } else {
        "/owned/cue.vtt"
    };
    const FONTS: &str = if cfg!(windows) {
        r"C:\owned\fonts"
    } else {
        "/owned/fonts"
    };

    fn descriptor(style: &Style) -> Subtitle<'_> {
        Subtitle {
            carrier: Path::new(CARRIER),
            fonts_dir: Path::new(FONTS),
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
        let rejected: &[&str] = if cfg!(windows) {
            &[
                "relative.vtt",
                r"C:relative.vtt",
                r"\owned\cue.vtt",
                r"C:\owned\..\a.vtt",
                r"\\?\C:\owned\cue.vtt",
                r"\\.\pipe\cue",
                // Device namespaces spelled with forward slashes, which Rust
                // parses as UNC shares on server `?` / `.`.
                "//?/C:/x",
                r"\\?/C:/x",
                r"/\?\C:\x",
                r"//?\C:\x",
                "//?/pipe/x",
                "//./pipe/x",
                r"\\.\C:\x",
                r"/\./pipe/x",
                r"\\?\UNC\server\share\x",
                "C:\\owned\\a\nb.vtt",
            ]
        } else {
            &[
                "relative.vtt",
                "/owned/../a.vtt",
                "/owned/a\nb.vtt",
                "//?/C:/x",
                "//./pipe/x",
            ]
        };
        for path in rejected {
            let mut subtitle = descriptor(&style);
            subtitle.carrier = Path::new(path);
            assert!(video(&[], Some(subtitle), 0.0).is_err(), "{path:?}");
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

    /// FFmpeg's `av_get_token`: `\` takes the next character literally, `'`
    /// quotes up to the next `'`, leading whitespace and trailing unescaped
    /// whitespace are dropped; stops before any `term` character.
    fn av_get_token<'a>(buf: &'a str, term: &str) -> (String, &'a str) {
        const WHITESPACES: [char; 4] = [' ', '\n', '\t', '\r'];
        let rest = buf.trim_start_matches(WHITESPACES);
        let mut chars = rest.char_indices();
        let (mut out, mut kept, mut end) = (String::new(), 0, rest.len());
        while let Some((at, c)) = chars.next() {
            if term.contains(c) {
                end = at;
                break;
            }
            if c == '\\' {
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                    kept = out.len();
                } else {
                    out.push(c);
                }
            } else if c == '\'' {
                let mut closed = false;
                for (_, quoted) in chars.by_ref() {
                    if quoted == '\'' {
                        closed = true;
                        break;
                    }
                    out.push(quoted);
                }
                if closed {
                    kept = out.len();
                }
            } else {
                out.push(c);
            }
        }
        let trailing = out[kept..].trim_end_matches(WHITESPACES).len();
        out.truncate(kept + trailing);
        (out, &rest[end..])
    }

    /// Parse the `subtitles` filter of a graph the way FFmpeg's graph parser
    /// and the filter's option parser do: its `key=value` options, and what
    /// follows it in the chain.
    fn parsed_subtitles_options(graph: &str) -> (Vec<(String, String)>, String) {
        let at = graph.find("subtitles=").unwrap() + "subtitles=".len();
        let (args, rest) = av_get_token(&graph[at..], "[],;");
        let mut options = Vec::new();
        let mut args = args.as_str();
        while !args.is_empty() {
            let (key, value) = args.split_once('=').unwrap();
            let (value, next) = av_get_token(value, ":");
            options.push((key.to_owned(), value));
            args = next.strip_prefix(':').unwrap_or(next);
        }
        (options, rest.to_owned())
    }

    #[test]
    fn filter_values_survive_both_ffmpeg_parser_levels() {
        assert_eq!(
            filter_value(r"C:\Users\Zoë\AppData\Local\Temp\a b.vtt"),
            r"C\\:\\\\Users\\\\Zoë\\\\AppData\\\\Local\\\\Temp\\\\a\\\ b.vtt"
        );
        for text in [
            r"C:\Users\Zoë\AppData\Local\Temp\vestra-subtitle-1a2B.vtt",
            r"C:\Users\Zoë\AppData\Local\Temp\vestra-subtitle-font-3c4D",
            r"C:\Users\张三\AppData\Local\Temp\vestra subtitle.vtt",
            r"\\server\share\Temp\cue.vtt",
            "/Users/zoë/Library/Caches/T/vestra-subtitle-x.vtt",
            r"/tmp/a b'c:d,e;f[g]h=i\j.vtt",
            " leading and trailing ",
            r"'quoted' \ 'ends with backslash\",
        ] {
            let graph_level = filter_value(text);
            let (option_level, rest) = av_get_token(&graph_level, "[],;");
            assert_eq!(rest, "", "{text:?} ended the filter early");
            let (value, rest) = av_get_token(&option_level, ":");
            assert_eq!(rest, "", "{text:?} ended the option early");
            assert_eq!(value, text);
        }
    }

    #[test]
    fn platform_temp_paths_with_spaces_and_non_ascii_names_are_accepted() {
        let style = subtitle_style::default_for("font".into());
        let accepted: &[(&str, &str)] = if cfg!(windows) {
            &[
                (
                    r"C:\Users\Zoë\AppData\Local\Temp\vestra-subtitle-a1.vtt",
                    r"C:\Users\Zoë\AppData\Local\Temp\vestra-subtitle-font-b2",
                ),
                (
                    r"C:\profiles\guest\Local\Temp\vestra-subtitle-x.srt",
                    r"D:\Temp dir\[x],y;z'q\vestra-subtitle-font-c3",
                ),
                (r"\\server\share\Temp\cue.vtt", r"\\server\share\Temp\fonts"),
            ]
        } else {
            &[
                (
                    "/var/folders/xy/T/vestra-subtitle-a1.vtt",
                    "/var/folders/xy/T/vestra-subtitle-font-b2",
                ),
                (
                    r"/home/zoë/字 dir/a'b:c,d;[e]\f.vtt",
                    "/home/zoë/字 dir/fonts",
                ),
            ]
        };
        for (carrier, fonts) in accepted {
            let mut subtitle = descriptor(&style);
            subtitle.carrier = Path::new(carrier);
            subtitle.fonts_dir = Path::new(fonts);
            let graph = video(&[], Some(subtitle), 0.0).unwrap();
            let (options, rest) = parsed_subtitles_options(&graph);
            assert_eq!(options.len(), 3, "{options:?}");
            assert_eq!(options[0], ("filename".into(), (*carrier).into()));
            assert_eq!(options[1], ("fontsdir".into(), (*fonts).into()));
            assert_eq!(options[2].0, "force_style");
            assert!(options[2].1.starts_with("FontName=Noto Sans CJK SC,"));
            assert!(rest.starts_with(",setpts=PTS+("), "{rest}");
        }
    }

    /// Real FFmpeg/libass: a carrier and font directory under a name with
    /// spaces, non-ASCII letters and filter-graph separators render exactly
    /// what the same files under a plain name render (the bundled font is
    /// loaded from that directory, not substituted).
    #[test]
    #[ignore = "needs ffmpeg with libass"]
    fn real_ffmpeg_reads_carrier_and_fonts_from_awkward_directory_names() {
        let render = |prefix: &str| {
            let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
            let fonts = dir.path().join("fonts");
            std::fs::create_dir(&fonts).unwrap();
            std::fs::write(
                fonts.join("NotoSansCJKsc-Regular.otf"),
                crate::font_asset::bundled_bytes(),
            )
            .unwrap();
            let carrier = dir.path().join("cue 字幕.vtt");
            std::fs::write(&carrier, "WEBVTT\n\n00:00.000 --> 00:05.000\n你好 Zoë\n").unwrap();
            let style = subtitle_style::default_for("font".into());
            let subtitle = Subtitle {
                carrier: &carrier,
                fonts_dir: &fonts,
                family: "Noto Sans CJK SC",
                style: &style,
                display_width: 320,
                display_height: 240,
            };
            let graph = video(&[], Some(subtitle), 0.0).unwrap();
            let output = std::process::Command::new(super::super::media_worker::real::ffmpeg())
                .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
                .arg("color=c=black:s=320x240:d=1")
                .arg("-vf")
                .arg(&graph)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{prefix}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout.len(), 320 * 240 * 3);
            output.stdout
        };
        let plain = render("vestra-plain-");
        assert!(plain.iter().any(|&b| b > 0), "the subtitle was not drawn");
        let awkward = if cfg!(windows) {
            "vestra Zoë 字 [a],b;c'd="
        } else {
            r"vestra Zoë 字 [a],b;c'd:e\f="
        };
        assert!(render(awkward) == plain, "{awkward} rendered differently");
    }
}
