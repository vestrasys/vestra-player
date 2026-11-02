//! Runtime images (R-U1, `plugin.ui.image`): a property value naming a PNG or
//! JPEG file in the plugin's package or data directory, as the whole value or
//! (SDK 1.5.19, STORE-ROW-LOGO) a field at any depth, such as each row's logo.
//! Checked on every use (path, size, format, dimensions), decoded once per
//! (plugin, path, size, modification time, `maxSide`), optionally scaled down
//! to `maxSide`; each plugin keeps at most 64 MB of decoded pixels, and only
//! images no property shows are evicted, least recently used first. The
//! images nested in the values of one view (a window, panel or settings page)
//! show at most 32 MB of decoded pixels together.
#![forbid(unsafe_code)]
use crate::host::*;
use crate::ui::ui_image;
use crate::ui::ui_markdown;
use crate::ui::ui_values;
use serde_json::Value;
use slint_interpreter::Value as SValue;
use std::io::Read;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SIDE: u32 = 4096;
pub const BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// Decoded pixels the nested images of one view may show (STORE-ROW-LOGO):
/// a write that would pass it is refused with `limit`.
pub const VIEW_BUDGET_BYTES: usize = 32 * 1024 * 1024;
/// The wire form the SDK sends for an `ImageRef` (an internal detail).
pub const WIRE_KEY: &str = "$vestraImage";

/// The path of a wire image reference:
/// `{ "$vestraImage": path, "ref": n, "maxSide"?: n }`.
pub fn reference(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    let path = object.get(WIRE_KEY)?;
    object
        .keys()
        .all(|key| key == WIRE_KEY || key == "ref" || key == "maxSide")
        .then_some(path)
}

/// A wire reference's path and `maxSide` (0: full size).
pub fn parts(value: &Value) -> std::result::Result<(&str, u32), RpcError> {
    let path = reference(value)
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid("image path must be a string"))?;
    let side = match value.get("maxSide") {
        None | Some(Value::Null) => 0,
        Some(side) => side
            .as_u64()
            .filter(|side| (1..=u64::from(MAX_SIDE)).contains(side))
            .ok_or_else(|| RpcError::invalid("image maxSide must be an integer 1..4096"))?
            as u32,
    };
    Ok((path, side))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    owner: u64,
    /// Shared: rows of a large list hold the same key many times.
    path: std::sync::Arc<Path>,
    size: u64,
    modified: u128,
    /// `maxSide`, 0 for the full size.
    side: u32,
}

struct Entry {
    image: slint::Image,
    bytes: usize,
    used: u64,
}

/// A view's property: (owner, view, property). Views are `w:<id>` (tool
/// windows), `$mount` (the panel, docked or floating).
type Slot = (u64, String, String);

/// What the last write of a property left for row patches (`ui_patch`):
/// the write's `seq` (0: the initial properties), and for an array without
/// Markdown documents its rows, in order.
#[derive(Default)]
pub struct Written {
    pub seq: u64,
    /// A whole write arriving in parts (`more`, player 0.6.0), held until
    /// its last part (`ui_patch::Parts`).
    pub parts: Option<crate::ui::ui_patch::Parts>,
    pub rows: Option<Vec<Row>>,
}

/// One row of an array a view shows: the images it shows and what it counts
/// toward the value totals (a patch checks the patched array's totals).
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub keys: Vec<Key>,
    pub cost: ui_values::Cost,
}

#[derive(Default)]
pub struct Images {
    entries: BTreeMap<Key, Entry>,
    shown: BTreeMap<Slot, Key>,
    clock: u64,
    /// The Markdown documents properties show, released with them.
    pub markdown: ui_markdown::Cache,
    /// Per property, released with its view like the images it shows.
    written: BTreeMap<Slot, Written>,
}

fn fail(code: &str, message: impl Into<String>) -> RpcError {
    RpcError::new(code, message)
}

