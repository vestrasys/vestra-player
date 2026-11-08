//! Host diagnostics: one line each on stderr, best effort.
//!
//! The release build is a GUI-subsystem program (`main.rs`), so its stderr
//! is whatever its launcher gave it, often a pipe whose reader may already
//! be gone. `eprintln!` panics when the write fails (a closed pipe: `EPIPE`,
//! on Windows `ERROR_NO_DATA`), and that panic aborts the host. `diag!`
//! writes the same line and drops a failed write; `clippy.toml` disallows
//! the std print macros in the host, so every diagnostic goes through here.
//! Plugin processes inherit the same stderr; the runners ignore failed
//! writes too (`src/runtime/runner.ts`, `shared-runner.ts`).
//!
//! The host log: a release build's stderr usually goes nowhere, so a few
//! lines also go to a small rolling file, as mpv's `--log-file` and
//! Chromium's `chrome_debug.log` do (doc/architecture/README.md §4.3):
//! `<app data>/logs/host.log`, at most [`LOG_CAP`] bytes, then renamed to
//! `host.log.1` (one older file kept). `logged!` writes a line to stderr
//! like `diag!` and to that file with the time and the process id; only the
//! lines the document lists use it (start and exit, the render mode and its
//! fallbacks and restarts, plugin shader changes, `host.error`, the memory
//! line), never paths of the user's media, links or secrets ([`redact`]).
//! Each line is one unbuffered write, so a process that dies loses nothing
//! written before; a panic writes its own line first ([`log_panics`]).
#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// `eprintln!` that never panics: a diagnostic that cannot be written is
/// dropped.
macro_rules! diag {
    ($($arg:tt)*) => {
        $crate::diag::line(format_args!($($arg)*))
    };
}

/// `diag!` that also goes to the host log (when one is open). Only for the
/// lines the host log keeps; nothing with a media path, link or secret.
macro_rules! logged {
    ($($arg:tt)*) => {
        $crate::diag::logged(format_args!($($arg)*))
    };
}

/// The host log's size before it rolls over to `host.log.1`.
pub const LOG_CAP: u64 = 512 * 1024;

/// A size-capped append-only log with one older file (`<name>.1`).
#[derive(Debug)]
pub struct LogFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    cap: u64,
    /// What `roll_over` said on stderr, each once per log (a log that cannot
    /// roll over fails again at every later attempt).
    reported: Reported,
}

/// A roll-over that went wrong.
enum Problem {
    /// The rename failed: writing goes on in the same file.
    Rename(String),
    /// The new file did not open: lines are dropped until it does.
    Reopen(String),
    /// The rename failed and the file reached twice its cap: the log stops.
    Full,
}

/// The roll-over problems said on stderr, and that the log opened again
/// after `reopen`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Reported {
    rename: bool,
    reopen: bool,
    reopened: bool,
    full: bool,
}

