//! The player starting itself again after the windows' GPU device was lost
//! (`platform::window_device_lost`; Windows wgpu windows only). The new
//! player is started as `rendering.restart` starts one
//! (`rendering::relaunch_command`); the loss is recorded in the crash guard
//! first, so a second loss within a day makes the next start render in
//! software (doc/architecture/README.md §4.3).
use super::rendering::{RESTARTING, Relaunch, relaunch_command, restart_refused};
use crate::platform;
use crate::playback::render_state::{self, GuardFile, Keys};
use serde_json::Value;
use std::sync::atomic::Ordering;

/// The newest `source.state` (any thread may read it: the restart after a
/// lost GPU device starts from the device's callback).
pub(crate) static LAST_SOURCE: std::sync::Mutex<Value> = std::sync::Mutex::new(Value::Null);

/// What a restart after a lost device needs, cloned into the device's
/// callback before the windows exist.
#[derive(Clone)]
pub(crate) struct LostDevice {
    pub relaunch: Relaunch,
    pub guard: GuardFile,
    pub keys: Keys,
    /// This player was started by a restart, at this time: it does not
    /// restart itself again within `rendering::RESTART_COOLDOWN`.
    pub restarted_at: Option<std::time::Instant>,
}

/// The windows' GPU device was lost: record it in the crash guard, then
/// start the player again, after this process's teardown, on what it was
/// playing (browsers restore the session after a GPU process loss the same
/// way). Once per process; `rendering.restart` and this share the flag.
///
/// The new player gets the current source if it is a local file, with its
/// position and paused state (`--vestra-resume`), the development plugins
/// and `--silent`; not the launch's files. It waits for this process to end
/// before it claims the single-instance lock (`--vestra-restart`). The new
/// player is told to render in software (`VESTRA_RENDER=software`, which
/// also beats a `VESTRA_RENDER=gpu` that would otherwise lose the device
/// again, forever) when the guard turned GPU rendering off, or when this
/// player was itself restarted less than `RESTART_COOLDOWN` ago (a GPU
/// relaunch is refused then; the player never just disappears).
pub(crate) fn restart_player(lost: &LostDevice, source: &Value) {
    if RESTARTING.swap(true, Ordering::AcqRel) {
        return;
    }
    // The shader guard first: a loss or stall before a new plugin shader
    // list was drawn counts as its crash, and a pending `applying` reaches
    // the disk; nothing settles the record after this.
    super::diagnostics::persist_shader_guard();
    let off = lost.guard.device_lost(&lost.keys, render_state::now());
    let why = software_after_loss(lost, off, std::time::Instant::now());
    let Ok(exe) = std::env::current_exe() else {
        restart_logged("render: the GPU device was lost; the player could not restart itself");
        return;
    };
    let mut command = command_after_loss(&exe, lost, source, why.is_some());
    restart_logged(&format!(
        "render: the GPU device was lost; restarting the player{}",
        why.map(|why| format!(" with software rendering ({why})"))
            .unwrap_or_default()
    ));
    restart_log(&format!("render: restarting with {command:?}"));
    // As `rendering.restart`: the new player may take the foreground, and
    // this one takes no more files from later launches.
    platform::allow_handoff_foreground();
    crate::stop_handoffs();
    match command.spawn() {
        Ok(child) => {
            restart_logged(&format!(
                "render: the restarted player is process {}",
                child.id()
            ));
            exit_if_stuck();
        }
        Err(error) => restart_logged(&format!("render: the restart failed: {error}")),
    }
}

/// Run the event loop (`run`, `slint::run_event_loop`). A panic out of it
/// once the windows' GPU device was lost (`lost`; a watchdog stall sets it
/// too, with the device healthy) ends the loop as the quit the loss asked
/// for would (`None`), whatever panicked; a panic before that goes on. Not
/// covered: an out-of-memory `create_buffer` on a healthy device, which
/// wgpu-core does not treat as a loss (`doc/architecture/README.md`).
///
/// Slint's femtovg-wgpu renderer keeps drawing the frames already asked for
/// until the loop sees that quit, and femtovg makes a new uniform buffer
/// with `wgpu::util::DeviceExt::create_buffer_init` (its viewport bind
/// group) whenever a draw switches between a layer texture and the window;
/// on a lost device that buffer cannot be mapped and wgpu panics (host
/// 0.6.3 and 0.6.4; fixed in 0.6.5). Unwinding out of `main` skipped the teardown (no
/// `exit: restarting`, plugins told `host-lost`, not `quit`), and the host,
/// still held by the UI thread's thread-locals, dropped only at process
/// exit, after Windows had ended its other threads: joining one of them
/// panicked again ("threads should not terminate unexpectedly"). winit
/// catches a panic in its callbacks and resumes it once out of the
/// dispatch, so it arrives here; nothing draws on the device after this.
pub(crate) fn run_through_loss<T>(run: impl FnOnce() -> T, lost: impl Fn() -> bool) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(ran) => Some(ran),
        // The panic hook already logged it (`diag::log_panics`).
        Err(_) if lost() => None,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// How long this player may take to end once its event loop ended after a
