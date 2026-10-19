//! Experimental package snapshot, not an installer or a sandbox.
//! Final manifest/API may differ. No code, Slint or import graph is executed or
//! validated here. Metadata checks detect ordinary source changes, not hostile
//! same-size/same-mtime writes by another process with the same OS permissions.
#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
    sync::Arc,
};
use tempfile::TempDir;

const MAX_FILES: usize = 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_PATH: usize = 1024;
const MAX_DEPTH: usize = 12;
const DIGEST_DOMAIN: &[u8] = b"vestra.experimental.package-stage/v1\0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageManifest {
    pub id: String,
    pub api_version: String,
    pub entry: String,
    pub capabilities: Vec<String>,
    pub version: String,
}

#[derive(Clone)]
pub struct Candidate {
    directory: Arc<TempDir>,
    manifest: PackageManifest,
    digest: String,
}

struct SourceFile {
    relative: String,
    metadata: Metadata,
}

impl Candidate {
    /// Copy first, then validate and hash precisely the staged bytes. Original
    /// manifest bytes participate in the digest; JSON is not reserialized.
    pub fn capture(source: &Path) -> Result<Self> {
        let initial_root = fs::symlink_metadata(source).context("package root metadata")?;
        ensure!(
            initial_root.is_dir() && !initial_root.file_type().is_symlink(),
            "package root must be a regular directory"
        );
        let source = source.canonicalize().context("package root")?;
        let mut files = Vec::new();
        let mut total = 0;
        collect(&source, &source, 0, &mut files, &mut total)?;
        files.sort_by(|a, b| a.relative.cmp(&b.relative));
        let directory = tempfile::Builder::new()
            .prefix("vestra-package-stage-")
            .tempdir()?;
        let mut hasher = Sha256::new();
        hasher.update(DIGEST_DOMAIN);
        let mut copied = 0_u64;
        for record in files {
            let original = source.join(&record.relative);
            let mut input = File::open(&original).context("open package file")?;
            ensure!(
                same_file(&record.metadata, &input.metadata()?),
                "package file changed before capture: {}",
                record.relative
            );
            let limit = MAX_FILE.min(MAX_TOTAL - copied);
            let mut bytes = Vec::new();
            (&mut input).take(limit + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= limit,
                "package file/total byte limit exceeded"
            );
            ensure!(
                bytes.len() as u64 == record.metadata.len(),
                "package file length changed: {}",
                record.relative
            );
            let after = fs::symlink_metadata(&original)?;
            ensure!(
                same_file(&record.metadata, &input.metadata()?)
                    && same_file(&record.metadata, &after),
                "package file changed during capture: {}",
                record.relative
            );
            copied += bytes.len() as u64;
            let target = directory.path().join(&record.relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            // Case-folding or other target filesystem aliases cannot overwrite
            // a previously staged file.
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            output.write_all(&bytes)?;
            output.flush()?;
            hasher.update((record.relative.len() as u64).to_le_bytes());
            hasher.update(record.relative.as_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        ensure!(
            same_directory(&initial_root, &fs::symlink_metadata(&source)?),
            "package root changed during capture"
        );
        let manifest: PackageManifest = serde_json::from_reader(
            File::open(directory.path().join("manifest.json"))
                .context("package manifest missing")?,
        )
        .context("candidate manifest")?;
        validate_manifest(&manifest)?;
        ensure!(
            directory.path().join(&manifest.entry).is_file(),
            "production entry missing from staged package"
        );
        Ok(Self {
            directory: Arc::new(directory),
            manifest,
            digest: format!("{:x}", hasher.finalize()),
        })
    }

    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

fn relative_name(path: &Path) -> Result<String> {
    let text = path.to_str().context("package path must be UTF-8")?;
    ensure!(
        !text.is_empty()
            && text.len() <= MAX_PATH
            && !text.chars().any(char::is_control)
            && !text.contains('\\'),
        "invalid package relative path"
    );
    ensure!(
        path.components().count() <= MAX_DEPTH
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "package path must contain only normal relative components within the depth limit"
    );
    ensure!(
        text.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".."),
        "invalid package path component"
    );
    Ok(text.to_owned())
}

fn collect(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<SourceFile>,
    total: &mut u64,
) -> Result<()> {
    ensure!(depth <= MAX_DEPTH, "package directory depth exceeded");
    // Also bound directories/empty paths, so a file-count cap cannot be evaded
    // with an unbounded empty tree. This counter is carried by the traversal.
    collect_entries(root, directory, depth, files, total, &mut 0)
}

fn collect_entries(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<SourceFile>,
    total: &mut u64,
    entries: &mut usize,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        *entries += 1;
        ensure!(
            *entries <= MAX_FILES * (MAX_DEPTH + 1),
            "package tree entry limit exceeded"
        );
        let path = entry.path();
        let relative = relative_name(path.strip_prefix(root)?)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("package symlinks are unsupported: {relative}");
        }
        if metadata.is_dir() {
            ensure!(depth < MAX_DEPTH, "package directory depth exceeded");
            collect_entries(root, &path, depth + 1, files, total, entries)?;
        } else if metadata.is_file() {
            ensure!(files.len() < MAX_FILES, "package file count exceeded");
            ensure!(
                metadata.len() <= MAX_FILE,
                "package file byte limit exceeded"
            );
            *total = total
                .checked_add(metadata.len())
                .context("package byte count overflow")?;
            ensure!(*total <= MAX_TOTAL, "package total byte limit exceeded");
            files.push(SourceFile { relative, metadata });
        } else {
            bail!("package special files are unsupported: {relative}");
        }
    }
    Ok(())
}

fn same_identity(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        true
    }
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    a.is_file()
        && b.is_file()
        && !b.file_type().is_symlink()
        && same_identity(a, b)
        && a.len() == b.len()
        && a.modified()
            .ok()
            .zip(b.modified().ok())
            .is_some_and(|(x, y)| x == y)
}
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    a.is_dir()
        && b.is_dir()
        && !b.file_type().is_symlink()
        && same_identity(a, b)
        && a.modified()
            .ok()
            .zip(b.modified().ok())
            .is_some_and(|(x, y)| x == y)
}

