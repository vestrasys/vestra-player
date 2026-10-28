//! `packages.validate` and `packages.validateArchive` off the UI thread.
//!
//! Unpacking, `node --check` and the Slint compile of every `.slint` file
//! take hundreds of milliseconds (more in a debug build), and the workshop
//! validates on every check while a video may be playing. The caller's
//! thread only reads the request; a `package-validate` thread does the rest,
//! and the result is answered from the host tick (`Validations::poll`), where
//! the candidate gets its token. At most [`MAX_RUNNING`] run at once; the
//! rest wait in order. A requester that stops is never answered: its waiting
//! requests are dropped and its running ones told to stop at the next step,
//! and their staged copies go with them.
#![forbid(unsafe_code)]
use crate::package_stage::Candidate;
use crate::{Reply, RpcError};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// The longest `packages.validateArchive` failure text (`javascript`, `slint`).
pub const CHECK_TEXT_LIMIT: usize = 8192;
/// Validations running at once; each may compile Slint, which takes memory.
const MAX_RUNNING: usize = 2;

/// What to validate, as read from the request on the caller's thread.
pub struct Request {
    source: Source,
    /// `digest` and `apiVersion` as given: the candidate must match them.
    digest: Option<Value>,
    api_version: Option<Value>,
}
enum Source {
    Directory(PathBuf),
    Archive(PathBuf),
}

impl Request {
    /// The request's own checks, cheap enough for the UI thread: the
    /// parameters, and an archive's file extension.
    pub fn read(method: &str, params: &Value) -> Result<Self, RpcError> {
        let source = if method == "packages.validate" {
            Source::Directory(PathBuf::from(crate::string(params, "directory")?))
        } else {
            let file = Path::new(crate::string(params, "file")?);
            crate::package_archive::check_extension(file)
                .map_err(|e| RpcError::invalid(e.to_string()))?;
            Source::Archive(file.to_owned())
        };
        Ok(Self {
            source,
            digest: params.get("digest").cloned(),
            api_version: params.get("apiVersion").cloned(),
        })
    }
}

/// Stops a running validation between its steps.
#[derive(Clone, Default)]
struct Cancel(Arc<AtomicBool>);
impl Cancel {
    fn check(&self) -> Result<(), RpcError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(RpcError::new("disabled", "plugin no longer running"));
        }
        Ok(())
    }
}

/// Everything validation does, on any thread: unpack, stage, the manifest
/// and digest, `node --check` of the entry, and each `.slint` file compiled.
fn validate(request: Request, cancel: &Cancel) -> Result<Candidate, RpcError> {
    let candidate = match &request.source {
        Source::Directory(directory) => Candidate::capture(directory)?,
        Source::Archive(file) => {
            let extracted = crate::package_archive::extract(file)
                .map_err(|e| RpcError::invalid(format!("{e:#}")))?;
            cancel.check()?;
            Candidate::capture(extracted.path())?
        }
    };
    crate::package_manager::check_reserved(&candidate.manifest().id, None)?;
    if request
        .digest
        .is_some_and(|v| v.as_str() != Some(candidate.digest()))
    {
        return Err(RpcError::new("staleDigest", "candidate digest changed"));
    }
    if request
        .api_version
        .is_some_and(|v| v.as_str() != Some(crate::plugins::API))
    {
        return Err(RpcError::new("versionMismatch", "contract mismatch"));
    }
    cancel.check()?;
    let entry = &candidate.manifest().entry;
    let result = crate::runtime_paths::get()
        .node_command()
        .arg("--check")
        .arg(candidate.directory().join(entry))
        .output()
        .map_err(anyhow::Error::from)?;
    if !result.status.success() {
        let text = String::from_utf8_lossy(&result.stderr);
        return Err(RpcError::new(
            "javascript",
            package_relative(&text, candidate.directory(), entry)
                .chars()
                .take(CHECK_TEXT_LIMIT)
                .collect::<String>(),
        ));
    }
    fn slint_files(root: &Path, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(root)? {
            let p = entry?.path();
            if p.is_dir() {
                slint_files(&p, found)?;
            } else if p.extension().is_some_and(|x| x == "slint") {
                found.push(p);
            }
        }
        Ok(())
    }
    let mut files = vec![];
    slint_files(candidate.directory(), &mut files)?;
    for path in files {
        cancel.check()?;
        // The interpreter's compiler is not tied to Slint's event loop or
        // platform: it parses and type-checks, and the result's component
        // definitions are dropped here, on this thread.
        let result = pollster::block_on(crate::ui::compiler().build_from_path(path));
        if result.has_errors() {
            let lines = result
                .diagnostics()
                .map(|d| SlintLine::of(&d, candidate.directory()))
                .collect::<Vec<_>>();
            return Err(RpcError::new("slint", slint_report(lines)));
        }
    }
    Ok(candidate)
}

