//! The command palette (menu redesign §6, H1): every registered command by
//! name. Pure data: which commands it lists, how a row reads, fuzzy
//! matching and ordering, and the rows in view. `host/palette.rs` drives
//! it; `ui/command-palette.slint` draws it on the menu layer.
//!
//! A row reads `Category: Title`. The category is the command's own
//! (`CommandOptions.category`), else the menu path above it, else its
//! plugin's name. Typing matches the shown row, its English source text,
//! the menu path (shown and keys) and the plugin (name and id): the letters
//! of each word in order, not necessarily together (fzf / VS Code style),
//! scored for runs and word starts.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::commands::{CommandSnapshot, Focus};
use crate::i18n::{tr, tr_args};
use serde_json::{Value, json};

/// Recently run commands kept (ui_state `paletteRecent`), newest first.
pub const RECENT_LIMIT: usize = 8;
/// Row elements the view keeps: never more rows than this in view. A fixed
/// count, written in place, so moving through the list never creates or
/// destroys an element (ui/menu-view.slint's no-churn rule).
pub const SLOTS: usize = 12;
/// Text runs of a row's label, alternately plain and matched (plain first).
pub const SEGMENTS: usize = 8;
/// Must match ui/command-palette.slint: row height, the panel's top, and the
/// height of everything but the rows (field, separator, footer, padding).
pub const ROW: f64 = 28.;
pub const TOP: f64 = 16.;
pub const CHROME: f64 = 92.;
/// The brand stays untranslated: the player's own commands' plugin name.
const PLAYER: &str = "Vestra Player";

/// One listed command, as the palette shows and matches it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub owner: u64,
    pub id: String,
    /// `<manifest id>/<command id>` (`host/…` for the player's own): what
    /// the recent list keeps, the same across launches.
    pub global: String,
    /// `Category: Title` as shown.
    pub label: String,
    /// The same in English source text (the declared keys).
    label_key: String,
    /// The plugin's name as shown.
    pub plugin: String,
    manifest: String,
    /// The menu path as shown and by its keys, segments joined by spaces.
    path: String,
    path_key: String,
    /// Where the command lives: the window its `home` command opens, its
    /// menu path, or the menu; empty for a command outside the menu.
    pub place: String,
    pub checked: Option<bool>,
    pub radio: bool,
    pub enabled: bool,
    pub busy: bool,
    pub reason: String,
    /// Its current keys as the menu shows them.
    pub keys: String,
}

/// A listed command that matches the query: its index in the items, the
/// score and the matched characters of its label (char indices).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub index: usize,
    score: i64,
    pub marks: BTreeSet<usize>,
}

fn joined(path: &[String]) -> String {
    path.join(" ▸ ")
}

/// A title with a trailing ellipsis removed: "Subtitle Tuning…" names a
/// place as "Subtitle Tuning".
fn place_name(title: &str) -> String {
    title
        .trim_end_matches('…')
        .trim_end_matches("...")
        .trim_end()
        .to_owned()
}

