//! Command registry: the shared menu model (core-ui-design.md §4).
//! Not a menu renderer or key-binding engine (see menu.rs and bindings.rs).
//! Callers authenticate live owner generations before registration and route
//! resolved execution to that exact instance.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::conditions::{Conditions, Facts};

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
    /// What the command is about (`CommandOptions.category`, SDK 1.6.4):
    /// English source text, the key; the command palette names a command
    /// `Category: Title` (`display.category` translated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The local id of the owner's command that opens where this one lives
    /// (`CommandOptions.home`, SDK 1.6.4): the palette shows that place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Running it cannot be taken back (`CommandOptions.destructive`):
    /// a run through `commands.run` asks the user first
    /// (api/commands_run.rs). Optional; absent is `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub destructive: bool,
    /// What the menu shows (doc/architecture/i18n.md §5), in the interface
    /// language; `title` and `path` stay as declared (English source text),
    /// the keys menus merge by. Filled by the player (`localize`), never by
    /// the plugin; empty = show the keys.
    #[serde(skip)]
    pub display: Display,
    /// A bottom-bar button (`CommandOptions.button`, player 0.5.10): read
    /// apart from the rest by the `commands` capability (its image is
    /// checked against the plugin's directories), so here it is skipped.
    #[serde(skip)]
    pub button: Option<Button>,
}

/// A command's bottom-bar button: the cell `BarButtons`
/// (`@vestra/bar-buttons.slint`) shows for it (bar_buttons.rs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub icon: ButtonIcon,
    /// Left to right, ascending, 0..=[`Registry::MAX_BUTTON_ORDER`].
    pub order: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ButtonIcon {
    /// One of [`BUTTON_ICONS`].
    Name(String),
    /// A plugin image (`plugin.ui.image`): its path and `maxSide` (0: none).
    Image { path: String, max_side: u32 },
}

/// The names of `@vestra/icons.slint` (`Icons.commands`), the icons a
/// button may name. Mirrored by `BAR_BUTTON_ICONS` in the SDK package;
/// tests/sdk/host-agreement.test.ts checks the installed SDK agrees, and a
/// test here that the list is the icon set's.
pub const BUTTON_ICONS: [&str; 40] = [
    "play",
    "pause",
    "stop",
    "back",
    "forward",
    "open",
    "volume",
    "mute",
    "playlist",
    "detach",
    "attach",
    "close",
    "failed",
    "move-top",
    "move-up",
    "move-down",
    "move-bottom",
    "search",
    "chapter",
    "bookmark",
    "check-task",
    "check-box",
    "menu-check",
    "submenu",
    "scroll-up",
    "scroll-down",
    "caret-down",
    "sort-asc",
    "sort-desc",
    "tag-remove",
    "preview-play",
    "add",
    "remove",
    "sort",
    "fullscreen",
    "exit-fullscreen",
    "store",
    "ai",
    "sliders",
    "search-ai",
];