/// `text` (a tool's output about the package) without the staging
/// directory: `<staging>\dist\main.js:3` reads `dist/main.js:3`.
fn package_relative(text: &str, root: &Path, entry: &str) -> String {
    let mut roots = vec![root.to_string_lossy().into_owned()];
    if let Ok(real) = dunce::canonicalize(root) {
        roots.push(real.to_string_lossy().into_owned());
    }
    // As given and with every separator one way or the other, then a
    // separator either way before the path within.
    let mut forms = vec![];
    for root in roots {
        let root = root.trim_end_matches(['/', '\\']).to_owned();
        forms.push(root.replace('/', "\\"));
        forms.push(root.replace('\\', "/"));
        forms.push(root);
    }
    let mut text = text.to_owned();
    for root in forms {
        for separator in ['\\', '/'] {
            let prefix = format!("{root}{separator}");
            let native = entry.replace(['/', '\\'], &separator.to_string());
            text = text.replace(&format!("{prefix}{native}"), entry);
            text = text.replace(&prefix, "");
        }
    }
    text
}

/// One Slint compiler diagnostic as validation reports it.
#[derive(Debug, PartialEq)]
struct SlintLine {
    error: bool,
    /// The file within the package (`/` separated), else as the compiler named it.
    file: String,
    line: usize,
    column: usize,
    message: String,
}

impl SlintLine {
    fn of(diagnostic: &slint_interpreter::Diagnostic, root: &Path) -> Self {
        let file = diagnostic.source_file().map_or_else(String::new, |path| {
            let real = dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned());
            let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_owned());
            let shown = real.strip_prefix(&root).unwrap_or(&real);
            shown
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        });
        let (line, column) = diagnostic.line_column();
        Self {
            error: diagnostic.level() == slint_interpreter::DiagnosticLevel::Error,
            file,
            line,
            column,
            message: diagnostic.message().to_owned(),
        }
    }
}

