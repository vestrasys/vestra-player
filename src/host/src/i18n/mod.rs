//! Interface languages (doc/architecture/i18n.md): the effective locale of
//! this run, the translation catalogs of the player and of each plugin, and
//! the template formatting the player applies to plugin sentences.
//!
//! English is the source language: every interface string is written in
//! English and the catalogs translate it. The locale is fixed at start-up
//! (`init`): changing the language in the options takes effect at the next
//! start, so nothing here changes while the player runs.
//!
//! - `catalog_for(dir)`: the catalog of the package (or the player's `ui/`
//!   directory) a `.slint` file belongs to; the Slint file loader
//!   (`ui::compiler`) rewrites `@tr` literals with it (`slint_tr`).
//! - `Catalog::text`, `Catalog::template`: declarations and sentences a plugin
//!   hands the player (menu titles, OSD messages) in that plugin's catalog.
//! - `tr`, `tr_args`: the player's own strings, in `src/host/locales/`.
#![forbid(unsafe_code)]

pub mod po;
pub mod slint_tr;
pub mod text;

use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// The interface languages: English (the source) first, then the
/// translations. Mirrored by `LOCALES` in the SDK (host-agreement test).
pub const LOCALES: [&str; 7] = ["en", "zh-Hans", "ja", "ko", "de", "fr", "es"];

/// `ui_state.appearance.language`: `"system"` or one of `LOCALES`.
pub const SYSTEM: &str = "system";

pub fn valid_preference(value: &str) -> bool {
    value == SYSTEM || LOCALES.contains(&value)
}

/// The supported locale for a BCP 47 tag (`zh-Hans-CN`, `zh_TW`, `de-AT`,
/// `en-US`), `None` when the language is not one of ours. Chinese: the
/// Simplified script is `zh-Hans`; Traditional (`zh-Hant`, or Taiwan, Hong
/// Kong, Macao without a script) is English, as the user decided
/// (2026-10-24: no Traditional catalog; those users read English better than
/// Simplified).
pub fn match_tag(tag: &str) -> Option<&'static str> {
    let tag = tag.replace('_', "-").to_ascii_lowercase();
    // POSIX forms: `zh_CN.UTF-8`, `de_DE@euro`.
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    if language == "zh" {
        let script = rest.iter().find(|part| part.len() == 4);
        return Some(match script {
            Some(&"hans") => "zh-Hans",
            Some(_) => "en",
            None if rest.iter().any(|r| matches!(*r, "tw" | "hk" | "mo")) => "en",
            None => "zh-Hans",
        });
    }
    LOCALES
        .iter()
        .copied()
        .find(|locale| !locale.contains('-') && *locale == language)
}

/// The effective locale: the saved preference (when it names a locale),
/// else the first of the system's preferred languages that we support,
/// else English.
pub fn resolve(preference: Option<&str>, preferred: &[String]) -> &'static str {
    if let Some(locale) = preference.and_then(|p| LOCALES.iter().copied().find(|l| *l == p)) {
        return locale;
    }
    preferred
        .iter()
        .find_map(|tag| match_tag(tag))
        .unwrap_or("en")
}

static LOCALE: OnceLock<&'static str> = OnceLock::new();

/// Fix the locale for this run (once; later calls are ignored).
pub fn init(locale: &'static str) {
    let _ = LOCALE.set(locale);
}

/// The effective locale (English until `init`).
pub fn locale() -> &'static str {
    LOCALE.get().copied().unwrap_or("en")
}

/// One language's messages: `key` is the msgid, or msgctxt U+0004 msgid.
#[derive(Debug, Default, PartialEq)]
pub struct Catalog {
    messages: HashMap<String, Message>,
}

#[derive(Debug, PartialEq)]
enum Message {
    One(String),
    Plural(String, String),
}

