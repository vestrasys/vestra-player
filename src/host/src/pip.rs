//! `window.setPictureInPicture` (R-W5) and the picture-in-picture window
//! (R-H8): who turned it on, the reply that waits for the window to open or
//! close, frames while it is open, and what its buttons ask of playback.
//! No OS calls here: the system window is `platform::PipWindow` (macOS:
//! AVKit through `vestra-pip`; other systems: `unsupported`) behind the
//! [`Surface`] trait, so the rules are tested without one.
#![forbid(unsafe_code)]

use crate::platform::{HostWindowId, PipEvent, Rect};
use crate::{Reply, RpcError};
use serde_json::Value;

/// How long a start or stop may take before the reply gives up (ms).
pub const TIMEOUT_MS: u64 = 5000;

/// The system PiP window, as the controller uses it.
pub trait Surface {
    /// A frame the GPU renderer drew (`platform::GpuFrame`).
    type Frame: ?Sized;
    /// Keep the source layer on `area` of `window`'s content (logical
    /// points, top-left origin). `false`: not placed (no such window, or an
    /// empty area).
    fn place(&self, window: HostWindowId, area: Rect) -> bool;
    /// One frame of `height` rows of `width` RGB24 pixels.
    fn push_rgb(&self, width: usize, height: usize, rgb: &[u8]) -> Result<(), String>;
    /// One frame the GPU renderer drew, shown without a copy.
    fn push_surface(&self, frame: &Self::Frame) -> Result<(), String>;
    /// Ask the system to open the window; `false` when it cannot yet.
    fn start(&self) -> bool;
    fn stop(&self);
    fn set_playback(&self, playback: &Playback);
    fn events(&self) -> Vec<PipEvent>;
    /// One line of system geometry for `VESTRA_PIP_LOG=1`.
    fn describe(&self, _window: HostWindowId) -> String {
        String::new()
    }
}

/// Playback as the PiP window's buttons and scrubber show it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub paused: bool,
    pub position: Option<f64>,
    /// The seekable length; `None` for live or unseekable sources.
    pub duration: Option<f64>,
    pub rate: f64,
}

/// What the player knows when a request or tick comes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// A native main window exists (not headless).
    pub window: bool,
    /// This system offers PiP.
    pub supported: bool,
    pub fullscreen: bool,
    /// A video frame is showing (not audio-only, not empty).
    pub video: bool,
}

