//! Mounted panels: preparing the shell for mount changes, their values
//! and callbacks.
use super::messages::{
    MountApplied, MountCancel, SetMount, UiCommand, UiReport, UiSnapshot, WindowFacts, WindowOp,
};
use super::*;
use crate::ui::sidebar_tabs::Column;

/// What `apply_mounts` decided for one column of tabs.
struct ColumnPlan {
    /// Its tabs in tab order.
    order: Vec<u64>,
    /// The tab that will be current.
    active: Option<u64>,
    /// The current tab is leaving.
    current_left: bool,
    /// It draws its tab strip.
    tabbed: bool,
}

/// A column's current tab for the host (`Host::current_in`, the start-up
/// rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Current {
    /// This tab (`None`: the column has none).
    Tab(Option<u64>),
    /// Start-up has not settled and the column's remembered tab has not
    /// mounted yet: no tab is current.
    Awaits,
}
impl Current {
    pub(crate) fn tab(self) -> Option<u64> {
        match self {
            Current::Tab(tab) => tab,
            Current::Awaits => None,
        }
    }
    pub(crate) fn awaits(self) -> bool {
        self == Current::Awaits
    }
}

/// The wire name of a panel key's slot.
fn slot_of(key: u64) -> &'static str {
    [
        composition::Slot::Bottom,
        composition::Slot::Sidebar,
        composition::Slot::Backdrop,
        composition::Slot::Adjust,
    ]
    .into_iter()
    .find(|&slot| panel_key(panel_plugin(key), slot) == key)
    .map_or("bottom", composition::Slot::name)
}

/// The policy's slot of a column.
pub(crate) fn policy_column(column: Column) -> window_state::Slot {
    match column {
        Column::Sidebar => window_state::Slot::Sidebar,
        Column::Adjust => window_state::Slot::Adjust,
    }
}

/// Initial properties of a panel or settings page are converted when the
/// view is built. A value over the limits is left out now (the property
/// keeps its default) and returned, to be reported (`report_refused`),
/// rather than failing the mount: a mount failing during activation would
/// fail the whole plugin.
pub(crate) fn refuse_initial(properties: &mut BTreeMap<String, Value>) -> Vec<RpcError> {
    let mut refused = Vec::new();
    properties.retain(|name, value| match RpcError::check_property(name, value) {
        Ok(()) => true,
        Err(error) => {
            refused.push(error);
            false
        }
    });
    refused
}

impl Core {
    /// The core's side of `UiCommand::Mount` (K6; plan step S2c(3)): its
    /// oldest accepted mount change, handed to the interface's batching
    /// window. Until plan step S3a the interface takes it in place, at the
    /// point it took it from the queue before (`prepare_mount_tick`).
    /// The core records the change it hands over (`Core::handed`, plan step
    /// S2d(2)): until the interface reports its window closed
    /// (`UiReport::Batching`), the change still counts as pending for the
    /// core's own checks.
    fn mount_command(&mut self) -> Option<UiCommand> {
        let change = self.mount_queue.pop_front()?;
        self.handed = Some(HandedMount::of(&change));
        Some(UiCommand::Mount(change))
    }
    /// The change of panel `key` the interface's batching window holds,
    /// as the core recorded it (`handed`), or one in the core's queue.
    pub(crate) fn mount_pending(&self, key: u64) -> bool {
        self.mount_queue.iter().any(|c| c.key == key)
            || self.handed.as_ref().is_some_and(|h| h.key == key)
    }
    /// Every accepted mount change, oldest first, for the batching window
    /// about to close (a `UiCommand::Mount` each).
    fn mount_commands(&mut self) -> Vec<UiCommand> {
        self.mount_queue.drain(..).map(UiCommand::Mount).collect()
    }
}

/// What the core records of the mount change the interface's batching
/// window holds (plan step S2d(2); `Core::handed`): enough for the core's
/// own checks, which read it instead of the interface's window (a change
/// still waiting there, the slot it claims, the call a recording keeps,
/// the host's own removal of a panel).
#[derive(Clone)]
pub(crate) struct HandedMount {
    pub(crate) reply: Reply,
    /// The panel (`panel_key`).
    pub(crate) key: u64,
    /// A mount's slot; `None`: an unmount.
    pub(crate) slot: Option<composition::Slot>,
    /// A mount's call as a recording keeps it (`Mount::call`); `Null` for
    /// an unmount.
    pub(crate) call: Value,
}
impl HandedMount {
    pub(crate) fn of(change: &MountChange) -> Self {
        Self {
            reply: change.reply.clone(),
            key: change.key,
            slot: change.contribution.as_ref().map(|m| m.spec.slot),
            call: change
                .contribution
                .as_ref()
                .map_or(Value::Null, |m| m.call.clone()),
        }
    }
    /// The host's own removal of panel `key` (an unmount with id 0), the
    /// change a plugin's own mount of that panel replaces.
    pub(crate) fn removes(&self, key: u64) -> bool {
        self.key == key && self.reply.id == 0 && self.slot.is_none()
    }
    /// The same change (debug check of the record against the window).
    fn same(&self, other: &Self) -> bool {
        self.reply.owner == other.reply.owner
            && self.reply.id == other.reply.id
            && self.key == other.key
            && self.slot == other.slot
            && self.call == other.call
    }
}

/// The change a `UiCommand::Mount` carries.
fn mount_change(command: UiCommand) -> MountChange {
    match command {
        UiCommand::Mount(change) => change,
        _ => unreachable!("the core hands the batching window only mount changes"),
    }
}

/// The changes `UiReport::MountPreparing` names: those not announced before
/// (a failed batch's changes come back announced).
fn unannounced<'a>(changes: impl IntoIterator<Item = &'a MountChange>) -> Vec<(Reply, u64)> {
    changes
        .into_iter()
        .filter(|change| !change.announced)
        .map(|change| (change.reply.clone(), change.key))
        .collect()
}

