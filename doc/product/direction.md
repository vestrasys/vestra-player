# Vestra Player: project purpose, product direction and architecture principles

This document records the principles and high-level architecture the project has settled on, as a shared basis for later discussion, design and implementation. It is not a detailed design and not a first-release feature commitment.

The technology stack (Rust host + Slint native UI + TypeScript plugins) is in [Technical decisions](technical-decisions.md); the plugin API is on the [SDK documentation site](https://vestrasys.github.io/vestra-player-sdk/); current progress is in [Status](../status.md).

The code in this document consists of **usage examples** that show how future developers will use the platform. It is not an existing implementation or a frozen API. Names, parameters, package format and communication details can still change during design.

## 1. Why this project exists

The project comes from a real everyday need: an open-source player with a rich, comfortable control experience, a modern interaction model and an extensible architecture.

What matters to us is not only "the video plays", but also:

- The playlist can stay docked in the sidebar, with clear browsing and actions.
- Playback, video, audio, subtitle and other features can be discovered from menus, without first learning configuration files and commands.
- Keyboard shortcuts are plentiful, searchable and configurable, and common actions take few steps.
- Advanced users can control filters, shaders, hardware decoding, rendering, and audio/video output.

The project's value lies in interaction, feature organisation and extensibility. For playback technology we reuse mature infrastructure first, and spend development effort on the experience users actually feel.

Early discussion compared several existing players. Those comparisons are background for the choice of topic. They should not be read as "there is no open-source competitor", and the popularity, versions and feature descriptions recorded there should not be taken as lasting market conclusions.

## 2. Product direction

**An open-source desktop player that uses mpv as its default playback engine, with modern interaction, deep control and a plugin ecosystem.**

mpv provides playback; VS Code is the main reference for the plugin architecture. Obsidian, foobar2000 and others inspire the shape of an extensible product.

### Modern interface alongside advanced capabilities

Modern does not only mean visual style. It also means searchable commands, a clear sidebar, a single place for settings, and advanced controls that expand on demand.

The interface can use progressive disclosure:

```text
Basic playback interface
    ├── Open playlist, subtitle and other panels on demand
    └── Open advanced video and audio controls on demand
```

The default experience should be easy to pick up, and advanced capabilities should also have a discoverable GUI entry. Users should not need to know mpv command names to adjust subtitle delay or configure video processing.

### mpv-first

By default mpv/libmpv does the playback work: demuxing, decoding, A/V sync, subtitles, filters and output. We build our own playback backend interface so that the UI and ordinary plugins do not call mpv directly all over the place.

Filters, shaders, rendering, hardware decoding and similar features are part of the product direction; the actual level of support depends on the backend, platform and device capabilities. Missing capabilities can be addressed through extensions or upstream contributions to mpv.

mpv's processing pipeline is not equivalent to the DirectShow component ecosystem. Using mpv does not mean LAV, madVR or similar components can be loaded directly, and it is not a promise of compatibility with arbitrary internal components of other players. Special native integrations need separate validation.

The first phase focuses on mpv. The backend interface keeps its boundary, but we do not implement several playback engines ahead of time because of it.

## 3. The most important idea: Everything is a plugin

**The product itself is made of plugins.**

The core keeps only the infrastructure needed for the player to run and for plugins to cooperate. User features such as the playlist, subtitle controls, audio/video controls, history, bookmarks and complex settings are provided by plugins wherever possible.

```text
Small core
    ├── Basic playback capability and backend interface
    ├── State, commands and events
    ├── Configuration and persistence infrastructure
    ├── Plugin lifecycle and communication
    └── UI shell and extension points
          ↑
          │ Contribute features through the public API
          │
    Playlist / Subtitles / Advanced video / History / Third-party extensions
```

"Everything" expresses a principle of feature organisation. It does not require every infrastructure module to be a dynamically unloadable plugin. The exact boundary between core and plugins will be settled in the detailed design.

### Official and third-party plugins use the same public API

The official playlist plugin and third-party extensions follow the same boundary in the feature API. Being bundled by default and maintained officially is no reason to bypass the public API.

If an official plugin needs a capability that the public API cannot express, the public API should be improved, instead of reaching into internal objects, raw pointers or hidden interfaces.

This is the test of whether the plugin API is really usable: our own features must also be buildable with it.

### Users choose their feature set

The product can bundle a set of official plugins by default for an out-of-the-box experience. Users can enable, disable or install feature plugins as needed; the exact dependency constraints will be specified in later design.

The quality of a third-party plugin is its author's responsibility, and users choose what to install. The project is responsible for a clear, stable API, lifecycle and failure boundaries. A plugin store is not a precondition for the architecture; it can be considered once the ecosystem creates a real need.

## 4. Key difference from Cameo

In the discussion, Cameo refers to MBSE software of the Cameo Systems Modeler / MagicDraw kind. It is used here to help explain the relationship between plugins and the host; this is not a summary of all its internals.

Plugin development in Cameo is closer to this: import the SDK, use the public application objects and mechanisms such as Action and Configurator, and add features to the application. It also has a public API and extension mechanisms.

We borrow **VS Code's capability API and Extension Host approach**: plugins work through public capabilities, run in a separate process, and do not depend on the host's internal object structure.

| Dimension | How a Cameo-style SDK plugin is understood | Our direction |
|---|---|---|
| What the developer faces | The application's public object model and SDK | Capability API and extension points |
| How features are wired in | Implement interfaces such as Action and Configurator | Declare contributions, register behaviour |
| How UI is added | Manipulate UI-related objects through the SDK | Contribute content to specified menus, sidebar, settings and other places |
| Dependency on the host implementation | Must understand the relevant object and interface system | No need to know the internal window or backend controller structure |
| How the product grows | Add features to the application | Official and third-party plugins together make up the product |

The difference is not decided by Java versus TypeScript, nor only by whether things cross a process boundary. The core of it is the public contract, extension points and dependency boundaries.

### The same feature: add "Pause" to the context menu

Below is a hypothetical style that "walks host objects directly to find the controller". It is **neither Cameo's real API nor ours**:

```ts
const window = app.getMainWindow();
const controller = window.getCurrentPlayer().getMpvController();

window.getContextMenu().addAction("Pause", () => {
  controller.setProperty("pause", true);
});
```

This style makes the plugin know how the window, player, mpv controller and menu objects are organised.

Our style ([plugin API](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html)):

```ts
export default async function activate(plugin: Plugin) {
  plugin.commands.add({
    id: "example.pause",
    path: ["Playback", "Pause"],
    enabled: () => plugin.player.state.can.pause,
    run: () => plugin.player.setPaused(true),
  });
}
```

The plugin only knows player capabilities and the domain path of the menu. How the menu is drawn and how the playback backend executes are the host's job. Callbacks run inside the plugin process; function objects are never passed to Rust.

## 5. Commands and entry points

### One feature, one command

Commands connect features to the places they are triggered from. The same command can appear in the context menu and carry a shortcut, and UI buttons call the same logic, so it does not have to be reimplemented for each entry point.

```text
Context menu ──┐
Shortcut ──────┼──→ Command ──→ Plugin logic ──→ Player capability
UI button ─────┘
```

Menus are organised by user domain (File, Playback, Video, Audio, Subtitles…), not by plugin. Commands from several plugins merge into one menu tree.

### The manifest only describes identity and capabilities

```json
{
  "id": "example.video-tools",
  "version": "0.1.0",
  "apiVersion": "vestra/1",
  "entry": "main.ts",
  "capabilities": ["commands", "playback.control"]
}
```

Commands, windows and panels are declared in code (`commands.add`, `windows.create`, `panels.mount`), so declaration and handling live together. The manifest does not repeat these contributions; it only says who the plugin is and which player capabilities it needs. The host uses it to reject undeclared calls; this is a contract, not a security sandbox.

### Check capabilities before offering features

The backend tells plugins its capabilities through state. For example, when `player.state.can.seek` is `false` (live streams, emulators), the seek command should be disabled rather than fail when called. Filters and other processing have clear value ranges and failure states; plugins use them to give feedback to the user and do not assume a given filter is always available.

## 6. How TypeScript, Rust and Slint relate

**Rust implements the host, TypeScript implements plugin logic, Slint describes the UI, and the three cooperate through explicit protocols.**

| Layer | Main responsibility |
|---|---|
| Plugin (TypeScript, one Node process per plugin) | Feature behaviour: commands, state, responses to user actions |
| Plugin UI (Slint) | Layout and controls of tool windows and bottom-bar/sidebar panels |
| SDK | The API plugins see; keeps protocol details (revision numbers, source generations, result queries) internal |
| Rust host | Authoritative playback state, commands and menus, UI composition, processing chain, plugin lifecycle |
| libmpv (behind a safe wrapper) | Actual media playback and processing |

```text
Plugin logic (TS) ── SDK ── JSONL pipe ──┐
Plugin UI (.slint, composed by host)     │
                                     Rust host
                         State, commands, UI composition, plugin management
                                       │
                                 libmpv (safe wrapper)
```

Data, errors and events in the cross-language protocol must all be explicit; TypeScript types are no substitute for input validation on the Rust side.

### How one pause happens

The plugin writes:

```ts
await plugin.player.setPaused(true);
```

Call path:

```text
SDK sends a request (conditioned on the playback state the plugin sees)
    ↓
Host checks the condition and capability, passes it to the playback backend
    ↓
libmpv pauses, the backend reports the actual state
    ↓
Host updates the authoritative state and publishes an event
    ↓
SDK updates player.state; UI and plugins learn that playback is paused
```

The UI must not assume the underlying operation is done just because a button was clicked.

### Plugin logic and plugin UI are separate

Plugin logic runs in a Node process; the UI is compiled by the host with Slint. Only data passes between them: the plugin sends property values to the UI, and the UI sends user actions to the plugin as callbacks. Functions and component objects are not passed.

## 7. Safety principles for Rust and native extensions

### Our own Rust code uses only Safe Rust

All first-party Rust crates should forbid `unsafe`:

```rust
#![forbid(unsafe_code)]
```

This is the final principle. It replaces the earlier suggestion of "allowing a little unsafe in our own mpv-backend or platform modules".

We call libmpv, the windowing system and other native capabilities through safe wrappers provided by dependencies. Those dependencies may still use `unsafe` or C/C++ internally; the principle constrains this project's own code and does not claim the final program contains no unsafe native code.

The specific wrapper libraries still need validation, especially for embedded rendering, window handles, threading and lifecycle interfaces. If an implementation route requires writing `unsafe` ourselves, we first look for a dependency or a process-boundary approach that fits the principle, and do not relax the principle silently.

Safe Rust does not replace verification of logic, concurrency and resource management. A workflow where AI does most of the coding and the maintainer reviews the code still needs compiler checks and appropriate tests.

### Native extensions use a separate process and IPC

An early example proposed dynamically loading native plugins and calling their ABI directly. That is no longer the chosen direction.

Special native integrations use a separate extension process that talks to the core over IPC:

```text
Rust Core
    ├── IPC ── Extension Host ── JS plugins
    └── IPC ── Native Extension Process
                    └── Special native capability
```

This avoids loading arbitrary DLLs and calling raw ABIs from our own code. A separate process also provides a failure-isolation boundary, but it is not automatically a full permission sandbox.

Connecting the default libmpv backend through a dependency's safe wrapper does not mean all playback code has to move out of process. IPC for third-party native extensions also does not automatically solve video frame transfer, GPU resource sharing or renderer replacement; those integrations need dedicated design and validation.

Third-party JS plugin logic should also be separated from the core. A failure of the Extension Host should not bring the core down directly; how basic playback continues when a plugin fails, and which UI features become temporarily unavailable, must be worked out in the failure-recovery design.

## 8. Settled principles and open design space

Settled and implemented:

- The product targets real playback workflows, with modern interaction and deep control side by side.
- mpv-first: reuse mature playback technology and build our own player capability API.
- Everything is a plugin: a small host plus 10 product plugins make up the product.
- Official and third-party features use the same public plugin API, with no backdoors into internal objects.
- Rust host, Slint native UI, TypeScript plugins, cooperating through explicit protocols.
- First-party Rust forbids `unsafe`; native boundaries rely on safe wrappers or isolated processes.
- The plugin API evolves driven by the needs of real plugins ([community workflow](../../agentic-workflow.md)).

Still to design and validate:

- Launch platforms and release format.
- Plugin failure recovery, state persistence, plugin management UI.
- API versioning policy and multi-version compatibility.
- Process and data channels for third-party native extensions.
