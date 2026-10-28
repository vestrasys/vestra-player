//! Offscreen screenshots of plugin windows and settings pages
//! (doc/architecture/ui-shots.md; `npm run ui:shots`).
//!
//! Plugin authors cannot see a GUI. This renders each view a plugin's tests
//! produced, with the player's own pipeline and no window on screen:
//!
//! - compiled by `ui::view::compile` (tool windows) or `settings_window::source` +
//!   `composition::build` (the Options window holding a settings page): the
//!   `@vestra/*` library, the palette, the fluent-dark style, and the `@tr`
//!   rewrite with the plugin's catalog in the chosen language (`i18n`);
//! - property values as the host converts them: text values with the
//!   plugin's catalog, Markdown documents, runtime images, then
//!   `ui_values::set` (a value Slint refuses is reported, as in the player);
//! - the player's fonts: `ui_fonts::install` (the CJK fallback chains) and
//!   `theme::apply_fonts`;
//! - drawn by Slint's software renderer into memory at the view's declared
//!   (preferred) size, like winit sizes a new window.
//!
//! `scripts/ui-shots.mjs` builds the job (the views and their states from the
//! plugin tests' `VESTRA_UI_STATES` log) and runs this ignored test once per
//! language, since the interface language is fixed per process (`i18n::init`).
#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use slint::ComponentHandle;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint_interpreter::{ComponentInstance, Value};

/// Visible height of the Options window's page area at its preferred size
/// (480 minus the footer: a line and the Close row). A page taller than this
/// scrolls; the tool then also renders the whole page.
const OPTIONS_PAGE_VISIBLE: f32 = 439.;
/// The width the Options window gives a page (`settings_window::source`).
const PAGE_WIDTH: f32 = 440.;

/// What `scripts/ui-shots.mjs` asks for (one language).
#[derive(Deserialize)]
struct Job {
    locale: String,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default = "one")]
    scale: f32,
    out: PathBuf,
    shots: Vec<Shot>,
}
fn one() -> f32 {
    1.
}

/// One view in one state.
#[derive(Deserialize)]
struct Shot {
    /// File name stem, unique in the job.
    id: String,
    plugin: PathBuf,
    /// `window` or `page`.
    kind: String,
    file: String,
    #[serde(default)]
    component: Option<String>,
    #[serde(default)]
    global: Option<String>,
    /// A page's declared title (a string or a text value).
    #[serde(default)]
    title: Option<Json>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    properties: BTreeMap<String, Json>,
}

/// One PNG written (or the reason none was).
#[derive(Serialize)]
struct Output {
    id: String,
    /// `window`, `options` (a page in the Options window) or `page` (a whole
    /// page taller than the Options window shows).
    variant: &'static str,
    png: Option<PathBuf>,
    width: u32,
    height: u32,
    /// Values the player would refuse, images it could not show.
    problems: Vec<String>,
}

/// The windows the platform hands out, so the tool can draw them.
type Created = Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>;

struct Offscreen(Created);
impl slint::platform::Platform for Offscreen {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        *self.0.borrow_mut() = Some(window.clone());
        Ok(window)
    }
}

/// Everything about one language's run.
struct Renderer {
    locale: &'static str,
    theme: String,
    scale: f32,
    created: Created,
    /// Plugin directory → its catalog in this language (registered for `@tr`).
    catalogs: BTreeMap<PathBuf, Option<std::sync::Arc<crate::i18n::Catalog>>>,
    /// Compiled views: the same for every state in this language.
    definitions: BTreeMap<String, slint_interpreter::ComponentDefinition>,
}

impl Renderer {
    fn new(job: &Job) -> Result<Self> {
        let locale = crate::i18n::LOCALES
            .iter()
            .copied()
            .find(|l| *l == job.locale)
            .ok_or_else(|| anyhow!("unknown language {}", job.locale))?;
        crate::i18n::init(locale);
        if crate::i18n::locale() != locale {
            bail!("the interface language is fixed per process: one job per run");
        }
        let created = Created::default();
        slint::platform::set_platform(Box::new(Offscreen(created.clone())))
            .map_err(|e| anyhow!("platform: {e}"))?;
        // As at start-up (main.rs): after the backend, before the first window.
        crate::ui::ui_fonts::install(locale);
        Ok(Self {
            locale,
            theme: crate::ui::theme::known(job.theme.as_deref().unwrap_or("vestra")).to_owned(),
            scale: job.scale,
            created,
            catalogs: BTreeMap::new(),
            definitions: BTreeMap::new(),
        })
    }