impl Catalog {
    pub fn from_entries(entries: &[po::Entry]) -> Self {
        let mut messages = HashMap::new();
        for entry in entries.iter().filter(|entry| entry.translated()) {
            let one = entry.strings[0].clone();
            let message = match &entry.plural {
                None => Message::One(one),
                Some(_) => {
                    let other = entry
                        .strings
                        .get(1)
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .unwrap_or_else(|| one.clone());
                    Message::Plural(one, other)
                }
            };
            messages.entry(entry.key()).or_insert(message);
        }
        Self { messages }
    }
    /// A compiled catalog (`locales/<lang>.json`, `po::compile`).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let messages = value["messages"]
            .as_object()
            .ok_or("no \"messages\" object")?;
        let mut out = HashMap::new();
        for (key, message) in messages {
            let message = match message {
                Value::String(one) => Message::One(one.clone()),
                Value::Array(forms) => match forms.as_slice() {
                    [Value::String(one), Value::String(other)] => {
                        Message::Plural(one.clone(), other.clone())
                    }
                    _ => return Err(format!("{key}: a plural is [one, other]")),
                },
                _ => return Err(format!("{key}: a string or [one, other]")),
            };
            out.insert(key.clone(), message);
        }
        Ok(Self { messages: out })
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.messages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
    fn get(&self, context: Option<&str>, id: &str) -> Option<&Message> {
        match context {
            None => self.messages.get(id),
            Some(context) => self.messages.get(&po::key(Some(context), id)),
        }
    }
    /// The translation of a sentence (plural entries give their one form).
    pub fn lookup(&self, context: Option<&str>, id: &str) -> Option<&str> {
        match self.get(context, id)? {
            Message::One(text) | Message::Plural(text, _) => Some(text),
        }
    }
    /// The translation of `id`, or `id` itself.
    pub fn text<'a>(&'a self, id: &'a str) -> &'a str {
        self.lookup(None, id).unwrap_or(id)
    }
    /// A plugin sentence: the translated template, formatted with `args`
    /// when there are any (without args the text is shown as it is, braces
    /// included, as before templates existed).
    #[cfg(test)]
    pub fn template(&self, text: &str, args: &Value) -> String {
        let template = self.text(text);
        if args.is_null() {
            template.to_owned()
        } else {
            format(template, args)
        }
    }
    /// What an `@tr` call becomes, `None` when untranslated.
    pub fn slint(&self, call: &slint_tr::Call) -> Option<slint_tr::Translation> {
        match (self.get(call.context.as_deref(), &call.id)?, &call.plural) {
            (Message::Plural(one, other), Some(_)) => Some((one.clone(), Some(other.clone()))),
            (Message::One(one), None) => Some((one.clone(), None)),
            // A singular entry for a plural call (or the reverse): the
            // translator's file does not fit the source; keep English.
            _ => None,
        }
    }
    /// `source` with its `@tr` literals translated.
    pub fn rewrite(&self, source: &str) -> String {
        if self.is_empty() || !source.contains("@") {
            return source.to_owned();
        }
        slint_tr::rewrite(source, |call| self.slint(call))
    }
}

/// Format a template the way Slint's `@tr` does: `{}` takes the next
/// argument, `{0}` the first, `{name}` a field of an object argument; `{{`
/// and `}}` are braces. A placeholder without an argument stays as written.
pub fn format(template: &str, args: &Value) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            out.push('}');
            rest = after;
            continue;
        }
        let Some(close) = tail.find('}') else {
            out.push_str(tail);
            rest = "";
            break;
        };
        let name = &tail[1..close];
        let value = if name.is_empty() {
            next += 1;
            args.get(next - 1)
        } else if let Ok(index) = name.parse::<usize>() {
            args.get(index)
        } else {
            args.get(name)
        };
        match value {
            Some(value) => out.push_str(&argument(value)),
            None => out.push_str(&tail[..=close]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// An argument as text: strings as they are, whole numbers without a
/// fraction (`5`, not `5.0`).
fn argument(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
            Some(n) => format!("{n}"),
            None => number.to_string(),
        },
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Check plugin template arguments (`args` of `osd.show`, dialogs):
/// absent, or an array or object of at most 16 strings, numbers and text
/// values (`text::check_args`).
pub fn valid_args(args: &Value) -> Result<(), String> {
    text::check_args(args)
}

/// The catalog of a package directory for the effective locale: the
/// compiled `locales/<lang>.json` of a built package, else the source
/// `locales/<lang>.po` of a development load. `None` for English and for a
/// package without that language. A broken file is reported once and
/// treated as missing.
pub fn load(directory: &Path) -> Option<Arc<Catalog>> {
    load_locale(directory, locale())
}

pub fn load_locale(directory: &Path, locale: &str) -> Option<Arc<Catalog>> {
    if locale == "en" {
        return None;
    }
    let base = directory.join("locales");
    let json = base.join(format!("{locale}.json"));
    let result = if json.is_file() {
        std::fs::read_to_string(&json)
            .map_err(|e| e.to_string())
            .and_then(|text| Catalog::from_json(&text))
            .map_err(|e| format!("{}: {e}", json.display()))
    } else {
        let source = base.join(format!("{locale}.po"));
        if !source.is_file() {
            return None;
        }
        std::fs::read_to_string(&source)
            .map_err(|e| e.to_string())
            .and_then(|text| po::parse(&text))
            .map(|entries| Catalog::from_entries(&entries))
            .map_err(|e| format!("{}: {e}", source.display()))
    };
    match result {
        Ok(catalog) => Some(Arc::new(catalog)),
        Err(error) => {
            diag!("i18n: {error}");
            None
        }
    }
}

// `HOST_CATALOGS`: (locale, compiled catalog) for every
// `src/host/locales/<lang>.po`, generated by `build.rs`.
include!(concat!(env!("OUT_DIR"), "/host_catalogs.rs"));

/// The player's own catalogs, compiled from `src/host/locales/*.po` by
/// `build.rs` (`HOST_CATALOGS`).
fn host_json(locale: &str) -> Option<&'static str> {
    HOST_CATALOGS
        .iter()
        .find(|(language, _)| *language == locale)
        .map(|(_, json)| *json)
}

/// The player's catalog of `locale`, whatever the locale of this run.
#[cfg(test)]
pub fn host_catalog(locale: &str) -> Option<Catalog> {
    host_json(locale).map(|text| Catalog::from_json(text).unwrap())
}

static HOST: OnceLock<Option<Arc<Catalog>>> = OnceLock::new();

/// The player's catalog for the effective locale.
pub fn host() -> Option<&'static Arc<Catalog>> {
    HOST.get_or_init(|| {
        host_json(locale())
            .map(|text| Arc::new(Catalog::from_json(text).expect("build.rs writes valid catalogs")))
    })
    .as_ref()
}

