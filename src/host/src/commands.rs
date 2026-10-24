//! Command registry: the shared menu model (core-ui-design.md §4).
//! Not a menu renderer or key-binding engine (see menu.rs and bindings.rs).
//! Callers authenticate live owner generations before registration and route
//! resolved execution to that exact instance.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub type OwnerGeneration = u64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandKey {
    pub owner: OwnerGeneration,
    pub local_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Focus {
    Panel,
}

fn domain() -> String {
    "domain".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    pub title: String,
    /// Menu path including the leaf, 0..=8 segments; empty = not in the main menu.
    #[serde(default)]
    pub path: Vec<String>,
    /// Sibling order, ascending; a submenu sits where its first command sits,
    /// except a built-in top-level menu (`MENU_ANCHORS`). Directly inside a
    /// built-in menu, 0 (the default) sorts after every other order.
    #[serde(default)]
    pub order: i32,
    /// Separator label between adjacent siblings.
    #[serde(default = "domain")]
    pub group: String,
    #[serde(default)]
    pub radio: bool,
    /// Canonical binding strings, parsed by bindings.rs.
    #[serde(default)]
    pub bindings: Vec<String>,
    #[serde(default)]
    pub focus: Option<Focus>,
    #[serde(default)]
    pub repeat: bool,
}