/// The commands the palette lists, from the registry's shown entries.
/// Left out: commands with `focus: "panel"` (their keys work only in their
/// panel) and disabled commands that give no reason, whatever their keys
/// (a greyed "No Audio Tracks" placeholder; VS Code hides a command whose
/// `when` is false the same way), so unbinding a key never hides a
/// command. A disabled command with a reason is listed greyed, with it.
/// `name` gives a plugin's shown name by manifest id, `keys` a command's
/// current keys.
pub fn items(
    entries: &[CommandSnapshot],
    name: &dyn Fn(&str) -> String,
    keys: &dyn Fn(u64, &str) -> Option<String>,
) -> Vec<Item> {
    let by_key: BTreeMap<(u64, &str), &CommandSnapshot> = entries
        .iter()
        .map(|entry| ((entry.key.owner, entry.key.local_id.as_str()), entry))
        .collect();
    entries
        .iter()
        .filter(|entry| entry.declaration.focus != Some(Focus::Panel))
        .filter_map(|entry| {
            let declaration = &entry.declaration;
            let state = &entry.state;
            let keys = keys(entry.key.owner, &entry.key.local_id).unwrap_or_default();
            let reason = state.reason.clone().unwrap_or_default();
            if !state.enabled && reason.is_empty() {
                return None;
            }
            let player = entry.manifest == "host";
            let plugin = if player {
                PLAYER.to_owned()
            } else {
                name(&entry.manifest)
            };
            let shown_path = declaration.shown_path();
            let parent = shown_path.len().saturating_sub(1);
            let key_parent = declaration.path.len().saturating_sub(1);
            let (category, category_key, from_path) = match declaration.category.as_deref() {
                Some(key) => (
                    declaration.shown_category().unwrap_or(key).to_owned(),
                    key.to_owned(),
                    false,
                ),
                None if parent > 0 => (
                    joined(&shown_path[..parent]),
                    joined(&declaration.path[..key_parent]),
                    true,
                ),
                None => (
                    plugin.clone(),
                    if player {
                        PLAYER.to_owned()
                    } else {
                        entry.manifest.clone()
                    },
                    false,
                ),
            };
            let title = declaration.shown_title();
            let label = tr_args(
                "{category}: {title}",
                &json!({"category":category,"title":title}),
            );
            let label_key = format!("{category_key}: {}", declaration.title);
            let home = declaration
                .home
                .as_deref()
                .and_then(|home| by_key.get(&(entry.key.owner, home)))
                .map(|home| place_name(home.declaration.shown_title()))
                .filter(|place| !place.is_empty());
            let place = match home {
                Some(place) => place,
                None if shown_path.is_empty() => String::new(),
                // The category already says where in the menu it is.
                None if from_path || parent == 0 => tr("Menu"),
                None => format!("{} ▸", joined(&shown_path[..parent])),
            };
            Some(Item {
                owner: entry.key.owner,
                id: entry.key.local_id.clone(),
                global: format!("{}/{}", entry.manifest, entry.key.local_id),
                label,
                label_key,
                plugin,
                manifest: entry.manifest.clone(),
                path: shown_path.join(" "),
                path_key: declaration.path.join(" "),
                place,
                checked: state.checked,
                radio: declaration.radio,
                enabled: state.enabled,
                busy: state.busy,
                reason,
                keys,
            })
        })
        .collect()
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ':' | '▸' | '/' | '-' | '_' | '.' | '(' | '[' | '·' | '：'
        )
}

/// How much starting a match at `index` is worth: the start of the text, of
/// a word, of a camel-case hump; a character of a script without spaces
/// (Han, kana, Hangul) is a word of its own.
fn start_bonus(text: &[char], index: usize) -> i64 {
    let c = text[index];
    let Some(&previous) = index.checked_sub(1).and_then(|i| text.get(i)) else {
        return 8;
    };
    if separator(previous) {
        7
    } else if previous.is_lowercase() && c.is_uppercase() {
        6
    } else if unicode_width::UnicodeWidthChar::width(c) == Some(2) {
        4
    } else if previous.is_ascii_digit() != c.is_ascii_digit() {
        3
    } else {
        0
    }
}

/// Every character of `query` (folded) in `text` in order: the best score
/// and the matched char indices of `text`, `None` when it does not match.
/// Each matched character is worth 16, a run continues for 8 more and a
/// word start for its [`start_bonus`]; each character skipped between two
/// matches costs 1. Linear in `query × text`.
pub fn fuzzy(query: &[char], text: &str) -> Option<(i64, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let original: Vec<char> = text.chars().collect();
    let folded: Vec<char> = original.iter().copied().map(fold).collect();
    let (n, m) = (query.len(), folded.len());
    // A quick refusal: not even a subsequence.
    let mut at = 0;
    for &q in query {
        at += folded[at..].iter().position(|&c| c == q)? + 1;
    }
    const NONE: i64 = i64::MIN / 4;
    // score[i][j]: the best with query[i] matched at text[j]; from[i][j]:
    // where query[i - 1] was matched for it.
    let mut score = vec![vec![NONE; m]; n];
    let mut from = vec![vec![usize::MAX; m]; n];
    let bonus: Vec<i64> = (0..m).map(|j| start_bonus(&original, j)).collect();
    for i in 0..n {
        // The best (score[i - 1][k] + k) over k < j - 1, and its k.
        let mut best: (i64, usize) = (NONE, usize::MAX);
        for j in 0..m {
            if j >= 2 && i > 0 && score[i - 1][j - 2] > NONE {
                let candidate = score[i - 1][j - 2] + (j as i64 - 2);
                if candidate > best.0 {
                    best = (candidate, j - 2);
                }
            }
            if folded[j] != query[i] {
                continue;
            }
            if i == 0 {
                score[0][j] = 16 + bonus[j];
                continue;
            }
            // The previous character right before: a run.
            if j >= 1 && score[i - 1][j - 1] > NONE {
                score[i][j] = score[i - 1][j - 1] + 16 + 8 + bonus[j] / 2;
                from[i][j] = j - 1;
            }
            if best.0 > NONE {
                let gap = best.0 - (j as i64 - 1) + 16 + bonus[j];
                if gap > score[i][j] {
                    score[i][j] = gap;
                    from[i][j] = best.1;
                }
            }
        }
    }
    let (end, &total) = score[n - 1]
        .iter()
        .enumerate()
        .filter(|(_, s)| **s > NONE)
        .max_by_key(|(j, s)| (**s, std::cmp::Reverse(*j)))?;
    let mut marks = vec![end];
    let mut j = end;
    for i in (1..n).rev() {
        j = from[i][j];
        marks.push(j);
    }
    marks.reverse();
    Some((total, marks))
}

