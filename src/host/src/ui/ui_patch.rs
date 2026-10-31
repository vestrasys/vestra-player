//! Row patches (player 0.6.0, SDK 1.5.44; doc/architecture/large-lists.md
//! §2): a property write (`ui.setMount`, `ui.set`, `settings.set`) may carry
//! `patch: {base, seq, ops}` instead of `value`. The ops (`set`, `insert`,
//! `remove`, `move`, by index, in order) are written into the live model
//! with `set_row_data`, `insert` and `remove`, without converting the old
//! or new array back to JSON: a selection change in a 200-row list costs one
//! row, not 200. `base` names the write the patch applies on (the `seq` of a
//! whole write or patch, 0 for the initial properties); any other base is
//! answered `stale` and the SDK sends the whole value.
//!
//! Everything is checked before anything changes: the ops against the
//! length, the carried rows against the value limits, their images against
//! the view's budget, each live model against the rows' type. Panels and
//! settings pages keep their properties as JSON for rebuilds; the patch
//! applies there too, and where a live model cannot take it in place the
//! patched whole value is written instead.
#![forbid(unsafe_code)]

use crate::host::{Host, RpcError};
use crate::ui::{ui_image, ui_values};
use serde_json::Value as Json;
use slint::{Model, VecModel};
use slint_interpreter::Value;
use std::cell::RefCell;

/// The feature the player advertises in `plugin.app` (`features`): the SDK
/// sends patches and `seq` only to a player that lists it.
pub const FEATURE: &str = "ui.patch";
/// The most ops one patch may carry (the SDK sends the whole value beyond).
pub const MAX_PATCH_OPS: usize = 1024;

/// One op, by index, applied to the result of the ones before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Row `at` is replaced by the next carried row.
    Set(usize),
    /// The next `count` carried rows are inserted at `at`.
    Insert(usize, usize),
    /// `count` rows starting at `at` are removed.
    Remove(usize, usize),
    /// `count` rows are taken out at `from` and put back at `to`, an index
    /// of the list without them.
    Move {
        from: usize,
        count: usize,
        to: usize,
    },
}

/// A patch's ops, checked against a list of some length: their shapes, the
/// rows they carry (in op order) and the length after them.
#[derive(Debug)]
pub struct Ops<'a> {
    pub shapes: Vec<Shape>,
    pub rows: Vec<&'a Json>,
    /// The length before and after the ops.
    pub before: usize,
    pub len: usize,
}

/// `patch` of a write: its `base`, its `seq` (at least 1) and its ops.
pub fn request(patch: &Json) -> std::result::Result<(u64, u64, &Json), RpcError> {
    let number = |key: &str| {
        patch[key].as_u64().ok_or_else(|| {
            RpcError::invalid(format!("patch.{key} requires a non-negative integer"))
        })
    };
    let (base, seq) = (number("base")?, number("seq")?);
    if seq == 0 {
        return Err(RpcError::invalid("patch.seq must be at least 1"));
    }
    Ok((base, seq, &patch["ops"]))
}

/// The answer to a patch whose base is not the property's last write.
pub fn stale() -> RpcError {
    RpcError::new(
        "stale",
        "the property changed since the write this patch applies on; send the whole value",
    )
}

