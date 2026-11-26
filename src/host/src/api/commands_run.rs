//! `commands.run` (built-in only, risky): the palette's command table
//! (`commands.list`) and running one of its commands the way the palette
//! runs it (`commands.run`), with a one-time ticket the palette minted for
//! a user's action (`palette.lead`, api/palette_lead.rs).
//!
//! The boundary is the player's, not the plugin's (plugin processes are not
//! sandboxed): a run needs a live ticket of the caller's, the rates are
//! capped ([`CommandRuns`]), a destructive command and any command of a
//! plugin that is not built-in asks the user first (the host's approval
//! sheet in the main window, `host/review_sheet.rs`, which the caller cannot
//! answer or skip; asked once, before the first step, for the whole plan a
//! run names with `then`), and every run is named before it runs: in the
//! status strip for the palette lead's holder, else on the OSD
//!. "Don't ask again" on the sheet
//! (Ask Vestra UX step 4, H4) trusts plugins that are not built in, per
//! caller, in the host's `ui-state.json`: their ordinary commands no longer
//! ask that caller; their destructive ones still do.
use super::{Capability, Handled, Info, Recorded};
use crate::{
    bindings,
    host::{
        Core, Event, Reply, RpcError,
        messages::{LeadOp, ReviewKind, ReviewRequest, RunSheet, UiCommand},
        palette::PALETTE,
        review_sheet, string,
    },
    i18n,
    ui::command_palette::Item,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

pub struct CommandsRun;
static INFO: Info = Info {
    name: "commands.run",
    builtin_only: true,
};
static METHODS: &[&str] = &["commands.list", "commands.run"];

/// A ticket ends this long after it was minted or last ran a command.
pub const TICKET_IDLE_MS: u64 = 120_000;
/// Commands one ticket may run.
pub const RUNS_PER_TICKET: u32 = 30;
/// Runs per second, per ticket.
pub const RUNS_PER_SECOND: usize = 4;
/// Runs per minute, per plugin (all its tickets).
pub const RUNS_PER_MINUTE: usize = 120;
/// The most commands one plan names: the run's own and its `then` (the
/// approval sheet's step rows, `ui/review-sheet.slint`; Ask Vestra's plan
/// cap).
pub const PLAN_STEPS: usize = 8;
/// The longest request text kept with a ticket (quoted in the sheet).
const QUERY_KEPT: usize = 200;
/// The player's own commands that ask first: Playback ▸ Close and Quit.
/// Only the player's: a plugin marks its own with `CommandOptions.destructive`
/// (the core never lists plugin commands).
pub(crate) const HOST_DESTRUCTIVE: [&str; 2] = ["stop", "quit"];
/// The built-in top-level menus whose every command asks first, by the key
/// `commands::normalize` gives them: `@settings` and `@reset`.
const DESTRUCTIVE_MENUS: [&str; 2] = ["Settings", "Reset"];

/// One ticket: whose, for which request, and its runs.
#[derive(Debug)]
struct Ticket {
    owner: u64,
    query: String,
    /// Minted or last run (ms of `Core::now_ms`).
    last: u64,
    runs: u32,
    /// Run times within the last second.
    recent: VecDeque<u64>,
    /// The rest of the plan the user approved on the sheet (the `then` of
    /// the run that named it; global ids, repeats counted): each runs once
    /// on this ticket without asking.
    plan: Vec<String>,
}

/// The tickets the palette minted and the per-plugin run times.
#[derive(Default)]
pub struct CommandRuns {
    tickets: BTreeMap<String, Ticket>,
    /// Run times within the last minute, per plugin.
    minute: BTreeMap<u64, VecDeque<u64>>,
    /// Runs waiting for the user's answer, by the caller's reply.
    confirming: Vec<ConfirmedRun>,
}

/// A run waiting for the user's answer on the approval sheet.
#[derive(Clone)]
pub struct ConfirmedRun {
    pub reply: Reply,
    pub ticket: String,
    pub global: String,
    pub step: Option<(u64, u64)>,
    /// The run's `then`: on Run, the ticket's plan.
    pub then: Option<Vec<String>>,
    /// What "Don't ask again" trusts (H4): the manifest ids of the plugins
    /// whose steps ask only because they are not built in; empty when the
    /// sheet offers no checkbox.
    pub remember: Vec<String>,
    /// Their shown names, in the same order (the checkbox's text).
    pub names: Vec<String>,
}

impl CommandRuns {
    /// A new ticket for `owner`'s request `query`, minted by a user's action
    /// in the palette. One request at a time: its earlier tickets end.
    pub fn mint(&mut self, owner: u64, query: &str, now: u64) -> String {
        self.tickets.retain(|_, ticket| ticket.owner != owner);
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.tickets.insert(
            id.clone(),
            Ticket {
                owner,
                query: query.chars().take(QUERY_KEPT).collect(),
                last: now,
                runs: 0,
                recent: VecDeque::new(),
                plan: Vec::new(),
            },
        );
        id
    }
    /// The request `ticket` of `owner` was minted for, if it is live.
    pub fn query(&self, owner: u64, ticket: &str, now: u64) -> Option<&str> {
        self.tickets
            .get(ticket)
            .filter(|t| t.owner == owner && now.saturating_sub(t.last) <= TICKET_IDLE_MS)
            .map(|t| t.query.as_str())
    }
    /// Count one run of `owner` with `ticket` at `now`, or refuse it:
    /// `ticket` (none, expired, revoked or another plugin's), `limit`.
    pub fn count(&mut self, owner: u64, ticket: &str, now: u64) -> Result<(), RpcError> {
        let refused = || {
            RpcError::new(
                "ticket",
                "no live ticket: it ended (palette.lead.done, a newer request, 120 s unused) or was never given",
            )
        };
        let Some(entry) = self.tickets.get_mut(ticket).filter(|t| t.owner == owner) else {
            return Err(refused());
        };
        if now.saturating_sub(entry.last) > TICKET_IDLE_MS {
            self.tickets.remove(ticket);
            return Err(refused());
        }
        if entry.runs >= RUNS_PER_TICKET {
            return Err(RpcError::new(
                "limit",
                format!("a ticket runs at most {RUNS_PER_TICKET} commands"),
            ));
        }
        while entry
            .recent
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 1000)
        {
            entry.recent.pop_front();
        }
        if entry.recent.len() >= RUNS_PER_SECOND {
            return Err(RpcError::new(
                "limit",
                format!("at most {RUNS_PER_SECOND} commands a second"),
            ));
        }
        let minute = self.minute.entry(owner).or_default();
        while minute
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 60_000)
        {
            minute.pop_front();
        }
        if minute.len() >= RUNS_PER_MINUTE {
            return Err(RpcError::new(
                "limit",
                format!("at most {RUNS_PER_MINUTE} commands a minute"),
            ));
        }
        minute.push_back(now);
        entry.recent.push_back(now);
        entry.runs += 1;
        entry.last = now;
        Ok(())
    }
    /// `ticket`'s plan becomes `plan` (empty: none), replacing what was
    /// left of an earlier one.
    pub fn set_plan(&mut self, ticket: &str, plan: Vec<String>) {
        if let Some(entry) = self.tickets.get_mut(ticket) {
            entry.plan = plan;
        }
    }
    /// Whether `global` is in what is left of `owner`'s `ticket`'s plan:
    /// if so, its first occurrence is used up (it runs without asking).
    pub fn take_planned(&mut self, owner: u64, ticket: &str, global: &str) -> bool {
        let Some(entry) = self.tickets.get_mut(ticket).filter(|t| t.owner == owner) else {
            return false;
        };
        match entry.plan.iter().position(|g| g == global) {
            Some(at) => {
                entry.plan.remove(at);
                true
            }
            None => false,
        }
    }
    /// `palette.lead.done`: the ticket ends (unknown ones are ignored).
    pub fn done(&mut self, owner: u64, ticket: &str) {
        if self.tickets.get(ticket).is_some_and(|t| t.owner == owner) {
            self.tickets.remove(ticket);
        }
    }
    /// Everything of `owner` ends (it stopped or was parked).
    pub fn revoke(&mut self, owner: u64) {
        self.tickets.retain(|_, ticket| ticket.owner != owner);
        self.minute.remove(&owner);
        self.confirming.retain(|run| run.reply.owner != owner);
    }
    /// A parked owner woke as `to`: the ticket its wake event carries moves.
    pub fn rekey(&mut self, from: u64, to: u64) {
        for ticket in self.tickets.values_mut() {
            if ticket.owner == from {
                ticket.owner = to;
            }
        }
        if let Some(minute) = self.minute.remove(&from) {
            self.minute.insert(to, minute);
        }
    }
    /// The run waiting for the sheet answered for `reply`, if its caller
    /// still runs.
    pub fn answered(&mut self, reply: &Reply) -> Option<ConfirmedRun> {
        let at = self
            .confirming
            .iter()
            .position(|run| run.reply.owner == reply.owner && run.reply.id == reply.id)?;
        Some(self.confirming.remove(at))
    }
    /// What the waiting sheets would trust ("Don't ask again") loses `id`
    /// (`None`: everything): it was uninstalled, replaced from another
    /// source, or the user asked again before plugin commands.
    pub fn forget_remembered(&mut self, id: Option<&str>) {
        for run in &mut self.confirming {
            let (remember, names) = std::mem::take(&mut run.remember)
                .into_iter()
                .zip(std::mem::take(&mut run.names))
                .filter(|(kept, _)| id.is_some_and(|id| kept != id))
                .unzip();
            run.remember = remember;
            run.names = names;
        }
    }
    /// The shown names of the plugins the waiting run answering `reply`
    /// would trust ("Don't ask again"), if it still waits (the interface
    /// reads its copy, `RunsView::offered`).
    #[cfg(test)]
    pub fn offered(&self, reply: &Reply) -> Option<&[String]> {
        self.confirming
            .iter()
            .find(|run| run.reply.owner == reply.owner && run.reply.id == reply.id)
            .map(|run| run.names.as_slice())
    }
    /// Whether the run answering `reply` still waits for the user (its
    /// caller did not stop).
    pub fn waiting(&self, reply: &Reply) -> bool {
        self.confirming
            .iter()
            .any(|run| run.reply.owner == reply.owner && run.reply.id == reply.id)
    }
    /// Whether `global` is in what is left of `owner`'s `ticket`'s plan
    /// (read-only).
    fn planned(&self, owner: u64, ticket: &str, global: &str) -> bool {
        self.tickets
            .get(ticket)
            .is_some_and(|t| t.owner == owner && t.plan.iter().any(|g| g == global))
    }
    /// A checked call of `owner` running `global` on `ticket` (`then`: it
    /// named a plan; `plan_asks`: a command of its plan, or the command
    /// alone without `then`, asks first): whether it asks the user.
    ///
    /// Decided read-only first: a call that would ask while any sheet waits
    /// for the user fails `busy` before it is counted and before it touches
    /// the ticket's plan (a refused call is not counted). One sheet at a
    /// time, whoever's: the waiting one is the user's open question, so a
    /// second caller neither replaces it (its caller would wait for ever)
    /// nor answers it in the user's place. Then the run is counted
    /// (`ticket`, `limit`), a new plan drops the rest of the old one, and a
    /// planned step uses up its entry and does not ask.
    pub fn admit(
        &mut self,
        owner: u64,
        ticket: &str,
        global: &str,
        then: bool,
        plan_asks: bool,
        now: u64,
    ) -> Result<bool, RpcError> {
        let planned = !then && self.planned(owner, ticket, global);
        let asks = self.would_ask(owner, ticket, global, then, plan_asks);
        // A dead or foreign ticket is refused as such by `count`.
        if asks && !self.confirming.is_empty() && self.query(owner, ticket, now).is_some() {
            // H7 (PROPOSAL F4): why, so the caller does not blame the command.
            return Err(RpcError::new(
                "busy",
                "another command is waiting for the user's answer",
            ));
        }
        self.count(owner, ticket, now)?;
        if then {
            self.set_plan(ticket, Vec::new());
        } else if planned {
            self.take_planned(owner, ticket, global);
        }
        Ok(asks)
    }
    /// Whether [`Self::admit`] would have the call ask (read-only): its plan
    /// asks and it is not a step of the approved plan.
    pub fn would_ask(
        &self,
        owner: u64,
        ticket: &str,
        global: &str,
        then: bool,
        plan_asks: bool,
    ) -> bool {
        plan_asks && (then || !self.planned(owner, ticket, global))
    }
    /// `owner`'s live ticket and the request it was minted for (one at a
    /// time, [`Self::mint`]), if any: a confirmation it asks then shows that
    /// request (`dialogs.confirm`, Ask Vestra's first-use consent).
    pub fn live_request(&self, owner: u64, now: u64) -> Option<(String, String)> {
        self.tickets
            .iter()
            .find(|(_, t)| t.owner == owner && now.saturating_sub(t.last) <= TICKET_IDLE_MS)
            .map(|(id, t)| (id.clone(), t.query.clone()))
    }
    #[cfg(test)]
    fn live(&self) -> usize {
        self.tickets.len()
    }
    /// What the interface reads of the tickets and the waiting runs (plan
    /// step S3a(1), its copy `RunsView`).
    pub fn view(&self) -> RunsView {
        RunsView {
            tickets: self
                .tickets
                .iter()
                .map(|(id, ticket)| (id.clone(), ticket.last))
                .collect(),
            confirming: self
                .confirming
                .iter()
                .map(|run| ((run.reply.owner, run.reply.id), run.names.clone()))
                .collect(),
        }
    }
}

