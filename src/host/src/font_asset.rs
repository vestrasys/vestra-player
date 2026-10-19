//! Experimental fixed-font bytes for subtitle snapshots; not the final SDK API.
//! A cmap check is not shaping, backend font selection, a fallback guarantee, or
//! a visible-frame fence. Renderers must separately confirm their selected face.
#![forbid(unsafe_code)]

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use ttf_parser::Face;

const FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/noto-cjk/NotoSansCJKsc-Regular.otf");
const DIGEST: &str = "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b";
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
static BUNDLED: OnceLock<FontAsset> = OnceLock::new();

/// The bundled OFL asset is retained independently of plugin/temporary-file life.
/// Copies share one immutable allocation, rather than copying 16 MiB per snapshot.
#[derive(Clone)]
pub struct FontAsset(Arc<[u8]>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontMetadata {
    pub font_id: &'static str,
    pub family: &'static str,
    pub postscript: &'static str,
    pub face_index: u32,
    pub digest: &'static str,
    pub size_bytes: usize,
}

impl FontAsset {
    /// Validate the fixed release asset once before sharing it with any task.
    /// Concurrent first callers may validate twice, but retain one allocation.
    pub fn bundled() -> Result<Self> {
        if let Some(asset) = BUNDLED.get() {
            return Ok(asset.clone());
        }
        if format!("{:x}", Sha256::digest(FONT_BYTES)) != DIGEST {
            bail!("fixed subtitle font digest mismatch");
        }
        Face::parse(FONT_BYTES, 0).context("fixed subtitle font face 0 is invalid")?;
        let candidate = Self(Arc::from(FONT_BYTES));
        Ok(BUNDLED.get_or_init(|| candidate).clone())
    }

    pub fn metadata(&self) -> FontMetadata {
        FontMetadata {
            font_id: "noto-cjk-sc-regular-2.004",
            family: "Noto Sans CJK SC",
            postscript: "NotoSansCJKsc-Regular",
            face_index: 0,
            digest: DIGEST,
            size_bytes: self.0.len(),
        }
    }

    pub fn bytes(&self) -> Arc<[u8]> {
        self.0.clone()
    }

    /// Check direct Unicode cmap coverage, without substituting another font.
    /// Layout whitespace needs no glyph. Other control characters are rejected
    /// rather than silently claiming coverage of unsupported invisible content.
    pub fn supports(&self, text: &str) -> Result<()> {
        if text.len() > MAX_TEXT_BYTES {
            bail!("unsupported: subtitle font coverage input exceeds 2 MiB");
        }
        let face = Face::parse(&self.0, 0).context("fixed subtitle font face is invalid")?;
        for character in text.chars() {
            if matches!(character, '\n' | '\r' | '\t') {
                continue;
            }
            if character.is_control()
                || face.glyph_index(character).is_none_or(|glyph| glyph.0 == 0)
            {
                bail!(
                    "unsupported: fixed subtitle font lacks a supported glyph for U+{:04X}",
                    u32::from(character)
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_asset_has_expected_identity_and_shared_immutable_storage() {
        let first = FontAsset::bundled().unwrap();
        let second = FontAsset::bundled().unwrap();
        assert_eq!(first.metadata().digest, DIGEST);
        assert_eq!(first.metadata().size_bytes, 16_437_364);
        assert_eq!(first.metadata().face_index, 0);
        assert!(Arc::ptr_eq(&first.bytes(), &second.bytes()));
        let held = first.bytes();
        drop(first);
        drop(second);
        assert!(Face::parse(&held, 0).is_ok());
    }

    #[test]
    fn chinese_literal_markup_and_layout_whitespace_use_the_same_face() {
        let font = FontAsset::bundled().unwrap();
        font.supports("中文字幕 测试 <b>原样</b> {\\b1} \\N END\r\n\t")
            .unwrap();
        font.supports("").unwrap();
    }

    #[test]
    fn missing_glyphs_controls_and_excess_input_are_explicitly_rejected() {
        let font = FontAsset::bundled().unwrap();
        assert!(font.supports("\u{10FFFF}").is_err());
        assert!(font.supports("\0").is_err());
        assert!(font.supports(&"a".repeat(MAX_TEXT_BYTES + 1)).is_err());
    }
}
