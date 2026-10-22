//! Standard window shortcuts (user ruling 2026-10-22, item 2): the keys each
//! system uses to close a secondary window. Tool windows, floating panels and
//! the options window close on them; the main window does not (it is the
//! player: closing it quits, which has its own keys, ⌘Q and Alt+F4).
//!
//! The table is a pure function of the system, so every system's entries are
//! tested on every machine; `imp::SYSTEM` picks the running one. These keys
//! never reach the binding table: the window takes the press before Slint
//! sees it (`ui::WindowShortcuts`), even while a text field has focus.
//!
//! A new system adds a `System` variant, its entries below and `SYSTEM` in
//! its own module. Android and iOS have no keyboard shortcuts (an empty
//! table); their back gesture is mapped to `WindowAction::Close` by their own
//! platform module, not through this table.

use super::System;

/// What a standard window shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// The same as the title bar's close button.
    Close,
}

/// A key of a standard shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutKey {
    /// A letter, as the keyboard layout types it (⌘W is the key that types
    /// `w` on AZERTY too); on a layout without Latin letters, the key in the
    /// letter's US position.
    Letter(char),
    /// A key with no text, by its physical code (`"F4"`).
    Named(&'static str),
}

/// Modifiers held, as the system reports them (`cmd`: ⌘ on macOS, the
/// Windows key on Windows, Super on Linux). They must match exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShortcutMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowShortcut {
    pub key: ShortcutKey,
    pub mods: ShortcutMods,
    pub action: WindowAction,
}

/// A key press: the physical key (`"KeyW"`, `"F4"`, winit's code names),
/// the character it types without modifiers, and the modifiers held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub code: String,
    pub base: Option<char>,
    pub mods: ShortcutMods,
}

const CTRL: ShortcutMods = ShortcutMods {
    ctrl: true,
    alt: false,
    shift: false,
    cmd: false,
};
const CMD: ShortcutMods = ShortcutMods {
    ctrl: false,
    alt: false,
    shift: false,
    cmd: true,
};
const fn close(key: ShortcutKey, mods: ShortcutMods) -> WindowShortcut {
    WindowShortcut {
        key,
        mods,
        action: WindowAction::Close,
    }
}

const MACOS: &[WindowShortcut] = &[close(ShortcutKey::Letter('w'), CMD)];
const WINDOWS: &[WindowShortcut] = &[
    close(ShortcutKey::Letter('w'), CTRL),
    close(ShortcutKey::Named("F4"), CTRL),
];
const LINUX: &[WindowShortcut] = &[close(ShortcutKey::Letter('w'), CTRL)];

/// Each system's standard window shortcuts.
pub fn window_shortcuts(system: System) -> &'static [WindowShortcut] {
    match system {
        System::MacOs => MACOS,
        System::Windows => WINDOWS,
        System::Linux => LINUX,
    }
}

/// The action `press` stands for on `system`, if any.
pub fn window_shortcut(system: System, press: &KeyPress) -> Option<WindowAction> {
    window_shortcuts(system)
        .iter()
        .find(|shortcut| shortcut.mods == press.mods && key_matches(shortcut.key, press))
        .map(|shortcut| shortcut.action)
}

fn key_matches(key: ShortcutKey, press: &KeyPress) -> bool {
    match key {
        ShortcutKey::Letter(letter) => match press.base {
            Some(base) if base.is_ascii_alphabetic() => base.eq_ignore_ascii_case(&letter),
            // No Latin letter on this key (Cyrillic, Greek…): its US position.
            _ => press.code == format!("Key{}", letter.to_ascii_uppercase()),
        },
        ShortcutKey::Named(code) => press.code == code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: &str, base: Option<char>, mods: ShortcutMods) -> KeyPress {
        KeyPress {
            code: code.into(),
            base,
            mods,
        }
    }
    const NONE: ShortcutMods = ShortcutMods {
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
    };
    const CTRL_SHIFT: ShortcutMods = ShortcutMods {
        ctrl: true,
        alt: false,
        shift: true,
        cmd: false,
    };
    const CLOSE: Option<WindowAction> = Some(WindowAction::Close);

    #[test]
    fn macos_closes_on_command_w_only() {
        let at = |p| window_shortcut(System::MacOs, &p);
        assert_eq!(at(press("KeyW", Some('w'), CMD)), CLOSE);
        assert_eq!(at(press("KeyW", Some('w'), CTRL)), None);
        assert_eq!(at(press("F4", None, CTRL)), None);
        assert_eq!(at(press("KeyW", Some('w'), NONE)), None);
        // ⌘⇧W, ⌘⌥W are other commands on macOS.
        let mut mods = CMD;
        mods.shift = true;
        assert_eq!(at(press("KeyW", Some('w'), mods)), None);
        // AZERTY: the key that types `w` is in the US `Z` position.
        assert_eq!(at(press("KeyZ", Some('w'), CMD)), CLOSE);
        assert_eq!(at(press("KeyW", Some('z'), CMD)), None);
        // Russian: no Latin letter, the US position counts.
        assert_eq!(at(press("KeyW", Some('ц'), CMD)), CLOSE);
    }

    #[test]
    fn windows_closes_on_ctrl_w_and_ctrl_f4() {
        let at = |p| window_shortcut(System::Windows, &p);
        assert_eq!(at(press("KeyW", Some('w'), CTRL)), CLOSE);
        assert_eq!(at(press("KeyW", Some('W'), CTRL)), CLOSE);
        assert_eq!(at(press("F4", None, CTRL)), CLOSE);
        assert_eq!(at(press("KeyW", Some('w'), CMD)), None);
        assert_eq!(at(press("KeyW", Some('w'), CTRL_SHIFT)), None);
        // Plain F4 is the player's Stop; Alt+F4 is the system's own close.
        assert_eq!(at(press("F4", None, NONE)), None);
        let mut alt = NONE;
        alt.alt = true;
        assert_eq!(at(press("F4", None, alt)), None);
    }

    #[test]
    fn linux_closes_on_ctrl_w() {
        let at = |p| window_shortcut(System::Linux, &p);
        assert_eq!(at(press("KeyW", Some('w'), CTRL)), CLOSE);
        assert_eq!(at(press("F4", None, CTRL)), None);
        assert_eq!(at(press("KeyW", Some('w'), CMD)), None);
    }
}
