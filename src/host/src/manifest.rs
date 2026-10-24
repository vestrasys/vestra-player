//! The plugin manifest (`manifest.json`): one structure for development
//! loads and packages, one rule table shared with `vestra-plugin check`
//! (`tests/sdk/host-agreement.test.ts` compares the limits).
//!
//! Unknown fields are rejected. `minPlayerVersion` is read before the strict
//! parse, so a manifest written for a newer player reports the version it
//! needs instead of an unknown field.
#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const NAME_MAX: usize = 64;
pub const AUTHOR_MAX: usize = 64;
pub const PUNCHLINE_MAX: usize = 120;
pub const DESCRIPTION_MAX: usize = 2000;
pub const TAGS_MAX: usize = 8;
pub const TAG_MAX: usize = 24;
pub const LICENSE_MAX: usize = 64;
pub const URL_MAX: usize = 2048;
pub const ICON_BYTES_MAX: u64 = 512 * 1024;
pub const ICON_SIDE_MAX: u32 = 1024;
pub const DOCUMENT_BYTES_MAX: u64 = 256 * 1024;
pub const ID_MAX: usize = 128;
pub const PATH_MAX: usize = 1024;
pub const PATH_DEPTH_MAX: usize = 12;

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub api_version: String,
    pub entry: String,
    pub capabilities: Vec<String>,
    /// Required in packages; a development load may leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punchline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_player_version: Option<String>,
}

/// The player is older than a manifest's `minPlayerVersion` (RPC code
/// `versionMismatch`).
#[derive(Debug)]
pub struct NeedsPlayer(pub String);
impl std::fmt::Display for NeedsPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "requires Vestra Player ≥ {}", self.0)
    }
}
impl std::error::Error for NeedsPlayer {}

/// This player's version (`plugin.app.version`).
pub fn player_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Strict `MAJOR.MINOR.PATCH` (u32 components, no leading zeros).
pub fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    if value.len() > 32 {
        return None;
    }
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || !parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
    {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// The player's own version: its first three numeric components (a
/// pre-release suffix is ignored).
fn player_triple(player: &str) -> (u32, u32, u32) {
    let core = player.split(['-', '+']).next().unwrap_or_default();
    parse_version(core).unwrap_or((0, 0, 0))
}

/// `Err(NeedsPlayer)` when `player` is older than `min`.
pub fn check_player(min: &str, player: &str) -> Result<()> {
    let wanted = parse_version(min).with_context(
        || "minPlayerVersion must be ASCII MAJOR.MINOR.PATCH without leading zeros",
    )?;
    if player_triple(player) < wanted {
        return Err(NeedsPlayer(min.to_owned()).into());
    }
    Ok(())
}

/// Parse `manifest.json` bytes: `minPlayerVersion` first, then strictly.
pub fn parse(bytes: &[u8], player: &str) -> Result<Manifest> {
    let value: serde_json::Value = serde_json::from_slice(bytes).context("manifest.json")?;
    ensure!(value.is_object(), "manifest.json must be an object");
    if let Some(min) = value.get("minPlayerVersion") {
        let min = min.as_str().context("minPlayerVersion must be a string")?;
        check_player(min, player)?;
    }
    serde_json::from_value(value).context("manifest.json")
}

/// Read and check a development load's manifest (`--plugin DIR`).
pub fn read_development(directory: &Path) -> Result<Manifest> {
    let manifest = parse(
        &std::fs::read(directory.join("manifest.json")).context("manifest.json")?,
        player_version(),
    )?;
    check(&manifest, directory, false)?;
    Ok(manifest)
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

/// An id reserved for the player's built-in plugins (ecosystem-plan.md,
/// 2026-10-24 ruling): `vestra` and `vestra.<name>`, ignoring ASCII case. Only
/// the installation's built-in set installs such a package; development loads
/// (`--plugin`) may use one but are never installed.
pub fn reserved_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower == "vestra" || lower.starts_with("vestra.")
}

/// A package-relative path: UTF-8, `/`-separated normal components only.
pub fn relative_path(text: &str) -> Result<()> {
    ensure!(
        !text.is_empty()
            && text.len() <= PATH_MAX
            && !text.chars().any(char::is_control)
            && !text.contains('\\'),
        "invalid package relative path"
    );
    let path = Path::new(text);
    ensure!(
        path.components().count() <= PATH_DEPTH_MAX
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "package path must contain only normal relative components within the depth limit"
    );
    ensure!(
        text.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".."),
        "invalid package path component"
    );
    Ok(())
}

fn text(field: &str, value: &Option<String>, max: usize, multiline: bool) -> Result<()> {
    let Some(value) = value else { return Ok(()) };
    let count = value.chars().count();
    ensure!(
        (1..=max).contains(&count),
        "{field} must have 1..{max} characters"
    );
    ensure!(
        value.trim() == value,
        "{field} must not start or end with whitespace"
    );
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() && !(multiline && (c == '\n' || c == '\t'))),
        "{field} must not contain control characters"
    );
    Ok(())
}

