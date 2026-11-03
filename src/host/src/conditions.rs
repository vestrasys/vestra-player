//! Declarative command state (SDK 1.5.40, `plugin.when`): a command's
//! `enabled` or `checked` given as a condition on state the player owns,
//! evaluated here, so a plugin that is not running (in standby,
//! doc/architecture/supervisor.md §4) still shows a menu
//! that follows playback, fullscreen and the appearance.
//!
//! Wire form (`commands.register` / `commands.update` `when`): an object of
//! the fields it covers, `{ enabled?: Condition, checked?: Condition }`;
//! a condition is an object of exactly one key:
//!
//! - `phase: [..]`: the source's phase is one of these (`empty`, `loading`,
//!   `ready`, `ended`, `error`)
//! - `paused: bool`, `kind: "media" | "stream"`, `fullscreen: bool`
//! - `can: "<control>"`: the source allows it (`PlayerState.can` names)
//! - `theme: "<id>"`, `language: "<preference>"` (`system` or a locale)
//! - `all: [..]`, `any: [..]`, `not: Condition`
use serde_json::Value;

/// The source controls a condition may name: `PlayerState.can` in the SDK.
const CONTROLS: &[&str] = &[
    "pause", "seek", "rate", "restart", "close", "snapshot", "next", "previous",
];
const PHASES: &[&str] = &["empty", "loading", "ready", "ended", "error"];
/// Runaway guards: a condition is a few nodes in practice.
const MAX_NODES: usize = 32;
const MAX_DEPTH: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    Phase(Vec<String>),
    Paused(bool),
    Kind(String),
    Can(String),
    Fullscreen(bool),
    Theme(String),
    Language(String),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

/// The state conditions read: the player's own (source, window,
/// appearance), never a plugin's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub phase: String,
    pub paused: bool,
    /// `media` or `stream` (a generated source).
    pub kind: String,
    /// The controls the source allows (`CONTROLS` names).
    pub can: Vec<String>,
    pub fullscreen: bool,
    pub theme: String,
    /// `system` or a locale.
    pub language: String,
}
impl Facts {
    /// From the last `source.state` (wire form), the last `window.state`,
    /// the palette and the saved language preference.
    pub fn new(source: &Value, window: &Value, theme: &str, language: &str) -> Self {
        let flag = |value: &Value| value.as_bool().unwrap_or(false);
        let capabilities = &source["capabilities"];
        let can = CONTROLS
            .iter()
            .filter(|control| match **control {
                "snapshot" => flag(&capabilities["snapshotSource"]),
                "next" | "previous" => flag(&source["sequence"][*control]),
                other => flag(&capabilities[other]),
            })
            .map(|control| (*control).to_owned())
            .collect();
        Self {
            phase: source["phase"].as_str().unwrap_or("empty").to_owned(),
            paused: flag(&source["paused"]),
            kind: if source["kind"] == "producer" {
                "stream"
            } else {
                "media"
            }
            .to_owned(),
            can,
            fullscreen: flag(&window["fullscreen"]),
            theme: theme.to_owned(),
            language: language.to_owned(),
        }
    }
}

