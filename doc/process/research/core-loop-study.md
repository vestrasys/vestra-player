# Core loop study: moving the host core off the Slint UI thread

**Status: research report, 2026-11-15.** Kept as written by the study agent. The decisions taken from it are in [core-thread.md](../../architecture/core-thread.md); where §7 below recommends something else (question 1: hold a plugin's later lines behind a pending UI reply), the design wins (two streams, no ordering queue). The stage numbers in §7 (0–2) are S1–S3 in [core-thread-plan.md](../core-thread-plan.md).

Read-only study of branch `evt-release-a` (worktree `C:\vestra\wt-evt-ra`, HEAD `3100d94f`). No code was changed, built or run. Paths below are relative to `src/host/src/` unless they start with `src/` or `doc/`. "(inferred)" marks what was concluded from reading, not from a compile or a run.

Rust notes for the reader (one line each, used throughout):

- **`Send`**: a type the compiler allows to move to another thread. A struct is `Send` only if every field is. One non-`Send` field makes the whole struct non-`Send`, and the compiler then refuses any code that hands it to another thread. There is no "unsafe but probably fine" escape in this crate (`#![forbid(unsafe_code)]`, `main.rs:1`).
- **`Rc` / `RefCell` / `Cell`**: single-thread reference counting and run-time borrow checks. Anything containing an `Rc` is not `Send`.
- **Slint objects** (`ComponentInstance`, `slint::Weak`, `ModelRc`, the window adapter) belong to the event-loop thread. Another thread reaches them only through `slint::invoke_from_event_loop(closure)` / `Weak::upgrade_in_event_loop`, which queue a `Send` closure onto the UI thread.
- **`&mut self`**: one mutable borrow at a time. The windowed tick holds `host.borrow_mut()` for the whole turn (`main.rs:874`); the frame, resize and PiP sinks use `try_borrow_mut` and skip when the tick holds it (`main.rs:814, 832, 840`).
- **`mpsc`**: `Sender` is cloneable and `Send`; a `Receiver` has one consumer.

## 0. Summary

- `Host` (`host/mod.rs:338-598`) has **130 fields**, implemented in **39 `impl Host` blocks over 37 files** (29,167 lines including tests). It is **not `Send`**: `shell`, `views`, `floating`, `panels`, `placements`, `settings`, `images`, `input`, `video_target` and `winit_draws` hold Slint objects or `Rc`s. So the core cannot simply run on another thread. The struct must be split.
- Everything reaches the host through **one unbounded `mpsc` queue** (`main.rs:426`), drained by a **16 ms Slint timer** (`main.rs:865-979`). Nothing wakes the loop when an event arrives, so a request waits 0–16 ms before it is handled even when nothing stalls. The headless driver uses `recv_timeout(10 ms)` on the same queue (`main.rs:676-730`).
- **mpv control is already off the UI thread.** The playback control worker owns mpv (`playback/backend.rs:1, 131`). No `rsmpv`/mpv call exists in `host/`, `capabilities/`, `ui/` or `main.rs`. But every plugin request reaches the worker only through the tick, and every worker reply and `source.state` comes back through the tick. During a UI stall, mpv keeps playing, but plugins can neither control nor observe it.
- Of the **119 RPC route entries**, **48 are forwards to the playback worker**, **36 are core-only**, **25 touch UI but answer without a UI result**, and **10 have a reply that depends on a UI result** (the (c) cases, §2.2). Of the **50 `Event` variants**, 26 come from the UI itself. Of the other 24, 10 are core-only, 11 reach UI through shared paths (OSD, title, `stop` revoking views and panels), 1 is (c) (`SettingsChecked`), and 2 (`Request`/`Prepared`) depend on the method.
- The core touches the UI from inside "core" paths in three main places: **`Host::stop_run`** (views, mounts, OSD, floating window, shell properties; `host/lifecycle.rs:282-445`), **`show_osd`/`put_osd`** (`host/actions.rs:346-400`), and the **`source.state` broadcast** (no-video, backdrop, window title; `host/handle.rs:231-268`, `host/state.rs:115-176`). The supervisor's activation also **waits for UI results**: a plugin's run is not "ready" until its initial `ui.open`/`ui.mount` were answered (`host/dispatch.rs:118-144`, `host/supervisor/mod.rs:995-1016`).
- Recommendation: boundary **(B)**, built in stages: first split the struct while staying on one thread, proven by the differential checks; then move the core to its own thread. Boundary (A) buys almost nothing (§4.1). Boundary (C) is a rewrite of the UI layer for no extra resilience.

## 1. Inventory of Host state

The classes:

- **CORE**: never touches a Slint object or a window.
- **UI**: holds or needs Slint objects or native windows.
- **MIXED**: core data that the UI reads or writes, or UI data that the core reads to answer requests. Each MIXED entry says what ties it.

The line numbers are field declarations in `host/mod.rs`.

### 1.1 Fields

| Class | Fields (line) | Notes |
| --- | --- | --- |
| CORE: process and supervisor | `shared` 343, `shared_policy` 345, `plugins` 346, `plugins_revision` 349, `reaper` 486, `supervisor` 470, `activation_receipts` 471, `on_demand` 589, `start_error` 517, `data_removals` 520, `dev_loading` 523, `development` 509, `dev_disabled` 511, `failures` 513, `warnings` 482 | `Plugin` holds `Child`, `SyncSender`s and `Arc`s (`process/mod.rs:300-382`), so it is `Send` (inferred). The supervisor core is pure (doc/architecture/supervisor.md §1). |
| CORE: playback | `backend` 339, `last_source` 456, `source_state_replies` 459, `sequence` 455, `step_seek` 499, `step_writes` 500, `actions_busy` 496, `file_types` 473 | `Backend` = command sender + `Arc`s (`playback/backend.rs:87-102`), `Send` (inferred). Its `frame`, `render` and `target` `Arc`s are also used by the UI frame path. |
| CORE: registries | `commands` 354, `timeline` 545, `triggers` 592, `trigger_state` 593, `session_storage` 525, `packages` 483, `held_mounts` 475, `prepared` 479, `command_runs` 409, `opens` 576, `panel_plugins` 368 | `commands` is read by the menus, the palette, the bar buttons and the tab keys on the UI side (see MIXED below). |
| CORE: notification bookkeeping | `bindings_sent` 527, `bindings_list_sent` 530, `bindings_seen` 534, `packages_sent` 535, `packages_seen` 538, `notified_at` 539, `keys_checked` 542, `keys_sent` 543, `host_revision` 464 | `notify_tick` reads the binding table out of `input` (`binding_manager.rs:173-179`). |
| CORE: misc | `events` 352 (the queue's `Sender`), `headless` 353, `clock` 411, `rendering` 458, `power` 487, `shader_guard` 595, `memory_logged` 597 | `power` on Windows is `SetThreadExecutionState`, which is **per calling thread** (`platform/windows.rs:842-858`). It works from any long-lived thread, but must always be the same one. |
| UI: windows and Slint objects | `shell` 362, `views` 351, `floating` 460, `panels` 422, `placements` 584, `last_image` 376, `video_texture` 379, `video_target` 382, `winit_draws` 387, `native_window` 438, `native_before` 439, `native_unavailable` 440, `pip` 557, `pip_log` 559, `pip_gpu` 569, `gpu_paced` 572 | Not `Send`: `ComponentInstance`, `Rc<Panel>` (`ui/panels.rs:600-607`), `rc::Weak<dyn WindowAdapter>` (`ui/view.rs:208-216`), `Rc<RefCell<Weak<…>>>`, `Rc<Cell<bool>>`. |
| UI: shell layout and generations | `shell_generation` 388, `next_shell_generation` 389, `measured_shell` 413, `columns` 419, `compiled_now` 425, `restarts` 428, `tabs_failed` 435, `bottom_shown` 437, `title_strip` 416, `bar` 586, `theme_pending` 582, `settings_category` 580, `mount_singly` 373, `preparing` 370 | `preparing` and `mount_singly` hold the 32 ms mount batching window (`host/mounts.rs:79-125`). |
| UI: menus, palette, sheets | `menu_path` 390, `menu_scroll` 392, `menu_aim` 394, `menu_panels` 395, `menu_shown` 398, `menu_open` 414, `menu_mode` 461, `menu_generation` 463, `palette` 400, `lead_strip` 404 | `menu_open` is also read by the policy and by `window_facts`. |
| UI: OSD | `osd_until` 443, `osd_actions` 444, `osd_shown_at` 446, `osd_clear_at` 448, `osd_shown` 450, `osd_from` 452 | `osd_actions` names plugin commands. Stopping a plugin drops them (`drop_osd_actions`). |
| UI: video area | `no_video` 549, `video_absent` 551, `video_cleared` 553, `render_area` 562, `render_sent` 563, `render_continuous` 566, `sizing` 491, `shown_size` 494, `frame_log` 493, `window_title` 465, `on_top_applied` 505 | Derived from `last_source` and from layout. |
| MIXED | `mounts` 363 | The mount registry (spec, properties, recorded `ui.mount` call). The core needs it for standby recording, `rekey`, `revoke` and replay (`capabilities/ui_mount.rs:179-256`). The UI needs it to build panels. |
| MIXED | `mount_queue` 369, `mount_events` 374, `mount_input` 375 | The queue carries `Reply`s (core). `mount_input` is fed by Slint panel callbacks (UI). |
| MIXED | `input` 441 (`Rc<RefCell<InputShared>>`) | It holds the **binding table**: written by the core (`commands.register/unregister`, `bindings.*`; `capabilities/commands.rs:113`, `binding_manager.rs:88-130`) and read **synchronously** by Slint key callbacks (`host/input.rs:18-309`). |
| MIXED | `ui_state` 442 | One file (`ui/ui_state.rs`) with window frame and column widths (UI), bindings overlay, appearance, language and palette recents (core). Flushed synchronously from `window_tick` (`ui/ui_state.rs:502-522`). |
| MIXED | `window_facts` 412 | Written only by `window_tick` from window geometry (`host/window.rs:306-312`). Read by the core for conditions and triggers (`host/triggers.rs:37-47`), `window.state` (`host/state.rs:213-225`) and `ui.menu` (`capabilities/ui_mount.rs:735-800`). |
| MIXED | `policy` 410 (`window_state::Policy`) | Pure state machine (layout, auto-hide, active tabs). Written by UI events and by `ui.interaction`/`layout.set`; read when answering `layout.state`/`ui.menu`. |
| MIXED | `settings` 578 | The registry of pages (core: recorded registrations, `checking` replies) plus the open options window (`Open { instance: ComponentInstance, … Rc<Cell<u64>> }`, `ui/settings_window.rs:83-95`). |
| MIXED | `images` 547 | `ui_image::Images` holds decoded `slint::Image`s (`ui/ui_image.rs:71-75`) **and** the per-property row-patch `seq` used to refuse a stale patch in a reply. |
| MIXED | `drops` 574 | The registry of providers (core) plus a pending drop that needs the pointer and the shell geometry (`ui/drops.rs:305-340`). |
| MIXED | `origins` 358, `review` 407, `lead` 402, `dialog_busy` 355, `confirm_open` 361 | Where the user last acted (routes `dialogs.confirm`); the approval sheet holds a `Reply`; the lead holder is shown in the palette. `dialog_busy` feeds the window policy. |
| MIXED | `info_panels` 453, `on_top` 502, `cursor_playing` 503, `area_replies` 507, `last_frame` 489, `startup_panels` 432, `cover_art_off` 555 | Data written by plugins and applied by the UI (`osd.panel`, always-on-top, cursor auto-hide, `window.setVideoArea` replies answered from layout). `last_frame` is written by the frame path and read by `power_tick`. `startup_panels` gates file opens on mounts. `cover_art_off` goes to mpv. |

Counts: about 55 CORE fields, about 45 UI fields, about 30 MIXED fields. The boundaries are fuzzy, so these are approximate counts, not exact ones.

### 1.2 Major sub-objects

| Object | Where | Class | Why |
| --- | --- | --- | --- |
| `Supervisor<Value>` | `supervisor/` (10,572 lines) | CORE | Pure. The time is passed in. Its actions run in the adapter (`host/supervisor/actions.rs:14-73`). Some actions reach UI: `Terminate` → `stop_run`, `Notify` → OSD notices, `Replay` → replayed `ui.mount`/settings pages. |
| `OnDemand` adapter | `host/supervisor/mod.rs:153` | CORE | Holds, work, `ui_pending` (a run waiting for its initial UI replies). |
| `Backend` | `playback/backend.rs:87` | CORE | See §5. |
| `package_manager::Manager` | `packages/package_manager.rs:30` | CORE | Store, operations and secrets. Its "Slint" mentions are a validation check in a child process and tests. |
| `commands::Registry` | `commands.rs:393` | CORE, read by UI | Plain maps (inferred `Send`). |
| `crate::triggers::Triggers`, `conditions`, `source_events` | `triggers.rs`, `conditions.rs`, `source_events.rs` | CORE | No `Rc` or Slint in these files. They write files synchronously (`triggers.rs:747, 900`). |
| `timeline::Timeline` | `timeline.rs` | CORE | Broadcasts `timeline.changed` only (`timeline.rs:267-272`). The timeline plugin draws it. |
| `ui::panels::Panels` | `ui/panels.rs:600` | UI | `Rc<Panel>`, `Rc<Compiled>`, `Rc<RefCell<String>>`. |
| `settings_window::Settings` | `ui/settings_window.rs:91` | MIXED | See above. |
| `pip::PictureInPicture<PipWindow>` | `ui/pip.rs:76` | UI | Native PiP window. |
| `window_state::Policy` | `ui/window_state.rs` | UI logic | Pure, but its meaning is layout. |
| `ui_state::Store` | `ui/ui_state.rs` | MIXED | See above. |

### 1.3 Non-`Send` types and why

| Type | Why it is not `Send` |
| --- | --- |
| `slint_interpreter::ComponentInstance` (`shell`, `views`, `floating`, `settings.open`) | Slint ties it to the event-loop thread. |
| `Rc<Panel>`, `Rc<Compiled>`, `Rc<RefCell<String>>` (`panels`) | `Rc`. |
| `Rc<RefCell<InputShared>>` (`input`) | `Rc`, shared with the key callbacks. |
| `Rc<RefCell<slint::Weak<…>>>` (`video_target`), `Rc<Cell<bool>>` (`winit_draws`) | `Rc`, shared with the rendering notifier and winit hook. |
| `std::rc::Weak<dyn WindowAdapter>` (`placements`) | `Rc` weak. |
| `slint::Image` (`last_image`, `images`) | Treated as UI-thread-only here (inferred; it may hold renderer textures). |
| `thread_local!` sinks `FRAME_SINK`, `PIP_GPU_SINK`, `RESIZE_SINK` (`host/video.rs:7-13`), the stall log (`host/stall.rs:173`), macOS window cells (`platform/macos.rs:54-62`) | Per-thread by construction. |

The headless driver proves that the core runs without Slint: it calls only 8 of the tick steps, and `shell`/`views` stay empty. But `Host` is still not `Send` there. It runs on the thread that built it.

## 2. Inventory of work

Classes:

- **(a)**: touches CORE state only.
- **(b)**: touches UI state or objects (directly, or through shared paths: `show_osd`, `stop_run`, `update_title`, menu refresh), but the reply does not wait for a UI result.
- **(c)**: the reply depends on a UI result.

### 2.1 Events (`host/events.rs:25-133`, handled in `host/handle.rs:18-560`)

50 variants. `PhysicalInput` is behind the `producer-audio` feature.

| Origin | Class | Variants |
| --- | --- | --- |
| UI (Slint callbacks and winit hooks on the UI thread, posted to the same queue) | (b) (26) | `PhysicalInput`, `WindowInput`, `Binding`, `SidebarResized`, `AdjustResized`, `Tab`, `TabOverflow`, `SidebarDetach`, `AdjustDismiss`, `PipReturn`, `FloatingClosed`, `MenuAction`, `UiCallback`, `ViewClosed`, `MainClosed`, `TitleStrip`, `OsdAction`, `LeadStrip`, `ReviewSheet`, `InjectKey`, `ModifiersReleased`, `Dropped`, `Settings`, `BarButton`, `BarMore`, `CommandRunAnswered` |
| Threads and processes | (a) (10) | `PluginMessage`, `OutOfMemory`, `Recorded`, `Spawned` (inferred), `Position`, `SubtitleTrack`, `ConfirmDone`, `DialogDone`, `SourceLocated`, `Reply`. `Reply` → `respond` → can end an activation (`ui_finished`), which runs supervisor actions. |
| Threads and processes | (b) (11) | `Broadcast` (`source.state` writes `video-frame`/`no-video`/`window-title` on the shell: `host/state.rs:115-176`), `ActionDone` (OSD), `Exited`, `RuntimeFault`, `Reaped`, `BackendCleaned`, `Runner`, `RunnerExited` and `OwnerResources` (all can stop a plugin → `stop_run` revokes views, mounts, OSD buttons, floating window and shell properties: `host/lifecycle.rs:330-433`), `OpenFiles` (raises the window when `front`), `SourceLocate` (file dialog) |
| Threads and processes | (c) (1) | `SettingsChecked`: answers `settings.register` after a compile on the `settings-check` thread, and may rebuild the open options window. |
| Threads and processes | per method (2) | `Request`, `Prepared`: see §2.2. |

Two UI-originated events have a core effect worth naming. `Binding`, `OsdAction`, `BarButton` and `MenuAction::Invoke` execute commands (`execute_command`: a message to a plugin or a playback action). `UiCallback` and `ViewClosed` send to plugins.

### 2.2 RPC methods (`capabilities/mod.rs:207-243` + `capabilities/uncapable.rs`)

32 areas plus the uncapable list. There are **119 route entries**: 112 exact names and 7 `prefix.*` entries.

| Class | Count | Entries |
| --- | --- | --- |
| Forward to the playback worker (CORE; answered later by `Event::Reply` from the worker) | 48 | `producer.*` 6; `source.state`, `processing.state`, `playback.stats`, `player.chapters`, `player.audioDevices`; `playback.set`, `source.submit`, `source.operation`, `audio.addTrack`, `audio.selectTrack`, `player.setLoop`, `player.setAudioDevice`, `player.step`; `sourceResolvers.*` 4; `snapshot.state`, `jobs.state`, `jobs.operation`, `snapshot.*`, `jobs.*`; `media.sources.open`, `media.sources.*`; `subtitles.state`, `subtitles.*`; `processing.replace/operation`, `video.view.*`, `video.render.*`, `video.shaders.set` (7); `mpv.*` 5; `video.motion.*` 2, `video.upscaling.*` 2 |
| (a) core in the host | 36 | `player.setStepSeek`, `power.preventSleep`, `sequence.set`, `sequence.step`; `timeline.marks`, `timeline.set`, `timeline.setRanges`; `events.subscribe/unsubscribe`; `triggers.set/off/declare`; `commands.keys`, `commands.list`; `secrets.*` 3 and `system.reveal/openPath/openUrl` (run on a `system-request` thread, answered by `Event::Reply`, `system.rs:135-216`); `storage.session.*` 4; `packages.list`, `packages.*`; `rendering.state`, `rendering.setPreference`, `rendering.restart`; `ui.markdownScan` (pure parse, `ui/ui_markdown.rs:658`); `window.state` (from the `window_facts` mirror); `drops.provide/withdraw/listen`; `window.setCursorAutoHide`; `appearance.setLanguage` |
| (b) touches UI, reply not UI-dependent | 25 | `osd.show`, `osd.panel`; `appearance.set` (`theme_all` over every instance, `ui/theme.rs:205-254`); `commands.unregister` (refreshes an open menu inline, `capabilities/commands.rs:113-117`); `commands.register`, `commands.update`, `bindings.list/set/reset` (the binding table in `input`); `ui.interaction`, `layout.state`, `layout.set`, `ui.menu` (opens a menu at a point, `capabilities/ui_mount.rs:735-817`); `window.setAlwaysOnTop` (native, `host/state.rs:192-210`); `palette.lead.*` 3 (lead strip); `commands.run` (approval sheet, answered later); `dialogs.open`, `dialogs.confirm` (answered later); `host.run` (the `fullscreen` action and OSD); `app.system` (displays read "on the UI thread, where the windows live", `system.rs:187`); `clipboard.*` 3 ("called on the UI thread", `clipboard.rs:2`; not required by the OS calls (inferred)) |
| **(c) reply depends on a UI result** | 10 | listed below |

Caveats on (a): `sequence.set` refusal and the resolver-taken rewrite in `respond` call `refuse_taken` → `show_osd` (`host/dispatch.rs:55-70, 95-120`). `packages.*` (enable, disable, uninstall, reload) starts and stops plugins, so it reaches `stop_run`. `rendering.restart` quits the Slint loop "after this request's reply went out" via `invoke_from_event_loop` (`host/rendering.rs:371-376`).

**The (c) cases.** These are where a split adds a cross-thread round trip:

1. **`ui.open`**: compiles and instantiates the `.slint` file synchronously, and replies with the result or a compile error (`capabilities/ui_window.rs:44-97` → `ui/view.rs:703-716`, compile at `ui/view.rs:493-498`). It also gates activation: `activation_ui_begin` (`host/dispatch.rs:68-87`).
2. **`ui.set`**, 3. **`ui.show`**, 4. **`ui.close`**: run on the view's `ComponentInstance`. Missing view, conversion and property errors are reply errors (`capabilities/ui_window.rs:103-…`).
5. **`ui.mount`**, 6. **`ui.unmount`**: deferred. Queued, batched for 32 ms, then `apply_mounts` compiles (`panels.precompile`, `host/mounts.rs:332` → `ui/panels.rs:729-755`). After that come `ui.preparing{ready|failed}` and the reply (`host/mounts.rs:79-200`). This also gates activation, and the held-mounts gate (`host/handle.rs:88-105`) can delay it.
7. **`ui.setMount`**: writes into the panel's instances (`capabilities/ui_mount.rs:538-617`, `panel.write` at 609). A conversion or bridge error is the reply.
8. **`settings.*`**: `register` is answered after the `settings-check` thread compiled the page (`ui/settings_window.rs:549-600`). It compiles inline when that thread is missing (`:590`). `set` writes into the open options window (`:640-700`).
9. **`window.setVideoArea`**: answered when the layout shows the target, or after a second (`host/state.rs:232-300`, `answer_video_area` at `:302`, called from `window_tick`).
10. **`window.setPictureInPicture`**: creates the native PiP window and may leave fullscreen. It is answered from the PiP state machine (`host/video.rs:291-312`).

Three more couplings have the same shape:

- **Activation gating**: `respond` → `ui_finished` → `became_ready` (`host/dispatch.rs:118-144`, `host/supervisor/mod.rs:995-1030`). The supervisor's "ready", and with it the delivery of held wake triggers, waits for the UI result of the initial `ui.open`/`ui.mount`.
- **`area_replies`** and the PiP pending reply.
- **`commands.run` / `dialogs.*`**: already asynchronous by design (answered by an event).

### 2.3 Tick steps

Windowed: `main.rs:891-948`. Headless: `main.rs:704-711`.

| Step | Class | Notes |
| --- | --- | --- |
| `drain_events` (128 events, then up to 4 ms; `host/events.rs:197-226`) | per event | |
| crash-guard first draws | UI | |
| `placements.tick` | UI | |
| `mount_callbacks` | MIXED | Slint panel callbacks → `ui.callback` to plugins (`host/mounts.rs:767-795`) |
| `prepare_mount_tick` | (c) | compiles; replies to `ui.mount` |
| `panel_restart_tick` | UI | |
| `window_tick` | MIXED | PiP tick, menu tick, policy, `window_facts` + `window.state` broadcast (→ conditions, triggers), video-area replies, bar, `ui_state.save_if_due` (`host/window.rs:18-320`) |
| `platform::tool_windows_tick` | UI | macOS only (`platform/macos.rs:132`) |
| `activation_tick` | CORE | can stop a plugin |
| `on_demand_tick` | CORE | holds, probes, memory, **`triggers_tick` (time atom)**, start-up settle and mute (`host/supervisor/mod.rs:1769-1804`) |
| `power_tick` | CORE | thread-affine on Windows |
| `diagnostics_tick` | CORE + OSD | |
| `package_tick` | CORE | package operations, **shared runner heartbeat ping** (`packages/package_manager.rs:361-377`) |
| `notify_tick` | CORE | reads the binding table in `input` |
| `timeline_tick` | CORE | |
| `drops_tick` | MIXED | pointer and shell geometry |
| `open_files_tick` | CORE | waits on `startup_panels` |
| `settings_tick` | UI | |
| `theme_all` (when pending) | UI | |
| binding table disabled set | UI-side data | |
| menu refresh | UI | |
| `show_frame` (missed) | UI | |
| `frame_pacing::tick` | UI | |
| stall log, snapshot | UI | |

That is 8 of 21 steps CORE, 4 MIXED and 9 UI. The headless loop runs exactly the 8 CORE steps (activation, on_demand, power, diagnostics, package, notify, timeline, open_files).

### 2.4 Where the core blocks, or can block, on the UI thread's behalf

1. `ui.open` compile and create, synchronous: `capabilities/ui_window.rs:78` → `ui/view.rs:713`. This is the measured 300–800 ms.
2. Mount batches: `host/mounts.rs:149` `apply_mounts` → `:332` `panels.precompile` → `ui/panels.rs:743`. First-shown panels compile in `sync_panels` (`host/mounts.rs:641`, `ui/panels.rs:785, 821`), called from `window_tick`.
3. `settings.register` inline compile when the checker thread is missing (`ui/settings_window.rs:590`), and `settings_tick` → `settings_build` (`:1212`).
4. Conversion of property values to Slint values (`ui.set`, `ui.setMount`, `settings.set`). Large arrays are already prepared as plain rows on the plugin's reader thread (`process/mod.rs:42-47`, `ui/ui_values.rs:915-927`). The rest runs on the UI thread.
5. `theme_all` over every instance (`ui/theme.rs:205`). `refresh_menu` (`host/menus.rs:519`) from the tick (`main.rs:933`) and inline on `commands.unregister`.
6. Synchronous file writes on the UI thread: `ui_state` flush (`ui/ui_state.rs:508-522`), trigger files (`triggers.rs:900-901`), package store writes (inferred: these are small).
7. The reverse direction: core work holds the UI. A request flood held everything for 21 s once (`host/events.rs:200-207`). Frame delivery is skipped while the tick holds the `RefCell` (`main.rs:814-816`, `busy.miss()`).

The stalls outside our code (Slint/winit modal move/size loop, wgpu waiting on the GPU, device loss) stop the Slint timer and with it the whole tick. That is the problem this study is about.

## 3. Threads today

All of these hand results to the host through the one `events` queue (`mpsc::Sender<Event>`) unless stated otherwise. The host writes to plugins only through `Host::send` (`host/lifecycle.rs:447`), `broadcast` (`:486`) and `respond` (`host/dispatch.rs:95`), all on the UI thread. Those calls push onto a per-plugin bounded queue. **Writer threads** do the pipe writes.

| Thread | Where | Owns | Hands back |
| --- | --- | --- | --- |
| UI / main (Slint event loop) | `main.rs:862-988`; offscreen: a "main" thread with a 64 MiB stack (`main.rs:151-157`) | `Host` (in `Rc<RefCell<…>>`, `main.rs:798`) | — |
| Own process plugin: stdout reader (unnamed) | `process/mod.rs:579-591` | parsing, `admit` limits | `Event::Request`/`Prepared`/`RuntimeFault`/`Exited`; **writes limit refusals directly to the plugin's outbound queue** (`process/mod.rs:652-669`) |
| Own process plugin: stdin writer (unnamed) | `process/mod.rs:553-574` | the pipe | `RuntimeFault` on error |
| Own process plugin: stderr forwarder (unnamed) | `process/mod.rs:539-548` | plugin log | `OutOfMemory` |
| `shared-runner-read` | `process/shared_runner.rs:436-442` | parsing, members' limits, **hello and pong recorded in `LinkState`** (`:753-756`) | `Request(g)`, `Runner`, `RunnerExited`; refusals direct |
| `shared-runner-write` | `process/shared_runner.rs:432-434` | the runner's stdin | — |
| shared runner stderr (unnamed) | `process/shared_runner.rs:416` | log | — |
| `plugin-reaper` | `process/reaper.rs:79-81` | stopped plugins, kill deadlines | `Reaped` |
| `plugin-spawner` | `host/supervisor/worker.rs:53` | process creation (~30 ms each) | `Spawned`; the process waits on a shelf for the host |
| `plugin-recorder` | `host/supervisor/checkpoint.rs:163` | standby recordings | `Recorded` |
| playback control worker (unnamed) | `playback/backend.rs:131` | **the mpv handle**, sources, processing, subtitles, position model | `Reply`, `ActionDone`, `Broadcast` (`source.state` etc.), `Position`, `SubtitleTrack`, `OwnerResources`, `BackendCleaned`, `SourceLocate` (`backend.rs:350-1282`) |
| renderer (unnamed) | `playback/backend.rs:213` | mpv render context (software, or starts GPU) | frames into `FrameSlot`; waker → `invoke_from_event_loop` (`main.rs:845-853`) |
| `vestra-gpu-render` | `unsafe/render/src/windows/mod.rs:623`, `macos.rs:229` | GPU rendering into shared textures | redraw via `upgrade_in_event_loop` (`windows/mod.rs:1578`) |
| `vestra-gpu-watchdog` | `windows/mod.rs:636` | GPU stall detection | restart path (`host/restart.rs`) |
| `vestra-gpu-complete`, `vestra-ui-gpu` | `windows/mod.rs:760, 239` | timing stats | — |
| `vestra-vblank` | `playback/frame_pacing.rs:1064` | display vblank waits | `invoke_from_event_loop` (`frame_pacing.rs:327, 925`) |
| `settings-check` | `ui/settings_window.rs:121` | page compiles (definition only, discarded) | `SettingsChecked` |
| `system-request` (one per call) | `system.rs:210` | reveal, open, secrets, `app.system` | `Reply` |
| file dialogs (unnamed, one per dialog) | `capabilities/ui_dialog.rs` (`open`) | `rfd` async dialog | `DialogDone` |
| `confirm` (Windows) | `platform/windows.rs:789` | system confirmation | callback → `ConfirmDone` |
| `single-instance` | `platform/instance.rs:135` | later launches | `OpenFiles` |
| `media-sources`, `media-sources-save`, `media-job-N`, `media-progress`, `media-stderr`, `media-diagnostic-out/err`, `cover-art`, `producer-binary`, `native-subtitle-stdout/err`, `vestra-upscaler-check` | `playback/…` | owned by the playback side | to the control worker (inferred), which posts events |
| `vestra-shader-guard` | `host/diagnostics.rs:165` | guard file writes | — |
| `builtin-verify`, `secrets-purge`, package cleanup, `package-validate`/`-drop`/`-sweep`/`-validate-stderr` | `packages/…` | package work | events / shared state (inferred) |
| `vestra-exit-after-loss` | `host/restart.rs:204` | exit deadline after device loss | — |
| libmpv's own threads | — | decoding, demux, audio output | through the control worker |

Where replies come from:

- Every reply and event line is produced on the UI thread, inside the tick.
- The exceptions are the limit refusals, written by reader threads.
- The shared runner's `runner.ping` is queued from the tick (`shared_runner.rs:559-563`), and the reader thread counts its pong.
- Heartbeat during a stall: `Heartbeat::tick` adds at most one miss per tick (`shared_runner.rs:811-824`), and pongs are counted off the UI thread. So a stall of any length adds at most one miss. A runner is declared hung only after 5 ticks with unanswered pings. Stalls alone do not restart the runner (inferred from the code).
- Nothing wakes the UI loop when an event is queued. The 16 ms timer polls.

## 4. Where the cut could go

What every option must keep. These are ordering guarantees plugins see today, read from the code:

- **O1**: per plugin, all host lines come from one thread in handling order. The exceptions are limit refusals.
- **O2**: requests of all plugins are handled in arrival order (one queue).
- **O3**: a request's side effects are sent before its reply. Examples: `triggers.set` delivers `triggers.fired` then replies `{seq}` (`capabilities/triggers.rs:43-55`); `appearance.set` broadcasts `appearance.changed` then replies (`ui/theme.rs:245-254`); a refusal broadcasts `host.error` before the error reply (`host/dispatch.rs:55-70`).
- **O4**: triggers that flip are delivered before the state broadcast that flipped them (`host/lifecycle.rs:493-500`). `source.state` precedes its derived `source.opening/ready/ended` (`host/handle.rs:256-267`).
- **O5**: `ui.preparing{ready|failed}` precedes the `ui.mount` reply (`host/mounts.rs:189-199`). An activation completes after the reply (`host/dispatch.rs:141-144`).
- **O6**: after `ui.open`'s reply the view exists. After `ui.mount`'s reply `ui.setMount` finds the mount (otherwise "owner has no ready mount").
- **O7**: `rendering.restart` replies before the loop quits.
- **O8**: playback worker replies and broadcasts keep their relative order (same queue).
- **O9**: `window.state` is broadcast at most once per tick, after layout (`host/window.rs:306-312`).

### 4.1 (A) Router and keep-alive thread

**What moves.** No `Host` state moves. Two possible pieces:

- (i) Send heartbeat pings from a timer thread instead of the tick.
- (ii) A "fast path" for the 48 forward-to-worker entries: the reader threads send them straight to `backend.commands`, and the worker writes their replies straight to the plugin's outbound queue.

**What it finds.**

- (i) buys nothing. The heartbeat is already stall-tolerant (§3).
- There is no pure plugin-to-plugin forwarding in the protocol. `sequence.step`, command execution and `PluginMessage` all read host registries.
- (ii) is not "pure forwarding" either. Each forward touches host state:
  - `plugins.contains_key` and `supervisor.touch` (use accounting for standby, `host/handle.rs:35-40`)
  - the spawning hold (`handle.rs:21-27`)
  - text resolution and the built-in check (`dispatch.rs:148-196`)
  - the held-mounts gate for `sourceResolvers.register` (`handle.rs:88-105`)
  - `worker_requested` (standby classification, `capabilities/mod.rs:313-324`)
  - on the way back, `source_state_replies` patching (`handle.rs:223-229`)
  - activation gating, `plugin.answered`, and the resolver-taken rewrite with OSD and `host.error` (`dispatch.rs:95-145`)

**Messages.** Router → host: `Forwarded{owner,id,method}`, `Answered{owner,id}`. Host → router: a route table snapshot (live owners, built-in flags, gate state).

**Ordering risk.** O1 and O8 break: a forwarded reply can overtake a broadcast the host has queued for the same plugin (for example the `source.state` that reflects a `playback.set`). O3 breaks for `source.state` replies, which today are patched with `sequence` flags.

**Stalls.**

| Stays blocked | Keeps running |
| --- | --- |
| everything else: triggers, supervisor timers, `source.state` broadcasts, core-only RPCs, `ui.*` | worker reads and controls from plugins |

**Size.** About 6–10 files, about 10^3 lines.

**Risks.** Duplicated bookkeeping, a second writer per plugin, and standby accounting done twice. It ends up as a partial (B).

**Verification.** V2 plus a wire trace. The trace would show the reordering.

**Verdict.** Not worth prototyping.

### 4.2 (B) Core thread for supervisor, triggers, RPC routing and playback control, with the UI thread as a client

**What moves to a `CoreHost` (`Send`), running the headless loop on its own thread.**

- `plugins`, `shared`, `reaper`, the spawner and recorder handles, `supervisor`, `on_demand`
- `backend` (handle), `last_source`, `sequence`, `step_seek`, `actions_busy`
- `commands` (authoritative), `triggers`, `trigger_state`, `timeline`, `session_storage`, `packages`, `power`, `shader_guard`, `rendering`
- the notification bookkeeping, `warnings`, `failures`, `development`
- the mount **registry** (`mounts` as data, `mount_queue`, `held_mounts`)
- the settings **registry**, the drops **registry**, `origins`, `info_panels`, `on_top`/`cursor_playing` data
- the binding table (authoritative)

`events` stays the core's queue.

**What stays on a `UiHost` on the UI thread.**

- `shell`, `views`, `floating`, `panels`, `placements`, `images` (Slint images and patch seqs)
- `policy`, menus, palette, lead strip, review sheet, OSD timing
- the video area, PiP, the settings window, title strip, bar, columns, `ui_state` (window part)
- the mount batching window and the compile

The key callbacks read an `Arc` snapshot of the binding table that the core publishes. The headless driver becomes the core loop itself, with a null UI port.

**Messages core → UI (`UiCommand`).**

- `Osd{text,from,actions}`, `OsdHide`
- `SourceShown{no_video, empty, title}` (last value wins)
- `Theme`
- `Registry(Arc<snapshot>)` and `Bindings(Arc<Table>)` (last value wins)
- `MountChange` (queue)
- `ViewOp{reply, open|set|show|close}`
- `SetMount{reply, key, property, value|prepared, seq}`
- `PanelMenu{reply,…}`, `Layout{reply,…}`, `Interaction{reply,…}`
- `SettingsOp{reply,…}`
- `WindowOp{reply, VideoArea|Pip|OnTop|CursorAutoHide|Fullscreen|Raise|Quit}`
- `Dialog{reply,…}`, `Confirm{reply,…}`, `Review{…}`, `Lead{…}`
- `RevokeOwner{owner, park}`, `RekeyOwner{from,to}`, `InfoPanels`

The UI drains these on its timer and, for latency, through `invoke_from_event_loop`.

**Messages UI → core.**

- The 26 existing UI-originated `Event` variants, unchanged.
- `WindowFacts(Value)` (last value wins; the core then broadcasts `window.state` and evaluates conditions and triggers, as in `window_tick` today)
- `UiReply(Reply, Result)`
- `MountApplied{batch, result, elapsedMs}` (the core then sends `ui.preparing`, `respond` and `ui_finished`, in today's order)
- `FrameShown(at)` (or an `AtomicU64`) for `power_tick`
- `Displays(Value)` for `app.system`
- `Acted{owner, origin}`

**(c) cases that need redesign.** All 10 in §2.2, plus:

- activation gating (`ui_begin`/`ui_finish` across the boundary)
- `area_replies` and the PiP pending reply
- the held-mounts gate (core) versus the mount batching window (UI)
- `rendering.restart` (the core writes the reply, then sends `WindowOp::Quit`)
- `stop_run`, which splits into a registry part on the core and `RevokeOwner` on the UI
- `startup_panels` (open files wait for mounts), which needs `MountApplied`

**Ordering.**

- O1–O4 and O8 hold, because the core is the single writer and drains one queue.
- O5 and O6 hold if the (c) replies are sent only from `UiReply`/`MountApplied`, and if `UiCommand`s are FIFO per owner. A plugin that pipelines `ui.open` then `ui.set` still finds the view.
- What changes: a (c) reply no longer leaves in the same tick as its request. A core line handled later (a `source.state` broadcast, a reply from the worker) can be written to that plugin before it. Per-plugin order across a UI round trip is the one real question (Q1).
- O9 becomes "once per `WindowFacts` message".
- UI-originated events keep their order among themselves. Their interleaving with plugin requests remains arrival order at the core queue, as today.

**Stalls.**

| Stays blocked | Keeps running |
| --- | --- |
| everything visible: `ui.*` replies, OSD, menus, keys (winit does not deliver them anyway), video frames, `window.state`, dialogs, the settings window. A plugin awaiting one of these waits | plugin RPC routing; all 48 worker forwards and their replies; `source.state` and source events; triggers (position reports and the time atom); supervisor timers (standby, start timeouts, wakes, crash pauses); package operations; the shared runner heartbeat; `timeline.changed`; storage; `commands.*`/`bindings.*` registry writes; `sequence.*`; `host.run` playback actions |

**Size.**

- Files with `impl Host`: 37 (29,167 lines).
- References to UI fields: `shell` 86 in 24 files; `panels` 112/14; `settings` 105/13; `policy` 112/20; `input` 82/21; `mounts` 63/11; `ui_state` 61/10; `floating` 31/12; `views` 16/6.
- Estimate: 30–40 files, 5,000–10,000 lines changed, plus about 1,500 lines of message types and dispatch (inferred).
- The supervisor core (10,572 lines), `triggers`, `conditions` and the playback worker are untouched.

**Risks.**

- The per-plugin ordering change above.
- Registry snapshots for menus, palette, bar and tab keys (cloning cost per revision: measure it).
- Thread-affine OS calls: `power` is fine on a long-lived core thread; clipboard, `rfd` and confirm stay on the UI side.
- Faster replies: the core can block on its queue instead of polling every 16 ms, which changes timing in the favourable direction (Q2).
- The large mechanical split of `impl Host` methods that touch both sides, especially `stop_run`, `respond`, `broadcast`→`refresh_conditions` and `window_tick`.

**Verification.**

- (1) V2 (below) for registries, standby and recordings.
- (2) A new wire trace differential: a test switch that logs each host↔plugin line with a per-plugin sequence; diff per plugin with ids and times normalised, and allowed reorders listed beforehand. It is a host test switch, not a wire change.
- (3) A stall test on the offscreen platform: a test-only switch that sleeps in the UI tick for N seconds while a test plugin calls `source.state` every 50 ms, a short `VESTRA_IDLE_MS` plugin is due to park, and a `time` trigger is due. Assert RPC latency stays at p99 < 50 ms, that the park happens on time, and that the trigger fires within one core tick. The existing offscreen integration tests are the harness (`src/host/tests/lazy_activation.rs`, `kept_state.rs`, `lifecycle_baseline.rs`, `startup_compiles.rs`, `shared_runner.rs`, `palette_lead.rs`).
- (4) `ci:local`.

### 4.3 (C) Full split: all Host state on the core, the UI a thin mirror

**What moves.** Everything in (B), plus the policy, menus (model building, hover aim), palette, review sheet, lead strip, OSD timing, bar, tabs, the PiP state machine and the settings model. The core computes the desired UI state. The UI applies diffs and only compiles and instantiates components.

**Messages.** A UI-state diff protocol (per window, per panel, per menu level). Every UI input becomes a core event, including menu hover and palette typing.

**(c) cases.** The same 10 as in (B), plus menu and palette interactions that are synchronous today (G-15 submenu aim, palette cursor). These would each take a round trip.

**Ordering.** As in (B). Menu and palette interaction latency now includes the core.

**Stalls.** The same as (B). A UI stall cannot be worked around for visible things in any design.

**Size.** Most of `host/` (18,618 lines) and `ui/` (29,198 lines), about 2–3 × 10^4 lines.

**Risks.** A rewrite of the UI layer. The `no-unrequested-UI` and `use-Slint-mechanisms` rulings would be touched everywhere. Interactive latency regressions.

**Verification.** As in (B), plus GUI checks for menus and palette.

**Verdict.** It adds no resilience over (B).

### 4.4 What the runtime differential (V2) covers

`src/scripts/verify-runtime.mjs` (1,115 lines; tests in `src/tests/sdk/verify-runtime.test.ts`; described in `doc/process/verify-additions.md:37-80`, named V2 in `doc/process/events-and-triggers-plan.md:28`) runs two staged players offscreen (`VESTRA_TEST_OFFSCREEN=1`) in fresh profiles, through four scenarios:

- `idle`: first and second start
- `park`: `VESTRA_TEST_PARK=all`, then wake
- `file`: warm, then open a file and play for 20 s
- `installed`: store plugins as installed packages

It compares, per step and plugin:

- the `VESTRA_TEST_REGISTRY` snapshots: commands with state and conditions, key bindings, settings pages, drop providers, wake events, recorded marks
- the host's standby log lines
- the plugin's console output
- its standby recording
- the host's `shell: compile`, `open:` and `source: phase` lines

It does **not** see:

- requests and events on the wire, so **not reply or event order**
- timing
- clicks, menus, dialogs and property values
- windows, GPU and audio
- source resolvers

So V2 can prove that (B) keeps standby, registries and recordings identical. It cannot prove ordering or latency. Those need the wire trace and the stall test from §4.2.

## 5. Playback path

- **Commands to mpv.** Plugin line → reader thread → `Event::Request` → **tick** → `dispatch` → `forward` (`capabilities/mod.rs:313-324`) → `backend.commands.send` (an `mpsc` plus a condvar `Wake`; `playback/backend.rs:44-85`) → the control worker. The worker takes up to 32 commands per turn and polls every 2 ms (`backend.rs:37-40`) → mpv. Keys: Slint key callback → `Event::Binding` (`host/input.rs:285, 302`) → **tick** → `run_core_action_with` → `host.action` command (`host/actions.rs:263-321`).
- **Replies and state.** The worker sends `Event::Reply`, `ActionDone`, `Broadcast("source.state")` (about every 100 ms while playing, plus on changes), `Position` and `SubtitleTrack` (`backend.rs:350-1282`) into the host queue → **tick** → `respond` / `broadcast` → per-plugin outbound queue → writer thread.
- **Is mpv control off the UI thread?** Yes: "Rendering and all control calls stay off the UI thread" (`backend.rs:1`), and no mpv call exists on the UI side. But both hops into and out of the worker go through the tick.
- **During a UI stall:**
  - mpv keeps decoding and audio keeps playing (worker and libmpv threads; inferred).
  - Frames keep rendering into `FrameSlot`, but are shown only when the loop runs (`main.rs:845-853`).
  - Plugin playback requests wait in the queue undispatched.
  - Worker replies, `source.state` and position reports pile up in the unbounded queue and arrive in a burst afterwards, at 128 events or 4 ms per tick.
  - Triggers on position fire late.
  - So **playback control from plugins does not work while the UI thread is stalled; playback itself continues.** Under (B) both hops leave the UI thread, and plugin control and observation keep working.

## 6. What release A added, and where it sits

| Addition | Where | Class |
| --- | --- | --- |
| Trigger registry: conditions, kept files, delivery bookkeeping | `triggers.rs` (909 lines), `triggers_sim.rs`, `conditions.rs` (+542) | CORE, pure. Synchronous file writes (`triggers.rs:747, 900`) |
| Host side of triggers: facts, position reports, time atom, `position.watch`, delivery (send / held / wake via the supervisor), start-up mute | `host/triggers.rs` (280 lines) | CORE. One UI input: `window_facts` (`host/triggers.rs:37-47`). **The time atom runs from `on_demand_tick` on the 16 ms UI tick** (`host/supervisor/mod.rs:1779`, `host/triggers.rs:91-101`). The design says so: "later only while the UI thread is busy" (doc/architecture/events-and-triggers.md §4.4) |
| Position model | `playback/position_model.rs` (261 lines), in the control worker | CORE, already off the UI thread. Reports `Event::Position`, which goes through the tick |
| Discrete source events `source.opening/ready/ended` | `source_events.rs` (397 lines), derived in `host/handle.rs:256-267` | CORE |
| `triggers.set/off/declare` | `capabilities/triggers.rs` | CORE. O3: fired before reply |
| Wake events with per-source merge keys | `capabilities/events.rs:11-69`, `supervisor/trigger.rs` | CORE, pure |
| Static state through standby: timeline layers, always-on-top, cursor | `host/lifecycle.rs:8-13, 368-376`, `ui/window_control.rs`, `timeline.rs` | Data CORE (kept under the parked owner and rekeyed). The always-on-top and cursor **effects** are UI (`apply_on_top`, `host/state.rs:192`; `window_tick`). MIXED |
| Quit reason to running plugins | `host/supervisor/shutdown.rs` (+28) | CORE |

Release A adds no new UI coupling except reading `window_facts`. Under (B), all of it is core, and the time atom stops depending on the UI thread. The design text in §4.4 would then need a one-line update (a doc change, not a behaviour change).

## 7. Open questions for the user

1. **Per-plugin line order across a UI round trip (B).** Today a `ui.set` reply leaves before anything handled after it.
   - (i) Keep that exactly: the core holds that plugin's later lines until the UI reply returns. Order is unchanged, but during a UI stall that plugin's events are delayed too, and only that plugin's.
   - (ii) Let later core lines (a `source.state`, a worker reply) go ahead of the pending UI reply. Lines carry ids and the SDK matches replies by id, so it is likely invisible, but it is a change in observable order.

   Recommendation: (i) for the prototype, with the cost measured.
2. **Core loop wake-up.**
   - (i) Block on the queue and handle each event at once. Replies arrive up to 16 ms sooner and there are fewer idle wake-ups. This changes timing and could expose races in plugins.
   - (ii) Keep a fixed 10–16 ms poll. Timing stays as today.

   The scheduling rules are unchanged either way.
3. **Binding table and command registry ownership.**
   - (i) The core owns them and publishes immutable `Arc` snapshots per revision. Key presses and menus may read a table one revision old, for under a frame; there are no locks on the key path.
   - (ii) A shared `Mutex`/`RwLock` read by key callbacks. Always current, but the UI thread can wait on a lock the core holds.

   Recommendation: (i).
4. **Scope.** (B) is about 10^4 lines across most host files. The cheaper option is to fix only our own stalls: compile `ui.open` and mount panels on a worker thread, as `settings-check` already does, then instantiate on the UI thread. That does nothing for the modal loop, wgpu resize waits or device loss, which are the measured multi-second cases. This is an architecture-size decision for the user.

**Recommendation: prototype (B), in three stages, each merged on its own.**

1. **Stage 0.** Add the wire-trace test switch and the offscreen stall test to today's code, and record baselines (V2, wire trace, stall latencies).
2. **Stage 1.** Split `Host` into `CoreHost` and `UiHost` with the `UiCommand`/`UiReply` interface, **still on one thread**, with messages executed in place at today's points. This is a structural refactor that must show **no** difference in V2, the wire trace or `ci:local`, in line with the names-only migration rule. It also makes the `Send` boundary compile-checked: if `CoreHost` is not `Send`, the split is not done.
3. **Stage 2.** Run `CoreHost` on its own thread. The headless loop becomes the core loop. Re-run V2 and the wire trace with only the reorders agreed in Q1, and the stall test must now pass.

Why (B): the core already reaches the UI only through a small set of paths (`stop_run`, OSD, source display, the (c) replies, window facts). The supervisor core, triggers, conditions and the playback worker need no change, and the headless driver already shows the core running without Slint. (A) cannot deliver without becoming (B). (C) costs about three times more and makes nothing more resilient.

## 8. What was not verified

- Nothing was compiled. The `Send`-ness of `Backend`, `Supervisor`, `Registry`, `Manager`, `Plugin` and `slint::Image` is inferred from their fields.
- Whether the Slint timer is really held during the Win32 modal loop and wgpu waits comes from the user's measurements, not from this study.
- The SDK's own request timeouts were not read (they live in the SDK repository).
- File lines cited for `packages/` threads and the `media-*` threads' hand-off are inferred from their names and the module headers.