/// device loss. What must survive is saved by then: the crash guard and the
/// resume position here, the UI state and the standby recordings first
/// thing after the event loop (`main.rs`); clean teardowns after a loss took
/// 1 to 5 s (restart.log of the M3 and 0.5.19 hardware tests). Up to host
/// 0.6.3 the only limit was 15 s from the restart, which a teardown stuck in
/// the driver used in full (0.6.2, shader upscaling at 4K: a black,
/// unresponsive window for 15 s).
const EXIT_AFTER_LOOP: std::time::Duration = std::time::Duration::from_secs(5);

/// The limit from the restart itself, for a UI thread that never leaves the
/// event loop (stuck in a driver call): under the 20 s the restarted player
/// waits for this one (`main.rs`), which it starts counting a little later.
const EXIT_AFTER_RESTART: std::time::Duration = std::time::Duration::from_secs(15);

/// The step of the teardown this process is in, for the line that says it
/// was ended ([`teardown_step`]).
static TEARDOWN: std::sync::Mutex<&str> = std::sync::Mutex::new("the event loop (not ended)");

/// When this process is ended after a device loss, and when the restart
/// started (`None`: no restart); the exit thread waits on [`EXIT_CHANGED`].
type ExitAt = Option<(std::time::Instant, std::time::Instant)>;
static EXIT_AT: std::sync::Mutex<ExitAt> = std::sync::Mutex::new(None);
static EXIT_CHANGED: std::sync::Condvar = std::sync::Condvar::new();
/// The event loop ended ([`event_loop_ended`]). Read and written only with
/// [`EXIT_AT`] locked: otherwise a restart that read it just before the
/// loop ended, and set its deadline just after, would keep 15 s, not 5.
static LOOP_ENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The event loop ended (`main.rs`, first thing after it): after a device
/// loss the teardown now has [`EXIT_AFTER_LOOP`], not the rest of
/// [`EXIT_AFTER_RESTART`].
pub(crate) fn event_loop_ended() {
    let mut exit = EXIT_AT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    LOOP_ENDED.store(true, Ordering::Release);
    if let Some((at, since)) = *exit {
        *exit = Some((exit_deadline(at, true, std::time::Instant::now()), since));
        EXIT_CHANGED.notify_all();
    }
}

/// When to end the process: `at` (the deadline so far), brought forward to
/// [`EXIT_AFTER_LOOP`] from `now` once the event loop ended.
fn exit_deadline(
    at: std::time::Instant,
    loop_ended: bool,
    now: std::time::Instant,
) -> std::time::Instant {
    if loop_ended {
        at.min(now + EXIT_AFTER_LOOP)
    } else {
        at
    }
}

/// The teardown reached `step` (`main.rs`, after the event loop): if the
/// process has to be ended after a device loss, restart.log says where it
/// was stuck.
pub(crate) fn teardown_step(step: &'static str) {
    *TEARDOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = step;
}

/// The restart.log line of a player ended after a device loss.
fn stuck_line(after: std::time::Duration, step: &str) -> String {
    format!(
        "render: the player did not end {} s after the device loss (teardown at: {step}); ending it",
        after.as_secs()
    )
}

