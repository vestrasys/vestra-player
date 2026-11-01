//! The model behind every array the host converts for a view (`Rows`), and
//! the plain row form that arrays are converted to, off the UI thread when
//! they are large (`Plain`; player 0.6.0, doc/architecture/large-lists.md
//! §4).
//!
//! `Rows` is Slint's `VecModel` with range operations: `insert_rows` and
//! `remove_rows` change any run of rows with one `row_added` or
//! `row_removed` notification, so a row patch that inserts or removes 1,000
//! rows costs the repeater one update, not 1,000 (`ui_patch`).
//!
//! A row is a converted Slint value (`Row::Value`), or a `Plain` value
//! converted when Slint reads the row (`Row::Plain`). A property's array
//! (`Host::prop_value`) and the rows a patch carries are plain unless they
//! hold Markdown documents: `Send`, a third of the memory of a Slint struct,
//! and compared without converting. A `ListView` reads only the rows it
//! shows, so installing 20,000 plain rows costs the UI thread a shape check,
//! not 20,000 conversions. Images in plain rows are references, loaded on
//! the UI thread once per distinct reference into the model's `Images`. A
//! nested array of a plain row is a new model each time the row is read (a
//! repeater keeps the value it read until the row changes).
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{Map, Number, Value as Json};
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString};
use slint_interpreter::{Struct, Value};

/// A struct field name, shared by the rows of one conversion.
pub type Name = Arc<str>;

/// A JSON value as Slint will see it (numbers finite, field names in
/// Slint's canonical spelling, checked against the value limits), without
/// Markdown documents.
#[derive(Clone, Debug, PartialEq)]
pub enum Plain {
    Void,
    Bool(bool),
    Number(f64),
    String(SharedString),
    /// A runtime image (`plugin.ui.image`): loaded on the UI thread, once
    /// per reference, into the model's `Images`.
    Image(Arc<ImageRef>),
    Array(Arc<[Plain]>),
    /// Fields in name order (as `serde_json::Map` keeps them).
    Struct(Arc<[(Name, Plain)]>),
}

/// An image reference of a plain value: the file and `maxSide` (0: full size).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub path: Box<str>,
    pub side: u32,
}

/// The pictures of a model's image references (a reference whose file
/// failed its checks is an empty image, as in a whole conversion).
pub type Images = Rc<HashMap<Arc<ImageRef>, slint::Image>>;

/// Where `Plain::to_value` finds the picture of an image reference.
pub trait Pictures {
    fn picture(&self, image: &Arc<ImageRef>) -> slint::Image;
}
impl Pictures for HashMap<Arc<ImageRef>, slint::Image> {
    fn picture(&self, image: &Arc<ImageRef>) -> slint::Image {
        self.get(image).cloned().unwrap_or_default()
    }
}

/// A picture a model shows, and how many references of its rows use it.
#[derive(Clone, Debug)]
struct Picture {
    image: slint::Image,
    uses: usize,
}
impl Pictures for HashMap<Arc<ImageRef>, Picture> {
    fn picture(&self, image: &Arc<ImageRef>) -> slint::Image {
        self.get(image)
            .map(|picture| picture.image.clone())
            .unwrap_or_default()
    }
}

/// `f` for each image reference in `row` (a plain row; a Slint value holds
/// its pictures itself).
fn each_image(row: &Row, f: &mut dyn FnMut(&Arc<ImageRef>)) {
    fn walk(value: &Plain, f: &mut dyn FnMut(&Arc<ImageRef>)) {
        match value {
            Plain::Image(image) => f(image),
            Plain::Array(rows) => rows.iter().for_each(|row| walk(row, f)),
            Plain::Struct(fields) => fields.iter().for_each(|(_, value)| walk(value, f)),
            _ => {}
        }
    }
    if let Row::Plain(plain) = row {
        walk(plain, f);
    }
}

/// Image references whose picture differs between two models.
pub type Changed = HashSet<Arc<ImageRef>>;

impl Plain {
    /// The Slint value (a nested array becomes a new `Rows` model with the
    /// same pictures).
    pub fn to_value(&self, images: &dyn Pictures) -> Value {
        match self {
            // No Slint property type takes `null`: in a row it is an empty
            // image (player 0.6.0), so a row without a picture costs 4 bytes
            // instead of a placeholder reference.
            Self::Void => Value::Image(slint::Image::default()),
            Self::Bool(value) => Value::Bool(*value),
            Self::Number(value) => Value::Number(*value),
            Self::String(value) => Value::String(value.clone()),
            Self::Image(image) => Value::Image(images.picture(image)),
            Self::Array(rows) => {
                // The nested rows get the pictures they show.
                let mut table = HashMap::new();
                for row in rows.iter() {
                    each_image(&Row::Plain(row.clone()), &mut |image| {
                        table
                            .entry(image.clone())
                            .or_insert_with(|| images.picture(image));
                    });
                }
                Value::Model(ModelRc::new(Rows::from_plain(
                    rows.to_vec(),
                    Rc::new(table),
                )))
            }
            Self::Struct(fields) => Value::Struct(
                fields
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_value(images)))
                    .collect::<Struct>(),
            ),
        }
    }

    /// The wire form (as `ui_values::to_json` writes the Slint value: an
    /// image has none).
    pub fn to_json(&self) -> Result<Json> {
        Ok(match self {
            Self::Image(_) => anyhow::bail!("unsupported UI value type: Image"),
            Self::Void => Json::Null,
            Self::Bool(value) => Json::Bool(*value),
            Self::Number(value) => {
                Json::Number(Number::from_f64(*value).context("UI number must be finite")?)
            }
            Self::String(value) => Json::String(value.to_string()),
            Self::Array(rows) => {
                Json::Array(rows.iter().map(Self::to_json).collect::<Result<_>>()?)
            }
            Self::Struct(fields) => {
                let mut map = Map::new();
                for (name, value) in fields.iter() {
                    map.insert(name.to_string(), value.to_json()?);
                }
                Json::Object(map)
            }
        })
    }

    /// `next` has this value's Slint type, as far as the values show it:
    /// the same fields with values that fit, arrays whose rows fit this
    /// array's first row (an empty array shows no row type: only an empty
    /// one fits it), values of the same kind.
    pub fn fits(&self, next: &Self) -> bool {
        match (self, next) {
            (Self::Struct(current), Self::Struct(next)) => {
                current.len() == next.len()
                    && current
                        .iter()
                        .zip(next.iter())
                        .all(|((a, x), (b, y))| a == b && x.fits(y))
            }
            (Self::Array(current), Self::Array(next)) => {
                next.is_empty()
                    || current
                        .first()
                        .is_some_and(|sample| next.iter().all(|row| sample.fits(row)))
            }
            // `null` reads as an empty image (no Slint type takes `null`).
            (Self::Void | Self::Image(_), Self::Void | Self::Image(_)) => true,
            (current, next) => std::mem::discriminant(current) == std::mem::discriminant(next),
        }
    }

    /// Whether the value holds an image reference.
    pub fn has_image(&self) -> bool {
        match self {
            Self::Image(_) => true,
            Self::Array(rows) => rows.iter().any(Self::has_image),
            Self::Struct(fields) => fields.iter().any(|(_, value)| value.has_image()),
            _ => false,
        }
    }

    /// Whether one of `images` is referenced anywhere in the value.
    fn shows(&self, images: &Changed) -> bool {
        match self {
            Self::Image(image) => images.contains(image),
            Self::Array(rows) => rows.iter().any(|row| row.shows(images)),
            Self::Struct(fields) => fields.iter().any(|(_, value)| value.shows(images)),
            _ => false,
        }
    }
}

