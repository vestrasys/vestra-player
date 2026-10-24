use super::*;
use serde_json::json;

fn catalog(messages: &Value) -> Catalog {
    Catalog::from_json(&json!({ "language": "zh-Hans", "messages": messages }).to_string()).unwrap()
}

#[test]
fn system_languages_map_to_the_supported_locales() {
    for (tag, locale) in [
        ("en-US", Some("en")),
        ("en", Some("en")),
        ("zh-Hans-CN", Some("zh-Hans")),
        ("zh-CN", Some("zh-Hans")),
        ("zh_CN.UTF-8", Some("zh-Hans")),
        ("zh-SG", Some("zh-Hans")),
        ("zh", Some("zh-Hans")),
        ("zh-Hans", Some("zh-Hans")),
        // Traditional Chinese reads English (user ruling 2026-10-24).
        ("zh-Hant-TW", Some("en")),
        ("zh-TW", Some("en")),
        ("zh-HK", Some("en")),
        ("zh-MO", Some("en")),
        ("zh-Hant", Some("en")),
        ("ja-JP", Some("ja")),
        ("ko-KR", Some("ko")),
        ("de-AT", Some("de")),
        ("de_DE@euro", Some("de")),
        ("fr-CA", Some("fr")),
        ("es-419", Some("es")),
        ("pt-BR", None),
        ("it", None),
        ("", None),
    ] {
        assert_eq!(match_tag(tag), locale, "{tag}");
    }
}

#[test]
fn the_preference_wins_then_the_first_supported_system_language() {
    let system = |tags: &[&str]| tags.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    assert_eq!(resolve(Some("ja"), &system(&["de-DE"])), "ja");
    assert_eq!(resolve(Some("system"), &system(&["de-DE"])), "de");
    assert_eq!(resolve(None, &system(&["pt-BR", "fr-FR", "de"])), "fr");
    assert_eq!(resolve(None, &system(&["pt-BR", "it"])), "en");
    assert_eq!(resolve(None, &[]), "en");
    // A value this player does not know (a newer player's) is "system".
    assert_eq!(resolve(Some("zh-Hant"), &system(&["ko"])), "ko");
    assert!(valid_preference("system") && valid_preference("es"));
    assert!(!valid_preference("zh-Hant") && !valid_preference("EN"));
}

#[test]
fn po_files_parse_with_context_plurals_fuzzy_and_obsolete_entries() {
    let text = r#"# Translator comment
msgid ""
msgstr ""
"Language: zh-Hans\n"
"Plural-Forms: nplurals=1; plural=0;\n"

#. Extracted
#: main.ts:3
msgid "Hello"
msgstr "你好"

msgctxt "Menu"
msgid "Open"
msgstr "打开"

msgid "{n} file"
msgid_plural "{n} files"
msgstr[0] "{n} 个文件"

#, fuzzy
msgid "Unsure"
msgstr "不确定"

msgid "Empty"
msgstr ""

msgid ""
"Long "
"line \"quoted\"\n"
msgstr "长\t行"
#~ msgid "Gone"
#~ msgstr "没了"
"#;
    let entries = po::parse(text).unwrap();
    let ids: Vec<_> = entries.iter().map(|e| e.key()).collect();
    assert_eq!(
        ids,
        [
            "Hello",
            "Menu\u{4}Open",
            "{n} file",
            "Unsure",
            "Empty",
            "Long line \"quoted\"\n"
        ]
    );
    assert!(entries[3].fuzzy && !entries[3].translated());
    assert!(!entries[4].translated());
    let catalog = Catalog::from_entries(&entries);
    assert_eq!(catalog.len(), 4);
    assert_eq!(catalog.lookup(Some("Menu"), "Open"), Some("打开"));
    assert_eq!(catalog.lookup(None, "Open"), None);
    assert_eq!(catalog.text("Unsure"), "Unsure");
    assert_eq!(catalog.text("Long line \"quoted\"\n"), "长\t行");
    // The compiled form loads to the same catalog.
    let compiled = po::compile("zh-Hans", &entries);
    assert_eq!(Catalog::from_json(&compiled).unwrap(), catalog);
    assert!(compiled.contains(r#""{n} file":["{n} 个文件","{n} 个文件"]"#));
}

#[test]
fn po_errors_name_the_line() {
    let error = po::parse("msgid \"a\"\nmsgstr \"b\\q\"\n").unwrap_err();
    assert!(error.starts_with("line 2:"), "{error}");
    assert!(po::parse("\"stray\"\n").unwrap_err().starts_with("line 1:"));
    assert!(po::parse("msgfoo \"x\"\n").is_err());
}

#[test]
fn templates_format_like_slint() {
    assert_eq!(format("Speed {}×", &json!([1.5])), "Speed 1.5×");
    assert_eq!(format("{} of {}", &json!([3, 10])), "3 of 10");
    assert_eq!(format("{1} / {0}", &json!(["a", "b"])), "b / a");
    assert_eq!(format("Hi {name}!", &json!({"name":"Ann"})), "Hi Ann!");
    assert_eq!(format("{{literal}} {}", &json!([5.0])), "{literal} 5");
    assert_eq!(
        format("missing {} {3} {x}", &json!([])),
        "missing {} {3} {x}"
    );
    assert_eq!(format("open { brace", &json!([1])), "open { brace");
    assert_eq!(format("stray } brace", &json!([1])), "stray } brace");
    let zh = catalog(&json!({"Speed {}×": "倍速 {}×"}));
    assert_eq!(zh.template("Speed {}×", &json!([2])), "倍速 2×");
    // Without args nothing is formatted: braces show as written.
    assert_eq!(zh.template("Set {a}", &Value::Null), "Set {a}");
    assert_eq!(
        zh.template("Untranslated {}", &json!(["x"])),
        "Untranslated x"
    );
}

#[test]
fn template_arguments_are_checked() {
    assert!(valid_args(&Value::Null).is_ok());
    assert!(valid_args(&json!([1, "a"])).is_ok());
    assert!(valid_args(&json!({"name":"x"})).is_ok());
    assert!(valid_args(&json!("x")).is_err());
    assert!(valid_args(&json!([[1]])).is_err());
    assert!(valid_args(&json!([true])).is_err());
    assert!(valid_args(&json!((0..17).collect::<Vec<_>>())).is_err());
}

/// The cases the SDK's extractor runs too (tests/sdk/host-agreement.test.ts).
#[test]
fn slint_tr_cases_shared_with_the_sdk() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/slint-tr.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let found: Vec<Value> = slint_tr::scan(source)
            .into_iter()
            .map(|call| {
                let mut message = json!({ "id": call.id });
                if let Some(context) = call.context {
                    message["context"] = context.into();
                }
                if let Some(plural) = call.plural {
                    message["plural"] = plural.into();
                }
                message
            })
            .collect();
        assert_eq!(Value::Array(found), case["messages"], "{name}: messages");
        let rewritten = catalog(&case["catalog"]).rewrite(source);
        assert_eq!(
            rewritten,
            case["output"].as_str().unwrap(),
            "{name}: output"
        );
    }
}

