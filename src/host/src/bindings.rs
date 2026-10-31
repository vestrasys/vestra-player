//! Input binding table (core-ui-design.md §3): command default bindings plus
//! the user's overlay, resolved by scope with deterministic conflict rules.
//! Pure data; the shell forwards Slint key and mouse events here.
#![forbid(unsafe_code)]

use crate::platform::KeyLabels;
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
    "Numpad0",
    "Numpad1",
    "Numpad2",
    "Numpad3",
    "Numpad4",
    "Numpad5",
    "Numpad6",
    "Numpad7",
    "Numpad8",
    "Numpad9",
    "NumpadPlus",
    "NumpadMinus",
    "NumpadMultiply",
    "NumpadDivide",
    "NumpadEnter",
    "NumpadDecimal",
];
const MOUSE: &[&str] = &[
    "LeftClick",
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

/// The physical key of the key press being handled (winit `KeyCode` name, e.g.
/// `KeyO`, `Numpad5`, `BracketLeft`) and its layout character without any
/// modifier, when the platform reports one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Physical {
    pub code: String,
    pub base: Option<char>,
    /// Modifiers the window system reported with this press. Preferred over
    /// Slint's flags, which only follow modifier key events and miss
    /// modifiers carried by the key event itself (e.g. synthesized input).
    pub mods: Option<Mods>,
}

/// Input name of a numeric-keypad key; keypad keys never match main-keyboard
/// names and vice versa.
fn numpad(code: &str) -> Option<String> {
    let rest = code.strip_prefix("Numpad")?;
    Some(match rest {
        "Add" => "NumpadPlus".into(),
        "Subtract" => "NumpadMinus".into(),
        "Multiply" | "Divide" | "Enter" | "Decimal" => code.into(),
        digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => code.into(),
        _ => return None,
    })
}

/// Slint's key code for a named key (winit `KeyCode` name).
pub fn slint_key(code: &str) -> Option<char> {
    let function = code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n));
    if let Some(n) = function {
        return char::from_u32(0xF704 + n - 1);
    }
    Some(match code {
        "Space" => ' ',
        "Enter" => '\n',
        "Tab" => '\t',
        "Escape" => '\u{1b}',
        "Backspace" => '\u{8}',
        "Delete" => '\u{7f}',
        "Insert" => '\u{F727}',
        "Home" => '\u{F729}',
        "End" => '\u{F72B}',
        "PageUp" => '\u{F72C}',
        "PageDown" => '\u{F72D}',
        "ArrowUp" => '\u{F700}',
        "ArrowDown" => '\u{F701}',
        "ArrowLeft" => '\u{F702}',
        "ArrowRight" => '\u{F703}',
        _ => return None,
    })
}

/// Input name of a named key by its winit `KeyCode` (`Delete` is ⌦, on
/// laptops fn+⌫; `Backspace` is ⌫).
fn named_key(code: &str) -> Option<String> {
    let named = matches!(
        code,
        "Space"
            | "Enter"
            | "Tab"
            | "Escape"
            | "Backspace"
            | "Delete"
            | "Insert"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "ArrowDown"
    );
    let function = code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    (named || function).then(|| code.to_owned())
}

/// The unshifted US-layout character of a character key.
fn us_character(code: &str) -> Option<char> {
    if let Some(letter) = code.strip_prefix("Key")
        && letter.len() == 1
    {
        return letter.chars().next().filter(char::is_ascii_uppercase);
    }
    if let Some(digit) = code.strip_prefix("Digit")
        && digit.len() == 1
    {
        return digit.chars().next().filter(char::is_ascii_digit);
    }
    Some(match code {
        "Minus" => '-',
        "Equal" => '=',
        "BracketLeft" => '[',
        "BracketRight" => ']',
        "Backslash" => '\\',
        "Semicolon" => ';',
        "Quote" => '\'',
        "Comma" => ',',
        "Period" => '.',
        "Slash" => '/',
        "Backquote" => '`',
        _ => return None,
    })
}

/// What Shift makes of a digit or symbol on a US layout.
fn us_shifted(c: char) -> Option<char> {
    let (plain, shifted) = ("1234567890-=[]\\;',./`", "!@#$%^&*()_+{}|:\"<>?~");
    plain
        .chars()
        .position(|p| p == c)
        .and_then(|i| shifted.chars().nth(i))
}

/// The input a character key names when its typed text cannot be used: with
/// Option (macOS composes another character, or a dead key), or with Ctrl
/// (an ASCII control code). Letters come from the layout without modifiers.
fn from_physical(physical: &Physical, mut mods: Mods) -> Option<Input> {
    let base = physical
        .base
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .or_else(|| us_character(&physical.code))?;
    let name = if base.is_ascii_alphabetic() {
        base.to_string()
    } else if mods.shift {
        mods.shift = false;
        symbol_name(us_shifted(base)?)
    } else {
        // Layouts where `+` has its own unshifted key (German ⌥+).
        symbol_name(base)
    };
    Some(Input { mods, name })
}

/// A digit or symbol as a key name: `+` is spelled `Plus` everywhere.
fn symbol_name(c: char) -> String {
    if c == '+' {
        "Plus".into()
    } else {
        c.to_string()
    }
}

fn modifier_key(code: &str) -> bool {
    matches!(
        code,
        "ShiftLeft"
            | "ShiftRight"
            | "ControlLeft"
            | "ControlRight"
            | "AltLeft"
            | "AltRight"
            | "SuperLeft"
            | "SuperRight"
            | "Meta"
            | "CapsLock"
            | "Fn"
    )
}