/// The shown text of a [`Declaration`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Display {
    pub title: String,
    /// One per path segment.
    pub path: Vec<String>,
    /// The category; empty without one (or untranslated: the key shows).
    pub category: String,
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
            category: None,
            home: None,
            destructive: false,
            display: Display::default(),
            button: None,
        }
    }
    /// Out of the menu (command palette and Keyboard Shortcuts only), keeping
    /// its title, shown title and category.
    pub fn without_path(mut self) -> Self {
        self.path.clear();
        self.display.path.clear();
        self
    }
    /// The menu text of this command.
    pub fn shown_title(&self) -> &str {
        if self.display.title.is_empty() {
            &self.title
        } else {
            &self.display.title
        }
    }
    /// The shown path (the keys where nothing was translated).
    pub fn shown_path(&self) -> Vec<String> {
        (0..self.path.len())
            .map(|i| self.shown_segment(i).to_owned())
            .collect()
    }
    /// The shown category, `None` without one.
    pub fn shown_category(&self) -> Option<&str> {
        let key = self.category.as_deref()?;
        Some(if self.display.category.is_empty() {
            key
        } else {
            &self.display.category
        })
    }
    fn shown_segment(&self, index: usize) -> &str {
        self.display
            .path
            .get(index)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.path[index])
    }
    /// Fill `display` from `text` (the declaring plugin's catalog): the
    /// title and each segment, except the player's own submenus
    /// ([`host_group_label`]), which keep the player's words whoever
    /// declares into them. Call after [`normalize`].
    pub fn localize(&mut self, text: impl Fn(&str) -> String) {
        let declared = self.path.clone();
        self.localize_declared(&declared, text);
    }
    /// [`Declaration::localize`] for a path [`normalize`] changed: the shown
    /// text comes from the segments as `declared` (an old Chinese title
    /// stays itself), the player's submenus from the keys.
    pub fn localize_declared(&mut self, declared: &[String], text: impl Fn(&str) -> String) {
        let path = &self.path;
        self.display.path = (0..path.len())
            .map(|i| {
                (i + 1 < path.len())
                    .then(|| host_group_label(&path[..=i]))
                    .flatten()
                    .unwrap_or_else(|| text(declared.get(i).unwrap_or(&path[i])))
            })
            .collect();
        self.display.title = text(&self.title);
        self.display.category = self.category.as_deref().map(&text).unwrap_or_default();
    }
    /// Overwrite the shown category when it was a text value (its key is
    /// the English rendering). Call after [`Declaration::localize_declared`].
    pub fn show_category(&mut self, category: Option<String>) {
        if let Some(category) = category {
            self.display.category = category;
        }
    }
    /// Overwrite the shown title and segments that were text values (their
    /// keys are the English renderings): `title`, and `path` by index.
    /// Call after [`Declaration::localize_declared`].
    pub fn show_texts(&mut self, title: Option<String>, path: &[Option<String>]) {
        if let Some(title) = title {
            self.display.title = title;
        }

        for (index, shown) in path.iter().enumerate() {
            if let (Some(shown), Some(slot)) = (shown, self.display.path.get_mut(index)) {
                *slot = shown.clone();
            }
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
        /// The key: the declared segment (English source text).
        title: String,
        /// What the menu shows: the player's word for its own submenus,
        /// else the first command's (in menu order) translation.
        label: String,
        /// Taken from the first command (in menu order) below this group.
        group: String,
        children: Vec<MenuNode>,
    },
    Command {
        /// Boxed: a snapshot is far larger than a group.
        entry: Box<CommandSnapshot>,
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
    /// Advances when a command comes, goes or is registered again with
    /// another declaration or manifest: what `bindings.list` shows of the
    /// registry. Not on a state update, nor when a woken plugin's commands
    /// move to its new owner generation (`rekey`) and it registers them
    /// again as they were (supervisor.md).
    listed: u64,
    sequence: u64,
    /// Standby (doc/architecture/supervisor.md §4):
    /// entries kept for an owner without a process (parked, or replayed from
    /// its recording). A live registration of the same key replaces one.
    recorded: std::collections::BTreeSet<CommandKey>,
    /// Each command as its plugin sent it (`declaration`, last `state`),
    /// for its recording.
    sent: BTreeMap<CommandKey, Sent>,
    /// Commands whose `enabled` or `checked` is a condition on the
    /// player's state (`plugin.when`, crate::conditions), evaluated here.
    conditions: BTreeMap<CommandKey, Conditions>,
    /// What the conditions were last evaluated against.
    facts: Facts,
    /// The keys of `commands` by menu path (non-empty paths only), so a
    /// registration finds the paths it collides with, its own and those
    /// above and below it, without scanning every command.
    paths: BTreeMap<Vec<String>, Vec<CommandKey>>,
    /// The last [`Registry::snapshot`], valid while `revision` is its own.
    /// The player builds its menu tree from a [`View`] (rule R8), once per
    /// copy; the registry's own tree and cache are its tests'.
    #[cfg(test)]
    menu: std::sync::Mutex<Option<Arc<MenuSnapshot>>>,
    /// A test's registry total in place of [`Registry::MAX_COMMANDS`].
    #[cfg(test)]
    max_commands: Option<usize>,
    /// Owners whose commands show disabled with this reason, whatever
    /// their state: a plugin held in standby after a crash until its
    /// restart hold is over (supervisor.md §7). Menus show
    /// them greyed out, and a key or click shows the reason.
    held: BTreeMap<OwnerGeneration, String>,
}

/// A command's registration and last state in the form its plugin sent
/// them (before localization): what a recording replays.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub declaration: serde_json::Value,
    pub state: serde_json::Value,
    /// Its conditions (`when`), `Null` without.
    pub when: serde_json::Value,
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

/// The player's built-in top-level menus: id, key, top-level order. A
/// plugin names one with `@<id>` as the first path segment
/// (`["@video", "Blur"]`) or with its key (`["Video", "Blur"]`); either way
/// its command joins that menu, shown in the interface language
/// ([`host_group_label`]). Reference §4. Mirrored by `MENU_IDS` in the SDK
/// package; tests/sdk/host-agreement.test.ts checks the installed SDK agrees.
///
/// A top-level submenu with one of these keys sits at this order, not at its
/// first command: a plugin item inside cannot move the menu.
/// doc/product/menus.md lists the orders. Reset (`@reset`) gathers every
/// reset-to-default command, directly below Settings and in its separator
/// group (`SHARES_GROUP_WITH`); like every built-in menu it shows only once
/// a command is in it.
///
/// Tools (`@tools`, the short menu, user ruling 2026-11-11): the
/// right-click menu's home for the built-in plugins' tools (tracks,
/// snapshot, the chapter list, Vestra Player AI, the store, the command
/// palette, About) and any plugin's own (the SDK's examples already name a
/// `Tools` menu). After the domain menus and before Options… (830), in the
/// settings group (`FIXED_GROUPS`). Appended, not inserted by order: the
/// SDK's `MENU_IDS` grows by additions only.
pub const MENU_ANCHORS: [(&str, &str, i32); 9] = [
    ("open", "Open", 110),
    ("playback", "Playback", 200),
    ("subtitles", "Subtitles", 300),
    ("video", "Video", 400),
    ("audio", "Audio", 500),
    ("screen", "Screen", 710),
    ("settings", "Settings", 830),
    ("reset", "Reset", 840),
    ("tools", "Tools", 820),
];

/// Built-in top-level menus drawn in a fixed separator group (key, group),
/// whatever groups their commands declare: Tools and Settings sit with
/// Options… and Quit (`settings-main`), so a plugin's first item inside
/// cannot draw a separator between them. Reset follows Settings
/// (`SHARES_GROUP_WITH`).
const FIXED_GROUPS: [(&str, &str); 2] = [("Tools", "settings-main"), ("Settings", "settings-main")];

/// Built-in top-level menus drawn in the separator group of another (key,
/// key it joins): Reset belongs with Settings, whatever groups the commands
/// inside either declare.
const SHARES_GROUP_WITH: [(&str, &str); 1] = [("Reset", "Settings")];

/// The player's Chinese menu titles from before English keys (players up
/// to 0.4.2): a plugin that still declares one lands in the same menu.
/// Parent path, old title, key. The last entries are submenus that a
/// migrated built-in plugin shares with plugins not migrated yet (the need
/// goes once every plugin declares English keys; doc/product/menus.md has
/// them).
const LEGACY_MENUS: [(&[&str], &str, &str); 10] = [
    (&[], "打开", "Open"),
    (&[], "播放", "Playback"),
    (&[], "字幕", "Subtitles"),
    (&[], "视频", "Video"),
    (&[], "声音", "Audio"),
    (&[], "屏幕", "Screen"),
    (&[], "设置", "Settings"),
    (&["Playback"], "速度", "Speed"),
    (&["Playback"], "跳转", "Seek"),
    (&["Settings"], "外观", "Appearance"),
];

/// How the player shows its own submenus (the built-in menus and the
/// submenus of its core actions), whoever declares commands into them.
pub fn host_group_label(path: &[String]) -> Option<String> {
    use crate::i18n::tr;
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    Some(match path.as_slice() {
        ["Open"] => tr("Open"),
        ["Playback"] => tr("Playback"),
        ["Subtitles"] => tr("Subtitles"),
        ["Video"] => tr("Video"),
        ["Audio"] => tr("Audio"),
        ["Screen"] => tr("Screen"),
        ["Settings"] => tr("Settings"),
        ["Reset"] => tr("Reset"),
        ["Tools"] => tr("Tools"),
        ["Playback", "Speed"] => tr("Speed"),
        ["Playback", "Seek"] => tr("Seek"),
        _ => return None,
    })
}

/// Top-level order of a built-in menu, by key.
fn built_in_menu(title: &str) -> Option<i32> {
    MENU_ANCHORS
        .iter()
        .find(|(_, menu, _)| *menu == title)
        .map(|(_, _, order)| *order)
}

/// The keys a path merges by: a leading `@<id>` becomes the built-in menu's
/// key, and the player's old Chinese titles of its own menus become theirs
/// (`LEGACY_MENUS`). Idempotent.
pub fn normalize(path: &mut [String]) -> Result<()> {
    if let Some(id) = path.first().and_then(|first| first.strip_prefix('@')) {
        let Some((_, key, _)) = MENU_ANCHORS.iter().find(|(anchor, _, _)| *anchor == id) else {
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
        path[0] = (*key).to_owned();
    }
    for (parent, old, key) in LEGACY_MENUS {
        let depth = parent.len();
        // Only a submenu (a segment with something below it) is renamed.
        if path.len() > depth + 1 && path[..depth] == *parent && path[depth] == old {
            path[depth] = key.to_owned();
        }
    }
    Ok(())
}

/// A button needs a menu entry (where the command stays reachable from a
/// bar without `BarButtons`), an order in range and a known icon.
fn button_valid(button: &Button, path: &[String]) -> Result<()> {
    if path.is_empty() {
        return Err(CommandError::Invalid(
            "button needs a menu path: a bar button is also a menu item".into(),
        ));
    }
    if !(0..=Registry::MAX_BUTTON_ORDER).contains(&button.order) {
        return Err(CommandError::Invalid(format!(
            "button order must be an integer 0–{}",
            Registry::MAX_BUTTON_ORDER
        )));
    }
    match &button.icon {
        ButtonIcon::Name(name) if !BUTTON_ICONS.contains(&name.as_str()) => {
            Err(CommandError::Invalid(format!(
                "unknown button icon {name:?}: a name of @vestra/icons.slint ({}) or an image from plugin.ui.image",
                BUTTON_ICONS.join(", ")
            )))
        }
        ButtonIcon::Image { path, .. } if path.is_empty() => Err(CommandError::Invalid(
            "button image needs a file path".into(),
        )),
        _ => Ok(()),
    }
}

/// The bottom-bar buttons, as [`View::buttons`] lays them out.
#[derive(Debug, Default)]
pub struct Buttons<'a> {
    /// The strip's, left to right: ascending order, then manifest id, then
    /// registration sequence; at most [`Registry::MAX_BUTTONS`].
    pub shown: Vec<&'a CommandSnapshot>,
    /// Past [`Registry::MAX_BUTTONS`], in the same order: the strip's `»`
    /// menu (menu redesign step 3a; before, they were left out).
    pub overflow: Vec<&'a CommandSnapshot>,
    /// A plugin's buttons after its first: no button (the command and its
    /// menu entry stay), with the limit's words.
    pub dropped: Vec<(&'a CommandSnapshot, &'static str)>,
}

fn state_valid(state: &CommandState) -> Result<()> {
    if let Some(reason) = &state.reason {
        text(reason, "reason", 512)?;
    }
    Ok(())
}

impl Registry {
    /// Runaway guards, not product limits (SDK site, Reference: Limits). Sized from the
    /// real menus: video registers 71 at idle and all plugins plus the host
    /// about 290, so one owner gets 160 and the registry 1024, enough for
    /// track lists and several plugins at once. Mirrored by
    /// `COMMAND_LIMITS` in the SDK package (vestra-player-sdk: src/commands.ts);
    /// tests/sdk/host-agreement.test.ts checks the installed SDK agrees.
    pub const MAX_COMMANDS: usize = 1024;
    pub const MAX_PER_OWNER: usize = 160;
    pub const MAX_DEPTH: usize = 8;
    /// Bottom-bar buttons (`CommandOptions.button`): one per plugin (a
    /// plugin's later ones are left out; their commands and menu entries
    /// stay), eight in the strip, the rest in its `»` menu. `order` is
    /// 0..=999. Mirrored by
    /// `BAR_BUTTON_LIMITS` in the SDK package; tests/sdk/host-agreement.test.ts
    /// checks the installed SDK agrees.
    pub const MAX_BUTTONS: usize = 8;
    pub const MAX_BUTTONS_PER_OWNER: usize = 1;
    pub const MAX_BUTTON_ORDER: i32 = 999;

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
        normalize(&mut declaration.path)?;
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
        if let Some(category) = &declaration.category {
            text(category, "category", 80)?;
        }
        if let Some(home) = &declaration.home {
            text(home, "home", 128)?;
            if !home
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'-'))
            {
                return Err(CommandError::Invalid(
                    "home permits ASCII letters, digits, '.', '_' and '-'".into(),
                ));
            }
        }
        if declaration.order.unsigned_abs() > 1_000_000 {
            return Err(CommandError::Invalid(
                "order must be within ±1000000".into(),
            ));
        }
        if let Some(button) = &declaration.button {
            button_valid(button, &declaration.path)?;
        }
        // Parsed and without repeats, as the binding table declares them: a
        // binding it would refuse refuses the registration before anything
        // changes (SDK 1.5.48 review: no menu item behind a failed RPC).
        crate::bindings::Table::canonical(&declaration.bindings).map_err(CommandError::Invalid)?;
        state_valid(&state)?;
        let key = CommandKey {
            owner,
            local_id: id,
        };
        // A recorded entry of the same key is replaced; its menu place (the
        // registration sequence) is kept.
        let replaced = self.recorded.contains(&key);
        if self.commands.contains_key(&key) && !replaced {
            return Err(CommandError::Conflict(
                "owner/localId already registered".into(),
            ));
        }
        // The entry this replaces and the same owner's other recorded
        // entries (the live one wins below) do not collide.
        let colliding = self.colliding(&declaration.path);
        if colliding
            .iter()
            .any(|each| **each != key && !(each.owner == owner && self.recorded.contains(*each)))
        {
            return Err(CommandError::Conflict(
                "menu path duplicates a command or collides with a leaf/group".into(),
            ));
        }
        if !replaced
            && (self.commands.len() >= self.max_commands()
                || self.keys_of(owner).take(Self::MAX_PER_OWNER).count() >= Self::MAX_PER_OWNER)
        {
            return Err(CommandError::Limit);
        }
        let next = self.next_revision()?;
        // The same owner's recorded entries on this path give way.
        let colliding = colliding
            .into_iter()
            .filter(|each| **each != key)
            .cloned()
            .collect::<Vec<_>>();
        // The same command registered again as it was (a woken plugin's
        // live registration of its recorded one) lists nothing new.
        let same = colliding.is_empty()
            && self.commands.get(&key).is_some_and(|entry| {
                replaced && entry.manifest == manifest && entry.declaration == declaration
            });
        if !same {
            self.listed += 1;
        }
        for gone in colliding {
            self.remove_entry(&gone);
            self.recorded.remove(&gone);
            self.sent.remove(&gone);
            self.conditions.remove(&gone);
        }
        let sequence = match self.commands.get(&key).filter(|_| replaced) {
            Some(entry) => entry.sequence,
            None => {
                self.sequence += 1;
                self.sequence
            }
        };
        self.recorded.remove(&key);
        self.sent.remove(&key);
        self.conditions.remove(&key);
        self.insert_entry(CommandSnapshot {
            key,
            revision,
            declaration,
            state,
            manifest: manifest.into(),
            sequence,
        });
        self.revision = next;
        Ok(())
    }

    fn max_commands(&self) -> usize {
        #[cfg(test)]
        if let Some(max) = self.max_commands {
            return max;
        }
        Self::MAX_COMMANDS
    }

    /// The keys of `owner`'s entries, in local id order.
    fn keys_of(&self, owner: OwnerGeneration) -> impl Iterator<Item = &CommandKey> {
        let first = CommandKey {
            owner,
            local_id: String::new(),
        };
        self.commands
            .range(first..)
            .map(|(key, _)| key)
            .take_while(move |key| key.owner == owner)
    }

    /// The entries whose menu path collides with `path`: the same path, a
    /// path below it (`path` would be their group) or above it (`path`
    /// would be inside their leaf). None for an empty path.
    fn colliding(&self, path: &[String]) -> Vec<&CommandKey> {
        if path.is_empty() {
            return Vec::new();
        }
        // Paths starting with `path` sort together, from `path` itself on.
        let below = self
            .paths
            .range::<[String], _>((Bound::Included(path), Bound::Unbounded))
            .take_while(|(existing, _)| existing.starts_with(path));
        let above = (1..path.len()).filter_map(|n| self.paths.get_key_value(&path[..n]));
        below.chain(above).flat_map(|(_, keys)| keys).collect()
    }

    /// Stores an entry, replacing the one with its key, and indexes its path.
    fn insert_entry(&mut self, entry: CommandSnapshot) {
        self.remove_entry(&entry.key);
        if !entry.declaration.path.is_empty() {
            self.paths
                .entry(entry.declaration.path.clone())
                .or_default()
                .push(entry.key.clone());
        }
        self.commands.insert(entry.key.clone(), entry);
    }

    /// Removes an entry and its path from the index.
    fn remove_entry(&mut self, key: &CommandKey) -> Option<CommandSnapshot> {
        let entry = self.commands.remove(key)?;
        let path = &entry.declaration.path;
        if let Some(keys) = self.paths.get_mut(path) {
            keys.retain(|each| each != key);
            if keys.is_empty() {
                self.paths.remove(path);
            }
        }
        Some(entry)
    }

    /// Keep what the plugin sent for `id`, for its recording: the
    /// declaration at `commands.register` (`Some`), the state every time.
    pub fn note_sent(
        &mut self,
        owner: OwnerGeneration,
        id: &str,
        declaration: Option<serde_json::Value>,
        state: serde_json::Value,
        when: serde_json::Value,
    ) {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        if !self.commands.contains_key(&key) {
            return;
        }
        match (declaration, self.sent.get_mut(&key)) {
            (Some(declaration), _) => {
                self.sent.insert(
                    key,
                    Sent {
                        declaration,
                        state,
                        when,
                    },
                );
            }
            (None, Some(sent)) => {
                sent.state = state;
                sent.when = when;
            }
            (None, None) => (),
        }
    }

    /// An owner's commands as sent: local id, current revision, what was
    /// sent; in registration order, so that a replay registers them in the
    /// same order and equal menu orders (and the owner's first bar button)
    /// come out as they did live.
    pub fn sent_of(&self, owner: OwnerGeneration) -> Vec<(&str, u64, &Sent)> {
        let mut sent = self
            .sent
            .iter()
            .filter(|(key, _)| key.owner == owner)
            .filter_map(|(key, sent)| {
                let entry = self.commands.get(key)?;
                Some((entry.sequence, key.local_id.as_str(), entry.revision, sent))
            })
            .collect::<Vec<_>>();
        sent.sort_by_key(|(sequence, ..)| *sequence);
        sent.into_iter()
            .map(|(_, id, revision, sent)| (id, revision, sent))
            .collect()
    }

    /// An owner's entries in registration order (the menu's tie-break),
    /// each with whether it is recorded and what its plugin sent
    /// (`VESTRA_TEST_REGISTRY`, host/supervisor/probe.rs).
    pub fn entries_of(
        &self,
        owner: OwnerGeneration,
    ) -> Vec<(&CommandSnapshot, bool, Option<&Sent>)> {
        entries_of(&self.commands, owner)
            .into_iter()
            .map(|entry| {
                (
                    entry,
                    self.recorded.contains(&entry.key),
                    self.sent.get(&entry.key),
                )
            })
            .collect()
    }

    /// A command of `owner`'s is busy (its run is in progress).
    pub fn busy(&self, owner: OwnerGeneration) -> bool {
        self.commands
            .values()
            .any(|entry| entry.key.owner == owner && entry.state.busy)
    }

    /// Whether `owner` has an entry that was not sent by its plugin (the
    /// host's own commands, or a test's): such an owner cannot be recorded.
    pub fn unsent(&self, owner: OwnerGeneration) -> bool {
        self.commands
            .keys()
            .any(|key| key.owner == owner && !self.sent.contains_key(key))
    }

    /// Park: every entry of `owner` stays, marked recorded.
    pub fn mark_recorded(&mut self, owner: OwnerGeneration) {
        let keys = self
            .commands
            .keys()
            .filter(|key| key.owner == owner)
            .cloned()
            .collect::<Vec<_>>();
        self.recorded.extend(keys);
    }

    /// At `runtime.ready` of a woken plugin: its recorded entries it did not
    /// register again go. Returns their local ids.
    pub fn remove_recorded(&mut self, owner: OwnerGeneration) -> Result<Vec<String>> {
        let keys = self
            .recorded
            .iter()
            .filter(|key| key.owner == owner)
            .cloned()
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let next = self.next_revision()?;
        for key in &keys {
            self.recorded.remove(key);
            self.sent.remove(key);
            self.conditions.remove(key);
            self.remove_entry(key);
        }
        self.revision = next;
        self.listed += 1;
        Ok(keys.into_iter().map(|key| key.local_id).collect())
    }

    /// Wake: `from`'s entries move to the new generation `to`, keeping their
    /// menu place, state and recorded mark.
    pub fn rekey(&mut self, from: OwnerGeneration, to: OwnerGeneration) -> Result<()> {
        let keys = self.keys_of(from).cloned().collect::<Vec<_>>();
        if keys.is_empty() {
            return Ok(());
        }
        let next = self.next_revision()?;
        for key in keys {
            let moved = CommandKey {
                owner: to,
                local_id: key.local_id.clone(),
            };
            // An entry `to` already has under this id is replaced.
            if self.commands.contains_key(&moved) {
                self.listed += 1;
            }
            if let Some(mut entry) = self.remove_entry(&key) {
                entry.key = moved.clone();
                self.insert_entry(entry);
            }
            if self.recorded.remove(&key) {
                self.recorded.insert(moved.clone());
            }
            if let Some(conditions) = self.conditions.remove(&key) {
                self.conditions.insert(moved.clone(), conditions);
            }
            if let Some(sent) = self.sent.remove(&key) {
                self.sent.insert(moved, sent);
            }
        }
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
        let keys = self.keys_of(owner).cloned().collect::<Vec<_>>();
        let count = keys.len();
        if count != 0 {
            let next = self.next_revision()?;
            for key in &keys {
                self.remove_entry(key);
            }
            self.recorded.retain(|key| key.owner != owner);
            self.sent.retain(|key, _| key.owner != owner);
            self.conditions.retain(|key, _| key.owner != owner);
            self.revision = next;
            self.listed += 1;
        }
        self.held.remove(&owner);
        Ok(count)
    }

    /// `owner`'s commands show disabled with `reason` (`None`: as their
    /// state says again), whatever their state: a crashed plugin held in
    /// standby until its restart hold is over (supervisor.md §7).
    /// Whether anything changed.
    pub fn set_held(&mut self, owner: OwnerGeneration, reason: Option<String>) -> bool {
        let changed = match reason {
            Some(reason) => self.held.insert(owner, reason.clone()) != Some(reason),
            None => self.held.remove(&owner).is_some(),
        };
        if changed && let Ok(next) = self.next_revision() {
            self.revision = next;
        }
        changed
    }

    /// Whether `owner`'s commands are held disabled (`set_held`).
    pub fn held(&self, owner: OwnerGeneration) -> bool {
        self.held.contains_key(&owner)
    }

    /// The state `entry` shows: its own, or disabled with the reason
    /// while its owner is held (`set_held`).
    fn shown_state(&self, entry: &CommandSnapshot) -> Option<CommandState> {
        shown_state(&self.held, entry)
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
        self.remove_entry(&key);
        self.recorded.remove(&key);
        self.sent.remove(&key);
        self.conditions.remove(&key);
        self.revision = next;
        self.listed += 1;
        Ok(())
    }

    /// The conditions of `id` after its register or update (`None`: its
    /// state is what the plugin sent). The state just stored is the
    /// plugin's; its condition fields are replaced by their value now.
    pub fn set_conditions(
        &mut self,
        owner: OwnerGeneration,
        id: &str,
        conditions: Option<Conditions>,
    ) -> Result<()> {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        let entry = self
            .commands
            .get(&key)
            .ok_or_else(|| CommandError::Missing(key.clone()))?;
        let Some(mut conditions) = conditions else {
            self.conditions.remove(&key);
            return Ok(());
        };
        conditions.reason = entry.state.reason.clone();
        let shown = conditions.apply(&entry.state, &self.facts);
        if shown != entry.state {
            let next = self.next_revision()?;
            self.commands
                .get_mut(&key)
                .expect("entry checked in this serial method")
                .state = shown;
            self.revision = next;
        }
        self.conditions.insert(key, conditions);
        Ok(())
    }

    /// The player's state changed: every condition is evaluated again.
    /// Whether a command's state changed (menus and keys follow the
    /// registry's revision).
    pub fn set_facts(&mut self, facts: Facts) -> bool {
        if facts == self.facts {
            return false;
        }
        self.facts = facts;
        let mut changed = false;
        for (key, conditions) in &self.conditions {
            let Some(entry) = self.commands.get_mut(key) else {
                continue;
            };
            let shown = conditions.apply(&entry.state, &self.facts);
            if shown != entry.state {
                entry.state = shown;
                changed = true;
            }
        }
        if changed && let Ok(next) = self.next_revision() {
            self.revision = next;
        }
        changed
    }

    pub fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot> {
        entry(&self.commands, owner, id)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The read-only copy of what the interface reads of the registry
    /// (rule R8 of doc/architecture/core-thread.md): the commands, the
    /// held owners and the revision. Its menu tree is built when first
    /// asked for ([`View::snapshot`]).
    pub fn view(&self) -> View {
        View {
            revision: self.revision,
            commands: self.commands.clone(),
            held: self.held.clone(),
            menu: OnceLock::new(),
        }
    }

    /// What `bindings.list` shows of the registry changed when this moved
    /// (`listed`).
    pub fn listed_revision(&self) -> u64 {
        self.listed
    }

    /// Every command as menus show it (a held owner's disabled with its
    /// reason), in registry order: what the command palette lists from.
    pub fn shown_entries(&self) -> Vec<CommandSnapshot> {
        shown_entries(&self.commands, &self.held)
    }

    /// Disabled commands (owner, id → reason), for the key bindings to skip.
    pub fn disabled(&self) -> BTreeMap<(OwnerGeneration, String), Option<String>> {
        self.commands
            .iter()
            .map(|(key, entry)| {
                (
                    key,
                    self.shown_state(entry)
                        .unwrap_or_else(|| entry.state.clone()),
                )
            })
            .filter(|(_, state)| !state.enabled)
            .map(|(key, state)| ((key.owner, key.local_id.clone()), state.reason))
            .collect()
    }

    /// The menu tree, built once per revision: every change to what it
    /// shows advances the revision (the interface language is fixed for
    /// the process, `i18n::init`). The player reads it from a [`View`]
    /// (rule R8), which builds it with the same code; kept for the tests.
    #[cfg(test)]
    pub fn snapshot(&self) -> Arc<MenuSnapshot> {
        let mut menu = self
            .menu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(snapshot) = menu.as_ref().filter(|menu| menu.revision == self.revision) {
            return Arc::clone(snapshot);
        }
        let snapshot = Arc::new(menu_snapshot(&self.commands, &self.held, self.revision));
        *menu = Some(Arc::clone(&snapshot));
        snapshot
    }

    /// The commands' bottom-bar buttons: each owner's first declared one
    /// (the others are `dropped`, with the limit), ranked by (order,
    /// manifest id, sequence); the lowest [`Self::MAX_BUTTONS`] ranks are
    /// the strip's, the rest its `»` menu (`overflow`). Lower orders stay
    /// in the strip longest (menu redesign step 3a). The player reads them
    /// from a [`View`] (rule R8), with the same code; kept for the tests.
    #[cfg(test)]
    pub fn buttons(&self) -> Buttons<'_> {
        buttons(&self.commands)
    }

    /// Resolve immediately before dispatch, at the serial registry arbiter.
    /// A cached menu snapshot is never execution authority. Runtime liveness is
    /// checked again by the caller when routing to this exact owner generation.
    pub fn resolve_execution(&self, owner: OwnerGeneration, id: &str) -> Result<Execution> {
        resolve_execution(&self.commands, &self.held, owner, id)
    }
}

