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
use crate::ui::ui_rows::{self, Row};
use crate::ui::{ui_image, ui_values};
use serde_json::Value as Json;
use slint::Model;
use slint_interpreter::Value;

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

/// `stored` (a property's array as JSON) with checked ops applied.
pub fn patched(stored: &[Json], ops: &Ops<'_>) -> Json {
    let mut rows = stored.to_vec();
    apply_vec(
        &mut rows,
        &ops.shapes,
        ops.rows.iter().map(|row| (*row).clone()),
    );
    Json::Array(rows)
}

/// `current` (a live property) can take the patch in place: a host-owned
/// model (`ui_rows::Rows`) of `len` rows, no Markdown documents in its rows,
/// and carried rows of the type of its first row (Slint checks the type of
/// an installed model, not of a written row). An empty model does not show
/// its row type.
pub fn applicable(current: &Value, len: usize, shapes: &[Shape], rows: &Rows) -> bool {
    let Value::Model(model) = current else {
        return false;
    };
    let Some(model) = ui_rows::rows(model) else {
        return false;
    };
    if model.row_count() != len {
        return false;
    }
    if shapes.is_empty() {
        return true;
    }
    let Some(sample) = model.row(0) else {
        return rows.values.is_empty();
    };
    if matches!(&sample, Row::Value(value) if ui_values::has_markdown(value)) {
        return false;
    }
    rows.values
        .iter()
        .all(|row| model.fits(&sample, row, &rows.images))
}

/// Write an `applicable` patch into `current`: each op is one model
/// operation, a run of rows inserted, removed or moved with one
/// notification each.
pub fn apply(current: &Value, shapes: &[Shape], values: Vec<Row>, images: &ui_rows::Images) {
    let Value::Model(model) = current else {
        return;
    };
    let Some(model) = ui_rows::rows(model) else {
        return;
    };
    model.add_images(images);
    let mut carried = values.into_iter();
    for shape in shapes {
        match *shape {
            Shape::Set(at) => {
                if let Some(row) = carried.next() {
                    model.set_row(at, row);
                }
            }
            Shape::Insert(at, count) => {
                model.insert_rows(at, carried.by_ref().take(count));
            }
            Shape::Remove(at, count) => {
                model.remove_rows(at, count);
            }
            Shape::Move { from, count, to } => {
                let moved = model.remove_rows(from, count);
                model.insert_rows(to, moved);
            }
        }
    }
    // Pictures of rows replaced or removed, and of carried rows not kept.
    model.prune();
}

/// The carried rows of a patch, converted for a view (plain rows, their
/// pictures), and the rows the property shows after it (`ui_image::Row`:
/// images, value totals).
#[derive(Clone)]
pub struct Rows {
    pub values: Vec<Row>,
    pub images: ui_rows::Images,
    pub written: Vec<ui_image::Row>,
}

/// What a tool window's patch did (a window keeps no stored value).
#[derive(Debug, PartialEq)]
pub enum InWindow {
    /// Written into the live model.
    Applied,
    /// Not in place: the shown rows with the ops applied, for the caller
    /// to write whole (no round trip to the plugin).
    Whole(Json),
}

