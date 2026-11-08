//! Commands whose full id (`"<manifest id>/<command id>"`) changed between
//! releases, and the one-time rewrite of what the player saved under the old
//! id: the user's key overrides (`bindings`) and the command palette's recent
//! list (`paletteRecent`). Plugin data (for example a plugin's own record of
//! command ids) is the plugin's to migrate.
//!
//! A rename is a plain `(old, new)` pair: a plugin command that became a core
//! action, or a command that moved from one plugin to another. Ids not in the
//! table are left alone, and applying the table again changes nothing, so it
//! runs at every start.

use std::collections::BTreeMap;

/// Every renamed command, oldest move first. The new id never appears as an
/// old id, so one pass is final.
pub const COMMAND_RENAMES: [(&str, &str); 30] = [
    // Host 0.5 (core actions): playback-plugin commands the host now owns.
    ("vestra.playback/toggle-pause", "host/play-pause"),
    ("vestra.playback/close-source", "host/stop"),
    ("vestra.playback/previous", "host/previous"),
    ("vestra.playback/next", "host/next"),
    ("vestra.playback/backward-5", "host/seek-backward"),
    ("vestra.playback/forward-5", "host/seek-forward"),
    ("vestra.playback/rate-down", "host/rate-down"),
    ("vestra.playback/rate-up", "host/rate-up"),
    ("vestra.playback/rate-10", "host/rate-normal"),
    ("vestra.playback/volume-up", "host/volume-up"),
    ("vestra.playback/volume-down", "host/volume-down"),
    ("vestra.playback/mute", "host/mute"),
    // 2026-11-08: the A-B Loop moved from Chapters and Bookmarks
    // (`vestra.marks` 0.1.8) to Basic Playback (`vestra.playback` 0.2.18)
    // with the same local ids.
    ("vestra.marks/ab.set-a", "vestra.playback/ab.set-a"),
    ("vestra.marks/ab.clear-a", "vestra.playback/ab.clear-a"),
    ("vestra.marks/ab.a-back", "vestra.playback/ab.a-back"),
    ("vestra.marks/ab.a-forward", "vestra.playback/ab.a-forward"),
    ("vestra.marks/ab.set-b", "vestra.playback/ab.set-b"),
    ("vestra.marks/ab.clear-b", "vestra.playback/ab.clear-b"),
    ("vestra.marks/ab.b-back", "vestra.playback/ab.b-back"),
    ("vestra.marks/ab.b-forward", "vestra.playback/ab.b-forward"),
    ("vestra.marks/ab.back", "vestra.playback/ab.back"),
    ("vestra.marks/ab.forward", "vestra.playback/ab.forward"),
    ("vestra.marks/ab.segment", "vestra.playback/ab.segment"),
    ("vestra.marks/ab.toggle", "vestra.playback/ab.toggle"),
    (
        "vestra.marks/ab.count.endless",
        "vestra.playback/ab.count.endless",
    ),
    ("vestra.marks/ab.count.1", "vestra.playback/ab.count.1"),
    ("vestra.marks/ab.count.2", "vestra.playback/ab.count.2"),
    ("vestra.marks/ab.count.3", "vestra.playback/ab.count.3"),
    ("vestra.marks/ab.count.5", "vestra.playback/ab.count.5"),
    ("vestra.marks/ab.count.10", "vestra.playback/ab.count.10"),
];

/// The current id of a command saved as `global`.
pub fn renamed(global: &str) -> &str {
    COMMAND_RENAMES
        .iter()
        .find(|(old, _)| *old == global)
        .map_or(global, |(_, new)| new)
}

/// Move overrides saved under an old id to the new one; an override already
/// under the new id wins and the old one is dropped. Returns whether
/// anything changed.
pub fn migrate_overlay(overlay: &mut BTreeMap<String, Vec<String>>) -> bool {
    let mut changed = false;
    for (old, new) in COMMAND_RENAMES {
        if let Some(bindings) = overlay.remove(old) {
            changed = true;
            overlay.entry(new.to_owned()).or_insert(bindings);
        }
    }
    changed
}

