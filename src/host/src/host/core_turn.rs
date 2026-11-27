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
        window_state_sent: Value::Null,
        window_change: Default::default(),
        layout_change: Default::default(),
        held_events: Default::default(),
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
        retry_changes: Vec::new(),
        clipboard: Default::default(),
        quit_asked: None,
        quitting: false,
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
    let route = api::lookup("window.state").unwrap();
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

/// A mounted bottom panel of plugin `plugin` at `key`.
fn bottom_mount(plugin: u64, key: u64) -> Mount {
    Mount {
        spec: composition::CompileSpec {
            owner: key,
            file: PathBuf::from("bar.slint"),
            component: "Bar".into(),
            global: None,
            slot: composition::Slot::Bottom,
        },
        plugin,
        properties: BTreeMap::new(),
        visible: true,
        title: "Bar".into(),
        manifest: "test.bar".into(),
        builtin: false,
        recorded: false,
        call: Value::Null,
    }
}

/// S3a(3) re-review F1: a host removal of a panel (an id 0 unmount) in the
/// batch in flight does not block its plugin's mount (it queues behind),
/// any other change of the panel in flight does.
#[test]
fn a_host_removal_in_flight_does_not_block_the_plugins_mount() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let key = panel_key(7, composition::Slot::Bottom);
    let removal = super::mounts::HandedMount::of(&unmount(7, 0, key));
    assert!(removal.removes(key));
    core.batch_in_flight = vec![removal];
    assert!(core.mount_pending(key), "pending for every other check");
    assert!(!core.mount_blocked(key), "not for the plugin's mount");
    core.batch_in_flight = vec![super::mounts::HandedMount::of(&unmount(7, 3, key))];
    assert!(
        core.mount_blocked(key),
        "the plugin's own unmount in flight blocks"
    );
}

/// S3a(3) re-review F2: a mount applied after its plugin stopped (it was
/// in flight when the stop ran) gets the stop's end state: removed, the
/// interface told, and an id 0 unmount queued; held in standby, a
/// placeholder.
#[test]
fn a_mount_applied_after_its_plugin_stopped_goes_as_the_stop_would_have_it() {
    use messages::{PanelsOp, UiCommand};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let key = panel_key(7, composition::Slot::Bottom);
    core.mounts.insert(key, bottom_mount(7, key));
    core.outbox.clear();
    core.stopped_owners_mount(7, key);
    assert!(!core.mounts.contains_key(&key), "the slot is free");
    assert!(
        core.mount_queue
            .front()
            .is_some_and(|c| c.key == key && c.reply.id == 0 && c.contribution.is_none()),
        "an id 0 unmount at the front"
    );
    assert!(core.outbox.iter().any(|out| matches!(
        out,
        super::outbox::Out::Ui(UiCommand::Panels(PanelsOp::Gone(panels))) if panels[0].0 == key
    )));
    // Another plugin's panel, or one already a placeholder: untouched.
    core.mount_queue.clear();
    core.mounts.insert(key, bottom_mount(8, key));
    core.stopped_owners_mount(7, key);
    assert!(core.mounts.contains_key(&key) && core.mount_queue.is_empty());
}

