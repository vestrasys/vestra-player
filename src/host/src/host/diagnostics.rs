//! The UI thread's part of the host log and of the plugin shaders' crash
//! guard (doc/architecture/README.md §4.3):
//!
//! - plugin shader lists (`video.shaders.set`, `video.render.set`'s
//!   `shaders`) are followed by plugin and number of files, never paths: a
//!   change goes to the host log, and a non-empty list arms the guard
//!   (`render_state`'s `shaders` record: `applying`) before the request goes
//!   to the playback worker. [`SHADER_STABLE_SECS`] of playback with them
//!   on, their lists emptying or the player quitting settles it (`ok`);
//! - a run that keeps plugin shaders off (the last one ended while they
//!   were on, `GuardFile::shader_start`) refuses non-empty lists with
//!   [`PREVIOUS_CRASH`] and says so once on the OSD;
//! - every [`MEMORY_EVERY`] the memory line goes to the host log.
//!
//! The lists are followed as requested: one the worker then refuses (the
//! limit, a write mpv refused) still counts until it is replaced, so the
//! guard may be armed for nothing, never the other way round.
use super::*;
use crate::playback::render_state::{GuardFile, GuardKey, SHADER_STABLE_SECS, ShaderBlock};

/// The error code of a plugin shader list refused because the last run
/// ended while plugin shaders were on.
pub(crate) const PREVIOUS_CRASH: &str = "previous-crash";
/// How often the memory line goes to the host log.
pub(crate) const MEMORY_EVERY: Duration = Duration::from_secs(60);

/// A `host.error` for the host log: its code and its message up to the
/// first path, link or quoted name (`diag::redact`); no other field.
pub(crate) fn host_error_line(data: &Value) -> String {
    let code = data["code"].as_str().unwrap_or("unknown");
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    format!(
        "host.error {code}: {}",
        crate::diag::redact(data["message"].as_str().unwrap_or_default())
    )
}

/// The plugin shaders' crash guard of this run.
#[derive(Debug, Default)]
pub(crate) struct ShaderGuard {
    file: GuardFile,
    key: GuardKey,
    /// This run keeps plugin shaders off.
    blocked: Option<ShaderBlock>,
    /// A list was refused for it; the OSD has not said so yet.
    refused: bool,
    /// The OSD said so (once per run).
    told: bool,
    /// The lists that are on: (owner, handle) → (plugin, files).
    lists: BTreeMap<(u64, String), (String, usize)>,
    /// `applying` is written and not settled yet.
    armed: bool,
    /// Playback with them on since they were armed.
    played: Duration,
    last_tick: Option<Instant>,
}

