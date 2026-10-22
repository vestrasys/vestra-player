//! Markdown documents (`plugin.ui.markdown`, store proposal §5): a property
//! value the player parses (pulldown-cmark) into the `[MarkdownBlock]` rows
//! `@vestra/markdown.slint` draws. Inline styles become Slint `styled-text`;
//! images load through the runtime image rules (`ui_image`), and remote ones
//! only from the files the SDK downloaded (`images`: address → local path).
//! Embedded HTML is dropped.
#![forbid(unsafe_code)]
use super::*;
use pulldown_cmark::{Event as Md, Options, Parser, Tag, TagEnd};
use slint::StyledText;
// Filling placeholders has no public API in Slint 1.18.1
// (`StyledText::from_markdown` takes no arguments): recheck on every upgrade.
use slint::private_unstable_api::re_exports::parse_markdown;
use slint_interpreter::Struct;

/// The wire form the SDK sends for a `MarkdownRef` (an internal detail).
pub const WIRE_KEY: &str = "$vestraMarkdown";
pub const MAX_TEXT: usize = 256 * 1024;
pub const MAX_BLOCKS: usize = 4000;
pub const MAX_IMAGES: usize = 64;
pub const MAX_COLUMNS: usize = 16;
pub const MAX_ROWS: usize = 500;
const MAX_INDENT: i32 = 3;
const MAX_QUOTE: i32 = 4;
const TRUNCATED: &str = "…（内容过长，已截断）";
/// Slint's interpolation placeholder (U+E541): user text never reaches the
/// format string, so it cannot inject Markdown into the inline renderer.
const SLOT: char = '\u{e541}';
/// Emphasis and strikethrough also get a colour: CJK system fonts (PingFang
/// SC, Microsoft YaHei, Hiragino Sans GB, which CJK text falls back to)
/// have no italic face and Slint 1.18.1's FemtoVG and software renderers do
/// not synthesize one (only Skia does), so italic CJK draws like body text;
/// the strikethrough line drew no pixel at 11–12px, the Markdown body sizes
/// (both measured 2026-10-22 under the then-forced Hiragino Sans GB). Literal colours (the format string
/// cannot read `Theme`), chosen to read on both palettes.
const EMPHASIS_COLOR: &str = "#e8a33d";
const STRIKE_COLOR: &str = "#8a8496";

/// `{ "$vestraMarkdown": { text, base, images }, "ref": n }`.
pub fn reference(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    let doc = object.get(WIRE_KEY)?;
    object
        .keys()
        .all(|key| key == WIRE_KEY || key == "ref")
        .then_some(doc)
}

#[derive(Debug, Default)]
pub struct Doc {
    pub text: String,
    /// An http(s) address or an absolute directory relative links and
    /// images resolve against.
    pub base: Option<String>,
    /// Remote image address → the local file the SDK downloaded.
    pub images: BTreeMap<String, String>,
}

pub fn parse_wire(value: &Value) -> std::result::Result<Doc, RpcError> {
    let invalid = |m: &str| RpcError::invalid(format!("markdown: {m}"));
    let object = value
        .as_object()
        .ok_or_else(|| invalid("expected an object"))?;
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("text must be a string"))?;
    if text.len() > MAX_TEXT {
        return Err(invalid("text exceeds 256 KB"));
    }
    let base = match object.get("base") {
        None | Some(Value::Null) => None,
        Some(Value::String(base)) => Some(base.clone()),
        _ => return Err(invalid("base must be a string")),
    };
    let mut images = BTreeMap::new();
    if let Some(map) = object.get("images").filter(|v| !v.is_null()) {
        let map = map
            .as_object()
            .ok_or_else(|| invalid("images must be an object"))?;
        if map.len() > MAX_IMAGES {
            return Err(invalid("too many images"));
        }
        for (url, path) in map {
            let path = path
                .as_str()
                .ok_or_else(|| invalid("image paths must be strings"))?;
            images.insert(url.clone(), path.to_owned());
        }
    }
    Ok(Doc {
        text: text.to_owned(),
        base,
        images,
    })
}