/// A read-only copy of what the interface reads of the [`Registry`]
/// (rule R8 of doc/architecture/core-thread.md; plan step S2c): the core
/// owns the registry and the interface holds the newest of these, replaced
/// whole when the registry's revision moves (`Registry::view`). It answers
/// as the registry did at that revision; its menu tree is built once, when
/// first asked for.
#[derive(Debug, Default)]
pub struct View {
    revision: u64,
    commands: Commands,
    held: Held,
    menu: OnceLock<Arc<MenuSnapshot>>,
}

impl View {
    /// The registry's revision this is a copy of ([`Registry::revision`]).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// See [`Registry::get`].
    pub fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot> {
        entry(&self.commands, owner, id)
    }

    /// See [`Registry::shown_entries`].
    pub fn shown_entries(&self) -> Vec<CommandSnapshot> {
        shown_entries(&self.commands, &self.held)
    }

    /// See [`MenuSnapshot`]: built once for this copy.
    pub fn snapshot(&self) -> Arc<MenuSnapshot> {
        Arc::clone(
            self.menu
                .get_or_init(|| Arc::new(menu_snapshot(&self.commands, &self.held, self.revision))),
        )
    }

    /// See [`Buttons`].
    pub fn buttons(&self) -> Buttons<'_> {
        buttons(&self.commands)
    }

    /// An owner's entries in registration order, as
    /// [`Registry::entries_of`] lists them.
    pub fn entries_of(&self, owner: OwnerGeneration) -> Vec<&CommandSnapshot> {
        entries_of(&self.commands, owner)
    }

    /// See [`Registry::resolve_execution`]: what the registry would answer
    /// at this revision. Not execution authority (the registry is).
    pub fn resolve_execution(&self, owner: OwnerGeneration, id: &str) -> Result<Execution> {
        resolve_execution(&self.commands, &self.held, owner, id)
    }
}

