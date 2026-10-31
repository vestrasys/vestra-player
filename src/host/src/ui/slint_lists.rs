//! Long lists use Slint's `ListView` (host 0.5.26, user rule): a `for` inside
//! a `ScrollView` or `Flickable` (directly or through a layout) builds every
//! row up front and keeps all of them, so a long list costs memory, startup
//! and frame time even on CPU rendering. `ListView` builds only the rows in
//! view.
//!
//! The test scans the host's Slint files (`host/ui`), the built-in plugins'
//! (`plugins/*`, without `dist`, `vendor` and `node_modules`) and the Slint
//! the host generates (shell, floating window, Options window). It finds:
//!
//! - a `for` with a `ScrollView` or `Flickable` around it in the same
//!   component (or a component that inherits one);
//! - an element whose type is a component of the same file that holds such
//!   a `for` outside any scrolling element of its own (directly, through
//!   another such component, or through the component it inherits), placed
//!   in a `ScrollView` or `Flickable`: `ScrollView { Rows {} }`.
//!
//! A `PopupWindow` or `ListView` between the `for` and the scrolling element
//! ends the search: a popup is not part of the scrolled content, and a
//! `ListView`'s rows are virtualized.
//!
//! Not found (a text scan, not the Slint compiler):
//! - components imported from another file (also under an alias), so
//!   `ScrollView { MarkdownBlocks {} }` in a plugin that imports it;
//! - a `for` that reaches a scrolling element through `@children` (a wrapper
//!   component whose `@children` sit in its `ScrollView`, filled by the user
//!   of the wrapper);
//! - lists built in Rust code rather than in Slint.
//!
//! A place not migrated yet, or a small fixed set where a `ListView` buys
//! nothing, is in [`ALLOWED`] with its line's text and a reason. The test
//! fails on a finding no entry names and on an entry that names nothing, so
//! moving or replacing an allowed `for` needs a new entry, not just the same
//! count.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// (file under `src/`, or `generated:<name>`; the found line's text, trimmed,
/// or a part of it that names it in that file; why).
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "plugins/playlist/ui/sidebar.slint",
        "for row[index] in PlaylistBridge.rows: Rectangle {",
        "playlist sidebar: moves to a ListView with the whole list in the host (large lists P3)",
    ),
    (
        "plugins/video/ui/shaders.slint",
        "for name[index] in root.model: Rectangle {",
        "video shaders: the built-in video plugin is left unchanged for now (user ruling 2026-10-31)",
    ),
];

/// A `for` inside a scrolling element, or a local component holding one
/// placed in a scrolling element: its line, the line's text and the
/// scrolling element.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub line: usize,
    pub text: String,
    pub scroll: String,
    /// The local component the `for` is in, for a placed component.
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Frame {
    /// An element (its type name).
    Element(String),
    /// A callback, function or `changed` body: code, not elements.
    Code,
    /// A struct, global, enum, `states`/`transitions` entry, binding braces.
    Other,
    /// A component boundary (its name, the type it inherits): scrolling
    /// elements outside it do not count.
    Component(String, String),
}

