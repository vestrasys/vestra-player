//! The mpv pass-through's rules, as data (capability `mpv`, SDK 1.8.0;
//! doc/architecture/README.md §4.9):
//! what a plugin may not send through `mpv.command`, `mpv.get`, `mpv.set`
//! and `mpv.observe`, by the boundary it would cross, and the channel's
//! limits. The list is closed: a command or property not named here goes
//! to mpv as it is. It is no sandbox (a plugin is a Node process); it keeps
//! the channel to what only the player can do, and keeps a plugin from
//! going around the state the player keeps of mpv.
//!
//! The SDK has the same table (`MPV_RULES`, `MPV_LIMITS`): it refuses early
//! with it and its TestHost models it. The committed `src/host/mpv-rules.json`
//! is generated from this file (`tests::json_is_current`;
//! `VESTRA_UPDATE_MPV_RULES=1 cargo test mpv_rules` rewrites it) and
//! src/tests/sdk/mpv-agreement.test.ts compares the two.
//!
//! The names and what they do are mpv's (its manual: List of Input
//! Commands, Properties, Options).
use serde_json::{Value, json};

/// Elements of one command, its name included.
pub const ARGS_MAX: usize = 32;
/// Characters of a command or property name.
pub const NAME_CHARS: usize = 128;
/// Characters of a filter label, after the `@`.
pub const LABEL_CHARS: usize = 64;
/// Different properties one plugin observes at the same time.
pub const OBSERVED_MAX: usize = 64;
/// A property's changes reach an observer at most this often (the newest
/// value wins), in milliseconds.
pub const OBSERVE_INTERVAL_MS: u64 = 100;

/// Why a command or a property is refused: the boundary it would cross.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Process,
    Code,
    Clients,
    Files,
    Quit,
    Input,
    Clipboard,
    Managed,
    Filters,
}
use Category::{Clients, Clipboard, Code, Files, Filters, Input, Managed, Process, Quit};

impl Category {
    #[cfg_attr(not(test), allow(dead_code))]
    const ALL: [Self; 9] = [
        Process, Code, Clients, Files, Quit, Input, Clipboard, Managed, Filters,
    ];
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            Process => "process",
            Code => "code",
            Clients => "clients",
            Files => "files",
            Quit => "quit",
            Input => "input",
            Clipboard => "clipboard",
            Managed => "managed",
            Filters => "filters",
        }
    }
    /// The end of the refusal's message.
    pub fn sentence(self) -> &'static str {
        match self {
            Process => "it starts a program (a plugin starts its own with Node)",
            Code => "it loads scripts or configuration into the player",
            Clients => "it opens a control channel to the player or talks to its other clients",
            Files => {
                "it makes the player write or delete files (a plugin writes its own with Node)"
            }
            Quit => "it ends the player",
            Input => {
                "the keyboard and the mouse are the player's (plugin.commands, plugin.bindings)"
            }
            Clipboard => "the clipboard has its own API (plugin.clipboard)",
            Managed => {
                "the player manages it and keeps its own state of it (use the plugin API's member for it)"
            }
            Filters => {
                "a plugin changes only its own labelled filters (vf/af add, pre, toggle or remove of @label:…, a label not starting with vestra-)"
            }
        }
    }
}

/// mpv commands that are refused, by name.
pub static COMMANDS: &[(&str, Category)] = &[
    // Programs.
    ("run", Process),
    ("subprocess", Process),
    // Scripts and configuration.
    ("load-script", Code),
    ("load-config-file", Code),
    ("load-input-conf", Code),
    ("apply-profile", Code),
    // Other clients of the player.
    ("script-message", Clients),
    ("script-message-to", Clients),
    ("script-binding", Clients),
    // Hooks make the player wait for a client at each load.
    ("hook-add", Clients),
    ("hook-continue", Clients),
    // Files the player would write or delete.
    ("screenshot", Files),
    ("screenshot-to-file", Files),
    ("write-watch-later-config", Files),
    ("delete-watch-later-config", Files),
    ("dump-cache", Files),
    ("ab-loop-dump-cache", Files),
    // The player's process.
    ("quit", Quit),
    ("quit-watch-later", Quit),
    // Input.
    ("keypress", Input),
    ("keydown", Input),
    ("keyup", Input),
    ("keybind", Input),
    ("mouse", Input),
    ("enable-section", Input),
    ("disable-section", Input),
    ("define-section", Input),
    ("begin-vo-dragging", Input),
    ("context-menu", Input),
    // The clipboard.
    ("update-clipboard", Clipboard),
    // What plays and the playlist (source_backend.rs).
    ("loadfile", Managed),
    ("loadlist", Managed),
    ("stop", Managed),
    ("playlist-next", Managed),
    ("playlist-prev", Managed),
    ("playlist-next-playlist", Managed),
    ("playlist-prev-playlist", Managed),
    ("playlist-play-index", Managed),
    ("playlist-clear", Managed),
    ("playlist-remove", Managed),
    ("playlist-move", Managed),
    ("playlist-shuffle", Managed),
    ("playlist-unshuffle", Managed),
    // The position.
    ("seek", Managed),
    ("revert-seek", Managed),
    ("sub-seek", Managed),
    ("sub-step", Managed),
    ("frame-step", Managed),
    ("frame-back-step", Managed),
    ("ab-loop", Managed),
    ("ab-loop-align-cache", Managed),
    // Tracks (source_backend.rs, subtitle_backend.rs).
    ("sub-add", Managed),
    ("sub-remove", Managed),
    ("sub-reload", Managed),
    ("audio-add", Managed),
    ("audio-remove", Managed),
    ("audio-reload", Managed),
    ("video-add", Managed),
    ("video-remove", Managed),
    ("video-reload", Managed),
    ("rescan-external-files", Managed),
    // The picture's overlays and snapshots (the OSD is the host's, drawn
    // in Slint; snapshots are media_jobs.rs).
    ("screenshot-raw", Managed),
    ("overlay-add", Managed),
    ("overlay-remove", Managed),
    ("osd-overlay", Managed),
    ("show-text", Managed),
    ("show-progress", Managed),
];

