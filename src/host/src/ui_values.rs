//! Bounded JSON ↔ Slint 1.16.1 value conversion (host-internal).
//! Equal values retain live properties; same-length Host-owned models reconcile
//! changed rows. Structural changes replace models; Rc ownership stays with the view.
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

pub const MAX_DEPTH: usize = 8;
pub const MAX_NODES: usize = 4096;
pub const MAX_ARRAY: usize = 1024;
pub const MAX_STRING_BYTES: usize = 64 * 1024;
/// Includes object field names, in both directions.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct Budget {
    nodes: usize,
    text: usize,
}

impl Budget {
    fn node(&mut self, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("UI value depth exceeds {MAX_DEPTH} (root is depth 0)");
        }
        if self.nodes == MAX_NODES {
            bail!("UI value exceeds {MAX_NODES} total nodes");
        }
        self.nodes += 1;
        Ok(())
    }

    fn text(&mut self, value: &str) -> Result<()> {
        if value.len() > MAX_STRING_BYTES {
            bail!("UI string or field name exceeds {MAX_STRING_BYTES} bytes");
        }
        // Both operands have independent small bounds, so addition cannot wrap.
        if self.text + value.len() > MAX_TEXT_BYTES {
            bail!("UI value text exceeds {MAX_TEXT_BYTES} total bytes");
        }
        self.text += value.len();
        Ok(())
    }

    fn array(&self, count: usize) -> Result<()> {
        if count > MAX_ARRAY {
            bail!("UI array exceeds {MAX_ARRAY} rows");
        }
        if count > MAX_NODES - self.nodes {
            bail!("UI array cannot fit in remaining node budget");
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

/// Reconcile an actual live property before replacing it. Bridge-created models
/// with stable length update only changed rows, preserving focused repeater items.
/// False means the caller must install the new value (including structural change).
pub fn reconcile(current: &Value, next: &Value) -> Result<bool> {
    // Runtime images (`plugin.ui.image`) are not JSON: always install them,
    // and replace an image with whatever comes next.
    if matches!(current, Value::Image(_)) || matches!(next, Value::Image(_)) {
        return Ok(false);
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
    {
        // Validate and collect before mutation, so invalid input cannot partly apply.
        let mut changes = Vec::new();
        for index in 0..rows.row_count() {
            let before = rows.row_data(index).context("current UI row missing")?;
            let after = next.row_data(index).context("new UI row missing")?;
            if to_json(&before)? != to_json(&after)? {
                changes.push((index, after));
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
fn trace_reconciliation(mode: &str, rows: usize) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if std::env::var_os("VESTRA_UI_PROPERTY_TRACE").is_some()
        && COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 32
    {
        eprintln!("ui.model-update mode={mode} rows={rows}");
    }
}

pub fn from_json(value: &Json) -> Result<Value> {
    encode(value, 0, &mut Budget::default())
}

fn encode(value: &Json, depth: usize, budget: &mut Budget) -> Result<Value> {
    budget.node(depth)?;
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
                rows.push(encode(value, depth + 1, budget)?);
            }
            let model: ModelRc<Value> = Rc::new(VecModel::from(rows)).into();
            Value::Model(model)
        }
        Json::Object(fields) => {
            if fields.len() > MAX_NODES - budget.nodes {
                bail!("UI object cannot fit in remaining node budget");
            }
            let mut object = Struct::default();
            for (name, value) in fields {
                budget.text(name)?;
                if object.get_field(name).is_some() {
                    bail!("UI object field names collide under Slint normalization");
                }
                let value = encode(value, depth + 1, budget)?;
                object.set_field(name.clone(), value);
                if !object.iter().any(|(stored, _)| stored == name) {
                    bail!("UI object field {name:?} is not in Slint canonical spelling");
                }
            }
            Value::Struct(object)
        }
    })
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
        // root + 4 arrays + (1024 + 1024 + 1024 + 1019) nulls = 4096.
        let exact = Json::Array(vec![
            Json::Array(vec![Json::Null; 1024]),
            Json::Array(vec![Json::Null; 1024]),
            Json::Array(vec![Json::Null; 1024]),
            Json::Array(vec![Json::Null; 1019]),
        ]);
        let converted = from_json(&exact).unwrap();
        assert_eq!(to_json(&converted).unwrap(), exact);
        let mut too_many = exact;
        too_many.as_array_mut().unwrap()[3]
            .as_array_mut()
            .unwrap()
            .push(Json::Null);
        assert!(from_json(&too_many).is_err());
        let rows: Vec<_> = [1024, 1024, 1024, 1020]
            .into_iter()
            .map(|count| Value::Model(Rc::new(VecModel::from(vec![Value::Void; count])).into()))
            .collect();
        assert!(to_json(&Value::Model(Rc::new(VecModel::from(rows)).into())).is_err());
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