impl LogFile {
    /// Open (or create) `path` for appending; a file already at `cap` rolls
    /// over first.
    pub fn open(path: PathBuf, cap: u64) -> std::io::Result<Self> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata().map_or(0, |meta| meta.len());
        let mut log = Self {
            path,
            file: Some(file),
            size,
            cap,
            reported: Reported::default(),
        };
        if log.size >= cap {
            log.roll_over();
        }
        Ok(log)
    }
    /// `<path>.1`: the previous file.
    pub fn older(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }
    /// The file becomes `<path>.1` (replacing the older one) and a new one
    /// starts. Where the rename fails, writing goes on in the same file
    /// (never truncated: another player may be writing it), until it is
    /// twice the cap; then this log stops.
    ///
    /// Two players at once (a development run with `VESTRA_LOG_FILE`, a
    /// player that could not claim the single-instance lock): each has its
    /// own handle in append mode, so lines interleave but are not lost.
    /// Rust opens files with `FILE_SHARE_DELETE` on Windows, so one player
    /// can rename the file while the other still has it open; the other then
    /// keeps writing into `<path>.1` until its own next roll-over.
    ///
    /// Each kind of failure is said once on stderr (`reported`), and that
    /// the log opened again after a failed open; the player goes on either
    /// way.
    fn roll_over(&mut self) {
        let problem = self.try_roll_over();
        if self.file.is_some() && self.reported.reopen && !self.reported.reopened {
            self.reported.reopened = true;
            line(format_args!("host log: open again"));
        }
        let Some(problem) = problem else {
            return;
        };
        let said = match problem {
            Problem::Rename(_) => &mut self.reported.rename,
            Problem::Reopen(_) => &mut self.reported.reopen,
            Problem::Full => &mut self.reported.full,
        };
        if !*said {
            *said = true;
            match problem {
                Problem::Rename(error) => line(format_args!(
                    "host log: could not roll over to the older file ({error}); writing on in the same file"
                )),
                Problem::Reopen(error) => line(format_args!(
                    "host log: could not open the log again after rolling over ({error}); lines are dropped until it opens"
                )),
                Problem::Full => line(format_args!(
                    "host log: could not roll over and the file is at twice its cap; the log stops"
                )),
            }
        }
    }
    /// `roll_over` without the report: what went wrong, if anything (a
    /// failed open is tried again at the next line).
    fn try_roll_over(&mut self) -> Option<Problem> {
        // Closed first: our own handle need not stay on the renamed file.
        self.file = None;
        let renamed = std::fs::rename(&self.path, Self::older(&self.path));
        let mut problem = match renamed {
            // The file is gone (removed by hand): a new one starts.
            Err(ref error) if error.kind() != std::io::ErrorKind::NotFound => {
                Some(Problem::Rename(error.to_string()))
            }
            _ => None,
        };
        self.file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => Some(file),
            // `size` stays over the cap: the next line tries again.
            Err(error) => return Some(Problem::Reopen(error.to_string())),
        };
        self.size = 0;
        if renamed.is_err() {
            let size = self
                .file
                .as_ref()
                .and_then(|file| file.metadata().ok())
                .map_or(0, |meta| meta.len());
            if size >= self.cap.saturating_mul(2) {
                self.file = None;
                problem = Some(Problem::Full);
            }
        }
        problem
    }
    /// One line (a newline is added); a write that fails is dropped.
    pub fn write(&mut self, line: &str) {
        let bytes = line.len() as u64 + 1;
        if self.size > 0 && self.size + bytes > self.cap {
            self.roll_over();
        }
        if let Some(file) = &mut self.file
            && file.write_all(format!("{line}\n").as_bytes()).is_ok()
        {
            self.size += bytes;
        }
    }
}

static LOG: Mutex<Option<LogFile>> = Mutex::new(None);

/// Start the host log at `path` (the default: [`log_path`]).
pub fn open_log(path: PathBuf) -> std::io::Result<()> {
    let log = LogFile::open(path, LOG_CAP)?;
    *LOG.lock().unwrap_or_else(PoisonError::into_inner) = Some(log);
    Ok(())
}

/// Whether the host log is open (the memory line is only for it).
pub fn log_open() -> bool {
    LOG.lock().unwrap_or_else(PoisonError::into_inner).is_some()
}

/// The plugin log's size before it rolls over to `plugins.log.1`.
pub const PLUGIN_LOG_CAP: u64 = 2 * 1024 * 1024;

static PLUGIN_LOG: Mutex<Option<LogFile>> = Mutex::new(None);

/// The plugin log beside the host log: `plugins.log` in the same folder.
/// It holds what plugins write to their console (each line starts with
/// `[<plugin id>]`), as VS Code keeps an extension host log apart from its
/// main log. The plugins' own words are kept as they are; the host log's
/// rule against the user's paths applies to the player's lines, not here.
pub fn plugin_log_path(host_log: &Path) -> PathBuf {
    host_log.with_file_name("plugins.log")
}

/// Start the plugin log at `path` (`plugin_log_path`), capped at
/// [`PLUGIN_LOG_CAP`] with one older file.
pub fn open_plugin_log(path: PathBuf) -> std::io::Result<()> {
    let log = LogFile::open(path, PLUGIN_LOG_CAP)?;
    *PLUGIN_LOG.lock().unwrap_or_else(PoisonError::into_inner) = Some(log);
    Ok(())
}

