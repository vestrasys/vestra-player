//! Gettext `.po` files: the parser and the compiled JSON catalog
//! (doc/architecture/i18n.md §3). Standard library only: `build.rs` includes
//! this file too, to compile `src/host/locales/<lang>.po` at build time.
//!
//! Supported: `msgctxt`, `msgid`, `msgid_plural`, `msgstr`, `msgstr[N]`,
//! continuation lines, the C escapes gettext writes (`\n \t \" \\`), the
//! `fuzzy` flag (left out of the catalog) and obsolete `#~` entries (ignored).
//! Plurals: only "one" (`n == 1`) and "other", as Slint's `@tr` without a
//! translator: `msgstr[0]` is the one form, `msgstr[1]` (else `msgstr[0]`)
//! the other.
#![forbid(unsafe_code)]

/// One message of a `.po` file. The header (`msgid ""`) is not an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub context: Option<String>,
    pub id: String,
    pub plural: Option<String>,
    /// `msgstr`, or `msgstr[0]`, `msgstr[1]`, … of a plural entry.
    pub strings: Vec<String>,
    pub fuzzy: bool,
}

impl Entry {
    /// The catalog key: `msgid`, or `msgctxt` U+0004 `msgid`.
    pub fn key(&self) -> String {
        key(self.context.as_deref(), &self.id)
    }
    /// Translated: a non-empty first string and not fuzzy.
    pub fn translated(&self) -> bool {
        !self.fuzzy && self.strings.first().is_some_and(|s| !s.is_empty())
    }
}

pub fn key(context: Option<&str>, id: &str) -> String {
    match context {
        Some(context) => format!("{context}\u{4}{id}"),
        None => id.to_owned(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    None,
    Context,
    Id,
    Plural,
    Str(usize),
}

/// Parse a `.po` file. Errors name the line (1-based).
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut entry = Entry::default();
    // The entry has a keyword line; `field` is the last one.
    let mut started = false;
    let mut field = Field::None;
    let mut fuzzy = false;
    let finish = |entry: &mut Entry, started: &mut bool, entries: &mut Vec<Entry>| {
        if *started && !(entry.id.is_empty() && entry.context.is_none()) {
            entries.push(std::mem::take(entry));
        }
        *entry = Entry::default();
        *started = false;
    };
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = |message: &str| format!("line {}: {message}", index + 1);
        if line.is_empty() {
            finish(&mut entry, &mut started, &mut entries);
            field = Field::None;
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            if comment.starts_with('~') {
                continue; // obsolete
            }
            // A comment after an entry's strings starts the next entry.
            if started {
                finish(&mut entry, &mut started, &mut entries);
                field = Field::None;
            }
            if let Some(flags) = comment.strip_prefix(',') {
                fuzzy |= flags.split(',').any(|flag| flag.trim() == "fuzzy");
            }
            continue;
        }
        let (keyword, rest) = if line.starts_with('"') {
            ("", line)
        } else {
            match line.find(char::is_whitespace) {
                Some(split) => (&line[..split], line[split..].trim()),
                None => return Err(at("a keyword without a string")),
            }
        };
        let next = match keyword {
            "" if field == Field::None => return Err(at("a string without a keyword")),
            "" => field,
            "msgctxt" => Field::Context,
            "msgid" => Field::Id,
            "msgid_plural" => Field::Plural,
            "msgstr" => Field::Str(0),
            other => Field::Str(
                other
                    .strip_prefix("msgstr[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| *n <= 16)
                    .ok_or_else(|| at(&format!("unknown keyword {other}")))?,
            ),
        };
        if !keyword.is_empty() {
            // msgctxt always opens an entry; msgid does unless it follows msgctxt.
            let opens = match next {
                Field::Context => true,
                Field::Id => field != Field::Context,
                _ => false,
            };
            if opens && started {
                finish(&mut entry, &mut started, &mut entries);
            }
            if !started {
                if !opens {
                    return Err(at("an entry starts with msgctxt or msgid"));
                }
                started = true;
                entry.fuzzy = std::mem::take(&mut fuzzy);
            }
        }
        let value = unquote(rest).map_err(|message| at(&message))?;
        match next {
            Field::Context => entry.context.get_or_insert_default().push_str(&value),
            Field::Id => entry.id.push_str(&value),
            Field::Plural => entry.plural.get_or_insert_default().push_str(&value),
            Field::Str(n) => {
                if entry.strings.len() <= n {
                    entry.strings.resize(n + 1, String::new());
                }
                entry.strings[n].push_str(&value);
            }
            Field::None => unreachable!("checked above"),
        }
        field = next;
    }
    finish(&mut entry, &mut started, &mut entries);
    Ok(entries)
}

fn unquote(text: &str) -> Result<String, String> {
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .ok_or("expected a quoted string")?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => return Err(format!("unknown escape \\{}", other.unwrap_or(' '))),
        }
    }
    Ok(out)
}

/// A JSON string literal.
#[cfg_attr(not(test), allow(dead_code))]
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The compiled catalog (`locales/<lang>.json`): translated entries only,
/// keys sorted; a plural entry is `[one, other]`. Used by `build.rs`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn compile(language: &str, entries: &[Entry]) -> String {
    let mut messages: Vec<(String, String)> = entries
        .iter()
        .filter(|entry| entry.translated())
        .map(|entry| {
            let value = match &entry.plural {
                None => json_string(&entry.strings[0]),
                Some(_) => {
                    let one = &entry.strings[0];
                    let other = entry
                        .strings
                        .get(1)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(one);
                    format!("[{},{}]", json_string(one), json_string(other))
                }
            };
            (entry.key(), value)
        })
        .collect();
    messages.sort();
    messages.dedup_by(|a, b| a.0 == b.0);
    let body = messages
        .iter()
        .map(|(key, value)| format!("{}:{value}", json_string(key)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"language\":{},\"messages\":{{{body}}}}}\n",
        json_string(language)
    )
}
