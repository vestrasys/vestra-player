//! Input binding table (core-ui-design.md §3): command default bindings plus
//! the user's overlay, resolved by scope with deterministic conflict rules.
//! Pure data; the shell forwards Slint key and mouse events here.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

pub type Owner = u64;
/// Owner generation of the host's own commands; lower generations win conflicts.
pub const HOST: Owner = 0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// `name` is canonical: a key ("A", "Space", ">", "F11") or a mouse event
/// ("Mouse:WheelUp", "Mouse:LeftDoubleClick:LeftHalf").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Input {
    pub mods: Mods,
    pub name: String,
}

const NAMED: &[&str] = &[
    "Space",
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Plus",
];
const MOUSE: &[&str] = &[
    "LeftDoubleClick",
    "LeftDoubleClick:LeftHalf",
    "LeftDoubleClick:RightHalf",
    "MiddleClick",
    "X1",
    "X2",
    "WheelUp",
    "WheelDown",
    "WheelLeft",
    "WheelRight",
];

#[derive(PartialEq)]
enum Kind {
    Letter,
    Digit,
    Named,
    Symbol,
    Mouse,
}

fn kind(name: &str) -> Option<Kind> {
    if let Some(mouse) = name.strip_prefix("Mouse:") {
        return MOUSE.contains(&mouse).then_some(Kind::Mouse);
    }
    let mut chars = name.chars();
    let (first, rest) = (chars.next()?, chars.as_str());
    if rest.is_empty() {
        return Some(match first {
            'A'..='Z' => Kind::Letter,
            '0'..='9' => Kind::Digit,
            c if c.is_alphanumeric() || c.is_whitespace() || c.is_control() || c == '+' => {
                return None;
            }
            _ => Kind::Symbol,
        });
    }
    if NAMED.contains(&name) {
        return Some(Kind::Named);
    }
    let function = name
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=24).contains(n) && !name.starts_with("F0"));
    function.map(|_| Kind::Named)
}

/// Parse one canonical binding string. Errors describe the rule broken.
pub fn parse(text: &str) -> Result<Input, String> {
    let mut parts: Vec<&str> = text.split('+').collect();
    let name = parts.pop().unwrap_or_default();
    let mut mods = Mods::default();
    let mut last = 0;
    for part in parts {
        let index = match part {
            "Ctrl" => 1,
            "Alt" => 2,
            "Shift" => 3,
            "Cmd" => 4,
            _ => return Err(format!("binding {text}: unknown modifier {part}")),
        };
        if index <= last {
            return Err(format!(
                "binding {text}: modifiers go in the order Ctrl, Alt, Shift, Cmd, once each"
            ));
        }
        last = index;
        match index {
            1 => mods.ctrl = true,
            2 => mods.alt = true,
            3 => mods.shift = true,
            _ => mods.cmd = true,
        }
    }
    let kind = kind(name).ok_or_else(|| format!("binding {text}: unknown input {name}"))?;
    if mods.shift && matches!(kind, Kind::Symbol | Kind::Digit) {
        return Err(format!(
            "binding {text}: write the character Shift produces instead of Shift+{name}"
        ));
    }
    Ok(Input {
        mods,
        name: name.into(),
    })
}

/// Map a Slint key event to an input. On Apple platforms Slint reports ⌘ as
/// `control` and ⌃ as `meta`; this swaps them back.
pub fn from_slint_key(
    text: &str,
    control: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Option<Input> {
    let apple = cfg!(target_os = "macos");
    let (ctrl, cmd) = if apple {
        (meta, control)
    } else {
        (control, meta)
    };
    let mut mods = Mods {
        ctrl,
        alt,
        shift,
        cmd,
    };
    let mut chars = text.chars();
    let first = chars.next()?;
    if !chars.as_str().is_empty() {
        return None;
    }
    let named = match first {
        ' ' => "Space",
        '\n' | '\r' => "Enter",
        '\t' => "Tab",
        '\u{19}' => {
            mods.shift = true;
            "Tab"
        }
        '\u{1b}' => "Escape",
        '\u{8}' => "Backspace",
        '\u{7f}' => "Delete",
        '\u{F727}' => "Insert",
        '\u{F729}' => "Home",
        '\u{F72B}' => "End",
        '\u{F72C}' => "PageUp",
        '\u{F72D}' => "PageDown",
        '\u{F700}' => "ArrowUp",
        '\u{F701}' => "ArrowDown",
        '\u{F702}' => "ArrowLeft",
        '\u{F703}' => "ArrowRight",
        '+' => {
            mods.shift = false;
            "Plus"
        }
        c @ '\u{F704}'..='\u{F71B}' => {
            return Some(Input {
                mods,
                name: format!("F{}", c as u32 - 0xF704 + 1),
            });
        }
        // Ctrl+letter may arrive as the ASCII control code.
        c @ '\u{1}'..='\u{1a}' if mods.ctrl || mods.cmd => {
            return Some(Input {
                mods,
                name: char::from(b'A' + (c as u8 - 1)).to_string(),
            });
        }
        c if c.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&c) => return None,
        c if c.is_ascii_alphabetic() => {
            return Some(Input {
                mods,
                name: c.to_ascii_uppercase().to_string(),
            });
        }
        c => {
            // Digits and symbols carry no Shift: the character already says it.
            mods.shift = false;
            return Some(Input {
                mods,
                name: c.to_string(),
            });
        }
    };
    Some(Input {
        mods,
        name: named.into(),
    })
}