    /// The view compiled once (`build`), a fresh instance per state.
    fn create(
        &mut self,
        key: String,
        build: impl FnOnce() -> Result<slint_interpreter::ComponentDefinition>,
    ) -> Result<(ComponentInstance, Rc<MinimalSoftwareWindow>)> {
        let definition = match self.definitions.get(&key) {
            Some(definition) => definition.clone(),
            None => {
                let definition = build()?;
                self.definitions.insert(key, definition.clone());
                definition
            }
        };
        let instance = definition.create()?;
        let window = self
            .created
            .borrow_mut()
            .take()
            .context("the component created no window")?;
        Ok((instance, window))
    }

    /// The plugin's catalog, loaded and registered like `process/mod.rs` does.
    fn catalog(&mut self, plugin: &Path) -> Option<std::sync::Arc<crate::i18n::Catalog>> {
        self.catalogs
            .entry(plugin.to_owned())
            .or_insert_with(|| {
                let catalog = crate::i18n::load(plugin);
                if let Some(catalog) = &catalog {
                    crate::i18n::register(plugin, catalog.clone());
                }
                catalog
            })
            .clone()
    }

    /// A property value as the host converts it (`prop_value` after the
    /// request's text values are resolved).
    fn convert(
        &mut self,
        plugin: &Path,
        property: &str,
        value: &Json,
        problems: &mut Vec<String>,
    ) -> Result<Value> {
        let catalog = self.catalog(plugin);
        let mut value = value.clone();
        crate::i18n::text::Texts {
            catalog: catalog.as_deref(),
            locale: self.locale,
        }
        .resolve(&mut value)
        .map_err(|e| anyhow!("{property}: {e}"))?;
        if crate::ui::ui_image::reference(&value).is_some() {
            let (path, side) = crate::ui::ui_image::parts(&value).unwrap_or_default();
            return Ok(Value::Image(image(path, side).unwrap_or_else(|why| {
                problems.push(format!("{property}: image not shown: {why}"));
                Default::default()
            })));
        }
        let mut cache = crate::ui::ui_markdown::Cache::default();
        let mut missing = Vec::new();
        let mut nested = Vec::new();
        let converted = crate::ui::ui_markdown::convert(
            &mut cache,
            (0, "shot", property),
            &value,
            &mut |path| match image(path, 0) {
                Ok(image) => Some((None, image)),
                Err(why) => {
                    missing.push(why);
                    None
                }
            },
            // Nested images (row logos) as the player shows them.
            &mut |node| {
                let (path, side) = crate::ui::ui_image::parts(node)?;
                Ok((
                    None,
                    image(path, side).unwrap_or_else(|why| {
                        nested.push(why);
                        Default::default()
                    }),
                ))
            },
        )
        .map(|(value, _)| value)
        .map_err(|e| anyhow!("{property}: {}", e.message));
        for why in missing {
            problems.push(format!("{property}: Markdown image not shown: {why}"));
        }
        for why in nested {
            problems.push(format!("{property}: image not shown: {why}"));
        }
        converted
    }

    /// The palette and fonts the player sets before showing a view.
    fn prepare(&self, instance: &ComponentInstance) -> Result<()> {
        crate::ui::theme::apply_fonts(instance)?;
        if self.theme != crate::ui::theme::THEMES[0] {
            crate::ui::theme::apply(instance, &self.theme)?;
        }
        Ok(())
    }

    fn shot(&mut self, shot: &Shot, out: &Path) -> Vec<Output> {
        let result = match shot.kind.as_str() {
            "window" => self.window(shot, out),
            "page" => self.page(shot, out),
            other => Err(anyhow!("unknown kind {other}")),
        };
        result.unwrap_or_else(|error| {
            vec![Output {
                id: shot.id.clone(),
                variant: if shot.kind == "page" {
                    "options"
                } else {
                    "window"
                },
                png: None,
                width: 0,
                height: 0,
                problems: vec![format!("not rendered: {error:#}")],
            }]
        })
    }

    /// A tool window (`ui.open`): `ui::view::open` without the event wiring.
    fn window(&mut self, shot: &Shot, out: &Path) -> Result<Vec<Output>> {
        let path = shot.plugin.join(&shot.file);
        self.catalog(&shot.plugin);
        let (instance, window) = self.create(format!("window {}", path.display()), || {
            crate::ui::view::definition_source(std::fs::read_to_string(&path)?, &path)
        })?;
        crate::ui::ui_values::own_default_models(&instance, None)?;
        let mut problems = Vec::new();
        for (name, value) in &shot.properties {
            let result = self
                .convert(&shot.plugin, name, value, &mut problems)
                .and_then(|value| crate::ui::ui_values::set(&instance, None, name, value));
            if let Err(error) = result {
                problems.push(format!("{error:#}"));
            }
        }
        self.prepare(&instance)?;
        let (png, width, height) = self.draw(&instance, &window, None, out, &shot.id)?;
        Ok(vec![Output {
            id: shot.id.clone(),
            variant: "window",
            png: Some(png),
            width,
            height,
            problems,
        }])
    }

