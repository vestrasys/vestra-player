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
//! demuxer cache for files on a network file system ([`on_network`]), which
//! libmpv turns on for remote files it opens by path. Reads are positioned
//! reads on the file's handle: they block no longer than the file system's
//! own reads, as libmpv's do when it opens the path itself (a stalled network
//! share stalls both alike). libmpv's close at stop, advance or quit waits
//! for a read in progress, as with a local file it reads by path; `stream_cb`
//! has no force-close.
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

/// The file is on a network file system (a share by UNC path or a mapped
/// drive, WebDAV, NFS…), where libmpv would turn its demuxer cache on
/// (`cache=auto` for remote files); it cannot tell through a stream
/// protocol. Asked of the open handle, as libmpv asks its own
/// (`vestra_win32::file_is_remote`), so it holds however the path was
/// written (a mapped drive, `subst`, a symbolic link or junction to a share).
#[cfg(windows)]
pub fn on_network(file: &File) -> bool {
    vestra_win32::file_is_remote(file)
}

/// Lets `mpv` play [`Registration`] urls (Windows; the tests on every system).
#[cfg_attr(not(windows), allow(dead_code))]
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

    #[cfg(windows)]
    #[test]
    fn local_files_are_not_on_the_network() {
        let (dir, _registration) = registered(b"x");
        assert!(!on_network(&File::open(dir.path().join("media")).unwrap()));
    }

    /// Real machine: a file opened through the local administrative share
    /// (SMB loopback) is on the network, written as a UNC path or through
    /// the `\\?\UNC\` prefix. Passes without the share.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs the local administrative share"]
    #[allow(clippy::disallowed_macros)] // a skip note for the terminal
    fn files_opened_through_a_share_are_on_the_network() {
        for path in [
            r"\\localhost\C$\Windows\win.ini",
            r"\\?\UNC\localhost\C$\Windows\win.ini",
        ] {
            let Ok(file) = File::open(path) else {
                eprintln!("skipped: no local administrative share on this machine");
                return;
            };
            assert!(on_network(&file), "{path}");
        }
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
