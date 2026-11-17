//! "The core turn takes no `Host`" (plan step S2f(3), its verification
//! matrix row): a `Core` built on its own, with no `Host` and no interface,
//! handles events (`Core::handle`), reports (`Core::report`), runs its
//! periodic steps (`Core::STEPS`) and serves a capability through the
//! `Capability` trait. That this module compiles is the proof that those
//! entry points take nothing but the core; the test runs a turn and reads
//! what the core asked of the interface in its outbox.
#![cfg(test)]

use super::outbox::Out;
use super::*;

/// A core with no playback worker (`Backend::detached`), no process, no
/// window and a package store under `root`; the events its threads send go
/// to the returned receiver.
fn core(root: &Path) -> (Core, mpsc::Receiver<Event>, mpsc::Receiver<Command>) {
    let (events, received) = mpsc::channel();
    let (backend, commands) = Backend::detached();
    let startup = rendering_startup(true);
    let decision = crate::playback::render_mode::decide(&startup.facts, startup.preference);
    let core = Core {
        backend,
        shared: None,
        shared_policy: Default::default(),
        plugins: BTreeMap::new(),
        plugins_revision: 0,
        events: events.clone(),
        headless: true,
        panel_plugins: Default::default(),
        command_runs: Default::default(),
        clock: Instant::now(),
        sequence: None,
        last_source: Value::Null,
        rendering: Rendering {
            facts: startup.facts.clone(),
            started: decision.mode,
            windows: decision.windows,
            preference: startup.preference,
            file: None,
            relaunch: Relaunch {
                directories: Vec::new(),
                silent: true,
            },
            restarted_at: None,
        },
        source_state_replies: Default::default(),
        host_revision: 0,
        supervisor: supervisor::new(),
        activation_receipts: Default::default(),
        file_types: json!([]),
        window_facts: Value::Null,
        window_facts_seq: None,
        ui_facts: Default::default(),
        restarts: Default::default(),
        restart_lines_sent: BTreeMap::new(),
        power: power::Power::system(),
        last_frame: None,
        held_mounts: Default::default(),
        mounts: BTreeMap::new(),
        mount_queue: std::collections::VecDeque::new(),
        settings: Default::default(),
        prepared: None,
        warnings: Default::default(),
        packages: package_manager::Manager::at(root.join("packages")).unwrap(),
        reaper: process::reaper::Reaper::spawn(events),
        actions_busy: Default::default(),
        step_seek: BTreeMap::new(),
        step_writes: 0,
        development: Default::default(),
        dev_disabled: Default::default(),
        failures: Default::default(),
        start_error: None,
        data_removals: BTreeMap::new(),
        dev_loading: None,
        session_storage: Default::default(),
        bindings_sent: 0,
        bindings_list_sent: Value::Null,
        bindings_seen: [0; 2],
        packages_sent: Value::Null,
        packages_seen: None,
        notified_at: 0,
        keys_checked: 0,
        keys_sent: BTreeMap::new(),
        timeline: Default::default(),
        opens: Default::default(),
        triggers: Default::default(),
        trigger_state: Default::default(),
        commands: commands::Registry::new(),
        tables: Default::default(),
        shader_guard: ShaderGuard::new(startup.guard.clone(), startup.keys.key.clone(), None),
        memory_logged: None,
        dialog_busy: false,
        confirm_open: false,
        origins: Default::default(),
        lead: None,
        panel_tabs: Default::default(),
        info_panels: BTreeMap::new(),
        on_top: window_control::LastWrite::new(false),
        cursor_playing: window_control::LastWrite::new(false),
        window_settings_sent: (None, None),
        cover_art_off: false,
        drops: Default::default(),
        ui_state: ui_state::Store::disabled(),
        area_replies: Vec::new(),
        handed: None,
        startup_panels: true,
        source_display: messages::SourceShown {
            no_video: true,
            empty: false,
            title: "Vestra Player".into(),
        },
        binding_revision: u64::MAX,
        outbox: Default::default(),
        ui_facts_seq: 0,
        handover_seq: 0,
        cancelled_handovers: Vec::new(),
        window: None,
        batch_in_flight: Vec::new(),
        mount_singly: 0,
        retry_due: false,
        clipboard: Default::default(),
        on_demand: supervisor::OnDemand::from_env(),
    };
    (core, received, commands)
}

