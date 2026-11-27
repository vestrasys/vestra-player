//! Removal of the host's own directories (a package directory, a plugin's
//! data directory) that never reaches anything else (user ruling
//! 2026-11-15, the forced uninstall of a record set aside):
//!
//! 1. Only a path strictly under the given root, checked on canonical paths;
//!    anything else is refused.
//! 2. Reparse points (symbolic links, junctions, mount points) are never
//!    followed: such an entry is removed as a link, its target untouched.
//! 3. No ACL is changed, no ownership taken, nothing escalated. A read-only
//!    attribute on a file or directory inside the tree is cleared once (on
//!    Windows, where it blocks deletion); anything still denied is left in
//!    place and reported with its path.

use anyhow::{Result, ensure};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

/// A reparse point: never followed, removed as a link.
pub(crate) fn is_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Whether a link entry is a directory link (removed with `remove_dir`).
fn is_directory_entry(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_dir()
    }
}

/// `path`'s canonical form when it is strictly under `root` (canonical),
/// judged on its parent so that a link entry is judged where it sits, not
/// where it points; for anything but a link, on the entry itself too.
pub fn within(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = dunce::canonicalize(root)?;
    let name = path
        .file_name()
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(|| anyhow::anyhow!("{}: no entry name", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{}: no parent", path.display()))?;
    let parent = dunce::canonicalize(parent)?;
    ensure!(
        parent.starts_with(&root),
        "{} is outside {}; refused",
        path.display(),
        root.display()
    );
    let entry = parent.join(name);
    let metadata = fs::symlink_metadata(&entry)?;
    if !is_reparse(&metadata) {
        let resolved = dunce::canonicalize(&entry)?;
        ensure!(
            resolved.starts_with(&root) && resolved != root,
            "{} resolves outside {}; refused",
            path.display(),
            root.display()
        );
    }
    ensure!(
        entry != root,
        "{} is the root itself; refused",
        path.display()
    );
    Ok(entry)
}

/// Remove `path` (strictly under `root`, `within`) and everything in it
/// under the rules above. Returns what was left, one `path: error` each;
/// empty when all of it went (or nothing was there: only `NotFound` means
/// absent; any other failure to read the entry is left and reported). A
/// path outside `root` is an error and nothing is touched.
///
/// Every entry is judged again right before it is touched: its parent must
/// still resolve under `root` (so a directory swapped for a junction while
/// it is walked is refused below it), and a directory or file must still
/// not be a reparse point when it is deleted (a swapped one is removed as
/// the link). What remains is the short window between that last check and
/// the path-based delete call itself: closing it needs deletion by handle
/// opened with `FILE_FLAG_OPEN_REPARSE_POINT` (`SetFileInformationByHandle`),
/// which std does not offer and first-party code may not call without
/// `unsafe`. Only a process of the same user that writes inside the
/// player's own folders can race it.
pub fn remove_within(root: &Path, path: &Path) -> Result<Vec<String>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Ok(vec![format!("{}: {error}", path.display())]),
    }
    let entry = within(root, path)?;
    let root = dunce::canonicalize(root)?;
    let mut left = Vec::new();
    remove_entry(&root, &entry, &mut left);
    Ok(left)
}

/// Whether `path`'s parent still resolves strictly under `root` (canonical).
fn parent_within(root: &Path, path: &Path) -> std::io::Result<bool> {
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    Ok(dunce::canonicalize(parent)?.starts_with(root))
}

fn remove_entry(root: &Path, path: &Path, left: &mut Vec<String>) {
    match parent_within(root, path) {
        Ok(true) => {}
        Ok(false) => {
            left.push(format!(
                "{}: now outside {}; refused",
                path.display(),
                root.display()
            ));
            return;
        }
        Err(error) => {
            left.push(format!("{}: {error}", path.display()));
            return;
        }
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            left.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    if is_reparse(&metadata) {
        remove_link(path, &metadata, left);
        return;
    }
    if metadata.is_dir() {
        let before = left.len();
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                left.push(format!("{}: {error}", path.display()));
                return;
            }
        };
        // Swapped for a link between the check above and the listing: the
        // listing may be the target's; not one entry of it is touched.
        if let Some(now) = swapped(path, left) {
            remove_link(path, &now, left);
            return;
        }
        for entry in entries {
            match entry {
                Ok(entry) => remove_entry(root, &entry.path(), left),
                Err(error) => left.push(format!("{}: {error}", path.display())),
            }
        }
        // Something inside stays: so does this directory (reported there).
        if left.len() == before {
            if let Some(now) = swapped(path, left) {
                remove_link(path, &now, left);
            } else if let Err(error) = retry_read_only(path, &metadata, remove_dir) {
                left.push(format!("{}: {error}", path.display()));
            }
        }
        return;
    }
    if let Some(now) = swapped(path, left) {
        remove_link(path, &now, left);
    } else if let Err(error) = retry_read_only(path, &metadata, remove_file) {
        left.push(format!("{}: {error}", path.display()));
    }
}

