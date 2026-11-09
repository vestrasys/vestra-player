//! Files the system or a second launch asks the player to open (`launch.rs`,
//! `platform::on_open_files`, `platform::single_instance`). They open like a
//! drop of the same paths on the video area, without modifier keys: each
//! goes to the plugin that provides its kind (`drops.drop`), so with the
//! playlist the first one plays and all of them are in the list. The core
//! does not open anything itself (it knows no plugin).
//!
//! At start-up the providers register while their plugins activate, so the
//! files wait until the installed packages have started, no activation is
//! pending and the shell compiled with every start-up panel (no compile
//! lands on early playback), or `SETTLE_LIMIT` at most. Paths no plugin
//! takes are reported.
//!
//! Files arriving within `QUIET` of each other are one open: Explorer starts
//! one process per selected file for "Open" on a multiple selection, and
//! each of those hands its file over separately (`single_instance`).
#![forbid(unsafe_code)]
use crate::host::*;
use crate::supervisor::activation;
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const SETTLE_LIMIT: Duration = Duration::from_secs(20);
const QUIET: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct OpenFiles {
    pending: Vec<PathBuf>,
    /// The first and the latest arrival of the waiting files.
    since: Option<(Instant, Instant)>,
}

impl OpenFiles {
    pub fn push(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let now = Instant::now();
        self.since = Some((self.since.map_or(now, |(first, _)| first), now));
        self.pending.extend(files);
    }
    /// The waiting files once `settled` (or waited long enough).
    pub fn due(&mut self, settled: bool) -> Option<Vec<PathBuf>> {
        let (first, latest) = self.since?;
        let ready = settled && latest.elapsed() >= QUIET;
        (ready || first.elapsed() >= SETTLE_LIMIT).then(|| {
            self.since = None;
            std::mem::take(&mut self.pending)
        })
    }
}

impl Host {
    /// Files to open (maybe none); `front`: another launch asked, so the
    /// main window comes to the front.
    pub(super) fn open_files(&mut self, files: Vec<PathBuf>, front: bool) {
        self.opens.push(files);
        if front {
            self.bring_to_front();
        }
        self.open_files_tick();
    }
    /// Files wait until the plugins started and the shell compiled with
    /// their start-up panels (`startup_panels`): a compile blocks the UI
    /// thread, and one landing just after playback began stalled it.
    pub(super) fn open_files_tick(&mut self) {
        if !self.startup_panels && self.now_ms() >= SETTLE_LIMIT.as_millis() as u64 {
            // A start that never settles (a plugin retried after a crash)
            // keeps no later file waiting.
            self.startup_panels = true;
        }
        if !self.startup_panels {
            self.startup_panels = self.startup_mounted
                && self.mount_queue.is_empty()
                && self.preparing.is_none()
                && self.held_mounts.is_empty()
                && !self.start_under_way()
                && self.startup_has_settled();
        }
        let settled =
            self.startup_panels && self.packages_started() && !self.builtin_first_run_queued() && {
                let now = self.now_ms();
                let owners = self.plugins.keys().copied().collect::<Vec<_>>();
                !owners.into_iter().any(|owner| {
                    matches!(
                        self.activation_status(owner, now),
                        Ok(activation::Status::Pending { .. })
                    )
                })
            };
        let Some(files) = self.opens.due(settled) else {
            return;
        };
        diag!(
            "open: {} file(s) handed over at +{} ms",
            files.len(),
            self.now_ms()
        );
        let taken = self.drops.groups(&files);
        for path in &files {
            let text = path.to_string_lossy();
            if !taken
                .iter()
                .any(|(_, _, paths)| paths.iter().any(|p| *p == text))
            {
                diag!("host.warning open: no plugin opens {text}");
            }
        }
        let modifiers = json!({"alt":false,"shift":false,"cmd":false});
        for (owner, id, paths) in taken {
            self.send(
                owner,
                json!({"event":"drops.drop","data":{"id":id,"paths":paths,"modifiers":modifiers}}),
            );
        }
    }
    fn bring_to_front(&mut self) {
        if let Some(shell) = &self.shell {
            crate::ui::view::raise(shell);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_wait_until_the_plugins_settled() {
        let mut opens = OpenFiles::default();
        assert_eq!(opens.due(true), None);
        opens.push(Vec::new());
        assert_eq!(opens.due(true), None, "nothing to open");
        opens.push(vec!["/a.mkv".into()]);
        opens.push(vec!["/b.mkv".into(), "/c.mp3".into()]);
        assert_eq!(opens.due(false), None, "plugins still starting");
        assert_eq!(opens.due(true), None, "more may follow");
        std::thread::sleep(QUIET);
        assert_eq!(
            opens.due(true),
            Some(vec!["/a.mkv".into(), "/b.mkv".into(), "/c.mp3".into()])
        );
        assert_eq!(opens.due(true), None);
        opens.push(vec!["/d.mkv".into()]);
        opens.since = Some((Instant::now() - SETTLE_LIMIT, Instant::now()));
        assert_eq!(
            opens.due(false),
            Some(vec!["/d.mkv".into()]),
            "waited long enough"
        );
    }
}