/// One row of a `Rows` model.
#[derive(Clone, Debug)]
pub enum Row {
    Value(Value),
    Plain(Plain),
}

impl Row {
    /// The wire form of the row (`None`: a value with no JSON form, such
    /// as an image or a document).
    pub fn to_json(&self) -> Option<Json> {
        match self {
            Self::Value(value) => crate::ui::ui_values::to_json(value).ok(),
            Self::Plain(plain) => plain.to_json().ok(),
        }
    }
}

/// A host-owned array model (see the module documentation).
#[derive(Default)]
pub struct Rows {
    rows: RefCell<Vec<Row>>,
    /// The pictures of the plain rows' image references, with how many
    /// references use each: a picture no row uses any more is dropped, so a
    /// list whose rows change pictures (thumbnails made as the user
    /// scrolls) keeps only what it shows.
    pictures: RefCell<HashMap<Arc<ImageRef>, Picture>>,
    /// Notifications sent (tests: a range is one).
    #[cfg(test)]
    pub notified: std::cell::Cell<usize>,
    notify: ModelNotify,
}

impl From<Vec<Value>> for Rows {
    fn from(values: Vec<Value>) -> Self {
        Self::from_rows(
            values.into_iter().map(Row::Value).collect(),
            Images::default(),
        )
    }
}