/// The commands that match `query`, best first; with an empty query the
/// recently run ones (newest first), then the rest by label. `recent` holds
/// global ids, newest first: a match among them scores higher, the newest
/// most.
pub fn search(items: &[Item], query: &str, recent: &[String]) -> Vec<Hit> {
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|word| word.chars().map(fold).collect())
        .collect();
    let rank = |item: &Item| recent.iter().position(|global| *global == item.global);
    let by_label = |a: &Item, b: &Item| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.global.cmp(&b.global))
    };
    if words.is_empty() {
        let mut hits: Vec<usize> = (0..items.len()).collect();
        hits.sort_by(|&a, &b| match (rank(&items[a]), rank(&items[b])) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => by_label(&items[a], &items[b]),
        });
        return hits
            .into_iter()
            .map(|index| Hit {
                index,
                score: 0,
                marks: BTreeSet::new(),
            })
            .collect();
    }
    let mut hits: Vec<Hit> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let mut total = 0;
            let mut marks = BTreeSet::new();
            for word in &words {
                let shown = fuzzy(word, &item.label);
                // The other fields count a little less than the row itself.
                let best = [
                    shown.as_ref().map(|(score, _)| *score),
                    fuzzy(word, &item.label_key).map(|(s, _)| s - 1),
                    fuzzy(word, &item.path).map(|(s, _)| s - 4),
                    fuzzy(word, &item.path_key).map(|(s, _)| s - 4),
                    fuzzy(word, &item.plugin).map(|(s, _)| s - 6),
                    fuzzy(word, &item.manifest).map(|(s, _)| s - 8),
                ]
                .into_iter()
                .flatten()
                .max()?;
                total += best;
                if let Some((_, at)) = shown {
                    marks.extend(at);
                }
            }
            if let Some(rank) = rank(item) {
                total += (RECENT_LIMIT - rank.min(RECENT_LIMIT)) as i64 * 4;
            }
            Some(Hit {
                index,
                score: total,
                marks,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| {
                items[a.index]
                    .label
                    .chars()
                    .count()
                    .cmp(&items[b.index].label.chars().count())
            })
            .then_with(|| by_label(&items[a.index], &items[b.index]))
    });
    hits
}

/// `label` cut into [`SEGMENTS`] runs, alternately plain and matched
/// (`marks`, char indices), plain first; runs past the last slot join it.
pub fn segments(label: &str, marks: &BTreeSet<usize>) -> Vec<String> {
    let mut runs: Vec<String> = vec![String::new()];
    for (index, c) in label.chars().enumerate() {
        let hit = marks.contains(&index);
        // Odd runs are matched ones.
        if runs.len().is_multiple_of(2) != hit && runs.len() < SEGMENTS {
            runs.push(String::new());
        }
        runs.last_mut().expect("one run").push(c);
    }
    runs.resize(SEGMENTS, String::new());
    runs
}

/// How many rows fit in a window `height` logical px high.
pub fn visible(height: f64) -> usize {
    let room = ((height - TOP - CHROME - 8.) / ROW).floor();
    if room.is_finite() && room >= 1. {
        (room as usize).min(SLOTS)
    } else {
        1
    }
}

