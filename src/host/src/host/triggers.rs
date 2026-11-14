//! The Host's side of triggers (`crate::triggers`, events-and-triggers.md
//! §4.9, §4.10): the facts it hands the registry, the position model's
//! reports and boundaries (`playback/position_model.rs`), the clock, and the
//! delivery of `triggers.fired`: sent to a running plugin, held for one
//! that starts, a wake for one in standby (the "event" lane, merged per
//! trigger and edge), kept over the start-up mute.
use super::*;
use crate::conditions::Facts;
use crate::playback::position_model::Report;
use crate::supervisor::{Delivery, Trigger};
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
    /// The facts conditions read, now.
    fn trigger_facts(&self) -> Facts {
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
        let facts = self.trigger_facts();
        let source = self.last_source["stamp"]["generation"]
            .as_u64()
            .unwrap_or(0);
        let position = self.last_source["position"].as_f64();
        let out = self.triggers.set_facts(facts, source, position, wall_ms());
        self.deliver_fired(out);
    }
    /// The playback worker's primary subtitle track changed.
    pub(crate) fn triggers_subtitle(&mut self, track: Option<String>) {
        self.trigger_state.subtitle = track;
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
    /// Delivers `out` (design §4.9), then hands over the boundaries.
    pub(crate) fn deliver_fired(&mut self, out: Vec<Notification>) {
        for notification in out {
            let key = notification.merge_key();
            let message = json!({"event":triggers::EVENT,
                "data":notification.data(&self.last_source)});
            self.fire(&notification.plugin, key, message);
        }
        self.sync_trigger_watch();
    }
    /// One notification for `plugin`: to its process when it runs (the SDK
    /// holds one that comes before `activate()` declared its triggers);
    /// held for its ready while it starts; a wake in standby; kept over the
    /// start-up mute; dropped for a plugin that is disabled or failed.
    fn fire(&mut self, plugin: &str, key: String, message: Value) {
        let pid = self
            .supervisor
            .process_of(plugin)
            .or_else(|| self.supervisor.holder_of(plugin));
        let Some(pid) = pid else {
            return;
        };
        if self.plugins.contains_key(&pid.0) && self.supervisor.waking(pid).is_none() {
            let failed = self
                .plugins
                .get_mut(&pid.0)
                .and_then(|p| p.send(message).err());
            if let Some(error) = failed {
                self.transport_failed(pid.0, &error);
            }
            return;
        }
        let now = self.now_ms();
        self.supervisor.set_now(now);
        let trigger = Trigger::event(key.clone(), message.clone());
        match self.supervisor.wake(pid, &trigger) {
            Delivery::Started => self.wake_started(pid.0),
            Delivery::Dropped if !self.on_demand.caught_up && self.supervisor.events_muted() => {
                let held = self
                    .trigger_state
                    .muted
                    .entry(plugin.to_owned())
                    .or_default();
                held.retain(|(other, _)| *other != key);
                held.push((key, message));
            }
            _ => {}
        }
    }
    /// The start-up mute ended (`wake_missed`): what is kept is evaluated
    /// once (T13), and what the mute kept is delivered.
    pub(crate) fn triggers_caught_up(&mut self) {
        let muted = std::mem::take(&mut self.trigger_state.muted);
        for (plugin, held) in muted {
            for (key, message) in held {
                self.fire(&plugin, key, message);
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
        let instance = self
            .checkpoint_key_of(owner)
            .map(|key| key.guid)
            .filter(|_| !self.development.contains_key(&id));
        if instance.is_some() {
            self.triggers.load(&id, instance);
        }
        Some(id)
    }
    /// A process of plugin `owner` was spawned: conditions given by another
    /// version of it lose their unnamed triggers (§4.2).
    pub(crate) fn triggers_spawned(&mut self, owner: u64) {
        let Some(plugin) = self.plugins.get(&owner) else {
            return;
        };
        let id = plugin.manifest.id.clone();
        let package = self.checkpoint_key_of(owner).map(|key| Package {
            digest: key.digest,
            version: key.version,
        });
        self.triggers_load(&id);
        self.triggers.spawned(&id, package);
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
