//! The position model (doc/architecture/events-and-triggers.md §4.4): the
//! playback thread's anchor `(p0, t0, rate, running)` and the deadline of
//! the nearest position boundary a trigger's condition waits for.
//!
//! Between two discrete changes the position is `p0 + rate × (t − t0)`;
//! everything that breaks this (a seek, a pause, a rate change, a source
//! change, a buffering stall, the end) is something this thread sees. So
//! the UI thread, which evaluates conditions, hands over only "the next
//! boundary after the position" of each range list (`set_watch`); this
//! model computes when playback reaches the nearest one, the thread reads
//! mpv's `time-pos` once at that moment (`sample`) and reports
//! "crossed, real position p" (`Report`), or "jumped" after a
//! discontinuity. It never samples on its own: the reads are the thread's
//! existing 100 ms state reads, the reads at a deadline, and the reads after
//! the events that anchor it again. With no watch it reports nothing and
//! adds no wait (the thread waits exactly as before).
//!
//! Pure: time is passed in (milliseconds on any monotonic clock), so the
//! simulation tests drive it without waiting.

/// A position read further than this from the prediction is a jump (a seek
/// the thread did not see as one): media seconds.
pub const JUMP: f64 = 0.3;
/// A read short of the boundary looks again after at least this long (ms):
/// `time-pos` moves by frames, so a read inside the frame before the
/// boundary would otherwise look again at once, again and again.
pub const MIN_WAIT_MS: f64 = 4.;
/// Running, but no movement for this long (ms): a stall. No deadline until
/// the position moves again or the model is anchored again.
pub const STALL_MS: f64 = 250.;
/// Movement smaller than this does not count as movement (media seconds).
const STILL: f64 = 1e-6;

/// One read of the playback state (the thread's own, at a push, a deadline
/// or after an anchoring event).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The source id (`stamp.generation`).
    pub source: u64,
    /// `time-pos`; `None` while there is none.
    pub position: Option<f64>,
    pub rate: f64,
    /// Ready, not paused, not seeking, not buffering.
    pub running: bool,
}

/// What the model tells the UI thread: the real position of `source` now,
/// and whether playback got there continuously since the previous report
/// (`false`: a jump, a new source, the first report of a watch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    /// The report's serial: a watch computed before it is stale.
    pub serial: u64,
    pub source: u64,
    pub position: f64,
    pub continuous: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    source: u64,
    position: f64,
    /// When `position` was read (ms).
    at: f64,
    rate: f64,
    running: bool,
}
impl Anchor {
    fn predict(&self, now: f64) -> f64 {
        if self.running {
            self.position + self.rate * (now - self.at).max(0.) / 1000.
        } else {
            self.position
        }
    }
}

/// What the UI thread waits for: the boundaries of `source`, sorted.
#[derive(Clone, Debug, PartialEq)]
struct Watch {
    source: u64,
    boundaries: Vec<f64>,
    /// Boundaries not reported yet: a report disarms the watch until the UI
    /// thread, having evaluated it, hands over the next boundaries.
    armed: bool,
}

#[derive(Debug, Default)]
pub struct PositionModel {
    watch: Option<Watch>,
    anchor: Option<Anchor>,
    /// A discontinuity since the last report (a seek began, a frame step).
    jumped: bool,
    /// Running without moving since then (ms).
    still_since: Option<f64>,
    /// Running without moving for `STALL_MS`.
    stalled: bool,
    /// A read short of the boundary: look again then (ms).
    retry: Option<f64>,
    /// The last report's serial.
    serial: u64,
}

