# Lazy activation: recording and replay, residency, and the capability refactor

API maintainer design, 2026-10-26. A plan, not code: nothing in the repositories changed. Paths are relative to `src/host/src/` for the host, `C:\vestra\sdk\src\` for the SDK, `src/plugins/` for the built-ins. Spike numbers come from `C:\vestra\spike-workers\out\*.json`.

## 0. Decisions in ten lines

1. Keep one Node process per plugin as the isolation model. The lever is **not starting** processes, not making them smaller (§1).
2. **A: record and replay.** The host records a plugin's user-triggered registrations (commands with keys and state, settings pages, resolvers, drop providers) from its own registries, and at later starts registers them under an owner with no process. The first use of such an entry starts the plugin, which re-registers normally; live wins over the recording (§2).
3. **B1, not B2.** A plugin is resident while the host holds a resource for it that cannot be replayed without the process (a panel, an OSD panel, a filter chain, a stream, a sequence provider, an open window, a live event listener). This is not a list: it is whether `Capability::record` can express the resource (§3.2). B2 breaks on filter chains, producers, the bottom bar and every `onChange` that reacts to a transition (§3.3).
4. One SDK addition makes B1 work: the SDK marks its own start-up state subscriptions as `mirror` so that only a plugin's real listeners count (§3.1). Without it, 15 of 17 built-ins would be resident.
5. Classification is **observed, not declared**: at `runtime.ready` the host notes whether the plugin holds a live resource; the recording stores it; the next start either starts the plugin at launch or replays it. Self-healing: a woken plugin that registers a live resource stays up (§3.4). No manifest field is needed; `"activation": "startup"` is deferred until a plugin needs it.
6. Expected built-in result: 8–9 of 17 become lazy (capture, hotkeys, info, nes, preferences, store, subtitle-online, workshop; probably marks), about 500 MB and 3 s of start CPU less; the real win is third-party scale: 2000 trivial plugins cost zero processes after their first run (§3.5, §5.2).
7. **C: `capabilities/` with a `Capability` trait** (name, label, requires, owned methods, `handle`, `record`, `revoke`, `rekey`). The dispatch table, the permission check, `CAPABILITIES`, the SDK's `CAPABILITY_INFO` and the store's labels are generated from `capabilities::ALL` through one committed `capabilities.json` that a host test keeps current (§4).
8. **Order: GPU branch lands → C (pure refactor, two steps) → A+B.** The GPU branch touches `main.rs` in five small hunks; C's second step moves `main.rs`, so it waits for that merge (§5.1).
9. Acceptance is measured the coordinator's way (headless start, node process count, working set, start CPU) for the 17 built-ins and for a synthetic library of 2000 trivial plugins; a cold menu click to its dialog must take under 600 ms p95 for a trivial plugin (§5.2).
10. Six behaviour changes need the user's approval before work starts (§5.4); everything else is an addition.

## 1. The problem and the numbers

Today every installed, enabled plugin starts at launch: `package_manager.rs:450-495` (`start_installed`) calls `load_package` (`:150-171`), which spawns Node (`plugins.rs:105-152`) with a 15 s activation deadline (`package_manager.rs:162`). Development loads do the same with 120 s (`main.rs:5265`). Nothing stops a plugin except disable, reload, uninstall, upgrade, a fault, or quit (`main.rs:628-732`; SDK `docs/concepts/plugins.md:98-106`).

User's measurement (installed build, headless, 15 built-ins): about 64 MB working set and ~360 ms start CPU per plugin process, ~1 GB in all. With 17 built-ins now, ~1.1 GB and ~6 s of CPU before the first frame.

Spike (`C:\vestra\spike-workers\`, harness `harness.mjs`, orchestrator `run-one.ps1`; a 4 GB available-memory floor aborted the larger runs):

| Run | Per plugin | Start | Notes |
| --- | --- | --- | --- |
| proc / tiny / N=1 | ~44 MB WS (84.1 total − 39.6 harness) | first ready 137 ms | idle CPU 0 |
| proc / tiny / N=15 | ~19 MB by free-memory delta (4277→3997 MB) | 607 ms for 15, sequential | OS snapshot missed: orchestrator aborted |
| proc / tiny / N=100, 500, 2000 | — | — | not run: below the memory floor before start |
| proc / heavy (store plugin) / N=1 | ~47 MB WS | first ready 176 ms | |
| worker / tiny / N=1 | ~15 MB RSS | 60 ms, 47 ms CPU | |
| worker / tiny / N=15 | ~14 MB RSS (254.5 total) | 163 ms for 15, 1.2 s CPU | |
| worker / tiny / N=100 | ~14 MB RSS (1435 total) | 772 ms, 8.4 s CPU | aborted after ready |
| worker / tiny / N=500, 2000 | — | 114 started, then aborted | free memory under 3000 MB |
| worker / heavy / N=15 | ~16 MB RSS | 189 ms, 1.6 s CPU | |

Reading: workers save 25–30% memory and start 3–4× faster, but 2000 of either model is 28–38 GB; this machine could not start 115 tiny workers. The model is not the lever. A plugin that only adds a menu item must cost **no process** until the item is used. That is lazy activation; the isolation model stays as it is (the user has not asked to change it).

## 2. A: recording and replay

### 2.1 What is recorded and where it comes from

The host does not transcribe RPC. It records from its own registries, which already hold everything needed to show the entry and to decide what a trigger means:

| Registration | Registry | Replayable without the process | Trigger that wakes the plugin |
| --- | --- | --- | --- |
| `commands.register` + last `commands.update` | `commands::Registry` (`commands.rs:274`, entries `:154-164` with declaration `:32-60` and state `:145-150`); keys in `bindings::Table` (`bindings.rs:609`, `declare` `:660`) | yes: menus and keys are built from `commands.snapshot()` (`main.rs:2310`, `:2532`) and the table | menu invoke (`main.rs:2882`), key (`:948-966`) |
| `settings.register` | `settings_window::Page` (`settings_window.rs:40-55`: file, component, global, initial properties) | yes: the Options window compiles pages from `Page` alone (`:166`) | the page is shown (its callbacks need the plugin), a property write |
| `sourceResolvers.register` | backend `source_resolvers::Registry` (`source_resolvers.rs:78`, `register` `:170`, `find` `:201`) | yes | a matching open: `source.resolveRequest` (`source_backend.rs:1297-1312`, 30 s deadline) |
| `drops.provide` | `drops::Drops` providers (`drops.rs:162-190`) | yes | a drop: `drops.drop` (`drops.rs:279`) |
| `events.subscribe` with a real listener | `Plugin.subscriptions` (`plugins.rs:94`, `main.rs:4403-4425`) | **no** (the push needs a receiver) | — resident |
| `ui.mount`, `osd.panel`, `ui.open` views, `timeline.set`, `processing.*`, `producer.*`, `sequence.set`, `power.preventSleep`, `window.setAlwaysOnTop` / `setCursorAutoHide` / PiP, `player.setStepSeek`, `video.view/render` | the resources `disable()` revokes at `main.rs:636-723` and `backend.rs:232-269` | **no** | — resident while held |

The recording is produced by `Capability::record(host, owner)` (§4.2) for each capability, as a list of `{ method, params }` in the form the plugin sent them (pre-localization: the host re-runs `plugin.texts()` and `localize_declared` at replay, `main.rs:4287-4331`, so the interface language is not part of the key). For commands the recorded `state` is the state at recording time (`enabled`, `checked`, `reason`; `busy` is recorded as `false`).

**Storage**: one file per installed instance, `<app data>/activation/<instance GUID>.json` (the GUID of `package_store::Instance`, `package_store.rs:60-66`), outside the plugin's data directory so the plugin cannot read or edit it. Header:

```json
{ "schema": 1, "player": "0.5.1", "sdk": "1.5.20", "digest": "<StoredPackage.digest>",
  "version": "<manifest.version>", "residentAtReady": false, "recordedAt": 1761460000000,
  "calls": [ { "method": "commands.register", "params": { "id": "hello", "revision": 1, "declaration": {...}, "state": {...} } }, ... ] }