/// The file a path field names, inside `directory` (no symlink escapes).
fn package_file(
    directory: &Path,
    field: &str,
    relative: &str,
    max: u64,
) -> Result<std::path::PathBuf> {
    relative_path(relative).with_context(|| format!("{field}: {relative}"))?;
    let root = dunce::canonicalize(directory)?;
    let path = dunce::canonicalize(directory.join(relative))
        .with_context(|| format!("{field} file {relative} is missing from the package"))?;
    ensure!(
        path.starts_with(&root),
        "{field} must remain inside the package"
    );
    let meta = std::fs::symlink_metadata(&path)?;
    ensure!(meta.is_file(), "{field} must name a file");
    ensure!(meta.len() <= max, "{field} file exceeds {} KB", max / 1024);
    Ok(path)
}

fn document(directory: &Path, field: &str, value: &Option<String>) -> Result<()> {
    let Some(relative) = value else { return Ok(()) };
    ensure!(
        relative.to_ascii_lowercase().ends_with(".md"),
        "{field} must be a .md file"
    );
    let path = package_file(directory, field, relative, DOCUMENT_BYTES_MAX)?;
    std::str::from_utf8(&std::fs::read(path)?)
        .map_err(|_| anyhow::anyhow!("{field} must be UTF-8"))?;
    Ok(())
}