/// Rename the palette's recent commands in place (newest first); when both
/// ids were there, the newer entry stays and the other goes. Returns
/// whether anything changed.
pub fn migrate_recent(recent: &mut Vec<String>) -> bool {
    let mut next: Vec<String> = Vec::with_capacity(recent.len());
    for global in recent.iter() {
        let global = renamed(global);
        if !next.iter().any(|known| known == global) {
            next.push(global.to_owned());
        }
    }
    let changed = next != *recent;
    *recent = next;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(id, key)| ((*id).to_owned(), vec![(*key).to_owned()]))
            .collect()
    }

    #[test]
    fn the_table_is_final_after_one_pass() {
        for (old, new) in COMMAND_RENAMES {
            assert_ne!(old, new);
            assert_eq!(renamed(new), new, "{new} is renamed again");
            assert_eq!(
                COMMAND_RENAMES.iter().filter(|(o, _)| *o == old).count(),
                1,
                "{old} twice"
            );
        }
    }

    #[test]
    fn the_a_b_loop_moved_with_its_local_ids() {
        let moved: Vec<_> = COMMAND_RENAMES
            .iter()
            .filter(|(old, _)| old.starts_with("vestra.marks/"))
            .collect();
        assert_eq!(moved.len(), 18);
        for (old, new) in moved {
            let local = old.strip_prefix("vestra.marks/").unwrap();
            assert!(local.starts_with("ab."), "{old}");
            assert_eq!(*new, format!("vestra.playback/{local}"));
        }
    }

    #[test]
    fn overlay_keys_move_once_and_new_keys_win() {
        let mut keys = overlay(&[
            ("vestra.playback/toggle-pause", "P"),
            ("vestra.playback/mute", "N"),
            ("host/mute", "Ctrl+M"),
            ("vestra.playback/jump", "J"),
            ("vestra.marks/ab.set-a", "Q"),
            ("vestra.marks/ab.toggle", "W"),
            ("vestra.playback/ab.toggle", "E"),
            ("vestra.marks/chapter.next", "PgDn"),
            ("other.plugin/ab.set-a", "R"),
        ]);
        keys.insert("vestra.marks/ab.count.2".into(), Vec::new());
        assert!(migrate_overlay(&mut keys));
        assert_eq!(keys["host/play-pause"], ["P"]);
        assert_eq!(keys["host/mute"], ["Ctrl+M"]);
        assert!(!keys.contains_key("vestra.playback/mute"));
        assert_eq!(keys["vestra.playback/ab.set-a"], ["Q"]);
        assert_eq!(keys["vestra.playback/ab.toggle"], ["E"], "not clobbered");
        // An empty override (keys removed, or Ignore Default Shortcuts) moves too.
        assert!(keys["vestra.playback/ab.count.2"].is_empty());
        assert!(!keys.keys().any(|id| id.starts_with("vestra.marks/ab.")));
        // Unknown ids and commands that stayed keep theirs.
        assert_eq!(keys["vestra.playback/jump"], ["J"]);
        assert_eq!(keys["vestra.marks/chapter.next"], ["PgDn"]);
        assert_eq!(keys["other.plugin/ab.set-a"], ["R"]);
        let once = keys.clone();
        assert!(!migrate_overlay(&mut keys));
        assert_eq!(keys, once);
    }

    #[test]
    fn recent_commands_are_renamed_in_place_once() {
        let mut recent: Vec<String> = [
            "vestra.marks/ab.set-b",
            "host/mute",
            "vestra.playback/ab.toggle",
            "vestra.marks/ab.toggle",
            "vestra.playback/toggle-pause",
            "vestra.marks/chapter.next",
            "other.plugin/ab.set-b",
        ]
        .map(String::from)
        .to_vec();
        assert!(migrate_recent(&mut recent));
        assert_eq!(
            recent,
            [
                "vestra.playback/ab.set-b",
                "host/mute",
                "vestra.playback/ab.toggle",
                "host/play-pause",
                "vestra.marks/chapter.next",
                "other.plugin/ab.set-b",
            ]
        );
        let once = recent.clone();
        assert!(!migrate_recent(&mut recent));
        assert_eq!(recent, once);
        let mut untouched = vec!["host/mute".to_owned(), "vestra.video/video.reset".into()];
        assert!(!migrate_recent(&mut untouched));
        assert_eq!(untouched, ["host/mute", "vestra.video/video.reset"]);
    }
}