impl Host {
    /// `Images::patch_window` with `owner`'s image directories.
    pub(crate) fn patch_window(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        current: &Value,
        ops: &Json,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<InWindow, RpcError> {
        let roots = self.core.image_roots(owner);
        self.images
            .patch_window(owner, &roots, view, property, current, ops, prepared)
    }
    /// `Images::patch_rows` with `owner`'s image directories.
    pub(crate) fn patch_rows(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        ops: &Ops<'_>,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<Option<Rows>, RpcError> {
        let roots = self.core.image_roots(owner);
        self.images
            .patch_rows(owner, &roots, view, property, ops, prepared)
    }
}

/// A whole array value larger than one request arrives in parts (player
/// 0.6.0, SDK 1.5.46; doc/architecture/large-lists.md §4.2): a whole write
/// of its first rows, then `insert` patches of the rest, each applying on
/// the one before; every part but the last carries `more: true`. The parts
/// are held here, not shown, and the last one writes the whole value at
/// once, so a list already shown is reconciled once (its rows, scroll
/// position and selection stay) instead of shrinking to the first part.
#[derive(Debug, Default)]
pub struct Parts {
    /// The `seq` of the last part (the next one's `base`).
    pub seq: u64,
    /// The rows so far, wire form (for the stored properties).
    pub rows: Vec<Json>,
    /// The rows so far, converted.
    pub plain: ui_values::PlainRows,
}

/// What `Host::parts` did with a write.
#[derive(Debug)]
pub enum Parted {
    /// Not a part: the caller writes as usual.
    Not,
    /// A part held: nothing to show yet (the request is answered `null`).
    Held,
    /// The last part: the whole value, its rows converted, and its `seq`.
    Done(Json, ui_values::PlainRows, u64),
}

/// `plain` with `shapes` applied, taking the carried rows from `carried`
/// (their image references merged into `plain`'s).
fn apply_plain(plain: &mut ui_values::PlainRows, shapes: &[Shape], carried: ui_values::PlainRows) {
    let mut index: std::collections::HashMap<std::sync::Arc<ui_rows::ImageRef>, u32> = plain
        .images
        .iter()
        .enumerate()
        .map(|(i, image)| (image.clone(), u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    let mapped: Vec<u32> = carried
        .images
        .into_iter()
        .map(|image| {
            let next = u32::try_from(plain.images.len()).unwrap_or(u32::MAX);
            *index.entry(image.clone()).or_insert_with(|| {
                plain.images.push(image);
                next
            })
        })
        .collect();
    let row_images = carried.row_images.into_iter().map(|images| {
        images
            .iter()
            .map(|i| mapped.get(*i as usize).copied().unwrap_or(u32::MAX))
            .collect::<Box<[u32]>>()
    });
    apply_vec(&mut plain.rows, shapes, carried.rows);
    apply_vec(&mut plain.costs, shapes, carried.costs);
    apply_vec(&mut plain.row_images, shapes, row_images);
}

/// The rows of a part, converted (on the reader thread when it was large).
fn part_rows(
    rows: &[&Json],
    prepared: Option<&ui_values::PlainRows>,
) -> std::result::Result<ui_values::PlainRows, RpcError> {
    if let Some(prepared) = prepared.filter(|prepared| prepared.rows.len() == rows.len()) {
        return Ok(prepared.clone());
    }
    match ui_values::plain_rows(rows) {
        Ok(plain) => Ok(plain),
        Err(ui_values::NotPlain::Markdown) => Err(RpcError::invalid(
            "a value sent in parts holds no Markdown documents",
        )),
        Err(ui_values::NotPlain::Image(error)) => Err(error),
        Err(ui_values::NotPlain::Refused(error)) => Err(error.into()),
    }
}

impl ui_image::Images {
    /// `Host::parts` (images are loaded when the last part is written).
    pub(crate) fn parts(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        params: &Json,
        prepared: Option<&ui_values::Prepared>,
    ) -> std::result::Result<Parted, RpcError> {
        let more = params.get("more").and_then(Json::as_bool) == Some(true);
        let held = self.take_parts(owner, view, property);
        let result = self.part(params, more, held, prepared, property);
        match result {
            Ok((Parted::Held, Some(parts))) => {
                self.put_parts(owner, view, property, parts);
                Ok(Parted::Held)
            }
            Ok((parted, _)) => Ok(parted),
            Err(error) => Err(error),
        }
    }
    fn part(
        &self,
        params: &Json,
        more: bool,
        held: Option<Parts>,
        prepared: Option<&ui_values::Prepared>,
        property: &str,
    ) -> std::result::Result<(Parted, Option<Parts>), RpcError> {
        let total = |parts: &Parts| {
            if parts.plain.rows.len() > ui_values::MAX_ARRAY {
                return Err(RpcError::new(
                    "limit",
                    format!("{property}: UI array exceeds {} rows", ui_values::MAX_ARRAY),
                ));
            }
            ui_values::check_totals(&parts.plain.costs)
                .map_err(|error| RpcError::from(error).for_property(property))
        };
        match (params.get("patch"), held) {
            // The first part: the value's first rows.
            (None, _) if more => {
                let value = match prepared.and_then(|prepared| prepared.whole.as_ref()) {
                    Some(value) => value,
                    None => &params["value"],
                };
                let rows = value
                    .as_array()
                    .ok_or_else(|| RpcError::invalid("a value sent in parts is an array"))?;
                let refs: Vec<&Json> = rows.iter().collect();
                let plain = part_rows(&refs, prepared.map(|prepared| &prepared.rows))?;
                let parts = Parts {
                    seq: params["seq"].as_u64().unwrap_or(0),
                    rows: rows.clone(),
                    plain,
                };
                total(&parts)?;
                Ok((Parted::Held, Some(parts)))
            }
            // A further part, on the one before.
            (Some(patch), Some(mut parts)) if patch["base"].as_u64() == Some(parts.seq) => {
                let (_, seq, ops) = request(patch)?;
                let ops = parse(ops, parts.rows.len())?;
                let carried = part_rows(&ops.rows, prepared.map(|prepared| &prepared.rows))?;
                let wire: Vec<Json> = match prepared {
                    Some(prepared) if prepared.carried.len() == ops.rows.len() => {
                        prepared.carried.clone()
                    }
                    _ => ops.rows.iter().map(|row| (*row).clone()).collect(),
                };
                apply_vec(&mut parts.rows, &ops.shapes, wire);
                apply_plain(&mut parts.plain, &ops.shapes, carried);
                parts.seq = seq;
                total(&parts)?;
                if more {
                    return Ok((Parted::Held, Some(parts)));
                }
                Ok((
                    Parted::Done(Json::Array(parts.rows), parts.plain, seq),
                    None,
                ))
            }
            // A part with nothing held (or held for another base): the
            // sequence broke (a part refused, the view reopened).
            (Some(_), _) if more => Err(stale()),
            // Any other write ends what was held.
            _ => Ok((Parted::Not, None)),
        }
    }
}

impl Host {
    /// A write that is a part of a larger whole value (`Parts`): held, or
    /// at its last part the whole value to write as usual (`Parted::Done`).
    /// A part that does not apply on the one before is `stale`; a part over
    /// the limits is refused, and what was held is dropped either way.
    pub(crate) fn parts(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        params: &Json,
        prepared: Option<&ui_values::Prepared>,
    ) -> std::result::Result<Parted, RpcError> {
        self.images.parts(owner, view, property, params, prepared)
    }
}
impl ui_image::Images {
    /// A row patch of a tool window's property shown as `current`. The ops
    /// apply on the rows the base write left: known only for an array the
    /// view converted, and only while its model still shows that many rows;
    /// otherwise (the write never reached the model) `stale`, and the SDK
    /// sends the whole value. Rows the model cannot take in place (a row of
    /// another shape than the first, an empty model, a document) are
    /// returned whole; rows holding images do not convert back: `stale`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn patch_window(
        &mut self,
        owner: u64,
        roots: &[std::path::PathBuf],
        view: &str,
        property: &str,
        current: &Value,
        ops: &Json,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<InWindow, RpcError> {
        let len = self
            .written_rows(owner, view, property)
            .map(Vec::len)
            .ok_or_else(stale)?;
        match current {
            Value::Model(model) if model.row_count() == len => {}
            _ => return Err(stale()),
        }
        let ops = parse(ops, len)?;
        let rows = self
            .patch_rows(owner, roots, view, property, &ops, prepared)?
            .filter(|rows| applicable(current, ops.before, &ops.shapes, rows));
        if let Some(rows) = rows {
            apply(current, &ops.shapes, rows.values, &rows.images);
            self.patched_rows(owner, view, property, rows.written);
            return Ok(InWindow::Applied);
        }
        let Ok(Json::Array(shown)) = ui_values::to_json(current) else {
            return Err(stale());
        };
        Ok(InWindow::Whole(patched(&shown, &ops)))
    }
    /// Convert the rows a patch carries for `owner`'s `view` to plain rows
    /// (`prepared` on the plugin's reader thread, or here; images loaded
    /// from `roots` once per reference and checked as for a whole value),
    /// and the rows the property shows after the ops: their images within
    /// the view's budget, the whole array within the value totals (the
    /// rows it keeps count too). `None`: the property cannot be patched in
    /// place (a row holds a Markdown document, or the property's rows are
    /// not known).
    pub(crate) fn patch_rows(
        &mut self,
        owner: u64,
        roots: &[std::path::PathBuf],
        view: &str,
        property: &str,
        ops: &Ops<'_>,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<Option<Rows>, RpcError> {
        let prepared = prepared.filter(|prepared| prepared.rows.len() == ops.rows.len());
        if prepared.is_none() && ops.rows.iter().any(|row| ui_image::has_markdown(row)) {
            return Ok(None);
        }
        let Some(mut written) = self
            .written_rows(owner, view, property)
            .filter(|rows| rows.len() == ops.before)
            .cloned()
        else {
            // Not an array converted for this view, or one written since
            // while the view was closed: nothing is patched in place.
            return Ok(None);
        };
        let converted;
        let plain = match prepared {
            Some(prepared) => prepared,
            None => {
                converted = match ui_values::plain_rows(&ops.rows) {
                    Ok(plain) => plain,
                    Err(ui_values::NotPlain::Markdown) => return Ok(None),
                    Err(ui_values::NotPlain::Image(error)) => return Err(error),
                    Err(ui_values::NotPlain::Refused(error)) => return Err(error.into()),
                };
                &converted
            }
        };
        let (pictures, keys) = self.load_refs(owner, roots, &plain.images);
        let carried: Vec<ui_image::Row> = plain
            .row_images
            .iter()
            .zip(&plain.costs)
            .map(|(images, cost)| ui_image::Row {
                keys: images
                    .iter()
                    .filter_map(|index| keys.get(*index as usize).cloned().flatten())
                    .collect(),
                cost: *cost,
            })
            .collect();
        apply_vec(&mut written, &ops.shapes, carried);
        ui_values::check_totals(written.iter().map(|row| &row.cost))
            .map_err(|error| RpcError::from(error).for_property(property))?;
        let distinct: std::collections::BTreeSet<&ui_image::Key> =
            written.iter().flat_map(|row| &row.keys).collect();
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
        Ok(Some(Rows {
            values: plain.rows.iter().cloned().map(Row::Plain).collect(),
            images: pictures,
            written,
        }))
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
    /// Carried rows as `patch_rows` converts them (no images).
    fn plain(rows: &[&Json]) -> Rows {
        let plain = ui_values::plain_rows(rows).unwrap();
        Rows {
            values: plain.rows.into_iter().map(Row::Plain).collect(),
            images: ui_rows::Images::default(),
            written: Vec::new(),
        }
    }
    /// The images of each recorded row.
    fn keys(rows: &[ui_image::Row]) -> Vec<Vec<ui_image::Key>> {
        rows.iter().map(|row| row.keys.clone()).collect()
    }
    /// A record of `rows` (wire form) as `prop_value` leaves it, no images.
    fn record(images: &mut Images, view: &str, rows: &Json) {
        let rows = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| ui_image::Row {
                keys: Vec::new(),
                cost: ui_values::cost(row),
            })
            .collect();
        images.patched_rows(1, view, "rows", rows);
    }

    /// `ops` written into the live model converted from `before`, and into
    /// the JSON array the same way: both must give the same rows.
    fn patched(before: &Json, ops: &Json) -> (Value, Json) {
        let current = from_json(before).unwrap();
        let identity = model(&current).as_any() as *const dyn std::any::Any;
        let parsed = parse(ops, before.as_array().unwrap().len()).unwrap();
        let values = plain(&parsed.rows);
        assert!(applicable(&current, parsed.before, &parsed.shapes, &values));
        apply(&current, &parsed.shapes, values.values, &values.images);
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
        // A run removed from the middle (back to front), rows put in the
        // middle and appended at the end (one `extend`).
        let ops = json!([
            {"op": "remove", "at": 2, "count": 3},
            {"op": "insert", "at": 1, "rows": [row("x", false), row("y", false)]},
            {"op": "insert", "at": 6, "rows": [row("z", false)]},
        ]);
        let (current, json) = patched(&rows(&["0", "1", "2", "3", "4", "5", "6"]), &ops);
        assert_eq!(json, rows(&["0", "x", "y", "1", "5", "6", "z"]));
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
        apply(&current, &parsed.shapes, values.values, &values.images);
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
        assert!(!applicable(&Value::Number(1.), 0, &[], &plain(&[])));
        // Nothing to do on a model is fine.
        assert!(applicable(&empty, 0, &[], &plain(&[])));
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
            [vec![ka.clone()], vec![], vec![kb.clone()]]
                .into_iter()
                .map(|keys| ui_image::Row {
                    keys,
                    cost: ui_values::Cost::default(),
                })
                .collect(),
        );
        assert_eq!(images.view_bytes(1, "$mount", "other"), (4 + 9) * 4);
        let ops = json!([
            {"op": "remove", "at": 0, "count": 1},
            {"op": "insert", "at": 1, "rows": [logo("c", &c)]},
            {"op": "set", "at": 0, "row": logo("x", &a)},
        ]);
        let parsed = parse(&ops, 3).unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
            .unwrap()
            .unwrap();
        let kc = images.load(1, &roots, &c).unwrap().0;
        assert_eq!(
            keys(&patch.written),
            vec![vec![ka.clone()], vec![kc.clone()], vec![kb.clone()]]
        );
        // The inserted row (c, 4×4): its logo is a picture of the patch's own.
        let shown = ui_rows::Rows::from_rows(vec![patch.values[0].clone()], patch.images.clone());
        let Some(Value::Struct(first)) = shown.row_data(0) else {
            panic!("a row")
        };
        let Some(Value::Image(picture)) = first.get_field("logo") else {
            panic!("an image")
        };
        assert_eq!((picture.size().width, picture.size().height), (4, 4));
        // Nothing is recorded until the patch is applied.
        assert_eq!(images.written_rows(1, "$mount", "rows").unwrap().len(), 3);
        images.patched_rows(1, "$mount", "rows", patch.written);
        assert_eq!(images.view_bytes(1, "$mount", "other"), (4 + 9 + 16) * 4);
        // A row without its image: the image is no longer shown.
        let parsed = parse(leak(json!([{"op": "remove", "at": 1, "count": 2}])), 3).unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
            .unwrap()
            .unwrap();
        images.patched_rows(1, "$mount", "rows", patch.written);
        assert_eq!(images.view_bytes(1, "$mount", "other"), 4 * 4);
        // A file that fails its checks is an empty image, as in a whole value.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": logo("y", "/gone.png")}])),
            1,
        )
        .unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
            .unwrap()
            .unwrap();
        assert_eq!(keys(&patch.written), vec![Vec::<ui_image::Key>::new()]);
        // Over the view's budget: refused before anything is recorded.
        let big = file("big.png", 2900);
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": logo("z", &big)}])),
            1,
        )
        .unwrap();
        let error = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
            .err()
            .unwrap();
        assert_eq!(error.code, "limit");
        assert_eq!(
            keys(images.written_rows(1, "$mount", "rows").unwrap()),
            vec![vec![ka]]
        );
        // A malformed reference refuses the patch.
        let parsed = parse(
            leak(json!([{"op": "set", "at": 0, "row": {"key": "q", "logo": {"$vestraImage": 3, "ref": 1}}}])),
            1,
        )
        .unwrap();
        assert_eq!(
            images
                .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
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
                .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
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
                .patch_rows(1, &roots, "$mount", "other", &parsed, None)
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
                .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
                .unwrap()
                .is_none()
        );
        // Rows with images are written into the live model like others.
        let current = Value::Model(
            std::rc::Rc::new(ui_rows::Rows::from_rows(
                plain(&[&row("a", false)]).values,
                ui_rows::Images::default(),
            ))
            .into(),
        );
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

    /// Each patch's rows are within the limits, but the rows it keeps count
    /// too: successive patches cannot grow the array past the totals (the
    /// stored value would then fail to convert when the view is rebuilt).
    #[test]
    fn successive_patches_are_checked_against_the_patched_arrays_totals() {
        let big = "x".repeat(ui_values::MAX_STRING_BYTES);
        let full = ui_values::MAX_TEXT_BYTES / ui_values::MAX_STRING_BYTES;
        let mut images = Images::default();
        record(&mut images, "$mount", &json!([]));
        for len in 0..full {
            let ops = leak(json!([{"op": "insert", "at": len, "rows": [big]}]));
            let patch = images
                .patch_rows(1, &[], "$mount", "rows", &parse(ops, len).unwrap(), None)
                .unwrap()
                .unwrap();
            images.patched_rows(1, "$mount", "rows", patch.written);
        }
        let whole = Json::Array(vec![Json::String(big.clone()); full]);
        assert!(
            ui_values::check(&whole).is_ok(),
            "exactly at the text total"
        );
        let ops = leak(json!([{"op": "insert", "at": 0, "rows": ["y"]}]));
        let error = images
            .patch_rows(1, &[], "$mount", "rows", &parse(ops, full).unwrap(), None)
            .err()
            .unwrap();
        assert_eq!(error.code, "limit");
        assert!(
            error.message.starts_with("rows: UI value text exceeds"),
            "{}",
            error.message
        );
        assert_eq!(
            images.written_rows(1, "$mount", "rows").unwrap().len(),
            full
        );
        // Replacing a row by a smaller one keeps the total down: fine.
        let ops = leak(json!([{"op": "set", "at": 0, "row": "y"}]));
        assert!(
            images
                .patch_rows(1, &[], "$mount", "rows", &parse(ops, full).unwrap(), None)
                .is_ok()
        );
        // Values: 1 + 1024 rows of 1023 values, and a row that fills the
        // total exactly; one value more is over.
        let wide = Json::Array(vec![Json::Null; 1022]);
        record(&mut images, "w:a", &Json::Array(vec![wide.clone(); 1024]));
        let room = ui_values::MAX_NODES - 1 - 1024 * 1023;
        let ops = leak(json!([{"op": "insert", "at": 1024, "rows": [vec![Json::Null; room - 1]]}]));
        let parsed = parse(ops, 1024).unwrap();
        assert!(
            images
                .patch_rows(1, &[], "w:a", "rows", &parsed, None)
                .is_ok(),
            "exactly the values total"
        );
        let ops = leak(json!([{"op": "insert", "at": 1024, "rows": [vec![Json::Null; room]]}]));
        let error = images
            .patch_rows(1, &[], "w:a", "rows", &parse(ops, 1024).unwrap(), None)
            .err()
            .unwrap();
        assert_eq!(error.code, "limit");
        assert!(
            error
                .message
                .contains(&format!("{} total nodes", ui_values::MAX_NODES)),
            "{}",
            error.message
        );
    }

    /// `cost` counts as `check` does: the totals agree with a whole check
    /// at both limits, for images and documents too.
    #[test]
    fn row_costs_agree_with_the_whole_value_check() {
        let image = json!({"$vestraImage": "/a.png", "ref": 1});
        let doc = json!({"$vestraMarkdown": {"text": "héllo"}, "ref": 2});
        let row = json!({"key": "a", "logo": image, "body": doc, "tags": ["x", null, 2.5]});
        assert_eq!(
            ui_values::cost(&row),
            ui_values::Cost {
                nodes: 8,
                text: 3 + 1 + 4 + 4 + 6 + 4 + 1
            }
        );
        let agree = |rows: Vec<Json>| {
            let costs: Vec<_> = rows.iter().map(ui_values::cost).collect();
            assert_eq!(
                ui_values::check_totals(&costs).is_ok(),
                ui_values::check(&Json::Array(rows)).is_ok()
            );
        };
        // 1 + 4 × 4095 + 3 = 16384 values, then one more.
        let row = Json::Array(vec![Json::Null; 4094]);
        for extra in [2, 3] {
            let mut rows = vec![row.clone(); 4];
            rows.push(Json::Array(vec![Json::Null; extra]));
            agree(rows);
        }
        let text = "x".repeat(ui_values::MAX_STRING_BYTES);
        for extra in ["", "y"] {
            let mut rows = vec![Json::String(text.clone()); 15];
            rows.push(json!({ "k": "x".repeat(ui_values::MAX_STRING_BYTES - 1) + extra }));
            agree(rows);
        }
    }

    /// A tool window keeps no stored value: a patch applies on the rows the
    /// base write left, while the model shows them; what the model cannot
    /// take in place comes back whole, not `stale`.
    #[test]
    fn window_patches_apply_on_the_recorded_rows_and_fall_back_whole() {
        let mut images = Images::default();
        let set = json!([{"op": "set", "at": 1, "row": row("b", true)}]);
        // No record of the property (not an array, or the write failed): stale.
        let current = from_json(&rows(&["a", "b"])).unwrap();
        let error = images
            .patch_window(1, &[], "w:a", "rows", &current, &set, None)
            .unwrap_err();
        assert_eq!(error.code, "stale");
        // Recorded, but the model does not show those rows (the property
        // kept its default): stale, not an index error.
        record(&mut images, "w:a", &rows(&["a", "b"]));
        let default = from_json(&json!([])).unwrap();
        let error = images
            .patch_window(1, &[], "w:a", "rows", &default, &set, None)
            .unwrap_err();
        assert_eq!(error.code, "stale");
        assert_eq!(
            images
                .patch_window(1, &[], "w:a", "rows", &Value::Number(1.), &set, None)
                .unwrap_err()
                .code,
            "stale"
        );
        // In place.
        assert_eq!(
            images
                .patch_window(1, &[], "w:a", "rows", &current, &set, None)
                .unwrap(),
            InWindow::Applied
        );
        assert_eq!(
            to_json(&current).unwrap(),
            json!([row("a", false), row("b", true)])
        );
        // A row with a field the first row omits: whole, with the ops applied.
        let ops = json!([{"op": "insert", "at": 2, "rows": [{"key": "c", "selected": false, "note": "n"}]}]);
        assert_eq!(
            images
                .patch_window(1, &[], "w:a", "rows", &current, &ops, None)
                .unwrap(),
            InWindow::Whole(json!([
                row("a", false),
                row("b", true),
                {"key": "c", "selected": false, "note": "n"}
            ]))
        );
        // An empty model: whole.
        record(&mut images, "w:b", &json!([]));
        let insert = json!([{"op": "insert", "at": 0, "rows": [row("a", false)]}]);
        assert_eq!(
            images
                .patch_window(1, &[], "w:b", "rows", &default, &insert, None)
                .unwrap(),
            InWindow::Whole(json!([row("a", false)]))
        );
        // Ops checked against the recorded length.
        assert_eq!(
            images
                .patch_window(
                    1,
                    &[],
                    "w:a",
                    "rows",
                    &current,
                    &json!([{"op": "set", "at": 2, "row": 1}]),
                    None,
                )
                .unwrap_err()
                .code,
            "invalid"
        );
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
