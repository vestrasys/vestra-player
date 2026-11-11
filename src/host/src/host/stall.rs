//! Slow turns of the UI thread, in the host log (always on).
//!
//! One queue and one thread carry every input, playback reply and plugin
//! message (`events.rs`), so anything that holds a turn of the UI thread's
//! loop delays a key press or a click by as long. A pause that came 0.5–1 s
//! late (user report, 2026-11-11) was gone after a restart and left nothing
//! to look at; this leaves a line: a turn of more than [`STALL`] says how
//! long it took and on what (the step of the tick, or the slowest event and
//! its plugin), and a gap of more than [`STALL`] between two ticks says the
//! loop was held outside them (Slint's rendering, window events, the
//! system). At most one line every [`EVERY`]; the turns left out meanwhile
//! are counted in the next line, so the log cannot fill with them.
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// A turn this long is logged.
pub(crate) const STALL: Duration = Duration::from_millis(100);
/// At most one line this often.
pub(crate) const EVERY: Duration = Duration::from_secs(1);
/// The tick's period (`main.rs`): a gap this much over [`STALL`] counts.
const PERIOD: Duration = Duration::from_millis(16);
/// A longer gap is the system's sleep or a debugger, not the loop.
const ASLEEP: Duration = Duration::from_secs(30);