/// S3a(3) review B3 and re-review F3: what each in-flight mark makes the
/// core's checks read. A confirmation counts as a waiting confirmation and
/// sheet; a `commands.run` sheet only as a waiting sheet; a tool window
/// opening, a picture-in-picture request and the options window opening
/// for their checks; `Applied` ends each.
#[test]
fn the_in_flight_marks_reach_their_checks() {
    use super::outbox::Mark;
    use messages::{
        ReviewKind, ReviewRequest, RunSheet, SettingsOp, UiCommand, UiReport, ViewKind, ViewOp,
        WindowOp,
    };
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    core.ui_command(UiCommand::Review(ReviewRequest {
        reply: Reply { owner: 7, id: 1 },
        kind: ReviewKind::CommandRun(RunSheet {
            ticket: "t".into(),
            query: "q".into(),
            steps: Vec::new(),
            remember: String::new(),
        }),
    }));
    assert!(core.sheet_waiting() && !core.confirm_waiting(), "F3");
    core.ui_command(UiCommand::View(ViewOp {
        reply: Reply { owner: 8, id: 2 },
        id: "w".into(),
        kind: ViewKind::Open {
            file: PathBuf::from("w.slint"),
            properties: BTreeMap::new(),
            callbacks: Vec::new(),
        },
    }));
    core.ui_command(UiCommand::Window(WindowOp::Pip {
        reply: Reply { owner: 9, id: 3 },
        on: true,
    }));
    core.ui_command(UiCommand::Settings(SettingsOp::Show {
        reply: Reply { owner: 9, id: 4 },
        params: json!({}),
    }));
    core.flush();
    assert!(core.in_flight(Mark::View(8)) && !core.in_flight(Mark::View(9)));
    assert!(core.in_flight(Mark::Pip(9)) && !core.in_flight(Mark::Pip(8)));
    assert!(core.in_flight(Mark::Options));
    let seq = core.outbox.pushed;
    core.report(UiReport::Applied(seq));
    for mark in [Mark::Review, Mark::View(8), Mark::Pip(9), Mark::Options] {
        assert!(!core.in_flight(mark), "{mark:?} ended");
    }
    assert!(!core.sheet_waiting());
}

/// S3a(3) re-review: the single-change retry of a failed batch ends when
/// none of the changes it gave back waits any more (here: dropped with
/// their plugin), so the next batch joins as usual.
#[test]
fn the_retry_ends_when_its_changes_are_gone() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let (a, b) = (
        panel_key(7, composition::Slot::Bottom),
        panel_key(7, composition::Slot::Sidebar),
    );
    core.batch_retry(vec![unmount(7, 1, a), unmount(7, 2, b)]);
    assert!(core.retry_due && core.mount_singly == 2);
    core.mount_queue.retain(|c| c.reply.owner != 7);
    core.turn_end();
    assert!(!core.retry_due && core.mount_singly == 0);
}

