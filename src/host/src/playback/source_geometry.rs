//! Software-renderer orientation preparation (host-internal).
//!
//! The initial metadata filter is a renderer safety prerequisite, not a pixel
//! rotation. Publication stays blocked until the physical transform is observed
//! in output parameters. This is a geometry check, not a decoded-frame fence.
//!
//! Output parameters are read after the whole chain, so a filter after the
//! orientation changes them: the upscaler scales (`upscaling.rs` holds it
//! out while the gate is closed), a plugin's chain may crop, pad or turn
//! the picture (`processing.rs` replays it at FileLoaded, before the
//! orientation is prepared). The output confirms the orientation when it
//! has the expected size ([`Evidence::Exact`]), or the expected aspect at
//! another scale while the other orientation's aspect does not fit
//! ([`Evidence::Aspect`]); otherwise, after [`TIMEOUT`] and once a frame
//! went through the new chain ([`Mark`]), the gate opens without evidence ([`Evidence::Timeout`], a diagnostic
//! line), so the picture never stays frozen while the audio plays.
#![forbid(unsafe_code)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use rsmpv::{Mpv, Node};

use crate::{
    host::RpcError,
    playback::session::{IntentTicket, Phase},
    playback::source_backend::Sources,
};

pub const INITIAL_FILTERS: &str =
    "@vestra-orientation:lavfi=[null],@vestra-orientation-metadata:format=rotate=0";

/// How long the gate waits for output that confirms the orientation before
/// it opens without evidence (a filter after the orientation changes the
/// output's shape: a plugin's crop, pad or quarter turn).
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// Why the output confirms the prepared orientation ([`evidence`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// The expected size.
    Exact,
    /// The expected aspect at another scale (a scaler after the
    /// orientation), and not the other orientation's aspect.
    Aspect,
    /// No evidence after [`TIMEOUT`]: a later filter changed the shape.
    Timeout,
}

impl Evidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Aspect => "aspect",
            Self::Timeout => "timeout",
        }
    }
}

/// Whether `output` has the aspect of `expected` at some scale, within two
/// output pixels a side: `w * eh - h * ew` moves by `eh` per pixel of width
/// and `ew` per pixel of height, whatever the scale.
fn same_aspect(output: (i64, i64), expected: (i64, i64)) -> bool {
    let ((w, h), (ew, eh)) = (output, expected);
    (w * eh - h * ew).abs() <= 2 * (ew + eh)
}

/// What the output (`video-out-params` w, h, rotate) says about the
/// prepared orientation after `waited`; `passed`: a frame has gone through
/// the new chain since the orientation filter went in ([`Mark::see`]).
/// Only rotate 0 counts (the metadata filter). A frame turned the other way
/// (still filtered by the previous file's orientation until `vf pre`
/// replaces it) has the other aspect, so it never passes as `Aspect`; a
/// picture within a few pixels of square has both aspects and passes only
/// as `Exact` or by the timeout.
pub fn evidence(
    expected: (i64, i64),
    output: (i64, i64, i64),
    waited: Duration,
    passed: bool,
) -> Option<Evidence> {
    let (w, h, rotate) = output;
    if rotate != 0 || w <= 0 || h <= 0 {
        return None;
    }
    if (w, h) == expected {
        return Some(Evidence::Exact);
    }
    let swapped = (expected.1, expected.0);
    // Evidence about the shape, not about each frame: a stale frame of the
    // previous chain with the same aspect passes too, for example one
    // turned by 180° or flipped and then scaled (the size check before it
    // had the same blind spot for those at the same size).
    if same_aspect((w, h), expected) && !same_aspect((w, h), swapped) {
        return Some(Evidence::Aspect);
    }
    // `video-out-params` keeps the last frame's values until a frame of the
    // new chain reaches the output: a stream that stalls after its first
    // frame, or starts paused, may still show the previous file's chain
    // (a sideways picture after a portrait file). The timeout opens only
    // once a frame went through the new chain.
    (passed && waited >= TIMEOUT).then_some(Evidence::Timeout)
}

/// How far playback must move on before it shows that frames went through
/// the new chain: more than the few frames filtered before `vf pre` that
/// the output may still hold.
const PASSED_SECONDS: f64 = 0.25;

/// The side-data removal after the transform in the orientation filter.
const SIDEDATA: &str = "sidedata=mode=delete:type=DISPLAYMATRIX";

