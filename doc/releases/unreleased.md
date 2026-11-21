# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## 0.10.3 (host; not released yet)

Material for `v0.10.3.md`. The player program goes to 0.10.3 (the host changed); the SDK goes to 1.10.1 (`AppInfo.temporary`, `App.openWindow` for built-in plugins).

### New

- **Play in New Window.** Right-click files in the playlist and choose Play in New Window: they play in a separate temporary window, as a browser's guest window. Its title says "(Temporary Window)". It starts with your settings (theme, language, keys, rendering, window layout) and your installed plugins, but with empty plugin data: no history, no remembered playlist, plugin options at their defaults. Nothing it changes is kept, and it changes nothing of the main window's: it cannot install, update or remove plugins or save sign-in details, and its temporary folder is deleted when it closes. You can close the main window while a temporary window plays; opening a file afterwards starts a new main window. Windows only for now (Playlist 0.2.10; Store 0.3.2 and Info 0.1.16 skip their automatic checks in a temporary window; Workshop 2.2.46 vendors SDK 1.10.1).