/// S3a(3) verification: `vestra.hotkeys` writes two lists of one page
/// (`rows`, `source-model`). The interface writes its copy of the page and
/// reports each write; a copy of the pages the core gives before it handled
/// a report (another page changed meanwhile) lacked that write, so the
/// interface's `rows` went back to the older value and the next row patch
/// was refused "insert at N is past the end of the list". The interface now
/// writes the writes not handled yet again into each copy it is given.
#[test]
fn a_copy_of_the_pages_given_before_a_write_is_handled_keeps_the_write() {
    use crate::ui::settings_window::{Page, StoredInFlight};
    use crate::ui::ui_patch;
    use messages::StoredWrite;
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let page = |id: &str, serial: u64| Page {
        owner: 4,
        manifest: "vestra.hotkeys".into(),
        id: id.into(),
        category: 0,
        title: id.into(),
        order: 0,
        serial,
        file: PathBuf::from("page.slint"),
        component: "Page".into(),
        global: "Bridge".into(),
        properties: [
            ("rows".into(), json!([])),
            ("source-model".into(), json!([])),
        ]
        .into_iter()
        .collect(),
    };
    let len = |pages: &[Page], property: &str| {
        pages
            .iter()
            .find(|p| p.id == "keyboard")
            .unwrap()
            .properties[property]
            .as_array()
            .unwrap()
            .len()
    };
    core.settings.add_page(page("keyboard", 1));
    let (mut mine, mut in_flight) = (Vec::new(), StoredInFlight::default());
    let copies = core.copies();
    in_flight.take(&mut mine, copies.stored, copies.pages);

    // The interface stores the whole `rows` (349) and reports it.
    let rows = json!((0..349).collect::<Vec<_>>());
    let whole = StoredWrite::Whole(rows.clone());
    mine[0].properties.insert("rows".into(), rows);
    in_flight.reported(4, "keyboard", "rows", &whole);
    // Before the core handles that report, another page changes: the
    // copy the core gives lacks the write.
    core.settings.add_page(page("mouse", 2));
    let copies = core.copies();
    assert_eq!(len(copies.pages.as_ref().unwrap(), "rows"), 0);
    assert_eq!(copies.stored, Some(0));
    in_flight.take(&mut mine, copies.stored, copies.pages);
    assert_eq!(len(&mine, "rows"), 349, "the interface keeps its write");

    // A patch of the other list, applied and reported.
    let ops = json!([{"op": "insert", "at": 0, "rows": [1, 2]}]);
    let patch = ui_patch::parse(&ops, len(&mine, "source-model")).unwrap();
    let patch = StoredWrite::Rows {
        shapes: patch.shapes.clone(),
        rows: patch.rows.iter().map(|row| (*row).clone()).collect(),
    };
    if let (Some(serde_json::Value::Array(stored)), StoredWrite::Rows { shapes, rows }) =
        (mine[0].properties.get_mut("source-model"), &patch)
    {
        ui_patch::apply_vec(stored, shapes, rows.clone());
    }
    in_flight.reported(4, "keyboard", "source-model", &patch);

    // The core handles the first report: its copies say so, and the
    // interface's echo needs no pages.
    core.settings_stored(4, "keyboard", "rows".into(), whole);
    let copies = core.copies();
    assert!(copies.pages.is_none());
    assert_eq!(copies.stored, Some(1));
    in_flight.take(&mut mine, copies.stored, copies.pages);
    assert_eq!(in_flight.pending(), 1);
    // Another page change before the second report: the copy has `rows`
    // and lacks the `source-model` patch, which the interface writes again.
    core.settings.add_page(page("wheel", 3));
    let copies = core.copies();
    assert_eq!(len(copies.pages.as_ref().unwrap(), "source-model"), 0);
    in_flight.take(&mut mine, copies.stored, copies.pages);
    assert_eq!(len(&mine, "source-model"), 2);
    // The next `rows` patch fits (it was refused before the fix).
    let next = json!([{"op": "insert", "at": 209, "rows": [0, 0]}]);
    assert!(ui_patch::parse(&next, len(&mine, "rows")).is_ok());

    // Once the core has handled both, both copies are the same.
    core.settings_stored(4, "keyboard", "source-model".into(), patch);
    let copies = core.copies();
    assert_eq!(copies.stored, Some(2));
    in_flight.take(&mut mine, copies.stored, copies.pages);
    assert_eq!(in_flight.pending(), 0);
    let core_page = core.settings.page(4, "keyboard").unwrap();
    assert_eq!(core_page.properties, mine[0].properties);
}

/// S3a(3) verification: the window facts of several reports one turn
/// handles (in-between states: backdrop null, 1280x720, then 960x720)
/// broadcast `window.state` once, with the final state, as the one facts
/// value per tick before S3a(3) did. It goes when the interface says its
/// tick's facts are in (`FactsSettled`, S3a(4) review F2), not at the end
/// of a turn: on two threads a turn can fall inside the interface's tick.
#[test]
fn the_reports_of_one_turn_broadcast_one_window_state() {
    use messages::{UiReport, WindowFacts};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let facts = |seq: u64, facts: Value| {
        UiReport::WindowFacts(Box::new(WindowFacts {
            seq,
            facts,
            shell: Some(1),
            menu_origin: true,
            pip_supported: true,
            pip_active: false,
            sidebar_panels: vec![],
            adjust_panels: vec![],
            bottom_panel: None,
        }))
    };
    core.report(facts(1, json!({"backdrop": null})));
    core.report(facts(2, json!({"size": [1280, 720]})));
    core.report(facts(3, json!({"size": [960, 720]})));
    assert_eq!(core.window_state_sent, Value::Null, "nothing per report");
    core.report(UiReport::FactsSettled);
    assert_eq!(core.window_state_sent, json!({"size": [960, 720]}));
    // A turn that ends inside the interface's tick broadcasts nothing.
    core.report(facts(4, json!({"size": [1280, 720]})));
    core.turn_end();
    assert_eq!(core.window_state_sent, json!({"size": [960, 720]}));
    core.report(UiReport::FactsSettled);
    assert_eq!(core.window_state_sent, json!({"size": [1280, 720]}));
    // Re-review B1: a report that writes a line (a reply) first sends the
    // facts the reports before it left (R13's `layout.set` exception: its
    // facts report, then its reply); an `Applied` with no reply waiting
    // does not.
    core.report(facts(5, json!({"size": [960, 658]})));
    core.report(UiReport::Applied(core.outbox.applied));
    assert_eq!(core.window_state_sent, json!({"size": [1280, 720]}));
    core.report(UiReport::Answered {
        reply: Reply { owner: 7, id: 1 },
        result: Ok(Value::Null),
        handoff: None,
        confirm_failed: false,
    });
    assert_eq!(core.window_state_sent, json!({"size": [960, 658]}));
}