/// Check `ops` (wire form) against a list of `len` rows.
pub fn parse(ops: &Json, len: usize) -> std::result::Result<Ops<'_>, RpcError> {
    let list = ops
        .as_array()
        .ok_or_else(|| RpcError::invalid("patch.ops requires an array"))?;
    if list.len() > MAX_PATCH_OPS {
        return Err(RpcError::new(
            "limit",
            format!("a patch carries at most {MAX_PATCH_OPS} operations"),
        ));
    }
    let mut out = Ops {
        shapes: Vec::with_capacity(list.len()),
        rows: Vec::new(),
        before: len,
        len,
    };
    for op in list {
        let index = |key: &str| {
            op[key]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    RpcError::invalid(format!("patch op {key} requires a non-negative integer"))
                })
        };
        let count = || {
            index("count").and_then(|count| {
                if count == 0 {
                    Err(RpcError::invalid("patch op count must be at least 1"))
                } else {
                    Ok(count)
                }
            })
        };
        let past = |what: String| RpcError::invalid(format!("{what} is past the end of the list"));
        let len = out.len;
        match op["op"].as_str() {
            Some("set") => {
                let at = index("at")?;
                if at >= len {
                    return Err(past(format!("set at {at}")));
                }
                let row = op
                    .get("row")
                    .ok_or_else(|| RpcError::invalid("patch op set requires row"))?;
                out.rows.push(row);
                out.shapes.push(Shape::Set(at));
            }
            Some("insert") => {
                let at = index("at")?;
                let rows = op["rows"]
                    .as_array()
                    .filter(|rows| !rows.is_empty())
                    .ok_or_else(|| RpcError::invalid("patch op insert requires rows"))?;
                if at > len {
                    return Err(past(format!("insert at {at}")));
                }
                out.len = len + rows.len();
                if out.len > ui_values::MAX_ARRAY {
                    return Err(RpcError::new(
                        "limit",
                        format!("UI array exceeds {} rows", ui_values::MAX_ARRAY),
                    ));
                }
                out.rows.extend(rows);
                out.shapes.push(Shape::Insert(at, rows.len()));
            }
            Some("remove") => {
                let (at, count) = (index("at")?, count()?);
                if at.checked_add(count).is_none_or(|end| end > len) {
                    return Err(past(format!("remove {count} at {at}")));
                }
                out.len = len - count;
                out.shapes.push(Shape::Remove(at, count));
            }
            Some("move") => {
                let (from, count, to) = (index("from")?, count()?, index("to")?);
                if from.checked_add(count).is_none_or(|end| end > len)
                    || to.checked_add(count).is_none_or(|end| end > len)
                {
                    return Err(past(format!("move {count} from {from} to {to}")));
                }
                out.shapes.push(Shape::Move { from, count, to });
            }
            _ => {
                return Err(RpcError::invalid(
                    "patch op must be set, insert, remove or move",
                ));
            }
        }
    }
    Ok(out)
}

/// Apply checked shapes to `items`, taking the carried rows from `rows`.
pub fn apply_vec<T>(items: &mut Vec<T>, shapes: &[Shape], rows: impl IntoIterator<Item = T>) {
    let mut rows = rows.into_iter();
    for shape in shapes {
        match *shape {
            Shape::Set(at) => {
                if let Some(row) = rows.next() {
                    items[at] = row;
                }
            }
            Shape::Insert(at, count) => {
                items.splice(at..at, rows.by_ref().take(count));
            }
            Shape::Remove(at, count) => {
                items.drain(at..at + count);
            }
            Shape::Move { from, count, to } => {
                let moved: Vec<T> = items.drain(from..from + count).collect();
                items.splice(to..to, moved);
            }
        }
    }
}

/// `current` (a live property) can take the patch in place: a host-owned
/// model of `len` rows, no Markdown documents in its rows, and carried rows
/// of the type of its first row (Slint checks the type of an installed
/// model, not of a written row). An empty model does not show its row type.
pub fn applicable(current: &Value, len: usize, shapes: &[Shape], rows: &[Value]) -> bool {
    let Value::Model(model) = current else {
        return false;
    };
    if !model.as_any().is::<VecModel<Value>>() || model.row_count() != len {
        return false;
    }
    if shapes.is_empty() {
        return true;
    }
    let Some(sample) = model.row_data(0) else {
        return rows.is_empty();
    };
    !ui_values::has_markdown(&sample)
        && rows
            .iter()
            .all(|row| ui_values::fits(&sample, row) && !ui_values::has_markdown(row))
}

