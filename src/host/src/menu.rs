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
/// G-03: a level taller than the window scrolls between two arrow bars this
/// high (must match ui/menu-view.slint).
pub const ARROW: f64 = 12.;
/// G-15: submenus open (and switch) after this delay on platforms without a
/// system setting; Windows reads its own (`platform::menu_show_delay`).
pub const SHOW_DELAY_MS: u64 = 400;

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

/// Rough text advance at the 12px menu font: wide characters (East Asian
/// Width W/F: CJK, kana, Hangul, fullwidth forms) are about one em, the rest
/// (Latin, accented Latin, Cyrillic, symbols) about half; combining marks
/// add nothing.
fn text_width(text: &str) -> f64 {
    use unicode_width::UnicodeWidthChar;
    text.chars()
        .map(|c| match c.width() {
            Some(0) if !c.is_ascii() => 0.,
            Some(2) => 12.5,
            _ => 7.,
        })
        .sum()
}

/// Key text for a command row (from the binding table).
pub type Labels<'a> = &'a dyn Fn(u64, &str) -> Option<String>;

fn command_row(entry: &CommandSnapshot, labels: Labels<'_>) -> Value {
    json!({"token":format!("c:{}:{}",entry.key.owner,entry.key.local_id),
        "title":entry.declaration.shown_title(),"enabled":entry.state.enabled,
        "checked":entry.state.checked.unwrap_or(false),"radio":entry.declaration.radio,
        "busy":entry.state.busy,"reason":entry.state.reason.clone().unwrap_or_default(),
        "has-children":false,"open":false,
        "keys":labels(entry.key.owner,&entry.key.local_id).unwrap_or_default()})
}