/// A command looked up by owner and local id, in the registry or in a
/// [`View`] of it.
pub trait Lookup {
    fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot>;
}

impl Lookup for Registry {
    fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot> {
        entry(&self.commands, owner, id)
    }
}

impl Lookup for View {
    fn get(&self, owner: OwnerGeneration, id: &str) -> Option<&CommandSnapshot> {
        entry(&self.commands, owner, id)
    }
}

type Commands = BTreeMap<CommandKey, CommandSnapshot>;
type Held = BTreeMap<OwnerGeneration, String>;

fn entry<'a>(
    commands: &'a Commands,
    owner: OwnerGeneration,
    id: &str,
) -> Option<&'a CommandSnapshot> {
    commands.get(&CommandKey {
        owner,
        local_id: id.into(),
    })
}

/// An owner's entries in registration order (the menu's tie-break).
fn entries_of(commands: &Commands, owner: OwnerGeneration) -> Vec<&CommandSnapshot> {
    let mut entries = commands
        .values()
        .filter(|entry| entry.key.owner == owner)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.sequence);
    entries
}

/// The state `entry` shows: its own, or disabled with the reason while its
/// owner is held (`Registry::set_held`).
fn shown_state(held: &Held, entry: &CommandSnapshot) -> Option<CommandState> {
    let reason = held.get(&entry.key.owner)?;
    Some(CommandState {
        enabled: false,
        busy: false,
        reason: Some(reason.clone()),
        ..entry.state.clone()
    })
}

/// See [`Registry::shown_entries`].
fn shown_entries(commands: &Commands, held: &Held) -> Vec<CommandSnapshot> {
    commands
        .values()
        .map(|entry| match shown_state(held, entry) {
            Some(state) => CommandSnapshot {
                state,
                ..entry.clone()
            },
            None => entry.clone(),
        })
        .collect()
}

/// The menu tree of [`View::snapshot`].
fn menu_snapshot(commands: &Commands, held: &Held, revision: u64) -> MenuSnapshot {
    // A held owner's entries show disabled with the reason
    // (`set_held`): copies, few (one plugin's).
    let shown_held: Vec<CommandSnapshot> = commands
        .values()
        .filter(|entry| !entry.declaration.path.is_empty())
        .filter_map(|entry| {
            shown_state(held, entry).map(|state| CommandSnapshot {
                state,
                ..entry.clone()
            })
        })
        .collect();
    let entries: Vec<_> = commands
        .values()
        .filter(|entry| !entry.declaration.path.is_empty())
        .filter(|entry| !held.contains_key(&entry.key.owner))
        .chain(shown_held.iter())
        .collect();
    MenuSnapshot {
        revision,
        roots: tree(&entries, 0, false),
    }
}

/// See [`View::buttons`].
fn buttons(commands: &Commands) -> Buttons<'_> {
    let mut per_owner: BTreeMap<OwnerGeneration, usize> = BTreeMap::new();
    let mut result = Buttons::default();
    let mut declared: Vec<_> = commands
        .values()
        .filter(|entry| entry.declaration.button.is_some())
        .collect();
    declared.sort_by_key(|entry| entry.sequence);
    let mut shown = Vec::new();
    for entry in declared {
        let count = per_owner.entry(entry.key.owner).or_default();
        if *count < Registry::MAX_BUTTONS_PER_OWNER {
            *count += 1;
            shown.push(entry);
        } else {
            result.dropped.push((entry, "one button per plugin"));
        }
    }
    let rank = |entry: &CommandSnapshot| {
        (
            entry.declaration.button.as_ref().map_or(0, |b| b.order),
            entry.manifest.clone(),
            entry.sequence,
        )
    };
    shown.sort_by_key(|entry| rank(entry));
    // Past the limit the highest ranks go to `»`, as in a narrow window.
    result.overflow = shown.split_off(shown.len().min(Registry::MAX_BUTTONS));
    result.shown = shown;
    result
}

