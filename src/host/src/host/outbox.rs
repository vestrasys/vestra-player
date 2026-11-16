//! The core's outbox (plan step S2f(3); design §5.3, "After S2f"): what
//! the core asks of the interface, in the core's order, and the core work
//! that must follow the interface's part of those requests.
//!
//! The core pushes; it never carries a request out itself. The driver
//! drains the outbox after every handled event, after every report the
//! interface sends, and after every core step ([`Host::drain_outbox`]):
//! each request is carried out by the interface, whose reports the core
//! handles at once (their own requests drained right after, before the
//! next entry), and each [`Then`] is run by the core in its place. So on
//! one thread every request is carried out in the turn that issued it,
//! after the rest of the handler that issued it, and before the next event.
//!
//! Before plan step S2f(3) a request was carried out in place, in the
//! middle of the handler. Where today's wire showed a plugin line the
//! interface's part wrote (through its report) *before* a later line of the
//! same handler, that later line rides behind the request as a [`Then`]
//! ("the core's later line rides with the command", plan S2f(3)); the
//! sites are listed at [`Then`]. S3a decides each of them again (its
//! wire-trace class 3), and moves this queue between the threads.
use super::messages::{SettingsOp, UiCommand};
use super::*;

/// The core's outbox: the requests to the interface in the core's order
/// ([`Out`]), and what the interface's copies of core data need (plan step
/// S3a(1), design §5.3 group D): the changes of the mount registry the
/// core made itself ([`MountOp`]), and what of the core's facts, plugins
/// and settings pages the interface was last given ([`Copies`]). The driver
/// gives the interface its copies at the start of every drain, before the
/// requests, so a request's interface part reads the core's data as the
/// handler left it, as it did when it read the core's own (S2f(3)).
#[derive(Default)]
pub(crate) struct Outbox {
    queue: std::collections::VecDeque<Out>,
    /// The core's own changes of `Core::mounts`, in order, since the last
    /// drain.
    pub(crate) mount_ops: Vec<MountOp>,
    /// The batching window's changes the core cancelled whose cancel the
    /// interface has not confirmed by a closed window yet
    /// (`Core::cancel_handed`, `Core::batching`; plan step S3a(2)).
    pub(crate) cancelled: Vec<super::mounts::HandedMount>,
    /// What the interface was last given (`Core::copies`).
    pub(crate) sent: Sent,
}

impl std::ops::Deref for Outbox {
    type Target = std::collections::VecDeque<Out>;
    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}
impl std::ops::DerefMut for Outbox {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.queue
    }
}

/// A change of the mount registry the core made itself (not one the
/// interface applied and reported): the interface's copy
/// (`Host::mounts`) follows it. The interface's copy keeps the values its
/// panels were given since (`ui.setMount`: the stored properties, and the
/// bottom bar's `visible`), which only the interface reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MountOp {
    /// The panel went.
    Remove(u64),
    /// The panel `from` is now `to`, of plugin `plugin` (a rekey).
    Move { from: u64, to: u64, plugin: u64 },
    /// The panel stays as a placeholder (its plugin parked).
    Recorded(u64),
}

/// What the interface was last given of the core's data (`Core::copies`).
#[derive(Default)]
pub(crate) struct Sent {
    facts: Option<CoreFacts>,
    tick: Option<TickFacts>,
    plugins: Option<(u64, u64)>,
    pages: Option<u64>,
}

/// Small core values the interface reads for drawing and for its own
/// checks (design §5.3 group D): the interface's copy is `Host::view`.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct CoreFacts {
    /// A native dialog is open (`Core::dialog_busy`).
    pub(crate) dialog_busy: bool,
    /// The palette's lead (`Core::lead`).
    pub(crate) lead: Option<crate::capabilities::Lead>,
    /// The `commands.run` tickets and the runs waiting for the sheet.
    pub(crate) runs: crate::capabilities::RunsView,
    /// Where the user last acted for each plugin (`Core::origins`).
    pub(crate) origins: origins::Origins,
}

