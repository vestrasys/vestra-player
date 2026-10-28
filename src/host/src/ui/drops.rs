//! File drops (R-U2, capability `ui.drop`). winit 0.30 reports one
//! `DroppedFile` per path without position or modifiers; the paths of one
//! drop arrive back to back, so a group closes after 30 ms without another.
//! On closing, the platform gives the pointer and modifier keys (AppKit on
//! macOS, Win32 on Windows; `cmd` is Ctrl there); the drop goes to
//! the panel under the pointer (if it listens) or, by extension, to the
//! providers (`drops.provide`). Directories match the reserved name `folder`.
#![forbid(unsafe_code)]
use crate::*;

const GROUP_MS: u128 = 30;
const MAX_EXTENSIONS: usize = 64;

#[derive(Clone, Debug)]
pub struct Provider {
    pub owner: u64,
    pub id: String,
    pub extensions: Vec<String>,
}

#[derive(Default)]
pub struct Drops {
    /// In registration order: the first one with an extension wins it.
    providers: Vec<Provider>,
    /// Plugins whose panel has an `onDrop` listener.
    pub panels: std::collections::BTreeSet<u64>,
    /// The drop being collected: window generation, paths, last arrival.
    pending: Option<(u64, Vec<PathBuf>, Instant)>,
}

/// The matching name of a path: `folder`, or its extension in lower case.
pub fn kind(path: &Path) -> Option<String> {
    if path.is_dir() {
        return Some("folder".into());
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| !e.is_empty())
}

