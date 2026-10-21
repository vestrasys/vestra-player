//! Runtime images (R-U1, `plugin.ui.image`): a property value naming a PNG or
//! JPEG file in the plugin's package or data directory. Checked on every use
//! (path, size, format, dimensions), decoded once per (plugin, path, size,
//! modification time); each plugin keeps at most 64 MB of decoded pixels, and
//! only images no property shows are evicted, least recently used first.
#![forbid(unsafe_code)]
use super::*;
use std::io::Read;

pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SIDE: u32 = 4096;
pub const BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// The wire form the SDK sends for an `ImageRef` (an internal detail).
pub const WIRE_KEY: &str = "$vestraImage";

/// The path of a wire image reference: `{ "$vestraImage": path, "ref": n }`.
pub fn reference(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    let path = object.get(WIRE_KEY)?;
    object
        .keys()
        .all(|key| key == WIRE_KEY || key == "ref")
        .then_some(path)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    owner: u64,
    path: PathBuf,
    size: u64,
    modified: u128,
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
    let real = std::fs::canonicalize(path).map_err(|error| match error.kind() {
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
pub fn decode(bytes: &[u8]) -> std::result::Result<(slint::Image, usize), RpcError> {
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
    let pixels = reader()
        .decode()
        .map_err(|e| fail("invalid", format!("unreadable image: {e}")))?
        .into_rgba8();
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
        let (path, size, modified) = locate(path, roots)?;
        let key = Key {
            owner,
            path,
            size,
            modified,
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
        let (image, size) = decode(&bytes)?;
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
    pub fn release_view(&mut self, owner: u64, view: &str) {
        self.shown
            .retain(|(o, v, _), _| !(*o == owner && v == view));
    }
    pub fn release_owner(&mut self, owner: u64) {
        self.shown.retain(|(o, ..), _| *o != owner);
        self.entries.retain(|key, _| key.owner != owner);
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
    fn image_roots(&self, owner: u64) -> Vec<PathBuf> {
        let Some(plugin) = self.plugins.get(&owner) else {
            return Vec::new();
        };
        let mut roots = vec![plugin.directory.clone()];
        if let Some(data) = plugin.data_directory().and_then(|d| d.canonicalize().ok()) {
            roots.push(data);
        }
        roots
    }
    /// A property value for `owner`'s `view`: JSON, or an image reference
    /// (checked again on every use, whoever produced the wire value).
    pub(super) fn prop_value(
        &mut self,
        owner: u64,
        view: &str,
        property: &str,
        value: &Value,
    ) -> std::result::Result<SValue, RpcError> {
        let Some(path) = ui_image::reference(value) else {
            self.images.assign(owner, view, property, None);
            return Ok(ui_values::from_json(value)?);
        };
        let path = path
            .as_str()
            .ok_or_else(|| RpcError::invalid("image path must be a string"))?;
        let roots = self.image_roots(owner);
        let (key, image) = self.images.load(owner, &roots, path)?;
        self.images.assign(owner, view, property, Some(key));
        Ok(SValue::Image(image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let root = dir.path().canonicalize().unwrap();
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
        let root = dir.path().canonicalize().unwrap();
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
            },
        );
        assert!(images.entries.contains_key(&first));
        assert!(!images.entries.contains_key(&changed));
        images.release_view(1, "$mount");
        images.release_owner(1);
        assert_eq!(images.bytes(1), 0);
    }
}