/// Inline content: a Slint Markdown format string whose only text is
/// placeholders, the texts that fill them, and the plain text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Inline {
    pub format: String,
    pub args: Vec<String>,
    pub plain: String,
}
impl Inline {
    fn text(&mut self, text: &str) {
        let text = text.replace(SLOT, "");
        if text.is_empty() {
            return;
        }
        self.format.push(SLOT);
        self.plain.push_str(&text);
        self.args.push(text);
    }
    fn is_empty(&self) -> bool {
        self.plain.trim().is_empty()
    }
    /// Inline code: one `Code` span (Slint's monospace capsule); CJK inside
    /// it is drawn through font fallback.
    fn code(&mut self, text: &str) {
        let slots = self.args.len();
        let start = self.format.len();
        self.format.push('`');
        self.text(text);
        if self.args.len() == slots {
            self.format.truncate(start);
        } else {
            self.format.push('`');
        }
    }
    /// A code block: plain lines separated by hard breaks, indentation and
    /// empty lines kept. Not the `Code` style, which would give every line
    /// its own capsule: `markdown.slint` draws the block in the monospace
    /// font.
    fn code_block(code: &str) -> Self {
        let mut inline = Self::default();
        for (index, line) in code.split('\n').enumerate() {
            if index > 0 {
                inline.format.push_str("  \n");
                inline.plain.push('\n');
            }
            if line.trim().is_empty() {
                // Keeps the empty line (a paragraph needs some text).
                inline.text("\u{a0}");
            } else {
                inline.text(line);
            }
        }
        inline
    }
    /// The Slint value; plain text if Slint's parser rejects the format.
    pub fn styled(&self, strong: bool) -> StyledText {
        let format = if strong
            && !self.format.is_empty()
            && !self.format.starts_with('*')
            && !self.format.ends_with('*')
        {
            format!("**{}**", self.format)
        } else {
            self.format.clone()
        };
        let args = self
            .args
            .iter()
            .map(|a| StyledText::from_plain_text(a))
            .collect::<Vec<_>>();
        // Slint 1.18 logs a parse error and returns what it parsed (1.16
        // panicked); the guard stays because `parse_markdown` is private
        // and may change again.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_markdown(&format, &args)
        }))
        .unwrap_or_else(|_| StyledText::from_plain_text(&self.plain))
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Block {
    pub kind: &'static str,
    pub level: i32,
    pub indent: i32,
    pub quote: i32,
    pub marker: String,
    /// Task list item: 0 none, 1 open, 2 done (drawn as a box, not a glyph).
    pub task: i32,
    pub inline: Inline,
    pub code: String,
    /// `image`: the resolved source (address or path) and alt text.
    pub source: Option<String>,
    pub alt: String,
    pub rows: Vec<(Vec<String>, bool)>,
    /// Table column alignment: "left", "center", "right" ("" for none).
    pub aligns: Vec<&'static str>,
}

/// Where a link or image points: absolute http(s)/mailto addresses as they
/// are; relative ones against an http(s) base or a directory base.
pub fn resolve(base: Option<&str>, target: &str) -> String {
    if target.starts_with('#') || url::Url::parse(target).is_ok() {
        return target.to_owned();
    }
    match base {
        Some(base) if base.starts_with("http://") || base.starts_with("https://") => {
            url::Url::parse(base)
                .and_then(|b| b.join(target))
                .map(|u| u.to_string())
                .unwrap_or_else(|_| target.to_owned())
        }
        Some(base) if Path::new(base).is_absolute() && !Path::new(target).is_absolute() => {
            Path::new(base).join(target).to_string_lossy().into_owned()
        }
        _ => target.to_owned(),
    }
}