/// Menu text: `Ctrl+O`, arrows as ← → ↑ ↓, Esc, PgUp/PgDn, Del.
pub fn label(input: &Input) -> String {
    let mut text = String::new();
    for (on, name) in [
        (input.mods.ctrl, "Ctrl+"),
        (input.mods.alt, "Alt+"),
        (input.mods.shift, "Shift+"),
        (input.mods.cmd, "Cmd+"),
    ] {
        if on {
            text.push_str(name);
        }
    }
    text.push_str(match input.name.as_str() {
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "Escape" => "Esc",
        "PageUp" => "PgUp",
        "PageDown" => "PgDn",
        "Delete" => "Del",
        "Plus" => "+",
        other => other,
    });
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Global,
    Panel(Owner),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub owner: Owner,
    pub id: String,
    pub repeat: bool,
}

#[derive(Clone, Debug)]
struct Declared {
    manifest: String,
    defaults: Vec<Input>,
    panel: bool,
    repeat: bool,
    sequence: u64,
}

#[derive(Default)]
pub struct Table {
    declared: BTreeMap<(Owner, String), Declared>,
    /// "<manifest id>/<command id>" → bindings; `[]` unbinds.
    overlay: BTreeMap<String, Vec<String>>,
    active: BTreeMap<(Scope, Input), Target>,
    conflicts: Vec<String>,
    next: u64,
}

impl Table {
    /// Bindings are validated before anything changes.
    pub fn declare(
        &mut self,
        owner: Owner,
        id: &str,
        manifest: &str,
        bindings: &[String],
        panel: bool,
        repeat: bool,
    ) -> Result<(), String> {
        let mut defaults = Vec::new();
        for text in bindings {
            let input = parse(text)?;
            if defaults.contains(&input) {
                return Err(format!("binding {text} is listed twice"));
            }
            defaults.push(input);
        }
        self.next += 1;
        self.declared.insert(
            (owner, id.into()),
            Declared {
                manifest: manifest.into(),
                defaults,
                panel,
                repeat,
                sequence: self.next,
            },
        );
        self.rebuild();
        Ok(())
    }

    pub fn remove_owner(&mut self, owner: Owner) {
        let before = self.declared.len();
        self.declared.retain(|(each, _), _| *each != owner);
        if before != self.declared.len() {
            self.rebuild();
        }
    }

    pub fn set_overlay(&mut self, overlay: BTreeMap<String, Vec<String>>) {
        self.overlay = overlay;
        self.rebuild();
    }

    /// Recompute from declarations: user overlay first, then defaults by owner
    /// generation (host 0 first, earlier-loaded plugins next), then declaration.
    fn rebuild(&mut self) {
        let mut claims = Vec::new();
        for ((owner, id), declared) in &self.declared {
            let scope = if declared.panel {
                Scope::Panel(*owner)
            } else {
                Scope::Global
            };
            let key = format!("{}/{id}", declared.manifest);
            let (user, inputs) = match self.overlay.get(&key) {
                Some(texts) => (
                    true,
                    texts
                        .iter()
                        .filter_map(|text| match parse(text) {
                            Ok(input) => Some(input),
                            Err(error) => {
                                eprintln!("binding overlay {key}: {error}");
                                None
                            }
                        })
                        .collect(),
                ),
                None => (false, declared.defaults.clone()),
            };
            for input in inputs {
                claims.push((!user, *owner, declared.sequence, scope, input, id.clone()));
            }
        }
        claims.sort();
        let mut active: BTreeMap<(Scope, Input), (bool, Target)> = BTreeMap::new();
        let mut conflicts = Vec::new();
        for (default, owner, _, scope, input, id) in claims {
            let target = Target {
                owner,
                id: id.clone(),
                repeat: self.declared[&(owner, id.clone())].repeat,
            };
            match active.get(&(scope, input.clone())) {
                None => {
                    active.insert((scope, input), (default, target));
                }
                // A user binding replaces a default one; that is not a conflict.
                Some((winner_default, winner)) if !*winner_default || default => {
                    if *winner_default == default {
                        let name = |owner: Owner, id: &str| {
                            let manifest = &self.declared[&(owner, id.to_owned())].manifest;
                            format!("{manifest}/{id}")
                        };
                        conflicts.push(format!(
                            "binding conflict: {} → {}; ignored: {}",
                            label(&input),
                            name(winner.owner, &winner.id),
                            name(owner, &target.id)
                        ));
                    }
                }
                Some(_) => unreachable!("claims are sorted user-first"),
            }
        }
        for conflict in &conflicts {
            if !self.conflicts.contains(conflict) {
                eprintln!("{conflict}");
            }
        }
        self.conflicts = conflicts;
        self.active = active
            .into_iter()
            .map(|(key, (_, target))| (key, target))
            .collect();
    }

