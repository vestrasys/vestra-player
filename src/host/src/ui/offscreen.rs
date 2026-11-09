//! A Slint platform with no window on screen, for tests that run the whole
//! windowed host (`VESTRA_TEST_OFFSCREEN=1`, tests/startup_compiles.rs):
//! each window is a software-rendered one in memory, never drawn, and the
//! event loop runs Slint's timers and `invoke_from_event_loop` work. The
//! host's own code paths (the shell, its mounts and recompiles, the 16 ms
//! tick) run as in the player; what needs a native window (title bars,
//! cloaking, the frame pacing hook, GPU textures) finds none and is skipped,
//! as it already is for a window that has no native one yet.
#![forbid(unsafe_code)]

use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{EventLoopError, PhysicalSize, PlatformError};

/// Whether this run asked for it (tests only).
pub fn requested() -> bool {
    std::env::var_os("VESTRA_TEST_OFFSCREEN").is_some_and(|v| v == "1")
}

enum Work {
    Run(Box<dyn FnOnce() + Send>),
    Quit,
}

struct Proxy(Mutex<mpsc::Sender<Work>>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(Work::Quit)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(Work::Run(event))
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}

struct Offscreen {
    sender: mpsc::Sender<Work>,
    receiver: Mutex<mpsc::Receiver<Work>>,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        window.set_size(PhysicalSize::new(1280, 720));
        Ok(window)
    }
    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            slint::platform::update_timers_and_animations();
            let wait = slint::platform::duration_until_next_timer_update()
                .unwrap_or(Duration::from_millis(16))
                .min(Duration::from_millis(16));
            let first = match receiver.recv_timeout(wait) {
                Ok(work) => Some(work),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            };
            for work in first.into_iter().chain(receiver.try_iter()) {
                match work {
                    Work::Run(run) => run(),
                    Work::Quit => return Ok(()),
                }
            }
        }
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(Mutex::new(self.sender.clone()))))
    }
}

/// Make the offscreen platform Slint's, before any window exists.
pub fn install() -> Result<(), PlatformError> {
    let (sender, receiver) = mpsc::channel();
    slint::platform::set_platform(Box::new(Offscreen {
        sender,
        receiver: Mutex::new(receiver),
    }))
    .map_err(|error| PlatformError::Other(error.to_string()))
}