/// The first row in view that keeps `cursor` in view, starting from
/// `offset`.
pub fn reveal(offset: usize, cursor: usize, visible: usize, count: usize) -> usize {
    let visible = visible.max(1);
    let offset = if cursor < offset {
        cursor
    } else if cursor >= offset + visible {
        cursor + 1 - visible
    } else {
        offset
    };
    offset.min(count.saturating_sub(visible))
}

/// The rows in view, always [`SLOTS`] of them (the rest not shown): what
/// `palette-rows` holds.
pub fn rows(items: &[Item], hits: &[Hit], offset: usize, visible: usize) -> Value {
    let rows: Vec<Value> = (0..SLOTS)
        .map(|slot| {
            let hit = (slot < visible).then(|| hits.get(offset + slot)).flatten();
            let Some(hit) = hit else {
                return json!({"token":"","segments":vec![""; SEGMENTS],"label":"",
                    "plugin":"","place":"","keys":"","enabled":false,"checked":false,
                    "radio":false,"checkable":false,"busy":false,"reason":"","shown":false});
            };
            let item = &items[hit.index];
            let reason = item.reason.clone();
            let mut segments = segments(&item.label, &hit.marks);
            if item.busy {
                // After the last run, plain or not: a busy item's ellipsis.
                if let Some(last) = segments.iter_mut().rev().find(|s| !s.is_empty()) {
                    last.push_str(" …");
                }
            }
            json!({"token":format!("c:{}:{}", item.owner, item.id),
                "segments":segments,"label":item.label,
                // A disabled command says why where its plugin's name was.
                "plugin":if item.enabled { item.plugin.clone() } else { reason.clone() },
                "place":item.place,"keys":item.keys,
                "enabled":item.enabled && !item.busy,
                "checked":item.checked.unwrap_or(false),"radio":item.radio,
                "checkable":item.checked.is_some(),"busy":item.busy,
                "reason":if item.enabled && !item.busy { String::new() } else { reason },
                "shown":true})
        })
        .collect();
    Value::Array(rows)
}

/// The footer's Enter, Shift+Enter and Escape as the menus write keys on
/// this system (`bindings::label`: `Shift+Enter`, `⇧Enter` on macOS).
pub fn footer_keys() -> [String; 3] {
    ["Enter", "Shift+Enter", "Escape"].map(|key| {
        crate::bindings::parse(key)
            .map(|input| crate::bindings::label(&input))
            .unwrap_or_else(|_| key.to_owned())
    })
}

/// Where the command with this global id is among `hits`, if listed: the
/// cursor follows a command run with Shift+Enter there.
pub fn position(items: &[Item], hits: &[Hit], global: &str) -> Option<usize> {
    hits.iter().position(|hit| {
        items
            .get(hit.index)
            .is_some_and(|item| item.global == global)
    })
}