/// A link destination inside the format string: `<…>` with the characters
/// that would end it percent-encoded.
fn destination(url: &str) -> String {
    let mut out = String::from("<");
    for c in url.chars() {
        match c {
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            ' ' => out.push_str("%20"),
            '\\' => out.push_str("%5C"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('>');
    out
}

struct Builder<'a> {
    base: Option<&'a str>,
    blocks: Vec<Block>,
    current: Option<Block>,
    quote: i32,
    lists: Vec<Option<u64>>,
    /// The next block opened in this item takes its marker.
    pending_marker: Option<String>,
    in_item: bool,
    heading: Option<i32>,
    code: Option<String>,
    image: Option<(String, String)>,
    table: Option<Vec<(Vec<String>, bool)>>,
    aligns: Vec<&'static str>,
    cell: Option<String>,
    in_head: bool,
    images: usize,
    truncated: bool,
}

impl Builder<'_> {
    fn indent(&self) -> i32 {
        (self.lists.len() as i32 - 1).clamp(0, MAX_INDENT)
    }
    fn open(&mut self) -> &mut Block {
        if self.current.is_none() {
            let (kind, marker) = if let Some(level) = self.heading {
                let _ = level;
                ("heading", String::new())
            } else if self.in_item {
                ("item", self.pending_marker.take().unwrap_or_default())
            } else {
                ("paragraph", String::new())
            };
            self.current = Some(Block {
                kind,
                level: self.heading.unwrap_or(0),
                indent: if self.in_item { self.indent() } else { 0 },
                quote: self.quote.min(MAX_QUOTE),
                marker,
                ..Default::default()
            });
        }
        self.current.as_mut().unwrap()
    }
    fn flush(&mut self) {
        if let Some(block) = self.current.take()
            && (!block.inline.is_empty() || !block.marker.is_empty() || block.task > 0)
        {
            self.push(block);
        }
    }
    fn push(&mut self, block: Block) {
        if self.blocks.len() >= MAX_BLOCKS - 1 {
            self.truncated = true;
            return;
        }
        self.blocks.push(block);
    }
    fn inline(&mut self, f: impl FnOnce(&mut Inline)) {
        if let Some(cell) = self.cell.as_mut() {
            let mut inline = Inline::default();
            f(&mut inline);
            cell.push_str(&inline.plain);
            return;
        }
        if let Some((_, alt)) = self.image.as_mut() {
            let mut inline = Inline::default();
            f(&mut inline);
            alt.push_str(&inline.plain);
            return;
        }
        f(&mut self.open().inline);
    }
    fn markup(&mut self, text: &str) {
        if self.cell.is_some() || self.image.is_some() {
            return;
        }
        self.open().inline.format.push_str(text);
    }

    fn event(&mut self, event: Md<'_>) {
        if let Some(code) = self.code.as_mut() {
            match event {
                Md::Text(text) => code.push_str(&text),
                Md::End(TagEnd::CodeBlock) => {
                    let mut code = self.code.take().unwrap_or_default();
                    while code.ends_with('\n') {
                        code.pop();
                    }
                    self.push(Block {
                        kind: "code",
                        indent: if self.in_item { self.indent() } else { 0 },
                        quote: self.quote.min(MAX_QUOTE),
                        inline: Inline::code_block(&code),
                        code,
                        ..Default::default()
                    });
                }
                _ => {}
            }
            return;
        }
        match event {
            Md::Start(Tag::Paragraph) => {
                // A task box opened the item's block before its paragraph.
                if !self
                    .current
                    .as_ref()
                    .is_some_and(|b| b.kind == "item" && b.inline.is_empty())
                {
                    self.flush();
                }
            }
            Md::End(TagEnd::Paragraph) => self.flush(),
            Md::Start(Tag::Heading { level, .. }) => {
                self.flush();
                self.heading = Some(level as i32);
            }
            Md::End(TagEnd::Heading(_)) => {
                self.flush();
                self.heading = None;
            }
            Md::Start(Tag::BlockQuote(_)) => {
                self.flush();
                self.quote += 1;
            }
            Md::End(TagEnd::BlockQuote(_)) => {
                self.flush();
                self.quote -= 1;
            }
            Md::Start(Tag::List(start)) => {
                self.flush();
                self.lists.push(start);
            }
            Md::End(TagEnd::List(_)) => {
                self.flush();
                self.lists.pop();
                self.in_item = !self.lists.is_empty();
            }
            Md::Start(Tag::Item) => {
                self.flush();
                self.in_item = true;
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}.");
                        *n += 1;
                        marker
                    }
                    _ => "•".to_owned(),
                };
                self.pending_marker = Some(marker);
            }
            Md::End(TagEnd::Item) => {
                self.flush();
                // An empty item still shows its marker.
                if let Some(marker) = self.pending_marker.take() {
                    self.current = Some(Block {
                        kind: "item",
                        indent: self.indent(),
                        quote: self.quote.min(MAX_QUOTE),
                        marker,
                        ..Default::default()
                    });
                    self.flush();
                }
            }
            Md::TaskListMarker(done) => {
                let task = if done { 2 } else { 1 };
                // The box replaces the bullet.
                self.pending_marker = None;
                let block = self.open();
                block.marker.clear();
                block.task = task;
            }
            Md::Start(Tag::CodeBlock(_)) => {
                self.flush();
                self.code = Some(String::new());
            }
            Md::Start(Tag::Emphasis) => {
                self.markup(&format!("<font color=\"{EMPHASIS_COLOR}\">*"));
            }
            Md::End(TagEnd::Emphasis) => self.markup("*</font>"),
            Md::Start(Tag::Strong) | Md::End(TagEnd::Strong) => self.markup("**"),
            Md::Start(Tag::Strikethrough) => {
                self.markup(&format!("<font color=\"{STRIKE_COLOR}\">~~"));
            }
            Md::End(TagEnd::Strikethrough) => self.markup("~~</font>"),
            Md::Start(Tag::Link { .. }) => self.markup("["),
            Md::End(TagEnd::Link) => {}
            Md::Start(Tag::Image { dest_url, .. }) => {
                self.flush();
                self.image = Some((resolve(self.base, &dest_url), String::new()));
            }
            Md::End(TagEnd::Image) => {
                if let Some((source, alt)) = self.image.take() {
                    self.images += 1;
                    self.push(Block {
                        kind: "image",
                        indent: if self.in_item { self.indent() } else { 0 },
                        quote: self.quote.min(MAX_QUOTE),
                        source: (self.images <= MAX_IMAGES).then_some(source),
                        alt,
                        ..Default::default()
                    });
                }
            }
            Md::Start(Tag::Table(alignments)) => {
                self.flush();
                self.table = Some(Vec::new());
                self.aligns = alignments
                    .iter()
                    .take(MAX_COLUMNS)
                    .map(|a| match a {
                        pulldown_cmark::Alignment::Left => "left",
                        pulldown_cmark::Alignment::Center => "center",
                        pulldown_cmark::Alignment::Right => "right",
                        pulldown_cmark::Alignment::None => "",
                    })
                    .collect();
            }
            Md::End(TagEnd::Table) => {
                let aligns = std::mem::take(&mut self.aligns);
                if let Some(mut rows) = self.table.take()
                    && !rows.is_empty()
                {
                    // Every row has one cell per column, so columns line up.
                    for (cells, _) in &mut rows {
                        cells.resize(aligns.len(), String::new());
                    }
                    self.push(Block {
                        kind: "table",
                        quote: self.quote.min(MAX_QUOTE),
                        rows,
                        aligns,
                        ..Default::default()
                    });
                }
            }
            Md::Start(Tag::TableHead) => {
                self.in_head = true;
                if let Some(rows) = self.table.as_mut() {
                    rows.push((Vec::new(), true));
                }
            }
            Md::End(TagEnd::TableHead) => self.in_head = false,
            Md::Start(Tag::TableRow) => {
                if let Some(rows) = self.table.as_mut()
                    && rows.len() < MAX_ROWS
                {
                    rows.push((Vec::new(), false));
                }
            }
            Md::Start(Tag::TableCell) => self.cell = Some(String::new()),
            Md::End(TagEnd::TableCell) => {
                if let Some(cell) = self.cell.take()
                    && let Some(rows) = self.table.as_mut()
                    && let Some((cells, _)) = rows.last_mut()
                    && cells.len() < MAX_COLUMNS
                {
                    cells.push(cell);
                }
            }
            Md::Text(text) => self.inline(|i| i.text(&text)),
            Md::Code(text) => {
                if self.cell.is_some() || self.image.is_some() {
                    self.inline(|i| i.text(&text));
                } else {
                    self.inline(|i| i.code(&text));
                }
            }
            Md::SoftBreak => self.inline(|i| i.text(" ")),
            Md::HardBreak => {
                if self.cell.is_none() && self.image.is_none() {
                    self.open().inline.format.push_str("  \n");
                    self.current.as_mut().unwrap().inline.plain.push('\n');
                }
            }
            Md::Rule => {
                self.flush();
                self.push(Block {
                    kind: "rule",
                    quote: self.quote.min(MAX_QUOTE),
                    ..Default::default()
                });
            }
            // Embedded HTML, footnotes, math and metadata are not shown.
            _ => {}
        }
    }
}

