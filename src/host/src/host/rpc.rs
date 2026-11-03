//! The plugin protocol's request, reply and error types.
use super::*;

#[derive(Serialize, Debug, Clone)]
pub(crate) struct RpcError {
    pub(crate) code: String,
    pub(crate) message: String,
    /// Extra fields beside `code` and `message` (a slot refusal's `reason`, `slot`, `owner`).
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<serde_json::Map<String, Value>>,
}

impl RpcError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }
    /// Refused because another plugin holds a single-owner resource:
    /// `conflict` with a `reason` (and `owner`, the holder's id). Such a
    /// refusal does not fail activation (`activation_outcome`, the runner).
    pub(crate) fn taken(reason: &str, message: String, detail: Value) -> Self {
        let mut error = Self::new("conflict", message);
        if let Value::Object(mut detail) = detail {
            detail.insert("reason".into(), reason.into());
            error.detail = Some(detail);
        }
        error
    }
    /// `ui.mount`: the slot shows another plugin's panel, or will.
    pub(crate) fn slot_occupied(slot: composition::Slot, holder: &str) -> Self {
        Self::taken(
            "slot-occupied",
            slot_claims::refusal(slot, holder),
            json!({"slot":slot.name(),"owner":holder}),
        )
    }
    /// `sequence.set`: another plugin provides next/previous.
    pub(crate) fn sequence_taken(holder: &str) -> Self {
        Self::taken(
            "sequence-taken",
            i18n::tr_args("Previous/Next is provided by {}", &json!([holder])),
            json!({"owner":holder}),
        )
    }
    pub(crate) fn is_taken(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| {
            matches!(
                d.get("reason").and_then(Value::as_str),
                Some("slot-occupied" | "sequence-taken" | "resolver-taken")
            )
        })
    }
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
    /// The same failure, naming the property it is about.
    pub(crate) fn for_property(mut self, property: &str) -> Self {
        self.message = format!("{property}: {}", self.message);
        self
    }
    /// A property value the player stores now and converts later, checked
    /// against the value limits now (`ui_values::check`).
    pub(crate) fn check_property(property: &str, value: &Value) -> std::result::Result<(), Self> {
        ui_values::check(value).map_err(|error| Self::from(error).for_property(property))
    }
}

impl From<anyhow::Error> for RpcError {
    fn from(value: anyhow::Error) -> Self {
        if value.downcast_ref::<manifest::NeedsPlayer>().is_some() {
            return Self::new("versionMismatch", value.to_string());
        }
        if value.downcast_ref::<ui_values::OverLimit>().is_some() {
            return Self::new("limit", value.to_string());
        }
        Self::new("host", value.to_string())
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) owner: u64,
    pub(crate) id: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) id: u64,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
}

/// A `ui.error` event: a failure the plugin did not ask about (a property
/// the player could not show when it rebuilt a view, a callback whose
/// arguments it could not convert). The SDK reports it to `onError`.
pub(crate) fn ui_error(error: &RpcError) -> Value {
    json!({"event":"ui.error","data":{"code":error.code,"message":error.message}})
}

pub(crate) fn string<'a>(params: &'a Value, key: &str) -> std::result::Result<&'a str, RpcError> {
    params[key]
        .as_str()
        .ok_or_else(|| RpcError::invalid(format!("{key} requires string")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_over_the_limits_is_limit_naming_the_property() {
        // D1: more playlist rows of 8 fields than fit in the values total
        // (9 values each); refused when stored, not later.
        let row = json!({"a":"1","b":"","c":"","d":"","e":false,"f":false,"g":false,"h":false});
        let rows = Value::Array(vec![row; crate::ui::ui_values::MAX_NODES / 9 + 1]);
        let error = RpcError::check_property("rows", &rows).unwrap_err();
        assert_eq!(error.code, "limit");
        assert!(error.message.starts_with("rows: UI "), "{}", error.message);
        assert!(RpcError::check_property("rows", &json!([1, 2])).is_ok());
        // Other conversion failures keep their code.
        let error = RpcError::check_property("x", &json!({"a_b": 1})).unwrap_err();
        assert_eq!(error.code, "host");
        assert_eq!(
            ui_error(&error),
            json!({"event":"ui.error","data":{"code":"host","message":error.message}})
        );
    }

    #[test]
    fn a_slot_refusal_is_a_conflict_naming_the_slot_and_its_holder() {
        let refusal = RpcError::slot_occupied(composition::Slot::Sidebar, "vestra.playlist");
        assert_eq!(
            serde_json::to_value(&refusal).unwrap(),
            json!({"code":"conflict","message":"The sidebar is used by vestra.playlist",
                "reason":"slot-occupied","slot":"sidebar","owner":"vestra.playlist"})
        );
        let plain = RpcError::new("conflict", "owner already mounted");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({"code":"conflict","message":"owner already mounted"})
        );
        let sequence = RpcError::sequence_taken("vestra.playlist");
        assert_eq!(
            serde_json::to_value(&sequence).unwrap(),
            json!({"code":"conflict","message":"Previous/Next is provided by vestra.playlist",
                "reason":"sequence-taken","owner":"vestra.playlist"})
        );
        assert!(refusal.is_taken() && sequence.is_taken() && !plain.is_taken());
    }
}
