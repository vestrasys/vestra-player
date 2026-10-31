//! One player per user (`single_instance`). The first launch holds an
//! exclusive lock on `instance.lock` in the application data directory and
//! listens on a loopback TCP port; `instance.endpoint` (owner-only on Unix)
//! names the port and a random token. A later launch finds the lock taken,
//! sends its files with the token and exits; the first one opens them and
//! comes to the front. The lock is the operating system's (`File::try_lock`),
//! so it ends with the process, also after a crash: a stale endpoint file
//! never blocks a new first launch.
//!
//! The same plain-std code on every system, so its tests (two processes)
//! run everywhere. It matters most on Windows, where every "Open With" and
//! double-click starts a new process; on macOS LaunchServices already sends
//! the documents to the running app (`on_open_files`), and this covers a
//! second launch from a terminal.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK: &str = "instance.lock";
const ENDPOINT: &str = "instance.endpoint";
/// A handoff is one line of JSON; more than this is not one.
const MAX_MESSAGE: u64 = 1 << 20;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// What a launch became.
pub enum Launch {
    /// The first player: serve later launches (`Primary::serve`).
    Primary(Primary),
    /// Another player is running and has the files: exit.
    HandedOff,
}

pub struct Primary {
    /// Held for the life of the process.
    lock: File,
    listener: TcpListener,
    token: String,
}

/// Claim `directory` for this process, or hand `files` to the player that
/// holds it, waiting up to `wait` for one that is still starting. An error
/// (no lock support, an unwritable directory, a player that never answers)
/// means: run on your own.
pub fn single_instance(directory: &Path, files: &[PathBuf], wait: Duration) -> io::Result<Launch> {
    std::fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(LOCK))?;
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Primary::start(directory, lock).map(Launch::Primary),
            Err(TryLockError::Error(error)) => return Err(error),
            Err(TryLockError::WouldBlock) => (),
        }
        match hand_off(directory, files) {
            Ok(()) => return Ok(Launch::HandedOff),
            // Starting (no endpoint yet) or just quitting (the lock frees soon).
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// A player that replaces one which restarted itself (`--vestra-restart`):
/// wait up to `wait` for that player to release the lock, then claim it;
/// never hand files over (the old player is quitting). An error means the
/// old player did not end in time: the caller runs on its own, as after any
/// failed claim.
pub fn single_instance_restarted(directory: &Path, wait: Duration) -> io::Result<Launch> {
    std::fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(LOCK))?;
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Primary::start(directory, lock).map(Launch::Primary),
            Err(TryLockError::Error(error)) => return Err(error),
            Err(TryLockError::WouldBlock) if Instant::now() >= deadline => {
                return Err(io::Error::other("the previous player did not end"));
            }
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

impl Primary {
    fn start(directory: &Path, lock: File) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let port = listener.local_addr()?.port();
        // Written beside, then renamed: a later launch reads all of it or none.
        let partial = directory.join(format!("{ENDPOINT}.{}", std::process::id()));
        {
            let mut options = OpenOptions::new();
            options.create(true).truncate(true).write(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&partial)?;
            writeln!(file, "{port} {token}")?;
            file.sync_all()?;
        }
        std::fs::rename(&partial, directory.join(ENDPOINT))?;
        Ok(Self {
            lock,
            listener,
            token,
        })
    }

    /// Answer later launches on a thread of its own for the rest of the
    /// process: `deliver` gets each one's files (maybe none: it only wants
    /// the player in front) and says whether it took them. A launch it
    /// refuses (a player in its last moments, its event loop ended) gets no
    /// acknowledgement, so it keeps trying until this process exits and
    /// frees the lock, and becomes the next player itself.
    pub fn serve(self, deliver: impl Fn(Vec<PathBuf>) -> bool + Send + 'static) {
        let Self {
            lock,
            listener,
            token,
        } = self;
        let spawned = std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                let _lock = lock;
                for stream in listener.incoming() {
                    match stream.and_then(|stream| receive(stream, &token, &deliver)) {
                        Ok(()) => (),
                        Err(error) => diag!("single instance: {error}"),
                    }
                }
            });
        if let Err(error) = spawned {
            diag!("single instance: {error}");
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Message {
    token: String,
    files: Vec<PathBuf>,
}

/// One later launch: its files go to `deliver`, and it is acknowledged when
/// `deliver` took them. A connection without the token is ignored.
fn receive(
    stream: TcpStream,
    token: &str,
    deliver: &impl Fn(Vec<PathBuf>) -> bool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_MESSAGE)).read_line(&mut line)?;
    let Ok(message) = serde_json::from_str::<Message>(&line) else {
        return Ok(());
    };
    if message.token != token || !deliver(message.files) {
        return Ok(());
    }
    (&stream).write_all(b"ok\n")
}