/// Write an `applicable` patch into `current`.
pub fn apply(current: &Value, shapes: &[Shape], rows: Vec<Value>) {
    let Value::Model(model) = current else {
        return;
    };
    let Some(model) = model.as_any().downcast_ref::<VecModel<Value>>() else {
        return;
    };
    let mut rows = rows.into_iter();
    for shape in shapes {
        match *shape {
            Shape::Set(at) => {
                if let Some(row) = rows.next() {
                    model.set_row_data(at, row);
                }
            }
            Shape::Insert(at, count) => {
                for (offset, row) in rows.by_ref().take(count).enumerate() {
                    model.insert(at + offset, row);
                }
            }
            Shape::Remove(at, count) => {
                for _ in 0..count {
                    model.remove(at);
                }
            }
            Shape::Move { from, count, to } => {
                let moved: Vec<Value> = (from..from + count)
                    .filter_map(|row| model.row_data(row))
                    .collect();
                for _ in 0..count {
                    model.remove(from);
                }
                for (offset, row) in moved.into_iter().enumerate() {
                    model.insert(to + offset, row);
                }
            }
        }
    }
}

/// The carried rows of a patch, converted for a view, with the images the
/// property shows after it (one list per row).
pub struct Rows {
    pub values: Vec<Value>,
    pub keys: Vec<Vec<ui_image::Key>>,
}

impl Host {
    /// `Images::patch_rows` with `owner`'s image directories.
    pub(crate) fn patch_rows(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        ops: &Ops<'_>,
    ) -> std::result::Result<Option<Rows>, RpcError> {
        let roots = self.image_roots(owner);
        self.images.patch_rows(owner, &roots, view, property, ops)
    }
}

