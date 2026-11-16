//! What the Host keeps for a unit in standby (formerly `lazy.rs`'s `Origin`
//! and `Parked`): the core holds its state and held triggers
//! (`UnitState::Standby`, `Unit::backlog`); the registrations it keeps
//! still need the package's catalog, image roots, name and refusals.
use crate::i18n::{self, Catalog};
use std::path::PathBuf;
use std::sync::Arc;

/// A package as registrations see it: a running plugin's
/// (`process::Plugin::origin`) or one in standby.
#[derive(Clone)]
pub(crate) struct Origin {
    pub id: String,
    pub directory: PathBuf,
    pub catalog: Option<Arc<Catalog>>,
    pub data_directory: Option<PathBuf>,
}
impl Origin {
    /// A string it declared, in the interface language.
    pub fn text(&self, source: &str) -> String {
        self.catalog
            .as_ref()
            .map_or(source, |catalog| catalog.text(source))
            .to_owned()
    }
    /// Text values in the interface language, with its catalog.
    pub fn texts(&self) -> i18n::text::Texts<'_> {
        i18n::text::Texts {
            catalog: self.catalog.as_deref(),
            locale: i18n::locale(),
        }
    }
    /// Where its images may come from: its package, its data directory.
    pub fn image_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.directory.clone()];
        if let Some(data) = self
            .data_directory
            .as_deref()
            .and_then(|d| dunce::canonicalize(d).ok())
        {
            roots.push(data);
        }
        roots
    }
}

/// A unit in standby as the Host sees it: no process, its registrations
/// kept under its holder (today's `Parked`; its held triggers are the
/// core's backlog now).
pub(crate) struct Held {
    pub id: String,
    /// Its package: its catalog (registered for its settings pages' Slint,
    /// `i18n::catalog_for`: an `Arc` of its own, since the stopped plugin
    /// unregisters its own on another thread), and where its images come
    /// from (`Core::image_roots`).
    pub origin: Origin,
    /// Its name, for `Starting <name>…`.
    pub name: String,
    /// What it was refused while it ran (`packages.list` `warnings`, and
    /// as text `error`): kept, so a park alone changes nothing there.
    pub warnings: Vec<serde_json::Value>,
    pub warning: Option<String>,
}
impl Held {
    pub(crate) fn new(id: String, origin: Origin, name: String) -> Self {
        if let Some(catalog) = &origin.catalog {
            i18n::register(&origin.directory, catalog.clone());
        }
        Self {
            id,
            origin,
            name,
            warnings: Vec::new(),
            warning: None,
        }
    }
    /// Held again (a start that failed after it was let go): its catalog
    /// registered again.
    pub(crate) fn reheld(self) -> Self {
        let Held {
            id,
            origin,
            name,
            warnings,
            warning,
        } = self;
        Held {
            warnings,
            warning,
            ..Held::new(id, origin, name)
        }
    }
    /// It is no longer held: its catalog goes (if still this one).
    pub(crate) fn release(&self) {
        if let Some(catalog) = &self.origin.catalog {
            i18n::unregister(&self.origin.directory, catalog);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn origin(directory: &Path, data: Option<PathBuf>) -> Origin {
        let catalog = Catalog::from_json(r#"{"language":"zh-Hans","messages":{"Hello":"你好"}}"#);
        Origin {
            id: "a.b".into(),
            directory: directory.to_owned(),
            catalog: Some(Arc::new(catalog.unwrap())),
            data_directory: data,
        }
    }

    /// Review M2: a parked plugin keeps its translations although the
    /// stopped plugin unregisters its own catalog later, on another thread.
    #[test]
    fn a_parked_catalog_outlives_the_stopped_plugins_own() {
        let directory = tempfile::tempdir().unwrap();
        let page = directory.path().join("page.slint");
        let running = origin(directory.path(), None);
        let own = running.catalog.clone().unwrap();
        i18n::register(directory.path(), own.clone());
        // As `park`: a catalog of its own (loaded again), then the plugin's
        // drop unregisters the plugin's.
        let held = Held::new("a.b".into(), origin(directory.path(), None), "A".into());
        i18n::unregister(directory.path(), &own);
        let shown = i18n::catalog_for(&page).expect("still registered");
        assert!(Arc::ptr_eq(&shown, held.origin.catalog.as_ref().unwrap()));
        assert_eq!(shown.text("Hello"), "你好");
        held.release();
        assert!(i18n::catalog_for(&page).is_none());
    }

    /// Review M3: a parked plugin's images come from its package and data
    /// directories, as a running one's (`Host::image_roots`).
    #[test]
    fn a_parked_origin_names_its_image_roots() {
        let package = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let origin = origin(package.path(), Some(data.path().to_owned()));
        assert_eq!(
            origin.image_roots(),
            [
                package.path().to_owned(),
                dunce::canonicalize(data.path()).unwrap()
            ]
        );
        let missing = Origin {
            data_directory: Some(data.path().join("gone")),
            ..origin
        };
        assert_eq!(missing.image_roots(), [package.path().to_owned()]);
        assert_eq!(missing.text("Hello"), "你好");
    }
}