/// After a device loss or a GPU stall (`vestra-gpu-render`'s watchdog) the
/// teardown can hang in the driver, the render thread being stuck in it;
/// the new player would then wait for this one and the lock. End the
/// process if it is still there
/// [`EXIT_AFTER_LOOP`] after its event loop ended, or
/// [`EXIT_AFTER_RESTART`] after the restart, whichever comes first;
/// terminated without DLL detach (`platform::exit_now`), which a stuck
/// driver could hang.
fn exit_if_stuck() {
    {
        let mut exit = EXIT_AT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        let at = exit_deadline(
            now + EXIT_AFTER_RESTART,
            LOOP_ENDED.load(Ordering::Acquire),
            now,
        );
        *exit = Some((at, now));
    }
    let _ = std::thread::Builder::new()
        .name("vestra-exit-after-loss".into())
        .spawn(|| {
            let mut exit = EXIT_AT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let since = loop {
                let Some((at, since)) = *exit else { return };
                let left = at.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break since;
                }
                exit = EXIT_CHANGED
                    .wait_timeout(exit, left)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            };
            drop(exit);
            let step = *TEARDOWN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            restart_logged(&stuck_line(since.elapsed(), step));
            platform::exit_now(3);
        });
}

/// Why the player restarted after a device loss renders in software, if it
/// does: the crash guard turned GPU rendering off (`off`), or this player
/// was itself restarted within the cooldown, where a GPU relaunch is
/// refused (`rendering::restart_refused`).
fn software_after_loss(lost: &LostDevice, off: bool, now: std::time::Instant) -> Option<String> {
    if off {
        Some("the second loss within a day; crash guard".into())
    } else {
        restart_refused(false, lost.restarted_at, now).map(|why| format!("{why}; no GPU relaunch"))
    }
}

/// The new player's command line after a device loss (`off`: it renders in
/// software).
fn command_after_loss(
    exe: &std::path::Path,
    lost: &LostDevice,
    source: &Value,
    off: bool,
) -> std::process::Command {
    let mut command = relaunch_command(exe, &lost.relaunch, source);
    // A test switch must not remove the restarted player's device too.
    command.env_remove("VESTRA_TEST_REMOVE_DEVICE");
    command.env_remove("VESTRA_TEST_EXECUTE");
    command.env_remove("VESTRA_TEST_GPU_STALL");
    // The registry probe (host/supervisor/probe.rs): one run's snapshots.
    command.env_remove("VESTRA_TEST_REGISTRY");
    command.env_remove("VESTRA_TEST_PARK");
    command.env_remove("VESTRA_TEST_WAKE");
    command.env_remove("VESTRA_TEST_OFFSCREEN");
    if off {
        // `VESTRA_RENDER=gpu` overrides the guard; the child must not.
        command.env("VESTRA_RENDER", "software");
    }
    command
}

/// `restart_log` that also goes to the host log (`diag.rs`): only for lines
/// without the restart's command line (it names the file played).
pub(crate) fn restart_logged(line: &str) {
    logged!("{line}");
    append_restart_log(line);
}

/// A `render:` line about a restart: to stderr, and appended to
/// `logs/restart.log` in the application data directory, so a restarted
/// player that exits early (its stderr goes nowhere) still says why.
pub(crate) fn restart_log(line: &str) {
    diag!("{line}");
    append_restart_log(line);
}