impl ShaderGuard {
    /// `blocked`: what `GuardFile::shader_start` said at start.
    pub(crate) fn new(file: GuardFile, key: GuardKey, blocked: Option<ShaderBlock>) -> Self {
        Self {
            file,
            key,
            blocked,
            ..Self::default()
        }
    }
    /// Why a list of `files` shader files is refused now, if it is: only
    /// non-empty lists while this run keeps plugin shaders off (clearing
    /// always works).
    pub(crate) fn refuse(&mut self, files: usize, now: u64) -> Option<RpcError> {
        let block = self.blocked.as_ref().filter(|_| files > 0)?;
        let whose = if block.plugins.is_empty() {
            String::new()
        } else {
            format!(" ({})", block.plugins.join(", "))
        };
        let message = match block.until {
            None => format!(
                "plugin shaders are off for this start: the player ended unexpectedly while they were on{whose}; the next start applies them again"
            ),
            Some(until) => format!(
                "plugin shaders are off: the player ended unexpectedly twice within a day while they were on{whose}; they apply again after a graphics driver or player update, or in {} days",
                until.saturating_sub(now).div_ceil(24 * 60 * 60)
            ),
        };
        if !self.refused && !self.told {
            logged!("shaders: a list was refused ({message})");
        }
        self.refused = true;
        Some(RpcError::new(PREVIOUS_CRASH, message))
    }
    /// The OSD line is due (once per run, after the first refusal, once a
    /// window can show it).
    pub(crate) fn take_osd(&mut self, can_show: bool) -> bool {
        let due = self.refused && !self.told && can_show;
        self.told |= due;
        due
    }
    /// The plugins whose lists are on, in order, once each.
    fn plugins(&self) -> Vec<String> {
        let mut plugins: Vec<String> = self.lists.values().map(|(id, _)| id.clone()).collect();
        plugins.sort();
        plugins.dedup();
        plugins
    }
    fn files(&self) -> usize {
        self.lists.values().map(|(_, files)| files).sum()
    }
    fn settle(&mut self) {
        if self.armed {
            self.armed = false;
            self.file.shaders_settle();
        }
    }
    /// A plugin's list `handle` (`render`, or `shaders:<id>`) now has
    /// `files` files, before the worker gets it: logged, and a non-empty one
    /// arms the guard (again: a new list may crash where the old one ran).
    pub(crate) fn set(&mut self, owner: u64, handle: &str, plugin: &str, files: usize, now: u64) {
        let key = (owner, handle.to_owned());
        let before = self.lists.get(&key).map_or(0, |(_, files)| *files);
        if files == 0 && before == 0 {
            return;
        }
        if files == 0 {
            self.lists.remove(&key);
        } else {
            self.lists.insert(key, (plugin.to_owned(), files));
        }
        logged!(
            "shaders: {plugin} {handle} has {files} file(s); {} in all",
            self.files()
        );
        if files > 0 {
            self.file.shaders_applying(&self.key, self.plugins(), now);
            self.armed = true;
            self.played = Duration::ZERO;
        } else if self.lists.is_empty() {
            self.settle();
        }
    }
    /// A plugin stopped or was parked: the worker removes its lists.
    pub(crate) fn owner_gone(&mut self, owner: u64) {
        let before = self.lists.len();
        self.lists.retain(|(holder, _), _| *holder != owner);
        if self.lists.len() == before {
            return;
        }
        logged!(
            "shaders: plugin {owner} stopped; {} file(s) in all",
            self.files()
        );
        if self.lists.is_empty() {
            self.settle();
        }
    }
    /// Every tick: playback time with shaders on settles the guard.
    pub(crate) fn tick(&mut self, playing: bool, now: Instant) {
        let since = self.last_tick.replace(now);
        if !self.armed {
            return;
        }
        if playing && let Some(since) = since {
            self.played += now.saturating_duration_since(since);
        }
        if self.played >= Duration::from_secs(SHADER_STABLE_SECS) {
            diag!(
                "shaders: {SHADER_STABLE_SECS} s of playback with plugin shaders on; the crash guard settled"
            );
            self.settle();
        }
    }
    /// The player quits normally.
    pub(crate) fn quit(&mut self) {
        self.settle();
    }
}