/// Whether a frame went through the new chain since the orientation filter
/// went in ([`Mark::see`], once per tick with mpv's output and position).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    /// The first output and position seen after `vf pre`. Not read at
    /// `vf pre` itself: before the new file's first frame shows, `time-pos`
    /// is still where the load started (0), and jumps to the video's start
    /// when that frame (maybe filtered by the old chain) shows.
    base: Option<((i64, i64, i64), Option<f64>)>,
    /// Where playback first moved after the base: the first frame shown,
    /// which the old chain may have filtered; forward movement counts from
    /// here.
    moved: Option<f64>,
    passed: bool,
}

impl Mark {
    /// `same`: the orientation filter before `vf pre` turned the picture as
    /// the new one does, so a frame of the old chain is already right.
    pub fn new(same: bool) -> Self {
        Self {
            base: None,
            moved: None,
            passed: same,
        }
    }

    /// Whether a frame went through the new chain (kept once seen): the
    /// output changed from the first one seen (mpv reconfigures it only for
    /// a frame with other parameters); playback went back (a seek flushes
    /// the chain); or playback moved forward by [`PASSED_SECONDS`] from
    /// where it first moved. None of these while a stream stalls or stays
    /// paused on a frame filtered before: the gate then waits for playback,
    /// unless the orientation stayed the same ([`Mark::new`]).
    pub fn see(&mut self, output: (i64, i64, i64), position: Option<f64>) -> bool {
        if self.passed {
            return true;
        }
        let Some((first, base)) = self.base else {
            self.base = Some((output, position));
            return false;
        };
        if output != first {
            self.passed = true;
            return true;
        }
        match (base, position) {
            (None, Some(now)) => self.base = Some((first, Some(now))),
            (Some(at), Some(now)) => {
                let from = self.moved.unwrap_or(at);
                if now <= from - PASSED_SECONDS {
                    self.passed = true;
                } else if self.moved.is_some() {
                    self.passed = now - from >= PASSED_SECONDS;
                } else if now != at {
                    self.moved = Some(now);
                }
            }
            _ => {}
        }
        self.passed
    }
}

/// The transform the `@vestra-orientation` filter applies now: its graph
/// without the side-data removal; `null` when the chain has none. `None`
/// when mpv's chain cannot be read as expected (then nothing is assumed).
fn current_transform(mpv: &Mpv) -> Option<String> {
    let Ok(Node::Array(filters)) = mpv.get_property::<Node>("vf") else {
        return None;
    };
    let field = |fields: &[(String, Node)], key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    for filter in &filters {
        let Node::Map(fields) = filter else {
            return None;
        };
        if !matches!(field(fields, "label"), Some(Node::String(label)) if label == "vestra-orientation")
        {
            continue;
        }
        let Some(Node::Map(params)) = field(fields, "params") else {
            return None;
        };
        let Some(Node::String(graph)) = field(&params, "graph") else {
            return None;
        };
        let graph = graph.trim().trim_start_matches('[').trim_end_matches(']');
        let transform = graph
            .strip_suffix(SIDEDATA)
            .map_or(graph, |rest| rest.trim_end_matches(','));
        return Some(transform.to_owned());
    }
    Some("null".into())
}

type Load = (i64, IntentTicket);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    transform: &'static str,
    width: i64,
    height: i64,
}

fn plan(width: i64, height: i64, clockwise: i64) -> Result<Plan, RpcError> {
    if width <= 0 || height <= 0 {
        return Err(RpcError::new(
            "source-geometry",
            "invalid decoder dimensions",
        ));
    }
    let (transform, swap) = match clockwise.rem_euclid(360) {
        0 => ("null", false),
        90 => ("transpose=clock", true),
        180 => ("hflip,vflip", false),
        270 => ("transpose=cclock", true),
        _ => {
            return Err(RpcError::new(
                "source-geometry-unsupported",
                "only cardinal source rotations are supported",
            ));
        }
    };
    Ok(Plan {
        transform,
        width: if swap { height } else { width },
        height: if swap { width } else { height },
    })
}

enum Preparation {
    WaitingOutput(Waiting),
    Confirmed,
    Failed,
}

/// The orientation filter is in the chain, its output not confirmed yet.
struct Waiting {
    plan: Plan,
    /// When the filter went in.
    since: Instant,
    /// Whether a frame went through the new chain since.
    mark: Mark,
}