fn hand_off(directory: &Path, files: &[PathBuf]) -> io::Result<()> {
    let endpoint = std::fs::read_to_string(directory.join(ENDPOINT))?;
    let (port, token) = endpoint
        .trim()
        .split_once(' ')
        .and_then(|(port, token)| Some((port.parse::<u16>().ok()?, token)))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed endpoint"))?;
    // Windows lets only the process the user just started take the foreground.
    super::allow_handoff_foreground();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let message = Message {
        token: token.into(),
        files: files.to_vec(),
    };
    let mut line = serde_json::to_string(&message).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = [0; 3];
    stream.read_exact(&mut answer)?;
    if &answer != b"ok\n" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no acknowledgement",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const CHILD_DIR: &str = "VESTRA_INSTANCE_TEST_DIR";

    fn primary(directory: &Path) -> mpsc::Receiver<Vec<PathBuf>> {
        let Ok(Launch::Primary(primary)) = single_instance(directory, &[], Duration::ZERO) else {
            panic!("the first launch is the primary");
        };
        let (send, receive) = mpsc::channel();
        primary.serve(move |files| send.send(files).is_ok());
        receive
    }

    /// A player whose event loop has ended refuses handoffs: the launch is
    /// not acknowledged and does not count as handed off (it becomes the
    /// next player once that one exits; here the lock stays held, so it
    /// runs on its own after the wait).
    #[test]
    fn a_refused_handoff_is_not_handed_off() {
        let directory = tempfile::tempdir().unwrap();
        let Ok(Launch::Primary(primary)) = single_instance(directory.path(), &[], Duration::ZERO)
        else {
            panic!("the first launch is the primary");
        };
        let (send, offered) = mpsc::channel();
        primary.serve(move |files| {
            let _ = send.send(files);
            false
        });
        let files = [PathBuf::from("/m/late.mkv")];
        let launch = single_instance(directory.path(), &files, Duration::from_millis(300));
        assert!(launch.is_err(), "refused, not handed off");
        assert_eq!(offered.recv_timeout(IO_TIMEOUT).unwrap(), files);
    }

    /// A restarted player waits for the lock and never hands off; once the
    /// old player is gone it is the primary.
    #[test]
    fn a_restarted_launch_waits_for_the_lock_instead_of_handing_off() {
        let directory = tempfile::tempdir().unwrap();
        let old = single_instance(directory.path(), &[], Duration::ZERO).unwrap();
        assert!(matches!(old, Launch::Primary(_)));
        let refused = single_instance_restarted(directory.path(), Duration::from_millis(100));
        assert!(refused.is_err(), "the old player still holds the lock");
        drop(old);
        let claimed = single_instance_restarted(directory.path(), Duration::from_secs(5)).unwrap();
        assert!(matches!(claimed, Launch::Primary(_)));
    }

    #[test]
    fn a_second_launch_in_this_process_hands_off() {
        let directory = tempfile::tempdir().unwrap();
        let received = primary(directory.path());
        let files = [PathBuf::from("/m/a.mkv"), PathBuf::from("/m/b c.mp4")];
        let launch = single_instance(directory.path(), &files, Duration::from_secs(5)).unwrap();
        assert!(matches!(launch, Launch::HandedOff));
        assert_eq!(received.recv_timeout(IO_TIMEOUT).unwrap(), files);
        // Without the token nothing arrives; the server keeps serving.
        let endpoint = std::fs::read_to_string(directory.path().join(ENDPOINT)).unwrap();
        let port: u16 = endpoint.split(' ').next().unwrap().parse().unwrap();
        let mut stranger = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stranger
            .write_all(b"{\"token\":\"nope\",\"files\":[\"/x\"]}\n")
            .unwrap();
        drop(stranger);
        assert!(matches!(
            single_instance(directory.path(), &[], Duration::from_secs(5)).unwrap(),
            Launch::HandedOff
        ));
        assert_eq!(
            received.recv_timeout(IO_TIMEOUT).unwrap(),
            Vec::<PathBuf>::new()
        );
        assert!(received.try_recv().is_err());
    }

    /// The other process of the test below: a second launch.
    #[test]
    #[allow(clippy::disallowed_macros)] // its report to the parent goes on stdout
    fn child_second_launch() {
        let Some(directory) = std::env::var_os(CHILD_DIR) else {
            return;
        };
        let files = [PathBuf::from("/m/from-child.mkv")];
        match single_instance(Path::new(&directory), &files, Duration::from_secs(10)) {
            Ok(Launch::HandedOff) => println!("launch=handed-off;"),
            Ok(Launch::Primary(_)) => println!("launch=primary;"),
            Err(error) => println!("launch=error {error};"),
        }
    }

    #[test]
    fn a_second_process_hands_off_and_a_stale_endpoint_does_not_block() {
        let directory = tempfile::tempdir().unwrap();
        let child = || {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::instance::tests::child_second_launch",
                    "--nocapture",
                ])
                .env(CHILD_DIR, directory.path())
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        // Nobody holds the lock (only a stale endpoint file): the child is first.
        std::fs::write(directory.path().join(ENDPOINT), "1 stale").unwrap();
        let output = child();
        assert!(output.contains("launch=primary;"), "{output}");
        let received = primary(directory.path());
        let output = child();
        assert!(output.contains("launch=handed-off;"), "{output}");
        assert_eq!(
            received.recv_timeout(IO_TIMEOUT).unwrap(),
            [PathBuf::from("/m/from-child.mkv")]
        );
    }
}