/// The interface's copy of what it reads of [`CommandRuns`] (plan step
/// S3a(1); design §5.3 group D): each ticket's last use, and the runs
/// waiting for the sheet with the names they offer. Its reads answer as
/// the core's do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunsView {
    tickets: BTreeMap<String, u64>,
    confirming: Vec<((u64, u64), Vec<String>)>,
}

impl RunsView {
    /// Whether this copy is still `runs`' (`CommandRuns::view` would give
    /// an equal one), compared without copying.
    pub fn matches(&self, runs: &CommandRuns) -> bool {
        self.tickets.len() == runs.tickets.len()
            && self
                .tickets
                .iter()
                .zip(&runs.tickets)
                .all(|((id, last), (other, ticket))| id == other && *last == ticket.last)
            && self.confirming.len() == runs.confirming.len()
            && self
                .confirming
                .iter()
                .zip(&runs.confirming)
                .all(|(((owner, id), names), run)| {
                    *owner == run.reply.owner && *id == run.reply.id && *names == run.names
                })
    }
    /// Whether `ticket` is live at `now`, whoever holds it (the status
    /// strip shows a ticket's steps while it is; the sheet asks while it is).
    pub fn is_live(&self, ticket: &str, now: u64) -> bool {
        self.tickets
            .get(ticket)
            .is_some_and(|last| now.saturating_sub(*last) <= TICKET_IDLE_MS)
    }
    /// [`CommandRuns::waiting`].
    pub fn waiting(&self, reply: &Reply) -> bool {
        self.confirming
            .iter()
            .any(|((owner, id), _)| *owner == reply.owner && *id == reply.id)
    }
    /// The interface's copy of `CommandRuns::offered` (a test helper
    /// there): the shown names the waiting run answering `reply` would
    /// trust, if it still waits.
    pub fn offered(&self, reply: &Reply) -> Option<&[String]> {
        self.confirming
            .iter()
            .find(|((owner, id), _)| *owner == reply.owner && *id == reply.id)
            .map(|(_, names)| names.as_slice())
    }
}

