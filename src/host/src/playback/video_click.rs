//! When a left press on the video becomes `Mouse:LeftClick` (R-H11).
//!
//! A click waits out the system double-click interval only when a double
//! click would do something else and a click repeated by it would not be
//! harmless. Otherwise it fires on release, so a click bound to Play/Pause
//! answers at once (it waited 0.5 s, the Windows default, before):
//!
//! - no `Mouse:LeftDoubleClick` binding for the press: nothing to wait for
//!   (the defaults since host 0.5.2: a click is Play/Pause, a double click
//!   is unbound);
//! - the click's command is checkable (an on/off toggle or a radio item):
//!   the double click's two clicks run it twice, which leaves its state as
//!   it was, as players do when a click pauses and a double click goes full
//!   screen.
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

/// A click moves at most this far (logical px) or it is a drag.
pub const SLOP: f64 = 4.;

#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub at: Instant,
    pub x: f64,
    pub y: f64,
    pub mods: crate::bindings::Mods,
    pub generation: u64,
    /// Where the button went up, once it has.
    pub released: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Wait,
    Fire,
    /// It moved: no click.
    Drop,
}

impl Press {
    pub fn new(at: Instant, x: f64, y: f64, mods: crate::bindings::Mods, generation: u64) -> Self {
        Self {
            at,
            x,
            y,
            mods,
            generation,
            released: None,
        }
    }

    fn near(&self, (x, y): (f64, f64)) -> bool {
        (self.x - x).hypot(self.y - y) <= SLOP
    }

    /// Whether a press at (x, y) now completes a double click with this one.
    pub fn doubled_by(&self, x: f64, y: f64, now: Instant, interval: Duration) -> bool {
        now.duration_since(self.at) < interval && self.near((x, y))
    }

    /// `immediate`: fire on release (see the module notes); otherwise once
    /// the interval has passed without a second press, where the pointer is
    /// then (or was at the release).
    pub fn verdict(
        &self,
        now: Instant,
        pointer: (f64, f64),
        interval: Duration,
        immediate: bool,
    ) -> Verdict {
        let at = match (immediate, self.released) {
            (true, Some(at)) => at,
            (true, None) => return Verdict::Wait,
            (false, _) if now.duration_since(self.at) < interval => return Verdict::Wait,
            (false, released) => released.unwrap_or(pointer),
        };
        if self.near(at) {
            Verdict::Fire
        } else {
            Verdict::Drop
        }
    }
}