fn append_restart_log(line: &str) {
    let Some(directory) = platform::app_data_dir().map(|dir| dir.join("logs")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&directory);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("restart.log"))
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "{} pid {}: {line}",
            render_state::now(),
            std::process::id()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::rendering::RESTART_COOLDOWN;

    fn lost(restarted_at: Option<std::time::Instant>) -> LostDevice {
        LostDevice {
            relaunch: Relaunch::default(),
            guard: GuardFile::new(None),
            keys: Keys::default(),
            restarted_at,
        }
    }
    fn env(command: &std::process::Command, name: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    /// 0.6.2 shader-upscaling loss: a teardown stuck in the driver left a
    /// black, unresponsive window for 15 s. The teardown gets 5 s from the
    /// end of the event loop (normal teardowns take 1 to 5 s), a UI thread
    /// that never leaves the loop 15 s from the restart, both under the
    /// restarted player's 20 s wait; the line says where it was stuck.
    #[test]
    fn a_stuck_teardown_is_ended_soon_and_says_where_it_was() {
        let second = std::time::Duration::from_secs(1);
        assert!(EXIT_AFTER_RESTART < 20 * second);
        let restart = std::time::Instant::now();
        let at = restart + EXIT_AFTER_RESTART;
        // The loop has not ended: the restart's limit.
        assert_eq!(exit_deadline(at, false, restart + 3 * second), at);
        // It ends 3 s later: 5 s from then.
        assert_eq!(
            exit_deadline(at, true, restart + 3 * second),
            restart + 8 * second
        );
        // It ends 12 s later: never later than the restart's limit.
        assert_eq!(exit_deadline(at, true, restart + 12 * second), at);
        // Restarted from the fallback after the loop already ended.
        assert_eq!(
            exit_deadline(restart + EXIT_AFTER_RESTART, true, restart),
            restart + EXIT_AFTER_LOOP
        );
        assert_eq!(
            *TEARDOWN.lock().unwrap(),
            "the event loop (not ended)",
            "inside the loop until main says otherwise"
        );
        teardown_step("hiding the main window");
        let step = *TEARDOWN.lock().unwrap();
        let line = stuck_line(8 * second, step);
        assert_eq!(
            line,
            "render: the player did not end 8 s after the device loss (teardown at: hiding the main window); ending it"
        );
    }

    /// 0.6.3/0.6.4: a draw on the lost window device panicked in wgpu
    /// (`create_buffer_init` from femtovg) and the panic left `main`, so
    /// the teardown never ran. After a loss it ends the loop instead; with
    /// the device fine it is still a panic.
    #[test]
    fn a_panic_after_the_loss_ends_the_event_loop_and_others_go_on() {
        let draw_on_lost_device = || -> Result<(), String> {
            panic!("Failed to get mapped range for buffer created with mapped_at_creation")
        };
        assert_eq!(run_through_loss(draw_on_lost_device, || true), None);
        assert_eq!(
            run_through_loss(|| Ok::<_, String>(()), || true),
            Some(Ok(()))
        );
        assert_eq!(
            run_through_loss(|| Err::<(), _>("loop failed".to_owned()), || false),
            Some(Err("loop failed".to_owned()))
        );
        let resumed = std::panic::catch_unwind(|| run_through_loss(draw_on_lost_device, || false));
        let payload = resumed.expect_err("no loss: the panic goes on");
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"Failed to get mapped range for buffer created with mapped_at_creation")
        );
    }

    /// M3 review M1: with `VESTRA_RENDER=gpu` a device that keeps getting
    /// lost must not restart the player on the GPU forever.
    #[test]
    fn once_the_guard_turned_gpu_off_the_new_player_renders_in_software() {
        let path = std::path::Path::new("player");
        let on = command_after_loss(path, &lost(None), &Value::Null, false);
        assert_eq!(env(&on, "VESTRA_RENDER"), None, "the environment as it is");
        assert_eq!(env(&on, "VESTRA_TEST_REMOVE_DEVICE"), Some(None), "removed");
        assert_eq!(env(&on, "VESTRA_TEST_EXECUTE"), Some(None), "removed");
        assert_eq!(env(&on, "VESTRA_TEST_GPU_STALL"), Some(None), "removed");
        for probe in [
            "VESTRA_TEST_REGISTRY",
            "VESTRA_TEST_PARK",
            "VESTRA_TEST_WAKE",
        ] {
            assert_eq!(env(&on, probe), Some(None), "removed");
        }
        let off = command_after_loss(path, &lost(None), &Value::Null, true);
        assert_eq!(env(&off, "VESTRA_RENDER"), Some(Some("software".into())));
        let args: Vec<_> = off.get_args().collect();
        assert_eq!(args[0], "--vestra-restart");
    }

    /// M3 re-review N1: a loss within the cooldown relaunches in software;
    /// the player never just exits.
    #[test]
    fn a_restarted_player_relaunches_in_software_within_the_cooldown() {
        let start = std::time::Instant::now();
        let restarted = lost(Some(start));
        let soon = start + std::time::Duration::from_secs(5);
        let why = software_after_loss(&restarted, false, soon);
        assert!(
            why.as_deref().is_some_and(|why| why.contains("30 seconds")),
            "{why:?}"
        );
        let later = start + RESTART_COOLDOWN;
        assert_eq!(
            software_after_loss(&restarted, false, later),
            None,
            "GPU again"
        );
        assert_eq!(software_after_loss(&lost(None), false, soon), None);
        assert!(
            software_after_loss(&lost(None), true, soon).is_some(),
            "guard off"
        );
        // Within the cooldown, guard crashed or not: software, never no restart.
        for off in [false, true] {
            let why = software_after_loss(&restarted, off, soon);
            let command = command_after_loss(
                std::path::Path::new("player"),
                &restarted,
                &Value::Null,
                why.is_some(),
            );
            assert_eq!(
                env(&command, "VESTRA_RENDER"),
                Some(Some("software".into())),
                "{off}"
            );
        }
    }
}