/// Parse a document into blocks (pure: no images loaded).
pub fn blocks(text: &str, base: Option<&str>) -> Vec<Block> {
    let mut text = text;
    let mut truncated = false;
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
        truncated = true;
    }
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        base,
        blocks: Vec::new(),
        current: None,
        quote: 0,
        lists: Vec::new(),
        pending_marker: None,
        in_item: false,
        heading: None,
        code: None,
        image: None,
        table: None,
        aligns: Vec::new(),
        cell: None,
        in_head: false,
        images: 0,
        truncated,
    };
    // Links need their destination at the end: rewrite the event stream so
    // `]( <dest> )` closes the bracket opened at the start.
    let mut links: Vec<String> = Vec::new();
    for event in Parser::new_ext(text, options) {
        match &event {
            Md::Start(Tag::Link { dest_url, .. }) => links.push(resolve(base, dest_url)),
            Md::End(TagEnd::Link) => {
                let url = links.pop().unwrap_or_default();
                builder.markup(&format!("]({})", destination(&url)));
            }
            _ => {}
        }
        builder.event(event);
    }
    builder.flush();
    if builder.truncated {
        let mut inline = Inline::default();
        inline.text(TRUNCATED);
        builder.blocks.push(Block {
            kind: "paragraph",
            inline,
            ..Default::default()
        });
    }
    builder.blocks
}

