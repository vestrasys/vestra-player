//! Slow turns of the UI thread, in the host log (always on).
//!
//! One queue and one thread carry every input, playback reply and plugin
//! message (`events.rs`), so anything that holds a turn of the UI thread's
//! loop delays a key press or a click by as long. A pause that came 0.5–1 s
//! late (user report, 2026-11-11) was gone after a restart and left nothing
//! to look at; this leaves a line: a turn of more than [`STALL`] says how
//! long it took and on what (the step of the tick, or the slowest event and
//! its plugin), and a gap of more than [`STALL`] from the end of one tick to
//! the start of the next says the loop was held outside them (Slint's
//! rendering, window events, the system). At most one line every [`EVERY`];
//! after [`BURST`] lines in a row (a lasting slow state: a request flood,
//! 4K software rendering) one line every [`SUMMARY`], back to [`EVERY`]
//! after [`QUIET`] without a slow turn. The turns left out are counted in
//! the next line, so the 512 KB log keeps its start and render lines.
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

/// A turn this long is logged.
pub(crate) const STALL: Duration = Duration::from_millis(100);
/// At most one line this often.
pub(crate) const EVERY: Duration = Duration::from_secs(1);
/// After this many lines in a row, [`SUMMARY`] apart.
pub(crate) const BURST: u32 = 10;
/// The interval of a lasting slow state's lines.
pub(crate) const SUMMARY: Duration = Duration::from_secs(60);
/// No slow turn for this long ends a lasting slow state.
pub(crate) const QUIET: Duration = Duration::from_secs(60);
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
    /// Lines written since the last quiet [`QUIET`].
    streak: u32,
    /// The last slow turn, written or not.
    seen: Option<Instant>,
    /// Slow turns left out since the last line, and the longest of them.
    left_out: u32,
    worst: Duration,
    /// The end of the last tick.
    last_end: Option<Instant>,
}

impl StallLog {
    /// A tick ended at `now` (every tick, slow or not).
    pub(crate) fn tick_ended(&mut self, now: Instant) {
        self.last_end = Some(now);
    }
    /// A tick began at `now`: the gap's line when the loop was held outside
    /// the ticks (from the last one's end) for [`STALL`] or more. A slow
    /// tick is its own line, not a gap.
    pub(crate) fn tick_began(&mut self, now: Instant) -> Option<(String, Duration)> {
        let gap = self
            .last_end
            .take()
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
            .seen
            .replace(now)
            .is_some_and(|seen| now.saturating_duration_since(seen) >= QUIET)
        {
            self.streak = 0;
        }
        let every = if self.streak >= BURST { SUMMARY } else { EVERY };
        if self
            .written
            .is_some_and(|written| now.saturating_duration_since(written) < every)
        {
            self.left_out += 1;
            self.worst = self.worst.max(took);
            return None;
        }
        self.written = Some(now);
        self.streak += 1;
        let left_out = std::mem::take(&mut self.left_out);
        let worst = std::mem::take(&mut self.worst);
        let mut line = if left_out == 0 {
            line
        } else {
            format!(
                "{line} [and {left_out} more since the last line, the longest {}]",
                ms(worst)
            )
        };
        if self.streak == BURST {
            line += " [slow turns go on: one line a minute until a quiet minute]";
        }
        Some(line)
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

/// The turn ended: logged when it was slow. Its end starts the next gap.
pub(crate) fn tick_ended(turn: &Turn, name: impl Fn(u64) -> String) {
    let now = Instant::now();
    let line = LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.tick_ended(now);
        let line = turn.finish(now, name)?;
        log.note(line, now.saturating_duration_since(turn.began), now)
    });
    if let Some(line) = line {
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
/// the end of its synchronous part on the playback worker (`received` to
/// `done`: `CoreActions::run`, the mpv calls it makes); what it leaves to
/// later turns (a seek landing, a frame step, a keyframe seek's next
/// press) is not in "ran there". `None` otherwise.
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

    /// Ten slow turns a second from `from` for `seconds`: the lines.
    fn flood(log: &mut StallLog, from: Instant, seconds: u32) -> Vec<String> {
        (0..seconds * 10)
            .filter_map(|n| log.note("x".into(), 100 * MS, from + n * 100 * MS))
            .collect()
    }

    #[test]
    fn a_lasting_slow_state_backs_off_to_one_line_a_minute() {
        let t = Instant::now();
        let mut log = StallLog::default();
        // The first ten a second apart, the tenth says so, then a minute.
        let lines = flood(&mut log, t, 60);
        assert_eq!(lines.len(), BURST as usize);
        assert!(lines[9].ends_with("until a quiet minute]"), "{}", lines[9]);
        assert!(lines[1].contains("[and 9 more"), "{}", lines[1]);
        let next = flood(&mut log, t + 60 * EVERY, 60);
        assert_eq!(next.len(), 1);
        assert!(next[0].contains("[and 599 more"), "{}", next[0]);
    }

    #[test]
    fn an_hour_of_slow_turns_writes_a_few_dozen_lines() {
        let t = Instant::now();
        let mut log = StallLog::default();
        let lines = flood(&mut log, t, 3600);
        // 10 at first, then one a minute: well under 1 KB a minute.
        assert!((60..=72).contains(&lines.len()), "{}", lines.len());
    }

    #[test]
    fn a_quiet_minute_ends_the_back_off() {
        let t = Instant::now();
        let mut log = StallLog::default();
        assert_eq!(flood(&mut log, t, 30).len(), BURST as usize);
        // A minute without a slow turn: one a second again.
        let again = t + 30 * EVERY + QUIET;
        assert_eq!(flood(&mut log, again, 5).len(), 5);
        // Less than a quiet minute keeps the back-off.
        let mut log = StallLog::default();
        assert_eq!(flood(&mut log, t, 30).len(), BURST as usize);
        let soon = t + 30 * EVERY + QUIET / 2;
        assert!(flood(&mut log, soon, 5).len() <= 1);
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
        log.tick_ended(t + 2 * MS);
        assert_eq!(log.tick_began(t + 16 * MS), None);
        log.tick_ended(t + 20 * MS);
        assert_eq!(log.tick_began(t + 20 * MS + 115 * MS), None, "under");
        log.tick_ended(t + 140 * MS);
        let (line, gap) = log.tick_began(t + 740 * MS).unwrap();
        assert_eq!(gap, 600 * MS);
        assert!(line.starts_with("ui stall: no tick for 600 ms"), "{line}");
        log.tick_ended(t + 750 * MS);
        assert_eq!(log.tick_began(t + 750 * MS + ASLEEP), None, "asleep");
    }

    #[test]
    fn a_slow_tick_is_not_also_a_gap() {
        let t = Instant::now();
        let mut log = StallLog::default();
        assert_eq!(log.tick_began(t), None);
        // A 150 ms tick, then the next one on the timer.
        log.tick_ended(t + 150 * MS);
        assert_eq!(log.tick_began(t + 160 * MS), None);
        log.tick_ended(t + 162 * MS);
        assert_eq!(log.tick_began(t + 176 * MS), None);
    }
}