/// S3a(3) re-review S1: a mount applied after its plugin was held (parked
/// while its batch was in flight) gets what the park gives a panel in
/// place: a tabbed panel stays as a placeholder (recorded), a bottom or
/// backdrop panel goes with an id 0 unmount.
#[test]
fn a_mount_applied_after_its_plugin_was_held_goes_as_the_park_would_have_it() {
    use super::supervisor::{Held, Origin};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let origin = Origin {
        id: "test.bar".into(),
        directory: root.path().to_path_buf(),
        catalog: None,
        data_directory: None,
    };
    core.hold(7, Held::new("test.bar".into(), origin, "Bar".into()));
    // A bottom panel: removed, its slot freed.
    let bottom = panel_key(7, composition::Slot::Bottom);
    core.mounts.insert(bottom, bottom_mount(7, bottom));
    core.stopped_owners_mount(7, bottom);
    assert!(
        !core.mounts.contains_key(&bottom),
        "not kept as a placeholder"
    );
    assert!(
        core.mount_queue
            .front()
            .is_some_and(|c| c.key == bottom && c.reply.id == 0 && c.contribution.is_none()),
        "an id 0 unmount at the front"
    );
    // A sidebar panel (tabbed): a placeholder.
    core.mount_queue.clear();
    let sidebar = panel_key(7, composition::Slot::Sidebar);
    let mut mount = bottom_mount(7, sidebar);
    mount.spec.slot = composition::Slot::Sidebar;
    core.mounts.insert(sidebar, mount);
    core.stopped_owners_mount(7, sidebar);
    assert!(
        core.mounts.get(&sidebar).is_some_and(|m| m.recorded),
        "recorded"
    );
    assert!(core.mount_queue.is_empty());
}

/// S3a(3) round-4 nit N1: an `Applied` that releases a waiting reply (one
/// `outbox.waiting` entry, a `commands.run` reply behind a column toggle)
/// sends the pending `window.state` first, then the reply; one that
/// releases nothing leaves it pending.
#[test]
fn an_applied_that_releases_a_reply_sends_the_pending_window_state_first() {
    use super::outbox::Then;
    use messages::{UiReport, WindowFacts};
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    core.outbox.pushed = 2;
    core.outbox.waiting.push_back((
        2,
        Then::Respond {
            reply: Reply { owner: 7, id: 1 },
            result: Ok(Value::Null),
        },
    ));
    core.report(UiReport::WindowFacts(Box::new(WindowFacts {
        seq: 1,
        facts: json!({"slots": {"sidebar": {"effective": false}}}),
        shell: Some(1),
        menu_origin: true,
        pip_supported: true,
        pip_active: false,
        sidebar_panels: vec![],
        adjust_panels: vec![],
        bottom_panel: None,
    })));
    // Command 1 applied: the reply waits for command 2, nothing goes.
    core.report(UiReport::Applied(1));
    assert_eq!(core.window_state_sent, Value::Null);
    assert_eq!(core.outbox.waiting.len(), 1);
    // Command 2 applied: the broadcast goes, then the reply is released.
    core.report(UiReport::Applied(2));
    assert_eq!(
        core.window_state_sent,
        json!({"slots": {"sidebar": {"effective": false}}})
    );
    assert!(core.outbox.waiting.is_empty(), "the reply went");
}

