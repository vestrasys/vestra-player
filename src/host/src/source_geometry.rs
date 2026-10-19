//! Experimental software-renderer orientation preparation; final API may differ.
//!
//! The initial metadata filter is a renderer safety prerequisite, not a pixel
//! rotation. Publication stays blocked until the physical transform is observed
//! in output parameters. This is a geometry check, not a decoded-frame fence.
#![forbid(unsafe_code)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rsmpv::{Mpv, Node};

use crate::{
    RpcError,
    session::{IntentTicket, Phase},
    source_backend::Sources,
};

pub const INITIAL_FILTERS: &str =
    "@vestra-orientation:lavfi=[null],@vestra-orientation-metadata:format=rotate=0";

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
    WaitingOutput(Plan),
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
        if matches!(sources.phase(), Phase::Empty | Phase::Error) {
            self.prepared = None;
            self.publish_gate.store(true, Ordering::Release);
            return Ok(());
        }
        let Some(load) = sources.presentation_load() else {
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
            if let Preparation::WaitingOutput(expected) = preparation {
                let width = mpv.get_property::<i64>("video-out-params/w");
                let height = mpv.get_property::<i64>("video-out-params/h");
                let rotation = mpv.get_property::<i64>("video-out-params/rotate");
                if matches!((width, height, rotation), (Ok(w), Ok(h), Ok(0)) if w == expected.width && h == expected.height)
                {
                    eprintln!(
                        "source geometry ready entry={} generation={} cycle={} output={}x{} rotate=0",
                        load.0,
                        load.1.stamp.generation,
                        load.1.cycle,
                        expected.width,
                        expected.height
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
                eprintln!(
                    "source geometry ready entry={} generation={} cycle={} no selected video",
                    load.0, load.1.stamp.generation, load.1.cycle
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
        eprintln!(
            "source geometry prepare entry={} generation={} cycle={} decoder={}x{} rotate={} filter={}",
            load.0, load.1.stamp.generation, load.1.cycle, width, height, rotation, filter
        );
        self.prepared = Some((load, Preparation::Failed));
        mpv.command(&["vf", "pre", &filter]).map_err(|error| {
            RpcError::new(
                "source-geometry",
                format!("orientation filter failed: {error}"),
            )
        })?;
        self.prepared = Some((load, Preparation::WaitingOutput(expected)));
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
}