impl Rows {
    pub fn from_rows(rows: Vec<Row>, images: Images) -> Self {
        let model = Self {
            rows: RefCell::new(rows),
            pictures: RefCell::default(),
            #[cfg(test)]
            notified: std::cell::Cell::default(),
            notify: ModelNotify::default(),
        };
        model.set_images(images);
        model.prune();
        model
    }
    pub fn from_plain(rows: Vec<Plain>, images: Images) -> Self {
        Self::from_rows(rows.into_iter().map(Row::Plain).collect(), images)
    }
    #[cfg(test)]
    fn count(&self) {
        self.notified.set(self.notified.get() + 1);
    }
    #[cfg(not(test))]
    fn count(&self) {}
    /// The pictures of the model's plain rows.
    pub fn images(&self) -> Images {
        Rc::new(
            self.pictures
                .borrow()
                .iter()
                .map(|(key, picture)| (key.clone(), picture.image.clone()))
                .collect(),
        )
    }
    /// How many pictures the model keeps (tests).
    #[cfg(test)]
    pub fn picture_count(&self) -> usize {
        self.pictures.borrow().len()
    }
    /// Use `images` for the plain rows from now on (a whole write: the
    /// pictures of the rows it leaves), counted for the rows now; those at
    /// no use are dropped by `prune`.
    pub fn set_images(&self, images: Images) {
        let mut pictures: HashMap<Arc<ImageRef>, Picture> = images
            .iter()
            .map(|(key, image)| {
                (
                    key.clone(),
                    Picture {
                        image: image.clone(),
                        uses: 0,
                    },
                )
            })
            .collect();
        if !pictures.is_empty() {
            for row in self.rows.borrow().iter() {
                each_image(row, &mut |image| {
                    if let Some(picture) = pictures.get_mut(image) {
                        picture.uses += 1;
                    }
                });
            }
        }
        *self.pictures.borrow_mut() = pictures;
    }
    /// Add pictures for plain rows about to be written (a patch's rows):
    /// counted as the rows go in, dropped by `prune` if none does.
    pub fn add_images(&self, images: &Images) {
        if images.is_empty() {
            return;
        }
        let mut pictures = self.pictures.borrow_mut();
        for (key, image) in images.iter() {
            pictures.entry(key.clone()).or_insert_with(|| Picture {
                image: image.clone(),
                uses: 0,
            });
        }
    }
    /// Drop the pictures no row uses.
    pub fn prune(&self) {
        self.pictures
            .borrow_mut()
            .retain(|_, picture| picture.uses > 0);
    }
    /// Count the image references of `rows` going in or out (a picture at
    /// no use is dropped by `prune`, after a whole patch or write).
    fn uses(&self, rows: &[Row], going_in: bool) {
        let mut pictures = self.pictures.borrow_mut();
        if pictures.is_empty() {
            return;
        }
        for row in rows {
            each_image(row, &mut |image| {
                if let Some(picture) = pictures.get_mut(image) {
                    if going_in {
                        picture.uses += 1;
                    } else {
                        picture.uses = picture.uses.saturating_sub(1);
                    }
                }
            });
        }
    }
    /// `row` (of this model) as Slint sees it.
    pub fn value(&self, row: &Row) -> Value {
        match row {
            Row::Value(value) => value.clone(),
            Row::Plain(plain) => plain.to_value(&*self.pictures.borrow()),
        }
    }
    /// Image references whose picture differs in `next` (a model about to
    /// be written into this one).
    pub fn changed_images(&self, next: &Self) -> Changed {
        let current = self.pictures.borrow();
        next.pictures
            .borrow()
            .iter()
            .filter(|(key, next)| {
                // Two empty pictures (files that failed their checks) are the same.
                let empty = |picture: &slint::Image| picture.size().width == 0;
                current.get(*key).is_some_and(|shown| {
                    shown.image != next.image && !(empty(&shown.image) && empty(&next.image))
                })
            })
            .map(|(key, _)| key.clone())
            .collect()
    }
    /// The same row: plain rows by value (not the same where they show an
    /// image of `changed`), others by their wire form.
    pub fn same(a: &Row, b: &Row, changed: &Changed) -> bool {
        match (a, b) {
            (Row::Plain(a), Row::Plain(b)) => a == b && (changed.is_empty() || !a.shows(changed)),
            _ => matches!((a.to_json(), b.to_json()), (Some(a), Some(b)) if a == b),
        }
    }
    /// `next` (a row whose pictures are `next_images`) can be written over
    /// `current` (a row of this model) without changing its Slint type.
    pub fn fits(&self, current: &Row, next: &Row, next_images: &Images) -> bool {
        match (current, next) {
            (Row::Plain(a), Row::Plain(b)) => a.fits(b),
            (_, Row::Plain(b)) => {
                crate::ui::ui_values::fits(&self.value(current), &b.to_value(&**next_images))
            }
            (_, Row::Value(b)) => crate::ui::ui_values::fits(&self.value(current), b),
        }
    }
    /// A clone of row `index` (a plain row is not converted).
    pub fn row(&self, index: usize) -> Option<Row> {
        self.rows.borrow().get(index).cloned()
    }
    /// `f` over the rows as they are stored. `f` must not change the model.
    pub fn with_rows<R>(&self, f: impl FnOnce(&[Row]) -> R) -> R {
        f(&self.rows.borrow())
    }
    /// Replace row `index` (one `row_changed`).
    pub fn set_row(&self, index: usize, row: Row) {
        let old = match self.rows.borrow_mut().get_mut(index) {
            Some(slot) => {
                self.uses(std::slice::from_ref(&row), true);
                std::mem::replace(slot, row)
            }
            None => return,
        };
        self.uses(&[old], false);
        self.count();
        self.notify.row_changed(index);
    }
    /// Insert `rows` at `at` (at most the row count) with one `row_added`.
    pub fn insert_rows(&self, at: usize, rows: impl IntoIterator<Item = Row>) {
        let rows: Vec<Row> = rows.into_iter().collect();
        self.uses(&rows, true);
        let (at, count) = {
            let mut stored = self.rows.borrow_mut();
            let before = stored.len();
            let at = at.min(before);
            stored.splice(at..at, rows);
            (at, stored.len() - before)
        };
        if count > 0 {
            self.count();
            self.notify.row_added(at, count);
        }
    }
    /// Remove `count` rows from `at` with one `row_removed`; the removed
    /// rows (their pictures stay until `prune`, so a move keeps them).
    pub fn remove_rows(&self, at: usize, count: usize) -> Vec<Row> {
        let (at, removed) = {
            let mut stored = self.rows.borrow_mut();
            let end = at.saturating_add(count).min(stored.len());
            let at = at.min(end);
            (at, stored.drain(at..end).collect::<Vec<Row>>())
        };
        if !removed.is_empty() {
            self.uses(&removed, false);
            self.count();
            self.notify.row_removed(at, removed.len());
        }
        removed
    }
    /// Take every row out (one `reset`); the pictures stay (`images`).
    pub fn take_rows(&self) -> Vec<Row> {
        let rows = std::mem::take(&mut *self.rows.borrow_mut());
        self.count();
        self.notify.reset();
        rows
    }
    /// Replace every row (one `reset`); pictures counted again.
    pub fn set_rows(&self, rows: Vec<Row>) {
        *self.rows.borrow_mut() = rows;
        let images = self.images();
        self.set_images(images);
        self.prune();
        self.count();
        self.notify.reset();
    }
    /// `VecModel::push`.
    pub fn push(&self, value: Value) {
        let at = self.row_count();
        self.insert_rows(at, [Row::Value(value)]);
    }
    /// `VecModel::extend`: one `row_added`.
    pub fn extend(&self, values: impl IntoIterator<Item = Value>) {
        let at = self.row_count();
        self.insert_rows(at, values.into_iter().map(Row::Value));
    }
    /// `VecModel::remove`.
    pub fn remove(&self, index: usize) -> Option<Value> {
        let row = self.remove_rows(index, 1).pop()?;
        Some(self.value(&row))
    }
    /// Remove the rows from `len` on (one `row_removed`).
    pub fn truncate(&self, len: usize) {
        let count = self.row_count().saturating_sub(len);
        self.remove_rows(len, count);
    }
}

impl Model for Rows {
    type Data = Value;

    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<Value> {
        // Converted outside the borrow: a nested model is created here.
        let stored = self.rows.borrow().get(row).cloned()?;
        Some(self.value(&stored))
    }

    /// A view writing into its own rows (`rows[i].x = …`, allowed by Slint
    /// on a writable model; doc/architecture/large-lists.md §2.3).
    fn set_row_data(&self, row: usize, data: Value) {
        self.set_row(row, Row::Value(data));
    }

    fn remove_row(&self, row: usize) -> std::result::Result<(), slint::ModelError> {
        if row >= self.row_count() {
            return Err(slint::ModelError::out_of_bounds(self.row_count()));
        }
        self.remove_rows(row, 1);
        Ok(())
    }

