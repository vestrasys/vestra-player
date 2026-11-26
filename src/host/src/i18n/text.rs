//! Text values: what a plugin hands the
//! player instead of a sentence it may not compose. `{"$vestraText":
//! source, "args": [...]}` is English source text with its arguments;
//! `{"$vestraList": [...], "style": "list"}` joins items the interface
//! language's way. Arguments are data (strings, numbers) or text values,
//! which are translated too. The player renders them with the plugin's
//! catalog wherever it shows a plugin's text: command titles, path
//! segments and reasons, OSD messages and their arguments, dialog texts,
//! file dialog titles and file names, resolver failures, and property
//! values (in place of a string, at any depth). The SDK has the same rules
//! (`textProblem`, `renderText`, `LIST_SEPARATORS`, `TEXT_LIMITS`; the
//! host-agreement test checks them).
#![forbid(unsafe_code)]

use super::{Catalog, format};
use serde_json::{Map, Value};

pub const TEXT_KEY: &str = "$vestraText";
pub const LIST_KEY: &str = "$vestraList";

/// Nesting depth of texts and lists; the value itself is depth 0.
pub const MAX_DEPTH: usize = 8;
/// Texts, lists and arguments in one value, the value itself included.
pub const MAX_NODES: usize = 1024;
/// Arguments of one text.
pub const MAX_ARGS: usize = 16;
/// Items of one list.
pub const MAX_ITEMS: usize = 256;
/// UTF-8 bytes of a source text or a string argument.
pub const MAX_STRING_BYTES: usize = 4096;

/// List styles, in the order of each row of [`SEPARATORS`].
pub const STYLES: [&str; 5] = ["list", "clauses", "semicolons", "sentences", "lines"];

/// The separator of each style per interface language.
pub const SEPARATORS: [(&str, [&str; 5]); 7] = [
    ("en", [", ", ", ", "; ", ". ", "\n"]),
    ("zh-Hans", ["、", "，", "；", "。", "\n"]),
    ("ja", ["、", "、", "；", "。", "\n"]),
    ("ko", [", ", ", ", "; ", ". ", "\n"]),
    ("de", [", ", ", ", "; ", ". ", "\n"]),
    ("fr", [", ", ", ", "\u{202f}; ", ". ", "\n"]),
    ("es", [", ", ", ", "; ", ". ", "\n"]),
];

pub fn separator(locale: &str, style: &str) -> &'static str {
    let row = SEPARATORS
        .iter()
        .find(|(l, _)| *l == locale)
        .unwrap_or(&SEPARATORS[0]);
    let index = STYLES.iter().position(|s| *s == style).unwrap_or(0);
    row.1[index]
}

/// A text or list value (by its marker key).
pub fn is_text(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.contains_key(TEXT_KEY) || o.contains_key(LIST_KEY))
}

/// Whether a value holds a text value anywhere.
pub fn contains(value: &Value) -> bool {
    match value {
        Value::Object(fields) => is_text(value) || fields.values().any(contains),
        Value::Array(items) => items.iter().any(contains),
        _ => false,
    }
}

struct Checker {
    nodes: usize,
}

impl Checker {
    fn count(&mut self) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("a text holds at most {MAX_NODES} values"));
        }
        Ok(())
    }
    fn args(&mut self, args: &Value, depth: usize) -> Result<(), String> {
        let values: Vec<&Value> = match args {
            Value::Array(values) => values.iter().collect(),
            Value::Object(fields) => fields.values().collect(),
            _ => return Err("args is an array or an object".into()),
        };
        if values.len() > MAX_ARGS {
            return Err(format!("args holds at most {MAX_ARGS} values"));
        }
        values.into_iter().try_for_each(|v| self.arg(v, depth))
    }
    fn arg(&mut self, value: &Value, depth: usize) -> Result<(), String> {
        match value {
            Value::Number(n) => {
                if !n.as_f64().is_some_and(f64::is_finite) {
                    return Err("a number argument is finite".into());
                }
                self.count()
            }
            Value::String(s) => {
                if s.len() > MAX_STRING_BYTES {
                    return Err(format!(
                        "a string argument is at most {MAX_STRING_BYTES} bytes"
                    ));
                }
                self.count()
            }
            v if is_text(v) => self.text(v, depth + 1),
            _ => Err("args are strings, numbers, texts and lists".into()),
        }
    }
    fn text(&mut self, value: &Value, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("texts nest at most {MAX_DEPTH} deep"));
        }
        let Some(fields) = value.as_object().filter(|_| is_text(value)) else {
            return Err("a text is a $vestraText or $vestraList object".into());
        };
        self.count()?;
        if let Some(source) = fields.get(TEXT_KEY) {
            if fields.keys().any(|k| k != TEXT_KEY && k != "args") {
                return Err("a text has only $vestraText and args".into());
            }
            let source = source
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("a text's source is a non-empty string")?;
            if source.len() > MAX_STRING_BYTES {
                return Err(format!(
                    "a text's source is at most {MAX_STRING_BYTES} bytes"
                ));
            }
            return match fields.get("args") {
                None => Ok(()),
                Some(args) => self.args(args, depth),
            };
        }
        if fields.keys().any(|k| k != LIST_KEY && k != "style") {
            return Err("a list has only $vestraList and style".into());
        }
        let items = fields[LIST_KEY]
            .as_array()
            .ok_or("a list's items are an array")?;
        if items.len() > MAX_ITEMS {
            return Err(format!("a list holds at most {MAX_ITEMS} items"));
        }
        if let Some(style) = fields.get("style")
            && !style.as_str().is_some_and(|s| STYLES.contains(&s))
        {
            return Err(format!("a list's style is one of {}", STYLES.join(", ")));
        }
        items.iter().try_for_each(|item| self.arg(item, depth))
    }
}