/// The remote (http(s)) images a document shows, at most `MAX_IMAGES`:
/// what the SDK downloads before sending the value (`ui.markdownScan`).
pub fn remote_images(text: &str, base: Option<&str>) -> Vec<String> {
    let mut found = Vec::new();
    for block in blocks(text, base) {
        if let Some(source) = block.source
            && (source.starts_with("https://") || source.starts_with("http://"))
            && !found.contains(&source)
        {
            found.push(source);
        }
    }
    found
}

impl Host {
    /// `ui.markdownScan { text, base }` → `{ images: [address…] }`.
    pub(super) fn markdown_scan(&self, params: &Value) -> std::result::Result<Value, RpcError> {
        let doc = parse_wire(params)?;
        Ok(json!({"images": remote_images(&doc.text, doc.base.as_deref())}))
    }

    /// The `[MarkdownBlock]` model for `owner`'s property.
    pub(super) fn markdown_value(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
    ) -> std::result::Result<SValue, RpcError> {
        let doc = parse_wire(value)?;
        let roots = self.image_roots(owner);
        let mut shown = Vec::new();
        let images = &mut self.images;
        let model = model(&doc, |path| {
            let (key, image) = images.load(owner, &roots, path).ok()?;
            shown.push(key);
            Some(image)
        });
        self.images.assign_many(owner, view, property, shown);
        Ok(model)
    }
}