/// Resolve and check the file: absolute, inside one of `roots` after
/// following links, a regular file of at most 16 MB.
pub fn locate(
    path: &str,
    roots: &[PathBuf],
) -> std::result::Result<(PathBuf, u64, u128), RpcError> {
    if !Path::new(path).is_absolute() {
        return Err(fail("invalid", "image path must be absolute"));
    }
    let real = dunce::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => fail("missing", format!("image not found: {path}")),
        _ => fail("invalid", format!("image unavailable: {error}")),
    })?;
    if !roots.iter().any(|root| real.starts_with(root)) {
        return Err(fail(
            "invalid",
            "image must be in the plugin's package or data directory",
        ));
    }
    let meta = std::fs::metadata(&real).map_err(|e| fail("invalid", e.to_string()))?;
    if !meta.is_file() {
        return Err(fail("invalid", "image path is not a file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(fail("invalid", "image file exceeds 16 MB"));
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Ok((real, meta.len(), modified))
}

/// PNG or JPEG by content, at most 4096×4096, as straight-alpha RGBA.
#[cfg(test)]
pub fn decode(bytes: &[u8]) -> std::result::Result<(slint::Image, usize), RpcError> {
    decode_within(bytes, 0)
}

/// `decode`, scaled down (aspect kept, never up) so that neither side is
/// over `side` (0: full size).
pub fn decode_within(
    bytes: &[u8],
    side: u32,
) -> std::result::Result<(slint::Image, usize), RpcError> {
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        image::ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        image::ImageFormat::Jpeg
    } else {
        return Err(fail("invalid", "image must be PNG or JPEG"));
    };
    let reader = || image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|e| fail("invalid", format!("unreadable image: {e}")))?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(fail("invalid", "image must be at most 4096×4096"));
    }
    let mut decoded = reader()
        .decode()
        .map_err(|e| fail("invalid", format!("unreadable image: {e}")))?;
    if side > 0 && (width > side || height > side) {
        decoded = decoded.thumbnail(side, side);
    }
    let pixels = decoded.into_rgba8();
    let (width, height) = pixels.dimensions();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        pixels.as_raw(),
        width,
        height,
    );
    Ok((
        slint::Image::from_rgba8(buffer),
        width as usize * height as usize * 4,
    ))
}

