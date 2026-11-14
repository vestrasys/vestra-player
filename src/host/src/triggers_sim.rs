//! A pure simulation of the position model (events-and-triggers.md §4.4,
//! plan step A2): the playback thread's model (`PositionModel`) and the UI
//! thread's evaluator (`Triggers`) exchanging reports and boundaries as the
//! Host does, over a simulated mpv whose `time-pos` moves by frames, with a
//! virtual clock. Nothing waits: time is passed in. Covered: entering,
//! jumping over, jumping back, pause, a rate change, a very short range,
//! adjacent ranges, a source change, a stall; and the worst-case evaluation
//! cost (estimate E5).
use super::*;
use crate::playback::position_model::{PositionModel, Sample};
use serde_json::json;

/// 25 frames a second: `time-pos` moves by 40 ms.
const FRAME: f64 = 0.04;
const PUSH_MS: f64 = 100.;

struct Sim {
    /// Virtual milliseconds.
    now: f64,
    source: u64,
    /// The true position (seconds), moving continuously.
    position: f64,
    rate: f64,
    paused: bool,
    /// Buffering: not moving although not paused.
    stalled: bool,
    /// mpv says it buffers (`paused-for-cache`): the thread knows.
    cache_seen: bool,
    model: PositionModel,
    triggers: Triggers,
    next_push: f64,
    /// The last report's serial the UI thread saw.
    serial: u64,
    /// The boundaries last handed to the thread.
    sent: Option<(u64, Vec<f64>)>,
    /// (virtual ms, true position, edge, range index)
    log: Vec<(f64, f64, &'static str, Option<u64>)>,
    /// Reads of `time-pos` by the thread.
    reads: usize,
}

impl Sim {
    fn new(ranges: Value) -> Self {
        let mut sim = Self {
            now: 0.,
            source: 1,
            position: 0.,
            rate: 1.,
            paused: false,
            stalled: false,
            cache_seen: false,
            model: PositionModel::default(),
            triggers: Triggers::new(None),
            next_push: 0.,
            serial: 0,
            sent: None,
            log: Vec::new(),
            reads: 0,
        };
        sim.push();
        let (_, out) = sim
            .triggers
            .set(
                "p",
                1,
                TriggerId::Unnamed(0),
                &json!({"position":ranges}),
                0.,
            )
            .unwrap();
        sim.note(out);
        sim.sync_watch();
        sim
    }
    fn time_pos(&self) -> f64 {
        (self.position / FRAME + 1e-9).floor() * FRAME
    }
    fn running(&self) -> bool {
        !self.paused && !(self.stalled && self.cache_seen)
    }
    fn note(&mut self, out: Vec<Notification>) {
        for n in out {
            let range = n.range.as_ref().map(|r| r["index"].as_u64().unwrap());
            self.log
                .push((self.now, self.position, n.edge.as_str(), range));
        }
    }
    /// The thread reads `time-pos` (and pause, rate, buffering).
    fn read(&mut self) {
        self.reads += 1;
        let sample = Sample {
            source: self.source,
            position: Some(self.time_pos()),
            rate: self.rate,
            running: self.running(),
        };
        if let Some(report) = self.model.sample(self.now, sample) {
            self.serial = report.serial;
            let out =
                self.triggers
                    .position(report.source, report.position, report.continuous, self.now);
            self.note(out);
            // A report disarms the model: the boundaries go again, changed
            // or not (`Host::triggers_position`).
            self.sent = None;
            self.sync_watch();
        }
    }
    /// The UI thread hands over the boundaries when they changed, with the
    /// last report it saw; the thread reads at once.
    fn sync_watch(&mut self) {
        let watch = self.triggers.watch();
        if self.sent.as_ref() == Some(&watch) {
            return;
        }
        self.sent = Some(watch.clone());
        let (source, boundaries) = watch;
        if self.model.set_watch(source, boundaries, self.serial) {
            self.read();
        }
    }
    /// The 100 ms state push: a read, then `source.state` to the UI thread.
    fn push(&mut self) {
        self.read();
        let facts = Facts::new(
            &json!({"phase":"ready","paused":self.paused,"rate":self.rate}),
            &Value::Null,
            "vestra",
            "system",
        );
        let out = self
            .triggers
            .set_facts(facts, self.source, Some(self.time_pos()), self.now);
        self.note(out);
        self.sync_watch();
    }
    fn move_to(&mut self, t: f64) {
        if !self.paused && !self.stalled {
            self.position += self.rate * (t - self.now) / 1000.;
        }
        self.now = t;
    }
    /// Runs the thread's loop for `ms`: it wakes at the next push or the
    /// model's deadline, whichever is first.
    fn run(&mut self, ms: f64) {
        let end = self.now + ms;
        for _ in 0..1_000_000 {
            let deadline = self.model.deadline(self.now).map(|d| self.now + d);
            let next = deadline
                .unwrap_or(f64::INFINITY)
                .min(self.next_push)
                .min(end);
            self.move_to(next);
            if self.now >= self.next_push {
                self.next_push = self.now + PUSH_MS;
                self.push();
            } else if deadline.is_some_and(|d| d <= self.now) {
                self.read();
            }
            if self.now >= end {
                return;
            }
        }
        panic!("the thread's loop does not advance");
    }
    /// A seek: mpv's `seek` event, then the read after `playback-restart`.
    fn seek(&mut self, to: f64) {
        self.model.jumped();
        self.run(20.);
        self.position = to;
        self.read();
    }
    fn pause(&mut self, paused: bool) {
        self.paused = paused;
        self.push();
    }
    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.read();
    }
    fn open_next(&mut self) {
        self.source += 1;
        self.position = 0.;
        self.push();
    }
    fn edges(&self) -> Vec<(&'static str, Option<u64>)> {
        self.log
            .iter()
            .map(|(_, _, edge, range)| (*edge, *range))
            .collect()
    }
    /// The flip at `index` came at most `max` ms after the true crossing of
    /// `at`, never before it.
    fn assert_late(&self, index: usize, at: f64, max: f64) {
        let late = self.late(index, at);
        assert!(
            (-1e-6..=max).contains(&late),
            "flip {index}: {late} ms late, max {max}"
        );
    }
    /// How late each flip came after the true crossing of `at` (ms).
    fn late(&self, index: usize, at: f64) -> f64 {
        let (_, position, ..) = self.log[index];
        (position - at) / self.rate * 1000.
    }
}