impl Declaration {
    pub fn new(title: &str, path: &[&str]) -> Self {
        Self {
            title: title.into(),
            path: path.iter().map(|s| s.to_string()).collect(),
            order: 0,
            group: domain(),
            radio: false,
            bindings: Vec::new(),
            focus: None,
            repeat: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandState {
    pub enabled: bool,
    pub checked: Option<bool>,
    pub busy: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSnapshot {
    pub key: CommandKey,
    /// Supplied by this command's owner, strictly increasing for state updates.
    pub revision: u64,
    pub declaration: Declaration,
    pub state: CommandState,
    /// Tie-breaks equal orders: manifest id, then registration sequence.
    pub manifest: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MenuNode {
    Group {
        title: String,
        /// Taken from the first command (in menu order) below this group.
        group: String,
        children: Vec<MenuNode>,
    },
    Command {
        entry: CommandSnapshot,
    },
}

impl MenuNode {
    /// Separator label of this node at its own level.
    pub fn group(&self) -> &str {
        match self {
            Self::Group { group, .. } => group,
            Self::Command { entry } => &entry.declaration.group,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSnapshot {
    /// Registry-local sequence for accepted registration/state/removal changes.
    pub revision: u64,
    /// Sorted by order (a submenu by its first command, a built-in menu by
    /// its `MENU_ANCHORS` order), then manifest id and
    /// registration sequence; independent of registration timing.
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

impl CommandError {
    /// Stable RPC code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::Conflict(_) => "conflict",
            Self::Missing(_) => "missing",
            Self::Limit => "limit",
            _ => "command",
        }
    }
}

type Result<T> = std::result::Result<T, CommandError>;

#[derive(Default)]
pub struct Registry {
    commands: BTreeMap<CommandKey, CommandSnapshot>,
    revision: u64,
    sequence: u64,
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

/// The player's built-in top-level menus: id, title, top-level order. A
/// plugin names one with `@<id>` as the first path segment
/// (`["@video", "Blur"]`), so its command joins that menu whatever the
/// player's language; the registry stores the menu's title. Reference §4.
/// Mirrored by `MENU_IDS` in the SDK package; tests/sdk/host-agreement.test.ts
/// checks the installed SDK agrees.
///
/// A top-level submenu with one of these titles (reached by id or by the
/// title itself) sits at this order, not at its first command: a plugin
/// item inside cannot move the menu. doc/product/menus.md lists the orders.
pub const MENU_ANCHORS: [(&str, &str, i32); 7] = [
    ("open", "打开", 110),
    ("playback", "播放", 200),
    ("subtitles", "字幕", 300),
    ("video", "视频", 400),
    ("audio", "声音", 500),
    ("screen", "屏幕", 710),
    ("settings", "设置", 830),
];

/// Top-level order of a built-in menu, by title.
fn built_in_menu(title: &str) -> Option<i32> {
    MENU_ANCHORS
        .iter()
        .find(|(_, menu, _)| *menu == title)
        .map(|(_, _, order)| *order)
}

/// Replace a leading `@<id>` with the built-in menu's title.
fn resolve_anchor(path: &mut [String]) -> Result<()> {
    let Some(id) = path.first().and_then(|first| first.strip_prefix('@')) else {
        return Ok(());
    };
    let Some((_, title, _)) = MENU_ANCHORS.iter().find(|(anchor, _, _)| *anchor == id) else {
        let known: Vec<_> = MENU_ANCHORS
            .iter()
            .map(|(id, _, _)| format!("@{id}"))
            .collect();
        return Err(CommandError::Invalid(format!(
            "unknown menu id @{id}; the built-in menus are {}",
            known.join(", ")
        )));
    };
    if path.len() < 2 {
        return Err(CommandError::Invalid(format!(
            "menu id @{id} needs an item after it"
        )));
    }
    path[0] = (*title).to_owned();
    Ok(())
}

fn state_valid(state: &CommandState) -> Result<()> {
    if let Some(reason) = &state.reason {
        text(reason, "reason", 512)?;
    }
    Ok(())
}

impl Registry {
    /// Runaway guards, not product limits (reference.md §4). Sized from the
    /// real menus: video registers 71 at idle and all plugins plus the host
    /// about 290, so one owner gets 160 and the registry 1024, enough for
    /// track lists and parallel trial packages. Mirrored by
    /// `COMMAND_LIMITS` in the SDK package (vestra-player-sdk: src/commands.ts);
    /// tests/sdk/host-agreement.test.ts checks the installed SDK agrees.
    pub const MAX_COMMANDS: usize = 1024;
    pub const MAX_PER_OWNER: usize = 160;
    pub const MAX_DEPTH: usize = 8;

    pub fn new() -> Self {
        Self::default()
    }

    fn next_revision(&self) -> Result<u64> {
        self.revision
            .checked_add(1)
            .ok_or(CommandError::RevisionExhausted)
    }

    pub fn register(
        &mut self,
        owner: OwnerGeneration,
        manifest: &str,
        id: String,
        revision: u64,
        mut declaration: Declaration,
        state: CommandState,
    ) -> Result<()> {
        local_id(&id)?;
        resolve_anchor(&mut declaration.path)?;
        if revision == 0 {
            return Err(CommandError::Invalid("command revision starts at 1".into()));
        }
        text(&declaration.title, "title", 128)?;
        if declaration.path.len() > Self::MAX_DEPTH {
            return Err(CommandError::Invalid(
                "menu path allows at most 8 segments".into(),
            ));
        }
        for segment in &declaration.path {
            text(segment, "path segment", 80)?;
        }
        text(&declaration.group, "group", 80)?;
        if declaration.order.unsigned_abs() > 1_000_000 {
            return Err(CommandError::Invalid(
                "order must be within ±1000000".into(),
            ));
        }
        for binding in &declaration.bindings {
            crate::bindings::parse(binding).map_err(CommandError::Invalid)?;
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
        if !declaration.path.is_empty() {
            for entry in self.commands.values() {
                let existing = &entry.declaration.path;
                if existing.is_empty() {
                    continue;
                }
                if existing.starts_with(&declaration.path) || declaration.path.starts_with(existing)
                {
                    return Err(CommandError::Conflict(
                        "menu path duplicates a command or collides with a leaf/group".into(),
                    ));
                }
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
        self.sequence += 1;
        self.commands.insert(
            key.clone(),
            CommandSnapshot {
                key,
                revision,
                declaration,
                state,
                manifest: manifest.into(),
                sequence: self.sequence,
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

    /// Remove one command (its owner disposed it); the local id is free again.
    pub fn remove(&mut self, owner: OwnerGeneration, id: &str) -> Result<()> {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        if !self.commands.contains_key(&key) {
            return Err(CommandError::Missing(key));
        }
        let next = self.next_revision()?;
        self.commands.remove(&key);
        self.revision = next;
        Ok(())
    }

    pub fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot> {
        self.commands.get(&CommandKey {
            owner,
            local_id: id.into(),
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn snapshot(&self) -> MenuSnapshot {
        let entries: Vec<_> = self
            .commands
            .values()
            .filter(|entry| !entry.declaration.path.is_empty())
            .collect();
        MenuSnapshot {
            revision: self.revision,
            roots: tree(&entries, 0, false),
        }
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

type Rank<'a> = (i32, &'a str, u64);

/// `appended`: the level directly inside a built-in menu, where order 0 (the
/// default) means after every ordered item, so a plugin that gives no order
/// lands at the end of the menu rather than above the player's own items.
fn rank(entry: &CommandSnapshot, appended: bool) -> Rank<'_> {
    let order = match entry.declaration.order {
        0 if appended => i32::MAX,
        order => order,
    };
    (order, entry.manifest.as_str(), entry.sequence)
}

/// One level: leaves and submenus, each keyed by its own (or first) command;
/// a built-in top-level menu by its fixed order (`MENU_ANCHORS`).
fn tree(entries: &[&CommandSnapshot], depth: usize, appended: bool) -> Vec<MenuNode> {
    let mut groups: Vec<(String, Vec<&CommandSnapshot>)> = Vec::new();
    let mut nodes: Vec<(Rank<'_>, MenuNode)> = Vec::new();
    for entry in entries {
        let path = &entry.declaration.path;
        if path.len() == depth + 1 {
            nodes.push((
                rank(entry, appended),
                MenuNode::Command {
                    entry: (*entry).clone(),
                },
            ));
        } else if let Some((_, members)) =
            groups.iter_mut().find(|(title, _)| title == &path[depth])
        {
            members.push(entry);
        } else {
            groups.push((path[depth].clone(), vec![entry]));
        }
    }
    for (title, members) in groups {
        let built_in = if depth == 0 {
            built_in_menu(&title)
        } else {
            None
        };
        let first = members
            .iter()
            .copied()
            .min_by(|a, b| rank(a, appended).cmp(&rank(b, appended)))
            .expect("a group has members");
        let key = match built_in {
            Some(order) => (order, "", 0),
            None => rank(first, appended),
        };
        nodes.push((
            key,
            MenuNode::Group {
                group: first.declaration.group.clone(),
                children: tree(&members, depth + 1, built_in.is_some()),
                title,
            },
        ));
    }
    nodes.sort_by(|a, b| a.0.cmp(&b.0));
    nodes.into_iter().map(|(_, node)| node).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(enabled: bool) -> CommandState {
        CommandState {
            enabled,
            checked: None,
            busy: false,
            reason: None,
        }
    }
    fn declaration(path: &[&str]) -> Declaration {
        Declaration::new(path.last().unwrap(), path)
    }
    fn ordered(path: &[&str], order: i32) -> Declaration {
        Declaration {
            order,
            ..declaration(path)
        }
    }
    /// Titles in menu order, depth-first, submenus as "▸title".
    fn flatten(nodes: &[MenuNode], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                MenuNode::Group {
                    title, children, ..
                } => {
                    out.push(format!("▸{title}"));
                    flatten(children, out);
                }
                MenuNode::Command { entry } => out.push(entry.declaration.title.clone()),
            }
        }
    }

    #[test]
    fn local_ids_are_namespaced_but_primary_paths_have_one_owner() {
        let mut registry = Registry::new();
        registry
            .register(
                1,
                "a",
                "open".into(),
                1,
                declaration(&["Playback", "Open"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                2,
                "b",
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
                "c",
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
                "a",
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
            let mut registry = Registry::new();
            let short = declaration(&["Tools", "Inspect"]);
            let long = declaration(&["Tools", "Inspect", "Details"]);
            let (first, second) = if leaf_first {
                (short, long)
            } else {
                (long, short)
            };
            registry
                .register(1, "a", "first".into(), 1, first, state(true))
                .unwrap();
            let before = registry.snapshot();
            assert!(matches!(
                registry.register(2, "b", "second".into(), 1, second, state(true)),
                Err(CommandError::Conflict(_))
            ));
            assert_eq!(registry.snapshot(), before);
        }
    }

    #[test]
    fn stale_state_cannot_reenable_a_disabled_command() {
        let mut registry = Registry::new();
        registry
            .register(
                1,
                "a",
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
        let mut registry = Registry::new();
        registry
            .register(
                10,
                "a",
                "run".into(),
                1,
                declaration(&["Tools", "Old"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                11,
                "a",
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
        assert_eq!(registry.resolve_execution(11, "run").unwrap().owner, 11);
        registry
            .register(
                12,
                "a",
                "reuse".into(),
                1,
                declaration(&["Tools", "Old"]),
                state(true),
            )
            .unwrap();
        assert_eq!(registry.resolve_execution(12, "reuse").unwrap().owner, 12);
    }

    #[test]
    fn order_sorts_siblings_and_submenus_take_their_first_command() {
        let mut registry = Registry::new();
        for (owner, id, path, order) in [
            (1, "z", vec!["Zulu"], 10),
            (2, "deep", vec!["Alpha", "Sub", "Deep"], 3),
            (1, "a", vec!["Alpha", "Item"], 5),
            (3, "top", vec!["Top"], 1),
        ] {
            registry
                .register(owner, "m", id.into(), 1, ordered(&path, order), state(true))
                .unwrap();
        }
        let mut titles = Vec::new();
        flatten(&registry.snapshot().roots, &mut titles);
        assert_eq!(titles, ["Top", "▸Alpha", "▸Sub", "Deep", "Item", "Zulu"]);
    }

    #[test]
    fn equal_orders_fall_back_to_manifest_then_declaration_not_title() {
        let mut registry = Registry::new();
        registry
            .register(
                9,
                "b.plugin",
                "x".into(),
                1,
                declaration(&["Z first"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                9,
                "b.plugin",
                "y".into(),
                1,
                declaration(&["A second"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                4,
                "a.plugin",
                "w".into(),
                1,
                declaration(&["M zero"]),
                state(true),
            )
            .unwrap();
        let mut titles = Vec::new();
        flatten(&registry.snapshot().roots, &mut titles);
        assert_eq!(titles, ["M zero", "Z first", "A second"]);
    }

    /// An English plugin's `["@video", "Blur"]` lands in the same 视频 menu
    /// as the built-in video plugin's `["视频", …]` commands.
    #[test]
    fn menu_ids_name_the_built_in_menus_in_any_language() {
        let mut registry = Registry::new();
        registry
            .register(
                1,
                "video",
                "rotate".into(),
                1,
                ordered(&["视频", "旋转"], 400),
                state(true),
            )
            .unwrap();
        registry
            .register(
                2,
                "examples.blur",
                "toggle".into(),
                1,
                ordered(&["@video", "Blur"], 401),
                state(true),
            )
            .unwrap();
        assert_eq!(
            registry.get(2, "toggle").unwrap().declaration.path,
            ["视频", "Blur"]
        );
        let roots = registry.snapshot().roots;
        let mut titles = Vec::new();
        flatten(&roots, &mut titles);
        assert_eq!(titles, ["▸视频", "旋转", "Blur"]);
        // The same leaf again collides whichever way the menu is named.
        assert!(matches!(
            registry.register(
                3,
                "c",
                "again".into(),
                1,
                declaration(&["视频", "Blur"]),
                state(true)
            ),
            Err(CommandError::Conflict(_))
        ));
        for bad in [&["@view", "X"][..], &["@video"][..], &["@", "X"][..]] {
            assert!(
                matches!(
                    registry.register(3, "c", "bad".into(), 1, declaration(bad), state(true)),
                    Err(CommandError::Invalid(_))
                ),
                "{bad:?}"
            );
        }
        // Only the first segment names a menu; later ones are titles.
        registry
            .register(
                3,
                "c",
                "at".into(),
                1,
                declaration(&["@audio", "@home", "X"]),
                state(true),
            )
            .unwrap();
        assert_eq!(
            registry.get(3, "at").unwrap().declaration.path,
            ["声音", "@home", "X"]
        );
        assert_eq!(MENU_ANCHORS.len(), 7);
    }

    /// A plugin item in a built-in menu, with no `order` (0), does not move
    /// the menu: 视频 stays between 字幕 and 声音 and the item goes last.
    #[test]
    fn plugin_items_do_not_move_built_in_menus() {
        let mut registry = Registry::new();
        for (owner, manifest, id, path, order) in [
            (1, "subtitles", "show", vec!["字幕", "显示字幕"], 301),
            (
                2,
                "video",
                "rotate",
                vec!["视频", "图像旋转", "按序切换"],
                405,
            ),
            (2, "video", "reset", vec!["视频", "复位"], 440),
            (3, "audio", "eq", vec!["声音", "均衡器"], 501),
            (4, "examples.blur", "blur", vec!["@video", "Blur…"], 0),
            (5, "examples.literal", "sharpen", vec!["视频", "Sharpen"], 0),
            (6, "examples.first", "first", vec!["@video", "First"], -5),
            (7, "examples.deep", "a", vec!["@audio", "Tools", "A"], 0),
            (7, "examples.deep", "b", vec!["@audio", "Tools", "B"], 0),
            (8, "examples.top", "top", vec!["Tools"], 0),
            (9, "examples.settings", "x", vec!["@settings", "X"], 0),
        ] {
            registry
                .register(
                    owner,
                    manifest,
                    id.into(),
                    1,
                    ordered(&path, order),
                    state(true),
                )
                .unwrap();
        }
        let mut titles = Vec::new();
        flatten(&registry.snapshot().roots, &mut titles);
        assert_eq!(
            titles,
            [
                // A plugin's own top-level item still sorts by its order.
                "Tools",
                "▸字幕",
                "显示字幕",
                "▸视频",
                // A negative order puts an item first inside, nothing more.
                "First",
                "▸图像旋转",
                "按序切换",
                "复位",
                // Order 0 by id or by title: after the menu's own items.
                "Blur…",
                "Sharpen",
                "▸声音",
                "均衡器",
                "▸Tools",
                // Below the plugin's own submenu, 0 is an ordinary order.
                "A",
                "B",
                // Only plugins fill 设置 here: it still sits at its own order.
                "▸设置",
                "X",
            ]
        );
    }

    #[test]
    fn groups_label_submenus_and_pathless_commands_stay_out_of_the_menu() {
        let mut registry = Registry::new();
        let mut open = ordered(&["打开", "文件…"], 100);
        open.group = "open".into();
        let mut subtitle = ordered(&["打开", "字幕…"], 150);
        subtitle.group = "subtitle".into();
        let mut hidden = Declaration::new("全选", &[]);
        hidden.bindings = vec!["Ctrl+A".into()];
        registry
            .register(1, "m", "open".into(), 1, open, state(true))
            .unwrap();
        registry
            .register(1, "m", "sub".into(), 1, subtitle, state(true))
            .unwrap();
        registry
            .register(1, "m", "all".into(), 1, hidden.clone(), state(true))
            .unwrap();
        registry
            .register(1, "m", "all2".into(), 1, hidden, state(true))
            .unwrap();
        let roots = registry.snapshot().roots;
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].group(), "open");
        let MenuNode::Group { children, .. } = &roots[0] else {
            panic!()
        };
        assert_eq!(
            children.iter().map(MenuNode::group).collect::<Vec<_>>(),
            ["open", "subtitle"]
        );
        assert!(registry.get(1, "all").is_some());
        let mut bad = Declaration::new("x", &["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
        assert!(matches!(
            registry.register(1, "m", "deep".into(), 1, bad.clone(), state(true)),
            Err(CommandError::Invalid(_))
        ));
        bad.path.truncate(1);
        bad.bindings = vec!["Shift+>".into()];
        assert!(matches!(
            registry.register(1, "m", "keys".into(), 1, bad, state(true)),
            Err(CommandError::Invalid(_))
        ));
    }

    #[test]
    fn removing_a_command_frees_its_id_and_path() {
        let mut registry = Registry::new();
        registry
            .register(
                3,
                "m",
                "track.0".into(),
                1,
                declaration(&["字幕", "轨 A"]),
                state(true),
            )
            .unwrap();
        let before = registry.revision();
        registry.remove(3, "track.0").unwrap();
        assert!(registry.revision() > before);
        assert!(registry.snapshot().roots.is_empty());
        assert!(matches!(
            registry.remove(3, "track.0"),
            Err(CommandError::Missing(_))
        ));
        registry
            .register(
                3,
                "m",
                "track.0".into(),
                1,
                declaration(&["字幕", "轨 B"]),
                state(true),
            )
            .unwrap();
        registry
            .register(
                4,
                "n",
                "other".into(),
                1,
                declaration(&["字幕", "轨 A"]),
                state(true),
            )
            .unwrap();
    }

    #[test]
    fn limits_reject_new_work() {
        let mut limited = Registry::new();
        for index in 0..Registry::MAX_PER_OWNER {
            limited
                .register(
                    5,
                    "m",
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
                "m",
                "extra".into(),
                1,
                declaration(&["Tools", "Extra"]),
                state(true)
            ),
            Err(CommandError::Limit)
        );
        // The per-owner cap is per owner: another owner is unaffected.
        limited
            .register(
                6,
                "n",
                "other".into(),
                1,
                declaration(&["Tools", "Other"]),
                state(true),
            )
            .unwrap();
    }

    #[test]
    fn registry_total_limit_spans_owners() {
        let mut full = Registry::new();
        for index in 0..Registry::MAX_COMMANDS {
            let owner = (index / Registry::MAX_PER_OWNER) as u64 + 1;
            full.register(
                owner,
                "m",
                format!("c{index}"),
                1,
                Declaration::new("C", &[]),
                state(true),
            )
            .unwrap();
        }
        assert_eq!(
            full.register(
                999,
                "fresh",
                "c".into(),
                1,
                Declaration::new("C", &[]),
                state(true)
            ),
            Err(CommandError::Limit)
        );
        assert_eq!(full.remove(1, "c0"), Ok(()));
        full.register(
            999,
            "fresh",
            "c".into(),
            1,
            Declaration::new("C", &[]),
            state(true),
        )
        .unwrap();
    }

    /// The menus of the shipped plugins fit with room to spare
    /// (doc/process/rounds/menus-snapshot.test.ts: video 71, all ~290).
    #[test]
    fn limits_leave_room_for_real_plugins() {
        const VIDEO_AT_IDLE: usize = 71;
        const ALL_AT_IDLE: usize = 292;
        const { assert!(Registry::MAX_PER_OWNER >= 2 * VIDEO_AT_IDLE) };
        const { assert!(Registry::MAX_COMMANDS >= 3 * ALL_AT_IDLE) };
    }

    /// core-ui-design.md §14: the migration orders reproduce the tree that the
    /// old registry (fixed domain list, then UTF-8 byte order of path[1..])
    /// built from the same 46 commands (doc/process/rounds/menus-before.json).
    #[test]
    fn migration_orders_reproduce_the_old_menu_tree() {
        const TABLE: &[(&str, &str, i32)] = &[
            ("vestra.playback", "文件/关闭当前来源", 201),
            ("vestra.playback", "文件/打开媒体…", 202),
            ("vestra.playlist", "文件/播放列表/上一项", 203),
            ("vestra.playlist", "文件/播放列表/上移", 204),
            ("vestra.playlist", "文件/播放列表/下一项", 205),
            ("vestra.playlist", "文件/播放列表/下移", 206),
            ("vestra.playlist", "文件/播放列表/播放选中项", 207),
            ("vestra.playlist", "文件/播放列表/添加文件…", 208),
            ("vestra.playlist", "文件/播放列表/添加链接…", 209),
            ("vestra.playlist", "文件/播放列表/清空列表", 210),
            ("vestra.playlist", "文件/播放列表/移除选中项", 211),
            ("zysilm.bilibili", "文件/网络媒体/Bilibili 视频…", 212),
            ("zysilm.youtube", "文件/网络媒体/YouTube 视频…", 213),
            ("vestra.playback", "播放/前进5秒", 261),
            ("vestra.playback", "播放/后退5秒", 262),
            ("vestra.playback", "播放/播放/暂停", 263),
            ("vestra.playback", "播放/跳转到…", 264),
            ("vestra.playback", "播放/速度/0.5倍", 265),
            ("vestra.playback", "播放/速度/1.5倍", 266),
            ("vestra.playback", "播放/速度/2倍", 267),
            ("vestra.playback", "播放/速度/正常速度", 268),
            ("vestra.playback", "播放/速度/自定义…", 269),
            ("vestra.capture", "视频/截图与导出/保存当前画面…", 321),
            ("vestra.capture", "视频/截图与导出/导出片段…", 322),
            ("vestra.capture", "视频/截图与导出/查看导出任务…", 323),
            ("vestra.capture", "视频/截图与导出/连拍…", 324),
            ("vestra.video", "视频/画面调节…", 325),
            ("vestra.video", "视频/高斯模糊…", 326),
            ("vestra.audio", "音频/均衡器预设/人声", 381),
            ("vestra.audio", "音频/均衡器预设/平直", 382),
            ("vestra.audio", "音频/均衡器预设/影院", 383),
            ("vestra.audio", "音频/均衡器预设/音乐", 384),
            ("vestra.audio", "音频/复位音频处理", 385),
            ("vestra.playback", "音频/静音", 386),
            ("vestra.playback", "音频/音量…", 387),
            ("vestra.audio", "音频/音频工作台…", 388),
            ("vestra.subtitles", "字幕/关闭字幕", 441),
            ("vestra.subtitles", "字幕/刷新字幕轨", 442),
            ("vestra.subtitles", "字幕/加载外部 SRT…", 443),
            ("vestra.subtitles", "字幕/字幕工作台…", 444),
            ("vestra.playlist", "视图/显示播放列表", 501),
            ("vestra.playback", "视图/显示播放栏", 502),
            ("vestra.nes", "模拟器/NES 工具", 561),
            ("vestra.nes", "模拟器/停止 NES", 562),
            ("vestra.nes", "模拟器/打开 NES ROM…", 563),
            ("vestra.workshop", "插件/插件工坊…", 621),
        ];
        // "播放/暂停" is one leaf title that contains a slash. The old top-level
        // titles get a prefix: 播放, 视频 and 字幕 are built-in menus now, placed
        // by MENU_ANCHORS rather than by their commands' orders.
        let split = |path: &str| -> Vec<String> {
            let path = path.replace("播放/暂停", "播放\u{0}暂停");
            let mut path: Vec<String> = path.split('/').map(|s| s.replace('\u{0}', "/")).collect();
            path[0] = format!("旧{}", path[0]);
            path
        };
        // The old algorithm, reimplemented: domain rank, then byte order of path[1..].
        let domains = [
            "文件",
            "播放",
            "视频",
            "音频",
            "字幕",
            "视图",
            "模拟器",
            "插件",
        ];
        let mut old: Vec<Vec<String>> = TABLE.iter().map(|(_, path, _)| split(path)).collect();
        old.sort_by(|a, b| {
            let rank = |p: &Vec<String>| {
                domains
                    .iter()
                    .position(|d| format!("旧{d}") == p[0])
                    .unwrap()
            };
            rank(a).cmp(&rank(b)).then_with(|| a[1..].cmp(&b[1..]))
        });
        let mut expected = Vec::new();
        let mut open: Vec<String> = Vec::new();
        for path in &old {
            let common = open.iter().zip(path).take_while(|(a, b)| a == b).count();
            for segment in &path[common..path.len() - 1] {
                expected.push(format!("▸{segment}"));
            }
            expected.push(path.last().unwrap().clone());
            open = path[..path.len() - 1].to_vec();
        }
        // Register in reverse and with owners interleaved: timing must not matter.
        let mut registry = Registry::new();
        for (index, (manifest, path, order)) in TABLE.iter().enumerate().rev() {
            let path = split(path);
            let refs: Vec<&str> = path.iter().map(String::as_str).collect();
            registry
                .register(
                    (index % 7) as u64 + 1,
                    manifest,
                    format!("c{index}"),
                    1,
                    ordered(&refs, *order),
                    state(true),
                )
                .unwrap();
        }
        let mut actual = Vec::new();
        flatten(&registry.snapshot().roots, &mut actual);
        assert_eq!(actual, expected);
        assert_eq!(TABLE.len(), 46);
    }
}
