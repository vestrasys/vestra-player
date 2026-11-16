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
    core.report(messages::UiReport::UiFacts(Box::new(messages::UiFacts {
        options_open: true,
        ..Default::default()
    })));
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