    /// A settings page in the Options window, as `settings_build` makes it
    /// (with only this page in the tree), and the whole page when it is
    /// taller than the window shows.
    fn page(&mut self, shot: &Shot, out: &Path) -> Result<Vec<Output>> {
        let catalog = self.catalog(&shot.plugin);
        let component = shot.component.clone().context("a page needs a component")?;
        let global = shot.global.clone().context("a page needs a global")?;
        let texts = crate::i18n::text::Texts {
            catalog: catalog.as_deref(),
            locale: self.locale,
        };
        let title = match &shot.title {
            Some(Json::String(title)) => texts.declared(&Json::String(title.clone())),
            Some(value) => texts.declared(value),
            None => Ok(String::new()),
        }
        .map_err(|e| anyhow!("title: {e}"))?;
        let category = crate::ui::settings_window::CATEGORIES
            .iter()
            .position(|c| Some(*c) == shot.category.as_deref())
            .unwrap_or(crate::ui::settings_window::CATEGORIES.len() - 1);
        let page = crate::ui::settings_window::Page {
            owner: 1,
            manifest: String::new(),
            id: "shot".into(),
            category,
            title,
            order: 0,
            serial: 1,
            file: shot.plugin.join(&shot.file),
            component: component.clone(),
            global: global.clone(),
            properties: BTreeMap::new(),
        };
        let alias = crate::ui::settings_window::alias(page.serial);
        let key = format!("{} {component} {}", page.file.display(), page.title);
        let mut outputs = Vec::new();

        // The Options window.
        let (instance, window) = self.create(format!("options {key}"), || {
            crate::ui::composition::build(
                crate::ui::settings_window::source(&[&page])?,
                "OptionsWindow",
            )
        })?;
        let mut problems = self.page_values(shot, &instance, &alias);
        let rows = crate::ui::settings_window::rows(&[&page], Some(page.serial));
        instance.set_property("rows", crate::ui::ui_values::from_json(&rows)?)?;
        instance.set_property("current", Value::Number(0.))?;
        self.prepare(&instance)?;
        let (png, width, height) = self.draw(&instance, &window, None, out, &shot.id)?;
        outputs.push(Output {
            id: shot.id.clone(),
            variant: "options",
            png: Some(png),
            width,
            height,
            problems: std::mem::take(&mut problems),
        });

        // The whole page, at the width the Options window gives it.
        let import = crate::ui::composition::import_path(&page.file)?;
        let source = format!(
            "import {{ {component} as ShotPageView }} from \"{import}\";\n\
             import {{ {global} as {alias} }} from \"{import}\";\n\
             export {{ {alias} }}\n\
             import {{ Theme }} from \"@vestra/theme.slint\";\n\
             export {{ VestraPalette }} from \"palette.slint\";\n\
             export component ShotPage inherits Window {{\n\
                 background: Theme.bg-panel;\n\
                 VerticalLayout {{ alignment: start; ShotPageView {{ width: {PAGE_WIDTH}px; }} }}\n\
             }}\n"
        );
        let (instance, window) = self.create(format!("page {key}"), || {
            crate::ui::composition::build(source, "ShotPage")
        })?;
        let problems = self.page_values(shot, &instance, &alias);
        self.prepare(&instance)?;
        let (_, height) = self.preferred(&instance, Some(PAGE_WIDTH));
        if height > OPTIONS_PAGE_VISIBLE {
            let id = format!("{}-page", shot.id);
            let (png, width, height) = self.draw(&instance, &window, Some(PAGE_WIDTH), out, &id)?;
            outputs.push(Output {
                id: shot.id.clone(),
                variant: "page",
                png: Some(png),
                width,
                height,
                problems,
            });
        }
        Ok(outputs)
    }

    /// A page's properties on its bridge global (`settings_build`).
    fn page_values(
        &mut self,
        shot: &Shot,
        instance: &ComponentInstance,
        alias: &str,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        if let Err(error) = crate::ui::ui_values::own_default_models(instance, Some(alias)) {
            problems.push(format!("{error:#}"));
        }
        for (name, value) in &shot.properties {
            let result = self
                .convert(&shot.plugin, name, value, &mut problems)
                .and_then(|value| {
                    crate::ui::composition::set_alias_value(instance, alias, name, value)
                });
            if let Err(error) = result {
                problems.push(format!("{name}: {error:#}"));
            }
        }
        problems
    }

