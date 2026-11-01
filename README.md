<p align="center"><img src=".github/assets/logo.png" width="96" height="96" alt="Vestra Player logo"></p>
<h1 align="center">Vestra Player</h1>
<p align="center"><b>A fast, keyboard-friendly desktop video player where every feature is a plugin.</b></p>
<p align="center">
  <a href="https://github.com/vestrasys/vestra-player/actions/workflows/main.yml"><img src="https://github.com/vestrasys/vestra-player/actions/workflows/main.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/vestrasys/vestra-player/releases"><img src="https://img.shields.io/badge/release-v0.3.0-blue" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Windows-lightgrey" alt="Platforms: macOS and Windows">
  <a href="https://vestrasys.github.io/vestra-player-sdk/"><img src="https://img.shields.io/badge/plugin%20API-vestra%2F1-6f42c1" alt="Plugin API vestra/1"></a>
  <a href="https://www.npmjs.com/package/@vestra-player/sdk"><img src="https://img.shields.io/npm/v/@vestra-player/sdk?label=SDK&logo=npm&color=cb3837" alt="SDK on npm"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green" alt="License: GPL-3.0"></a>
</p>
<p align="center">
  <a href="https://github.com/vestrasys/vestra-player/releases">Releases</a> ·
  <a href="https://vestrasys.github.io/vestra-player-sdk/">Plugin SDK docs</a> ·
  <a href="https://github.com/vestrasys/vestra-player-plugins">Plugins</a> ·
  <a href="doc/README.md">Design docs</a> ·
  <a href="agentic-workflow.md">Agentic Workflow</a>
</p>

Vestra Player is an open-source video player for macOS and Windows. It aims for rich, discoverable controls (menus, shortcuts, a docked playlist, deep video and audio settings) on a modern, extensible architecture. Playback is handled by [libmpv](https://mpv.io); the interface is native [Slint](https://slint.dev); and everything you use (playback bar, playlist, video and audio adjustments, subtitles, screenshots and export, history, hotkeys, the plugin store) is a TypeScript plugin, and the store adds more, such as the NES emulator VestraNES.

## Core ideas

- **Everything is a plugin.** The Rust host only provides shared infrastructure: playback, UI composition, processing chains, plugin lifecycle. Product features live in plugins.
- **One public API for everyone.** Built-in and third-party plugins use the same API, `vestra/1`, through the [`@vestra-player/sdk`](https://www.npmjs.com/package/@vestra-player/sdk) package. If a built-in plugin needs something, the public API grows; there are no private back doors.
- **Plugins out of process.** Plugins run in Node.js, outside the host: the built-ins share one process, every other plugin has its own. The host checks every call against the capabilities the plugin declares, and stopping a plugin releases everything it owns.
- **Native UI.** Plugin windows and panels are Slint components, compiled and composed by the host with a shared theme and widget set.
- **Signed plugin store.** Plugins install from a signed index (Ed25519), built from source in [vestra-player-plugins](https://github.com/vestrasys/vestra-player-plugins).
- **Cross-platform.** All OS-specific code sits in a thin platform layer (`src/host/src/platform/`), one module per system; everything else is shared.

## Architecture

```text
┌──────────────────────── Host process (Rust) ────────────────────────┐
│  Playback (libmpv)        Slint UI composition     Package store    │
│  Processing chains        Plugin process manager   Platform layer   │
└───────────▲─────────────────────────▲───────────────────────▲───────┘
            │ JSON-RPC, stdin/stdout  │ protocol vestra/1     │
            │ one message per line    │                       │
┌───────────┴──────────┐     ┌────────┴───────┐      ┌────────┴───────┐
│ Shared runner (Node) │     │ Node process   │      │ Node process   │
│ all built-in plugins │     │ store plugin   │ ...  │ store plugin   │
│ + SDK + .slint       │     │ + SDK + .slint │      │ + SDK + .slint │
└──────────────────────┘     └────────────────┘      └────────────────┘
```

Plugins describe their windows and panels in `.slint` files; the host compiles them, places them in the main window or a tool window, and forwards their callbacks back to the plugin. Plugins send values and row changes, not pixels, so a 20,000-row playlist is a native `ListView` over rows the host holds ([large lists](doc/architecture/large-lists.md)).

Built-in plugins share one Node process, the shared runner; every other plugin gets its own, apart from the player and from each other. A Node process costs about 50 MB, one more plugin in the shared runner about 1 MB: on Windows, the 14 built-ins of a headless start measured 704 MB as separate processes and 61 MB shared ([measurements](doc/process/rounds/2026-10-shared-runner.md#163-measurements)).

[Lazy activation](doc/process/rounds/2026-10-lazy-activation.md), behind `VESTRA_LAZY=1` until its hardware checks pass, bounds how many plugins run at once. As with a Chrome MV3 service worker, a plugin holding nothing live (a mounted panel such as the playback bar, an open window, a filter chain, a playback listener) is stopped after a minute idle; the host keeps its menus, keys and file types, the next use starts it again in tens of milliseconds, and session storage carries state across the stop. An installed plugin nobody uses costs no process after its first run.

Faults stay with their plugin. The host checks each call against the declared capabilities and caps messages at 1 MiB; a stopped plugin loses everything it owned and has 300 ms to exit; a crashed plugin process takes down only its plugin. In the shared runner, an exception traced to one plugin moves that plugin to a process of its own; a crash or hang of the runner (2 s heartbeat) restarts the built-ins in about 1.3 s while playback continues, and a third restart within a minute splits them up until the player quits. More in the [architecture overview](doc/architecture/README.md#2-loading-and-stopping-plugins).

## Repository layout

| Path | Contents |
| --- | --- |
| `src/host/` | The Rust host: playback, UI composition, plugin and package management, platform layer |
| `src/runtime/` | The Node.js runners: one shared by the built-in plugins, one per other plugin |
| `src/plugins/` | The built-in product plugins |
| `src/packaging/` | Installer builds for macOS and Windows |
| `doc/` | Design documents, architecture and process notes ([index](doc/README.md)) |

The SDK lives in [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk); the YouTube and Bilibili plugins and the store index live in [vestra-player-plugins](https://github.com/vestrasys/vestra-player-plugins).

## License

Vestra Player is licensed under [GPL-3.0-or-later](LICENSE). The SDK and the plugins repository are Apache-2.0.
