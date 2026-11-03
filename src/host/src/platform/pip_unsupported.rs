//! Picture-in-picture on systems without one (Windows, Linux): `unsupported`.
//! Later work (an open topic) gives them a borderless always-on-top window of the
//! player's own; until then `PipWindow` cannot be made, so its methods are
//! never reached.

use super::{HostWindowId, PipEvent, Rect};
use crate::host::RpcError;
use crate::ui::pip::Playback;

pub fn pip_supported() -> bool {
    false
}

/// Never constructed here.
pub enum PipWindow {}

impl PipWindow {
    pub fn new() -> Result<Self, RpcError> {
        Err(super::unsupported("picture-in-picture"))
    }
}

impl crate::ui::pip::Surface for PipWindow {
    type Frame = super::GpuFrame;
    fn place(&self, _window: HostWindowId, _area: Rect) -> bool {
        match *self {}
    }
    fn push_rgb(&self, _width: usize, _height: usize, _rgb: &[u8]) -> Result<(), String> {
        match *self {}
    }
    fn push_surface(&self, _frame: &super::GpuFrame) -> Result<(), String> {
        match *self {}
    }
    fn start(&self) -> bool {
        match *self {}
    }
    fn stop(&self) {
        match *self {}
    }
    fn set_playback(&self, _playback: &Playback) {
        match *self {}
    }
    fn events(&self) -> Vec<PipEvent> {
        match *self {}
    }
}
