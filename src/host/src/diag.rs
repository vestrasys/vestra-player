//! Host diagnostics: one line each on stderr, best effort.
//!
//! The release build is a GUI-subsystem program (`main.rs`), so its stderr
//! is whatever its launcher gave it, often a pipe whose reader may already
//! be gone. `eprintln!` panics when the write fails (a closed pipe: `EPIPE`,
//! on Windows `ERROR_NO_DATA`), and that panic aborts the host. `diag!`
//! writes the same line and drops a failed write; `clippy.toml` disallows
//! the std print macros in the host, so every diagnostic goes through here.
//! Plugin processes inherit the same stderr; the runner ignores failed
//! writes too (`runtime/runner.ts`).
#![forbid(unsafe_code)]

use std::io::Write;

/// `eprintln!` that never panics: a diagnostic that cannot be written is
/// dropped.
macro_rules! diag {
    ($($arg:tt)*) => {
        $crate::diag::line(format_args!($($arg)*))
    };
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

    #[test]
    fn a_failed_write_is_dropped() {
        line_to(Broken, format_args!("lost {}", 1));
        let mut out = Vec::new();
        line_to(&mut out, format_args!("kept {}", 2));
        assert_eq!(out, b"kept 2\n");
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
