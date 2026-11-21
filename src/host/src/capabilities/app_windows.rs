//! `app.windows` (built-in only): `app.openWindow` starts a guest window
//! on files, the playlist's "Play in New Window" (user ruling 2026-11-21,
//! doc/architecture/guest-windows.md §7). Built-in only: the ruling makes
//! the playlist's row menu the only entry, and starting player processes
//! is for a plugin only once an SDK member says so. A wire method
//! (`plugin.wire.request`), not an SDK member yet.
use super::{Capability, Handled, Info, Recorded};
use crate::guest;
use crate::host::{Core, Event, Reply, RpcError};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct AppWindows;
static INFO: Info = Info {
    name: "app.windows",
    builtin_only: true,
};
static METHODS: &[&str] = &["app.openWindow"];
impl Capability for AppWindows {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    /// `{ paths }` (1 to `guest::MAX_PATHS` absolute paths of files): a new
    /// guest window plays the first and lists them all; answers `{ pid }`
    /// once its process started (on a thread: starting a process takes a
    /// few milliseconds on Windows).
    fn handle(&self, core: &mut Core, reply: &Reply, _: &str, params: &Value) -> Handled {
        let paths = paths(params)?;
        if cfg!(target_os = "macos") {
            // A second instance of the bundle needs LaunchServices
            // (`open -n`); a follow-up with a Mac (guest-windows.md §9).
            return Err(RpcError::new(
                "unsupported",
                "new windows are not available on macOS yet",
            ));
        }
        if core.headless {
            return Err(RpcError::new(
                "unsupported",
                "a player without windows opens no windows",
            ));
        }
        let events = core.events.clone();
        let reply = reply.clone();
        std::thread::Builder::new()
            .name("guest-start".into())
            .spawn(move || {
                let result = guest::spawn(&paths)
                    .map(|pid| {
                        logged!("guest: started process {pid}");
                        json!({ "pid": pid })
                    })
                    .map_err(|error| {
                        RpcError::new("failed", format!("the new window did not start: {error}"))
                    });
                let _ = events.send(Event::Reply(reply, result));
            })
            .map_err(|error| RpcError::new("busy", error.to_string()))?;
        Ok(None)
    }
}

/// The files of an `app.openWindow`: absolute paths of existing files, in
/// order, at least one and at most `guest::MAX_PATHS`.
fn paths(params: &Value) -> std::result::Result<Vec<PathBuf>, RpcError> {
    let list = params["paths"]
        .as_array()
        .ok_or_else(|| RpcError::invalid("paths requires a list of file paths"))?;
    if list.is_empty() || list.len() > guest::MAX_PATHS {
        return Err(RpcError::invalid(format!(
            "paths takes 1 to {} files",
            guest::MAX_PATHS
        )));
    }
    list.iter()
        .map(|value| {
            let path = value
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| RpcError::invalid("each path is a string"))?;
            if !path.is_absolute() {
                return Err(RpcError::invalid(format!(
                    "{} is not an absolute path",
                    path.display()
                )));
            }
            if !path.is_file() {
                return Err(RpcError::new(
                    "missing",
                    format!("{} is not a file", path.display()),
                ));
            }
            Ok(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paths_are_existing_absolute_files_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mkv");
        std::fs::write(&file, b"x").unwrap();
        let text = file.to_str().unwrap();
        assert_eq!(
            paths(&json!({"paths":[text, text]})).unwrap(),
            [file.clone(), file.clone()]
        );
        for (params, code) in [
            (json!({}), "invalid"),
            (json!({"paths":[]}), "invalid"),
            (json!({"paths":[1]}), "invalid"),
            (json!({"paths":["a.mkv"]}), "invalid"),
            (json!({"paths":[dir.path().join("gone.mkv")]}), "missing"),
            (json!({"paths":[dir.path()]}), "missing"),
            (json!({"paths":vec![text; guest::MAX_PATHS + 1]}), "invalid"),
        ] {
            assert_eq!(paths(&params).unwrap_err().code, code, "{params}");
        }
        assert_eq!(
            paths(&json!({"paths":vec![text; guest::MAX_PATHS]}))
                .unwrap()
                .len(),
            guest::MAX_PATHS
        );
    }
}