impl Images {
    /// The image for `path`, decoded or from the cache.
    pub fn load(
        &mut self,
        owner: u64,
        roots: &[PathBuf],
        path: &str,
    ) -> std::result::Result<(Key, slint::Image), RpcError> {
        self.load_within(owner, roots, path, 0)
    }
    /// `load`, scaled down to `side` (0: full size).
    pub fn load_within(
        &mut self,
        owner: u64,
        roots: &[PathBuf],
        path: &str,
        side: u32,
    ) -> std::result::Result<(Key, slint::Image), RpcError> {
        let (path, size, modified) = locate(path, roots)?;
        let key = Key {
            owner,
            path: path.into(),
            size,
            modified,
            side,
        };
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            return Ok((key, entry.image.clone()));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&key.path)
            .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| fail("invalid", format!("image unreadable: {e}")))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(fail("invalid", "image file exceeds 16 MB"));
        }
        let (image, size) = decode_within(&bytes, side)?;
        self.entries.insert(
            key.clone(),
            Entry {
                image: image.clone(),
                bytes: size,
                used: self.clock,
            },
        );
        self.evict(owner, &key);
        Ok((key, image))
    }
    /// Record what a property shows now (`None`: not an image).
    pub fn assign(&mut self, owner: u64, view: &str, property: &str, key: Option<Key>) {
        let slot = (owner, view.to_owned(), property.to_owned());
        match key {
            Some(key) => self.shown.insert(slot, key),
            None => self.shown.remove(&slot),
        };
    }
    /// A Markdown property, or an array's rows, show several images:
    /// `<property>#<n>` slots, one per distinct image (what is shown
    /// matters, not how often: 20,000 rows with the same placeholder are
    /// one slot).
    pub fn assign_many(&mut self, owner: u64, view: &str, property: &str, keys: Vec<Key>) {
        let prefix = format!("{property}#");
        self.shown
            .retain(|(o, v, p), _| !(*o == owner && v == view && p.starts_with(&prefix)));
        self.assign(owner, view, property, None);
        let distinct: std::collections::BTreeSet<Key> = keys.into_iter().collect();
        for (index, key) in distinct.into_iter().enumerate() {
            self.assign(owner, view, &format!("{prefix}{index}"), Some(key));
        }
    }
    /// Load the image references of plain rows (`ui_rows`), each once: the
    /// pictures by reference, and each reference's file in `refs` order
    /// (`None`: it failed its checks and shows as an empty image, as in a
    /// whole conversion).
    pub fn load_refs(
        &mut self,
        owner: u64,
        roots: &[PathBuf],
        refs: &[std::sync::Arc<crate::ui::ui_rows::ImageRef>],
    ) -> (crate::ui::ui_rows::Images, Vec<Option<Key>>) {
        let mut pictures = std::collections::HashMap::with_capacity(refs.len());
        let mut keys = Vec::with_capacity(refs.len());
        for image in refs {
            match self.load_within(owner, roots, &image.path, image.side) {
                Ok((key, picture)) => {
                    pictures.insert(image.clone(), picture);
                    keys.push(Some(key));
                }
                Err(error) => {
                    diag!("ui.image {}: not shown: {}", image.path, error.message);
                    pictures.insert(image.clone(), slint::Image::default());
                    keys.push(None);
                }
            }
        }
        (std::rc::Rc::new(pictures), keys)
    }
    pub fn release_view(&mut self, owner: u64, view: &str) {
        self.shown
            .retain(|(o, v, _), _| !(*o == owner && v == view));
        self.markdown.release(owner, Some(view), None);
        self.reset_writes(owner, view);
    }
    pub fn release_owner(&mut self, owner: u64) {
        self.shown.retain(|(o, ..), _| *o != owner);
        self.markdown.release(owner, None, None);
        self.entries.retain(|key, _| key.owner != owner);
        self.written.retain(|(o, ..), _| *o != owner);
    }
    /// The view shows its initial properties again (`ui.mount`, `ui.open`,
    /// `settings.register`): every property's write record is back at 0.
    pub fn reset_writes(&mut self, owner: u64, view: &str) {
        self.written
            .retain(|(o, v, _), _| !(*o == owner && v == view));
    }
    /// The `seq` of the property's last write (0: none since the initial
    /// properties).
    pub fn seq(&self, owner: u64, view: &str, property: &str) -> u64 {
        self.written
            .get(&(owner, view.to_owned(), property.to_owned()))
            .map_or(0, |written| written.seq)
    }
    /// The parts of a whole write held for the property (`ui_patch::Parts`), taken.
    pub fn take_parts(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
    ) -> Option<crate::ui::ui_patch::Parts> {
        self.written
            .get_mut(&(owner, view.to_owned(), property.to_owned()))
            .and_then(|written| written.parts.take())
    }
    /// Hold the parts of a whole write until its last part.
    pub fn put_parts(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        parts: crate::ui::ui_patch::Parts,
    ) {
        self.written
            .entry((owner, view.to_owned(), property.to_owned()))
            .or_default()
            .parts = Some(parts);
    }
    pub fn set_seq(&mut self, owner: u64, view: &str, property: &str, seq: u64) {
        self.written
            .entry((owner, view.to_owned(), property.to_owned()))
            .or_default()
            .seq = seq;
    }
    /// The rows of the property's array as the view shows them (`None`: not
    /// an array converted by `prop_value`, or one holding documents).
    pub fn written_rows(&self, owner: u64, view: &str, property: &str) -> Option<&Vec<Row>> {
        self.written
            .get(&(owner, view.to_owned(), property.to_owned()))?
            .rows
            .as_ref()
    }
    fn set_row_keys(&mut self, owner: u64, view: &str, property: &str, rows: Option<Vec<Row>>) {
        let slot = (owner, view.to_owned(), property.to_owned());
        match rows {
            Some(rows) => self.written.entry(slot).or_default().rows = Some(rows),
            None => {
                if let Some(written) = self.written.get_mut(&slot) {
                    written.rows = None;
                }
            }
        }
    }
    /// A row patch applied: the rows the property shows now. The property's
    /// shown images are replaced only when some row showed or shows one.
    pub fn patched_rows(&mut self, owner: u64, view: &str, property: &str, rows: Vec<Row>) {
        let before = self
            .written_rows(owner, view, property)
            .is_some_and(|rows| rows.iter().any(|row| !row.keys.is_empty()));
        if before || rows.iter().any(|row| !row.keys.is_empty()) {
            let keys = rows.iter().flat_map(|row| &row.keys).cloned().collect();
            self.assign_many(owner, view, property, keys);
        }
        self.set_row_keys(owner, view, property, Some(rows));
    }
    /// Decoded pixels `view` shows, each image once, leaving out what
    /// `property` shows (the value being replaced).
    pub fn view_bytes(&self, owner: u64, view: &str, property: &str) -> usize {
        let prefix = format!("{property}#");
        let keys: std::collections::BTreeSet<&Key> = self
            .shown
            .iter()
            .filter(|((o, v, p), _)| {
                *o == owner && v == view && p != property && !p.starts_with(&prefix)
            })
            .map(|(_, key)| key)
            .collect();
        keys.into_iter().map(|key| self.entry_bytes(key)).sum()
    }
    /// Decoded bytes of a cached image (0 when it is not cached).
    pub fn entry_bytes(&self, key: &Key) -> usize {
        self.entries.get(key).map_or(0, |entry| entry.bytes)
    }
    /// Bytes of decoded pixels this plugin keeps.
    pub fn bytes(&self, owner: u64) -> usize {
        self.entries
            .iter()
            .filter(|(key, _)| key.owner == owner)
            .map(|(_, entry)| entry.bytes)
            .sum()
    }
    /// Over budget: drop the least recently used images no property shows.
    fn evict(&mut self, owner: u64, keep: &Key) {
        while self.bytes(owner) > BUDGET_BYTES {
            let shown: std::collections::BTreeSet<&Key> = self.shown.values().collect();
            let victim = self
                .entries
                .iter()
                .filter(|(key, _)| key.owner == owner && *key != keep && !shown.contains(key))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            match victim {
                Some(key) => {
                    self.entries.remove(&key);
                }
                None => break,
            }
        }
    }
}