/// Whether running `item` asks first: a command that always asks
/// ([`always_asks`]) or any command of a plugin that is not built-in,
/// unless the caller trusts that plugin (`trusted`, H4: "Don't ask again").
/// Opening a dialog ("…") does not ask: the user is in the dialog then.
pub fn dangerous(item: &Item, builtin: bool, trusted: bool) -> bool {
    (!builtin && !trusted) || always_asks(item)
}

/// Whether `item` asks every time, trusted or not: a destructive command
/// (its own `destructive`, the player's Close and Quit, anything in the
/// built-in `@settings` and `@reset` menus). The menu is judged by the
/// whole path normalized (`commands::normalize`: `@settings`, an old root
/// "设置" with items below it), as the SDK's TestHost does.
pub fn always_asks(item: &Item) -> bool {
    item.destructive
        || (item.owner == bindings::HOST && HOST_DESTRUCTIVE.contains(&item.id.as_str()))
        || in_destructive_menu(&item.path_keys)
}

/// Whether a command at `path` is under the built-in Settings or Reset menu.
fn in_destructive_menu(path: &[String]) -> bool {
    let mut path = path.to_vec();
    // A path the registry refused never got here; keep it as it is if so.
    let _ = crate::commands::normalize(&mut path);
    path.first()
        .is_some_and(|root| DESTRUCTIVE_MENUS.contains(&root.as_str()))
}

/// Whether the commands of `owner` are a built-in's: the player's own, a
/// running built-in, or a built-in in standby (its recorded commands).
fn builtin_owner(core: &Core, owner: u64) -> bool {
    owner == bindings::HOST
        || core.is_builtin(owner)
        || core
            .on_demand
            .held(owner)
            .is_some_and(|held| core.is_builtin_package(&held.origin))
}

/// The manifest id of the plugin whose commands `owner` registered,
/// running or in standby; `None` for the player's own.
fn owner_id(core: &Core, owner: u64) -> Option<String> {
    if owner == bindings::HOST {
        return None;
    }
    core.plugins
        .get(&owner)
        .map(|plugin| plugin.manifest.id.clone())
        .or_else(|| core.held_id(owner).map(str::to_owned))
}

/// The plugins `caller` runs without asking (H4), by manifest id.
fn trusted_by(core: &Core, caller: u64) -> &[String] {
    core.ui_state
        .state
        .trusted_plugins
        .get(&core.plugin_id(caller))
        .map_or(&[], Vec::as_slice)
}

/// A command as `caller` would run it: whether its plugin is built in, its
/// plugin's manifest id, and whether the caller trusts that plugin (never a
/// built-in: there is nothing to trust).
#[derive(Clone, Debug)]
struct Judged {
    item: Item,
    builtin: bool,
    plugin_id: Option<String>,
    trusted: bool,
}

/// For a command of `owner`: built in, its plugin's manifest id, trusted
/// (by a caller that trusts `trusted`).
fn judge(core: &Core, trusted: &[String], owner: u64) -> (bool, Option<String>, bool) {
    let builtin = builtin_owner(core, owner);
    let plugin_id = owner_id(core, owner);
    let trusted = !builtin && plugin_id.as_ref().is_some_and(|id| trusted.contains(id));
    (builtin, plugin_id, trusted)
}

impl Judged {
    fn new(core: &Core, trusted: &[String], item: Item) -> Self {
        let (builtin, plugin_id, trusted) = judge(core, trusted, item.owner);
        Self {
            item,
            builtin,
            plugin_id,
            trusted,
        }
    }
    fn dangerous(&self) -> bool {
        dangerous(&self.item, self.builtin, self.trusted)
    }
}

/// What "Don't ask again" offers to trust on a sheet of `steps` (H4):
/// the plugins of the steps that ask only because they are not built in,
/// as (manifest id, shown name), in plan order, each once. None when a step
/// always asks: the checkbox is never offered with a destructive step (user
/// ruling 3 of 2026-11-12, PROPOSAL §4.3).
fn trust_offer(steps: &[Judged]) -> Vec<(String, String)> {
    if steps.iter().any(|step| always_asks(&step.item)) {
        return Vec::new();
    }
    let mut plugins: Vec<(String, String)> = Vec::new();
    for step in steps {
        if !step.dangerous() {
            continue;
        }
        if let Some(id) = &step.plugin_id
            && !plugins.iter().any(|(known, _)| known == id)
        {
            plugins.push((id.clone(), step.item.plugin.clone()));
        }
    }
    plugins
}

/// "Don't ask again" was checked on `caller`'s sheet: it trusts `plugins`
/// from now on (kept in `ui-state.json`, across restarts).
fn trust_plugins(core: &mut Core, caller: u64, plugins: &[String]) {
    let caller_id = core.plugin_id(caller);
    if !core.ui_state.state.trust(&caller_id, plugins) {
        return;
    }
    let now = core.now_ms();
    core.ui_state.touch(now);
    // Manifest ids only (README §4.3, Host log).
    crate::diag::log_only(&format!(
        "commands.run: {caller_id} no longer asks before the commands of {}",
        plugins.join(", ")
    ));
}