/// `file:line:column: error|warning: message`, one per line, errors first
/// (each kind in the compiler's order), at most [`CHECK_TEXT_LIMIT`]
/// characters, like the `javascript` check's text. Warnings stay: they only
/// appear beside an error (a package without errors passes), they often
/// explain it (a deprecated name next to the unknown one), and the limit
/// keeps them from crowding the errors out, which come first.
fn slint_report(mut lines: Vec<SlintLine>) -> String {
    lines.sort_by_key(|line| !line.error);
    lines
        .iter()
        .map(|d| {
            let kind = if d.error { "error" } else { "warning" };
            let message = d.message.replace(['\r', '\n'], " ");
            format!("{}:{}:{}: {kind}: {message}", d.file, d.line, d.column)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(CHECK_TEXT_LIMIT)
        .collect()
}

/// A validation on its thread.
struct Running {
    reply: Reply,
    cancel: Cancel,
}
/// A finished validation: its serial and outcome.
type Finished = (u64, Result<Candidate, RpcError>);
/// A validation to answer: its requester and outcome.
pub type Answer = (Reply, Result<Candidate, RpcError>);

/// The validations in flight (module docs).
pub struct Validations {
    waiting: VecDeque<(Reply, Request)>,
    running: BTreeMap<u64, Running>,
    serial: u64,
    results: (mpsc::Sender<Finished>, mpsc::Receiver<Finished>),
}

impl Default for Validations {
    fn default() -> Self {
        Self {
            waiting: VecDeque::new(),
            running: BTreeMap::new(),
            serial: 0,
            results: mpsc::channel(),
        }
    }
}

impl Validations {
    /// Queue `request`; it starts at once when fewer than [`MAX_RUNNING`]
    /// run. Answered by a later `poll`.
    pub fn start(&mut self, reply: Reply, request: Request) -> Vec<(Reply, RpcError)> {
        self.waiting.push_back((reply, request));
        self.pump()
    }
    /// Start waiting validations while there is room; those whose thread
    /// could not be created fail at once.
    fn pump(&mut self) -> Vec<(Reply, RpcError)> {
        let mut failed = vec![];
        while self.running.len() < MAX_RUNNING {
            let Some((reply, request)) = self.waiting.pop_front() else {
                break;
            };
            self.serial += 1;
            let serial = self.serial;
            let cancel = Cancel::default();
            let results = self.results.0.clone();
            let stop = cancel.clone();
            let spawned = std::thread::Builder::new()
                .name("package-validate".into())
                .spawn(move || {
                    // A panic (in the Slint compiler, say) still answers,
                    // and frees its place for the next request.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        validate(request, &stop)
                    }))
                    .unwrap_or_else(|_| Err(RpcError::new("host", "package validation failed")));
                    let _ = results.send((serial, result));
                });
            match spawned {
                Ok(_) => {
                    self.running.insert(serial, Running { reply, cancel });
                }
                Err(error) => failed.push((
                    reply,
                    RpcError::new("host", format!("validation thread: {error}")),
                )),
            }
        }
        failed
    }
    /// Finished validations, for the requesters still running (`alive`);
    /// the others' requests are dropped or stopped. The candidates that
    /// passed are returned for `Manager::remember`, in finishing order.
    pub fn poll(&mut self, alive: impl Fn(u64) -> bool) -> Vec<Answer> {
        self.waiting.retain(|(reply, _)| alive(reply.owner));
        for running in self.running.values() {
            if !alive(running.reply.owner) {
                running.cancel.0.store(true, Ordering::Relaxed);
            }
        }
        let mut done = vec![];
        while let Ok((serial, result)) = self.results.1.try_recv() {
            let Some(running) = self.running.remove(&serial) else {
                continue;
            };
            if alive(running.reply.owner) {
                done.push((running.reply, result));
            }
        }
        done.extend(
            self.pump()
                .into_iter()
                .map(|(reply, error)| (reply, Err(error))),
        );
        done
    }
    /// Whether any validation is running or waiting.
    #[cfg(test)]
    pub fn busy(&self) -> bool {
        !self.waiting.is_empty() || !self.running.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn line(error: bool, file: &str, at: usize, message: &str) -> SlintLine {
        SlintLine {
            error,
            file: file.into(),
            line: at,
            column: 5,
            message: message.into(),
        }
    }

    #[test]
    fn diagnostics_read_as_package_paths_errors_first_and_bounded() {
        let text = slint_report(vec![
            line(false, "ui/bar.slint", 3, "unused"),
            line(true, "ui/bar.slint", 9, "Unknown property foo"),
            line(true, "ui/parts/row.slint", 2, "two\nlines"),
        ]);
        assert_eq!(
            text,
            "ui/bar.slint:9:5: error: Unknown property foo\n\
             ui/parts/row.slint:2:5: error: two lines\n\
             ui/bar.slint:3:5: warning: unused"
        );
        let many = (0..2000)
            .map(|i| line(true, "ui/x.slint", i, "Unknown element"))
            .collect();
        assert_eq!(slint_report(many).chars().count(), CHECK_TEXT_LIMIT);
    }

    /// A real compile: the package's relative path, a line and column, no
    /// absolute path of the extraction directory, no Debug escapes.
    #[test]
    fn a_broken_slint_file_reports_its_package_path_line_and_column() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("ui")).unwrap();
        let file = root.path().join("ui/bad.slint");
        std::fs::write(
            &file,
            "export component Bad inherits Rectangle {\n    nonsense: 3px;\n}\n",
        )
        .unwrap();
        let result = pollster::block_on(crate::ui::compiler().build_from_path(file));
        assert!(result.has_errors());
        let text = slint_report(
            result
                .diagnostics()
                .map(|d| SlintLine::of(&d, root.path()))
                .collect(),
        );
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("ui/bad.slint:2:5: error: "), "{text}");
        let root_text = root.path().to_string_lossy().into_owned();
        assert!(!text.contains(&root_text), "{text}");
        assert!(!text.contains("Diagnostic {"), "{text}");
    }

    #[test]
    fn tool_output_names_package_paths_not_the_staging_directory() {
        let root = tempfile::tempdir().unwrap();
        let shown = root.path().to_string_lossy().into_owned();
        let native = root.path().join("dist").join("main.js");
        let text = format!(
            "{}:1\nconst a = ;\n\nSyntaxError: Unexpected token ';'\n at {shown}/lib/x.js:2",
            native.display()
        );
        let relative = package_relative(&text, root.path(), "dist/main.js");
        assert!(
            relative.starts_with("dist/main.js:1\nconst a = ;"),
            "{relative}"
        );
        assert!(relative.ends_with(" at lib/x.js:2"), "{relative}");
        assert!(!relative.contains(&shown), "{relative}");
    }

    /// A package directory: a manifest, `main.js`, and `slint` files.
    fn package(id: &str, main: &str, slint: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::json!({"apiVersion":crate::plugins::API,"id":id,
                "version":"1.0.0","entry":"main.js","capabilities":[]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.path().join("main.js"), main).unwrap();
        for (name, source) in slint {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        dir
    }

    fn request(dir: &Path) -> Request {
        Request::read(
            "packages.validate",
            &serde_json::json!({"directory":dir.to_string_lossy()}),
        )
        .unwrap()
    }

    fn reply(owner: u64, id: u64) -> Reply {
        Reply { owner, id }
    }

    /// Poll as the host tick does until nothing runs; every poll returns at
    /// once, so the tick keeps its rate while validations run.
    fn drain(
        validations: &mut Validations,
        alive: impl Fn(u64) -> bool + Copy,
    ) -> (Vec<Answer>, usize) {
        let began = Instant::now();
        let (mut done, mut ticks) = (vec![], 0);
        while validations.busy() {
            assert!(
                began.elapsed() < Duration::from_secs(120),
                "validation hung"
            );
            let tick = Instant::now();
            done.extend(validations.poll(alive));
            assert!(tick.elapsed() < Duration::from_millis(50), "a tick blocked");
            ticks += 1;
            std::thread::sleep(Duration::from_millis(5));
        }
        (done, ticks)
    }

    /// Validation answers from later ticks, not from the request: the tick
    /// runs while the Slint compile is pending. Two requests run side by
    /// side and each gets its own answer; a broken one fails with `slint`.
    #[test]
    fn validation_runs_off_the_requesting_thread() {
        let good = package(
            "example.good",
            "export {};\n",
            &[(
                "ui/view.slint",
                "export component View inherits Rectangle { width: 10px; }\n",
            )],
        );
        let bad = package(
            "example.bad",
            "export {};\n",
            &[(
                "ui/view.slint",
                "export component View inherits Rectangle {\n    nonsense: 3px;\n}\n",
            )],
        );
        let mut validations = Validations::default();
        let began = Instant::now();
        assert!(
            validations
                .start(reply(1, 7), request(good.path()))
                .is_empty()
        );
        assert!(
            validations
                .start(reply(2, 8), request(bad.path()))
                .is_empty()
        );
        assert!(began.elapsed() < Duration::from_millis(50));
        assert!(validations.poll(|_| true).is_empty(), "answered too early");
        let (done, ticks) = drain(&mut validations, |_| true);
        assert!(ticks > 1);
        assert_eq!(done.len(), 2);
        for (reply, result) in done {
            match reply.owner {
                1 => {
                    assert_eq!(reply.id, 7);
                    assert_eq!(result.unwrap().manifest().id, "example.good");
                }
                _ => {
                    let error = result.err().unwrap();
                    assert_eq!(error.code, "slint");
                    assert!(
                        error.message.starts_with("ui/view.slint:2:5: error: "),
                        "{}",
                        error.message
                    );
                }
            }
        }
    }

    /// Past `MAX_RUNNING` requests wait; a stopped requester's requests are
    /// dropped (waiting) or stopped (running) and never answered.
    #[test]
    fn a_stopped_requester_is_not_answered_and_others_still_are() {
        let dirs = (0..4)
            .map(|i| package(&format!("example.p{i}"), "export {};\n", &[]))
            .collect::<Vec<_>>();
        let mut validations = Validations::default();
        for (i, dir) in dirs.iter().enumerate() {
            let owner = if i % 2 == 0 { 1 } else { 2 };
            assert!(
                validations
                    .start(reply(owner, i as u64), request(dir.path()))
                    .is_empty()
            );
        }
        assert_eq!(validations.running.len(), MAX_RUNNING);
        assert_eq!(validations.waiting.len(), 4 - MAX_RUNNING);
        let (done, _) = drain(&mut validations, |owner| owner == 2);
        let mut ids = done
            .iter()
            .map(|(reply, result)| {
                assert_eq!(reply.owner, 2);
                assert!(result.is_ok());
                reply.id
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, [1, 3]);
    }

    #[test]
    fn a_cancelled_validation_stops_before_its_next_step() {
        let dir = package("example.cancel", "export {};\n", &[]);
        let cancel = Cancel::default();
        cancel.0.store(true, Ordering::Relaxed);
        let error = validate(request(dir.path()), &cancel).err().unwrap();
        assert_eq!(error.code, "disabled");
    }
}