impl Condition {
    /// Reads the wire form; `Err` names the problem (`invalid`).
    pub fn parse(value: &Value) -> Result<Self, String> {
        let mut nodes = 0;
        Self::read(value, 0, &mut nodes)
    }
    fn read(value: &Value, depth: usize, nodes: &mut usize) -> Result<Self, String> {
        *nodes += 1;
        if *nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(format!(
                "a condition has at most {MAX_NODES} parts, {MAX_DEPTH} levels deep"
            ));
        }
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or("a condition is an object with one key")?;
        let (key, value) = object.iter().next().expect("one key");
        let text = |what: &str| {
            value
                .as_str()
                .filter(|text| !text.is_empty() && text.len() <= 64)
                .map(str::to_owned)
                .ok_or(format!("condition {key} is {what}"))
        };
        let flag = || {
            value
                .as_bool()
                .ok_or(format!("condition {key} is true or false"))
        };
        let list = |nodes: &mut usize| -> Result<Vec<Self>, String> {
            let items = value
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or(format!("condition {key} is a list of conditions"))?;
            items
                .iter()
                .map(|item| Self::read(item, depth + 1, nodes))
                .collect()
        };
        Ok(match key.as_str() {
            "phase" => {
                let phases = value
                    .as_array()
                    .filter(|items| !items.is_empty())
                    .and_then(|items| {
                        items
                            .iter()
                            .map(|item| item.as_str().filter(|p| PHASES.contains(p)))
                            .collect::<Option<Vec<_>>>()
                    })
                    .ok_or(format!(
                        "condition phase is a list of {}",
                        PHASES.join(", ")
                    ))?;
                Self::Phase(phases.into_iter().map(str::to_owned).collect())
            }
            "paused" => Self::Paused(flag()?),
            "fullscreen" => Self::Fullscreen(flag()?),
            "kind" => match value.as_str() {
                Some(kind @ ("media" | "stream")) => Self::Kind(kind.to_owned()),
                _ => return Err("condition kind is media or stream".into()),
            },
            "can" => match value.as_str() {
                Some(control) if CONTROLS.contains(&control) => Self::Can(control.to_owned()),
                _ => return Err(format!("condition can is one of {}", CONTROLS.join(", "))),
            },
            "theme" => match value.as_str() {
                Some(theme) if crate::ui::theme::THEMES.contains(&theme) => {
                    Self::Theme(theme.to_owned())
                }
                _ => {
                    return Err(format!(
                        "condition theme is one of {}",
                        crate::ui::theme::THEMES.join(", ")
                    ));
                }
            },
            "language" => {
                let language = text("a language preference")?;
                if !crate::i18n::valid_preference(&language) {
                    return Err(format!(
                        "condition language is \"system\" or one of {}",
                        crate::i18n::LOCALES.join(", ")
                    ));
                }
                Self::Language(language)
            }
            "all" => Self::All(list(nodes)?),
            "any" => Self::Any(list(nodes)?),
            "not" => Self::Not(Box::new(Self::read(value, depth + 1, nodes)?)),
            other => return Err(format!("unknown condition {other}")),
        })
    }
    pub fn holds(&self, facts: &Facts) -> bool {
        match self {
            Self::Phase(phases) => phases.contains(&facts.phase),
            Self::Paused(paused) => facts.paused == *paused,
            Self::Kind(kind) => facts.kind == *kind,
            Self::Can(control) => facts.can.contains(control),
            Self::Fullscreen(fullscreen) => facts.fullscreen == *fullscreen,
            Self::Theme(theme) => facts.theme == *theme,
            Self::Language(language) => facts.language == *language,
            Self::All(all) => all.iter().all(|c| c.holds(facts)),
            Self::Any(any) => any.iter().any(|c| c.holds(facts)),
            Self::Not(not) => !not.holds(facts),
        }
    }
}

