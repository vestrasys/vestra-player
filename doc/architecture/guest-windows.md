# Guest windows ("Play in New Window")

Player 0.10.3. User ruling 2026-11-21 (doc/process/rounds/history.md, Loop log, "guest windows"). Chrome's guest window, built the way VS Code's `--user-data-dir` is: a **guest** is a separate `vestra-host` process with a temporary profile directory of its own. Core, wire and playback are unchanged: a guest is an ordinary player whose profile directory is somewhere else and whose writes to the shared parts are refused.

Nothing here needs the owner: the only new visible text is the ruled title marker and the ruled menu item.

## 1. Launch

| Argument | Who writes it | What it does |
| --- | --- | --- |
| `--guest` | the host only (`app.openWindow`); internal, not in the usage text: the playlist's row menu is the only user entry | Start as a guest with a new temporary profile. |
| `--vestra-guest-dir DIR` | the host only (`rendering.restart`, a lost GPU device) | Start as the guest whose directory is `DIR` (it implies `--guest`): a guest that restarts itself keeps its profile. |

The guest's files come after `--`, like any launch's: the first one plays, all go to the list (`drops.drop`).

`src/host/src/guest.rs` owns the guest state; `platform::app_data_dir()` returns the guest's profile directory once `guest::start` ran (the per-system function is `imp::app_data_dir`, read by `platform::main_app_data_dir()`). So all six callers (logs, render files, `ui-state.json`, `media-sources.json`, `plugin-data`, the instance lock) follow without a change of their own, except the three below that must still see the main profile or must not see it at all.

The test switches that name profile files (`VESTRA_UI_STATE`, `VESTRA_PLUGIN_DATA`, `VESTRA_INSTANCE_DIR`, `VESTRA_LOG_FILE`) name the **main** profile's files; a guest inherits them from the player that started it and ignores them (it reads `VESTRA_UI_STATE` once, as the snapshot's source). `VESTRA_PACKAGE_STORE` and `VESTRA_BUILTIN_PLUGINS` keep naming what the guest reads. `guest::spawn` does not pass on the switches that write files of the starting run or script its input (`guest::NOT_INHERITED`: `VESTRA_LOG_FILE`, `VESTRA_TRACE`, `VESTRA_TEST_WIRE_TRACE`, the UI traces, recordings and shots, `VESTRA_FRAME_LOG`, `VESTRA_PIP_LOG`, `VESTRA_PROBE_OUT`, `VESTRA_UPDATE_MPV_RULES`, the scripted `VESTRA_TEST_*` inputs, `VESTRA_INSTANCE_DIR`); `VESTRA_TEST_OFFSCREEN` and `VESTRA_TEST_DISPLAYS` stay, so a guest of an offscreen run is offscreen.

## 2. The directory

```
<app cache>/guests/<uuid>/          %LOCALAPPDATA%\Vestra\guests\<uuid> on Windows
    guest.lock                      held (OS lock) for the guest's life
    packages.json                   the package digests it runs from (§4)
    profile/                        platform::app_data_dir() in the guest
        ui-state.json render.json render-state.json render-denylist.json   (snapshot, §3)
        plugin-data/ logs/ media-sources.json packages/activation packages/triggers   (its own, start empty)
```

Under the cache directory, not the data directory: it is not roaming, it is data nobody needs back, and the main profile (`%APPDATA%\Vestra`) is never touched, so a test can compare it byte for byte. The shader cache and the export folder stay shared (both are already safe between processes: content-named cache files, export folders owned by process).

## 3. The settings snapshot

Copied from the main profile when the guest starts (not on `--vestra-guest-dir`), each file only if it exists; the guest's changes to them die with it.