/// Comments and string contents blanked (newlines kept, for line numbers).
fn strip(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '"' {
            out.push('"');
            i += 1;
            // String interpolation `\{…}` holds no elements: blank it too.
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                if chars.get(i) == Some(&'\n') {
                    out.push('\n');
                }
                i += 1;
            }
            out.push('"');
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_ident(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// What the text before a `{` opens.
fn classify(header: &str, inside_code: bool) -> (Frame, bool) {
    if inside_code {
        return (Frame::Code, false);
    }
    let header = header.trim().trim_start_matches(']').trim();
    if header.ends_with("=>") || header.starts_with("function ") || header.contains(" function ") {
        return (Frame::Code, false);
    }
    let words: Vec<&str> = header.split_whitespace().collect();
    if let Some(at) = words.iter().position(|w| *w == "component") {
        let base = words
            .iter()
            .position(|w| *w == "inherits")
            .and_then(|i| words.get(i + 1))
            .copied()
            .unwrap_or("component");
        if let Some(name) = words.get(at + 1) {
            return (Frame::Component((*name).to_owned(), base.to_owned()), false);
        }
    }
    if words
        .first()
        .is_some_and(|w| matches!(*w, "struct" | "global" | "enum" | "export"))
    {
        return (Frame::Other, false);
    }
    // `for x[i] in model: [id :=] Type`, `if cond: [id :=] Type`,
    // `[id :=] Type`.
    let repeated = words.first() == Some(&"for");
    let conditional = words.first() == Some(&"if");
    let element = if repeated || conditional {
        header.rsplit_once(':').map(|(_, rest)| rest)
    } else {
        Some(header)
    };
    let Some(element) = element else {
        return (Frame::Other, false);
    };
    let element = element.rsplit("=").next().unwrap_or("").trim();
    if is_ident(element) && element.chars().next().is_some_and(char::is_uppercase) {
        (Frame::Element(element.to_owned()), repeated)
    } else {
        (Frame::Other, false)
    }
}

fn scrolls(name: &str) -> bool {
    matches!(name, "ScrollView" | "Flickable")
}

/// Ends the search for a scrolling element: a popup is not the scrolled
/// content, a `ListView`'s rows are virtualized.
fn stops(name: &str) -> bool {
    matches!(name, "PopupWindow" | "ListView")
}

/// The scrolling element around the top of `stack` in its component (the
/// component itself when it inherits one), if any.
fn scroll_around(stack: &[Frame]) -> Option<String> {
    for frame in stack.iter().rev() {
        match frame {
            Frame::Element(name) if stops(name) => return None,
            Frame::Element(name) if scrolls(name) => return Some(name.clone()),
            Frame::Component(_, base) => return scrolls(base).then(|| base.clone()),
            _ => {}
        }
    }
    None
}

/// The component the top of `stack` is in.
fn component_of(stack: &[Frame]) -> Option<&str> {
    stack.iter().rev().find_map(|frame| match frame {
        Frame::Component(name, _) => Some(name.as_str()),
        _ => None,
    })
}

/// What one component holds, for the second pass.
#[derive(Default)]
struct Component {
    /// A `for` outside any scrolling element of the component's own.
    list: bool,
    /// Element types placed outside any scrolling element (and the type it
    /// inherits): a list in one of them is a list here.
    uses: BTreeSet<String>,
    /// (line, element type, scrolling element) of elements placed in one.
    scrolled: Vec<(usize, String, String)>,
}

/// Every `for` in `source` with a scrolling element around it (in the
/// same component), and every local component holding an unscrolled `for`
/// placed in a scrolling element.
pub(crate) fn findings(source: &str) -> Vec<Finding> {
    let text = strip(source);
    let lines: Vec<&str> = source.lines().collect();
    let line_text = |line: usize| lines.get(line - 1).map_or("", |l| l.trim()).to_owned();
    let mut stack: Vec<Frame> = Vec::new();
    let mut components: BTreeMap<String, Component> = BTreeMap::new();
    let mut header = String::new();
    let mut header_line = 1;
    let mut line = 1;
    let mut found = Vec::new();
    for c in text.chars() {
        match c {
            '{' => {
                let inside_code = matches!(stack.last(), Some(Frame::Code));
                let (frame, repeated) = classify(&header, inside_code);
                let scroll = scroll_around(&stack);
                let owner = component_of(&stack).map(str::to_owned);
                if let Frame::Component(name, base) = &frame {
                    components
                        .entry(name.clone())
                        .or_default()
                        .uses
                        .insert(base.clone());
                }
                if let (Some(owner), Frame::Element(kind)) = (&owner, &frame) {
                    let component = components.entry(owner.clone()).or_default();
                    match &scroll {
                        Some(scroll) => {
                            component
                                .scrolled
                                .push((header_line, kind.clone(), scroll.clone()))
                        }
                        // In a popup or a list view a use is not this
                        // component's list either.
                        None if !stack
                            .iter()
                            .rev()
                            .any(|f| matches!(f, Frame::Element(name) if stops(name))) =>
                        {
                            component.uses.insert(kind.clone());
                        }
                        None => {}
                    }
                }
                if repeated {
                    match &scroll {
                        Some(scroll) => found.push(Finding {
                            line: header_line,
                            text: line_text(header_line),
                            scroll: scroll.clone(),
                            via: None,
                        }),
                        None => {
                            let open = !stack.iter().any(|f| match f {
                                Frame::Element(name) | Frame::Component(_, name) => stops(name),
                                _ => false,
                            });
                            if let (Some(owner), true) = (&owner, open) {
                                components.entry(owner.clone()).or_default().list = true;
                            }
                        }
                    }
                }
                stack.push(frame);
                header.clear();
            }
            '}' => {
                stack.pop();
                header.clear();
            }
            ';' => header.clear(),
            _ => {
                if header.trim().is_empty() && !c.is_whitespace() {
                    header_line = line;
                }
                header.push(c);
            }
        }
        if c == '\n' {
            line += 1;
        }
    }
    // Which local components hold a list, through the ones they use.
    let mut lists: BTreeSet<String> = components
        .iter()
        .filter(|(_, c)| c.list)
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let more: Vec<String> = components
            .iter()
            .filter(|(name, c)| !lists.contains(*name) && c.uses.iter().any(|u| lists.contains(u)))
            .map(|(name, _)| name.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        lists.extend(more);
    }
    for component in components.values() {
        for (line, kind, scroll) in &component.scrolled {
            if lists.contains(kind) {
                found.push(Finding {
                    line: *line,
                    text: line_text(*line),
                    scroll: scroll.clone(),
                    via: Some(kind.clone()),
                });
            }
        }
    }
    found.sort_by_key(|f| f.line);
    found
}

/// `.slint` files under `dir`, without build output and dependencies.
fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "dist" | "vendor" | "node_modules" | "target") {
                slint_files(&path, out);
            }
        } else if name.ends_with(".slint") {
            out.push(path);
        }
    }
}

