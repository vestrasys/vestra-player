//! R-a: Markdown documents inside rows (chat messages), the per-property
//! document cache and in-place updates (`ui_markdown::convert`,
//! `ui_values::reconcile`).
use super::*;
use slint::Model;
use std::sync::atomic::{AtomicU64, Ordering as Atomic};

static REF: AtomicU64 = AtomicU64::new(1);

/// A document as the SDK sends it; every call is a new reference.
fn wire(text: &str) -> Value {
    json!({
        "$vestraMarkdown": {"text": text, "base": null, "images": {}},
        "ref": REF.fetch_add(1, Atomic::Relaxed),
    })
}

fn chat(bodies: &[&str]) -> Value {
    Value::Array(
        bodies
            .iter()
            .enumerate()
            .map(
                |(i, body)| json!({"id": format!("m{i}"), "role": "assistant", "body": wire(body)}),
            )
            .collect(),
    )
}

const SLOT: (u64, &str, &str) = (1, "w:chat", "messages");

/// Nested images as an empty image each (their loading is `ui_image`'s).
fn no_images(node: &Value) -> std::result::Result<(Option<ui_image::Key>, slint::Image), RpcError> {
    ui_image::parts(node)?;
    Ok((None, slint::Image::default()))
}

fn convert_plain(cache: &mut Cache, slot: (u64, &str, &str), value: &Value) -> SValue {
    convert(cache, slot, value, &mut |_| None, &mut no_images)
        .unwrap()
        .0
}

fn field(row: &SValue, name: &str) -> SValue {
    let SValue::Struct(row) = row else {
        panic!("struct row expected: {row:?}")
    };
    row.get_field(name).cloned().unwrap()
}

fn model(value: &SValue) -> slint::ModelRc<SValue> {
    let SValue::Model(model) = value else {
        panic!("model expected")
    };
    model.clone()
}

fn rows(value: &SValue) -> Vec<Rc<RowKey>> {
    let model = model(value);
    let rows = model
        .as_any()
        .downcast_ref::<Rows>()
        .expect("document rows");
    rows.rows.borrow().iter().map(|r| r.key.clone()).collect()
}

fn identity(model: &slint::ModelRc<SValue>) -> *const dyn std::any::Any {
    model.as_any() as *const dyn std::any::Any
}

#[test]
fn unchanged_documents_are_not_parsed_again() {
    let mut cache = Cache::default();
    convert_plain(&mut cache, SLOT, &chat(&["# 一\n\n段落", "你好"]));
    assert_eq!((cache.parsed, cache.reused), (2, 0));
    // New references (a plugin calls `ui.markdown` again), same texts.
    convert_plain(&mut cache, SLOT, &chat(&["# 一\n\n段落", "你好"]));
    assert_eq!((cache.parsed, cache.reused), (2, 2));
    // Only the growing message is parsed.
    convert_plain(&mut cache, SLOT, &chat(&["# 一\n\n段落", "你好，世界"]));
    assert_eq!((cache.parsed, cache.reused), (3, 3));
    // The same document twice in one value is parsed once.
    convert_plain(&mut cache, SLOT, &chat(&["同一", "同一"]));
    assert_eq!((cache.parsed, cache.reused), (4, 4));
    // Another property has its own documents.
    convert_plain(&mut cache, (1, "w:chat", "other"), &chat(&["同一"]));
    assert_eq!((cache.parsed, cache.reused), (5, 4));
    // A slot keeps only what its last value showed.
    assert_eq!(
        cache.slots[&(1, "w:chat".into(), "messages".into())].len(),
        1
    );
    convert_plain(&mut cache, SLOT, &json!([]));
    assert!(
        !cache
            .slots
            .contains_key(&(1, "w:chat".into(), "messages".into()))
    );
    // Released with the view, and with the plugin.
    convert_plain(&mut cache, SLOT, &chat(&["a"]));
    cache.release(1, Some("w:chat"), None);
    assert!(cache.slots.is_empty());
    convert_plain(&mut cache, SLOT, &chat(&["a"]));
    cache.release(2, None, None);
    assert_eq!(cache.slots.len(), 1);
    cache.release(1, None, None);
    assert!(cache.slots.is_empty());
}

