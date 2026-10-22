//! Where an installed player keeps the files it ships (packaging proposal
//! §2.4, §3.1, §4.2). The layouts are a pure function of the system, the
//! executable's path and a file test, so every system's layout is tested on
//! every machine; `super::install_layout` picks the running system.
//!
//! | | macOS `.app` | Windows install directory |
//! | --- | --- | --- |
//! | recognised by | `<X>.app/Contents/MacOS/<exe>` with `Contents/Info.plist` | `<dir>\<exe>` with `<dir>\resources\runtime\runner.ts` |
//! | resources | `Contents/Resources` | `<dir>\resources` |
//! | ui, runtime, plugins | `<resources>/{ui,runtime,plugins}` | the same |
//! | Node | `Contents/MacOS/node` | `<dir>\node\node.exe` |
//! | FFmpeg | `Contents/MacOS/{ffmpeg,ffprobe}` | `<dir>\ffmpeg\{ffmpeg,ffprobe}.exe` |
//! | licenses | `Contents/Resources/licenses` | `<dir>\licenses` |
//!
//! Anything else (a development build in `target/`) is no installation:
//! `None`, and the host uses the source tree (`runtime_paths.rs`). Linux has
//! no packaged layout yet.

use super::System;
use std::path::{Path, PathBuf};

/// The files of one installation. The paths are where they belong; whether
/// they are there is checked by the caller (`RuntimePaths::missing`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallLayout {
    /// `host.slint` and the `@vestra` library.
    pub ui: PathBuf,
    /// The plugin runtime: `runner.ts` and the SDK (`stage-runtime.mjs`).
    pub runtime: PathBuf,
    /// The built-in plugins: `builtin.json` and the `.vxp` files
    /// (`stage-plugins.mjs`).
    pub plugins: PathBuf,
    /// Third-party licenses and the player's own `LICENSE`.
    pub licenses: PathBuf,
    /// The bundled Node executable.
    pub node: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// The installation `exe` runs from on `system`, if it is one.
pub fn install_layout(
    system: System,
    exe: &Path,
    is_file: impl Fn(&Path) -> bool,
) -> Option<InstallLayout> {
    let directory = exe.parent()?;
    match system {
        System::MacOs => {
            if directory.file_name()? != "MacOS" {
                return None;
            }
            let contents = directory.parent()?;
            if contents.file_name()? != "Contents"
                || !contents
                    .parent()?
                    .extension()
                    .is_some_and(|extension| extension == "app")
                || !is_file(&contents.join("Info.plist"))
            {
                return None;
            }
            let resources = contents.join("Resources");
            Some(InstallLayout {
                ui: resources.join("ui"),
                runtime: resources.join("runtime"),
                plugins: resources.join("plugins"),
                licenses: resources.join("licenses"),
                node: directory.join("node"),
                ffmpeg: directory.join("ffmpeg"),
                ffprobe: directory.join("ffprobe"),
            })
        }
        System::Windows => {
            let resources = directory.join("resources");
            if !is_file(&resources.join("runtime").join("runner.ts")) {
                return None;
            }
            let tools = directory.join("ffmpeg");
            Some(InstallLayout {
                ui: resources.join("ui"),
                runtime: resources.join("runtime"),
                plugins: resources.join("plugins"),
                licenses: directory.join("licenses"),
                node: directory.join("node").join("node.exe"),
                ffmpeg: tools.join("ffmpeg.exe"),
                ffprobe: tools.join("ffprobe.exe"),
            })
        }
        System::Linux => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: BTreeSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |path| set.contains(path)
    }

    #[test]
    fn a_mac_app_bundle_keeps_resources_and_helpers_apart() {
        let exe = Path::new("/Applications/Vestra Player.app/Contents/MacOS/vestra-host");
        let layout = install_layout(
            System::MacOs,
            exe,
            files(&["/Applications/Vestra Player.app/Contents/Info.plist"]),
        )
        .unwrap();
        let contents = Path::new("/Applications/Vestra Player.app/Contents");
        assert_eq!(
            layout,
            InstallLayout {
                ui: contents.join("Resources/ui"),
                runtime: contents.join("Resources/runtime"),
                plugins: contents.join("Resources/plugins"),
                licenses: contents.join("Resources/licenses"),
                node: contents.join("MacOS/node"),
                ffmpeg: contents.join("MacOS/ffmpeg"),
                ffprobe: contents.join("MacOS/ffprobe"),
            }
        );
    }

    #[test]
    fn a_mac_build_outside_a_bundle_is_no_installation() {
        let none = files(&[]);
        for exe in [
            "/src/host/target/debug/vestra-host",
            "/target/release/deps/vestra_host-0123",
            // No Info.plist: a directory that only looks like a bundle.
            "/tmp/Vestra Player.app/Contents/MacOS/vestra-host",
            "/vestra-host",
        ] {
            assert_eq!(
                install_layout(System::MacOs, Path::new(exe), &none),
                None,
                "{exe}"
            );
        }
        assert_eq!(
            install_layout(
                System::MacOs,
                Path::new("/x/Contents/MacOS/vestra-host"),
                files(&["/x/Contents/Info.plist"])
            ),
            None,
            "not an .app"
        );
    }

    #[test]
    fn a_windows_install_directory_has_resources_beside_the_exe() {
        let root = Path::new("/Users/u/AppData/Local/Programs/Vestra Player");
        let runner = root.join("resources").join("runtime").join("runner.ts");
        let layout = install_layout(
            System::Windows,
            &root.join("vestra-host.exe"),
            |path: &Path| path == runner,
        )
        .unwrap();
        assert_eq!(
            layout,
            InstallLayout {
                ui: root.join("resources").join("ui"),
                runtime: root.join("resources").join("runtime"),
                plugins: root.join("resources").join("plugins"),
                licenses: root.join("licenses"),
                node: root.join("node").join("node.exe"),
                ffmpeg: root.join("ffmpeg").join("ffmpeg.exe"),
                ffprobe: root.join("ffmpeg").join("ffprobe.exe"),
            }
        );
        // A build in target\debug has no staged runtime beside it.
        assert_eq!(
            install_layout(
                System::Windows,
                Path::new("/src/host/target/debug/vestra-host.exe"),
                files(&[])
            ),
            None
        );
    }

    #[test]
    fn linux_has_no_packaged_layout_yet() {
        assert_eq!(
            install_layout(
                System::Linux,
                Path::new("/usr/lib/vestra/vestra-host"),
                |_: &Path| true
            ),
            None
        );
    }
}
