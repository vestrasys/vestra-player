//! `window.control`: the main window's video area, always-on-top, cursor
//! hiding and picture-in-picture. `window.state` needs no capability
//! (uncapable.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Host, Reply, RpcError, string};
use serde_json::Value;

pub struct WindowControl;
static INFO: Info = Info {
    name: "window.control",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "window.setVideoArea",
    "window.setAlwaysOnTop",
    "window.setCursorAutoHide",
    "window.setPictureInPicture",
];
impl Capability for WindowControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// Always-on-top and the cursor setting are plain values the host keeps
    /// through standby by the installed instance (events-and-triggers.md
    /// §4.8): they hold nothing that keeps the plugin running.
    /// Picture-in-picture and a video area being applied still do.
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.pip.holds(owner) || host.area_replies.iter().any(|a| a.reply.owner == owner) {
            return Recorded::Live("a window setting");
        }
        Recorded::Nothing
    }
    fn rekey(&self, host: &mut Host, from: u64, to: u64) {
        host.on_top.rekey(from, to);
        host.cursor_playing.rekey(from, to);
    }
    /// A held owner's standby ends for good: its kept settings go (the
    /// window follows at its next tick, `Host::apply_on_top`).
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool) {
        if !keep_recorded {
            host.on_top.remove(owner);
            host.cursor_playing.remove(owner);
        }
    }
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "window.setVideoArea" => {
                host.set_video_area(reply, params)?;
                Ok(None)
            }
            "window.setAlwaysOnTop" => {
                let on = params["onTop"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("onTop requires a boolean"))?;
                if host.headless {
                    return Err(RpcError::new("unavailable", "no native window"));
                }
                host.on_top.set(reply.owner, on);
                host.apply_on_top();
                Ok(Some(Value::Null))
            }
            "window.setPictureInPicture" => host.set_picture_in_picture(reply, params),
            _ => {
                let playing = match string(params, "mode")? {
                    "fullscreen" => false,
                    "playing" => true,
                    _ => return Err(RpcError::invalid("mode is fullscreen or playing")),
                };
                host.cursor_playing.set(reply.owner, playing);
                Ok(Some(Value::Null))
            }
        }
    }
}