#[test]
fn a_core_turn_runs_on_a_core_without_a_host() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, commands) = core(root.path());
    // An event of a process that does not run: nothing happens.
    core.handle(Event::Request(42, json!({"id":1,"method":"window.state"})));
    assert!(core.outbox.is_empty());
    // A new `source.state`: the interface's copy, then what it shows, wait
    // in the outbox in that order; nothing is carried out.
    core.handle(Event::Broadcast(
        "source.state".into(),
        json!({"phase":"ready","paused":false,"source":"C:/clips/a.mkv"}),
    ));
    let kinds: Vec<_> = core
        .outbox
        .iter()
        .map(|out| match out {
            Out::Ui(messages::UiCommand::Snapshot(messages::UiSnapshot::LastSource(_))) => {
                "LastSource"
            }
            Out::Ui(messages::UiCommand::Snapshot(messages::UiSnapshot::Source(shown))) => {
                assert_eq!(shown.title, "a.mkv - Vestra Player");
                "Source"
            }
            Out::Ui(_) => "other command",
            Out::Then(_) => "then",
        })
        .collect();
    assert_eq!(kinds, ["LastSource", "Source"]);
    // A report of the interface reaches the core alone: the interface's
    // facts.
    core.report(messages::UiReport::UiFacts(
        1,
        Box::new(messages::UiFacts {
            options_open: true,
            ..Default::default()
        }),
    ));
    assert!(core.ui_facts.options_open);
    // The periodic steps, a headless run's: on the core alone.
    for step in Core::STEPS.iter().filter(|step| !step.windowed) {
        (step.run)(&mut core);
    }
    // A capability on the core alone (`Capability::handle` takes `Core`).
    let route = capabilities::lookup("window.state").unwrap();
    let reply = Reply { owner: 42, id: 2 };
    let state = route
        .handle(&mut core, &reply, "window.state", &Value::Null)
        .unwrap()
        .unwrap();
    assert_eq!(state["fullscreen"], false);
    // No playback command left the core for an event it ignored.
    assert!(commands.try_recv().is_err());
}

/// Plan step S3a(1): what the interface's copies need comes from the core
/// alone. The first call gives every copied value; a call with nothing
/// changed gives nothing; the core's own changes of its mount registry come
/// once each, in order; a changed value comes again.
#[test]
fn the_interfaces_copies_follow_the_core() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let first = core.copies();
    assert!(first.facts.is_some() && first.plugins.is_some() && first.pages.is_some());
    assert!(first.mount_ops.is_empty());
    let none = core.copies();
    assert!(none.facts.is_none() && none.plugins.is_none() && none.pages.is_none());
    // A panel, recorded, moved to the plugin's next run, then gone.
    let key = panel_key(7, composition::Slot::Sidebar);
    let moved = panel_key(8, composition::Slot::Sidebar);
    core.mounts.insert(
        key,
        Mount {
            spec: composition::CompileSpec {
                owner: key,
                file: PathBuf::from("tab.slint"),
                component: "Tab".into(),
                global: Some("Bridge".into()),
                slot: composition::Slot::Sidebar,
            },
            plugin: 7,
            properties: BTreeMap::new(),
            visible: true,
            title: "Tab".into(),
            manifest: "test.tab".into(),
            builtin: false,
            recorded: false,
            call: Value::Null,
        },
    );
    core.mount_recorded(key);
    assert!(core.mount_move(key, moved, 8));
    assert!(core.mount_remove(moved).is_some());
    assert!(core.mount_remove(moved).is_none());
    use super::outbox::MountOp;
    assert_eq!(
        core.copies().mount_ops,
        [
            MountOp::Recorded(key),
            MountOp::Move {
                from: key,
                to: moved,
                plugin: 8
            },
            MountOp::Remove(moved),
        ]
    );
    assert!(core.copies().mount_ops.is_empty());
    core.dialog_busy = true;
    assert!(core.copies().facts.is_some_and(|facts| facts.dialog_busy));
    assert!(core.copies().facts.is_none());
}