    fn insert_row(&self, row: usize, data: Value) -> std::result::Result<(), slint::ModelError> {
        if row > self.row_count() {
            return Err(slint::ModelError::out_of_bounds(self.row_count()));
        }
        self.insert_rows(row, [Row::Value(data)]);
        Ok(())
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// The host's own model in `model`, if it is one.
pub fn rows(model: &ModelRc<Value>) -> Option<&Rows> {
    model.as_any().downcast_ref::<Rows>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ui_image;
    use crate::ui::ui_patch;
    use crate::ui::ui_values::{self, from_json, plain_rows, to_json};
    use serde_json::json;

    fn row(i: usize) -> Json {
        json!({"key": format!("r{i}"), "label": format!("{i}. Episode {i}.mkv"), "selected": false, "n": i})
    }
    fn list(range: std::ops::Range<usize>) -> Json {
        Json::Array(range.map(row).collect())
    }
    /// A property's array as `prop_value` converts it (no images).
    fn plain_model(rows: &Json) -> Rc<Rows> {
        let refs: Vec<&Json> = rows.as_array().unwrap().iter().collect();
        let plain = plain_rows(&refs).unwrap();
        Rc::new(Rows::from_plain(plain.rows, Images::default()))
    }
    fn value(model: &Rc<Rows>) -> Value {
        Value::Model(ModelRc::from(model.clone()))
    }
    fn json_of(model: &Rows) -> Json {
        Json::Array(
            (0..model.row_count())
                .map(|row| to_json(&model.row_data(row).unwrap()).unwrap())
                .collect(),
        )
    }
    fn png(side: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![120; (side * side * 4) as usize])
            .unwrap();
        bytes
    }

    /// Range operations notify once per range, whatever its length (a
    /// `VecModel` notified once per row; `ui_patch` used to loop).
    #[test]
    fn a_range_is_one_notification() {
        let model = Rows::from((0..10).map(|n| Value::Number(n.into())).collect::<Vec<_>>());
        let numbers = |model: &Rows| -> Vec<f64> {
            model
                .iter()
                .map(|value| match value {
                    Value::Number(n) => n,
                    _ => f64::NAN,
                })
                .collect()
        };
        model.insert_rows(3, (100..1100).map(|n| Row::Value(Value::Number(n.into()))));
        assert_eq!(model.row_count(), 1010);
        assert_eq!(model.notified.get(), 1);
        assert_eq!(numbers(&model)[2..5], [2., 100., 101.]);
        let removed = model.remove_rows(3, 1000);
        assert_eq!((removed.len(), model.notified.get()), (1000, 2));
        assert_eq!(numbers(&model), (0..10).map(f64::from).collect::<Vec<_>>());
        // Past the end: clamped; nothing to do sends nothing.
        assert_eq!(model.remove_rows(8, 100).len(), 2);
        assert_eq!(model.remove_rows(8, 100).len(), 0);
        model.insert_rows(50, [Row::Value(Value::Number(9.))]);
        assert_eq!(numbers(&model), [0., 1., 2., 3., 4., 5., 6., 7., 9.]);
        assert_eq!(model.notified.get(), 4);
        model.truncate(2);
        model.set_row(1, Row::Value(Value::Number(7.)));
        model.set_row(5, Row::Value(Value::Number(7.)));
        assert_eq!(numbers(&model), [0., 7.]);
        assert_eq!(model.notified.get(), 6, "a row past the end is no change");
        // Slint's own row operations (`Model::insert_row`, `remove_row`).
        assert!(model.insert_row(3, Value::Void).is_err());
        assert!(model.remove_row(2).is_err());
        model.insert_row(0, Value::Number(5.)).unwrap();
        model.remove_row(1).unwrap();
        assert_eq!(numbers(&model), [5., 7.]);
    }

    /// `plain_rows` accepts and refuses what the whole conversion does, and
    /// Slint sees the same values (read from the model, converted then).
    #[test]
    fn plain_rows_convert_as_from_json_does() {
        let corpus = json!([
            {"key": "a", "n": 1.5, "on": true, "tags": ["x", "y"], "deep": {"a": [1, {"b": "é"}]}},
            {"key": "b", "n": -2, "on": false, "tags": [], "deep": {"a": [], "c": "段"}},
            "a string", 3, [1, 2, [3]], {}, [], {"_lead": 1, "x-y": 2}
        ]);
        let refs: Vec<&Json> = corpus.as_array().unwrap().iter().collect();
        let plain = plain_rows(&refs).unwrap();
        let model = Rows::from_plain(plain.rows.clone(), Images::default());
        let numbers_as_f64 =
            |json: &Json| from_json(json).and_then(|value| to_json(&value)).unwrap();
        assert_eq!(json_of(&model), numbers_as_f64(&corpus));
        let whole = ui_values::from_json_rows_with(&refs, &mut |_, _| None).unwrap();
        let whole = Json::Array(whole.iter().map(|value| to_json(value).unwrap()).collect());
        assert_eq!(whole, json_of(&model));
        for (row, cost) in refs.iter().zip(&plain.costs) {
            assert_eq!(*cost, ui_values::cost(row), "{row}");
        }
        // `null` in a row is an empty image, and fits where a picture is.
        let thumb = json!({"$vestraImage": "/x.png", "ref": 1});
        let (with, without) = (json!({"thumb": thumb}), json!({"thumb": null}));
        let plain = plain_rows(&[&with, &without]).unwrap();
        assert!(plain.rows[0].fits(&plain.rows[1]) && plain.rows[1].fits(&plain.rows[0]));
        let model = Rows::from_plain(plain.rows, Images::default());
        let Some(Value::Struct(row)) = model.row_data(1) else {
            panic!("a row")
        };
        let Some(Value::Image(thumb)) = row.get_field("thumb") else {
            panic!("an image")
        };
        assert_eq!(thumb.size().width, 0, "empty");
        // Refused alike: renamed fields, the limits (depth, string, rows).
        let mut deep = json!(1);
        for _ in 0..ui_values::MAX_DEPTH {
            deep = json!([deep]);
        }
        let long = json!("x".repeat(ui_values::MAX_STRING_BYTES + 1));
        let many = Json::Array(vec![json!(0); ui_values::MAX_ARRAY + 1]);
        for (bad, limit) in [
            (json!({"row_id": 1}), false),
            (json!({"-x": 1}), false),
            (deep, true),
            (long, true),
            (json!([many]), true),
        ] {
            let error = match plain_rows(&[&bad]) {
                Err(ui_values::NotPlain::Refused(error)) => error,
                other => panic!("{bad}: {other:?}"),
            };
            let whole = ui_values::from_json_rows_with(&[&bad], &mut |_, _| None).unwrap_err();
            let over =
                |error: &anyhow::Error| error.downcast_ref::<ui_values::OverLimit>().is_some();
            assert_eq!((over(&error), over(&whole)), (limit, limit), "{bad}");
        }
        // Rows over the rows limit; documents (converted as before); a
        // malformed image reference (`invalid`, as before).
        let rows = vec![json!(0); ui_values::MAX_ARRAY + 1];
        assert!(plain_rows(&rows.iter().collect::<Vec<_>>()).is_err());
        let doc = json!({"body": {"$vestraMarkdown": {"text": "x"}, "ref": 1}});
        assert!(matches!(
            plain_rows(&[&doc]),
            Err(ui_values::NotPlain::Markdown)
        ));
        let bad_image = json!({"icon": {"$vestraImage": 3, "ref": 1}});
        assert!(matches!(
            plain_rows(&[&bad_image]),
            Err(ui_values::NotPlain::Image(error)) if error.code == "invalid"
        ));
    }

    /// A whole write of plain rows into a host model: the model stays, only
    /// changed rows are written, rows appended or removed at the end are one
    /// range, a new list is one reset, and another row type is installed by
    /// the caller (nothing written).
    #[test]
    fn whole_writes_compare_plain_rows_and_write_only_what_changed() {
        let live = plain_model(&list(0..2000));
        let current = value(&live);
        let mut next = list(0..2003);
        next[5]["selected"] = json!(true);
        assert!(ui_values::reconcile(&current, &value(&plain_model(&next))).unwrap());
        assert_eq!(live.notified.get(), 2, "one row written, three appended");
        assert_eq!(json_of(&live), json_of(&plain_model(&next)));
        // Equal: nothing.
        assert!(ui_values::reconcile(&current, &value(&plain_model(&next))).unwrap());
        assert_eq!(live.notified.get(), 2);
        // Shorter: one range removed (and row 5 written back).
        let shorter = plain_model(&list(0..1000));
        assert!(ui_values::reconcile(&current, &value(&shorter)).unwrap());
        assert_eq!(live.notified.get(), 4);
        assert_eq!(json_of(&live), json_of(&plain_model(&list(0..1000))));
        // Every row differs (a new list): one reset.
        let other = list(5000..6000);
        assert!(ui_values::reconcile(&current, &value(&plain_model(&other))).unwrap());
        assert_eq!(live.notified.get(), 5);
        assert_eq!(json_of(&live), json_of(&plain_model(&other)));
        // Another row type: not written; the caller installs it (Slint checks).
        let mut wrong = list(5000..6000);
        wrong[999]["n"] = json!("text");
        assert!(!ui_values::reconcile(&current, &value(&plain_model(&wrong))).unwrap());
        assert_eq!(json_of(&live), json_of(&plain_model(&other)), "untouched");
        // An empty model shows no row type.
        let empty = value(&plain_model(&json!([])));
        assert!(!ui_values::reconcile(&empty, &value(&plain_model(&other))).unwrap());
        // A row the view wrote itself (`set_row_data`, a Slint value)
        // compares by its wire form.
        live.set_row_data(0, from_json(&row(1)).unwrap());
        let notified = live.notified.get();
        assert!(ui_values::reconcile(&current, &value(&plain_model(&other))).unwrap());
        assert_eq!(
            live.notified.get() - notified,
            1,
            "only the row the view wrote"
        );
        assert_eq!(json_of(&live), json_of(&plain_model(&other)));
    }

    struct Headless(Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
        {
            Ok(self.0.clone())
        }
    }

    /// Slint checks every row of a model it is given; `ui_values::set`
    /// gives it the first row and adds the others (of that row's shape) in
    /// one range once it is accepted. A refusal still names what is wrong.
    #[test]
    fn a_large_model_is_installed_after_its_first_row_is_checked() {
        let window =
            slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default());
        let _ = slint::platform::set_platform(Box::new(Headless(window)));
        let dir = tempfile::tempdir().unwrap();
        let source = r#"
            export component W inherits Window {
                in-out property <[{key: string, label: string, selected: bool, n: int}]> rows;
                out property <int> count: rows.length;
                out property <string> last: rows.length > 0 ? rows[rows.length - 1].label : "";
            }
        "#;
        let result = pollster::block_on(
            crate::ui::view::compiler()
                .build_from_source(source.into(), dir.path().join("w.slint")),
        );
        let instance = result.component("W").unwrap().create().unwrap();
        let model = plain_model(&list(0..20_000));
        ui_values::set(&instance, None, "rows", value(&model)).unwrap();
        assert_eq!(
            instance.get_property("count").unwrap(),
            Value::Number(20_000.)
        );
        assert_eq!(
            instance.get_property("last").unwrap(),
            Value::String("19999. Episode 19999.mkv".into())
        );
        assert_eq!(
            model.notified.get(),
            2,
            "the tail out, then back in one range"
        );
        let Value::Model(installed) = instance.get_property("rows").unwrap() else {
            panic!("a model")
        };
        assert!(std::ptr::eq(
            rows(&installed).unwrap() as *const Rows,
            Rc::as_ptr(&model)
        ));
        // A first row of another type: refused, named.
        let mut wrong = list(0..3);
        wrong[0]["n"] = json!("x");
        let error =
            ui_values::set(&instance, None, "rows", value(&plain_model(&wrong))).unwrap_err();
        assert!(error.to_string().contains("rows[0].n"), "{error}");
        // A later row of another shape: the whole model is checked, refused.
        let mut wrong = list(0..3);
        wrong[2]["extra"] = json!(1);
        let model = plain_model(&wrong);
        let error = ui_values::set(&instance, None, "rows", value(&model)).unwrap_err();
        assert!(error.to_string().contains("rows[2].extra"), "{error}");
        assert_eq!(model.row_count(), 3, "rows kept");
        assert_eq!(
            instance.get_property("count").unwrap(),
            Value::Number(20_000.)
        );
    }

