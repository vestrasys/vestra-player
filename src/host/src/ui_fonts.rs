//! CJK fallback fonts for every Slint window of the process (main window,
//! panels, tool windows, plugin windows, options): they share one font
//! collection per UI thread, `slint::fontique_011::shared_collection()`.
//!
//! Slint lays text out with parley; a character the window's font lacks is
//! looked up in fontique's fallback families for its script (key: script, no
//! locale; Slint sets none). Without help fontique asks the system for one
//! family per script, and on Windows that one answer can lack the
//! characters or be missing (`platform/fonts.rs` has why). `install` sets,
//! per CJK script, a chain parley tries in order, character by character:
//!
//! 1. the system families of `platform::ui_cjk_fallback_families` that are
//!    installed, in the interface language's order (Windows only);
//! 2. the family the system's own fallback picks (DirectWrite, CoreText,
//!    fontconfig), so nothing it covered is lost;
//! 3. the bundled Noto Sans CJK SC (`font_asset`, already in the binary for
//!    subtitles), where `platform::ui_bundled_cjk_fallback` says so.
//!
//! The chain only adds fallbacks: Latin text keeps the system UI font, and a
//! window's `default-font-family` stays first. `VESTRA_DEBUG_FONTS=1`
//! prints each chain at start-up.
//!
//! `install` also keeps the interface's Latin font (the one Slint lays out
//! text without a `font-family` in) so the host can measure text it lays out
//! itself: the context menu's panel widths (`text_width`, `menu.rs`).
#![forbid(unsafe_code)]

use crate::platform::{self, CjkScript};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};
use slint::fontique_011::fontique::{
    Attributes, Blob, Collection, FallbackKey, FamilyId, FontInfoOverride, GenericFamily,
    QueryStatus, Script, SourceCache,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// One script's chain as installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    pub script: CjkScript,
    pub families: Vec<String>,
    /// How many of `families` (from the front) are system fonts.
    pub system: usize,
}

/// Set the CJK chains on Slint's collection. Call on the UI thread after the
/// backend is selected (`shared_collection` would otherwise create a default
/// backend) and before the first window, so no text is laid out with the
/// old chain.
pub fn install(locale: &str) {
    let bundled = platform::ui_bundled_cjk_fallback().then(|| {
        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> =
            Arc::new(crate::font_asset::bundled_bytes());
        Blob::new(bytes)
    });
    let mut collection = slint::fontique_011::shared_collection();
    set_measure(TextMeasure::interface(&mut collection));
    let chains = apply(&mut collection, bundled, |script| {
        platform::ui_cjk_fallback_families(script, locale)
    });
    let debug = std::env::var_os("VESTRA_DEBUG_FONTS").is_some_and(|v| v == "1");
    for chain in &chains {
        if debug {
            diag!(
                "fonts: {} fallback: {}",
                chain.script.code(),
                chain.families.join(", ")
            );
        }
        if chain.system == 0 && !chain.families.is_empty() {
            diag!(
                "fonts: no system font for {} text; using the bundled {}",
                chain.script.code(),
                chain.families.join(", ")
            );
        } else if chain.families.is_empty() {
            diag!(
                "fonts: no font for {} text; it will show as boxes",
                chain.script.code()
            );
        }
    }
}

/// Advance widths of one font's characters.
pub struct TextMeasure {
    blob: Blob<u8>,
    index: u32,
    /// Advance per pixel of font size; `None`: the font lacks the character.
    advances: RefCell<HashMap<char, Option<f64>>>,
}

impl TextMeasure {
    pub fn new(blob: Blob<u8>, index: u32) -> Option<Self> {
        FontRef::from_index(blob.as_ref(), index).ok()?;
        Some(Self {
            blob,
            index,
            advances: RefCell::default(),
        })
    }