| File | Copied | Why |
| --- | --- | --- |
| `ui-state.json` | yes, without `paletteRecent` | Theme, language, key bindings, sidebar and adjust layout, the window frame, "Don't ask again" trust: the settings the user expects. The palette's recent commands are usage history, which a guest neither inherits nor leaves behind. The window frame is moved 32 px right and down (logical and desktop pixels), as Windows and browsers cascade a new window, so the guest is not exactly over the main window. |
| `render.json` | yes | The user's rendering choice (GPU or CPU windows). |
| `render-state.json` | yes | The crash guard: a mode that crashed the main player is not tried again by a guest. A guest's own crash or device loss is recorded only in its copy (a later restart of that guest still sees it, §6). |
| `render-denylist.json` | yes | The same driver facts. |
| `media-sources.json` | no | References plugins remembered (their lists' files): plugin data, which starts empty by ruling. |
| `plugin-data/` | no | Ruling: empty. History, the remembered playlist, marks and plugin options all start fresh. |
| `logs/` | no | The guest writes its own, deleted with it. |
| `instance.lock`, `instance.endpoint` | no | §5. |
| `packages/` | no | Read in place, §4. |

## 4. Packages, the store and the keychain

**Read-only from the main profile.** The guest opens the main package library with `Store::open_read_only`: it reads `catalog.json` and `quarantine.json`, screens the records in memory as `open` does, and writes nothing: no backup, no `quarantine.json`, no catalog (`publish` changes only the copy in memory, so the built-in records `sync_builtins` adds and a pending install's recovery behave as in the main player, without a write). Activation recordings and trigger files (`activation_dir`, `triggers_dir`) go to the guest's own `profile/packages/`, so lazy activation starts from scratch, as it does on a first run.

The package manager in a guest:
- refuses `packages.install`, `packages.uninstall` and `packages.setEnabled` with `unavailable` ("a temporary window cannot install, update, remove, enable or disable plugins; use the main window"); `packages.list`, `packages.installed`, `packages.validate*`, `packages.reload` and `packages.operation` work as before;
- runs no library cleanup (`collect_garbage`, `discard`, `hold_freed`), no stale-staging sweep and no keychain purge of retired instances.

**Background checks.** The Plugin Store's automatic repository check and Info's automatic player-update check are the plugins' own timers. The host marks a guest in `plugin.app`'s source as `"temporary": true` (absent in a main player, so nothing changes there): in `VESTRA_APP_INFO` of an own process and in the shared runner's per-member `app`; the SDK (1.10.1) gives it to plugins as `plugin.app.temporary` (`AppInfo.temporary`), and the two built-ins skip their automatic checks when it is set (a check the user starts still runs). Chrome gives extensions the same fact (`inIncognitoContext`).

**A running guest keeps its package versions.** The library holds each version in its own directory named by its digest (`packages/packages/<digest>`); an update or uninstall in the main player removes the old directory afterwards (`discard`, on the `library-cleanup` thread), and the next start collects anything unreferenced (`collect`). Rule: **the cleanup thread never removes a digest a live guest lists.** Before it deletes, it reads `packages.json` of every guest whose `guest.lock` it cannot take (`guest::leased`) and leaves those directories; the next main start's `collect` removes them once that guest is gone. Fail-safe: such a guest whose lease cannot be read or parsed (not written yet, or broken), or a guests directory that cannot be listed, keeps everything for that pass (`Leases::All`). The same rule holds for the forced removal of a set-aside record's versions (`hold_freed`). The guests directory is the manager's (`Manager::guests`), `None` in the host's unit tests, which never read the user's. Ordering (no window where a guest runs from a removed version): the guest writes `packages.json` from the catalog it read, then reads the catalog again and repeats until the second read names nothing new; the main player commits the new catalog before it reads the leases. Built-ins run from the installation, not the library, so this does not concern them (a player update replaces the installation for every window alike, as today).

**Keychain read-only.** `secrets.get` reads the main profile's entries (same instance, same service name). `secrets.set` and `secrets.delete` are refused with `unavailable` ("a temporary window cannot save or delete sign-in details; use the main window"); an existing code, so no SDK change. The secret-name index is not opened for writing.

## 5. Single instance, closing, media keys

- A guest takes no part in the single-instance lock: it neither claims `instance.lock` nor hands its files over, and later launches never reach it. So a guest never becomes the main window, closing the main window leaves guests running, and with only guests left the next open (double-click, "Open With") finds the lock free and starts a new main window.
- Media keys: the player has no system media session or global hotkeys today; the keyboard's media keys reach the window that has the focus, like every key, so they act on the last active window. Nothing to change. (A future system media session per process: the system routes to the most recent session; note it there.)

## 6. Lifetime and cleanup

- **Start** (`guest::start`, first thing after the command line): sweep leftovers (below), make `<uuid>/`, take `guest.lock`, copy the snapshot, install the profile override. With `--vestra-guest-dir`: wait up to 20 s for that directory's lock (the old process is ending), then use it as it is; if it is gone, start fresh.
- **Exit**: the guest deletes its directory when `player()` returns, after the plugins stopped (`guest::Session`'s drop), unless it is restarting itself (`rendering::RESTARTING`): then the new process adopts the directory.
- **Leftovers** (a crash, a kill): every start, main or guest, sweeps `<app cache>/guests` on a thread: a directory whose `guest.lock` it can lock is dead and is deleted **while the sweep holds that lock** (`guest::probe` never makes a lock file); one without a lock file older than a minute too. A live guest's lock is held, so it is never swept.
- **Starting against a sweep**: a new guest makes its directory and lock file, then takes the lock, waiting up to 5 s while a probe holds it; once locked it checks that the directory and its lock file are still there (a sweep that locked first may have deleted them, taking the unlocked directory for a leftover) and otherwise starts over in a new directory. A restarted guest adopting its directory checks the same, and its profile, after it locked; a swept one makes it start fresh.

## 7. The host action and the menu item

- **`app.openWindow`** `{ paths: string[] }` (1 to 64 absolute paths; `invalid` otherwise, checked on the core's thread; whether each is a file is checked on the guest-start thread, `missing` naming the first that is not): starts `vestra-host --guest [--silent] -- <paths>` and answers `{ pid }`. A new area `capabilities/app_windows.rs`, **built-in only** (`Info::builtin_only`): the ruling makes the playlist row menu the only entry, and starting player processes is a power a third-party plugin should get only with an SDK member the owner approves. Plugins call it as `plugin.app.openWindow(paths)` (SDK 1.10.1, `App.openWindow`, built-in only; `TestHost.openedWindows` records it).
- **Playlist** (0.2.10): "Play in New Window" in the row menu, right after Play. It sends the selected local files (the first 64) in list order through `plugin.app.openWindow`; disabled with the reasons Show in Folder gives ("Links have no local location", "Nothing to act on") when none is a local file. `focus: "panel"` without a key keeps it out of the command palette, so the row menu stays the only entry.

## 8. Title marker

The window title gets the marker in every language (`guest::decorate`, applied where the interface writes `window-title`, so it follows every title change): `{} (Temporary Window)`, zh-Hans `{}（临时窗口）`, and de, es, fr, ja, ko in the host catalog. Before a file plays the title is "Vestra Player (Temporary Window)". That is the only visible difference.

## 9. Not now / later

- **SDK**: done in SDK 1.10.1 (`App.openWindow`, `AppInfo.temporary`, `TestHost.openedWindows` and `app.temporary`, `unavailable` documented, the playlist's command in `BUILTIN_COMMANDS`); the player uses 1.10.1.
- **macOS**: the same code builds; what it needs is a launch that LaunchServices accepts as a second instance of the bundle: `open -n -a <bundle> --args --guest -- <paths>` (or `NSWorkspace.openApplication` with `createsNewApplicationInstance`), so the guest gets its own Dock icon and menu bar. The cache root is `~/Library/Caches/Vestra/guests`. Until then `app.openWindow` answers `unsupported` on macOS and the playlist leaves the item out of its row menu there; enabling it is a follow-up with a Mac.
- Linux: the direct spawn works as on Windows; not tested.

## 10. Tests

- Unit: launch flags; snapshot copy (files, `paletteRecent` gone, cascade); `Store::open_read_only` writes nothing (every file and directory, bytes before and after, with a record and an unknown field set aside, built-ins added and a package disabled in memory); manager refusals in a guest; leases (held lock read, dead lock ignored, an unreadable lease of a live guest keeps everything); the cleanup thread keeps a leased digest for `Work::Only` and `Work::Unreferenced` and removes it once the guest is gone, and `hold_freed` leaves it in place; leftover sweep; the start and adoption races against a sweep (a guest waits for a probe that lets go, and gives up a directory the probing sweep deleted); `app.openWindow` shape checks and the existence check; title decoration.
- Integration, `tests/guest_windows.rs` (offscreen and headless, isolated profiles, ignored by default like the other real-host tests, run by ci:local): (1) a guest run with plugins that write their data, secrets and settings leaves every file of the main profile byte-identical (hash of the whole tree before and after), headless and offscreen (where the guest writes its window and render state); (2) the guest takes no instance lock: a later launch with the main closed becomes the main (it holds the lock); a guest started while the main runs keeps running after the main quit; (3) the guest's directory is gone after a normal exit, and a killed guest's leftover directory is swept by the next start; (4) a package update in the main player while a guest runs keeps the old version's directory until the guest is gone, and the next main start removes it; (5) a development (non-built-in) plugin's `app.openWindow` is refused with `capability`, a built-in's starts a guest.