    /// The component's preferred logical size, as winit creates a new window
    /// (`WinitWindowAdapter::preferred_size`): the width, then the height at
    /// that width. Slint has no public interface for it; recheck on upgrades.
    fn preferred(&self, instance: &ComponentInstance, width: Option<f32>) -> (f32, f32) {
        use slint::private_unstable_api::re_exports::{ItemTreeRc, Orientation, WindowInner};
        let window = instance.window();
        let info = |orientation| {
            WindowInner::from_pub(window)
                .try_component()
                .map(|component| {
                    let component = ItemTreeRc::borrow_pin(&component);
                    component
                        .as_ref()
                        .layout_info(orientation)
                        .preferred_bounded()
                })
                .unwrap_or_default()
        };
        crate::ui::view::settle_layout(window);
        let width = width
            .unwrap_or_else(|| info(Orientation::Horizontal))
            .ceil();
        window.set_size(slint::LogicalSize::new(width, 1.));
        crate::ui::view::settle_layout(window);
        let height = info(Orientation::Vertical).ceil();
        (width, height)
    }

    /// Show the view offscreen at its preferred size and write it as PNG.
    fn draw(
        &self,
        instance: &ComponentInstance,
        window: &MinimalSoftwareWindow,
        width: Option<f32>,
        out: &Path,
        id: &str,
    ) -> Result<(PathBuf, u32, u32)> {
        instance
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: self.scale,
            });
        instance.show()?;
        let (width, height) = self.preferred(instance, width);
        if width < 1. || height < 1. {
            bail!("no preferred size ({width}×{height})");
        }
        instance
            .window()
            .set_size(slint::LogicalSize::new(width, height));
        let size = instance.window().size();
        let (w, h) = (size.width, size.height);
        slint::platform::update_timers_and_animations();
        crate::ui::view::settle_layout(instance.window());
        let mut buffer = vec![PremultipliedRgbaColor::default(); w as usize * h as usize];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, w as usize);
        });
        instance.hide()?;
        let mut pixels = Vec::with_capacity(buffer.len() * 4);
        for p in buffer {
            let unmultiply = |c: u8| {
                if p.alpha == 0 {
                    0
                } else {
                    ((c as u32 * 255 + p.alpha as u32 / 2) / p.alpha as u32).min(255) as u8
                }
            };
            pixels.extend_from_slice(&[
                unmultiply(p.red),
                unmultiply(p.green),
                unmultiply(p.blue),
                p.alpha,
            ]);
        }
        let png = out.join(format!("{id}.png"));
        image::RgbaImage::from_raw(w, h, pixels)
            .context("pixel buffer")?
            .save(&png)
            .with_context(|| format!("writing {}", png.display()))?;
        Ok((png, w, h))
    }
}

/// A runtime image or a Markdown image from a local file (the player also
/// checks that it is in the plugin's package or data directory; recorded
/// paths may point at test folders that no longer exist).
fn image(path: &str, side: u32) -> std::result::Result<slint::Image, String> {
    if !Path::new(path).is_absolute() {
        return Err(format!("{path}: not a local file"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    crate::ui::ui_image::decode_within(&bytes, side)
        .map(|(image, _)| image)
        .map_err(|e| format!("{path}: {}", e.message))
}

/// Run one job file (`VESTRA_UI_SHOTS`); the outputs go to
/// `<out>/results-<locale>.json`.
fn run(job: &Path) -> Result<()> {
    let mut job: Job = serde_json::from_str(
        &std::fs::read_to_string(job).with_context(|| format!("reading {}", job.display()))?,
    )?;
    std::fs::create_dir_all(&job.out)?;
    let mut renderer = Renderer::new(&job)?;
    let mut outputs = Vec::new();
    for shot in &mut job.shots {
        // The catalog is registered for the directory as the host does.
        if let Ok(plugin) = dunce::canonicalize(&shot.plugin) {
            shot.plugin = plugin;
        }
        outputs.extend(renderer.shot(shot, &job.out));
    }
    let results = job.out.join(format!("results-{}.json", job.locale));
    std::fs::write(&results, serde_json::to_string_pretty(&outputs)?)?;
    diag!(
        "ui-shots: {} images in {} ({})",
        outputs.iter().filter(|o| o.png.is_some()).count(),
        job.out.display(),
        job.locale
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "the screenshot tool's renderer: run by `npm run ui:shots`"]
    fn render_plugin_screenshots() {
        let Some(job) = std::env::var_os("VESTRA_UI_SHOTS") else {
            diag!("VESTRA_UI_SHOTS is not set: nothing to render (npm run ui:shots)");
            return;
        };
        run(Path::new(&job)).unwrap();
    }
}