    /// Images in the rows of a large list: each file loaded once, the rows'
    /// keys recorded, a missing file an empty image; a whole write where one
    /// file changed rewrites the rows that show it and no others; a patch
    /// that moves rows moves their keys and pictures.
    #[test]
    fn images_per_row_in_a_big_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let roots = [root.clone()];
        let file = |name: &str, side: u32| {
            let path = root.join(name);
            std::fs::write(&path, png(side)).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let paths = [file("a.png", 2), file("b.png", 3), file("c.png", 4)];
        let gone = root.join("gone.png").to_str().unwrap().to_owned();
        let wire = Json::Array(
            (0..5000)
                .map(|i| {
                    let path = if i == 7 { &gone } else { &paths[i % 3] };
                    json!({"key": format!("r{i}"), "icon": {"$vestraImage": path, "ref": 1, "maxSide": 64}})
                })
                .collect(),
        );
        let refs: Vec<&Json> = wire.as_array().unwrap().iter().collect();
        let plain = plain_rows(&refs).unwrap();
        assert_eq!(plain.images.len(), 4, "each reference once");
        let mut images = ui_image::Images::default();
        let (pictures, keys) = images.load_refs(1, &roots, &plain.images);
        assert_eq!(
            keys.iter().filter(|key| key.is_none()).count(),
            1,
            "the missing file"
        );
        let written: Vec<ui_image::Row> = plain
            .row_images
            .iter()
            .zip(&plain.costs)
            .map(|(images, cost)| ui_image::Row {
                keys: images
                    .iter()
                    .filter_map(|index| keys[*index as usize].clone())
                    .collect(),
                cost: *cost,
            })
            .collect();
        assert_eq!((written[7].keys.len(), written[8].keys.len()), (0, 1));
        images.patched_rows(1, "$mount", "rows", written.clone());
        let model = Rc::new(Rows::from_plain(plain.rows, pictures));
        let side = |row: usize| {
            let Some(Value::Struct(row)) = model.row_data(row) else {
                panic!("a row")
            };
            let Some(Value::Image(icon)) = row.get_field("icon") else {
                panic!("an image")
            };
            icon.size().width
        };
        assert_eq!((side(0), side(1), side(2), side(7)), (2, 3, 4, 0));
        // b.png changes on disk: a new picture for the same reference.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("b.png"), png(5)).unwrap();
        let plain = plain_rows(&refs).unwrap();
        let (pictures, _) = images.load_refs(1, &roots, &plain.images);
        let next = Value::Model(ModelRc::new(Rows::from_plain(plain.rows, pictures)));
        let current = value(&model);
        let before = model.notified.get();
        assert!(ui_values::reconcile(&current, &next).unwrap());
        let shows_b = (0..5000).filter(|i| *i != 7 && i % 3 == 1).count();
        assert_eq!(
            model.notified.get() - before,
            shows_b,
            "the rows showing b.png"
        );
        assert_eq!((side(0), side(1), side(4), side(7)), (2, 5, 5, 0));
        // A patch moves 50 rows from 100 to the end: keys and pictures move.
        let ops = json!([{"op": "move", "from": 100, "count": 50, "to": 4950}]);
        let parsed = ui_patch::parse(&ops, 5000).unwrap();
        let patch = images
            .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
            .unwrap()
            .unwrap();
        assert!(ui_patch::applicable(&current, 5000, &parsed.shapes, &patch));
        let moved: Vec<_> = written[100..150]
            .iter()
            .map(|row| row.keys.clone())
            .collect();
        let at_end: Vec<_> = patch.written[4950..]
            .iter()
            .map(|row| row.keys.clone())
            .collect();
        assert_eq!(moved, at_end);
        let before = (model.notified.get(), side(100));
        ui_patch::apply(&current, &parsed.shapes, patch.values, &patch.images);
        images.patched_rows(1, "$mount", "rows", patch.written);
        assert_eq!(model.notified.get() - before.0, 2, "out and back in");
        assert_eq!(side(4950), before.1);
    }