```

`digest` is `StoredPackage.digest` (`package_store.rs:115`): an upgrade or reinstall changes it. `builtWith` does not exist in the manifest today (`manifest.rs:31-60`; §1.6 item 4 lists it as open), so the SDK version the player ships is the second key: a new player or SDK release invalidates every recording once, which is the safe side.

**When it is written**: at park (§2.3) and at quit for every lazily classified plugin, from the registries; and at quit for resident plugins too, with `residentAtReady: true` and no calls, which is what makes classification persistent (§3.4).

### 2.2 Replay at start and wake

`start_installed` (`package_manager.rs:450`) gains one branch: if a valid recording exists and says `residentAtReady: false`, the host allocates an owner generation, registers each recorded call through the normal dispatch with a `Replay` marker (no process, no activation book entry), and files the owner as **parked**: `Host.parked: BTreeMap<u64, Parked { id, instance, directory, manifest, recording, woken_by: Vec<Pending> }>`. The owner is not in `Host.plugins`, so `broadcast` (`main.rs:743-760`) and `handle(Event::Request)` (`:762-765`) ignore it by construction.

**Wake on send.** The one choke point is `Host::send(owner, message)` (`main.rs:733-741`): a message for a parked owner (a `commands.execute` from `execute_command` `:2882-2893`, a `source.resolveRequest` relayed by `backend.rs:213`, a `drops.drop`, a settings page's `ui.callback`) is queued in `Parked.woken_by` and the plugin is started with `load_package` under a **new** owner generation; the registries are re-keyed from the parked generation to the new one (`Capability::rekey`, §4.2; the backend gets `owner.rekey { from, to }` for its resolver registry). The old generation has no plugin, so a stale event from it is dropped by the existing guard at `main.rs:762-765`. One generation per process is what the whole host assumes (`commands.rs:549-551`); this keeps it.

Then the plugin registers normally. Each live registration meets a recorded entry under the same owner: `commands::Registry::register` returns `Conflict` for an existing key today (`commands.rs:466-470`); `settings.register` refuses a duplicate page (`settings_window.rs:335`); `source_resolvers::Registry::register` likewise (`source_resolvers.rs:183`). Each gets a `recorded: bool` flag on its entry and **replaces** a recorded entry instead of refusing. Replacement rules:

- same local id, same declaration: mark live, take the live state (the plugin's first `commands.register` carries it, SDK `plugin/commands.ts:118-135`);
- same id, different declaration: replace, rebuild the menu (revision bump as today);
- recorded entry not re-registered by `runtime.ready`: removed; a queued trigger for it is answered with an OSD "<title> is no longer available" (`show_osd`, `main.rs:3116`);
- extra live entries: added as today.

At `runtime.ready` the queued triggers are forwarded in order, each re-checked against the live state through `resolve_execution` (`commands.rs:620-647`); a command that is now disabled shows its reason the way a key does (`main.rs:958`). A settings page's compile is skipped when the recorded page's Slint file digest equals the live one (the compile is 60–160 ms per page, `settings_window.rs:96-99`).

**Mismatch of the recording's key** (digest, player, sdk, schema): the recording is ignored, the plugin starts at launch as today, and is recorded again at its first park or at quit. A corrupt file is the same case.

### 2.3 Idle and park

A **lazy** plugin (one whose `residentAtReady` was false when it last ran, or that is running for the first time) is parked when, for `VESTRA_IDLE_MS` (default 60 000; tests set it low):

- it holds no live resource (§3.2): no view in `Host.views` for the owner, no mount, no OSD panel, no timeline layer, no subscription with a listener, no backend resource (the backend answers `owner.resources { owner }` with its jobs, sources, processing, view and render holdings, from the same tables `owner.disabled` cleans at `backend.rs:232-269`), no power lease, no on-top or cursor or PiP write, no sequence;
- no request from it and no reply or dialog for it is in flight (`dialog_busy`, pending `settings.checking`, pending resolutions, `activations` not `Ready`);
- no command of it is `busy`;
- `packages` is not operating on it (`package_busy`, `package_manager.rs:496`).

Park = write the recording, then stop the process the way `disable()` does **minus** the registry removals: the commands, bindings, pages, providers and resolvers stay under the parked generation with `recorded: true`; the backend gets `owner.parked` (cleans as `owner.disabled` but keeps resolver entries). The stop itself sends `runtime.stop` and waits as today: 300 ms then kill (`plugins.rs:235-254`). The runner gives its own cleanup 5 s (`src/runtime/runner.ts:21`), which the host never honours; parking makes this routine rather than rare, so P1 raises the host's wait for a park to 2 s (an implementation choice inside the existing contract: "a process that has not exited after 300 ms is killed" becomes "after 2 s", listed in §5.4 for visibility).

Display while parked: menus show the recorded `enabled`/`checked`/`reason`; keys resolve against the same entries (`bindings::Table::set_disabled`, `bindings.rs:889`). This is the conservative choice: the last state the plugin itself asserted. A plugin whose menu state must follow playback (checked track, "enabled while playing") always has a listener that computes it, so it is resident and never shows a stale state (§3.3).

### 2.4 Special cases

- **Fresh install from the store**: `packages.install` starts the new instance live as today (`package_manager.rs:1642` `install_instance` and the operation flow); no recording exists, so the first run is live; it is recorded at its first park or at quit. Same after an upgrade (new digest) and after `packages.setEnabled(true)` (`:790`), which starts live through `start_stopped` (`:847`).
- **Disabled plugins**: never replayed; `setEnabled(false)` on a parked owner removes its recorded entries (the registry half of `disable()`), keeps the file. Uninstall removes the file.
- **Development loads** (`--plugin`, `Host::load` `main.rs:573`): always live, never parked, never recorded; `packages.list` keeps `origin: "development"`.
- **`packages.reload`** (`package_manager.rs:882-918`): a parked plugin has no process, so `reload` takes the "stopped" branch and starts it live; the recording is discarded first so a changed package is observed fresh.
- **Headless** (`--headless`, `launch.rs:52`; loop `main.rs:5269`): identical rules; commands register headless, mounts are refused anyway (`main.rs:4057`). `VESTRA_LAZY=0` forces every plugin live for tooling and for the old tests that count on it. The acceptance runs are headless.
- **First run of many plugins** (a library of 2000, or a fresh profile with 17 built-ins): first runs are throttled to 8 concurrent processes beyond the first 8; a lazily classified plugin is parked right after `runtime.ready` (idle timeout 0 for a first run), so the pass is a background index that never holds more than 8 processes. The window is interactive from the start; menus fill in as plugins are recorded.
- **Built-ins** cannot be disabled (`package_manager.rs:806-816`) but can be parked; the built-in mount gate (`builtins_starting`, `:580-600`) only looks at running built-ins, and every built-in with `ui.mount` is resident (§3.5), so slot claims are unchanged.

## 3. B: residency without a table

### 3.1 The fact that shapes the rule

The SDK subscribes before `activate` runs: `createPlugin` calls `playerOf(runtime).init()`, `windowOf(runtime).init()` and `timelineOf(runtime).init()` (`plugin/index.ts:44-48`), and the `PlayerService` constructor subscribes to `source.state` unconditionally (`plugin/player.ts:152`), as `window.ts:49` does for `window.state` and `timeline.ts:40` for `timeline.changed`; `subtitles.ts:51,58` and `appearance.ts:38` do the same when first touched. `Runtime.on` sends one `events.subscribe` per name when the manifest has `events` (`plugin/runtime.ts:214-245`). So every plugin with `events` (15 of 17 built-ins) subscribes to three events whether or not it listens; these keep `plugin.player.state` current (`types.ts:3811`), nothing more. The host cannot tell a mirror from a listener.

**Addition (SDK patch, wire-compatible)**: `events.subscribe { event, mirror: true }` for the SDK's service-level subscriptions; when a plugin adds its first listener for that event (`Runtime.on` with a plugin listener, `player.onChange`, `subtitles.onChange`, `window.onChange`, `timeline.onChange`, `appearance.onChange`, `PanelSlot.onChange`, `bindings.onChange`, `packages.onChange`), the SDK sends `events.subscribe { event }` again (the host's subscription set is idempotent, `main.rs:4421`); when the last listener goes, `events.unsubscribe` then `subscribe { mirror: true }`. The host keeps `subscriptions: BTreeMap<String, bool /* active */>`. An old host ignores the field (`serde_json::Value` params); plugins are unaffected (they never bundle the SDK, §1.6). A mirror subscription is not a held resource: on wake the SDK re-reads state at init (`player.ts:162-164`) and drops anything older than what it has (`:166-168`), so a parked plugin's `state` is current the moment it runs again.

### 3.2 B1, derived from push direction, stated as a property of the code

A **live resource** is anything the host holds for an owner that `Capability::record` cannot express as a replayable call: the host must push to the plugin (an active subscription, a panel's callbacks and `drops.listen`, an open window's callbacks, a resolve in flight, a sequence step, a producer's input and acks), or the resource is the plugin's running computation (a filter chain, a stream, a timeline layer the plugin updates, an OSD panel, a power lease, on-top/cursor/PiP writes, step-seek). It is the list of things `disable()` revokes at `main.rs:636-723` and `backend.rs:232-269` minus the five replayable registries of §2.1, and the code makes it so: each capability implements `record` and returns `Recorded::Live(reason)` for whatever it cannot replay (§4.2). Adding a capability later means answering that question in its own file; nothing central is edited.

**Rule**: a plugin is resident while it holds a live resource. Parkable otherwise (§2.3). `residentAtReady` (§3.4) is this rule evaluated once at `runtime.ready`.

### 3.3 B2, everything lazy: where it breaks

Analysed on the current code, with the SDK's event semantics as found: each `source.state` event carries the full state and older sequences are dropped (`player.ts:166-168`), so a late subscriber does see the latest state; but `onChange` fires only when the state changes after the listener exists (`player.ts:175-181`, `accept` notifies watchers only on a change). "Latest state" holds for `state`; it does **not** hold for transitions.

- **Processing chains**: the chain is applied in mpv under the owner and withdrawn by `owner.disabled` (`backend.rs:241`, `processing.disable`). Stopping `vestra.video` or `vestra.audio` while a chain is applied removes the picture adjustment or the equalizer. Keeping the chain alive without an owner needs an "orphaned resource" concept in `processing.rs`, `effects.rs` and `processing_operations.rs`, and the plugin's `revision`/`expected_generation` protocol (`processing.rs:18-30`) would then meet a chain it did not apply on wake. Not cleaner than B1.
- **Producers**: a stream is a socket and a thread per owner (`producer.rs`); the plugin feeds it. There is nothing to keep without the process.
- **Subtitles**: `vestra.subtitles` reloads external files when they change and keeps track menus checked (`subtitles.onChange`); the 26 commands' states follow subtitle state. Without the process the menu is stale and the auto-reload is lost.
- **Bottom bar and sidebar**: a mount's bindings are pushed through `ui.setMount` and its callbacks come back through `ui.callback` (`main.rs:1034`); "keep showing the last values" would freeze the seek bar and swallow every click, or wake the plugin on every click and every `source.state` tick (~10/s while playing, `plugins.rs:46-47`), which is worse than resident.
- **Transitions**: `vestra.window` resizes on playback start, `vestra.history` records the open and offers resume, `vestra.marks` loads bookmarks per file. Waking "on the next state" would miss exactly the event they react to.

So B2 is not viable alone, and the "hybrid" that fixes it is B1. B1 is also the only version with zero author burden and no table: it is whatever the plugin actually registers.

### 3.4 Observed classification, self-healing, and the escape hatch

- At `runtime.ready` the host evaluates §3.2 and writes `residentAtReady` into the recording at the next park or at quit. Next start: `true` → start at launch (as today); `false` → replay and park.
- A parked plugin that is woken and then mounts, opens a window, subscribes with a listener or applies a chain holds a live resource and is not parked while it does; if at its (new) `runtime.ready` it held one, `residentAtReady` becomes `true` and it starts at launch from then on. A resident plugin whose update stops holding live resources at ready becomes lazy at the next start. Nobody maintains anything.
- `"activation": "startup"` in the manifest: accepted as an optional field by `manifest.rs` and the SDK manifest type, honoured as "never park". **Deferred**: no built-in needs it (§3.5), and adding it now invites authors to use it instead of trusting the rule. Added when a real plugin shows a need (a plugin whose correctness depends on running before any trigger and that holds no live resource, which is rare by construction).

### 3.5 The 17 built-ins under the rule

From the manifests and a grep of each plugin's non-test sources for registrations and listeners (counts are call sites; whether a listener is added at activation or inside a window is marked "verify", settled in P1 by the recording itself):

| Plugin | Live resources at activation | Class | Note |
| --- | --- | --- | --- |
| playback | bottom bar mount, `player.onChange`, `bindings.onChange`, `preventSleep` | resident | |
| playlist | sidebar mount, `provideSequence`, `player.onChange` ×2, `onDrop` | resident | |
| visualizer | backdrop mount, `backdrop.onChange`, `player.onChange` | resident | |
| video | `player.onChange` ×2, `video.filter/view/render` | resident | 33 commands follow playback |
| audio | `player.onChange` ×4, `audio.filter` | resident | |
| subtitles | `subtitles.onChange`; 26 commands with state | resident | |
| window | `window.onChange` ×5, `player.onChange`, on-top/cursor writes | resident | resizes on playback start |
| history | `player.onChange` | resident | records opens, resume |
| marks | `timeline.layer`, `player.onChange` | resident (verify) | the layer could become replayable later; not in this round |
| info | `player.onChange`, `packages.onChange` ×2, `osd.panel` | lazy (verify) | overlay and About window are on demand; if the listener is at activation it stays resident |
| hotkeys | `bindings.onChange` ×6, 2 settings pages | lazy (verify) | listeners are expected inside the pages/window |
| nes | resolver, `drops.provide`, `player.onChange` inside the stream | lazy | resident while a ROM plays (producer) |
| capture | 5 windows, 1 command | lazy | |
| subtitle-online | 4 windows, 2 commands, `player.onChange` (verify) | lazy | |
| preferences | 4 commands, 2 settings pages | lazy | |
| store | 1 command, page, `drops.provide`, `packages.onChange` (verify) | lazy | its revocation check at activation moves to first use: §5.4 item 2 |
| workshop | 4 commands, page, 3 windows | lazy | |

Expected: 8 lazy (9 with marks), 8–9 resident. At 64 MB and 360 ms each: **about 510 MB and 2.9 s of start CPU saved** on the built-ins, 17 → 8–9 node processes at start. The built-ins are the floor, because half of them are the player's playback features; third-party plugins are where the rule pays: a plugin that adds a menu item, a resolver, a settings page or a drop target costs nothing until used.

## 4. C: directories and a `Capability` trait

### 4.1 Layout

`src/host/src/` has 75 flat files (52 518 lines); `main.rs` is 5 697 lines holding `Host`, the event loop, `dispatch` (`:3572-4535`) and `required_capability` (`:5482-5557`). Target (file moves are `git mv`; the crate stays one binary; `platform/` stays):

| Directory | Files | Note |
| --- | --- | --- |
| `main.rs` | argument parsing, `Host` construction, the two event loops | target under 800 lines |
| `host/` | `mod.rs` (`Host` struct), `events.rs` (`Event`), `handle.rs` (`Host::handle`), `lifecycle.rs` (`load`, `load_package`, `disable`, park, wake, `activation_tick`), `dispatch.rs` (capability lookup, text resolution, `answer`/`respond`), `recording.rs` (A), `package_activation.rs` | the `Host` methods that today sit in `impl Host` blocks of other files keep doing so (`settings_window.rs:301`, `package_manager.rs:145`) |
| `capabilities/` | `mod.rs` (trait, `ALL`, lookup, generated `CAPABILITIES`, the JSON test), one file per manifest capability: `producer_av.rs`, `playback_read.rs`, `playback_control.rs`, `source_resolve.rs`, `events.rs`, `commands.rs` (uses `commands::Registry` and `bindings::Table`), `ui_window.rs`, `ui_mount.rs`, `ui_dialog.rs`, `plugins_manage.rs`, `processing_video.rs`, `processing_audio.rs`, `subtitles_read.rs`, `subtitles_control.rs`, `media_read.rs`, `media_control.rs`, `ui_osd.rs`, `clipboard.rs`, `window_control.rs`, `bindings_manage.rs`, `timeline_marks.rs`, `ui_drop.rs`, `ui_settings.rs`, `ui_theme.rs`, `secrets.rs`; plus `uncapable.rs` for methods that need none (`packages.list`, `window.state`, `system.*`, `app.system`, `ui.markdownScan`, `events.subscribe` of `packages.changed`) | the arms of `dispatch` move into `handle` of their capability; the registries they use stay where they are |
| `packages/` | `package_manager.rs`, `package_store.rs`, `package_stage.rs`, `package_archive.rs`, `manifest.rs`, `secret_index.rs` | `plugins_manage.rs` is the RPC face; install/stage/store logic stays here |
| `playback/` | `backend.rs`, `source_backend.rs`, `session.rs`, `media_worker.rs`, `media_tracks.rs`, `media_jobs.rs`, `processing.rs`, `processing_operations.rs`, `effects.rs`, `producer*.rs`, `offline_*.rs`, `native_subtitle.rs`, `subtitle_*.rs`, `source_geometry.rs`, `source_lease.rs`, `source_resolvers.rs`, `frame_pacing.rs`, `render_target.rs`, `renderer.rs`, `video_render.rs`, `video_view.rs`, `video_click.rs` | the backend thread and everything it owns |
| `ui/` | `ui.rs` → `view.rs`, `ui_*.rs`, `composition.rs`, `slot_claims.rs`, `menu.rs`, `theme.rs`, `settings_window.rs`, `window_state.rs`, `pip.rs`, `drops.rs`, `confirm.rs`, `ui_shots.rs`, `i18n/` | Slint-facing |
| `process/` | `plugins.rs` → `process.rs`, `runtime_paths.rs` | the Node process and its pipes |
| root | `commands.rs`, `bindings.rs`, `binding_manager.rs`, `core_actions.rs`, `timeline.rs`, `power.rs`, `clipboard.rs`, `system.rs`, `diag.rs`, `input_trace.rs`, `launch.rs`, `open_files.rs`, `font_*.rs` | registries and host-level services used by several capabilities |

### 4.2 The trait

```rust
pub struct Info {
    /// Exactly the manifest string: "commands", "ui.mount", ...
    pub name: &'static str,
    /// The store's and the SDK's wording (CAPABILITY_INFO.label, store labels.ts).
    pub label: &'static str,
    pub risky: bool,
    /// Capabilities every method here also needs (CAPABILITY_INFO.requires).
    pub requires: &'static [&'static str],
}
pub enum Needs { This, None, ByParams(fn(&Value, &dyn Fn(&str) -> bool) -> Option<&'static str>) }
pub struct Method { pub name: &'static str, pub needs: Needs }
pub enum Recorded { Nothing, Calls(Vec<Call>), Live(&'static str) }
pub struct Call { pub method: &'static str, pub params: Value }

pub trait Capability: Sync {
    fn info(&self) -> &'static Info;
    /// The RPC methods this capability owns; each method has exactly one owner (checked by a test).
    fn methods(&self) -> &'static [Method];
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value, replay: bool)
        -> Result<Option<Value>, RpcError>;
    /// What this owner registered through this capability, as calls that recreate it without
    /// the process; `Live` when the owner holds something only its process can serve (B1).
    fn record(&self, host: &Host, owner: u64) -> Recorded;
    /// The pieces of today's `disable()` that belong to this capability.
    fn revoke(&self, host: &mut Host, owner: u64, keep_recorded: bool);
    /// Move this owner's entries to a new generation (wake).
    fn rekey(&self, host: &mut Host, from: u64, to: u64);
}
pub static ALL: &[&dyn Capability] = &[&ProducerAv, &PlaybackRead, /* ... in plugins.rs:51 order */];
```

`Needs::ByParams` covers the four parameter-dependent cases in `required_capability` (`events.subscribe` by event name `:5490-5494`, `source.operation` by `allows("playback.control")` `:5523-5527`, `processing.replace` by stage `:5543-5547`, `packages.list` none). The backend-passthrough arm (`main.rs:4359-4402`) becomes the `handle` of `playback_read`/`playback_control`/`producer_av`/`processing_*`/`subtitles_*`/`media_*`, each listing its methods, with a shared helper that forwards a `Command`.

**Dispatch and permission check**: `capabilities::lookup(method) -> Option<&'static dyn Capability>` from a `OnceLock<HashMap>` built from `ALL`; `Host::dispatch` becomes: look up, compute `needs`, check `plugin.allows` (the same error codes as `main.rs:3579-3587`), resolve texts, call `handle`. `plugins::CAPABILITIES` is replaced by `capabilities::names()`.

**Generated, not maintained**: a host test `capabilities::tests::json_is_current` serialises `ALL` (`name, label, risky, requires, methods[].name, methods[].needs`) and compares it with the committed `src/host/capabilities.json`; it fails with the diff when they differ (the maintainer reruns it with `VESTRA_UPDATE_CAPABILITIES=1` to rewrite). From that file:

- SDK `src/capabilities.ts`: `CAPABILITY_INFO`'s `label`, `risky`, `requires` and the key order are generated by a script in the SDK repository from the JSON (committed output, header "generated from vestra-player src/host/capabilities.json <player version>"); the `apis` member lists stay hand-written, since they are the SDK's own member-to-wire mapping, and `host-agreement.test.ts:689-780` already checks them against the methods per capability. That test and `:176-180` stop regex-parsing `plugins.rs` and `main.rs` (which the refactor would break anyway) and read the JSON.
- Store `plugins/store/labels.ts:7-32`: generated from the same JSON (the `as const satisfies` shape is kept so `text.ts` templates still type-check); `host-agreement.test.ts:502-520` keeps checking it.

**Outside the trait**: core actions and host commands (`core_actions.rs`, `register_host_commands` `main.rs:2895`), the package operations state machine (`packages/`), window state and layout policy (`window_state.rs`, the ticks in `main.rs:1522`), the activation book, the process and its pipes (`process/`), the backend thread, menus and key delivery (`menu.rs`, `bindings.rs`), the broadcast fan-out, platform.

### 4.3 Behaviour

C is a pure move: the same RPC methods, the same capability per method, the same error codes, the same `CAPABILITIES` order. The host-agreement test and the full static checks (§6.1) are the gate; `required_capability`'s five unit tests (`main.rs:5560-5700`) are kept against `lookup`.

## 5. D: phasing, estimates, acceptance, risks, approvals

### 5.1 Order

**GPU branch first, then C, then A+B.** The GPU branch (`C:\vestra\wt-gpu`) changes `main.rs` in five hunks, 41 lines: a `Host` field (`@473`), two hook calls and a new `gpu_pacing_hook` (`@1319`, `@1402`, +31 lines), two lines in `main()` (`@5138`, `@5258`); plus `renderer.rs` (12 lines) and `video_render.rs` (79). C's second step moves `Host`, `main()` and `renderer.rs`; starting it before the GPU merge guarantees a conflict in exactly those hunks. The conflict is mechanical (an hour to rebase C) but the GPU session should not have to rebase over a moved `main.rs`. So:

| Phase | Owner | Content | Days | Gate |
| --- | --- | --- | --- | --- |
| P0 | Coordinator | Baseline with the §5.2 method: node count, working set, CPU for the 17 built-ins; line counts | 0.5 | recorded |
| C1 | Maintainer | `capabilities/` trait, one file per capability with `handle` moved from `dispatch`, `lookup`, `capabilities.json` + test, SDK and store generation scripts, agreement test reads the JSON. **Does not touch `Host`, `main()`, `renderer.rs`, `backend.rs`**; can run while GPU is in flight | 3 | all §6.1 checks; `required_capability` tests pass against `lookup`; no behaviour change |
| C2 | Maintainer, after GPU merges | `git mv` into `host/`, `packages/`, `playback/`, `ui/`, `process/`; split `main.rs` | 2 | checks; `main.rs` under 800 lines |
| P2 | Coordinator | Review C against §4 | 0.5 | |
| A1 | Maintainer | `recorded` flags and `rekey` in the five registries, `record`/`revoke`/`rekey` per capability, `owner.parked`/`owner.rekey`/`owner.resources` in the backend, `Parked` + wake-on-send + queued triggers, recording file, replay in `start_installed`, idle tick, `VESTRA_IDLE_MS`, `VESTRA_LAZY` | 4 | unit tests per registry; headless test: a trivial plugin is replayed, woken by `commands.execute`, parked again |
| B1 | Maintainer | SDK `mirror` flag (SDK patch, §1.6 release steps), host `active` subscriptions, `residentAtReady`, first-run throttle, `packages.list` `process` field, store "Idle" label and `revocations.ts:85` | 2 | the 17 built-ins classify as §3.5 (differences explained) |
| P4 | Maintainer + coordinator | synthetic 2000-plugin library, measurements, click-to-dialog timing | 2 | §5.2 numbers |
| P5 | Coordinator | full GUI test (§6.2) with lazy on | | |

About 14 maintainer-days plus review and GUI. A+B before C would put ~800 new lines into the 5.7k-line `main.rs` and then move them; C first is the same total work with less churn, and `record`/`revoke`/`rekey` give A its shape.

### 5.2 Acceptance

**Method** (the coordinator's, made precise): installed layout (`VESTRA_BUILTIN_PLUGINS`), `vestra.exe --headless --silent --seconds 20`, a profile that has already run once (recordings exist); at t = 15 s: `Get-Process node | Measure-Object WorkingSet64 -Sum`, the count, and `TotalProcessorTime` summed; median of three runs. Two profiles: lazy on (default) and `VESTRA_LAZY=0` (today's behaviour, the baseline).

| Scenario | Today (user's numbers) | Accept when |
| --- | --- | --- |
| 17 built-ins, second start | 17 node processes, ~1.1 GB, ~6 s CPU | ≤ 9 processes; node working set ≤ 600 MB; node start CPU ≤ 3.5 s; the set of running plugins is exactly the resident list of §3.5 (verified ones included) |
| 17 built-ins, first start of a fresh profile | same | the window is interactive within the baseline's time; every menu item of every plugin present within 10 s; the pass never has more than 8 lazily classified processes alive at once |
| 2000 trivial plugins (a script seeds `VESTRA_PACKAGE_STORE` with 2000 copies of the spike's tiny plugin under distinct ids; `builtin.json` is capped at 64, `package_manager.rs:1468`), second start | not startable (spike) | 0 node processes at t = 15 s; all 2000 items in the menu (scrolling, G-03); host working set ≤ baseline + 60 MB; start time ≤ baseline + 1 s; the recordings directory ≤ 10 MB |
| 2000 trivial plugins, first start | — | completes the background pass in ≤ 15 min with ≤ 8 processes alive; the player is usable throughout |
| Cold menu click → dialog, trivial plugin | — | ≤ 400 ms p50, ≤ 600 ms p95 on this machine, measured from the `MenuAction::Invoke` event to the `dialogs.confirm` request (two `diag!` timestamps behind `VESTRA_DEBUG_MENU`); the spike's 137 ms first-ready for a tiny plugin leaves room |
| Cold wake of a built-in (e.g. preferences) | — | ≤ 1 s p95 to its first registration; an OSD "Starting <name>…" after 400 ms |

### 5.3 Risks

| Risk | Where | Mitigation |
| --- | --- | --- |
| Wake latency on a real plugin (bundle load, 360 ms CPU) | §5.2 | OSD after 400 ms; commands keep their key feel because the host answers the key immediately; resolvers have 30 s (`source_backend.rs:1312`); nothing waits on the UI thread |
| Lost first events | §3.3 | none by construction: a plugin with a listener is resident; a mirror subscription carries no event the plugin would act on; the SDK re-reads state at wake |
| Recording drift (registrations that depend on stored settings, OS, or a previous run) | §2.2 | live wins at every wake; the recording is rewritten at every park; a trigger for a vanished entry gets an OSD, never a silent drop |
| Plugins that keep state in memory between uses (not in the data directory) lose it at park | SDK `docs/concepts/plugins.md:98-106`, `:65` | this is the behaviour change of §5.4 item 1; the SDK docs and CHANGELOG say "activate may run more than once per session; keep what matters in `plugin.storage`"; built-ins are audited in B1 (playlist, marks and history already persist) |
| Long `onDispose` cleanup killed at 300 ms | `plugins.rs:240-252` vs runner 5 s | park waits 2 s (§2.3) |
| `packages.reload`, disable, enable, uninstall on a parked owner | §2.4 | each has a defined branch; tests per case |
| `packages.changed` churn on every wake/park | `notify_tick`, `types.ts:5184` | `process` is excluded from the change digest; `state` stays `running` |
| Store shows "Running" for an idle plugin until updated; `revocations.ts:85` only asks running plugins to disable | `store.ts:812-823` | the store ships in the same release with "Idle" and treats `process: idle` like running |
| `ui:shots` / `ui-record` | `ui_shots.rs:1-22`, `scripts/ui-shots.mjs:10` | unaffected: they render from plugin tests under the SDK TestHost, not from the running player; `menus-snapshot.test.ts` likewise |
| Coordinator GUI scripts that count node processes or expect `state: running` | §6.2 | `VESTRA_LAZY=0` for the old expectations; new expectations written in P4 |
| Headless tests that rely on a plugin doing work at activation | `src/tests/` | same switch; listed per test in A1 |
| Owner re-keying misses a registry | §2.2 | `rekey` is a trait method, so a capability without one does not compile; a test parks and wakes every built-in and diffs the registries |
| Third-party plugins that misbehave when `activate` runs twice in one session | | they already face reload and upgrade; the CHANGELOG entry opens with who must change (§1.6 item 3) |

### 5.4 Behaviour changes that need the user's approval (§1.6 item 2)

1. **Plugins stop when idle and start on demand.** New stop reason beyond the documented five (`docs/concepts/plugins.md:98-100`); `activate` can run several times per player session; memory-only state is lost between uses. Affects every plugin that holds no live resource at ready. Migration: keep state in `plugin.storage`/the data directory (already the documented way).
2. **Work a plugin does at activation that is not a registration happens at first use, not at launch.** Known case: the store's revocation check (`plugins/store/revocations.ts`) runs when the store first wakes; revoked versions stay enabled until then. Alternative if not acceptable: the store becomes the one `"activation": "startup"` user, which brings the escape hatch forward.
3. **Menu state of an idle plugin is its last recorded state** until a trigger wakes it; a trigger on a now-disabled command shows the reason instead of running.
4. **A first trigger is slower** (wake); an OSD "Starting <name>…" appears after 400 ms.
5. **Headless runs start only resident plugins** unless `VESTRA_LAZY=0`; existing headless tests and tooling change their expectations.
6. **Park waits up to 2 s for the process** instead of 300 ms (the documented "killed after 300 ms" becomes "after 2 s" for a park; disable and quit keep 300 ms).

Pure additions (maintainer decides, patch releases): the `Capability` trait and the directory move (no behaviour change), `capabilities.json` and the generated SDK/store tables, `events.subscribe { mirror }`, `packages.list` `process: "running" | "idle"` (the SDK `PluginInfo` gains an optional field; `state` unchanged), `VESTRA_LAZY`, `VESTRA_IDLE_MS`, `owner.parked`/`owner.rekey`/`owner.resources` (host-internal), the first-run throttle (no plugin-visible change), the manifest field `activation` (deferred).

Question for the user, one sentence: approve items 1–6 as the behaviour of lazy activation, or name which of them must instead stay as today (item 2 is the only one with a cheap alternative).

### User ruling (2026-10-27)

The user approved starting the work ("可以开工") after the coordinator presented §5.4 items 1–6 with its recommendations; all six behaviour changes are approved as written in §5.4 (idle plugins are stopped and woken on demand; work done at activation moves to first use, with the store's revocation check allowed to use `"activation": "startup"` if needed; parked plugins' menu state is the recorded one; a first trigger may be slower, with an OSD after 400 ms; headless runs start only resident plugins by default, `VESTRA_LAZY=0` restores the old behaviour; parking waits up to 2 s). Direction per the user's later rulings: an event-wake model like Chrome MV3 (the host keeps resources and wakes plugins on recorded events), so that only the bottom bar and the open sidebar stay resident; order with the shared runner per `2026-10-shared-runner.md` §12.

### User ruling (2026-10-27, later): no idle stop in v1

v1 is **start on first use only**: a plugin that is not used is not started (installed plugins cost no process until used), but once started it runs until the player quits. There is **no idle stop**, so plugin authors keep their habits (state in memory survives). §5.4 item 1 (idle plugins stopped, in-memory state lost) and item 6 (parking waits up to 2 s) are withdrawn; items 2–5 stand. If idle stopping is added later it is opt-in per plugin (a plugin declares it may be stopped); a plugin that does not declare it is never stopped. Reason: most of the gain comes from not starting unused plugins, and with the shared runner a resident built-in costs about 0.8 MB, so idle stopping would save little for a change in how every author writes plugins (the MV2 → MV3 pain).

### User ruling (2026-10-27, 08:59): idle stop as in Chrome MV3; correction (14:20)

After the "no idle stop" ruling above (08:19), the user asked how to reclaim plugin memory and ruled (08:59): when the change in authors' habits can be predicted, follow Google's way: existing APIs stay as they are, a new API keeps state for the life of the player process, and the docs stress it. That is Chrome MV3: an idle extension service worker is stopped and woken by its events, and state that must outlive it goes to `chrome.storage.session`. The coordinator recorded only the API half ([2026-10-session-storage](2026-10-session-storage.md), shipped in host 0.5.5) and left the ruling above in force; the user pointed this out at 14:20. **This ruling supersedes "no idle stop in v1":** §5.4 items 1 (idle plugins are stopped and woken on demand; memory-only state is lost, session storage keeps it) and 6 (a park waits up to 2 s) are restored, so all six items stand as in the first ruling. Idle stop applies to every plugin that holds no live resource (§2.3), as in Chrome, not opt-in. With the shared runner, parking a built-in unloads its context in the shared process; the larger gain is a third-party plugin in its own process.

## 6. Implementation status

| Step | Status (2026-10-27, branch `capabilities-c1` from `main` d8e7763) |
| --- | --- |
| C1 | Done, pending review: `src/host/src/capabilities/`, `src/host/capabilities.json`, the agreement test reads the JSON |
| C2 | Done 2026-10-28 (branch `refactor-c2`, two commits: moves, then the `main.rs` split; see §6.5) |
| A1 | Done 2026-10-31, pending review (branch `lazy-a1`; see §6.6), off unless `VESTRA_LAZY=1` |
| B1 | Done 2026-10-31, pending review (branch `lazy-b1`; see §6.8), off unless `VESTRA_LAZY=1`; SDK 1.5.38 prepared, not tagged |

### 6.1 C1: what landed

- `capabilities/mod.rs`: `Info { name, label, risky, requires, residency }`, `Needs`, `Method`, the `Capability` trait (`info`, `methods`, `handle`), `ALL` (25 capabilities in the old `plugins::CAPABILITIES` order), `lookup` (exact name, then a `prefix.*` entry, from a `OnceLock`), `Route::needs`, `required` (the old `required_capability`, generated), `known`/`count` (replace `plugins::CAPABILITIES` in `plugins.rs`, `manifest.rs`, `package_stage.rs`), `json()` and the tests. One file per capability holds its metadata, its methods and the `dispatch` arms moved from `main.rs` (`self.` → `host.`, otherwise unchanged); `uncapable.rs` holds the eleven methods any plugin may call. `Host::dispatch` in `main.rs` is now: plugin, `lookup`, `needs`, the same `capability` refusal, the same text resolution, `route.handle`.
- `src/host/capabilities.json` (649 lines): `apiVersion`, then per capability `name, label, risky, requires, residency, methods[{name, needs}]`, then `uncapable`. `capabilities::tests::json_is_current` fails with the first differing line; `VESTRA_UPDATE_CAPABILITIES=1 cargo test capabilities` rewrites it. serde_json writes object keys sorted.
- `src/tests/sdk/host-agreement.test.ts`: the manifest-rules test and the per-member capability test read the JSON instead of regex-parsing `plugins.rs` and the `required_capability` match (the `matchArms` reader is gone); a new test checks the JSON's `label`, `risky`, `requires` and order against the installed SDK's `CAPABILITY_INFO`, which holds the drift until the SDK and store tables are generated. The OSD limits moved with `osd.*` to `capabilities/ui_osd.rs`; the step-seek test reads `capabilities/playback_control.rs` and `capabilities/commands.rs`.
- Not touched: `Host`, `main()`, `renderer.rs`, `backend.rs`, the rest of `plugins.rs` (only its table and the one check that read it). The `main.rs` hunks are the `mod` line, the OSD constants, `dispatch` and the removed `required_capability` with its test: away from S2's `disable`/`handle`/`send`/`Event` and from the GPU hunks.

### 6.2 C1: where it differs from §4.2

- `Needs` is data, not a function pointer: `This`, `Nothing`, `Param { key, cases, otherwise }` (events by `event`, processing by `stage`) and `IfDeclared { declared, then, otherwise }` (`source.operation`), so the JSON carries the rule and the agreement test evaluates the same rule the host does.
- The trait has only `info`, `methods` and `handle`; `record`, `revoke` and `rekey` come with A1, which defines what they do. The residency hint (`none`, `recordable`, `live`) sits in `Info` and the JSON; nothing reads it yet.
- `prefix.*` method entries (`packages.*`, `sourceResolvers.*`, `settings.*`, `snapshot.*`, `jobs.*`, `subtitles.*`) keep the old prefix arms, so an unknown method under a prefix still gets the capability check and then its handler's `unsupported`, as before.
- `processing.replace` and `processing.operation` are owned by `processing_video.rs` and ask for `processing.video` or `processing.audio` by stage; `processing.audio` owns no method. `packages.list` and `window.state` are in `uncapable.rs`.
- `plugins::CAPABILITIES` is replaced by `capabilities::known` and `count`, not `names()`.
- The list of methods whose parameters get text resolution stays in `Host::dispatch` (a candidate for a per-method flag later).

### 6.3 C1: equivalence evidence

A temporary test (removed before commit) put the old `required_capability` beside `capabilities::required` and compared them, result and error code, over 240 method names (every name of the old match, every method of the new table, prefix neighbours and unknown names) × 8 parameter sets × 3 declared-capability sets: 5 760 cases, all equal. `cargo test --locked`: 479 passed before, 483 after (the moved `required_capability` test plus four new ones), 8 ignored both times.

One difference that is not a table difference: a plugin that declares `source.resolve` and sends an unknown `sourceResolvers.<x>` reached `unreachable!()` in the old `dispatch` (a host panic); it is now refused `unsupported` ("unknown method"), the answer every other unknown method gets. Recorded for the coordinator: it is the only reachable path whose outcome changed.

### 6.5 C2: what landed (2026-10-28, user request of 2026-10-27)

- `src/host/src/` now has `host/` (15 files: `Host`, events, handle, lifecycle, dispatch, rpc, actions, state, window, shell, menus, input, mounts, video, package_activation), `packages/` (8), `playback/` (34), `ui/` (21), `process/` (3), plus the unchanged `capabilities/`, `platform/`, `i18n/`; 16 files stay at the root. `main.rs` is 567 lines (was 4875).
- Differences from §4.1: `plugins.rs` became `process/mod.rs` (a `process/process.rs` trips clippy's `module_inception`); `i18n/` stays at the root (`build.rs` reads it by path); `host/` has more files than planned because `impl Host` was over 3 200 lines; files the plan did not list: `bar_buttons.rs`, `slot_claims.rs`, `ui_state.rs`, `ui_types.rs`, `ui_fonts.rs`, `window_control.rs` → `ui/`; `lease_stream.rs` → `playback/`; `package_validate.rs` → `packages/`; `shared_runner.rs` → `process/`; `session_storage.rs` stays at the root (a host service several capabilities use).
- Equivalence (independent review): an item-by-item token comparison of the whole tree before and after each commit found only path edits, visibility (`pub(super)` → `pub(crate)`, crate-internal only), `include_str!` depth and rustfmt re-wrapping; the moved items, including `Host`'s field order, are token-identical. `cargo test --include-ignored` 545 passed before and after; name-filtered CI tests still match one test each.
- Mac test list: compile the host on macOS once (the review checked the unix-only code paths by reading).
- The Windows wgpu branch needs a rebase: renamed files follow automatically; its `main.rs` hunks move by hand into `host/mod.rs`, `host/handle.rs`, `host/video.rs`, `host/window.rs` and `main.rs` (about half a day).

### 6.4 C1: follow-ups

- SDK: generate `CAPABILITY_INFO`'s `label`, `risky`, `requires` and key order from `capabilities.json` by a script in the SDK repository (the `apis` lists stay hand-written); store `plugins/store/labels.ts` likewise. Not needed for C1: the agreement test already fails on any drift between the JSON and the installed SDK.
- Host version unchanged (0.5.5): no user-visible, wire or error-code change (apart from the panic above), and the repository has no rule that asks for a bump on an internal change.

### 6.6 A1: what landed (2026-10-31, branch `lazy-a1` from `main` b3cefd3, host 0.5.16)

Off by default: `VESTRA_LAZY=1` turns it on (§5.2 makes lazy the eventual default; this step lacks B1's classification pieces, so the default waits for B1 and the coordinator's hardware checks). With it unset or `VESTRA_LAZY=0`, every plugin starts at launch as before; the only paths that change are `Host::disable`, which now revokes the five registries through `Capability::revoke`, and `drops.provide`, which no longer reports a provider's own extensions as taken when it provides them again.

- Registries: `commands::Registry` (`recorded`, `sent` with each command's declaration and last state as the plugin sent them, `mark_recorded`, `rekey`, `remove_recorded`, `busy`; a live registration replaces a recorded entry in its menu place and the same owner's colliding recorded entries give way), `bindings::Table` (`rank`, kept by `rekey` and by a second `declare`, so a woken plugin keeps the keys it won), `settings_window::Settings` (recorded pages, the sent registration, `recording`, `rekey` rebuilds an open window since its callbacks name the owner), `drops::Drops` (recorded providers keep their place), `source_resolvers::Registry` (recorded entries hold their types against other plugins). One unit test each.
- `Capability` trait: `record` and `rekey` required on all 27; `revoke`, `prune`, `replay` with defaults, implemented by the four recordable capabilities (`commands` covers the binding table). What the playback worker holds is answered by `owner.resources` as a whole (`{ live, resolvers }`): jobs and snapshots, processing chains, subtitle tracks and styles, producers, a resolution in flight, the audio device, the video view and render options, and upscaling (never parked, ai-upscaling.md §8.5).
- Backend: `owner.parked` (as `owner.disabled`, resolvers kept), `owner.rekey { to }` (resolvers and waiting resolutions), `owner.prune`, `owner.replay`, `owner.resources`. All are commands, so the worker's event-driven wait is unchanged.
- Host (`host/lazy.rs`): `Lazy` (parked, waking, last use per owner), `Host::send` wakes a parked owner on `commands.execute`, `source.resolveRequest`, `drops.drop` or `ui.callback` and holds triggers until `runtime.ready`, where unreplaced recorded entries are pruned and held commands are re-resolved (a disabled one shows its reason, a vanished one "<title> is no longer available"). "Starting <name>…" after 400 ms. The idle tick runs in the existing host loops every 250 ms and asks the worker only about plugins idle for `VESTRA_IDLE_MS` (default 60 000) with nothing in flight. A park waits 2 s for an own process (§5.4 item 6); a shared-runner member unloads its context as for any stop.
- Recording: `<package store>/activation/<instance GUID>.json` (beside `catalog.json`, outside plugin data), keyed by package digest (a built-in's from `builtin.json`; a built-in listed without a digest is never parked), version, player version and build; written at park. `start_installed` replays a valid one instead of starting the plugin. Disable forgets the parked entries and keeps the file; reload, uninstall and install/upgrade remove it, parked or not and with lazy activation on or off. `packages.list` shows a parked plugin as `running`. A parked plugin keeps a catalog `Arc` of its own (the stopped plugin unregisters its own on the thread that drops it) and its package and data directories for images (`Host::image_roots`: its bar button and settings-page images).
- Development loads are never parked or recorded.
- A wake waits for the process the plugin was parked from to be gone and cleaned up (`Event::Reaped` and `BackendCleaned`, as `packages.reload` waits, at most 20 s, then "<name> could not start"): two instances never share its data directory. A wake that fails shows the same OSD. After a wake, triggers still addressed to the old generation (a resolution the worker asked for before `owner.rekey`) go to the new one.
- What counts as in flight for the idle check: activation not `ready`, a dialog open (any plugin's), a package operation or reload on it, a busy command, a settings page compiling or the Options window open with its page, a `ui.open`/`ui.mount` in activation, a `source.state` reply, a request held for the built-ins' start; in the worker, a resolution its resolver has not answered, jobs and snapshots. Replies to requests the plugin sent to the worker are answered before `owner.resources` (one queue). A request the plugin sent elsewhere and awaits off the host (its own network fetch, a timer) is not seen: such a plugin is parked after the idle time as in Chrome MV3, and its work is lost (state it needs belongs in session storage). The B1 docs say so.
- `VESTRA_TEST_EXECUTE=<manifest id>/<command id>@<ms>[,…]` (or `@park+<ms>`: that long after the plugin's next park) runs a plugin command as a menu click would. A test switch read in any build, like `VESTRA_TEST_REMOVE_DEVICE`, and like it not passed on to a player restarted after a device loss.
- Headless test `tests/lazy_activation.rs` (ignored by default like `shared_runner.rs`): a trivial built-in is started and parked, replayed without a process at the next start, woken by `commands.execute` (the command runs), parked, woken again 100 ms after that park while its first process still runs a 1.2 s `onDispose` (the start waits for it; session storage carries its start count, "hello in start 2"), and parked again; `VESTRA_LAZY=0` starts it at launch; with the shared runner and with own processes.

Host 0.5.17 (recordings are keyed on the player version).

Not in A1 (B1 or later): recordings at quit and `residentAtReady: true` (a plugin woken into a live resource is not parked, but its recording stays and it is replayed at the next start instead of starting at launch); the SDK `mirror` flag and active subscriptions (until then any subscription counts as live, so plugins with `events` stay resident); the first-run throttle; `packages.list` `process`, the store's "Idle" label and keeping park/wake out of the `packages.changed` digest; waking when a settings page is shown (now: a plugin is not parked while the Options window shows its page, and a parked one wakes on its page's first callback); moving the rest of `disable()` into `revoke`; the park-and-wake-every-built-in registry diff test (§5.3); P4 measurements and the GUI test.

### 6.7 Coordinator GUI checks (2026-10-31, release build of 9fb9cb1, host 0.5.17)

Separate application data directory, internal panel, CPU mode.

- Lazy off (default): the context menu has the same items and order as 0.5.16; playback normal; every plugin starts at launch.
- `VESTRA_LAZY=1`, `VESTRA_IDLE_MS=5000`: as expected for A1 only `vestra.workshop` parks (2 recorded calls); every plugin with an event listener stays running, `vestra.video` stays for its processing chain and video view, `vestra.playlist` for its playback setting.
- Context menu ▸ Vestra Player AI… on the parked plugin: "woken by commands.execute (owner 16 → 17)", ready after 39 ms, the window opens.
- While the window is open the plugin stays running ("an open window"); after closing it parks again.
- Restart with lazy on: "replayed (2 calls) … not started"; the bottom-bar AI button is present from the recording; clicking it wakes the plugin (ready after 63 ms).

Not checked on hardware: a click within the 2 s after a park (M1 path is covered by the headless test), settings pages and images of a parked plugin, two players at once.

### 6.8 B1: what landed (2026-10-31, branch `lazy-b1` from `main` 8fdf89b, host 0.5.18, SDK 1.5.38 prepared on the SDK branch `lazy-b1-138`, not tagged)

Still off by default (`VESTRA_LAZY=1` turns it on).

- **SDK 1.5.38 (mirror subscriptions, §3.1).** `Runtime.on` takes a `mirror` flag; the services' own subscriptions (`source.state` for `player` and `subtitles`, `playback.backend`, `window.state` for `window` and the panel layout, `timeline.changed`, `appearance.changed`, `resolvers.fileTypes`) are mirrors and subscribe with `events.subscribe { event, mirror: true }`. A service `watch` (every `onChange`, `onEnd`, `onFileTypesChange`, the SDK's own waits such as `media.loaded`, scrubs, filters and views) holds the event as a listener; so does a **binding that reads the state**: the update pass records which service states each binding read (`Runtime.reading`, called by the `state`/`marks`/`theme`/`language`/`fileTypes` getters) and holds those events while it reads them. This goes beyond §3.1, which counted only listeners: a command whose `enabled`, `checked` or `reason` is a function of `plugin.player.state` is kept current by the mirror alone, so without it such a plugin would be parked with a stale menu (§2.3 assumed every such plugin has a listener; capture and preferences do not). Transitions send `events.subscribe { event }` or `{ event, mirror: true }` again; the host's subscribe replaces the mark. Older players ignore the field. `PluginInfo.process` (`"running"`/`"idle"`), `TestHost.listening`, a new section _Idle plugins are stopped_ in `docs/concepts/plugins.md` (MV3 semantics: what stays, what keeps a plugin running, and that network requests, timers and awaited promises do not; session storage for what must outlive the process), CHANGELOG, `test/mirror-subscriptions.test.ts`.
- **Host: active subscriptions.** `Plugin.subscriptions` is `event → listener?`; `events.subscribe` with `mirror: true` stores `false`; broadcasts go to both; `Events::record` is `Live` only for a listener.
- **`residentAtReady` (§3.4).** At every `runtime.ready` of an installed plugin (a start, a wake, a first run) the host evaluates `capabilities::record` on its own state at once (before a woken plugin's triggers run) and asks the worker (`owner.resources`) for its half; the answer is its class. A resident plugin's recording is written then (`residentAtReady: true`, no calls) unless it already says so, so the next start launches it even after a crash; a lazy one keeps its recording from its park. `start_installed`: resident recording → start at launch ("starts: resident at its last ready"); lazy recording → replay; none → first run. A park writes the ready class into the recording (a plugin resident at ready that let go of everything later is parked, and still launched at the next start). Self-healing both ways: every ready reclassifies.
- **Recordings at quit** (both event loops, before the plugins stop): a resident one as resident (if not already), a lazy one with its registrations now (with the resolvers the worker reported at its ready); one that holds a live resource at that moment keeps its file as it was.
- **First-run throttle (§2.4).** A plugin without a valid recording starts as a first run, at most 8 in flight; the others wait in `Lazy.first_queue` (listed as `starting`). A first run classified lazy is parked at once (no idle time); its place frees when its process is gone; a resident one frees its place at classification, a failed or timed-out activation when it fails. While a built-in waits there, `builtins_starting` stays true (other plugins' mounts wait, slot claims unchanged) and opened files wait (bounded by `SETTLE_LIMIT` as before).
- **`packages.list`**: `process: "running"` for a running plugin, `"idle"` for a parked one (state stays `running`); a woken plugin activating is `running`, not `starting`; `packages.changed` compares the list without `process`, so a park or wake alone sends no event (§5.3).
- **Settings page shown** wakes its parked plugin (A1 left: only its first callback did). `Host::lazy_page_shown` from `settings_select` while the Options window is open.
- **A1 review open low: a start right after a park waits for the parked process.** `Lazy.stopping` keeps each plugin's last parked owner until its process is gone and the worker cleaned up. `packages.reload` of a just-parked plugin goes through `reloads` (started by `reload_tick` once gone, as for a running one); install/upgrade's operation waits on it (`Operation.waiting`); `setEnabled(true)` right after disable-of-a-parked plugin waits the same way; uninstall with `removeData` deletes the data after its `Reaped`.
- **Headless test** `tests/lazy_activation.rs`: the A1 test unchanged (now its first run parks at once); `mirror_subscriptions_park_and_wake_a_listener_keeps_a_plugin_resident` (a plugin whose only subscription is the SDK's `source.state` mirror is classified lazy, parked at its first ready, replayed, woken by its command, which prints `plugin.player.state.phase` read at its start, and parked again; a plugin with `player.onChange` is resident, recorded `residentAtReady: true` at its ready and started at launch the next time, never replayed; shared runner and own processes); `first_runs_start_eight_at_a_time` (10 trivial built-ins on a fresh profile: the 9th starts only after the first park; all 10 parked and recorded).

Host 0.5.18 (recordings are keyed on the player version: every recording of 0.5.17 is a first run again).

#### 6.8.1 Observed classification (headless, `VESTRA_LAZY=1`, `VESTRA_IDLE_MS=3000`, staged built-ins, shared runner, fresh profile then second start)

16 built-ins: `vestra.nes` left the player (it is VestraNES in the plugins repository). The reason is the host's log line; the event names are the subscriptions the plugin really listens to.

| Plugin | Record §3.5 | Observed | Why (code) |
| --- | --- | --- | --- |
| playback | resident | resident (headless: activation never ready) | bottom bar mount; headless refuses mounts and its activation does not complete, so it is a first run on every headless start (on a display it mounts and is resident) |
| playlist | resident | as playback | sidebar mount |
| visualizer | resident | as playback | backdrop mount |
| video | resident | resident: listener `source.state`; the worker holds the video view | `player.onChange` in `main.ts` |
| audio | resident | resident: `source.state` | `player.onChange` in `main.ts` |
| subtitles | resident | resident: `source.state`, `playback.backend` | `subtitles.onChange` |
| window | resident | resident: `source.state`, `window.state` | `window.onChange`, `player.onChange` |
| history | resident | resident: `source.state` | `player.onChange` in `main.ts:25` |
| marks | resident (verify) | resident: `source.state` | `player.onChange` in `main.ts:48` |
| info | lazy (verify) | **resident**: `packages.changed`, `source.state` | at activation: `packages.onChange` (`about.ts:148`) and the properties window's `player.onChange` (`dialog.ts:183`), both registered when the plugin starts, not when the window opens |
| hotkeys | lazy (verify) | **resident**: `bindings.changed` | `IgnoreDefaults` subscribes `bindings.onChange` in its constructor at activation (`ignore.ts:32`); `main.ts:53` reads it for a checkmark |
| capture | lazy | **resident**: `source.state` | its commands' state is a function of the player (`main.ts:353`: `enabled: () => usable(player.state)`, `reason` likewise): the new binding tracking keeps it so the menu does not go stale |
| subtitle-online | lazy | **resident**: `source.state` | `player.onChange` at activation for the automatic search on a new source (`main.ts:570`): a transition listener, rightly resident by §3.3 |
| preferences | lazy | **resident**: `appearance.changed` | theme and language checkmarks are functions of `plugin.appearance` (`main.ts:97`, `:107`, `:117`) |
| store | lazy | **resident**: `packages.changed` | `packages.onChange` at activation (`store.ts:268`) |
| workshop | lazy | **lazy**: parked at its first ready (2 recorded calls), replayed at the second start without a process | |
| nes | lazy | — | not a built-in any more |

So 1 of 16 is lazy today, against 8–9 expected. The rule and the host work as designed (the fixtures and workshop show it); the difference is that the built-ins listen at activation. That is plugin-author work; per plugin, what would make it lazy and what it costs:

- **capture**: compute `enabled`/`reason` when run instead of as bindings (a disabled-looking item becomes "No frame to capture" on the OSD when chosen), or accept residency: its menu state follows playback.
- **preferences**: set `checked` with `command.set(...)` after its own `setTheme`/`setLanguage` instead of a function of `plugin.appearance`. A theme changed by another plugin (`ui.theme`) then shows a stale checkmark until the next wake; the user's only theme menu is this plugin's, so that is acceptable.
- **hotkeys**: subscribe `bindings.onChange` only while the Keys page or window is open (open a child scope with the page), and read `bindings.list()` when a command runs.
- **info**: register `packages.onChange` and the properties window's `player.onChange` inside the window's scope (when it opens), not at activation.
- **store**: subscribe `packages.onChange` when its window opens. The revocation pass then runs at its first use (the user's §5.4 item 2 ruling allows it); `revocations.ts:85` needs no change, since a parked plugin keeps `state: "running"`.
- **subtitle-online**: stays resident while it offers the automatic search on a new source (a transition, §3.3); lazy only if the automatic search becomes an explicit command.
- **store "Idle" label** (record §5.1 B1 row): `store.ts:1025` `#status` shows "Running" for a parked plugin; add `info.process === "idle"` → "Idle" (needs `msg("Idle")` in `text.ts` and its six translations in `locales/`, a CHANGELOG entry and SDK 1.5.38 in the player). Listed for the store author, not done here: it is translation and wording, not a mechanical change.

#### 6.8.2 Decisions and notes for the merge

- **Binding-dependency tracking is a coordinator-accepted decision** (2026-10-31): a command's `enabled`, `checked`, `busy` or `reason` function, or a view binding, that reads a service's state counts as listening to that state's event, beyond §3.1's "only listeners". It keeps a menu that follows playback current instead of showing a stale recorded state while parked; it is why capture and preferences are resident (§6.8.1).
- **SDK 1.5.38 in the player**: the player commit that moves `src/package.json` to SDK 1.5.38 must also run `npm run vendor:sdk` for the workshop (its vendored SDK copy); the coordinator does that through the workshop author.

#### 6.8.3 Review fixes (independent review of 1df8ca5 / SDK b2704ed; lazy on only)

- **M, first-run throttle vs `builtins_starting`**: start-up goes in id order, so other plugins whose ids sort before `vestra.` could take all eight first-run places while their mounts, sequence and resolvers waited for the built-ins (`builtins_starting`), until their activation timed out. Now a first run of a plugin that is not built in waits while a built-in is still to start (`to_start`) or queued, and the queue keeps built-ins ahead of the others (`Lazy::queue_first_run`); the tick stops when the plugin it popped is queued again. Test `built_in_first_runs_go_before_other_plugins`: nine installed plugins `aa.fixture-0..8` registering a resolver at activation and one built-in; the built-in starts first, none times out, all ten are parked and recorded (headless does not hold those requests, so the order is what it checks).
- **L2**: a parked plugin keeps the `warnings` and `error` text it had when it ran (`Parked.warnings`, `warning`), so a park or wake of a plugin with warnings does not fire `packages.changed` either.
- **L3**: a disabled plugin left in the first-run queue is listed `disabled`, not `starting`.
- **L4 (SDK)**: the mark `events.subscribe { event, mirror? }` sent during activation is part of `startup`, so `runtime.ready` never comes before it (test: a listener added in `activate` is marked when `startPlugin` returns).
- **L5**: the classification at ready waits at most 5 s for the worker's `owner.resources`; without an answer the plugin is classified resident (logged), which frees its first-run place.

#### 6.8.4 Not in B1

- The rest of `disable()` into `Capability::revoke` (a pure refactor; A1 left it, B1 does not need it).
- The park-and-wake-every-built-in registry diff test (§5.3): with 1 of 16 built-ins lazy it would cover only workshop; worth writing once the built-ins above change.
- P4 measurements (the 2000-plugin library, click-to-dialog timing) and the GUI test with lazy on.
- Turning lazy on by default (coordinator, after the hardware checks).
