//! Projection of the shared command tree into the compact cascading context menu.
//! Pure data and geometry; the Slint view in ui/menu-view.slint only draws it.

use crate::commands::MenuNode;
use serde_json::{Value, json};

/// Must match MenuItem height and panel padding in ui/menu-view.slint.
const ROW: f64 = 24.;
const PAD: f64 = 4.;
const MIN_WIDTH: f64 = 168.;
const MAX_WIDTH: f64 = 360.;
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
        MenuNode::Group { title: t, children } if t == title => Some(children.as_slice()),
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

/// Rough text advance at the 13px menu font: CJK is about one em, Latin about half.
fn text_width(text: &str) -> f64 {
    text.chars()
        .map(|c| if c.is_ascii() { 7.5 } else { 13.5 })
        .sum()
}

fn row(node: &MenuNode, path: &[String], open: bool) -> Value {
    match node {
        MenuNode::Group { title, .. } => {
            let mut next = path.to_vec();
            next.push(title.clone());
            json!({"token":format!("g:{}",json!(next)),"title":title,"enabled":true,"checked":false,
                "busy":false,"reason":"","has-children":true,"open":open})
        }
        MenuNode::Command { entry } => {
            json!({"token":format!("c:{}:{}",entry.key.owner,entry.key.local_id),
                "title":entry.declaration.title,"enabled":entry.state.enabled,
                "checked":entry.state.checked.unwrap_or(false),"busy":entry.state.busy,
                "reason":entry.state.reason.clone().unwrap_or_default(),"has-children":false,"open":false})
        }
    }
}

fn title(node: &MenuNode) -> &str {
    match node {
        MenuNode::Group { title, .. } => title,
        MenuNode::Command { entry } => &entry.declaration.title,
    }
}

/// One panel per open level, root first. `path` must come from `valid_path`.
/// Panels open at `origin`, cascade to the right of their open row, flip to
/// the left at the window edge and are clamped inside `window`.
pub fn levels(roots: &[MenuNode], path: &[String], origin: Point, window: Point) -> Value {
    let mut output = Vec::new();
    let mut nodes = roots;
    let mut previous: Option<(f64, f64, f64, usize)> = None; // x, y, width, open row
    for depth in 0..=path.len() {
        let open_title = path.get(depth);
        let open = nodes.iter().position(
            |node| matches!(node, MenuNode::Group { title, .. } if Some(title) == open_title),
        );
        let rows: Vec<Value> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| row(node, &path[..depth], Some(index) == open))
            .collect();
        let widest = nodes
            .iter()
            .map(|node| text_width(title(node)))
            .fold(0., f64::max);
        let width = (widest + 16. + 12. + 4. + 12. + 2. * PAD).clamp(MIN_WIDTH, MAX_WIDTH);
        let height = ROW * nodes.len().max(1) as f64 + 2. * PAD;
        let (mut x, mut y) = match previous {
            None => {
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
                (x, y)
            }
            Some((px, py, pw, row)) => {
                let right = px + pw - OVERLAP;
                let x = if right + width > window.x {
                    px - width + OVERLAP
                } else {
                    right
                };
                (x, py + row as f64 * ROW)
            }
        };
        x = x.min(window.x - width).max(0.);
        y = y.min(window.y - height).max(0.);
        output.push(
            json!({"rows":rows,"x":x,"y":y,"width":width,"height":height,
            "open":open.map_or(-1, |index| index as i64)}),
        );
        let Some(open) = open else { break };
        previous = Some((x, y, width, open));
        nodes = match &nodes[open] {
            MenuNode::Group { children, .. } => children,
            MenuNode::Command { .. } => break,
        };
    }
    Value::Array(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(title: &str, children: Vec<MenuNode>) -> MenuNode {
        MenuNode::Group {
            title: title.into(),
            children,
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

    #[test]
    fn open_levels_cascade_beside_their_open_row() {
        let path = vec!["文件".to_owned(), "网络".to_owned()];
        let value = levels(&tree(), &path, Point { x: 100., y: 50. }, WINDOW);
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
        let value = levels(&tree(), &path, origin, WINDOW);
        let levels = value.as_array().unwrap();
        for level in levels {
            let (x, y) = (level["x"].as_f64().unwrap(), level["y"].as_f64().unwrap());
            assert!(x >= 0. && x + level["width"].as_f64().unwrap() <= WINDOW.x);
            assert!(y >= 0. && y + level["height"].as_f64().unwrap() <= WINDOW.y);
        }
        // Root opens to the left of the pointer; its child flips left again.
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
}