/// One line a plugin wrote to its console (its stderr, read by the
/// player), for the plugin log with the time; `prefix` is put in front
/// unless the line has it already (the shared runner prefixes its own).
pub fn plugin_line(prefix: Option<&str>, bytes: &[u8]) {
    let mut log = PLUGIN_LOG.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(log) = log.as_mut() {
        log.write(&stamped(&plugin_text(prefix, bytes)));
    }
}

/// `bytes` as one log line: lossy UTF-8 without its line end, with
/// `prefix` (`[<plugin id>]`) in front unless it starts with it.
pub fn plugin_text(prefix: Option<&str>, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches(['\n', '\r']);
    match prefix {
        Some(prefix) if !text.starts_with(prefix) => format!("{prefix} {text}"),
        _ => text.to_owned(),
    }
}

/// Where the host log goes: `VESTRA_LOG_FILE` when set (empty: no log, in
/// any build), else `<app data>/logs/host.log` for a release build with
/// windows that loads no development plugins (`--plugin`: those runs keep
/// their stderr, and two players must not roll one file over each other).
pub fn log_path(
    env: Option<std::ffi::OsString>,
    release: bool,
    headless: bool,
    development: bool,
    app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    match env {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(PathBuf::from(value)),
        None => (release && !headless && !development)
            .then(|| app_data.map(|directory| directory.join("logs").join("host.log")))
            .flatten(),
    }
}

/// `diag!` plus the host log, with the time and process id there.
pub fn logged(args: std::fmt::Arguments<'_>) {
    let text = args.to_string();
    line(format_args!("{text}"));
    log_only(&text);
}

/// A line for the host log alone (one already went to stderr).
pub fn log_only(text: &str) {
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(log) = log.as_mut() {
        log.write(&stamped(text));
    }
}

/// `<UTC time> [pid] text`.
fn stamped(text: &str) -> String {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{} [{}] {text}",
        utc(since.as_secs(), since.subsec_millis()),
        std::process::id()
    )
}

/// `2026-11-01T13:45:07.250Z` (proleptic Gregorian, from Unix seconds).
pub fn utc(seconds: u64, millis: u32) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// A message for the host log without the user's paths, links or file
/// names: cut at the first word that has a path separator, a scheme, a
/// quote (error messages quote what they could not open) or a file
/// extension (`holiday.mkv`), and at 160 characters. A backstop only:
/// `host.error` messages are logged for known codes alone
/// (`host::diagnostics::host_error_line`).
pub fn redact(text: &str) -> String {
    let mut kept = Vec::new();
    let mut cut = false;
    for word in text.split_whitespace() {
        if word.contains(['/', '\\', '"', '`'])
            || word.starts_with('\'')
            || word.ends_with('\'')
            || has_extension(word)
        {
            cut = true;
            break;
        }
        kept.push(word);
    }
    let mut out: String = kept.join(" ");
    if out.chars().count() > 160 {
        out = out.chars().take(160).collect();
        cut = true;
    }
    if cut {
        out.push_str(" …");
    }
    out
}

/// Whether `word` looks like a file name: a stem, a dot and 1 to 5 letters
/// or digits with at least one letter (`a.mkv`, `x.mp4`), punctuation after
/// it aside; `0.5`, `1.18.1` and a sentence's full stop are not.
fn has_extension(word: &str) -> bool {
    let word = word.trim_end_matches([',', ';', ':', ')', ']', '!', '?']);
    let word = word.strip_suffix('.').unwrap_or(word);
    word.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (1..=5).contains(&extension.len())
            && extension.chars().all(|c| c.is_ascii_alphanumeric())
            && extension.chars().any(|c| c.is_ascii_alphabetic())
    })
}

/// An identifier a plugin or the worker chose (an error code, a shader
/// handle id) for the host log: ASCII letters, digits, `-` and `_`
/// only (no dots: no file names), at most `max` of them; `unknown` when
/// nothing is left.
pub fn token(text: &str, max: usize) -> String {
    let token: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(max)
        .collect();
    if token.is_empty() {
        "unknown".into()
    } else {
        token
    }
}

