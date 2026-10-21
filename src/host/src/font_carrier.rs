//! Fixed-font materialization for the host and frozen subtitles.
//! This owns a private directory, not a filesystem sandbox or a renderer fence.
//! Construct once per host; clones keep the same directory alive for snapshots.
#![forbid(unsafe_code)]

use std::{fs::OpenOptions, io::Write, path::Path, sync::Arc};

use anyhow::{Context, Result};
use tempfile::TempDir;

use crate::font_asset::FontAsset;

#[derive(Clone)]
pub struct FontDirectory {
    asset: FontAsset,
    directory: Arc<TempDir>,
}

impl FontDirectory {
    /// Materialize only the fixed, validated OFL font; no user paths or names.
    /// If creation/writing fails, the local TempDir drops and removes its files.
    pub fn bundled() -> Result<Self> {
        let asset = FontAsset::bundled()?;
        let directory = tempfile::Builder::new()
            .prefix("vestra-subtitle-font-")
            .tempdir()
            .context("fixed-font directory could not be created")?;
        let filename = directory.path().join("NotoSansCJKsc-Regular.otf");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&filename)
            .context("fixed-font file could not be created")?;
        file.write_all(asset.bytes().as_ref())
            .context("fixed-font bytes could not be written")?;
        file.sync_all()
            .context("fixed-font file could not be flushed")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Directory stays writable for TempDir cleanup. This prevents
            // accidental writes, not modification by trusted same-user code.
            file.set_permissions(std::fs::Permissions::from_mode(0o400))
                .context("fixed-font file permissions could not be set")?;
        }
        drop(file);
        Ok(Self {
            asset,
            directory: Arc::new(directory),
        })
    }

    pub fn asset(&self) -> &FontAsset {
        &self.asset
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn cloned_snapshot_retains_exact_font_until_last_directory_owner_leaves() {
        let directory = FontDirectory::bundled().unwrap();
        let path = directory.path().to_path_buf();
        let snapshot = directory.clone();
        assert_eq!(snapshot.path(), path);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
        let materialized = std::fs::read(path.join("NotoSansCJKsc-Regular.otf")).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&materialized)),
            directory.asset().metadata().digest
        );
        assert!(Arc::ptr_eq(
            &directory.asset().bytes(),
            &snapshot.asset().bytes()
        ));
        drop(directory);
        assert!(path.is_dir());
        snapshot.asset().supports("中文字幕").unwrap();
        drop(snapshot);
        assert!(!path.exists());
    }
}