/// What tells the interface that a report it sends every window tick would
/// change nothing in the core (plan step S3a(2), change-only reports): each
/// is the state the core's handler of that report returns early on, as of
/// the last drain (the core's state changes only in a core call, and every
/// core call is drained). The interface skips such a report
/// (`Host::tick_report_needed`); the reports it sends still reach the core
/// at today's points, so the core's answers are today's.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TickFacts {
    /// A `window.setVideoArea` reply waits (`Core::area_replies`;
    /// `UiReport::VideoArea`).
    pub(crate) area_waiting: bool,
    /// A request waits for the built-ins to start (`Core::held_mounts`;
    /// `UiInput::MountTick`).
    pub(crate) held_mounts: bool,
    /// The runs whose declared work has `Busy::ShownTab` already
    /// (`UiInput::TabWork`).
    pub(crate) shown_tab: std::collections::BTreeSet<u64>,
    /// The host's Toggle Fullscreen command's checked state, and whether
    /// mpv was told not to show cover art (`UiInput::WindowTick`).
    pub(crate) fullscreen_checked: Option<bool>,
    pub(crate) cover_art_off: bool,
    /// The bottom bar's column toggles' states in the registry, sidebar
    /// then adjust (`UiInput::ColumnCommands`).
    pub(crate) columns: [Option<commands::CommandState>; 2],
}

/// What the interface knows of the running and the held plugins: their
/// ids, names and packages (`Host::plugins`), as of the core's
/// `plugins_revision` and `held_revision`.
#[derive(Default)]
pub(crate) struct PluginsView {
    /// Per running plugin: its manifest id and its shown name.
    pub(crate) running: BTreeMap<u64, (String, String)>,
    /// The packages of the running plugins and, after them, of the held
    /// ones (each image read resolves its roots from these, as before).
    pub(crate) origins: BTreeMap<u64, crate::host::Origin>,
    pub(crate) held: BTreeMap<u64, crate::host::Origin>,
    /// Shown names by manifest id: the running plugins', and the held
    /// ones' (`OnDemand::name_of`).
    pub(crate) names: BTreeMap<String, String>,
    pub(crate) held_names: BTreeMap<String, String>,
}

impl PluginsView {
    /// Debug builds: what two copies are compared by (the packages by
    /// their id and directories; a catalog is shared, not compared).
    #[cfg(debug_assertions)]
    #[allow(clippy::type_complexity)]
    pub(crate) fn shape(
        &self,
    ) -> (
        &BTreeMap<u64, (String, String)>,
        Vec<(u64, &str, &Path, Option<&Path>)>,
        Vec<(u64, &str, &Path, Option<&Path>)>,
        &BTreeMap<String, String>,
        &BTreeMap<String, String>,
    ) {
        fn packages(
            origins: &BTreeMap<u64, crate::host::Origin>,
        ) -> Vec<(u64, &str, &Path, Option<&Path>)> {
            origins
                .iter()
                .map(|(owner, o)| {
                    (
                        *owner,
                        o.id.as_str(),
                        o.directory.as_path(),
                        o.data_directory.as_deref(),
                    )
                })
                .collect()
        }
        (
            &self.running,
            packages(&self.origins),
            packages(&self.held),
            &self.names,
            &self.held_names,
        )
    }
    /// `plugin_id`: the manifest id of a running plugin, else the owner.
    pub(crate) fn plugin_id(&self, owner: u64) -> String {
        self.running
            .get(&owner)
            .map_or_else(|| owner.to_string(), |(id, _)| id.clone())
    }
    /// `shown_name`: the shown name of a running plugin, else the owner.
    pub(crate) fn shown_name(&self, owner: u64) -> String {
        self.running
            .get(&owner)
            .map_or_else(|| owner.to_string(), |(_, name)| name.clone())
    }
    /// `Core::image_roots`, from the copies.
    pub(crate) fn image_roots(&self, owner: u64) -> Vec<PathBuf> {
        if let Some(origin) = self.origins.get(&owner) {
            return origin.image_roots();
        }
        self.held
            .get(&owner)
            .map(|origin| origin.image_roots())
            .unwrap_or_default()
    }
    /// A plugin's shown name by manifest id (the palette's rows): a
    /// running plugin's, a held one's, else the id.
    pub(crate) fn name_of(&self, id: &str) -> String {
        self.names
            .get(id)
            .or_else(|| self.held_names.get(id))
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    }
}

/// What changed of the core's data since the interface's copies were last
/// given (`Core::copies`).
#[derive(Default)]
pub(crate) struct Copies {
    pub(crate) facts: Option<CoreFacts>,
    pub(crate) tick: Option<TickFacts>,
    pub(crate) plugins: Option<PluginsView>,
    pub(crate) pages: Option<Vec<crate::ui::settings_window::Page>>,
    pub(crate) mount_ops: Vec<MountOp>,
}

