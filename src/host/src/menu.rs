//! Projection of the shared command tree into the compact cascading context menu.
//! Pure data and geometry; the Slint view in ui/menu-view.slint only draws it.

use crate::commands::{CommandSnapshot, MenuNode};
use serde_json::{Value, json};

/// Must match MenuItem height, separator height and panel padding in ui/menu-view.slint.
const ROW: f64 = 24.;
const SEPARATOR: f64 = 7.;
const PAD: f64 = 2.;
const MIN_WIDTH: f64 = 168.;
const MAX_WIDTH: f64 = 420.;
/// Neighbouring panels overlap slightly so the pointer can cross between them.
const OVERLAP: f64 = 2.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Token for a row; parsed back by `parse_token`.
pub enum Token {
    Group(Vec<String>),
    Command { owner: u64, id: String },
}

pub fn parse_token(token: &str) -> Option<Token> {
    if let Some(group) = token.strip_prefix("g:") {
        return serde_json::from_str(group).ok().map(Token::Group);
    }
    let (owner, id) = token.strip_prefix("c:")?.split_once(':')?;
    Some(Token::Command {
        owner: owner.parse().ok()?,
        id: id.to_owned(),
    })
}

fn children<'a>(nodes: &'a [MenuNode], title: &str) -> Option<&'a [MenuNode]> {
    nodes.iter().find_map(|node| match node {
        MenuNode::Group {
            title: t, children, ..
        } if t == title => Some(children.as_slice()),
        _ => None,
    })
}

/// Drops path segments that no longer name a group (the registry changed).
pub fn valid_path(roots: &[MenuNode], path: &[String]) -> Vec<String> {
    let mut nodes = roots;
    let mut valid = Vec::new();
    for title in path {
        match children(nodes, title) {
            Some(next) => {
                nodes = next;
                valid.push(title.clone());
            }
            None => break,
        }
    }
    valid
}

/// Rough text advance at the 12px menu font: CJK is about one em, Latin about half.
fn text_width(text: &str) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 7. } else { 12.5 })
        .sum()
}

/// Shortcut text for a command row (from the binding table).
pub type Labels<'a> = &'a dyn Fn(u64, &str) -> Option<String>;

fn command_row(entry: &CommandSnapshot, labels: Labels<'_>) -> Value {
    json!({"token":format!("c:{}:{}",entry.key.owner,entry.key.local_id),
        "title":entry.declaration.title,"enabled":entry.state.enabled,
        "checked":entry.state.checked.unwrap_or(false),"radio":entry.declaration.radio,
        "busy":entry.state.busy,"reason":entry.state.reason.clone().unwrap_or_default(),
        "has-children":false,"open":false,
        "shortcut":labels(entry.key.owner,&entry.key.local_id).unwrap_or_default()})
}

fn row(node: &MenuNode, path: &[String], open: bool, labels: Labels<'_>) -> Value {
    match node {
        MenuNode::Group { title, .. } => {
            let mut next = path.to_vec();
            next.push(title.clone());
            json!({"token":format!("g:{}",json!(next)),"title":title,"enabled":true,"checked":false,
                "radio":false,"busy":false,"reason":"","has-children":true,"open":open,"shortcut":""})
        }
        MenuNode::Command { entry } => command_row(entry, labels),
    }
}

/// Lay out one panel's rows: separator flags, row tops and panel size.
fn panel(mut rows: Vec<Value>, separators: Vec<bool>) -> (Vec<Value>, f64, f64) {
    let mut y = PAD;
    let mut widest: f64 = 0.;
    for (row, separator) in rows.iter_mut().zip(separators) {
        if separator {
            y += SEPARATOR;
        }
        row["separator"] = separator.into();
        row["y"] = y.into();
        y += ROW;
        let title = text_width(row["title"].as_str().unwrap_or_default());
        let shortcut = text_width(row["shortcut"].as_str().unwrap_or_default());
        widest = widest.max(title + if shortcut > 0. { 24. + shortcut } else { 0. });
    }
    let width = (widest + 22. + 4. + 14. + 2. * PAD).clamp(MIN_WIDTH, MAX_WIDTH);
    let height = if rows.is_empty() { ROW } else { y - PAD } + 2. * PAD;
    (rows, width, height)
}