/// The rows for `doc`; `load` turns a local image path into an image.
pub fn model(doc: &Doc, mut load: impl FnMut(&str) -> Option<slint::Image>) -> SValue {
    let list =
        |values: Vec<SValue>| SValue::Model(std::rc::Rc::new(slint::VecModel::from(values)).into());
    let mut rows = Vec::new();
    for block in blocks(&doc.text, doc.base.as_deref()) {
        let mut row = Struct::default();
        let mut set = |name: &str, value: SValue| row.set_field(name.into(), value);
        set("kind", SValue::String(block.kind.into()));
        set("level", SValue::Number(block.level.into()));
        set("indent", SValue::Number(block.indent.into()));
        set("quote", SValue::Number(block.quote.into()));
        set("marker", SValue::String(block.marker.as_str().into()));
        set("task", SValue::Number(block.task.into()));
        set(
            "aligns",
            list(
                block
                    .aligns
                    .iter()
                    .map(|a| SValue::String((*a).into()))
                    .collect(),
            ),
        );
        set(
            "text",
            SValue::from(block.inline.styled(block.kind == "heading")),
        );
        set("code", SValue::String(block.code.as_str().into()));
        set("alt", SValue::String(block.alt.as_str().into()));
        // Remote images come only from the SDK's downloads.
        let local = block.source.as_deref().and_then(|source| {
            if source.starts_with("http://") || source.starts_with("https://") {
                doc.images.get(source).cloned()
            } else if Path::new(source).is_absolute() {
                Some(source.to_owned())
            } else {
                None
            }
        });
        let image = local.and_then(|path| load(&path));
        let size = image.as_ref().map(slint::Image::size).unwrap_or_default();
        set("has-image", SValue::Bool(size.width > 0));
        set("image", SValue::Image(image.unwrap_or_default()));
        set("image-width", SValue::Number(size.width.into()));
        set("image-height", SValue::Number(size.height.into()));
        let table = block
            .rows
            .into_iter()
            .map(|(cells, header)| {
                let mut row = Struct::default();
                row.set_field(
                    "cells".into(),
                    list(
                        cells
                            .into_iter()
                            .map(|c| SValue::String(c.into()))
                            .collect(),
                    ),
                );
                row.set_field("header".into(), SValue::Bool(header));
                SValue::Struct(row)
            })
            .collect();
        set("rows", list(table));
        rows.push(SValue::Struct(row));
    }
    list(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<&'static str> {
        blocks(text, None).iter().map(|b| b.kind).collect()
    }

    #[test]
    fn block_kinds_cover_the_supported_subset() {
        let text = "# 标题\n\n段落 **粗** *斜* ~~删~~ `码` [链接](https://a.example/x)\n\n- 一\n- [x] 完成\n  1. 甲\n  2. 乙\n\n> 引用\n\n```\nlet a = 1;\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n![图](https://a.example/i.png)\n\n<div>html</div>\n";
        assert_eq!(
            kinds(text),
            [
                "heading",
                "paragraph",
                "item",
                "item",
                "item",
                "item",
                "paragraph",
                "code",
                "rule",
                "table",
                "image"
            ]
        );
        let all = blocks(text, None);
        assert_eq!(all[0].level, 1);
        assert_eq!(all[1].inline.plain, "段落 粗 斜 删 码 链接");
        assert_eq!(
            all[1].inline.format,
            "S**S**S<font color=\"#e8a33d\">*S*</font>S<font color=\"#8a8496\">~~S~~</font>S`S`S[S](<https://a.example/x>)"
                .replace('S', "\u{e541}")
        );
        assert_eq!((all[2].marker.as_str(), all[2].indent), ("•", 0));
        assert_eq!((all[3].marker.as_str(), all[3].task), ("", 2));
        assert_eq!((all[4].marker.as_str(), all[4].indent), ("1.", 1));
        assert_eq!(all[5].marker, "2.");
        assert_eq!(all[6].quote, 1);
        assert_eq!(all[7].code, "let a = 1;");
        assert_eq!(
            all[9].rows,
            vec![
                (vec!["a".to_owned(), "b".to_owned()], true),
                (vec!["1".to_owned(), "2".to_owned()], false)
            ]
        );
        assert_eq!(
            (all[10].source.as_deref(), all[10].alt.as_str()),
            (Some("https://a.example/i.png"), "图")
        );
        assert!(
            !all.iter().any(|b| b.inline.plain.contains("html")),
            "HTML dropped"
        );
    }

    /// The same text as literal Markdown (each placeholder replaced by its
    /// text), parsed through Slint's public API: equal values prove Slint
    /// reads the format as intended. Only for texts without Markdown syntax.
    fn literal(inline: &Inline) -> StyledText {
        let mut args = inline.args.iter();
        let text: String = inline
            .format
            .chars()
            .map(|c| match c {
                SLOT => args.next().unwrap().clone(),
                c => c.to_string(),
            })
            .collect();
        StyledText::from_markdown(&text).unwrap()
    }

    /// GUI review 2026-10-21: italic and strikethrough invisible with the
    /// (then forced) CJK font, the open task box a missing glyph, table header
    /// alignment off. Slint 1.18: code falls back to a CJK font itself.
    #[test]
    fn styles_survive_fonts_without_italic_and_cjk_code() {
        let all = blocks(
            "*斜体* *italic* ~~删除线~~ `行内代码` `assets/icon.png` `a 中 b`",
            None,
        );
        let inline = &all[0].inline;
        // Italic and strikethrough carry a colour; each code span is one
        // `Code` span (one capsule), CJK included.
        assert_eq!(
            inline.format,
            "<font color=\"#e8a33d\">*S*</font>S<font color=\"#e8a33d\">*S*</font>S<font color=\"#8a8496\">~~S~~</font>S`S`S`S`S`S`"
                .replace('S', "\u{e541}")
        );
        assert_eq!(
            inline.args,
            [
                "斜体",
                " ",
                "italic",
                " ",
                "删除线",
                " ",
                "行内代码",
                " ",
                "assets/icon.png",
                " ",
                "a 中 b"
            ]
        );
        let styled = inline.styled(false);
        assert_eq!(styled, literal(inline));
        assert_ne!(styled, StyledText::from_plain_text(&inline.plain));
        // Code blocks: plain lines (the view sets the monospace font), not
        // `Code` spans; indentation and empty lines kept.
        let code = &blocks("```\nfn main() {\n\n    // 中文注释\n}\n```", None)[0];
        assert_eq!(
            code.inline.format,
            "S  \nS  \nS  \nS".replace('S', "\u{e541}")
        );
        assert_eq!(
            code.inline.args,
            ["fn main() {", "\u{a0}", "    // 中文注释", "}"]
        );
        assert_ne!(
            code.inline.styled(false),
            StyledText::from_plain_text(&code.inline.plain),
            "hard breaks parsed"
        );
        // Empty inline code adds nothing.
        let mut empty = Inline::default();
        empty.code("");
        assert_eq!(empty, Inline::default());
        // Task items: a drawn box (task 1/2), no glyph marker; loose lists too.
        let tasks = blocks("- [ ] open\n- [x] done\n\n- [ ] loose\n\n  more\n", None);
        let tasks: Vec<_> = tasks
            .iter()
            .map(|b| (b.task, b.marker.as_str(), b.inline.plain.as_str()))
            .collect();
        assert_eq!(
            tasks,
            [
                (1, "", "open"),
                (2, "", "done"),
                (1, "", "loose"),
                (0, "", "more")
            ]
        );
        // Table columns follow the delimiter row; short rows are padded.
        let table = &blocks("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 |\n", None)[0];
        assert_eq!(table.aligns, ["left", "center", "right"]);
        assert_eq!(table.rows[1].0, ["1", "2", ""]);
    }

    #[test]
    fn user_text_never_reaches_the_format_string() {
        let block = &blocks("**[x](y)** \\*not\\* `a**b` \u{e541}", None)[0];
        assert!(
            block
                .inline
                .format
                .chars()
                .all(|c| "*[]()<>`~ \u{e541}y".contains(c)),
            "{}",
            block.inline.format
        );
        assert!(block.inline.plain.contains("*not*"));
        // Slint parses it; styled() never panics.
        let _ = block.inline.styled(false);
        let _ = blocks("[a](<x y>) [b](c\\)d)", None)[0].inline.styled(true);
    }

    #[test]
    fn links_and_images_resolve_against_the_base() {
        let base = Some("https://raw.example/owner/repo/main/README.md");
        let all = blocks(
            "[doc](docs/a.md) [abs](https://x.example/) [top](#usage)\n\n![s](img/s.png)",
            base,
        );
        assert!(
            all[0]
                .inline
                .format
                .contains("<https://raw.example/owner/repo/main/docs/a.md>")
        );
        assert!(all[0].inline.format.contains("<https://x.example/>"));
        assert!(all[0].inline.format.contains("<#usage>"));
        assert_eq!(
            all[1].source.as_deref(),
            Some("https://raw.example/owner/repo/main/img/s.png")
        );
        // A directory base is a path of this system (`/…` is not absolute
        // on Windows, where it stays relative like any other non-path base).
        let base = std::env::temp_dir().join("plugin");
        let base = base.to_str().unwrap();
        let local = blocks("![s](img/s.png)", Some(base));
        assert_eq!(
            local[0].source.as_deref(),
            Path::new(base).join("img/s.png").to_str()
        );
        let relative = blocks("![s](img/s.png)", Some("data/plugin"));
        assert_eq!(relative[0].source.as_deref(), Some("img/s.png"));
        assert_eq!(
            remote_images(
                "![a](https://x/a.png) ![b](https://x/a.png) ![c](c.png)",
                None
            ),
            ["https://x/a.png"]
        );
    }

    #[test]
    fn limits_truncate_instead_of_failing() {
        let long = "段落\n\n".repeat(MAX_BLOCKS + 10);
        let all = blocks(&long, None);
        assert_eq!(all.len(), MAX_BLOCKS);
        assert_eq!(all.last().unwrap().inline.plain, TRUNCATED);
        let huge = "x".repeat(MAX_TEXT + 100);
        assert_eq!(blocks(&huge, None).last().unwrap().inline.plain, TRUNCATED);
        let images = "![i](https://x/i.png)\n\n".repeat(MAX_IMAGES + 2);
        let all = blocks(&images, None);
        assert!(all[MAX_IMAGES - 1].source.is_some());
        assert!(all[MAX_IMAGES].source.is_none());
        let wide = format!("|{}\n|{}\n", "a|".repeat(20), "-|".repeat(20));
        assert_eq!(blocks(&wide, None)[0].rows[0].0.len(), MAX_COLUMNS);
    }

    #[test]
    fn wire_values_are_exact_and_bounded() {
        assert!(reference(&json!({"$vestraMarkdown":{"text":""},"ref":1})).is_some());
        assert!(reference(&json!({"$vestraMarkdown":{"text":""},"x":1})).is_none());
        assert!(parse_wire(&json!({"text":1})).is_err());
        assert!(parse_wire(&json!({"text":"x".repeat(MAX_TEXT + 1)})).is_err());
        assert!(parse_wire(&json!({"text":"","images":{"a":1}})).is_err());
        let doc = parse_wire(
            &json!({"text":"t","base":"https://a/","images":{"https://a/i.png":"/p/i.png"}}),
        )
        .unwrap();
        assert_eq!(doc.images["https://a/i.png"], "/p/i.png");
    }

    struct Headless(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<
            std::rc::Rc<dyn slint::platform::WindowAdapter>,
            slint::PlatformError,
        > {
            Ok(self.0.clone())
        }
    }

    /// M0 (store proposal §5.4): styled text and images in a model render
    /// through the interpreter, from a plugin-style file importing the library.
    #[test]
    fn markdown_view_renders_the_model() {
        let window =
            slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default());
        slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
        let source = r#"
import { MarkdownView, MarkdownBlock } from "@vestra/markdown.slint";
export component W inherits Window {
    width: 320px; height: 400px; background: white;
    in property <[MarkdownBlock]> doc;
    out property <string> clicked;
    MarkdownView { blocks: root.doc; link-clicked(url) => { root.clicked = url; } }
}"#;
        let dir = tempfile::tempdir().unwrap();
        let result = pollster::block_on(
            crate::ui::compiler().build_from_source(source.into(), dir.path().join("w.slint")),
        );
        let errors: Vec<_> = result
            .diagnostics()
            .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
            .map(|d| d.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let instance = result.component("W").unwrap().create().unwrap();
        let png = {
            let mut bytes = Vec::new();
            image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 0, 0, 255]))
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            bytes
        };
        let doc = Doc {
            text: "# 标题\n\n段落 **粗** [链接](https://a.example/)\n\n- 项\n\n```\ncode\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n![红](https://a.example/r.png)\n".into(),
            base: None,
            images: [("https://a.example/r.png".to_owned(), "/r.png".to_owned())].into(),
        };
        let mut asked = Vec::new();
        let value = model(&doc, |path| {
            asked.push(path.to_owned());
            Some(crate::ui_image::decode(&png).unwrap().0)
        });
        assert_eq!(asked, ["/r.png"]);
        instance.set_property("doc", value).unwrap();
        instance
            .window()
            .set_size(slint::PhysicalSize::new(320, 400));
        instance.show().unwrap();
        let mut buffer = vec![slint::platform::software_renderer::Rgb565Pixel(0); 320 * 400];
        window.draw_if_needed(|r| {
            r.render(&mut buffer, 320);
        });
        let red = buffer.iter().filter(|p| p.0 == 0xc800).count();
        assert!(red >= 40 * 20 / 2, "image drawn ({red} red pixels)");
        let inked = buffer
            .iter()
            .filter(|p| p.0 != 0xffff && p.0 != 0xc800)
            .count();
        assert!(inked > 500, "text drawn ({inked} pixels)");
        // A link reports its address.
        let link = Doc {
            text: "[链接链接链接链接链接](https://b.example/)".into(),
            ..Default::default()
        };
        instance
            .set_property("doc", model(&link, |_| None))
            .unwrap();
        window.draw_if_needed(|r| {
            r.render(&mut buffer, 320);
        });
        for event in [
            slint::platform::WindowEvent::PointerPressed {
                position: slint::LogicalPosition::new(30., 17.),
                button: slint::platform::PointerEventButton::Left,
            },
            slint::platform::WindowEvent::PointerReleased {
                position: slint::LogicalPosition::new(30., 17.),
                button: slint::platform::PointerEventButton::Left,
            },
        ] {
            instance.window().dispatch_event(event);
        }
        assert_eq!(
            instance.get_property("clicked").unwrap(),
            SValue::String("https://b.example/".into())
        );
        // The open task box is drawn with shapes (no font glyph needed).
        let task = Doc {
            text: "- [ ] 任务".into(),
            ..Default::default()
        };
        instance
            .set_property("doc", model(&task, |_| None))
            .unwrap();
        buffer.fill(slint::platform::software_renderer::Rgb565Pixel(0xffff));
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buffer, 320);
        });
        let boxed = (8..32)
            .flat_map(|y| (10..30).map(move |x| y * 320 + x))
            .filter(|&i| buffer[i].0 != 0xffff)
            .count();
        assert!(boxed >= 20, "task box drawn ({boxed} pixels)");
    }
}
