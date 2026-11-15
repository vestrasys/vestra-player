//! The wire trace, a test switch (`VESTRA_TEST_WIRE_TRACE=<file>`, core
//! thread plan S1.1, doc/architecture/core-thread.md §8): every line the
//! host reads from or writes to a plugin, one JSON line each, appended to
//! `<file>`:
//!
//! ```text
//! {"plugin":"vestra.osd","seq":12,"dir":"out","owner":7,"ms":1834,"line":{…}}
//! ```
//!
//! - `plugin`: the plugin's installed id (its manifest id); the shared
//!   runner's own control lines (hello, ping, pong, stop) are under
//!   [`RUNNER`].
//! - `seq`: per plugin, over both directions and every process of it, in
//!   the order the lines were seen by the reader and writer threads.
//! - `dir`: `in` (read from the plugin) or `out` (written to it).
//! - `owner` (the process generation) and `ms` (since the trace began) are
//!   for people; `src/scripts/verify-wire.mjs` normalises them away.
//! - `line`: the protocol line as parsed (in) or as written (out, without
//!   the shared runner's `g`). A line that could not be parsed has
//!   `invalid` (the reason) in its place.
//!
//! Lines are taken where they cross the pipe: on each own process's reader
//! and writer threads (`process/mod.rs`) and the shared runner's
//! (`shared_runner.rs`), so replies, events, broadcasts, the readers' limit
//! refusals and `runtime.stop` are all in it. Not in it: a line that was
//! queued but never written (the pipe closed), and what a reader drains
//! without reading after a fault or once the host stopped listening (at
//! quit). The first line of each player run is `{"start":<pid>}`; the file
//! is appended to.
//!
//! Off (the variable unset or empty), every hook is one check of a
//! once-initialised `None`: nothing is serialised, written or allocated.
//! Not passed on to anything: a player restarted after a device loss gets
//! no offscreen run (`VESTRA_TEST_OFFSCREEN` is removed) and its trace, if
//! any, goes on in the same file.
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// The `plugin` of the shared runner's own control lines.
pub const RUNNER: &str = "(shared runner)";

/// Which way a line went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Read from the plugin.
    In,
    /// Written to the plugin.
    Out,
}
impl Dir {
    fn as_str(self) -> &'static str {
        match self {
            Dir::In => "in",
            Dir::Out => "out",
        }
    }
}

struct State {
    file: Box<dyn Write + Send>,
    /// Each plugin's next sequence number.
    seq: BTreeMap<String, u64>,
    /// Owner numbers (process generations) and the plugin each one is.
    owners: BTreeMap<u64, String>,
}

pub struct Trace {
    began: Instant,
    state: Mutex<State>,
}

static TRACE: OnceLock<Option<Trace>> = OnceLock::new();

fn trace() -> Option<&'static Trace> {
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("VESTRA_TEST_WIRE_TRACE").filter(|p| !p.is_empty())?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| {
                    diag!(
                        "VESTRA_TEST_WIRE_TRACE {}: {error}; no wire trace",
                        std::path::Path::new(&path).display()
                    )
                })
                .ok()?;
            let trace = Trace::new(Box::new(file));
            diag!(
                "test: wire trace to {}",
                std::path::Path::new(&path).display()
            );
            Some(trace)
        })
        .as_ref()
}