/// The palette command Ask Again Before Plugin Commands: every caller asks
/// again before any command of a plugin that is not built in; its OSD line.
pub(crate) fn ask_again(core: &mut Core) {
    // A sheet open now does not trust anything either.
    core.command_runs.forget_remembered(None);
    // The open sheet's offer is the interface's (plan step S2f(3)).
    core.ui_command(UiCommand::ReviewOffer);
    if core.ui_state.state.clear_trust() {
        // Written now, not a second later: a crash right after must not
        // bring the trust back.
        core.ui_state.flush();
        crate::diag::log_only("commands.run: plugin commands ask again");
    }
    core.show_osd(&i18n::tr("Plugin commands will ask again"));
}

/// `id` was uninstalled, or replaced from another source: no caller
/// trusts it any more (written at once, as for [`ask_again`]).
pub(crate) fn forget_trust(core: &mut Core, id: &str) {
    // Nor does a sheet opened before (it would trust the new source).
    core.command_runs.forget_remembered(Some(id));
    // Its checkbox no longer names it (and goes when it named only it).
    // The open sheet's offer is the interface's (plan step S2f(3)).
    core.ui_command(UiCommand::ReviewOffer);
    if core.ui_state.state.forget_trust(id) {
        core.ui_state.flush();
    }
}

/// One row of `commands.list` for a caller that trusts `trusted`.
fn listed(core: &Core, trusted: &[String], item: &Item) -> Value {
    let (builtin, _, trusted) = judge(core, trusted, item.owner);
    let keys = core.bindings().labels(item.owner, &item.id);
    let mut row = json!({
        "id": item.global,
        "label": item.label,
        "labelSource": item.label_key,
        "title": item.title,
        "category": item.category,
        "path": item.path_shown,
        "place": item.place,
        "keys": keys,
        "plugin": item.plugin,
        "builtin": builtin,
        "enabled": item.enabled,
        "checkable": item.checked.is_some(),
        "radio": item.radio,
        "group": item.group,
        "busy": item.busy,
        "dangerous": dangerous(item, builtin, trusted),
    });
    if let Some(checked) = item.checked {
        row["checked"] = checked.into();
    }
    if !item.enabled && !item.reason.is_empty() {
        row["reason"] = item.reason.clone().into();
    }
    row
}

/// `step` and `of` of a run (1 ≤ step ≤ of ≤ 99), both or neither.
fn step(params: &Value) -> Result<Option<(u64, u64)>, RpcError> {
    let read = |key: &str| match &params[key] {
        Value::Null => Ok(None),
        value => value
            .as_u64()
            .filter(|n| (1..=99).contains(n))
            .map(Some)
            .ok_or_else(|| RpcError::invalid(format!("{key} is an integer 1–99"))),
    };
    match (read("step")?, read("of")?) {
        (None, None) => Ok(None),
        (Some(step), Some(of)) if step <= of => Ok(Some((step, of))),
        _ => Err(RpcError::invalid("step and of go together, step ≤ of")),
    }
}