impl Host {
    /// The interface's mount batching window and the apply (K6, plan step
    /// S2c(3)). The core hands over its accepted changes as
    /// `UiCommand::Mount`; the interface reports `UiReport::MountPreparing`
    /// when changes leave its queue (R6: their `ui.preparing` goes out
    /// then) and `UiReport::MountApplied` once a batch was applied (R13:
    /// the core applies the state it carries, then answers). Until plan
    /// step S3a both run on this thread: each message is delivered in
    /// place, at the point today's code did its work, so order and timing
    /// are unchanged.
    pub(crate) fn prepare_mount_tick(&mut self) {
        // The core answers what it held first (plan step S2f(1)).
        self.ui_report(UiReport::Input(messages::UiInput::MountTick));
        if self.preparing.is_none() {
            if let Some(change) = self.core.mount_command().map(mount_change) {
                let preparing = unannounced([&change]);
                // Handed over, not yet in the window: its `ui.preparing` goes
                // out first (a full pipe can stop its plugin right there,
                // `transport_failed`, and `stop_run` cancels the record), so
                // the debug check of the record waits until the window has it.
                #[cfg(debug_assertions)]
                {
                    self.handing_over = true;
                }
                if !preparing.is_empty() {
                    self.ui_report(UiReport::MountPreparing(preparing));
                }
                #[cfg(debug_assertions)]
                {
                    self.handing_over = false;
                }
                self.set_preparing(Some((Instant::now(), change)));
            }
            return;
        }
        // A short wait, so that mounts arriving together (the plugins'
        // activations at start-up) are applied as one change. Nothing waits
        // for plugins that have not asked yet: the shell never compiles
        // again, and each panel compiles on its own when it is first shown
        // (`ui/panels.rs`), so a late mount costs only its own panel.
        let waited = self.preparing.as_ref().unwrap().0.elapsed();
        if waited < Duration::from_millis(32) {
            return;
        }
        let (_, change) = self.preparing.take().unwrap();
        // The window closed: its change is applied now, not held.
        self.ui_report(UiReport::Batching(None));
        let started = Instant::now();
        // Every change queued meanwhile joins this one. After a failed
        // batch its changes go one at a time again, so that one bad
        // component fails only its own mount.
        let mut changes = vec![change];
        if self.mount_singly > 0 {
            self.mount_singly -= 1;
        } else {
            let joining = self
                .core
                .mount_commands()
                .into_iter()
                .map(mount_change)
                .collect::<Vec<_>>();
            let preparing = unannounced(&joining);
            if !preparing.is_empty() {
                self.ui_report(UiReport::MountPreparing(preparing));
            }
            changes.extend(joining);
        }
        // A mount of a plugin stopped since is dropped, as before; a tab
        // replayed from a recording belongs to a plugin in standby, which has
        // no process (dropped when it is no longer held either).
        changes.retain(|change| {
            let owner = change.reply.owner;
            match &change.contribution {
                None => true,
                Some(mount) if mount.recorded => {
                    self.core.plugins.contains_key(&owner)
                        || self.core.on_demand.held(owner).is_some()
                }
                Some(_) => self.core.plugins.contains_key(&owner),
            }
        });
        if changes.is_empty() {
            return;
        }
        let mut next = self.core.mounts.clone();
        for change in &changes {
            let owner = change.reply.owner;
            if let Some(mount) = &change.contribution {
                // Mounted after all: an earlier refusal no longer applies.
                self.core.clear_warning(owner, "slot-occupied");
                next.insert(change.key, mount.clone());
            } else {
                next.remove(&change.key);
            }
        }
        let batch = changes.iter().map(|c| c.key).collect::<Vec<_>>();
        let result = self.apply_mounts(next, &batch);
        self.log_prepared(changes.len(), started, result.is_ok());
        if result.is_err() && changes.len() > 1 {
            // Again one at a time, from the next tick on: the first, then
            // the rest in their order.
            self.mount_singly = changes.len();
            let mut changes = changes.into_iter();
            let first = changes.next().expect("more than one change");
            // The queue is the core's: it puts them back at its front, in
            // their order (plan step S2f(1), design §5.3 group E).
            let rest = changes
                .map(|mut change| {
                    change.announced = true;
                    change
                })
                .collect();
            self.ui_report(UiReport::Input(messages::UiInput::MountRetry(rest)));
            let due = Instant::now()
                .checked_sub(Duration::from_millis(32))
                .unwrap_or_else(Instant::now);
            self.set_preparing(Some((due, first)));
            return;
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        // One report per change, in batch order, each right after the
        // change's own interface work (its restart line): the interleaving
        // of the two is today's, and a reply's effects (an activation that
        // completes, R12) come before the next change's restart line, as
        // before. The first carries the R13 handoff.
        let mut first = true;
        for change in changes {
            // A panel kept over its plugin's restart, mounted again (from
            // its compiled file when that did not change) or removed: its
            // restart line goes (`panel_restart.rs`).
            if result.is_ok() && self.restart_waiting(change.key) {
                if let Some(mount) = &change.contribution {
                    diag!(
                        "shell: {} mounted its {} panel again after its restart ({})",
                        mount.manifest,
                        mount.spec.slot.name(),
                        if self.compiled_now.contains(&change.key) {
                            "compiled"
                        } else {
                            "no compile"
                        }
                    );
                }
                self.ui_report(UiReport::Input(messages::UiInput::PanelBack(change.key)));
            }
            let placeholders = change
                .contribution
                .as_ref()
                .filter(|mount| mount.recorded)
                .map(|_| change.key)
                .into_iter()
                .collect();
            let handoff = if std::mem::take(&mut first) {
                self.facts_handoff()
            } else {
                None
            };
            self.ui_report(UiReport::MountApplied(MountApplied {
                batch: vec![(change.reply, change.key)],
                result: result.clone(),
                elapsed_ms: elapsed,
                handoff,
                placeholders,
            }));
        }
    }
    /// The interface's batching window now holds `preparing` (plan step
    /// S2d(2)): the core is told which change it holds, or none
    /// (`UiReport::Batching`), and reads that record (`Core::handed`)
    /// instead of the window.
    pub(crate) fn set_preparing(&mut self, preparing: Option<(Instant, MountChange)>) {
        let held = preparing
            .as_ref()
            .map(|(_, change)| HandedMount::of(change));
        self.preparing = preparing;
        self.ui_report(UiReport::Batching(held));
    }
    /// The interface's side of `UiCommand::CancelMount` (plan step S2d(2)):
    /// the change the batching window holds is dropped when it is the one
    /// the core cancels.
    pub(crate) fn cancel_mount(&mut self, cancel: MountCancel) {
        let dropped = self
            .preparing
            .as_ref()
            .is_some_and(|(_, change)| match cancel {
                MountCancel::Removal { key } => {
                    change.key == key && change.reply.id == 0 && change.contribution.is_none()
                }
                MountCancel::Owner(owner) => change.reply.owner == owner,
            });
        if dropped {
            self.set_preparing(None);
        }
    }
    /// Debug builds: the core's record of the change the batching window
    /// holds is the window's at the core's reads of it (plan step S2d(2)),
    /// except while a change is being handed over (`handing_over`: the
    /// record is set, the window not yet). Removed with the in-place
    /// delivery (plan S3a).
    #[cfg(debug_assertions)]
    pub(crate) fn check_handed(&self) {
        if self.handing_over {
            return;
        }
        let window = self
            .preparing
            .as_ref()
            .map(|(_, change)| HandedMount::of(change));
        let same = match (&self.core.handed, &window) {
            (None, None) => true,
            (Some(core), Some(window)) => core.same(window),
            _ => false,
        };
        debug_assert!(
            same,
            "the core's record of the batching window is behind it"
        );
    }
    /// The R13 handoff of a mount batch and of `layout.set`: the window
    /// facts the interface last reported, with the tabs as the apply left it
    /// (`window_facts_report`). The facts themselves are measured at the
    /// next `window_tick` as before (its `window.state` goes out then);
    /// `None` without a shell.
    pub(crate) fn facts_handoff(&mut self) -> Option<Box<WindowFacts>> {
        let shell = self.shell.as_ref().map(ComponentHandle::clone_strong)?;
        let facts = self.window_facts.clone();
        Some(Box::new(self.window_facts_report(Some(&shell), facts)))
    }
    /// The core's side of `UiReport::MountPreparing` (R6): the changes left
    /// the batching queue, so their plugins hear `ui.preparing`
    /// (`phase: preparing`), in order.
    pub(crate) fn mounts_preparing(&mut self, changes: Vec<(Reply, u64)>) {
        for (reply, key) in changes {
            self.send(
                reply.owner,
                json!({"event":"ui.preparing","data":{"phase":"preparing","slot":slot_of(key)}}),
            );
        }
    }
    /// The core's side of `UiReport::MountApplied`, in R13's order: (1) the
    /// state the report carries (the handoff's facts and tab order; the
    /// mount record was made the registry, `Core::mounts`, where the
    /// interface applied it, `apply_mounts`), (2) per change its
    /// `ui.preparing` (`ready` or `failed`; none for a placeholder) and its
    /// reply (a change of the host's own, id 0: a failure is announced as
    /// `host.error`), whose activation completes after it (R12, `respond`),
    /// (3) `window.state` if the facts changed.
    pub(crate) fn mounts_applied(&mut self, applied: MountApplied) {
        let MountApplied {
            batch,
            result,
            elapsed_ms,
            handoff,
            placeholders,
        } = applied;
        let changed = handoff.is_some_and(|facts| self.core.apply_facts(*facts));
        for (reply, key) in batch {
            let owner = reply.owner;
            if !placeholders.contains(&key) {
                self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed_ms,"slot":slot_of(key)}}));
            }
            if reply.id == 0 {
                if let Err(error) = &result {
                    self.broadcast(
                        "host.error",
                        json!({"code":"ui-owner-cleanup","owner":owner,"message":error.message}),
                    );
                }
            } else {
                self.respond(reply, result.clone());
            }
        }
        if changed {
            let facts = self.core.window_facts.clone();
            self.broadcast("window.state", facts);
        }
    }
    /// A command of the core reaches the interface (rule R3), delivered in
    /// place on this thread until plan step S3a. Plan step S2c(3):
    /// `ui.setMount`, answered from the interface's `UiReport::Answered`
    /// (a mount change is taken by the batching window instead,
    /// `Core::mount_command`); S2c(4): `settings.*`
    /// (`Host::settings_command`, ui/settings_window.rs); S2c(5): the
    /// layout, the palette's lead, the file dialogs, the system
    /// confirmation and the sheet (`Host::layout_command`,
    /// `lead_command`, `dialog_command`, `confirm_command`,
    /// `review_command`); S2c(6): a view's row-patch records dropped
    /// (`ResetWrites`, `ui::ui_image::Images::reset_writes`); S2c(7): the
    /// last `source.state`, the info panels, the palette, the window
    /// settings in effect (`LastSource`, `InfoPanels`, `Theme`,
    /// `CursorAutoHide`, `WindowOp::OnTop`); S2d(1): the view requests
    /// (`View`, `Host::view_command`), the video area and
    /// picture-in-picture (`WindowOp::VideoArea`, `WindowOp::Pip`;
    /// `video_area_command`, `pip_command`). The other commands follow in
    /// later steps.
    pub(crate) fn ui_command(&mut self, command: UiCommand) {
        self.apply_ui_command(command);
        // Plan step S2f(1): what the command changed in the interface's
        // facts reaches the core before the core reads them again.
        self.report_ui_facts();
    }
    fn apply_ui_command(&mut self, command: UiCommand) {
        match command {
            UiCommand::Snapshot(UiSnapshot::LastSource(source)) => self.source = source,
            UiCommand::Snapshot(UiSnapshot::InfoPanels(panels)) => {
                self.info_panels = panels;
                self.show_info_panels();
            }
            UiCommand::Snapshot(UiSnapshot::Theme(theme)) => {
                self.shown_theme = theme;
                self.theme_all();
            }
            UiCommand::Snapshot(UiSnapshot::CursorAutoHide(playing)) => {
                self.cursor_playing = playing;
            }
            UiCommand::Window(WindowOp::OnTop(on)) => self.on_top = on,
            UiCommand::ResetWrites { owner, view } => self.images.reset_writes(owner, &view),
            UiCommand::SetMount(op) => {
                let SetMount {
                    reply,
                    key,
                    params,
                    prepared,
                } = op;
                let result = crate::capabilities::ui_mount::write_mount(
                    self, &reply, key, &params, prepared,
                );
                self.ui_report(UiReport::Answered {
                    reply,
                    result,
                    handoff: None,
                    confirm_failed: false,
                });
            }
            UiCommand::Settings(op) => self.settings_command(op),
            UiCommand::Layout(op) => self.layout_command(op),
            UiCommand::Lead(op) => self.lead_command(op),
            UiCommand::Dialog(request) => self.dialog_command(request),
            UiCommand::Confirm(request) => self.confirm_command(request),
            UiCommand::Review(request) => self.review_command(request),
            UiCommand::View(op) => self.view_command(op),
            UiCommand::Window(WindowOp::VideoArea {
                reply,
                width,
                height,
            }) => self.video_area_command(reply, width, height),
            UiCommand::Window(WindowOp::Pip { reply, on }) => self.pip_command(reply, on),
            // Plan step S2d(2): the core cancels the change the batching
            // window holds; start-up settled.
            UiCommand::CancelMount(cancel) => self.cancel_mount(cancel),
            // Plan step S2d(3): a stopped run's interface part, panels that
            // leave or move, a stopped plugin's waiting sheet.
            UiCommand::RevokeOwner { owner, park: _ } => self.revoke_owner(owner),
            UiCommand::Panels(op) => self.panels_command(op),
            UiCommand::DropConfirm { owner } => self.drop_confirm_of(owner),
            // Plan step S2d(4): the OSD, what `source.state` shows, the
            // window's one-way operations, the palette, a panel's menu.
            UiCommand::Osd(op) => self.osd_command(op),
            UiCommand::Snapshot(UiSnapshot::Source(shown)) => self.show_source(shown),
            UiCommand::Window(
                op @ (WindowOp::Fullscreen(_) | WindowOp::Raise | WindowOp::Quit),
            ) => self.window_op(op),
            UiCommand::OpenPalette => self.open_palette(),
            UiCommand::PanelMenu(menu) => self.panel_menu(menu),
            UiCommand::Snapshot(UiSnapshot::StartupSettled(settled)) => {
                self.startup_panels = settled;
            }
            // Plan step S2f(1) (design §5.3, group B): interface work the
            // core called in place before.
            UiCommand::RefreshMenu => {
                if self.menu_open {
                    let result = self.refresh_menu();
                    self.report_menu(result);
                }
            }
            UiCommand::ShowWaiting => self.show_waiting_confirmation(),
            UiCommand::Activity => self.policy.activity(self.core.now_ms()),
            UiCommand::Window(WindowOp::ApplyOnTop) => self.apply_on_top(),
            _ => unreachable!(
                "not sent here: the tables go by `sync_tables`; `RekeyOwner` is not sent"
            ),
        }
    }
    fn log_prepared(&self, changes: usize, started: Instant, ok: bool) {
        if self.frame_log.is_some() {
            diag!(
                "vestra shell: {} change(s), {} panel(s), {} compiled, prepared in {:.1} ms ({})",
                changes,
                self.core.mounts.len(),
                self.panels.definitions(),
                started.elapsed().as_secs_f64() * 1000.,
                if ok { "ready" } else { "failed" }
            );
        }
    }
    /// Make `next` the mounted panels (sidebar-tabs.md); `batch`: the panels
    /// whose mount or unmount this applies. The shell is never compiled
    /// again: each panel is compiled on its own, when it is first shown
    /// (`ui/panels.rs`). A panel of the batch that shows at once (the
    /// bottom bar, the backdrop, the tab that will be current in a column
    /// that is wanted shown) is compiled here, before anything changes, so
    /// one that does not compile fails its mount as before; a tab mounted
    /// in the background compiles nothing (but an adjust tab joining the
    /// front tabs of the column wanted shown: right after, `preload_front`),
    /// and one that turns out not to compile when it is first shown is
    /// reported to its plugin then (`show_panel`). The policy, the panel
    /// keys and the remembered state follow.
    pub(crate) fn apply_mounts(
        &mut self,
        next: BTreeMap<u64, Mount>,
        batch: &[u64],
    ) -> Result<Value, RpcError> {
        let started = Instant::now();
        // A new mount gets a new chance to compile.
        for owner in batch {
            self.tabs_failed.remove(owner);
        }
        self.tabs_failed.retain(|owner| next.contains_key(owner));
        // Gone, or moved to another slot (no tab left behind).
        let leaving = self
            .core
            .mounts
            .iter()
            .filter(|(owner, mount)| {
                next.get(owner)
                    .is_none_or(|m| m.spec.slot != mount.spec.slot)
            })
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        // The tab that will be current in each column, so the first frame
        // shows it: the policy's after the leaving tabs went (the next one
        // takes over a removed current tab). A placeholder that would take
        // over that way is passed over for a running plugin's tab, so that
        // removing a tab never wakes a neighbour unasked.
        let mut planned = self.policy.clone();
        for owner in &leaving {
            planned.remove_owner(*owner);
        }
        let recorded = |owner: u64| next.get(&owner).is_some_and(|m| m.recorded);
        let plans = Column::ALL.map(|column| {
            let order = self.tab_order(column, &next);
            let remembered = self.column_memory(column).active;
            let current_left = self
                .policy
                .active(policy_column(column))
                .is_some_and(|owner| leaving.contains(&owner));
            let active = sidebar_tabs::next_active(
                &order,
                planned.active(policy_column(column)),
                current_left,
                recorded,
            )
            .or_else(|| {
                order
                    .iter()
                    .copied()
                    .find(|o| next.get(o).map(|m| m.manifest.as_str()) == remembered.as_deref())
            })
            .or_else(|| order.first().copied());
            let tabbed = if column.always_tabbed() {
                !order.is_empty()
            } else {
                order.len() > 1
            };
            ColumnPlan {
                order,
                active,
                current_left,
                tabbed,
            }
        });
        let titles = plans
            .iter()
            .map(|plan| {
                ui_values::from_json(&json!(
                    plan.order
                        .iter()
                        .filter_map(|owner| next.get(owner).map(|m| m.title.clone()))
                        .collect::<Vec<_>>()
                ))
                .map_err(|error| RpcError::new("ui-prepare", error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let tabbed = [plans[0].tabbed, plans[1].tabbed];
        // The batch's mounts (a placeholder replayed from a recording has
        // no process and draws nothing).
        let mounted = batch
            .iter()
            .filter_map(|key| next.get(key))
            .filter(|mount| !mount.recorded)
            .cloned()
            .collect::<Vec<_>>();
        // The ones that show at once compile first: a failure leaves
        // everything as it was.
        self.compiled_now.clear();
        self.panels.clear_pending();
        for mount in &mounted {
            let shows = match Column::of(mount.spec.slot) {
                None => true,
                // The tab current for the host once `next` is mounted
                // (`current_in`: none while the column waits for its
                // remembered tab).
                Some(column) => {
                    self.column_wanted(column)
                        && self
                            .current_in(column, plans[column.index()].active, &next)
                            .tab()
                            == Some(mount.spec.owner)
                }
            };
            if !shows {
                continue;
            }
            let compiled = self.panels.precompile(&mount.spec);
            if !matches!(compiled, Ok(None)) {
                self.log_compile(mount, compiled.as_ref().ok().copied().flatten());
            }
            match compiled {
                Ok(Some(_)) => {
                    self.compiled_now.insert(mount.spec.owner);
                }
                Ok(None) => {}
                Err(error) => return Err(RpcError::new("ui-prepare", error.to_string())),
            }
        }
        // From here on nothing fails. Each mount of the batch is a new
        // panel with the values it was mounted with (the panel it replaces,
        // a placeholder's kept one or one kept over its plugin's restart,
        // goes with its instances); panels no longer mounted go.
        let mut failures = Vec::new();
        for mount in &mounted {
            let panel = self.panels.insert(mount.spec.clone());
            for (name, value) in self.mount_values(mount) {
                if let Err(error) = panel.write(&name, value) {
                    failures.push((mount.spec.owner, format!("{name}: {error:#}")));
                }
            }
            if let Some(slot) = policy_slot(mount.spec.slot)
                && self.core.mounts.contains_key(&mount.spec.owner)
                && let Err(error) =
                    self.policy
                        .set_locks(slot, mount.spec.owner, window_state::Locks::default())
            {
                diag!("mount policy reset: {error}");
            }
        }
        self.report_value_failures(failures);
        self.panels.clear_pending();
        self.panels.retain(|key| next.contains_key(&key));
        for old in &leaving {
            self.policy.remove_owner(*old);
            if !next.contains_key(old)
                && let Some(mount) = self.core.mounts.get(old)
            {
                self.images
                    .release_view(mount.plugin, mount_view(mount.spec.slot));
            }
        }
        let memory = Column::ALL.map(|column| self.column_memory(column));
        let attach = next
            .values()
            .filter(|m| !m.spec.slot.tabbed())
            .chain(
                plans
                    .iter()
                    .flat_map(|plan| plan.order.iter().filter_map(|owner| next.get(owner))),
            )
            .filter(|m| {
                !self.core.mounts.contains_key(&m.spec.owner) || leaving.contains(&m.spec.owner)
            })
            .map(|m| (m.spec.owner, m.spec.slot, m.visible, m.manifest.clone()))
            .collect::<Vec<_>>();
        for (id, mount_slot, mount_visible, manifest) in attach {
            let Some(policy) = policy_slot(mount_slot) else {
                continue;
            };
            let column = Column::of(mount_slot);
            let (visible, detached) = match column {
                Some(column) => {
                    let memory = &memory[column.index()];
                    (memory.visible, memory.detached)
                }
                None => (mount_visible, false),
            };
            if let Err(error) = self.policy.attach(policy, id, visible, detached) {
                diag!("mount policy attach: {error}");
            }
            // The tab that was current at the last run is current again.
            if let Some(column) = column
                && memory[column.index()].active.as_deref() == Some(&manifest)
            {
                let _ = self.policy.set_active(id);
            }
        }
        for (column, plan) in Column::ALL.into_iter().zip(&plans) {
            // The tab chosen above, when the current one left.
            if plan.current_left
                && let Some(active) = plan.active
            {
                let _ = self.policy.set_active(active);
            }
            self.policy.order_column(policy_column(column), &plan.order);
        }
        // The tabs as the batch left them reach the core (rule R9).
        self.report_tabs();
        self.policy.activity(self.core.now_ms());
        {
            let mut input = self.input.borrow_mut();
            input.bottom = next
                .values()
                .find(|m| m.spec.slot == composition::Slot::Bottom)
                .map(|m| m.plugin);
        }
        // The mount record of the R13 handoff, given to the core's registry
        // here, in place, before any reply (`mounts_applied`): what follows
        // (the panel keys, the panels shown) reads the record it makes.
        // From plan step S3a the interface keeps its own copy and the
        // record travels in `MountApplied`.
        self.core.mounts = next;
        {
            // The panel keys' plugins: their commands take the panel's keys
            // (none while a column waits for its remembered tab).
            let sidebar = self.current_tab(Column::Sidebar).map(panel_plugin);
            let adjust = self.current_tab(Column::Adjust).map(panel_plugin);
            let mut input = self.input.borrow_mut();
            input.sidebar = sidebar;
            input.adjust = adjust;
        }
        self.core.note_panel_plugins();
        for (state, plan) in self.columns.iter_mut().zip(&plans) {
            state.tabs = plan.order.clone();
            state.tabbed = plan.tabbed;
        }
        for column in Column::ALL {
            self.remember_column(column, false);
        }
        // The strips (a tab came or went) and the panels, in the same
        // frame: each column's current tab, the bar and the backdrop.
        for mount in self.core.mounts.values() {
            if let Some(column) = Column::of(mount.spec.slot)
                && let Some(panel) = self.panels.panel(mount.spec.owner)
            {
                panel.set_tabbed(tabbed[column.index()]);
            }
        }
        if let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) {
            let error = |error: anyhow::Error| RpcError::new("ui-prepare", error.to_string());
            for ((column, titles), plan) in Column::ALL.into_iter().zip(&titles).zip(&plans) {
                let p = column.prefix();
                shell
                    .set_property(&format!("{p}-titles"), titles.clone())
                    .map_err(|e| error(e.into()))?;
                if let Some(index) = plan
                    .active
                    .and_then(|a| plan.order.iter().position(|&o| o == a))
                {
                    shell
                        .set_property(&format!("{p}-active"), SValue::Number(index as f64))
                        .map_err(|e| error(e.into()))?;
                }
            }
            shell
                .set_property("sidebar-tabbed", tabbed[0].into())
                .map_err(|e| error(e.into()))?;
        }
        self.sync_panels();
        Ok(
            json!({"status":"ready","shellGeneration":self.shell_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
        )
    }
    /// Whether a column is wanted shown (also while it is hidden for the
    /// moment: collapsed in fullscreen, detached): its current tab's panel
    /// is compiled and instantiated only then. Before its first panel is
    /// attached: as remembered from the last run.
    pub(crate) fn column_wanted(&self, column: Column) -> bool {
        self.policy.column_state(policy_column(column)).map_or_else(
            || self.column_memory(column).visible,
            |(preferred, _)| preferred,
        )
    }
    /// The start-up rule for a column's current tab, its one source of
    /// truth: while start-up has not settled (`startup_panels`), a column
    /// whose remembered current tab (the last run's) has not mounted yet
    /// has no current tab (`Current::Awaits`), instead of the tab that is
    /// current meanwhile (plugins start one after another, and a
    /// placeholder replayed from a recording mounts at once: a flash of
    /// another tab, a compile for nothing, a wake for nothing). Otherwise
    /// it is `active`, the policy's (once settled, whatever it is: the
    /// remembered plugin may be gone). Of the column with the mounts
    /// `mounts`: `next` while a mount change is planned, else the mounted
    /// ones. Every consumer asks this, through `current` or `current_tab`:
    /// what the column shows and compiles (`apply_mounts`, `sync_panels`),
    /// the floating sidebar, the panel keys, the tab's wake and its
    /// kept-running work (`sync_column`, `tab_demand`) and
    /// `Busy::ShownTab`. The tab strip marks the policy's current tab all
    /// the same (the user's choice, kept).
    fn current_in(
        &self,
        column: Column,
        active: Option<u64>,
        mounts: &BTreeMap<u64, Mount>,
    ) -> Current {
        let awaits = crate::ui::sidebar_tabs::awaits_remembered(
            self.startup_panels,
            self.column_memory(column).active.as_deref(),
            active
                .and_then(|active| mounts.get(&active))
                .map(|mount| mount.manifest.as_str()),
            mounts
                .values()
                .filter(|mount| mount.spec.slot == column.slot())
                .map(|mount| mount.manifest.as_str()),
        );
        if awaits {
            Current::Awaits
        } else {
            Current::Tab(active)
        }
    }
    /// A column's current tab for the host now (`current_in` over the
    /// policy's current tab and the mounted panels).
    pub(crate) fn current(&self, column: Column) -> Current {
        self.current_in(
            column,
            self.policy.active(policy_column(column)),
            &self.core.mounts,
        )
    }
    /// The tab `current` names: none while the column waits for its
    /// remembered tab (neither a placeholder, which would be woken, nor a
    /// running plugin's, which would be kept running).
    pub(crate) fn current_tab(&self, column: Column) -> Option<u64> {
        self.current(column).tab()
    }
    /// One line per compile of a panel's file (tests/startup_compiles.rs
    /// counts them): a compile blocks the UI thread. `took`: `None` when it
    /// failed.
    fn log_compile(&self, mount: &Mount, took: Option<f64>) {
        diag!(
            "panel: compile [{}:{}]{} at +{} ms ({})",
            mount.manifest,
            mount.spec.slot.name(),
            took.map_or_else(String::new, |ms| format!(" in {ms:.1} ms")),
            self.core.now_ms(),
            if took.is_some() { "ready" } else { "failed" }
        );
    }
    /// Show panel `key` (`None`, a placeholder or one that does not
    /// compile: nothing) in the container of `slot` of `window`. A panel
    /// first shown compiles now; one that does not compile is left out
    /// until its plugin mounts it again and reported to its plugin
    /// (`ui.error`), so it never fails another plugin's panel. Whether the
    /// container's content changed.
    pub(crate) fn show_panel(
        &mut self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
        slot: composition::Slot,
        key: Option<u64>,
    ) -> bool {
        use crate::ui::panels::Shown;
        let key = key.filter(|key| self.panels.contains(*key) && !self.tabs_failed.contains(key));
        let events = self.mount_events.clone();
        match self.panels.show(window, generation, slot, key, &events) {
            Ok(Shown::Same) => false,
            Ok(Shown::Changed { compiled_ms }) => {
                if let (Some(took), Some(mount)) =
                    (compiled_ms, key.and_then(|key| self.core.mounts.get(&key)))
                {
                    self.log_compile(mount, Some(took));
                }
                true
            }
            Err(error) => {
                let Some(key) = key else {
                    diag!("panel container {}: {error:#}", slot.name());
                    return false;
                };
                if let Some(mount) = self.core.mounts.get(&key) {
                    self.log_compile(mount, None);
                }
                diag!("panel {key}: it does not compile: {error:#}");
                self.tabs_failed.insert(key);
                let refusal = RpcError::new("ui-prepare", format!("ui-prepare: {error:#}"));
                self.report_ui_error(panel_plugin(key), &refusal);
                matches!(
                    self.panels.show(window, generation, slot, None, &events),
                    Ok(Shown::Changed { .. })
                )
            }
        }
    }
    /// Compile the front tabs of `column` (`sidebar_tabs::front`) that are
    /// not loaded, without showing them; logged as any compile. One that
    /// does not compile is left out and reported to its plugin, as at its
    /// showing (`show_panel`). A placeholder tab (no panel in this run) is
    /// skipped. Whether anything was compiled.
    fn preload_front(&mut self, column: Column, active: Option<u64>) -> bool {
        let front = crate::ui::sidebar_tabs::front(&self.columns[column.index()].tabs, active);
        let mut loaded = false;
        for key in front {
            if self.tabs_failed.contains(&key) || !self.panels.contains(key) {
                continue;
            }
            match self.panels.load(key) {
                Ok(None) => {}
                Ok(Some(took)) => {
                    loaded = true;
                    if let (Some(took), Some(mount)) = (took, self.core.mounts.get(&key)) {
                        self.log_compile(mount, Some(took));
                    }
                }
                Err(error) => {
                    if let Some(mount) = self.core.mounts.get(&key) {
                        self.log_compile(mount, None);
                    }
                    diag!("panel {key}: it does not compile: {error:#}");
                    self.tabs_failed.insert(key);
                    let refusal = RpcError::new("ui-prepare", format!("ui-prepare: {error:#}"));
                    self.report_ui_error(panel_plugin(key), &refusal);
                }
            }
        }
        loaded
    }
    /// Every slot's container shows its panel (each tick, and at once after
    /// a mount change): the bottom bar, the backdrop, and each column's
    /// current tab while the column is wanted shown, in the shell and (the
    /// sidebar) the floating window. Only what changed touches Slint; a
    /// changed container gets its instance before the next frame, and
    /// before the host reads geometry.
    pub(crate) fn sync_panels(&mut self) {
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        use composition::Slot;
        let started = Instant::now();
        let generation = self.shell_generation;
        // The palette a new instance gets.
        self.panels.set_theme(self.theme());
        let floating = self
            .floating
            .as_ref()
            .map(|f| (f.instance.clone_strong(), f.generation));
        match &floating {
            Some((_, floating)) => self.panels.prune(&[generation, *floating]),
            None => self.panels.prune(&[generation]),
        }
        let mut changed = false;
        for slot in [Slot::Bottom, Slot::Backdrop] {
            let key = self
                .core
                .mounts
                .values()
                .find(|m| m.spec.slot == slot)
                .map(|m| m.spec.owner);
            changed |= self.show_panel(&shell, generation, slot, key);
        }
        let mounted = SValue::Bool(self.panels.shown(generation, Slot::Bottom).is_some());
        if shell.get_property("bottom-mounted").ok().as_ref() != Some(&mounted) {
            let _ = shell.set_property("bottom-mounted", mounted);
            changed = true;
        }
        if changed {
            view::settle_layout(shell.window());
        }
        self.mirror_bar(&shell);
        for column in Column::ALL {
            let slot = column.slot();
            let active = self.policy.active(policy_column(column));
            let current = self.current(column);
            let wanted = self.column_wanted(column) && !current.awaits();
            // Not wanted shown (or waiting for its remembered tab): what it
            // showed stays (nothing compiles), unless that is no longer its
            // current tab.
            let key = active.filter(|&a| wanted || self.panels.shown(generation, slot) == Some(a));
            let shown = self.show_panel(&shell, generation, slot, key);
            changed |= shown;
            // The adjust column wanted shown has all its front tabs compiled
            // (user ruling 2026-11-11): its first opening compiles them at
            // once, and a tab mounted into the front while it is open
            // compiles at that mount, so a switch never compiles.
            let loaded = column == Column::Adjust && wanted && self.preload_front(column, active);
            if shown || loaded {
                self.panels.evict(slot);
            }
            // The floating sidebar shows its current tab whatever it showed
            // (none while the column waits for its remembered tab).
            if column == Column::Sidebar
                && let Some((window, floating)) = &floating
                && self.show_panel(window, *floating, slot, current.tab())
            {
                view::settle_layout(window.window());
                self.panels.evict(slot);
            }
        }
        if changed {
            view::settle_layout(shell.window());
            // `VESTRA_FRAME_LOG`: what a container change cost on the UI
            // thread (a compile, an instance, or the kept one embedded again).
            if self.frame_log.is_some() {
                diag!(
                    "vestra panels: containers changed in {:.1} ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
            }
        }
        let refused = self.panels.take_refused();
        if !refused.is_empty() {
            self.report_value_failures(refused);
        }
        self.sync_restart_lines();
    }
    /// The bottom bar's reported width and `placed` to the shell's strip
    /// (`panels::mirror_bar`): a change shows there one tick later at most.
    fn mirror_bar(&self, shell: &slint_interpreter::ComponentInstance) {
        let bar = self.panels.bar_instance(self.shell_generation);
        crate::ui::panels::mirror_bar(shell, bar.as_ref().map(|(_, bar)| bar));
    }
    /// Values a new instance could not take (a property the panel does not
    /// declare, a value of the wrong type): logged and reported to their
    /// plugin (`ui.error`, its `onError`), as `mount_values` does; the panel
    /// is drawn with its defaults for them.
    pub(crate) fn report_value_failures(&mut self, failures: Vec<(u64, String)>) {
        for (owner, message) in failures {
            diag!("mount property {owner}/{message}");
            self.report_ui_error(panel_plugin(owner), &RpcError::new("ui", message));
        }
    }

    /// Initial properties `refuse_initial` left out: logged and reported
    /// to the plugin (`ui.error`, its `onError`).
    pub(crate) fn report_refused(&mut self, owner: u64, refused: Vec<RpcError>) {
        for error in refused {
            diag!("initial property {owner}: {}", error.message);
            self.send(owner, ui_error(&error));
        }
    }
    /// A mount's properties converted for a new instance. They were checked
    /// when written, so a value that no longer converts is an image file
    /// removed since (or a document over the text total): it is left out (the
    /// property keeps its default), logged and reported to the plugin
    /// (`ui.error`, its `onError`).
    pub(crate) fn mount_values(&mut self, mount: &Mount) -> Vec<(String, SValue)> {
        let owner = mount.plugin;
        let view = mount_view(mount.spec.slot);
        let mut values = Vec::new();
        for (name, value) in &mount.properties {
            match self.prop_value(owner, view, name, value) {
                Ok(value) => values.push((name.clone(), value)),
                Err(error) => {
                    diag!("mount property {owner}/{name}: {}", error.message);
                    self.report_ui_error(owner, &error);
                }
            }
        }
        values
    }
    pub(crate) fn mount_callbacks(&mut self) {
        for event in self.mount_input.try_iter().take(64).collect::<Vec<_>>() {
            // A callback names the key its panel was mounted under: one
            // whose plugin woke or restarted under another owner since is
            // no longer mounted under it, and is dropped.
            if !self.window_generation_live(event.shell_generation)
                || !self.core.mounts.contains_key(&event.owner)
            {
                continue;
            }
            // Its process is gone: dropped, never held for the next run
            // (a stale click). The panel takes no pointer meanwhile.
            if self.restart_waiting(event.owner) {
                diag!(
                    "mount callback {} of a panel waiting for its plugin's restart: dropped",
                    event.callback
                );
                continue;
            }
            // To the panel's plugin: `view` stays `"$mount"` for SDKs before
            // 1.6.15 (one panel each); `slot` says which panel.
            let (owner, slot) = (panel_plugin(event.owner), slot_of(event.owner));
            // The main window's panel, or the floating sidebar's.
            self.acted_in(owner, event.shell_generation);
            // The core writes the plugin's line (plan step S2f(1)).
            let line = match event.args {
                Ok(args) => messages::PluginLine::UiCallback {
                    owner,
                    data: json!({"view":"$mount","slot":slot,"callback":event.callback,"args":args}),
                },
                Err(error) => messages::PluginLine::UiError {
                    owner,
                    data: json!({"code":"ui","slot":slot,"message":format!("callback {}: {error}",event.callback)}),
                },
            };
            self.report_line(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDK 1.6.15: a plugin mounts one panel per slot. Each panel has a key
    /// of its own (the shell, the policy and the compile sets know it by
    /// that); the plugin and the slot come back from it, and keys of
    /// different plugins or slots never meet.
    #[test]
    fn each_panel_of_a_plugin_has_its_own_key() {
        use composition::Slot;
        let slots = [Slot::Bottom, Slot::Sidebar, Slot::Backdrop, Slot::Adjust];
        let mut seen = std::collections::BTreeSet::new();
        for owner in [1, 2, 7, 1000] {
            for slot in slots {
                let key = panel_key(owner, slot);
                assert!(seen.insert(key), "{owner} {slot:?}");
                assert_eq!(panel_plugin(key), owner);
                assert_eq!(slot_of(key), slot.name());
            }
        }
        // Images and row patches per panel: the bottom bar keeps `$mount`.
        assert_eq!(mount_view(Slot::Bottom), "$mount");
        let views = slots.map(mount_view);
        assert_eq!(
            views
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
    }
}

/// Plan step S2d(2): the core's record of the change the batching window
/// holds.
#[cfg(test)]
mod handed_tests {
    use super::*;

    fn held(id: u64, slot: Option<composition::Slot>) -> HandedMount {
        HandedMount {
            reply: Reply { owner: 7, id },
            key: 28,
            slot,
            call: Value::Null,
        }
    }

    /// Only the host's own unmount (id 0) of that panel gives way to the
    /// plugin's mount; a plugin's unmount or any mount does not.
    #[test]
    fn only_the_hosts_own_unmount_of_the_panel_is_a_removal() {
        assert!(held(0, None).removes(28));
        assert!(!held(0, None).removes(29));
        assert!(!held(5, None).removes(28));
        assert!(!held(0, Some(composition::Slot::Sidebar)).removes(28));
    }

    #[test]
    fn records_are_the_same_only_when_every_field_is() {
        let base = held(3, Some(composition::Slot::Sidebar));
        assert!(base.same(&base.clone()));
        assert!(!base.same(&held(4, Some(composition::Slot::Sidebar))));
        assert!(!base.same(&held(3, None)));
        let mut called = base.clone();
        called.call = json!({"slot":"sidebar"});
        assert!(!base.same(&called));
    }
}