impl Trace {
    fn new(file: Box<dyn Write + Send>) -> Self {
        let trace = Self {
            began: Instant::now(),
            state: Mutex::new(State {
                file,
                seq: BTreeMap::new(),
                owners: BTreeMap::new(),
            }),
        };
        trace.write_raw(&json!({"start": std::process::id()}));
        trace
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn write_raw(&self, value: &Value) {
        let mut state = self.lock();
        let mut text = value.to_string();
        text.push('\n');
        let _ = state.file.write_all(text.as_bytes());
    }
    fn bind(&self, owner: u64, plugin: &str) {
        self.lock().owners.insert(owner, plugin.to_owned());
    }
    /// One line of `owner`'s (or of the plugin named `plugin` when given).
    fn line(&self, owner: u64, plugin: Option<&str>, dir: Dir, line: Value) {
        let ms = self.began.elapsed().as_millis() as u64;
        let mut state = self.lock();
        let plugin = match plugin {
            Some(plugin) => plugin.to_owned(),
            None => state
                .owners
                .get(&owner)
                .cloned()
                .unwrap_or_else(|| format!("(owner {owner})")),
        };
        let seq = state.seq.entry(plugin.clone()).or_insert(0);
        *seq += 1;
        let record = json!({
            "plugin": plugin,
            "seq": *seq,
            "dir": dir.as_str(),
            "owner": owner,
            "ms": ms,
            "line": line,
        });
        let mut text = record.to_string();
        text.push('\n');
        let _ = state.file.write_all(text.as_bytes());
    }
    fn control(&self, dir: Dir, line: Value) {
        let data = &line["data"];
        if line["event"] == "plugin.start"
            && let Some(id) = data["id"].as_str()
        {
            let (owner, id) = (data["g"].as_u64().unwrap_or(0), id.to_owned());
            self.line(owner, Some(&id), dir, line);
        } else if let Some(owner) = data["g"].as_u64().filter(|g| *g > 0) {
            self.line(owner, None, dir, line);
        } else {
            self.line(0, Some(RUNNER), dir, line);
        }
    }
}

/// Owner `owner` is the plugin `id` (at its load, before its first line).
pub fn bind(owner: u64, id: &str) {
    if let Some(trace) = trace() {
        trace.bind(owner, id);
    }
}

/// A line of an own process's, or of a shared-runner member's (`owner`).
pub fn line(owner: u64, dir: Dir, line: &Value) {
    if let Some(trace) = trace() {
        trace.line(owner, None, dir, line.clone());
    }
}

/// A line read from an own process that could not be parsed (`why`).
pub fn invalid(owner: u64, why: &str) {
    if let Some(trace) = trace() {
        trace.line(owner, None, Dir::In, json!({ "invalid": why }));
    }
}

/// A control line of the shared runner (`g` 0): one that names a member
/// (`plugin.start`'s `data.id`, `runner.stopped`'s and `runner.fault`'s
/// `data.g`) goes to that plugin, the rest to [`RUNNER`].
pub fn control(dir: Dir, line: &Value) {
    if let Some(trace) = trace() {
        trace.control(dir, line.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Sink {
        fn records(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    #[test]
    fn lines_carry_the_plugin_a_sequence_per_plugin_and_the_direction() {
        let sink = Sink::default();
        let trace = Trace::new(Box::new(sink.clone()));
        trace.bind(3, "a.one");
        trace.bind(4, "b.two");
        trace.line(3, None, Dir::In, json!({"id":1,"method":"osd.show"}));
        trace.line(4, None, Dir::In, json!({"id":1,"method":"source.state"}));
        trace.line(3, None, Dir::Out, json!({"id":1,"result":null}));
        // A later process of the same plugin goes on with its sequence.
        trace.bind(9, "a.one");
        trace.line(9, None, Dir::Out, json!({"event":"runtime.stop"}));
        trace.line(5, None, Dir::In, json!({"event":"runtime.ready"}));
        let records = sink.records();
        assert_eq!(records[0]["start"], std::process::id());
        let seen: Vec<_> = records[1..]
            .iter()
            .map(|r| {
                (
                    r["plugin"].as_str().unwrap().to_owned(),
                    r["seq"].as_u64().unwrap(),
                    r["dir"].as_str().unwrap().to_owned(),
                    r["owner"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("a.one".into(), 1, "in".into(), 3),
                ("b.two".into(), 1, "in".into(), 4),
                ("a.one".into(), 2, "out".into(), 3),
                ("a.one".into(), 3, "out".into(), 9),
                ("(owner 5)".into(), 1, "in".into(), 5),
            ]
        );
        assert_eq!(records[3]["line"], json!({"id":1,"result":null}));
    }

    #[test]
    fn shared_runner_control_lines_go_to_the_member_they_name_or_to_the_runner() {
        let sink = Sink::default();
        let trace = Trace::new(Box::new(sink.clone()));
        trace.bind(7, "c.three");
        let start = json!({"event":"plugin.start","data":{"g":7,"id":"c.three"}});
        trace.control(Dir::Out, start);
        trace.control(Dir::Out, json!({"event":"runner.ping","data":{"seq":1}}));
        trace.control(Dir::In, json!({"event":"runner.stopped","data":{"g":7}}));
        trace.control(Dir::In, json!({"event":"runner.fault","data":{"g":0}}));
        let plugins: Vec<_> = sink.records()[1..]
            .iter()
            .map(|r| {
                (
                    r["plugin"].as_str().unwrap().to_owned(),
                    r["seq"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            plugins,
            [
                ("c.three".into(), 1),
                (RUNNER.into(), 1),
                ("c.three".into(), 2),
                (RUNNER.into(), 2),
            ]
        );
    }
}