/// See [`Registry::resolve_execution`].
fn resolve_execution(
    commands: &Commands,
    held: &Held,
    owner: OwnerGeneration,
    id: &str,
) -> Result<Execution> {
    let key = CommandKey {
        owner,
        local_id: id.into(),
    };
    let entry = commands.get(&key).ok_or(CommandError::Missing(key))?;
    if let Some(reason) = held.get(&owner) {
        return Err(CommandError::Disabled {
            reason: Some(reason.clone()),
        });
    }
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
    // Groups in the order their first member comes, found by title.
    let mut groups: Vec<(String, Vec<&CommandSnapshot>)> = Vec::new();
    let mut found: HashMap<&str, usize> = HashMap::new();
    let mut nodes: Vec<(Rank<'_>, MenuNode)> = Vec::new();
    for entry in entries {
        let path = &entry.declaration.path;
        if path.len() == depth + 1 {
            nodes.push((
                rank(entry, appended),
                MenuNode::Command {
                    entry: Box::new((*entry).clone()),
                },
            ));
        } else if let Some(&index) = found.get(path[depth].as_str()) {
            groups[index].1.push(entry);
        } else {
            found.insert(&path[depth], groups.len());
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
        let label = host_group_label(&first.declaration.path[..=depth])
            .unwrap_or_else(|| first.declaration.shown_segment(depth).to_owned());
        nodes.push((
            key,
            MenuNode::Group {
                group: first.declaration.group.clone(),
                children: tree(&members, depth + 1, built_in.is_some()),
                title,
                label,
            },
        ));
    }
    nodes.sort_by(|a, b| a.0.cmp(&b.0));
    let mut nodes: Vec<MenuNode> = nodes.into_iter().map(|(_, node)| node).collect();
    if depth == 0 {
        share_groups(&mut nodes);
    }
    nodes
}

/// A top-level built-in menu in `FIXED_GROUPS` takes its fixed separator
/// group; one in `SHARES_GROUP_WITH` takes the group of the menu it joins,
/// when that menu is shown.
fn share_groups(nodes: &mut [MenuNode]) {
    for node in nodes.iter_mut() {
        if let MenuNode::Group { title, group, .. } = node
            && let Some((_, fixed)) = FIXED_GROUPS.iter().find(|(key, _)| key == title)
        {
            *group = (*fixed).to_owned();
        }
    }
    for (key, joins) in SHARES_GROUP_WITH {
        let shared = nodes.iter().find_map(|node| match node {
            MenuNode::Group { title, group, .. } if title == joins => Some(group.clone()),
            _ => None,
        });
        let Some(shared) = shared else { continue };
        for node in nodes.iter_mut() {
            if let MenuNode::Group { title, group, .. } = node
                && title == key
            {
                *group = shared.clone();
            }
        }
    }
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

    /// `bindings.list`'s gate (supervisor.md): the listed revision
    /// moves when a command comes, goes or changes its declaration, not
    /// when a woken plugin's commands move to their new owner and are
    /// registered again as they were, nor on a state update.
    #[test]
    fn the_listed_revision_ignores_a_wake_that_registers_the_same() {
        let mut registry = Registry::new();
        let listed = |registry: &Registry| registry.listed_revision();
        registry
            .register(
                1,
                "a",
                "x".into(),
                1,
                declaration(&["Tools", "X"]),
                state(true),
            )
            .unwrap();
        let first = listed(&registry);
        assert!(first > 0, "a new command");
        registry.mark_recorded(1);
        registry.rekey(1, 9).unwrap();
        registry
            .register(
                9,
                "a",
                "x".into(),
                2,
                declaration(&["Tools", "X"]),
                state(true),
            )
            .unwrap();
        registry.update(9, "x", 3, state(false)).unwrap();
        assert_eq!(listed(&registry), first, "parked, woken, registered again");
        registry.mark_recorded(9);
        registry.rekey(9, 10).unwrap();
        registry
            .register(
                10,
                "a",
                "x".into(),
                4,
                declaration(&["Tools", "Y"]),
                state(true),
            )
            .unwrap();
        let renamed = listed(&registry);
        assert!(renamed > first, "registered again with another title");
        registry
            .register(
                10,
                "a",
                "z".into(),
                1,
                declaration(&["Tools", "Z"]),
                state(true),
            )
            .unwrap();
        let added = listed(&registry);
        assert!(added > renamed);
        registry.remove(10, "z").unwrap();
        let removed = listed(&registry);
        assert!(removed > added);
        registry.mark_recorded(10);
        registry.rekey(10, 11).unwrap();
        assert_eq!(listed(&registry), removed);
        assert_eq!(registry.remove_recorded(11).unwrap(), vec!["x".to_owned()]);
        assert!(
            listed(&registry) > removed,
            "a recorded command not registered again"
        );
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

    /// A plugin held after a fault (supervisor.md §7): its commands
    /// show disabled with the reason in the menu, and are refused with it,
    /// until the hold is released; a revoke ends it too.
    #[test]
    fn a_held_owner_shows_disabled_with_the_reason() {
        let mut registry = Registry::new();
        registry
            .register(
                7,
                "p",
                "go".into(),
                1,
                declaration(&["Tools", "Go"]),
                state(true),
            )
            .unwrap();
        let shown = |registry: &Registry| {
            let snapshot = registry.snapshot();
            let MenuNode::Group { children, .. } = &snapshot.roots[0] else {
                panic!("a group");
            };
            let MenuNode::Command { entry } = &children[0] else {
                panic!("a command");
            };
            entry.state.clone()
        };
        assert!(shown(&registry).enabled);
        assert!(registry.set_held(7, Some("p stopped".into())));
        assert!(!registry.set_held(7, Some("p stopped".into())), "no change");
        assert_eq!(
            (shown(&registry).enabled, shown(&registry).reason),
            (false, Some("p stopped".into()))
        );
        assert!(matches!(
            registry.resolve_execution(7, "go"),
            Err(CommandError::Disabled { reason: Some(r) }) if r == "p stopped"
        ));
        assert_eq!(
            registry.disabled().get(&(7, "go".into())),
            Some(&Some("p stopped".into()))
        );
        assert!(registry.set_held(7, None));
        assert!(shown(&registry).enabled);
        assert!(registry.resolve_execution(7, "go").is_ok());
        registry.set_held(7, Some("p stopped".into()));
        registry.remove_owner(7).unwrap();
        assert!(!registry.held(7));
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
        assert_eq!(
            registry.disabled(),
            [((1, "play".to_owned()), Some("No source".to_owned()))]
                .into_iter()
                .collect()
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
        assert!(registry.disabled().is_empty());
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

    /// An English plugin's `["@video", "Blur"]` lands in the same Video menu
    /// as the built-in video plugin's old `["视频", …]` commands.
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
            ["Video", "Blur"]
        );
        let roots = registry.snapshot().roots.clone();
        let mut titles = Vec::new();
        flatten(&roots, &mut titles);
        assert_eq!(titles, ["▸Video", "旋转", "Blur"]);
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
            ["Audio", "@home", "X"]
        );
        assert_eq!(MENU_ANCHORS.len(), 9);
    }

    /// A plugin item in a built-in menu, with no `order` (0), does not move
    /// the menu: Video stays between Subtitles and Audio and the item goes last.
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
                "▸Subtitles",
                "显示字幕",
                "▸Video",
                // A negative order puts an item first inside, nothing more.
                "First",
                "▸图像旋转",
                "按序切换",
                "复位",
                // Order 0 by id or by title: after the menu's own items.
                "Blur…",
                "Sharpen",
                "▸Audio",
                "均衡器",
                "▸Tools",
                // Below the plugin's own submenu, 0 is an ordinary order.
                "A",
                "B",
                // Only plugins fill Settings here: it still sits at its own order.
                "▸Settings",
                "X",
            ]
        );
    }

    /// `@reset` sits directly below Settings and in its separator group,
    /// whatever its items declare; with no item in it, it is not shown.
    #[test]
    fn reset_follows_settings_in_its_group_and_hides_when_empty() {
        let mut registry = Registry::new();
        let add = |registry: &mut Registry, id: &str, path: &[&str], order, group: &str| {
            let mut declaration = ordered(path, order);
            declaration.group = group.into();
            registry
                .register(1, "m", id.into(), 1, declaration, state(true))
                .unwrap();
        };
        add(&mut registry, "store", &["Plugin Store…"], 822, "windows");
        add(
            &mut registry,
            "options",
            &["@settings", "Options…"],
            830,
            "settings-main",
        );
        add(&mut registry, "about", &["About…"], 880, "windows");
        let shown = |registry: &Registry| {
            registry
                .snapshot()
                .roots
                .iter()
                .map(|node| match node {
                    MenuNode::Group { title, group, .. } => format!("▸{title}:{group}"),
                    MenuNode::Command { entry } => {
                        format!("{}:{}", entry.declaration.title, entry.declaration.group)
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(&registry),
            [
                "Plugin Store…:windows",
                "▸Settings:settings-main",
                "About…:windows"
            ]
        );
        add(
            &mut registry,
            "picture",
            &["@reset", "Picture"],
            100,
            "reset-video",
        );
        add(
            &mut registry,
            "sync",
            &["Reset", "Subtitle Sync"],
            300,
            "reset-subtitles",
        );
        assert_eq!(
            shown(&registry),
            [
                "Plugin Store…:windows",
                "▸Settings:settings-main",
                "▸Reset:settings-main",
                "About…:windows",
            ]
        );
        let roots = registry.snapshot().roots.clone();
        let MenuNode::Group {
            label, children, ..
        } = &roots[2]
        else {
            panic!("Reset is a submenu");
        };
        assert_eq!(label, "Reset");
        // Inside, the items keep their own groups (separators between domains).
        assert_eq!(
            children.iter().map(MenuNode::group).collect::<Vec<_>>(),
            ["reset-video", "reset-subtitles"]
        );
    }

    /// The short menu (user ruling 2026-11-11): `@tools` sits after the
    /// domain menus and before Options…, in the settings group whatever its
    /// first item declares, and a plugin's `["Tools", …]` without an order
    /// joins it at the end; Settings, shown by a plugin, keeps that group
    /// too, so no separator splits Tools ▸, Settings ▸, Options… and Quit.
    #[test]
    fn tools_sits_before_options_in_the_settings_group() {
        let mut registry = Registry::new();
        let add = |registry: &mut Registry, owner, id: &str, path: &[&str], order, group: &str| {
            let mut declaration = ordered(path, order);
            declaration.group = group.into();
            registry
                .register(owner, "m", id.into(), 1, declaration, state(true))
                .unwrap();
        };
        add(&mut registry, 1, "play", &["Play/Pause"], 201, "domain");
        add(
            &mut registry,
            1,
            "track",
            &["@tools", "Audio Track", "A"],
            301,
            "domain",
        );
        add(
            &mut registry,
            1,
            "ai",
            &["@tools", "Vestra Player AI…"],
            820,
            "windows",
        );
        add(
            &mut registry,
            1,
            "options",
            &["Options…"],
            830,
            "settings-main",
        );
        add(&mut registry, 1, "quit", &["Quit"], 990, "settings-main");
        add(&mut registry, 2, "nes", &["Games", "NES"], 600, "domain");
        add(
            &mut registry,
            3,
            "hello",
            &["Tools", "Say hello"],
            0,
            "domain",
        );
        add(
            &mut registry,
            3,
            "pref",
            &["@settings", "Hello…"],
            0,
            "domain",
        );
        let roots = registry.snapshot().roots.clone();
        let shown: Vec<String> = roots
            .iter()
            .map(|node| match node {
                MenuNode::Group { title, group, .. } => format!("▸{title}:{group}"),
                MenuNode::Command { entry } => {
                    format!("{}:{}", entry.declaration.title, entry.declaration.group)
                }
            })
            .collect();
        assert_eq!(
            shown,
            [
                "Play/Pause:domain",
                "▸Games:domain",
                "▸Tools:settings-main",
                "▸Settings:settings-main",
                "Options…:settings-main",
                "Quit:settings-main",
            ]
        );
        let MenuNode::Group {
            children, label, ..
        } = &roots[2]
        else {
            panic!("Tools is a submenu");
        };
        assert_eq!(label, "Tools");
        let titles: Vec<&str> = children
            .iter()
            .map(|node| match node {
                MenuNode::Group { title, .. } => title.as_str(),
                MenuNode::Command { entry } => entry.declaration.title.as_str(),
            })
            .collect();
        assert_eq!(titles, ["Audio Track", "Vestra Player AI…", "Say hello"]);
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
        let roots = registry.snapshot().roots.clone();
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

    /// SDK 1.5.48 review: a binding listed twice refuses the registration
    /// before anything changes, so no menu item stays behind a failed RPC.
    #[test]
    fn a_repeated_binding_refuses_the_registration_unchanged() {
        let mut registry = Registry::new();
        registry
            .register(
                1,
                "m",
                "kept".into(),
                1,
                declaration(&["Tools", "Kept"]),
                state(true),
            )
            .unwrap();
        let (before, revision) = (registry.snapshot(), registry.revision());
        let mut twice = declaration(&["Tools", "Twice"]);
        twice.bindings = vec!["Ctrl+K".into(), "F2".into(), "Ctrl+K".into()];
        let refused = registry.register(1, "m", "twice".into(), 1, twice.clone(), state(true));
        assert!(
            matches!(&refused, Err(CommandError::Invalid(why)) if why.contains("Ctrl+K is listed twice")),
            "{refused:?}"
        );
        assert!(registry.get(1, "twice").is_none());
        assert_eq!(registry.snapshot(), before);
        assert_eq!(registry.revision(), revision);
        // The plugin's corrected registration takes the id and the path.
        twice.bindings.pop();
        registry
            .register(1, "m", "twice".into(), 1, twice, state(true))
            .unwrap();
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
    /// (src/tests/menus-snapshot.test.ts: video 71, all ~290).
    #[test]
    fn limits_leave_room_for_real_plugins() {
        const VIDEO_AT_IDLE: usize = 71;
        const ALL_AT_IDLE: usize = 292;
        const { assert!(Registry::MAX_PER_OWNER >= 2 * VIDEO_AT_IDLE) };
        const { assert!(Registry::MAX_COMMANDS >= 3 * ALL_AT_IDLE) };
    }

    /// core-ui-design.md §14: the migration orders reproduce the tree that the
    /// old registry (fixed domain list, then UTF-8 byte order of path[1..])
    /// built from the same 46 commands (the old menu tree, recorded before
    /// the migration).
    /// The 模拟器 lines came from the built-in NES plugin, now the store
    /// plugin zysilm.vestranes; the paths and orders stay as recorded; owner
    /// ids follow the current plugin ids.
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
            ("zysilm.vestrabili", "文件/网络媒体/Bilibili 视频…", 212),
            ("zysilm.vestratube", "文件/网络媒体/YouTube 视频…", 213),
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
            ("zysilm.vestranes", "模拟器/NES 工具", 561),
            ("zysilm.vestranes", "模拟器/停止 NES", 562),
            ("zysilm.vestranes", "模拟器/打开 NES ROM…", 563),
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

    /// i18n.md §5: the player's old Chinese titles of its own menus are
    /// aliases of their English keys, so an unmigrated plugin's items land
    /// in the player's submenus (and `@id` still works).
    #[test]
    fn old_chinese_titles_of_the_players_menus_merge_with_the_english_keys() {
        let mut registry = Registry::new();
        for (owner, id, path) in [
            (1, "normal", vec!["Playback", "Speed", "Normal"]),
            (2, "half", vec!["播放", "速度", "0.5x"]),
            (2, "jump", vec!["播放", "跳转", "跳到开头"]),
            (3, "frame", vec!["@playback", "Seek", "Next Frame"]),
            // Only a submenu is renamed: a leaf titled 速度 stays itself.
            (4, "leaf", vec!["播放", "速度…"]),
            // A submenu shared with a migrated plugin.
            (5, "logo", vec!["设置", "外观", "显示徽标"]),
            (6, "dark", vec!["Settings", "Appearance", "Vestra Dark"]),
        ] {
            registry
                .register(owner, "m", id.into(), 1, declaration(&path), state(true))
                .unwrap();
        }
        assert_eq!(
            registry.get(2, "half").unwrap().declaration.path,
            ["Playback", "Speed", "0.5x"]
        );
        let mut titles = Vec::new();
        flatten(&registry.snapshot().roots, &mut titles);
        assert_eq!(
            titles,
            [
                "▸Playback",
                "▸Speed",
                "Normal",
                "0.5x",
                "▸Seek",
                "跳到开头",
                "Next Frame",
                "速度…",
                "▸Settings",
                "▸Appearance",
                "显示徽标",
                "Vestra Dark"
            ]
        );
        // The unmigrated plugin's own words still show.
        let mut logo = declaration(&["设置", "外观", "显示徽标"]);
        let declared = logo.path.clone();
        normalize(&mut logo.path).unwrap();
        logo.localize_declared(&declared, |text| text.to_owned());
        assert_eq!(logo.path, ["Settings", "Appearance", "显示徽标"]);
        assert_eq!(logo.shown_path(), ["Settings", "外观", "显示徽标"]);
        // Normalizing twice changes nothing.
        let mut path = vec!["Playback".to_owned(), "Speed".into(), "x".into()];
        normalize(&mut path).unwrap();
        assert_eq!(path, ["Playback", "Speed", "x"]);
    }

    /// i18n.md §7: a text value's key is its English rendering, its shown
    /// text the translation (set by `show_texts` after `localize_declared`).
    #[test]
    fn text_value_segments_show_their_rendering() {
        let mut track = declaration(&["@audio", "Audio Track", "Track 2 (opus)"]);
        let declared = track.path.clone();
        normalize(&mut track.path).unwrap();
        track.localize_declared(&declared, |text| match text {
            "Audio Track" => "音轨".to_owned(),
            other => other.to_owned(),
        });
        track.show_texts(
            Some("音轨 2 (opus)".into()),
            &[None, None, Some("音轨 2 (opus)".into())],
        );
        assert_eq!(track.path, ["Audio", "Audio Track", "Track 2 (opus)"]);
        assert_eq!(track.shown_path()[1..], ["音轨", "音轨 2 (opus)"]);
        assert_eq!(track.shown_title(), "音轨 2 (opus)");
    }

    /// i18n.md §5: menus merge by key and show labels: the player's words for
    /// its own submenus, the declaring plugin's catalog for the rest.
    #[test]
    fn groups_and_items_show_their_translations() {
        let mut registry = Registry::new();
        let catalog = |text: &str| match text {
            "Tools" => "工具".to_owned(),
            "Blur…" => "模糊…".to_owned(),
            other => other.to_owned(),
        };
        let mut blur = declaration(&["@video", "Tools", "Blur…"]);
        normalize(&mut blur.path).unwrap();
        blur.localize(catalog);
        assert_eq!(blur.shown_title(), "模糊…");
        // English run: the player's own "Video" is itself.
        assert_eq!(blur.shown_path(), ["Video", "工具", "模糊…"]);
        registry
            .register(1, "m", "blur".into(), 1, blur, state(true))
            .unwrap();
        // Another plugin without a translation joins the same submenu by key.
        let mut other = declaration(&["Video", "Tools", "Sharpen"]);
        other.localize(|text| text.to_owned());
        registry
            .register(2, "n", "sharpen".into(), 1, other, state(true))
            .unwrap();
        let roots = registry.snapshot().roots.clone();
        let MenuNode::Group {
            label, children, ..
        } = &roots[0]
        else {
            panic!("a group")
        };
        assert_eq!(label, "Video");
        let MenuNode::Group {
            title,
            label,
            children,
            ..
        } = &children[0]
        else {
            panic!("a group")
        };
        assert_eq!((title.as_str(), label.as_str()), ("Tools", "工具"));
        assert_eq!(children.len(), 2);
        // Undeclared display: the keys show.
        let plain = declaration(&["A", "B"]);
        assert_eq!(
            (plain.shown_title(), plain.shown_path()),
            ("B", vec!["A".to_owned(), "B".into()])
        );
    }
}

#[cfg(test)]
mod button_tests {
    use super::*;

    fn state() -> CommandState {
        CommandState {
            enabled: true,
            checked: None,
            busy: false,
            reason: None,
        }
    }
    fn with_button(path: &[&str], icon: &str, order: i32) -> Declaration {
        Declaration {
            button: Some(Button {
                icon: ButtonIcon::Name(icon.into()),
                order,
            }),
            ..Declaration::new(path.last().copied().unwrap_or("Title"), path)
        }
    }
    fn shown(registry: &Registry) -> Vec<String> {
        registry
            .buttons()
            .shown
            .iter()
            .map(|entry| entry.key.local_id.clone())
            .collect()
    }

    #[test]
    fn buttons_need_a_menu_path_a_known_icon_and_an_order_in_range() {
        let mut registry = Registry::new();
        let mut register =
            |id: &str, declaration| registry.register(1, "a", id.into(), 1, declaration, state());
        let invalid = |result: Result<()>| matches!(result, Err(CommandError::Invalid(_)));
        assert!(invalid(register("a", with_button(&[], "store", 1))));
        assert!(invalid(register("b", with_button(&["B"], "shopping", 1))));
        assert!(invalid(register("c", with_button(&["C"], "store", -1))));
        assert!(invalid(register("d", with_button(&["D"], "store", 1000))));
        let image = Declaration {
            button: Some(Button {
                icon: ButtonIcon::Image {
                    path: String::new(),
                    max_side: 0,
                },
                order: 1,
            }),
            ..Declaration::new("E", &["E"])
        };
        assert!(invalid(register("e", image)));
        register("f", with_button(&["F"], "store", 0)).unwrap();
        register("g", with_button(&["G"], "ai", 999)).unwrap();
    }

    #[test]
    fn one_button_per_plugin_eight_in_the_strip_the_rest_in_its_overflow() {
        let mut registry = Registry::new();
        // Owner 1 declares two: the first declared keeps its button; the
        // second stays a menu item.
        registry
            .register(
                1,
                "z.one",
                "first".into(),
                1,
                with_button(&["One"], "ai", 50),
                state(),
            )
            .unwrap();
        registry
            .register(
                1,
                "z.one",
                "second".into(),
                1,
                with_button(&["Two"], "store", 1),
                state(),
            )
            .unwrap();
        // Equal orders: by plugin id.
        registry
            .register(
                2,
                "b.two",
                "b".into(),
                1,
                with_button(&["B"], "add", 10),
                state(),
            )
            .unwrap();
        registry
            .register(
                3,
                "a.three",
                "a".into(),
                1,
                with_button(&["A"], "add", 10),
                state(),
            )
            .unwrap();
        assert_eq!(shown(&registry), ["a", "b", "first"]);
        let dropped = registry.buttons().dropped;
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].0.key.local_id, "second");
        assert!(
            registry.snapshot().roots.len() == 4,
            "the menu keeps every entry"
        );
        for owner in 10..17 {
            registry
                .register(
                    owner,
                    &format!("p{owner}"),
                    "x".into(),
                    1,
                    with_button(&[&format!("P{owner}")], "add", 100 + owner as i32),
                    state(),
                )
                .unwrap();
        }
        let buttons = registry.buttons();
        assert_eq!(buttons.shown.len(), Registry::MAX_BUTTONS);
        // Step 3a: nothing past eight is left out. The lowest ranks are the
        // strip's; the rest go to its `»` menu, in the same order.
        let ids = |list: &[&CommandSnapshot]| {
            list.iter()
                .map(|entry| entry.manifest.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&buttons.shown),
            [
                "a.three", "b.two", "z.one", "p10", "p11", "p12", "p13", "p14"
            ]
        );
        assert_eq!(ids(&buttons.overflow), ["p15", "p16"]);
        // Only a plugin's second button is left out (its menu entry stays).
        let out: Vec<_> = buttons
            .dropped
            .iter()
            .map(|(entry, limit)| (entry.key.local_id.as_str(), *limit))
            .collect();
        assert_eq!(out, [("second", "one button per plugin")]);
        assert_eq!(buttons.shown[2].key.local_id, "first");
        // A removed plugin's button goes with its commands, and the first
        // of `»` moves into the strip.
        registry.remove_owner(3).unwrap();
        let buttons = registry.buttons();
        assert_eq!(buttons.shown[0].manifest, "b.two");
        assert_eq!(buttons.shown[7].manifest, "p15");
        assert_eq!(ids(&buttons.overflow), ["p16"]);
        registry.remove_owner(10).unwrap();
        assert!(registry.buttons().overflow.is_empty());
    }

    /// Equal order and plugin id (two runs of one plugin id): the earlier
    /// registration goes first.
    #[test]
    fn equal_order_and_id_keep_the_registration_order() {
        let mut registry = Registry::new();
        for owner in [21, 20] {
            registry
                .register(
                    owner,
                    "same.id",
                    "b".into(),
                    1,
                    with_button(&[&format!("B{owner}")], "add", 5),
                    state(),
                )
                .unwrap();
        }
        let owners: Vec<_> = registry
            .buttons()
            .shown
            .iter()
            .map(|entry| entry.key.owner)
            .collect();
        assert_eq!(owners, [21, 20]);
    }

    /// `BUTTON_ICONS` is every name `Icons.commands` draws.
    #[test]
    fn button_icons_are_the_icon_sets_names() {
        let slint = include_str!("../ui/vestra/icons.slint");
        let names: Vec<_> = slint
            .split("if name == \"")
            .skip(1)
            .filter_map(|rest| {
                let (name, after) = rest.split_once('"')?;
                after.starts_with(" { return \"").then_some(name)
            })
            .collect();
        assert_eq!(names, BUTTON_ICONS);
    }

    /// Lazy activation: a recording lists an owner's commands in the order
    /// it registered them, not by id, so a replay at the next start gives
    /// siblings of equal order the same menu places (registry diff test,
    /// tests/lazy_activation.rs: every built-in's menu came back in id
    /// order).
    #[test]
    fn a_replay_keeps_the_registration_order_of_equal_orders() {
        let declaration = |path: &[&str]| Declaration::new(path.last().unwrap(), path);
        fn flatten(nodes: &[MenuNode], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    MenuNode::Group { children, .. } => flatten(children, out),
                    MenuNode::Command { entry } => out.push(entry.declaration.title.clone()),
                }
            }
        }
        let mut live = Registry::new();
        for id in ["zoom", "fit", "mini"] {
            live.register(1, "a", id.into(), 1, declaration(&["Screen", id]), state())
                .unwrap();
            live.note_sent(
                1,
                id,
                Some(serde_json::json!({})),
                serde_json::json!({}),
                serde_json::Value::Null,
            );
        }
        let recorded = live
            .sent_of(1)
            .iter()
            .map(|(id, ..)| id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(recorded, ["zoom", "fit", "mini"]);
        let mut replayed = Registry::new();
        for id in &recorded {
            replayed
                .register(7, "a", id.clone(), 1, declaration(&["Screen", id]), state())
                .unwrap();
        }
        let titles = |registry: &Registry| {
            let mut out = Vec::new();
            flatten(&registry.snapshot().roots, &mut out);
            out
        };
        assert_eq!(titles(&replayed), titles(&live));
    }

    /// Lazy activation: a parked owner's entries stay recorded, move to the
    /// woken generation in their menu place, are replaced by the live
    /// registrations, and the ones not registered again go at ready.
    #[test]
    fn recorded_entries_rekey_are_replaced_by_live_ones_and_the_rest_pruned() {
        let decl = |path: &[&str]| Declaration::new(path.last().unwrap(), path);
        let st = |enabled| CommandState {
            enabled,
            checked: None,
            busy: false,
            reason: None,
        };
        fn flatten(nodes: &[MenuNode], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    MenuNode::Group { children, .. } => flatten(children, out),
                    MenuNode::Command { entry } => out.push(entry.declaration.title.clone()),
                }
            }
        }
        let mut registry = Registry::new();
        for (owner, manifest, id, path) in [
            (1, "a", "first", ["Tools", "A first"]),
            (2, "b", "hello", ["Tools", "Hello"]),
            (2, "b", "gone", ["Tools", "Gone"]),
            (3, "c", "last", ["Tools", "Z last"]),
        ] {
            registry
                .register(owner, manifest, id.into(), 1, decl(&path), st(true))
                .unwrap();
            registry.note_sent(
                owner,
                id,
                Some(serde_json::json!({"title":path[1]})),
                serde_json::json!({"enabled":true}),
                serde_json::Value::Null,
            );
        }
        registry.note_sent(
            2,
            "hello",
            None,
            serde_json::json!({"enabled":false}),
            serde_json::Value::Null,
        );
        assert_eq!(registry.sent_of(2).len(), 2);
        // In registration order: hello, then gone.
        assert_eq!(registry.sent_of(2)[0].0, "hello");
        assert_eq!(
            registry.sent_of(2)[0].2.state,
            serde_json::json!({"enabled":false})
        );
        let order = |registry: &Registry| {
            let mut out = Vec::new();
            flatten(&registry.snapshot().roots, &mut out);
            out
        };
        let before = order(&registry);
        // Parked: still in the menu, still resolvable.
        registry.mark_recorded(2);
        assert!(registry.resolve_execution(2, "hello").is_ok());
        // Woken as generation 9: same menu, the old generation is gone.
        registry.rekey(2, 9).unwrap();
        assert_eq!(order(&registry), before);
        assert!(registry.resolve_execution(2, "hello").is_err());
        assert!(registry.resolve_execution(9, "hello").is_ok());
        // The live registration replaces the recorded one (no conflict) and
        // keeps its place; another owner still collides.
        registry
            .register(
                9,
                "b",
                "hello".into(),
                1,
                decl(&["Tools", "Hello"]),
                st(false),
            )
            .unwrap();
        assert_eq!(order(&registry), before);
        assert!(registry.sent_of(9).iter().all(|(id, ..)| *id != "hello"));
        assert!(matches!(
            registry.register(
                9,
                "b",
                "hello".into(),
                1,
                decl(&["Tools", "Hello"]),
                st(true)
            ),
            Err(CommandError::Conflict(_))
        ));
        assert!(matches!(
            registry.register(
                4,
                "d",
                "clash".into(),
                1,
                decl(&["Tools", "Gone"]),
                st(true)
            ),
            Err(CommandError::Conflict(_))
        ));
        // At ready the entry not registered again goes, and only it.
        assert_eq!(registry.remove_recorded(9).unwrap(), vec!["gone"]);
        assert!(registry.get(9, "gone").is_none());
        assert!(registry.get(9, "hello").is_some());
        assert_eq!(registry.remove_recorded(9).unwrap(), Vec::<String>::new());
        // A live path that collides with the same owner's recorded entry wins.
        registry.mark_recorded(1);
        registry
            .register(
                1,
                "a",
                "renamed".into(),
                1,
                decl(&["Tools", "A first"]),
                st(true),
            )
            .unwrap();
        assert!(registry.get(1, "first").is_none());
        assert!(registry.get(1, "renamed").is_some());
        // Removing an owner forgets its recorded marks and what it sent.
        registry.remove_owner(3).unwrap();
        assert!(registry.sent_of(3).is_empty());
    }

    /// `plugin.when`: the registry evaluates a command's conditions against
    /// the player's state, again whenever it changes, for a parked owner
    /// too, and through a wake.
    #[test]
    fn conditions_follow_the_players_state_through_park_and_wake() {
        use crate::conditions::{Conditions, Facts};
        let facts = |phase: &str, fullscreen: bool| {
            Facts::new(
                &serde_json::json!({"phase":phase,"capabilities":{"snapshotSource":true}}),
                &serde_json::json!({"fullscreen":fullscreen}),
                "vestra",
                "system",
            )
        };
        let when = |value| Conditions::parse(&value).unwrap();
        let mut registry = Registry::new();
        registry.set_facts(facts("empty", false));
        let sent = CommandState {
            enabled: true,
            checked: Some(false),
            busy: false,
            reason: Some("No frame to capture".into()),
        };
        registry
            .register(
                2,
                "capture",
                "save".into(),
                1,
                Declaration::new("Save", &["Video", "Save"]),
                sent.clone(),
            )
            .unwrap();
        registry
            .set_conditions(
                2,
                "save",
                when(serde_json::json!({
                    "enabled":{"all":[{"can":"snapshot"},{"phase":["ready","ended"]}]},
                    "checked":{"fullscreen":true}})),
            )
            .unwrap();
        let shown = |registry: &Registry, owner| registry.get(owner, "save").unwrap().state.clone();
        assert!(!shown(&registry, 2).enabled);
        assert_eq!(
            shown(&registry, 2).reason.as_deref(),
            Some("No frame to capture")
        );
        assert!(registry.resolve_execution(2, "save").is_err());
        // Parked: the state still follows; the key resolves as shown.
        registry.mark_recorded(2);
        let revision = registry.revision();
        assert!(registry.set_facts(facts("ready", true)));
        assert!(registry.revision() > revision);
        assert_eq!(
            (shown(&registry, 2).enabled, shown(&registry, 2).checked),
            (true, Some(true))
        );
        assert_eq!(shown(&registry, 2).reason, None);
        assert!(registry.resolve_execution(2, "save").is_ok());
        // The same facts again: nothing changes.
        let revision = registry.revision();
        assert!(!registry.set_facts(facts("ready", true)));
        assert_eq!(registry.revision(), revision);
        // Woken: the conditions move with the entry.
        registry.rekey(2, 9).unwrap();
        assert!(registry.set_facts(facts("empty", true)));
        assert!(!shown(&registry, 9).enabled);
        // An update without conditions: the plugin's own state again.
        registry.update(9, "save", 2, sent.clone()).unwrap();
        registry.set_conditions(9, "save", None).unwrap();
        assert!(!registry.set_facts(facts("ready", false)));
        assert_eq!(shown(&registry, 9), sent);
    }
}

