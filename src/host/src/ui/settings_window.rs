//! Settings pages (R-S1, `plugin.settings`) and the options window that holds
//! them (R-H7): a category tree on the left, the page (440 wide, scrolling) on
//! the right, Close at the bottom. The player owns the window; each page is a
//! plugin Slint component with its own bridge global, composed like panels.
#![forbid(unsafe_code)]
use crate::host::*;
use crate::process;
use crate::ui::{composition, ui_image, ui_patch, ui_values};
use anyhow::Context;
use anyhow::Result;
use serde_json::{Value, json};
use slint::ComponentHandle;
use slint_interpreter::Value as SValue;
use std::{cell::Cell, collections::BTreeMap, path::PathBuf, rc::Rc, sync::mpsc};

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
    /// Page serials in the order the window was built with: a page's slot
    /// (the `current` index, a row's `page`) points here. The tree may show
    /// them in another order since (a page's order or category changed in
    /// place); the slots stay.
    order: Vec<u64>,
    /// Each page's owner as its callbacks read it (by serial): a woken
    /// plugin's pages move to its new generation without a rebuild.
    owners: BTreeMap<u64, Rc<Cell<u64>>>,
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
    /// Registrations whose page compiles on the `settings-check` thread:
    /// (request, page, initial properties refused). The reply waits for it.
    checking: Vec<(Reply, Page, Vec<RpcError>)>,
    /// The `settings-check` thread's queue: (page serial, options window
    /// source with that page alone).
    checker: Option<mpsc::Sender<(u64, String)>>,
    /// Lazy activation: pages (by serial) kept for an owner without a
    /// process; a live registration of the same id replaces one.
    recorded: std::collections::BTreeSet<u64>,
    /// Each page's `settings.register` as its plugin sent it, without the
    /// properties (the page keeps the current ones): its recording.
    sent: BTreeMap<u64, Value>,
}

/// Compiles each registered page on its own on a thread of its own, so that
/// a broken page fails its own registration (and not the shared window)
/// without the compile, 60–160 ms each, holding the UI thread: at start-up
/// the plugins register 8 pages, and the ticks that answered them took up to
/// a second. The result comes back as `Event::SettingsChecked`.
fn spawn_checker(events: mpsc::Sender<Event>) -> Option<mpsc::Sender<(u64, String)>> {
    let (sender, receiver) = mpsc::channel::<(u64, String)>();
    std::thread::Builder::new()
        .name("settings-check".into())
        .spawn(move || {
            for (serial, source) in receiver {
                // A panic in the compiler answers this registration with an
                // error instead of leaving it (and the ones after) unanswered.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    composition::build(source, "OptionsWindow")
                        .map(|_| ())
                        .map_err(|e| format!("{e:#}"))
                }))
                .unwrap_or_else(|_| Err("the page's Slint compile failed (compiler panic)".into()));
                if events.send(Event::SettingsChecked(serial, result)).is_err() {
                    return;
                }
            }
        })
        .map_err(|error| diag!("settings-check thread: {error}"))
        .ok()?;
    Some(sender)
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