/// A command's conditions and the reason its plugin sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub enabled: Option<Condition>,
    pub checked: Option<Condition>,
    /// The state's `reason` as sent (translated): with an `enabled`
    /// condition it shows only while the command is disabled or busy.
    pub reason: Option<String>,
}
impl Conditions {
    /// `when` of a register or update: `None` when absent or empty.
    pub fn parse(value: &Value) -> Result<Option<Self>, String> {
        if value.is_null() {
            return Ok(None);
        }
        let object = value.as_object().ok_or("when is { enabled?, checked? }")?;
        if let Some(key) = object.keys().find(|k| *k != "enabled" && *k != "checked") {
            return Err(format!("when has no field {key}: only enabled and checked"));
        }
        let field = |name: &str| object.get(name).map(Condition::parse).transpose();
        let (enabled, checked) = (field("enabled")?, field("checked")?);
        Ok((enabled.is_some() || checked.is_some()).then_some(Self {
            enabled,
            checked,
            reason: None,
        }))
    }
    /// The state shown: `state` (as sent) with the condition fields
    /// evaluated against `facts`.
    pub fn apply(
        &self,
        state: &crate::commands::CommandState,
        facts: &Facts,
    ) -> crate::commands::CommandState {
        let mut shown = state.clone();
        if let Some(enabled) = &self.enabled {
            shown.enabled = enabled.holds(facts);
            shown.reason = (!shown.enabled || shown.busy)
                .then(|| self.reason.clone())
                .flatten();
        }
        if let Some(checked) = &self.checked {
            shown.checked = Some(checked.holds(facts));
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandState;
    use serde_json::json;

    fn facts() -> Facts {
        Facts::new(
            &json!({"phase":"ready","paused":false,"kind":"file",
                "capabilities":{"pause":true,"seek":true,"snapshotSource":true},
                "sequence":{"next":true,"previous":false}}),
            &json!({"fullscreen":false}),
            "vestra",
            "system",
        )
    }

    #[test]
    fn facts_read_the_wire_state_in_the_sdk_names() {
        let facts = facts();
        assert_eq!(facts.phase, "ready");
        assert_eq!(facts.kind, "media");
        assert_eq!(facts.can, ["pause", "seek", "snapshot", "next"]);
        let empty = Facts::new(&Value::Null, &Value::Null, "vestra", "system");
        assert_eq!((empty.phase.as_str(), empty.fullscreen), ("empty", false));
        assert!(empty.can.is_empty());
        let stream = Facts::new(&json!({"kind":"producer"}), &Value::Null, "", "");
        assert_eq!(stream.kind, "stream");
    }

    #[test]
    fn conditions_evaluate_and_combine() {
        let facts = facts();
        let holds = |value: Value| Condition::parse(&value).unwrap().holds(&facts);
        assert!(holds(json!({"phase":["ready","ended"]})));
        assert!(!holds(json!({"phase":["empty"]})));
        assert!(holds(json!({"paused":false})));
        assert!(holds(json!({"kind":"media"})));
        assert!(holds(json!({"can":"snapshot"})));
        assert!(!holds(json!({"can":"rate"})));
        assert!(!holds(json!({"fullscreen":true})));
        assert!(holds(json!({"theme":"vestra"})));
        assert!(holds(json!({"language":"system"})));
        // `when.playing`, and capture's "a frame to save".
        assert!(holds(json!({"all":[{"phase":["ready"]},{"paused":false}]})));
        assert!(holds(
            json!({"all":[{"can":"snapshot"},{"phase":["ready","ended"]},
            {"not":{"kind":"stream"}}]})
        ));
        assert!(holds(
            json!({"any":[{"fullscreen":true},{"theme":"vestra"}]})
        ));
        assert!(!holds(json!({"not":{"theme":"vestra"}})));
    }

    #[test]
    fn malformed_conditions_are_refused() {
        for bad in [
            json!(true),
            json!({}),
            json!({"phase":"ready"}),
            json!({"phase":["playing"]}),
            json!({"paused":1}),
            json!({"kind":"file"}),
            json!({"can":"fly"}),
            json!({"theme":""}),
            json!({"theme":"dark"}),
            json!({"language":"xx"}),
            json!({"all":[]}),
            json!({"phase":["ready"],"paused":true}),
            json!({"source":true}),
        ] {
            assert!(Condition::parse(&bad).is_err(), "{bad}");
        }
        let mut deep = json!({"paused":true});
        for _ in 0..8 {
            deep = json!({ "not": deep });
        }
        assert!(Condition::parse(&deep).is_err());
        let wide = json!({"any": vec![json!({"paused":true}); 40]});
        assert!(Condition::parse(&wide).is_err());
        assert!(Conditions::parse(&json!({"busy":{"paused":true}})).is_err());
        assert_eq!(Conditions::parse(&json!({})).unwrap(), None);
        assert_eq!(Conditions::parse(&Value::Null).unwrap(), None);
    }

    #[test]
    fn an_enabled_condition_shows_the_reason_only_while_disabled() {
        let conditions = Conditions {
            reason: Some("No frame to capture".into()),
            ..Conditions::parse(
                &json!({"enabled":{"phase":["ready"]},"checked":{"fullscreen":true}}),
            )
            .unwrap()
            .unwrap()
        };
        let sent = CommandState {
            enabled: true,
            checked: Some(true),
            busy: false,
            reason: Some("No frame to capture".into()),
        };
        let ready = conditions.apply(&sent, &facts());
        assert_eq!(
            (ready.enabled, ready.checked, ready.reason),
            (true, Some(false), None)
        );
        let empty = Facts {
            phase: "empty".into(),
            ..facts()
        };
        let shown = conditions.apply(&sent, &empty);
        assert!(!shown.enabled);
        assert_eq!(shown.reason.as_deref(), Some("No frame to capture"));
    }
}
