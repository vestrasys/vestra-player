//! `playback.control`: opening media and controlling playback (the playback
//! worker), Previous/Next, step seek and keeping the computer awake.
use super::{Capability, Handled, Info, Recorded, forward};
use crate::{
    core_actions,
    host::{Core, Reply, RpcError, string},
    i18n, input_trace,
};
use serde_json::{Value, json};

pub struct PlaybackControl;
static INFO: Info = Info {
    name: "playback.control",
    builtin_only: false,
};
static METHODS: &[&str] = &[
    "playback.set",
    "source.submit",
    // The outcome of this plugin's own source operation (operation books
    // are per plugin): a control that went `pending` (an open a resolver
    // handles) is followed up here.
    "source.operation",
    "audio.addTrack",
    "audio.selectTrack",
    "player.setLoop",
    "player.setAudioDevice",
    "player.step",
    "player.setStepSeek",
    "power.preventSleep",
    "sequence.set",
    "sequence.step",
];
impl Capability for PlaybackControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    /// What it holds in the playback worker is answered there (`owner.resources`).
    fn record(&self, core: &Core, owner: u64, _: &Value) -> Recorded {
        if core.step_seek.contains_key(&owner)
            || core.power.holds(owner)
            || core
                .sequence
                .is_some_and(|(provider, ..)| provider == owner)
        {
            return Recorded::Live("a playback setting, the sequence or a sleep lease");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Core, _: u64, _: u64) {}
    fn handle(&self, core: &mut Core, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "player.setStepSeek" => {
                let seconds = params["seconds"]
                    .as_f64()
                    .filter(|s| core_actions::valid_step(*s))
                    .ok_or_else(|| RpcError::invalid("seconds must be 0.1..3600 in tenths"))?;
                let keyframes = match &params["keyframes"] {
                    Value::Null => false,
                    Value::Bool(keyframes) => *keyframes,
                    _ => return Err(RpcError::invalid("keyframes must be a boolean")),
                };
                let before = core.step_seconds();
                core.step_writes += 1;
                core.step_seek
                    .insert(reply.owner, (core.step_writes, seconds, keyframes));
                core.step_seek_changed(before);
                Ok(Some(Value::Null))
            }
            "power.preventSleep" => {
                core.power
                    .request(reply.owner, params["mode"].as_str().unwrap_or(""))
                    .map_err(RpcError::invalid)?;
                core.power_tick();
                Ok(Some(Value::Null))
            }
            "sequence.set" => {
                if let Some((provider, ..)) = core.sequence
                    && provider != reply.owner
                {
                    let holder = core.plugin_id(provider);
                    let text = i18n::tr_args(
                        "Previous/Next not taken over: provided by {}",
                        &json!([holder]),
                    );
                    return Err(core.refuse_taken(
                        reply.owner,
                        RpcError::sequence_taken(&holder),
                        text,
                    ));
                }
                core.clear_warning(reply.owner, "sequence-taken");
                let flags = &params["flags"];
                core.sequence = if flags.is_null() {
                    None
                } else {
                    let flag = |key: &str| {
                        flags[key]
                            .as_bool()
                            .ok_or_else(|| RpcError::invalid(format!("{key} boolean required")))
                    };
                    Some((reply.owner, flag("canNext")?, flag("canPrevious")?))
                };
                core.rebroadcast_source();
                Ok(Some(Value::Null))
            }
            "sequence.step" => {
                let direction = string(params, "direction")?;
                let (provider, next, previous) = core
                    .sequence
                    .ok_or_else(|| RpcError::new("unavailable", "no sequence provider"))?;
                let allowed = match direction {
                    "next" => next,
                    "previous" => previous,
                    _ => return Err(RpcError::invalid("direction is next or previous")),
                };
                if !allowed {
                    return Err(RpcError::new(
                        "unavailable",
                        format!("cannot go {direction}"),
                    ));
                }
                core.send(
                    provider,
                    json!({"event":"sequence.step","data":{"direction":direction}}),
                );
                Ok(Some(Value::Null))
            }
            _ => {
                if method == "source.submit" {
                    // A seek control names its mode: `Control::Seek`'s
                    // optional `keyframes` (SDK 1.5.22 `Scrub.seek(seconds,
                    // { keyframes: true })`) is read by the playback worker
                    // (source_backend.rs); here it is only traced.
                    let control = &params["intent"]["control"];
                    let detail = match control["kind"].as_str() {
                        Some("seek") if control["keyframes"] == true => {
                            " seek+keyframes".to_owned()
                        }
                        Some(kind) => format!(" {kind}"),
                        None => String::new(),
                    };
                    input_trace::follow(&format!(
                        "host loop: source.submit {}{} from plugin {}",
                        params["intent"]["kind"].as_str().unwrap_or("?"),
                        detail,
                        reply.owner
                    ));
                }
                forward(core, reply, method, params)
            }
        }
    }
}
