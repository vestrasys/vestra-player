//! `playback.control`: opening media and controlling playback (the playback
//! worker), Previous/Next, step seek and keeping the computer awake.
use super::{Capability, Handled, Info, Method, Needs, Recorded, Residency, forward};
use crate::{
    core_actions,
    host::{Host, Reply, RpcError, string},
    i18n, input_trace,
};
use serde_json::{Value, json};

pub struct PlaybackControl;
static INFO: Info = Info {
    name: "playback.control",
    label: "Open media, control playback, keep the computer awake",
    risky: false,
    requires: &[],
    residency: Residency::Live,
    builtin_only: false,
};
static METHODS: &[Method] = &[
    Method::this("playback.set"),
    Method::this("source.submit"),
    // The outcome of this plugin's own source operation (operation books
    // are per plugin): a control that went `pending` (an open a resolver
    // handles) is followed up with `playback.control`.
    Method {
        name: "source.operation",
        needs: Needs::IfDeclared {
            declared: "playback.control",
            then: "playback.control",
            otherwise: "playback.read",
        },
    },
    Method::this("audio.addTrack"),
    Method::this("audio.selectTrack"),
    Method::this("player.setLoop"),
    Method::this("player.setAudioDevice"),
    Method::this("player.step"),
    Method::this("player.setStepSeek"),
    Method::this("power.preventSleep"),
    Method::this("sequence.set"),
    Method::this("sequence.step"),
];
impl Capability for PlaybackControl {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    /// What it holds in the playback worker is answered there (`owner.resources`).
    fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
        if host.step_seek.contains_key(&owner)
            || host.power.holds(owner)
            || host
                .sequence
                .is_some_and(|(provider, ..)| provider == owner)
        {
            return Recorded::Live("a playback setting, the sequence or a sleep lease");
        }
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
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
                let before = host.step_seconds();
                host.step_writes += 1;
                host.step_seek
                    .insert(reply.owner, (host.step_writes, seconds, keyframes));
                host.step_seek_changed(before);
                Ok(Some(Value::Null))
            }
            "power.preventSleep" => {
                host.power
                    .request(reply.owner, params["mode"].as_str().unwrap_or(""))
                    .map_err(RpcError::invalid)?;
                host.power_tick();
                Ok(Some(Value::Null))
            }
            "sequence.set" => {
                if let Some((provider, ..)) = host.sequence
                    && provider != reply.owner
                {
                    let holder = host.plugin_id(provider);
                    let text = i18n::tr_args(
                        "Previous/Next not taken over: provided by {}",
                        &json!([holder]),
                    );
                    return Err(host.refuse_taken(
                        reply.owner,
                        RpcError::sequence_taken(&holder),
                        text,
                    ));
                }
                host.clear_warning(reply.owner, "sequence-taken");
                let flags = &params["flags"];
                host.sequence = if flags.is_null() {
                    None
                } else {
                    let flag = |key: &str| {
                        flags[key]
                            .as_bool()
                            .ok_or_else(|| RpcError::invalid(format!("{key} boolean required")))
                    };
                    Some((reply.owner, flag("canNext")?, flag("canPrevious")?))
                };
                host.rebroadcast_source();
                Ok(Some(Value::Null))
            }
            "sequence.step" => {
                let direction = string(params, "direction")?;
                let (provider, next, previous) = host
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
                host.send(
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
                forward(host, reply, method, params)
            }
        }
    }
}