fn place(origin: Point, width: f64, height: f64, window: Point) -> (f64, f64) {
    let x = if origin.x + width > window.x {
        origin.x - width
    } else {
        origin.x
    };
    let y = if origin.y + height > window.y {
        origin.y - height
    } else {
        origin.y
    };
    (
        x.min(window.x - width).max(0.),
        y.min(window.y - height).max(0.),
    )
}

/// One panel per open level, root first. `path` must come from `valid_path`.
/// Panels open at `origin`, cascade to the right of their open row, flip to
/// the left at the window edge and are clamped inside `window`.
pub fn levels(
    roots: &[MenuNode],
    path: &[String],
    origin: Point,
    window: Point,
    labels: Labels<'_>,
) -> Value {
    let mut output = Vec::new();
    let mut nodes = roots;
    let mut previous: Option<(f64, f64, f64, f64)> = None; // x, y, width, open row top
    for depth in 0..=path.len() {
        let open_title = path.get(depth);
        let open = nodes.iter().position(
            |node| matches!(node, MenuNode::Group { title, .. } if Some(title) == open_title),
        );
        let rows: Vec<Value> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| row(node, &path[..depth], Some(index) == open, labels))
            .collect();
        let separators = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| index > 0 && nodes[index - 1].group() != node.group())
            .collect();
        let (rows, width, height) = panel(rows, separators);
        let (x, y) = match previous {
            None => place(origin, width, height, window),
            Some((px, py, pw, row_y)) => {
                let right = px + pw - OVERLAP;
                let x = if right + width > window.x {
                    px - width + OVERLAP
                } else {
                    right
                };
                let y = py + row_y - PAD;
                (
                    x.min(window.x - width).max(0.),
                    y.min(window.y - height).max(0.),
                )
            }
        };
        let open_y = open.and_then(|index| rows[index]["y"].as_f64());
        output.push(
            json!({"rows":rows,"x":x,"y":y,"width":width,"height":height,
            "open":open.map_or(-1, |index| index as i64)}),
        );
        let (Some(open), Some(open_y)) = (open, open_y) else {
            break;
        };
        previous = Some((x, y, width, open_y));
        nodes = match &nodes[open] {
            MenuNode::Group { children, .. } => children,
            MenuNode::Command { .. } => break,
        };
    }
    Value::Array(output)
}

