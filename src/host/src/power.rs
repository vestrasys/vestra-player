//! `plugin.power.preventSleep`: while any plugin asks for `"playing"` and a
//! source is ready and not paused, hold a system sleep assertion: display
//! sleep too when video frames are being shown, system sleep only for audio.
//! How the assertion is held is the platform's (`platform::sleep_launcher`:
//! `caffeinate` bound to the host process on macOS, the thread execution
//! state on Windows).
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// A started assertion; `stop` ends it (it also ends with the host process).
pub trait Assertion {
    fn stop(&mut self);
}

/// What to keep awake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    /// The display as well as the system.
    pub display: bool,
}

/// Starts an assertion; injectable for tests.
pub type Launcher = Box<dyn FnMut(Hold) -> std::io::Result<Box<dyn Assertion>>>;

/// What the host currently plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playback {
    /// Source phase is `ready` and not paused (media or generated stream).
    pub playing: bool,
    /// Video frames were shown recently.
    pub video: bool,
}

pub struct Power {
    requesters: BTreeSet<u64>,
    launch: Launcher,
    held: Option<(Hold, Box<dyn Assertion>)>,
    failed: Option<Hold>,
}

impl Power {
    pub fn new(launch: Launcher) -> Self {
        Self {
            requesters: BTreeSet::new(),
            launch,
            held: None,
            failed: None,
        }
    }
    /// The real launcher of this system.
    pub fn system() -> Self {
        Self::new(crate::platform::sleep_launcher())
    }
    /// `mode` is `"off"` or `"playing"`; anything else is refused.
    pub fn request(&mut self, owner: u64, mode: &str) -> Result<(), &'static str> {
        match mode {
            "playing" => self.requesters.insert(owner),
            "off" => self.requesters.remove(&owner),
            _ => return Err("mode must be \"off\" or \"playing\""),
        };
        Ok(())
    }
    /// `owner` keeps the display awake.
    pub fn holds(&self, owner: u64) -> bool {
        self.requesters.contains(&owner)
    }
    /// A stopped plugin's request goes with it.
    pub fn remove_owner(&mut self, owner: u64) {
        self.requesters.remove(&owner);
    }
    /// The merged mode, as the test host reports it.
    #[cfg(test)]
    pub fn effective(&self) -> &'static str {
        if self.requesters.is_empty() {
            "off"
        } else {
            "playing"
        }
    }
    /// The assertion this state needs, `None` for none.
    pub fn wanted(&self, playback: Playback) -> Option<Hold> {
        (!self.requesters.is_empty() && playback.playing).then_some(Hold {
            display: playback.video,
        })
    }
    /// Bring the held assertion in line: end the old one before starting a
    /// different one. A failed start is not retried until the wanted
    /// assertion changes.
    pub fn update(&mut self, playback: Playback) -> Option<String> {
        let wanted = self.wanted(playback);
        if self.held.as_ref().map(|(hold, _)| *hold) == wanted {
            return None;
        }
        if let Some((_, mut assertion)) = self.held.take() {
            assertion.stop();
        }
        let hold = wanted?;
        if self.failed == Some(hold) {
            return None;
        }
        match (self.launch)(hold) {
            Ok(assertion) => {
                self.failed = None;
                self.held = Some((hold, assertion));
                None
            }
            Err(error) => {
                self.failed = Some(hold);
                Some(error.to_string())
            }
        }
    }
}
impl Drop for Power {
    fn drop(&mut self) {
        if let Some((_, mut assertion)) = self.held.take() {
            assertion.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    type Log = Rc<RefCell<Vec<String>>>;
    struct Fake(Log, String);
    impl Assertion for Fake {
        fn stop(&mut self) {
            self.0.borrow_mut().push(format!("stop {}", self.1));
        }
    }
    fn name(hold: Hold) -> String {
        if hold.display { "display" } else { "system" }.into()
    }
    fn power() -> (Power, Log) {
        let log: Log = Rc::default();
        let sink = log.clone();
        let power = Power::new(Box::new(move |hold| {
            sink.borrow_mut().push(format!("start {}", name(hold)));
            Ok(Box::new(Fake(sink.clone(), name(hold))) as Box<dyn Assertion>)
        }));
        (power, log)
    }
    const VIDEO: Playback = Playback {
        playing: true,
        video: true,
    };
    const AUDIO: Playback = Playback {
        playing: true,
        video: false,
    };
    const PAUSED: Playback = Playback {
        playing: false,
        video: true,
    };

    #[test]
    fn requesters_times_playback_times_video_choose_the_assertion() {
        let (mut power, log) = power();
        assert_eq!(power.wanted(VIDEO), None, "nobody asked");
        power.request(1, "playing").unwrap();
        power.request(2, "playing").unwrap();
        assert_eq!(power.effective(), "playing");
        assert_eq!(power.wanted(VIDEO), Some(Hold { display: true }));
        assert_eq!(power.wanted(AUDIO), Some(Hold { display: false }));
        assert_eq!(power.wanted(PAUSED), None);
        power.update(AUDIO);
        power.update(AUDIO);
        power.update(VIDEO);
        power.update(PAUSED);
        assert_eq!(
            *log.borrow(),
            [
                "start system",
                "stop system",
                "start display",
                "stop display"
            ]
        );
    }

    #[test]
    fn any_requester_holds_it_and_a_stopped_plugin_releases_only_its_own() {
        let (mut power, log) = power();
        power.request(1, "playing").unwrap();
        power.request(2, "playing").unwrap();
        power.update(VIDEO);
        power.remove_owner(1);
        power.update(VIDEO);
        assert_eq!(log.borrow().len(), 1, "owner 2 still wants it");
        power.request(2, "off").unwrap();
        assert_eq!(power.effective(), "off");
        power.update(VIDEO);
        assert_eq!(log.borrow().last().unwrap(), "stop display");
        assert!(power.request(3, "always").is_err());
    }

    #[test]
    fn a_failed_start_waits_for_a_different_assertion_and_drop_ends_it() {
        let log: Log = Rc::default();
        let sink = log.clone();
        let mut power = Power::new(Box::new(move |hold| {
            if hold.display {
                return Err(std::io::Error::other("denied"));
            }
            sink.borrow_mut().push(name(hold));
            Ok(Box::new(Fake(sink.clone(), name(hold))) as Box<dyn Assertion>)
        }));
        power.request(1, "playing").unwrap();
        assert_eq!(power.update(VIDEO).as_deref(), Some("denied"));
        assert!(power.update(VIDEO).is_none(), "not retried");
        assert!(power.update(AUDIO).is_none());
        drop(power);
        assert_eq!(*log.borrow(), ["system", "stop system"]);
    }
}
