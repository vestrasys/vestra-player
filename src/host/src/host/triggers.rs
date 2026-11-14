//! The Host's side of triggers (`crate::triggers`, events-and-triggers.md
//! §4.9, §4.10): the facts it hands the registry, the position model's
//! reports and boundaries (`playback/position_model.rs`), the clock, and the
//! delivery of `triggers.fired`: sent to a running plugin, held for one
//! that starts, a wake for one in standby (the "event" lane, merged per
//! trigger and edge), kept over the start-up mute.
use super::*;
use crate::conditions::Facts;
use crate::playback::position_model::Report;
use crate::supervisor::{Delivery, Pid, Trigger};
use crate::triggers::{self, Notification, Package};

/// Unix milliseconds now (the `time` atom's clock).
pub(crate) fn wall_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |d| d.as_secs_f64() * 1000.)
}

/// What the triggers of the Host keep beside the registry.
#[derive(Default)]
pub(crate) struct TriggerState {
    /// The boundaries last handed to the playback thread.
    pub watch: (u64, Vec<f64>),
    /// The last report of the position model seen here.
    pub serial: u64,
    /// The primary subtitle track (`Event::SubtitleTrack`).
    pub subtitle: Option<String>,
    /// Notifications the start-up mute kept from plugins not running, by
    /// plugin, as (merge key, message): delivered when it ends.
    pub muted: BTreeMap<String, Vec<(String, Value)>>,
}