/// Whether this `Mouse:LeftClick` need not wait for a double click.
pub fn immediate(
    table: &crate::bindings::Table,
    commands: &crate::commands::Registry,
    click: &crate::bindings::Input,
) -> bool {
    let Some(target) = table.resolve(click, None) else {
        return true;
    };
    let doubled = ["LeftHalf", "RightHalf"].iter().any(|half| {
        let double = crate::bindings::Input {
            mods: click.mods,
            name: format!("Mouse:LeftDoubleClick:{half}"),
        };
        table.resolve(&double, None).is_some()
    });
    !doubled
        || commands
            .get(target.owner, &target.id)
            .is_some_and(|command| command.state.checked.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{Input, Table};
    use crate::commands::{CommandState, Declaration, Registry};

    const HALF: Duration = Duration::from_millis(500);

    fn press(at: Instant) -> Press {
        Press::new(at, 100., 100., crate::bindings::Mods::default(), 1)
    }

    /// When the click fires, `None` for never, ticking every 16 ms like the
    /// host loop, for a press at 0 released at `up` ms.
    fn fires_at(up: u64, immediate: bool) -> Option<u64> {
        let start = Instant::now();
        let mut press = press(start);
        for ms in (0..2000).step_by(16) {
            let now = start + Duration::from_millis(ms);
            if ms >= up {
                press.released = Some((101., 100.));
            }
            match press.verdict(now, (101., 100.), HALF, immediate) {
                Verdict::Wait => {}
                Verdict::Fire => return Some(ms),
                Verdict::Drop => return None,
            }
        }
        None
    }

    #[test]
    fn a_click_that_may_be_repeated_fires_on_release_not_after_the_interval() {
        // A 90 ms click: before, the click fired after the 500 ms interval.
        assert_eq!(fires_at(90, false), Some(512));
        assert_eq!(fires_at(90, true), Some(96));
    }

    #[test]
    fn a_held_press_fires_on_release_and_a_moved_one_never() {
        assert_eq!(fires_at(800, true), Some(800));
        let start = Instant::now();
        let mut moved = press(start);
        moved.released = Some((110., 100.));
        assert_eq!(
            moved.verdict(start, (110., 100.), HALF, true),
            Verdict::Drop
        );
        assert_eq!(
            moved.verdict(start + HALF, (100., 100.), HALF, false),
            Verdict::Drop,
            "the release point counts once there is one"
        );
        let held = press(start);
        assert_eq!(
            held.verdict(start + HALF, (120., 100.), HALF, false),
            Verdict::Drop
        );
        assert_eq!(
            held.verdict(start + HALF, (102., 100.), HALF, false),
            Verdict::Fire
        );
    }

    #[test]
    fn a_second_press_doubles_only_near_and_within_the_interval() {
        let start = Instant::now();
        let first = press(start);
        let soon = start + Duration::from_millis(200);
        assert!(first.doubled_by(103., 100., soon, HALF));
        assert!(!first.doubled_by(110., 100., soon, HALF));
        assert!(!first.doubled_by(100., 100., start + HALF, HALF));
    }

    /// The host's fullscreen on double click, the user's click on a command
    /// that is checkable or not.
    fn bound(checked: Option<bool>, double: &[&str]) -> (Table, Registry) {
        let (mut table, mut commands) = (Table::default(), Registry::new());
        let state = CommandState {
            enabled: true,
            checked,
            busy: false,
            reason: None,
        };
        for (id, inputs) in [("fullscreen", double), ("action", &["Mouse:LeftClick"][..])] {
            let inputs: Vec<String> = inputs.iter().map(|s| s.to_string()).collect();
            table.declare(0, id, "host", &inputs, false, false).unwrap();
            commands
                .register(
                    0,
                    "host",
                    id.into(),
                    1,
                    Declaration::new(id, &[]),
                    state.clone(),
                )
                .unwrap();
        }
        (table, commands)
    }

    /// The host's default bindings (user ruling 2026-10-26): a click on the
    /// video is Play/Pause, nothing is bound to a double click, so the click
    /// fires on release in every playback phase.
    #[test]
    fn by_default_a_click_plays_or_pauses_on_release() {
        let (mut table, mut commands) = (Table::default(), Registry::new());
        // main.rs register_host_commands: fullscreen keeps only its keys.
        let keys: Vec<String> = ["Enter", "Alt+Enter", "F11"].map(String::from).to_vec();
        table
            .declare(0, "fullscreen", "host", &keys, false, false)
            .unwrap();
        let playing = crate::core_actions::Facts {
            phase_ready: true,
            can_pause: true,
            ..Default::default()
        };
        for (id, declaration) in crate::core_actions::declarations(5.) {
            table
                .declare(0, id, "host", &declaration.bindings, false, false)
                .unwrap();
            let state = crate::core_actions::state(id, &playing, false);
            commands
                .register(0, "host", id.into(), 1, declaration, state)
                .unwrap();
        }
        let input = |name: &str| Input {
            mods: crate::bindings::Mods::default(),
            name: name.into(),
        };
        assert!(table.has_left_click());
        let target = table.resolve(&input("Mouse:LeftClick"), None).unwrap();
        assert_eq!((target.owner, target.id.as_str()), (0, "play-pause"));
        for double in [
            "Mouse:LeftDoubleClick:LeftHalf",
            "Mouse:LeftDoubleClick:RightHalf",
        ] {
            assert!(table.resolve(&input(double), None).is_none(), "{double}");
        }
        assert!(immediate(&table, &commands, &input("Mouse:LeftClick")));
        // A 90 ms click fires on release (the next 16 ms tick), not after
        // the double-click interval.
        assert_eq!(fires_at(90, true), Some(96));
        // With nothing open the command is not checkable, and the click is
        // still immediate: nothing waits for a double click.
        let mut commands = Registry::new();
        for (id, declaration) in crate::core_actions::declarations(5.) {
            let state = crate::core_actions::state(id, &Default::default(), false);
            commands
                .register(0, "host", id.into(), 1, declaration, state)
                .unwrap();
        }
        assert!(immediate(&table, &commands, &input("Mouse:LeftClick")));
    }

    #[test]
    fn a_click_waits_only_for_a_double_click_that_would_disturb_it() {
        let click = Input {
            mods: crate::bindings::Mods::default(),
            name: "Mouse:LeftClick".into(),
        };
        let (table, commands) = bound(Some(false), &["Mouse:LeftDoubleClick"]);
        assert!(
            immediate(&table, &commands, &click),
            "a toggle (Play/Pause) fires at once"
        );
        let (table, commands) = bound(None, &["Mouse:LeftDoubleClick:RightHalf"]);
        assert!(
            !immediate(&table, &commands, &click),
            "any half's double click makes others wait"
        );
        let (table, commands) = bound(None, &["Enter"]);
        assert!(
            immediate(&table, &commands, &click),
            "no double click to wait for"
        );
        let shifted = Input {
            mods: crate::bindings::Mods {
                shift: true,
                ..Default::default()
            },
            ..click.clone()
        };
        let (table, commands) = bound(None, &["Shift+Mouse:LeftDoubleClick"]);
        assert!(
            immediate(&table, &commands, &click),
            "a double click with Shift does not hold a plain click"
        );
        assert!(
            immediate(&table, &commands, &shifted),
            "Shift+click is not bound at all"
        );
    }
}
