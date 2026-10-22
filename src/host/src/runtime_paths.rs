//! Where the host finds what it runs besides itself (packaging proposal
//! §2.4): Node, the plugin runner, the host's Slint files, the built-in
//! plugins, FFmpeg and ffprobe. Resolved once per process (`get`).
//!
//! | | packaged (`platform::current_install_layout`) | development build |
//! | --- | --- | --- |
//! | Node | `$VESTRA_NODE`, else the bundled one; **never `PATH`** | `$VESTRA_NODE`, else `node` on `PATH` |
//! | runner | `<runtime>/runner.ts` | `src/runtime/runner.ts` |
//! | ui | `<ui>` | `src/host/ui` |
//! | built-in plugins | `$VESTRA_BUILTIN_PLUGINS`, else `<plugins>` | `$VESTRA_BUILTIN_PLUGINS`, else none |
//! | FFmpeg, ffprobe | `$VESTRA_FFMPEG` / `$VESTRA_FFPROBE`, else the bundled ones | the same variables, else `PATH` |
//! | licenses | `<licenses>` | none |
//!
//! A packaged player that misses one of its own files says so at start-up
//! (`missing`) instead of failing each plugin. Its plugin processes get the
//! bundled FFmpeg as `VESTRA_FFMPEG` / `VESTRA_FFPROBE` (what the SDK's
//! `plugin.tools` reads) and none of the user's `NODE_OPTIONS` / `NODE_PATH`
//! (`node_command`).
#![forbid(unsafe_code)]

use crate::platform::{self, InstallLayout};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePaths {
    /// Running from an installation, not from a development build.
    pub packaged: bool,
    /// The Node executable; in a development build without `$VESTRA_NODE`
    /// the bare name `node`, which the system looks up on `PATH`.
    pub node: PathBuf,
    pub runner: PathBuf,
    pub ui: PathBuf,
    /// The directory `builtin.json` is read from, if any.
    pub builtin_plugins: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub licenses: Option<PathBuf>,
}

/// The paths of this process (resolved on first use).
pub fn get() -> &'static RuntimePaths {
    static PATHS: OnceLock<RuntimePaths> = OnceLock::new();
    PATHS.get_or_init(|| {
        resolve(
            platform::current_install_layout(),
            |key| std::env::var_os(key),
            Path::new(env!("CARGO_MANIFEST_DIR")),
            on_path,
        )
    })
}