/// Plan step S3a(2), change-only reports: where the interface skips a
/// per-tick report because the core's facts (`TickFacts`) say its handler
/// would change nothing, handling it changes nothing indeed: no registry
/// write, no command to the playback worker, no request to the interface,
/// no reply. The reports it still sends reach the core as before.
#[test]
fn a_skipped_tick_report_changes_nothing_in_the_core() {
    use messages::{UiInput, UiReport};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, commands) = core(root.path());
    core.register_host_commands().unwrap();
    let _ = core.copies();
    // Nothing waits: the video area and the mount step are skipped.
    let facts = core.tick_facts();
    assert!(!facts.area_waiting && !facts.held_mounts);
    let unchanged = |core: &mut Core, report: UiReport| {
        let revision = core.commands.revision();
        let host = core.host_revision;
        let replies = core.area_replies.len();
        core.report(report);
        assert_eq!(core.commands.revision(), revision);
        assert_eq!(core.host_revision, host);
        assert_eq!(core.area_replies.len(), replies);
        assert!(core.outbox.is_empty());
        assert!(
            commands.try_recv().is_err(),
            "a command to the playback worker"
        );
    };
    unchanged(
        &mut core,
        UiReport::VideoArea {
            size: (640., 360.),
            now: 5,
        },
    );
    unchanged(&mut core, UiReport::Input(UiInput::MountTick));
    // The window tick: sent while the fullscreen command's state differs;
    // once the core has it (and the cover art as no backdrop wants it),
    // the same report changes nothing.
    let window_tick = |fullscreen| UiReport::Input(UiInput::WindowTick { fullscreen });
    if core.tick_facts().fullscreen_checked != Some(false) {
        core.report(window_tick(false));
    }
    let facts = core.tick_facts();
    assert_eq!(facts.fullscreen_checked, Some(false));
    assert!(!facts.cover_art_off);
    unchanged(&mut core, window_tick(false));
    // A change is not skipped and does write.
    assert_ne!(core.tick_facts().fullscreen_checked, Some(true));
    let revision = core.commands.revision();
    core.report(window_tick(true));
    assert_ne!(core.commands.revision(), revision);
    assert_eq!(core.tick_facts().fullscreen_checked, Some(true));
    // The column toggles: equal to the registry's, nothing changes.
    let [Some(sidebar), Some(adjust)] = core.tick_facts().columns else {
        panic!("the column toggles are host commands");
    };
    unchanged(
        &mut core,
        UiReport::Input(UiInput::ColumnCommands([
            (crate::ui::bar_buttons::SIDEBAR, sidebar),
            (crate::ui::bar_buttons::ADJUST, adjust),
        ])),
    );
}

/// Plan step S3a(3), the S3a brief's risk 3 (the rule moved here from
/// S3a(2), where on one thread today's re-record stayed): a cancel takes
/// the record and tells the interface; the window's report of that change,
/// sent before the cancel reached the interface, is dropped by its
/// hand-over number; a closed window ends the wait, and a change handed
/// over again is recorded.
#[test]
fn a_cancelled_changes_late_report_does_not_bring_it_back() {
    use messages::{MountCancel, UiCommand};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    core.headless = false;
    let key = panel_key(7, composition::Slot::Bottom);
    core.mount_queue.push_back(unmount(7, 3, key));
    core.turn_end();
    let seq = core.handed.as_ref().map(|h| h.seq).expect("handed over");
    let held = super::mounts::HandedMount {
        seq,
        ..core.handed.clone().unwrap()
    };
    core.outbox.clear();
    core.cancel_handed(MountCancel::Owner(7));
    assert!(core.handed.is_none() && core.window.is_none());
    assert!(matches!(
        core.outbox.front(),
        Some(super::outbox::Out::Ui(UiCommand::CancelMount(_)))
    ));
    // The window's report of the cancelled hand-over: stale.
    core.batching(Some(held));
    assert!(core.handed.is_none());
    assert!(!core.mount_pending(key));
    // No hand-over while the cancel waits for the window to report empty.
    core.mount_queue.push_back(unmount(7, 4, key));
    core.outbox.clear();
    core.turn_end();
    assert!(!handed_over(&core));
    // The window reported itself empty: the next change is handed over.
    core.batching(None);
    core.turn_end();
    assert!(handed_over(&core) && core.handed.is_some());
}

/// A queued unmount of panel `key` of plugin `owner` (request `id`).
fn unmount(owner: u64, id: u64, key: u64) -> MountChange {
    MountChange {
        reply: Reply { owner, id },
        key,
        contribution: None,
        announced: false,
    }
}

/// The outbox holds a hand-over.
fn handed_over(core: &Core) -> bool {
    core.outbox.iter().any(|out| {
        matches!(
            out,
            super::outbox::Out::Ui(messages::UiCommand::HandOver { .. })
        )
    })
}

/// The outbox's batches, by their number of changes.
fn batches(core: &Core) -> Vec<usize> {
    core.outbox
        .iter()
        .filter_map(|out| match out {
            super::outbox::Out::Ui(messages::UiCommand::Batch(changes)) => Some(changes.len()),
            _ => None,
        })
        .collect()
}