    /// The reader thread converts a large write before the request goes to
    /// the UI thread, so a patch sent after it is handled after it: nothing
    /// is in flight on the UI thread. A whole value is moved out of the
    /// request; a large patch carries its rows converted and copied.
    #[test]
    fn large_writes_are_converted_in_line_order_before_the_ui_thread() {
        let whole = json!({"id": 1, "method": "ui.setMount",
            "params": {"property": "rows", "value": list(0..20_000), "seq": 1}});
        let small = json!({"id": 2, "method": "ui.setMount", "params": {"property": "rows",
            "patch": {"base": 1, "seq": 2, "ops": [{"op": "set", "at": 5, "row": row(99_999)}]}}});
        let inserted = list(30_000..31_000);
        let large = json!({"id": 3, "method": "ui.setMount", "params": {"property": "rows",
            "patch": {"base": 2, "seq": 3, "ops": [
                {"op": "remove", "at": 0, "count": 10},
                {"op": "insert", "at": 100, "rows": inserted}]}}});
        let other = json!({"id": 4, "method": "osd.show", "params": {"value": list(0..1000)}});
        let (send, receive) = std::sync::mpsc::channel();
        let lines = vec![whole, small, large, other];
        std::thread::spawn(move || {
            for line in lines {
                send.send(crate::process::request(9, line)).unwrap();
            }
        })
        .join()
        .unwrap();
        let events: Vec<_> = receive.try_iter().collect();
        assert_eq!(events.len(), 4);
        // 1. The whole value: prepared, and taken out of the request.
        let crate::host::Event::Prepared(9, message, prepared) = &events[0] else {
            panic!("prepared")
        };
        assert_eq!(message["id"], 1);
        assert!(message["params"]["value"].is_null());
        assert_eq!(prepared.whole.as_ref(), Some(&list(0..20_000)));
        let model = Rc::new(Rows::from_plain(
            prepared.rows.rows.clone(),
            Images::default(),
        ));
        let current = value(&model);
        let mut images = ui_image::Images::default();
        let recorded = prepared
            .rows
            .costs
            .iter()
            .map(|cost| ui_image::Row {
                keys: Vec::new(),
                cost: *cost,
            })
            .collect();
        images.patched_rows(9, "$mount", "rows", recorded);
        // 2. A small patch: converted on the UI thread, applied after it.
        let crate::host::Event::Request(9, message) = &events[1] else {
            panic!("a request")
        };
        let parsed = ui_patch::parse(&message["params"]["patch"]["ops"], 20_000).unwrap();
        let patch = images
            .patch_rows(9, &[], "$mount", "rows", &parsed, None)
            .unwrap()
            .unwrap();
        ui_patch::apply(&current, &parsed.shapes, patch.values, &patch.images);
        images.patched_rows(9, "$mount", "rows", patch.written);
        // 3. A large patch: its rows converted on the reader thread.
        let crate::host::Event::Prepared(9, message, prepared) = &events[2] else {
            panic!("prepared")
        };
        assert!(prepared.whole.is_none());
        assert_eq!(&prepared.carried, inserted.as_array().unwrap());
        let parsed = ui_patch::parse(&message["params"]["patch"]["ops"], 20_000).unwrap();
        let before = model.notified.get();
        let patch = images
            .patch_rows(9, &[], "$mount", "rows", &parsed, Some(&prepared.rows))
            .unwrap()
            .unwrap();
        ui_patch::apply(&current, &parsed.shapes, patch.values, &patch.images);
        assert_eq!(model.notified.get() - before, 2, "1,000 rows in one range");
        let mut expected = list(0..20_000).as_array().unwrap().clone();
        expected[5] = row(99_999);
        expected.drain(0..10);
        expected.splice(100..100, inserted.as_array().unwrap().clone());
        assert_eq!(
            json_of(&model),
            json_of(&plain_model(&Json::Array(expected)))
        );
        // 4. Not a property write: untouched.
        assert!(matches!(
            &events[3],
            crate::host::Event::Request(9, m) if m["params"]["value"].is_array()
        ));
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::ui::ui_image;
    use crate::ui::ui_patch;
    use crate::ui::ui_values::{self, plain_rows};
    use serde_json::json;

    fn row(i: usize) -> Json {
        json!({"key": format!("r{i}"), "label": format!("{i}. Episode {i}.mkv"), "selected": false})
    }
    fn list(range: std::ops::Range<usize>) -> Vec<Json> {
        range.map(row).collect()
    }
    fn model_of(rows: &[Json]) -> Rc<Rows> {
        let plain = plain_rows(&rows.iter().collect::<Vec<_>>()).unwrap();
        Rc::new(Rows::from_plain(plain.rows, Images::default()))
    }

    /// Review item 1: text values (`$vestraText`) are resolved by the
    /// dispatcher in the plugin's catalog on the UI thread, so a large write
    /// holding one is not converted on the reader thread (it would become a
    /// struct, and a closed settings page would store it unresolved).
    #[test]
    fn writes_with_text_values_are_left_to_the_ui_thread() {
        let mut rows = list(0..300);
        rows[150]["label"] = json!({"$vestraText": "Episode", "args": {}});
        let whole = json!({"id": 1, "method": "settings.set",
            "params": {"id": "p", "property": "rows", "value": rows, "seq": 1}});
        let crate::host::Event::Request(3, message) = crate::process::request(3, whole) else {
            panic!("not prepared")
        };
        assert_eq!(
            message["params"]["value"][150]["label"]["$vestraText"],
            "Episode"
        );
        let mut carried = list(0..300);
        carried[7]["label"] = json!({"$vestraList": ["a", "b"]});
        let patch = json!({"id": 2, "method": "ui.setMount", "params": {"property": "rows",
            "patch": {"base": 1, "seq": 2, "ops": [{"op": "insert", "at": 0, "rows": carried}]}}});
        assert!(matches!(
            crate::process::request(3, patch),
            crate::host::Event::Request(3, _)
        ));
        // Without text values the same writes are prepared.
        let plain = json!({"id": 3, "method": "ui.set",
            "params": {"id": "w", "property": "rows", "value": list(0..300), "seq": 1}});
        assert!(matches!(
            crate::process::request(3, plain),
            crate::host::Event::Prepared(3, _, _)
        ));
    }

    /// Review item 2: a whole value sent in parts is held until its last
    /// part, then written once: a list already shown keeps its model and
    /// changes only the rows that differ (it never shrinks to the first
    /// part). A part on another base is `stale`, any other write drops what
    /// was held, and the parts together are checked against the limits.
    #[test]
    fn parts_of_a_whole_write_are_held_and_written_once() {
        let mut images = ui_image::Images::default();
        let view = ("$mount", "rows");
        let mut next = list(0..2000);
        next[1500]["selected"] = json!(true);
        let first = json!({"property": "rows", "value": next[..700], "seq": 4, "more": true});
        let part = |base: u64, at: usize, rows: &[Json], more: bool| {
            let mut params = json!({"property": "rows",
                "patch": {"base": base, "seq": base + 1, "ops": [{"op": "insert", "at": at, "rows": rows}]}});
            if more {
                params["more"] = json!(true);
            }
            params
        };
        let parts = |images: &mut ui_image::Images, params: &Json| {
            images.parts(1, view.0, view.1, params, None)
        };
        assert!(matches!(
            parts(&mut images, &first).unwrap(),
            ui_patch::Parted::Held
        ));
        assert!(matches!(
            parts(&mut images, &part(4, 700, &next[700..1400], true)).unwrap(),
            ui_patch::Parted::Held
        ));
        let ui_patch::Parted::Done(value, plain, seq) =
            parts(&mut images, &part(5, 1400, &next[1400..], false)).unwrap()
        else {
            panic!("done")
        };
        assert_eq!(
            (seq, value.as_array().unwrap().len(), plain.rows.len()),
            (6, 2000, 2000)
        );
        assert_eq!(value, Json::Array(next.clone()));
        assert_eq!(
            plain.costs,
            next.iter().map(ui_values::cost).collect::<Vec<_>>()
        );
        // Written into the list shown: one row changes, nothing else.
        let live = model_of(&list(0..2000));
        let current = Value::Model(ModelRc::from(live.clone()));
        let incoming = Value::Model(ModelRc::new(Rows::from_plain(
            plain.rows,
            Images::default(),
        )));
        assert!(ui_values::reconcile(&current, &incoming).unwrap());
        assert_eq!(live.notified.get(), 1);
        // A part on another base: stale, and nothing stays held.
        assert!(matches!(
            parts(&mut images, &first).unwrap(),
            ui_patch::Parted::Held
        ));
        assert_eq!(
            parts(&mut images, &part(9, 700, &next[700..800], true))
                .unwrap_err()
                .code,
            "stale"
        );
        assert_eq!(
            parts(&mut images, &part(4, 700, &next[700..800], true))
                .unwrap_err()
                .code,
            "stale"
        );
        // Another write ends the parts.
        assert!(matches!(
            parts(&mut images, &first).unwrap(),
            ui_patch::Parted::Held
        ));
        let other = json!({"property": "rows", "value": [], "seq": 7});
        assert!(matches!(
            parts(&mut images, &other).unwrap(),
            ui_patch::Parted::Not
        ));
        assert!(matches!(
            parts(&mut images, &part(4, 700, &next[700..800], false)).unwrap(),
            ui_patch::Parted::Not
        ));
        // The parts together within the limits (text total here).
        let big = json!("x".repeat(ui_values::MAX_STRING_BYTES));
        let rows = vec![big; ui_values::MAX_TEXT_BYTES / ui_values::MAX_STRING_BYTES];
        let start = json!({"property": "rows", "value": rows, "seq": 1, "more": true});
        assert!(matches!(
            parts(&mut images, &start).unwrap(),
            ui_patch::Parted::Held
        ));
        let over = part(1, 0, &[json!("y")], false);
        let error = parts(&mut images, &over).unwrap_err();
        assert_eq!(error.code, "limit");
        assert!(
            error.message.starts_with("rows: UI value text exceeds"),
            "{}",
            error.message
        );
        // Released with the view.
        assert!(matches!(
            parts(&mut images, &first).unwrap(),
            ui_patch::Parted::Held
        ));
        images.release_view(1, "$mount");
        assert!(images.take_parts(1, "$mount", "rows").is_none());
    }

    fn png(side: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![60; (side * side * 4) as usize])
            .unwrap();
        bytes
    }