/// The path index and the cached menu tree give the
/// answers the scans over every command gave.
#[cfg(test)]
mod index_tests {
    use super::*;

    fn state() -> CommandState {
        CommandState {
            enabled: true,
            checked: None,
            busy: false,
            reason: None,
        }
    }

    /// `register`'s outcome as the scan over every command decided it
    /// (before the path index), and the recorded entries it removes.
    fn scanned(
        registry: &Registry,
        owner: OwnerGeneration,
        id: &str,
        path: &[String],
    ) -> (std::result::Result<(), String>, Vec<CommandKey>) {
        let key = CommandKey {
            owner,
            local_id: id.into(),
        };
        let replaced = registry.recorded.contains(&key);
        if registry.commands.contains_key(&key) && !replaced {
            return (
                Err(CommandError::Conflict("owner/localId already registered".into()).to_string()),
                Vec::new(),
            );
        }
        let collides = |existing: &[String]| {
            !path.is_empty()
                && !existing.is_empty()
                && (existing.starts_with(path) || path.starts_with(existing))
        };
        for entry in registry.commands.values() {
            if entry.key == key
                || (entry.key.owner == owner && registry.recorded.contains(&entry.key))
            {
                continue;
            }
            if collides(&entry.declaration.path) {
                return (
                    Err(CommandError::Conflict(
                        "menu path duplicates a command or collides with a leaf/group".into(),
                    )
                    .to_string()),
                    Vec::new(),
                );
            }
        }
        let owned = registry
            .commands
            .keys()
            .filter(|each| each.owner == owner)
            .count();
        if !replaced
            && (registry.commands.len() >= registry.max_commands()
                || owned >= Registry::MAX_PER_OWNER)
        {
            return (Err(CommandError::Limit.to_string()), Vec::new());
        }
        let gone = registry
            .commands
            .values()
            .filter(|entry| {
                entry.key != key
                    && entry.key.owner == owner
                    && registry.recorded.contains(&entry.key)
                    && collides(&entry.declaration.path)
            })
            .map(|entry| entry.key.clone())
            .collect();
        (Ok(()), gone)
    }