    /// The font Slint lays out text without a `font-family` in: the first
    /// match of its generic families (`i_slint_common::sharedfontique::
    /// FALLBACK_FAMILIES`: sans-serif, then system-ui) at the default
    /// attributes; Arial on Windows, Helvetica on macOS.
    pub fn interface(collection: &mut Collection) -> Option<Self> {
        let mut cache = SourceCache::default();
        let mut query = collection.query(&mut cache);
        query.set_families([GenericFamily::SansSerif, GenericFamily::SystemUi]);
        query.set_attributes(Attributes::default());
        let mut found = None;
        query.matches_with(|font| {
            found = Some((font.blob.clone(), font.index));
            QueryStatus::Stop
        });
        let (blob, index) = found?;
        Self::new(blob, index)
    }

    fn advance(&self, c: char) -> Option<f64> {
        *self.advances.borrow_mut().entry(c).or_insert_with(|| {
            let font = FontRef::from_index(self.blob.as_ref(), self.index).ok()?;
            let glyph = font.charmap().map(c)?;
            let metrics = font.glyph_metrics(Size::new(1.), LocationRef::default());
            metrics.advance_width(glyph).map(f64::from)
        })
    }

    /// Width of `text` at `size` px from the advance widths, without shaping
    /// (kerning narrows Latin text, so this does not under-measure it). A
    /// character the font lacks is drawn in a fallback font and measured by
    /// `missing`.
    pub fn width(&self, text: &str, size: f64, missing: impl Fn(char) -> f64) -> f64 {
        text.chars()
            .map(|c| self.advance(c).map_or_else(|| missing(c), |a| a * size))
            .sum()
    }
}

thread_local! {
    /// This thread's interface font (`install` on the UI thread).
    static MEASURE: RefCell<Option<TextMeasure>> = const { RefCell::new(None) };
}

/// Measure this thread's text with `measure` from now on (`install`, tests).
pub fn set_measure(measure: Option<TextMeasure>) {
    MEASURE.with(|slot| *slot.borrow_mut() = measure);
}

/// Width of `text` at `size` px in the interface font; `None` before
/// `install` on this thread or without a system Latin font.
pub fn text_width(text: &str, size: f64, missing: impl Fn(char) -> f64) -> Option<f64> {
    MEASURE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|measure| measure.width(text, size, missing))
    })
}

/// fontique's fallback key for `script` without a locale: what parley asks
/// for, since Slint sets no locale on text.
fn key(script: CjkScript) -> FallbackKey {
    let mut tag = [0u8; 4];
    tag.copy_from_slice(script.code().as_bytes());
    FallbackKey::new(Script::from_bytes(tag), None)
}

