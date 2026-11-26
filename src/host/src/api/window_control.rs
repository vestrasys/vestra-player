//! `window.control`: the main window's video area, always-on-top, cursor
//! hiding and picture-in-picture. `window.state` needs no capability
//! (open.rs).
use super::{Capability, Handled, Info, Recorded};
use crate::host::{Core, Reply, RpcError, string};
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
    /// through standby by the installed instance: they hold nothing that keeps the plugin running.
    /// Picture-in-picture and a video area being applied still do.
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        // Picture-in-picture's holder is the interface's fact (plan step
        // S2f(3)).
        #[cfg(debug_assertions)]
        {
            core.check_settled(&[crate::host::outbox::Fact::PipHolder(owner)]);
        }
        // A picture-in-picture request issued, not yet applied, counts
        // (S3a(3) review B3).
        if core.ui_facts.pip.holder == Some(owner)
            || core.in_flight(crate::host::outbox::Mark::Pip(owner))
            || core.area_replies.iter().any(|a| a.reply.owner == owner)
        {
            return Recorded::Live("a window setting");
        }
        Recorded::Nothing
    }
    fn rekey(&self, core: &mut Core, from: u64, to: u64) {
        core.on_top.rekey(from, to);
        core.cursor_playing.rekey(from, to);
    }
    /// A held owner's standby ends for good: its kept settings go (the
    /// window follows at its next tick, `Host::apply_on_top`).
    fn revoke(&self, core: &mut Core, owner: u64, keep_recorded: bool) {
        if !keep_recorded {
            core.on_top.remove(owner);
            core.cursor_playing.remove(owner);
        }
    }
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "window.setVideoArea" => {
                core.set_video_area(reply, params)?;
                Ok(None)
            }
            "window.setAlwaysOnTop" => {
                let on = params["onTop"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("onTop requires a boolean"))?;
                if core.headless {
                    return Err(RpcError::new("unavailable", "no native window"));
                }
                core.on_top.set(reply.owner, on);
                core.ui_command(crate::host::messages::UiCommand::Window(
                    crate::host::messages::WindowOp::ApplyOnTop,
                ));
                Ok(Some(Value::Null))
            }
            "window.setPictureInPicture" => core.set_picture_in_picture(reply, params),
            _ => {
                let playing = match string(params, "mode")? {
                    "fullscreen" => false,
                    "playing" => true,
                    _ => return Err(RpcError::invalid("mode is fullscreen or playing")),
                };
                core.cursor_playing.set(reply.owner, playing);
                Ok(Some(Value::Null))
            }
        }
    }
}
