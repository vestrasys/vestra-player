use super::*;

fn playing(source: u64, position: f64) -> Sample {
    Sample {
        source,
        position: Some(position),
        rate: 1.,
        running: true,
    }
}

/// A model watching `boundaries` of source 1 from `position` at 0 ms: the
/// first read of a watch is a jump (nothing was anchored), then the UI
/// thread hands the boundaries over again.
fn start(position: f64, boundaries: &[f64]) -> PositionModel {
    let mut model = PositionModel::default();
    assert!(model.set_watch(1, boundaries.to_vec(), 0));
    let first = model.sample(0., playing(1, position)).unwrap();
    assert!(!first.continuous);
    assert!(model.set_watch(1, boundaries.to_vec(), first.serial));
    assert_eq!(model.sample(0., playing(1, position)), None);
    model
}

#[test]
fn no_watch_no_report_and_no_deadline() {
    let mut model = PositionModel::default();
    assert!(!model.watching());
    assert_eq!(model.sample(0., playing(1, 5.)), None);
    assert_eq!(model.sample(100., playing(1, 5.1)), None);
    assert_eq!(model.deadline(100.), None);
}

#[test]
fn the_deadline_is_the_predicted_crossing() {
    let mut model = start(5., &[7.5, 20.]);
    assert_eq!(model.deadline(0.), Some(2500.));
    // Twice as fast: half the time.
    let fast = Sample {
        rate: 2.,
        ..playing(1, 6.)
    };
    assert_eq!(model.sample(1000., fast), None);
    assert_eq!(model.deadline(1000.), Some(750.));
    // At the deadline the read is at the boundary: reported once.
    let at = Sample {
        position: Some(7.5),
        ..fast
    };
    let report = model.sample(1750., at).unwrap();
    assert!(report.continuous);
    assert_eq!((report.source, report.position), (1, 7.5));
    // Disarmed until the next boundaries come.
    assert_eq!(model.deadline(1750.), None);
    assert_eq!(model.sample(1800., playing(1, 7.6)), None);
    // A watch computed before that report is stale.
    assert!(!model.set_watch(1, vec![7.5], report.serial - 1));
    assert!(model.set_watch(1, vec![20.], report.serial));
    assert_eq!(model.deadline(1800.), Some(12_400.));
}

#[test]
fn a_short_read_looks_again_for_the_rest_never_at_once() {
    let mut model = start(0., &[1.]);
    assert_eq!(model.deadline(0.), Some(1000.));
    // A frame behind at the deadline: the rest, 40 ms.
    assert_eq!(model.sample(1000., playing(1, 0.96)), None);
    let rest = model.deadline(1000.).unwrap();
    assert!((rest - 40.).abs() < 1e-6, "{rest}");
    // Almost there: still at least `MIN_WAIT_MS`.
    assert_eq!(model.sample(1040., playing(1, 0.999_99)), None);
    assert_eq!(model.deadline(1040.), Some(MIN_WAIT_MS));
    assert!(model.sample(1044., playing(1, 1.004)).unwrap().continuous);
}

#[test]
fn pause_and_a_stall_have_no_deadline() {
    let mut model = start(0., &[10.]);
    let paused = Sample {
        running: false,
        ..playing(1, 0.)
    };
    assert_eq!(model.sample(0., paused), None);
    assert_eq!(model.deadline(0.), None);
    // Running again (a resume is an anchoring read).
    model.sample(500., playing(1, 0.));
    assert_eq!(model.deadline(500.), Some(10_000.));
    // Running but not moving: after `STALL_MS`, a stall.
    model.sample(600., playing(1, 0.));
    assert!(model.deadline(600.).is_some());
    assert_eq!(model.sample(800., playing(1, 0.)), None);
    assert_eq!(model.deadline(800.), None);
    // Moving again: a deadline from there, and not a jump.
    assert_eq!(model.sample(900., playing(1, 0.1)), None);
    assert_eq!(model.deadline(900.), Some(9_900.));
}

#[test]
fn a_seek_or_a_new_source_is_reported_as_a_jump() {
    let mut model = start(0., &[10.]);
    // mpv's `seek`: not running until the read after it.
    model.jumped();
    assert_eq!(model.deadline(10.), None);
    let report = model.sample(50., playing(1, 30.)).unwrap();
    assert!(!report.continuous);
    assert_eq!(report.position, 30.);
    // A jump the thread did not see as one: the read is far from the
    // prediction.
    model.set_watch(1, vec![40.], report.serial);
    assert_eq!(model.sample(60., playing(1, 30.01)), None);
    let back = model.sample(160., playing(1, 2.)).unwrap();
    assert!(!back.continuous);
    // A new source: its watch comes later; the first report then is a jump.
    model.set_watch(1, vec![10.], back.serial);
    assert_eq!(model.sample(200., playing(2, 0.)), None);
    model.set_watch(2, vec![5.], back.serial);
    assert!(!model.sample(210., playing(2, 0.01)).unwrap().continuous);
    // The watch ends.
    model.set_watch(0, Vec::new(), 0);
    assert!(!model.watching());
    assert_eq!(model.deadline(300.), None);
}

#[test]
fn a_boundary_already_behind_is_reported_at_the_first_read() {
    let mut model = start(12., &[20.]);
    // Computed from an older position (a source.state 100 ms old).
    assert!(model.set_watch(1, vec![11.9], 1));
    assert_eq!(model.deadline(0.), Some(0.));
    assert!(model.sample(1., playing(1, 12.001)).unwrap().continuous);
}