/// `install` on any collection (tests use one without system fonts).
/// `preferred` names the system families to try first for a script.
pub fn apply(
    collection: &mut Collection,
    bundled: Option<Blob<u8>>,
    preferred: impl Fn(CjkScript) -> Vec<&'static str>,
) -> Vec<Chain> {
    let bundled: Vec<FamilyId> = bundled
        .map(|blob| {
            collection
                .register_fonts(blob, None::<FontInfoOverride<'_>>)
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default();
    let mut chains = Vec::new();
    for script in CjkScript::ALL {
        let key = key(script);
        // The system's own pick, asked before ours replaces it.
        let system_pick: Vec<FamilyId> = collection.fallback_families(key).collect();
        let mut chain: Vec<FamilyId> = Vec::new();
        let names = preferred(script);
        for name in &names {
            if let Some(id) = collection.family_id(name)
                && !chain.contains(&id)
                && !bundled.contains(&id)
                && collection.family(id).is_some()
            {
                chain.push(id);
            }
        }
        for id in system_pick {
            if !chain.contains(&id) && !bundled.contains(&id) {
                chain.push(id);
            }
        }
        let system = chain.len();
        for id in &bundled {
            if !chain.contains(id) {
                chain.push(*id);
            }
        }
        // Nothing of ours to add: leave the system's fallback untouched.
        if !names.is_empty() || !bundled.is_empty() {
            collection.set_fallbacks(key, chain.iter().copied());
        }
        let families = chain
            .iter()
            .map(|id| {
                collection
                    .family_name(*id)
                    .map_or_else(|| format!("{id:?}"), str::to_owned)
            })
            .collect();
        chains.push(Chain {
            script,
            families,
            system,
        });
    }
    chains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{System, cjk_fallback_families};
    use slint::fontique_011::fontique::CollectionOptions;

    fn bundled() -> Blob<u8> {
        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> =
            Arc::new(crate::font_asset::bundled_bytes());
        Blob::new(bytes)
    }

    fn empty() -> Collection {
        Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        })
    }

    fn fallbacks(collection: &mut Collection, script: CjkScript) -> Vec<String> {
        let key = key(script);
        let ids: Vec<FamilyId> = collection.fallback_families(key).collect();
        ids.into_iter()
            .map(|id| collection.family_name(id).unwrap().to_owned())
            .collect()
    }

    /// A Windows install without CJK fonts: every chain is the bundled font,
    /// and parley's lookup (`fallback_families`) returns it.
    #[test]
    fn without_system_fonts_the_bundled_font_covers_every_cjk_script() {
        let mut collection = empty();
        let chains = apply(&mut collection, Some(bundled()), |script| {
            cjk_fallback_families(System::Windows, script, "en")
        });
        assert_eq!(chains.len(), CjkScript::ALL.len());
        for chain in &chains {
            assert_eq!(chain.families, ["Noto Sans CJK SC"], "{:?}", chain.script);
            assert_eq!(chain.system, 0);
            assert_eq!(
                fallbacks(&mut collection, chain.script),
                ["Noto Sans CJK SC"]
            );
        }
        // It has the characters the user saw as boxes (and kana, hangul).
        let font = skrifa::FontRef::new(crate::font_asset::bundled_bytes()).unwrap();
        use skrifa::MetadataProvider;
        let charmap = font.charmap();
        for c in "设置视频播放列表字幕，。ひらがなカタカナ한국어ㄅㄆ".chars()
        {
            assert!(charmap.map(c).is_some(), "{c}");
        }
    }

    /// Installed Windows families come first, in the interface language's
    /// order, then the bundled font; missing ones are skipped.
    #[test]
    fn installed_system_families_come_first_in_language_order() {
        let mut collection = empty();
        for name in ["Microsoft YaHei UI", "Yu Gothic UI", "Malgun Gothic"] {
            let fonts = collection.register_fonts(
                bundled(),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..Default::default()
                }),
            );
            assert!(!fonts.is_empty());
        }
        let chains = apply(&mut collection, Some(bundled()), |script| {
            cjk_fallback_families(System::Windows, script, "ja")
        });
        let han = &chains[0];
        assert_eq!(han.script, CjkScript::Han);
        assert_eq!(
            han.families,
            [
                "Yu Gothic UI",
                "Microsoft YaHei UI",
                "Malgun Gothic",
                "Noto Sans CJK SC"
            ]
        );
        assert_eq!(han.system, 3);
        assert_eq!(fallbacks(&mut collection, CjkScript::Han), han.families);
        let hangul = chains
            .iter()
            .find(|c| c.script == CjkScript::Hangul)
            .unwrap();
        assert_eq!(hangul.families[0], "Malgun Gothic");

        let mut collection = empty();
        for name in ["Microsoft YaHei UI", "Yu Gothic UI"] {
            collection.register_fonts(
                bundled(),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..Default::default()
                }),
            );
        }
        let chains = apply(&mut collection, Some(bundled()), |script| {
            cjk_fallback_families(System::Windows, script, "zh-Hans")
        });
        assert_eq!(
            chains[0].families,
            ["Microsoft YaHei UI", "Yu Gothic UI", "Noto Sans CJK SC"]
        );
    }

    /// macOS: no table, no bundled font, so the system's fallback is kept as is.
    #[test]
    fn nothing_to_add_leaves_the_system_fallback_alone() {
        let mut collection = empty();
        let chains = apply(&mut collection, None, |script| {
            cjk_fallback_families(System::MacOs, script, "zh-Hans")
        });
        assert!(chains.iter().all(|chain| chain.families.is_empty()));
        assert!(fallbacks(&mut collection, CjkScript::Han).is_empty());
    }
}