/// What one event was, for the line: its kind and the plugin it came from.
pub(crate) type Label = (&'static str, Option<u64>);

/// The timing of one turn: its steps and its events.
#[derive(Debug)]
pub(crate) struct Turn {
    began: Instant,
    mark: Instant,
    slowest_step: Option<(&'static str, Duration)>,
    events: usize,
    slowest_event: Option<(Label, Duration)>,
}

impl Turn {
    pub(crate) fn new(began: Instant) -> Self {
        Self {
            began,
            mark: began,
            slowest_step: None,
            events: 0,
            slowest_event: None,
        }
    }
    /// The time since the last step (or the start) went to `what`.
    pub(crate) fn step(&mut self, what: &'static str) {
        let now = Instant::now();
        self.step_took(what, now - self.mark);
        self.mark = now;
    }
    fn step_took(&mut self, what: &'static str, took: Duration) {
        if self.slowest_step.is_none_or(|(_, worst)| took > worst) {
            self.slowest_step = Some((what, took));
        }
    }
    /// One event handled in `took` (part of the step it falls in).
    pub(crate) fn event(&mut self, label: Label, took: Duration) {
        self.events += 1;
        if self.slowest_event.is_none_or(|(_, worst)| took > worst) {
            self.slowest_event = Some((label, took));
        }
    }
    /// The turn's line when it took [`STALL`] or more; `name` gives a
    /// plugin's id.
    pub(crate) fn finish(&self, now: Instant, name: impl Fn(u64) -> String) -> Option<String> {
        let took = now.saturating_duration_since(self.began);
        if took < STALL {
            return None;
        }
        let mut text = format!("ui stall: a tick took {}", ms(took));
        if let Some((what, step)) = self.slowest_step {
            text += &format!(" ({what} {})", ms(step));
        }
        if let Some(((kind, owner), event)) = self.slowest_event {
            text += &format!("; {} events, the slowest {kind}", self.events);
            if let Some(owner) = owner {
                text += &format!(" from {}", name(owner));
            }
            text += &format!(" {}", ms(event));
        }
        Some(text)
    }
}

/// The rate limit, and the gap between ticks.
#[derive(Debug, Default)]
pub(crate) struct StallLog {
    written: Option<Instant>,
    /// Slow turns left out since the last line, and the longest of them.
    left_out: u32,
    worst: Duration,
    last_tick: Option<Instant>,
}

impl StallLog {
    /// A tick began at `now`: the gap's line when the loop was held outside
    /// the ticks for [`STALL`] or more.
    pub(crate) fn tick_began(&mut self, now: Instant) -> Option<(String, Duration)> {
        let gap = self
            .last_tick
            .replace(now)
            .map(|last| now.saturating_duration_since(last))?;
        (gap >= STALL + PERIOD && gap < ASLEEP).then(|| {
            let line = format!(
                "ui stall: no tick for {} (held outside the tick: drawing, window events or the system)",
                ms(gap)
            );
            (line, gap)
        })
    }
    /// A slow turn's `line` (it took `took`) at `now`: what to write, if
    /// anything, with the turns left out before it.
    pub(crate) fn note(&mut self, line: String, took: Duration, now: Instant) -> Option<String> {
        if self
            .written
            .is_some_and(|written| now.saturating_duration_since(written) < EVERY)
        {
            self.left_out += 1;
            self.worst = self.worst.max(took);
            return None;
        }
        self.written = Some(now);
        let left_out = std::mem::take(&mut self.left_out);
        let worst = std::mem::take(&mut self.worst);
        Some(if left_out == 0 {
            line
        } else {
            format!(
                "{line} [and {left_out} more since the last line, the longest {}]",
                ms(worst)
            )
        })
    }
}

thread_local! {
    static LOG: RefCell<StallLog> = RefCell::default();
}

/// The UI thread's tick began (`main.rs`): a gap before it is logged.
pub(crate) fn tick_began(now: Instant) {
    let line = LOG.with(|log| {
        let mut log = log.borrow_mut();
        let (line, gap) = log.tick_began(now)?;
        log.note(line, gap, now)
    });
    if let Some(line) = line {
        logged!("{line}");
    }
}

/// The turn ended: logged when it was slow.
pub(crate) fn tick_ended(turn: &Turn, name: impl Fn(u64) -> String) {
    let now = Instant::now();
    let Some(line) = turn.finish(now, name) else {
        return;
    };
    let took = now.saturating_duration_since(turn.began);
    if let Some(line) = LOG.with(|log| log.borrow_mut().note(line, took, now)) {
        logged!("{line}");
    }
}

/// Microseconds on one clock for the whole process: a core action's
/// `queuedUs` (`host/actions.rs`), read again on the playback worker.
pub(crate) fn clock_us() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// The line for a core action (Play/Pause and the others) that took
/// [`STALL`] or more from the host loop's send (`queued`, [`clock_us`]) to
/// the end of its work on the playback worker; `None` otherwise.
pub(crate) fn action_line(action: &str, queued: u64, received: u64, done: u64) -> Option<String> {
    let wait = Duration::from_micros(received.saturating_sub(queued));
    let work = Duration::from_micros(done.saturating_sub(received));
    (wait + work >= STALL).then(|| {
        format!(
            "ui stall: the core action {action} waited {} for the playback worker and ran there {}",
            ms(wait),
            ms(work)
        )
    })
}

/// The playback worker finished a core action: logged when it was slow
/// (rate-limited on that thread as on the UI thread).
pub(crate) fn action_done(action: &str, queued: Option<u64>, received: u64) {
    let done = clock_us();
    let Some(line) = queued.and_then(|queued| action_line(action, queued, received, done)) else {
        return;
    };
    let took = Duration::from_micros(done.saturating_sub(queued.unwrap_or(done)));
    if let Some(line) = LOG.with(|log| log.borrow_mut().note(line, took, Instant::now())) {
        logged!("{line}");
    }
}

fn ms(took: Duration) -> String {
    format!("{:.0} ms", took.as_secs_f64() * 1000.)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_quick_turn_writes_nothing() {
        let t = Instant::now();
        let mut turn = Turn::new(t);
        turn.step_took("window_tick", 90 * MS);
        assert_eq!(turn.finish(t + 99 * MS, |o| o.to_string()), None);
    }

    #[test]
    fn a_slow_turn_names_its_slowest_step_and_event() {
        let t = Instant::now();
        let mut turn = Turn::new(t);
        turn.step_took("events", 130 * MS);
        turn.step_took("window_tick", 20 * MS);
        turn.event(("Binding", None), 2 * MS);
        turn.event(("Request", Some(7)), 120 * MS);
        turn.event(("Broadcast", None), 1 * MS);
        let name = |owner: u64| format!("plugin-{owner}");
        assert_eq!(
            turn.finish(t + 150 * MS, name).unwrap(),
            "ui stall: a tick took 150 ms (events 130 ms); 3 events, the slowest Request from plugin-7 120 ms"
        );
    }

    #[test]
    fn one_line_a_second_and_the_next_counts_the_rest() {
        let t = Instant::now();
        let mut log = StallLog::default();
        assert_eq!(log.note("a".into(), 200 * MS, t).as_deref(), Some("a"));
        // Within the second: left out, counted.
        for (n, took) in [150, 900, 120].into_iter().enumerate() {
            let at = t + (100 + 200 * n as u32) * MS;
            assert_eq!(log.note("b".into(), took * MS, at), None);
        }
        assert_eq!(
            log.note("c".into(), 110 * MS, t + 1000 * MS).as_deref(),
            Some("c [and 3 more since the last line, the longest 900 ms]")
        );
        // The count starts again.
        assert_eq!(
            log.note("d".into(), 110 * MS, t + 2500 * MS).as_deref(),
            Some("d")
        );
    }

    #[test]
    fn a_flood_of_slow_turns_writes_one_line_a_second() {
        let t = Instant::now();
        let mut log = StallLog::default();
        // Ten slow turns a second for a minute: 60 lines.
        let lines = (0..600u32)
            .filter_map(|n| log.note("x".into(), 100 * MS, t + n * 100 * MS))
            .count();
        assert_eq!(lines, 60);
    }

    #[test]
    fn a_core_action_is_a_line_from_100_ms_on() {
        assert_eq!(action_line("play-pause", 1_000, 1_500, 2_000), None);
        assert_eq!(action_line("play-pause", 0, 60_000, 99_999), None);
        assert_eq!(
            action_line("play-pause", 0, 480_000, 482_000).as_deref(),
            Some(
                "ui stall: the core action play-pause waited 480 ms for the playback worker and ran there 2 ms"
            )
        );
        // A clock read out of order never underflows.
        assert_eq!(action_line("mute", 500, 100, 50), None);
        assert!(clock_us() <= clock_us());
    }

    #[test]
    fn a_gap_between_ticks_is_a_line_but_not_a_sleep() {
        let t = Instant::now();
        let mut log = StallLog::default();
        assert_eq!(log.tick_began(t), None, "the first tick");
        assert_eq!(log.tick_began(t + 16 * MS), None);
        assert_eq!(log.tick_began(t + 16 * MS + 115 * MS), None, "under");
        let (line, gap) = log.tick_began(t + 131 * MS + 600 * MS).unwrap();
        assert_eq!(gap, 600 * MS);
        assert!(line.starts_with("ui stall: no tick for 600 ms"), "{line}");
        assert_eq!(log.tick_began(t + 731 * MS + ASLEEP), None, "asleep");
    }
}
