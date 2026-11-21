# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## 0.10.2 (host; not released yet)

Material for `v0.10.2.md`. The player program goes to 0.10.2 (the host changed); the SDK stays 1.10.0.

### Improvements

- **A busy window no longer holds up plugins and playback control.** The player's core (plugin requests, triggers, plugin start and stop, playback control) now runs on its own thread instead of inside the window's event loop. While a window is being built, dragged or resized, or waits for the graphics card, plugins keep getting answers, timed triggers fire on time, idle plugins are put to standby on time, and Play/Pause still reaches playback. Measured with the window held for 4 s: 99% of plugin requests were answered within 35 ms (before: over 4 s).
- **Faster answers to plugins.** The core now handles each request when it arrives instead of on a 16 ms tick: most requests that change the interface are answered within about a millisecond (a panel property change: 0–1 ms, before 30 ms; a settings write: 2–3 ms, before about 31 ms). The window's own frame rate is unchanged.
- **The window keeps drawing while it is moved or resized on Windows** (the player's tick keeps running inside the system's move/size loop).
- **Fullscreen and file dialogs:** with the player fullscreen and always on top, file dialogs (open, save, folder, locate) no longer hide behind it: they belong to the window that asked, a key or click the open dialog blocks brings the dialog forward, and the player is not on top while a dialog is open. "Always on top" now defaults to Never (as in mpv and VLC).
- **Store 0.3.1:** a card grid, a Not installed tab, a filter for built-in plugins, and actions on each card.

### For plugin authors

- No SDK or wire change: SDK 1.10.0 as in 0.10.0. Who must change: nobody.
- Replies to requests a plugin sends at the same time (without waiting between them) were never promised in any order, and now arrive in whatever order they are ready: a reply that waits for the playback worker (`source.state`, for example) may come before or after the reply to another request sent with it. Each reply is matched to its request by id, as always.