/// A player string in the interface language.
pub fn tr(text: &str) -> String {
    host().map_or(text, |catalog| catalog.text(text)).to_owned()
}

/// A player sentence with arguments: `tr_args("Back {}s", &json!([5]))`.
pub fn tr_args(text: &str, args: &Value) -> String {
    format(host().map_or(text, |catalog| catalog.text(text)), args)
}

/// Which catalog a `.slint` file is translated with: the package (or the
/// player's own `ui/` directory) whose directory holds it. The longest
/// matching directory wins.
static OWNERS: Mutex<Vec<(PathBuf, Arc<Catalog>)>> = Mutex::new(Vec::new());

/// A package's catalog for its `.slint` files (replaces an earlier one for
/// the same directory).
pub fn register(directory: &Path, catalog: Arc<Catalog>) {
    let mut owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    owners.retain(|(dir, _)| dir != directory);
    owners.push((directory.to_owned(), catalog));
}

/// Forget a package's catalog, if it is still this one.
pub fn unregister(directory: &Path, catalog: &Arc<Catalog>) {
    let mut owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    owners.retain(|(dir, owned)| !(dir == directory && Arc::ptr_eq(owned, catalog)));
}

pub fn catalog_for(file: &Path) -> Option<Arc<Catalog>> {
    let canonical = dunce::canonicalize(file).unwrap_or_else(|_| file.to_owned());
    let owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let found = owners
        .iter()
        .filter(|(dir, _)| canonical.starts_with(dir) || file.starts_with(dir))
        .max_by_key(|(dir, _)| dir.as_os_str().len())
        .map(|(_, catalog)| catalog.clone());
    drop(owners);
    found.or_else(|| {
        let ui = crate::ui::view::ui_dir();
        let ui = dunce::canonicalize(&ui).unwrap_or(ui);
        (canonical.starts_with(&ui) || file.starts_with(&ui))
            .then(|| host().cloned())
            .flatten()
    })
}

/// The source of a `.slint` file in the interface language.
pub fn translate_slint(file: &Path, source: String) -> String {
    if !source.contains("@tr") {
        return source;
    }
    match catalog_for(file) {
        Some(catalog) => catalog.rewrite(&source),
        None => source,
    }
}

/// CJK glyph variants (doc/architecture/i18n.md §8). Slint picks the
/// system UI font and falls back per character; Chinese, Japanese and Korean
/// share code points whose preferred glyphs differ by language (骨, 直, 角).
/// The user allowed a per-language system UI font for `ja` and `ko` only if
/// the default glyphs turn out wrong in phase verification; until then this
/// names none. TODO(i18n phase verification): fill in only after screenshots
/// show Chinese glyph forms in a `ja` or `ko` interface, and set it where
/// `theme::apply_fonts` sets the per-system fonts.
#[cfg_attr(not(test), allow(dead_code))]
pub fn ui_font_family(_locale: &str) -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests;