    #[cfg(test)]
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// The focused panel's bindings first, then global ones. On macOS an event
    /// with ⌘ falls back to the `Ctrl` form ("Ctrl" accepts Cmd there).
    pub fn resolve(&self, input: &Input, panel: Option<Owner>) -> Option<&Target> {
        let mut candidates = vec![input.clone()];
        if cfg!(target_os = "macos") && input.mods.cmd && !input.mods.ctrl {
            let mut alias = input.clone();
            alias.mods.cmd = false;
            alias.mods.ctrl = true;
            candidates.push(alias);
        }
        if let Some(zoneless) = input
            .name
            .strip_suffix(":LeftHalf")
            .or(input.name.strip_suffix(":RightHalf"))
        {
            let mut wider = input.clone();
            wider.name = zoneless.into();
            candidates.push(wider);
        }
        let scopes = panel.map(Scope::Panel).into_iter().chain([Scope::Global]);
        for scope in scopes {
            for candidate in &candidates {
                if let Some(target) = self.active.get(&(scope, candidate.clone())) {
                    return Some(target);
                }
            }
        }
        None
    }

    /// Whether any active binding uses this mouse wheel axis (`Mouse:WheelUp`…).
    pub fn has_wheel(&self, vertical: bool) -> bool {
        let names: &[&str] = if vertical {
            &["Mouse:WheelUp", "Mouse:WheelDown"]
        } else {
            &["Mouse:WheelLeft", "Mouse:WheelRight"]
        };
        self.active
            .keys()
            .any(|(_, input)| names.contains(&input.name.as_str()))
    }

    /// First active keyboard binding of a command, as menu text.
    pub fn label(&self, owner: Owner, id: &str) -> Option<String> {
        let declared = self.declared.get(&(owner, id.to_owned()))?;
        let mut keys: Vec<_> = self
            .active
            .iter()
            .filter(|(_, target)| target.owner == owner && target.id == id)
            .map(|((_, input), _)| input)
            .filter(|input| !input.name.starts_with("Mouse:"))
            .collect();
        // Keep the declaration order of the command's own list.
        let order = |input: &Input| {
            declared
                .defaults
                .iter()
                .position(|each| each == input)
                .unwrap_or(usize::MAX)
        };
        keys.sort_by_key(|input| order(input));
        keys.first().map(|input| label(input))
    }