/// Map a Slint key event to an input. On Apple platforms Slint reports ⌘ as
/// `control` and ⌃ as `meta`; this swaps them back.
#[cfg(test)]
pub fn from_slint_key(
    text: &str,
    control: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Option<Input> {
    from_key(text, control, alt, shift, meta, None)
}

/// The character an Option combination typed, as an input of its own: on
/// layouts where Option is how a symbol is typed (German ⌥5 = `[`, ⌥7 = `|`,
/// ⌥⇧7 = `\`), a binding for the symbol is meant. Tried before the physical
/// `Alt+…` form; letters (⌥O, ⌥H) only resolve physically. Ctrl and ⌘ stay.
pub fn typed_with_option(
    text: &str,
    control: bool,
    meta: bool,
    physical: Option<&Physical>,
) -> Option<Input> {
    let physical = physical?;
    let (ctrl, cmd) = if crate::platform::SLINT_CONTROL_IS_COMMAND {
        (meta, control)
    } else {
        (control, meta)
    };
    let mods = physical.mods.unwrap_or(Mods {
        ctrl,
        alt: true,
        shift: false,
        cmd,
    });
    let mut chars = text.chars();
    let typed = chars.next()?;
    if !mods.alt
        || !chars.as_str().is_empty()
        || us_character(&physical.code).is_none()
        || typed.is_alphanumeric()
        || typed.is_whitespace()
        || typed.is_control()
        || ('\u{F700}'..='\u{F8FF}').contains(&typed)
    {
        return None;
    }
    Some(Input {
        mods: Mods {
            alt: false,
            shift: false,
            ..mods
        },
        name: symbol_name(typed),
    })
}

/// Like `from_slint_key`, with the physical key when the window saw it:
/// keypad keys, Option combinations and Ctrl control codes resolve by key.
pub fn from_key(
    text: &str,
    control: bool,
    alt: bool,
    shift: bool,
    meta: bool,
    physical: Option<&Physical>,
) -> Option<Input> {
    let (ctrl, cmd) = if crate::platform::SLINT_CONTROL_IS_COMMAND {
        (meta, control)
    } else {
        (control, meta)
    };
    let mut mods = physical.and_then(|p| p.mods).unwrap_or(Mods {
        ctrl,
        alt,
        shift,
        cmd,
    });
    let (ctrl, alt, cmd) = (mods.ctrl, mods.alt, mods.cmd);
    if let Some(physical) = physical {
        if modifier_key(&physical.code) {
            return None;
        }
        if let Some(name) = numpad(&physical.code) {
            return Some(Input { mods, name });
        }
        // Named keys go by key: with Option, macOS may give them no text
        // (⌥⌦ would otherwise be lost).
        if let Some(name) = named_key(&physical.code) {
            return Some(Input { mods, name });
        }
        let control_code = text.chars().count() == 1
            && text
                .chars()
                .next()
                .is_some_and(|c| c < ' ' || c == '\u{7f}');
        let character_key = us_character(&physical.code).is_some();
        if character_key && (alt || ((ctrl || cmd) && (control_code || text.is_empty()))) {
            return from_physical(physical, mods);
        }
        if text.is_empty() {
            return None;
        }
    }
    let mut chars = text.chars();
    let first = chars.next()?;
    if !chars.as_str().is_empty() {
        return None;
    }
    // Without the physical key: with Ctrl or ⌘, an ASCII control code is
    // Ctrl+letter (0x1b is Ctrl+[), never the Tab/Backspace/Enter/Escape it
    // shares a code with. 0x10..=0x18 stay ambiguous with Slint's codes for
    // the modifier keys themselves, which the physical key resolves.
    if physical.is_none() && (mods.ctrl || mods.cmd) {
        let name = match first {
            c @ '\u{1}'..='\u{1a}' if !('\u{10}'..='\u{18}').contains(&c) => {
                char::from(b'A' + (c as u8 - 1)).to_string()
            }
            '\u{1b}' => "[".into(),
            '\u{1c}' => "\\".into(),
            '\u{1d}' => "]".into(),
            _ => String::new(),
        };
        if !name.is_empty() {
            return Some(Input { mods, name });
        }
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
        // Slint's own codes for the modifier keys themselves (Shift, Control,
        // Alt, AltGr, CapsLock, ShiftR, ControlR, Meta, MetaR). Pressing ⌘
        // alone arrives as '\u{11}' with cmd set and must not read as Ctrl+Q.
        '\u{10}'..='\u{18}' => return None,
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

/// The canonical binding string (`parse` round-trips it): `Ctrl+Shift+A`.
pub fn text(input: &Input) -> String {
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
    text + &input.name
}

/// Whether a menu may show `input` as a command's shortcut: a key, not a
/// mouse binding, and no `Cmd` binding unless the system's menus show them
/// (`platform::MENU_SHOWS_CMD`: on Windows `Cmd` is the Windows key, whose
/// combinations the system takes, so `host/quit`'s `Cmd+Q` would name a
/// shortcut that does nothing). The binding itself stays.
pub fn label_shown(input: &Input, shows_cmd: bool) -> bool {
    !input.name.starts_with("Mouse:") && (shows_cmd || !input.mods.cmd)
}

/// Menu text in the running system's style (`platform::KEY_LABELS`):
/// `Ctrl+O` on Windows and Linux, `⌘O` on macOS.
pub fn label(input: &Input) -> String {
    label_in(input, crate::platform::KEY_LABELS)
}

/// Menu text in `style`. Text: `Ctrl+O`, arrows as ← → ↑ ↓, Esc,
/// PgUp/PgDn, Del. Symbols (macOS): `⌘O`, `⇧⌘S`, `⌥→`; a `Ctrl` binding
/// shows ⌘ (it answers to ⌘ there), and ⌃ only next to ⌘. Named keys stay
/// words in both styles: the UI font has no ⇥ ⌤ ⎋, and ↩ draws as an emoji.
pub fn label_in(input: &Input, style: KeyLabels) -> String {
    let mods = &input.mods;
    let mut text = String::new();
    let modifiers = match style {
        KeyLabels::Text => [
            (mods.ctrl, "Ctrl+"),
            (mods.alt, "Alt+"),
            (mods.shift, "Shift+"),
            (mods.cmd, "Cmd+"),
        ],
        // Apple's order ⌃⌥⇧⌘.
        KeyLabels::Symbols => [
            (mods.ctrl && mods.cmd, "⌃"),
            (mods.alt, "⌥"),
            (mods.shift, "⇧"),
            (mods.ctrl || mods.cmd, "⌘"),
        ],
    };
    for (on, name) in modifiers {
        if on {
            text.push_str(name);
        }
    }
    let named = match input.name.as_str() {
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "Escape" => "Esc",
        "PageUp" => "PgUp",
        "PageDown" => "PgDn",
        "Delete" => "Del",
        "Plus" => "+",
        "NumpadPlus" => "Num+",
        "NumpadMinus" => "Num-",
        "NumpadMultiply" => "Num*",
        "NumpadDivide" => "Num/",
        "NumpadEnter" => "NumEnter",
        "NumpadDecimal" => "Num.",
        other => match other.strip_prefix("Numpad") {
            Some(digit) if digit.len() == 1 => {
                text.push_str("Num");
                digit
            }
            _ => other,
        },
    };
    text + named
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
    /// Where its defaults rank against other owners' (lower first): the
    /// owner generation that first declared it, kept when a parked plugin
    /// wakes under a new generation (`rekey`), so that waking does not
    /// change who wins a key.
    rank: Owner,
}

#[derive(Default)]
pub struct Table {
    declared: BTreeMap<(Owner, String), Declared>,
    /// `"<manifest id>/<command id>"` → bindings; `[]` unbinds.
    overlay: BTreeMap<String, Vec<String>>,
    active: BTreeMap<(Scope, Input), Target>,
    /// Commands that lost an input to the active one, in claim order: the
    /// key goes to them while the winner is disabled.
    fallback: BTreeMap<(Scope, Input), Vec<Target>>,
    /// Disabled commands and their reasons, mirrored from the registry.
    disabled: BTreeMap<(Owner, String), Option<String>>,
    conflicts: Vec<String>,
    /// Inputs a command claims but another command holds: (input, winner).
    shadowed: Shadows,
    next: u64,
    /// Advances on every rebuild (declarations or overlay changed).
    revision: u64,
}

/// What an input resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// An enabled command takes it.
    Fire(&'a Target),
    /// Every command bound to it is disabled: the first one and its reason.
    Disabled {
        target: &'a Target,
        reason: Option<&'a str>,
    },
    Unbound,
}

/// Per command: inputs it claims that another command holds, and the winner.
type Shadows = BTreeMap<(Owner, String), Vec<(Input, Owner, String)>>;

/// One command's bindings for `plugin.bindings.list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub owner: Owner,
    pub id: String,
    pub manifest: String,
    pub panel: bool,
    pub defaults: Vec<Input>,
    /// The user overlay's inputs when overridden, else the defaults.
    pub current: Vec<Input>,
    pub overridden: bool,
    /// (input, winning manifest, winning command id).
    pub shadowed: Vec<(Input, String, String)>,
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
        // Declared again (a woken plugin's live registration): its rank and
        // place stay.
        let (rank, sequence) = match self.declared.get(&(owner, id.into())) {
            Some(existing) => (existing.rank, existing.sequence),
            None => {
                self.next += 1;
                (owner, self.next)
            }
        };
        self.declared.insert(
            (owner, id.into()),
            Declared {
                manifest: manifest.into(),
                defaults,
                panel,
                repeat,
                sequence,
                rank,
            },
        );
        self.rebuild();
        Ok(())
    }

    /// Wake: `from`'s declarations move to generation `to`, keeping their
    /// rank, so the same inputs go to the same commands.
    pub fn rekey(&mut self, from: Owner, to: Owner) {
        let keys = self
            .declared
            .keys()
            .filter(|(owner, _)| *owner == from)
            .cloned()
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return;
        }
        for key in keys {
            if let Some(declared) = self.declared.remove(&key) {
                self.declared.insert((to, key.1), declared);
            }
        }
        // The registry's disabled mirror follows (until its next copy).
        self.disabled = std::mem::take(&mut self.disabled)
            .into_iter()
            .map(|((owner, id), reason)| ((if owner == from { to } else { owner }, id), reason))
            .collect();
        self.rebuild();
    }

    /// One command's bindings go; claims it was shadowing become active.
    pub fn remove(&mut self, owner: Owner, id: &str) {
        if self.declared.remove(&(owner, id.to_owned())).is_some() {
            self.rebuild();
        }
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

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The overlay inputs a command (or an orphan key) would get: each one
    /// parsed and canonical, no repeats.
    pub fn canonical(inputs: &[String]) -> Result<Vec<String>, String> {
        let mut seen = Vec::new();
        for text in inputs {
            let input = parse(text)?;
            if seen.contains(&input) {
                return Err(format!("binding {text} is listed twice"));
            }
            seen.push(input);
        }
        Ok(seen.iter().map(self::text).collect())
    }

    /// Whether `<manifest>/<id>` is a declared command.
    pub fn declares(&self, manifest: &str, id: &str) -> bool {
        self.declared
            .iter()
            .any(|((_, each), declared)| each == id && declared.manifest == manifest)
    }

    /// Every declared command, in owner and id order.
    pub fn infos(&self) -> Vec<Info> {
        self.declared
            .iter()
            .map(|((owner, id), declared)| {
                let key = format!("{}/{id}", declared.manifest);
                let overridden = self.overlay.get(&key);
                Info {
                    owner: *owner,
                    id: id.clone(),
                    manifest: declared.manifest.clone(),
                    panel: declared.panel,
                    defaults: declared.defaults.clone(),
                    current: overridden.map_or_else(
                        || declared.defaults.clone(),
                        |texts| texts.iter().filter_map(|t| parse(t).ok()).collect(),
                    ),
                    overridden: overridden.is_some(),
                    shadowed: self
                        .shadowed
                        .get(&(*owner, id.clone()))
                        .into_iter()
                        .flatten()
                        .map(|(input, winner, winner_id)| {
                            let manifest = self.declared[&(*winner, winner_id.clone())]
                                .manifest
                                .clone();
                            (input.clone(), manifest, winner_id.clone())
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Overlay entries whose command is not declared (its plugin is stopped
    /// or uninstalled): kept, and applied again when it returns.
    pub fn orphans(&self) -> Vec<(String, Vec<String>)> {
        self.overlay
            .iter()
            .filter(|(key, _)| {
                key.split_once('/')
                    .is_none_or(|(manifest, id)| !self.declares(manifest, id))
            })
            .map(|(key, inputs)| (key.clone(), inputs.clone()))
            .collect()
    }

    /// `Mouse:LeftClick` is bound (the shell then waits out a double click).
    pub fn has_left_click(&self) -> bool {
        self.active
            .keys()
            .any(|(_, input)| input.name == "Mouse:LeftClick")
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
                                diag!("binding overlay {key}: {error}");
                                None
                            }
                        })
                        .collect(),
                ),
                None => (false, declared.defaults.clone()),
            };
            for input in inputs {
                claims.push((
                    !user,
                    declared.rank,
                    declared.sequence,
                    *owner,
                    scope,
                    input,
                    id.clone(),
                ));
            }
        }
        claims.sort();
        let mut active: BTreeMap<(Scope, Input), (bool, Target)> = BTreeMap::new();
        let mut conflicts = Vec::new();
        let mut shadowed: Shadows = BTreeMap::new();
        let mut fallback: BTreeMap<(Scope, Input), Vec<Target>> = BTreeMap::new();
        for (default, _, _, owner, scope, input, id) in claims {
            let target = Target {
                owner,
                id: id.clone(),
                repeat: self.declared[&(owner, id.clone())].repeat,
            };
            if let Some((_, winner)) = active.get(&(scope, input.clone())) {
                shadowed.entry((owner, id.clone())).or_default().push((
                    input.clone(),
                    winner.owner,
                    winner.id.clone(),
                ));
                fallback
                    .entry((scope, input.clone()))
                    .or_default()
                    .push(target.clone());
            }
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
                diag!("{conflict}");
            }
        }
        self.conflicts = conflicts;
        self.shadowed = shadowed;
        self.fallback = fallback;
        self.revision += 1;
        self.active = active
            .into_iter()
            .map(|(key, (_, target))| (key, target))
            .collect();
    }

    #[cfg(test)]
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// The registry's disabled commands (owner, id → reason). Keys bound to
    /// them pass to the next command bound to the same input.
    pub fn set_disabled(&mut self, disabled: BTreeMap<(Owner, String), Option<String>>) {
        self.disabled = disabled;
    }

    /// The enabled command an input fires, if any (see [`Table::lookup`]).
    pub fn resolve(&self, input: &Input, panel: Option<Owner>) -> Option<&Target> {
        match self.lookup(input, panel) {
            Lookup::Fire(target) => Some(target),
            _ => None,
        }
    }

    /// The focused panel's bindings first, then global ones. On macOS an event
    /// with ⌘ falls back to the `Ctrl` form ("Ctrl" accepts Cmd there). A
    /// disabled command is skipped: the input goes to the next command bound
    /// to it (one that lost the conflict, then the wider scope).
    pub fn lookup(&self, input: &Input, panel: Option<Owner>) -> Lookup<'_> {
        let mut candidates = vec![input.clone()];
        if crate::platform::CTRL_ACCEPTS_CMD && input.mods.cmd && !input.mods.ctrl {
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
        let mut disabled = None;
        let scopes = panel.map(Scope::Panel).into_iter().chain([Scope::Global]);
        for scope in scopes {
            for candidate in &candidates {
                let key = (scope, candidate.clone());
                let targets = self
                    .active
                    .get(&key)
                    .into_iter()
                    .chain(self.fallback.get(&key).into_iter().flatten());
                for target in targets {
                    match self.disabled.get(&(target.owner, target.id.clone())) {
                        None => return Lookup::Fire(target),
                        Some(reason) => {
                            disabled.get_or_insert(Lookup::Disabled {
                                target,
                                reason: reason.as_deref(),
                            });
                        }
                    }
                }
            }
        }
        disabled.unwrap_or(Lookup::Unbound)
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
            .filter(|input| label_shown(input, crate::platform::MENU_SHOWS_CMD))
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
    fn modifier_keys_alone_are_not_inputs() {
        // Pressing ⌘ alone on macOS: Slint's Control code with cmd held.
        for code in '\u{10}'..='\u{18}' {
            let text = code.to_string();
            assert_eq!(from_slint_key(&text, true, false, false, false), None);
            assert_eq!(from_slint_key(&text, false, false, false, true), None);
        }
        // A real ASCII control code for a letter still maps.
        let ctrl_e = from_slint_key("\u{5}", false, false, false, true).unwrap();
        assert_eq!(ctrl_e.name, "E");
    }

    /// Slint flags in its own order (`control` is ⌘ on macOS, `meta` is ⌃).
    fn ctrl() -> (bool, bool) {
        if crate::platform::SLINT_CONTROL_IS_COMMAND {
            (false, true)
        } else {
            (true, false)
        }
    }
    fn typed(
        text: &str,
        ctrl_held: bool,
        alt: bool,
        shift: bool,
        code: Option<(&str, Option<char>)>,
    ) -> Option<String> {
        let (control, meta) = if ctrl_held { ctrl() } else { (false, false) };
        let physical = code.map(|(code, base)| Physical {
            code: code.into(),
            base,
            mods: None,
        });
        from_key(text, control, alt, shift, meta, physical.as_ref())
            .map(|input| label_canonical(&input))
    }
    fn label_canonical(input: &Input) -> String {
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
        text + &input.name
    }

    #[test]
    fn control_codes_are_ctrl_letters_only_with_ctrl() {
        for (code, plain, with_ctrl) in [
            ("\u{19}", "Shift+Tab", "Ctrl+Y"),
            ("\t", "Tab", "Ctrl+I"),
            ("\u{8}", "Backspace", "Ctrl+H"),
            ("\r", "Enter", "Ctrl+M"),
            ("\u{1b}", "Escape", "Ctrl+["),
        ] {
            assert_eq!(
                typed(code, false, false, false, None).as_deref(),
                Some(plain),
                "{code:?}"
            );
            assert_eq!(
                typed(code, true, false, false, None).as_deref(),
                Some(with_ctrl),
                "{code:?}"
            );
        }
        // With the physical key, Ctrl+Tab stays Tab and Ctrl+[ is the bracket.
        assert_eq!(
            typed("\t", true, false, false, Some(("Tab", None))).as_deref(),
            Some("Ctrl+Tab")
        );
        assert_eq!(
            typed(
                "\u{1b}",
                true,
                false,
                false,
                Some(("BracketLeft", Some('[')))
            )
            .as_deref(),
            Some("Ctrl+[")
        );
        assert_eq!(
            typed("\u{1b}", true, false, false, Some(("Escape", None))).as_deref(),
            Some("Ctrl+Escape")
        );
        // Ctrl+P..X share codes with Slint's modifier keys: the physical key decides.
        assert_eq!(
            typed("\u{10}", true, false, false, Some(("KeyP", Some('p')))).as_deref(),
            Some("Ctrl+P")
        );
        assert_eq!(
            typed("\u{11}", true, false, false, Some(("SuperLeft", None))),
            None
        );
        assert_eq!(
            typed("\u{19}", true, false, false, Some(("KeyY", Some('y')))).as_deref(),
            Some("Ctrl+Y")
        );
    }

    #[test]
    fn option_combinations_resolve_by_physical_key_including_dead_keys() {
        // ⌥O types "ø", ⌥E is a dead key with no text.
        assert_eq!(
            typed("ø", false, true, false, Some(("KeyO", Some('o')))).as_deref(),
            Some("Alt+O")
        );
        assert_eq!(
            typed("", false, true, false, Some(("KeyE", Some('e')))).as_deref(),
            Some("Alt+E")
        );
        assert_eq!(
            typed("", false, true, true, Some(("KeyN", None))).as_deref(),
            Some("Alt+Shift+N")
        );
        assert_eq!(
            typed("¥", true, true, false, Some(("KeyY", Some('y')))).as_deref(),
            Some("Ctrl+Alt+Y")
        );
        // A layout letter wins over the US position (AZERTY: physical KeyQ is A).
        assert_eq!(
            typed("æ", false, true, false, Some(("KeyQ", Some('a')))).as_deref(),
            Some("Alt+A")
        );
        assert_eq!(
            typed("", false, true, false, Some(("Backquote", Some('`')))).as_deref(),
            Some("Alt+`")
        );
        assert_eq!(
            typed("≥", false, true, false, Some(("Period", Some('.')))).as_deref(),
            Some("Alt+.")
        );
        assert_eq!(
            typed("˘", false, true, true, Some(("Period", Some('.')))).as_deref(),
            Some("Alt+>")
        );
        assert_eq!(
            typed("±", false, true, true, Some(("Equal", Some('=')))).as_deref(),
            Some("Alt+Plus")
        );
        // Without Option the typed text is used as before.
        assert_eq!(
            typed("?", false, false, true, Some(("Slash", Some('/')))).as_deref(),
            Some("?")
        );
        assert_eq!(
            typed("", false, false, false, Some(("KeyE", Some('e')))),
            None
        );
        assert_eq!(typed("", false, true, false, Some(("AltLeft", None))), None);
    }

    /// Synthesized input (System Events) carries modifiers on the key event
    /// only; Slint's flags then stay false while winit reports them.
    #[test]
    fn window_system_modifiers_win_over_missing_slint_flags() {
        let press = |text: &str, code: &str, base: char, mods: Mods| {
            let physical = Physical {
                code: code.into(),
                base: Some(base),
                mods: Some(mods),
            };
            from_key(text, false, false, false, false, Some(&physical))
                .map(|input| label_canonical(&input))
        };
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(press("ø", "KeyO", 'o', alt).as_deref(), Some("Alt+O"));
        assert_eq!(press("˙", "KeyH", 'h', alt).as_deref(), Some("Alt+H"));
        assert_eq!(
            press("\u{19}", "KeyY", 'y', ctrl).as_deref(),
            Some("Ctrl+Y")
        );
        assert_eq!(press("\u{9}", "KeyI", 'i', ctrl).as_deref(), Some("Ctrl+I"));
        assert_eq!(
            press("o", "KeyO", 'o', Mods::default()).as_deref(),
            Some("O")
        );
        // German QWERTZ: the key in the US Z position types y. Letters follow
        // the layout, so ⌃Y and ⌥Z are what the keycaps say.
        assert_eq!(
            press("\u{19}", "KeyZ", 'y', ctrl).as_deref(),
            Some("Ctrl+Y")
        );
        assert_eq!(press("Ω", "KeyY", 'z', alt).as_deref(), Some("Alt+Z"));
        // Option+Shift+digit/symbol: the layout's shifted character is not
        // reported with Option held, so it is named by the US shift table
        // (on QWERTZ, ⌥⇧. is Alt+> although Shift+. types : there).
        let alt_shift = Mods { shift: true, ..alt };
        assert_eq!(
            press("÷", "Period", '.', alt_shift).as_deref(),
            Some("Alt+>")
        );
        // A layout symbol without a US position keeps its own character.
        assert_eq!(press("•", "Backquote", '^', alt).as_deref(), Some("Alt+^"));
        // `+` is always Plus. German has an unshifted `+` key (US `]`
        // position): ⌥+ types ± and resolves physically to Alt+Plus, as
        // the plain key gives Plus. US ⌥⇧= gets there via the shift table.
        assert_eq!(
            press("±", "BracketRight", '+', alt).as_deref(),
            Some("Alt+Plus")
        );
        assert_eq!(
            press("+", "BracketRight", '+', Mods::default()).as_deref(),
            Some("Plus")
        );
        assert_eq!(
            press("±", "Equal", '=', alt_shift).as_deref(),
            Some("Alt+Plus")
        );
        let ctrl_alt = Mods { ctrl: true, ..alt };
        assert_eq!(
            press("±", "BracketRight", '+', ctrl_alt).as_deref(),
            Some("Ctrl+Alt+Plus")
        );
        // Every result is a valid binding string.
        assert!(parse("Alt+Plus").is_ok());
    }

    #[test]
    fn keypad_keys_are_separate_from_the_main_keyboard() {
        assert_eq!(
            typed("5", false, false, false, Some(("Numpad5", Some('5')))).as_deref(),
            Some("Numpad5")
        );
        assert_eq!(
            typed("5", false, false, false, Some(("Digit5", Some('5')))).as_deref(),
            Some("5")
        );
        assert_eq!(
            typed("+", false, false, false, Some(("NumpadAdd", None))).as_deref(),
            Some("NumpadPlus")
        );
        assert_eq!(
            typed("-", false, false, false, Some(("NumpadSubtract", None))).as_deref(),
            Some("NumpadMinus")
        );
        assert_eq!(
            typed("\r", false, false, false, Some(("NumpadEnter", None))).as_deref(),
            Some("NumpadEnter")
        );
        assert_eq!(
            typed(".", true, false, false, Some(("NumpadDecimal", None))).as_deref(),
            Some("Ctrl+NumpadDecimal")
        );
        for name in [
            "Numpad0",
            "Numpad9",
            "NumpadPlus",
            "NumpadMinus",
            "NumpadMultiply",
            "NumpadDivide",
            "NumpadEnter",
            "NumpadDecimal",
            "Shift+Numpad5",
        ] {
            assert!(parse(name).is_ok(), "{name}");
        }
        assert!(parse("Numpad10").is_err());
        assert!(parse("NumpadAdd").is_err());
        let mut table = Table::default();
        table
            .declare(1, "five", "m", &["5".into()], false, false)
            .unwrap();
        table
            .declare(1, "pad", "m", &["Numpad5".into()], false, false)
            .unwrap();
        let pad = from_key(
            "5",
            false,
            false,
            false,
            false,
            Some(&Physical {
                code: "Numpad5".into(),
                base: None,
                mods: None,
            }),
        )
        .unwrap();
        let main = from_key(
            "5",
            false,
            false,
            false,
            false,
            Some(&Physical {
                code: "Digit5".into(),
                base: None,
                mods: None,
            }),
        )
        .unwrap();
        assert_eq!(table.resolve(&pad, None).unwrap().id, "pad");
        assert_eq!(table.resolve(&main, None).unwrap().id, "five");
        let text = |input: &str| label_in(&parse(input).unwrap(), KeyLabels::Text);
        assert_eq!(text("Numpad5"), "Num5");
        assert_eq!(text("Ctrl+NumpadPlus"), "Ctrl+Num+");
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
        if crate::platform::SLINT_CONTROL_IS_COMMAND {
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
            crate::platform::CTRL_ACCEPTS_CMD.then_some("open")
        );
        // The menu writes it as the system does (⌘O on macOS).
        let menu = if cfg!(target_os = "macos") {
            "⌘O"
        } else {
            "Ctrl+O"
        };
        assert_eq!(table.label(5, "open").as_deref(), Some(menu));
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
        table.remove(3, "early");
        assert_eq!(table.resolve(&key("Space"), None).unwrap().owner, 7);
        assert!(table.resolve(&key("ArrowLeft"), None).is_none());
        table.remove_owner(3);
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
    fn disabled_commands_pass_the_key_on_or_report_their_reason() {
        let mut table = Table::default();
        table
            .declare(HOST, "exit", "host", &["Escape".into()], false, false)
            .unwrap();
        table
            .declare(4, "close", "p.panel", &["Escape".into()], false, false)
            .unwrap();
        table
            .declare(5, "next", "p.list", &["N".into()], true, false)
            .unwrap();
        table
            .declare(6, "skip", "p.skip", &["N".into()], false, false)
            .unwrap();
        table
            .declare(7, "save", "p.save", &["S".into()], false, false)
            .unwrap();
        assert_eq!(table.resolve(&key("Escape"), None).unwrap().id, "exit");
        table.set_disabled(
            [
                ((HOST, "exit".to_owned()), None),
                ((5, "next".to_owned()), Some("Empty list".to_owned())),
                ((7, "save".to_owned()), Some("Nothing to save".to_owned())),
            ]
            .into_iter()
            .collect(),
        );
        // The conflict's loser takes the key while the winner is disabled.
        assert_eq!(table.resolve(&key("Escape"), None).unwrap().id, "close");
        // The focused panel's disabled command yields to the global one.
        assert_eq!(table.resolve(&key("N"), Some(5)).unwrap().id, "skip");
        let Lookup::Disabled { target, reason } = table.lookup(&key("S"), None) else {
            panic!("only a disabled command holds S");
        };
        assert_eq!(
            (target.id.as_str(), reason),
            ("save", Some("Nothing to save"))
        );
        assert!(table.resolve(&key("S"), None).is_none());
        table.set_disabled([((4, "close".to_owned()), None)].into_iter().collect());
        assert_eq!(table.resolve(&key("Escape"), None).unwrap().id, "exit");
        table.set_disabled(
            [
                ((HOST, "exit".to_owned()), None),
                ((4, "close".to_owned()), None),
            ]
            .into_iter()
            .collect(),
        );
        assert!(matches!(
            table.lookup(&key("Escape"), None),
            Lookup::Disabled { reason: None, .. }
        ));
        assert_eq!(table.lookup(&key("X"), None), Lookup::Unbound);
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
        assert_eq!(label_in(&key("Ctrl+ArrowLeft"), KeyLabels::Text), "Ctrl+←");
        assert_eq!(label_in(&key("PageDown"), KeyLabels::Text), "PgDn");
    }

    /// Menus show keys only, and `Cmd` bindings only where the system's
    /// menus do (macOS); on Windows `host/quit` shows no shortcut.
    #[test]
    fn menus_show_cmd_bindings_only_where_cmd_is_a_menu_key() {
        assert!(label_shown(&key("Cmd+Q"), true));
        assert!(!label_shown(&key("Cmd+Q"), false));
        assert!(!label_shown(&key("Ctrl+Cmd+F"), false));
        assert!(label_shown(&key("Ctrl+O"), false));
        assert!(label_shown(&key("Alt+Enter"), false));
        assert!(!label_shown(&key("Mouse:LeftDoubleClick"), true));
        let mut table = Table::default();
        table
            .declare(HOST, "quit", "host", &["Cmd+Q".into()], false, false)
            .unwrap();
        table
            .declare(
                HOST,
                "open",
                "host",
                &["Cmd+O".into(), "Ctrl+O".into()],
                false,
                false,
            )
            .unwrap();
        let shown = |id| table.label(HOST, id);
        if crate::platform::MENU_SHOWS_CMD {
            assert_eq!(shown("quit").as_deref(), Some("⌘Q"));
        } else {
            assert_eq!(shown("quit"), None);
            assert_eq!(shown("open").as_deref(), Some("Ctrl+O"));
        }
    }

    /// Menu shortcut text in both styles; `text` stays the canonical string.
    #[test]
    fn menu_labels_are_text_or_macos_symbols() {
        let both = |input: &str| {
            let parsed = parse(input).unwrap();
            assert_eq!(text(&parsed), input, "canonical form unchanged");
            (
                label_in(&parsed, KeyLabels::Text),
                label_in(&parsed, KeyLabels::Symbols),
            )
        };
        let cases = [
            ("Ctrl+O", "Ctrl+O", "⌘O"),
            ("Cmd+Q", "Cmd+Q", "⌘Q"),
            ("Ctrl+Shift+S", "Ctrl+Shift+S", "⇧⌘S"),
            ("Alt+ArrowRight", "Alt+→", "⌥→"),
            ("Ctrl+Alt+Shift+Cmd+F", "Ctrl+Alt+Shift+Cmd+F", "⌃⌥⇧⌘F"),
            ("Ctrl+Cmd+F", "Ctrl+Cmd+F", "⌃⌘F"),
            ("Shift+Tab", "Shift+Tab", "⇧Tab"),
            ("Alt+Delete", "Alt+Del", "⌥Del"),
            ("Ctrl+Backspace", "Ctrl+Backspace", "⌘Backspace"),
            ("Escape", "Esc", "Esc"),
            ("Enter", "Enter", "Enter"),
            ("PageUp", "PgUp", "PgUp"),
            ("End", "End", "End"),
            ("Space", "Space", "Space"),
            ("Ctrl+Plus", "Ctrl++", "⌘+"),
            ("Numpad5", "Num5", "Num5"),
            ("Shift+Numpad5", "Shift+Num5", "⇧Num5"),
            ("NumpadEnter", "NumEnter", "NumEnter"),
            ("F11", "F11", "F11"),
        ];
        for (input, text, symbols) in cases {
            assert_eq!(both(input), (text.into(), symbols.into()), "{input}");
        }
        // The running system's style.
        let expected = if cfg!(target_os = "macos") {
            "⌘O"
        } else {
            "Ctrl+O"
        };
        assert_eq!(label(&parse("Ctrl+O").unwrap()), expected);
    }

    #[test]
    fn symbols_typed_with_option_are_tried_before_the_physical_key() {
        let option = |code: &str, base: char, shift: bool| Physical {
            code: code.into(),
            base: Some(base),
            mods: Some(Mods {
                alt: true,
                shift,
                ..Mods::default()
            }),
        };
        // German layout: ⌥5 types "[", ⌥7 "|", ⌥⇧7 "\".
        let five = option("Digit5", '5', false);
        assert_eq!(
            typed_with_option("[", false, false, Some(&five)),
            Some(key("["))
        );
        assert_eq!(
            from_key("[", false, true, false, false, Some(&five)),
            Some(key("Alt+5"))
        );
        let seven = option("Digit7", '7', false);
        assert_eq!(
            typed_with_option("|", false, false, Some(&seven)),
            Some(key("|"))
        );
        let shifted = option("Digit7", '7', true);
        assert_eq!(
            typed_with_option("\\", false, false, Some(&shifted)),
            Some(key("\\"))
        );
        // Letters keep their physical meaning: ⌥O is Alt+O, not "ø".
        let o = option("KeyO", 'o', false);
        assert_eq!(typed_with_option("ø", false, false, Some(&o)), None);
        // Without Option, or for a non-character key, there is no alternative.
        let plain = Physical {
            mods: Some(Mods::default()),
            ..five.clone()
        };
        assert_eq!(typed_with_option("5", false, false, Some(&plain)), None);
        assert_eq!(typed_with_option("[", false, false, None), None);
        // US layout: ⌥[ types "“", which nothing binds, so Alt+[ still wins.
        let mut table = Table::default();
        table
            .declare(1, "a", "m", &["Alt+[".into(), "[".into()], false, false)
            .unwrap();
        let bracket = option("BracketLeft", '[', false);
        let typed = typed_with_option("“", false, false, Some(&bracket)).unwrap();
        assert!(table.resolve(&typed, None).is_none());
        assert!(
            table
                .resolve(
                    &from_key("“", false, true, false, false, Some(&bracket)).unwrap(),
                    None
                )
                .is_some()
        );
        // German ⌥5 reaches a "[" binding.
        assert!(table.resolve(&key("["), None).is_some());
    }

    #[test]
    fn named_keys_have_slint_codes_that_resolve_back() {
        for code in [
            "Delete",
            "Backspace",
            "ArrowLeft",
            "PageDown",
            "Home",
            "F11",
            "Enter",
        ] {
            let key = slint_key(code).unwrap().to_string();
            let physical = Physical {
                code: code.into(),
                base: None,
                mods: Some(Mods {
                    alt: true,
                    ..Mods::default()
                }),
            };
            // The re-dispatched key resolves by its recorded physical key.
            assert_eq!(
                from_key(&key, false, false, false, false, Some(&physical)),
                Some(key_input(&format!("Alt+{code}"))),
                "{code}"
            );
        }
        assert_eq!(slint_key("F24"), Some('\u{F71B}'));
        assert_eq!(slint_key("KeyA"), None);
    }

    fn key_input(text: &str) -> Input {
        parse(text).unwrap()
    }

    #[test]
    fn named_keys_resolve_by_key_even_without_text() {
        let press = |code: &str, alt: bool, shift: bool| Physical {
            code: code.into(),
            base: None,
            mods: Some(Mods {
                alt,
                shift,
                ..Mods::default()
            }),
        };
        // ⌥⌦ (fn+⌫ on laptops) arrives without text.
        assert_eq!(
            from_key(
                "",
                false,
                true,
                false,
                false,
                Some(&press("Delete", true, false))
            ),
            Some(key("Alt+Delete"))
        );
        assert_eq!(
            from_key(
                "",
                false,
                true,
                false,
                false,
                Some(&press("ArrowLeft", true, false))
            ),
            Some(key("Alt+ArrowLeft"))
        );
        assert_eq!(
            from_key(
                "\u{8}",
                false,
                true,
                false,
                false,
                Some(&press("Backspace", true, false))
            ),
            Some(key("Alt+Backspace"))
        );
        assert_eq!(
            from_key(
                "\u{19}",
                false,
                false,
                true,
                false,
                Some(&press("Tab", false, true))
            ),
            Some(key("Shift+Tab"))
        );
        assert_eq!(
            from_key(
                "",
                false,
                false,
                false,
                false,
                Some(&press("F11", false, false))
            ),
            Some(key("F11"))
        );
    }

    #[test]
    fn management_lists_current_shadowed_and_orphan_bindings() {
        let mut table = Table::default();
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        table
            .declare(
                HOST,
                "play-pause",
                "host",
                &strings(&["Space"]),
                false,
                false,
            )
            .unwrap();
        table
            .declare(
                4,
                "toggle",
                "a.plugin",
                &strings(&["Space", "P"]),
                false,
                false,
            )
            .unwrap();
        let before = table.revision();
        let infos = table.infos();
        let toggle = infos.iter().find(|i| i.id == "toggle").unwrap();
        assert_eq!(
            toggle.shadowed,
            vec![(key("Space"), "host".into(), "play-pause".into())]
        );
        assert!(!toggle.overridden);
        // The user moves play/pause to K and keeps a binding of a stopped plugin.
        let mut overlay = BTreeMap::new();
        overlay.insert("host/play-pause".into(), strings(&["K"]));
        overlay.insert("gone.plugin/x".into(), strings(&["Mouse:LeftClick"]));
        table.set_overlay(overlay);
        assert!(table.revision() > before);
        let infos = table.infos();
        let play = infos.iter().find(|i| i.id == "play-pause").unwrap();
        assert!(play.overridden);
        assert_eq!(play.current, vec![key("K")]);
        assert_eq!(play.defaults, vec![key("Space")]);
        assert!(
            infos
                .iter()
                .find(|i| i.id == "toggle")
                .unwrap()
                .shadowed
                .is_empty()
        );
        assert_eq!(table.resolve(&key("Space"), None).unwrap().id, "toggle");
        assert_eq!(
            table.orphans(),
            vec![("gone.plugin/x".into(), strings(&["Mouse:LeftClick"]))]
        );
        assert!(!table.has_left_click(), "an orphan binds nothing");
        assert!(table.declares("host", "play-pause") && !table.declares("host", "toggle"));
        assert_eq!(
            Table::canonical(&strings(&["Ctrl+Shift+A", "Mouse:LeftClick"])).unwrap(),
            strings(&["Ctrl+Shift+A", "Mouse:LeftClick"])
        );
        assert!(Table::canonical(&strings(&["K", "K"])).is_err());
        assert!(Table::canonical(&strings(&["Shift+Ctrl+A"])).is_err());
        table
            .declare(
                4,
                "click",
                "a.plugin",
                &strings(&["Mouse:LeftClick"]),
                false,
                false,
            )
            .unwrap();
        assert!(table.has_left_click());
        assert_eq!(text(&key("Alt+Shift+Cmd+F5")), "Alt+Shift+Cmd+F5");
    }

    /// Lazy activation: a parked plugin woken under a later generation keeps
    /// the keys it won, also after declaring them again live.
    #[test]
    fn a_rekeyed_owner_keeps_its_rank_and_its_panel_scope() {
        let mut table = Table::default();
        table
            .declare(2, "early", "a.early", &["Space".into()], false, false)
            .unwrap();
        table
            .declare(2, "pad", "a.early", &["Enter".into()], true, false)
            .unwrap();
        table
            .declare(5, "late", "b.late", &["Space".into()], false, false)
            .unwrap();
        assert_eq!(table.resolve(&key("Space"), None).unwrap().id, "early");
        let mut disabled = BTreeMap::new();
        disabled.insert((2, "pad".to_owned()), Some("why".to_owned()));
        table.set_disabled(disabled);
        table.rekey(2, 9);
        let target = table.resolve(&key("Space"), None).unwrap();
        assert_eq!((target.owner, target.id.as_str()), (9, "early"));
        assert!(matches!(
            table.lookup(&key("Enter"), Some(9)),
            Lookup::Disabled {
                reason: Some("why"),
                ..
            }
        ));
        assert!(table.resolve(&key("Enter"), Some(2)).is_none());
        // Declared again by the live plugin: still first.
        table
            .declare(9, "early", "a.early", &["Space".into()], false, false)
            .unwrap();
        assert_eq!(table.resolve(&key("Space"), None).unwrap().owner, 9);
        assert!(
            table
                .conflicts()
                .iter()
                .all(|c| c.contains("ignored: b.late"))
        );
        table.remove_owner(9);
        assert_eq!(table.resolve(&key("Space"), None).unwrap().id, "late");
    }
}