/// A Markdown document (wire form) anywhere in `value`.
pub fn has_markdown(value: &Value) -> bool {
    if ui_markdown::reference(value).is_some() {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(has_markdown),
        Value::Object(fields) => reference(value).is_none() && fields.values().any(has_markdown),
        _ => false,
    }
}

/// Nested image references (wire form) in `value`, as the image hook of
/// `prop_value` meets them.
pub fn image_count(value: &Value) -> usize {
    if reference(value).is_some() {
        return 1;
    }
    match value {
        Value::Array(items) => items.iter().map(image_count).sum(),
        Value::Object(fields) => fields.values().map(image_count).sum(),
        _ => 0,
    }
}

impl Host {
    /// Where a plugin's images may live: its package and data directories;
    /// a parked plugin's too (lazy activation: its bar button and settings
    /// pages still show).
    pub(crate) fn image_roots(&self, owner: u64) -> Vec<PathBuf> {
        if let Some(plugin) = self.plugins.get(&owner) {
            return plugin.origin().image_roots();
        }
        self.on_demand
            .held(owner)
            .map(|held| held.origin.image_roots())
            .unwrap_or_default()
    }
    /// A property value for `owner`'s `view`: JSON, Markdown documents at
    /// any depth, image references as the whole value or at any depth (checked again on
    /// every use, whoever produced the wire value). A failure names the
    /// property (`rows: UI array exceeds 4096 rows`).
    pub(crate) fn prop_value(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
    ) -> std::result::Result<SValue, RpcError> {
        self.prop_value_with(owner, view, property, value, None)
    }
    /// `prop_value` of a whole write whose rows the plugin's reader thread
    /// converted already (`ui_values::prepare_request`).
    pub(crate) fn prop_value_with(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<SValue, RpcError> {
        self.convert_prop(owner, view, property, value, prepared)
            .map_err(|error| error.for_property(property))
    }
    fn convert_prop(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
        prepared: Option<&ui_values::PlainRows>,
    ) -> std::result::Result<SValue, RpcError> {
        let roots = self.image_roots(owner);
        // An array without documents: plain rows (`ui_rows`), images
        // loaded once per reference.
        if let Some(rows) = value.as_array()
            && let Some(plain) = match prepared {
                Some(prepared) if prepared.rows.len() == rows.len() => {
                    Some(std::borrow::Cow::Borrowed(prepared))
                }
                _ if rows.iter().any(has_markdown) => None,
                _ => match ui_values::plain_rows(&rows.iter().collect::<Vec<_>>()) {
                    Ok(plain) => Some(std::borrow::Cow::Owned(plain)),
                    Err(ui_values::NotPlain::Markdown) => None,
                    Err(ui_values::NotPlain::Image(error)) => return Err(error),
                    Err(ui_values::NotPlain::Refused(error)) => return Err(error.into()),
                },
            }
        {
            let base = self.images.view_bytes(owner, view, property);
            let (pictures, keys) = self.images.load_refs(owner, &roots, &plain.images);
            let distinct: std::collections::BTreeSet<&Key> = keys.iter().flatten().collect();
            let shown: usize = distinct
                .iter()
                .map(|key| self.images.entry_bytes(key))
                .sum();
            if base + shown > VIEW_BUDGET_BYTES {
                return Err(RpcError::new(
                    "limit",
                    "the images of one view exceed 32 MB of decoded pixels \
                     (use maxSide, or send fewer rows)",
                ));
            }
            let distinct = distinct.into_iter().cloned().collect();
            let written = plain
                .row_images
                .iter()
                .zip(&plain.costs)
                .map(|(images, cost)| Row {
                    keys: images
                        .iter()
                        .filter_map(|index| keys.get(*index as usize).cloned().flatten())
                        .collect(),
                    cost: *cost,
                })
                .collect();
            self.images
                .markdown
                .release(owner, Some(view), Some(property));
            self.images.assign_many(owner, view, property, distinct);
            self.images
                .set_row_keys(owner, view, property, Some(written));
            return Ok(SValue::Model(slint::ModelRc::new(
                crate::ui::ui_rows::Rows::from_plain(plain.rows.clone(), pictures),
            )));
        }
        if ui_image::reference(value).is_none() {
            // An array without documents: the images of each row, for row
            // patches (`ui_patch`). The image hook runs once per nested
            // image, in row order; a file that fails its checks is `None`.
            let per_row = value
                .as_array()
                .filter(|rows| !rows.iter().any(has_markdown))
                .map(|rows| rows.iter().map(image_count).collect::<Vec<_>>());
            let mut order: Vec<Option<Key>> = Vec::new();
            let mut cache = std::mem::take(&mut self.images.markdown);
            let mut total = self.images.view_bytes(owner, view, property);
            let mut counted = std::collections::BTreeSet::new();
            let images = RefCell::new(&mut self.images);
            let converted = ui_markdown::convert(
                &mut cache,
                (owner, view, property),
                value,
                &mut |path| {
                    images
                        .borrow_mut()
                        .load(owner, &roots, path)
                        .ok()
                        .map(|(key, image)| (Some(key), image))
                },
                // A nested image (STORE-ROW-LOGO): a file that fails its
                // checks is an empty image in its field, as a Markdown image
                // shows its alt text; only a malformed reference or the
                // view's budget refuses the whole write.
                &mut |node| {
                    let (path, side) = ui_image::parts(node)?;
                    let mut images = images.borrow_mut();
                    match images.load_within(owner, &roots, path, side) {
                        Ok((key, image)) => {
                            if counted.insert(key.clone()) {
                                total += images.entry_bytes(&key);
                            }
                            if total > VIEW_BUDGET_BYTES {
                                return Err(RpcError::new(
                                    "limit",
                                    "the images of one view exceed 32 MB of decoded pixels \
                                     (use maxSide, or send fewer rows)",
                                ));
                            }
                            order.push(Some(key.clone()));
                            Ok((Some(key), image))
                        }
                        Err(error) => {
                            diag!("ui.image {path}: not shown: {}", error.message);
                            order.push(None);
                            Ok((None, slint::Image::default()))
                        }
                    }
                },
            );
            self.images.markdown = cache;
            let (converted, shown) = converted?;
            self.images.assign_many(owner, view, property, shown);
            let rows = per_row.map(|counts| {
                let mut order = order.into_iter();
                counts
                    .into_iter()
                    .zip(value.as_array().into_iter().flatten())
                    .map(|(count, row)| Row {
                        keys: order.by_ref().take(count).flatten().collect(),
                        cost: ui_values::cost(row),
                    })
                    .collect()
            });
            self.images.set_row_keys(owner, view, property, rows);
            return Ok(converted);
        }
        let (path, side) = ui_image::parts(value)?;
        let (key, image) = self.images.load_within(owner, &roots, path, side)?;
        self.images.set_row_keys(owner, view, property, None);
        self.images.assign_many(owner, view, property, Vec::new());
        self.images
            .markdown
            .release(owner, Some(view), Some(property));
        self.images.assign(owner, view, property, Some(key));
        Ok(SValue::Image(image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![200; (width * height * 4) as usize])
            .unwrap();
        bytes
    }

    #[test]
    fn wire_references_are_exact_objects() {
        assert!(reference(&json!({"$vestraImage":"/a.png","ref":3})).is_some());
        assert!(reference(&json!({"$vestraImage":"/a.png"})).is_some());
        assert!(reference(&json!({"$vestraImage":"/a.png","x":1})).is_none());
        assert!(reference(&json!("/a.png")).is_none());
    }

    #[test]
    fn paths_formats_and_sizes_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let inside = root.join("cover.png");
        std::fs::write(&inside, png(3, 2)).unwrap();
        let roots = [root.join("pkg"), root.clone()];
        let path = inside.to_str().unwrap();
        assert!(locate(path, &roots).is_ok());
        assert_eq!(locate("cover.png", &roots).unwrap_err().code, "invalid");
        let gone = root.join("gone.png");
        assert_eq!(
            locate(gone.to_str().unwrap(), &roots).unwrap_err().code,
            "missing"
        );
        // Outside the roots, also through a symbolic link.
        let other = tempfile::tempdir().unwrap();
        let outside = other.path().join("x.png");
        std::fs::write(&outside, png(1, 1)).unwrap();
        assert_eq!(
            locate(outside.to_str().unwrap(), &roots[..1])
                .unwrap_err()
                .code,
            "invalid"
        );
        // Windows symlinks need a privilege test machines may lack.
        #[cfg(unix)]
        {
            let link = root.join("link.png");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert_eq!(
                locate(link.to_str().unwrap(), &roots).unwrap_err().code,
                "invalid"
            );
        }
        // Content decides, not the extension.
        let (image, bytes) = decode(&png(3, 2)).unwrap();
        assert_eq!((image.size().width, image.size().height, bytes), (3, 2, 24));
        assert_eq!(decode(b"GIF89a....").unwrap_err().code, "invalid");
        assert_eq!(decode(&png(MAX_SIDE + 1, 1)).unwrap_err().code, "invalid");
        let mut jpeg = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        assert_eq!(decode(&jpeg).unwrap().0.size().width, 4);
    }

    #[test]
    fn cache_keeps_shown_images_and_reloads_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let roots = [root.clone()];
        let path = root.join("a.png");
        std::fs::write(&path, png(2, 2)).unwrap();
        let mut images = Images::default();
        let (first, _) = images.load(1, &roots, path.to_str().unwrap()).unwrap();
        let (again, _) = images.load(1, &roots, path.to_str().unwrap()).unwrap();
        assert_eq!(first, again);
        assert_eq!(images.bytes(1), 16);
        images.assign(1, "$mount", "cover", Some(first.clone()));
        // A rewritten file is a new entry; the shown one stays until replaced.
        std::fs::write(&path, png(4, 4)).unwrap();
        let (changed, image) = images.load(1, &roots, path.to_str().unwrap()).unwrap();
        assert_ne!(first, changed);
        assert_eq!(image.size().width, 4);
        assert_eq!(images.bytes(1), 16 + 64);
        // Over budget: only images no property shows are evicted.
        images.entries.get_mut(&changed).unwrap().bytes = BUDGET_BYTES;
        images.evict(
            1,
            &Key {
                owner: 1,
                path: Path::new("").into(),
                size: 0,
                modified: 0,
                side: 0,
            },
        );
        assert!(images.entries.contains_key(&first));
        assert!(!images.entries.contains_key(&changed));
        images.release_view(1, "$mount");
        images.release_owner(1);
        assert_eq!(images.bytes(1), 0);
    }

