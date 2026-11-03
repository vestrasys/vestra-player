//! Checkpoints (doc/architecture/supervisor.md §4; formerly `lazy.rs`'s
//! recordings): what a unit in standby registered, so the next start
//! replays it instead of starting the plugin. The core decides
//! when one is written (`Action::Checkpoint`) and what it says (resident or
//! not); the adapter writes and reads the file.
//!
//! A standby's recording is written on the `plugin-recorder` thread
//! (`Writer`, supervisor.md): a write took up to 8 ms on the UI thread
//! in a 200-wake storm. The thread writes in order and answers each
//! write with `Event::Recorded`; the standby (or the classification) it was
//! written ahead for is decided then, as it was right after the write. Every
//! other read, write or removal of a recording first waits until the
//! thread wrote what it was given (`Writer::flush`): what was written before
//! is what is read back, a removal is never undone by an older write, and
//! the quit writes its checkpoints after them.
use super::probe;
use crate::capabilities::Call;
use crate::host::Event;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

/// The recording format.
pub(crate) const SCHEMA: u32 = 1;

/// What a recording is valid for: the installed instance, the package digest
/// and version; the player version and build are checked too (a new player
/// records again, the safe side).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub guid: String,
    pub digest: String,
    pub version: Option<String>,
}

/// `<package store>/activation/<instance GUID>.json`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Recording {
    pub schema: u32,
    pub player: String,
    #[serde(default)]
    pub build: Option<String>,
    pub digest: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Whether it held a live resource at its last `runtime.ready`: the next
    /// start launches it instead of replaying `calls`.
    pub resident_at_ready: bool,
    pub recorded_at: u64,
    pub calls: Vec<Call>,
    /// Written under `VESTRA_TEST_PARK=all` (`probe.rs`): a resident plugin
    /// recorded as lazy, without what it holds live. Only a run with the
    /// same switch reads it back; any other run ignores it as stale.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub test_park: bool,
}
impl Recording {
    fn new(key: &Key, calls: Vec<Call>, resident_at_ready: bool) -> Self {
        let recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Self {
            schema: SCHEMA,
            player: env!("CARGO_PKG_VERSION").into(),
            build: option_env!("VESTRA_BUILD").map(str::to_owned),
            digest: key.digest.clone(),
            version: key.version.clone(),
            resident_at_ready,
            recorded_at,
            calls,
            test_park: probe::park_all(),
        }
    }
    /// Valid for this package and this player.
    fn matches(&self, key: &Key) -> bool {
        self.schema == SCHEMA
            && self.player == env!("CARGO_PKG_VERSION")
            && self.build.as_deref() == option_env!("VESTRA_BUILD")
            && self.digest == key.digest
            && self.version == key.version
            && self.test_park == probe::park_all()
    }
}

fn file(directory: &Path, guid: &str) -> PathBuf {
    directory.join(format!("{guid}.json"))
}

/// The recording of `key`'s instance, if there is a valid one; a stale or
/// corrupt one is ignored (the plugin starts as before).
pub(crate) fn read(directory: &Path, key: &Key) -> Option<Recording> {
    let path = file(directory, &key.guid);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<Recording>(&bytes) {
        Ok(recording) if recording.matches(key) => Some(recording),
        Ok(_) => {
            diag!("lazy: {} is of another package or player", path.display());
            None
        }
        Err(error) => {
            diag!("lazy: {}: {error}", path.display());
            None
        }
    }
}

