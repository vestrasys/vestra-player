//! `plugin.power.preventSleep`: while any plugin asks for `"playing"` and a
//! source is ready and not paused, hold a `caffeinate` assertion bound to the
//! host process: display sleep too when video frames are being shown, system
//! sleep only for audio.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

/// A started assertion; `stop` ends it (the process also exits with the host).
pub trait Assertion {
    fn stop(&mut self);
}

/// Starts `/usr/bin/caffeinate` with the given arguments; injectable for tests.
pub type Launcher = Box<dyn FnMut(&[String]) -> std::io::Result<Box<dyn Assertion>>>;

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
    pid: u32,
    launch: Launcher,
    held: Option<(Vec<String>, Box<dyn Assertion>)>,
    failed: Option<Vec<String>>,
}

impl Power {
    pub fn new(pid: u32, launch: Launcher) -> Self {
        Self {
            requesters: BTreeSet::new(),
            pid,
            launch,
            held: None,
            failed: None,
        }
    }
    /// The real launcher: `/usr/bin/caffeinate <args>`, detached from stdio.
    pub fn system() -> Self {
        Self::new(
            std::process::id(),
            Box::new(|args| {
                let child = std::process::Command::new("/usr/bin/caffeinate")
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;
                Ok(Box::new(Child(child)) as Box<dyn Assertion>)
            }),
        )
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
    /// The `caffeinate` arguments this state needs, `None` for no assertion.
    pub fn wanted(&self, playback: Playback) -> Option<Vec<String>> {
        if self.requesters.is_empty() || !playback.playing {
            return None;
        }
        let mut args = Vec::new();
        if playback.video {
            args.push("-d".to_owned());
        }
        args.extend(["-i".to_owned(), "-w".to_owned(), self.pid.to_string()]);
        Some(args)
    }
    /// Bring the held assertion in line: end the old process before starting
    /// one with different arguments. A failed start is not retried until the
    /// wanted arguments change.
    pub fn update(&mut self, playback: Playback) -> Option<String> {
        let wanted = self.wanted(playback);
        if self.held.as_ref().map(|(args, _)| args) == wanted.as_ref() {
            return None;
        }
        if let Some((_, mut assertion)) = self.held.take() {
            assertion.stop();
        }
        let args = wanted?;
        if self.failed.as_ref() == Some(&args) {
            return None;
        }
        match (self.launch)(&args) {
            Ok(assertion) => {
                self.failed = None;
                self.held = Some((args, assertion));
                None
            }
            Err(error) => {
                self.failed = Some(args);
                Some(format!("caffeinate: {error}"))
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

struct Child(std::process::Child);
impl Assertion for Child {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    fn power() -> (Power, Log) {
        let log: Log = Rc::default();
        let sink = log.clone();
        let power = Power::new(
            42,
            Box::new(move |args| {
                let line = args.join(" ");
                sink.borrow_mut().push(format!("start {line}"));
                Ok(Box::new(Fake(sink.clone(), line)) as Box<dyn Assertion>)
            }),
        );
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
        assert_eq!(power.wanted(VIDEO).unwrap(), ["-d", "-i", "-w", "42"]);
        assert_eq!(power.wanted(AUDIO).unwrap(), ["-i", "-w", "42"]);
        assert_eq!(power.wanted(PAUSED), None);
        power.update(AUDIO);
        power.update(AUDIO);
        power.update(VIDEO);
        power.update(PAUSED);
        assert_eq!(
            *log.borrow(),
            [
                "start -i -w 42",
                "stop -i -w 42",
                "start -d -i -w 42",
                "stop -d -i -w 42"
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
        assert_eq!(log.borrow().last().unwrap(), "stop -d -i -w 42");
        assert!(power.request(3, "always").is_err());
    }

    #[test]
    fn a_failed_start_waits_for_new_arguments_and_drop_ends_the_assertion() {
        let log: Log = Rc::default();
        let sink = log.clone();
        let mut power = Power::new(
            7,
            Box::new(move |args| {
                if args.contains(&"-d".to_owned()) {
                    return Err(std::io::Error::other("denied"));
                }
                sink.borrow_mut().push(args.join(" "));
                Ok(Box::new(Fake(sink.clone(), args.join(" "))) as Box<dyn Assertion>)
            }),
        );
        power.request(1, "playing").unwrap();
        assert!(power.update(VIDEO).is_some());
        assert!(power.update(VIDEO).is_none(), "not retried");
        assert!(power.update(AUDIO).is_none());
        drop(power);
        assert_eq!(*log.borrow(), ["-i -w 7", "stop -i -w 7"]);
    }
}