/// Every manifest rule. `package` requires a version and a production
/// `.js`/`.mjs` entry; a development load accepts `.ts` and no version.
pub fn check(manifest: &Manifest, directory: &Path, package: bool) -> Result<()> {
    ensure!(valid_id(&manifest.id), "invalid plugin id");
    if manifest.api_version != crate::plugins::API {
        bail!("unsupported apiVersion: {}", manifest.api_version);
    }
    ensure!(
        manifest
            .capabilities
            .iter()
            .all(|c| crate::plugins::CAPABILITIES.contains(&c.as_str())),
        "unknown required capability"
    );
    ensure!(
        manifest
            .capabilities
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == manifest.capabilities.len(),
        "capabilities must be unique"
    );
    match &manifest.version {
        Some(version) => ensure!(
            parse_version(version).is_some(),
            "version must be ASCII MAJOR.MINOR.PATCH without leading zeros"
        ),
        None => ensure!(!package, "a package requires a version"),
    }
    relative_path(&manifest.entry)?;
    if package {
        ensure!(
            manifest.entry.ends_with(".js") || manifest.entry.ends_with(".mjs"),
            "candidate requires a production .js or .mjs entry"
        );
    }
    if let Some(min) = &manifest.min_player_version {
        check_player(min, player_version())?;
    }
    text("name", &manifest.name, NAME_MAX, false)?;
    text("author", &manifest.author, AUTHOR_MAX, false)?;
    text("punchline", &manifest.punchline, PUNCHLINE_MAX, false)?;
    text("description", &manifest.description, DESCRIPTION_MAX, true)?;
    text("license", &manifest.license, LICENSE_MAX, false)?;
    if let Some(tags) = &manifest.tags {
        ensure!(tags.len() <= TAGS_MAX, "at most {TAGS_MAX} tags");
        let mut seen = std::collections::BTreeSet::new();
        for tag in tags {
            text("tag", &Some(tag.clone()), TAG_MAX, false)?;
            ensure!(seen.insert(tag.to_lowercase()), "duplicate tag {tag}");
        }
    }
    if let Some(url) = &manifest.source_url {
        ensure!(url.len() <= URL_MAX, "sourceUrl exceeds {URL_MAX} bytes");
        let parsed = url::Url::parse(url).context("sourceUrl must be a URL")?;
        ensure!(
            parsed.scheme() == "https",
            "sourceUrl must be an https: address"
        );
    }
    if let Some(icon) = &manifest.icon {
        let lower = icon.to_ascii_lowercase();
        ensure!(
            [".png", ".jpg", ".jpeg"].iter().any(|x| lower.ends_with(x)),
            "icon must be a .png, .jpg or .jpeg file"
        );
        let path = package_file(directory, "icon", icon, ICON_BYTES_MAX)?;
        let reader = image::ImageReader::new(std::io::Cursor::new(std::fs::read(path)?))
            .with_guessed_format()?;
        ensure!(
            matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
            ),
            "icon must be a PNG or JPEG image"
        );
        let (width, height) = reader.into_dimensions().context("icon")?;
        ensure!(
            width <= ICON_SIDE_MAX && height <= ICON_SIDE_MAX,
            "icon sides must be at most {ICON_SIDE_MAX} pixels"
        );
    }
    document(directory, "readme", &manifest.readme)?;
    document(directory, "changelog", &manifest.changelog)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vestra_ids_are_reserved_for_built_ins() {
        for id in ["vestra", "vestra.video", "Vestra.youtube", "VESTRA.x.y"] {
            assert!(reserved_id(id), "{id}");
        }
        for id in [
            "zysilm.youtube",
            "vestra-fan.x",
            "vestrax.y",
            "my.vestra.z",
            "workshop.counter",
        ] {
            assert!(!reserved_id(id), "{id}");
        }
    }

    fn package(extra: serde_json::Value) -> (tempfile::TempDir, Manifest) {
        let root = tempfile::tempdir().unwrap();
        let mut value = json!({"id":"someone.demo","apiVersion":crate::plugins::API,
            "version":"1.0.0","entry":"main.js","capabilities":[]});
        for (k, v) in extra.as_object().unwrap() {
            value[k] = v.clone();
        }
        std::fs::write(root.path().join("main.js"), "").unwrap();
        let manifest = serde_json::from_value(value).unwrap();
        (root, manifest)
    }
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }
    fn ok(extra: serde_json::Value) -> bool {
        let (root, manifest) = package(extra);
        check(&manifest, root.path(), true).is_ok()
    }

    #[test]
    fn display_text_fields_are_bounded() {
        assert!(ok(
            json!({"name":"演示","author":"someone","punchline":"一句话",
            "description":"第一行\n第二行","tags":["字幕","工具"],"license":"MIT",
            "sourceUrl":"https://github.com/someone/demo"})
        ));
        assert!(!ok(json!({"name":""})));
        assert!(!ok(json!({"name":"x".repeat(65)})));
        assert!(ok(json!({"name":"字".repeat(64)})));
        assert!(!ok(json!({"name":" padded"})));
        assert!(!ok(json!({"punchline":"two\nlines"})));
        assert!(!ok(json!({"description":"x".repeat(2001)})));
        assert!(!ok(json!({"tags":["a","b","c","d","e","f","g","h","i"]})));
        assert!(!ok(json!({"tags":["Tag","tag"]})));
        assert!(!ok(json!({"sourceUrl":"http://example.com"})));
        assert!(!ok(json!({"sourceUrl":"not a url"})));
        assert!(!ok(json!({"license":"x".repeat(65)})));
    }

    #[test]
    fn icon_and_documents_must_be_inside_and_well_formed() {
        let (root, manifest) = package(json!({"icon":"icon.png","readme":"docs/README.md"}));
        assert!(
            check(&manifest, root.path(), true).is_err(),
            "missing files"
        );
        std::fs::write(root.path().join("icon.png"), png(128, 128)).unwrap();
        std::fs::create_dir(root.path().join("docs")).unwrap();
        std::fs::write(root.path().join("docs/README.md"), "# 演示").unwrap();
        check(&manifest, root.path(), true).unwrap();
        std::fs::write(root.path().join("icon.png"), png(1025, 8)).unwrap();
        assert!(check(&manifest, root.path(), true).is_err(), "too large");
        std::fs::write(root.path().join("icon.png"), b"GIF89a").unwrap();
        assert!(check(&manifest, root.path(), true).is_err(), "not PNG");
        for bad in ["../icon.png", "/icon.png", "a\\icon.png", "icon.gif"] {
            let (root, manifest) = package(json!({ "icon": bad }));
            assert!(check(&manifest, root.path(), true).is_err(), "{bad}");
        }
        let (root, manifest) = package(json!({"readme":"README.txt"}));
        std::fs::write(root.path().join("README.txt"), "x").unwrap();
        assert!(check(&manifest, root.path(), true).is_err());
        // Default README.md / CHANGELOG.md are optional.
        let (root, manifest) = package(json!({}));
        check(&manifest, root.path(), true).unwrap();
    }

    #[test]
    fn min_player_version_is_checked_before_unknown_fields() {
        let bytes = serde_json::to_vec(&json!({"id":"a","apiVersion":crate::plugins::API,
            "version":"1.0.0","entry":"main.js","capabilities":[],
            "minPlayerVersion":"9.0.0","futureField":true}))
        .unwrap();
        let error = parse(&bytes, "0.2.0").unwrap_err();
        assert!(error.downcast_ref::<NeedsPlayer>().is_some(), "{error}");
        assert!(
            parse(&bytes, "9.0.0")
                .unwrap_err()
                .to_string()
                .contains("manifest")
        );
        check_player("0.2.0", "0.2.0").unwrap();
        check_player("0.2.0", "0.2.0-beta.1").unwrap();
        assert!(check_player("0.2.1", "0.2.0").is_err());
        assert!(check_player("0.10.0", "0.9.9").is_err());
        assert!(check_player("01.0.0", "9.0.0").is_err(), "strict format");
    }

    #[test]
    fn packages_need_a_version_and_a_production_entry() {
        let (root, mut manifest) = package(json!({}));
        manifest.version = None;
        assert!(check(&manifest, root.path(), true).is_err());
        check(&manifest, root.path(), false).unwrap();
        manifest.entry = "main.ts".into();
        assert!(check(&manifest, root.path(), true).is_err());
        check(&manifest, root.path(), false).unwrap();
    }
}
