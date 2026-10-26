//! Bounded JSON ↔ Slint value conversion (host-internal).
//! Equal values retain live properties; same-length Host-owned models reconcile
//! changed rows. Structural changes replace models; Rc ownership stays with the view.
//! Values holding Markdown documents (`ui_markdown::Rows`, at any depth) are
//! updated in place as far as their shape allows: changed rows, rows added or
//! removed at the end, document rows diffed, so a growing chat message does
//! not rebuild the list.
//! This is data conversion only, with no layout or model-control protocol; runtime
//! images (`ui_image.rs`) are converted by the caller and only installed here.
//! Slint numbers use f64. Struct keys must already have Slint's canonical spelling
//! so conversion cannot silently rename or merge JSON object fields.
#![forbid(unsafe_code)]

use std::rc::Rc;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Number, Value as Json};
use slint::{Model, ModelRc, VecModel};
use slint_interpreter::{Struct, Value};

// The limits of one property value, in both directions (plugin writes and
// callback arguments). They bound what one write costs the UI thread: about
// 0.35 µs per value to convert and reconcile (release build, Apple M-series),
// so a full value of 16384 is about 6 ms. The text total cannot
// usefully grow past the 1 MiB protocol line (`plugins.rs`). The SDK repeats
// them (`UI_VALUE_LIMITS`) and its TestHost enforces them; the player's
// tests check that the two agree (src/tests/sdk/host-agreement.test.ts; SDK site, Reference: Limits).
pub const MAX_DEPTH: usize = 8;
pub const MAX_NODES: usize = 16384;
pub const MAX_ARRAY: usize = 4096;
pub const MAX_STRING_BYTES: usize = 64 * 1024;
/// Includes object field names, in both directions.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// A value over one of the limits above: the plugin sees code `limit`.
#[derive(Debug)]
pub struct OverLimit(String);
impl std::fmt::Display for OverLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for OverLimit {}
fn over(message: String) -> anyhow::Error {
    anyhow::Error::new(OverLimit(message))
}

#[derive(Default)]
struct Budget {
    nodes: usize,
    text: usize,
}

impl Budget {
    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(over(format!(
                "UI value depth exceeds {MAX_DEPTH} (root is depth 0)"
            )));
        }
        if self.nodes == MAX_NODES {
            return Err(over(format!("UI value exceeds {MAX_NODES} total nodes")));
        }
        self.nodes += 1;
        Ok(())
    }

    fn text(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_STRING_BYTES {
            return Err(over(format!(
                "UI string or field name exceeds {MAX_STRING_BYTES} bytes"
            )));
        }
        // Both operands have independent small bounds, so addition cannot wrap.
        if self.text + value.len() > MAX_TEXT_BYTES {
            return Err(over(format!(
                "UI value text exceeds {MAX_TEXT_BYTES} total bytes"
            )));
        }
        self.text += value.len();
        Ok(())
    }

    /// Text that is not a JSON string (a Markdown document): only the total counts.
    fn bulk_text(&mut self, bytes: usize) -> Result<()> {
        if self.text + bytes > MAX_TEXT_BYTES {
            return Err(over(format!(
                "UI value text exceeds {MAX_TEXT_BYTES} total bytes"
            )));
        }
        self.text += bytes;
        Ok(())
    }

    fn array(&self, count: usize) -> Result<()> {
        if count > MAX_ARRAY {
            return Err(over(format!("UI array exceeds {MAX_ARRAY} rows")));
        }
        if count > MAX_NODES - self.nodes {
            return Err(over("UI array cannot fit in remaining node budget".into()));
        }
        Ok(())
    }
}