/// S3a(4) re-review: `Busy::Request`'s source. A request of a plugin read
/// into the core's held events keeps it busy until it is taken to be
/// handled; another plugin's request does not. (`host_busy` itself needs a
/// running plugin process in `Core::plugins`, which a unit test cannot
/// build; it inserts `Busy::Request` exactly when this holds.)
#[test]
fn a_held_request_keeps_its_plugin_busy_until_it_is_taken() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    assert!(!core.request_waiting(7));
    core.held_events.push_back(Event::Request(
        7,
        json!({"id": 1, "method": "bindings.list"}),
    ));
    core.held_events.push_back(Event::MainClosed);
    assert!(core.request_waiting(7), "its request waits");
    assert!(!core.request_waiting(8), "not another plugin's");
    let taken = core.held_events.pop_front();
    assert!(matches!(taken, Some(Event::Request(7, _))));
    assert!(
        !core.request_waiting(7),
        "taken to be handled: no longer busy"
    );
}

/// Guest windows: a core on a
/// read-only library refuses the three changes to the plugins with
/// `unavailable` and still answers the reads.
#[test]
fn a_guest_core_refuses_plugin_changes() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    core.packages = package_manager::Manager::at_read_only(
        root.path().join("main-packages"),
        root.path().join("guest-packages"),
    )
    .unwrap();
    assert!(core.packages.read_only());
    for (method, params) in [
        (
            "packages.install",
            json!({"requestId":"r","token":"t","expectedRevision":0}),
        ),
        ("packages.uninstall", json!({"id":"x"})),
        ("packages.setEnabled", json!({"id":"x","enabled":false})),
    ] {
        let refused = core.package_dispatch(1, method, &params).unwrap_err();
        assert_eq!(refused.code, "unavailable", "{method}");
        assert_eq!(refused.message, package_manager::GUEST_PACKAGES);
    }
    assert!(
        core.package_dispatch(1, "packages.list", &json!({}))
            .is_ok()
    );
    assert!(!root.path().join("main-packages").exists(), "nothing made");
}

/// Guest windows: the forced removal of versions a
/// record set aside no longer keeps (`hold_freed`) leaves a version a
/// running guest leases where it is; the others leave the library.
#[test]
fn hold_freed_keeps_a_version_a_running_guest_leases() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    let library = root.path().join("packages").join("packages");
    let (leased, other) = ("a".repeat(64), "b".repeat(64));
    for digest in [&leased, &other] {
        std::fs::create_dir_all(library.join(digest)).unwrap();
        std::fs::write(library.join(digest).join("main.js"), "x").unwrap();
    }
    let guests = tempfile::tempdir().unwrap();
    let directory = guests.path().join("guest");
    std::fs::create_dir(&directory).unwrap();
    let lock = std::fs::File::create(directory.join("guest.lock")).unwrap();
    lock.try_lock().unwrap();
    std::fs::write(directory.join("packages.json"), format!("[\"{leased}\"]")).unwrap();
    core.packages.set_guests(Some(guests.path().into()));
    core.hold_freed(&[leased.clone(), other.clone()].into());
    assert!(library.join(&leased).is_dir(), "leased: left in place");
    assert!(!library.join(&other).exists(), "taken out of the library");
    drop(lock);
}

/// Once the quit has begun (the `quit` command, before the plugins'
/// shutdown, or the shutdown itself), no shared runner starts or restarts:
/// a runner that ends then (Restart Manager closing it for an installer)
/// stays down for the quit to finish.
#[test]
fn no_runner_starts_once_the_quit_has_begun() {
    let root = tempfile::tempdir().unwrap();
    let (mut core, _events, _commands) = core(root.path());
    assert!(core.runners_may_start());
    core.host_command("quit");
    assert!(core.quitting);
    assert!(!core.runners_may_start());
    // Nothing to restart, nothing restarted.
    core.restart_shared("the shared runner exited");
    assert!(core.shared.is_none());

    let (mut core, _events, _commands) = self::core(root.path());
    core.on_demand.quitting = true;
    assert!(!core.runners_may_start());
}