    /// The menu tree with groups found by a linear search, as before.
    fn scanned_tree(entries: &[&CommandSnapshot], depth: usize, appended: bool) -> Vec<MenuNode> {
        let mut groups: Vec<(String, Vec<&CommandSnapshot>)> = Vec::new();
        let mut nodes: Vec<(Rank<'_>, MenuNode)> = Vec::new();
        for entry in entries {
            let path = &entry.declaration.path;
            if path.len() == depth + 1 {
                nodes.push((
                    rank(entry, appended),
                    MenuNode::Command {
                        entry: Box::new((*entry).clone()),
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
                .unwrap();
            let key = match built_in {
                Some(order) => (order, "", 0),
                None => rank(first, appended),
            };
            let label = host_group_label(&first.declaration.path[..=depth])
                .unwrap_or_else(|| first.declaration.shown_segment(depth).to_owned());
            nodes.push((
                key,
                MenuNode::Group {
                    group: first.declaration.group.clone(),
                    children: scanned_tree(&members, depth + 1, built_in.is_some()),
                    title,
                    label,
                },
            ));
        }
        nodes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut nodes: Vec<MenuNode> = nodes.into_iter().map(|(_, node)| node).collect();
        if depth == 0 {
            share_groups(&mut nodes);
        }
        nodes
    }

    fn check(registry: &Registry, step: usize) {
        let mut indexed = BTreeMap::<Vec<String>, Vec<CommandKey>>::new();
        for entry in registry.commands.values() {
            if !entry.declaration.path.is_empty() {
                indexed
                    .entry(entry.declaration.path.clone())
                    .or_default()
                    .push(entry.key.clone());
            }
        }
        let mut index = registry.paths.clone();
        for keys in index.values_mut() {
            keys.sort();
        }
        assert_eq!(index, indexed, "step {step}");
        let entries: Vec<_> = registry
            .commands
            .values()
            .filter(|entry| !entry.declaration.path.is_empty())
            .collect();
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.revision, registry.revision(), "step {step}");
        assert_eq!(
            snapshot.roots,
            scanned_tree(&entries, 0, false),
            "step {step}"
        );
    }

    struct Steps(u64);
    impl Steps {
        fn next(&mut self, below: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % below
        }
    }

    #[test]
    fn indexed_collisions_match_a_scan_of_every_command() {
        const SEGMENTS: [&str; 4] = ["Video", "A", "B", "Ab"];
        const IDS: [&str; 6] = ["a", "b", "c", "d", "e", "f"];
        let mut steps = Steps(0x2545_f491_4f6c_dd1d);
        let mut registry = Registry {
            max_commands: Some(24),
            ..Registry::default()
        };
        let mut revision = 1;
        let (mut updated, mut changed) = (0, 0);
        for step in 0..4000 {
            let owner = steps.next(5);
            let id = IDS[steps.next(IDS.len() as u64) as usize];
            match steps.next(13) {
                0..=5 => {
                    let path = (0..steps.next(4))
                        .map(|_| SEGMENTS[steps.next(SEGMENTS.len() as u64) as usize])
                        .collect::<Vec<_>>();
                    let mut declaration = Declaration::new("x", &path);
                    declaration.order = steps.next(3) as i32;
                    let path = declaration.path.clone();
                    let (expected, gone) = scanned(&registry, owner, id, &path);
                    let before = registry.commands.keys().cloned().collect::<Vec<_>>();
                    revision += 1;
                    let result = registry
                        .register(
                            owner,
                            &format!("m{owner}"),
                            id.into(),
                            revision,
                            declaration,
                            state(),
                        )
                        .map_err(|error| error.to_string());
                    assert_eq!(result, expected, "step {step}: {owner}/{id} at {path:?}");
                    if result.is_ok() {
                        for key in &gone {
                            assert!(!registry.commands.contains_key(key), "step {step}");
                        }
                        let kept = before.iter().filter(|key| !gone.contains(key)).count();
                        let added = usize::from(
                            !before
                                .iter()
                                .any(|key| key.owner == owner && key.local_id == id),
                        );
                        assert_eq!(registry.commands.len(), kept + added, "step {step}");
                    }
                }
                6 => {
                    let _ = registry.remove(owner, id);
                }
                7 => {
                    let _ = registry.remove_owner(owner);
                }
                8 => registry.rekey(owner, steps.next(7)).unwrap(),
                // State changes advance the revision too: the cached
                // tree (checked below) must follow them.
                9 => {
                    revision += 1;
                    let state = CommandState {
                        enabled: steps.next(2) == 0,
                        checked: Some(steps.next(2) == 0),
                        busy: false,
                        reason: Some("why".into()),
                    };
                    updated += usize::from(registry.update(owner, id, revision, state).is_ok());
                }
                10 => {
                    let conditions = if steps.next(3) == 0 {
                        None
                    } else {
                        Conditions::parse(&serde_json::json!({
                            "enabled":{"phase":["ready"]},
                            "checked":{"fullscreen":true}}))
                        .unwrap()
                    };
                    let _ = registry.set_conditions(owner, id, conditions);
                }
                11 => {
                    let phase = ["empty", "ready"][steps.next(2) as usize];
                    changed += usize::from(registry.set_facts(Facts::new(
                        &serde_json::json!({"phase":phase}),
                        &serde_json::json!({"fullscreen":steps.next(2) == 0}),
                        "vestra",
                        "system",
                    )));
                }
                _ => {
                    if steps.next(2) == 0 {
                        registry.mark_recorded(owner);
                    } else {
                        registry.remove_recorded(owner).unwrap();
                    }
                }
            }
            check(&registry, step);
        }
        assert!(
            updated > 20 && changed > 5,
            "{updated} updates, {changed} fact changes"
        );
    }

    /// 2000 plugins of three commands (the probe's
    /// seed: two in a submenu of their own under Tools, one without a
    /// path), registered and laid out in a menu tree. Scanning every
    /// command per registration took 1.4 s in a release build.
    #[test]
    fn six_thousand_registrations_stay_fast() {
        let started = std::time::Instant::now();
        let mut registry = Registry {
            max_commands: Some(6000),
            ..Registry::default()
        };
        for owner in 1..=2000_u64 {
            let manifest = format!("probe.tiny-{owner:04}");
            let tiny = format!("Tiny {owner:04}");
            for (id, path) in [
                ("go", vec!["Tools", tiny.as_str(), "Go"]),
                ("info", vec!["Tools", tiny.as_str(), "Info"]),
                ("key", vec![]),
            ] {
                registry
                    .register(
                        owner,
                        &manifest,
                        id.into(),
                        1,
                        Declaration::new(id, &path),
                        state(),
                    )
                    .unwrap();
            }
        }
        // Taken: the same path again, a leaf over a group, a group over a leaf.
        for path in [
            vec!["Tools", "Tiny 0001", "Go"],
            vec!["Tools", "Tiny 0002"],
            vec!["Tools", "Tiny 0003", "Go", "More"],
        ] {
            assert!(matches!(
                registry.register(
                    1,
                    "probe.tiny-0001",
                    "more".into(),
                    1,
                    Declaration::new("x", &path),
                    state()
                ),
                Err(CommandError::Conflict(_))
            ));
        }
        assert!(matches!(
            registry.register(
                9999,
                "x",
                "x".into(),
                1,
                Declaration::new("x", &[]),
                state()
            ),
            Err(CommandError::Limit)
        ));
        let snapshot = registry.snapshot();
        let elapsed = started.elapsed();
        diag!("6000 registrations and a menu tree: {elapsed:?}");
        let [
            MenuNode::Group {
                title, children, ..
            },
        ] = snapshot.roots.as_slice()
        else {
            panic!("one Tools menu");
        };
        assert_eq!((title.as_str(), children.len()), ("Tools", 2000));
        // The same revision gives the same tree without building it again.
        assert!(Arc::ptr_eq(&snapshot, &registry.snapshot()));
        assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
    }
}