    #[test]
    fn max_side_scales_down_and_views_count_what_they_show() {
        // Scaled down with the aspect kept, never up.
        let (image, bytes) = decode_within(&png(200, 100), 64).unwrap();
        assert_eq!((image.size().width, image.size().height), (64, 32));
        assert_eq!(bytes, 64 * 32 * 4);
        assert_eq!(decode_within(&png(20, 10), 64).unwrap().1, 20 * 10 * 4);
        // The wire field: absent, or an integer 1..4096.
        let wire = |side: Value| json!({"$vestraImage": "/a.png", "ref": 1, "maxSide": side});
        assert_eq!(parts(&wire(json!(64))).unwrap(), ("/a.png", 64));
        assert_eq!(parts(&json!({"$vestraImage": "/a.png"})).unwrap().1, 0);
        for bad in [json!(0), json!(4097), json!(1.5), json!("64")] {
            assert_eq!(parts(&wire(bad)).unwrap_err().code, "invalid");
        }
        // One entry per size of the same file.
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let roots = [root.clone()];
        let path = root.join("logo.png");
        std::fs::write(&path, png(128, 128)).unwrap();
        let path = path.to_str().unwrap();
        let mut images = Images::default();
        let (full, _) = images.load(1, &roots, path).unwrap();
        let (small, image) = images.load_within(1, &roots, path, 32).unwrap();
        assert_ne!(full, small);
        assert_eq!(image.size().width, 32);
        assert_eq!(images.entry_bytes(&small), 32 * 32 * 4);
        // A view counts each image once, without the property being replaced.
        images.assign_many(1, "w:1", "rows", vec![small.clone(), small.clone()]);
        images.assign(1, "w:1", "cover", Some(full.clone()));
        images.assign(1, "w:2", "cover", Some(full.clone()));
        assert_eq!(images.view_bytes(1, "w:1", "x"), (128 * 128 + 32 * 32) * 4);
        assert_eq!(images.view_bytes(1, "w:1", "rows"), 128 * 128 * 4);
        assert_eq!(images.view_bytes(1, "w:1", "cover"), 32 * 32 * 4);
        assert_eq!(images.view_bytes(2, "w:1", "x"), 0);
    }
}