#[test]
fn files_take_the_catalog_of_the_directory_that_holds_them() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dunce::canonicalize(dir.path()).unwrap();
    let nested = plugin.join("nested");
    std::fs::create_dir_all(nested.join("ui")).unwrap();
    let outer = Arc::new(catalog(&json!({"A":"甲"})));
    let inner = Arc::new(catalog(&json!({"A":"乙"})));
    register(&plugin, outer.clone());
    register(&nested, inner.clone());
    let file = |path: &Path| catalog_for(path).map(|c| c.text("A").to_owned());
    assert_eq!(file(&plugin.join("ui/w.slint")).as_deref(), Some("甲"));
    assert_eq!(file(&nested.join("ui/w.slint")).as_deref(), Some("乙"));
    unregister(&nested, &outer); // not its catalog: kept
    assert_eq!(file(&nested.join("w.slint")).as_deref(), Some("乙"));
    unregister(&nested, &inner);
    assert_eq!(file(&nested.join("w.slint")).as_deref(), Some("甲"));
    unregister(&plugin, &outer);
    assert_eq!(file(&plugin.join("w.slint")), None);
}

#[test]
fn package_catalogs_load_compiled_json_before_the_source_po() {
    let dir = tempfile::tempdir().unwrap();
    let locales = dir.path().join("locales");
    std::fs::create_dir_all(&locales).unwrap();
    assert!(load_locale(dir.path(), "zh-Hans").is_none());
    std::fs::write(locales.join("zh-Hans.po"), "msgid \"A\"\nmsgstr \"源\"\n").unwrap();
    assert_eq!(load_locale(dir.path(), "zh-Hans").unwrap().text("A"), "源");
    std::fs::write(
        locales.join("zh-Hans.json"),
        r#"{"language":"zh-Hans","messages":{"A":"编译"}}"#,
    )
    .unwrap();
    assert_eq!(
        load_locale(dir.path(), "zh-Hans").unwrap().text("A"),
        "编译"
    );
    // English is the source: never a catalog.
    std::fs::write(locales.join("en.po"), "msgid \"A\"\nmsgstr \"B\"\n").unwrap();
    assert!(load_locale(dir.path(), "en").is_none());
    // A broken file counts as missing.
    std::fs::write(locales.join("ja.po"), "msgid \"A\nmsgstr").unwrap();
    assert!(load_locale(dir.path(), "ja").is_none());
}