impl Host {
    /// Before a plugin shader list of `files` files goes to the worker
    /// (`capabilities::processing_video`): refused while this run keeps
    /// plugin shaders off; otherwise followed (GPU rendering only: under
    /// software rendering the worker refuses non-empty lists).
    pub(crate) fn plugin_shaders(
        &mut self,
        owner: u64,
        handle: &str,
        files: usize,
    ) -> std::result::Result<(), RpcError> {
        let now = crate::playback::render_state::now();
        if let Some(error) = self.shader_guard.refuse(files, now) {
            return Err(error);
        }
        if files > 0 && !self.backend.render.now().0 {
            return Ok(());
        }
        let plugin = self
            .plugins
            .get(&owner)
            .map(|plugin| plugin.manifest.id.clone())
            .unwrap_or_else(|| format!("plugin {owner}"));
        self.shader_guard.set(owner, handle, &plugin, files, now);
        Ok(())
    }
    /// Every tick: the shader guard's playback time and OSD line, and the
    /// memory line every [`MEMORY_EVERY`] while the host log is open.
    pub(crate) fn diagnostics_tick(&mut self) {
        let now = Instant::now();
        let playing = self.last_source["phase"] == "ready" && self.last_source["paused"] == false;
        self.shader_guard.tick(playing, now);
        if self.shader_guard.take_osd(self.shell.is_some()) {
            self.show_osd(&i18n::tr(
                "Plugin shaders are off: the player ended unexpectedly while they were on",
            ));
        }
        if crate::diag::log_open()
            && self
                .memory_logged
                .is_none_or(|at| now.saturating_duration_since(at) >= MEMORY_EVERY)
        {
            self.memory_logged = Some(now);
            if let Some(status) = platform::memory_status() {
                logged!("{}", crate::diag::memory_line(&status));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::render_state::{ShaderState, now as unix_now};

    fn key() -> GuardKey {
        GuardKey {
            adapter: "AMD Radeon 780M".into(),
            driver: "32.0.32015.7029".into(),
            app: "0.6.3".into(),
        }
    }

    fn state(file: &GuardFile) -> Option<ShaderState> {
        file.read().shaders.map(|record| record.state)
    }

    #[test]
    fn a_non_empty_list_arms_and_playback_settles() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let mut guard = ShaderGuard::new(file.clone(), key(), None);
        assert!(guard.refuse(3, 0).is_none(), "nothing blocks");
        // Clearing what was never set writes nothing.
        guard.set(1, "shaders:x", "acme.a", 0, 10);
        assert_eq!(state(&file), None);
        guard.set(1, "shaders:x", "acme.a", 2, 10);
        guard.set(2, "render", "acme.b", 1, 11);
        let record = file.read().shaders.unwrap();
        assert_eq!(record.state, ShaderState::Applying);
        assert_eq!(record.plugins, ["acme.a", "acme.b"]);
        assert_eq!(record.key, key());
        // Paused or idle time does not count; playback does.
        let start = Instant::now();
        guard.tick(false, start);
        guard.tick(false, start + Duration::from_secs(30));
        assert_eq!(state(&file), Some(ShaderState::Applying));
        guard.tick(true, start + Duration::from_secs(35));
        assert_eq!(state(&file), Some(ShaderState::Applying), "5 s played");
        guard.tick(true, start + Duration::from_secs(41));
        assert_eq!(state(&file), Some(ShaderState::Ok));
        // A new list arms again, and the count starts over.
        guard.set(1, "shaders:x", "acme.a", 1, 50);
        assert_eq!(state(&file), Some(ShaderState::Applying));
        guard.tick(true, start + Duration::from_secs(45));
        assert_eq!(state(&file), Some(ShaderState::Applying));
        // Every list emptied (cleared, or its plugin stopped): settled.
        guard.set(1, "shaders:x", "acme.a", 0, 60);
        assert_eq!(state(&file), Some(ShaderState::Applying), "acme.b's is on");
        guard.owner_gone(2);
        assert_eq!(state(&file), Some(ShaderState::Ok));
        // Quitting settles too.
        guard.set(3, "render", "acme.c", 4, 70);
        guard.quit();
        assert_eq!(state(&file), Some(ShaderState::Ok));
    }

    /// The run after a crash with shaders on: non-empty lists are refused
    /// with `previous-crash`, clearing still works, and the OSD line comes
    /// once, when a window can show it.
    #[test]
    fn a_blocked_run_refuses_lists_and_tells_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let block = ShaderBlock {
            plugins: vec!["acme.sharpen".into()],
            until: None,
        };
        let mut guard = ShaderGuard::new(file.clone(), key(), Some(block));
        assert!(!guard.take_osd(true), "nothing refused yet");
        assert!(guard.refuse(0, 0).is_none(), "clearing works");
        let error = guard.refuse(1, 0).unwrap();
        assert_eq!(error.code, PREVIOUS_CRASH);
        assert!(
            error.message.contains("for this start"),
            "{}",
            error.message
        );
        assert!(error.message.contains("acme.sharpen"), "{}", error.message);
        assert!(!guard.take_osd(false), "no window yet");
        assert!(guard.take_osd(true));
        assert!(guard.refuse(2, 0).is_some());
        assert!(!guard.take_osd(true), "once per run");
        // Off for days: the message says until when.
        let now = unix_now();
        let mut guard = ShaderGuard::new(
            file,
            key(),
            Some(ShaderBlock {
                plugins: vec![],
                until: Some(now + 3 * 24 * 60 * 60),
            }),
        );
        let error = guard.refuse(1, now).unwrap();
        assert!(error.message.contains("in 3 days"), "{}", error.message);
    }

    #[test]
    fn host_errors_reach_the_log_without_paths() {
        assert_eq!(
            host_error_line(
                &json!({"code":"source-load","message":"cannot open C:\\Users\\me\\a.mkv","generation":3})
            ),
            "host.error source-load: cannot open …"
        );
        assert_eq!(
            host_error_line(&json!({"code":"menu-projection","message":"bad item"})),
            "host.error menu-projection: bad item"
        );
        assert_eq!(
            host_error_line(&json!({"code":"x y\nz","message":null})),
            "host.error xyz: "
        );
        assert_eq!(host_error_line(&Value::Null), "host.error unknown: ");
    }

    #[test]
    fn without_a_file_nothing_is_written() {
        let mut guard = ShaderGuard::default();
        guard.set(1, "render", "a", 2, 1);
        guard.tick(true, Instant::now());
        guard.quit();
        assert_eq!(GuardFile::default().read(), Default::default());
    }
}