fn validate_manifest(manifest: &PackageManifest) -> Result<()> {
    ensure!(
        !manifest.id.is_empty()
            && manifest.id.len() <= 128
            && manifest
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)),
        "invalid plugin id"
    );
    ensure!(
        manifest.api_version == crate::plugins::API,
        "unsupported candidate apiVersion"
    );
    ensure!(
        manifest
            .capabilities
            .iter()
            .all(|c| crate::plugins::CAPABILITIES.contains(&c.as_str())),
        "unknown required capability"
    );
    ensure!(
        manifest.capabilities.len() <= crate::plugins::CAPABILITIES.len()
            && manifest
                .capabilities
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == manifest.capabilities.len(),
        "candidate capabilities must be unique and bounded"
    );
    // Three u32 components need at most 32 ASCII bytes, including two dots.
    ensure!(
        manifest.version.len() <= 32,
        "version exceeds three u32 components"
    );
    let parts = manifest.version.split('.').collect::<Vec<_>>();
    ensure!(
        parts.len() == 3
            && parts.iter().all(|p| !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
                && p.parse::<u32>().is_ok()),
        "version must be ASCII MAJOR.MINOR.PATCH without leading zeros"
    );
    relative_name(Path::new(&manifest.entry))?;
    ensure!(
        manifest.entry.ends_with(".js") || manifest.entry.ends_with(".mjs"),
        "candidate requires a production .js or .mjs entry"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&PackageManifest {
                id: "counter".into(),
                api_version: crate::plugins::API.into(),
                entry: "main.js".into(),
                capabilities: vec![],
                version: "1.0.0".into(),
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.path().join("main.js"),
            "export async function activate() {}\n",
        )
        .unwrap();
        root
    }
    fn set_manifest(root: &Path, field: &str, value: &str) {
        let file = root.join("manifest.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        json[field] = value.into();
        fs::write(file, serde_json::to_vec(&json).unwrap()).unwrap();
    }

    #[test]
    fn digest_binds_sorted_paths_bytes_and_original_manifest() {
        let root = fixture();
        fs::create_dir(root.path().join("ui")).unwrap();
        fs::write(
            root.path().join("ui/window.slint"),
            "export component Panel {}",
        )
        .unwrap();
        let first = Candidate::capture(root.path()).unwrap();
        assert_eq!(
            first.digest(),
            Candidate::capture(root.path()).unwrap().digest()
        );
        assert_eq!(first.manifest().version, "1.0.0");
        fs::write(
            root.path().join("main.js"),
            "export async function activate() { return 1; }\n",
        )
        .unwrap();
        let second = Candidate::capture(root.path()).unwrap();
        assert_ne!(first.digest(), second.digest());
        assert_ne!(
            fs::read(first.directory().join("main.js")).unwrap(),
            fs::read(root.path().join("main.js")).unwrap()
        );
        fs::rename(
            root.path().join("ui/window.slint"),
            root.path().join("ui/other.slint"),
        )
        .unwrap();
        assert_ne!(
            second.digest(),
            Candidate::capture(root.path()).unwrap().digest()
        );
    }

    #[test]
    fn strict_version_and_production_entry_are_required() {
        for value in [
            "",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.3-beta",
            "4294967296.0.0",
            "１.2.3",
        ] {
            let root = fixture();
            set_manifest(root.path(), "version", value);
            assert!(Candidate::capture(root.path()).is_err(), "{value}");
        }
        for value in [
            "../main.js",
            "/main.js",
            "./main.js",
            "a//main.js",
            "a/../main.js",
            "main.ts",
            "missing.js",
            "a\\main.js",
        ] {
            let root = fixture();
            set_manifest(root.path(), "entry", value);
            assert!(Candidate::capture(root.path()).is_err(), "{value}");
        }
    }

    #[test]
    fn contract_duplicate_or_unbounded_capabilities_are_rejected() {
        let root = fixture();
        let path = root.path().join("manifest.json");
        let mut manifest: PackageManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.capabilities = vec!["commands".into(), "commands".into()];
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(Candidate::capture(root.path()).is_err());
    }

    #[test]
    fn clone_keeps_snapshot_alive_after_source_and_first_handle_drop() {
        let root = fixture();
        let original = Candidate::capture(root.path()).unwrap();
        let path = original.directory().to_path_buf();
        let clone = original.clone();
        drop(root);
        drop(original);
        assert!(clone.directory().join("main.js").is_file());
        drop(clone);
        assert!(!path.exists());
    }

    #[test]
    fn oversized_file_and_excess_files_are_rejected_before_copying() {
        let root = fixture();
        File::create(root.path().join("large.bin"))
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(Candidate::capture(root.path()).is_err());
        fs::remove_file(root.path().join("large.bin")).unwrap();
        for n in 0..MAX_FILES {
            File::create(root.path().join(format!("f{n}"))).unwrap();
        }
        assert!(Candidate::capture(root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected_even_if_target_is_inside_package() {
        let root = fixture();
        std::os::unix::fs::symlink("main.js", root.path().join("alias.js")).unwrap();
        assert!(Candidate::capture(root.path()).is_err());
    }
}