/// `then` of a run: the global ids the same ticket runs next (absent or
/// null: none named). Each a non-empty string; with the run's own at most
/// [`PLAN_STEPS`]. The messages are the SDK's TestHost's, word for word.
fn then(params: &Value) -> Result<Option<Vec<String>>, RpcError> {
    let list = match &params["then"] {
        Value::Null => return Ok(None),
        Value::Array(list) => list,
        _ => return Err(RpcError::invalid("then is an array of command ids")),
    };
    if list.len() >= PLAN_STEPS {
        return Err(RpcError::invalid(format!(
            "then has at most {} command ids",
            PLAN_STEPS - 1
        )));
    }
    list.iter()
        .map(|id| match id {
            Value::String(id) if !id.is_empty() => Ok(id.clone()),
            _ => Err(RpcError::invalid("then has command ids: non-empty strings")),
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// The listed command with this global id (any state), refused as the
/// palette refuses it: unknown (`missing`), the palette's own (`invalid`).
fn listed_command(items: &[Item], global: &str) -> Result<Item, RpcError> {
    let item = items
        .iter()
        .find(|item| item.global == global)
        .cloned()
        .ok_or_else(|| {
            let mut error = RpcError::new("missing", format!("no command {global}"));
            error.detail = Some(
                [("id".to_owned(), Value::from(global))]
                    .into_iter()
                    .collect(),
            );
            error
        })?;
    if item.owner == bindings::HOST && item.id == PALETTE {
        return Err(RpcError::invalid("the command palette is not run this way"));
    }
    Ok(item)
}

/// The listed command with this global id, checked as the palette checks
/// a row before running it.
fn runnable(core: &Core, global: &str) -> Result<Item, RpcError> {
    let item = listed_command(&core.listed_items(), global)?;
    if !item.enabled {
        let mut error = RpcError::new("disabled", format!("{} is disabled", item.label));
        if !item.reason.is_empty() {
            error.message = item.reason.clone();
            error.detail = Some(
                [("reason".to_owned(), Value::from(item.reason.clone()))]
                    .into_iter()
                    .collect(),
            );
        }
        return Err(error);
    }
    if item.busy {
        return Err(RpcError::new("busy", format!("{} is busy", item.label)));
    }
    Ok(item)
}

/// The shown name of a running plugin.
pub(crate) fn shown_name(core: &Core, owner: u64) -> String {
    core.plugins.get(&owner).map_or_else(
        || owner.to_string(),
        |plugin| {
            crate::packages::package_manager::display_name(
                &plugin.manifest,
                plugin.origin().catalog.as_deref(),
            )
        },
    )
}

/// Runs `item` for `caller` the way the palette does (not added to the
/// palette's recent list), after the line naming it: in the status strip
/// when the caller holds the palette lead (whose ticket this is), else on
/// the OSD.
fn perform(
    core: &mut Core,
    caller: u64,
    ticket: &str,
    item: &Item,
    step: Option<(u64, u64)>,
) -> Result<(), RpcError> {
    let holds_lead = core.lead.as_ref().is_some_and(|lead| lead.owner == caller);
    let line = if holds_lead {
        // The strip's icon says who is working: the step and the command.
        match step {
            Some((step, of)) => i18n::tr_args(
                "{step}/{of} · {command}",
                &json!({"step":step,"of":of,"command":item.label}),
            ),
            None => item.label.clone(),
        }
    } else {
        let name = shown_name(core, caller);
        match step {
            Some((step, of)) => i18n::tr_args(
                "{plugin} · {step}/{of} · {command}",
                &json!({"plugin":name,"step":step,"of":of,"command":item.label}),
            ),
            None => i18n::tr_args(
                "{plugin} · {command}",
                &json!({"plugin":name,"command":item.label}),
            ),
        }
    };
    let place = if holds_lead { "strip" } else { "osd" };
    diag!(
        "commands.run: {} ran {} ({place}: {line})",
        core.plugin_id(caller),
        item.global
    );
    // In the host log too: what a plugin ran on the user's behalf, for a bug
    // report from an installed build. The ids only: a label may name the
    // user's file (a Recently Played row), and the host log never does.
    let logged = step.map_or_else(String::new, |(step, of)| format!(" (step {step}/{of})"));
    crate::diag::log_only(&format!(
        "commands.run: {} ran {}{logged}",
        core.plugin_id(caller),
        item.global
    ));
    if holds_lead {
        let progress = step.map_or(1., |(step, of)| step as f32 / of.max(1) as f32);
        // The strip is the interface's (plan step S2f(3)).
        core.ui_command(UiCommand::Lead(LeadOp::Step {
            ticket: ticket.to_owned(),
            line,
            progress,
        }));
    } else {
        core.show_osd(&line);
    }
    // Run on the caller's behalf: a confirmation goes where the caller's would.
    core.origins.inherit(caller, item.owner);
    core.execute_command(item.owner, &item.id)
}

/// `commands.run` (the order and rules are the SDK's TestHost's, 1.7.16):
/// checked (`step`/`of`, `then`'s shape, the command, `then`'s ids), counted
/// against the ticket, then run now or after the user's answer on the
/// approval sheet (`Event::CommandRunAnswered`). With `then`, the ticket's
/// earlier plan goes; when any command of the plan asks, the sheet lists the
/// whole plan once, before the first step, and Run keeps `then` as the
/// ticket's plan. Without `then`, a command in that plan runs without
/// asking (its first occurrence used up, even a destructive one); any other
/// asks as it would alone.
fn run(core: &mut Core, reply: &Reply, params: &Value) -> Handled {
    let global = string(params, "id")?;
    let ticket = match &params["ticket"] {
        Value::String(ticket) => ticket.clone(),
        Value::Null => {
            return Err(RpcError::new(
                "ticket",
                "commands.run needs the ticket of a palette.lead event",
            ));
        }
        _ => return Err(RpcError::invalid("ticket requires string")),
    };
    let step = step(params)?;
    let then = then(params)?;
    let item = runnable(core, global)?;
    // The rest of the plan: listed now (their state is checked when each
    // runs; an earlier step may enable a later one).
    let mut plan = vec![item.clone()];
    if let Some(then) = &then {
        let items = core.listed_items();
        for global in then {
            plan.push(listed_command(&items, global)?);
        }
    }
    let trusted = trusted_by(core, reply.owner).to_vec();
    let steps: Vec<Judged> = plan
        .into_iter()
        .map(|item| Judged::new(core, &trusted, item))
        .collect();
    let plan_asks = steps.iter().any(Judged::dangerous);
    let now = core.now_ms();
    #[cfg(debug_assertions)]
    {
        core.check_settled(&[crate::host::outbox::Fact::Sheet]);
    }
    // A confirmation on the sheet (`dialogs.confirm`, H5): one sheet at a
    // time, as `admit` keeps it among runs; a dead ticket is refused as such.
    if core.confirm_waiting()
        && core
            .command_runs
            .would_ask(reply.owner, &ticket, global, then.is_some(), plan_asks)
        && core.command_runs.query(reply.owner, &ticket, now).is_some()
    {
        // The confirmation to answer first comes into view.
        core.ui_command(crate::host::messages::UiCommand::ShowWaiting);
        return Err(RpcError::new(
            "busy",
            "another confirmation is waiting for the user's answer",
        ));
    }
    let asks =
        core.command_runs
            .admit(reply.owner, &ticket, global, then.is_some(), plan_asks, now)?;
    if !asks {
        // Nothing asks, or a step of the approved plan (used up now): no
        // sheet, and no new plan is kept.
        perform(core, reply.owner, &ticket, &item, step)?;
        // After the lines the run's interface part writes (a column
        // toggle's window tick), as before (plan step S2f(3),
        // `outbox::Then::Respond`).
        core.respond_in_order(reply.clone(), Ok(json!({"ran": true})));
        return Ok(None);
    }
    let query = core
        .command_runs
        .query(reply.owner, &ticket, now)
        .unwrap_or_default()
        .to_owned();
    let (remember, names): (Vec<String>, Vec<String>) = trust_offer(&steps).into_iter().unzip();
    ask(core, reply, &ticket, &steps, &query, &names);
    core.command_runs.confirming.push(ConfirmedRun {
        reply: reply.clone(),
        ticket,
        global: global.to_owned(),
        step,
        then,
        remember,
        names,
    });
    Ok(None)
}

/// The host's approval sheet before the plan `steps` runs (the first is the
/// run's own), with "Don't ask again" for the plugins named `remember`
/// (none: no checkbox): `host/review_sheet.rs` shows it in the main window,
/// after any native dialog closes. Headless (tests) there is no sheet:
/// `VESTRA_TEST_CONFIRM=run` answers Run for the whole sheet, `always` Run
/// with "Don't ask again" checked (remembered only where the sheet would
/// offer it), anything else Don't Run.
fn ask(
    core: &mut Core,
    reply: &Reply,
    ticket: &str,
    steps: &[Judged],
    query: &str,
    remember: &[String],
) {
    if core.headless {
        let answer = std::env::var("VESTRA_TEST_CONFIRM");
        let yes = matches!(answer.as_deref(), Ok("run" | "always"));
        let always = answer.as_deref() == Ok("always");
        let globals: Vec<&str> = steps.iter().map(|step| step.item.global.as_str()).collect();
        diag!(
            "commands.run: {} asks to run {} (headless: {yes}{})",
            core.plugin_id(reply.owner),
            globals.join(", "),
            if always && !remember.is_empty() {
                ", remember"
            } else {
                ""
            }
        );
        let _ = core
            .events
            .send(Event::CommandRunAnswered(reply.clone(), yes, always));
        return;
    }
    let rows: Vec<review_sheet::Step> = steps
        .iter()
        .map(|step| review_sheet::Step {
            label: step.item.label.clone(),
            plugin: step.item.plugin.clone(),
            reason: reason(&step.item, step.builtin),
            asks: step.dangerous(),
        })
        .collect();
    // The sheet is the interface's (`UiCommand::Review`, plan step S2c(5)).
    core.ui_command(UiCommand::Review(ReviewRequest {
        reply: reply.clone(),
        kind: ReviewKind::CommandRun(RunSheet {
            ticket: ticket.to_owned(),
            query: query.to_owned(),
            steps: rows,
            remember: review_sheet::remember_text(remember),
        }),
    }));
}

/// The reason a step shows on the sheet (Ask Vestra UX §4.5, user ruling 2
/// of 2026-11-12: facts only, none for a destructive command): the
/// player's Close and Quit; none for a plugin that is not built in (user,
/// 2026-11-23: the confirmation is enough).
fn reason(item: &Item, builtin: bool) -> String {
    if !builtin {
        return String::new();
    }
    if item.owner != bindings::HOST {
        return String::new();
    }
    match item.id.as_str() {
        "stop" => i18n::tr("Closes the video"),
        "quit" => i18n::tr("Quits the player"),
        _ => String::new(),
    }
}

/// The user answered the sheet of a run (`Event::CommandRunAnswered`): Run
/// with "Don't ask again" checked (`remember`) first trusts the plugins the
/// sheet offered (the host's list, not the view's); Run keeps the run's
/// `then` as the ticket's plan and runs this one if the ticket is still
/// live and the command still runnable; Don't Run runs nothing, keeps no
/// plan and trusts nothing. The caller gets the answer either way.
pub(crate) fn answered(core: &mut Core, reply: Reply, yes: bool, remember: bool) {
    let Some(run) = core.command_runs.answered(&reply) else {
        return;
    };
    if !yes {
        core.respond(reply, Ok(json!({"ran": false, "declined": true})));
        return;
    }
    if remember && !run.remember.is_empty() {
        // Re-resolved now: a plugin uninstalled while the sheet was open
        // is not trusted again.
        let still: Vec<String> = run
            .remember
            .iter()
            .filter(|id| core.trustable_plugin(id))
            .cloned()
            .collect();
        trust_plugins(core, reply.owner, &still);
    }
    let now = core.now_ms();
    let result = if core
        .command_runs
        .query(reply.owner, &run.ticket, now)
        .is_none()
    {
        Err(RpcError::new(
            "ticket",
            "the ticket ended while the user was asked",
        ))
    } else {
        if let Some(then) = run.then {
            core.command_runs.set_plan(&run.ticket, then);
        }
        runnable(core, &run.global)
            .and_then(|item| perform(core, reply.owner, &run.ticket, &item, run.step))
            .map(|()| json!({"ran": true}))
    };
    // After the lines the run's interface part writes, as at once (plan
    // step S2f(3)).
    core.respond_in_order(reply, result);
}

impl Capability for CommandsRun {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Nothing outlives its process: tickets end with it.
    fn record(&self, _: &Core, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        core.command_runs.rekey(from, to);
    }
    fn revoke(&self, core: &mut Core, owner: u64, _keep_recorded: bool) {
        core.command_runs.revoke(owner);
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "commands.list" => {
                let items = core.listed_items();
                // `dangerous` as this caller's runs judge it (H4).
                let trusted = trusted_by(core, reply.owner);
                let commands: Vec<Value> = items
                    .iter()
                    .map(|item| listed(core, trusted, item))
                    .collect();
                Ok(Some(json!({
                    "revision": core.commands.revision(),
                    "commands": commands,
                })))
            }
            _ => run(core, reply, params),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(owner: u64, id: &str, path: &[&str]) -> Item {
        Item {
            owner,
            id: id.into(),
            path_keys: path.iter().map(|s| (*s).to_owned()).collect(),
            ..Item::default()
        }
    }

    #[test]
    fn a_ticket_is_the_owners_ends_when_done_and_a_newer_one_replaces_it() {
        let mut runs = CommandRuns::default();
        let first = runs.mint(7, "warmer", 0);
        assert_eq!(runs.query(7, &first, 10), Some("warmer"));
        // Another plugin's ticket is no ticket.
        assert_eq!(runs.count(8, &first, 10).unwrap_err().code, "ticket");
        assert_eq!(runs.count(7, "made-up", 10).unwrap_err().code, "ticket");
        runs.count(7, &first, 10).unwrap();
        // A new request: the earlier ticket ends.
        let second = runs.mint(7, "faster", 20);
        assert_eq!(runs.count(7, &first, 30).unwrap_err().code, "ticket");
        runs.count(7, &second, 30).unwrap();
        runs.done(8, &second);
        runs.count(7, &second, 300).unwrap();
        runs.done(7, &second);
        assert_eq!(runs.count(7, &second, 600).unwrap_err().code, "ticket");
        assert_eq!(runs.live(), 0);
    }

    #[test]
    fn a_ticket_unused_for_two_minutes_ends() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "", 1_000);
        runs.count(1, &ticket, 1_000 + TICKET_IDLE_MS).unwrap();
        // Each run restarts the wait.
        let later = 1_000 + 2 * TICKET_IDLE_MS;
        runs.count(1, &ticket, later).unwrap();
        assert_eq!(
            runs.count(1, &ticket, later + TICKET_IDLE_MS + 1)
                .unwrap_err()
                .code,
            "ticket"
        );
        assert_eq!(runs.live(), 0);
    }

    #[test]
    fn runs_are_capped_per_second_per_ticket_and_per_minute() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "", 0);
        for i in 0..RUNS_PER_SECOND as u64 {
            runs.count(1, &ticket, i * 10).unwrap();
        }
        assert_eq!(runs.count(1, &ticket, 100).unwrap_err().code, "limit");
        // A second later there is room again.
        runs.count(1, &ticket, 1_000).unwrap();
        // 30 per ticket.
        let mut now = 2_000;
        while runs.count(1, &ticket, now).is_ok() {
            now += 300;
        }
        assert_eq!(
            runs.count(1, &ticket, now + 5_000).unwrap_err().code,
            "limit"
        );
        assert_eq!(runs.tickets[&ticket].runs, RUNS_PER_TICKET);
        // 120 a minute across tickets.
        let mut runs = CommandRuns::default();
        let mut done = 0;
        let mut now = 0;
        while done < RUNS_PER_MINUTE {
            let ticket = runs.mint(1, "", now);
            for _ in 0..RUNS_PER_SECOND {
                if done == RUNS_PER_MINUTE {
                    break;
                }
                runs.count(1, &ticket, now).unwrap();
                done += 1;
            }
            now += 1_000;
        }
        assert!(now < 60_000);
        let ticket = runs.mint(1, "", now);
        assert_eq!(runs.count(1, &ticket, now).unwrap_err().code, "limit");
        // Another plugin has its own minute.
        let other = runs.mint(2, "", now);
        runs.count(2, &other, now).unwrap();
        runs.count(1, &ticket, 60_000).unwrap();
    }

    #[test]
    fn a_stopped_plugin_loses_its_tickets_and_a_woken_one_keeps_them() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(5, "x", 0);
        runs.rekey(5, 9);
        runs.count(9, &ticket, 1).unwrap();
        assert_eq!(runs.count(5, &ticket, 2).unwrap_err().code, "ticket");
        runs.revoke(9);
        assert_eq!(runs.count(9, &ticket, 3).unwrap_err().code, "ticket");
    }

    #[test]
    fn destructive_and_third_party_commands_ask_first() {
        // Untrusted (the trusted case: the next test).
        let asks = |item: &Item, builtin: bool| dangerous(item, builtin, false);
        let plain = item(3, "louder", &["Audio", "Louder"]);
        assert!(!asks(&plain, true));
        // Any command of a plugin that is not built-in.
        assert!(asks(&plain, false));
        let mut marked = plain.clone();
        marked.destructive = true;
        assert!(asks(&marked, true));
        // The player's Close and Quit; the @settings and @reset menus.
        assert!(asks(&item(bindings::HOST, "stop", &["Close"]), true));
        assert!(asks(&item(bindings::HOST, "quit", &["Quit"]), true));
        assert!(!asks(
            &item(bindings::HOST, "play-pause", &["Play/Pause"]),
            true
        ));
        assert!(asks(&item(4, "theme", &["Settings", "Theme"]), true));
        assert!(asks(&item(4, "picture", &["Reset", "Picture"]), true));
        // The whole path normalized: a menu id, an old root with items below.
        assert!(asks(&item(4, "theme", &["@settings", "Theme"]), true));
        assert!(asks(&item(4, "look", &["设置", "外观", "Dark"]), true));
        assert!(asks(&item(4, "start", &["@reset", "Start Over"]), true));
        // "Plugins" is a category, not a built-in menu.
        assert!(!asks(&item(4, "manage", &["Plugins", "Manage…"]), true));
        // A dialog ("…") does not ask by itself.
        assert!(!asks(
            &item(4, "tune", &["Subtitles", "Subtitle Tuning…"]),
            true
        ));
    }

    /// H4: a trusted plugin's ordinary commands no longer ask; its
    /// destructive ones (and any under @settings or @reset) still do.
    #[test]
    fn a_trusted_plugins_ordinary_commands_do_not_ask() {
        let plain = item(3, "last30", &["Export", "Export Last 30 Seconds"]);
        assert!(dangerous(&plain, false, false));
        assert!(!dangerous(&plain, false, true));
        let mut marked = plain.clone();
        marked.destructive = true;
        assert!(always_asks(&marked));
        assert!(dangerous(&marked, false, true));
        assert!(dangerous(
            &item(3, "theme", &["@settings", "Export Theme"]),
            false,
            true
        ));
        assert!(!always_asks(&plain));
    }

    fn judged(owner: u64, id: &str, plugin: &str, builtin: bool, trusted: bool) -> Judged {
        let mut item = item(owner, id, &["Tools", id]);
        item.plugin = plugin.to_uppercase();
        Judged {
            item,
            builtin,
            plugin_id: (owner != bindings::HOST).then(|| plugin.to_owned()),
            trusted,
        }
    }

    /// H4, user ruling 3 of 2026-11-12: "Don't ask again" names the plugins
    /// of the steps that ask only because they are not built in, each once,
    /// in plan order; never with a destructive step in the plan.
    #[test]
    fn dont_ask_again_offers_the_untrusted_plugins_and_never_with_a_destructive_step() {
        let export = judged(9, "last30", "zysilm.clipexport", false, false);
        let frame = judged(8, "speaker", "zysilm.autoframe", false, false);
        let mute = judged(bindings::HOST, "mute", "host", true, false);
        let trusted = judged(7, "upscale", "zysilm.shaderupscale", false, true);
        assert_eq!(
            trust_offer(&[
                export.clone(),
                mute.clone(),
                frame.clone(),
                export.clone(),
                trusted.clone()
            ]),
            [
                (
                    "zysilm.clipexport".to_owned(),
                    "ZYSILM.CLIPEXPORT".to_owned()
                ),
                ("zysilm.autoframe".to_owned(), "ZYSILM.AUTOFRAME".to_owned()),
            ]
        );
        // Nothing to remember: built-in or trusted steps only.
        assert!(trust_offer(&[mute.clone(), trusted.clone()]).is_empty());
        // A destructive step anywhere in the plan: no checkbox.
        let mut reset = judged(4, "reset", "vestra.video", true, false);
        reset.item.destructive = true;
        assert!(trust_offer(&[export.clone(), reset]).is_empty());
        let mut wipe = export.clone();
        wipe.item.destructive = true;
        assert!(trust_offer(&[wipe, frame.clone()]).is_empty());
        let close = judged(bindings::HOST, "stop", "host", true, false);
        assert!(trust_offer(&[frame, close]).is_empty());
    }

    #[test]
    fn step_and_of_go_together() {
        assert_eq!(step(&json!({})).unwrap(), None);
        assert_eq!(step(&json!({"step":1,"of":2})).unwrap(), Some((1, 2)));
        for bad in [
            json!({"step":1}),
            json!({"of":2}),
            json!({"step":3,"of":2}),
            json!({"step":0,"of":2}),
            json!({"step":1,"of":100}),
            json!({"step":"1","of":2}),
        ] {
            assert_eq!(step(&bad).unwrap_err().code, "invalid", "{bad}");
        }
    }

    /// `then`: command ids (non-empty strings), the run's own and its
    /// `then` at most `PLAN_STEPS`; absent or null names no plan.
    #[test]
    fn then_names_a_plan_of_at_most_eight_commands() {
        assert_eq!(then(&json!({})).unwrap(), None);
        assert_eq!(then(&json!({"then":null})).unwrap(), None);
        assert_eq!(then(&json!({"then":[]})).unwrap(), Some(vec![]));
        assert_eq!(
            then(&json!({"then":["a/b","a/b"]})).unwrap(),
            Some(vec!["a/b".to_owned(), "a/b".to_owned()])
        );
        let seven = vec!["a/b"; PLAN_STEPS - 1];
        assert_eq!(then(&json!({ "then": seven })).unwrap().unwrap().len(), 7);
        let eight = vec!["a/b"; PLAN_STEPS];
        // The SDK's TestHost's messages, word for word.
        for (bad, message) in [
            (json!({"then":"a/b"}), "then is an array of command ids"),
            (json!({"then":{}}), "then is an array of command ids"),
            (
                json!({"then":[7]}),
                "then has command ids: non-empty strings",
            ),
            (
                json!({"then":[""]}),
                "then has command ids: non-empty strings",
            ),
            (
                json!({"then":[null]}),
                "then has command ids: non-empty strings",
            ),
            (json!({ "then": eight }), "then has at most 7 command ids"),
        ] {
            let error = then(&bad).unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                ("invalid", message),
                "{bad}"
            );
        }
    }

    /// A `then` id must name a listed command other than the palette's own.
    #[test]
    fn a_plan_names_listed_commands() {
        let mut hello = item(3, "hello", &["Tools", "Hello"]);
        hello.global = "x/hello".into();
        hello.enabled = false;
        let mut palette = item(bindings::HOST, PALETTE, &["Command Palette…"]);
        palette.global = "host/palette".into();
        let items = [hello, palette];
        // Disabled is fine: it is checked when it runs.
        assert_eq!(listed_command(&items, "x/hello").unwrap().id, "hello");
        let missing = listed_command(&items, "x/none").unwrap_err();
        assert_eq!(missing.code, "missing");
        assert_eq!(
            missing.detail.unwrap().get("id"),
            Some(&Value::from("x/none"))
        );
        assert_eq!(
            listed_command(&items, "host/palette").unwrap_err().code,
            "invalid"
        );
    }

    /// A run of `owner` waiting for the user on the sheet.
    fn waiting_run(runs: &mut CommandRuns, owner: u64, ticket: &str) {
        runs.confirming.push(ConfirmedRun {
            reply: Reply { owner, id: 1 },
            ticket: ticket.into(),
            global: "a/wipe".into(),
            step: None,
            then: None,
            remember: Vec::new(),
            names: Vec::new(),
        });
    }

    /// `admit`: what asks and what does not; a planned step uses up its
    /// entry; a new plan drops the old one.
    #[test]
    fn admit_decides_who_asks_and_keeps_the_plan() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "plan", 0);
        assert!(!runs.admit(1, &ticket, "a/louder", false, false, 0).unwrap());
        assert!(runs.admit(1, &ticket, "a/wipe", false, true, 0).unwrap());
        runs.set_plan(&ticket, vec!["a/wipe".into()]);
        // Planned: no ask, the entry used up; then it asks again.
        assert!(!runs.admit(1, &ticket, "a/wipe", false, true, 0).unwrap());
        assert!(runs.admit(1, &ticket, "a/wipe", false, true, 0).unwrap());
        // A new plan (`then`, even `[]`) drops the rest of the old one.
        runs.set_plan(&ticket, vec!["a/wipe".into()]);
        assert!(runs.admit(1, &ticket, "a/wipe", true, true, 1_000).unwrap());
        assert!(!runs.planned(1, &ticket, "a/wipe"));
        assert_eq!(runs.tickets[&ticket].runs, 5);
    }

    /// A waiting sheet forgets a plugin uninstalled or replaced meanwhile,
    /// and everything on Ask Again.
    #[test]
    fn a_waiting_sheet_forgets_a_replaced_plugin_and_everything_on_ask_again() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "x", 0);
        waiting_run(&mut runs, 1, &ticket);
        runs.confirming[0].remember = vec!["a.b".into(), "c.d".into()];
        runs.confirming[0].names = vec!["A B".into(), "C D".into()];
        let reply = runs.confirming[0].reply.clone();
        runs.forget_remembered(Some("a.b"));
        assert_eq!(runs.confirming[0].remember, ["c.d"]);
        // The names follow: the sheet's checkbox names only what is left.
        assert_eq!(runs.offered(&reply), Some(&["C D".to_owned()][..]));
        runs.forget_remembered(Some("x.y"));
        assert_eq!(runs.confirming[0].remember, ["c.d"]);
        runs.forget_remembered(None);
        assert!(runs.confirming[0].remember.is_empty());
        assert_eq!(runs.offered(&reply), Some(&[][..]));
        assert_eq!(runs.offered(&Reply { owner: 1, id: 9 }), None);
    }

    /// Review fix 1: a call that would ask while a sheet waits fails `busy`
    /// before it is counted and before it touches the plan; a call that
    /// does not ask (plain, or planned) still runs.
    #[test]
    fn a_busy_refusal_is_not_counted_and_keeps_the_plan() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "plan", 0);
        runs.set_plan(&ticket, vec!["a/wipe".into()]);
        waiting_run(&mut runs, 1, &ticket);
        for then in [false, true] {
            let error = runs
                .admit(1, &ticket, "a/forget", then, true, 0)
                .unwrap_err();
            assert_eq!(error.code, "busy");
            assert_eq!(
                error.message,
                "another command is waiting for the user's answer"
            );
        }
        assert_eq!(runs.tickets[&ticket].runs, 0);
        assert!(runs.planned(1, &ticket, "a/wipe"));
        // Not asking: runs (counted) while the sheet waits.
        assert!(!runs.admit(1, &ticket, "a/louder", false, false, 0).unwrap());
        assert!(!runs.admit(1, &ticket, "a/wipe", false, true, 0).unwrap());
        assert_eq!(runs.tickets[&ticket].runs, 2);
        // A dead ticket is refused as one, not as busy.
        assert_eq!(
            runs.admit(1, "made-up", "a/forget", false, true, 0)
                .unwrap_err()
                .code,
            "ticket"
        );
    }

    /// Review fix 2: one sheet at a time, whoever's: another plugin's run
    /// that would ask fails `busy` instead of replacing the waiting sheet.
    #[test]
    fn another_callers_run_that_asks_waits_for_no_sheet_but_fails_busy() {
        let mut runs = CommandRuns::default();
        let first = runs.mint(1, "first", 0);
        let second = runs.mint(2, "second", 0);
        waiting_run(&mut runs, 1, &first);
        assert_eq!(
            runs.admit(2, &second, "b/wipe", false, true, 0)
                .unwrap_err()
                .code,
            "busy"
        );
        assert_eq!(runs.tickets[&second].runs, 0);
        // The first sheet's run still waits for its answer.
        assert!(runs.waiting(&Reply { owner: 1, id: 1 }));
        // Once answered, the other caller may ask.
        assert!(runs.answered(&Reply { owner: 1, id: 1 }).is_some());
        assert!(runs.admit(2, &second, "b/wipe", false, true, 0).unwrap());
    }

    /// The ticket's plan: each id of it runs once without asking (repeats
    /// counted, first occurrence used up); a new plan replaces the rest;
    /// another plugin's or a new request's ticket has none.
    #[test]
    fn a_plan_runs_each_of_its_ids_once_without_asking() {
        let mut runs = CommandRuns::default();
        let ticket = runs.mint(1, "plan", 0);
        let ids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(!runs.take_planned(1, &ticket, "a/wipe"));
        runs.set_plan(&ticket, ids(&["a/wipe", "b/export", "a/wipe"]));
        // Another plugin's ticket: nothing.
        assert!(!runs.take_planned(2, &ticket, "a/wipe"));
        assert!(runs.take_planned(1, &ticket, "a/wipe"));
        assert!(runs.take_planned(1, &ticket, "a/wipe"));
        assert!(!runs.take_planned(1, &ticket, "a/wipe"));
        // A new plan replaces what is left.
        runs.set_plan(&ticket, ids(&["d/close"]));
        assert!(!runs.take_planned(1, &ticket, "b/export"));
        runs.set_plan(&ticket, ids(&["d/close"]));
        // A new request's ticket starts with none; the old one ended.
        let next = runs.mint(1, "again", 10);
        assert!(!runs.take_planned(1, &next, "d/close"));
        assert!(!runs.take_planned(1, &ticket, "d/close"));
    }

    /// H7: the sheet's reasons are facts: the player's Close and Quit;
    /// none for a destructive command (user ruling 2 of 2026-11-12), a plain
    /// one or one of a plugin that is not built in (user, 2026-11-23).
    #[test]
    fn the_sheet_gives_reasons_only_for_facts() {
        let mut reset = item(4, "reset", &["Picture", "Reset"]);
        reset.destructive = true;
        assert_eq!(reason(&reset, true), "");
        assert_eq!(reason(&item(4, "louder", &["Audio"]), true), "");
        assert_eq!(reason(&reset, false), "");
        assert_eq!(reason(&item(9, "last30", &["Export"]), false), "");
        assert_eq!(reason(&item(bindings::HOST, "stop", &["Close"]), false), "");
        assert_eq!(
            reason(&item(bindings::HOST, "stop", &["Close"]), true),
            "Closes the video"
        );
        assert_eq!(
            reason(&item(bindings::HOST, "quit", &["Quit"]), true),
            "Quits the player"
        );
        assert_eq!(
            reason(&item(bindings::HOST, "play-pause", &["Play/Pause"]), true),
            ""
        );
    }
}
