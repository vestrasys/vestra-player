//! `plugin.clipboard`: plain text and PNG/JPEG images on the system
//! clipboard. Called on the UI thread; the headless host has no clipboard.
//! The checks are here; the system side is `platform::clipboard_*`.
#![forbid(unsafe_code)]

use crate::{RpcError, platform};
use serde_json::Value;
use std::path::Path;

pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    Png,
    Jpeg,
}

/// The image kind from the file's leading bytes, not its name.
pub fn image_kind(head: &[u8]) -> Option<Image> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Image::Png)
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Image::Jpeg)
    } else {
        None
    }
}

/// Read and check an image file: absolute path, at most 16 MB, PNG or JPEG.
pub fn read_image(path: &str) -> Result<(Image, Vec<u8>), RpcError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(RpcError::invalid("image path must be absolute"));
    }
    let size = std::fs::metadata(path)
        .map_err(|e| RpcError::new("missing", e.to_string()))?
        .len();
    if size > MAX_IMAGE_BYTES {
        return Err(RpcError::invalid("image must be at most 16 MB"));
    }
    let bytes = std::fs::read(path).map_err(|e| RpcError::new("missing", e.to_string()))?;
    let kind = image_kind(&bytes).ok_or_else(|| RpcError::invalid("image must be PNG or JPEG"))?;
    Ok((kind, bytes))
}

pub fn text(params: &Value) -> Result<&str, RpcError> {
    let text = params["text"]
        .as_str()
        .ok_or_else(|| RpcError::invalid("text requires a string"))?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(RpcError::invalid("text must be at most 1 MiB"));
    }
    Ok(text)
}

/// `clipboard.readText` / `writeText` / `writeImage`.
pub fn dispatch(method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "clipboard.readText" => {
            Ok(platform::clipboard_read_text()?.map_or(Value::Null, Value::String))
        }
        "clipboard.writeText" => {
            platform::clipboard_write_text(text(params)?).map(|()| Value::Null)
        }
        "clipboard.writeImage" => {
            let path = params["path"]
                .as_str()
                .ok_or_else(|| RpcError::invalid("path requires a string"))?;
            let (kind, bytes) = read_image(path)?;
            platform::clipboard_write_image(kind, bytes).map(|()| Value::Null)
        }
        _ => Err(RpcError::new("unsupported", "unknown clipboard method")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn images_are_checked_by_content_size_and_absolute_path() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some(Image::Png));
        assert_eq!(image_kind(&[0xff, 0xd8, 0xff, 0xe0]), Some(Image::Jpeg));
        assert_eq!(image_kind(b"GIF89a"), None);
        assert_eq!(read_image("relative.png").unwrap_err().code, "invalid");
        assert_eq!(
            read_image("/nonexistent/vestra.png").unwrap_err().code,
            "missing"
        );
        let gif = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        std::fs::write(gif.path(), b"GIF89a....").unwrap();
        assert_eq!(
            read_image(gif.path().to_str().unwrap()).unwrap_err().code,
            "invalid"
        );
        let png = tempfile::Builder::new().suffix(".dat").tempfile().unwrap();
        std::fs::write(png.path(), b"\x89PNG\r\n\x1a\n0000").unwrap();
        assert_eq!(
            read_image(png.path().to_str().unwrap()).unwrap().0,
            Image::Png
        );
        let large = tempfile::NamedTempFile::new().unwrap();
        large.as_file().set_len(MAX_IMAGE_BYTES + 1).unwrap();
        assert_eq!(
            read_image(large.path().to_str().unwrap()).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn text_must_be_a_bounded_string() {
        assert_eq!(text(&json!({"text":"中文"})).unwrap(), "中文");
        assert_eq!(text(&json!({"text":3})).unwrap_err().code, "invalid");
        let long = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(text(&json!({ "text": long })).unwrap_err().code, "invalid");
        assert_eq!(
            dispatch("clipboard.paste", &json!({})).unwrap_err().code,
            "unsupported"
        );
    }
}
