//! Files a plugin hands the video processing: `lut3d` `.cube` files
//! (`VideoNode::Lut3d`) and user shaders (`plugin.video.shaders()`, SDK
//! 1.5.39, player 0.5.20). Like images (`ui_image::locate`), a file must be
//! the plugin's own: inside its package directory or its data directory
//! after following links *at the time of the check*. This keeps a plugin
//! from naming arbitrary paths through the API; it is not a sandbox. The
//! file is read later by mpv/FFmpeg (at each load, snapshot or export), so
//! a plugin can still replace it with a link or use a hard link to a file
//! elsewhere after the check, and plugins run as unsandboxed Node processes
//! that can read the user's files anyway. Checked on the main (UI) thread
//! when the request arrives (the capability layer has the roots), like
//! `ui.image`: a canonicalize, a stat and for `.cube` up to 64 KiB read per
//! request; the playback worker gets the canonical path.
#![forbid(unsafe_code)]

use crate::host::RpcError;
use crate::playback::effects;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// Bytes of one shader file.
pub const MAX_SHADER_BYTES: u64 = 16 * 1024 * 1024;
/// Bytes of one `.cube` file.
pub const MAX_LUT_BYTES: u64 = 16 * 1024 * 1024;
/// Largest `LUT_3D_SIZE` (points per axis; 65 is the usual maximum of
/// grading tools).
pub const MAX_LUT_SIZE: u32 = 65;
/// The header is read up to here looking for `LUT_3D_SIZE`.
const HEADER_BYTES: u64 = 64 * 1024;

/// The canonical path of `path`: absolute, inside one of `roots` after
/// following links, a regular file of at most `max` bytes. `what` names it
/// in messages ("shader", "lut3d file").
fn locate(path: &str, roots: &[PathBuf], what: &str, max: u64) -> Result<PathBuf, RpcError> {
    if !Path::new(path).is_absolute() {
        return Err(RpcError::invalid(format!(
            "{what} path must be absolute: {path}"
        )));
    }
    let real = dunce::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            RpcError::new("missing", format!("{what} not found: {path}"))
        }
        _ => RpcError::invalid(format!("{what} unavailable: {path}: {error}")),
    })?;
    if !roots.iter().any(|root| real.starts_with(root)) {
        return Err(RpcError::invalid(format!(
            "{what} must be in the plugin's package or data directory: {path}"
        )));
    }
    let meta = std::fs::metadata(&real).map_err(|e| RpcError::invalid(e.to_string()))?;
    if !meta.is_file() {
        return Err(RpcError::invalid(format!("{what} is not a file: {path}")));
    }
    if meta.len() > max {
        return Err(RpcError::invalid(format!(
            "{what} is larger than {} MiB: {path}",
            max / 1024 / 1024
        )));
    }
    Ok(real)
}

fn text(path: PathBuf, what: &str) -> Result<String, RpcError> {
    path.into_os_string()
        .into_string()
        .map_err(|_| RpcError::invalid(format!("{what} path is not valid Unicode")))
}

/// A user shader for mpv's `glsl-shaders` (any name: mpv reads `.glsl`
/// and `.hook` files alike).
pub fn shader(path: &str, roots: &[PathBuf]) -> Result<String, RpcError> {
    text(locate(path, roots, "shader", MAX_SHADER_BYTES)?, "shader")
}

/// A `lut3d` `.cube` file: located like a shader, with its `LUT_3D_SIZE`
/// header checked (a 1D `.cube` or a size over [`MAX_LUT_SIZE`] is
/// refused here instead of failing in the filter graph later).
pub fn lut(path: &str, roots: &[PathBuf]) -> Result<String, RpcError> {
    let real = locate(path, roots, "lut3d file", MAX_LUT_BYTES)?;
    let real = text(real, "lut3d file")?;
    // The canonical name is what FFmpeg sees (it picks the parser by the
    // extension), and what the graph must carry.
    effects::lut_path(&real).map_err(|e| RpcError::invalid(e.to_string()))?;
    let file = std::fs::File::open(&real).map_err(|e| RpcError::invalid(e.to_string()))?;
    cube_size(BufReader::new(file.take(HEADER_BYTES)))
        .map_err(|message| RpcError::invalid(format!("lut3d file {path}: {message}")))?;
    Ok(real)
}

