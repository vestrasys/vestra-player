//! Shared by the integration tests that start the host: every such start
//! runs in its own profile under a test folder, never the user's.
//!
//! A host started with the user's `APPDATA`/`LOCALAPPDATA` (or `HOME`, `XDG_*`
//! elsewhere) shares the real player's data and cache: its start-up cleans
//! the cache's export folders, writes the log and the window state there.
//! Evidence (2026-11-13/15): test players removed a running export's folder
//! of intermediate parts in the user's `%LOCALAPPDATA%\Vestra\export-parts`.

use std::path::Path;
use std::process::Command;

/// Points every per-user folder of the host started by `command` into
/// `root`: the system's data and cache roots, the store, the plugin data,
/// the window state, the shader cache; the log goes there too (no
/// `VESTRA_LOG_FILE`), and a headless run coordinates with no other player
/// (no `VESTRA_INSTANCE_DIR`; a caller that wants one sets it after this).
pub fn isolate<'a>(command: &'a mut Command, root: &Path) -> &'a mut Command {
    command
        .env("APPDATA", root.join("appdata"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env("VESTRA_UI_STATE", root.join("ui-state.json"))
        .env("VESTRA_SHADER_CACHE", root.join("shader-cache"))
        .env_remove("VESTRA_LOG_FILE")
        .env_remove("VESTRA_INSTANCE_DIR");
    // macOS keeps the player's data and cache under `HOME/Library`.
    if cfg!(target_os = "macos") {
        command.env("HOME", root.join("home"));
    }
    command
}