/// mpv properties and options whose write is refused (a read too for the
/// categories of [`READS`]), by the name before the first `/`.
pub static PROPERTIES: &[(&str, Category)] = &[
    // Programs (the ytdl hook runs one).
    ("ytdl", Process),
    ("ytdl-format", Process),
    ("ytdl-raw-options", Process),
    // Scripts and configuration.
    ("scripts", Code),
    ("script", Code),
    ("script-opts", Code),
    ("include", Code),
    ("profile", Code),
    ("config", Code),
    ("config-dir", Code),
    ("osc", Code),
    // Per-directory configuration files, read at each load.
    ("use-filedir-conf", Code),
    // Control channels.
    ("wid", Clients),
    // Files the player would write.
    ("stream-record", Files),
    ("stream-dump", Files),
    ("record-file", Files),
    ("log-file", Files),
    ("dump-stats", Files),
    ("o", Files),
    ("ao-pcm-file", Files),
    ("cache-dir", Files),
    ("demuxer-cache-dir", Files),
    ("icc-cache", Files),
    ("icc-cache-dir", Files),
    ("gpu-shader-cache", Files),
    ("gpu-shader-cache-dir", Files),
    ("save-position-on-quit", Files),
    // The player's process and window.
    ("idle", Quit),
    ("keep-open", Quit),
    ("keep-open-pause", Quit),
    ("force-window", Quit),
    ("cursor-autohide", Input),
    // The clipboard.
    ("clipboard", Clipboard),
    // Playback state (source_backend.rs, the host's own state of it).
    ("pause", Managed),
    ("speed", Managed),
    ("volume", Managed),
    ("mute", Managed),
    // The gain on top of the volume, and the system's own mixer.
    ("volume-gain", Managed),
    ("ao-volume", Managed),
    ("ao-mute", Managed),
    ("time-pos", Managed),
    ("playback-time", Managed),
    ("percent-pos", Managed),
    ("chapter", Managed),
    ("chapter-list", Managed),
    ("edition", Managed),
    ("start", Managed),
    ("end", Managed),
    ("length", Managed),
    ("loop", Managed),
    ("loop-file", Managed),
    ("loop-playlist", Managed),
    ("shuffle", Managed),
    ("ab-loop-a", Managed),
    ("ab-loop-b", Managed),
    ("ab-loop-count", Managed),
    ("audio-device", Managed),
    ("audio-delay", Managed),
    ("stop-screensaver", Managed),
    // Tracks and their files.
    ("aid", Managed),
    ("vid", Managed),
    ("sid", Managed),
    ("secondary-sid", Managed),
    ("audio", Managed),
    ("video", Managed),
    ("sub", Managed),
    ("audio-files", Managed),
    ("audio-file-auto", Managed),
    ("external-files", Managed),
    ("cover-art-auto", Managed),
    ("cover-art-files", Managed),
    ("audio-display", Managed),
    ("embeddedfonts", Managed),
    // Which tracks a load selects.
    ("alang", Managed),
    ("slang", Managed),
    ("vlang", Managed),
    // The filter chains as a whole, the output and the decoder
    // (processing.rs, audio_hold.rs, render_mode.rs).
    ("vf", Managed),
    ("af", Managed),
    // A complex graph takes the tracks and the chains from the player.
    ("lavfi-complex", Managed),
    ("vo", Managed),
    ("ao", Managed),
    ("hwdec", Managed),
    ("hwdec-software-fallback", Managed),
    ("gpu-api", Managed),
    ("gpu-context", Managed),
    ("deinterlace", Managed),
    ("framedrop", Managed),
    ("audio-stream-silence", Managed),
    // The renderer's options (video_render.rs, video_shaders.rs).
    ("glsl-shaders", Managed),
    ("video-sync", Managed),
    ("display-fps-override", Managed),
    ("interpolation", Managed),
    ("tone-mapping", Managed),
    ("gamut-mapping-mode", Managed),
    ("hdr-compute-peak", Managed),
    ("target-peak", Managed),
    // The display layer (video_view.rs).
    ("video-zoom", Managed),
    ("video-pan-x", Managed),
    ("video-pan-y", Managed),
    ("video-aspect-override", Managed),
    // The picture's orientation is the player's (source_geometry.rs).
    ("video-rotate", Managed),
    // Where the picture sits in the video area (video_view.rs lays it out).
    ("video-crop", Managed),
    ("video-scale-x", Managed),
    ("video-scale-y", Managed),
    ("keepaspect", Managed),
    ("panscan", Managed),
    // The window.
    ("fullscreen", Managed),
    ("fs", Managed),
    ("ontop", Managed),
    ("border", Managed),
    ("title", Managed),
    ("geometry", Managed),
    ("window-scale", Managed),
    ("window-minimized", Managed),
    ("window-maximized", Managed),
];

/// Properties refused by the start of their name, checked after
/// [`PROPERTIES`], in this order (`input-ipc-` before `input-`).
pub static PROPERTY_PREFIXES: &[(&str, Category)] = &[
    ("input-ipc-", Clients),
    ("input-", Input),
    ("load-", Code),
    ("clipboard-", Clipboard),
    ("screenshot-", Files),
    ("watch-later-", Files),
    ("playlist-", Managed),
    ("autofit", Managed),
    ("video-align-", Managed),
    // Subtitle selection, files and style (subtitle_backend.rs).
    ("sub-", Managed),
    ("secondary-sub-", Managed),
];

