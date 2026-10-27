//! Windows: libmpv reads a leased local file through `vestra-lease://<id>`
//! (libmpv `stream_cb`), the counterpart of Unix `fd://`. A file descriptor
//! cannot cross into libmpv's C runtime on Windows, and a path would let
//! libmpv open another object than the lease holds. The registered `File` is
//! opened by the host next to the lease and verified to be the same object
//! (`Lease::matches_file`); it shares read, write and delete access like
//! libmpv's own handles, so a playing file can still be renamed or deleted.
//!
//! As with `fd://`, libmpv sees no path: nothing it derives from one applies
//! (relative references next to the file, mkv ordered chapters/editions in
//! other files, which the forced lavf demuxer would not follow either;
//! `sub-auto` is off anyway). The host gives what matters itself
//! (`source_backend::leased_options`): cover art beside audio files, and the
//! demuxer cache for files on a network share ([`on_network`]), which libmpv
//! turns on for remote files it opens by path. Reads are local positioned reads
//! that do not block indefinitely, so libmpv's close at stop, advance or quit
//! does not need a force-close, which `stream_cb` does not have.
#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fs::File,
    io,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use rsmpv::stream_cb::Stream;

pub const PROTOCOL: &str = "vestra-lease";

static FILES: Mutex<BTreeMap<u64, Arc<File>>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn files() -> MutexGuard<'static, BTreeMap<u64, Arc<File>>> {
    FILES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A playback file libmpv can open as [`Registration::url`] until this drops.
/// A stream libmpv already opened keeps reading its file after that.
pub struct Registration {
    id: u64,
}

impl Registration {
    pub fn new(file: File) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        files().insert(id, Arc::new(file));
        Self { id }
    }
    pub fn url(&self) -> String {
        format!("{PROTOCOL}://{}", self.id)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        files().remove(&self.id);
    }
}

/// One libmpv stream over a registered file: its own position and
/// positioned reads, so streams opened on the same file (a restart while the
/// old load closes) do not move each other's position.
pub struct FileStream {
    file: Arc<File>,
    position: u64,
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}
#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

impl Stream for FileStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = read_at(&self.file, buf, self.position)?;
        self.position += read as u64;
        Ok(read)
    }
    fn seek(&mut self, offset: u64) -> io::Result<u64> {
        self.position = offset;
        Ok(offset)
    }
    fn size(&mut self) -> Option<u64> {
        self.file.metadata().ok().map(|metadata| metadata.len())
    }
}

/// libmpv's open callback for [`PROTOCOL`] (called on its demuxer thread).
pub fn open(uri: &str) -> Result<Box<dyn Stream>, rsmpv::Error> {
    uri.strip_prefix(PROTOCOL)
        .and_then(|rest| rest.strip_prefix("://"))
        .and_then(|id| id.parse::<u64>().ok())
        .and_then(|id| files().get(&id).cloned())
        .map(|file| Box::new(FileStream { file, position: 0 }) as Box<dyn Stream>)
        .ok_or(rsmpv::Error::LoadingFailed)
}

/// Where a Windows path lives, as far as its text tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Location {
    /// A UNC path (`\\server\share\…`, `\\?\UNC\server\share\…`).
    Share,
    /// On drive letter (upper case).
    Drive(char),
    Other,
}

fn location(path: &str) -> Location {
    let path = path.replace('/', "\\");
    let drive = |rest: &str| {
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => {
                Location::Drive(letter.to_ascii_uppercase())
            }
            _ => Location::Other,
        }
    };
    if let Some(rest) = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
    {
        return match rest.get(..4) {
            Some(unc) if unc.eq_ignore_ascii_case(r"UNC\") => Location::Share,
            _ => drive(rest),
        };
    }
    if path.starts_with(r"\\") {
        return Location::Share;
    }
    drive(&path)
}

/// The file is on a network share or a mapped network drive, where libmpv
/// would turn its demuxer cache on (`cache=auto` for remote files); it cannot
/// tell through a stream protocol.
#[cfg(windows)]
pub fn on_network(path: &str) -> bool {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.into());
    match location(&path.to_string_lossy()) {
        Location::Share => true,
        Location::Drive(letter) => vestra_win32::drive_is_remote(letter),
        Location::Other => false,
    }
}

/// Lets `mpv` play [`Registration`] urls.
pub fn register(mpv: &rsmpv::Mpv) -> rsmpv::Result<()> {
    mpv.register_protocol(PROTOCOL, open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(bytes: &[u8]) -> (tempfile::TempDir, Registration) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("media");
        std::fs::write(&path, bytes).unwrap();
        let registration = Registration::new(File::open(&path).unwrap());
        (dir, registration)
    }

    #[test]
    fn a_stream_reads_seeks_and_sizes_its_registered_file() {
        let (_dir, registration) = registered(b"0123456789");
        let mut stream = open(&registration.url()).unwrap();
        assert_eq!(stream.size(), Some(10));
        let mut buf = [0; 4];
        assert_eq!(stream.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"0123");
        assert_eq!(stream.seek(8).unwrap(), 8);
        assert_eq!(stream.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"89");
        assert_eq!(stream.read(&mut buf).unwrap(), 0, "end of file");
        assert_eq!(stream.seek(20).unwrap(), 20);
        assert_eq!(stream.read(&mut buf).unwrap(), 0, "past the end");
        // A second stream on the same file has its own position.
        let mut other = open(&registration.url()).unwrap();
        stream.seek(5).unwrap();
        assert_eq!(other.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"0123");
        assert_eq!(stream.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"5678");
    }

    #[test]
    fn shares_and_drive_letters_are_told_apart() {
        for (path, expected) in [
            (r"\\server\share\a.mp4", Location::Share),
            ("//server/share/a.mp4", Location::Share),
            (r"\\?\UNC\server\share\a.mp4", Location::Share),
            (r"\\?\unc\server\share\a.mp4", Location::Share),
            (r"C:\Videos\a.mp4", Location::Drive('C')),
            ("z:/a.mp4", Location::Drive('Z')),
            (r"\\?\D:\a.mp4", Location::Drive('D')),
            (r"\\.\E:\a.mp4", Location::Drive('E')),
            ("a.mp4", Location::Other),
            (r"\a.mp4", Location::Other),
            ("", Location::Other),
        ] {
            assert_eq!(location(path), expected, "{path}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn local_files_are_not_on_the_network() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!on_network(&dir.path().join("a.mp4").to_string_lossy()));
        assert!(on_network(r"\\server\share\a.mp4"));
        assert!(!on_network("a.mp4"), "relative: the current drive");
    }

    #[test]
    fn only_live_registrations_open() {
        let (_dir, registration) = registered(b"x");
        let url = registration.url();
        let mut opened = open(&url).unwrap();
        for other in [
            "vestra-lease://",
            "vestra-lease://x",
            "vestra-lease:/1",
            "other://1",
            "vestra-lease://18446744073709551615",
        ] {
            assert!(open(other).is_err(), "{other}");
        }
        drop(registration);
        assert!(open(&url).is_err(), "dropped");
        // A stream opened before keeps reading.
        let mut buf = [0; 1];
        assert_eq!(opened.read(&mut buf).unwrap(), 1);
    }
}
