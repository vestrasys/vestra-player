//! Settings pages (R-S1, `plugin.settings`) and the options window that holds
//! them (R-H7): a category tree on the left, the page (440 wide, scrolling) on
//! the right, Close at the bottom. The player owns the window; each page is a
//! plugin Slint component with its own bridge global, composed like panels.
#![forbid(unsafe_code)]
use super::*;
use anyhow::Context;

/// Fixed order of the tree (J04); `category_label` has the headers.
pub const CATEGORIES: [&str; 9] = [
    "general",
    "playback",
    "subtitles",
    "video",
    "audio",
    "input",
    "appearance",
    "plugins",
    "other",
];

/// A category header of the tree, in the interface language.
fn category_label(name: &str) -> String {
    use crate::i18n::tr;
    match name {
        "general" => tr("General"),
        "playback" => tr("Playback"),
        "subtitles" => tr("Subtitles"),
        "video" => tr("Video"),
        "audio" => tr("Audio"),
        "input" => tr("Keys and Mouse"),
        "appearance" => tr("Appearance"),
        "plugins" => tr("Plugins"),
        _ => tr("Other"),
    }
}
pub const MAX_PAGES: usize = 8;

#[derive(Clone, Debug)]
pub struct Page {
    pub owner: u64,
    pub manifest: String,
    pub id: String,
    pub category: usize,
    /// As shown: the declared title in the interface language (the
    /// plugin's catalog).
    pub title: String,
    pub order: i64,
    /// Unique per registration: names the page's alias in generated source.
    pub serial: u64,
    pub file: PathBuf,
    pub component: String,
    pub global: String,
    pub properties: BTreeMap<String, Value>,
}

impl Page {
    fn key(&self) -> String {
        format!("{}/{}", self.manifest, self.id)
    }
    pub fn view(&self) -> String {
        format!("$settings:{}", self.id)
    }
}

pub fn alias(serial: u64) -> String {
    format!("Settings{serial}Bridge")
}

struct Open {
    instance: slint_interpreter::ComponentInstance,
    generation: u64,
    /// Page serials in display order: the `current` index points here.
    order: Vec<u64>,
}

#[derive(Default)]
pub struct Settings {
    pages: Vec<Page>,
    next_serial: u64,
    open: Option<Open>,
    /// The page shown (or to show): manifest/id.
    current: Option<String>,
    /// The page set changed while the window is open: rebuild it.
    stale: bool,
}

/// Category index, order, plugin id, page id.
fn sorted(pages: &[Page]) -> Vec<&Page> {
    let mut pages: Vec<&Page> = pages.iter().collect();
    pages.sort_by(|a, b| {
        (a.category, a.order, &a.manifest, &a.id).cmp(&(b.category, b.order, &b.manifest, &b.id))
    });
    pages
}

/// The page to show: `wanted` if it exists, else the first of `category`,
/// else the first of 常规, else the first page.
fn pick<'a>(pages: &[&'a Page], wanted: Option<&str>, category: Option<usize>) -> Option<&'a Page> {
    wanted
        .and_then(|key| pages.iter().find(|p| p.key() == key))
        .or_else(|| category.and_then(|c| pages.iter().find(|p| p.category == c)))
        .or_else(|| pages.iter().find(|p| p.category == 0))
        .or_else(|| pages.first())
        .copied()
}

/// The tree model: a header per non-empty category, then its pages.
fn rows(pages: &[&Page], current: Option<u64>) -> Value {
    let mut rows = Vec::new();
    for (index, name) in CATEGORIES.iter().enumerate() {
        let label = category_label(name);
        let mut first = true;
        for (position, page) in pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.category == index)
        {
            if first {
                rows.push(json!({"label":label,"page":-1,"header":true,"selected":false}));
                first = false;
            }
            rows.push(json!({"label":page.title,"page":position,"header":false,
                "selected":Some(page.serial) == current}));
        }
    }
    Value::Array(rows)
}