fn row(node: &MenuNode, path: &[String], open: bool, labels: Labels<'_>) -> Value {
    match node {
        MenuNode::Group { title, label, .. } => {
            let mut next = path.to_vec();
            next.push(title.clone());
            json!({"token":format!("g:{}",json!(next)),"title":label,"enabled":true,"checked":false,
                "radio":false,"busy":false,"reason":"","has-children":true,"open":open,"keys":""})
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
        let keys = text_width(row["keys"].as_str().unwrap_or_default());
        widest = widest.max(title + if keys > 0. { 24. + keys } else { 0. });
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

/// G-03: where a submenu of `width` opens beside its parent panel at `px`
/// (`pw` wide) in a window `window_x` wide, and whether it went left.
/// It keeps the direction of the cascade so far (right at first) and flips
/// when that side has no room. When neither side has room it takes the side
/// that leaves more of the parent uncovered (the right on a tie), clamped
/// into the window: the deeper panel is drawn above and takes the pointer,
/// so whatever it covers of the parent cannot be hovered or clicked.
fn cascade(px: f64, pw: f64, width: f64, window_x: f64, leftward: bool) -> (f64, bool) {
    let right = px + pw - OVERLAP;
    let left = px - width + OVERLAP;
    let go_left = match (right + width <= window_x, left >= 0.) {
        (true, true) => leftward,
        (true, false) => false,
        (false, true) => true,
        // Room left of the parent against room right of it.
        (false, false) => px > window_x - (px + pw),
    };
    let x = if go_left { left } else { right };
    (x.min(window_x - width).max(0.), go_left)
}

/// G-03: a level's height in a window `available` high, whether it scrolls,
/// and its scroll offset (`requested`, clamped). A scrolling level fills the
/// window; its rows show between two arrow bars.
fn fit(content: f64, available: f64, requested: f64) -> (f64, bool, f64) {
    if content <= available {
        return (content, false, 0.);
    }
    let height = available.max(2. * ARROW + ROW + 2. * PAD);
    let visible = height - 2. * ARROW;
    let offset = if requested.is_finite() {
        requested.clamp(0., content - visible)
    } else {
        0.
    };
    (height, true, offset)
}

/// Where a level's row tops (panel content coordinates) are drawn in the panel.
fn row_shift(scrolling: bool, offset: f64) -> f64 {
    if scrolling { ARROW - offset } else { 0. }
}

/// One panel per open level, root first. `path` must come from `valid_path`.
/// Panels open at `origin` and cascade beside their open row (`cascade`):
/// to the right, to the left after a flip at the window edge, clamped
/// inside `window`. A level taller than the window scrolls; `scroll[depth]`
/// is its requested offset.
pub fn levels(
    roots: &[MenuNode],
    path: &[String],
    origin: Point,
    window: Point,
    scroll: &[f64],
    labels: Labels<'_>,
) -> Value {
    let mut output = Vec::new();
    let mut nodes = roots;
    let mut previous: Option<(f64, f64, f64, f64)> = None; // x, y, width, open row top
    let mut leftward = false;
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
        let (rows, width, content) = panel(rows, separators);
        let requested = scroll.get(depth).copied().unwrap_or(0.);
        let (height, scrolling, offset) = fit(content, window.y, requested);
        let (x, y) = match previous {
            None => place(origin, width, height, window),
            Some((px, py, pw, row_y)) => {
                let x;
                (x, leftward) = cascade(px, pw, width, window.x, leftward);
                let y = py + row_y - PAD;
                (x, y.min(window.y - height).max(0.))
            }
        };
        let open_y = open.and_then(|index| rows[index]["y"].as_f64());
        output.push(
            json!({"rows":rows,"x":x,"y":y,"width":width,"height":height,
            "open":open.map_or(-1, |index| index as i64),
            "scroll":scrolling,"viewport-y":offset}),
        );
        let (Some(open), Some(open_y)) = (open, open_y) else {
            break;
        };
        previous = Some((x, y, width, open_y + row_shift(scrolling, offset)));
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
    scroll: f64,
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
    let (rows, width, content) = panel(rows, separators);
    let (height, scrolling, offset) = fit(content, window.y, scroll);
    let (x, y) = place(origin, width, height, window);
    json!([{"rows":rows,"x":x,"y":y,"width":width,"height":height,"open":-1,
        "scroll":scrolling,"viewport-y":offset}])
}

/// The open chain a hover asks for: a group row opens that group (closing
/// anything deeper); a command row at `level` closes the levels below it.
pub fn hover_target(current: &[String], level: usize, token: &str) -> Option<Vec<String>> {
    match parse_token(token)? {
        Token::Group(mut path) => {
            path.truncate(level + 1);
            Some(path)
        }
        Token::Command { .. } => Some(current.iter().take(level).cloned().collect()),
    }
}

/// G-15: Win32 submenu timing. The pointer resting on a group row opens it
/// only after the show delay; with another submenu open, hovering a sibling
/// switches after the same delay, and entering the open submenu meanwhile
/// cancels the switch (so a diagonal move does not flip submenus). Clicks
/// and keys open at once (the caller applies their path directly).
#[derive(Debug, Default)]
pub struct HoverDelay {
    pending: Option<PendingHover>,
}

#[derive(Debug)]
struct PendingHover {
    path: Vec<String>,
    since_ms: u64,
}

impl HoverDelay {
    /// The pointer entered a row that asks for `target` (`hover_target`), while
    /// `current` is open.
    pub fn hover(&mut self, current: &[String], target: Vec<String>, now_ms: u64) {
        if target == current {
            // Back on the open row, or inside the open submenu.
            self.pending = None;
            return;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.path == target)
        {
            return;
        }
        self.pending = Some(PendingHover {
            path: target,
            since_ms: now_ms,
        });
    }

    /// The path to open now that the delay passed, if any.
    pub fn due(&mut self, now_ms: u64, delay_ms: u64) -> Option<Vec<String>> {
        let ready = self
            .pending
            .as_ref()
            .is_some_and(|pending| now_ms.saturating_sub(pending.since_ms) >= delay_ms);
        ready
            .then(|| self.pending.take().map(|pending| pending.path))
            .flatten()
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandKey, CommandState, Declaration};

    fn group(title: &str, children: Vec<MenuNode>) -> MenuNode {
        MenuNode::Group {
            title: title.into(),
            label: title.into(),
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
        let value = levels(
            &tree(),
            &path,
            Point { x: 100., y: 50. },
            WINDOW,
            &[],
            &none,
        );
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
        let value = levels(&tree(), &path, origin, WINDOW, &[], &none);
        let levels = value.as_array().unwrap();
        for level in levels {
            let (x, y) = (level["x"].as_f64().unwrap(), level["y"].as_f64().unwrap());
            assert!(x >= 0. && x + level["width"].as_f64().unwrap() <= WINDOW.x);
            assert!(y >= 0. && y + level["height"].as_f64().unwrap() <= WINDOW.y);
        }
        assert!(levels[1]["x"].as_f64().unwrap() < levels[0]["x"].as_f64().unwrap());
    }

    fn spans(value: &Value) -> Vec<(f64, f64)> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|level| {
                let x = level["x"].as_f64().unwrap();
                (x, x + level["width"].as_f64().unwrap())
            })
            .collect()
    }

    /// Coordinator report 2026-10-24: a submenu drawn over its parent takes
    /// the pointer, so the parent's rows under it can be neither hovered nor
    /// clicked. After a flip to the left the cascade keeps going left
    /// instead of opening back over the top level.
    #[test]
    fn a_cascade_that_flipped_left_keeps_going_left() {
        let path = vec!["文件".to_owned(), "网络".to_owned()];
        let origin = Point { x: 700., y: 50. };
        let spans = spans(&levels(&tree(), &path, origin, WINDOW, &[], &none));
        assert_eq!(spans[0].0, 700.);
        assert!(spans[1].1 <= spans[0].0 + OVERLAP, "{spans:?}");
        assert!(spans[2].1 <= spans[1].0 + OVERLAP, "{spans:?}");
    }

    /// With room on neither side, the submenu takes the side that covers
    /// less of its parent; the rest of the parent stays reachable.
    #[test]
    fn with_room_on_neither_side_a_submenu_covers_as_little_of_its_parent_as_it_can() {
        let path = vec!["文件".to_owned()];
        let narrow = Point { x: 330., y: 400. };
        for origin_x in [10., 40., 80., 120., 160.] {
            let origin = Point {
                x: origin_x,
                y: 20.,
            };
            let spans = spans(&levels(&tree(), &path, origin, narrow, &[], &none));
            let ((x0, r0), (x1, r1)) = (spans[0], spans[1]);
            assert!(x1 >= 0. && r1 <= narrow.x);
            let uncovered = if x1 > x0 { x1 - x0 } else { r0 - r1 };
            let best = (narrow.x - (r1 - x1) - x0).max(r0 - (r1 - x1));
            assert_eq!(uncovered, best.min(r0 - x0), "origin {origin_x}: {spans:?}");
        }
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
            &[],
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
    fn group_changes_insert_separators_and_key_labels_come_from_bindings() {
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
        let value = levels(&nodes, &[], Point { x: 0., y: 0. }, WINDOW, &[], &labels);
        let rows = value[0]["rows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["separator"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(rows[2]["y"].as_f64().unwrap(), PAD + 2. * ROW + SEPARATOR);
        assert_eq!(rows[2]["keys"], "Enter");
        assert_eq!(
            value[0]["height"].as_f64().unwrap(),
            3. * ROW + SEPARATOR + 2. * PAD
        );
        let (a, b) = (command("a", "x"), command("b", "x"));
        let context = context_level(
            &[None, Some(&a), None, None, Some(&b), None],
            Point { x: 5., y: 5. },
            WINDOW,
            0.,
            &labels,
        );
        let rows = context[0]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["separator"], false);
        assert_eq!(rows[1]["separator"], true);
    }

    fn commands(count: usize) -> Vec<MenuNode> {
        (0..count)
            .map(|index| MenuNode::Command {
                entry: command(&format!("c{index}"), "g"),
            })
            .collect()
    }

    /// G-03 step 1: 25 rows in a 182-high window; every row can be scrolled
    /// fully into view between the arrow bars.
    #[test]
    fn a_level_taller_than_the_window_scrolls_and_every_row_is_reachable() {
        let nodes = commands(25);
        let window = Point { x: 320., y: 182. };
        let origin = Point { x: 10., y: 100. };
        let level = |scroll: f64| levels(&nodes, &[], origin, window, &[scroll], &none)[0].clone();
        let top = level(0.);
        assert_eq!(top["scroll"], true);
        assert_eq!(top["height"].as_f64().unwrap(), 182.);
        assert_eq!(top["y"].as_f64().unwrap(), 0.);
        let visible = 182. - 2. * ARROW;
        let content = 25. * ROW + 2. * PAD;
        let rows = top["rows"].as_array().unwrap().clone();
        for (index, row) in rows.iter().enumerate() {
            let row_y = row["y"].as_f64().unwrap();
            // Scroll so the row's bottom meets the visible bottom.
            let shown = level(row_y + ROW - visible);
            let offset = shown["viewport-y"].as_f64().unwrap();
            assert!(
                row_y >= offset && row_y + ROW <= offset + visible,
                "row {index} at {row_y} not visible at offset {offset}"
            );
        }
        // Offsets are clamped to the content.
        assert_eq!(level(-50.)["viewport-y"].as_f64().unwrap(), 0.);
        assert_eq!(
            level(1e6)["viewport-y"].as_f64().unwrap(),
            content - visible
        );
        assert_eq!(level(f64::NAN)["viewport-y"].as_f64().unwrap(), 0.);
        // A short menu does not scroll.
        let short = levels(&commands(3), &[], origin, window, &[40.], &none);
        assert_eq!(short[0]["scroll"], false);
        assert_eq!(short[0]["viewport-y"].as_f64().unwrap(), 0.);
        // Panel menus scroll the same way.
        let entries: Vec<CommandSnapshot> =
            (0..25).map(|i| command(&format!("p{i}"), "g")).collect();
        let items: Vec<_> = entries.iter().map(Some).collect();
        let panel = context_level(&items, origin, Point { x: 300., y: 250. }, 1e6, &none);
        assert_eq!(panel[0]["scroll"], true);
        assert_eq!(panel[0]["height"].as_f64().unwrap(), 250.);
        assert_eq!(
            panel[0]["viewport-y"].as_f64().unwrap(),
            content - (250. - 2. * ARROW)
        );
    }

    #[test]
    fn a_submenu_opens_beside_its_row_as_scrolled() {
        let mut nodes = commands(30);
        nodes.push(group("更多", vec![group("子", vec![])]));
        let window = Point { x: 1000., y: 300. };
        let path = vec!["更多".to_owned()];
        let value = levels(&nodes, &path, Point { x: 0., y: 0. }, window, &[1e6], &none);
        let parent = &value[0];
        let offset = parent["viewport-y"].as_f64().unwrap();
        let row_y = parent["rows"][30]["y"].as_f64().unwrap();
        let shown = parent["y"].as_f64().unwrap() + ARROW + row_y - offset;
        assert_eq!(value[1]["y"].as_f64().unwrap(), shown - PAD);
    }

    /// G-15: hover timing, without a clock or a view.
    #[test]
    fn submenus_open_and_switch_after_the_delay_and_entering_the_open_one_cancels() {
        let tree = tree(); // 文件 ▶ (播放列表 ▶, 网络 ▶), 播放 ▶
        let token = |path: &[&str]| format!("g:{}", json!(path));
        let mut delay = HoverDelay::default();
        let mut current: Vec<String> = Vec::new();
        // Resting on 文件 for less than the delay, then leaving: nothing opens.
        let target = hover_target(&current, 0, &token(&["文件"])).unwrap();
        delay.hover(&current, target, 0);
        assert_eq!(delay.due(399, SHOW_DELAY_MS), None);
        let away = hover_target(&current, 0, "c:3:open").unwrap();
        delay.hover(&current, away, 200);
        assert_eq!(delay.due(1000, SHOW_DELAY_MS), None);
        // Resting long enough opens it.
        let target = hover_target(&current, 0, &token(&["文件"])).unwrap();
        delay.hover(&current, target, 1000);
        assert_eq!(delay.due(1399, SHOW_DELAY_MS), None);
        current = delay.due(1400, SHOW_DELAY_MS).unwrap();
        assert_eq!(current, ["文件"]);
        assert_eq!(valid_path(&tree, &current), current);
        // Crossing 播放 on the way into the open submenu: no switch.
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        delay.hover(&current, target, 2000);
        let inside = hover_target(&current, 1, "c:3:x").unwrap();
        delay.hover(&current, inside, 2100);
        assert_eq!(delay.due(5000, SHOW_DELAY_MS), None);
        assert_eq!(current, ["文件"]);
        // Resting on 播放: after the delay 文件 closes and 播放 opens.
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        delay.hover(&current, target, 6000);
        current = delay.due(6400, SHOW_DELAY_MS).unwrap();
        assert_eq!(current, ["播放"]);
        // A command row at the top level closes the open submenu after the delay.
        let target = hover_target(&current, 0, "c:3:open").unwrap();
        delay.hover(&current, target, 7000);
        assert_eq!(
            delay.due(7400, SHOW_DELAY_MS).unwrap(),
            Vec::<String>::new()
        );
        // Repeated hover events of the same row keep the first time.
        let target = hover_target(&[], 0, &token(&["文件"])).unwrap();
        delay.hover(&[], target.clone(), 8000);
        delay.hover(&[], target, 8300);
        assert!(delay.due(8400, SHOW_DELAY_MS).is_some());
        delay.hover(&[], vec!["播放".into()], 9000);
        delay.clear();
        assert_eq!(delay.due(99_999, SHOW_DELAY_MS), None);
        assert_eq!(hover_target(&[], 0, "bad"), None);
    }
}
