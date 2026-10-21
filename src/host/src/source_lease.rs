//! Local regular-file lease, not a strong immutable snapshot.
//! Holding the open file preserves its object across rename/unlink/path replacement;
//! it does not prevent in-place writes. Length/mtime checks can miss same-length
//! rewrites with preserved timestamps and cannot defeat adversarial concurrent
//! writers. A worker must validate before AND after reading, discard failed output,
//! and report source-unavailable rather than reread a replacement path.
#![forbid(unsafe_code)]

use std::{
    fs::{File, Metadata},
    io::{Seek, SeekFrom},
    path::Path,
    process::Stdio,
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub len: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    facts: Facts,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl Identity {
    fn read(metadata: &Metadata) -> Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        if !metadata.is_file() {
            bail!("source lease requires a regular file");
        }
        Ok(Self {
            facts: Facts {
                len: metadata.len(),
                modified: metadata
                    .modified()
                    .context("source modification time is unavailable")?,
            },
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

pub struct Lease {
    file: Mutex<File>,
    accepted: Identity,
}

/// A single serial read interval. Keep this guard alive until every child using
/// stdin() has exited AND been waited for. try_clone shares the file cursor: do
/// not use two returned stdin handles concurrently, and do not reset the cursor
/// for another child before the first child has finished. This lifetime contract
/// is the caller's responsibility; Stdio itself cannot borrow this guard.
pub struct Reader<'a> {
    file: MutexGuard<'a, File>,
    accepted: &'a Identity,
}

impl Lease {
    pub fn open(path: &Path) -> Result<Self> {
        // Preliminary rejection avoids opening an ordinary directory or FIFO.
        // Recheck the held object as well; this is not a malicious path-race lock.
        if !std::fs::metadata(path)
            .context("source metadata is unavailable")?
            .is_file()
        {
            bail!("source lease requires a regular file");
        }
        let mut file = File::open(path).context("source file could not be opened")?;
        let accepted = Identity::read(
            &file
                .metadata()
                .context("held source metadata is unavailable")?,
        )?;
        file.seek(SeekFrom::Start(0))
            .context("source file is not seekable")?;
        Ok(Self {
            file: Mutex::new(file),
            accepted,
        })
    }

    /// Accepted size/mtime only; no saved user path or general filesystem RPC.
    pub fn facts(&self) -> Facts {
        self.accepted.facts
    }

    pub fn matches_file(&self, file: &File) -> Result<bool> {
        Ok(Identity::read(&file.metadata()?)? == self.accepted)
    }

    /// Nonblocking admission for cancellable offline workers. Never wait on the UI/control thread.
    pub fn try_reader(&self) -> Result<Option<Reader<'_>>> {
        match self.file.try_lock() {
            Ok(file) => Ok(Some(Reader {
                file,
                accepted: &self.accepted,
            })),
            Err(std::sync::TryLockError::WouldBlock) => Ok(None),
            Err(std::sync::TryLockError::Poisoned(_)) => bail!("source reader lock is poisoned"),
        }
    }

    pub fn lock_reader(&self) -> Result<Reader<'_>> {
        let file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("source reader lock is poisoned"))?;
        Ok(Reader {
            file,
            accepted: &self.accepted,
        })
    }
}

impl Reader<'_> {
    /// Duplicates the held object, never reopens its path. The clone shares the
    /// held cursor, so rewind happens inside the serial guard. This does not pick
    /// a platform-specific FFmpeg filename such as /dev/fd/0 or pipe:0.
    pub fn stdin(&mut self) -> Result<Stdio> {
        let mut file = self
            .file
            .try_clone()
            .context("held source handle could not be cloned")?;
        file.seek(SeekFrom::Start(0))
            .context("held source could not be rewound")?;
        Ok(Stdio::from(file))
    }

    /// Check the original held object only. Rename/unlink are allowed; normal
    /// in-place length/mtime changes fail. Matching metadata is limited evidence,
    /// not proof that bytes stayed unchanged during a concurrent read.
    pub fn validate(&self) -> Result<()> {
        let current = Identity::read(
            &self
                .file
                .metadata()
                .context("held source metadata is unavailable")?,
        )?;
        if current != *self.accepted {
            bail!("source-unavailable: held file metadata changed after lease acceptance");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "vestra-source-lease-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory creation failed: {error}"),
                }
            }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rename_and_path_replacement_still_read_the_original_held_object() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        let moved = temp.0.join("moved");
        fs::write(&source, b"original source").unwrap();
        let lease = Lease::open(&source).unwrap();
        fs::rename(&source, &moved).unwrap();
        fs::write(&source, b"replacement at old path").unwrap();
        #[cfg(unix)]
        fs::remove_file(&moved).unwrap();
        let mut reader = lease.lock_reader().unwrap();
        reader.validate().unwrap();
        // Exercise rewind through the public stdin adapter without running a
        // child; dropping this unused handle leaves the shared cursor at zero.
        reader.file.seek(SeekFrom::End(0)).unwrap();
        drop(reader.stdin().unwrap());
        let mut bytes = Vec::new();
        reader.file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original source");
        reader.validate().unwrap();
        assert_eq!(lease.facts().len, b"original source".len() as u64);
        assert_eq!(fs::read(&source).unwrap(), b"replacement at old path");
    }

    #[test]
    fn a_second_reader_waits_until_the_first_read_interval_ends() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        fs::write(&source, b"bytes").unwrap();
        let lease = Arc::new(Lease::open(&source).unwrap());
        let reader = lease.lock_reader().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let other = lease.clone();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let reader = other.lock_reader().unwrap();
            reader.validate().unwrap();
            acquired_tx.send(()).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            acquired_rx.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(reader);
        acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn in_place_length_change_is_detected_without_reopening_the_source() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        fs::write(&source, b"before").unwrap();
        let lease = Lease::open(&source).unwrap();
        let reader = lease.lock_reader().unwrap();
        reader.validate().unwrap();
        let mut writer = fs::OpenOptions::new().append(true).open(&source).unwrap();
        writer.write_all(b" changed").unwrap();
        writer.sync_all().unwrap();
        assert!(reader.validate().is_err());
        assert_eq!(lease.facts().len, 6);
    }

    #[test]
    fn directories_are_rejected_as_sources() {
        let temp = TempDir::new();
        assert!(Lease::open(&temp.0).is_err());
    }
}