#[test]
fn a_changed_document_reuses_its_unchanged_blocks() {
    let mut cache = Cache::default();
    let first = convert_plain(&mut cache, SLOT, &chat(&["# 标题\n\n第一段\n\n第二"]));
    let second = convert_plain(&mut cache, SLOT, &chat(&["# 标题\n\n第一段\n\n第二段继续"]));
    let (before, after) = (
        rows(&field(&model(&first).row_data(0).unwrap(), "body")),
        rows(&field(&model(&second).row_data(0).unwrap(), "body")),
    );
    assert_eq!((before.len(), after.len()), (3, 3));
    assert!(Rc::ptr_eq(&before[0], &after[0]) && Rc::ptr_eq(&before[1], &after[1]));
    assert!(!Rc::ptr_eq(&before[2], &after[2]));
}

#[test]
fn nested_images_limits_and_failures() {
    let mut cache = Cache::default();
    // Images nest (SDK 1.5.19): an `image` field of each row.
    let nested = json!([{"cover": {"$vestraImage": "/a.png", "ref": 1, "maxSide": 64}}]);
    let (value, _) = convert(&mut cache, SLOT, &nested, &mut |_| None, &mut no_images).unwrap();
    assert!(matches!(
        field(&model(&value).row_data(0).unwrap(), "cover"),
        SValue::Image(_)
    ));
    // A malformed reference is `invalid` wherever it is.
    for bad in [
        json!([{"cover": {"$vestraImage": 1, "ref": 1}}]),
        json!([{"cover": {"$vestraImage": "/a.png", "ref": 1, "maxSide": 0}}]),
        json!([{"cover": {"$vestraImage": "/a.png", "ref": 1, "maxSide": 5000}}]),
    ] {
        let error = convert(&mut cache, SLOT, &bad, &mut |_| None, &mut no_images).unwrap_err();
        assert_eq!(error.code, "invalid", "{bad}");
    }
    // Per document: the wire limit, `invalid`.
    let big = json!([{"body": {"$vestraMarkdown": {"text": "x".repeat(MAX_TEXT + 1)}, "ref": 1}}]);
    assert_eq!(
        convert(&mut cache, SLOT, &big, &mut |_| None, &mut no_images)
            .unwrap_err()
            .code,
        "invalid"
    );
    // Per value: 1 MiB of text in all, documents included.
    let part = "段".repeat(70_000);
    let five = [part.as_str(); 5];
    assert!(
        convert(
            &mut cache,
            SLOT,
            &chat(&five[..4]),
            &mut |_| None,
            &mut no_images
        )
        .is_ok()
    );
    let error = convert(
        &mut cache,
        SLOT,
        &chat(&five),
        &mut |_| None,
        &mut no_images,
    )
    .unwrap_err();
    assert!(error.message.contains("total bytes"), "{}", error.message);
    // A failed write keeps the documents the property still shows.
    let before = cache.parsed;
    convert_plain(&mut cache, SLOT, &chat(&five[..4]));
    assert_eq!(cache.parsed, before);
    // Malformed documents are `invalid` at any depth.
    let bad = json!({"a": [{"$vestraMarkdown": {"text": 1}}]});
    assert_eq!(
        convert(&mut cache, SLOT, &bad, &mut |_| None, &mut no_images)
            .unwrap_err()
            .code,
        "invalid"
    );
}

struct Headless(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
impl slint::platform::Platform for Headless {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError>
    {
        Ok(self.0.clone())
    }
}

fn compile(
    source: &str,
    (width, height): (u32, u32),
) -> (
    slint_interpreter::ComponentInstance,
    std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
) {
    let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default());
    slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let result = pollster::block_on(
        crate::ui::compiler().build_from_source(source.into(), dir.path().join("w.slint")),
    );
    let errors: Vec<_> = result
        .diagnostics()
        .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
        .map(|d| d.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let instance = result.component("W").unwrap().create().unwrap();
    instance
        .window()
        .set_size(slint::PhysicalSize::new(width, height));
    instance.show().unwrap();
    (instance, window)
}

fn draw(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    width: usize,
    height: usize,
) -> usize {
    pixels(window, width, height)
        .iter()
        .filter(|p| p.0 != 0xffff)
        .count()
}

fn pixels(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    width: usize,
    height: usize,
) -> Vec<slint::platform::software_renderer::Rgb565Pixel> {
    let mut buffer = vec![slint::platform::software_renderer::Rgb565Pixel(0xffff); width * height];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buffer, width);
    });
    buffer
}