/// The memory line: this process's private bytes and the system commit
/// charge (`platform::memory_status`).
pub fn memory_line(status: &crate::platform::MemoryStatus) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: f64 = 1024. * 1024. * 1024.;
    let percent = (status.commit_used as f64 * 100. / status.commit_limit.max(1) as f64).round();
    let physical = status
        .physical_load
        .map(|load| format!("; physical memory {load}%"))
        .unwrap_or_default();
    format!(
        "memory: private {} MB; system commit {percent}% ({:.1} of {:.1} GB){physical}",
        status.private_bytes / MB,
        status.commit_used as f64 / GB,
        status.commit_limit as f64 / GB,
    )
}

/// A panic writes `panic: <thread> at <file:line>: <message>` to the host
/// log (redacted) before the default report. A panic while the log is being
/// written (the lock is held) writes nothing more.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_default();
        let thread = std::thread::current();
        let text = format!(
            "panic: {} at {at}: {}",
            thread.name().unwrap_or("unnamed thread"),
            redact(&payload)
        );
        if let Ok(mut log) = LOG.try_lock()
            && let Some(log) = log.as_mut()
        {
            log.write(&stamped(&text));
        }
        previous(info);
    }));
}

/// One line to stderr, ignoring write errors. Test builds go through
/// `eprintln!`, which the test harness captures.
pub fn line(args: std::fmt::Arguments<'_>) {
    #[cfg(test)]
    {
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!("{args}");
        }
    }
    #[cfg(not(test))]
    line_to(std::io::stderr().lock(), args);
}