/// The categories whose properties may not be read either.
pub static READS: &[Category] = &[Clipboard];

/// Commands that write the property their first argument names
/// (`cycle-values` after an optional `!reverse`): checked as a write of it.
pub static WRITERS: &[&str] = &[
    "set",
    "del",
    "add",
    "multiply",
    "cycle",
    "cycle-values",
    "change-list",
];

/// The filter chain commands, and the operations a plugin may use with
/// them, on its own labelled filters only.
pub static FILTER_STAGES: &[&str] = &["vf", "af"];
pub static FILTER_OPERATIONS: &[&str] = &["add", "pre", "toggle", "remove"];
/// Commands whose first argument is a filter label, which must be the
/// plugin's own, with the stage each addresses.
pub static FILTER_MESSAGES: &[(&str, &str)] = &[("vf-command", "vf"), ("af-command", "af")];
/// The start of the player's own filter labels (`@vestra-owned`,
/// `@vestra-orientation`, ...): no plugin's.
/// mpv's command prefixes (its input/cmd.c `cmd_flags`, v0.41.0, the whole
/// list). The node array form takes them too: mpv consumes every leading
/// prefix word before it reads the command's name, so `["no-osd", "quit"]`
/// would run `quit` past every table here, and `expand-properties` would
/// turn property expansion on. No command starts with one.
pub static PREFIXES: &[&str] = &[
    "no-osd",
    "osd-bar",
    "osd-msg",
    "osd-msg-bar",
    "osd-auto",
    "expand-properties",
    "raw",
    "repeatable",
    "nonrepeatable",
    "nonscalable",
    "async",
    "sync",
];
pub const RESERVED_LABEL: &str = "vestra-";
/// Labels with a meaning of mpv's own, no plugin's either: `all` addresses
/// every filter of a chain in `vf-command` / `af-command`, the player's
/// included.
pub static RESERVED_LABELS: &[&str] = &["all"];

/// A command or property name as mpv writes one:
/// `^[a-z0-9][a-z0-9/_-]*$`, case ignored, at most [`NAME_CHARS`].
fn is_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= NAME_CHARS
        && bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-'))
}

/// What is wrong with a property name (`invalid`); `None`: nothing.
pub fn name_problem(name: &Value) -> Option<String> {
    (!name.as_str().is_some_and(is_name)).then(|| {
        format!(
            "a property name is 1–{NAME_CHARS} characters of letters, digits, \"-\", \"_\" and \"/\""
        )
    })
}

/// What is wrong with a command's shape (`invalid`); `None`: nothing. The
/// first element is the command's name, the others its arguments: strings,
/// finite numbers or booleans.
pub fn command_problem(args: &Value) -> Option<String> {
    let Some(args) = args
        .as_array()
        .filter(|args| (1..=ARGS_MAX).contains(&args.len()))
    else {
        return Some(format!(
            "a command is an array of 1–{ARGS_MAX} elements, its name first"
        ));
    };
    // A name, and no prefix word of mpv's: it would take the next element
    // for the command. Looked up as the tables are (`_` for `-`): mpv does
    // not read `no_osd` as a prefix, but nothing by that name is a command
    // either.
    if !args[0].as_str().is_some_and(is_name) || PREFIXES.contains(&command_key(args).as_str()) {
        return Some(
            "a command's first element is its name (no prefixes such as no-osd or async)".into(),
        );
    }
    // A JSON number is finite.
    if args[1..]
        .iter()
        .any(|arg| !(arg.is_string() || arg.is_number() || arg.is_boolean()))
    {
        return Some("a command's arguments are strings, finite numbers or booleans".into());
    }
    // A message to a filter names the filter (a string): without a label
    // there is nothing to check the owner of. The SDK's words.
    (!args.get(1).is_some_and(Value::is_string)
        && FILTER_MESSAGES
            .iter()
            .any(|(command, _)| *command == command_key(args)))
    .then(|| "vf-command and af-command take a filter label as their first argument".into())
}

fn refusal(what: &str, name: &str, category: Category) -> String {
    // The name as JSON writes it (the SDK's `JSON.stringify`).
    format!(
        "mpv {what} {} is refused: {}",
        Value::from(name),
        category.sentence()
    )
}

/// The refusal of a filter chain command (`vf`, `af`) that would change
/// more than the caller's own labelled filter.
pub fn filters_refusal(stage: &str) -> String {
    refusal("command", stage, Filters)
}

/// The rule's name of a property: without an `options/` or
/// `file-local-options/` prefix and without its sub-path.
fn property_key(name: &str) -> &str {
    let bare = name
        .strip_prefix("file-local-options/")
        .or_else(|| name.strip_prefix("options/"))
        .unwrap_or(name);
    bare.split('/').next().unwrap_or(bare)
}

/// The category that refuses a property, for a read or a `write`.
pub fn property_category(name: &str, write: bool) -> Option<Category> {
    let key = property_key(name);
    let category = PROPERTIES
        .iter()
        .find(|(exact, _)| *exact == key)
        .or_else(|| {
            PROPERTY_PREFIXES
                .iter()
                .find(|(prefix, _)| key.starts_with(prefix))
        })
        .map(|(_, category)| *category)?;
    (write || READS.contains(&category)).then_some(category)
}

/// Why a property is refused (`refused`); `None`: it is not.
pub fn property_refusal(name: &str, write: bool) -> Option<String> {
    property_category(name, write).map(|category| refusal("property", name, category))
}

