//! Experimental command registry; final API may differ substantially.
//! This is a shared menu/context-menu model, not a menu renderer, key-binding
//! engine or plugin policy layer. Callers authenticate live owner generations
//! before registration and route resolved execution to that exact instance.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub type OwnerGeneration = u64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandKey {
    pub owner: OwnerGeneration,
    pub local_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    pub title: String,
    /// Complete primary menu path, including an allowed top-level domain.
    pub path: Vec<String>,
    /// Main-canvas keys Space/ArrowLeft/ArrowRight are routed by the host.
    /// Other strings remain display declarations.
    pub shortcut: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandState {
    pub enabled: bool,
    pub checked: Option<bool>,
    pub busy: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSnapshot {
    pub key: CommandKey,
    /// Supplied by this command's owner, strictly increasing for state updates.
    pub revision: u64,
    pub declaration: Declaration,
    pub state: CommandState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MenuNode {
    Group {
        title: String,
        children: Vec<MenuNode>,
    },
    Command {
        entry: CommandSnapshot,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSnapshot {
    /// Registry-local sequence for accepted registration/state/removal changes.
    pub revision: u64,
    /// Top-level domains follow shell configuration; descendants use lexical
    /// path order, independent of registration timing.
    /// Both primary and context-menu projections consume this same tree.
    pub roots: Vec<MenuNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execution {
    pub owner: OwnerGeneration,
    pub local_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    Invalid(String),
    Conflict(String),
    Missing(CommandKey),
    Stale { current: u64, submitted: u64 },
    Disabled { reason: Option<String> },
    Busy { reason: Option<String> },
    Limit,
    RevisionExhausted,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid command: {message}"),
            Self::Conflict(message) => write!(f, "command conflict: {message}"),
            Self::Missing(key) => {
                write!(f, "command missing: owner {} / {}", key.owner, key.local_id)
            }
            Self::Stale { current, submitted } => write!(
                f,
                "stale command revision {submitted}; current is {current}"
            ),
            Self::Disabled { reason } => write!(
                f,
                "command disabled{}",
                reason
                    .as_ref()
                    .map(|r| format!(": {r}"))
                    .unwrap_or_default()
            ),
            Self::Busy { reason } => write!(
                f,
                "command busy{}",
                reason
                    .as_ref()
                    .map(|r| format!(": {r}"))
                    .unwrap_or_default()
            ),
            Self::Limit => f.write_str("command registry limit reached"),
            Self::RevisionExhausted => f.write_str("command registry revision exhausted"),
        }
    }
}
impl std::error::Error for CommandError {}

type Result<T> = std::result::Result<T, CommandError>;

pub struct Registry {
    domains: BTreeSet<String>,
    domain_order: Vec<String>,
    commands: BTreeMap<CommandKey, CommandSnapshot>,
    revision: u64,
}

fn text(value: &str, label: &str, limit: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > limit
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(CommandError::Invalid(format!(
            "{label} requires 1..={limit} bytes, no controls or surrounding whitespace"
        )));
    }
    Ok(())
}

fn local_id(value: &str) -> Result<()> {
    text(value, "localId", 128)?;
    if !value
        .bytes()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-'))
    {
        return Err(CommandError::Invalid(
            "localId permits ASCII letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(())
}

fn state_valid(state: &CommandState) -> Result<()> {
    if let Some(reason) = &state.reason {
        text(reason, "reason", 512)?;
    }
    Ok(())
}

impl Registry {
    pub const MAX_COMMANDS: usize = 512;
    pub const MAX_PER_OWNER: usize = 64;

    /// Domain names come from the shell's menu contract, never plugin identity.
    /// This finite candidate requires at least one and at most 16 domains.
    pub fn new(allowed_domains: Vec<String>) -> Result<Self> {
        if allowed_domains.is_empty() || allowed_domains.len() > 16 {
            return Err(CommandError::Invalid(
                "expected 1..=16 top-level domains".into(),
            ));
        }
        let mut domains = BTreeSet::new();
        for domain in &allowed_domains {
            text(domain, "domain", 80)?;
            if !domains.insert(domain.clone()) {
                return Err(CommandError::Conflict("duplicate top-level domain".into()));
            }
        }
        Ok(Self {
            domains,
            domain_order: allowed_domains,
            commands: BTreeMap::new(),
            revision: 0,
        })
    }

    fn next_revision(&self) -> Result<u64> {
        self.revision
            .checked_add(1)
            .ok_or(CommandError::RevisionExhausted)
    }

    pub fn register(
        &mut self,
        owner: OwnerGeneration,
        id: String,
        revision: u64,
        declaration: Declaration,
        state: CommandState,
    ) -> Result<()> {
        local_id(&id)?;
        if revision == 0 {
            return Err(CommandError::Invalid("command revision starts at 1".into()));
        }
        text(&declaration.title, "title", 128)?;
        if !(2..=6).contains(&declaration.path.len()) {
            return Err(CommandError::Invalid(
                "menu path requires 2..=6 segments including domain and leaf".into(),
            ));
        }
        for segment in &declaration.path {
            text(segment, "path segment", 80)?;
        }
        if !self.domains.contains(&declaration.path[0]) {
            return Err(CommandError::Invalid(
                "menu domain is not allowed by this shell".into(),
            ));
        }
        if let Some(shortcut) = &declaration.shortcut {
            text(shortcut, "shortcut", 64)?;
        }
        state_valid(&state)?;
        let key = CommandKey {
            owner,
            local_id: id,
        };
        if self.commands.contains_key(&key) {
            return Err(CommandError::Conflict(
                "owner/localId already registered".into(),
            ));
        }
        for entry in self.commands.values() {
            if declaration.shortcut.as_deref().is_some_and(canvas_shortcut)
                && declaration.shortcut == entry.declaration.shortcut
            {
                return Err(CommandError::Conflict(
                    "main-canvas shortcut already registered".into(),
                ));
            }
            let existing = &entry.declaration.path;
            if existing.starts_with(&declaration.path) || declaration.path.starts_with(existing) {
                return Err(CommandError::Conflict(
                    "menu path duplicates a command or collides with a leaf/group".into(),
                ));
            }
        }
        if self.commands.len() >= Self::MAX_COMMANDS
            || self
                .commands
                .keys()
                .filter(|key| key.owner == owner)
                .count()
                >= Self::MAX_PER_OWNER
        {
            return Err(CommandError::Limit);
        }
        let next = self.next_revision()?;
        self.commands.insert(
            key.clone(),
            CommandSnapshot {
                key,
                revision,
                declaration,
                state,
            },
        );
        self.revision = next;
        Ok(())
    }

    /// Declaration/path identity stays fixed. State updates are complete facts,
    /// never a toggle, so a delayed older update cannot undo a newer state.
    pub fn update(
        &mut self,
        owner: OwnerGeneration,
        id: &str,
        revision: u64,
        state: CommandState,
    ) -> Result<()> {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        let entry = self
            .commands
            .get(&key)
            .ok_or_else(|| CommandError::Missing(key.clone()))?;
        if revision <= entry.revision {
            return Err(CommandError::Stale {
                current: entry.revision,
                submitted: revision,
            });
        }
        state_valid(&state)?;
        let next = self.next_revision()?;
        let entry = self
            .commands
            .get_mut(&key)
            .expect("entry checked in this serial method");
        entry.revision = revision;
        entry.state = state;
        self.revision = next;
        Ok(())
    }

    /// Removes only the specified generation. A replacement instance has a new
    /// owner generation, even if its manifest and local command IDs are equal.
    /// The outer runtime rejects future RPC from stopped owners; this model does
    /// not retain an unbounded history of dead owner tombstones.
    pub fn remove_owner(&mut self, owner: OwnerGeneration) -> Result<usize> {
        let count = self
            .commands
            .keys()
            .filter(|key| key.owner == owner)
            .count();
        if count != 0 {
            let next = self.next_revision()?;
            self.commands.retain(|key, _| key.owner != owner);
            self.revision = next;
        }
        Ok(count)
    }

    pub fn snapshot(&self) -> MenuSnapshot {
        let mut entries: Vec<_> = self.commands.values().collect();
        let rank = |entry: &CommandSnapshot| {
            self.domain_order
                .iter()
                .position(|domain| domain == &entry.declaration.path[0])
                .unwrap_or(usize::MAX)
        };
        entries.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| a.declaration.path[1..].cmp(&b.declaration.path[1..]))
        });
        MenuSnapshot {
            revision: self.revision,
            roots: tree(&entries, 0),
        }
    }

    /// Resolve only the finite canonical canvas keys against current state.
    pub fn resolve_shortcut(&self, shortcut: &str) -> Option<Execution> {
        if !canvas_shortcut(shortcut) {
            return None;
        }
        self.commands
            .values()
            .find(|entry| entry.declaration.shortcut.as_deref() == Some(shortcut))
            .and_then(|entry| {
                self.resolve_execution(entry.key.owner, &entry.key.local_id)
                    .ok()
            })
    }

    /// Resolve immediately before dispatch, at the serial registry arbiter.
    /// A cached menu snapshot is never execution authority. Runtime liveness is
    /// checked again by the caller when routing to this exact owner generation.
    pub fn resolve_execution(&self, owner: OwnerGeneration, id: &str) -> Result<Execution> {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        let entry = self.commands.get(&key).ok_or(CommandError::Missing(key))?;
        if entry.state.busy {
            return Err(CommandError::Busy {
                reason: entry.state.reason.clone(),
            });
        }
        if !entry.state.enabled {
            return Err(CommandError::Disabled {
                reason: entry.state.reason.clone(),
            });
        }
        Ok(Execution {
            owner,
            local_id: id.into(),
            revision: entry.revision,
        })
    }
}

fn canvas_shortcut(value: &str) -> bool {
    matches!(value, "Space" | "ArrowLeft" | "ArrowRight")
}

fn tree(entries: &[&CommandSnapshot], depth: usize) -> Vec<MenuNode> {
    let mut nodes = Vec::new();
    let mut cursor = 0;
    while cursor < entries.len() {
        let entry = entries[cursor];
        let title = &entry.declaration.path[depth];
        if depth + 1 == entry.declaration.path.len() {
            nodes.push(MenuNode::Command {
                entry: entry.clone(),
            });
            cursor += 1;
        } else {
            let end = entries[cursor..]
                .iter()
                .position(|entry| &entry.declaration.path[depth] != title)
                .map(|offset| cursor + offset)
                .unwrap_or(entries.len());
            nodes.push(MenuNode::Group {
                title: title.clone(),
                children: tree(&entries[cursor..end], depth + 1),
            });
            cursor = end;
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registry() -> Registry {
        Registry::new(vec!["Playback".into(), "Tools".into()]).unwrap()
    }
    fn state(enabled: bool) -> CommandState {
        CommandState {
            enabled,
            checked: None,
            busy: false,
            reason: None,
        }
    }
    fn declaration(path: &[&str]) -> Declaration {
        Declaration {
            title: path.last().unwrap().to_string(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: None,
        }
    }

    #[test]
    fn canvas_shortcut_uses_current_command_state_and_releases_with_owner() {
        let mut registry = registry();
        let mut first = declaration(&["Playback", "Toggle"]);
        first.shortcut = Some("Space".into());
        registry
            .register(1, "toggle".into(), 1, first.clone(), state(true))
            .unwrap();
        let mut conflict = first.clone();
        conflict.path = vec!["Tools".into(), "Other".into()];
        assert!(matches!(
            registry.register(2, "other".into(), 1, conflict.clone(), state(true)),
            Err(CommandError::Conflict(_))
        ));
        assert_eq!(registry.resolve_shortcut("Space").unwrap().owner, 1);
        registry.update(1, "toggle", 2, state(false)).unwrap();
        assert!(registry.resolve_shortcut("Space").is_none());
        registry
            .update(
                1,
                "toggle",
                3,
                CommandState {
                    busy: true,
                    ..state(true)
                },
            )
            .unwrap();
        assert!(registry.resolve_shortcut("Space").is_none());
        registry.remove_owner(1).unwrap();
        assert!(registry.resolve_shortcut("Space").is_none());
        registry
            .register(2, "other".into(), 1, conflict, state(true))
            .unwrap();
        assert_eq!(registry.resolve_shortcut("Space").unwrap().owner, 2);
        assert!(registry.resolve_shortcut("F11").is_none());
    }

    #[test]
    fn local_ids_are_namespaced_but_primary_paths_have_one_owner() {
        let mut registry = registry();
        registry
            .register(
                1,
                "open".into(),
                1,
                declaration(&["Playback", "Open"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                2,
                "open".into(),
                1,
                declaration(&["Tools", "Open"]),
                state(true),
            )
            .unwrap();
        assert_eq!(registry.resolve_execution(2, "open").unwrap().owner, 2);
        assert!(matches!(
            registry.register(
                3,
                "different".into(),
                1,
                declaration(&["Playback", "Open"]),
                state(true)
            ),
            Err(CommandError::Conflict(_))
        ));
        assert!(matches!(
            registry.register(
                1,
                "open".into(),
                2,
                declaration(&["Tools", "Other"]),
                state(true)
            ),
            Err(CommandError::Conflict(_))
        ));
    }

    #[test]
    fn both_leaf_group_collision_directions_fail_without_changing_tree() {
        for leaf_first in [false, true] {
            let mut registry = registry();
            let short = declaration(&["Tools", "Inspect"]);
            let long = declaration(&["Tools", "Inspect", "Details"]);
            let (first, second) = if leaf_first {
                (short, long)
            } else {
                (long, short)
            };
            registry
                .register(1, "first".into(), 1, first, state(true))
                .unwrap();
            let before = registry.snapshot();
            assert!(matches!(
                registry.register(2, "second".into(), 1, second, state(true)),
                Err(CommandError::Conflict(_))
            ));
            assert_eq!(registry.snapshot(), before);
        }
    }

    #[test]
    fn stale_state_cannot_reenable_a_disabled_command_and_same_tree_exposes_reason() {
        let mut registry = registry();
        registry
            .register(
                1,
                "play".into(),
                1,
                declaration(&["Playback", "Play"]),
                state(true),
            )
            .unwrap();
        let mut disabled = state(false);
        disabled.checked = Some(true);
        disabled.reason = Some("No source".into());
        registry.update(1, "play", 3, disabled.clone()).unwrap();
        let current = registry.snapshot();
        assert_eq!(
            registry.update(1, "play", 2, state(true)),
            Err(CommandError::Stale {
                current: 3,
                submitted: 2
            })
        );
        assert_eq!(registry.snapshot(), current);
        assert_eq!(
            registry.resolve_execution(1, "play"),
            Err(CommandError::Disabled {
                reason: Some("No source".into())
            })
        );
        let MenuNode::Group { children, .. } = &current.roots[0] else {
            panic!("expected domain")
        };
        let MenuNode::Command { entry } = &children[0] else {
            panic!("expected shared command fact")
        };
        assert_eq!(entry.state, disabled);
        let mut busy = state(true);
        busy.busy = true;
        busy.reason = Some("Preparing".into());
        registry.update(1, "play", 4, busy).unwrap();
        assert_eq!(
            registry.resolve_execution(1, "play"),
            Err(CommandError::Busy {
                reason: Some("Preparing".into())
            })
        );
    }

    #[test]
    fn stopped_owner_cleanup_cannot_remove_replacement_and_path_is_reusable() {
        let mut registry = registry();
        registry
            .register(
                10,
                "run".into(),
                1,
                declaration(&["Tools", "Old"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                11,
                "run".into(),
                1,
                declaration(&["Tools", "New"]),
                state(true),
            )
            .unwrap();
        assert_eq!(registry.remove_owner(10).unwrap(), 1);
        assert_eq!(registry.remove_owner(10).unwrap(), 0);
        assert!(matches!(
            registry.resolve_execution(10, "run"),
            Err(CommandError::Missing(_))
        ));
        assert!(matches!(
            registry.update(10, "run", 99, state(true)),
            Err(CommandError::Missing(_))
        ));
        assert_eq!(registry.resolve_execution(11, "run").unwrap().owner, 11);
        registry
            .register(
                12,
                "reuse".into(),
                1,
                declaration(&["Tools", "Old"]),
                state(true),
            )
            .unwrap();
        assert_eq!(registry.resolve_execution(12, "reuse").unwrap().owner, 12);
    }

    #[test]
    fn top_level_domains_keep_host_order_even_when_reverse_lexical() {
        let mut registry = Registry::new(vec!["Zulu".into(), "Alpha".into()]).unwrap();
        for (id, path) in [
            ("first", ["Alpha", "Item"]),
            ("second", ["Zulu", "Beta"]),
            ("third", ["Zulu", "Able"]),
        ] {
            registry
                .register(1, id.into(), 1, declaration(&path), state(true))
                .unwrap();
        }
        let snapshot = registry.snapshot();
        let titles: Vec<_> = snapshot
            .roots
            .iter()
            .map(|node| {
                let MenuNode::Group { title, .. } = node else {
                    panic!("expected domain group")
                };
                title.as_str()
            })
            .collect();
        assert_eq!(titles, ["Zulu", "Alpha"]);
        let MenuNode::Group { children, .. } = &snapshot.roots[0] else {
            panic!("expected first domain")
        };
        let titles: Vec<_> = children
            .iter()
            .map(|node| {
                let MenuNode::Command { entry } = node else {
                    panic!("expected command")
                };
                entry.declaration.title.as_str()
            })
            .collect();
        assert_eq!(titles, ["Able", "Beta"]);
    }

    #[test]
    fn registration_order_does_not_change_tree_and_limits_reject_new_work() {
        let mut a = registry();
        let mut b = registry();
        let mut declarations = vec![
            (1, "z", declaration(&["Tools", "Z"])),
            (2, "a", declaration(&["Playback", "A"])),
        ];
        for (owner, id, declaration) in declarations.clone() {
            a.register(owner, id.into(), 1, declaration, state(true))
                .unwrap();
        }
        declarations.reverse();
        for (owner, id, declaration) in declarations {
            b.register(owner, id.into(), 1, declaration, state(true))
                .unwrap();
        }
        assert_eq!(a.snapshot(), b.snapshot());
        assert!(matches!(
            a.register(
                4,
                "bad".into(),
                1,
                declaration(&["Unknown", "X"]),
                state(true)
            ),
            Err(CommandError::Invalid(_))
        ));
        let mut limited = registry();
        for index in 0..Registry::MAX_PER_OWNER {
            limited
                .register(
                    5,
                    format!("c{index}"),
                    1,
                    declaration(&["Tools", &format!("Item {index}")]),
                    state(true),
                )
                .unwrap();
        }
        assert_eq!(
            limited.register(
                5,
                "extra".into(),
                1,
                declaration(&["Tools", "Extra"]),
                state(true)
            ),
            Err(CommandError::Limit)
        );
    }
}