/// One line to `out`, ignoring write errors.
pub fn line_to(mut out: impl Write, args: std::fmt::Arguments<'_>) {
    let _ = out.write_fmt(format_args!("{args}\n"));
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};
    use std::process::{Command, Stdio};

    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    /// The plugin log sits beside the host log; each line names its plugin
    /// once (the shared runner prefixes its members' lines itself).
    #[test]
    fn plugin_log_lines_name_their_plugin_once() {
        let host = Path::new("C:/data/Vestra/logs/host.log");
        assert_eq!(
            plugin_log_path(host),
            Path::new("C:/data/Vestra/logs/plugins.log")
        );
        assert_eq!(
            plugin_text(Some("[vestra.capture]"), b"export done\r\n"),
            "[vestra.capture] export done"
        );
        assert_eq!(
            plugin_text(Some("[vestra.capture]"), b"[vestra.capture] already\n"),
            "[vestra.capture] already"
        );
        assert_eq!(
            plugin_text(None, b"[vestra.marks] shared\n"),
            "[vestra.marks] shared"
        );
        assert_eq!(plugin_text(None, b"bad \xff byte"), "bad \u{fffd} byte");
        // Capped like the host log, with its own (larger) cap.
        let dir = tempfile::tempdir().unwrap();
        let path = plugin_log_path(&dir.path().join("logs").join("host.log"));
        let mut log = LogFile::open(path.clone(), 64).unwrap();
        for n in 0..10 {
            log.write(&plugin_text(Some("[p]"), format!("line {n}\n").as_bytes()));
        }
        assert!(LogFile::older(&path).exists());
        assert!(std::fs::metadata(&path).unwrap().len() <= 64);
        const { assert!(PLUGIN_LOG_CAP > LOG_CAP) };
    }

    #[test]
    fn a_failed_write_is_dropped() {
        line_to(Broken, format_args!("lost {}", 1));
        let mut out = Vec::new();
        line_to(&mut out, format_args!("kept {}", 2));
        assert_eq!(out, b"kept 2\n");
    }

    #[test]
    fn the_log_rolls_over_to_one_older_file_at_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("host.log");
        let mut log = LogFile::open(path.clone(), 100).unwrap();
        for n in 0..5 {
            log.write(&format!("line {n} {}", "x".repeat(20)));
        }
        let current = std::fs::read_to_string(&path).unwrap();
        let older = std::fs::read_to_string(LogFile::older(&path)).unwrap();
        assert!(current.len() <= 100 && older.len() <= 100, "{current:?}");
        assert!(
            older.starts_with("line 0") && older.contains("line 2"),
            "{older:?}"
        );
        assert!(current.starts_with("line 3") && current.ends_with("\n"));
        // Once more: the older file is replaced, never a `.2`.
        for n in 5..9 {
            log.write(&format!("line {n} {}", "x".repeat(20)));
        }
        let older = std::fs::read_to_string(LogFile::older(&path)).unwrap();
        assert!(older.starts_with("line 3"), "{older:?}");
        assert!(!dir.path().join("logs").join("host.log.2").exists());
        assert_eq!(log.reported, Reported::default(), "nothing went wrong");
        drop(log);
        // A file already at its cap rolls over when opened; a smaller one
        // is appended to.
        let mut log = LogFile::open(path.clone(), 10).unwrap();
        log.write("new");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        drop(log);
        let mut log = LogFile::open(path.clone(), 100).unwrap();
        log.write("more");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\nmore\n");
    }

    #[test]
    fn log_lines_carry_utc_time_and_the_process() {
        assert_eq!(utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(utc(1_793_540_707, 250), "2026-11-01T13:45:07.250Z");
        let line = stamped("start");
        assert!(
            line.ends_with(&format!(" [{}] start", std::process::id())),
            "{line}"
        );
    }

    /// No paths, links or quoted names of the user's media reach the file.
    #[test]
    fn redaction_cuts_at_the_first_path_link_or_quote() {
        assert_eq!(redact("decoder failed"), "decoder failed");
        assert_eq!(
            redact(r"cannot open C:\Users\me\My Movie.mkv: denied"),
            "cannot open …"
        );
        assert_eq!(redact("load https://example.com/a?b failed"), "load …");
        assert_eq!(redact("no such file 'holiday.mkv'"), "no such file …");
        assert_eq!(redact("no such file \"holiday.mkv\""), "no such file …");
        assert_eq!(
            redact("the player's renderer stopped"),
            "the player's renderer stopped"
        );
        assert_eq!(redact(&"word ".repeat(100)).chars().count(), 162);
        // Bare file names (review item 4).
        assert_eq!(redact("cannot open holiday.mkv"), "cannot open …");
        assert_eq!(redact("cannot open holiday.MP4: denied"), "cannot open …");
        assert_eq!(redact("failed on a.b.webm."), "failed on …");
        // Versions, numbers and full stops are not file names.
        assert_eq!(
            redact("mpv 0.40.0 needs 1.5 s. Retry."),
            "mpv 0.40.0 needs 1.5 s. Retry."
        );
    }

    #[test]
    fn tokens_keep_only_identifier_characters() {
        assert_eq!(token("source-load", 64), "source-load");
        assert_eq!(token("x y\nz", 64), "xyz");
        assert_eq!(token("holiday.mkv", 64), "holidaymkv", "no dots");
        assert_eq!(token("C:\\Users\\me", 64), "CUsersme");
        assert_eq!(token("abcdef", 3), "abc");
        assert_eq!(token("/ \\ ..", 64), "unknown");
    }

    /// A rename that fails (the file is held elsewhere without delete
    /// sharing) never truncates: lines go on, then stop at twice the cap.
    #[cfg(windows)]
    #[test]
    fn a_failed_roll_over_keeps_appending_and_never_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.log");
        let mut log = LogFile::open(path.clone(), 40).unwrap();
        // The older file is a directory: renaming over it fails.
        std::fs::create_dir(LogFile::older(&path)).unwrap();
        for n in 0..10 {
            log.write(&format!("line {n} xxxxxxxxxx"));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("line 0"), "{text:?}");
        assert!(text.len() as u64 <= 3 * 40, "{}", text.len());
        assert!(!text.contains("line 9"), "stopped past twice the cap");
        // Said on stderr: the failed roll-over (once, though every later
        // line tried again) and the stop.
        assert_eq!(
            log.reported,
            Reported {
                rename: true,
                full: true,
                ..Reported::default()
            }
        );
    }

    #[test]
    fn a_log_that_cannot_open_again_drops_lines_and_tries_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("host.log");
        // A full log whose folder is gone, with a file in its place: the
        // rename finds nothing and the open fails.
        std::fs::write(path.parent().unwrap(), b"").unwrap();
        let mut log = LogFile {
            path: path.clone(),
            file: None,
            size: 40,
            cap: 40,
            reported: Reported::default(),
        };
        log.write("line 1 xxxxxxxxxxxxxxxxxxxx");
        log.write("line 2 xxxxxxxxxxxxxxxxxxxx");
        assert_eq!(
            log.reported,
            Reported {
                reopen: true,
                ..Reported::default()
            }
        );
        assert!(log.file.is_none());
        // The folder is back: the next line opens a new file.
        std::fs::remove_file(path.parent().unwrap()).unwrap();
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        log.write("line 3");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line 3\n");
        assert!(log.reported.reopened, "said that it opened again");
    }

    #[test]
    fn the_log_goes_to_app_data_for_release_runs_with_windows_only() {
        let app = Some(PathBuf::from("/data/Vestra"));
        let default = Some(PathBuf::from("/data/Vestra/logs/host.log"));
        assert_eq!(log_path(None, true, false, false, app.clone()), default);
        assert_eq!(
            log_path(None, false, false, false, app.clone()),
            None,
            "debug"
        );
        assert_eq!(
            log_path(None, true, true, false, app.clone()),
            None,
            "headless"
        );
        assert_eq!(
            log_path(None, true, false, true, app.clone()),
            None,
            "--plugin"
        );
        assert_eq!(log_path(None, true, false, false, None), None);
        assert_eq!(
            log_path(Some("/tmp/h.log".into()), false, true, true, None),
            Some(PathBuf::from("/tmp/h.log"))
        );
        assert_eq!(log_path(Some("".into()), true, false, false, app), None);
    }

    #[test]
    fn the_memory_line_names_private_bytes_and_the_commit_charge() {
        let gb = 1024 * 1024 * 1024;
        let line = memory_line(&crate::platform::MemoryStatus {
            private_bytes: 812 * 1024 * 1024,
            commit_used: 23 * gb,
            commit_limit: 32 * gb,
            physical_load: Some(64),
        });
        assert_eq!(
            line,
            "memory: private 812 MB; system commit 72% (23.0 of 32.0 GB); physical memory 64%"
        );
        let line = memory_line(&crate::platform::MemoryStatus::default());
        assert!(line.contains("system commit 0%"), "{line}");
    }

    const CHILD: &str = "VESTRA_DIAG_TEST_CHILD";

    /// The other process of the test below: once its stdin closes (the
    /// parent closed the read end of its stderr first), it writes to
    /// stderr the way `mode` says and reports on stdout.
    #[test]
    fn child_writes_to_a_closed_stderr() {
        let Ok(mode) = std::env::var(CHILD) else {
            return;
        };
        let _ = io::stdin().read_to_end(&mut Vec::new());
        for n in 0..3 {
            if mode == "eprintln" {
                #[allow(clippy::disallowed_macros)]
                {
                    eprintln!("diagnostic {n}");
                }
            } else {
                line_to(io::stderr().lock(), format_args!("diagnostic {n}"));
            }
        }
        #[allow(clippy::disallowed_macros)]
        {
            println!("child-survived;");
        }
    }

    fn run_child(mode: &str) -> (bool, String) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diag::tests::child_writes_to_a_closed_stderr",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The reader goes away before anything is written.
        drop(child.stderr.take());
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    /// The release host's case: its stderr is a pipe whose reader closed.
    /// `eprintln!` panics there; `line_to` (what `diag!` uses) carries on.
    #[test]
    fn a_closed_stderr_pipe_does_not_stop_the_process() {
        let (ok, stdout) = run_child("diag");
        assert!(ok && stdout.contains("child-survived;"), "{stdout}");
        let (ok, stdout) = run_child("eprintln");
        assert!(
            !ok && !stdout.contains("child-survived;"),
            "eprintln! to a closed pipe was expected to panic: {stdout}"
        );
    }
}