/// `path`'s metadata when it is now a reparse point (checked right before
/// it is touched); `None` when it is not, or is gone (an error to read it
/// is reported and then the delete reports again).
fn swapped(path: &Path, left: &mut Vec<String>) -> Option<Metadata> {
    match fs::symlink_metadata(path) {
        Ok(now) if is_reparse(&now) => Some(now),
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            left.push(format!("{}: {error}", path.display()));
            None
        }
    }
}

/// The link itself only: `remove_dir` on a directory link or a junction
/// removes the link and never its target's contents.
fn remove_link(path: &Path, metadata: &Metadata, left: &mut Vec<String>) {
    let removed = if is_directory_entry(metadata) {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    if let Err(error) = removed {
        left.push(format!("{} (link): {error}", path.display()));
    }
}

fn remove_dir(path: &Path) -> std::io::Result<()> {
    fs::remove_dir(path)
}
fn remove_file(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)
}

/// `remove(path)`; on a refusal of an entry marked read-only (Windows), the
/// attribute is cleared and `remove` tried once more. Nothing else is
/// changed: a denial for any other reason is returned as it is.
fn retry_read_only(
    path: &Path,
    metadata: &Metadata,
    remove: fn(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    match remove(path) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                && clear_read_only(path, metadata) =>
        {
            remove(path)
        }
        Err(error) => Err(error),
    }
}

