//! The command line: development and diagnostic flags, and the files to open.
//!
//! Positional arguments are files (or folders) to open, the way the system
//! passes them for "Open With" and file associations (Windows puts the path
//! on the command line; macOS sends an open-documents event instead, see
//! `platform::on_open_files`). `file://` URLs become paths. They open like
//! a drop of the same paths on the video area (`drops.drop`): the first one
//! plays, all of them go to the list of the plugin that takes their kind.
//! After `--` every argument is a file, so a file named `--plugin` opens.
#![forbid(unsafe_code)]
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq)]
pub struct Launch {
    pub directories: Vec<PathBuf>,
    pub headless: bool,
    pub silent: bool,
    pub seconds: Option<f64>,
    pub snapshot: Option<PathBuf>,
    pub frame_evidence: Option<PathBuf>,
    /// Files to open, absolute, in command-line order.
    pub files: Vec<PathBuf>,
    /// `http(s)` and other non-file URLs: plugins open those through their
    /// own commands; a drop carries only paths (reported and skipped).
    pub urls: Vec<String>,
}

const USAGE: &str = "use [FILE…] [--plugin DIR] [--headless] [--silent] [--seconds N]";

/// Parse `args` (without the program name); relative paths resolve against `cwd`.
pub fn parse(args: &[String], cwd: &Path) -> Result<Launch> {
    let mut launch = Launch::default();
    let mut args = args.iter();
    let mut only_files = false;
    while let Some(arg) = args.next() {
        if only_files || !arg.starts_with('-') {
            launch.add(arg, cwd);
            continue;
        }
        let mut value = |what: &str| {
            args.next()
                .ok_or_else(|| anyhow!("{arg} requires {what}"))
                .cloned()
        };
        match arg.as_str() {
            "--" => only_files = true,
            "--frame-evidence-dir" => {
                launch.frame_evidence = Some(PathBuf::from(value("path")?));
            }
            "--snapshot" => launch.snapshot = Some(PathBuf::from(value("PNG path")?)),
            "--headless" => launch.headless = true,
            "--silent" => launch.silent = true,
            "--plugin" => launch.directories.push(PathBuf::from(value("directory")?)),
            "--seconds" => launch.seconds = Some(value("number")?.parse::<f64>()?),
            // Old macOS versions pass the process serial number to bundled
            // apps started from the Finder.
            _ if arg.starts_with("-psn_") => (),
            _ => bail!("unknown argument {arg}; {USAGE}"),
        }
    }
    if launch.frame_evidence.is_some() && !launch.headless {
        bail!("--frame-evidence-dir is headless diagnostic only");
    }
    Ok(launch)
}

impl Launch {
    fn add(&mut self, arg: &str, cwd: &Path) {
        match url::Url::parse(arg) {
            // A one-letter scheme is a Windows drive (`C:\…`), not a URL.
            Ok(url) if url.scheme().len() > 1 => match (url.scheme(), url.to_file_path()) {
                ("file", Ok(path)) => self.files.push(path),
                _ => self.urls.push(arg.into()),
            },
            _ => self.files.push(absolute(Path::new(arg), cwd)),
        }
    }
}

/// `path` against `cwd`, without touching the file system (a missing file
/// is the opening plugin's error to report).
pub fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.into()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    fn cwd() -> PathBuf {
        std::env::temp_dir().join("launch-cwd")
    }

    #[test]
    fn flags_keep_working() {
        let launch = parse(
            &args(&[
                "--plugin",
                "a",
                "--headless",
                "--silent",
                "--seconds",
                "2.5",
                "--plugin",
                "b",
            ]),
            &cwd(),
        )
        .unwrap();
        assert_eq!(launch.directories, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(launch.headless && launch.silent);
        assert_eq!(launch.seconds, Some(2.5));
        assert!(launch.files.is_empty());
        for bad in [
            &["--plugin"][..],
            &["--seconds", "x"],
            &["--bogus"],
            &["--frame-evidence-dir", "d"],
        ] {
            assert!(parse(&args(bad), &cwd()).is_err(), "{bad:?}");
        }
        let message = parse(&args(&["--bogus"]), &cwd()).unwrap_err().to_string();
        assert!(message.contains("unknown argument --bogus"), "{message}");
    }

    #[test]
    fn positional_arguments_are_files_in_order() {
        let base = cwd();
        let absolute = base.join("abs.mkv");
        let launch = parse(
            &args(&[
                "movie.mp4",
                "--headless",
                absolute.to_str().unwrap(),
                "sub/song.flac",
                "-psn_0_12345",
            ]),
            &base,
        )
        .unwrap();
        assert!(launch.headless);
        assert_eq!(
            launch.files,
            [base.join("movie.mp4"), absolute, base.join("sub/song.flac")]
        );
        // After `--` everything is a file.
        let launch = parse(&args(&["--", "--plugin", "-x.mp4"]), &base).unwrap();
        assert!(launch.directories.is_empty());
        assert_eq!(launch.files, [base.join("--plugin"), base.join("-x.mp4")]);
    }

    #[test]
    fn urls_become_paths_or_are_kept_apart() {
        let base = cwd();
        let file = url::Url::from_file_path(base.join("a b.mp4")).unwrap();
        let launch = parse(
            &args(&[file.as_str(), "https://example.com/v.mp4", "C:movie.mp4"]),
            &base,
        )
        .unwrap();
        assert_eq!(launch.files[0], base.join("a b.mp4"));
        assert_eq!(launch.urls, ["https://example.com/v.mp4"]);
        // A drive letter is not a URL scheme.
        assert_eq!(launch.files.len(), 2);
        assert!(launch.files[1].to_string_lossy().ends_with("movie.mp4"));
    }
}