fn click(instance: &slint_interpreter::ComponentInstance, x: f32, y: f32) {
    for event in [
        slint::platform::WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(x, y),
            button: slint::platform::PointerEventButton::Left,
        },
        slint::platform::WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(x, y),
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        instance.window().dispatch_event(event);
    }
}

const LIST: &str = r#"
import { MarkdownBlocks, MarkdownBlock } from "@vestra/markdown.slint";
import { ListView } from "std-widgets.slint";
export struct Message { id: string, role: string, body: [MarkdownBlock] }
export struct Wrong { id: string, role: string, body: string }
export component W inherits Window {
    width: 360px; height: 480px; background: white;
    in property <[Message]> messages;
    in property <[Wrong]> wrong;
    out property <string> copied;
    ListView {
        for message in root.messages: VerticalLayout {
            padding: 8px;
            spacing: 4px;
            Text { text: message.role; }
            MarkdownBlocks {
                blocks: message.body;
                code-copy: true;
                copy-code(code) => { root.copied = code; }
            }
        }
    }
}"#;

/// The fixture of R-a: `MarkdownBlocks` in a `ListView` row. A growing last
/// message and an added one update the live model in place.
#[test]
fn messages_in_a_list_update_in_place() {
    let (instance, window) = compile(LIST, (360, 480));
    let mut cache = Cache::default();
    let first = convert_plain(
        &mut cache,
        SLOT,
        &chat(&["# 你好\n\n第一条", "第二条 **粗体**", "第三条"]),
    );
    // The type decides: a document in a `string` field is refused whole.
    assert_eq!(
        instance.set_property("wrong", first.clone()),
        Err(slint_interpreter::SetPropertyError::WrongType)
    );
    let current = instance.get_property("messages").unwrap();
    assert!(!crate::ui_values::reconcile(&current, &first).unwrap());
    instance.set_property("messages", first).unwrap();
    assert!(draw(&window, 360, 480) > 500, "messages drawn");

    let live = model(&instance.get_property("messages").unwrap());
    let outer = identity(&live);
    let body = |index: usize| model(&field(&live.row_data(index).unwrap(), "body"));
    let (one, three) = (identity(&body(0)), identity(&body(2)));
    let prefix = rows(&SValue::Model(body(2)));

    // The last message grows by a code block; a fourth arrives.
    let next = convert_plain(
        &mut cache,
        SLOT,
        &chat(&[
            "# 你好\n\n第一条",
            "第二条 **粗体**",
            "第三条\n\n```\nlet a = 1;\n```",
            "第四条",
        ]),
    );
    assert_eq!((cache.parsed, cache.reused), (5, 2));
    let current = instance.get_property("messages").unwrap();
    assert!(crate::ui_values::reconcile(&current, &next).unwrap());
    assert_eq!(identity(&live), outer, "the list model is kept");
    assert_eq!(live.row_count(), 4, "the new message is pushed");
    assert_eq!(
        identity(&body(0)),
        one,
        "unchanged messages keep their rows"
    );
    assert_eq!(
        identity(&body(2)),
        three,
        "the growing message keeps its model"
    );
    let grown = rows(&SValue::Model(body(2)));
    assert_eq!(grown.len(), 2);
    assert!(Rc::ptr_eq(&prefix[0], &grown[0]), "its first block is kept");
    assert!(draw(&window, 360, 480) > 500);

    // A changed scalar field rewrites that row only, keeping its document.
    let mut renamed = chat(&[
        "# 你好\n\n第一条",
        "第二条 **粗体**",
        "第三条\n\n```\nlet a = 1;\n```",
        "第四条",
    ]);
    renamed[1]["role"] = json!("user");
    let next = convert_plain(&mut cache, SLOT, &renamed);
    let two = identity(&body(1));
    assert!(
        crate::ui_values::reconcile(&instance.get_property("messages").unwrap(), &next).unwrap()
    );
    assert_eq!(
        field(&live.row_data(1).unwrap(), "role"),
        SValue::String("user".into())
    );
    assert_eq!(identity(&body(1)), two);

    // Removed at the end, still in place.
    let next = convert_plain(&mut cache, SLOT, &chat(&["# 你好\n\n第一条"]));
    assert!(
        crate::ui_values::reconcile(&instance.get_property("messages").unwrap(), &next).unwrap()
    );
    assert_eq!((identity(&live), live.row_count()), (outer, 1));

    // A different shape is installed whole (Slint checks the type).
    let next = convert_plain(&mut cache, SLOT, &json!([{"id": "x", "body": wire("t")}]));
    assert!(
        !crate::ui_values::reconcile(&instance.get_property("messages").unwrap(), &next).unwrap()
    );
    let next = convert_plain(
        &mut cache,
        SLOT,
        &json!([{"id": "x", "role": "r", "body": "t"}]),
    );
    assert!(
        !crate::ui_values::reconcile(&instance.get_property("messages").unwrap(), &next).unwrap()
    );
    assert!(instance.set_property("messages", next).is_err());
}