impl PositionModel {
    /// Whether a trigger waits on the position (the thread reads and reports
    /// only then).
    pub fn watching(&self) -> bool {
        self.watch.is_some()
    }
    /// The UI thread's boundaries for `source`, computed after report
    /// `after` (a watch older than the last report is stale: dropped); an
    /// empty list with `source` 0 ends the watch. Whether the thread should
    /// read the position now (a boundary may be behind it already).
    pub fn set_watch(&mut self, source: u64, mut boundaries: Vec<f64>, after: u64) -> bool {
        if source == 0 {
            *self = Self {
                serial: self.serial,
                anchor: self.anchor,
                ..Self::default()
            };
            return false;
        }
        if after < self.serial {
            return false;
        }
        boundaries.retain(|b| b.is_finite());
        boundaries.sort_by(f64::total_cmp);
        boundaries.dedup();
        self.watch = Some(Watch {
            source,
            boundaries,
            armed: true,
        });
        self.retry = None;
        true
    }
    /// A discontinuity began (mpv's `seek`, a frame step): the next report
    /// says it jumped; the position does not run until the next read.
    pub fn jumped(&mut self) {
        self.jumped = true;
        if let Some(anchor) = &mut self.anchor {
            anchor.running = false;
        }
    }
    /// One read at `now`: the anchor follows it; a report when the watched
    /// source jumped, or reached a boundary.
    pub fn sample(&mut self, now: f64, sample: Sample) -> Option<Report> {
        let Some(position) = sample.position else {
            // Nothing to anchor on (no source, still loading): it starts
            // again from the next position.
            if self.anchor.is_some_and(|a| a.source != sample.source) || sample.source == 0 {
                self.jumped = true;
            }
            self.anchor = None;
            return None;
        };
        // Movement since the last read; not moving is never a jump.
        let moved = self
            .anchor
            .is_none_or(|a| a.source != sample.source || (position - a.position).abs() > STILL);
        let continuous = match self.anchor {
            Some(anchor) if anchor.source == sample.source && !self.jumped => {
                !moved || (position - anchor.predict(now)).abs() <= JUMP
            }
            _ => false,
        };
        // A stall: running without moving since `still_since`.
        let still = !moved
            && sample.running
            && self
                .anchor
                .is_some_and(|a| a.running && a.source == sample.source);
        self.still_since = if still {
            self.still_since.or(self.anchor.map(|a| a.at))
        } else {
            None
        };
        self.stalled = self
            .still_since
            .is_some_and(|since| now - since >= STALL_MS);
        self.anchor = Some(Anchor {
            source: sample.source,
            position,
            at: now,
            rate: if sample.rate.is_finite() && sample.rate > 0. {
                sample.rate
            } else {
                1.
            },
            running: sample.running,
        });
        self.retry = None;
        if !continuous {
            self.jumped = true;
        }
        let watch = self.watch.as_mut().filter(|w| w.source == sample.source)?;
        if self.jumped {
            self.jumped = false;
            watch.armed = false;
            return Some(self.report(sample.source, position, false));
        }
        if !watch.armed {
            return None;
        }
        let next = watch
            .boundaries
            .iter()
            .find(|b| **b > position - STILL)
            .copied();
        let crossed = watch
            .boundaries
            .first()
            .is_some_and(|b| *b <= position + STILL);
        if crossed {
            watch.armed = false;
            return Some(self.report(sample.source, position, true));
        }
        // Short of it (a frame behind, the clock): look again for the rest.
        if let Some(next) = next
            && sample.running
            && !self.stalled
        {
            let rest = (next - position) / self.anchor.map_or(1., |a| a.rate) * 1000.;
            self.retry = Some(now + rest.max(MIN_WAIT_MS));
        }
        None
    }
    fn report(&mut self, source: u64, position: f64, continuous: bool) -> Report {
        self.serial += 1;
        Report {
            serial: self.serial,
            source,
            position,
            continuous,
        }
    }
    /// How long until the thread should read the position (ms from `now`):
    /// the predicted moment the nearest boundary is reached, or a retry;
    /// `None`: nothing to wait for (no watch, paused, stalled, nothing
    /// ahead).
    pub fn deadline(&self, now: f64) -> Option<f64> {
        let watch = self.watch.as_ref().filter(|w| w.armed)?;
        let anchor = self.anchor.filter(|a| a.source == watch.source)?;
        if !anchor.running || self.stalled {
            return None;
        }
        let first = *watch.boundaries.first()?;
        let due = if first <= anchor.position {
            now
        } else {
            anchor.at + (first - anchor.position) / anchor.rate * 1000.
        };
        // A short read looks again for the rest, never sooner.
        let due = self.retry.unwrap_or(due);
        Some((due - now).max(0.))
    }
}

#[cfg(test)]
#[path = "position_model_tests.rs"]
mod tests;