/// What the player has to do after a tick.
pub enum Effect {
    Respond(Reply, Result<Value, RpcError>),
    /// Run this core action, with this seek step in seconds (`None`: the
    /// user's step).
    Action(&'static str, Option<f64>),
}

struct Pending {
    reply: Reply,
    on: bool,
    deadline: u64,
}

pub struct PictureInPicture<S> {
    surface: Option<S>,
    active: bool,
    /// The plugin whose `setPictureInPicture(true)` is in effect.
    owner: Option<u64>,
    pending: Option<Pending>,
    /// The player asked the window to close (not the user in the window).
    stopping: bool,
    /// `start` was asked since the surface was made.
    asked: bool,
    /// A frame went to the surface since it was made.
    fed: bool,
    /// The number of the last GPU frame pushed (each goes once).
    gpu_frame: Option<u64>,
}

impl<S: Surface> Default for PictureInPicture<S> {
    fn default() -> Self {
        Self {
            surface: None,
            active: false,
            owner: None,
            pending: None,
            stopping: false,
            asked: false,
            fed: false,
            gpu_frame: None,
        }
    }
}

impl<S: Surface> PictureInPicture<S> {
    /// The PiP window is showing.
    pub fn active(&self) -> bool {
        self.active
    }
    /// Frames should go to the surface (it exists: starting, open or closing).
    pub fn feeding(&self) -> bool {
        self.surface.is_some()
    }
    /// Starting but no frame yet: the player should push the frame on
    /// screen (paused playback renders none).
    pub fn needs_frame(&self) -> bool {
        self.surface.is_some() && !self.fed
    }
    pub fn push(&mut self, width: usize, height: usize, rgb: &[u8]) {
        if let Some(surface) = &self.surface {
            match surface.push_rgb(width, height, rgb) {
                Ok(()) => self.fed = true,
                Err(error) => eprintln!("pip: {error}"),
            }
        }
    }
    /// The surface's geometry line (`VESTRA_PIP_LOG=1`); `None` without one.
    pub fn describe(&self, window: HostWindowId) -> Option<String> {
        self.surface.as_ref().map(|s| s.describe(window))
    }
    /// The GPU renderer's frame number `sequence` (render thread's frame
    /// callback, or the newest frame when starting paused). A frame already
    /// pushed is skipped.
    pub fn push_surface(&mut self, sequence: u64, frame: &S::Frame) {
        let Some(surface) = &self.surface else {
            return;
        };
        if self.gpu_frame == Some(sequence) {
            return;
        }
        match surface.push_surface(frame) {
            Ok(()) => (self.fed, self.gpu_frame) = (true, Some(sequence)),
            Err(error) => eprintln!("pip: {error}"),
        }
    }
    /// `window.setPictureInPicture { on }`. `Ok(Some)`: answered now;
    /// `Ok(None)`: answered by a later [`Effect::Respond`]. A start from
    /// fullscreen waits until the player has left it (the caller leaves).
    pub fn request(
        &mut self,
        reply: Reply,
        on: bool,
        facts: &Facts,
        now: u64,
        create: impl FnOnce() -> Result<S, RpcError>,
    ) -> Result<Option<Value>, RpcError> {
        if self.pending.is_some() {
            return Err(RpcError::new("busy", "画中画正在切换"));
        }
        if !on {
            let Some(surface) = &self.surface else {
                return Ok(Some(Value::Null));
            };
            self.stopping = true;
            surface.stop();
            self.pending = Some(Pending {
                reply,
                on,
                deadline: now + TIMEOUT_MS,
            });
            return Ok(None);
        }
        if self.active {
            self.owner = Some(reply.owner);
            return Ok(Some(Value::Null));
        }
        if !facts.window {
            return Err(RpcError::new("unavailable", "no native window"));
        }
        if !facts.supported {
            return Err(RpcError::new("unsupported", "此系统不支持画中画"));
        }
        if !facts.video {
            return Err(RpcError::new("unavailable", "没有视频"));
        }
        self.surface = Some(create()?);
        self.owner = Some(reply.owner);
        (self.stopping, self.asked, self.fed) = (false, false, false);
        self.gpu_frame = None;
        self.pending = Some(Pending {
            reply,
            on,
            deadline: now + TIMEOUT_MS,
        });
        Ok(None)
    }
    /// A plugin stopped: if its request is in effect, the window closes.
    pub fn remove_owner(&mut self, owner: u64) {
        if self.owner != Some(owner) {
            return;
        }
        self.owner = None;
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.reply.owner == owner)
        {
            self.pending = None;
        }
        if self.active {
            self.stopping = true;
            if let Some(surface) = &self.surface {
                surface.stop();
            }
        } else {
            self.close();
        }
    }
    /// Once per UI tick: place the source layer on the video area, report
    /// playback, start when ready, and turn the window's events into effects.
    pub fn tick(
        &mut self,
        facts: &Facts,
        place: Option<(HostWindowId, Rect)>,
        playback: &Playback,
        now: u64,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        let Some(surface) = &self.surface else {
            return effects;
        };
        let placed = place.is_some_and(|(window, area)| surface.place(window, area));
        surface.set_playback(playback);
        for event in surface.events() {
            match event {
                PipEvent::Pause(pause) if pause != playback.paused => {
                    effects.push(Effect::Action("play-pause", None));
                }
                PipEvent::Pause(_) => {}
                PipEvent::Skip(seconds) if seconds.is_finite() && seconds != 0. => {
                    let action = if seconds < 0. {
                        "seek-backward"
                    } else {
                        "seek-forward"
                    };
                    effects.push(Effect::Action(action, Some(seconds.abs())));
                }
                PipEvent::Skip(_) => {}
                PipEvent::Started => {
                    self.active = true;
                    if let Some(pending) = self.pending.take_if(|p| p.on) {
                        effects.push(Effect::Respond(pending.reply, Ok(Value::Null)));
                    }
                }
                PipEvent::Stopped { restored } => {
                    // The window's own close button pauses (Safari, QuickTime);
                    // its "back" button and the player's own stop do not.
                    if !self.stopping && !restored && !playback.paused {
                        effects.push(Effect::Action("play-pause", None));
                    }
                    effects.extend(self.answer(Ok(Value::Null), "画中画窗口已关闭"));
                    self.close();
                    return effects;
                }
                PipEvent::Failed(reason) => {
                    let failed = RpcError::new("failed", format!("无法进入画中画：{reason}"));
                    effects.extend(self.answer(Err(failed), ""));
                    self.close();
                    return effects;
                }
            }
        }
        let starting = self.pending.as_ref().is_some_and(|p| p.on) && !self.active;
        if starting && !facts.video {
            let gone = RpcError::new("unavailable", "没有视频");
            effects.extend(self.answer(Err(gone), ""));
            self.close();
            return effects;
        }
        if starting && !self.asked && !facts.fullscreen && placed && self.fed {
            self.asked = surface.start();
        }
        if self.active && !facts.video && !self.stopping {
            // The new source has no video (or the source closed).
            self.stopping = true;
            surface.stop();
        }
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            let late = RpcError::new("failed", "画中画没有启动");
            effects.extend(self.answer(Err(late), ""));
            if !self.active {
                self.close();
            }
        }
        effects
    }
    /// Answer the pending reply: a start gets `start` (an error, or `ok`
    /// replaced by `failed: closed` when the window closed instead), a stop
    /// gets `Ok`.
    fn answer(&mut self, start: Result<Value, RpcError>, closed: &str) -> Option<Effect> {
        let pending = self.pending.take()?;
        let result = match (pending.on, start) {
            (false, _) => Ok(Value::Null),
            (true, Ok(_)) if !closed.is_empty() => Err(RpcError::new("failed", closed)),
            (true, result) => result,
        };
        Some(Effect::Respond(pending.reply, result))
    }
    fn close(&mut self) {
        self.surface = None;
        self.active = false;
        self.owner = None;
        (self.stopping, self.asked, self.fed) = (false, false, false);
        self.gpu_frame = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Log {
        events: Vec<PipEvent>,
        possible: bool,
        started: u32,
        stopped: u32,
        frames: u32,
        /// GPU frames pushed, by their `u32` stand-in.
        surfaces: Vec<u32>,
        placed: Option<(HostWindowId, Rect)>,
        playback: Option<Playback>,
    }
    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<Log>>);
    impl Surface for Fake {
        type Frame = u32;
        fn place(&self, window: HostWindowId, area: Rect) -> bool {
            self.0.borrow_mut().placed = Some((window, area));
            area.width >= 1. && area.height >= 1.
        }
        fn push_rgb(&self, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
            assert_eq!(rgb.len(), width * height * 3);
            self.0.borrow_mut().frames += 1;
            Ok(())
        }
        fn push_surface(&self, frame: &u32) -> Result<(), String> {
            self.0.borrow_mut().surfaces.push(*frame);
            Ok(())
        }
        fn start(&self) -> bool {
            let mut log = self.0.borrow_mut();
            log.started += u32::from(log.possible);
            log.possible
        }
        fn stop(&self) {
            self.0.borrow_mut().stopped += 1;
        }
        fn set_playback(&self, playback: &Playback) {
            self.0.borrow_mut().playback = Some(*playback);
        }
        fn events(&self) -> Vec<PipEvent> {
            std::mem::take(&mut self.0.borrow_mut().events)
        }
    }
    impl Fake {
        fn emit(&self, event: PipEvent) {
            self.0.borrow_mut().events.push(event);
        }
    }

    const FACTS: Facts = Facts {
        window: true,
        supported: true,
        fullscreen: false,
        video: true,
    };
    const PLAYING: Playback = Playback {
        paused: false,
        position: Some(3.),
        duration: Some(60.),
        rate: 1.,
    };
    const AREA: Rect = Rect {
        x: 0.,
        y: 0.,
        width: 640.,
        height: 360.,
    };
    fn place() -> Option<(HostWindowId, Rect)> {
        Some((HostWindowId(7), AREA))
    }
    fn reply(owner: u64) -> Reply {
        Reply {
            owner,
            id: owner * 10,
        }
    }
    fn responses(effects: &[Effect]) -> Vec<(u64, Result<(), String>)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Respond(reply, result) => Some((
                    reply.owner,
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|e| format!("{}: {}", e.code, e.message)),
                )),
                Effect::Action(..) => None,
            })
            .collect()
    }
    fn actions(effects: &[Effect]) -> Vec<(&'static str, Option<f64>)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Action(action, step) => Some((*action, *step)),
                Effect::Respond(..) => None,
            })
            .collect()
    }
    /// A controller whose window opened for plugin 1.
    fn started(fake: &Fake) -> PictureInPicture<Fake> {
        let mut pip = PictureInPicture::default();
        let made = fake.clone();
        assert!(
            pip.request(reply(1), true, &FACTS, 0, move || Ok(made))
                .unwrap()
                .is_none()
        );
        pip.push(2, 1, &[0; 6]);
        fake.0.borrow_mut().possible = true;
        assert!(pip.tick(&FACTS, place(), &PLAYING, 16).is_empty());
        fake.emit(PipEvent::Started);
        assert_eq!(
            responses(&pip.tick(&FACTS, place(), &PLAYING, 32)),
            [(1, Ok(()))]
        );
        assert!(pip.active());
        pip
    }

    #[test]
    fn requests_are_checked_before_anything_is_made() {
        let mut pip = PictureInPicture::<Fake>::default();
        let never = || -> Result<Fake, RpcError> { panic!("must not create") };
        let code = |r: Result<Option<Value>, RpcError>| r.map(|_| ()).map_err(|e| e.code);
        let no_window = Facts {
            window: false,
            ..FACTS
        };
        assert_eq!(
            code(pip.request(reply(1), true, &no_window, 0, never)),
            Err("unavailable".into())
        );
        let unsupported = Facts {
            supported: false,
            ..FACTS
        };
        assert_eq!(
            code(pip.request(reply(1), true, &unsupported, 0, never)),
            Err("unsupported".into())
        );
        let audio = Facts {
            video: false,
            ..FACTS
        };
        assert_eq!(
            code(pip.request(reply(1), true, &audio, 0, never)),
            Err("unavailable".into())
        );
        // Turning off what is not on is answered at once, on every system.
        assert_eq!(
            pip.request(reply(1), false, &unsupported, 0, never)
                .unwrap(),
            Some(Value::Null)
        );
        let failing = || Err(RpcError::new("unsupported", "no AVKit"));
        assert_eq!(
            code(pip.request(reply(1), true, &FACTS, 0, failing)),
            Err("unsupported".into())
        );
        assert!(!pip.feeding());
    }

    #[test]
    fn a_start_waits_for_a_frame_a_placed_layer_and_the_system() {
        let fake = Fake::default();
        let mut pip = PictureInPicture::default();
        let made = fake.clone();
        assert!(
            pip.request(reply(1), true, &FACTS, 0, move || Ok(made))
                .unwrap()
                .is_none()
        );
        // A second request while it switches is busy.
        let busy = pip.request(reply(2), false, &FACTS, 0, || Ok(Fake::default()));
        assert_eq!(busy.unwrap_err().code, "busy");
        assert!(pip.needs_frame());
        fake.0.borrow_mut().possible = true;
        // No frame yet; then a frame but no placement; then fullscreen.
        assert!(pip.tick(&FACTS, place(), &PLAYING, 16).is_empty());
        pip.push(2, 1, &[0; 6]);
        assert!(!pip.needs_frame());
        assert!(pip.tick(&FACTS, None, &PLAYING, 32).is_empty());
        let fullscreen = Facts {
            fullscreen: true,
            ..FACTS
        };
        assert!(pip.tick(&fullscreen, place(), &PLAYING, 48).is_empty());
        assert_eq!(fake.0.borrow().started, 0);
        assert!(pip.tick(&FACTS, place(), &PLAYING, 64).is_empty());
        assert!(pip.tick(&FACTS, place(), &PLAYING, 80).is_empty());
        assert_eq!(fake.0.borrow().started, 1, "asked once");
        assert_eq!(fake.0.borrow().placed, place());
        assert_eq!(fake.0.borrow().playback, Some(PLAYING));
        fake.emit(PipEvent::Started);
        assert_eq!(
            responses(&pip.tick(&FACTS, place(), &PLAYING, 96)),
            [(1, Ok(()))]
        );
        // Already on: answered at once, and the new caller owns it.
        let again = pip.request(reply(2), true, &FACTS, 100, || panic!("not again"));
        assert_eq!(again.unwrap(), Some(Value::Null));
        pip.remove_owner(1);
        assert_eq!(fake.0.borrow().stopped, 0);
        pip.remove_owner(2);
        assert_eq!(fake.0.borrow().stopped, 1);
    }

    #[test]
    fn gpu_frames_feed_the_window_once_each_and_only_while_it_exists() {
        let fake = Fake::default();
        let mut pip = PictureInPicture::default();
        pip.push_surface(1, &10);
        let made = fake.clone();
        pip.request(reply(1), true, &FACTS, 0, move || Ok(made))
            .unwrap();
        assert!(pip.needs_frame());
        // The newest frame when starting, then the frame callback's (the
        // same frame twice goes once).
        pip.push_surface(4, &40);
        pip.push_surface(4, &40);
        assert!(!pip.needs_frame());
        fake.0.borrow_mut().possible = true;
        assert!(pip.tick(&FACTS, place(), &PLAYING, 16).is_empty());
        assert_eq!(fake.0.borrow().started, 1, "a GPU frame counts as fed");
        pip.push_surface(5, &50);
        assert_eq!(fake.0.borrow().surfaces, [40, 50]);
        assert_eq!(fake.0.borrow().frames, 0, "no CPU copy");
        fake.emit(PipEvent::Started);
        pip.tick(&FACTS, place(), &PLAYING, 32);
        fake.emit(PipEvent::Stopped { restored: true });
        pip.tick(&FACTS, place(), &PLAYING, 48);
        assert!(!pip.feeding());
        pip.push_surface(6, &60);
        assert_eq!(fake.0.borrow().surfaces, [40, 50], "closed: nothing goes");
        // A new window takes the same frame number again.
        let made = fake.clone();
        pip.request(reply(1), true, &FACTS, 100, move || Ok(made))
            .unwrap();
        pip.push_surface(5, &50);
        assert_eq!(fake.0.borrow().surfaces, [40, 50, 50]);
    }

    #[test]
    fn buttons_become_core_actions() {
        let fake = Fake::default();
        let mut pip = started(&fake);
        fake.emit(PipEvent::Pause(true));
        fake.emit(PipEvent::Pause(false)); // already playing: nothing
        fake.emit(PipEvent::Skip(15.));
        fake.emit(PipEvent::Skip(-15.));
        fake.emit(PipEvent::Skip(f64::NAN));
        let effects = pip.tick(&FACTS, place(), &PLAYING, 48);
        assert_eq!(
            actions(&effects),
            [
                ("play-pause", None),
                ("seek-forward", Some(15.)),
                ("seek-backward", Some(15.))
            ]
        );
    }

    #[test]
    fn closing_in_the_window_pauses_but_back_and_stop_do_not() {
        // The close button: no restore, not asked by the player.
        let fake = Fake::default();
        let mut pip = started(&fake);
        fake.emit(PipEvent::Stopped { restored: false });
        assert_eq!(
            actions(&pip.tick(&FACTS, place(), &PLAYING, 48)),
            [("play-pause", None)]
        );
        assert!(!pip.active() && !pip.feeding());
        // Paused already: nothing to pause.
        let fake = Fake::default();
        let mut pip = started(&fake);
        fake.emit(PipEvent::Stopped { restored: false });
        let paused = Playback {
            paused: true,
            ..PLAYING
        };
        assert!(pip.tick(&FACTS, place(), &paused, 48).is_empty());
        // The back button.
        let fake = Fake::default();
        let mut pip = started(&fake);
        fake.emit(PipEvent::Stopped { restored: true });
        assert!(pip.tick(&FACTS, place(), &PLAYING, 48).is_empty());
        // The player's own stop, answered when the window has closed.
        let fake = Fake::default();
        let mut pip = started(&fake);
        assert!(
            pip.request(reply(3), false, &FACTS, 50, || panic!())
                .unwrap()
                .is_none()
        );
        assert_eq!(fake.0.borrow().stopped, 1);
        assert!(pip.tick(&FACTS, place(), &PLAYING, 60).is_empty());
        fake.emit(PipEvent::Stopped { restored: false });
        let effects = pip.tick(&FACTS, place(), &PLAYING, 70);
        assert_eq!(responses(&effects), [(3, Ok(()))]);
        assert!(actions(&effects).is_empty());
    }

    #[test]
    fn failures_time_outs_and_lost_video_end_it() {
        let fake = Fake::default();
        let mut pip = PictureInPicture::default();
        let made = fake.clone();
        pip.request(reply(1), true, &FACTS, 0, move || Ok(made))
            .unwrap();
        fake.emit(PipEvent::Failed("not possible".into()));
        let effects = pip.tick(&FACTS, place(), &PLAYING, 16);
        assert_eq!(
            responses(&effects),
            [(1, Err("failed: 无法进入画中画：not possible".into()))]
        );
        assert!(!pip.feeding());
        // Never possible: the start times out.
        let mut pip = PictureInPicture::default();
        pip.request(reply(1), true, &FACTS, 0, || Ok(Fake::default()))
            .unwrap();
        assert!(
            pip.tick(&FACTS, place(), &PLAYING, TIMEOUT_MS - 1)
                .is_empty()
        );
        let effects = pip.tick(&FACTS, place(), &PLAYING, TIMEOUT_MS);
        assert_eq!(
            responses(&effects),
            [(1, Err("failed: 画中画没有启动".into()))]
        );
        assert!(!pip.feeding());
        // The video goes away while starting, or while open.
        let mut pip = PictureInPicture::default();
        pip.request(reply(1), true, &FACTS, 0, || Ok(Fake::default()))
            .unwrap();
        let audio = Facts {
            video: false,
            ..FACTS
        };
        let effects = pip.tick(&audio, place(), &PLAYING, 16);
        assert_eq!(
            responses(&effects),
            [(1, Err("unavailable: 没有视频".into()))]
        );
        let fake = Fake::default();
        let mut pip = started(&fake);
        assert!(pip.tick(&audio, place(), &PLAYING, 48).is_empty());
        assert!(pip.tick(&audio, place(), &PLAYING, 64).is_empty());
        assert_eq!(fake.0.borrow().stopped, 1, "stopped once");
        fake.emit(PipEvent::Stopped { restored: true });
        assert!(pip.tick(&audio, place(), &PLAYING, 80).is_empty());
        assert!(!pip.active());
    }

    #[test]
    fn a_stopped_owner_takes_its_window_with_it() {
        // Still starting: dropped at once, the reply goes nowhere.
        let mut pip = PictureInPicture::default();
        pip.request(reply(1), true, &FACTS, 0, || Ok(Fake::default()))
            .unwrap();
        pip.remove_owner(1);
        assert!(!pip.feeding());
        assert!(pip.tick(&FACTS, place(), &PLAYING, 16).is_empty());
        // Open: closed through the system, without pausing.
        let fake = Fake::default();
        let mut pip = started(&fake);
        pip.remove_owner(1);
        assert_eq!(fake.0.borrow().stopped, 1);
        fake.emit(PipEvent::Stopped { restored: true });
        assert!(pip.tick(&FACTS, place(), &PLAYING, 48).is_empty());
        assert!(!pip.active());
    }
}