pub struct Geometry {
    publish_gate: Arc<AtomicBool>,
    prepared: Option<(Load, Preparation)>,
}

impl Geometry {
    pub fn new(publish_gate: Arc<AtomicBool>) -> Self {
        Self {
            publish_gate,
            prepared: None,
        }
    }

    /// Whether `tick` is preparing the current load's geometry from mpv's
    /// properties (no event says when they are there), so the control worker
    /// keeps calling it every few milliseconds. Not once confirmed or failed:
    /// a new load comes with an mpv event or a command.
    pub fn polling(&self) -> bool {
        !self.publish_gate.load(Ordering::Acquire)
            && !matches!(self.prepared, Some((_, Preparation::Failed)))
    }

    pub fn ready_for(&self, load: (i64, IntentTicket)) -> bool {
        matches!(&self.prepared,Some((prepared,Preparation::Confirmed)) if *prepared==load)
            && self.publish_gate.load(Ordering::Acquire)
    }
    pub fn begin(&mut self) {
        self.publish_gate.store(false, Ordering::Release);
        self.prepared = None;
    }

    /// Called serially by the control thread. Missing decoder/output properties
    /// mean "not prepared yet", never permission to publish an unverified frame.
    pub fn tick(&mut self, mpv: &Mpv, sources: &Sources) -> Result<(), RpcError> {
        self.advance(mpv, sources.phase(), sources.presentation_load(), TIMEOUT)
    }

    /// [`Geometry::tick`] from the source's phase and presentation load,
    /// with the wait for evidence (tests drive it with a plain mpv and a
    /// shorter wait).
    fn advance(
        &mut self,
        mpv: &Mpv,
        phase: Phase,
        load: Option<Load>,
        timeout: Duration,
    ) -> Result<(), RpcError> {
        if matches!(phase, Phase::Empty | Phase::Error) {
            self.prepared = None;
            self.publish_gate.store(true, Ordering::Release);
            return Ok(());
        }
        let Some(load) = load else {
            // In particular, natural EOF retains the last confirmed geometry.
            return Ok(());
        };
        if self
            .prepared
            .as_ref()
            .is_some_and(|(previous, _)| *previous != load)
        {
            self.begin();
        }
        if let Some((_, preparation)) = &mut self.prepared {
            if let Preparation::WaitingOutput(waiting) = preparation {
                let (Ok(width), Ok(height), Ok(rotation)) = (
                    mpv.get_property::<i64>("video-out-params/w"),
                    mpv.get_property::<i64>("video-out-params/h"),
                    mpv.get_property::<i64>("video-out-params/rotate"),
                ) else {
                    return Ok(());
                };
                let position = mpv.get_property::<f64>("time-pos").ok();
                let passed = waiting.mark.see((width, height, rotation), position);
                let expected = waiting.plan;
                let waited = waiting.since.elapsed();
                // `evidence` counts against TIMEOUT; a shorter `timeout`
                // (tests) counts as if it had waited the difference.
                let counted = waited + TIMEOUT.saturating_sub(timeout);
                if let Some(evidence) = evidence(
                    (expected.width, expected.height),
                    (width, height, rotation),
                    counted,
                    passed,
                ) {
                    let note = if evidence == Evidence::Timeout {
                        format!(
                            " after {} ms: a filter after the orientation changes the output's shape; opened without evidence",
                            waited.as_millis()
                        )
                    } else {
                        String::new()
                    };
                    diag!(
                        "source geometry ready entry={} generation={} cycle={} expected={}x{} output={}x{} rotate=0 evidence={}{}",
                        load.0,
                        load.1.stamp.generation,
                        load.1.cycle,
                        expected.width,
                        expected.height,
                        width,
                        height,
                        evidence.as_str(),
                        note
                    );
                    self.prepared = Some((load, Preparation::Confirmed));
                    self.publish_gate.store(true, Ordering::Release);
                }
            }
            return Ok(());
        }
        self.publish_gate.store(false, Ordering::Release);

        // Only a successfully read, structurally valid actual track list can
        // establish audio-only playback. A missing property is not evidence.
        let Ok(tracks) = mpv.get_property::<Node>("track-list") else {
            return Ok(());
        };
        match selected_video(&tracks) {
            Some(false) => {
                diag!(
                    "source geometry ready entry={} generation={} cycle={} no selected video",
                    load.0,
                    load.1.stamp.generation,
                    load.1.cycle
                );
                self.prepared = Some((load, Preparation::Confirmed));
                self.publish_gate.store(true, Ordering::Release);
                return Ok(());
            }
            Some(true) => {}
            None => return Ok(()),
        }
        let (Ok(width), Ok(height), Ok(rotation)) = (
            mpv.get_property::<i64>("video-dec-params/w"),
            mpv.get_property::<i64>("video-dec-params/h"),
            mpv.get_property::<i64>("video-dec-params/rotate"),
        ) else {
            return Ok(());
        };
        let expected = match plan(width, height, rotation) {
            Ok(expected) => expected,
            Err(error) => {
                self.prepared = Some((load, Preparation::Failed));
                return Err(error);
            }
        };
        // Generated constants only: no paths, expressions, or plugin input.
        // vf pre replaces this reserved label before the metadata-zero filter.
        let filter = format!(
            "@vestra-orientation:lavfi=[{},{SIDEDATA}]",
            expected.transform
        );
        // The transform before it: when it is the same, the frames the old
        // chain filtered are already turned right (a paused resume of the
        // next episode with a plugin's crop: no frame changes anything).
        let previous = current_transform(mpv);
        let same = previous.as_deref() == Some(expected.transform);
        diag!(
            "source geometry prepare entry={} generation={} cycle={} decoder={}x{} rotate={} filter={} previous={}",
            load.0,
            load.1.stamp.generation,
            load.1.cycle,
            width,
            height,
            rotation,
            filter,
            previous.as_deref().unwrap_or("unknown")
        );
        self.prepared = Some((load, Preparation::Failed));
        mpv.command(&["vf", "pre", &filter]).map_err(|error| {
            RpcError::new(
                "source-geometry",
                format!("orientation filter failed: {error}"),
            )
        })?;
        // The mark takes its base from the first tick that sees an output.
        self.prepared = Some((
            load,
            Preparation::WaitingOutput(Waiting {
                plan: expected,
                since: Instant::now(),
                mark: Mark::new(same),
            }),
        ));
        // Deliberately do not inspect output in the configuration tick.
        Ok(())
    }
}