/// Writes `key`'s recording (a temporary file renamed over the old one).
pub(crate) fn write(
    directory: &Path,
    key: &Key,
    calls: Vec<Call>,
    resident_at_ready: bool,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = file(directory, &key.guid);
    let temporary = path.with_extension("json.tmp");
    let recording = Recording::new(key, calls, resident_at_ready);
    std::fs::write(&temporary, serde_json::to_vec(&recording)?)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

/// One recording for the writer thread (`Writer::write`).
struct Job {
    directory: PathBuf,
    key: Key,
    calls: Vec<Call>,
    resident: bool,
    /// The run it is written for and the write's serial
    /// (`Event::Recorded`).
    owner: u64,
    serial: u64,
}

/// Writes done by the thread, and the condition `flush` waits on.
type Done = Arc<(Mutex<u64>, Condvar)>;

/// The Host's handle on the `plugin-recorder` thread.
pub(crate) struct Writer {
    jobs: mpsc::Sender<Job>,
    /// Writes given to the thread (the last serial).
    sent: u64,
    done: Done,
}
impl Writer {
    /// Starts the thread (`None`: it could not start; the Host then writes
    /// on its own thread, as before).
    pub(crate) fn start(events: mpsc::Sender<Event>) -> Option<Self> {
        let (jobs, received) = mpsc::channel::<Job>();
        let done: Done = Arc::default();
        let finished = done.clone();
        let spawned = std::thread::Builder::new()
            .name("plugin-recorder".into())
            .spawn(move || {
                serve(received, &finished, |job| {
                    let calls = job.calls;
                    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        write(&job.directory, &job.key, calls, job.resident)
                    }));
                    let result = match written {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(error)) => Err(format!("{error:#}")),
                        Err(_) => Err("its recording could not be written (panicked)".into()),
                    };
                    let _ = events.send(Event::Recorded(job.owner, job.serial, result));
                });
            });
        match spawned {
            Ok(_) => Some(Self {
                jobs,
                sent: 0,
                done,
            }),
            Err(error) => {
                diag!("plugin recorder: {error}");
                None
            }
        }
    }
    /// Queues `key`'s recording for run `owner`: its serial, answered by
    /// `Event::Recorded(owner, serial, result)`. The calls back when the
    /// thread is gone (nothing was queued).
    pub(crate) fn write(
        &mut self,
        directory: PathBuf,
        key: Key,
        calls: Vec<Call>,
        resident: bool,
        owner: u64,
    ) -> std::result::Result<u64, Vec<Call>> {
        let serial = self.sent + 1;
        let job = Job {
            directory,
            key,
            calls,
            resident,
            owner,
            serial,
        };
        self.jobs
            .send(job)
            .map_err(|mpsc::SendError(job)| job.calls)?;
        self.sent = serial;
        Ok(serial)
    }
    /// Waits until every write queued so far is on disk (or failed), at
    /// most `budget`: whether they all are.
    pub(crate) fn flush(&self, budget: Duration) -> bool {
        let (count, all) = &*self.done;
        let deadline = Instant::now() + budget;
        let mut done = count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *done < self.sent {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            done = all
                .wait_timeout(done, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// The thread's loop: each job in order, then counted as done (`flush`).
fn serve<J>(jobs: mpsc::Receiver<J>, done: &Done, mut run: impl FnMut(J)) {
    for job in jobs {
        run(job);
        let (count, all) = &**done;
        *count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        all.notify_all();
    }
}

/// Removes `guid`'s recording (uninstall, reload, upgrade).
pub(crate) fn remove(directory: &Path, guid: &str) {
    let path = file(directory, guid);
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        diag!("lazy: {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn key() -> Key {
        Key {
            guid: "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41".into(),
            digest: "abc".into(),
            version: Some("1.0.0".into()),
        }
    }

    #[test]
    fn a_recording_is_read_back_only_for_its_package_and_player() {
        let directory = tempfile::tempdir().unwrap();
        let calls = vec![Call::new(
            "commands.register",
            json!({"id":"hello","revision":1,"declaration":{"title":"Hello"},"state":{"enabled":true}}),
        )];
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        let read_back = read(directory.path(), &key()).unwrap();
        assert_eq!(read_back.calls, calls);
        assert!(!read_back.resident_at_ready);
        // Resident at its ready: read back as such (the start launches it).
        write(directory.path(), &key(), Vec::new(), true).unwrap();
        let resident = read(directory.path(), &key()).unwrap();
        assert!(resident.resident_at_ready && resident.calls.is_empty());
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        // Another digest, version or instance: not this package's.
        for other in [
            Key {
                digest: "other".into(),
                ..key()
            },
            Key {
                version: Some("1.0.1".into()),
                ..key()
            },
        ] {
            assert!(read(directory.path(), &other).is_none());
        }
        let path = file(directory.path(), &key().guid);
        let mut json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // A normal run writes no test mark and ignores a recording made
        // under `VESTRA_TEST_PARK=all` (incomplete for a resident plugin).
        assert!(json.get("testPark").is_none(), "{json}");
        json["testPark"] = true.into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_none());
        json["testPark"] = false.into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_some());
        // Another player release records again.
        json["player"] = "0.0.1".into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_none());
        // Corrupt: ignored.
        std::fs::write(&path, "{").unwrap();
        assert!(read(directory.path(), &key()).is_none());
        remove(directory.path(), &key().guid);
        assert!(!path.exists());
        remove(directory.path(), &key().guid);
    }

    /// The writer thread writes in order, answers each write with its
    /// serial, and `flush` returns once everything queued is on disk: the
    /// last of several writes of one recording is what is read back.
    #[test]
    fn the_writer_writes_in_order_and_flush_waits_for_it() {
        let directory = tempfile::tempdir().unwrap();
        let (events, received) = mpsc::channel();
        let mut writer = Writer::start(events).unwrap();
        assert!(writer.flush(Duration::ZERO), "nothing queued");
        let call = |n: u64| Call::new("commands.register", json!({"id":format!("c{n}")}));
        let mut serials = Vec::new();
        for n in 1..=20 {
            let calls = (1..=n).map(call).collect();
            serials.push(
                writer
                    .write(directory.path().into(), key(), calls, false, 7)
                    .unwrap(),
            );
        }
        assert_eq!(serials, (1..=20).collect::<Vec<_>>());
        assert!(writer.flush(Duration::from_secs(10)));
        assert_eq!(read(directory.path(), &key()).unwrap().calls.len(), 20);
        let answered = received
            .try_iter()
            .map(|event| match event {
                Event::Recorded(owner, serial, result) => (owner, serial, result.is_ok()),
                _ => panic!("another event"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            answered,
            (1..=20).map(|serial| (7, serial, true)).collect::<Vec<_>>()
        );
        // A write that fails (the directory is a file) is answered as such.
        let file = directory.path().join("not-a-directory");
        std::fs::write(&file, "x").unwrap();
        let serial = writer.write(file, key(), Vec::new(), false, 8).unwrap();
        assert!(writer.flush(Duration::from_secs(10)));
        assert!(matches!(
            received.recv().unwrap(),
            Event::Recorded(8, s, Err(_)) if s == serial
        ));
    }

    /// `flush` gives up at its budget while a write is still going.
    #[test]
    fn flush_gives_up_at_its_budget() {
        let (jobs, received) = mpsc::channel::<u64>();
        let done: Done = Arc::default();
        let (gate, open) = mpsc::channel::<()>();
        let finished = done.clone();
        let thread = std::thread::spawn(move || {
            serve(received, &finished, |_| {
                let _ = open.recv();
            })
        });
        jobs.send(1).unwrap();
        let writer = Writer {
            jobs: mpsc::channel().0,
            sent: 1,
            done: done.clone(),
        };
        assert!(!writer.flush(Duration::from_millis(50)));
        gate.send(()).unwrap();
        assert!(writer.flush(Duration::from_secs(10)));
        drop(jobs);
        thread.join().unwrap();
    }
}