/// (name, source) of every scanned file and generated source.
fn sources() -> Vec<(String, String)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut files = Vec::new();
    slint_files(&src.join("host/ui"), &mut files);
    slint_files(&src.join("plugins"), &mut files);
    let mut sources: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            let name = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, std::fs::read_to_string(path).unwrap())
        })
        .collect();
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
    let specs = [
        crate::ui::composition::CompileSpec {
            owner: 10,
            file: base.join("bar.slint"),
            component: "SourceBar".into(),
            global: Some("Bridge".into()),
            slot: crate::ui::composition::Slot::Bottom,
        },
        crate::ui::composition::CompileSpec {
            owner: 20,
            file: base.join("sidebar.slint"),
            component: "RowsSidebar".into(),
            global: Some("Bridge".into()),
            slot: crate::ui::composition::Slot::Sidebar,
        },
    ];
    let page = crate::ui::settings_window::Page {
        owner: 1,
        manifest: "a.plugin".into(),
        id: "look".into(),
        category: 6,
        title: "look".into(),
        order: 0,
        serial: 1,
        file: base.join("backdrop.slint"),
        component: "Backdrop".into(),
        global: "Bridge".into(),
        properties: Default::default(),
    };
    sources.push((
        "generated:shell".into(),
        crate::ui::composition::shell_source(&specs).unwrap(),
    ));
    sources.push((
        "generated:floating".into(),
        crate::ui::composition::floating_source(&specs[1..], "列表").unwrap(),
    ));
    sources.push((
        "generated:options".into(),
        crate::ui::settings_window::source(&[&page]).unwrap(),
    ));
    sources
}

#[test]
fn long_lists_use_list_view() {
    let mut problems = Vec::new();
    let mut used = BTreeSet::new();
    let mut scanned = BTreeSet::new();
    for (name, source) in sources() {
        for finding in findings(&source) {
            // Each entry allows one finding: a second identical list is reported.
            let entry = ALLOWED
                .iter()
                .enumerate()
                .position(|(index, (file, text, _))| {
                    *file == name && finding.text.contains(text) && !used.contains(&index)
                });
            match entry {
                Some(entry) => {
                    used.insert(entry);
                }
                None => problems.push(format!(
                    "{name}:{}: `{}` is a list inside a {}{}; use a `ListView` (or, for a small \
                     fixed set, add it to slint_lists::ALLOWED with the reason)",
                    finding.line,
                    finding.text,
                    finding.scroll,
                    finding
                        .via
                        .map(|via| format!(" (through the `for` in {via})"))
                        .unwrap_or_default()
                )),
            }
        }
        scanned.insert(name);
    }
    for (index, (file, text, _)) in ALLOWED.iter().enumerate() {
        if !scanned.contains(*file) {
            problems.push(format!(
                "ALLOWED names {file}, which is not scanned: remove it"
            ));
        } else if !used.contains(&index) {
            problems.push(format!(
                "ALLOWED names `{text}` in {file}, which is not found there: remove the entry"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_scanner_finds_lists_in_scrolling_elements_only() {
    let source = r#"
import { ScrollView, ListView } from "std-widgets.slint";
// for x in y: Rectangle { } in a comment
component A inherits ScrollView {
    for x in [1, 2]: Rectangle { }
}
export component W inherits Window {
    callback c();
    c => { if true { } }
    ScrollView {
        VerticalLayout {
            Text { text: "for x in y: Rectangle {"; }
            for row[i] in root.rows: Rectangle {
                clicked => { root.c(); }
            }
            if root.x: Rectangle { }
        }
    }
    ListView {
        for row in root.rows: Rectangle {
            for cell in row.cells: Text { }
        }
    }
    Rectangle { for dot in 3: Rectangle { } }
}
"#;
    let found = findings(source);
    let lines: Vec<(usize, &str)> = found.iter().map(|f| (f.line, f.scroll.as_str())).collect();
    assert_eq!(lines, [(5, "ScrollView"), (13, "ScrollView")]);
    assert_eq!(found[1].text, "for row[i] in root.rows: Rectangle {");
}

#[test]
fn the_scanner_follows_local_components_and_skips_popups() {
    let source = r#"
component Rows {
    VerticalLayout {
        for row in root.rows: Text { }
    }
}
component Wrapped inherits Rows { }
component Uses {
    Rows { }
}
component Listed {
    ListView {
        for row in root.rows: Text { }
    }
}
component Scrolls {
    ScrollView {
        for row in root.rows: Text { }
    }
}
export component W inherits Window {
    ScrollView {
        VerticalLayout {
            Rows { }
            Uses { }
            Wrapped { }
            Listed { }
            Scrolls { }
            Text { }
            popup := PopupWindow {
                for item in root.items: Text { }
                Rows { }
            }
        }
    }
}
"#;
    let found: Vec<(usize, String, Option<String>)> = findings(source)
        .into_iter()
        .map(|f| (f.line, f.scroll, f.via))
        .collect();
    let scroll = || "ScrollView".to_owned();
    assert_eq!(
        found,
        [
            // The `for` in Scrolls, inside its own ScrollView.
            (18, scroll(), None),
            (24, scroll(), Some("Rows".into())),
            (25, scroll(), Some("Uses".into())),
            (26, scroll(), Some("Wrapped".into())),
        ]
    );
}
