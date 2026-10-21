//! `plugin.app`: the build's git short hash and the linked Slint version.
#![forbid(unsafe_code)]
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let slint = lock
        .split("[[package]]")
        .find(|package| package.contains("\nname = \"slint\"\n"))
        .and_then(|package| {
            package.lines().find_map(|line| {
                Some(
                    line.strip_prefix("version = \"")?
                        .strip_suffix('"')?
                        .to_owned(),
                )
            })
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=VESTRA_SLINT_VERSION={slint}");
    windows_manifest();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    // No git (a source tarball): `build` is null.
    if let Some(directory) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={directory}/HEAD");
        println!("cargo:rerun-if-changed={directory}/logs/HEAD");
    }
    if let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) {
        println!("cargo:rustc-env=VESTRA_BUILD={hash}");
    }
}

/// Windows (MSVC): embed `windows/vestra.manifest` in every linked binary
/// (the host and its test harness). It turns on Common Controls v6, which
/// rfd's `common-controls-v6` needs to start at all (custom dialog buttons).
fn windows_manifest() {
    println!("cargo:rerun-if-changed=windows/vestra.manifest");
    let var = |key: &str| std::env::var(key).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" {
        return;
    }
    if var("CARGO_CFG_TARGET_ENV") != "msvc" {
        println!(
            "cargo:warning=no application manifest for this Windows toolchain; dialogs need Common Controls v6"
        );
        return;
    }
    let manifest = std::path::Path::new(&var("CARGO_MANIFEST_DIR")).join("windows/vestra.manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
