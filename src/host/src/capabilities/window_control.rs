//! `window.control`: the main window's video area, always-on-top, cursor
//! hiding and picture-in-picture. `window.state` needs no capability
//! (uncapable.rs).
use super::{Capability, Handled, Info, Method, Recorded, Residency};
use crate::host::{Host, Reply, RpcError, string};
use serde_json::Value;

pub struct WindowControl;
static INFO: Info = Info {
    name: "window.control",
    label: "Adjust the main window (size, on top, picture-in-picture)",
    risky: false,
    requires: &[],
    residency: Residency::Live,
    builtin_only: false,
};
static METHODS: &[Method] = &[
    Method::this("window.setVideoArea"),
    Method::this("window.setAlwaysOnTop"),
    Method::this("window.setCursorAutoHide"),
    Method::this("window.setPictureInPicture"),
];
impl Capability for WindowControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.on_top.holds(owner)
            || host.cursor_playing.holds(owner)
            || host.pip.holds(owner)
            || host.area_replies.iter().any(|a| a.reply.owner == owner)
        {
            return Recorded::Live("a window setting");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
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