/// Every player string the code asks for (`tr("…")`, `tr_args("…", …)`, a
/// `@tr` in `ui/` or in Slint source written by the player) is in
/// `locales/messages.pot`, and every entry there is still used; each
/// translation has exactly the template's entries and placeholders.
#[test]
fn the_host_template_lists_exactly_the_strings_the_player_uses() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut used = std::collections::BTreeSet::new();
    let mut stack = vec![root.join("src"), root.join("ui")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.parent().unwrap().ends_with("i18n") {
                continue;
            }
            let mut text = std::fs::read_to_string(&path).unwrap_or_default();
            // Only production code: test modules come last in every file.
            if let Some(at) = text.find("#[cfg(test)]\nmod tests") {
                text.truncate(at);
            }
            if path.extension().is_some_and(|e| e == "slint") {
                for call in slint_tr::scan(&text) {
                    used.insert(call.id);
                }
            }
            if path.extension().is_some_and(|e| e == "rs") {
                // Slint source the player writes (raw strings): each `@tr(` on its own,
                // since the Slint lexer does not know Rust's character literals.
                for (at, _) in text.match_indices("@tr(") {
                    if let Some(call) = slint_tr::call_at(&text, at) {
                        used.insert(call.id);
                    }
                }
            }
            if path.extension().is_some_and(|e| e == "rs") {
                for marker in ["tr(", "tr_args("] {
                    for (at, _) in text.match_indices(marker) {
                        let before = text[..at].chars().last();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = text[at + marker.len()..].trim_start();
                        if let Some(value) = rust_string(rest) {
                            used.insert(value);
                        }
                    }
                }
            }
        }
    }
    let template =
        po::parse(&std::fs::read_to_string(root.join("locales/messages.pot")).unwrap()).unwrap();
    let listed: std::collections::BTreeSet<_> = template.iter().map(|e| e.id.clone()).collect();
    let missing: Vec<_> = used.difference(&listed).collect();
    let unused: Vec<_> = listed.difference(&used).collect();
    assert!(
        missing.is_empty(),
        "not in locales/messages.pot: {missing:#?}"
    );
    assert!(unused.is_empty(), "no longer used: {unused:#?}");
    for entry in std::fs::read_dir(root.join("locales")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "po") {
            continue;
        }
        let entries = po::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ids: std::collections::BTreeSet<_> = entries.iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids, listed, "{} has the template's entries", path.display());
        for entry in entries.iter().filter(|e| e.translated()) {
            for text in &entry.strings {
                assert_eq!(
                    placeholders(text),
                    placeholders(&entry.id),
                    "{}: placeholders of {:?}",
                    path.display(),
                    entry.id
                );
            }
        }
    }
}

/// `{}` count and named / numbered placeholders, sorted.
fn placeholders(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text.replace("{{", "").replace("}}", "");
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        found.push(rest[open + 1..open + close].to_owned());
        rest = rest[open + close + 1..].to_owned();
    }
    found.sort();
    found
}

/// The Rust string literal at the start of `text` (plain escapes only).
fn rust_string(text: &str) -> Option<String> {
    let mut chars = text.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return None,
            },
            c => out.push(c),
        }
    }
}

#[test]
fn no_per_language_ui_font_until_verification_asks_for_one() {
    for locale in LOCALES {
        assert_eq!(ui_font_family(locale), None, "{locale}");
    }
}

/// The player's Simplified Chinese is its interface text from before English
/// became the source, character for character (the playback plugin's tests
/// and users' eyes know these).
#[test]
fn the_simplified_chinese_catalog_keeps_the_old_texts() {
    let zh = host_catalog("zh-Hans").unwrap();
    assert!(host_catalog("en").is_none());
    let args = |values: &[&str]| json!(values);
    for (template, values, old) in [
        ("Paused", &[][..], "已暂停"),
        ("Playing", &[], "播放"),
        (
            "Back {} s: {} / {}",
            &["10", "00:40", "01:40"],
            "后退 10秒: 00:40 / 01:40",
        ),
        (
            "Forward {} s: {} / {}",
            &["90", "01:40", "01:40"],
            "前进 90秒: 01:40 / 01:40",
        ),
        ("Speed: {}x", &["1.5"], "播放速度: 1.5x"),
        ("Volume: {}%", &["35"], "音量: 35%"),
        ("Mute: On", &[], "静音: 开"),
        ("Mute: Off", &[], "静音: 关"),
        ("Back {} s", &["5"], "后退5秒"),
        ("Forward {} s", &["10.5"], "前进10.5秒"),
        (
            "Next frame: {}",
            &["00:01:23.456"],
            "定位: 下一帧 00:01:23.456",
        ),
        (
            "The {} is used by {}",
            &["侧栏", "vestra.playlist"],
            "侧栏已被 vestra.playlist 使用",
        ),
        (
            "Previous/Next is provided by {}",
            &["vestra.playlist"],
            "上一个/下一个已由 vestra.playlist 提供",
        ),
        (
            "{} is resolved by {}",
            &["文件类型 .nes", "vestra.nes"],
            "文件类型 .nes 已由 vestra.nes 解析",
        ),
        (
            "{}: {}",
            &["vestra.x", "面板未显示：…"],
            "vestra.x 的面板未显示：…",
        ),
    ] {
        let shown = if values.is_empty() {
            zh.text(template).to_owned()
        } else {
            zh.template(template, &args(values))
        };
        assert_eq!(shown, old, "{template}");
    }
    for (key, old) in [
        ("Playback", "播放"),
        ("Speed", "速度"),
        ("Seek", "跳转"),
        ("Settings", "设置"),
        ("Fullscreen", "全屏"),
        ("Quit", "退出"),
        ("Keys and Mouse", "快捷键与鼠标"),
        ("Options", "选项"),
    ] {
        assert_eq!(zh.text(key), old);
    }
}