impl Core {
    /// What the interface's copies need (plan step S3a(1)): the core's
    /// changes of the mount registry since the last call, and each copied
    /// value that differs from what the interface was last given.
    pub(crate) fn copies(&mut self) -> Copies {
        let mut copies = Copies {
            mount_ops: std::mem::take(&mut self.outbox.mount_ops),
            ..Copies::default()
        };
        // Compared before anything is copied: most drains change none.
        let same = self.outbox.sent.facts.as_ref().is_some_and(|sent| {
            sent.dialog_busy == self.dialog_busy
                && sent.lead == self.lead
                && sent.runs.matches(&self.command_runs)
                && sent.origins == self.origins
        });
        if !same {
            let facts = self.facts_now();
            self.outbox.sent.facts = Some(facts.clone());
            copies.facts = Some(facts);
        }
        let tick = self.tick_facts();
        if self.outbox.sent.tick.as_ref() != Some(&tick) {
            self.outbox.sent.tick = Some(tick.clone());
            copies.tick = Some(tick);
        }
        let plugins = (self.plugins_revision, self.on_demand.held_revision());
        if self.outbox.sent.plugins != Some(plugins) {
            self.outbox.sent.plugins = Some(plugins);
            copies.plugins = Some(self.plugins_view());
        }
        let pages = self.settings.pages_revision();
        if self.outbox.sent.pages != Some(pages) {
            self.outbox.sent.pages = Some(pages);
            copies.pages = Some(self.settings.pages().to_vec());
        }
        copies
    }
    /// The interface's copy of a settings page's stored values was written
    /// by the interface itself (`settings.set`), and the core's was written
    /// from its report just now: the interface need not be given the pages
    /// again for that write alone.
    pub(crate) fn pages_echoed(&mut self, before: u64) {
        if self.outbox.sent.pages == Some(before) {
            self.outbox.sent.pages = Some(self.settings.pages_revision());
        }
    }
    /// What makes the interface's per-tick reports change nothing
    /// ([`TickFacts`]).
    pub(crate) fn tick_facts(&self) -> TickFacts {
        let state = |id: &str| {
            self.commands
                .get(crate::bindings::HOST, id)
                .map(|entry| entry.state.clone())
        };
        TickFacts {
            area_waiting: !self.area_replies.is_empty(),
            held_mounts: !self.held_mounts.is_empty(),
            shown_tab: self.on_demand.shown_tab_owners(),
            fullscreen_checked: state("fullscreen").and_then(|state| state.checked),
            cover_art_off: self.cover_art_off,
            columns: [
                crate::ui::bar_buttons::SIDEBAR,
                crate::ui::bar_buttons::ADJUST,
            ]
            .map(state),
        }
    }
    /// The core's small values as the interface copies them.
    pub(crate) fn facts_now(&self) -> CoreFacts {
        CoreFacts {
            dialog_busy: self.dialog_busy,
            lead: self.lead.clone(),
            runs: self.command_runs.view(),
            origins: self.origins.clone(),
        }
    }
    /// What the interface copies of the running and the held plugins.
    pub(crate) fn plugins_view(&self) -> PluginsView {
        let mut view = PluginsView::default();
        for (owner, plugin) in &self.plugins {
            let origin = plugin.origin();
            let name = package_manager::display_name(&plugin.manifest, origin.catalog.as_deref());
            view.names.insert(plugin.manifest.id.clone(), name.clone());
            view.running
                .insert(*owner, (plugin.manifest.id.clone(), name));
            view.origins.insert(*owner, origin);
        }
        for (owner, held) in self.on_demand.held_all() {
            view.held.insert(*owner, held.origin.clone());
        }
        view.held_names = self.on_demand.names().clone();
        view
    }
}

/// One entry of the outbox.
pub(crate) enum Out {
    /// A request of the core to the interface (rule R3).
    Ui(UiCommand),
    /// Core work that follows the interface's part of the requests before
    /// it.
    Then(Then),
}

/// Core work kept behind the requests issued before it (one thread only:
/// plan step S2f(3)). Each use keeps a wire order of today's:
pub(crate) enum Then {
    /// A reply after the lines the interface's part of a request before it
    /// writes: `commands.run` that ran a host command whose interface part
    /// lays the window out (`LayoutOp::Toggle` runs a window tick, whose
    /// `window.state` and video-area replies went out before the run's
    /// reply), at once or after the sheet's Run.
    Respond {
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
    },
    /// `settings.register` of a recorded page again (`SettingsOp::Updated`):
    /// the open window's `ui.error` lines come first (design §5.1 note 11),
    /// then the refused initial values' `ui.error` lines and the reply.
    Registered {
        reply: Reply,
        refused: Vec<RpcError>,
    },
    /// What a process said while its spawn was not answered yet
    /// (`spawned`): each event handled after the requests of the ones
    /// before it were carried out, as when each was handled alone.
    Event(Event),
    /// The supervisor's pending jobs, after the held events (`spawned`).
    SupervisorRun,
}