/// The window's Slint source: every page imported under its own aliases.
pub fn source(pages: &[&Page]) -> Result<String> {
    let mut source = String::new();
    let mut views = String::new();
    for (index, page) in pages.iter().enumerate() {
        composition::identifier(&page.component)?;
        composition::identifier(&page.global)?;
        let path = composition::import_path(&page.file)?;
        let alias = alias(page.serial);
        source.push_str(&format!(
            "import {{ {} as Page{}View }} from \"{path}\";\nimport {{ {} as {alias} }} from \"{path}\";\nexport {{ {alias} }}\n",
            page.component, page.serial, page.global
        ));
        views.push_str(&format!(
            "                    if root.current == {index}: Page{}View {{ width: 440px; }}\n",
            page.serial
        ));
    }
    source.push_str(&format!(
        r#"import {{ ScrollView }} from "std-widgets.slint";
import {{ Theme }} from "@vestra/theme.slint";
import {{ Button }} from "@vestra/widgets.slint";
export {{ VestraPalette }} from "palette.slint";
struct SettingsRow {{ label: string, page: int, header: bool, selected: bool }}
export component OptionsWindow inherits Window {{
    title: @tr("Options");
    preferred-width: 640px; preferred-height: 480px;
    min-width: 600px; min-height: 420px;
    background: Theme.bg-panel;
    in property <[SettingsRow]> rows;
    in property <int> current: -1;
    callback select(int);
    // Not `close`: Slint 1.17+ has a builtin `Window.close()`.
    callback close-window();
    VerticalLayout {{
        HorizontalLayout {{
            vertical-stretch: 1;
            Rectangle {{
                width: 160px;
                background: Theme.bg-list;
                tree := ScrollView {{
                    VerticalLayout {{
                        alignment: start;
                        padding-top: 4px;
                        for row in root.rows: Rectangle {{
                            height: 20px;
                            background: row.selected ? Theme.selected-bg : touch.has-hover && !row.header ? Theme.hover-bg : transparent;
                            if row.selected: Rectangle {{ x: 8px; y: 4px; width: 2px; height: 12px; border-radius: 1px; background: Theme.accent; }}
                            Text {{
                                x: row.header ? 8px : 20px;
                                width: parent.width - self.x - 4px;
                                text: row.label;
                                font-size: Theme.font-body;
                                vertical-alignment: center;
                                overflow: elide;
                                color: row.header ? Theme.text-dim : row.selected ? Theme.text : Theme.text-list;
                            }}
                            touch := TouchArea {{
                                enabled: !row.header;
                                clicked => {{ root.select(row.page); }}
                            }}
                        }}
                    }}
                }}
            }}
            Rectangle {{ width: 1px; background: Theme.line; }}
            pages := ScrollView {{
                content-width: 440px;
                content-height: max(self.visible-height, column.preferred-height);
                column := VerticalLayout {{
                    alignment: start;
{views}                    if root.current < 0: Text {{
                        width: 440px; height: 80px;
                        text: @tr("Nothing to set here");
                        color: Theme.text-dim; font-size: Theme.font-body;
                        horizontal-alignment: center; vertical-alignment: center;
                    }}
                }}
            }}
        }}
        // Fixed-height footer: every extra pixel goes to the tree and the
        // page (a Button has no max-height, so a stretchable row grew it).
        Rectangle {{ height: 1px; vertical-stretch: 0; background: Theme.line; }}
        buttons := HorizontalLayout {{
            vertical-stretch: 0;
            padding: 8px;
            alignment: end;
            close := Button {{ height: Theme.button-height; text: @tr("Close"); clicked => {{ root.close-window(); }} }}
        }}
    }}
}}
"#,
    ));
    Ok(source)
}

impl Settings {
    pub fn window(&self) -> Option<&slint_interpreter::ComponentInstance> {
        self.open.as_ref().map(|open| &open.instance)
    }
    pub fn page(&self, owner: u64, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.owner == owner && p.id == id)
    }
    fn page_mut(&mut self, owner: u64, id: &str) -> Option<&mut Page> {
        self.pages
            .iter_mut()
            .find(|p| p.owner == owner && p.id == id)
    }
    /// The serial of the page at a tree index of the open window.
    fn at(&self, generation: u64, index: usize) -> Option<u64> {
        let open = self.open.as_ref().filter(|o| o.generation == generation)?;
        open.order.get(index).copied()
    }
    pub fn remove_owner(&mut self, owner: u64) -> bool {
        let before = self.pages.len();
        self.pages.retain(|p| p.owner != owner);
        let changed = self.pages.len() != before;
        self.stale |= changed && self.open.is_some();
        changed
    }
}