/// The `LUT_3D_SIZE` of a `.cube` header (Adobe/Resolve format): keywords
/// and comments come before the data lines.
fn cube_size(reader: impl BufRead) -> Result<u32, String> {
    for line in reader.lines() {
        let line = line.map_err(|_| "not a text file".to_owned())?;
        let line = line.trim();
        let mut words = line.split_whitespace();
        match words.next() {
            Some("LUT_3D_SIZE") => {
                let size = words
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .filter(|n| (2..=MAX_LUT_SIZE).contains(n))
                    .ok_or_else(|| format!("LUT_3D_SIZE must be 2 to {MAX_LUT_SIZE}"))?;
                return Ok(size);
            }
            Some("LUT_1D_SIZE") => return Err("a 1D LUT; lut3d needs LUT_3D_SIZE".into()),
            // A data line before the size: not a 3D cube.
            Some(word) if word.parse::<f64>().is_ok() => break,
            _ => (),
        }
    }
    Err("no LUT_3D_SIZE line before the data".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        vec![dunce::canonicalize(dir.path()).unwrap()]
    }

    #[test]
    fn files_must_be_the_plugins_own_regular_files() {
        let package = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let inside = package.path().join("sharpen.glsl");
        std::fs::write(&inside, "//!HOOK MAIN\n").unwrap();
        let outside = other.path().join("x.glsl");
        std::fs::write(&outside, "//!HOOK MAIN\n").unwrap();
        let roots = roots(&package);
        let found = shader(inside.to_str().unwrap(), &roots).unwrap();
        assert_eq!(PathBuf::from(&found), dunce::canonicalize(&inside).unwrap());
        let code = |path: &Path| shader(path.to_str().unwrap(), &roots).unwrap_err().code;
        assert_eq!(code(&outside), "invalid", "another directory");
        assert_eq!(code(&package.path().join("none.glsl")), "missing");
        assert_eq!(code(package.path()), "invalid", "a directory");
        assert_eq!(shader("relative.glsl", &roots).unwrap_err().code, "invalid");
        // `..` out of the package resolves outside it.
        let escape = package
            .path()
            .join("..")
            .join(other.path().file_name().unwrap())
            .join("x.glsl");
        assert_eq!(code(&escape), "invalid");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_package_is_refused() {
        let package = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let target = other.path().join("x.cube");
        std::fs::write(&target, "LUT_3D_SIZE 2\n").unwrap();
        let link = package.path().join("x.cube");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = lut(link.to_str().unwrap(), &roots(&package)).unwrap_err();
        assert_eq!(error.code, "invalid");
    }

    #[test]
    fn cube_headers_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(&dir);
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let good = write(
            "good.cube",
            "# comment\nTITLE \"x\"\nDOMAIN_MIN 0 0 0\nLUT_3D_SIZE 2\n0 0 0\n",
        );
        assert!(lut(&good, &roots).is_ok());
        assert!(lut(&write("UPPER.CUBE", "LUT_3D_SIZE 33\n"), &roots).is_ok());
        for (name, text) in [
            ("one.cube", "LUT_1D_SIZE 1024\n0 0 0\n"),
            ("big.cube", "LUT_3D_SIZE 66\n"),
            ("small.cube", "LUT_3D_SIZE 1\n"),
            ("data.cube", "0 0 0\nLUT_3D_SIZE 2\n"),
            ("none.cube", "TITLE \"x\"\n"),
            ("binary.cube", "\u{0}\u{1}\u{2}"),
        ] {
            assert_eq!(
                lut(&write(name, text), &roots).unwrap_err().code,
                "invalid",
                "{name}"
            );
        }
        assert_eq!(
            lut(&write("x.3dl", "LUT_3D_SIZE 2\n"), &roots)
                .unwrap_err()
                .code,
            "invalid",
            "only .cube"
        );
        let bracket = dir.path().join("a[1]");
        std::fs::create_dir(&bracket).unwrap();
        std::fs::write(bracket.join("x.cube"), "LUT_3D_SIZE 2\n").unwrap();
        assert_eq!(
            lut(bracket.join("x.cube").to_str().unwrap(), &roots)
                .unwrap_err()
                .code,
            "invalid",
            "the graph cannot carry brackets"
        );
    }
}