/// S3a(3) review B1 and B2, and item 6: the core closes the batching window
/// itself 32 ms after the hand-over, joining what was accepted meanwhile;
/// the batch stays pending until its outcome, and no change is handed over
/// while it is in flight. A failed batch's changes come back and are tried
/// one at a time, the first at once.
#[test]
fn a_batch_in_flight_is_pending_and_blocks_the_next_hand_over() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    core.headless = false;
    let (a, b, c) = (
        panel_key(7, composition::Slot::Bottom),
        panel_key(8, composition::Slot::Bottom),
        panel_key(9, composition::Slot::Bottom),
    );
    core.mount_queue.push_back(unmount(7, 1, a));
    core.turn_end();
    assert!(handed_over(&core));
    // Accepted while the window is open: joins at the close.
    core.mount_queue.push_back(unmount(8, 2, b));
    core.outbox.clear();
    core.turn_end();
    assert!(batches(&core).is_empty(), "not due yet");
    // Due: the window closes on the core's clock.
    core.window.as_mut().unwrap().1 = Instant::now() - Duration::from_millis(40);
    core.outbox.clear();
    core.turn_end();
    assert_eq!(batches(&core), [2]);
    assert!(
        core.mount_pending(a) && core.mount_pending(b),
        "B2: pending in flight"
    );
    assert!(!core.mounts_idle());
    // B1: a change accepted now is not handed over while the batch flies.
    core.mount_queue.push_back(unmount(9, 3, c));
    core.outbox.clear();
    core.turn_end();
    assert!(!handed_over(&core));
    // The batch failed: its changes come back, tried singly, first at once.
    let back = vec![unmount(7, 1, a), unmount(8, 2, b)];
    core.batch_retry(back);
    assert!(core.batch_in_flight.is_empty());
    core.outbox.clear();
    core.turn_end();
    assert!(handed_over(&core));
    core.outbox.clear();
    core.turn_end();
    assert_eq!(batches(&core), [1], "the first alone, due at once");
    // Its outcome: pending no longer, and the next is handed over.
    core.batch_applied(&Reply { owner: 7, id: 1 }, a);
    assert!(!core.mount_pending(a));
    core.outbox.clear();
    core.turn_end();
    assert!(handed_over(&core));
}

/// S3a(3) review B3: a confirmation issued and not yet applied by the
/// interface counts as waiting, so a second one is refused `busy`, as when
/// the sheet's fact had already arrived; the interface's `Applied` ends it.
#[test]
fn an_ask_in_flight_keeps_the_next_one_waiting() {
    use messages::{AskRequest, UiCommand, UiReport};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    assert!(!core.sheet_waiting() && !core.confirm_waiting());
    let options =
        crate::ui::confirm::parse(&json!({"message":"Go?","confirm":"Go","cancel":"Stop"}))
            .unwrap();
    core.ui_command(UiCommand::Ask(AskRequest {
        reply: Reply { owner: 7, id: 1 },
        options,
        request: None,
    }));
    assert!(
        core.sheet_waiting() && core.confirm_waiting(),
        "in the outbox"
    );
    core.flush();
    assert!(core.sheet_waiting(), "in flight");
    let seq = core.outbox.pushed;
    core.report(UiReport::Applied(seq));
    assert!(
        !core.sheet_waiting(),
        "applied: the interface's facts say it now"
    );
}

/// Plan step S3a(3): the core→UI queue. A newer snapshot replaces a waiting
/// one of its kind in place (keeping its number); a command is never
/// coalesced; the numbers have no gaps; a reply kept behind a command whose
/// interface part writes lines (`Then::Respond`) waits for the interface's
/// `Applied` of that command, then goes.
#[test]
fn the_core_to_ui_queue_coalesces_snapshots_and_keeps_replies_behind() {
    use super::outbox::ToUi;
    use messages::{UiCommand, UiReport, UiSnapshot};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let theme = |name: &str| UiCommand::Snapshot(UiSnapshot::Theme(name.into()));
    core.ui_command(theme("a"));
    core.ui_command(UiCommand::Activity);
    core.ui_command(theme("b"));
    core.flush();
    let entries = core.outbox.to_ui.take_all();
    let commands = entries
        .into_iter()
        .filter_map(|entry| match entry {
            ToUi::Command(seq, command) => Some((seq, command)),
            ToUi::Copies(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(commands.len(), 2, "the second theme replaced the first");
    assert!(
        matches!(&commands[0], (1, UiCommand::Snapshot(UiSnapshot::Theme(name))) if name == "b")
    );
    assert!(matches!(commands[1], (2, UiCommand::Activity)));
    assert_eq!(core.outbox.pushed, 2);
    // A reply behind a command whose interface part writes lines.
    core.ui_command(UiCommand::Layout(messages::LayoutOp::Toggle(
        crate::ui::sidebar_tabs::Column::Sidebar,
    )));
    let reply = Reply { owner: 9, id: 4 };
    core.respond_in_order(reply, Ok(Value::Null));
    core.flush();
    assert_eq!(core.outbox.waiting.len(), 1, "the reply waits");
    assert!(core.lines_pending());
    core.report(UiReport::Applied(2));
    assert_eq!(core.outbox.waiting.len(), 1, "not applied that far yet");
    core.report(UiReport::Applied(3));
    assert!(core.outbox.waiting.is_empty(), "the reply went");
    assert!(!core.lines_pending());
}
