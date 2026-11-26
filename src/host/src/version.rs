//! The two versions ("Versions").
//!
//! - The **player version** ([`player`]): the release, from `release.json`
//!   (`src/release.json`, packaged as `<resources>/release.json`). It goes
//!   up on every release and is what users and plugins see: the About
//!   window and the update check (through `plugin.app.version`), the
//!   store's User-Agent, `minPlayerVersion`, the start-up log line.
//! - The **host version** ([`HOST`]): `src/host/Cargo.toml`, the version of
//!   this binary. It goes up only when host or runtime code changes, so a
//!   release that only changes built-in plugins reuses a built host. Never
//!   higher than the player version. The Windows file version resource,
//!   the GPU crash guard and lazy-activation recordings use it: they are
//!   about this binary.
//!
//! A missing or invalid `release.json` (a test, a source tree without one)
//! falls back to the host version, with one log line.
#![forbid(unsafe_code)]

use crate::packages::manifest::parse_version;
use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::sync::OnceLock;

/// This binary's version (`src/host/Cargo.toml`).
pub const HOST: &str = env!("CARGO_PKG_VERSION");

/// The player release version (`plugin.app.version`), read once from
/// `release.json` (`runtime_paths`); the host version without one.
pub fn player() -> &'static str {
    static PLAYER: OnceLock<String> = OnceLock::new();
    PLAYER.get_or_init(|| {
        let path = &crate::process::runtime_paths::get().release;
        let (version, warning) = resolve(path);
        if let Some(warning) = warning {
            diag!("version: {warning}");
        }
        version
    })
}

/// `player` as a pure function: the version in `path`, else [`HOST`] and
/// why.
pub fn resolve(path: &Path) -> (String, Option<String>) {
    match read(path) {
        Ok(version) => (version, None),
        Err(error) => (
            HOST.to_owned(),
            Some(format!(
                "{} is not used ({error:#}); the player version is the host version {HOST}",
                path.display()
            )),
        ),
    }
}

/// The `version` of a `release.json`: `MAJOR.MINOR.PATCH`, optionally with
/// a pre-release or build suffix (`-rc.1`, `+x`).
pub fn read(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).context("read")?;
    parse(&bytes)
}

/// [`read`] on the file's bytes.
pub fn parse(bytes: &[u8]) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).context("release.json")?;
    let version = value
        .get("version")
        .and_then(|version| version.as_str())
        .context("release.json needs a string \"version\"")?;
    let core = version.split(['-', '+']).next().unwrap_or_default();
    ensure!(
        version.len() <= 64 && version.is_ascii() && parse_version(core).is_some(),
        "release.json version {version:?} is not MAJOR.MINOR.PATCH"
    );
    Ok(version.to_owned())
}

/// The start-up log line's version: `0.7.2 (host 0.7.1, <build>)`.
pub fn describe(player: &str, build: Option<&str>) -> String {
    match build {
        Some(build) => format!("{player} (host {HOST}, {build})"),
        None => format!("{player} (host {HOST})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_version_comes_from_release_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release.json");
        std::fs::write(&path, r#"{ "version": "9.8.7" }"#).unwrap();
        assert_eq!(resolve(&path), ("9.8.7".to_owned(), None));
        std::fs::write(&path, r#"{"version":"1.2.3-rc.1"}"#).unwrap();
        assert_eq!(resolve(&path).0, "1.2.3-rc.1");
    }

    #[test]
    fn a_missing_or_invalid_file_falls_back_to_the_host_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release.json");
        let (version, warning) = resolve(&path);
        assert_eq!(version, HOST);
        assert!(warning.unwrap().contains(HOST));
        for bad in [
            "",
            "[]",
            "{}",
            r#"{"version":7}"#,
            r#"{"version":"1.2"}"#,
            r#"{"version":"01.2.3"}"#,
            r#"{"version":"v1.2.3"}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            let (version, warning) = resolve(&path);
            assert_eq!(version, HOST, "{bad}");
            assert!(warning.is_some(), "{bad}");
        }
    }

    #[test]
    fn a_source_run_reads_src_release_json() {
        // `runtime_paths` points a development build at `src/release.json`.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("release.json");
        assert_eq!(crate::process::runtime_paths::get().release, path);
        assert_eq!(player(), read(&path).unwrap());
    }

    /// The rule of the two versions: the release is never older than the
    /// host it ships.
    #[test]
    fn the_release_version_is_not_below_the_host_version() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("release.json");
        let release = read(&path).unwrap();
        let triple = |v: &str| parse_version(v.split(['-', '+']).next().unwrap()).unwrap();
        assert!(
            triple(&release) >= triple(HOST),
            "src/release.json {release} is below src/host/Cargo.toml {HOST}"
        );
    }

    #[test]
    fn the_start_line_names_both_versions() {
        assert_eq!(
            describe("9.0.0", Some("abc1234")),
            format!("9.0.0 (host {HOST}, abc1234)")
        );
        assert_eq!(describe("9.0.0", None), format!("9.0.0 (host {HOST})"));
    }
}
