//! `commands.run` (built-in only, risky): the palette's command table
//! (`commands.list`) and running one of its commands the way the palette
//! runs it (`commands.run`), with a one-time ticket the palette minted for
//! a user's action (`palette.lead`, capabilities/palette_lead.rs).
//!
//! The boundary is the player's, not the plugin's (plugin processes are not
//! sandboxed): a run needs a live ticket of the caller's, the rates are
//! capped ([`CommandRuns`]), a destructive command and any command of a
//! plugin that is not built-in asks the user first (the host's own alert,
//! which the caller cannot answer or skip), and every run puts an OSD line
//! naming the caller and the command (doc/architecture/palette-lead.md).
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::{
    bindings,
    host::{Event, Host, Reply, RpcError, palette::PALETTE, string},
    i18n,
    ui::command_palette::Item,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

pub struct CommandsRun;
static INFO: Info = Info {
    name: "commands.run",
    label: "Run any player or plugin command",
    risky: true,
    requires: &[],
    residency: Residency::None,
    builtin_only: true,
};
static METHODS: &[Method] = &[Method::this("commands.list"), Method::this("commands.run")];

/// A ticket ends this long after it was minted or last ran a command.
pub const TICKET_IDLE_MS: u64 = 120_000;
/// Commands one ticket may run.
pub const RUNS_PER_TICKET: u32 = 30;
/// Runs per second, per ticket.
pub const RUNS_PER_SECOND: usize = 4;
/// Runs per minute, per plugin (all its tickets).
pub const RUNS_PER_MINUTE: usize = 120;
/// The longest request text kept with a ticket (quoted in the alert).
const QUERY_KEPT: usize = 200;
/// The player's own commands that ask first: Playback ▸ Close and Quit.
/// Only the player's: a plugin marks its own with `CommandOptions.destructive`
/// (the core never lists plugin commands).
const HOST_DESTRUCTIVE: [&str; 2] = ["stop", "quit"];
/// The built-in top-level menus whose every command asks first, by the key
/// `commands::normalize` gives them: `@settings` and `@reset`.
const DESTRUCTIVE_MENUS: [&str; 2] = ["Settings", "Reset"];

/// One ticket: whose, for which request, and its runs.
#[derive(Debug)]
struct Ticket {
    owner: u64,
    query: String,
    /// Minted or last run (ms of `Host::now_ms`).
    last: u64,
    runs: u32,
    /// Run times within the last second.
    recent: VecDeque<u64>,
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

/// A run waiting for the user's answer to the host's alert.
#[derive(Clone)]
pub struct ConfirmedRun {
    pub reply: Reply,
    pub ticket: String,
    pub global: String,
    pub step: Option<(u64, u64)>,
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
    /// The run waiting for the alert answered for `reply`, if its caller
    /// still runs.
    pub fn answered(&mut self, reply: &Reply) -> Option<ConfirmedRun> {
        let at = self
            .confirming
            .iter()
            .position(|run| run.reply.owner == reply.owner && run.reply.id == reply.id)?;
        Some(self.confirming.remove(at))
    }
    #[cfg(test)]
    fn live(&self) -> usize {
        self.tickets.len()
    }
}

/// Whether running `item` asks first: a destructive command (its own
/// `destructive`, the player's Close and Quit, anything in the built-in
/// `@settings` and `@reset` menus) or any command of a plugin that is not built-in. Opening
/// a dialog ("…") does not ask: the user is in the dialog then.
pub fn dangerous(item: &Item, builtin: bool) -> bool {
    !builtin
        || item.destructive
        || (item.owner == bindings::HOST && HOST_DESTRUCTIVE.contains(&item.id.as_str()))
        || item
            .path_keys
            .first()
            .is_some_and(|root| DESTRUCTIVE_MENUS.contains(&root.as_str()))
}

/// Whether the commands of `owner` are a built-in's: the player's own, a
/// running built-in, or a built-in in standby (its recorded commands).
fn builtin_owner(host: &Host, owner: u64) -> bool {
    owner == bindings::HOST
        || host.is_builtin(owner)
        || host
            .on_demand
            .held(owner)
            .is_some_and(|held| host.is_builtin_package(&held.origin))
}

/// One row of `commands.list`.
fn listed(host: &Host, item: &Item) -> Value {
    let builtin = builtin_owner(host, item.owner);
    let keys = host.input.borrow().table.labels(item.owner, &item.id);
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
        "dangerous": dangerous(item, builtin),
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

/// The listed command with this global id, checked as the palette checks
/// a row before running it.
fn runnable(host: &Host, global: &str) -> Result<Item, RpcError> {
    let item = host
        .palette_items()
        .into_iter()
        .find(|item| item.global == global)
        .ok_or_else(|| RpcError::new("missing", format!("no command {global}")))?;
    if item.owner == bindings::HOST && item.id == PALETTE {
        return Err(RpcError::invalid("the command palette is not run this way"));
    }
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
fn shown_name(host: &Host, owner: u64) -> String {
    host.plugins.get(&owner).map_or_else(
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
/// palette's recent list), after the OSD line naming it.
fn perform(
    host: &mut Host,
    caller: u64,
    item: &Item,
    step: Option<(u64, u64)>,
) -> Result<(), RpcError> {
    let name = shown_name(host, caller);
    let line = match step {
        Some((step, of)) => i18n::tr_args(
            "{plugin} · {step}/{of} · {command}",
            &json!({"plugin":name,"step":step,"of":of,"command":item.label}),
        ),
        None => i18n::tr_args(
            "{plugin} · {command}",
            &json!({"plugin":name,"command":item.label}),
        ),
    };
    diag!(
        "commands.run: {} ran {} (osd: {line})",
        host.plugin_id(caller),
        item.global
    );
    host.show_osd(&line);
    host.execute_command(item.owner, &item.id)
}

/// `commands.run`: checked, counted against the ticket, then run now or
/// after the user's answer (`Event::CommandRunAnswered`).
fn run(host: &mut Host, reply: &Reply, params: &Value) -> Handled {
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
    let item = runnable(host, global)?;
    let now = host.now_ms();
    host.command_runs.count(reply.owner, &ticket, now)?;
    let builtin = builtin_owner(host, item.owner);
    if !dangerous(&item, builtin) {
        perform(host, reply.owner, &item, step)?;
        return Ok(Some(json!({"ran": true})));
    }
    let query = host
        .command_runs
        .query(reply.owner, &ticket, now)
        .unwrap_or_default()
        .to_owned();
    ask(host, reply, &item, builtin, &query)?;
    host.command_runs.confirming.push(ConfirmedRun {
        reply: reply.clone(),
        ticket,
        global: global.to_owned(),
        step,
    });
    Ok(None)
}

/// The host's alert before a dangerous run: the caller's name, the command,
/// the request and why it asks; "Run" / "Don't Run". Headless (tests) there
/// is no alert: `VESTRA_TEST_CONFIRM=run` answers Run, anything else Don't Run.
fn ask(
    host: &mut Host,
    reply: &Reply,
    item: &Item,
    builtin: bool,
    query: &str,
) -> Result<(), RpcError> {
    let events = host.events.clone();
    let answer = reply.clone();
    if host.headless {
        let yes = std::env::var("VESTRA_TEST_CONFIRM").as_deref() == Ok("run");
        diag!(
            "commands.run: {} asks to run {} (headless: {yes})",
            host.plugin_id(reply.owner),
            item.global
        );
        let _ = events.send(Event::CommandRunAnswered(answer, yes));
        return Ok(());
    }
    if host.dialog_busy {
        return Err(RpcError::new("busy", "a native dialog is already open"));
    }
    let name = shown_name(host, reply.owner);
    let message = i18n::tr_args(
        "{plugin} wants to run “{command}”",
        &json!({"plugin":name,"command":item.label}),
    );
    let why = if builtin {
        i18n::tr("This command cannot be undone.")
    } else {
        i18n::tr_args(
            "This command belongs to {owner}, which is not a built-in plugin: the player cannot tell what it does.",
            &json!({"owner":item.plugin}),
        )
    };
    let detail = if query.trim().is_empty() {
        why
    } else {
        let request = i18n::tr_args("Request: “{}”", &json!([query.trim()]));
        format!("{request}\n\n{why}")
    };
    let options = crate::ui::confirm::Options {
        message: message.chars().take(200).collect(),
        detail: Some(detail.chars().take(2000).collect()),
        confirm: i18n::tr("Run"),
        cancel: i18n::tr("Don't Run"),
        destructive: true,
        checkbox: None,
    };
    host.dialog_busy = true;
    #[cfg(feature = "producer-audio")]
    host.physical_revoke(crate::playback::producer_input::RevokeReason::Dialog);
    if let Err(error) = crate::platform::confirm(&options, host.native_window, move |yes, _| {
        let _ = events.send(Event::CommandRunAnswered(answer.clone(), yes));
    }) {
        host.dialog_busy = false;
        return Err(error);
    }
    Ok(())
}

/// The user answered the alert of a run (`Event::CommandRunAnswered`): run
/// it if the ticket is still live and the command still runnable, and
/// answer the caller.
pub(crate) fn answered(host: &mut Host, reply: Reply, yes: bool) {
    if !host.headless {
        host.dialog_busy = false;
    }
    let Some(run) = host.command_runs.answered(&reply) else {
        return;
    };
    if !yes {
        host.respond(reply, Ok(json!({"ran": false, "declined": true})));
        return;
    }
    let now = host.now_ms();
    let result = if host
        .command_runs
        .query(reply.owner, &run.ticket, now)
        .is_none()
    {
        Err(RpcError::new(
            "ticket",
            "the ticket ended while the user was asked",
        ))
    } else {
        runnable(host, &run.global)
            .and_then(|item| perform(host, reply.owner, &item, run.step))
            .map(|()| json!({"ran": true}))
    };
    host.respond(reply, result);
}

impl Capability for CommandsRun {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// Nothing outlives its process: tickets end with it.
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.command_runs.rekey(from, to);
    }
    fn revoke(&self, host: &mut Host, owner: u64, _keep_recorded: bool) {
        host.command_runs.revoke(owner);
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "commands.list" => {
                let items = host.palette_items();
                let commands: Vec<Value> = items.iter().map(|item| listed(host, item)).collect();
                Ok(Some(json!({
                    "revision": host.commands.revision(),
                    "commands": commands,
                })))
            }
            _ => run(host, reply, params),
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
        let plain = item(3, "louder", &["Audio", "Louder"]);
        assert!(!dangerous(&plain, true));
        // Any command of a plugin that is not built-in.
        assert!(dangerous(&plain, false));
        let mut marked = plain.clone();
        marked.destructive = true;
        assert!(dangerous(&marked, true));
        // The player's Close and Quit; the @settings and @reset menus.
        assert!(dangerous(&item(bindings::HOST, "stop", &["Close"]), true));
        assert!(dangerous(&item(bindings::HOST, "quit", &["Quit"]), true));
        assert!(!dangerous(
            &item(bindings::HOST, "play-pause", &["Play/Pause"]),
            true
        ));
        assert!(dangerous(&item(4, "theme", &["Settings", "Theme"]), true));
        assert!(dangerous(&item(4, "picture", &["Reset", "Picture"]), true));
        // "Plugins" is a category, not a built-in menu.
        assert!(!dangerous(
            &item(4, "manage", &["Plugins", "Manage…"]),
            true
        ));
        // A dialog ("…") does not ask by itself.
        assert!(!dangerous(
            &item(4, "tune", &["Subtitles", "Subtitle Tuning…"]),
            true
        ));
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
}
