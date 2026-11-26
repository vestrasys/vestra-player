//! `@tr(...)` in Slint source: find the translatable literals and put the
//! translations in their place. The player has
//! no Slint translator: it rewrites each `.slint` file as it is loaded, and
//! Slint then only formats (`{}`, `{0}`, `{n}`) and picks the plural form
//! (`n == 1`). The SDK's `vestra-plugin i18n extract` has the same scanner;
//! both run the cases in `tests/fixtures/slint-tr.json`.
//!
//! The lexer knows just enough Slint: line and nested block comments, string
//! literals with their escapes (`\" \\ \n \u{…}`) and string interpolation
//! (`"\{expr}"`), so that `@tr` inside a comment or a string is left alone.
//! A malformed call is left as it is: the Slint compiler reports it.
#![forbid(unsafe_code)]

use std::ops::Range;

/// One `@tr` call: `@tr("context" => "id" | "plural" % n, args…)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub context: Option<String>,
    pub id: String,
    pub plural: Option<String>,
    /// 1-based line of `@tr`.
    pub line: usize,
    id_span: Range<usize>,
    plural_span: Option<Range<usize>>,
}

/// Every `@tr` call, in source order.
pub fn scan(source: &str) -> Vec<Call> {
    let mut calls = Vec::new();
    let bytes = source.as_bytes();
    // Open string interpolations: the brace depth inside each.
    let mut templates: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = line_end(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = comment_end(bytes, i),
            b'"' => i = string_end(bytes, i + 1, &mut templates),
            b'{' if !templates.is_empty() => {
                *templates.last_mut().expect("not empty") += 1;
                i += 1;
            }
            b'}' if !templates.is_empty() => {
                let depth = templates.last_mut().expect("not empty");
                if *depth == 0 {
                    templates.pop();
                    // Back inside the string literal.
                    i = string_end(bytes, i + 1, &mut templates);
                } else {
                    *depth -= 1;
                    i += 1;
                }
            }
            b'@' => match call(source, i) {
                Some((found, end)) => {
                    calls.push(found);
                    i = end;
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    calls
}

/// The call that starts at `at` (an `@`), if it is one.
#[cfg(test)]
pub fn call_at(source: &str, at: usize) -> Option<Call> {
    call(source, at).map(|(call, _)| call)
}

/// The translation of one call: `(one, other)`; `other` only for a plural call.
pub type Translation = (String, Option<String>);

/// `source` with each translated literal replaced; untranslated calls stay.
pub fn rewrite(source: &str, translate: impl Fn(&Call) -> Option<Translation>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    for call in scan(source) {
        let Some((one, other)) = translate(&call) else {
            continue;
        };
        out.push_str(&source[copied..call.id_span.start]);
        out.push_str(&quote(&one));
        copied = call.id_span.end;
        if let (Some(span), Some(other)) = (&call.plural_span, other) {
            out.push_str(&source[copied..span.start]);
            out.push_str(&quote(&other));
            copied = span.end;
        }
    }
    out.push_str(&source[copied..]);
    out
}

/// A Slint string literal for `text` (no interpolation can come of it).
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|b| matches!(b, b'\n' | b'\r'))
        .map_or(bytes.len(), |p| from + p)
}

/// After a `/* … */` comment (nested like Slint's), or the end.
fn comment_end(bytes: &[u8], from: usize) -> usize {
    let mut depth = 0;
    let mut i = from + 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            if depth == 0 {
                return i + 2;
            }
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Inside a string at `from`: after its closing quote, or after `\{` (an
/// interpolation opens: pushed on `templates`).
fn string_end(bytes: &[u8], from: usize, templates: &mut Vec<usize>) -> usize {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i + 1,
            b'\\' if bytes.get(i + 1) == Some(&b'{') => {
                templates.push(0);
                return i + 2;
            }
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Skip white space and comments.
fn skip(bytes: &[u8], mut i: usize) -> usize {
    loop {
        match bytes.get(i) {
            Some(b) if b.is_ascii_whitespace() => i += 1,
            Some(b'/') if bytes.get(i + 1) == Some(&b'/') => i = line_end(bytes, i),
            Some(b'/') if bytes.get(i + 1) == Some(&b'*') => i = comment_end(bytes, i),
            _ => return i,
        }
    }
}

/// A plain string literal at `i` (no interpolation): its span and value.
fn literal(source: &str, i: usize) -> Option<(Range<usize>, String)> {
    let bytes = source.as_bytes();
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    let mut value = String::new();
    let mut plain = j;
    loop {
        match bytes.get(j)? {
            b'"' => {
                value.push_str(&source[plain..j]);
                return Some((i..j + 1, value));
            }
            b'\\' => {
                value.push_str(&source[plain..j]);
                match bytes.get(j + 1)? {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    b'u' => {
                        if bytes.get(j + 2) != Some(&b'{') {
                            return None;
                        }
                        let close = j + 3 + source[j + 3..].find('}')?;
                        let code = u32::from_str_radix(&source[j + 3..close], 16).ok()?;
                        value.push(char::from_u32(code)?);
                        j = close + 1;
                        plain = j;
                        continue;
                    }
                    // `\{` (interpolation) is not allowed in @tr; anything else is invalid.
                    _ => return None,
                }
                j += 2;
                plain = j;
            }
            _ => j += 1,
        }
    }
}

/// `@tr(` at `at`: the call and where scanning continues (after its last
/// literal; the arguments are ordinary code).
fn call(source: &str, at: usize) -> Option<(Call, usize)> {
    let bytes = source.as_bytes();
    let mut i = skip(bytes, at + 1);
    if !source[i..].starts_with("tr") {
        return None;
    }
    i += 2;
    if bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return None;
    }
    i = skip(bytes, i);
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    let (mut span, mut text) = literal(source, skip(bytes, i + 1))?;
    let mut context = None;
    let mut end = skip(bytes, span.end);
    if source[end..].starts_with("=>") {
        context = Some(text);
        (span, text) = literal(source, skip(bytes, end + 2))?;
        end = skip(bytes, span.end);
    }
    let mut plural = None;
    let mut plural_span = None;
    let mut resume = span.end;
    if bytes.get(end) == Some(&b'|') {
        let (other_span, other) = literal(source, skip(bytes, end + 1))?;
        resume = other_span.end;
        plural = Some(other);
        plural_span = Some(other_span);
    }
    let line = source[..at].matches('\n').count() + 1;
    Some((
        Call {
            context,
            id: text,
            plural,
            line,
            id_span: span,
            plural_span,
        },
        resume,
    ))
}
