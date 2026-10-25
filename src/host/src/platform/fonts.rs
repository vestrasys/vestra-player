//! Per-system table: the system fonts the interface falls back to for
//! Chinese, Japanese and Korean text (`ui_fonts::install`), in the order the
//! interface language prefers. Pure, so every system's table is tested on
//! every machine.
//!
//! Why Windows needs one: Slint asks fontique for one fallback family per
//! script, and on Windows fontique asks DirectWrite's system fallback with
//! an empty locale. What DirectWrite answers for Han text then depends on
//! the system's languages (a Japanese or Korean UI font lacks most
//! simplified characters), or it answers nothing; both show boxes. The
//! families below ship with Windows 10 and 11 whatever languages are
//! installed (some editions put the extra ones, such as DengXian, in
//! optional font packs; `ui_fonts` skips families that are not installed,
//! and the bundled Noto Sans CJK SC comes last).

use super::System;

/// The scripts `ui_fonts` sets a fallback chain for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CjkScript {
    /// Han ideographs (and the CJK punctuation parley looks up with them).
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Bopomofo,
}

impl CjkScript {
    pub const ALL: [CjkScript; 5] = [
        CjkScript::Han,
        CjkScript::Hiragana,
        CjkScript::Katakana,
        CjkScript::Hangul,
        CjkScript::Bopomofo,
    ];

    /// The ISO 15924 code fontique keys fallbacks by.
    pub fn code(self) -> &'static str {
        match self {
            CjkScript::Han => "Hani",
            CjkScript::Hiragana => "Hira",
            CjkScript::Katakana => "Kana",
            CjkScript::Hangul => "Hang",
            CjkScript::Bopomofo => "Bopo",
        }
    }
}

/// Which CJK typeface tradition an interface language prefers for Han.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    Simplified,
    Traditional,
    Japanese,
    Korean,
}

/// `zh-Hans`, `zh_CN` → Simplified; `zh-Hant`, `zh-TW`/`HK`/`MO` →
/// Traditional; `ja`, `ko`; anything else → Simplified (Han text in file
/// names and titles is most often Chinese).
fn region(locale: &str) -> Region {
    let tag = locale.replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    match parts.next().unwrap_or_default() {
        "ja" => Region::Japanese,
        "ko" => Region::Korean,
        "zh" if parts.any(|part| matches!(part, "hant" | "tw" | "hk" | "mo")) => {
            Region::Traditional
        }
        _ => Region::Simplified,
    }
}

const WINDOWS_SIMPLIFIED: &[&str] = &[
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "DengXian",
    "SimSun",
    "NSimSun",
    "SimHei",
];
const WINDOWS_TRADITIONAL: &[&str] = &[
    "Microsoft JhengHei UI",
    "Microsoft JhengHei",
    "PMingLiU",
    "MingLiU",
];
const WINDOWS_JAPANESE: &[&str] = &[
    "Yu Gothic UI",
    "Meiryo UI",
    "Yu Gothic",
    "Meiryo",
    "MS UI Gothic",
    "MS Gothic",
];
const WINDOWS_KOREAN: &[&str] = &["Malgun Gothic", "Gulim", "Dotum", "Batang"];

fn windows_group(region: Region) -> &'static [&'static str] {
    match region {
        Region::Simplified => WINDOWS_SIMPLIFIED,
        Region::Traditional => WINDOWS_TRADITIONAL,
        Region::Japanese => WINDOWS_JAPANESE,
        Region::Korean => WINDOWS_KOREAN,
    }
}

/// The Han order for an interface language: its own tradition first.
fn han_order(region: Region) -> [Region; 4] {
    use Region::*;
    match region {
        Simplified => [Simplified, Traditional, Japanese, Korean],
        Traditional => [Traditional, Simplified, Japanese, Korean],
        Japanese => [Japanese, Simplified, Traditional, Korean],
        Korean => [Korean, Traditional, Simplified, Japanese],
    }
}

