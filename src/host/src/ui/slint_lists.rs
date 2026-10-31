//! Long lists use Slint's `ListView` (host 0.5.26, user rule): a `for` inside
//! a `ScrollView` or `Flickable` (directly or through a layout) builds every
//! row up front and keeps all of them, so a long list costs memory, startup
//! and frame time even on CPU rendering. `ListView` builds only the rows in
//! view.
//!
//! The test scans the host's Slint files (`host/ui`), the built-in plugins'
//! (`plugins/*`, without `dist`, `vendor` and `node_modules`) and the Slint
//! the host generates (shell, floating window, Options window). A place not
//! migrated yet, or a small fixed set where a `ListView` buys nothing, is in
//! [`ALLOWED`] with a reason and its count: lower the count (or remove the
//! entry) when a place is migrated; the test fails while an entry allows
//! more than it finds, so the list only shrinks.

use std::path::{Path, PathBuf};

/// (file under `src/`, or `generated:<name>`; `for`s allowed; why).
const ALLOWED: &[(&str, usize, &str)] = &[
    (
        "plugins/audio/ui/workbench.slint",
        1,
        "audio workbench popup: its author migrates it",
    ),
    (
        "plugins/capture/ui/tasks.slint",
        1,
        "capture tasks: its author migrates it",
    ),
    (
        "plugins/info/ui/about.slint",
        1,
        "info about: its author migrates it",
    ),
    (
        "plugins/info/ui/properties.slint",
        1,
        "info properties: its author migrates it",
    ),
    (
        "plugins/playlist/ui/sidebar.slint",
        1,
        "playlist sidebar: its author migrates it",
    ),
    (
        "plugins/video/ui/shaders.slint",
        1,
        "video shaders / picture popup: its author migrates it",
    ),
];

/// A `for` inside a scrolling element: its line and the scrolling element.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub line: usize,
    pub scroll: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Frame {
    /// An element (its type name), or a component (the type it inherits).
    Element(String),
    /// A callback, function or `changed` body: code, not elements.
    Code,
    /// A struct, global, enum, `states`/`transitions` entry, binding braces.
    Other,
    /// A component boundary: scrolling elements outside it do not count.
    Component(String),
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
        if at + 1 < words.len() {
            return (Frame::Component(base.to_owned()), false);
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

/// Every `for` in `source` with a scrolling element around it (in the
/// same component).
pub(crate) fn findings(source: &str) -> Vec<Finding> {
    let text = strip(source);
    let mut stack: Vec<Frame> = Vec::new();
    let mut header = String::new();
    let mut header_line = 1;
    let mut line = 1;
    let mut found = Vec::new();
    for c in text.chars() {
        match c {
            '{' => {
                let inside_code = matches!(stack.last(), Some(Frame::Code));
                let (frame, repeated) = classify(&header, inside_code);
                if repeated {
                    let scroll = stack
                        .iter()
                        .rev()
                        .take_while(|f| !matches!(f, Frame::Component(_)))
                        .chain(
                            stack
                                .iter()
                                .rev()
                                .find(|f| matches!(f, Frame::Component(_))),
                        )
                        .find_map(|f| match f {
                            Frame::Element(name) | Frame::Component(name) if scrolls(name) => {
                                Some(name.clone())
                            }
                            _ => None,
                        });
                    if let Some(scroll) = scroll {
                        found.push(Finding {
                            line: header_line,
                            scroll,
                        });
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
    let mut seen = std::collections::BTreeMap::new();
    for (name, source) in sources() {
        let found = findings(&source);
        let allowed = ALLOWED
            .iter()
            .find(|(file, ..)| *file == name)
            .map_or(0, |(_, count, _)| *count);
        if found.len() > allowed {
            let lines: Vec<String> = found
                .iter()
                .map(|f| format!("line {} (in a {})", f.line, f.scroll))
                .collect();
            problems.push(format!(
                "{name}: `for` inside a scrolling element at {}; use a `ListView` (or, for a \
                 small fixed set, add it to slint_lists::ALLOWED with the reason)",
                lines.join(", ")
            ));
        }
        seen.insert(name, found.len());
    }
    for (file, count, _) in ALLOWED {
        match seen.get(*file) {
            None => problems.push(format!(
                "ALLOWED names {file}, which is not scanned: remove it"
            )),
            Some(found) if found < count => problems.push(format!(
                "{file}: ALLOWED allows {count}, {found} found: lower it to {found}"
            )),
            _ => {}
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
    assert_eq!(
        found,
        [
            Finding {
                line: 5,
                scroll: "ScrollView".into()
            },
            Finding {
                line: 13,
                scroll: "ScrollView".into()
            },
        ]
    );
}