pub fn parse_extensions(value: &Value) -> std::result::Result<Vec<String>, String> {
    let list = value.as_array().ok_or("extensions must be a list")?;
    if list.is_empty() || list.len() > MAX_EXTENSIONS {
        return Err(format!("1–{MAX_EXTENSIONS} extensions"));
    }
    list.iter()
        .map(|e| {
            e.as_str()
                .filter(|e| {
                    !e.is_empty()
                        && e.len() <= 16
                        && e.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
                .map(str::to_owned)
                .ok_or_else(|| {
                    "extensions are lower-case letters and digits, without the dot".into()
                })
        })
        .collect()
}

impl Drops {
    /// Register; returns the extensions an earlier provider already owns (logged).
    pub fn provide(&mut self, provider: Provider) -> Vec<String> {
        let taken = provider
            .extensions
            .iter()
            .filter(|e| self.providers.iter().any(|p| p.extensions.contains(e)))
            .cloned()
            .collect();
        self.providers
            .retain(|p| !(p.owner == provider.owner && p.id == provider.id));
        self.providers.push(provider);
        taken
    }
    pub fn withdraw(&mut self, owner: u64, id: &str) {
        self.providers.retain(|p| !(p.owner == owner && p.id == id));
    }
    pub fn remove_owner(&mut self, owner: u64) {
        self.providers.retain(|p| p.owner != owner);
        self.panels.remove(&owner);
    }
    /// Paths by provider, each group in the order the paths came; paths no
    /// provider takes are dropped.
    pub fn groups(&self, paths: &[PathBuf]) -> Vec<(u64, String, Vec<String>)> {
        let mut groups: Vec<(u64, String, Vec<String>)> = Vec::new();
        for path in paths {
            let Some(kind) = kind(path) else { continue };
            let Some(provider) = self.providers.iter().find(|p| p.extensions.contains(&kind))
            else {
                continue;
            };
            let text = path.to_string_lossy().into_owned();
            match groups
                .iter_mut()
                .find(|(owner, id, _)| *owner == provider.owner && *id == provider.id)
            {
                Some(group) => group.2.push(text),
                None => groups.push((provider.owner, provider.id.clone(), vec![text])),
            }
        }
        groups
    }
    pub fn arrived(&mut self, generation: u64, path: PathBuf) {
        match &mut self.pending {
            Some((pending, paths, at)) if *pending == generation => {
                paths.push(path);
                *at = Instant::now();
            }
            _ => self.pending = Some((generation, vec![path], Instant::now())),
        }
    }
    /// A complete drop: no further path for 30 ms.
    pub fn due(&mut self) -> Option<(u64, Vec<PathBuf>)> {
        let quiet = self
            .pending
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed().as_millis() >= GROUP_MS);
        quiet
            .then(|| {
                self.pending
                    .take()
                    .map(|(generation, paths, _)| (generation, paths))
            })
            .flatten()
    }
}

/// Where a drop at `(x, y)` in the main window lands: a docked panel (with
/// panel coordinates) or the video column.
pub fn hit(
    point: (f64, f64),
    video: (f64, f64),
    bottom: Option<u64>,
    sidebar: Option<u64>,
) -> Option<(u64, (f64, f64))> {
    let (x, y) = point;
    if let Some(owner) = sidebar
        && x > video.0 + 3.
    {
        return Some((owner, (x - video.0 - 3., y)));
    }
    if let Some(owner) = bottom
        && x <= video.0
        && y > video.1
    {
        return Some((owner, (x, y - video.1)));
    }
    None
}

impl Host {
    pub(crate) fn drops_dispatch(
        &mut self,
        owner: u64,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, RpcError> {
        match method {
            "drops.provide" => {
                let id = string(params, "id")?;
                let extensions =
                    parse_extensions(&params["extensions"]).map_err(RpcError::invalid)?;
                let taken = self.drops.provide(Provider {
                    owner,
                    id: id.into(),
                    extensions,
                });
                if !taken.is_empty() {
                    diag!(
                        "drops: {} already provided for {taken:?}; the earlier provider keeps them",
                        self.plugins
                            .get(&owner)
                            .map_or("?", |p| p.manifest.id.as_str())
                    );
                }
                Ok(Value::Null)
            }
            "drops.withdraw" => {
                self.drops.withdraw(owner, string(params, "id")?);
                Ok(Value::Null)
            }
            "drops.listen" => {
                let listen = params["listen"]
                    .as_bool()
                    .ok_or_else(|| RpcError::invalid("listen requires a boolean"))?;
                if listen {
                    self.drops.panels.insert(owner);
                } else {
                    self.drops.panels.remove(&owner);
                }
                Ok(Value::Null)
            }
            _ => Err(RpcError::new("unsupported", "unknown drops method")),
        }
    }
    /// Deliver a completed drop.
    pub(crate) fn drops_tick(&mut self) {
        let Some((generation, paths)) = self.drops.due() else {
            return;
        };
        let (alt, shift, cmd) = platform::modifier_keys();
        let modifiers = json!({"alt":alt,"shift":shift,"cmd":cmd});
        let paths_json = json!(
            paths
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
        );
        // The panel under the pointer, in its own coordinates.
        let panel = if self
            .floating
            .as_ref()
            .is_some_and(|f| f.generation == generation)
        {
            let floating = self.floating.as_ref().unwrap();
            let at = floating
                .native
                .and_then(|id| platform::pointer_in(id).ok())
                .unwrap_or((0., 0.));
            self.policy.sidebar_active().map(|owner| (owner, at))
        } else if generation == self.shell_generation {
            if let Some(shell) = &self.shell {
                view::settle_layout(shell.window());
            }
            let number = |name: &str| match self.shell.as_ref().map(|s| s.get_property(name)) {
                Some(Ok(SValue::Number(n))) => n,
                _ => 0.,
            };
            let fullscreen = self.window_facts["fullscreen"] == true;
            let point = self
                .native_window
                .and_then(|id| platform::pointer_in(id).ok());
            let input = self.input.borrow();
            // G-01: in fullscreen only the popped-out sidebar takes drops.
            let overlay = self.sidebar_popped.then(|| {
                let width = number("logical-width");
                width - number("sidebar-pop-width")
            });
            point.and_then(|point| match (fullscreen, overlay) {
                (false, _) => hit(
                    point,
                    (number("video-width"), number("video-height")),
                    input.bottom,
                    input.sidebar,
                ),
                (true, Some(left)) if point.0 > left => input
                    .sidebar
                    .map(|owner| (owner, (point.0 - left, point.1))),
                (true, _) => None,
            })
        } else {
            // A tool window or the options window: ignored.
            return;
        };
        // Sidebar tabs: the current panel starts below the host's tab strip.
        let tabbed = self.policy.sidebar_panels().len() > 1;
        let panel = panel.map(|(owner, (x, y))| {
            if tabbed && Some(owner) == self.policy.sidebar_active() {
                (owner, (x, y - crate::ui::composition::TAB_STRIP_HEIGHT))
            } else {
                (owner, (x, y))
            }
        });
        if let Some((owner, (x, y))) = panel
            && self.drops.panels.contains(&owner)
        {
            self.send(
                owner,
                json!({"event":"drops.panel","data":{"paths":paths_json,"at":{"x":x,"y":y},"modifiers":modifiers}}),
            );
            return;
        }
        for (owner, id, paths) in self.drops.groups(&paths) {
            self.send(
                owner,
                json!({"event":"drops.drop","data":{"id":id,"paths":paths,"modifiers":modifiers}}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_providers_in_registration_order() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Album");
        std::fs::create_dir(&folder).unwrap();
        let mut drops = Drops::default();
        assert!(
            drops
                .provide(Provider {
                    owner: 1,
                    id: "media".into(),
                    extensions: vec!["mkv".into(), "folder".into()]
                })
                .is_empty()
        );
        let taken = drops.provide(Provider {
            owner: 2,
            id: "subs".into(),
            extensions: vec!["srt".into(), "mkv".into()],
        });
        assert_eq!(taken, ["mkv"]);
        let paths = [
            PathBuf::from("/m/b.MKV"),
            PathBuf::from("/m/a.srt"),
            folder.clone(),
            PathBuf::from("/m/c.txt"),
            PathBuf::from("/m/noext"),
            PathBuf::from("/m/a.mkv"),
        ];
        let groups = drops.groups(&paths);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, 1);
        assert_eq!(
            groups[0].2,
            [
                "/m/b.MKV".to_owned(),
                folder.to_string_lossy().into_owned(),
                "/m/a.mkv".into()
            ]
        );
        assert_eq!(
            (groups[1].0, groups[1].2.clone()),
            (2, vec!["/m/a.srt".to_owned()])
        );
        drops.remove_owner(1);
        assert_eq!(
            drops.groups(&paths)[0].2,
            ["/m/b.MKV", "/m/a.srt", "/m/a.mkv"]
        );
        assert!(parse_extensions(&json!(["mkv", "m3u8"])).is_ok());
        for bad in [json!([]), json!([".mkv"]), json!(["MKV"]), json!("mkv")] {
            assert!(parse_extensions(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_of_one_drop_group_and_panels_are_hit_in_their_coordinates() {
        let mut drops = Drops::default();
        drops.arrived(1, "/a".into());
        drops.arrived(1, "/b".into());
        assert!(drops.due().is_none(), "still collecting");
        std::thread::sleep(std::time::Duration::from_millis(35));
        let (generation, paths) = drops.due().unwrap();
        assert_eq!((generation, paths.len()), (1, 2));
        assert!(drops.due().is_none());
        // Video 400×300, bottom bar under it, sidebar right of the splitter.
        assert_eq!(hit((100., 100.), (400., 300.), Some(1), Some(2)), None);
        assert_eq!(
            hit((100., 320.), (400., 300.), Some(1), Some(2)),
            Some((1, (100., 20.)))
        );
        assert_eq!(
            hit((413., 50.), (400., 300.), Some(1), Some(2)),
            Some((2, (10., 50.)))
        );
        assert_eq!(hit((413., 50.), (400., 300.), Some(1), None), None);
    }
}