/// `get` as a pure function: `layout` the installation (none in a
/// development build), `env` the environment, `source` the `src/host`
/// directory of a development build, `on_path` a `PATH` lookup.
pub fn resolve(
    layout: Option<InstallLayout>,
    env: impl Fn(&str) -> Option<OsString>,
    source: &Path,
    on_path: impl Fn(&str) -> Option<PathBuf>,
) -> RuntimePaths {
    let var = |key: &str| {
        env(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let tool = |key: &str, name: &str, bundled: Option<&PathBuf>| {
        var(key).or_else(|| match bundled {
            Some(path) => Some(path.clone()),
            None => on_path(name),
        })
    };
    match layout {
        Some(layout) => RuntimePaths {
            packaged: true,
            node: var("VESTRA_NODE").unwrap_or(layout.node),
            runner: layout.runtime.join("runner.ts"),
            ui: layout.ui,
            builtin_plugins: Some(var("VESTRA_BUILTIN_PLUGINS").unwrap_or(layout.plugins)),
            ffmpeg: tool("VESTRA_FFMPEG", "ffmpeg", Some(&layout.ffmpeg)),
            ffprobe: tool("VESTRA_FFPROBE", "ffprobe", Some(&layout.ffprobe)),
            licenses: Some(layout.licenses),
        },
        None => RuntimePaths {
            packaged: false,
            node: var("VESTRA_NODE").unwrap_or_else(|| "node".into()),
            runner: source.join("..").join("runtime").join("runner.ts"),
            ui: source.join("ui"),
            builtin_plugins: var("VESTRA_BUILTIN_PLUGINS"),
            ffmpeg: tool("VESTRA_FFMPEG", "ffmpeg", None),
            ffprobe: tool("VESTRA_FFPROBE", "ffprobe", None),
            licenses: None,
        },
    }
}

/// The first `<name>` (`<name>.exe` on Windows) on `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(&file))
        .find(|candidate| candidate.is_file())
}

impl RuntimePaths {
    /// The files a packaged player cannot run without that are not there,
    /// one line each; empty for a development build (its Node is on `PATH`).
    pub fn missing(&self, is_file: impl Fn(&Path) -> bool) -> Vec<String> {
        if !self.packaged {
            return Vec::new();
        }
        let mut required = vec![
            ("Node", self.node.clone()),
            ("plugin runner", self.runner.clone()),
            ("interface", self.ui.join("host.slint")),
        ];
        if let Some(plugins) = &self.builtin_plugins {
            required.push(("built-in plugins", plugins.join("builtin.json")));
        }
        required
            .into_iter()
            .filter(|(_, path)| !is_file(path))
            .map(|(what, path)| format!("{what}: {} is missing", path.display()))
            .collect()
    }

    /// The built-in plugins to seed: `builtin_plugins` when it has a
    /// `builtin.json`.
    pub fn builtin_directory(&self) -> Option<PathBuf> {
        self.builtin_plugins
            .clone()
            .filter(|directory| directory.join("builtin.json").is_file())
    }

    /// A Node process (plugins, `node --check`). Packaged: the bundled Node,
    /// without the user's `NODE_OPTIONS` / `NODE_PATH` (a `--require` there
    /// would run in every plugin), with the bundled FFmpeg for `plugin.tools`
    /// unless `VESTRA_FFMPEG` / `VESTRA_FFPROBE` already name one.
    /// `NODE_EXTRA_CA_CERTS` stays: proxies with their own CA need it. No
    /// console window on Windows (`platform::hide_console`).
    pub fn node_command(&self) -> Command {
        let mut command = Command::new(&self.node);
        platform::hide_console(&mut command);
        if self.packaged {
            command.env_remove("NODE_OPTIONS").env_remove("NODE_PATH");
            for (key, path) in [
                ("VESTRA_FFMPEG", &self.ffmpeg),
                ("VESTRA_FFPROBE", &self.ffprobe),
            ] {
                if let Some(path) = path.as_ref().filter(|path| path.is_file()) {
                    command.env(key, path);
                }
            }
        }
        command
    }

    /// The reason a Node process did not start, for the plugin's failure.
    pub fn node_failure(&self) -> String {
        if self.packaged {
            format!("start the bundled Node ({})", self.node.display())
        } else {
            format!(
                "start Node ({}; Node 26 on PATH, or VESTRA_NODE)",
                self.node.display()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn layout(root: &Path) -> InstallLayout {
        InstallLayout {
            ui: root.join("res/ui"),
            runtime: root.join("res/runtime"),
            plugins: root.join("res/plugins"),
            licenses: root.join("licenses"),
            node: root.join("bin/node"),
            ffmpeg: root.join("bin/ffmpeg"),
            ffprobe: root.join("bin/ffprobe"),
        }
    }
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: BTreeMap<String, OsString> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), OsString::from(value)))
            .collect();
        move |key| map.get(key).cloned()
    }
    fn path_with(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name| {
            names
                .contains(&name)
                .then(|| Path::new("/usr/bin").join(name))
        }
    }

    #[test]
    fn a_packaged_player_uses_its_own_files_and_never_path() {
        let root = Path::new("/app");
        let paths = resolve(
            Some(layout(root)),
            env(&[]),
            Path::new("/src/host"),
            path_with(&["node", "ffmpeg", "ffprobe"]),
        );
        assert_eq!(
            paths,
            RuntimePaths {
                packaged: true,
                node: root.join("bin/node"),
                runner: root.join("res/runtime").join("runner.ts"),
                ui: root.join("res/ui"),
                builtin_plugins: Some(root.join("res/plugins")),
                ffmpeg: Some(root.join("bin/ffmpeg")),
                ffprobe: Some(root.join("bin/ffprobe")),
                licenses: Some(root.join("licenses")),
            }
        );
    }

    #[test]
    fn variables_override_both_kinds_of_run_and_empty_ones_do_not() {
        let variables = env(&[
            ("VESTRA_NODE", "/opt/node26/bin/node"),
            ("VESTRA_BUILTIN_PLUGINS", "/stage/plugins"),
            ("VESTRA_FFMPEG", "/ff/ffmpeg"),
            ("VESTRA_FFPROBE", ""),
        ]);
        let packaged = resolve(
            Some(layout(Path::new("/app"))),
            &variables,
            Path::new("/s"),
            path_with(&[]),
        );
        assert_eq!(packaged.node, Path::new("/opt/node26/bin/node"));
        assert_eq!(
            packaged.builtin_plugins.as_deref(),
            Some(Path::new("/stage/plugins"))
        );
        assert_eq!(packaged.ffmpeg.as_deref(), Some(Path::new("/ff/ffmpeg")));
        assert_eq!(
            packaged.ffprobe.as_deref(),
            Some(Path::new("/app/bin/ffprobe"))
        );
        let development = resolve(None, &variables, Path::new("/s"), path_with(&["ffprobe"]));
        assert_eq!(development.node, Path::new("/opt/node26/bin/node"));
        assert_eq!(
            development.ffprobe.as_deref(),
            Some(Path::new("/usr/bin/ffprobe"))
        );
    }

    #[test]
    fn a_development_build_uses_the_source_tree_and_path() {
        let source = Path::new("/repo/src/host");
        let paths = resolve(None, env(&[]), source, path_with(&["ffmpeg"]));
        assert_eq!(
            paths,
            RuntimePaths {
                packaged: false,
                node: "node".into(),
                runner: source.join("..").join("runtime").join("runner.ts"),
                ui: source.join("ui"),
                builtin_plugins: None,
                ffmpeg: Some(Path::new("/usr/bin").join("ffmpeg")),
                ffprobe: None,
                licenses: None,
            }
        );
        assert!(paths.missing(|_: &Path| false).is_empty());
    }

    #[test]
    fn a_packaged_player_names_each_missing_file() {
        let paths = resolve(
            Some(layout(Path::new("/app"))),
            env(&[]),
            Path::new("/s"),
            path_with(&[]),
        );
        let runner = paths.runner.clone();
        let missing = paths.missing(|path: &Path| path == runner);
        assert_eq!(missing.len(), 3, "{missing:?}");
        assert!(
            missing[0].starts_with("Node: ") && missing[0].contains("bin"),
            "{missing:?}"
        );
        assert!(missing[1].starts_with("interface: "));
        assert!(missing[2].starts_with("built-in plugins: "));
        assert!(paths.missing(|_: &Path| true).is_empty());
    }

    #[test]
    fn plugins_run_on_the_bundled_node_with_the_bundled_ffmpeg() {
        let root = tempfile::tempdir().unwrap();
        let mut paths = resolve(
            Some(layout(root.path())),
            env(&[]),
            Path::new("/s"),
            path_with(&[]),
        );
        std::fs::create_dir_all(root.path().join("bin")).unwrap();
        std::fs::write(root.path().join("bin/ffmpeg"), "").unwrap();
        let command = paths.node_command();
        assert_eq!(command.get_program(), root.path().join("bin/node"));
        let envs: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(envs[std::ffi::OsStr::new("NODE_OPTIONS")], None);
        assert_eq!(envs[std::ffi::OsStr::new("NODE_PATH")], None);
        assert_eq!(
            envs[std::ffi::OsStr::new("VESTRA_FFMPEG")],
            Some(root.path().join("bin/ffmpeg").as_os_str())
        );
        // ffprobe is not there: plugins look it up themselves.
        assert!(!envs.contains_key(std::ffi::OsStr::new("VESTRA_FFPROBE")));

        // A development build changes nothing in the environment.
        paths.packaged = false;
        assert_eq!(paths.node_command().get_envs().count(), 0);
    }
}
