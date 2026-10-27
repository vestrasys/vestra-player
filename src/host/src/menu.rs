//! Projection of the shared command tree into the compact cascading context menu.
//! Pure data and geometry; the Slint view in ui/menu-view.slint only draws it.

use crate::commands::{CommandSnapshot, MenuNode};
use serde_json::{Value, json};

/// Must match MenuItem height, separator height and panel padding in ui/menu-view.slint.
const ROW: f64 = 24.;
const SEPARATOR: f64 = 7.;
const PAD: f64 = 2.;
const MIN_WIDTH: f64 = 168.;
pub(crate) const MAX_WIDTH: f64 = 420.;
/// MenuItem's row in ui/menu-view.slint (must match): font size, left and
/// right padding, check column, layout spacing, submenu arrow column.
const FONT: f64 = 12.;
const ITEM_PAD_LEFT: f64 = 4.;
const ITEM_PAD_RIGHT: f64 = 8.;
const CHECK: f64 = 18.;
const SPACING: f64 = 4.;
const SUBMENU: f64 = 10.;
/// Space between a title and its key text (one layout spacing included).
const KEY_GAP: f64 = 24.;
/// Neighbouring panels overlap slightly so the pointer can cross between them.
const OVERLAP: f64 = 2.;
/// G-03: a level taller than the window scrolls between two arrow bars this
/// high (must match ui/menu-view.slint).
pub const ARROW: f64 = 12.;

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

/// Rough advance of `c` at the 12px menu font, for characters the interface
/// font lacks and before it is known: wide characters (East Asian Width W/F:
/// CJK, kana, Hangul, fullwidth forms) are about one em, the rest (Latin,
/// accented Latin, Cyrillic, symbols) about half; combining marks add nothing.
fn char_estimate(c: char) -> f64 {
    use unicode_width::UnicodeWidthChar;
    match c.width() {
        Some(0) if !c.is_ascii() => 0.,
        Some(2) => 12.5,
        _ => 7.,
    }
}

/// Advance of a character the interface font lacks, drawn in a fallback
/// font: CJK (Han, kana, Hangul, fullwidth forms) about one em, any other
/// symbol (▸, ⚙, an emoji, drawn from a symbol or emoji font) up to one and a
/// half em; combining marks nothing.
fn missing_estimate(c: char) -> f64 {
    use unicode_width::UnicodeWidthChar;
    let cjk = matches!(u32::from(c),
        0x1100..=0x11FF | 0x2E80..=0xA4CF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFF);
    match c.width() {
        Some(0) if !c.is_ascii() => 0.,
        Some(2) if cjk => 12.5,
        _ => 1.5 * FONT,
    }
}

/// Advance of `text` at the 12px menu font: measured in the interface font
/// (`ui_fonts`) on the UI thread, estimated elsewhere.
pub(crate) fn text_width(text: &str) -> f64 {
    crate::ui_fonts::text_width(text, FONT, missing_estimate)
        .unwrap_or_else(|| text.chars().map(char_estimate).sum())
}

/// Panel width that shows every row of `rows` unelided (as standard menus are: each level
/// as wide as its widest row, between MIN_WIDTH and MAX_WIDTH). A row is, in
/// ui/menu-view.slint: the panel inset, MenuItem's left padding, the check
/// column, the title, the key column (after a gap) if any, the submenu arrow
/// column and the right padding, with the layout spacing between columns.
pub(crate) fn panel_width(rows: &[Value]) -> f64 {
    let widest = rows
        .iter()
        .map(|row| {
            let busy = row["busy"].as_bool().unwrap_or(false);
            let title = row["title"].as_str().unwrap_or_default();
            let title = text_width(title) + if busy { text_width(" …") } else { 0. };
            let keys = text_width(row["keys"].as_str().unwrap_or_default());
            title + if keys > 0. { KEY_GAP + keys } else { 0. }
        })
        .fold(0., f64::max);
    let chrome = 2. * PAD + ITEM_PAD_LEFT + CHECK + SPACING + SPACING + SUBMENU + ITEM_PAD_RIGHT;
    // Slack for Slint's rounding: its preferred text width is whole pixels
    // and up to 2px over the sum of advances (ui.rs tests).
    (widest + chrome + 2.).ceil().clamp(MIN_WIDTH, MAX_WIDTH)
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
    for (row, separator) in rows.iter_mut().zip(separators) {
        if separator {
            y += SEPARATOR;
        }
        row["separator"] = separator.into();
        row["y"] = y.into();
        y += ROW;
    }
    let width = panel_width(&rows);
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

/// A menu panel in window coordinates (logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x < self.x + self.width
            && point.y >= self.y
            && point.y < self.y + self.height
    }
}