/// A one-level panel menu (C11): commands in the given order, `None` = separator.
pub fn context_level(
    items: &[Option<&CommandSnapshot>],
    origin: Point,
    window: Point,
    labels: Labels<'_>,
) -> Value {
    let mut rows = Vec::new();
    let mut separators = Vec::new();
    let mut pending = false;
    for item in items {
        match item {
            None => pending = !rows.is_empty(),
            Some(entry) => {
                rows.push(command_row(entry, labels));
                separators.push(pending);
                pending = false;
            }
        }
    }
    let (rows, width, height) = panel(rows, separators);
    let (x, y) = place(origin, width, height, window);
    json!([{"rows":rows,"x":x,"y":y,"width":width,"height":height,"open":-1}])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandKey, CommandState, Declaration};

    fn group(title: &str, children: Vec<MenuNode>) -> MenuNode {
        MenuNode::Group {
            title: title.into(),
            group: "domain".into(),
            children,
        }
    }
    fn command(id: &str, group: &str) -> CommandSnapshot {
        let mut declaration = Declaration::new(id, &[id]);
        declaration.group = group.into();
        CommandSnapshot {
            key: CommandKey {
                owner: 3,
                local_id: id.into(),
            },
            revision: 1,
            declaration,
            state: CommandState {
                enabled: true,
                checked: Some(true),
                busy: false,
                reason: None,
            },
            manifest: "m".into(),
            sequence: 1,
        }
    }
    fn tree() -> Vec<MenuNode> {
        vec![
            group(
                "文件",
                vec![group("播放列表", vec![]), group("网络", vec![])],
            ),
            group("播放", vec![]),
        ]
    }
    const WINDOW: Point = Point { x: 1000., y: 700. };
    fn none(_: u64, _: &str) -> Option<String> {
        None
    }

    #[test]
    fn open_levels_cascade_beside_their_open_row() {
        let path = vec!["文件".to_owned(), "网络".to_owned()];
        let value = levels(&tree(), &path, Point { x: 100., y: 50. }, WINDOW, &none);
        let levels = value.as_array().unwrap();
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0]["open"], 0);
        assert_eq!(levels[1]["open"], 1);
        assert_eq!(levels[2]["open"], -1);
        assert_eq!(levels[0]["x"], 100.);
        assert_eq!(levels[0]["y"], 50.);
        let right = levels[0]["x"].as_f64().unwrap() + levels[0]["width"].as_f64().unwrap();
        assert_eq!(levels[1]["x"].as_f64().unwrap(), right - OVERLAP);
        assert_eq!(levels[2]["y"].as_f64().unwrap(), 50. + ROW);
        assert_eq!(levels[1]["rows"][1]["open"], true);
        assert_eq!(levels[1]["rows"][0]["open"], false);
    }

    #[test]
    fn panels_flip_and_clamp_at_window_edges() {
        let path = vec!["文件".to_owned()];
        let origin = Point { x: 990., y: 690. };
        let value = levels(&tree(), &path, origin, WINDOW, &none);
        let levels = value.as_array().unwrap();
        for level in levels {
            let (x, y) = (level["x"].as_f64().unwrap(), level["y"].as_f64().unwrap());
            assert!(x >= 0. && x + level["width"].as_f64().unwrap() <= WINDOW.x);
            assert!(y >= 0. && y + level["height"].as_f64().unwrap() <= WINDOW.y);
        }
        assert!(levels[1]["x"].as_f64().unwrap() < levels[0]["x"].as_f64().unwrap());
    }

    #[test]
    fn stale_paths_are_truncated_and_tokens_round_trip() {
        let path = vec!["文件".to_owned(), "gone".to_owned(), "网络".to_owned()];
        assert_eq!(valid_path(&tree(), &path), vec!["文件".to_owned()]);
        let value = levels(
            &tree(),
            &valid_path(&tree(), &path),
            Point { x: 0., y: 0. },
            WINDOW,
            &none,
        );
        let token = value[1]["rows"][1]["token"].as_str().unwrap().to_owned();
        match parse_token(&token) {
            Some(Token::Group(path)) => assert_eq!(path, vec!["文件", "网络"]),
            _ => panic!("group token did not round-trip"),
        }
        match parse_token("c:7:open") {
            Some(Token::Command { owner, id }) => assert_eq!((owner, id.as_str()), (7, "open")),
            _ => panic!("command token did not parse"),
        }
    }

    #[test]
    fn group_changes_insert_separators_and_shortcuts_come_from_labels() {
        let nodes = vec![
            MenuNode::Command {
                entry: command("a", "open"),
            },
            MenuNode::Command {
                entry: command("b", "open"),
            },
            MenuNode::Command {
                entry: command("c", "screen"),
            },
        ];
        let labels = |_: u64, id: &str| (id == "c").then(|| "Enter".to_owned());
        let value = levels(&nodes, &[], Point { x: 0., y: 0. }, WINDOW, &labels);
        let rows = value[0]["rows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["separator"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(rows[2]["y"].as_f64().unwrap(), PAD + 2. * ROW + SEPARATOR);
        assert_eq!(rows[2]["shortcut"], "Enter");
        assert_eq!(
            value[0]["height"].as_f64().unwrap(),
            3. * ROW + SEPARATOR + 2. * PAD
        );
        let (a, b) = (command("a", "x"), command("b", "x"));
        let context = context_level(
            &[None, Some(&a), None, None, Some(&b), None],
            Point { x: 5., y: 5. },
            WINDOW,
            &labels,
        );
        let rows = context[0]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["separator"], false);
        assert_eq!(rows[1]["separator"], true);
    }
}