/// The recent list after running `global`: it first, at most
/// [`RECENT_LIMIT`].
pub fn remember(recent: &mut Vec<String>, global: &str) {
    recent.retain(|known| known != global);
    recent.insert(0, global.to_owned());
    recent.truncate(RECENT_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandKey, CommandState, Declaration};

    fn entry(owner: u64, manifest: &str, id: &str, declaration: Declaration) -> CommandSnapshot {
        CommandSnapshot {
            key: CommandKey {
                owner,
                local_id: id.into(),
            },
            revision: 1,
            declaration,
            state: CommandState {
                enabled: true,
                checked: None,
                busy: false,
                reason: None,
            },
            manifest: manifest.into(),
            sequence: 0,
        }
    }
    fn chars(text: &str) -> Vec<char> {
        text.chars().map(fold).collect()
    }
    fn names(id: &str) -> String {
        match id {
            "vestra.subtitles" => "Subtitles".into(),
            "vestra.audio" => "Audio".into(),
            other => other.into(),
        }
    }
    fn none(_: u64, _: &str) -> Option<String> {
        None
    }

    #[test]
    fn fuzzy_matches_letters_in_order_and_prefers_runs_and_word_starts() {
        assert_eq!(
            fuzzy(&chars("sync"), "Sync: Earlier 0.5 s").unwrap().1,
            [0, 1, 2, 3]
        );
        assert!(fuzzy(&chars("cnys"), "Sync").is_none());
        assert!(fuzzy(&chars("xyz"), "Sync").is_none());
        // Word starts: "se" picks Subtitle Earlier's S and E.
        let (_, marks) = fuzzy(&chars("se"), "Subtitle Sync: Earlier").unwrap();
        assert_eq!(marks, [9, 15]);
        // A run beats scattered letters.
        let run = fuzzy(&chars("sync"), "Audio Sync: Later").unwrap().0;
        let scattered = fuzzy(&chars("sync"), "Show your new caption").unwrap().0;
        assert!(run > scattered, "{run} > {scattered}");
        // Case and width folding: CJK characters are words of their own.
        assert_eq!(fuzzy(&chars("同步"), "字幕同步: 提前").unwrap().1, [2, 3]);
        assert_eq!(fuzzy(&[], "x"), Some((0, Vec::new())));
    }

    #[test]
    fn rows_read_category_colon_title_from_the_category_the_path_or_the_plugin() {
        let mut earlier = Declaration::new("Earlier 0.5 s", &[]);
        earlier.category = Some("Subtitle Sync".into());
        earlier.home = Some("tune".into());
        earlier.localize(|text| match text {
            "Subtitle Sync" => "字幕同步".into(),
            "Earlier 0.5 s" => "提前 0.5 秒".into(),
            other => other.into(),
        });
        let tune = Declaration::new("Subtitle Tuning…", &["Subtitles", "Subtitle Tuning…"]);
        let snapshot = Declaration::new("Save", &["Video", "Snapshot", "Save"]);
        let loose = Declaration::new("Brief Info", &[]);
        let full = Declaration::new("Toggle Fullscreen", &["Toggle Fullscreen"]);
        let mut panel = Declaration::new("Play", &[]);
        panel.focus = Some(Focus::Panel);
        let entries = vec![
            entry(1, "vestra.subtitles", "sync-earlier", earlier),
            entry(1, "vestra.subtitles", "tune", tune),
            entry(2, "vestra.capture", "save", snapshot),
            entry(3, "vestra.info", "brief", loose),
            entry(0, "host", "fullscreen", full),
            entry(4, "vestra.playlist", "play", panel),
        ];
        let items = items(&entries, &names, &none);
        let row = |id: &str| items.iter().find(|item| item.id == id).unwrap();
        assert_eq!(items.len(), 5, "the panel command is left out");
        // The category, translated; the place is its home command's window.
        assert_eq!(row("sync-earlier").label, "字幕同步: 提前 0.5 秒");
        assert_eq!(
            row("sync-earlier").label_key,
            "Subtitle Sync: Earlier 0.5 s"
        );
        assert_eq!(row("sync-earlier").place, "Subtitle Tuning");
        assert_eq!(row("sync-earlier").global, "vestra.subtitles/sync-earlier");
        // The menu path above the item; it lives in the menu.
        assert_eq!(row("save").label, "Video ▸ Snapshot: Save");
        assert_eq!(row("save").place, "Menu");
        // Outside the menu: the plugin's name, no place.
        assert_eq!(row("brief").label, "vestra.info: Brief Info");
        assert_eq!(row("brief").place, "");
        // The player's own.
        assert_eq!(row("fullscreen").label, "Vestra Player: Toggle Fullscreen");
        assert_eq!(row("fullscreen").global, "host/fullscreen");
    }

    /// The player's own commands and the core actions read like the
    /// plugins' rows: "Seek: Back 5 s", not "Vestra Player: …" or the path.
    #[test]
    fn the_player_commands_read_category_colon_title() {
        let own = crate::host::Host::host_declarations()
            .into_iter()
            .map(|(id, declaration, _)| (id, declaration));
        let entries: Vec<CommandSnapshot> = own
            .chain(crate::core_actions::declarations(5.))
            .map(|(id, declaration)| entry(0, "host", id, declaration))
            .collect();
        let items = items(&entries, &names, &none);
        let row = |id: &str| items.iter().find(|item| item.id == id).unwrap();
        assert_eq!(items.len(), entries.len());
        for item in &items {
            assert!(!item.label.starts_with("Vestra Player:"), "{}", item.label);
        }
        assert_eq!(row("play-pause").label, "Playback: Play/Pause");
        assert_eq!(row("stop").label, "Playback: Close");
        assert_eq!(row("stop").place, "Menu");
        assert_eq!(row("seek-backward").label, "Seek: Back 5 s");
        // Palette only (menu redesign step 4): no place, found by "Seek".
        assert_eq!(row("seek-backward").place, "");
        assert_eq!(row("rate-normal").label, "Speed: Normal");
        assert_eq!(row("volume-up").label, "Audio: Volume +");
        assert_eq!(row("fullscreen").label, "Window: Toggle Fullscreen");
        assert_eq!(row("quit").label, "File: Quit");
        // In the menu since the slim menu (B5), with its ellipsis.
        assert_eq!(row("palette").label, "Help: Command Palette…");
        assert_eq!(row("palette").place, "Menu");
        // Palette only (B5): Previous, Next, the volume steps and Mute.
        assert_eq!(row("previous").label, "Playback: Previous");
        assert_eq!(row("previous").place, "");
        assert_eq!(row("mute").label, "Audio: Mute");
        assert_eq!(row("mute").place, "");
        assert_eq!(row("sidebar").global, "host/sidebar");
    }

    #[test]
    fn placeholders_are_left_out_and_disabled_commands_say_why() {
        let mut entries = vec![
            entry(
                1,
                "vestra.audio",
                "none",
                Declaration::new(
                    "No Audio Tracks",
                    &["Audio", "Audio Track", "No Audio Tracks"],
                ),
            ),
            entry(
                1,
                "vestra.audio",
                "why",
                Declaration::new("Mono", &["Audio", "Mono"]),
            ),
            entry(
                1,
                "vestra.audio",
                "keyed",
                Declaration::new("Louder", &["Audio", "Louder"]),
            ),
        ];
        for entry in &mut entries {
            entry.state.enabled = false;
        }
        entries[1].state.reason = Some("Needs a file".into());
        // A key does not keep a disabled command without a reason listed;
        // unbinding one never hides a command (coordinator ruling,
        // following VS Code's `when`).
        let keys = |_: u64, id: &str| (id == "keyed").then(|| "L".to_owned());
        let items = items(&entries, &names, &keys);
        let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["why"]);
        let hits = search(&items, "", &[]);
        let rows = rows(&items, &hits, 0, 3);
        assert_eq!(rows[0]["label"], "Audio: Mono");
        assert_eq!(rows[0]["enabled"], false);
        assert_eq!(rows[0]["reason"], "Needs a file");
        assert_eq!(rows[0]["plugin"], "Needs a file");
        assert_eq!(rows[1]["shown"], false);
        // With a reason it is listed whether or not it has a key.
        let mut keyed = entries.clone();
        keyed[2].state.reason = Some("Nothing to raise".into());
        let listed = super::items(&keyed, &names, &keys);
        assert_eq!(listed.len(), 2);
        let none = |_: u64, _: &str| None;
        assert_eq!(super::items(&keyed, &names, &none).len(), 2);
        assert_eq!(rows.as_array().unwrap().len(), SLOTS);
    }

    #[test]
    fn search_ranks_matches_matches_english_under_a_translation_and_puts_recent_first() {
        let mut sync = Declaration::new("Earlier 0.5 s", &[]);
        sync.category = Some("Subtitle Sync".into());
        sync.localize(|text| match text {
            "Subtitle Sync" => "字幕同步".into(),
            "Earlier 0.5 s" => "提前 0.5 秒".into(),
            other => other.into(),
        });
        let entries = vec![
            entry(1, "vestra.subtitles", "sync-earlier", sync),
            entry(
                2,
                "vestra.audio",
                "sync-later",
                Declaration::new("Later", &["Audio", "Audio Sync", "Later"]),
            ),
            entry(
                2,
                "vestra.audio",
                "mute",
                Declaration::new("Mono", &["Audio", "Mono"]),
            ),
        ];
        let items = items(&entries, &names, &none);
        let ids = |hits: Vec<Hit>| -> Vec<String> {
            hits.into_iter()
                .map(|hit| items[hit.index].id.clone())
                .collect()
        };
        // English source text under a Chinese row; both sync commands.
        assert_eq!(
            ids(search(&items, "sync", &[])),
            ["sync-later", "sync-earlier"]
        );
        // Every word must match somewhere.
        assert_eq!(ids(search(&items, "sync later", &[])), ["sync-later"]);
        // The plugin name matches too.
        assert_eq!(ids(search(&items, "subtitles", &[])), ["sync-earlier"]);
        // A recent command scores higher.
        let recent = vec!["vestra.subtitles/sync-earlier".to_owned()];
        assert_eq!(ids(search(&items, "sync", &recent))[0], "sync-earlier");
        // Empty: recent first, then by label.
        assert_eq!(
            ids(search(&items, "", &recent)),
            ["sync-earlier", "sync-later", "mute"]
        );
        // The row's own matched letters are marked.
        let hits = search(&items, "later", &[]);
        assert_eq!(
            segments(&items[hits[0].index].label, &hits[0].marks)[..3],
            ["Audio ▸ Audio Sync: ", "Later", ""]
        );
    }

    /// Shift+Enter: the run command moves to the top of an empty query's
    /// list, and `position` finds it there for the cursor (the review of
    /// host 0.7.4: the cursor stayed on the old index, now another command).
    #[test]
    fn the_footer_keys_are_the_menus_labels() {
        use crate::bindings::{label_in, parse};
        use crate::platform::KeyLabels;
        let shift_enter = parse("Shift+Enter").unwrap();
        assert_eq!(label_in(&shift_enter, KeyLabels::Text), "Shift+Enter");
        assert_eq!(label_in(&shift_enter, KeyLabels::Symbols), "⇧Enter");
        let keys = footer_keys();
        let expected = match crate::platform::KEY_LABELS {
            KeyLabels::Text => ["Enter", "Shift+Enter", "Esc"],
            KeyLabels::Symbols => ["Enter", "⇧Enter", "Esc"],
        };
        assert_eq!(keys, expected);
    }

    #[test]
    fn the_cursor_follows_a_command_run_and_kept_open() {
        let entries = vec![
            entry(
                2,
                "vestra.audio",
                "louder",
                Declaration::new("Louder", &["Audio", "Louder"]),
            ),
            entry(
                2,
                "vestra.audio",
                "mono",
                Declaration::new("Mono", &["Audio", "Mono"]),
            ),
            entry(
                2,
                "vestra.audio",
                "quieter",
                Declaration::new("Quieter", &["Audio", "Quieter"]),
            ),
        ];
        let items = items(&entries, &names, &none);
        let mut recent = Vec::new();
        let before = search(&items, "", &recent);
        assert_eq!(position(&items, &before, "vestra.audio/quieter"), Some(2));
        remember(&mut recent, "vestra.audio/quieter");
        let after = search(&items, "", &recent);
        assert_eq!(position(&items, &after, "vestra.audio/quieter"), Some(0));
        // The old index now holds another command.
        assert_eq!(items[after[2].index].id, "mono");
        assert_eq!(position(&items, &after, "nobody/gone"), None);
    }

    #[test]
    fn segments_alternate_and_fold_extra_runs_into_the_last() {
        let marks: BTreeSet<usize> = [0, 2, 4, 6, 8, 10].into();
        let runs = segments("abcdefghijkl", &marks);
        assert_eq!(runs.len(), SEGMENTS);
        assert_eq!(runs[..7], ["", "a", "b", "c", "d", "e", "f"]);
        assert_eq!(runs[7], "ghijkl");
        assert_eq!(segments("ab", &BTreeSet::new())[0], "ab");
    }

    #[test]
    fn the_view_follows_the_cursor_and_the_recent_list_keeps_eight() {
        assert_eq!(visible(1000.), SLOTS);
        assert_eq!(visible(182.), 2);
        assert_eq!(visible(10.), 1);
        assert_eq!(reveal(0, 13, 12, 40), 2);
        assert_eq!(reveal(5, 3, 12, 40), 3);
        assert_eq!(reveal(30, 39, 12, 40), 28);
        assert_eq!(reveal(30, 0, 12, 5), 0);
        let mut recent = Vec::new();
        for i in 0..10 {
            remember(&mut recent, &format!("p/{i}"));
        }
        remember(&mut recent, "p/5");
        assert_eq!(recent.len(), RECENT_LIMIT);
        assert_eq!(recent[..3], ["p/5", "p/9", "p/8"]);
    }
}