    /// Review item 3: a model keeps the pictures its rows show and drops
    /// the others when patches replace or remove rows (thumbnails made as
    /// the user scrolls do not pile up); a move keeps them.
    #[test]
    fn pictures_no_row_shows_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let roots = [root.clone()];
        let path = |name: &str, side: u32| {
            let path = root.join(name);
            std::fs::write(&path, png(side)).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let [a, b, c, d] = [
            path("a.png", 2),
            path("b.png", 3),
            path("c.png", 4),
            path("d.png", 5),
        ];
        let thumb =
            |key: &str, path: &str| json!({"key": key, "thumb": {"$vestraImage": path, "ref": 1}});
        let wire = [
            thumb("0", &a),
            thumb("1", &b),
            thumb("2", &c),
            thumb("3", &a),
        ];
        let plain = plain_rows(&wire.iter().collect::<Vec<_>>()).unwrap();
        let mut images = ui_image::Images::default();
        let (pictures, keys) = images.load_refs(1, &roots, &plain.images);
        let written = plain
            .row_images
            .iter()
            .zip(&plain.costs)
            .map(|(refs, cost)| ui_image::Row {
                keys: refs
                    .iter()
                    .filter_map(|i| keys[*i as usize].clone())
                    .collect(),
                cost: *cost,
            })
            .collect();
        images.patched_rows(1, "$mount", "rows", written);
        let model = Rc::new(Rows::from_plain(plain.rows, pictures));
        assert_eq!(model.picture_count(), 3);
        let current = Value::Model(ModelRc::from(model.clone()));
        let mut patch = |ops: Json, len: usize| {
            let parsed = ui_patch::parse(&ops, len).unwrap();
            let rows = images
                .patch_rows(1, &roots, "$mount", "rows", &parsed, None)
                .unwrap()
                .unwrap();
            ui_patch::apply(&current, &parsed.shapes, rows.values, &rows.images);
            images.patched_rows(1, "$mount", "rows", rows.written);
        };
        // c's only row removed: c goes.
        patch(json!([{"op": "remove", "at": 2, "count": 1}]), 4);
        assert_eq!(model.picture_count(), 2);
        // a's rows: one replaced by d, then the other moved: a stays, d comes.
        patch(json!([{"op": "set", "at": 0, "row": thumb("0", &d)}]), 3);
        assert_eq!(model.picture_count(), 3);
        patch(json!([{"op": "move", "from": 2, "count": 1, "to": 0}]), 3);
        assert_eq!(model.picture_count(), 3, "a move keeps its picture");
        patch(json!([{"op": "remove", "at": 0, "count": 1}]), 3);
        assert_eq!(model.picture_count(), 2, "a's last row gone");
        // A thousand thumbnails replaced one after another stay one.
        for side in 0..20 {
            let e = path(&format!("e{side}.png"), 6 + side);
            patch(json!([{"op": "set", "at": 0, "row": thumb("0", &e)}]), 2);
        }
        assert_eq!(model.picture_count(), 2);
        let Some(Value::Struct(row)) = model.row_data(0) else {
            panic!("a row")
        };
        let Some(Value::Image(picture)) = row.get_field("thumb") else {
            panic!("a picture")
        };
        assert_eq!(picture.size().width, 25);
    }
}