/// Why a text value would be refused (the SDK's `textProblem`).
pub fn check(value: &Value) -> Result<(), String> {
    Checker { nodes: 0 }.text(value, 0)
}

/// Check template arguments: absent, or an array or object of at most 16
/// strings, finite numbers and text values.
pub fn check_args(args: &Value) -> Result<(), String> {
    if args.is_null() {
        return Ok(());
    }
    Checker { nodes: 0 }.args(args, 0)
}

/// Renders text values in one language with one plugin's catalog.
#[derive(Clone, Copy)]
pub struct Texts<'a> {
    pub catalog: Option<&'a Catalog>,
    pub locale: &'a str,
}

impl<'a> Texts<'a> {
    /// English without a catalog: what a menu merges by.
    pub fn english() -> Texts<'static> {
        Texts {
            catalog: None,
            locale: "en",
        }
    }
    fn source<'b>(&self, id: &'b str) -> &'b str
    where
        'a: 'b,
    {
        self.catalog.map_or(id, |c| c.text(id))
    }
    /// A checked text value as shown.
    pub fn render(&self, value: &Value) -> String {
        if let Some(source) = value.get(TEXT_KEY).and_then(Value::as_str) {
            let args = value
                .get("args")
                .map_or(Value::Array(Vec::new()), |a| self.args(a));
            return format(self.source(source), &args);
        }
        let style = value.get("style").and_then(Value::as_str).unwrap_or("list");
        let separator = separator(self.locale, style);
        value[LIST_KEY]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| match self.arg(item) {
                        Value::String(s) => s,
                        other => format("{}", &Value::Array(vec![other])),
                    })
                    .collect::<Vec<_>>()
                    .join(separator)
            })
            .unwrap_or_default()
    }
    fn arg(&self, value: &Value) -> Value {
        if is_text(value) {
            Value::String(self.render(value))
        } else {
            value.clone()
        }
    }
    /// Checked template arguments with their text values rendered.
    pub fn args(&self, args: &Value) -> Value {
        match args {
            Value::Array(items) => Value::Array(items.iter().map(|a| self.arg(a)).collect()),
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.arg(v)))
                    .collect::<Map<_, _>>(),
            ),
            other => other.clone(),
        }
    }
    /// A declared text (a menu title, a reason, a button): a string is
    /// translated as a whole, a text value rendered.
    pub fn declared(&self, value: &Value) -> Result<String, String> {
        self.sentence(value, &Value::Null)
    }
    /// A sentence (`osd.show`, dialogs): a string is a template, translated
    /// and formatted with `args` when there are any (without, it shows as
    /// it is); a text value carries its own arguments.
    pub fn sentence(&self, text: &Value, args: &Value) -> Result<String, String> {
        check_args(args)?;
        match text {
            Value::String(text) => {
                let template = self.source(text);
                Ok(if args.is_null() {
                    template.to_owned()
                } else {
                    format(template, &self.args(args))
                })
            }
            value if is_text(value) => {
                check(value)?;
                if !args.is_null() {
                    return Err("a text value takes no args: they go inside it".into());
                }
                Ok(self.render(value))
            }
            _ => Err("a text is a string or a text value".into()),
        }
    }
    /// Replace every text value in a property value (or request) with the
    /// string it shows; a runtime image or Markdown document is left alone.
    pub fn resolve(&self, value: &mut Value) -> Result<(), String> {
        if is_text(value) {
            check(value)?;
            *value = Value::String(self.render(value));
            return Ok(());
        }
        match value {
            Value::Array(items) => items.iter_mut().try_for_each(|v| self.resolve(v)),
            Value::Object(fields) => {
                if fields.contains_key("$vestraImage") || fields.contains_key("$vestraMarkdown") {
                    return Ok(());
                }
                fields.values_mut().try_for_each(|v| self.resolve(v))
            }
            _ => Ok(()),
        }
    }
}

/// Columns of one character: 2 for East Asian wide and full-width, 0 for
/// a combining mark, else 1 (`unicode-width`).
fn columns(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    match c.width() {
        Some(2) => 2,
        Some(0) if !c.is_ascii() => 0,
        _ => 1,
    }
}

/// Each line cut to `max` columns, ending with `…` when cut (an OSD
/// panel's `maxColumns`: cut after translation, so the plugin need not
/// measure translated text).
pub fn fit_columns(text: &str, max: usize) -> String {
    text.split('\n')
        .map(|line| {
            if line.chars().map(columns).sum::<usize>() <= max {
                return line.to_owned();
            }
            let mut used = 0;
            let mut out = String::new();
            for c in line.chars() {
                let width = columns(c);
                if used + width > max.saturating_sub(1) {
                    break;
                }
                out.push(c);
                used += width;
            }
            out.push('…');
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}