    /// Keys bound to the host's own commands (the producer capture lets them through).
    #[cfg(feature = "producer-audio")]
    pub fn host_key(&self, input: &Input) -> bool {
        self.resolve(input, None)
            .is_some_and(|target| target.owner == HOST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Input {
        parse(text).unwrap()
    }

    #[test]
    fn syntax_accepts_canonical_forms_and_rejects_the_rest() {
        for ok in [
            "Space",
            "Ctrl+O",
            "Ctrl+Alt+Shift+Cmd+F24",
            "Shift+E",
            ">",
            "/",
            "Plus",
            "1",
            "Mouse:WheelUp",
            "Ctrl+Mouse:LeftDoubleClick:RightHalf",
            "PageDown",
        ] {
            assert!(parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "space",
            "Shift+Ctrl+O",
            "Ctrl+Ctrl+O",
            "Shift+>",
            "Shift+1",
            "Ctrl++",
            "F0",
            "F25",
            "Mouse:RightClick",
            "Meta+O",
            "ab",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn slint_events_map_to_canonical_inputs() {
        assert_eq!(
            from_slint_key(" ", false, false, false, false),
            Some(key("Space"))
        );
        assert_eq!(
            from_slint_key("\u{F702}", false, false, false, false),
            Some(key("ArrowLeft"))
        );
        assert_eq!(
            from_slint_key("\u{F70E}", false, false, false, false),
            Some(key("F11"))
        );
        assert_eq!(
            from_slint_key("E", false, false, true, false),
            Some(key("Shift+E"))
        );
        assert_eq!(
            from_slint_key("e", false, false, false, false),
            Some(key("E"))
        );
        assert_eq!(
            from_slint_key(">", false, false, true, false),
            Some(key(">"))
        );
        assert_eq!(
            from_slint_key("\n", false, false, false, false),
            Some(key("Enter"))
        );
        assert_eq!(from_slint_key("\u{10}", false, false, true, false), None);
        let command = from_slint_key("o", true, false, false, false).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(command, key("Cmd+O"));
        } else {
            assert_eq!(command, key("Ctrl+O"));
        }
    }

    #[test]
    fn panel_scope_wins_while_focused_and_ctrl_accepts_cmd_on_macos() {
        let mut table = Table::default();
        table
            .declare(HOST, "fullscreen", "host", &["Enter".into()], false, false)
            .unwrap();
        table
            .declare(5, "play", "x.list", &["Enter".into()], true, false)
            .unwrap();
        table
            .declare(5, "open", "x.list", &["Ctrl+O".into()], false, false)
            .unwrap();
        assert!(table.conflicts().is_empty());
        assert_eq!(table.resolve(&key("Enter"), None).unwrap().id, "fullscreen");
        assert_eq!(table.resolve(&key("Enter"), Some(5)).unwrap().id, "play");
        assert_eq!(
            table.resolve(&key("Enter"), Some(6)).unwrap().id,
            "fullscreen"
        );
        assert_eq!(
            table.resolve(&key("Cmd+O"), None).map(|t| t.id.as_str()),
            cfg!(target_os = "macos").then_some("open")
        );
        assert_eq!(table.label(5, "open").as_deref(), Some("Ctrl+O"));
    }

    #[test]
    fn conflicts_keep_the_earlier_owner_and_release_with_it() {
        let mut table = Table::default();
        table
            .declare(7, "late", "b.late", &["Space".into()], false, false)
            .unwrap();
        table
            .declare(
                3,
                "early",
                "a.early",
                &["Space".into(), "ArrowLeft".into()],
                false,
                false,
            )
            .unwrap();
        assert_eq!(table.resolve(&key("Space"), None).unwrap().owner, 3);
        assert_eq!(
            table.conflicts(),
            ["binding conflict: Space → a.early/early; ignored: b.late/late"]
        );
        assert_eq!(table.label(7, "late"), None);
        assert_eq!(table.label(3, "early").as_deref(), Some("Space"));
        table.remove_owner(3);
        assert_eq!(table.resolve(&key("Space"), None).unwrap().owner, 7);
        assert!(table.conflicts().is_empty());
        assert!(
            table
                .declare(8, "bad", "c", &["Shift+>".into()], false, false)
                .is_err()
        );
    }

    #[test]
    fn user_overlay_replaces_defaults_without_conflict_and_can_unbind() {
        let mut table = Table::default();
        table
            .declare(1, "pause", "p.play", &["Space".into()], false, false)
            .unwrap();
        table
            .declare(2, "info", "p.info", &["Tab".into()], false, true)
            .unwrap();
        table.set_overlay(
            [
                ("p.info/info".to_owned(), vec!["Space".to_owned()]),
                ("p.play/pause".to_owned(), vec![]),
            ]
            .into_iter()
            .collect(),
        );
        let target = table.resolve(&key("Space"), None).unwrap();
        assert_eq!((target.id.as_str(), target.repeat), ("info", true));
        assert!(table.resolve(&key("Tab"), None).is_none());
        assert!(table.conflicts().is_empty());
        assert_eq!(table.label(1, "pause"), None);
    }

    #[test]
    fn zoned_mouse_bindings_win_over_zoneless_ones() {
        let mut table = Table::default();
        table
            .declare(
                HOST,
                "fullscreen",
                "host",
                &["Mouse:LeftDoubleClick".into()],
                false,
                false,
            )
            .unwrap();
        table
            .declare(
                4,
                "back",
                "x.y",
                &["Mouse:LeftDoubleClick:LeftHalf".into()],
                false,
                false,
            )
            .unwrap();
        let left = key("Mouse:LeftDoubleClick:LeftHalf");
        let right = key("Mouse:LeftDoubleClick:RightHalf");
        assert_eq!(table.resolve(&left, None).unwrap().id, "back");
        assert_eq!(table.resolve(&right, None).unwrap().id, "fullscreen");
        assert!(table.conflicts().is_empty());
        assert!(!table.has_wheel(true));
        assert_eq!(label(&key("Ctrl+ArrowLeft")), "Ctrl+←");
        assert_eq!(label(&key("PageDown")), "PgDn");
    }
}