/// The tree model: a header per non-empty category, then its pages. A
/// row's `page` is its page's slot in `slots` (the open window's build
/// order), -1 for a page the window does not hold yet.
pub(crate) fn rows(pages: &[&Page], current: Option<u64>, slots: &[u64]) -> Value {
    let mut rows = Vec::new();
    for (index, name) in CATEGORIES.iter().enumerate() {
        let label = category_label(name);
        let mut first = true;
        for page in pages.iter().filter(|p| p.category == index) {
            if first {
                rows.push(json!({"label":label,"page":-1,"header":true,"selected":false}));
                first = false;
            }
            let slot = slots
                .iter()
                .position(|serial| *serial == page.serial)
                .map_or(-1, |slot| slot as i64);
            rows.push(json!({"label":page.title,"page":slot,"header":false,
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
        r#"import {{ ListView, ScrollView }} from "std-widgets.slint";
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
                // A virtualized list (only the rows in view exist), 4px
                // below the top as the tree always was.
                tree := ListView {{
                    width: parent.width;
                    y: 4px;
                    height: parent.height - 4px;
                    for row in root.rows: Rectangle {{
                        height: 20px;
                        background: row.selected ? Theme.selected-bg : touch.has-hover && !row.header ? Theme.hover-bg : transparent;
                        // Always present, shown by `visible`: selecting a
                        // page creates or destroys no element in a row.
                        Rectangle {{ visible: row.selected; x: 8px; y: 4px; width: 2px; height: 12px; border-radius: 1px; background: Theme.accent; }}
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

/// A window component of `definition`, set up by `setup` before it shows:
/// with an `old` window open, its tree replaces the old one's inside the
/// same native window (`view::replace_shell`; size, position, maximized
/// state and z-order stay, nothing hides or flashes), and if that fails the
/// old tree shows again. Without one, a new window the caller shows.
fn create_window(
    definition: &slint_interpreter::ComponentDefinition,
    old: Option<&slint_interpreter::ComponentInstance>,
    setup: impl FnOnce(&slint_interpreter::ComponentInstance) -> Result<()>,
) -> Result<slint_interpreter::ComponentInstance> {
    crate::ui::view::replace_shell(old, |window| {
        let instance = match window {
            Some(window) => definition.create_with_existing_window(window)?,
            None => definition.create()?,
        };
        setup(&instance)?;
        Ok(instance)
    })
}

/// A page's bridge callbacks: each goes to the plugin that owns the page
/// when it is called (`owner`, which a wake moves to the new generation).
fn wire_page(
    definition: &slint_interpreter::ComponentDefinition,
    instance: &slint_interpreter::ComponentInstance,
    alias: &str,
    owner: Rc<Cell<u64>>,
    view: String,
    events: mpsc::Sender<Event>,
) -> Result<()> {
    let callbacks = definition
        .global_callbacks(alias)
        .with_context(|| format!("settings global unavailable: {alias}"))?;
    for callback in callbacks {
        let (owner, view, events, name) = (
            owner.clone(),
            view.clone(),
            events.clone(),
            callback.clone(),
        );
        instance.set_global_callback(alias, &callback, move |args| {
            let owner = owner.get();
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
    Ok(())
}

/// The properties `after` sets to other values than `before` had: what a
/// page registered again writes into an open window. One it no longer sets
/// keeps the value the window shows.
fn changed_values<'a>(before: &Page, after: &'a Page) -> Vec<(&'a str, &'a Value)> {
    after
        .properties
        .iter()
        .filter(|(name, value)| before.properties.get(*name) != Some(*value))
        .map(|(name, value)| (name.as_str(), value))
        .collect()
}

/// Whether the tree shows `after` elsewhere or under another title.
fn moved(before: &Page, after: &Page) -> bool {
    before.title != after.title || before.category != after.category || before.order != after.order
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
        self.checking.retain(|(reply, ..)| reply.owner != owner);
        let before = self.pages.len();
        self.pages.retain(|p| p.owner != owner);
        let changed = self.pages.len() != before;
        self.forget_gone();
        self.stale |= changed && self.open.is_some();
        changed
    }
    /// Drops the recorded marks and sent registrations of removed pages.
    fn forget_gone(&mut self) {
        let serials = self
            .pages
            .iter()
            .chain(self.checking.iter().map(|(_, page, _)| page))
            .map(|p| p.serial)
            .collect::<std::collections::BTreeSet<_>>();
        self.recorded.retain(|serial| serials.contains(serial));
        self.sent.retain(|serial, _| serials.contains(serial));
    }
    fn is_recorded(&self, page: &Page) -> bool {
        self.recorded.contains(&page.serial)
    }
    /// Park: `owner`'s pages stay, marked recorded.
    pub fn mark_recorded(&mut self, owner: u64) {
        let serials = self
            .pages
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| p.serial)
            .collect::<Vec<_>>();
        self.recorded.extend(serials);
    }
    /// At `runtime.ready` of a woken plugin: its recorded pages it did not
    /// register again go.
    pub fn remove_recorded(&mut self, owner: u64) -> bool {
        let before = self.pages.len();
        let recorded = std::mem::take(&mut self.recorded);
        self.pages
            .retain(|p| !(p.owner == owner && recorded.contains(&p.serial)));
        self.recorded = recorded;
        self.forget_gone();
        let changed = self.pages.len() != before;
        self.stale |= changed && self.open.is_some();
        changed
    }
    /// A woken plugin's registration of a recorded page with the same files
    /// (the recording is of this package, so the same source): the page
    /// takes it under its serial, and the window, open or not, is left to
    /// be updated in place; the page as it was. `None`: not such a
    /// re-registration (nothing changed).
    fn reregister(&mut self, page: &Page, sent: Value) -> Option<Page> {
        let slot = self.pages.iter_mut().find(|p| {
            p.owner == page.owner
                && p.id == page.id
                && self.recorded.contains(&p.serial)
                && p.file == page.file
                && p.component == page.component
                && p.global == page.global
        })?;
        let serial = slot.serial;
        let before = std::mem::replace(
            slot,
            Page {
                serial,
                ..page.clone()
            },
        );
        self.recorded.remove(&serial);
        self.sent.insert(serial, sent);
        Some(before)
    }
    /// Wake: `from`'s pages move to generation `to`, in an open window too
    /// (its callbacks read their page's owner when called): nothing is
    /// built again.
    pub fn rekey(&mut self, from: u64, to: u64) {
        for page in self.pages.iter_mut().filter(|p| p.owner == from) {
            page.owner = to;
            if let Some(owner) = self.open.as_ref().and_then(|o| o.owners.get(&page.serial)) {
                owner.set(to);
            }
        }
    }
    /// `owner`'s pages as `settings.register` calls, each with its current
    /// properties; `None` when one has no sent form (cannot be recorded).
    pub fn recording(&self, owner: u64) -> Option<Vec<Value>> {
        self.pages
            .iter()
            .filter(|p| p.owner == owner)
            .map(|page| {
                let mut call = self.sent.get(&page.serial)?.clone();
                call["properties"] = json!(page.properties);
                Some(call)
            })
            .collect()
    }
    /// `owner`'s pages in registration order, each with whether it is
    /// recorded (`VESTRA_TEST_REGISTRY`, host/supervisor/probe.rs).
    pub fn pages_of(&self, owner: u64) -> Vec<(&Page, bool)> {
        self.pages
            .iter()
            .filter(|p| p.owner == owner)
            .map(|page| (page, self.is_recorded(page)))
            .collect()
    }
    /// Whether `owner` has a page compiling or showing in the open window
    /// (it waits for, or answers, its plugin).
    pub fn busy(&self, owner: u64) -> bool {
        self.checking.iter().any(|(reply, ..)| reply.owner == owner)
            || self.open.is_some() && self.pages.iter().any(|p| p.owner == owner)
    }
}

/// A `settings.register` as recorded: what the plugin sent, without the
/// initial properties (the recording carries the page's current ones).
fn sent_page(params: &Value) -> Value {
    let mut sent = params.clone();
    if let Some(object) = sent.as_object_mut() {
        object.remove("properties");
    }
    sent
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
    pub(crate) fn settings_dispatch(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let owner = reply.owner;
        match method {
            "settings.register" => {
                let origin = self
                    .plugins
                    .get(&owner)
                    .map(process::Plugin::origin)
                    .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
                let (page, refused) = self.settings_page(owner, &origin, params)?;
                // Row patches apply on the initial properties again (seq 0).
                self.images.reset_writes(owner, &page.view());
                // A woken plugin registers its recorded page again: the same
                // files need no compile (the recording is of this package).
                if let Some(before) = self.settings.reregister(&page, sent_page(params)) {
                    // The same source: an open window keeps its component
                    // and its native window; only what changed is written.
                    self.settings_update(&before);
                    self.report_refused(owner, refused);
                    return Ok(Some(Value::Null));
                }
                // Compile the page alone, so a broken page fails its own
                // registration instead of the shared window: on the
                // `settings-check` thread (`spawn_checker`); the reply waits.
                let checked = source(&[&page])?;
                self.settings.sent.insert(page.serial, sent_page(params));
                if self.settings.checker.is_none() {
                    self.settings.checker = spawn_checker(self.events.clone());
                }
                let queued =
                    self.settings.checker.as_ref().is_some_and(|checker| {
                        checker.send((page.serial, checked.clone())).is_ok()
                    });
                if !queued {
                    // No thread: compiled here, as before.
                    self.settings.checker = None;
                    if let Err(error) = composition::build(checked, "OptionsWindow") {
                        self.settings.forget_gone();
                        return Err(RpcError::new("ui-prepare", format!("{error:#}")));
                    }
                    self.settings_add(page, refused);
                    return Ok(Some(Value::Null));
                }
                self.settings.checking.push((reply.clone(), page, refused));
                Ok(None)
            }
            "settings.unregister" => {
                let id = params_page(params)?.to_owned();
                // Still compiling: the registration ends with an error and the
                // page is never added.
                if let Some(index) = self
                    .settings
                    .checking
                    .iter()
                    .position(|(_, page, _)| page.owner == owner && page.id == id)
                {
                    let (pending, ..) = self.settings.checking.remove(index);
                    self.respond(
                        pending,
                        Err(RpcError::new(
                            "stale",
                            format!("page {id} was unregistered before it was ready"),
                        )),
                    );
                }
                let before = self.settings.pages.len();
                self.settings
                    .pages
                    .retain(|p| !(p.owner == owner && p.id == id));
                self.settings.forget_gone();
                if self.settings.pages.len() != before {
                    self.images.release_view(owner, &format!("$settings:{id}"));
                    self.settings.stale |= self.settings.open.is_some();
                }
                Ok(Some(Value::Null))
            }
            "settings.set" => {
                // A large array converted on the reader thread (P3): its
                // whole value was taken out of `params`.
                let mut prepared = self.take_prepared(reply);
                let id = params_page(params)?.to_owned();
                let property = string(params, "property")?.to_owned();
                let page = self
                    .settings
                    .page(owner, &id)
                    .ok_or_else(|| RpcError::new("missing", "page not registered"))?;
                let (view, serial) = (page.view(), page.serial);
                let window = self.settings.window().map(ComponentHandle::clone_strong);
                // A part of a whole value sent in parts: held until the last.
                let parted = self.parts(owner, &view, &property, params, prepared.as_deref())?;
                let done = match parted {
                    ui_patch::Parted::Held => return Ok(Some(Value::Null)),
                    ui_patch::Parted::Done(value, rows, seq) => Some((value, Some(rows), seq)),
                    ui_patch::Parted::Not => None,
                };
                let (value, rows, seq) = match (done, params.get("patch")) {
                    (Some(done), _) => done,
                    (None, Some(patch)) => {
                        let (base, seq, ops) = ui_patch::request(patch)?;
                        if self.images.seq(owner, &view, &property) != base {
                            return Err(ui_patch::stale());
                        }
                        match self.patch_page(
                            owner,
                            &id,
                            serial,
                            window.as_ref(),
                            &property,
                            ops,
                            prepared.take(),
                        )? {
                            None => {
                                self.images.set_seq(owner, &view, &property, seq);
                                return Ok(Some(Value::Null));
                            }
                            Some(patched) => (patched, None, seq),
                        }
                    }
                    (None, None) => {
                        let seq = params["seq"].as_u64().unwrap_or(0);
                        match prepared.map(|prepared| *prepared) {
                            Some(prepared) => {
                                (prepared.whole.unwrap_or_default(), Some(prepared.rows), seq)
                            }
                            None => (params["value"].clone(), None, seq),
                        }
                    }
                };
                if let Some(window) = window {
                    let value =
                        self.prop_value_with(owner, &view, &property, &value, rows.as_ref())?;
                    let alias = alias(serial);
                    let current = window
                        .get_global_property(&alias, &property)
                        .map_err(|e| RpcError::new("ui", e.to_string()))?;
                    if !ui_values::reconcile_in(window.window(), &current, &value)? {
                        ui_values::set(&window, Some(&alias), &property, value)
                            .map_err(|e| RpcError::new("ui", e.to_string()))?;
                    }
                } else if ui_image::reference(&value).is_some() {
                    // Checked now, shown when the window opens.
                    self.prop_value(owner, &view, &property, &value)?;
                } else if rows.is_none() {
                    // Prepared rows were checked on the reader thread.
                    RpcError::check_property(&property, &value)?;
                }
                if let Some(page) = self.settings.page_mut(owner, &id) {
                    page.properties.insert(property.clone(), value);
                }
                self.images.set_seq(owner, &view, &property, seq);
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
    /// A row patch of a settings page's property (`ui_patch`): in place in
    /// the open window's model and in the stored properties (`None`), or,
    /// where the window's model cannot take it, the patched whole value
    /// for the caller to write. A closed window's page checks the patched
    /// whole value against the value limits, as a closed page's whole write
    /// does.
    #[allow(clippy::too_many_arguments)]
    fn patch_page(
        &mut self,
        owner: u64,
        id: &str,
        serial: u64,
        window: Option<&slint_interpreter::ComponentInstance>,
        property: &str,
        ops: &Value,
        prepared: Option<Box<ui_values::Prepared>>,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let page = self
            .settings
            .page(owner, id)
            .ok_or_else(|| RpcError::new("missing", "page not registered"))?;
        let view = page.view();
        let Some(Value::Array(stored)) = page.properties.get(property) else {
            return Err(ui_patch::stale());
        };
        let ops = ui_patch::parse(ops, stored.len())?;
        if let Some(window) = window {
            let current = window
                .get_global_property(&alias(serial), property)
                .map_err(|e| RpcError::new("ui", e.to_string()))?;
            let rows = self
                .patch_rows(
                    owner,
                    &view,
                    property,
                    &ops,
                    prepared.as_ref().map(|prepared| &prepared.rows),
                )?
                .filter(|rows| ui_patch::applicable(&current, ops.before, &ops.shapes, rows));
            let Some(rows) = rows else {
                return match self
                    .settings
                    .page(owner, id)
                    .and_then(|page| page.properties.get(property))
                {
                    Some(Value::Array(stored)) => Ok(Some(ui_patch::patched(stored, &ops))),
                    _ => Err(ui_patch::stale()),
                };
            };
            ui_patch::apply(&current, &ops.shapes, rows.values, &rows.images);
            self.images
                .patched_rows(owner, &view, property, rows.written);
        } else {
            // The rows it keeps count toward the totals too.
            let Some(Value::Array(stored)) = self
                .settings
                .page(owner, id)
                .and_then(|page| page.properties.get(property))
            else {
                return Err(ui_patch::stale());
            };
            let whole = ui_patch::patched(stored, &ops);
            RpcError::check_property(property, &whole)?;
            if let Some(page) = self.settings.page_mut(owner, id) {
                page.properties.insert(property.to_owned(), whole);
            }
            return Ok(None);
        }
        // The stored rows: copied on the reader thread when large.
        let rows: Vec<Value> = match prepared {
            Some(prepared) if prepared.carried.len() == ops.rows.len() => prepared.carried,
            _ => ops.rows.iter().map(|row| (*row).clone()).collect(),
        };
        if let Some(Value::Array(stored)) = self
            .settings
            .page_mut(owner, id)
            .and_then(|page| page.properties.get_mut(property))
        {
            ui_patch::apply_vec(stored, &ops.shapes, rows);
        }
        Ok(None)
    }
    /// A `settings.register` of `owner`'s (from `origin`, its package),
    /// checked, with a new serial: the page and its refused initial
    /// properties. A recorded page of the same id does not count against it.
    fn settings_page(
        &mut self,
        owner: u64,
        origin: &crate::host::Origin,
        params: &Value,
    ) -> std::result::Result<(Page, Vec<RpcError>), RpcError> {
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
        let settings = &self.settings;
        let live = || {
            settings
                .pages
                .iter()
                .filter(|p| p.owner == owner && !settings.is_recorded(p))
                .chain(
                    settings
                        .checking
                        .iter()
                        .map(|(_, page, _)| page)
                        .filter(|p| p.owner == owner),
                )
        };
        if live().any(|p| p.id == id) {
            return Err(RpcError::invalid(format!("page {id} already exists")));
        }
        if live().count() >= MAX_PAGES {
            return Err(RpcError::invalid("a plugin has at most 8 settings pages"));
        }
        let file = process::package_file(&origin.directory, string(params, "file")?)?;
        let mut properties: BTreeMap<String, Value> =
            serde_json::from_value(params.get("properties").cloned().unwrap_or(json!({})))
                .map_err(|e| RpcError::invalid(e.to_string()))?;
        let refused = crate::host::refuse_initial(&mut properties);
        self.settings.next_serial += 1;
        let page = Page {
            owner,
            manifest: origin.id.clone(),
            id,
            category,
            title: origin.text(title),
            order,
            serial: self.settings.next_serial,
            file,
            component: string(params, "component")?.into(),
            global: string(params, "global")?.into(),
            properties,
        };
        Ok((page, refused))
    }
    /// Replays a recorded `settings.register` for a parked `owner`: the page
    /// is added as recorded, without a compile (it compiled when recorded,
    /// and the recording is of this package).
    pub(crate) fn settings_replay(
        &mut self,
        owner: u64,
        origin: &crate::host::Origin,
        params: &Value,
    ) -> std::result::Result<(), RpcError> {
        let (page, _) = self.settings_page(owner, origin, params)?;
        if self.settings.page(owner, &page.id).is_some() {
            return Err(RpcError::invalid(format!(
                "page {} already exists",
                page.id
            )));
        }
        self.settings.sent.insert(page.serial, sent_page(params));
        self.settings.recorded.insert(page.serial);
        self.settings.pages.push(page);
        self.settings.stale |= self.settings.open.is_some();
        Ok(())
    }
    /// Open (or rebuild) the window, select the current page, bring it front.
    fn settings_open(&mut self) -> Result<()> {
        if self.settings.open.is_none() || self.settings.stale {
            self.settings_build()?;
        }
        self.settings_select(None)?;
        if let Some(open) = &self.settings.open {
            open.instance.show()?;
            crate::platform::shown(&open.instance);
            crate::ui::view::show_again(&open.instance, crate::platform::app_in_foreground());
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
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        // An open window keeps its native window: the new component tree
        // takes the old one's place in it (`view::replace_shell`), so a
        // page added or removed while it is open neither closes it nor
        // moves, resizes or flashes it.
        let old = self.settings.open.take();
        let mut owners = BTreeMap::new();
        let built = create_window(&definition, old.as_ref().map(|o| &o.instance), |instance| {
            owners = self.settings_wire(&definition, instance, &pages, generation)?;
            Ok(())
        });
        let instance = match built {
            Ok(instance) => instance,
            Err(error) => {
                // The old window shows its old tree again (`replace_shell`).
                self.settings.open = old;
                return Err(error);
            }
        };
        // Not hidden: it shared the native window with the new tree.
        drop(old);
        self.settings.open = Some(Open {
            instance,
            generation,
            order,
            owners,
        });
        self.settings.stale = false;
        Ok(())
    }
    /// A new window component's values, callbacks, window handlers and
    /// theme; each page's owner as its callbacks read it.
    fn settings_wire(
        &mut self,
        definition: &slint_interpreter::ComponentDefinition,
        instance: &slint_interpreter::ComponentInstance,
        pages: &[Page],
        generation: u64,
    ) -> Result<BTreeMap<u64, Rc<Cell<u64>>>> {
        let mut owners = BTreeMap::new();
        let mut reports = Vec::new();
        for page in pages {
            let alias = alias(page.serial);
            crate::ui::ui_values::own_default_models(instance, Some(&alias))?;
            for (name, value) in &page.properties {
                match self.prop_value(page.owner, &page.view(), name, value) {
                    Ok(value) => composition::set_alias_value(instance, &alias, name, value)?,
                    Err(error) => {
                        // Checked when written (`check_property`): an image
                        // removed since. Logged and reported to the plugin.
                        diag!("settings property {}/{name}: {}", page.key(), error.message);
                        reports.push((page.owner, crate::host::ui_error(&error)));
                    }
                }
            }
            let owner = Rc::new(Cell::new(page.owner));
            owners.insert(page.serial, owner.clone());
            wire_page(
                definition,
                instance,
                &alias,
                owner,
                page.view(),
                self.events.clone(),
            )?;
        }
        for (owner, report) in reports {
            self.send(owner, report);
        }
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
        // The native window's handlers: set again for each new tree (they
        // name its generation).
        let events = self.events.clone();
        instance.window().on_close_requested(move || {
            let _ = events.send(Event::Settings(generation, None));
            slint::CloseRequestResponse::KeepWindowShown
        });
        // ⌘W / Ctrl+W (the platform's standard keys) close it like 关闭.
        let shortcuts = crate::ui::view::WindowShortcuts::default();
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
        self.theme_view(instance);
        Ok(owners)
    }
    /// A woken plugin registered `before`'s page again with the same files
    /// (the page now holds the new registration): an open window takes the
    /// changed values in place, and its tree the new title, category or
    /// order. Nothing is compiled; the component and its native window
    /// stay.
    fn settings_update(&mut self, before: &Page) {
        let Some(instance) = self
            .settings
            .window()
            .map(slint::ComponentHandle::clone_strong)
        else {
            return;
        };
        let Some(page) = self
            .settings
            .pages
            .iter()
            .find(|p| p.serial == before.serial)
            .cloned()
        else {
            return;
        };
        let alias = alias(page.serial);
        for (name, value) in changed_values(before, &page) {
            let written = self
                .prop_value(page.owner, &page.view(), name, value)
                .map_err(|error| {
                    self.send(page.owner, crate::host::ui_error(&error));
                    anyhow::anyhow!(error.message)
                })
                .and_then(|value| composition::set_alias_value(&instance, &alias, name, value));
            if let Err(error) = written {
                diag!("settings property {}/{name}: {error:#}", page.key());
            }
        }
        if moved(before, &page)
            && let Err(error) = self.settings_rows()
        {
            diag!("host.error settings window: {error:#}");
        }
    }
    /// The open window's tree, with the current page selected.
    fn settings_rows(&mut self) -> Result<()> {
        let Some(open) = &self.settings.open else {
            return Ok(());
        };
        let pages = sorted(&self.settings.pages);
        let current = self
            .settings
            .current
            .as_deref()
            .and_then(|key| pages.iter().find(|p| p.key() == key))
            .map(|p| p.serial);
        let tree = rows(&pages, current, &open.order);
        open.instance
            .set_property("rows", ui_values::from_json(&tree)?)?;
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
        let shown = chosen.map(|page| page.owner);
        let (key, chosen_serial, category) = match chosen {
            Some(page) => (Some(page.key()), Some(page.serial), Some(page.category)),
            None => (None, None, None),
        };
        self.settings.current = key.clone();
        self.settings_category = category;
        if let Some(open) = &self.settings.open {
            let tree = rows(&pages, chosen_serial, &open.order);
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
        // Lazy activation: a parked plugin's page shown starts it (its
        // values and callbacks are its own).
        if let Some(owner) = shown
            && self.settings.open.is_some()
        {
            self.page_shown(owner);
        }
        Ok(())
    }
    /// The window's tree and close button.
    pub(crate) fn settings_event(&mut self, generation: u64, index: Option<usize>) {
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
    fn settings_add(&mut self, page: Page, refused: Vec<RpcError>) {
        let owner = page.owner;
        // A recorded page of the same id (a woken plugin's, with other
        // files) is replaced.
        let recorded = &self.settings.recorded;
        match self
            .settings
            .pages
            .iter()
            .position(|p| p.owner == owner && p.id == page.id && recorded.contains(&p.serial))
        {
            Some(index) => self.settings.pages[index] = page,
            None => self.settings.pages.push(page),
        }
        self.settings.forget_gone();
        self.settings.stale |= self.settings.open.is_some();
        self.report_refused(owner, refused);
    }
    /// A page's compile on the `settings-check` thread finished: the
    /// registration is answered now (dropped if its plugin stopped since).
    pub(crate) fn settings_checked(
        &mut self,
        serial: u64,
        result: std::result::Result<(), String>,
    ) {
        let Some(index) = self
            .settings
            .checking
            .iter()
            .position(|(_, page, _)| page.serial == serial)
        else {
            return;
        };
        let (reply, page, refused) = self.settings.checking.remove(index);
        if !self.plugins.contains_key(&reply.owner) {
            self.settings.forget_gone();
            return;
        }
        match result {
            Ok(()) => {
                self.settings_add(page, refused);
                self.respond(reply, Ok(Value::Null));
            }
            Err(message) => {
                self.settings.forget_gone();
                self.respond(reply, Err(RpcError::new("ui-prepare", message)))
            }
        }
    }
    /// Pages added or removed while the window is open: rebuild it.
    pub(crate) fn settings_tick(&mut self) {
        if self.settings.stale
            && self.settings.open.is_some()
            && let Err(error) = self
                .settings_build()
                .and_then(|()| self.settings_select(None))
        {
            diag!("host.error settings window: {error:#}");
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
        let slots: Vec<u64> = sorted.iter().map(|p| p.serial).collect();
        let tree = rows(&sorted, Some(2), &slots);
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

    /// Lazy activation: a parked owner's pages stay recorded with their
    /// current properties, move to the woken generation, and the ones not
    /// registered again go at ready.
    #[test]
    fn recorded_pages_record_rekey_and_prune() {
        let mut settings = Settings::default();
        for (serial, id) in [(1, "keys"), (2, "mouse")] {
            let mut page = page("x.hotkeys", id, 5, 0, serial);
            page.owner = 4;
            page.properties.insert("value".into(), json!(serial));
            settings.pages.push(page);
            settings.sent.insert(
                serial,
                sent_page(&json!({"id":id,"title":id,"properties":{"value":0}})),
            );
        }
        settings.pages.push(page("y.other", "other", 0, 0, 3));
        let calls = settings.recording(4).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            json!({"id":"keys","title":"keys","properties":{"value":1}})
        );
        // A page without its sent form cannot be recorded.
        assert!(settings.recording(3).is_none());
        settings.mark_recorded(4);
        assert!(settings.is_recorded(&settings.pages[0]));
        assert!(!settings.is_recorded(&settings.pages[2]));
        settings.rekey(4, 9);
        assert!(settings.page(4, "keys").is_none());
        assert!(settings.page(9, "keys").is_some());
        // The live plugin registers "keys" again (its mark goes); "mouse" goes.
        settings.recorded.remove(&1);
        assert!(settings.remove_recorded(9));
        assert!(settings.page(9, "keys").is_some());
        assert!(settings.page(9, "mouse").is_none());
        assert!(!settings.sent.contains_key(&2));
        assert!(!settings.remove_recorded(9));
        assert!(settings.remove_owner(9));
        assert!(settings.sent.is_empty() && settings.recorded.is_empty());
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
                .global_properties(crate::ui::theme::PALETTE)
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
    /// (a fix: the footer used to take half the window and stretch the
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
                // The tree's list starts 4px down (its top padding).
                assert_eq!(tree, height - footer - 4.0, "{label}: tree height");
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
        let slots: Vec<u64> = pages.iter().map(|p| p.serial).collect();
        instance
            .set_property(
                "rows",
                ui_values::from_json(&rows(pages, None, &slots)).unwrap(),
            )
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
            diag!("shown with every global changed: {}", shown.component);
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

    /// Opens `settings`' window as `settings_build` does (the values as
    /// they are, each page's callbacks reading its owner cell), in place of
    /// `old` when given.
    fn open_window(settings: &mut Settings, events: &mpsc::Sender<Event>, old: Option<Open>) {
        let order: Vec<u64> = sorted(&settings.pages).iter().map(|p| p.serial).collect();
        let pages: Vec<Page> = order
            .iter()
            .map(|serial| {
                settings
                    .pages
                    .iter()
                    .find(|p| p.serial == *serial)
                    .unwrap()
                    .clone()
            })
            .collect();
        let definition = composition::build(
            source(&pages.iter().collect::<Vec<_>>()).unwrap(),
            "OptionsWindow",
        )
        .unwrap();
        let mut owners = BTreeMap::new();
        let instance = create_window(&definition, old.as_ref().map(|o| &o.instance), |instance| {
            for page in &pages {
                let alias = alias(page.serial);
                for (name, value) in &page.properties {
                    let value = ui_values::from_json(value)?;
                    composition::set_alias_value(instance, &alias, name, value)?;
                }
                let owner = Rc::new(Cell::new(page.owner));
                owners.insert(page.serial, owner.clone());
                wire_page(
                    &definition,
                    instance,
                    &alias,
                    owner,
                    page.view(),
                    events.clone(),
                )?;
            }
            Ok(())
        })
        .unwrap();
        if old.is_none() {
            instance
                .window()
                .set_size(slint::PhysicalSize::new(640, 480));
            instance.show().unwrap();
        }
        drop(old);
        settings.open = Some(Open {
            instance,
            generation: 1,
            order,
            owners,
        });
        settings.stale = false;
    }

    fn caption(settings: &Settings, serial: u64) -> SValue {
        settings
            .window()
            .unwrap()
            .get_global_property(&alias(serial), "caption")
            .unwrap()
    }

    /// A parked plugin woken while the Options window is open registers its
    /// page again with the same files: nothing is compiled or rebuilt, the
    /// window and its component stay, the changed value is written in
    /// place, the tree takes the new title and order, and the page's
    /// callbacks go to the new generation.
    #[test]
    fn a_woken_page_with_the_same_files_changes_in_place_without_a_new_window() {
        let native_calls = crate::ui::view::recording_platform();
        let (events, received) = mpsc::channel();
        let mut settings = Settings::default();
        let mut keys = page("x.hotkeys", "keys", 5, 0, 1);
        keys.owner = 4;
        keys.properties.insert("caption".into(), json!("before"));
        settings.pages = vec![keys.clone(), page("y.other", "other", 5, 10, 2)];
        open_window(&mut settings, &events, None);
        assert_eq!(
            native_calls().iter().filter(|c| *c == "new window").count(),
            1
        );
        let instance = settings.window().unwrap().clone_strong();
        let tree = crate::ui::view::ShownTree::of(instance.window()).unwrap();
        assert_eq!(
            caption(&settings, 1),
            SValue::from(slint::SharedString::from("before"))
        );

        // Park, wake (the new generation is 9), register again.
        settings.mark_recorded(4);
        settings.rekey(4, 9);
        assert!(!settings.stale, "a wake does not rebuild the window");
        let mut again = Page {
            owner: 9,
            serial: 77,
            title: "Keys!".into(),
            order: 20,
            ..keys.clone()
        };
        again.properties.insert("caption".into(), json!("after"));
        let before = settings.reregister(&again, json!({"id":"keys"})).unwrap();
        assert!(!settings.stale, "the same source is not compiled again");
        assert!(settings.recorded.is_empty());
        let page = settings.page(9, "keys").unwrap().clone();
        assert_eq!(page.serial, 1, "the page keeps its serial (its alias)");

        // What `settings_update` writes: only the changed value.
        let changed = changed_values(&before, &page);
        assert_eq!(changed, [("caption", &json!("after"))]);
        let window = settings.window().unwrap();
        for (name, value) in changed {
            composition::set_alias_value(
                window,
                &alias(1),
                name,
                ui_values::from_json(value).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            caption(&settings, 1),
            SValue::from(slint::SharedString::from("after"))
        );
        // The tree: "keys" now after "other", still pointing at its slot.
        assert!(moved(&before, &page));
        let open = settings.open.as_ref().unwrap();
        let tree_rows = rows(&sorted(&settings.pages), Some(1), &open.order);
        let labels: Vec<_> = tree_rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["label"].as_str().unwrap().to_owned(),
                    r["page"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            labels,
            [
                ("Keys and Mouse".to_owned(), -1),
                ("other".to_owned(), 1),
                ("Keys!".to_owned(), 0)
            ]
        );
        assert_eq!(open.owners[&1].get(), 9);

        // The same component in the same native window: nothing was asked
        // of the windowing system.
        assert!(native_calls().is_empty());
        assert!(tree.is_shown_in(instance.window()));
        assert!(settings.window().unwrap().as_weak().upgrade().is_some());
        drop(received);
    }

    /// The page's bridge callbacks name the owner it has when called: a
    /// wake moves them to the new generation without a new component.
    #[test]
    fn page_callbacks_follow_the_owner_through_a_wake() {
        let _ = crate::ui::view::recording_platform();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let (events, received) = mpsc::channel();
        let mut settings = Settings::default();
        let mut page = Page {
            file: base.join("window/ui/page.slint"),
            component: "WindowSettingsPage".into(),
            global: "WindowPage".into(),
            ..page("built.in", "window", 1, 0, 3)
        };
        page.owner = 4;
        settings.pages = vec![page];
        open_window(&mut settings, &events, None);
        let instance = settings.window().unwrap().clone_strong();
        let fire = || {
            instance
                .invoke_global(&alias(3), "set-auto", &[SValue::Number(1.)])
                .unwrap();
        };
        fire();
        settings.mark_recorded(4);
        settings.rekey(4, 9);
        fire();
        let owners: Vec<u64> = received
            .try_iter()
            .filter_map(|event| match event {
                Event::UiCallback(owner, message) => {
                    assert_eq!(message["callback"], "set-auto");
                    Some(owner)
                }
                _ => None,
            })
            .collect();
        assert_eq!(owners, [4, 9]);
    }

    /// A page added, removed or with other files needs a new compile; an
    /// open window then takes the new component tree in its own native
    /// window (no new window, nothing hidden, moved or resized).
    #[test]
    fn a_changed_page_source_compiles_again_in_the_same_native_window() {
        let native_calls = crate::ui::view::recording_platform();
        let (events, _received) = mpsc::channel();
        let mut settings = Settings::default();
        let mut keys = page("x.hotkeys", "keys", 5, 0, 1);
        keys.owner = 4;
        settings.pages = vec![keys.clone()];
        open_window(&mut settings, &events, None);
        let instance = settings.window().unwrap().clone_strong();
        instance
            .window()
            .set_position(slint::PhysicalPosition::new(120, 64));
        let old_tree = crate::ui::view::ShownTree::of(instance.window()).unwrap();
        let before = source(&[&keys]).unwrap();
        native_calls();

        // Woken with another file: not a re-registration of the same source.
        settings.mark_recorded(4);
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings");
        let other = Page {
            file: base.join("tall.slint"),
            component: "Tall".into(),
            serial: 2,
            ..keys.clone()
        };
        assert!(settings.reregister(&other, json!({})).is_none());
        assert_ne!(source(&[&other]).unwrap(), before);
        settings.pages = vec![other];
        let old = settings.open.take();
        open_window(&mut settings, &events, old);
        let calls = native_calls();
        assert!(!calls.iter().any(|c| c == "new window"), "{calls:?}");
        assert!(!calls.iter().any(|c| c == "set_visible false"), "{calls:?}");
        let window = settings.window().unwrap().window();
        assert!(!old_tree.is_shown_in(window));
        assert!(window.is_visible());
        assert_eq!(window.size(), slint::PhysicalSize::new(640, 480));
        assert_eq!(window.position(), slint::PhysicalPosition::new(120, 64));
        drop(instance);
    }
}