/// Windows: clears `FILE_ATTRIBUTE_READONLY` of an entry marked read-only
/// (nothing else); `true` when it was cleared. Elsewhere a read-only file
/// does not block deletion: nothing is changed.
#[cfg(windows)]
fn clear_read_only(path: &Path, metadata: &Metadata) -> bool {
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return false;
    }
    // On Windows this clears the read-only attribute only.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).is_ok()
}
#[cfg(not(windows))]
fn clear_read_only(_: &Path, _: &Metadata) -> bool {
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_path_outside_the_root_is_refused_and_untouched() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"x").unwrap();
        for path in [
            outside.clone(),
            root.join("..").join("outside"),
            root.clone(),
            base.path().to_path_buf(),
        ] {
            assert!(remove_within(&root, &path).is_err(), "{}", path.display());
        }
        assert!(outside.join("keep.txt").exists());
        assert!(root.exists());
    }

    #[test]
    fn a_read_only_file_and_directory_are_removed() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let tree = root.join("tree");
        fs::create_dir_all(tree.join("sub")).unwrap();
        let file = tree.join("sub").join("locked.txt");
        fs::write(&file, b"x").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        assert_eq!(remove_within(root, &tree).unwrap(), Vec::<String>::new());
        assert!(!tree.exists());
    }

    /// A link (Windows: a junction, which needs no privilege) inside the
    /// tree pointing outside the root: the link goes, its target stays.
    #[test]
    fn a_link_inside_is_removed_as_a_link_and_its_target_survives() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let tree = root.join("tree");
        let outside = base.path().join("outside");
        fs::create_dir_all(&tree).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious.txt"), b"keep").unwrap();
        let link = tree.join("link");
        if !make_directory_link(&outside, &link) {
            // Skipped: no directory link can be made here.
            return;
        }
        assert!(fs::symlink_metadata(&link).is_ok());
        assert_eq!(remove_within(&root, &tree).unwrap(), Vec::<String>::new());
        assert!(!tree.exists());
        assert_eq!(fs::read(outside.join("precious.txt")).unwrap(), b"keep");
        // The tree itself a link: removed as the link, the target untouched.
        let top = root.join("top");
        if make_directory_link(&outside, &top) {
            assert_eq!(remove_within(&root, &top).unwrap(), Vec::<String>::new());
            assert!(fs::symlink_metadata(&top).is_err());
            assert!(outside.join("precious.txt").exists());
        }
    }

    /// An entry below a link judged against the real root (not the link):
    /// its parent resolves outside, so it is refused and the target's
    /// contents survive, whether asked for directly or met while walking
    /// (a directory swapped for a junction during the walk).
    #[test]
    fn an_entry_below_a_link_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(outside.join("x")).unwrap();
        fs::write(outside.join("x").join("precious.txt"), b"keep").unwrap();
        let held = root.join("held");
        assert!(
            make_directory_link(&outside, &held),
            "a junction needs no privilege"
        );
        assert!(remove_within(&root, &held.join("x")).is_err());
        let canonical = dunce::canonicalize(&root).unwrap();
        let mut left = Vec::new();
        remove_entry(&canonical, &held.join("x"), &mut left);
        assert!(left.iter().any(|line| line.contains("refused")), "{left:?}");
        assert_eq!(
            fs::read(outside.join("x").join("precious.txt")).unwrap(),
            b"keep"
        );
    }

    /// Only `NotFound` means absent: an entry that cannot even be read is
    /// left and reported, never passed over in silence.
    #[test]
    fn an_unreadable_entry_is_reported_not_taken_as_absent() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        assert_eq!(
            remove_within(root, &root.join("missing")).unwrap(),
            Vec::<String>::new()
        );
        #[cfg(windows)]
        let bad = root.join("bad<name");
        #[cfg(not(windows))]
        let bad = root.join("bad\0name");
        let left = remove_within(root, &bad).unwrap();
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(left[0].contains("bad"), "{left:?}");
    }

    /// A file that cannot be deleted (held open without delete sharing, as
    /// an antivirus or a running process would) is left and reported; the
    /// rest goes.
    #[cfg(windows)]
    #[test]
    fn a_file_that_cannot_be_deleted_is_left_and_reported() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let tree = root.join("tree");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("free.txt"), b"x").unwrap();
        let held = tree.join("held.txt");
        fs::write(&held, b"x").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&held)
            .unwrap();
        let left = remove_within(root, &tree).unwrap();
        assert!(
            left.iter().any(|line| line.contains("held.txt")),
            "{left:?}"
        );
        assert!(held.exists() && !tree.join("free.txt").exists());
        drop(handle);
        assert_eq!(remove_within(root, &tree).unwrap(), Vec::<String>::new());
    }

    /// A file whose deletion its ACL denies (set by the test on its own
    /// temporary file, then lifted): left and reported, never forced.
    #[cfg(windows)]
    #[test]
    fn an_access_denied_file_is_left_and_reported() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let tree = root.join("tree");
        fs::create_dir_all(&tree).unwrap();
        let denied = tree.join("denied.txt");
        fs::write(&denied, b"x").unwrap();
        let probe = tree.join("probe.txt");
        fs::write(&probe, b"x").unwrap();
        if !(deny_delete(&tree, &denied, true) & deny_delete(&tree, &probe, true)) {
            // Skipped: icacls could not set the deny entries here.
            deny_delete(&tree, &probe, false);
            deny_delete(&tree, &denied, false);
            return;
        }
        // Precondition: the deny binds this process. A plain delete of a
        // file denied the same way must be refused; where it is not (an
        // elevated process whose token lets it delete past the ACL, as on
        // GitHub's hosted Windows runners), there is nothing to observe and
        // the test is skipped.
        if fs::remove_file(&probe).is_ok() {
            deny_delete(&tree, &denied, false);
            return;
        }
        let left = remove_within(root, &tree).unwrap();
        deny_delete(&tree, &probe, false);
        deny_delete(&tree, &denied, false);
        assert!(
            left.iter().any(|line| line.contains("denied.txt")),
            "{left:?}"
        );
        assert!(denied.exists());
        assert_eq!(remove_within(root, &tree).unwrap(), Vec::<String>::new());
    }

    /// Everyone denied deleting `file` and deleting children of `parent`
    /// (`set`), or those deny entries removed again.
    #[cfg(windows)]
    pub(crate) fn deny_delete(parent: &Path, file: &Path, set: bool) -> bool {
        let icacls = |path: &Path, right: &str| {
            let mut command = std::process::Command::new("icacls");
            command.arg(path);
            if set {
                command.args(["/deny", &format!("*S-1-1-0:({right})")]);
            } else {
                command.args(["/remove:d", "*S-1-1-0"]);
            }
            command.output().is_ok_and(|out| out.status.success())
        };
        icacls(file, "DE") & icacls(parent, "DC")
    }

    #[cfg(windows)]
    pub(crate) fn make_directory_link(target: &Path, link: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|out| out.status.success())
    }
    #[cfg(unix)]
    pub(crate) fn make_directory_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}