/// The panels of a `levels` value, by depth.
pub fn panels(levels: &Value) -> Vec<Rect> {
    levels
        .as_array()
        .into_iter()
        .flatten()
        .map(|level| {
            let number = |key: &str| level[key].as_f64().unwrap_or(0.);
            Rect {
                x: number("x"),
                y: number("y"),
                width: number("width"),
                height: number("height"),
            }
        })
        .collect()
}

/// G-15: how long a held switch waits once the pointer stops moving toward
/// the open submenu.
pub const AIM_GRACE_MS: u64 = 120;
/// The triangle's near edge reaches this far above and below the submenu,
/// so a move aimed at its first or last row counts.
const AIM_SLACK: f64 = 8.;

/// Whether a pointer move from `from` to `to` heads for `panel`: `to` is in
/// the triangle from `from` to the panel's near edge (or in the panel).
pub fn heading(from: Point, to: Point, panel: Rect) -> bool {
    if panel.contains(to) {
        return true;
    }
    if from == to {
        return false;
    }
    let near = if panel.x >= from.x {
        panel.x
    } else {
        panel.x + panel.width
    };
    let top = Point {
        x: near,
        y: panel.y - AIM_SLACK,
    };
    let bottom = Point {
        x: near,
        y: panel.y + panel.height + AIM_SLACK,
    };
    let side = |a: Point, b: Point, p: Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let (d1, d2, d3) = (
        side(from, top, to),
        side(top, bottom, to),
        side(bottom, from, to),
    );
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

/// The pointer must travel this far (logical px) from where a hold began
/// before its direction is judged: nearer is a rest or jitter, not a move.
pub const AIM_SLOP: f64 = 4.;

/// G-15 (host 0.5.2, user ruling): submenus open and switch at once on
/// hover, on every platform, with no system delay. Only a switch away from
/// an open submenu can wait, while the pointer is moving toward that submenu
/// (`heading`: the diagonal from the row to the submenu crosses siblings):
/// it happens as soon as the pointer turns away, or once it has not moved
/// toward the submenu for `AIM_GRACE_MS`. Reaching the submenu cancels it.
/// The direction is the net displacement from where the hold began
/// (`anchor`), never one 16 ms step, so a slow diagonal (1 px a tick, ticks
/// without horizontal motion) and a 1 px jitter do not read as turning
/// away. Clicks and keys open at once (the caller applies their path
/// directly).
#[derive(Debug, Default)]
pub struct HoverAim {
    pending: Option<Vec<String>>,
    /// The open submenu a held switch waits for.
    toward: Option<Rect>,
    /// Where the pointer was when the hold began.
    anchor: Option<Point>,
    /// The pointer at the last tick.
    last: Option<Point>,
    /// When the pointer last moved toward `toward`.
    moved_ms: u64,
}

impl HoverAim {
    /// The pointer entered a row that asks for `target` (`hover_target`)
    /// while `current` is open; `submenu` is the open panel beside the row's
    /// level, if any, and `pointer` where the pointer is (unknown: no aim).
    /// Returns the path to open now.
    pub fn hover(
        &mut self,
        current: &[String],
        target: Vec<String>,
        pointer: Option<Point>,
        submenu: Option<Rect>,
        now_ms: u64,
    ) -> Option<Vec<String>> {
        if target == current {
            // Back on the open row, or inside the open submenu.
            self.pending = None;
            return None;
        }
        if let (Some(panel), Some(to)) = (submenu, pointer) {
            // The step from the last tick decides only when it is long
            // enough to have a direction: a clear move away (straight down
            // a column) switches now; a short one holds and lets the
            // following ticks judge the net displacement.
            let from = self.last.unwrap_or(to);
            if distance(from, to) < AIM_SLOP || heading(from, to, panel) {
                self.pending = Some(target);
                self.toward = Some(panel);
                self.anchor = Some(from);
                self.moved_ms = now_ms;
                return None;
            }
        }
        self.pending = None;
        Some(target)
    }

    /// Each host tick, with the pointer: a held switch to apply now.
    pub fn tick(&mut self, pointer: Option<Point>, now_ms: u64) -> Option<Vec<String>> {
        let previous = self.last;
        if pointer.is_some() {
            self.last = pointer;
        }
        let panel = self.toward.filter(|_| self.pending.is_some())?;
        let (Some(anchor), Some(to)) = (self.anchor, pointer) else {
            // The pointer left the window: nothing to aim with.
            return self.pending.take();
        };
        if distance(anchor, to) >= AIM_SLOP {
            if !heading(anchor, to, panel) {
                return self.pending.take();
            }
            if previous.is_some_and(|from| from != to) {
                self.moved_ms = now_ms;
            }
        }
        if now_ms.saturating_sub(self.moved_ms) >= AIM_GRACE_MS {
            return self.pending.take();
        }
        None
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
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

    /// G-15 (host 0.5.2): the submenu-aim triangle.
    #[test]
    fn a_move_heads_for_a_submenu_inside_the_triangle_to_its_near_edge() {
        let panel = Rect {
            x: 300.,
            y: 100.,
            width: 200.,
            height: 120.,
        };
        let at = |x, y| Point { x, y };
        // From a row at the left, diagonally down toward the submenu.
        assert!(heading(at(200., 110.), at(220., 125.), panel));
        // Up toward its top row (within the slack above it).
        assert!(heading(at(200., 110.), at(230., 106.), panel));
        // Straight down the parent column, or back to the left: away.
        assert!(!heading(at(200., 110.), at(200., 130.), panel));
        assert!(!heading(at(200., 110.), at(180., 112.), panel));
        // Far below the submenu's bottom edge: away.
        assert!(!heading(at(200., 110.), at(210., 300.), panel));
        // Not moving is not heading; inside the panel always is.
        assert!(!heading(at(200., 110.), at(200., 110.), panel));
        assert!(heading(at(200., 110.), at(310., 110.), panel));
        // A submenu that opened to the left (near the window's right edge).
        let left = Rect { x: 0., ..panel };
        assert!(heading(at(260., 110.), at(240., 125.), left));
        assert!(!heading(at(260., 110.), at(280., 125.), left));
    }

    /// G-15 (host 0.5.2): hovers open at once; only a switch while the
    /// pointer heads for the open submenu waits, until it turns away or
    /// stops for `AIM_GRACE_MS`, and reaching the submenu cancels it.
    #[test]
    fn submenus_open_at_once_and_aiming_at_the_open_one_holds_a_switch() {
        let tree = tree(); // 文件 ▶ (播放列表 ▶, 网络 ▶), 播放 ▶
        let token = |path: &[&str]| format!("g:{}", json!(path));
        let at = |x, y| Some(Point { x, y });
        let submenu = Some(Rect {
            x: 300.,
            y: 100.,
            width: 200.,
            height: 120.,
        });
        let mut aim = HoverAim::default();
        let mut current: Vec<String> = Vec::new();
        // Nothing open: a group row opens at once, with or without a pointer.
        assert_eq!(aim.tick(at(200., 110.), 0), None);
        let target = hover_target(&current, 0, &token(&["文件"])).unwrap();
        current = aim.hover(&current, target, None, None, 0).unwrap();
        assert_eq!(current, ["文件"]);
        assert_eq!(valid_path(&tree, &current), current);
        // Moving straight down onto 播放 (not toward the submenu): at once.
        assert_eq!(aim.tick(at(200., 110.), 16), None);
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        current = aim
            .hover(&current, target, at(200., 134.), submenu, 32)
            .unwrap();
        assert_eq!(current, ["播放"]);
        // Back to 文件, then a diagonal toward its submenu crosses 播放: held.
        let target = hover_target(&current, 0, &token(&["文件"])).unwrap();
        current = aim
            .hover(&current, target, at(200., 112.), None, 48)
            .unwrap();
        aim.tick(at(200., 112.), 48);
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        assert_eq!(
            aim.hover(&current, target, at(215., 125.), submenu, 64),
            None
        );
        // Still moving toward it: held past the grace.
        assert_eq!(aim.tick(at(215., 125.), 64), None);
        assert_eq!(aim.tick(at(240., 140.), 160), None);
        assert_eq!(aim.tick(at(270., 150.), 260), None);
        // Reaching the submenu (a row of it) cancels the switch.
        let inside = hover_target(&current, 1, "c:3:x").unwrap();
        assert_eq!(aim.hover(&current, inside, at(305., 150.), None, 270), None);
        assert_eq!(aim.tick(at(305., 150.), 1000), None);
        assert_eq!(current, ["文件"]);
        // Aimed, then resting on 播放: it switches after the grace.
        aim.tick(at(200., 112.), 2000);
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        assert_eq!(
            aim.hover(&current, target, at(215., 125.), submenu, 2016),
            None
        );
        assert_eq!(aim.tick(at(215., 125.), 2016), None);
        assert_eq!(aim.tick(at(215., 125.), 2016 + AIM_GRACE_MS - 1), None);
        current = aim.tick(at(215., 125.), 2016 + AIM_GRACE_MS).unwrap();
        assert_eq!(current, ["播放"]);
        // Aimed, then turning away (down the column): it switches at once.
        let target = hover_target(&current, 0, &token(&["文件"])).unwrap();
        current = aim
            .hover(&current, target, at(200., 112.), None, 3000)
            .unwrap();
        aim.tick(at(200., 112.), 3000);
        let target = hover_target(&current, 0, &token(&["播放"])).unwrap();
        assert_eq!(
            aim.hover(&current, target, at(215., 125.), submenu, 3016),
            None
        );
        aim.tick(at(215., 125.), 3016);
        current = aim.tick(at(212., 150.), 3032).unwrap();
        assert_eq!(current, ["播放"]);
        // A command row at the top level closes the open submenu at once.
        let target = hover_target(&current, 0, "c:3:open").unwrap();
        assert_eq!(
            aim.hover(&current, target, at(212., 170.), None, 3048),
            Some(Vec::<String>::new())
        );
        // A cleared hold never fires.
        aim.tick(at(200., 112.), 4000);
        assert_eq!(
            aim.hover(
                &["文件".into()],
                vec!["播放".into()],
                at(215., 125.),
                submenu,
                4016
            ),
            None
        );
        aim.clear();
        assert_eq!(aim.tick(at(215., 125.), 99_999), None);
        assert_eq!(hover_target(&[], 0, "bad"), None);
    }

    /// Review finding (2026-10-27): the direction is the net displacement
    /// from where the hold began, so a slow diagonal with ticks that move
    /// only vertically, and a 1 px jitter, keep the submenu; the pointer
    /// has to leave the triangle, or rest, before the switch.
    #[test]
    fn a_slow_diagonal_and_a_jitter_keep_the_open_submenu() {
        let at = |x, y| Some(Point { x, y });
        let submenu = Some(Rect {
            x: 300.,
            y: 100.,
            width: 200.,
            height: 120.,
        });
        let open = vec!["文件".to_owned()];
        let sibling = vec!["播放".to_owned()];
        // 1 px steps toward the submenu, a third of them straight down
        // (dx = 0) and a third straight right: a step alone would read as
        // turning away or as resting.
        let mut aim = HoverAim::default();
        aim.tick(at(200., 112.), 0);
        aim.tick(at(200., 113.), 16);
        assert_eq!(
            aim.hover(&open, sibling.clone(), at(200., 113.), submenu, 16),
            None,
            "a short step holds"
        );
        let (mut x, mut y, mut now) = (200., 113., 16);
        for step in 0..36 {
            now += 16;
            let (dx, dy) = [(1., 0.), (0., 1.), (1., 1.)][step % 3];
            x += dx;
            y += dy;
            assert_eq!(aim.tick(at(x, y), now), None, "step {step} at ({x}, {y})");
        }
        // 24 px right and down: inside the triangle, still held after far
        // more than the grace. Turning down the column now switches at once.
        assert_eq!(aim.tick(at(x, y + 30.), now + 16), Some(sibling.clone()));
        // A jitter around the hover point: no judgment, no switch, until
        // the grace says the pointer rests.
        let mut aim = HoverAim::default();
        aim.tick(at(200., 112.), 1000);
        assert_eq!(
            aim.hover(&open, sibling.clone(), at(201., 113.), submenu, 1016),
            None
        );
        let wobble = [
            (201., 114.),
            (200., 113.),
            (201., 112.),
            (202., 113.),
            (201., 114.),
        ];
        for (i, (x, y)) in wobble.into_iter().enumerate() {
            let now = 1032 + 16 * i as u64;
            assert_eq!(aim.tick(at(x, y), now), None, "wobble {i}");
        }
        assert_eq!(aim.tick(at(201., 114.), 1016 + AIM_GRACE_MS), Some(sibling));
    }

    /// Each level is as wide as its widest row (title, key column,
    /// arrow, paddings), measured in the interface font, between MIN_WIDTH
    /// and MAX_WIDTH. Coordinator report 2026-10-27: "VestraNES Window…"
    /// was elided in a panel sized by the old per-character estimate.
    #[test]
    fn each_level_is_as_wide_as_its_widest_row() {
        let measure = crate::ui_fonts::TextMeasure::new(
            slint::fontique_011::fontique::Blob::new(std::sync::Arc::new(
                crate::font_asset::bundled_bytes(),
            )),
            0,
        )
        .unwrap();
        crate::ui_fonts::set_measure(Some(measure));
        let chrome = 2. * PAD + ITEM_PAD_LEFT + CHECK + 2. * SPACING + SUBMENU + ITEM_PAD_RIGHT;
        let width = |rows: Vec<Value>| panel(rows, vec![false; 4]).1;
        let row = |title: &str, keys: &str| json!({"title": title, "keys": keys, "busy": false});
        let title = "VestraNES Window…";
        let measured = text_width(title);
        assert!(measured > 0. && measured != title.chars().map(char_estimate).sum::<f64>());
        // The widest row decides; the result is whole pixels.
        let wide = width(vec![row("Load ROM…", ""), row("Stop", ""), row(title, "")]);
        assert_eq!(wide, (measured + chrome + 2.).ceil().max(MIN_WIDTH));
        assert!(wide - chrome >= measured);
        // A long German label gets its whole width, beyond the minimum.
        let german = "Untertitel-Synchronisierung zurücksetzen";
        let long = width(vec![row(german, "")]);
        assert!(long > MIN_WIDTH && long - chrome >= text_width(german));
        // Key text adds its column and the gap before it; a busy row its " …".
        let keyed = width(vec![row(german, "Ctrl+Shift+Alt+F12")]);
        assert!(keyed - chrome >= text_width(german) + KEY_GAP + text_width("Ctrl+Shift+Alt+F12"));
        let busy = width(vec![json!({"title": german, "keys": "", "busy": true})]);
        assert!(busy - chrome >= text_width(german) + text_width(" …"));
        // Short rows keep the minimum; absurd ones stop at the maximum.
        assert_eq!(width(vec![row("Stop", "")]), MIN_WIDTH);
        assert_eq!(width(vec![row(&"W".repeat(200), "")]), MAX_WIDTH);
        assert_eq!(width(vec![]), MIN_WIDTH);
        // Characters the font lacks: CJK about one em, other symbols and
        // emoji one and a half, combining marks nothing.
        assert_eq!(missing_estimate('中'), 12.5);
        assert_eq!(missing_estimate('한'), 12.5);
        assert_eq!(missing_estimate('▸'), 18.);
        assert_eq!(missing_estimate('\u{1F3AC}'), 18.);
        assert_eq!(missing_estimate('\u{301}'), 0.);
        // Off the UI thread (no interface font) the estimate still applies.
        crate::ui_fonts::set_measure(None);
        assert_eq!(text_width("abc"), 21.);
    }
}
