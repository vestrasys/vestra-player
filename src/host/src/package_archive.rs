//! `.vxp` plugin packages: a zip whose root holds `manifest.json`. Extracted
//! into a private temporary directory under the same limits as
//! `package_stage`, enforced while decompressing (zip bombs stop at the
//! limit, not after). The result is then captured like any package directory.
#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use tempfile::TempDir;

pub const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;

/// Extract `file` into a new temporary directory.
pub fn extract(file: &Path) -> Result<TempDir> {
    let meta = fs::symlink_metadata(file).context("package file")?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "package must be a regular file"
    );
    ensure!(meta.len() <= MAX_ARCHIVE, "package file exceeds 64 MB");
    let mut archive =
        zip::ZipArchive::new(File::open(file)?).context("package is not a valid .vxp (zip)")?;
    ensure!(
        archive.len() <= MAX_FILES * 2,
        "package entry count exceeded"
    );
    let directory = tempfile::Builder::new().prefix("vestra-vxp-").tempdir()?;
    let mut names = BTreeSet::new();
    let mut files = 0;
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.into_owned();
        ensure!(
            !entry.encrypted(),
            "encrypted package entries are unsupported"
        );
        ensure!(
            matches!(
                entry.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            ),
            "package entry {name}: only stored and deflate entries are supported"
        );
        ensure!(
            !entry.is_symlink(),
            "package symlinks are unsupported: {name}"
        );
        let relative = name.strip_suffix('/').unwrap_or(&name);
        crate::manifest::relative_path(relative)
            .with_context(|| format!("package entry {name}"))?;
        // Case-insensitive file systems (macOS, Windows) would merge these.
        ensure!(
            names.insert(relative.to_lowercase()),
            "duplicate package entry (ignoring case): {name}"
        );
        let target = directory.path().join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        ensure!(
            entry.is_file(),
            "package special entries are unsupported: {name}"
        );
        files += 1;
        ensure!(files <= MAX_FILES, "package file count exceeded");
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .with_context(|| format!("package entry {name}"))?;
        let limit = MAX_FILE.min(MAX_TOTAL - total);
        let copied = copy_limited(&mut entry, &mut output, limit)?;
        ensure!(
            copied <= limit,
            "package file/total byte limit exceeded: {name}"
        );
        total += copied;
    }
    ensure!(
        directory.path().join("manifest.json").is_file(),
        "manifest.json must be at the root of the package"
    );
    Ok(directory)
}

/// Copy at most `limit + 1` bytes, so an over-limit entry is detected
/// without decompressing it further.
fn copy_limited(input: &mut impl Read, output: &mut File, limit: u64) -> Result<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied += read as u64;
        if copied > limit {
            return Ok(copied);
        }
        output.write_all(&buffer[..read])?;
    }
    output.flush()?;
    Ok(copied)
}

/// The lowercase hex SHA-256 of a file.
pub fn sha256(file: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut input = File::open(file)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Reject a path that is not a `.vxp` file before opening it.
pub fn check_extension(file: &Path) -> Result<()> {
    match file.extension().and_then(|x| x.to_str()) {
        Some(x) if x.eq_ignore_ascii_case("vxp") => Ok(()),
        _ => bail!("plugin packages are .vxp files"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn archive(
        entries: &[(&str, &[u8])],
        method: zip::CompressionMethod,
    ) -> tempfile::NamedTempFile {
        let file = tempfile::Builder::new().suffix(".vxp").tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(File::create(file.path()).unwrap());
        for (name, data) in entries {
            if name.ends_with('/') {
                writer
                    .add_directory(*name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                writer
                    .start_file(
                        *name,
                        SimpleFileOptions::default().compression_method(method),
                    )
                    .unwrap();
                writer.write_all(data).unwrap();
            }
        }
        writer.finish().unwrap();
        file
    }
    fn manifest() -> Vec<u8> {
        serde_json::to_vec(
            &serde_json::json!({"id":"someone.demo","apiVersion":crate::plugins::API,
            "version":"1.0.0","entry":"main.js","capabilities":[]}),
        )
        .unwrap()
    }

    #[test]
    fn extracts_a_package_that_captures_like_its_directory() {
        let m = manifest();
        let file = archive(
            &[
                ("manifest.json", &m),
                ("ui/", b""),
                ("ui/a.slint", b"export component A {}"),
                ("main.js", b"export default () => {}"),
            ],
            zip::CompressionMethod::Deflated,
        );
        let directory = extract(file.path()).unwrap();
        let from_archive = crate::package_stage::Candidate::capture(directory.path()).unwrap();
        let plain = tempfile::tempdir().unwrap();
        fs::write(plain.path().join("manifest.json"), &m).unwrap();
        fs::create_dir(plain.path().join("ui")).unwrap();
        fs::write(plain.path().join("ui/a.slint"), "export component A {}").unwrap();
        fs::write(plain.path().join("main.js"), "export default () => {}").unwrap();
        let from_directory = crate::package_stage::Candidate::capture(plain.path()).unwrap();
        assert_eq!(from_archive.digest(), from_directory.digest());
    }

    #[test]
    fn hostile_entries_are_rejected() {
        let m = manifest();
        for name in ["../evil.js", "/abs.js", "a\\b.js", "a/../b.js", "./x.js"] {
            let file = archive(
                &[("manifest.json", &m), (name, b"x")],
                zip::CompressionMethod::Stored,
            );
            assert!(extract(file.path()).is_err(), "{name}");
        }
        let file = archive(
            &[("manifest.json", &m), ("A.js", b"1"), ("a.js", b"2")],
            zip::CompressionMethod::Stored,
        );
        assert!(extract(file.path()).is_err(), "case duplicate");
        let file = archive(&[("main.js", b"x")], zip::CompressionMethod::Stored);
        assert!(extract(file.path()).is_err(), "no manifest at root");
        let file = archive(&[("pkg/manifest.json", &m)], zip::CompressionMethod::Stored);
        assert!(extract(file.path()).is_err(), "nested root");
        let bad = tempfile::Builder::new().suffix(".vxp").tempfile().unwrap();
        fs::write(bad.path(), b"not a zip").unwrap();
        assert!(extract(bad.path()).is_err());
    }

    #[test]
    fn decompression_stops_at_the_byte_limit() {
        let m = manifest();
        // 33 MB of zeros deflate to a few dozen KB.
        let big = vec![0_u8; (MAX_FILE + 1) as usize];
        let file = archive(
            &[("manifest.json", &m), ("big.bin", &big)],
            zip::CompressionMethod::Deflated,
        );
        assert!(fs::metadata(file.path()).unwrap().len() < 1024 * 1024);
        let error = extract(file.path()).unwrap_err().to_string();
        assert!(error.contains("limit"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_entries_are_rejected() {
        let m = manifest();
        let file = tempfile::Builder::new().suffix(".vxp").tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(File::create(file.path()).unwrap());
        writer
            .start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&m).unwrap();
        writer
            .add_symlink("link.js", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        assert!(extract(file.path()).is_err());
    }

    #[test]
    fn extension_is_checked() {
        assert!(check_extension(Path::new("/a/b.vxp")).is_ok());
        assert!(check_extension(Path::new("/a/b.VXP")).is_ok());
        assert!(check_extension(Path::new("/a/b.zip")).is_err());
    }
}