impl ui_image::Images {
    /// Convert the rows a patch carries for `owner`'s `view` (images loaded
    /// from `roots` and checked as for a whole value), and the images each
    /// row of the property shows after the ops, within the view's budget.
    /// `None`: the property cannot be patched in place (a row holds a
    /// Markdown document, or the images of its rows are not known).
    pub(crate) fn patch_rows(
        &mut self,
        owner: u64,
        roots: &[std::path::PathBuf],
        view: &str,
        property: &str,
        ops: &Ops<'_>,
    ) -> std::result::Result<Option<Rows>, RpcError> {
        if ops.rows.iter().any(|row| ui_image::has_markdown(row)) {
            return Ok(None);
        }
        let Some(mut keys) = self
            .row_keys(owner, view, property)
            .filter(|rows| rows.len() == ops.before)
            .cloned()
        else {
            // Not an array converted for this view, or one written since
            // while the view was closed: nothing is patched in place.
            return Ok(None);
        };
        let failure: RefCell<Option<RpcError>> = RefCell::new(None);
        let mut order: Vec<Option<ui_image::Key>> = Vec::new();
        let converted = ui_values::from_json_rows_with(&ops.rows, &mut |node, depth| {
            if depth == 0 || ui_image::reference(node).is_none() {
                return None;
            }
            let (path, side) = match ui_image::parts(node) {
                Ok(parts) => parts,
                Err(error) => {
                    let message = error.message.clone();
                    *failure.borrow_mut() = Some(error);
                    return Some(Err(anyhow::anyhow!(message)));
                }
            };
            Some(Ok(match self.load_within(owner, roots, path, side) {
                Ok((key, image)) => {
                    order.push(Some(key));
                    (Value::Image(image), 0)
                }
                Err(error) => {
                    diag!("ui.image {path}: not shown: {}", error.message);
                    order.push(None);
                    (Value::Image(slint::Image::default()), 0)
                }
            }))
        });
        let values = match converted {
            Ok(values) => values,
            Err(error) => return Err(failure.into_inner().unwrap_or_else(|| error.into())),
        };
        let mut order = order.into_iter();
        let carried: Vec<Vec<ui_image::Key>> = ops
            .rows
            .iter()
            .map(|row| {
                order
                    .by_ref()
                    .take(ui_image::image_count(row))
                    .flatten()
                    .collect()
            })
            .collect();
        apply_vec(&mut keys, &ops.shapes, carried);
        let distinct: std::collections::BTreeSet<&ui_image::Key> = keys.iter().flatten().collect();
        if !distinct.is_empty() {
            let shown: usize = distinct.iter().map(|key| self.entry_bytes(key)).sum();
            if self.view_bytes(owner, view, property) + shown > ui_image::VIEW_BUDGET_BYTES {
                return Err(RpcError::new(
                    "limit",
                    "the images of one view exceed 32 MB of decoded pixels \
                     (use maxSide, or send fewer rows)",
                ));
            }
        }
        Ok(Some(Rows { values, keys }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ui_image::Images;
    use crate::ui::ui_values::{from_json, to_json};
    use serde_json::json;

    fn row(key: &str, selected: bool) -> Json {
        json!({"key": key, "selected": selected})
    }
    fn rows(keys: &[&str]) -> Json {
        Json::Array(keys.iter().map(|key| row(key, false)).collect())
    }
    fn model(value: &Value) -> &slint::ModelRc<Value> {
        let Value::Model(model) = value else {
            panic!("a model")
        };
        model
    }
    /// A wire value that outlives the Ops borrowing it.
    fn leak(value: Json) -> &'static Json {
        Box::leak(Box::new(value))
    }
    fn plain(rows: &[&Json]) -> Vec<Value> {
        ui_values::from_json_rows_with(rows, &mut |_, _| None).unwrap()
    }

    /// `ops` written into the live model converted from `before`, and into
    /// the JSON array the same way: both must give the same rows.
    fn patched(before: &Json, ops: &Json) -> (Value, Json) {
        let current = from_json(before).unwrap();
        let identity = model(&current).as_any() as *const dyn std::any::Any;
        let parsed = parse(ops, before.as_array().unwrap().len()).unwrap();
        let values = plain(&parsed.rows);
        assert!(applicable(&current, parsed.before, &parsed.shapes, &values));
        apply(&current, &parsed.shapes, values);
        let mut json = before.as_array().unwrap().clone();
        apply_vec(
            &mut json,
            &parsed.shapes,
            parsed.rows.iter().map(|row| (*row).clone()),
        );
        assert_eq!(model(&current).row_count(), parsed.len);
        assert!(std::ptr::eq(
            identity,
            model(&current).as_any() as *const dyn std::any::Any
        ));
        (current, Json::Array(json))
    }

    #[test]
    fn set_insert_remove_and_move_apply_in_place_and_in_order() {
        let ops = json!([
            {"op": "remove", "at": 1, "count": 1},
            {"op": "move", "from": 3, "count": 1, "to": 0},
            {"op": "insert", "at": 2, "rows": [row("x", false), row("y", false)]},
            {"op": "set", "at": 5, "row": row("d", true)},
        ]);
        let (current, json) = patched(&rows(&["a", "b", "c", "d", "e"]), &ops);
        assert_eq!(
            json,
            json!([
                row("e", false),
                row("a", false),
                row("x", false),
                row("y", false),
                row("c", false),
                row("d", true)
            ])
        );
        assert_eq!(to_json(&current).unwrap(), json);
        // A window of 10 rows shifted by 3 (a keyed diff's ops): three
        // rows go at the top, three come at the end, the rest stay.
        let before = rows(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]);
        let ops = json!([
            {"op": "remove", "at": 0, "count": 3},
            {"op": "insert", "at": 7, "rows": [row("10", false), row("11", false), row("12", false)]},
        ]);
        let (current, json) = patched(&before, &ops);
        assert_eq!(
            json,
            rows(&["3", "4", "5", "6", "7", "8", "9", "10", "11", "12"])
        );
        assert_eq!(to_json(&current).unwrap(), json);
        // A block moved down, then back.
        let ops = json!([
            {"op": "move", "from": 0, "count": 2, "to": 3},
            {"op": "move", "from": 3, "count": 2, "to": 0},
        ]);
        let (current, json) = patched(&rows(&["a", "b", "c", "d", "e"]), &ops);
        assert_eq!(json, rows(&["a", "b", "c", "d", "e"]));
        assert_eq!(to_json(&current).unwrap(), json);
        // Only the named rows are written: a row the patch does not name
        // keeps its value even where the stored copy differs.
        let current = from_json(&rows(&["a", "b"])).unwrap();
        model(&current).set_row_data(0, from_json(&row("a", true)).unwrap());
        let parsed = parse(
            leak(json!([{"op": "set", "at": 1, "row": row("b", true)}])),
            2,
        )
        .unwrap();
        let values = plain(&parsed.rows);
        apply(&current, &parsed.shapes, values);
        assert_eq!(
            to_json(&current).unwrap(),
            json!([row("a", true), row("b", true)])
        );
    }

    #[test]
    fn ops_are_checked_against_the_length_before_anything_changes() {
        let code = |ops: Json, len: usize| parse(&ops, len).unwrap_err().code;
        assert_eq!(
            code(json!([{"op": "set", "at": 2, "row": 1}]), 2),
            "invalid"
        );
        assert_eq!(code(json!([{"op": "set", "at": 0}]), 2), "invalid");
        assert_eq!(
            code(json!([{"op": "insert", "at": 3, "rows": [1]}]), 2),
            "invalid"
        );
        assert_eq!(
            code(json!([{"op": "insert", "at": 0, "rows": []}]), 2),
            "invalid"
        );
        assert_eq!(
            code(json!([{"op": "remove", "at": 1, "count": 2}]), 2),
            "invalid"
        );
        assert_eq!(
            code(json!([{"op": "remove", "at": 0, "count": 0}]), 2),
            "invalid"
        );
        assert_eq!(
            code(json!([{"op": "move", "from": 0, "count": 1, "to": 2}]), 2),
            "invalid"
        );
        assert_eq!(code(json!([{"op": "swap", "at": 0}]), 2), "invalid");
        assert_eq!(
            code(json!([{"op": "set", "at": -1, "row": 1}]), 2),
            "invalid"
        );
        assert_eq!(code(json!({"op": "set"}), 2), "invalid");
        // Each op sees the length the ones before left.
        assert!(
            parse(
                &json!([{"op": "remove", "at": 0, "count": 2}, {"op": "set", "at": 0, "row": 1}]),
                2
            )
            .is_err()
        );
        let ok = parse(
            leak(json!([{"op": "insert", "at": 2, "rows": [1, 2]}, {"op": "set", "at": 3, "row": 1}])),
            2,
        )
        .unwrap();
        assert_eq!((ok.before, ok.len, ok.rows.len()), (2, 4, 3));
        // Limits: ops per patch, rows of the result.
        let many = Json::Array(vec![
            json!({"op": "set", "at": 0, "row": 1});
            MAX_PATCH_OPS + 1
        ]);
        assert_eq!(code(many, 1), "limit");
        let grow = json!([{"op": "insert", "at": 0, "rows": [1, 2]}]);
        assert_eq!(code(grow, ui_values::MAX_ARRAY - 1), "limit");
        // The request: base, and a seq of at least 1.
        assert!(request(&json!({"base": 0, "seq": 1, "ops": []})).is_ok());
        assert_eq!(
            request(&json!({"base": 0, "seq": 0, "ops": []}))
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            request(&json!({"seq": 2, "ops": []})).unwrap_err().code,
            "invalid"
        );
        assert_eq!(stale().code, "stale");
    }

    #[test]
    fn a_patch_the_model_cannot_take_in_place_is_refused_whole() {
        let ops = json!([{"op": "set", "at": 0, "row": {"key": 1, "selected": false}}]);
        let current = from_json(&rows(&["a", "b"])).unwrap();
        let parsed = parse(&ops, 2).unwrap();
        // Another row type (a number for a string).
        assert!(!applicable(
            &current,
            2,
            &parsed.shapes,
            &plain(&parsed.rows)
        ));
        // Another field set.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": {"key": "a"}}])),
            2,
        )
        .unwrap();
        assert!(!applicable(
            &current,
            2,
            &parsed.shapes,
            &plain(&parsed.rows)
        ));
        // The live model is not the length the patch was checked against.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": row("a", true)}])),
            3,
        )
        .unwrap();
        assert!(!applicable(
            &current,
            3,
            &parsed.shapes,
            &plain(&parsed.rows)
        ));
        // An empty model does not show its row type.
        let empty = from_json(&json!([])).unwrap();
        let parsed = parse(
            leak(json!([{"op": "insert", "at": 0, "rows": [row("a", false)]}])),
            0,
        )
        .unwrap();
        assert!(!applicable(&empty, 0, &parsed.shapes, &plain(&parsed.rows)));
        // Not a model.
        assert!(!applicable(&Value::Number(1.), 0, &[], &[]));
        // Nothing to do on a model is fine.
        assert!(applicable(&empty, 0, &[], &[]));
        assert_eq!(to_json(&current).unwrap(), rows(&["a", "b"]), "untouched");
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![90; (width * height * 4) as usize])
            .unwrap();
        bytes
    }

    /// STORE-ROW-LOGO rows (a logo per row) patched: the images each row
    /// shows move with the rows, a removed row's image is no longer shown,
    /// and the view's 32 MB budget counts the result.
    #[test]
    fn rows_with_images_keep_their_images_in_step() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let roots = [root.clone()];
        let file = |name: &str, side: u32| {
            let path = root.join(name);
            std::fs::write(&path, png(side, side)).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let (a, b, c) = (file("a.png", 2), file("b.png", 3), file("c.png", 4));
        let logo =
            |key: &str, path: &str| json!({"key": key, "logo": {"$vestraImage": path, "ref": 1}});
        let mut images = Images::default();
        let (ka, _) = images.load(1, &roots, &a).unwrap();
        let (kb, _) = images.load(1, &roots, &b).unwrap();
        // As `prop_value` records a whole array of three rows.
        images.patched_rows(
            1,
            "$mount",
            "rows",
            vec![vec![ka.clone()], vec![], vec![kb.clone()]],
        );
        assert_eq!(images.view_bytes(1, "$mount", "other"), (4 + 9) * 4);
        let ops = json!([
            {"op": "remove", "at": 0, "count": 1},
            {"op": "insert", "at": 1, "rows": [logo("c", &c)]},
            {"op": "set", "at": 0, "row": logo("x", &a)},
        ]);
        let parsed = parse(&ops, 3).unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed)
            .unwrap()
            .unwrap();
        let kc = images.load(1, &roots, &c).unwrap().0;
        assert_eq!(
            patch.keys,
            vec![vec![ka.clone()], vec![kc.clone()], vec![kb.clone()]]
        );
        let Value::Struct(first) = &patch.values[1] else {
            panic!("a row")
        };
        assert!(matches!(first.get_field("logo"), Some(Value::Image(_))));
        // Nothing is recorded until the patch is applied.
        assert_eq!(images.row_keys(1, "$mount", "rows").unwrap().len(), 3);
        images.patched_rows(1, "$mount", "rows", patch.keys);
        assert_eq!(images.view_bytes(1, "$mount", "other"), (4 + 9 + 16) * 4);
        // A row without its image: the image is no longer shown.
        let parsed = parse(leak(json!([{"op": "remove", "at": 1, "count": 2}])), 3).unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed)
            .unwrap()
            .unwrap();
        images.patched_rows(1, "$mount", "rows", patch.keys);
        assert_eq!(images.view_bytes(1, "$mount", "other"), 4 * 4);
        // A file that fails its checks is an empty image, as in a whole value.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": logo("y", "/gone.png")}])),
            1,
        )
        .unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed)
            .unwrap()
            .unwrap();
        assert_eq!(patch.keys, vec![Vec::<ui_image::Key>::new()]);
        // Over the view's budget: refused before anything is recorded.
        let big = file("big.png", 2900);
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": logo("z", &big)}])),
            1,
        )
        .unwrap();
        let error = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed)
            .err()
            .unwrap();
        assert_eq!(error.code, "limit");
        assert_eq!(
            images.row_keys(1, "$mount", "rows").unwrap(),
            &vec![vec![ka]]
        );
        // A malformed reference refuses the patch.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": {"key": "q", "logo": {"$vestraImage": 3, "ref": 1}}}])),
            1,
        )
        .unwrap();
        assert_eq!(
            images
                .patch_rows(1, &roots, "$mount", "rows", &parsed)
                .err()
                .unwrap()
                .code,
            "invalid"
        );
        // Not patched in place: a Markdown row, a property whose rows are
        // not known, a record of another length.
        let doc = json!({"key": "m", "body": {"$vestraMarkdown": {"text": "x"}, "ref": 1}});
        let parsed = parse(leak(json!([{"op": "set", "at": 0, "row": doc}])), 1).unwrap();
        assert!(
            images
                .patch_rows(1, &roots, "$mount", "rows", &parsed)
                .unwrap()
                .is_none()
        );
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": row("a", true)}])),
            1,
        )
        .unwrap();
        assert!(
            images
                .patch_rows(1, &roots, "$mount", "other", &parsed)
                .unwrap()
                .is_none()
        );
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": row("a", true)}])),
            2,
        )
        .unwrap();
        assert!(
            images
                .patch_rows(1, &roots, "$mount", "rows", &parsed)
                .unwrap()
                .is_none()
        );
        // Rows with images are written into the live model like others.
        let current =
            Value::Model(std::rc::Rc::new(VecModel::from(plain(&[&row("a", false)]))).into());
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": row("b", true)}])),
            1,
        )
        .unwrap();
        assert!(applicable(
            &current,
            1,
            &parsed.shapes,
            &plain(&parsed.rows)
        ));
    }

    #[test]
    fn write_records_start_at_the_initial_properties_and_go_with_the_view() {
        let mut images = Images::default();
        assert_eq!(images.seq(1, "w:a", "rows"), 0);
        images.set_seq(1, "w:a", "rows", 7);
        images.set_seq(1, "w:b", "rows", 3);
        images.set_seq(2, "w:a", "rows", 5);
        assert_eq!(images.seq(1, "w:a", "rows"), 7);
        images.reset_writes(1, "w:a");
        assert_eq!(images.seq(1, "w:a", "rows"), 0);
        assert_eq!(images.seq(1, "w:b", "rows"), 3);
        images.release_view(1, "w:b");
        assert_eq!(images.seq(1, "w:b", "rows"), 0);
        images.release_owner(2);
        assert_eq!(images.seq(2, "w:a", "rows"), 0);
    }

    #[test]
    fn wire_rows_count_their_images_and_documents() {
        let image = json!({"$vestraImage": "/a.png", "ref": 1});
        let doc = json!({"$vestraMarkdown": {"text": "x"}, "ref": 2});
        assert_eq!(
            ui_image::image_count(&json!({"a": image, "b": [image, {"c": image}]})),
            3
        );
        assert_eq!(ui_image::image_count(&json!({"a": 1})), 0);
        assert!(ui_image::has_markdown(&json!([{"a": [doc]}])));
        assert!(!ui_image::has_markdown(&json!([{"a": image}])));
    }
}