fn selected_video(tracks: &Node) -> Option<bool> {
    let Node::Array(tracks) = tracks else {
        return None;
    };
    let mut selected = false;
    for track in tracks {
        let Node::Map(fields) = track else {
            return None;
        };
        let Some(Node::String(kind)) = fields
            .iter()
            .find(|(key, _)| key == "type")
            .map(|(_, value)| value)
        else {
            return None;
        };
        if kind == "video" {
            let Some(Node::Flag(value)) = fields
                .iter()
                .find(|(key, _)| key == "selected")
                .map(|(_, value)| value)
            else {
                return None;
            };
            selected |= *value;
        }
    }
    Some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwise_cardinal_rotations_transform_pixels_and_dimensions() {
        for (angle, transform, dimensions) in [
            (0, "null", (320, 180)),
            (90, "transpose=clock", (180, 320)),
            (180, "hflip,vflip", (320, 180)),
            (270, "transpose=cclock", (180, 320)),
        ] {
            let actual = plan(320, 180, angle).unwrap();
            assert_eq!(actual.transform, transform);
            assert_eq!((actual.width, actual.height), dimensions);
        }
    }

    #[test]
    fn equivalent_angles_preserve_direction_and_unsupported_angles_fail() {
        assert_eq!(plan(320, 180, -90).unwrap(), plan(320, 180, 270).unwrap());
        assert_eq!(plan(320, 180, 450).unwrap(), plan(320, 180, 90).unwrap());
        assert!(plan(320, 180, 45).is_err());
        assert!(plan(0, 180, 90).is_err());
        assert!(plan(320, -1, 0).is_err());
    }

    #[test]
    fn the_output_confirms_by_size_by_aspect_or_after_the_timeout() {
        let now = Duration::ZERO;
        // Frames have gone through the new chain (see the next test).
        let evidence = |expected, output, waited| evidence(expected, output, waited, true);
        // The expected size, at once.
        assert_eq!(
            evidence((1920, 1080), (1920, 1080, 0), now),
            Some(Evidence::Exact)
        );
        // An upscaler after the orientation (×1.33, even rounding; ×2).
        assert_eq!(
            evidence((1920, 1080), (2554, 1436, 0), now),
            Some(Evidence::Aspect)
        );
        assert_eq!(
            evidence((1080, 1920), (2160, 3840, 0), now),
            Some(Evidence::Aspect)
        );
        // A frame the previous file's quarter turn still filters, at any
        // scale: the other aspect is no evidence, before the timeout.
        assert_eq!(evidence((1080, 1920), (1920, 1080, 0), now), None);
        assert_eq!(evidence((1920, 1080), (1436, 2554, 0), now), None);
        // A crop after the orientation: only the timeout.
        assert_eq!(evidence((1920, 1080), (1920, 816, 0), now), None);
        assert_eq!(
            evidence((1920, 1080), (1920, 816, 0), TIMEOUT),
            Some(Evidence::Timeout)
        );
        // Square (and almost square): both orientations have the aspect, so
        // only the exact size or the timeout.
        assert_eq!(
            evidence((1080, 1080), (1080, 1080, 0), now),
            Some(Evidence::Exact)
        );
        assert_eq!(evidence((1080, 1080), (2160, 2160, 0), now), None);
        assert_eq!(evidence((1080, 1081), (2160, 2162, 0), now), None);
        // Two pixels off square is far enough at ×1.5.
        assert_eq!(
            evidence((1080, 1082), (1620, 1623, 0), now),
            Some(Evidence::Aspect)
        );
        assert_eq!(
            evidence((1080, 1080), (2160, 2160, 0), TIMEOUT),
            Some(Evidence::Timeout)
        );
        // Never with display rotation left in the output, or no picture.
        assert_eq!(evidence((1920, 1080), (1920, 1080, 90), TIMEOUT), None);
        assert_eq!(evidence((1920, 1080), (0, 1080, 0), TIMEOUT), None);
    }

    /// Review M1: `video-out-params` keeps the previous chain's values until
    /// a frame of the new chain reaches the output. A portrait file (its
    /// quarter turn in the chain), then a landscape stream that stalls after
    /// its first frame or starts paused: the stale output is the swapped
    /// aspect, and the timeout must not publish that sideways picture.
    #[test]
    fn the_timeout_waits_for_a_frame_through_the_new_chain() {
        let expected = (1920, 1080);
        let stale = (1080, 1920, 0);
        let long = TIMEOUT * 5;
        // Seen at 12.0 s on the first tick after the filter went in.
        let seen = |output, position| {
            let mut mark = Mark::new(false);
            assert!(!mark.see(output, position));
            mark
        };
        // No new frame: the same output, the position where it was (paused,
        // or a stall); closed however long it waits.
        let mut mark = seen(stale, Some(12.0));
        for _ in 0..3 {
            let passed = mark.see(stale, Some(12.0));
            assert!(!passed);
            assert_eq!(evidence(expected, stale, long, passed), None);
        }
        assert!(!mark.see(stale, None));
        // A frame of the new chain: the output changes (here to the
        // expected size, which confirms at once anyway).
        let fresh = (1920, 1080, 0);
        assert!(mark.see(fresh, Some(12.0)));
        assert!(mark.see(stale, Some(12.0)), "kept once seen");
        assert_eq!(
            evidence(expected, fresh, Duration::ZERO, true),
            Some(Evidence::Exact)
        );
        // With a crop after the orientation that keeps the output's size:
        // playback moves on. The first move (one frame, 40 ms) may be a
        // frame of the old chain; 0.25 s from there counts.
        let cropped = (1920, 816, 0);
        let mut mark = seen(cropped, Some(12.0));
        assert!(!mark.see(cropped, Some(12.04)));
        assert!(!mark.see(cropped, Some(12.2)));
        assert_eq!(evidence(expected, cropped, long, false), None);
        assert!(mark.see(cropped, Some(12.3)));
        assert_eq!(
            evidence(expected, cropped, TIMEOUT, true),
            Some(Evidence::Timeout)
        );
        assert_eq!(
            evidence(expected, cropped, TIMEOUT - Duration::from_millis(1), true),
            None
        );
        // A seek back flushes the chain: the next frames are the new chain's.
        let mut mark = seen(cropped, Some(12.0));
        assert!(mark.see(cropped, Some(3.0)));
        // No position at first: the first one seen is the base, not a move.
        let mut mark = seen(cropped, None);
        assert!(!mark.see(cropped, Some(7.0)));
        assert!(!mark.see(cropped, Some(7.0)));
        assert!(!mark.see(cropped, Some(7.04)));
        assert!(mark.see(cropped, Some(7.3)));
    }

    /// Review 2: the video starts later than the audio. The first tick sees
    /// the position where the load started (0) with the previous output;
    /// the first frame (maybe filtered by the old chain) then moves it to
    /// the video's start, 5 s on. That jump is no evidence; only playback
    /// from there is.
    #[test]
    fn a_late_video_start_is_not_a_frame_through_the_new_chain() {
        let stale = (1080, 1920, 0);
        let mut mark = Mark::new(false);
        assert!(!mark.see(stale, Some(0.0)));
        assert!(!mark.see(stale, Some(5.0)), "the first frame shows");
        // It stalls there.
        for _ in 0..3 {
            assert!(!mark.see(stale, Some(5.0)));
        }
        assert!(!mark.see(stale, Some(5.2)));
        assert!(mark.see(stale, Some(5.25)));
    }

    /// Review 1: the same transform before and after `vf pre` (consecutive
    /// episodes in one orientation): an old-chain frame is already right,
    /// so the timeout needs no new frame.
    #[test]
    fn the_same_orientation_needs_no_new_frame() {
        let mut mark = Mark::new(true);
        assert!(mark.see((1920, 816, 0), Some(0.0)));
        assert!(mark.see((1920, 816, 0), None));
    }

    // --------------------------------------------------- with a real libmpv

    fn one_mpv_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        static MPV: std::sync::Mutex<()> = std::sync::Mutex::new(());
        MPV.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The worker's mpv (null outputs), with `extra` after the orientation
    /// filters: what the previous file left in the chain.
    fn mpv(extra: &str) -> Mpv {
        let mpv = Mpv::builder()
            .unwrap()
            .set_property("vo", "null")
            .unwrap()
            .set_property("ao", "null")
            .unwrap()
            .set_property("vf", INITIAL_FILTERS)
            .unwrap()
            .build()
            .unwrap();
        mpv.command(&["vf", "add", extra]).unwrap();
        mpv
    }

    fn load() -> Load {
        use crate::playback::session::SourceStamp;
        (
            1,
            IntentTicket {
                stamp: SourceStamp {
                    generation: 1,
                    intent_version: 1,
                },
                cycle: 1,
                owner: 1,
            },
        )
    }

    /// A new load of a generated 320×240 picture (closed gate), up to its
    /// FileLoaded: the presentation load exists from then on (before it the
    /// track list is empty, which would read as audio only).
    fn start(mpv: &Mpv, paused: bool) -> (Arc<AtomicBool>, Geometry) {
        let gate = Arc::new(AtomicBool::new(true));
        let mut geometry = Geometry::new(gate.clone());
        geometry.begin();
        mpv.set_property("pause", paused).unwrap();
        mpv.command(&["loadfile", "av://lavfi:testsrc=size=320x240:rate=25"])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(mpv.poll_event(), Some(rsmpv::Event::FileLoaded)) {
            assert!(Instant::now() < deadline, "not loaded");
            std::thread::sleep(Duration::from_millis(1));
        }
        (gate, geometry)
    }

    fn output(mpv: &Mpv) -> (i64, i64) {
        (
            mpv.get_property::<i64>("video-out-params/w").unwrap(),
            mpv.get_property::<i64>("video-out-params/h").unwrap(),
        )
    }

    /// Loads a generated 320×240 picture and ticks the geometry (as the
    /// control worker does, every 2 ms) until the gate opens; returns how
    /// long after the orientation filter went in.
    fn opens(mpv: &Mpv, timeout: Duration) -> Duration {
        let (gate, mut geometry) = start(mpv, false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut prepared = None;
        while !geometry.ready_for(load()) {
            assert!(Instant::now() < deadline, "the gate never opened");
            assert!(!gate.load(Ordering::Acquire));
            geometry
                .advance(mpv, Phase::Ready, Some(load()), timeout)
                .unwrap();
            if prepared.is_none()
                && matches!(geometry.prepared, Some((_, Preparation::WaitingOutput(..))))
            {
                prepared = Some(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(gate.load(Ordering::Acquire));
        prepared.expect("prepared").elapsed()
    }

    /// The bug: an upscaler left in the chain (RTX VSR on, then the next
    /// file) scaled the output, the exact size never came and the picture
    /// stayed frozen on the last file's frame while the audio played. A
    /// scaler keeps the aspect: the gate opens on it, not by the timeout.
    #[test]
    fn a_scaling_filter_left_into_the_next_load_still_reaches_ready() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("@vestra-upscale:lavfi=[scale=w=iw*1.50:h=ih*1.50]");
        let waited = opens(&mpv, TIMEOUT);
        assert!(waited < TIMEOUT, "{waited:?}");
        assert_eq!(mpv.get_property::<i64>("video-out-params/w").unwrap(), 480);
    }

    /// A plugin's crop (Picture ▸ Crop, replayed into every load) changes
    /// the aspect: the gate opens after the timeout, not never.
    #[test]
    fn a_plugin_crop_in_the_chain_opens_the_gate_after_the_timeout() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("@vestra-owned:lavfi=[crop=w=320:h=136]");
        let timeout = Duration::from_millis(300);
        let waited = opens(&mpv, timeout);
        assert!(waited >= timeout, "{waited:?}");
        assert_eq!(mpv.get_property::<i64>("video-out-params/h").unwrap(), 136);
    }

    /// Review 1 with a real mpv: the next episode in the same orientation
    /// resumes paused (`pause=yes`) with a plugin's crop. The old and the
    /// new chain give the same output and time does not move, so no frame
    /// shows anything; the transform is the same, so the gate opens after
    /// the timeout without play.
    #[test]
    fn the_same_orientation_paused_with_a_crop_opens_without_play() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("@vestra-owned:lavfi=[crop=w=iw:h=ih/2]");
        assert_eq!(current_transform(&mpv).as_deref(), Some("null"));
        let (gate, mut geometry) = start(&mpv, true);
        let timeout = Duration::from_millis(300);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gate.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the gate never opened");
            geometry
                .advance(&mpv, Phase::Ready, Some(load()), timeout)
                .unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(mpv.get_property::<bool>("pause").unwrap(), "still paused");
        assert_eq!(output(&mpv), (320, 120));
        // The orientation filter as `vf pre` left it reads back as its
        // transform; another one is told apart.
        assert_eq!(current_transform(&mpv).as_deref(), Some("null"));
        mpv.command(&["vf", "add", "@vestra-orientation:lavfi=[transpose=clock]"])
            .unwrap();
        assert_eq!(current_transform(&mpv).as_deref(), Some("transpose=clock"));
    }

    /// Review M1 with a real mpv: the previous (portrait) file's quarter
    /// turn is still the orientation filter and a plugin's crop follows it;
    /// a landscape picture starts paused. Its first frame went through the
    /// old chain (240×160, sideways). Past the timeout the gate opens only
    /// on a frame of the new chain (320×120): at once if mpv refreshed the
    /// paused frame, else once playback goes on.
    #[test]
    fn a_stale_output_of_the_previous_chain_never_opens_the_gate() {
        let _one = one_mpv_at_a_time();
        let mpv = mpv("@vestra-owned:lavfi=[crop=w=iw:h=ih/2]");
        mpv.command(&["vf", "add", "@vestra-orientation:lavfi=[transpose=clock]"])
            .unwrap();
        let (gate, mut geometry) = start(&mpv, true);
        let timeout = Duration::from_millis(300);
        let tick = |geometry: &mut Geometry| {
            geometry
                .advance(&mpv, Phase::Ready, Some(load()), timeout)
                .unwrap();
            std::thread::sleep(Duration::from_millis(2));
            gate.load(Ordering::Acquire)
        };
        // Paused, well past the timeout.
        let paused = Instant::now() + timeout * 5;
        while Instant::now() < paused {
            if tick(&mut geometry) {
                assert_eq!(output(&mpv), (320, 120), "opened on a stale frame");
                return;
            }
        }
        assert!(
            matches!(geometry.prepared, Some((_, Preparation::WaitingOutput(_)))),
            "prepared while paused"
        );
        // Playback goes on: frames of the new chain.
        mpv.set_property("pause", false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !tick(&mut geometry) {
            assert!(Instant::now() < deadline, "the gate never opened");
        }
        assert_eq!(output(&mpv), (320, 120));
    }
}
