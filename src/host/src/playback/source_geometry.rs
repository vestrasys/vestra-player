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
//! ([`Evidence::Aspect`]); otherwise, after [`TIMEOUT`] with some output,
//! the gate opens without evidence ([`Evidence::Timeout`], a diagnostic
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
/// prepared orientation after `waited`. Only rotate 0 counts (the metadata
/// filter). A frame turned the other way (still filtered by the previous
/// file's orientation until `vf pre` replaces it) has the other aspect, so
/// it never passes as `Aspect`; a picture within a few pixels of square has
/// both aspects and passes only as `Exact` or by the timeout.
pub fn evidence(
    expected: (i64, i64),
    output: (i64, i64, i64),
    waited: Duration,
) -> Option<Evidence> {
    let (w, h, rotate) = output;
    if rotate != 0 || w <= 0 || h <= 0 {
        return None;
    }
    if (w, h) == expected {
        return Some(Evidence::Exact);
    }
    let swapped = (expected.1, expected.0);
    if same_aspect((w, h), expected) && !same_aspect((w, h), swapped) {
        return Some(Evidence::Aspect);
    }
    (waited >= TIMEOUT).then_some(Evidence::Timeout)
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
    /// The orientation filter is in the chain since then.
    WaitingOutput(Plan, Instant),
    Confirmed,
    Failed,
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
        if let Some((_, preparation)) = &self.prepared {
            if let Preparation::WaitingOutput(expected, since) = preparation {
                let (Ok(width), Ok(height), Ok(rotation)) = (
                    mpv.get_property::<i64>("video-out-params/w"),
                    mpv.get_property::<i64>("video-out-params/h"),
                    mpv.get_property::<i64>("video-out-params/rotate"),
                ) else {
                    return Ok(());
                };
                let waited = since.elapsed();
                // `evidence` counts against TIMEOUT; a shorter `timeout`
                // (tests) counts as if it had waited the difference.
                let counted = waited + TIMEOUT.saturating_sub(timeout);
                if let Some(evidence) = evidence(
                    (expected.width, expected.height),
                    (width, height, rotation),
                    counted,
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
            "@vestra-orientation:lavfi=[{},sidedata=mode=delete:type=DISPLAYMATRIX]",
            expected.transform
        );
        diag!(
            "source geometry prepare entry={} generation={} cycle={} decoder={}x{} rotate={} filter={}",
            load.0,
            load.1.stamp.generation,
            load.1.cycle,
            width,
            height,
            rotation,
            filter
        );
        self.prepared = Some((load, Preparation::Failed));
        mpv.command(&["vf", "pre", &filter]).map_err(|error| {
            RpcError::new(
                "source-geometry",
                format!("orientation filter failed: {error}"),
            )
        })?;
        self.prepared = Some((load, Preparation::WaitingOutput(expected, Instant::now())));
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

    /// Loads a generated 320×240 picture and ticks the geometry (as the
    /// control worker does, every 2 ms) until the gate opens; returns how
    /// long after the orientation filter went in.
    fn opens(mpv: &Mpv, timeout: Duration) -> Duration {
        let gate = Arc::new(AtomicBool::new(true));
        let mut geometry = Geometry::new(gate.clone());
        geometry.begin();
        mpv.command(&["loadfile", "av://lavfi:testsrc=size=320x240:rate=25"])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        // The presentation load exists from FileLoaded on (before it the
        // track list is empty, which would read as audio only).
        while !matches!(mpv.poll_event(), Some(rsmpv::Event::FileLoaded)) {
            assert!(Instant::now() < deadline, "not loaded");
            std::thread::sleep(Duration::from_millis(1));
        }
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
}