/// `code-copy` is opt-in: without it there is no button; with it the button
/// reports the code. `MarkdownView` passes it on.
#[test]
fn code_copy_is_opt_in() {
    let (instance, window) = compile(
        r#"
import { MarkdownBlocks, MarkdownView, MarkdownBlock } from "@vestra/markdown.slint";
export component W inherits Window {
    width: 320px; height: 300px; background: white;
    in property <[MarkdownBlock]> doc;
    in property <bool> copy;
    in property <bool> view;
    in-out property <string> copied;
    if !root.view: MarkdownBlocks {
        y: 0px;
        width: 320px;
        blocks: root.doc;
        code-copy: root.copy;
        copy-code(code) => { root.copied = code; }
    }
    if root.view: MarkdownView {
        blocks: root.doc;
        inset: 0px;
        code-copy: root.copy;
        copy-code(code) => { root.copied = code; }
    }
}"#,
        (320, 300),
    );
    let doc = Doc {
        text: "```\nfn main() {}\n```".into(),
        ..Default::default()
    };
    instance
        .set_property("doc", super::model(&doc, |_| None))
        .unwrap();
    let plain = pixels(&window, 320, 300);
    click(&instance, 300., 12.);
    assert_eq!(
        instance.get_property("copied").unwrap(),
        SValue::String("".into())
    );
    instance.set_property("copy", SValue::Bool(true)).unwrap();
    assert!(pixels(&window, 320, 300) != plain, "the button is drawn");
    click(&instance, 300., 12.);
    assert_eq!(
        instance.get_property("copied").unwrap(),
        SValue::String("fn main() {}".into())
    );
    instance
        .set_property("copied", SValue::String("".into()))
        .unwrap();
    instance.set_property("view", SValue::Bool(true)).unwrap();
    draw(&window, 320, 300);
    click(&instance, 300., 12.);
    assert_eq!(
        instance.get_property("copied").unwrap(),
        SValue::String("fn main() {}".into())
    );
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

/// Perf probe (R-a): 200 messages, the last one streamed to 64 KB in
/// 100 ms steps. Measures what `ui.set` does on the host (the value's JSON
/// parsed, converted, reconciled or installed) and, separately, the frame
/// after it. Run with
/// `VESTRA_MARKDOWN_PROBE=1 cargo test --release --locked ui_markdown::rows_tests::stream_probe -- --ignored --nocapture`
/// (without the variable it returns at once: CI runs ignored tests, and the
/// probe takes minutes in a debug build).
#[test]
#[ignore = "perf probe"]
fn stream_probe() {
    if std::env::var_os("VESTRA_MARKDOWN_PROBE").is_none() {
        return;
    }
    let (instance, window) = compile(
        r#"
import { MarkdownBlocks, MarkdownBlock } from "@vestra/markdown.slint";
import { ListView } from "std-widgets.slint";
export struct Message { id: string, role: string, body: [MarkdownBlock] }
export component W inherits Window {
    width: 480px; height: 640px; background: white;
    in property <[Message]> messages;
    in-out property <length> offset;
    out property <length> total: list.viewport-height;
    out property <length> shown: list.visible-height;
    list := ListView {
        viewport-y <=> root.offset;
        for message in root.messages: VerticalLayout {
            padding: 8px;
            MarkdownBlocks { blocks: message.body; code-copy: true; }
        }
    }
}"#,
        (480, 640),
    );
    let message = |i: usize| {
        format!(
            "## 消息 {i}\n\n这是第 {i} 条消息，包含 **粗体**、*斜体*、`行内代码` 和[链接](https://example.com/{i})。\
             Some English text to make the paragraph wrap across a few lines in the view.\n\n\
             第二段：较长的回答通常有好几段，每段若干句。这里再写几句，让历史消息接近真实的长度，\
             也让每条消息有多个块。A second paragraph with more words so that the message is about a kilobyte long, \
             which is what a typical chat answer looks like.\n\n\
             - 第一项\n- 第二项\n  1. 嵌套\n\n> 引用一句\n\n```rust\nfn main() {{\n    println!(\"{i}\");\n}}\n```\n\n\
             | 列 | 值 |\n|---|---|\n| a | {i} |\n"
        )
    };
    let history: Vec<String> = (0..199).map(message).collect();
    let chunk = "流式输出的一段文字，含 **强调** 与 `code`。Streaming text keeps arriving. ";
    let mut last = String::new();
    let mut ticks = 0;
    let mut mode = |label: &str, in_place: bool| {
        let mut cache = Cache::default();
        let (mut set, mut frame) = (Vec::new(), Vec::new());
        last.clear();
        instance
            .set_property("messages", crate::ui_values::from_json(&json!([])).unwrap())
            .unwrap();
        while last.len() < 64 * 1024 {
            last.push_str(chunk);
            if last.len() % 1500 < chunk.len() {
                last.push_str("\n\n");
            }
            if last.len() % 6000 < chunk.len() {
                last.push_str("\n\n```\nlet x = 1;\n```\n\n");
            }
            let bodies: Vec<&str> = history
                .iter()
                .map(String::as_str)
                .chain([last.as_str()])
                .collect();
            let text = serde_json::to_string(
                &json!({"id": "chat", "property": "messages", "value": chat(&bodies)}),
            )
            .unwrap();
            let started = std::time::Instant::now();
            let params: Value = serde_json::from_str(&text).unwrap();
            if !in_place {
                cache = Cache::default();
            }
            let next = convert_plain(&mut cache, SLOT, &params["value"]);
            let current = instance.get_property("messages").unwrap();
            if !in_place
                || !crate::ui_values::reconcile_in(instance.window(), &current, &next).unwrap()
            {
                instance.set_property("messages", next).unwrap();
            }
            set.push(started.elapsed().as_secs_f64() * 1000.);
            let started = std::time::Instant::now();
            let (total, visible) = match (
                instance.get_property("total").unwrap(),
                instance.get_property("shown").unwrap(),
            ) {
                (SValue::Number(t), SValue::Number(v)) => (t, v),
                _ => (0., 0.),
            };
            instance
                .set_property("offset", SValue::Number((visible - total).min(0.)))
                .unwrap();
            draw(&window, 480, 640);
            frame.push(started.elapsed().as_secs_f64() * 1000.);
        }
        ticks = set.len();
        let mut tail = set[set.len() - 50..].to_vec();
        let bytes = serde_json::to_string(&chat(&[last.as_str()]))
            .unwrap()
            .len();
        diag!(
            "{label}: {ticks} ticks, last message {} KB (value {} KB); ui.set ms p50 {:.2} p95 {:.2} max {:.2} (last 50 ticks p95 {:.2}); frame ms p50 {:.2} p95 {:.2} max {:.2}; parsed {} reused {}",
            last.len() / 1024,
            (history.iter().map(String::len).sum::<usize>() + bytes) / 1024,
            percentile(&mut set.clone(), 0.5),
            percentile(&mut set.clone(), 0.95),
            percentile(&mut set, 1.),
            percentile(&mut tail, 0.95),
            percentile(&mut frame.clone(), 0.5),
            percentile(&mut frame.clone(), 0.95),
            percentile(&mut frame, 1.),
            cache.parsed,
            cache.reused,
        );
    };
    mode("before R-a (parse all, replace the model)", false);
    mode("R-a (cache, in place)", true);
    assert!(ticks > 300);
}

/// Images in nested documents load like a whole document's: the files are
/// checked on every write, a reused document only while they are unchanged.
#[test]
fn nested_document_images_are_checked_on_every_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let file = root.join("a.png");
    let png = |width: u32| {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, 2, image::Rgba([200, 0, 0, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    };
    std::fs::write(&file, png(2)).unwrap();
    // Relative to an absolute directory `base`, the documented form: an
    // absolute Windows path written into the text would lose backslashes
    // before punctuation to CommonMark escapes (`\.tmpXYZ` → `.tmpXYZ`).
    let base = root.to_str().unwrap().to_owned();
    let mut images = crate::ui_image::Images::default();
    let mut cache = Cache::default();
    let mut write = |cache: &mut Cache| {
        let roots = [root.clone()];
        let mut value = chat(&["![图](a.png)", "无图"]);
        value[0]["body"]["$vestraMarkdown"]["base"] = json!(base);
        convert(
            cache,
            SLOT,
            &value,
            &mut |path| {
                images
                    .load(1, &roots, path)
                    .ok()
                    .map(|(key, image)| (Some(key), image))
            },
            &mut no_images,
        )
        .unwrap()
    };
    let (value, shown) = write(&mut cache);
    assert_eq!(shown.len(), 1);
    let first = model(&field(&model(&value).row_data(0).unwrap(), "body"));
    assert_eq!(
        field(&first.row_data(0).unwrap(), "has-image"),
        SValue::Bool(true)
    );
    write(&mut cache);
    assert_eq!((cache.parsed, cache.reused), (2, 2));
    // A rewritten file is a changed document; a removed one shows its alt text.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&file, png(4)).unwrap();
    let (value, _) = write(&mut cache);
    assert_eq!((cache.parsed, cache.reused), (3, 3));
    let body = model(&field(&model(&value).row_data(0).unwrap(), "body"));
    assert_eq!(
        field(&body.row_data(0).unwrap(), "image-width"),
        SValue::Number(4.)
    );
    std::fs::remove_file(&file).unwrap();
    let (value, shown) = write(&mut cache);
    assert!(shown.is_empty());
    let body = model(&field(&model(&value).row_data(0).unwrap(), "body"));
    assert_eq!(
        field(&body.row_data(0).unwrap(), "has-image"),
        SValue::Bool(false)
    );
}

/// A chat written several times between two frames, the way the host
/// handles a plugin's `ui.set` calls (all those of one tick, then a frame):
/// the frame shows the last value. Each write is installed whole into a
/// fresh model for the reference.
fn frames_after(
    (instance, window): &(
        slint_interpreter::ComponentInstance,
        std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
    ),
    writes: &[Value],
) -> (Vec<u16>, Vec<u16>) {
    let empty = crate::ui_values::from_json(&json!([])).unwrap();
    instance.set_property("messages", empty).unwrap();
    draw(window, 360, 480);
    let mut cache = Cache::default();
    let mut write = |value: &Value| {
        let next = convert_plain(&mut cache, SLOT, value);
        let current = instance.get_property("messages").unwrap();
        if !crate::ui_values::reconcile_in(instance.window(), &current, &next).unwrap() {
            instance.set_property("messages", next).unwrap();
        }
    };
    write(&writes[0]);
    draw(window, 360, 480);
    for value in &writes[1..] {
        write(value);
    }
    let shown = pixels(window, 360, 480).iter().map(|p| p.0).collect();
    let fresh = convert_plain(&mut Cache::default(), SLOT, writes.last().unwrap());
    instance.set_property("messages", fresh).unwrap();
    let expected = pixels(window, 360, 480).iter().map(|p| p.0).collect();
    (shown, expected)
}

/// Review of the context-menu fix (host 0.5.10): a struct field above a
/// document changed in place leaves the document's repeater with a dirty
/// model binding until the next frame, and Slint 1.18 drops a row change
/// that arrives meanwhile (`RepeaterTracker::row_changed`). A chat message
/// whose status changed and whose text then grew before the frame kept its
/// old text (before `ui_values::reconcile_in`: 868 and 398 stale pixels).
#[test]
fn a_document_written_after_its_message_changed_shows_its_new_text() {
    let message = |role: &str, body: &str| {
        let mut value = chat(&["第一条", body]);
        value[1]["role"] = json!(role);
        value
    };
    let cases = [
        // The status changes, then the text (two `ui.set` calls).
        vec![
            message("assistant", "旧文字"),
            message("thinking", "旧文字"),
            message("thinking", "新的文字，长一些。New text, longer."),
        ],
        // Status and text together, then text streamed into the same
        // paragraph (an added paragraph marks the whole list dirty, which
        // hides the dropped change).
        vec![
            message("assistant", "旧文字"),
            message("thinking", "流式"),
            message("thinking", "流式输出继续"),
            message("thinking", "流式输出继续，**更多**文字。"),
        ],
    ];
    let view = compile(LIST, (360, 480));
    for (index, writes) in cases.iter().enumerate() {
        let (shown, expected) = frames_after(&view, writes);
        let differ = shown.iter().zip(&expected).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "case {index}: {differ} pixels show stale text");
    }
}