impl Core {
    /// A request to the interface (rule R3): into the outbox, carried out
    /// when the driver drains it (plan step S2f(3)).
    pub(crate) fn ui_command(&mut self, command: UiCommand) {
        self.outbox.push_back(Out::Ui(command));
    }
    /// Core work kept behind the requests issued so far ([`Then`]).
    pub(crate) fn then(&mut self, then: Then) {
        self.outbox.push_back(Out::Then(then));
    }
    /// A request waits in the outbox whose interface part writes a plugin
    /// line through its report ([`writes_lines`]): a line of the core's
    /// that followed it on today's wire rides behind it ([`Then`]).
    pub(crate) fn lines_pending(&self) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(command) if writes_lines(command)))
    }
    /// Debug builds: where core code reads its copy of the interface's
    /// facts `read`, no request that changes them waits in the outbox. The
    /// copy is the interface's as of the last drain, so it is then the
    /// interface's now, as when every request was carried out in place
    /// (plan step S2f(3)).
    #[cfg(debug_assertions)]
    pub(crate) fn check_settled(&self, read: &[Fact]) {
        for &fact in read {
            debug_assert!(
                !self.outbox.iter().any(|out| match out {
                    // The core's copy dropped that confirmation already
                    // (`UiDialog::revoke`): a drop that finds none changes
                    // nothing.
                    Out::Ui(UiCommand::DropConfirm { owner }) => {
                        matches!(fact, Fact::Sheet)
                            && self
                                .ui_facts
                                .sheet
                                .is_some_and(|sheet| sheet.owner == *owner && sheet.confirm)
                    }
                    Out::Ui(command) => changes(command, fact),
                    Out::Then(_) => false,
                }),
                "the core reads {fact:?} while a request that changes it waits"
            );
        }
    }
    /// Debug builds: a `DropConfirm` of `owner` waits in the outbox (the
    /// core's copy of the sheet is ahead of the interface's until it is
    /// carried out, `UiDialog::revoke`).
    #[cfg(debug_assertions)]
    pub(crate) fn drop_pending(&self, owner: u64) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(UiCommand::DropConfirm { owner: o }) if *o == owner))
    }
    /// Debug builds: a cancel of the batching window's change waits in the
    /// outbox (the core cleared its record already; the window drops it
    /// when the request is carried out).
    #[cfg(debug_assertions)]
    pub(crate) fn cancel_pending(&self) -> bool {
        self.outbox
            .iter()
            .any(|out| matches!(out, Out::Ui(UiCommand::CancelMount(_))))
    }
}

/// Whether the interface's part of `command` writes a plugin line through
/// its report outside a failure of its own (a Slint call that fails): the
/// UI-bound replies (`UiReport::Answered`, with `layout.set`'s
/// `window.state`), the window tick a column toggle runs, the open options
/// window's `ui.error` lines of a page registered again. A column toggle
/// and `layout.set` can also write another plugin's `ui.error`: a panel
/// they bring into view whose file does not compile is reported to its
/// plugin then (`show_panel`).
pub(crate) fn writes_lines(command: &UiCommand) -> bool {
    use super::messages::{LayoutOp, WindowOp};
    matches!(
        command,
        UiCommand::View(_)
            | UiCommand::SetMount(_)
            | UiCommand::Settings(
                SettingsOp::Updated { .. } | SettingsOp::Set { .. } | SettingsOp::Show { .. }
            )
            | UiCommand::Layout(LayoutOp::Set { .. } | LayoutOp::Toggle(_))
            | UiCommand::Window(WindowOp::VideoArea { .. } | WindowOp::Pip { .. })
            | UiCommand::Confirm(_)
            | UiCommand::Ask(_)
    )
}

