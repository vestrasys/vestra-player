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
    path: PathBuf,
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

#[derive(Default)]
pub struct Images {
    entries: BTreeMap<Key, Entry>,
    shown: BTreeMap<Slot, Key>,
    clock: u64,
    /// The Markdown documents properties show, released with them.
    pub markdown: ui_markdown::Cache,
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
            path,
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
    /// A Markdown property shows several images: `<property>#<n>` slots.
    pub fn assign_many(&mut self, owner: u64, view: &str, property: &str, keys: Vec<Key>) {
        let prefix = format!("{property}#");
        self.shown
            .retain(|(o, v, p), _| !(*o == owner && v == view && p.starts_with(&prefix)));
        self.assign(owner, view, property, None);
        for (index, key) in keys.into_iter().enumerate() {
            self.assign(owner, view, &format!("{prefix}{index}"), Some(key));
        }
    }
    pub fn release_view(&mut self, owner: u64, view: &str) {
        self.shown
            .retain(|(o, v, _), _| !(*o == owner && v == view));
        self.markdown.release(owner, Some(view), None);
    }
    pub fn release_owner(&mut self, owner: u64) {
        self.shown.retain(|(o, ..), _| *o != owner);
        self.markdown.release(owner, None, None);
        self.entries.retain(|key, _| key.owner != owner);
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

impl Host {
    /// Where a plugin's images may live: its package and data directories.
    pub(crate) fn image_roots(&self, owner: u64) -> Vec<PathBuf> {
        let Some(plugin) = self.plugins.get(&owner) else {
            return Vec::new();
        };
        let mut roots = vec![plugin.directory.clone()];
        if let Some(data) = plugin
            .data_directory()
            .and_then(|d| dunce::canonicalize(d).ok())
        {
            roots.push(data);
        }
        roots
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
        self.convert_prop(owner, view, property, value)
            .map_err(|error| error.for_property(property))
    }
    fn convert_prop(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
    ) -> std::result::Result<SValue, RpcError> {
        let roots = self.image_roots(owner);
        if ui_image::reference(value).is_none() {
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
                            Ok((Some(key), image))
                        }
                        Err(error) => {
                            diag!("ui.image {path}: not shown: {}", error.message);
                            Ok((None, slint::Image::default()))
                        }
                    }
                },
            );
            self.images.markdown = cache;
            let (converted, shown) = converted?;
            self.images.assign_many(owner, view, property, shown);
            return Ok(converted);
        }
        let (path, side) = ui_image::parts(value)?;
        let (key, image) = self.images.load_within(owner, &roots, path, side)?;
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
                path: PathBuf::new(),
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