#[test]
fn entering_and_leaving_a_range_while_playing() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(25_000.);
    assert_eq!(sim.edges(), [("enter", Some(0)), ("leave", Some(0))]);
    // At most one frame and a read late, never early.
    sim.assert_late(0, 10., 45.);
    sim.assert_late(1, 20., 45.);
    // The reads are the 100 ms pushes plus a few at the deadlines: no
    // sampling of its own.
    assert!(sim.reads < 250 + 20, "{} reads", sim.reads);
}

#[test]
fn a_lead_enters_before_the_start() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]],"lead":2}));
    sim.run(9_000.);
    assert_eq!(sim.edges(), [("enter", Some(0))]);
    sim.assert_late(0, 8., 45.);
}

#[test]
fn jumping_over_a_range_enters_nothing() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(5_000.);
    sim.seek(25.);
    sim.run(5_000.);
    assert!(sim.edges().is_empty(), "{:?}", sim.log);
}

#[test]
fn jumping_back_leaves_and_enters_again() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(15_000.);
    sim.seek(5.);
    assert_eq!(sim.edges(), [("enter", Some(0)), ("leave", Some(0))]);
    // Seeking into it is an enter (T11).
    sim.seek(12.);
    sim.seek(3.);
    sim.run(8_000.);
    assert_eq!(
        sim.edges()[2..],
        [("enter", Some(0)), ("leave", Some(0)), ("enter", Some(0))]
    );
    sim.assert_late(4, 10., 45.);
}

#[test]
fn a_pause_holds_the_deadline() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(9_900.);
    sim.pause(true);
    let reads = sim.reads;
    sim.run(5_000.);
    assert!(sim.edges().is_empty());
    // Paused: the pushes only (each 100 ms).
    assert!(sim.reads - reads <= 51, "{}", sim.reads - reads);
    sim.pause(false);
    sim.run(500.);
    assert_eq!(sim.edges(), [("enter", Some(0))]);
    sim.assert_late(0, 10., 45.);
}

#[test]
fn a_rate_change_moves_the_deadline() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(5_000.);
    sim.set_rate(2.);
    sim.run(2_400.);
    assert!(sim.edges().is_empty(), "{:?}", sim.log);
    sim.run(200.);
    assert_eq!(sim.edges(), [("enter", Some(0))]);
    sim.assert_late(0, 10., 45.);
    // Slower again: the end comes later.
    sim.set_rate(0.5);
    sim.run(19_000.);
    assert_eq!(sim.edges().len(), 1);
    sim.run(1_100.);
    assert_eq!(sim.edges().len(), 2);
}

#[test]
fn a_very_short_range_is_entered_and_left_once() {
    // 5 ms: shorter than a frame and than the thread's timer.
    let mut sim = Sim::new(json!({"ranges":[[10, 10.005], [10.5, 10.502]]}));
    sim.run(12_000.);
    assert_eq!(
        sim.edges(),
        [
            ("enter", Some(0)),
            ("leave", Some(0)),
            ("enter", Some(1)),
            ("leave", Some(1))
        ]
    );
}