/// A command's name as mpv reads it, for every lookup: mpv takes `_` for
/// `-` in a command's name (`cycle_values` is `cycle-values`), so the rules
/// do too. Nothing else is folded: mpv knows no command or property in
/// another case or with `_` in a property's name (it answers those itself).
fn command_key(args: &[Value]) -> String {
    args[0].as_str().unwrap_or_default().replace('_', "-")
}

/// The label of a filter value, `@label` or `@label:…`: 1–[`LABEL_CHARS`]
/// of letters, digits, `.`, `_`, `-`, not the player's and not one of
/// mpv's own ([`RESERVED_LABELS`]).
fn filter_label(value: &str) -> Option<&str> {
    let label = value.strip_prefix('@')?.split(':').next()?;
    ((1..=LABEL_CHARS).contains(&label.len())
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !label.starts_with(RESERVED_LABEL)
        && !RESERVED_LABELS.contains(&label))
    .then_some(label)
}

/// A command argument as the SDK's `String(value)` reads it, for the
/// places a rule looks at one.
fn shown(arg: Option<&Value>) -> String {
    match arg {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "undefined".into(),
    }
}

/// Why a command is refused (`refused`); `None`: it is not. Its shape was
/// checked ([`command_problem`]). What only the worker can tell is left to
/// it (playback/mpv_passthrough.rs): whose a label is, and whether a filter
/// value holds more than the one labelled filter.
pub fn command_refusal(args: &[Value]) -> Option<String> {
    // The name as given in the message, as mpv reads it for the rules.
    let given = args[0].as_str()?;
    let key = command_key(args);
    let name = key.as_str();
    if let Some((_, category)) = COMMANDS.iter().find(|(command, _)| *command == name) {
        return Some(refusal("command", given, *category));
    }
    if WRITERS.contains(&name) {
        let reverse =
            name == "cycle-values" && args.get(1).and_then(Value::as_str) == Some("!reverse");
        let target = args.get(if reverse { 2 } else { 1 })?.as_str()?;
        return property_refusal(target, true);
    }
    if FILTER_STAGES.contains(&name) {
        let allowed = FILTER_OPERATIONS.contains(&shown(args.get(1)).as_str())
            && args
                .get(2)
                .and_then(Value::as_str)
                .is_some_and(|value| filter_label(value).is_some());
        return (!allowed).then(|| refusal("command", given, Filters));
    }
    if FILTER_MESSAGES.iter().any(|(command, _)| *command == name) {
        let label = format!("@{}", shown(args.get(1)));
        return filter_label(&label)
            .is_none()
            .then(|| refusal("command", given, Filters));
    }
    None
}

/// A filter chain command's change: its stage (`vf`, `af`) and the label
/// it adds, replaces or removes.
#[derive(Debug, PartialEq, Eq)]
pub struct FilterChange<'a> {
    pub stage: &'static str,
    pub label: &'a str,
}

/// The change of a filter chain command that passed [`command_refusal`];
/// `None` for any other command.
pub fn filter_change(args: &[Value]) -> Option<FilterChange<'_>> {
    let key = command_key(args);
    let stage = FILTER_STAGES.iter().find(|stage| **stage == key)?;
    Some(FilterChange {
        stage,
        label: filter_label(args.get(2)?.as_str()?)?,
    })
}

/// The stage and label a filter message (`vf-command`, `af-command`) that
/// passed [`command_refusal`] addresses; `None` for any other command.
pub fn filter_message(args: &[Value]) -> Option<(&'static str, String)> {
    let key = command_key(args);
    let (_, stage) = FILTER_MESSAGES
        .iter()
        .find(|(command, _)| *command == key)?;
    let label = format!("@{}", shown(args.get(1)));
    filter_label(&label).map(|label| (*stage, label.to_owned()))
}