fn params_page(params: &Value) -> std::result::Result<&str, RpcError> {
    let id = string(params, "id")?;
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b));
    if !valid {
        return Err(RpcError::invalid("page id: 1–64 letters, digits, . - _"));
    }
    Ok(id)
}

impl Host {
    pub(super) fn settings_dispatch(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let owner = reply.owner;
        match method {
            "settings.register" => {
                let id = params_page(params)?.to_owned();
                let category = CATEGORIES
                    .iter()
                    .position(|name| params["category"] == *name)
                    .ok_or_else(|| RpcError::invalid("unknown settings category"))?;
                let title = string(params, "title")?;
                if title.trim().is_empty() || title.chars().count() > 24 {
                    return Err(RpcError::invalid("title needs 1–24 characters"));
                }
                let order = match &params["order"] {
                    Value::Null => 0,
                    order => order
                        .as_i64()
                        .filter(|o| o.abs() <= 1_000_000)
                        .ok_or_else(|| RpcError::invalid("order: an integer within ±1000000"))?,
                };
                if self.settings.page(owner, &id).is_some() {
                    return Err(RpcError::invalid(format!("page {id} already exists")));
                }
                if self
                    .settings
                    .pages
                    .iter()
                    .filter(|p| p.owner == owner)
                    .count()
                    >= MAX_PAGES
                {
                    return Err(RpcError::invalid("a plugin has at most 8 settings pages"));
                }
                let plugin = self
                    .plugins
                    .get(&owner)
                    .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
                let file = plugins::package_file(&plugin.directory, string(params, "file")?)?;
                let manifest = plugin.manifest.id.clone();
                let mut properties: BTreeMap<String, Value> =
                    serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                        .map_err(|e| RpcError::invalid(e.to_string()))?;
                let refused = crate::refuse_initial(&mut properties);
                self.settings.next_serial += 1;
                let page = Page {
                    owner,
                    manifest,
                    id,
                    category,
                    title: plugin.text(title),
                    order,
                    serial: self.settings.next_serial,
                    file,
                    component: string(params, "component")?.into(),
                    global: string(params, "global")?.into(),
                    properties,
                };
                // Compile the page alone, so a broken page fails its own
                // registration instead of the shared window.
                composition::build(source(&[&page])?, "OptionsWindow")
                    .map_err(|e| RpcError::new("ui-prepare", format!("{e:#}")))?;
                self.settings.pages.push(page);
                self.settings.stale |= self.settings.open.is_some();
                self.report_refused(owner, refused);
                Ok(Some(Value::Null))
            }
            "settings.unregister" => {
                let id = params_page(params)?.to_owned();
                let before = self.settings.pages.len();
                self.settings
                    .pages
                    .retain(|p| !(p.owner == owner && p.id == id));
                if self.settings.pages.len() != before {
                    self.images.release_view(owner, &format!("$settings:{id}"));
                    self.settings.stale |= self.settings.open.is_some();
                }
                Ok(Some(Value::Null))
            }
            "settings.set" => {
                let id = params_page(params)?.to_owned();
                let property = string(params, "property")?.to_owned();
                let page = self
                    .settings
                    .page(owner, &id)
                    .ok_or_else(|| RpcError::new("missing", "page not registered"))?;
                let (view, serial) = (page.view(), page.serial);
                let window = self.settings.window().map(ComponentHandle::clone_strong);
                if let Some(window) = window {
                    let value = self.prop_value(owner, &view, &property, &params["value"])?;
                    let alias = alias(serial);
                    let current = window
                        .get_global_property(&alias, &property)
                        .map_err(|e| RpcError::new("ui", e.to_string()))?;
                    if !ui_values::reconcile(&current, &value)? {
                        window
                            .set_global_property(&alias, &property, value)
                            .map_err(|e| RpcError::new("ui", e.to_string()))?;
                    }
                } else if ui_image::reference(&params["value"]).is_some() {
                    // Checked now, shown when the window opens.
                    self.prop_value(owner, &view, &property, &params["value"])?;
                } else {
                    RpcError::check_property(&property, &params["value"])?;
                }
                if let Some(page) = self.settings.page_mut(owner, &id) {
                    page.properties.insert(property, params["value"].clone());
                }
                Ok(Some(Value::Null))
            }
            "settings.show" => {
                if self.headless {
                    return Err(RpcError::new("unavailable", "no user interface"));
                }
                if !params["id"].is_null() {
                    let id = params_page(params)?;
                    let page = self
                        .settings
                        .page(owner, id)
                        .ok_or_else(|| RpcError::invalid("not a settings page of this plugin"))?;
                    self.settings.current = Some(page.key());
                }
                self.settings_open()
                    .map_err(|e| RpcError::new("ui", format!("{e:#}")))?;
                Ok(Some(Value::Null))
            }
            _ => Err(RpcError::new("unsupported", "unknown settings method")),
        }
    }
    /// Open (or rebuild) the window, select the current page, bring it front.
    fn settings_open(&mut self) -> Result<()> {
        if self.settings.open.is_none() || self.settings.stale {
            self.settings_build()?;
        }
        self.settings_select(None)?;
        if let Some(open) = &self.settings.open {
            open.instance.show()?;
            crate::platform::shown(open.instance.window());
            use slint::winit_030::WinitWindowAccessor;
            open.instance
                .window()
                .with_winit_window(|window| window.focus_window());
        }
        Ok(())
    }
    fn settings_build(&mut self) -> Result<()> {
        let order: Vec<u64> = sorted(&self.settings.pages)
            .iter()
            .map(|p| p.serial)
            .collect();
        let pages: Vec<Page> = order
            .iter()
            .filter_map(|serial| {
                self.settings
                    .pages
                    .iter()
                    .find(|p| p.serial == *serial)
                    .cloned()
            })
            .collect();
        let refs: Vec<&Page> = pages.iter().collect();
        let definition = composition::build(source(&refs)?, "OptionsWindow")?;
        let instance = definition.create()?;
        let mut reports = Vec::new();
        for page in &pages {
            let alias = alias(page.serial);
            crate::ui_values::own_default_models(&instance, Some(&alias))?;
            for (name, value) in &page.properties {
                match self.prop_value(page.owner, &page.view(), name, value) {
                    Ok(value) => composition::set_alias_value(&instance, &alias, name, value)?,
                    Err(error) => {
                        // Checked when written (`check_property`): an image
                        // removed since. Logged and reported to the plugin.
                        eprintln!("settings property {}/{name}: {}", page.key(), error.message);
                        reports.push((page.owner, crate::ui_error(&error)));
                    }
                }
            }
            let callbacks = definition
                .global_callbacks(&alias)
                .with_context(|| format!("settings global unavailable: {alias}"))?;
            for callback in callbacks {
                let events = self.events.clone();
                let (owner, view, name) = (page.owner, page.view(), callback.clone());
                instance.set_global_callback(&alias, &callback, move |args| {
                    let message = match composition::json_args(args) {
                        Ok(args) => json!({"view":view,"callback":name,"args":args}),
                        Err(error) => {
                            let _ = events.send(Event::PluginMessage(
                                owner,
                                "ui.error".into(),
                                json!({"code":"ui","message":format!("callback {name}: {error}")}),
                            ));
                            return SValue::Void;
                        }
                    };
                    let _ = events.send(Event::UiCallback(owner, message));
                    SValue::Void
                })?;
            }
        }
        for (owner, report) in reports {
            self.send(owner, report);
        }
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        let events = self.events.clone();
        instance.set_callback("select", move |args| {
            if let Some(SValue::Number(index)) = args.first()
                && *index >= 0.
            {
                let _ = events.send(Event::Settings(generation, Some(*index as usize)));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        instance.set_callback("close-window", move |_| {
            let _ = events.send(Event::Settings(generation, None));
            SValue::Void
        })?;
        let events = self.events.clone();
        instance.window().on_close_requested(move || {
            let _ = events.send(Event::Settings(generation, None));
            slint::CloseRequestResponse::KeepWindowShown
        });
        // ⌘W / Ctrl+W (the platform's standard keys) close it like 关闭.
        let shortcuts = crate::ui::WindowShortcuts::default();
        {
            use slint::winit_030::{EventResult, WinitWindowAccessor};
            instance
                .window()
                .on_winit_window_event(move |window, event| {
                    if shortcuts.filter(window, event) {
                        EventResult::PreventDefault
                    } else {
                        EventResult::Propagate
                    }
                });
        }
        self.theme_view(&instance);
        if let Some(old) = self.settings.open.take() {
            let window = old.instance.window();
            let scale = window.scale_factor();
            let size = window.size();
            instance.window().set_size(slint::LogicalSize::new(
                size.width as f32 / scale,
                size.height as f32 / scale,
            ));
            self.placements.show_at(&instance, window.position())?;
            crate::platform::shown(instance.window());
            let _ = old.instance.hide();
        }
        self.settings.open = Some(Open {
            instance,
            generation,
            order,
        });
        self.settings.stale = false;
        Ok(())
    }
    /// Show `serial`, or the current / last page with the fallbacks of `pick`.
    fn settings_select(&mut self, serial: Option<u64>) -> Result<()> {
        let pages = sorted(&self.settings.pages);
        let wanted = match serial {
            Some(serial) => pages.iter().find(|p| p.serial == serial).map(|p| p.key()),
            None => self.settings.current.clone().or_else(|| {
                self.ui_state
                    .state
                    .settings
                    .as_ref()
                    .and_then(|s| s.last_page.clone())
            }),
        };
        // A removed page falls back to its category (the one last shown), then 常规.
        let chosen = pick(&pages, wanted.as_deref(), self.settings_category);
        let (key, chosen_serial, category) = match chosen {
            Some(page) => (Some(page.key()), Some(page.serial), Some(page.category)),
            None => (None, None, None),
        };
        let tree = rows(&pages, chosen_serial);
        self.settings.current = key.clone();
        self.settings_category = category;
        if let Some(open) = &self.settings.open {
            let index = chosen_serial
                .and_then(|serial| open.order.iter().position(|s| *s == serial))
                .map_or(-1., |i| i as f64);
            open.instance
                .set_property("rows", ui_values::from_json(&tree)?)?;
            open.instance.set_property("current", index.into())?;
        }
        if key.is_some() {
            let last = self
                .ui_state
                .state
                .settings
                .get_or_insert_with(Default::default);
            if last.last_page != key {
                last.last_page = key;
                let now = self.now_ms();
                self.ui_state.touch(now);
            }
        }
        Ok(())
    }
    /// The window's tree and close button.
    pub(super) fn settings_event(&mut self, generation: u64, index: Option<usize>) {
        match index {
            Some(index) => {
                if let Some(serial) = self.settings.at(generation, index) {
                    let _ = self.settings_select(Some(serial));
                }
            }
            None => {
                if let Some(open) = self.settings.open.take_if(|o| o.generation == generation) {
                    let _ = open.instance.hide();
                }
            }
        }
    }
    /// Pages added or removed while the window is open: rebuild it.
    pub(super) fn settings_tick(&mut self) {
        if self.settings.stale
            && self.settings.open.is_some()
            && let Err(error) = self
                .settings_build()
                .and_then(|()| self.settings_select(None))
        {
            eprintln!("host.error settings window: {error:#}");
            self.settings.stale = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(manifest: &str, id: &str, category: usize, order: i64, serial: u64) -> Page {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mount");
        Page {
            owner: serial,
            manifest: manifest.into(),
            id: id.into(),
            category,
            title: id.into(),
            order,
            serial,
            file: base.join("backdrop.slint"),
            component: "Backdrop".into(),
            global: "Bridge".into(),
            properties: BTreeMap::new(),
        }
    }

    #[test]
    fn tree_orders_categories_pages_and_falls_back() {
        let pages = [
            page("b.plugin", "keys", 5, 0, 1),
            page("a.plugin", "look", 6, 10, 2),
            page("c.plugin", "logo", 6, 10, 3),
            page("d.plugin", "first", 6, 0, 4),
        ];
        let sorted = sorted(&pages);
        let ids: Vec<_> = sorted.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["keys", "first", "look", "logo"]);
        let tree = rows(&sorted, Some(2));
        let labels: Vec<_> = tree
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            [
                "Keys and Mouse",
                "keys",
                "Appearance",
                "first",
                "look",
                "logo"
            ]
        );
        assert_eq!(tree[4]["selected"], true);
        assert_eq!(tree[4]["page"], 2);
        assert_eq!(
            pick(&sorted, Some("c.plugin/logo"), None).unwrap().id,
            "logo"
        );
        // Gone: the first page of its category, else 常规, else the first page.
        assert_eq!(pick(&sorted, Some("x/gone"), Some(6)).unwrap().id, "first");
        assert_eq!(pick(&sorted, Some("x/gone"), Some(1)).unwrap().id, "keys");
        assert!(pick(&[], None, None).is_none());
    }

    #[test]
    fn options_window_compiles_with_pages_and_without() {
        let pages = [
            page("a.plugin", "one", 6, 0, 7),
            page("b.plugin", "two", 0, 0, 8),
        ];
        let refs: Vec<&Page> = sorted(&pages);
        let definition = composition::build(source(&refs).unwrap(), "OptionsWindow").unwrap();
        assert!(definition.global_properties(&alias(7)).is_some());
        assert!(
            definition
                .global_properties(crate::theme::PALETTE)
                .is_some()
        );
        composition::build(source(&[]).unwrap(), "OptionsWindow").unwrap();
    }

    /// A windowless platform so the options window can be laid out in a test.
    struct Headless(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> std::result::Result<
            std::rc::Rc<dyn slint::platform::WindowAdapter>,
            slint::PlatformError,
        > {
            Ok(self.0.clone())
        }
    }

    /// Geometry of the window's parts at a size, as `[tree height, page top,
    /// page height, close top, close width, close height]` in logical pixels.
    fn measure(pages: &[&Page], width: f32, height: f32) -> [f32; 6] {
        thread_local! {
            static PLATFORM: () = {
                let window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
                    Default::default(),
                );
                slint::platform::set_platform(Box::new(Headless(window))).unwrap();
            };
        }
        PLATFORM.with(|_| {});
        // Read the parts back through test-only out properties.
        let probes = "    callback close-window();\n\
            out property <length> t-tree-h: tree.height;\n\
            out property <length> t-pages-y: pages.absolute-position.y;\n\
            out property <length> t-pages-h: pages.height;\n\
            out property <length> t-close-y: close.absolute-position.y;\n\
            out property <length> t-close-w: close.width;\n\
            out property <length> t-close-h: close.height;\n";
        let text = source(pages).unwrap();
        assert!(text.contains("    callback close-window();\n"));
        let text = text.replacen("    callback close-window();\n", probes, 1);
        let definition = composition::build(text, "OptionsWindow").unwrap();
        let instance = definition.create().unwrap();
        instance
            .window()
            .set_size(slint::LogicalSize::new(width, height));
        let read = |name: &str| match instance.get_property(name).unwrap() {
            slint_interpreter::Value::Number(n) => n as f32,
            other => panic!("{name}: {other:?}"),
        };
        [
            read("t-tree-h"),
            read("t-pages-y"),
            read("t-pages-h"),
            read("t-close-y"),
            read("t-close-w"),
            read("t-close-h"),
        ]
    }

    /// The tree and the page fill everything above a fixed-height footer
    /// (round 4a fix: the footer used to take half the window and stretch the
    /// 关闭 button into a 60×200 block).
    #[test]
    fn options_window_fills_the_height_above_a_fixed_footer() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings");
        let tall = Page {
            file: base.join("tall.slint"),
            component: "Tall".into(),
            ..page("a.plugin", "keys", 5, 0, 1)
        };
        let short = page("b.plugin", "look", 6, 0, 2);
        // Footer: 1px line, 8px padding around a 21px button.
        let footer = 1.0 + 8.0 + 21.0 + 8.0;
        for pages in [vec![&tall], vec![&short], vec![]] {
            for (width, height) in [(600.0, 460.0), (600.0, 420.0), (900.0, 800.0)] {
                let [tree, pages_y, pages_h, close_y, close_w, close_h] =
                    measure(&pages, width, height);
                let label = format!("{} page(s) at {width}x{height}", pages.len());
                assert_eq!(close_h, 21.0, "{label}: button height");
                assert!(close_w < 120.0, "{label}: button width {close_w}");
                assert_eq!(
                    close_y,
                    height - 8.0 - 21.0,
                    "{label}: button at the bottom"
                );
                assert_eq!(pages_y, 0.0, "{label}");
                assert_eq!(pages_h, height - footer, "{label}: page height");
                assert_eq!(tree, height - footer, "{label}: tree height");
            }
        }
    }

    /// Shows the options window of `pages` headless and changes every
    /// scalar global property of every page while each page is shown.
    fn change_every_global(pages: &[&Page]) -> slint_interpreter::ComponentInstance {
        let window =
            slint::platform::software_renderer::MinimalSoftwareWindow::new(Default::default());
        slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
        let definition = composition::build(source(pages).unwrap(), "OptionsWindow").unwrap();
        let instance = definition.create().unwrap();
        instance
            .set_property("rows", ui_values::from_json(&rows(pages, None)).unwrap())
            .unwrap();
        instance
            .window()
            .set_size(slint::PhysicalSize::new(640, 480));
        instance.show().unwrap();
        let mut buffer = vec![slint::platform::software_renderer::Rgb565Pixel(0); 640 * 480];
        let mut draw = || {
            slint::platform::update_timers_and_animations();
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(&mut buffer, 640);
            });
        };
        draw();
        for (index, shown) in pages.iter().enumerate() {
            instance
                .set_property("current", (index as f64).into())
                .unwrap();
            draw();
            for page in pages {
                let alias = alias(page.serial);
                for (name, _) in definition.global_properties(&alias).unwrap() {
                    let changed = match instance.get_global_property(&alias, &name).unwrap() {
                        SValue::Bool(b) => SValue::Bool(!b),
                        SValue::Number(n) => SValue::Number(n + 1.),
                        _ => continue,
                    };
                    let _ = instance.set_global_property(&alias, &name, changed);
                }
            }
            draw();
            eprintln!("shown with every global changed: {}", shown.component);
        }
        instance
    }

    /// 2026-10-21, real machine (Slint 1.16.1): opening 选项 or 插件管理
    /// panicked in the event loop ("accessing deleted parent (issue #6426)")
    /// once a page's `ComboBox { current-index <=> Global.index; }` saw its
    /// global change. Fixed upstream in Slint 1.18; this fails if an older
    /// Slint comes back.
    #[test]
    fn a_combo_box_two_way_bound_to_a_global_survives_changes() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings");
        let combo = Page {
            file: base.join("combo.slint"),
            component: "ComboPage".into(),
            global: "ComboBridge".into(),
            ..page("a.plugin", "combo", 6, 0, 1)
        };
        let instance = change_every_global(&[&combo]);
        let alias = alias(combo.serial);
        for index in [2., 0., 1.] {
            instance
                .set_global_property(&alias, "index", SValue::Number(index))
                .unwrap();
            slint::platform::update_timers_and_animations();
            assert_eq!(
                instance.get_global_property(&alias, "index").unwrap(),
                SValue::Number(index)
            );
        }
    }

    /// Every built-in page compiles alone (as `settings.register` checks),
    /// and the window of all of them survives every global property
    /// changing on every page.
    #[test]
    fn options_window_of_the_builtin_pages_survives_global_changes() {
        let plugins = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let real = [
            (
                "preferences/ui/appearance.slint",
                "AppearancePage",
                "AppearanceBridge",
                6,
            ),
            (
                "playlist/ui/settings.slint",
                "PlaylistSettingsPage",
                "PlaylistSettings",
                1,
            ),
            ("visualizer/ui/page.slint", "LogoCoverPage", "LogoPage", 6),
            (
                "window/ui/page.slint",
                "WindowSettingsPage",
                "WindowPage",
                1,
            ),
            ("hotkeys/ui/keys.slint", "KeysPage", "Keys", 5),
            ("hotkeys/ui/mouse.slint", "MousePage", "Mouse", 5),
            ("plugins/ui/plugins.slint", "PluginsPage", "Plugins", 7),
        ];
        let mut pages = Vec::new();
        for (i, (file, component, global, category)) in real.into_iter().enumerate() {
            let page = Page {
                file: plugins.join(file),
                component: component.into(),
                global: global.into(),
                ..page("built.in", component, category, 0, i as u64 + 1)
            };
            composition::build(source(&[&page]).unwrap(), "OptionsWindow")
                .unwrap_or_else(|e| panic!("{file}: {e:#}"));
            pages.push(page);
        }
        change_every_global(&sorted(&pages));
    }
}