#[test]
fn adjacent_ranges_leave_one_and_enter_the_next() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20], [20, 30]]}));
    sim.run(31_000.);
    assert_eq!(
        sim.edges(),
        [
            ("enter", Some(0)),
            ("leave", Some(0)),
            ("enter", Some(1)),
            ("leave", Some(1))
        ]
    );
    // The leave and the enter come together.
    assert_eq!(sim.log[1].0, sim.log[2].0);
}

#[test]
fn a_new_source_leaves_and_the_list_does_not_match_it() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(15_000.);
    sim.open_next();
    sim.run(15_000.);
    assert_eq!(sim.edges(), [("enter", Some(0)), ("leave", Some(0))]);
    // Nothing is watched for the new source.
    assert_eq!(sim.triggers.watch(), (0, vec![]));
    assert_eq!(sim.model.deadline(sim.now), None);
}

#[test]
fn a_stall_waits_without_looping_and_the_range_comes_after_it() {
    let mut sim = Sim::new(json!({"ranges":[[10, 20]]}));
    sim.run(9_970.);
    // Buffering that mpv does not report as such: running, not moving.
    sim.stalled = true;
    let reads = sim.reads;
    sim.run(3_000.);
    assert!(sim.edges().is_empty());
    assert!(sim.reads - reads <= 40, "{} reads", sim.reads - reads);
    // Reported (`paused-for-cache`): not running at all.
    sim.cache_seen = true;
    sim.read();
    let reads = sim.reads;
    sim.run(2_000.);
    assert!(sim.reads - reads <= 21, "{} reads", sim.reads - reads);
    sim.stalled = false;
    sim.cache_seen = false;
    sim.run(500.);
    assert_eq!(sim.edges(), [("enter", Some(0))]);
    sim.assert_late(0, 10., 145.);
}

/// Estimate E5: 32 plugins × 64 triggers × 32 nodes, every condition
/// reading the source's facts, all evaluated by one change. Recorded with
/// `cargo test --release worst_case -- --nocapture`.
#[test]
fn worst_case_evaluation_cost() {
    let mut triggers = Triggers::new(None);
    let facts = |paused: bool| {
        Facts::new(
            &json!({"phase":"ready","paused":paused}),
            &Value::Null,
            "vestra",
            "system",
        )
    };
    triggers.set_facts(facts(true), 1, Some(1000.5), 0.);
    let ranges: Vec<_> = (0..1000).map(|i| json!([i * 2, i * 2 + 1])).collect();
    let mut nodes = 0;
    for plugin in 0..32 {
        for place in 0..64u32 {
            // 31 atoms under one `all`: 32 nodes, every one evaluated (the
            // one that flips is the last); one in four has a range list of
            // 1000 ranges.
            let mut atoms = vec![json!({"phase":["ready"]}); 28];
            atoms.push(json!({"volume":{"min":0}}));
            atoms.push(if place % 4 == 0 {
                json!({"position":{"ranges":ranges}})
            } else {
                json!({"phase":["ready"]})
            });
            atoms.push(json!({"paused":false}));
            nodes += 32;
            triggers
                .set(
                    &format!("p{plugin}"),
                    1,
                    TriggerId::Unnamed(place),
                    &json!({ "all": atoms }),
                    0.,
                )
                .unwrap();
        }
    }
    assert_eq!(nodes, 65_536);
    let rounds = 50;
    let began = std::time::Instant::now();
    let mut flips = 0;
    for round in 0..rounds {
        flips += triggers
            .set_facts(facts(round % 2 == 1), 1, Some(1000.5), 0.)
            .len();
    }
    let each = began.elapsed() / rounds;
    diag!("E5 worst case: 2048 conditions, 65 536 nodes, one change evaluates all: {each:?}");
    // Every condition flips at every change.
    assert_eq!(flips, rounds as usize * 2048);
    // The same evaluation without a flip (the volume moves: every
    // condition reads the source's facts, none changes its truth).
    let began = std::time::Instant::now();
    for round in 0..rounds {
        // Paused as the last round left it.
        let mut changed = facts(true);
        changed.volume = 50. + f64::from(round % 2);
        assert!(triggers.set_facts(changed, 1, Some(1000.5), 0.).is_empty());
    }
    let quiet = began.elapsed() / rounds;
    diag!("E5 worst case without a flip: {quiet:?}");
    // Generous for a debug build on a busy machine; the release figure is
    // the one recorded.
    assert!(each < std::time::Duration::from_millis(200), "{each:?}");
}