/// The system families to try, in order, for `script` text in an interface
/// in `locale`. Empty where the system's own fallback is enough (macOS:
/// CoreText; Linux: fontconfig).
pub fn cjk_fallback_families(system: System, script: CjkScript, locale: &str) -> Vec<&'static str> {
    if system != System::Windows {
        return Vec::new();
    }
    let han = han_order(region(locale));
    // A script of one tradition: that tradition's fonts first, then Han's order.
    let first = match script {
        CjkScript::Han => None,
        CjkScript::Hiragana | CjkScript::Katakana => Some(Region::Japanese),
        CjkScript::Hangul => Some(Region::Korean),
        CjkScript::Bopomofo => Some(Region::Traditional),
    };
    first
        .into_iter()
        .chain(han.into_iter().filter(|region| Some(*region) != first))
        .flat_map(windows_group)
        .copied()
        .collect()
}

/// Whether the bundled Noto Sans CJK SC (the subtitle font, `font_asset`)
/// ends every CJK fallback chain: on systems that may lack CJK fonts
/// (Windows editions with optional font packs, minimal Linux). macOS always
/// has PingFang and Hiragino, so its fallback stays CoreText's alone.
pub fn bundled_cjk_fallback(system: System) -> bool {
    system != System::MacOs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_han_follows_the_interface_language() {
        let first = |locale| cjk_fallback_families(System::Windows, CjkScript::Han, locale)[0];
        assert_eq!(first("zh-Hans"), "Microsoft YaHei UI");
        assert_eq!(first("zh_CN"), "Microsoft YaHei UI");
        assert_eq!(first("en"), "Microsoft YaHei UI");
        assert_eq!(first("de-AT"), "Microsoft YaHei UI");
        assert_eq!(first("zh-Hant"), "Microsoft JhengHei UI");
        assert_eq!(first("zh-TW"), "Microsoft JhengHei UI");
        assert_eq!(first("ja"), "Yu Gothic UI");
        assert_eq!(first("ko-KR"), "Malgun Gothic");
    }

    #[test]
    fn windows_chains_name_every_group_once() {
        for locale in ["en", "zh-Hans", "zh-Hant", "ja", "ko"] {
            for script in CjkScript::ALL {
                let chain = cjk_fallback_families(System::Windows, script, locale);
                let total = WINDOWS_SIMPLIFIED.len()
                    + WINDOWS_TRADITIONAL.len()
                    + WINDOWS_JAPANESE.len()
                    + WINDOWS_KOREAN.len();
                assert_eq!(chain.len(), total, "{locale} {script:?}");
                let mut unique = chain.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(unique.len(), total, "{locale} {script:?}");
            }
        }
        let kana = cjk_fallback_families(System::Windows, CjkScript::Katakana, "zh-Hans");
        assert_eq!(&kana[..2], ["Yu Gothic UI", "Meiryo UI"]);
        assert_eq!(kana[WINDOWS_JAPANESE.len()], "Microsoft YaHei UI");
        let hangul = cjk_fallback_families(System::Windows, CjkScript::Hangul, "en");
        assert_eq!(hangul[0], "Malgun Gothic");
        let bopomofo = cjk_fallback_families(System::Windows, CjkScript::Bopomofo, "en");
        assert_eq!(bopomofo[0], "Microsoft JhengHei UI");
    }

    #[test]
    fn other_systems_keep_their_own_fallback() {
        for script in CjkScript::ALL {
            assert!(cjk_fallback_families(System::MacOs, script, "zh-Hans").is_empty());
            assert!(cjk_fallback_families(System::Linux, script, "zh-Hans").is_empty());
        }
        assert!(!bundled_cjk_fallback(System::MacOs));
        assert!(bundled_cjk_fallback(System::Windows));
        assert!(bundled_cjk_fallback(System::Linux));
    }
}