/// `src/host/mpv-rules.json`: the tables and limits as data, in the shape of
/// the SDK's `MPV_RULES` and `MPV_LIMITS` (the prefixes as a list: their
/// order matters).
#[cfg_attr(not(test), allow(dead_code))]
pub fn json() -> Value {
    let table = |rows: &[(&str, Category)]| {
        rows.iter()
            .map(|(name, category)| ((*name).to_owned(), Value::from(category.name())))
            .collect::<serde_json::Map<_, _>>()
    };
    json!({
        "about": "Generated from src/host/src/playback/mpv_rules.rs (VESTRA_UPDATE_MPV_RULES=1 cargo test mpv_rules); do not edit. \
            The mpv pass-through's closed refusal rules and limits; the SDK's MPV_RULES and MPV_LIMITS hold the same.",
        "limits": {
            "args": ARGS_MAX,
            "nameChars": NAME_CHARS,
            "labelChars": LABEL_CHARS,
            "observed": OBSERVED_MAX,
            "observeIntervalMs": OBSERVE_INTERVAL_MS,
        },
        "rules": {
            "categories": Category::ALL.iter()
                .map(|category| (category.name().to_owned(), Value::from(category.sentence())))
                .collect::<serde_json::Map<_, _>>(),
            "commands": table(COMMANDS),
            "properties": table(PROPERTIES),
            "propertyPrefixes": PROPERTY_PREFIXES.iter()
                .map(|(prefix, category)| json!([prefix, category.name()]))
                .collect::<Vec<_>>(),
            "reads": READS.iter().map(|category| category.name()).collect::<Vec<_>>(),
            "writers": WRITERS,
            "filterCommands": FILTER_STAGES.iter()
                .map(|stage| ((*stage).to_owned(), json!(FILTER_OPERATIONS)))
                .collect::<serde_json::Map<_, _>>(),
            "filterMessages": FILTER_MESSAGES.iter().map(|(command, _)| *command).collect::<Vec<_>>(),
            "reservedLabel": RESERVED_LABEL,
            "reservedLabels": RESERVED_LABELS,
            "prefixes": PREFIXES,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn json_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/mpv-rules.json");
        let generated = serde_json::to_string_pretty(&json()).unwrap() + "\n";
        if std::env::var("VESTRA_UPDATE_MPV_RULES").as_deref() == Ok("1") {
            std::fs::write(path, &generated).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == generated,
            "src/host/mpv-rules.json is not what playback/mpv_rules.rs generates; \
             rerun with VESTRA_UPDATE_MPV_RULES=1 and commit the file"
        );
    }

    #[test]
    fn every_name_is_listed_once_and_no_prefix_hides_a_later_one() {
        let mut commands = BTreeSet::new();
        for (name, _) in COMMANDS {
            assert!(commands.insert(*name), "{name} listed twice");
            assert!(is_name(name), "{name}");
            // A refused command is not also a writer or a filter command.
            assert!(
                !WRITERS.contains(name) && !FILTER_STAGES.contains(name),
                "{name}"
            );
        }
        let mut properties = BTreeSet::new();
        for (name, _) in PROPERTIES {
            assert!(properties.insert(*name), "{name} listed twice");
            assert!(is_name(name) && !name.contains('/'), "{name}");
        }
        for (at, (prefix, category)) in PROPERTY_PREFIXES.iter().enumerate() {
            for (earlier, other) in &PROPERTY_PREFIXES[..at] {
                assert!(
                    !prefix.starts_with(earlier) || other == category,
                    "{earlier} hides {prefix}"
                );
            }
        }
    }

    fn args(list: &[&str]) -> Vec<Value> {
        list.iter().map(|arg| Value::from(*arg)).collect()
    }

    #[test]
    fn a_command_is_an_array_of_a_name_and_plain_arguments() {
        for good in [
            json!(["frame-drop-count"]),
            json!(["set", "brightness", 5]),
            json!(["cycle-values", "!reverse", "video-unscaled", 0, 90.5, true]),
            json!(["vf", "add", "@mine:lavfi=[hflip]"]),
            json!(["Seek", 1]),
            Value::Array(vec![Value::from("x"); ARGS_MAX]),
        ] {
            assert_eq!(command_problem(&good), None, "{good}");
        }
        let count = "a command is an array of 1–32 elements, its name first";
        let name = "a command's first element is its name (no prefixes such as no-osd or async)";
        let plain = "a command's arguments are strings, finite numbers or booleans";
        for (bad, message) in [
            (json!("seek 1"), count),
            (json!(null), count),
            (json!([]), count),
            (Value::Array(vec![Value::from("x"); ARGS_MAX + 1]), count),
            (json!([1, 2]), name),
            (json!([""]), name),
            (json!(["no-osd seek", 1]), name),
            (json!(["-seek"]), name),
            (json!(["x".repeat(129)]), name),
            (json!(["set", null]), plain),
            (json!(["set", ["a"]]), plain),
            (json!(["set", {"a": 1}]), plain),
        ] {
            assert_eq!(command_problem(&bad).as_deref(), Some(message), "{bad}");
        }
        // A string argument may hold anything: only the name is a name.
        assert_eq!(command_problem(&json!(["set", "a b", "${x}\n"])), None);
        let property =
            "a property name is 1–128 characters of letters, digits, \"-\", \"_\" and \"/\"";
        for good in [
            "brightness",
            "options/vf",
            "track-list/0/id",
            "Volume_x",
            &"x".repeat(128),
        ] {
            assert_eq!(name_problem(&json!(good)), None, "{good}");
        }
        for bad in [
            json!(""),
            json!("a b"),
            json!("${x}"),
            json!(3),
            json!(null),
            json!("x".repeat(129)),
            json!("/x"),
        ] {
            assert_eq!(name_problem(&bad).as_deref(), Some(property), "{bad}");
        }
    }

    #[test]
    fn what_crosses_another_boundary_is_refused_with_its_reason() {
        let refused = |list: &[&str]| command_refusal(&args(list));
        assert_eq!(
            refused(&["run", "calc.exe"]).as_deref(),
            Some(
                "mpv command \"run\" is refused: it starts a program (a plugin starts its own with Node)"
            )
        );
        assert_eq!(
            refused(&["quit"]).as_deref(),
            Some("mpv command \"quit\" is refused: it ends the player")
        );
        for (command, category) in COMMANDS {
            let message = refused(&[command, "x"]).unwrap();
            assert!(message.ends_with(category.sentence()), "{command}");
        }
        // What is not named goes to mpv.
        for passed in [
            &["frame-drop-count"][..],
            &["set", "brightness", "5"],
            &["add", "contrast", "1"],
            &["cycle", "deband"],
            &["vf-command", "mine", "strength", "2"],
            &["expand-text", "${filename}"],
            &["set"],
        ] {
            assert_eq!(refused(passed), None, "{passed:?}");
        }
        // The commands that write a property are checked as a write of it.
        for writer in WRITERS {
            assert_eq!(
                refused(&[writer, "pause", "yes"]).as_deref(),
                Some(
                    "mpv property \"pause\" is refused: the player manages it and keeps its own state of it (use the plugin API's member for it)"
                ),
                "{writer}"
            );
            assert_eq!(refused(&[writer, "brightness", "1"]), None, "{writer}");
        }
        assert!(refused(&["cycle-values", "!reverse", "hwdec", "no", "auto"]).is_some());
        assert_eq!(
            refused(&["cycle-values", "!reverse", "deband-iterations", "0", "90"]),
            None
        );
        assert!(refused(&["change-list", "glsl-shaders", "append", "x.glsl"]).is_some());
        assert!(refused(&["change-list", "vf", "append", "hflip"]).is_some());
    }

    #[test]
    fn a_property_is_looked_up_without_its_options_prefix_and_sub_path() {
        let write = |name: &str| property_category(name, true);
        assert_eq!(write("pause"), Some(Managed));
        assert_eq!(write("options/pause"), Some(Managed));
        assert_eq!(write("file-local-options/hwdec"), Some(Managed));
        assert_eq!(write("vf/0/enabled"), Some(Managed));
        assert_eq!(write("options/vf/0/enabled"), Some(Managed));
        // Exact names first, then the prefixes in their order.
        assert_eq!(write("input-ipc-server"), Some(Clients));
        assert_eq!(write("input-default-bindings"), Some(Input));
        assert_eq!(write("load-scripts"), Some(Code));
        assert_eq!(write("sub-delay"), Some(Managed));
        assert_eq!(write("secondary-sub-pos"), Some(Managed));
        assert_eq!(write("autofit-larger"), Some(Managed));
        assert_eq!(write("screenshot-directory"), Some(Files));
        assert_eq!(write("playlist-pos"), Some(Managed));
        // What the host itself writes is managed (video_render.rs, video_view.rs).
        for name in [
            "interpolation",
            "keepaspect",
            "panscan",
            "video-zoom",
            "tone-mapping",
        ] {
            assert_eq!(write(name), Some(Managed), "{name}");
        }
        for free in [
            "brightness",
            "contrast",
            "video-unscaled",
            "subtitle",
            "pausex",
            "volume-max",
            "user-data/mine/x",
            "deband",
        ] {
            assert_eq!(write(free), None, "{free}");
        }
        // A read is refused for the clipboard only.
        for (name, _) in PROPERTIES.iter().chain(PROPERTY_PREFIXES) {
            let refused = property_category(name, false);
            assert_eq!(refused.is_some(), refused == Some(Clipboard), "{name}");
        }
        assert_eq!(property_category("clipboard/text", false), Some(Clipboard));
        assert_eq!(
            property_category("clipboard-backends", false),
            Some(Clipboard)
        );
        assert_eq!(
            property_refusal("clipboard/text", false).as_deref(),
            Some(
                "mpv property \"clipboard/text\" is refused: the clipboard has its own API (plugin.clipboard)"
            )
        );
        assert_eq!(property_refusal("time-pos", false), None);
    }

    /// `Managed` and the other categories follow the host: every mpv
    /// property the player itself sets is refused to a plugin's write. The
    /// names come from the code that writes them where it lists them; the
    /// rest are named by their file. A new host feature that writes an mpv
    /// option adds it here and to `PROPERTIES` (the SDK's copy follows).
    #[test]
    fn what_the_player_writes_is_managed() {
        use crate::playback::{backend, subtitle_style, video_render, video_view};
        let mut written: Vec<(String, &str)> = Vec::new();
        // The renderer's options, with and without a plugin's settings.
        for (name, _) in video_render::properties(None, 60.) {
            written.push((name.into(), "video_render.rs"));
        }
        // The display layer.
        for (name, _) in video_view::Settings::default().properties() {
            written.push((name.into(), "video_view.rs"));
        }
        // The subtitle style.
        let style = subtitle_style::default_for("font".into());
        for (name, _) in subtitle_style::mpv_properties(&style, "Family").unwrap() {
            written.push((name.into(), "subtitle_style.rs"));
        }
        // mpv's own scripts, off at start.
        for name in backend::BUILTIN_SCRIPTS_OFF {
            written.push((name.into(), "backend.rs no_builtin_scripts"));
        }
        for (name, file) in [
            // The player's mpv as it is built.
            ("vo", "backend.rs"),
            ("ao", "backend.rs"),
            ("hwdec", "backend.rs, processing.rs"),
            ("hwdec-software-fallback", "backend.rs"),
            ("keep-open", "backend.rs"),
            ("sub-auto", "backend.rs"),
            (
                "vf",
                "backend.rs, processing.rs, source_geometry.rs, upscaling.rs, motion.rs",
            ),
            ("af", "processing.rs"),
            ("sub-fonts-dir", "backend.rs"),
            ("sub-font-provider", "backend.rs"),
            ("embeddedfonts", "backend.rs"),
            ("audio-stream-silence", "backend.rs, audio_hold.rs"),
            ("gpu-shader-cache-dir", "backend.rs"),
            ("audio-display", "backend.rs CoverArt"),
            ("cover-art-auto", "backend.rs CoverArt"),
            ("mute", "backend.rs"),
            ("volume", "backend.rs"),
            // Playback and tracks.
            ("pause", "source_backend.rs"),
            ("speed", "source_backend.rs"),
            ("ab-loop-a", "source_backend.rs"),
            ("ab-loop-b", "source_backend.rs"),
            ("ab-loop-count", "source_backend.rs"),
            ("audio-device", "source_backend.rs"),
            ("aid", "source_backend.rs"),
            ("sid", "subtitle_backend.rs"),
            ("secondary-sid", "subtitle_backend.rs"),
            // The processing chain.
            ("deinterlace", "processing.rs"),
            ("framedrop", "processing.rs"),
            ("audio-delay", "processing.rs"),
            // Plugin shaders.
            ("glsl-shaders", "video_shaders.rs"),
        ] {
            written.push((name.into(), file));
        }
        for (name, file) in &written {
            assert!(
                property_category(name, true).is_some(),
                "{name} is written by the player ({file}) and not refused to a plugin"
            );
        }
        // What the display layer and the renderer write is theirs alone.
        for name in [
            "interpolation",
            "keepaspect",
            "panscan",
            "video-zoom",
            "tone-mapping",
        ] {
            assert_eq!(property_category(name, true), Some(Managed), "{name}");
        }
    }

    #[test]
    fn a_filter_command_names_one_own_label() {
        let refused = |list: &[&str]| command_refusal(&args(list));
        let filters = "mpv command \"vf\" is refused: a plugin changes only its own labelled filters (vf/af add, pre, toggle or remove of @label:…, a label not starting with vestra-)";
        for stage in ["vf", "af"] {
            for operation in FILTER_OPERATIONS {
                assert_eq!(refused(&[stage, operation, "@mine:lavfi=[hflip]"]), None);
                assert_eq!(refused(&[stage, operation, "@a.b_c-9"]), None);
            }
        }
        for bad in [
            &["vf", "set", "@mine:hflip"][..],
            &["vf", "clr", ""],
            &["vf", "del", "0"],
            &["vf", "add", "hflip"],
            &["vf", "add", "@:hflip"],
            &["vf", "add", "@vestra-owned:lavfi=[null]"],
            &["vf", "remove", "@vestra-orientation"],
            &["vf", "add", "@my label:hflip"],
            &["vf", "add"],
            &["vf"],
        ] {
            assert_eq!(refused(bad).as_deref(), Some(filters), "{bad:?}");
        }
        let long = format!("@{}:hflip", "x".repeat(65));
        assert!(refused(&["af", "add", &long]).is_some());
        assert!(refused(&["af", "add", &format!("@{}:volume=1", "x".repeat(64))]).is_none());
        assert_eq!(
            filter_change(&args(&["af", "pre", "@loud:lavfi=[volume=2]"])),
            Some(FilterChange {
                stage: "af",
                label: "loud"
            })
        );
        assert_eq!(
            filter_change(&args(&["vf", "remove", "@mine"]))
                .unwrap()
                .label,
            "mine"
        );
        assert_eq!(filter_change(&args(&["set", "vf", "x"])), None);
        // A filter message names its label without the `@`.
        assert_eq!(refused(&["vf-command", "mine", "angle", "1"]), None);
        assert!(refused(&["vf-command", "vestra-owned", "angle", "1"]).is_some());
        assert!(refused(&["af-command", "", "volume", "1"]).is_some());
        // Without the label it is for, a filter message is no command
        // (`invalid`, checked with the shape).
        for message in ["vf-command", "af-command"] {
            let problem = "vf-command and af-command take a filter label as their first argument";
            for bad in [
                json!([message]),
                json!([message, 5]),
                json!([message, true, "x"]),
            ] {
                assert_eq!(command_problem(&bad).as_deref(), Some(problem), "{bad}");
            }
            assert_eq!(command_problem(&json!([message, "mine"])), None);
        }
        // What a plugin could stall or turn the player with.
        for command in ["hook-add", "hook-continue"] {
            assert!(
                refused(&[command, "on_load", "1", "0"])
                    .unwrap()
                    .ends_with(Clients.sentence())
            );
        }
        assert!(
            refused(&["sub-step", "1"])
                .unwrap()
                .ends_with(Managed.sentence())
        );
        for name in ["lavfi-complex", "video-rotate"] {
            assert_eq!(property_category(name, true), Some(Managed), "{name}");
            assert_eq!(property_category(name, false), None, "{name}");
            assert!(refused(&["cycle", name]).is_some(), "{name}");
        }
        assert_eq!(
            filter_message(&args(&["af-command", "loud", "volume", "1"])),
            Some(("af", "loud".to_owned()))
        );
        assert_eq!(filter_message(&args(&["vf", "add", "@mine:hflip"])), None);
    }

    /// mpv reads `_` as `-` in a command's name (libmpv 0.41: `cycle_values`
    /// runs `cycle-values`), so every table is looked up by the name as mpv
    /// reads it; the message keeps the name as given.
    #[test]
    fn an_underscore_in_a_commands_name_is_a_hyphen_for_every_table() {
        let refused = |list: &[&str]| command_refusal(&args(list));
        // The refused commands, each under its `_` spelling.
        for (command, category) in COMMANDS {
            let spelled = command.replace('-', "_");
            assert_eq!(
                refused(&[spelled.as_str(), "x"]),
                Some(format!(
                    "mpv command {} is refused: {}",
                    Value::from(spelled.as_str()),
                    category.sentence()
                )),
                "{command}"
            );
        }
        for spelled in [
            "playlist_clear",
            "load_script",
            "quit_watch_later",
            "screenshot_to_file",
            "hook_add",
            "script_message_to",
            "playlist-play_index",
        ] {
            assert!(refused(&[spelled, "x"]).is_some(), "{spelled}");
        }
        // The writers: a write of their first argument.
        for writer in WRITERS {
            let spelled = writer.replace('-', "_");
            assert_eq!(
                refused(&[spelled.as_str(), "pause", "yes"]),
                refused(&[writer, "pause", "yes"]),
                "{writer}"
            );
            assert!(
                refused(&[spelled.as_str(), "pause", "yes"]).is_some(),
                "{writer}"
            );
            assert_eq!(
                refused(&[spelled.as_str(), "brightness", "1"]),
                None,
                "{writer}"
            );
        }
        assert!(refused(&["cycle_values", "!reverse", "hwdec", "no", "auto"]).is_some());
        assert!(refused(&["change_list", "glsl-shaders", "append", "x.glsl"]).is_some());
        // The filter messages: the label is checked, and the worker finds
        // the stage and the label for the owner's check.
        for (message, stage) in FILTER_MESSAGES {
            let spelled = message.replace('-', "_");
            assert_eq!(
                refused(&[spelled.as_str(), "vestra-owned", "x", "1"]),
                Some(format!(
                    "mpv command {} is refused: {}",
                    Value::from(spelled.as_str()),
                    Filters.sentence()
                ))
            );
            assert!(refused(&[spelled.as_str(), "all", "x", "1"]).is_some());
            assert_eq!(refused(&[spelled.as_str(), "mine", "x", "1"]), None);
            assert_eq!(
                filter_message(&args(&[spelled.as_str(), "mine", "x", "1"])),
                Some((*stage, "mine".to_owned()))
            );
            assert_eq!(
                command_problem(&json!([spelled])).as_deref(),
                Some("vf-command and af-command take a filter label as their first argument")
            );
        }
        // The stages have no `-` to spell otherwise; a name that only looks
        // like one is no command of the rules (mpv answers it).
        for stage in FILTER_STAGES {
            assert!(!stage.contains('-'));
            assert!(refused(&[*stage, "set", "x"]).is_some());
        }
        assert_eq!(refused(&["v_f", "set", "x"]), None);
        // Nothing else is folded: mpv knows no command in another case and
        // no property with `_` (it answers `property not found`).
        assert_eq!(refused(&["QUIT"]), None);
        assert_eq!(property_category("sub_delay", true), None);
    }

    /// `all` is mpv's own label (every filter of the chain): no plugin's,
    /// as a filter's label or in a filter message.
    #[test]
    fn the_label_all_is_reserved() {
        let refused = |list: &[&str]| command_refusal(&args(list));
        for stage in FILTER_STAGES {
            for operation in FILTER_OPERATIONS {
                assert!(refused(&[*stage, *operation, "@all:lavfi=[null]"]).is_some());
                assert!(refused(&[*stage, *operation, "@all"]).is_some());
            }
        }
        for (message, _) in FILTER_MESSAGES {
            assert_eq!(
                refused(&[*message, "all", "x", "1"]),
                Some(format!(
                    "mpv command \"{message}\" is refused: {}",
                    Filters.sentence()
                ))
            );
            assert_eq!(filter_message(&args(&[*message, "all", "x", "1"])), None);
        }
        assert_eq!(
            filter_change(&args(&["vf", "add", "@all:lavfi=[null]"])),
            None
        );
        // Only that word: a longer or differently cased label is a label.
        for label in ["@ALL:x", "@all2:x", "@all.mine:x", "@small:x"] {
            assert_eq!(refused(&["vf", "add", label]), None, "{label}");
        }
        assert_eq!(RESERVED_LABELS, ["all"]);
    }

    /// What else of the player's a property would go around (the P2
    /// review): per-directory configuration, the volume beside the
    /// player's, the play order, the chapters, where the picture sits,
    /// which tracks a load selects.
    #[test]
    fn what_goes_around_the_players_state_is_refused() {
        let write = |name: &str| property_category(name, true);
        assert_eq!(write("use-filedir-conf"), Some(Code));
        for name in [
            "volume-gain",
            "ao-volume",
            "ao-mute",
            "shuffle",
            "chapter-list",
            "video-crop",
            "video-scale-x",
            "video-scale-y",
            "video-align-x",
            "video-align-y",
            "alang",
            "slang",
            "vlang",
        ] {
            assert_eq!(write(name), Some(Managed), "{name}");
            // Reading stays open.
            assert_eq!(property_category(name, false), None, "{name}");
        }
        // Their neighbours stay free.
        for free in [
            "volume-max",
            "video-unscaled",
            "video-margin-ratio-left",
            "ao-null-untimed",
        ] {
            assert_eq!(write(free), None, "{free}");
        }
    }

    /// mpv's node array form consumes leading prefix words before the
    /// command's name (input/cmd.c `cmd_node_array`), so one in the name's
    /// place is no command: `invalid`, whatever follows it.
    #[test]
    fn a_prefix_word_is_no_commands_name() {
        let message = "a command's first element is its name (no prefixes such as no-osd or async)";
        assert_eq!(
            PREFIXES,
            [
                "no-osd",
                "osd-bar",
                "osd-msg",
                "osd-msg-bar",
                "osd-auto",
                "expand-properties",
                "raw",
                "repeatable",
                "nonrepeatable",
                "nonscalable",
                "async",
                "sync"
            ]
        );
        for prefix in PREFIXES {
            for rest in [
                json!([prefix]),
                json!([prefix, "quit"]),
                json!([prefix, "set", "pause", "no"]),
                json!([prefix, "frame-drop-count"]),
                json!([prefix, "async", "run", "calc.exe"]),
            ] {
                assert_eq!(command_problem(&rest).as_deref(), Some(message), "{rest}");
            }
            // Under the `_` spelling too (mpv itself takes that for an
            // unknown command).
            let spelled = prefix.replace('-', "_");
            assert_eq!(
                command_problem(&json!([spelled, "quit"])).as_deref(),
                Some(message),
                "{spelled}"
            );
            // No table names one, so none is ever a refused or free command.
            assert!(COMMANDS.iter().all(|(command, _)| command != prefix));
            assert!(!WRITERS.contains(prefix) && !FILTER_STAGES.contains(prefix));
        }
        // The reviewer's five.
        for probe in [
            json!(["no-osd", "set", "pause", "no"]),
            json!(["async", "cycle", "pause"]),
            json!(["osd-auto", "raw", "playlist-clear"]),
            json!(["expand-properties", "set", "title", "${pause}"]),
            json!(["no-osd", "quit"]),
        ] {
            assert_eq!(command_problem(&probe).as_deref(), Some(message), "{probe}");
        }
        // A prefix word later in the array is an argument like any other,
        // and a command that only starts like one is a command.
        assert_eq!(command_problem(&json!(["set", "brightness", "raw"])), None);
        assert_eq!(command_problem(&json!(["print-text", "async"])), None);
        assert_eq!(command_problem(&json!(["rawr"])), None);
        assert_eq!(command_problem(&json!(["sync-playback"])), None);
    }
}