/// Adopt writable JSON array defaults before the instance is shown. Otherwise
/// an equal first host update can leave a static Slint model alive until a user
/// focuses one of its repeated controls and a later update replaces that model.
/// Public output properties reject writes with AccessDenied and retain bindings.
pub fn own_default_models(
    instance: &slint_interpreter::ComponentInstance,
    global: Option<&str>,
) -> Result<()> {
    let definition = instance.definition();
    let properties = if let Some(global) = global {
        definition
            .global_properties(global)
            .context("UI global unavailable")?
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    } else {
        definition
            .properties()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    };
    for name in properties {
        let current = if let Some(global) = global {
            instance.get_global_property(global, &name)?
        } else {
            instance.get_property(&name)?
        };
        let Value::Model(model) = &current else {
            continue;
        };
        if model.as_any().is::<VecModel<Value>>() {
            continue;
        }
        // Models outside the bounded JSON bridge (e.g. image rows) keep their
        // native defaults; this helper does not widen the transport contract.
        let Ok(json) = to_json(&current) else {
            continue;
        };
        let owned = from_json(&json)?;
        let result = if let Some(global) = global {
            instance.set_global_property(global, &name, owned)
        } else {
            instance.set_property(&name, owned)
        };
        match result {
            Ok(()) => trace_reconciliation("adopt-default", model.row_count()),
            Err(slint_interpreter::SetPropertyError::AccessDenied) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Write `value` to a property of the instance (or of one of its globals). A
/// refusal names the property and, as far as the current value shows the
/// declared type, what does not match (`banner: banner.n is not a field of
/// the declared struct (args, code)`) instead of Slint's bare "wrong type".
pub fn set(
    instance: &slint_interpreter::ComponentInstance,
    global: Option<&str>,
    name: &str,
    value: Value,
) -> Result<()> {
    let shown = value.clone();
    let result = match global {
        Some(global) => instance.set_global_property(global, name, value),
        None => instance.set_property(name, value),
    };
    result.map_err(|error| {
        let current = match global {
            Some(global) => instance.get_global_property(global, name).ok(),
            None => instance.get_property(name).ok(),
        };
        anyhow::anyhow!(refusal(&error, name, current.as_ref(), &shown))
    })
}

/// Slint's refusal of `next` for property `name` (now `current`), in words.
pub fn refusal(
    error: &slint_interpreter::SetPropertyError,
    name: &str,
    current: Option<&Value>,
    next: &Value,
) -> String {
    use slint_interpreter::SetPropertyError as E;
    match error {
        E::NoSuchProperty => format!("{name}: no such property"),
        E::AccessDenied => format!("{name}: not writable (an out property)"),
        _ => {
            let detail = current
                .and_then(|current| mismatch(name, current, next))
                .unwrap_or_else(|| match current {
                    Some(current) => format!(
                        "{name} is {} that does not match the declared {} type",
                        kind(next),
                        kind(current)
                    ),
                    None => format!("{name} is {}", kind(next)),
                });
            format!("{name}: wrong type: {detail}")
        }
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Void => "null",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Bool(_) => "a bool",
        Value::Image(_) => "an image",
        Value::Model(_) => "an array",
        Value::Struct(_) => "a struct",
        Value::Brush(_) => "a brush",
        Value::EnumerationValue(..) => "an enum value",
        _ => "a value",
    }
}

/// The first place where `next` differs in shape from `current` (a value of
/// the declared type: its default or last value). An array whose current
/// value has no rows does not show its element type, so its rows are not
/// compared.
fn mismatch(path: &str, current: &Value, next: &Value) -> Option<String> {
    match (current, next) {
        (Value::Struct(declared), Value::Struct(given)) => {
            let mut fields: Vec<_> = given.iter().collect();
            fields.sort_by_key(|(name, _)| *name);
            fields.into_iter().find_map(|(field, value)| {
                let path = format!("{path}.{field}");
                match declared.get_field(field) {
                    Some(declared) => mismatch(&path, declared, value),
                    None => {
                        let mut names: Vec<_> = declared.iter().map(|(name, _)| name).collect();
                        names.sort_unstable();
                        Some(format!(
                            "{path} is not a field of the declared struct ({})",
                            names.join(", ")
                        ))
                    }
                }
            })
        }
        (Value::Model(declared), Value::Model(given)) => {
            let sample = declared.row_data(0)?;
            given
                .iter()
                .enumerate()
                .find_map(|(index, row)| mismatch(&format!("{path}[{index}]"), &sample, &row))
        }
        (declared, given) if std::mem::discriminant(declared) == std::mem::discriminant(given) => {
            None
        }
        (declared, given) => Some(format!(
            "{path} is {} where the declaration has {}",
            kind(given),
            kind(declared)
        )),
    }
}

/// Reconcile an actual live property before replacing it. Bridge-created models
/// with stable length update only changed rows, preserving focused repeater items.
/// False means the caller must install the new value (including structural change).
pub fn reconcile(current: &Value, next: &Value) -> Result<bool> {
    // Runtime images (`plugin.ui.image`) are not JSON: always install them,
    // and replace an image with whatever comes next.
    if matches!(current, Value::Image(_)) || matches!(next, Value::Image(_)) {
        return Ok(false);
    }
    if has_documents(current) || has_documents(next) {
        // Checked before anything changes: a different shape is installed
        // whole, so Slint checks its type.
        if !fits(current, next) {
            trace_reconciliation("replace", rows(current));
            return Ok(false);
        }
        trace_reconciliation("in-place", rows(current));
        return Ok(!merge(current, next).1);
    }
    // Markdown rows (`plugin.ui.markdown`) carry images and styled text:
    // not JSON, so they are always installed.
    let (Ok(current_json), Ok(next_json)) = (to_json(current), to_json(next)) else {
        return Ok(false);
    };
    if current_json == next_json {
        return Ok(true);
    }
    if let (Value::Model(current), Value::Model(next)) = (current, next)
        && current.row_count() == next.row_count()
        && let Some(rows) = current.as_any().downcast_ref::<VecModel<Value>>()
        && let (Json::Array(before), Json::Array(after)) = (&current_json, &next_json)
    {
        // Validate and collect before mutation, so invalid input cannot partly apply.
        // The rows were converted above; comparing them again here used to
        // double the cost of a write.
        let mut changes = Vec::new();
        for (index, (before, after)) in before.iter().zip(after).enumerate() {
            if before != after {
                changes.push((index, next.row_data(index).context("new UI row missing")?));
            }
        }
        trace_reconciliation("in-place", rows.row_count());
        for (index, value) in changes {
            rows.set_row_data(index, value);
        }
        return Ok(true);
    }
    if let Value::Model(model) = current {
        trace_reconciliation("replace", model.row_count());
    }
    Ok(false)
}
/// `reconcile` that also keeps models nested in rows: when `next` has the
/// shape of `current` it is written in place at every depth, so a row whose
/// field changes keeps the model it holds and the repeater elements drawn
/// from it. The context menu needs this: a changed level used to replace
/// its row elements, which loses a press in progress (no click) and the
/// pointer's hover until it moves again.
pub fn reconcile_nested(current: &Value, next: &Value) -> Result<bool> {
    if fits(current, next) {
        trace_reconciliation("in-place", rows(current));
        return Ok(!merge(current, next).1);
    }
    reconcile(current, next)
}
fn rows(value: &Value) -> usize {
    match value {
        Value::Model(model) => model.row_count(),
        _ => 0,
    }
}

fn documents(model: &ModelRc<Value>) -> Option<&crate::ui_markdown::Rows> {
    model.as_any().downcast_ref::<crate::ui_markdown::Rows>()
}

/// A Markdown document or (SDK 1.5.19) a runtime image anywhere below the
/// top of the value: not JSON, so written in place by `merge`.
fn has_documents(value: &Value) -> bool {
    match value {
        Value::Model(model) => {
            documents(model).is_some()
                || (0..model.row_count()).any(|row| {
                    model
                        .row_data(row)
                        .is_some_and(|v| matches!(v, Value::Image(_)) || has_documents(&v))
                })
        }
        Value::Struct(object) => object
            .iter()
            .any(|(_, v)| matches!(v, Value::Image(_)) || has_documents(v)),
        _ => false,
    }
}

/// `next` can be written into `current` in place without changing a Slint
/// type: documents for documents; host models whose shared rows fit and
/// whose added rows fit the first current row; structs with the same
/// fields; values of the same kind.
fn fits(current: &Value, next: &Value) -> bool {
    match (current, next) {
        (Value::Model(current), Value::Model(next)) => {
            match (documents(current), documents(next)) {
                (Some(_), Some(_)) => return true,
                (None, None) => {}
                _ => return false,
            }
            if !current.as_any().is::<VecModel<Value>>() {
                return false;
            }
            let shared = current.row_count().min(next.row_count());
            for row in 0..next.row_count() {
                let reference = if row < shared { row } else { 0 };
                let (Some(before), Some(after)) = (current.row_data(reference), next.row_data(row))
                else {
                    return false;
                };
                if !fits(&before, &after) {
                    return false;
                }
            }
            true
        }
        (Value::Struct(current), Value::Struct(next)) => {
            current.iter().count() == next.iter().count()
                && current.iter().all(|(name, before)| {
                    next.get_field(name)
                        .is_some_and(|after| fits(before, after))
                })
        }
        // Nested images (row logos) are leaves: replaced in place.
        (Value::Image(_), Value::Image(_)) => true,
        (Value::Image(_), _) | (_, Value::Image(_)) => false,
        (current, next) => current.value_type() == next.value_type(),
    }
}

/// Write `next` into `current` (they `fit`). Models change in place; the
/// result is the value to hold and whether the holder must store it (a
/// struct field or plain value changed).
fn merge(current: &Value, next: &Value) -> (Value, bool) {
    match (current, next) {
        (Value::Model(model), Value::Model(incoming)) => {
            if let (Some(rows), Some(new)) = (documents(model), documents(incoming)) {
                rows.update(new);
            } else if let Some(rows) = model.as_any().downcast_ref::<VecModel<Value>>() {
                let shared = rows.row_count().min(incoming.row_count());
                for index in 0..shared {
                    let (Some(before), Some(after)) =
                        (rows.row_data(index), incoming.row_data(index))
                    else {
                        continue;
                    };
                    let (value, store) = merge(&before, &after);
                    if store {
                        rows.set_row_data(index, value);
                    }
                }
                while rows.row_count() > incoming.row_count() {
                    rows.remove(rows.row_count() - 1);
                }
                for index in shared..incoming.row_count() {
                    if let Some(row) = incoming.row_data(index) {
                        rows.push(row);
                    }
                }
            }
            (current.clone(), false)
        }
        (Value::Struct(before), Value::Struct(after)) => {
            let mut merged = Struct::default();
            let mut store = false;
            for (name, value) in before.iter() {
                let Some(incoming) = after.get_field(name) else {
                    continue;
                };
                let (value, changed) = merge(value, incoming);
                store |= changed;
                merged.set_field(name.into(), value);
            }
            if store {
                (Value::Struct(merged), true)
            } else {
                (current.clone(), false)
            }
        }
        (current, next) if current == next => (current.clone(), false),
        (_, next) => (next.clone(), true),
    }
}

fn trace_reconciliation(mode: &str, rows: usize) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if std::env::var_os("VESTRA_UI_PROPERTY_TRACE").is_some()
        && COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 32
    {
        diag!("ui.model-update mode={mode} rows={rows}");
    }
}

pub fn from_json(value: &Json) -> Result<Value> {
    from_json_with(value, &mut |_, _| None)
}

/// A value converted by the caller instead (a Markdown document): the
/// value and its text bytes, or `None` for plain JSON.
pub type Hook<'a> = dyn FnMut(&Json, usize) -> Option<Result<(Value, usize)>> + 'a;

/// `from_json`, with `hook` converting the nodes it recognises (at any
/// depth; each is one node of the budget, its text counts toward the total).
pub fn from_json_with(value: &Json, hook: &mut Hook<'_>) -> Result<Value> {
    encode(value, 0, &mut Budget::default(), hook)
}

fn encode(value: &Json, depth: usize, budget: &mut Budget, hook: &mut Hook<'_>) -> Result<Value> {
    budget.node(depth)?;
    if let Some(converted) = hook(value, depth) {
        let (value, text) = converted?;
        budget.bulk_text(text)?;
        return Ok(value);
    }
    Ok(match value {
        Json::Null => Value::Void,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(value) => {
            let number = value
                .as_f64()
                .context("UI number cannot be represented as f64")?;
            if !number.is_finite() {
                bail!("UI number must be finite");
            }
            Value::Number(number)
        }
        Json::String(value) => {
            budget.text(value)?;
            Value::String(value.as_str().into())
        }
        Json::Array(values) => {
            budget.array(values.len())?;
            let mut rows = Vec::with_capacity(values.len());
            for value in values {
                rows.push(encode(value, depth + 1, budget, hook)?);
            }
            let model: ModelRc<Value> = Rc::new(VecModel::from(rows)).into();
            Value::Model(model)
        }
        Json::Object(fields) => {
            if fields.len() > MAX_NODES - budget.nodes {
                return Err(over("UI object cannot fit in remaining node budget".into()));
            }
            let mut object = Struct::default();
            for (name, value) in fields {
                budget.text(name)?;
                if object.get_field(name).is_some() {
                    bail!("UI object field names collide under Slint normalization");
                }
                let value = encode(value, depth + 1, budget, hook)?;
                object.set_field(name.clone(), value);
                if !object.iter().any(|(stored, _)| stored == name) {
                    bail!("UI object field {name:?} is not in Slint canonical spelling");
                }
            }
            Value::Struct(object)
        }
    })
}

/// Refuse now what `from_json` would refuse later: a property value (wire
/// form) the player stores and converts when a view is built (initial panel
/// and settings page properties, settings writes while the window is closed).
/// Images are checked when used and count as one node; a Markdown document
/// counts as one node and its text.
pub fn check(value: &Json) -> Result<()> {
    if crate::ui_image::reference(value).is_some() {
        return Ok(());
    }
    from_json_with(value, &mut |node, _| {
        if crate::ui_image::reference(node).is_some() {
            return Some(Ok((Value::Void, 0)));
        }
        let doc = crate::ui_markdown::reference(node)?;
        let text = doc.get("text").and_then(Json::as_str).map_or(0, str::len);
        Some(Ok((Value::Void, text)))
    })
    .map(drop)
}

/// Unsupported types, non-finite numbers, missing rows and models changing size
/// return errors. No error is replaced with JSON null; only genuine Void maps to null.
pub fn to_json(value: &Value) -> Result<Json> {
    decode(value, 0, &mut Budget::default())
}

fn decode(value: &Value, depth: usize, budget: &mut Budget) -> Result<Json> {
    budget.node(depth)?;
    Ok(match value {
        Value::Void => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Number(value) => {
            Json::Number(Number::from_f64(*value).context("UI number must be finite")?)
        }
        Value::String(value) => {
            budget.text(value.as_str())?;
            Json::String(value.to_string())
        }
        Value::Model(model) => {
            let count = model.row_count();
            budget.array(count)?;
            let mut rows = Vec::with_capacity(count);
            for row in 0..count {
                let value = model
                    .row_data(row)
                    .with_context(|| format!("UI model row {row} is unavailable"))?;
                rows.push(decode(&value, depth + 1, budget)?);
            }
            if model.row_count() != count {
                bail!("UI model changed size while reading");
            }
            Json::Array(rows)
        }
        Value::Struct(object) => {
            let mut fields = Map::new();
            for (name, value) in object.iter() {
                budget.text(name)?;
                fields.insert(name.into(), decode(value, depth + 1, budget)?);
            }
            Json::Object(fields)
        }
        _ => bail!("unsupported UI value type: {:?}", value.value_type()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn nested(depth: usize) -> Json {
        let mut value = Json::Null;
        for _ in 0..depth {
            value = Json::Array(vec![value]);
        }
        value
    }

    #[test]
    fn rows_preserve_identity_structures_and_own_their_models() {
        let json = json!({
            "rows": [
                {"rowId": "track-a", "title": "First", "selected": true},
                {"rowId": "track-b", "title": "Second", "selected": false}
            ],
            "empty": [], "missing": null
        });
        let value = from_json(&json).unwrap();
        let retained = value.clone();
        drop(value);
        assert_eq!(to_json(&retained).unwrap(), json);
        let Value::Struct(object) = retained else {
            panic!("object should become Struct")
        };
        let Some(Value::Model(rows)) = object.get_field("rows") else {
            panic!("array should become Model")
        };
        assert_eq!(rows.row_count(), 2);
    }

    #[test]
    fn exact_depth_and_array_limits_apply_both_directions() {
        let allowed = nested(MAX_DEPTH);
        let converted = from_json(&allowed).unwrap();
        assert_eq!(to_json(&converted).unwrap(), allowed);
        assert!(from_json(&nested(MAX_DEPTH + 1)).is_err());
        let array = Json::Array(vec![Json::Null; MAX_ARRAY]);
        let converted = from_json(&array).unwrap();
        assert_eq!(to_json(&converted).unwrap(), array);
        assert!(from_json(&Json::Array(vec![Json::Null; MAX_ARRAY + 1])).is_err());
        let oversized =
            Value::Model(Rc::new(VecModel::from(vec![Value::Void; MAX_ARRAY + 1])).into());
        assert!(to_json(&oversized).is_err());
        let mut deep = Value::Void;
        for _ in 0..=MAX_DEPTH {
            deep = Value::Model(Rc::new(VecModel::from(vec![deep])).into());
        }
        assert!(to_json(&deep).is_err());
    }

    #[test]
    fn nested_containers_share_one_total_node_budget() {
        // root + 4 arrays + (3 × 4096 + 4091) nulls = MAX_NODES.
        let last = MAX_NODES - 5 - 3 * MAX_ARRAY;
        let exact = Json::Array(vec![
            Json::Array(vec![Json::Null; MAX_ARRAY]),
            Json::Array(vec![Json::Null; MAX_ARRAY]),
            Json::Array(vec![Json::Null; MAX_ARRAY]),
            Json::Array(vec![Json::Null; last]),
        ]);
        let converted = from_json(&exact).unwrap();
        assert_eq!(to_json(&converted).unwrap(), exact);
        let mut too_many = exact;
        too_many.as_array_mut().unwrap()[3]
            .as_array_mut()
            .unwrap()
            .push(Json::Null);
        assert!(from_json(&too_many).is_err());
        let rows: Vec<_> = [MAX_ARRAY, MAX_ARRAY, MAX_ARRAY, last + 1]
            .into_iter()
            .map(|count| Value::Model(Rc::new(VecModel::from(vec![Value::Void; count])).into()))
            .collect();
        assert!(to_json(&Value::Model(Rc::new(VecModel::from(rows)).into())).is_err());
    }

    #[test]
    fn limit_failures_are_typed_and_check_matches_conversion() {
        let over = |value: &Json| {
            let error = from_json(value).unwrap_err();
            assert!(error.downcast_ref::<OverLimit>().is_some(), "{error}");
            let error = check(value).unwrap_err();
            assert!(error.downcast_ref::<OverLimit>().is_some(), "{error}");
            error.to_string()
        };
        // D1: a playlist row of 8 fields is 9 values.
        let row = json!({"a":"1","b":"","c":"","d":"","e":false,"f":false,"g":false,"h":false});
        let fits = Json::Array(vec![row.clone(); MAX_NODES / 9 - 1]);
        check(&fits).unwrap();
        let rows = Json::Array(vec![row; MAX_NODES / 9 + 1]);
        assert!(over(&rows).contains("node"));
        assert!(over(&Json::Array(vec![Json::Null; MAX_ARRAY + 1])).contains("rows"));
        assert!(over(&json!("x".repeat(MAX_STRING_BYTES + 1))).contains("bytes"));
        // Not a limit: a different code.
        assert!(
            from_json(&json!({"a_b": 1}))
                .unwrap_err()
                .downcast_ref::<OverLimit>()
                .is_none()
        );
        // An image is checked when used; a document is one value and its text.
        check(&json!({"$vestraImage": "/x.png", "ref": 1})).unwrap();
        // Nested images (SDK 1.5.19) are one node each, not an object.
        check(&json!([{"key": "a", "icon": {"$vestraImage": "/x.png", "ref": 2, "maxSide": 64}}]))
            .unwrap();
        let doc = |n: usize| json!({"$vestraMarkdown": {"text": "x".repeat(n)}, "ref": 1});
        check(&json!([doc(200_000), doc(200_000)])).unwrap();
        assert!(over(&Json::Array(vec![doc(200_000); 6])).contains("total bytes"));
    }

    #[test]
    fn strings_field_names_and_total_text_are_bounded() {
        let exact = "x".repeat(MAX_STRING_BYTES);
        assert_eq!(
            to_json(&from_json(&Json::String(exact.clone())).unwrap()).unwrap(),
            Json::String(exact.clone())
        );
        let too_long = format!("{exact}x");
        assert!(from_json(&Json::String(too_long.clone())).is_err());
        assert!(to_json(&Value::String(too_long.clone().into())).is_err());
        let mut object = Map::new();
        object.insert(too_long.clone(), Json::Null);
        assert!(from_json(&Json::Object(object)).is_err());
        let mut object = Struct::default();
        object.set_field(too_long, Value::Void);
        assert!(to_json(&Value::Struct(object)).is_err());
        let within = Json::Array(vec![Json::String(exact.clone()); 16]);
        assert!(from_json(&within).is_ok());
        let over = Json::Array(vec![Json::String(exact.clone()); 17]);
        assert!(from_json(&over).is_err());
        let model =
            Value::Model(Rc::new(VecModel::from(vec![Value::String(exact.into()); 17])).into());
        assert!(to_json(&model).is_err());
    }

    #[test]
    fn unsupported_values_nan_and_renamed_fields_fail_explicitly() {
        for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(to_json(&Value::Number(number)).is_err());
        }
        assert!(to_json(&Value::Image(slint::Image::default())).is_err());
        assert!(from_json(&json!({"row_id": "would-be-renamed"})).is_err());
        assert_eq!(to_json(&Value::Void).unwrap(), Json::Null);
    }
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    #[test]
    fn live_slint_model_keeps_identity_across_equal_and_changed_gain_rows() {
        let current = from_json(&serde_json::json!([0, 0, 0, 0, 0, 0])).unwrap();
        let Value::Model(model) = &current else {
            panic!("model required")
        };
        let identity = model.as_any() as *const dyn std::any::Any;
        assert!(
            reconcile(
                &current,
                &from_json(&serde_json::json!([0, 0, 0, 0, 0, 0])).unwrap()
            )
            .unwrap()
        );
        assert!(
            reconcile(
                &current,
                &from_json(&serde_json::json!([0, 0, 2.5, 0, 0, 0])).unwrap()
            )
            .unwrap()
        );
        assert!(std::ptr::eq(
            identity,
            model.as_any() as *const dyn std::any::Any
        ));
        assert_eq!(
            to_json(&model.row_data(2).unwrap()).unwrap(),
            serde_json::json!(2.5)
        );
        assert_eq!(model.row_count(), 6);
        // A structural replacement must be installed by the caller, not mistaken
        // for an equal-size update that retains obsolete data or menu structure.
        assert!(!reconcile(&current, &from_json(&serde_json::json!([0, 0])).unwrap()).unwrap());
        assert_eq!(model.row_count(), 6);
    }
}

#[cfg(test)]
mod image_rows {
    use super::*;

    fn row(key: &str, icon: &slint::Image) -> Value {
        let mut row = Struct::default();
        row.set_field("key".into(), Value::String(key.into()));
        row.set_field("icon".into(), Value::Image(icon.clone()));
        Value::Struct(row)
    }
    fn list(rows: Vec<Value>) -> Value {
        Value::Model(Rc::new(VecModel::from(rows)).into())
    }

    /// STORE-ROW-LOGO: rows with an image each update in place (the list
    /// keeps its model, so a ListView keeps its scroll position).
    #[test]
    fn rows_with_images_update_in_place() {
        let pixels = |width| slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(width, 1);
        let (a, b) = (
            slint::Image::from_rgba8(pixels(1)),
            slint::Image::from_rgba8(pixels(2)),
        );
        let current = list(vec![row("a", &a), row("b", &a)]);
        let next = list(vec![row("a", &a), row("b", &b), row("c", &b)]);
        assert!(reconcile(&current, &next).unwrap());
        let Value::Model(model) = &current else {
            panic!("a model")
        };
        assert_eq!(model.row_count(), 3);
        let Some(Value::Struct(second)) = model.row_data(1) else {
            panic!("a row")
        };
        assert_eq!(second.get_field("icon"), Some(&Value::Image(b)));
        // A whole-value image is still installed, never merged.
        assert!(!reconcile(&Value::Image(a.clone()), &Value::Image(a)).unwrap());
    }
}