/// What core code reads of the interface's facts (`UiFacts`) and tabs
/// (`WindowFacts`), for [`Core::check_settled`].
#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fact {
    /// The approval sheet (`UiFacts::sheet`).
    Sheet,
    /// The options window open (`UiFacts::options_open`).
    Options,
    /// The columns' policy facts (`UiFacts::columns`).
    Columns,
    /// No video (`UiFacts::no_video`).
    NoVideo,
    /// Whether the OSD message's buttons are this plugin's
    /// (`UiFacts::osd_buttons`).
    OsdButtons(u64),
    /// The panel policy's tabs (`Core::panel_tabs`).
    Tabs,
    /// The floating window's panels (`UiFacts::floating`).
    Floating,
    /// Whether this plugin has a window of its own (`UiFacts::views`).
    Views(u64),
    /// Picture-in-picture offered and shown (`UiFacts::pip`).
    Pip,
    /// Whether this plugin's picture-in-picture request is in effect
    /// (`PipFact::holder`).
    PipHolder(u64),
}

/// Whether carrying out `command` can change `fact`.
#[cfg(debug_assertions)]
fn changes(command: &UiCommand, fact: Fact) -> bool {
    use super::messages::{UiSnapshot, WindowOp};
    let revokes =
        |plugin: u64| matches!(command, UiCommand::RevokeOwner { owner, .. } if *owner == plugin);
    match fact {
        Fact::Sheet => matches!(
            command,
            UiCommand::Review(_) | UiCommand::Ask(_) | UiCommand::DropConfirm { .. }
        ),
        // Only `settings.show` opens the options window (`settings_show`);
        // it closes only on its own close button (`settings_event`, input,
        // not a command). The other settings commands mark an open window
        // stale or write into it and leave `options_open` as it was.
        Fact::Options => matches!(command, UiCommand::Settings(SettingsOp::Show { .. })),
        Fact::Columns | Fact::Tabs | Fact::Floating => {
            matches!(command, UiCommand::Layout(_) | UiCommand::Panels(_))
        }
        Fact::NoVideo => matches!(command, UiCommand::Snapshot(UiSnapshot::Source(_))),
        Fact::OsdButtons(plugin) => matches!(command, UiCommand::Osd(_)) || revokes(plugin),
        Fact::Views(plugin) => {
            matches!(command, UiCommand::View(op) if op.reply.owner == plugin) || revokes(plugin)
        }
        Fact::Pip => matches!(
            command,
            UiCommand::Window(WindowOp::Pip { .. }) | UiCommand::RevokeOwner { .. }
        ),
        Fact::PipHolder(plugin) => {
            matches!(command, UiCommand::Window(WindowOp::Pip { reply, .. }) if reply.owner == plugin)
                || revokes(plugin)
        }
    }
}

impl Core {
    /// `reply` with `result`, now or, while a request whose interface part
    /// writes lines waits ([`Core::lines_pending`]), behind it
    /// ([`Then::Respond`]).
    pub(crate) fn respond_in_order(
        &mut self,
        reply: Reply,
        result: std::result::Result<Value, RpcError>,
    ) {
        if self.lines_pending() {
            self.then(Then::Respond { reply, result });
        } else {
            self.respond(reply, result);
        }
    }
    /// The core's [`Then`].
    pub(crate) fn run_then(&mut self, then: Then) {
        match then {
            Then::Respond { reply, result } => self.respond(reply, result),
            Then::Registered { reply, refused } => {
                self.report_refused(reply.owner, refused);
                self.respond(reply, Ok(Value::Null));
            }
            Then::Event(event) => self.handle(event),
            Then::SupervisorRun => self.supervisor_run(),
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    /// Plan step S2f(3) review: of the settings commands only `Show` can
    /// change the options window's open fact, so a stop's `OwnerGone` or a
    /// wake's `Rekeyed` queued before a supervisor batch's busy check does
    /// not read as an unsettled `Fact::Options`.
    #[test]
    fn only_settings_show_changes_the_options_fact() {
        let reply = |id| Reply { owner: 7, id };
        let show = UiCommand::Settings(SettingsOp::Show {
            reply: reply(1),
            params: json!({}),
        });
        assert!(changes(&show, Fact::Options));
        for command in [
            UiCommand::Settings(SettingsOp::Removed {
                owner: 7,
                id: "main".into(),
            }),
            UiCommand::Settings(SettingsOp::Set {
                reply: reply(2),
                params: json!({}),
                prepared: None,
            }),
            UiCommand::Settings(SettingsOp::OwnerGone { owner: 7 }),
            UiCommand::Settings(SettingsOp::Rekeyed {
                serials: vec![1],
                to: 8,
            }),
            UiCommand::DropConfirm { owner: 7 },
        ] {
            assert!(!changes(&command, Fact::Options));
        }
    }
}