impl Host {
    /// The facts conditions read, now: a trigger's and a command's state
    /// (`capabilities::refresh_conditions`) read the same.
    pub(crate) fn condition_facts(&self) -> Facts {
        let language = self.language_preference();
        let mut facts = Facts::new(
            &self.last_source,
            &self.window_facts,
            crate::ui::theme::known(self.theme()),
            language.as_deref().unwrap_or(i18n::SYSTEM),
        );
        facts.subtitle_track = self.trigger_state.subtitle.clone();
        facts
    }
    /// `source.state`, `window.state` or `appearance.changed` was broadcast,
    /// or the subtitle track changed: the conditions that read what changed
    /// are evaluated.
    pub(crate) fn triggers_facts(&mut self) {
        if self.triggers.is_empty() {
            return;
        }
        self.triggers_facts_now();
    }
    /// The registry's facts brought up to date whether or not a condition
    /// is kept: before a condition is given (`triggers.set`), so a range
    /// list binds to the current source and T3 reads the state now.
    pub(crate) fn triggers_facts_now(&mut self) {
        let facts = self.condition_facts();
        let source = self.last_source["stamp"]["generation"]
            .as_u64()
            .unwrap_or(0);
        let position = self.last_source["position"].as_f64();
        let out = self.triggers.set_facts(facts, source, position, wall_ms());
        self.deliver_fired(out);
    }
    /// The playback worker's primary subtitle track changed: commands whose
    /// state reads it (`when.subtitleTrack`) follow it, as do triggers.
    pub(crate) fn triggers_subtitle(&mut self, track: Option<String>) {
        self.trigger_state.subtitle = track;
        capabilities::refresh_conditions(self);
        self.triggers_facts();
    }
    /// The position model reported (`Event::Position`).
    pub(crate) fn triggers_position(&mut self, report: Report) {
        self.trigger_state.serial = self.trigger_state.serial.max(report.serial);
        let out =
            self.triggers
                .position(report.source, report.position, report.continuous, wall_ms());
        // A report disarms the model until it has the next boundaries:
        // they go again, changed or not.
        self.trigger_state.watch = (u64::MAX, Vec::new());
        self.deliver_fired(out);
    }
    /// Every UI tick: the time conditions whose bound the clock reached
    /// (one comparison of the clock while one is kept: the tick is the
    /// timer, and a changed system clock or a resume from sleep is seen at
    /// the next tick), the registry's diagnostic lines.
    pub(crate) fn triggers_tick(&mut self) {
        for line in self.triggers.diagnostics() {
            diag!("{line}");
        }
        if self.triggers.is_empty() {
            return;
        }
        let out = self.triggers.time(wall_ms());
        self.deliver_fired(out);
    }
    /// Hands the playback thread the boundaries the position conditions
    /// wait for, when they changed.
    pub(crate) fn sync_trigger_watch(&mut self) {
        let watch = self.triggers.watch();
        if watch == self.trigger_state.watch {
            return;
        }
        self.trigger_state.watch = watch.clone();
        let (source, boundaries) = watch;
        let _ = self.backend.commands.send(Command {
            reply: Reply { owner: 0, id: 0 },
            method: "position.watch".into(),
            params: json!({"source":source,"boundaries":boundaries,"after":self.trigger_state.serial}),
        });
    }
    /// Delivers `out` (design §4.9), then hands over the boundaries. A
    /// notification that cannot be delivered (no process to take it, its
    /// plugin disabled or failed, its backlog or the start lane full) is
    /// not lost silently behind "entered": the registry is told
    /// (`Triggers::undelivered`), writes a diagnostic line and un-latches
    /// it, so its next evaluation fires it again.
    pub(crate) fn deliver_fired(&mut self, out: Vec<Notification>) {
        for notification in out {
            let key = notification.merge_key();
            let data = notification.data(&self.last_source);
            let message = json!({"event":triggers::EVENT,"data":data});
            match self.fire(&notification.plugin, key, message) {
                Ok(()) => self
                    .triggers
                    .delivered(&notification.plugin, &notification.trigger),
                Err(why) => self.triggers.undelivered(&notification.plugin, &data, why),
            }
        }
        self.sync_trigger_watch();
    }
    /// One notification for `plugin`: to its process when it runs (the SDK
    /// holds one that comes before `activate()` declared its triggers; as a
    /// wake trigger to a running unit it counts as use and cancels a
    /// standby on its way, `Supervisor::delivering`); held for its ready
    /// while it starts, or for the next start while its process stops for
    /// standby; a wake in standby; kept over the start-up mute. `Err`: it
    /// could not be delivered, and why.
    fn fire(&mut self, plugin: &str, key: String, message: Value) -> Result<(), &'static str> {
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let running = self.supervisor.process_of(plugin);
        if let Some(pid) = running
            && self.plugins.contains_key(&pid.0)
            && self.supervisor.waking(pid).is_none()
        {
            self.supervisor.delivering(pid);
            self.fire_to(pid, message);
            return Ok(());
        }
        // A start without its process yet holds it under the holder it
        // replaces, as `wake` does (`Supervisor::trigger_pid`).
        let pid = running
            .or_else(|| self.supervisor.trigger_pid(plugin))
            .ok_or("it has no process to take it")?;
        let trigger = Trigger::event(key.clone(), message.clone());
        match self.supervisor.wake(pid, &trigger) {
            Delivery::Started => {
                self.wake_started(pid.0);
                Ok(())
            }
            Delivery::Held => Ok(()),
            Delivery::Now if self.plugins.contains_key(&pid.0) => {
                self.fire_to(pid, message);
                Ok(())
            }
            Delivery::Forward(to) if self.plugins.contains_key(&to.0) => {
                self.fire_to(to, message);
                Ok(())
            }
            Delivery::Dropped if !self.on_demand.caught_up && self.supervisor.events_muted() => {
                let held = self
                    .trigger_state
                    .muted
                    .entry(plugin.to_owned())
                    .or_default();
                held.retain(|(other, _)| *other != key);
                held.push((key, message));
                Ok(())
            }
            Delivery::Refused => Err("its backlog or the start lane is full"),
            Delivery::Now | Delivery::Forward(_) | Delivery::Dropped => {
                Err("its plugin is disabled, failed or stopping")
            }
        }
    }
    /// Sends `message` to running `pid`. A full queue stops the run as a
    /// fault, which evaluates its conditions again (T13).
    fn fire_to(&mut self, pid: Pid, message: Value) {
        let failed = self
            .plugins
            .get_mut(&pid.0)
            .and_then(|p| p.send(message).err());
        if let Some(error) = failed {
            self.transport_failed(pid.0, &error);
        }
    }
    /// The start-up mute ended (`wake_missed`): what is kept is evaluated
    /// once (T13), and what the mute kept is delivered.
    pub(crate) fn triggers_caught_up(&mut self) {
        let muted = std::mem::take(&mut self.trigger_state.muted);
        for (plugin, held) in muted {
            for (key, message) in held {
                let data = message["data"].clone();
                if let Err(why) = self.fire(&plugin, key, message) {
                    self.triggers.undelivered(&plugin, &data, why);
                }
            }
        }
        if !self.triggers.is_empty() {
            self.triggers_facts_now();
        }
        let out = self.triggers.evaluate_all(wall_ms());
        self.deliver_fired(out);
    }
    /// Installed plugin `id` was loaded: its kept conditions are read.
    pub(crate) fn triggers_load(&mut self, id: &str) {
        let instance = self.installed_instance(id);
        if instance.is_some() {
            self.triggers.load(id, instance);
        }
    }
    /// The instance of `owner`'s plugin before its first trigger request:
    /// an installed one keeps its conditions in its file, a development
    /// load in memory only.
    pub(crate) fn triggers_plugin(&mut self, owner: u64) -> Option<String> {
        let plugin = self.plugins.get(&owner)?;
        let id = plugin.manifest.id.clone();
        // The installed instance (as `triggers_load`), not the recording's
        // key: a run without one (a built-in listed without a digest) still
        // keeps its conditions in its instance's file.
        if !self.development.contains_key(&id) {
            self.triggers_load(&id);
        }
        Some(id)
    }
    /// A process of plugin `owner` was spawned: conditions given by another
    /// version of it lose their unnamed triggers (§4.2). An installed run
    /// whose version cannot be told (no recording key: a built-in listed
    /// without a digest, a run not from the record's current package) is
    /// not judged another version: its conditions stay as they are, and
    /// the version they were given by is not overwritten.
    pub(crate) fn triggers_spawned(&mut self, owner: u64) {
        let Some(plugin) = self.plugins.get(&owner) else {
            return;
        };
        let id = plugin.manifest.id.clone();
        let development = self.development.contains_key(&id);
        let package = self.checkpoint_key_of(owner).map(|key| Package {
            digest: key.digest,
            version: key.version,
        });
        self.triggers_load(&id);
        if package.is_none() && !development {
            return;
        }
        self.triggers.spawned(&id, package);
        // Conditions removed: the position model's boundaries follow.
        self.sync_trigger_watch();
    }
    /// Run `owner` of plugin `id` stops for `reason`: after a fault,
    /// "entered" is forgotten and what holds enters again (T13); a
    /// development reload is another version (§4.2). A park, an upgrade or
    /// a reload of a package keeps everything (`spawned` judges the next
    /// version); a disable or an uninstall is `triggers_clear`.
    pub(crate) fn triggers_stopped(&mut self, id: &str, reason: StopReason) {
        match reason {
            StopReason::Failed => {
                let out = self.triggers.fault(id, wall_ms());
                self.deliver_fired(out);
            }
            StopReason::Reload if self.development.contains_key(id) => self.triggers.reloaded(id),
            _ => {}
        }
    }
    /// The user disabled or uninstalled plugin `id`: its conditions go.
    pub(crate) fn triggers_clear(&mut self, id: &str) {
        self.triggers.clear(id);
        self.sync_trigger_watch();
    }
}
