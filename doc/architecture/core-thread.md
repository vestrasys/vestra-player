# The core thread (design)

**Status: decided, not implemented.** Written 2026-11-15 by the API maintainer from the user's rulings of that day. Work starts when release A (SDK 1.10.0 / player 0.10.0) is on main. The execution plan is [core-thread-plan.md](../process/core-thread-plan.md). The source inventory behind the numbers is the study [core-loop-study.md](../process/research/core-loop-study.md) (called "the study" below). This file is the authority for the design; where the study recommends something else, this file wins.

**Process note (user ruling, 2026-11-15).** The work follows the pattern of SDK 1.10.0: this design and the plan are written by Fable; the implementation is by Opus 5.5; static review (diffs and source read against this design, no builds, no logs) is by Fable at medium effort; builds, test runs, `ci:local` and the V1/V2 verification are run by Opus, so that review tokens are not spent on build and test noise.

No code was written or built for this document. Measured values are quoted with where they were measured. Everything the study marked "(inferred)" is still inferred here.

**In one paragraph.** The host's core (plugin request routing, the supervisor, triggers, playback control, packages) is a single-threaded state machine that never waits. Today it runs inside a callback on Slint's UI thread, so it stops whenever anything else holds that thread: Slint compiling a window, Windows' own loop while a window is dragged, the graphics library waiting for the GPU. The decision is to give the core its own thread and make the UI thread one of its clients. A stalled interface then costs a frozen window, not stalled plugins, late triggers or lost playback control. No SDK change, no wire change, and the scheduling rules stay as they are.

**How to read it.** What was observed: [§1](#1-background-what-was-observed). How the core is driven today: [§2](#2-how-the-core-is-driven-today). What other projects do: [§3](#3-precedents). What was set aside: [§4](#4-options-considered). The rules: [§5](#5-the-decision). What it buys: [§6](#6-what-a-ui-stall-costs-before-and-after). What can go wrong: [§7](#7-risks-and-open-points). How it is proven: [§8](#8-verification). Words: [§9](#9-glossary).

**How sources are written.** `P:` is the player repository, read on branch `evt-release-a` at `3100d94f` (release A before its last fixes); paths without a prefix directory are under `src/host/src/`. `L:` is local evidence on the development laptop, not in any repository.

## 1. Background: what was observed

### 1.1 The machine and the runs

The measurements are from the user's laptop on 2026-11-14 and 2026-11-15: internal panel driven by the AMD Radeon 780M, GPU rendering on the NVIDIA RTX 4070, so on the internal panel every presented frame crosses from one adapter to the other ("cross-adapter"). With an external screen driven by the NVIDIA adapter there is no crossing.

The user ran the portable builds of 0.7.6, 0.8.2 (the installed version), 0.8.4, 0.9.0 and the release A trial (0.10.0, `3100d94f`) by hand, each in an isolated profile with GPU rendering. All of them behave the same. **None of this is a regression of release A or of 0.9.0**; the installed 0.8.2 log shows the same lines for the days before.

| Evidence | Where |
| --- | --- |
| Host logs of each version, with the `ui stall:` lines | `L:C:\vestra\compare\profile-<version>\host.log`; the user's own 0.8.2 history in `L:C:\vestra\backup-profile-20261114-trial\Roaming-Vestra\logs\host.log` |
| Thread stacks taken automatically while the window did not answer (12 dumps, video playing) | `L:C:\vestra\compare\stacks-trial\` |
| The same with the video paused, ending in a lost device (6 dumps) | `L:C:\vestra\compare\stacks-trial-paused\` |
| The watcher that took them (asks the window every 0.2 s, dumps stacks after 1 s without an answer) | `L:C:\vestra\compare\watch.ps1`, started through `start.ps1` |
| Symbols for the trial build | `L:C:\vestra\compare\sym\` |

### 1.2 Three classes of stall

"Tick" is the host's 16 ms turn of work (§2). "UI thread" is the thread that runs Slint's event loop.

| Class | What holds the UI thread | Measured | Where the thread was |
| --- | --- | --- | --- |
| S1. Opening a tool window | Slint compiles the window's `.slint` file and creates it, synchronously, inside the request | 290–800 ms per open (Store 371–803 ms, About 257–453 ms, Workshop 430 ms); every open compiles again | `ui.open`: P:capabilities/ui_window.rs:78 → P:ui/view.rs:713 |
| S2. Dragging or resizing a window | Windows runs its own message loop inside `DefWindowProc` until the mouse button is released. Window messages still arrive, but winit's timers do not fire, so the tick does not run | "no tick" for 5–18 s in every version (for example 10,100 ms, 14,946 ms, 17,941 ms) | winit 0.30.13, the modal move/size loop |
| S3. Resizing on the cross-adapter path | The graphics library waits for the GPU on the UI thread | Three blocks of 5.6–5.9 s with identical stacks; with the video paused, repeated 1 s waits for 8.8 s, then `the GPU device was lost; restarting the player` | Slint `resize_event` → wgpu `Surface::configure` → `Device::maintain` (waits without a time limit for all submitted work) → `wgpu_hal::dx12::Device::wait`; and Slint draw → `Surface::get_current_texture` → `acquire_texture` (1 s timeout) |

Notes on S3:

- It appears only on the internal panel (cross-adapter). On the external screen the same code path returns quickly.
- It does not depend on video: it is worse with the video paused. The video render thread and the two fences shared with it are not the cause.
- wgpu has no 5 s constant. The likely release after about 5 s is Windows declaring the window hung after 5 s without message pumping ([IsHungAppWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-ishungappwindow)); that link in the chain is inferred, the stacks are not.
- Private memory rose from about 440 MB to about 1,750 MB across these stalls. No queue of ours is unbounded on that path (every per-frame post to the UI thread is coalesced); the likely cause is swap-chain buffers re-created on every size change and released only once the GPU queue drains (inferred).

### 1.3 What stops when the UI thread stalls

Everything the tick does, including work that has nothing to do with the interface:

| What stops | Seen as |
| --- | --- |
| Plugin requests (all 119 route entries) | `rpc player.audioDevices still waiting after 10 seconds`, `rpc ui.setMount still waiting after 10 seconds` in the plugin log during the 12–18 s gaps |
| Supervisor timers: standby after 30 s idle, start timeouts, wakes | Late by the length of the stall |
| Triggers: position reports and the `time` atom | Late; `on_demand_tick` runs the time atom from the tick (P:host/supervisor/mod.rs:1779) |
| Playback control from plugins and playback state to plugins | mpv itself is controlled off the UI thread (P:playback/backend.rs:1), but a plugin's command reaches the playback thread only through the tick, and `source.state` comes back only through the tick. Playback continues; plugins can neither control nor observe it |
| Video frames | The frame is handed to Slint and drawn on the UI thread (P:host/video.rs:433) |
| All windows | No repaint, no answer to clicks |

The core does not wait anywhere in this. It is held by the thread it lives on.

## 2. How the core is driven today

The host's state is one object, `Host` (P:host/mod.rs:338-598). It has two kinds of entry point: "handle one event" (`host.handle(event)`) and "do one periodic step" (the `*_tick` methods). Who calls them is outside the object, and there are two callers:

| Driver | Loop | Used for |
| --- | --- | --- |
| Windowed | Slint's event loop owns the thread. The host registers a repeating 16 ms `slint::Timer`; its callback drains the event queue (at least 128 events, then what a time budget allows) and runs 21 steps (P:main.rs:855-948) | Every real run |
| Headless | Our own loop: wait on the event queue for at most 10 ms, handle, run 8 steps (P:main.rs:676-730) | Offscreen tests |

```mermaid
flowchart TB
    subgraph T["Threads that only do input and output"]
        R["Plugin pipe readers"]
        W["Plugin pipe writers"]
        PB["Playback thread (owns mpv)"]
        O["Reaper, spawner, recorder, settings check, dialogs"]
    end
    Q[("One event queue (unbounded)")]
    R --> Q
    PB --> Q
    O --> Q
    subgraph UI["UI thread (Slint event loop)"]
        SL["Slint: window messages, layout, drawing"]
        TK["16 ms timer callback = the tick<br/>drain queue · handle events · 21 steps<br/>(routing, supervisor, triggers, packages, mounts, menus …)"]
        SL -- "calls when the timer is due" --> TK
    end
    Q -- "polled every 16 ms" --> TK
    TK -- "replies, events" --> W
    TK -- "commands" --> PB
```

Facts from the study (§0–§3 there, with file and line):

- `Host` has **130 fields** in 37 files: about 55 core, about 45 interface, about 30 used by both. Because some fields are Slint objects, the Rust compiler refuses to move the object to another thread (a type that may cross threads is called `Send`; one field that is not makes the whole object not `Send`, and the host forbids `unsafe`). The object has to be split before any part of it can run elsewhere.
- Everything reaches the host through **one queue** that the 16 ms timer polls. Nothing wakes the loop when an event arrives, so today a request waits 0–16 ms before it is handled even when nothing stalls.
- Every reply and event line to a plugin is produced on the UI thread. The only exceptions are request-limit refusals, written by the reader threads.
- The headless loop runs exactly the 8 core steps (activation, on-demand, power, diagnostics, packages, notifications, timeline, open files). It is the proof that the core runs without Slint's loop; the offscreen tests run it that way every day.

**The gap this document records.** Running the core on Slint's event loop was never a ruling. It was an early implementation choice: Slint objects can be touched only on Slint's thread, many plugin requests end in an interface change, so putting everything on that thread was the shortest path. It was never brought to the user as an architecture decision, and no architecture document states it as one. On 2026-11-15 the user called it a sword of Damocles: the core's "never waits" guarantee holds only for our own code, and any wait by Slint, the graphics library, a driver or the operating system on that thread voids it.

## 3. Precedents

Two research passes were made on 2026-11-15 (web and public source; the claims below are those the researcher could open and read).

### 3.1 Core logic on the UI loop, with one iron rule

| Project | What runs on the UI thread | The rule |
| --- | --- | --- |
| VS Code | Extensions run in a separate process, but the host-side RPC endpoint lives in the workbench, the renderer's UI thread ([extensionHostManager.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/extensions/common/extensionHostManager.ts), [extension host](https://code.visualstudio.com/api/advanced-topics/extension-host)). The closest analogue to today's Vestra | The peer is marked unresponsive after 3 s without an acknowledgement |
| Chromium | Service-worker lifecycle and timeout timers on the browser UI thread ([service_worker_version.cc](https://github.com/chromium/chromium/blob/main/content/browser/service_worker/service_worker_version.cc)); the extensions `ProcessManager` in the browser process | "Do not perform expensive computation or blocking IO on the main thread"; IPC arrives on a separate IO thread ([threading_and_tasks.md](https://chromium.googlesource.com/chromium/src/+/main/docs/threading_and_tasks.md)) |
| Zed | Foreground executor on the main thread, background executor elsewhere | "The main thread should never, ever block." ([blog](https://zed.dev/blog/zed-decoded-async-rust)) |
| Electron | The main process runs the UI thread | "Under no circumstances should you block this process and the UI thread" ([performance](https://www.electronjs.org/docs/latest/tutorial/performance)) |
| JetBrains IDEs | Writes only on the event dispatch thread, which "must be as fast as possible" ([threading model](https://plugins.jetbrains.com/docs/intellij/threading-model.html)) | A freeze watchdog runs off that thread and dumps stacks ([PerformanceWatcherImpl.kt](https://github.com/JetBrains/intellij-community/blob/master/platform/platform-impl/src/com/intellij/diagnostic/PerformanceWatcherImpl.kt)) |
| Kodi | The application thread is the GUI thread; other threads post messages to it ([ApplicationMessenger.cpp](https://github.com/xbmc/xbmc/blob/master/xbmc/messaging/ApplicationMessenger.cpp)) | A guard against waiting on itself |
| Celluloid | mpv events are processed on the GTK main loop ([celluloid-mpv.c](https://github.com/celluloid-player/celluloid/blob/master/src/celluloid-mpv.c)) | — |
| Figma | Plugins run on the main thread by design, for synchronous document access | The accepted cost: "When plugins freeze, it affects the perceived stability of Figma." ([blog](https://www.figma.com/blog/how-we-built-the-figma-plugin-system/)) |

So the design we have is common and defensible. Every project that keeps it enforces that nothing on that thread may wait. We cannot enforce that: the three stalls of §1.2 are in Slint, wgpu, the driver and Windows, not in our code.

### 3.2 Separated designs and what they cost

| Project | Structure | Cost they name |
| --- | --- | --- |
| mpv | The core is detached from any interface: "Not calling mpv_wait_event() will not stop playback". Synchronisation by two rules: property changes are coalesced ("only one event per changed property"), and commands are asynchronous with replies as events that "may be reordered" ([client.h](https://github.com/mpv-player/mpv/blob/master/include/mpv/client.h)) | A render thread waiting on a core API can "freeze the mpv core thread forever" (`render.h`) |
| mpv, 2014 | It hit our S2 exactly. Commit `97a82bcd` (2014-07-26), "win32: move window handling to a separate thread": "the message dispatcher won't return while the dragging is active, so mpv couldn't update the video at all." It moved the window message pump to its own thread and left the core and the renderer where they were; the dedicated GUI thread is still there (`video/out/w32_common.c`) | The author calls it "rough"; it needed a `WM_CLOSE` workaround |
| VLC | The Qt interface runs on its own thread, except on macOS where Cocoa forces the main thread ([qt.cpp](https://github.com/videolan/vlc/blob/master/modules/gui/qt/qt.cpp)) | A ready-barrier at start so the core does not start video before the window exists |
| IINA | A dedicated queue reads mpv events; processing hops to the main queue; queue overflow is a critical failure ([MPVController.swift](https://github.com/iina/iina/blob/develop/iina/MPVController.swift)) | — |
| Haruna / mpvqt | The mpv controller lives on a worker thread, driven by queued calls ([mpvabstractitem.cpp](https://github.com/KDE/mpvqt/blob/master/src/mpvabstractitem.cpp)) | — |

mpv's 2014 fix isolates the party that gets blocked (the window pump) instead of moving the core. We cannot do the same: Slint binds "receive window messages" and "draw the interface" to one thread. The equivalent for us is the reverse: leave Slint's thread to Slint and move what is not interface off it. Moving the core is this document; moving video is the independent video layer (§6.2). Both together give mpv's three threads: core, video, window.

### 3.3 Slint's own advice

"You should perform the minimum amount of work in the main thread and delegate the actual logic to another thread to avoid blocking animations", with `invoke_from_event_loop` to come back ([slint lib.rs, "Threading and Event-loop"](https://raw.githubusercontent.com/slint-ui/slint/master/api/rs/slint/lib.rs), [invoke_from_event_loop](https://docs.slint.dev/latest/docs/rust/slint/fn.invoke_from_event_loop)). By Slint's own documentation, today's structure is not the recommended use. No survey of larger open-source Slint applications was found.

### 3.4 Upstream state of the three stalls

| Stall | Upstream | State on 2026-11-15 |
| --- | --- | --- |
| S2 modal loop | winit [#3272](https://github.com/rust-windowing/winit/issues/3272) "`ControlFlow` is ignored while window is being resized on Windows", [#1484](https://github.com/rust-windowing/winit/issues/1484) (system menu / title bar freezes the event loop) | Both open. Slint [#5720](https://github.com/slint-ui/slint/issues/5720) open; [#5791](https://github.com/slint-ui/slint/issues/5791) closed as its duplicate, called "a bug in winit" |
| S3 configure wait | wgpu still calls `maintain(wait_indefinitely())` before configuring a surface | By design; no issue asks for its removal |
| S3 reconfigure on every resize event | Slint [#12730](https://github.com/slint-ui/slint/issues/12730) "Window resizing is laggy in release mode" (closed); [PR #12733](https://github.com/slint-ui/slint/pull/12733) "wgpu: defer swapchain reconfiguration to the next frame" | The PR was closed unmerged because it did not fix the Wayland case it was written for |
| S3 present mode | Slint [#8693](https://github.com/slint-ui/slint/issues/8693), where AutoVsync was hard-coded; a maintainer said "perhaps we can make it configurable" | Never done; no public setting |
| S3 `acquire_texture` 1 s waits | wgpu [PR #10511](https://github.com/gfx-rs/wgpu/pull/10511) "fix(dx12): fix 1 second hang for dropped `SurfaceTexture`s", merged 2026-11-14; [PR #10548](https://github.com/gfx-rs/wgpu/pull/10548) discards queued old-size frames after a resize | Merged, unreleased; reaches us only through a wgpu release and a Slint upgrade |

No report of the exact cross-adapter symptom was found in any project. A request to Slint is reasonable and was deferred by the user (2026-11-15: "the issue can wait, our problem is bigger").

## 4. Options considered

| Option | What it is | Why it was set aside |
| --- | --- | --- |
| A. Router thread | Only forwarding and keep-alive leave the UI thread | Little to gain. The shared runner's heartbeat already tolerates stalls. No request is a pure plugin-to-plugin forward: each of the 48 playback forwards touches host bookkeeping (standby accounting, activation tracking, `source.state` patching). A fast path would put a second writer on each plugin's pipe and break today's order. It ends up as a partial B (study §4.1) |
| **B. Core thread** | Supervisor, triggers, routing, playback control and packages on their own thread; the UI thread is a client | **Chosen** (§5). About 30–40 files and 5,000–10,000 changed lines, plus about 1,500 lines of messages (inferred) |
| C. Full split | All state on the core, the interface a thin mirror that applies diffs | 20,000–30,000 lines, a rewrite of the interface layer. Menu hover and palette typing would each take a round trip. No more resilient than B: a stalled interface cannot show anything in any design |
| Compile off-thread only | Compile `ui.open` and mount panels on a worker, as the settings check already does | Helps only S1. Does nothing for S2 and S3, the multi-second cases. Slint's compiled definitions hold `Rc` (single-thread reference counts), so only the check could move, not the result |
| Default adapter rule | At start, render on the adapter that drives the main window's screen, as mpv and Chromium effectively do; manual choice kept | A way around S3, not a fix: a user who selects the NVIDIA adapter on the internal panel still gets it, and the next cause of a wait on the UI thread is not covered. **Still planned as a separate item**, to keep the user's machine usable in the meantime |
| Keep-alive during the modal loop | A Win32 timer armed between `WM_ENTERSIZEMOVE` and `WM_EXITSIZEMOVE` posts the tick (branch `fix-modal-tick`, written, awaiting the user's manual check) | Removes S2 only. **Still planned as a separate item.** Under this design it stays useful for the interface side (content re-laid-out while dragging) |
| Per-plugin in-order queue | Keep today's order exactly: while a plugin has a UI-bound request unanswered, hold that plugin's later lines. Equivalent in effect to in-order delivery in selective-repeat ARQ or TCP: a resequencing buffer | Head-of-line blocking. One request stuck behind a frozen interface would hold every later request of that plugin, including those that never touch the interface. That brings back per plugin what the design removes globally. The study recommended it for the prototype (its §7 question 1); the user ruled against a queue |
| **Two independent streams** | As in QUIC: order inside a stream, none across streams, so one blocked stream does not hold the other | **Chosen** (rule R4). The order inside each stream comes for free from each thread handling its inbox in arrival order |

## 5. The decision

Rulings of the user, 2026-11-15, as numbered rules. "Core" is the new thread; "UI" is Slint's thread.

```mermaid
flowchart TB
    subgraph IO["Threads that only do input and output (unchanged)"]
        R["Plugin pipe readers"]
        W["Plugin pipe writers"]
        PB["Playback thread (owns mpv)"]
    end
    subgraph CORE["Core thread: own loop, never waits, no locks"]
        C["Routing · supervisor · triggers · playback control<br/>packages · registries (commands, bindings, mounts, settings pages)<br/>single writer of every plugin's lines"]
    end
    subgraph UI["UI thread: Slint event loop, a client of the core"]
        U["Windows · panels · menus · palette · OSD · video area<br/>compiles and draws; holds read-only snapshots"]
    end
    R -- "requests" --> C
    PB -- "replies, source.state, position" --> C
    C -- "commands" --> PB
    C -- "replies, events" --> W
    C -- "state snapshots (last value wins)<br/>UI-bound requests, with ids" --> U
    U -- "UI events (keys, clicks, callbacks)<br/>window facts (last value wins)<br/>results of UI-bound requests" --> C
```

| No. | Rule |
| --- | --- |
| R1 | **The core has its own thread and its own loop.** It owns the plugin processes and the shared runner, the supervisor, triggers and conditions, request routing, playback control (the handle to the playback thread, `last_source`, the sequence), packages, timeline, session storage, power, and the notification bookkeeping. It stays single-threaded, lock-free and never waits |
| R2 | **The UI thread is a client of the core.** It owns the Slint objects and the windows: shell, views, panels, menus, palette, OSD timing, the video area, picture-in-picture, the settings window. It compiles, instantiates and draws. It decides nothing about plugins |
| R3 | **Two kinds of message cross between them, and nothing else.** (1) State snapshots: an immutable copy of a whole table or value; the receiver replaces what it had; several in a row collapse to the last ("last value wins"); no locks. (2) Requests and replies with ids, for work one side asks of the other |
| R4 | **Two streams per plugin, no ordering queue.** Requests that need the interface are carried out on the UI thread in arrival order. Requests that do not are carried out on the core in arrival order. There is no order between the two streams: the reply to a later UI-independent request may arrive before the reply to an earlier UI-bound one |
| R5 | **What a reply means (user ruling 2026-11-15).** For the UI-bound methods whose reply carries an interface result (10 in the ruling; 15 method names in the table of §5.1), the request is answered only when the UI thread has reported that result (after `ui.open` the view exists; after `ui.mount` the mount is ready; `ui.preparing` still precedes the `ui.mount` reply). For every other request that touches the interface (25 "reply independent" methods in the ruling; 23 method names in §5.1), a reply means the host has accepted and recorded the change; the interface shows it as soon as it can. This is what plugins could rely on before as well: a reply never meant the user had already seen the change. The move of `dialogs.open`, `dialogs.confirm`, `commands.run` (with a plan that asks) and `media.sources.locate` to the UI-bound set follows this rule's own definition (the reply carries an interface result: the user's answer); decided by the coordinator on 2026-11-15 and reported to the user |
| R6 | **Events and broadcasts keep the core's order.** The core is the single writer of every plugin's lines, as the UI thread is today. Side effects of a request are still sent before its reply; triggers that flip are still delivered before the state broadcast that flipped them |
| R7 | **A UI-independent request never waits for the UI.** This is the purpose of the change and a hard rule for every later addition: a new method that does not need the interface must not be routed through it |
| R8 | **The binding table and the command registry belong to the core.** The UI holds a read-only snapshot, replaced whole on every revision. Key callbacks on the UI thread look up the snapshot synchronously (Slint's key callback must answer "handled or not" at once and cannot wait for a round trip). Table updates and key events go through one queue on the UI side, so a key is never judged by a table older than the one that preceded it in that queue |
| R9 | **Facts the UI owns flow to the core as snapshots.** Window size, fullscreen, focus, layout and the menu state (`window_facts`) are sent by the UI when they change; the core evaluates conditions and triggers and broadcasts `window.state` from them |
| R10 | **Cadence in two steps.** First the core loop keeps today's fixed cadence, so that timing is unchanged and the differential checks can show zero difference. Then, as a separate merge, it handles each event when it arrives (the loop sleeps on its queue; a UI-independent request is answered in well under a millisecond instead of up to 16 ms). In the other direction the core wakes the UI with `invoke_from_event_loop`, not through the 16 ms timer |
| R11 | **No SDK change, no wire change, no change to the scheduling rules.** When a plugin starts, parks, wakes and is stopped is decided by the same supervisor rules ([supervisor.md](supervisor.md)); only the thread that runs them changes |
| R12 | **The readiness coupling is kept.** A plugin's run is ready only after its first `ui.open` / `ui.mount` was answered (P:host/dispatch.rs:118-144, P:host/supervisor/mod.rs:995-1030). A run that starts during an interface stall therefore becomes ready when the stall ends |
| R13 | **Handoff: state first, then the reply, then the broadcast** (from the plugin scan S0.3, 2026-11-15). For a UI-bound request, the core handles the UI's report in this order: (1) apply the state the report carries to the core's copy: the window facts snapshot (with the picture-in-picture `active` flag that `window.state` reports, host/state.rs:219, 225-228), the mount record and the layout (panel policy); (2) send the reply; (3) broadcast what changed (`window.state`). A request the plugin sends after the reply therefore reads the new state. The wire order (reply, then `window.state`) is today's: `window_tick` answers the video area (host/window.rs:301) before it replaces `window_facts` and broadcasts `window.state` (:308-311), so the differential checks stay equal. Only the core's copy is updated earlier than today, which no line on the wire shows |

**Evidence for R13**, all in code that awaits correctly:

1. The SDK's `plugin/view/panel.ts:52-63` calls `layout.state` right after `await ui.mount`: the core's mount and layout record must be updated before the `ui.mount` reply goes out.
2. `plugin/window.ts:137-138` reads `window.state` right after `await window.setVideoArea`; the Window plugin's Mini Size compares it (`window/size.ts:31-34, 92-96`).
3. `plugin/window.ts:158-159` does the same after `window.setPictureInPicture` (`window/pip.ts:62`).

**The 1 s timeout of `window.setVideoArea`.** The reply is due when the layout shows the size or when its deadline (request time + 1000 ms, host/state.rs:298) passes (:304). On the timeout path R13 holds for the state the core has, not for the size asked: the core applies the last window facts snapshot it received, then sends the reply (with the size the host computed, as today), and `window.state` reads that snapshot, which may still show the old size. That is today's behaviour too.

**Test target (plan S2d on one thread, S3a on two):** for each of the three cases, the request followed at once by the read returns the post-change state, and the wire shows the reply before the `window.state` broadcast; for the timeout path, the reply and a read give the last reported state.

### 5.1 Which requests are in which stream

From the study's inventory of the 119 route entries (its §2.2, with every method named). The summary as the study had it; the method table below (plan S0.2) is the authority where they differ:

| Stream | Count | What |
| --- | --- | --- |
| UI-independent, forwarded to the playback thread | 48 | `source.*`, `playback.*`, `player.*`, `subtitles.*`, `processing.*`, `video.*`, `mpv.*`, `media.sources.*`, `snapshot.*`, `jobs.*`, `producer.*`, `sourceResolvers.*` |
| UI-independent, answered by the core | 36 | `triggers.*`, `events.*`, `timeline.*`, `sequence.*`, `storage.session.*`, `packages.*`, `secrets.*`, `system.*`, `rendering.*`, `commands.keys/list`, `drops.*`, `window.state`, `power.preventSleep`, and others |
| Touches the interface, but the reply does not depend on its result | 25 | `osd.*`, `appearance.set`, `commands.register/update/unregister`, `bindings.*`, `layout.*`, `ui.interaction`, `ui.menu`, `window.setAlwaysOnTop`, `palette.lead.*`, `commands.run`, `dialogs.*`, `host.run`, `app.system`, `clipboard.*` |
| **UI-bound: the reply depends on an interface result** | 10 | `ui.open`, `ui.set`, `ui.show`, `ui.close`, `ui.mount`, `ui.unmount`, `ui.setMount`, `settings.*`, `window.setVideoArea`, `window.setPictureInPicture` |

The reply-independent methods are answered by the core as soon as its own part is done (registry write, snapshot sent); the interface catches up when it can (R5, K13). Which of them stay there and which join the UI-bound stream is settled method by method in the table below.

#### The method table (plan S0.2)

Read on player 0.10.0 (tag `v0.10.0`, `c3f8ebea`). **Route count: 119**, the same as the study: 32 areas in `capabilities::ALL` (`capabilities/mod.rs:207-240`) with 107 names, plus 12 names of `capabilities/uncapable.rs:6-21`; 112 exact names and 7 `prefix.*` entries (`snapshot.*`, `jobs.*`, `media.sources.*`, `packages.*`, `sourceResolvers.*`, `subtitles.*`, `settings.*`). The router is `capabilities::lookup` (`mod.rs:300-308`), called from `Host::dispatch` (`host/dispatch.rs:149-196`) after the start-up gates of `host/handle.rs:68-105`. Files below are under `src/host/src/capabilities/` unless a directory is named. The table has one row per route entry, except the two prefix entries whose names differ in stream: `media.sources.*` (rows 65a–65e) and `settings.*` (rows 83a–83d) have one row per name. So it has **126 rows for the 119 entries**.

Columns. **Today**: F = forwarded to the playback worker, answered later by `Event::Reply`; C = core only; U = touches the interface, reply independent of it; R = the reply needs an interface result. **Stream** (rule R4): I = UI-independent, answered by the core (or the worker) without waiting for the UI; B = UI-bound. **UI work**: what the core sends to the UI under the design, as a `UiCommand` kind (names from study §4.2, regrouped in plan S2a), and when the reply leaves. "Snapshot" kinds are last-value-wins (R3). A dash: no UI work.

| # | Entry | Handler | Today | Stream | UI work (`UiCommand`) · reply |
| --- | --- | --- | --- | --- | --- |
| 1 | `producer.prepare` | producer_av.rs:31 | F | I | — · worker |
| 2 | `producer.state` | producer_av.rs:31 | F | I | — · worker |
| 3 | `producer.release` | producer_av.rs:31 | F | I | — · worker |
| 4 | `producer.ack` | producer_av.rs:31 | F | I | — · worker |
| 5 | `producer.declareInput` | producer_av.rs:31 | F | I | — · worker |
| 6 | `producer.ackInput` | producer_av.rs:31 | F | I | — · worker |
| 7 | `source.state` | playback_read.rs:37-40 | F | I | — · worker; sequence flags patched by the core (host/handle.rs:223-229) |
| 8 | `processing.state` | playback_read.rs:40 | F | I | — · worker |
| 9 | `playback.stats` | playback_read.rs:40 | F | I | — · worker |
| 10 | `player.chapters` | playback_read.rs:40 | F | I | — · worker |
| 11 | `player.audioDevices` | playback_read.rs:40 | F | I | — · worker |
| 12 | `timeline.marks` | playback_read.rs:33 | C | I | — · at once |
| 13 | `playback.set` | playback_control.rs:152 | F | I | — · worker |
| 14 | `source.submit` | playback_control.rs:152 | F | I | — · worker |
| 15 | `source.operation` | playback_control.rs:152 | F | I | — · worker |
| 16 | `audio.addTrack` | playback_control.rs:152 | F | I | — · worker |
| 17 | `audio.selectTrack` | playback_control.rs:152 | F | I | — · worker |
| 18 | `player.setLoop` | playback_control.rs:152 | F | I | — · worker |
| 19 | `player.setAudioDevice` | playback_control.rs:152 | F | I | — · worker |
| 20 | `player.step` | playback_control.rs:152 | F | I | — · worker |
| 21 | `player.setStepSeek` | playback_control.rs:55; host/actions.rs:206-219 | U | I | `Registry` snapshot when the step changes (the seek commands' titles are re-registered) · at once |
| 22 | `power.preventSleep` | playback_control.rs:72 | C | I | — · at once (`power` on the core, K10) |
| 23 | `sequence.set` | playback_control.rs:79 | C | I | `Osd` on a refusal (note 3) · at once; start-up gate (note 6) |
| 24 | `sequence.step` | playback_control.rs:109 | C | I | — · at once |
| 25 | `sourceResolvers.register` | source_resolve.rs:74 | F | I | `Osd` on a taken refusal (note 3) · worker; start-up gate (note 6) |
| 26 | `sourceResolvers.unregister` | source_resolve.rs:74 | F | I | — · worker; start-up gate (note 6) |
| 27 | `sourceResolvers.respond` | source_resolve.rs:69 | F | I | — · worker |
| 28 | `sourceResolvers.*` | source_resolve.rs:77 | C | I | — · at once (`unsupported`; note 2) |
| 29 | `events.subscribe` | events.rs:168 | C | I | — · at once |
| 30 | `events.unsubscribe` | events.rs:168 | C | I | — · at once |
| 31 | `commands.register` | commands.rs:120, 126 | U | I | `Registry`, `Bindings` snapshots · at once |
| 32 | `commands.update` | commands.rs:120, 126 | U | I | `Registry` snapshot · at once |
| 33 | `commands.unregister` | commands.rs:108 | U | I | `Registry`, `Bindings` snapshots (open menu refreshed from them) · at once |
| 34 | `host.run` | commands.rs:90; host/actions.rs:263 | U | I | `WindowOp::Fullscreen` · at once; playback actions: worker, `Osd` from the result (note 4) |
| 35 | `commands.keys` | commands.rs:107 | C | I | — · at once |
| 36 | `ui.open` | ui_window.rs:37, 44 | R | B | `ViewOp::Open` · after the UI reports the view created or the compile error; readiness (note 5) |
| 37 | `ui.set` | ui_window.rs:38, 103 | R | B | `ViewOp::Set` · after the UI wrote it |
| 38 | `ui.show` | ui_window.rs:38, 103 | R | B | `ViewOp::Show` · after the UI showed it |
| 39 | `ui.close` | ui_window.rs:38, 103 | R | B | `ViewOp::Close` · after the UI closed it |
| 40 | `ui.mount` | ui_mount.rs:160, 281 | R | B | `MountChange` · after `MountApplied`, `ui.preparing` first; readiness and start-up gate (notes 5, 6); R13 |
| 41 | `ui.setMount` | ui_mount.rs:164, 538 | R | B | `SetMount` · after the UI wrote it |
| 42 | `ui.unmount` | ui_mount.rs:160, 281 | R | B | `MountChange` · after `MountApplied`; start-up gate (note 6) |
| 43 | `ui.interaction` | ui_mount.rs:165, 701 | U | I | `Layout` snapshot (panel locks) · at once |
| 44 | `ui.menu` | ui_mount.rs:166, 735 | U | I | `PanelMenu` · at once; refusals judged on the last window facts; today's property error kept (note 7) |
| 45 | `layout.state` | ui_mount.rs:167; host/state.rs:322 | C | I | — · at once, from the window facts snapshot (note 1) |
| 46 | `layout.set` | ui_mount.rs:168, 819 | U | I | `Layout` snapshot · at once |
| 47 | `dialogs.open` | ui_dialog.rs:51, 167 | R | B | `Dialog` · on the user's choice (`DialogDone`); refusals at once (note 8) |
| 48 | `dialogs.confirm` | ui_dialog.rs:50, 60 | R | B | `Confirm` or `Review` · on the user's answer (`ConfirmDone`); refusals at once (note 8) |
| 49 | `packages.*` | plugins_manage.rs:25-31 | C | I | `RevokeOwner` when a plugin stops (K3) · at once, or from the operation / validation thread |
| 50 | `processing.replace` | processing_video.rs:45 | F | I | — · worker |
| 51 | `processing.operation` | processing_video.rs:49 | F | I | — · worker |
| 52 | `video.view.set` | processing_video.rs:72 | F | I | — · worker |
| 53 | `video.view.release` | processing_video.rs:72 | F | I | — · worker |
| 54 | `video.render.set` | processing_video.rs:59 | F | I | — · worker |
| 55 | `video.render.release` | processing_video.rs:68 | F | I | — · worker |
| 56 | `video.shaders.set` | processing_video.rs:50 | F | I | — · worker |
| 57 | `subtitles.state` | subtitles_read.rs:24 | F | I | — · worker |
| 58 | `subtitles.*` | subtitles_control.rs:26 | F | I | — · worker |
| 59 | `snapshot.state` | media_read.rs:24 | F | I | — · worker |
| 60 | `jobs.state` | media_read.rs:24 | F | I | — · worker |
| 61 | `jobs.operation` | media_read.rs:24 | F | I | — · worker |
| 62 | `snapshot.*` | media_control.rs:35-37 | F | I | — · worker |
| 63 | `jobs.*` | media_control.rs:35-37 | F | I | — · worker |
| 64 | `media.sources.open` | media_sources.rs:40-44 | F | I | — · worker |
| 65a | `media.sources.*`: `remember` | media_sources.rs:40-44 | F | I | — · worker |
| 65b | `media.sources.*`: `get` | media_sources.rs:40-44 | F | I | — · worker |
| 65c | `media.sources.*`: `locate` | media_sources.rs:40-44; playback/media_sources.rs:1176-1183 | R | B | `Dialog` · worker, after the user's choice (note 9) |
| 65d | `media.sources.*`: `forget` | media_sources.rs:40-44 | F | I | — · worker |
| 65e | `media.sources.*`: `list` | media_sources.rs:40-44 | F | I | — · worker |
| 66 | `osd.show` | ui_osd.rs:47, 54 | U | I | `Osd` · at once |
| 67 | `osd.panel` | ui_osd.rs:48 | U | I | `InfoPanels` snapshot · at once |
| 68 | `clipboard.readText` | clipboard.rs:27 | U | I | — · from a worker thread (note 10) |
| 69 | `clipboard.writeText` | clipboard.rs:27 | U | I | — · from a worker thread (note 10) |
| 70 | `clipboard.writeImage` | clipboard.rs:27 | U | I | — · from a worker thread (note 10) |
| 71 | `window.setVideoArea` | window_control.rs:50; host/state.rs:232-309 | R | B | `WindowOp::VideoArea` · when the window facts show the size, or after 1 s; R13 |
| 72 | `window.setAlwaysOnTop` | window_control.rs:54; host/state.rs:192 | U | I | `WindowOp::OnTop` · at once (errors are only logged today) |
| 73 | `window.setCursorAutoHide` | window_control.rs:66; host/window.rs:284-289 | U | I | cursor mode snapshot (the UI hides the pointer from it) · at once |
| 74 | `window.setPictureInPicture` | window_control.rs:65; host/video.rs:291 | R | B | `WindowOp::Pip` · from the PiP state machine; R13 |
| 75 | `bindings.list` | bindings_manage.rs:25; binding_manager.rs:94 | C | I | — · at once (note 1) |
| 76 | `bindings.set` | bindings_manage.rs:25; binding_manager.rs:95 | U | I | `Bindings` snapshot (open menu refreshed from it) · at once |
| 77 | `bindings.reset` | bindings_manage.rs:25; binding_manager.rs:113 | U | I | `Bindings` snapshot · at once |
| 78 | `timeline.set` | timeline_marks.rs:37 | C | I | — · at once |
| 79 | `timeline.setRanges` | timeline_marks.rs:37 | C | I | — · at once |
| 80 | `drops.provide` | ui_drop.rs:60 | C | I | — · at once |
| 81 | `drops.withdraw` | ui_drop.rs:60 | C | I | — · at once |
| 82 | `drops.listen` | ui_drop.rs:60 | C | I | — · at once |
| 83a | `settings.*`: `register` | ui_settings.rs:57; ui/settings_window.rs:557-599 | U | I | `SettingsOp` after the answer (open window updated) · after the `settings-check` thread (note 11) |
| 83b | `settings.*`: `unregister` | ui/settings_window.rs:600-629 | U | I | `SettingsOp` (open window rebuilt) · at once |
| 83c | `settings.*`: `set` | ui/settings_window.rs:630-705 | R | B | `SettingsOp` · after the UI wrote it into the open window (note 11) |
| 83d | `settings.*`: `show` | ui/settings_window.rs:706-721 | R | B | `SettingsOp` · after the UI opened the window |
| 84 | `appearance.set` | ui_theme.rs:25; ui/theme.rs:228 | U | I | `Theme` · at once |
| 85 | `appearance.setLanguage` | ui_theme.rs:26; ui/theme.rs:266 | C | I | — · at once |
| 86 | `secrets.get` | secrets.rs:24; system.rs:155 | C | I | — · from the `system-request` thread |
| 87 | `secrets.set` | secrets.rs:24; system.rs:155 | C | I | — · from the `system-request` thread |
| 88 | `secrets.delete` | secrets.rs:24; system.rs:155 | C | I | — · from the `system-request` thread |
| 89 | `rendering.setPreference` | rendering_manage.rs:26; host/rendering.rs:316 | C | I | — · at once |
| 90 | `rendering.restart` | rendering_manage.rs:26; host/rendering.rs:326 | C | I | `WindowOp::Quit` after the reply · at once |
| 91 | `video.upscaling.state` | video_upscaling.rs:34 | F | I | — · worker |
| 92 | `video.upscaling.set` | video_upscaling.rs:34 | F | I | — · worker |
| 93 | `video.motion.state` | video_motion.rs:33 | F | I | — · worker |
| 94 | `video.motion.set` | video_motion.rs:33 | F | I | — · worker |
| 95 | `commands.list` | commands_run.rs:911 | C | I | — · at once |
| 96 | `commands.run` | commands_run.rs:924, 701 | R | B | `Review` when the plan asks · `{ran:true}` at once when nothing asks, else on the user's answer (note 8) |
| 97 | `palette.lead.set` | palette_lead.rs:409 | U | I | `Lead` (palette row, strip) · at once |
| 98 | `palette.lead.clear` | palette_lead.rs:413 | U | I | `Lead` · at once |
| 99 | `palette.lead.done` | palette_lead.rs:416 | C | I | — · at once (note 1) |
| 100 | `mpv.command` | mpv.rs:74 | F | I | — · worker |
| 101 | `mpv.get` | mpv.rs:74 | F | I | — · worker |
| 102 | `mpv.set` | mpv.rs:74 | F | I | — · worker |
| 103 | `mpv.observe` | mpv.rs:74 | F | I | — · worker |
| 104 | `mpv.unobserve` | mpv.rs:74 | F | I | — · worker |
| 105 | `triggers.set` | triggers.rs:40 | C | I | — · at once |
| 106 | `triggers.off` | triggers.rs:57 | C | I | — · at once |
| 107 | `triggers.declare` | triggers.rs:67 | C | I | — · at once |
| 108 | `packages.list` | uncapable.rs:25 | C | I | — · at once |
| 109 | `window.state` | uncapable.rs:26; host/state.rs:213 | C | I | — · at once, from the window facts snapshot (R9) |
| 110 | `system.reveal` | uncapable.rs:37; system.rs:143 | C | I | — · from the `system-request` thread |
| 111 | `system.openPath` | uncapable.rs:37; system.rs:147 | C | I | — · from the `system-request` thread |
| 112 | `system.openUrl` | uncapable.rs:37; system.rs:151 | C | I | — · from the `system-request` thread |
| 113 | `app.system` | uncapable.rs:37; system.rs:186-203 | U | I | — · from the `system-request` thread, displays from a `Displays` report (note 10) |
| 114 | `ui.markdownScan` | uncapable.rs:27; ui/ui_markdown.rs:658 | C | I | — · at once |
| 115 | `storage.session.get` | uncapable.rs:29 | C | I | — · at once |
| 116 | `storage.session.set` | uncapable.rs:29 | C | I | — · at once |
| 117 | `storage.session.delete` | uncapable.rs:29 | C | I | — · at once |
| 118 | `storage.session.keys` | uncapable.rs:29 | C | I | — · at once |
| 119 | `rendering.state` | uncapable.rs:28 | C | I | — · at once |

**Totals.**

| Today | Count | Stream |
| --- | --- | --- |
| F, forwarded to the playback worker | 50 | I |
| C, core only | 38 | I |
| U, touches the interface, reply independent | 23 | I (answered by the core at once, R5 / K13) |
| R, reply needs an interface result | 15 | B |
| **All rows** | **126** | **111 I, 15 B** |

By route entry (119): 105 entries are wholly I, 12 wholly B (`ui.open`, `ui.set`, `ui.show`, `ui.close`, `ui.mount`, `ui.setMount`, `ui.unmount`, `window.setVideoArea`, `window.setPictureInPicture`, `dialogs.open`, `dialogs.confirm`, `commands.run`), and 2 are split by name (`media.sources.*`: 4 names I, `locate` B; `settings.*`: `register` and `unregister` I, `set` and `show` B). Check: 105 + 12 + 2 = 119 entries; 105 + 12 + 5 + 4 = 126 rows.

The study had 48 / 36 / 25 / 10 over 119 entries. The differences, each from the source:

| Entry | Study | Here | Evidence |
| --- | --- | --- | --- |
| `sourceResolvers.*` | F | C | The prefix entry reaches only names other than `register`/`unregister`/`respond`, and those are refused `unsupported` at once (source_resolve.rs:77); nothing is forwarded |
| `layout.state`, `bindings.list`, `palette.lead.done` | U | C | Pure reads or bookkeeping: `layout_state` reads `window_facts`, the panel policy and mounts (host/state.rs:322-360); `bindings_list` reads the binding table (binding_manager.rs:142); `done` drops a ticket (commands_run.rs:202). None touches a Slint object |
| `dialogs.open`, `dialogs.confirm`, `commands.run` | U | R | The reply carries the user's answer, collected by an interface element (note 8) |
| `media.sources.locate` | F | R | The reply waits for a file dialog's choice (note 9); the same criterion as `dialogs.open` |
| `settings.*` | R | split | `register` and `unregister` do not wait for the UI; `set` and `show` do (note 11) |
| `player.setStepSeek`, `window.setCursorAutoHide` | C | U | The first re-registers the host's seek commands, whose titles the menus show (host/actions.rs:206-219); the UI hides the pointer from the second (host/window.rs:284-289). Both answer at once |

**Notes.**

1. *Core-only reads of shared tables.* `layout.state`, `bindings.list` and `window.state` read tables the design gives to the core (R8) or mirrors the UI feeds (R9). They answer from the core's copy at once, as today they answer from the mirror.
2. *`sourceResolvers.*`.* The prefix entry exists in the area's list but its handler accepts only the three exact names, which are routed before the prefix. It answers every other name with a refusal on the core.
3. *OSD side effects of core refusals.* `sequence.set` refused as taken, and a `resolver-taken` reply rewritten in `respond` (the worker's refusal of `sourceResolvers.register`, playback/source_resolvers.rs:203), call `refuse_taken` → `show_osd` (host/dispatch.rs:55-71, 97-115). The reply does not wait for it: the core sends `Osd` and answers.
4. *`host.run`.* `fullscreen` is a window command: today `window_command` toggles the shell window and answers `true` if one exists (host/actions.rs:269-277, 531-543); under the design the core answers from "a shell exists" and sends `WindowOp::Fullscreen`. `previous`/`next` send `sequence.step` to the provider and answer at once (:286-298). The other 12 actions go to the worker as `host.action` and are answered from `Event::ActionDone`, which may also show an OSD (:308-342).
5. *Readiness coupling (R12).* For `ui.open` and `ui.mount` of a run whose activation is `Pending`, `handle.rs:68-87` calls `activation_ui_begin`/`activation_ui_wait` and `ui_begun`; `respond` takes it back (`activation_ui_take`, `activation_ui_finish`, host/dispatch.rs:118-131) and, after the reply line, calls `ui_finished` (:142-144) → `became_ready` (host/supervisor/mod.rs:1002-1030). **Mapping:** begin, wait, take, finish and `ui_finished` stay on the core with the supervisor; the UI's part is only the result (`UiReport` of the view, `MountApplied`). The core answers from that report in today's order: the reply, then `ui_finished`. During a UI stall the run stays `Pending` and becomes ready when the report arrives, as R12 says. Nothing of the supervisor moves to the UI.
6. *Start-up gate (held mounts).* On a windowed run, `ui.mount`, `ui.unmount`, `sequence.set`, `sourceResolvers.register` and `sourceResolvers.unregister` of a plugin that is not built-in are held while any built-in is still starting (`builtins_starting`: packages not started, a built-in's first run queued, or a built-in's activation `Pending`), and after that behind any earlier held request of the same plugin (host/handle.rs:88-105, ui/slot_claims.rs:73-75, packages/package_manager.rs:1308-1327). They are released in order by `release_held_mounts` (host/dispatch.rs:18-28). A built-in's `Pending` ends only when its first `ui.open`/`ui.mount` was answered (note 5), so during start-up **three UI-independent methods can wait, indirectly, for a built-in's interface result**. This is a scheduling rule (built-ins claim the sequence, resolvers and slots first), not a route through the interface: the gate stays on the core and reads core state. R7 is not broken, and R11 keeps the rule as it is. It is listed so the stall test does not count it as a violation at start-up.
7. *`ui.menu`.* Its refusals read `window_facts["slots"]`, the floating window's owners and the active tab (ui_mount.rs:784-806). Under the design they read the core's last window facts snapshot (R9), which can be a frame older than the UI. The UI drops a `PanelMenu` whose generation is gone, as `window_for(generation)` does today. Today's error path is kept: a failure to set `menu-origin-x`/`-y` on the window is the reply's error (ui_mount.rs:806-813). These are fixed properties of the host's own shell component, so the core decides it from the shell's compiled definition (a fact the UI reports once per shell generation, with the window facts) and answers at once with the same error; a failure of the write itself on the UI is logged.
8. *Replies that carry the user's answer (moved to B).* `dialogs.open` replies with the paths the user chose (`DialogDone`, host/handle.rs:512); `dialogs.confirm` with `{confirmed, checked}` (`ConfirmDone`, :470-475, or the sheet's answer); `commands.run`, when its plan asks, with the answer on the approval sheet (`CommandRunAnswered`, :466-469; commands_run.rs:747-772). These answers are interface results, so by R5's own definition the three are UI-bound. **No behaviour changes:** they are already answered by a later event today, and a later request of the same plugin is already answered before them. The core keeps the admission: the headless refusal, `dialog_busy`, `sheet_waiting`, the ticket and rate checks are core state and are refused at once; only the dialog or sheet and its answer are the UI's. `commands.run` with nothing to ask answers `{ran:true}` at once from the core.
9. *`media.sources.locate`.* One of the six names behind `media.sources.*` (media_sources.rs:21-28). The worker answers it by asking for the file dialog (`Event::SourceLocate`, playback/media_sources.rs:1176-1183); the dialog's choice goes back to the worker (`owner.sourceLocated`, host/handle.rs:476-510), which then replies. Under the design the core receives `SourceLocate`, sends `Dialog`, and forwards the choice to the worker. The reply carries the user's choice, as `dialogs.open`'s does, so `locate` is UI-bound (row 65c; coordinator decision 2026-11-15). The other four names behind the prefix, and `media.sources.open`, stay I.
10. *Replies made on the UI thread today, but not interface results.* `app.system` reads the displays from the live winit window, "on the UI thread, where the windows live" (system.rs:187-192; platform/windows.rs:1250-1270; on macOS `NSScreen` is main-thread only, platform/macos.rs:763-769). The `Displays` report of study §4.2 is a UI fact like those of R9: the UI sends it when it changes, and the core answers from its copy. `clipboard.*` runs `arboard` (Windows) or `NSPasteboard` (macOS) where it is called; the "called on the UI thread" of clipboard.rs:2 describes where it runs today, not a requirement of the Windows call (study: inferred). The reply carries the system call's result, not an interface result, so by R5/K13 the entries stay I on every platform (coordinator decision 2026-11-15): the calls run on **one serial clipboard thread**, not a thread per request as `system.*` does (system.rs:209-213), so that an unawaited write followed by a read stays ordered as today, where the call is synchronous (clipboard.rs:34). K10 is amended to match. Whether `NSPasteboard` may be used off the main thread on macOS is checked in plan S3a, with the fallback named there.
11. *`settings.*`.* One route entry with four names, split by name (rows 83a–83d). `settings.register` is answered after the `settings-check` thread compiled the page (ui/settings_window.rs:575-598), not after the UI (K7); a page registered again with the same files is answered at once, and its changes are written into an open options window (`settings_update`, :568-573), which under the design is a `SettingsOp` the core sends after its answer. `settings.unregister` changes the page registry and marks an open window for rebuilding (:600-629). Both are I. `settings.set` writes into the open options window, and conversion and reconcile errors there are the reply (:630-705); `settings.show` replies with the window's open error (:706-721). Both are B.

**Check (R7).** No I entry is routed through the UI: every F entry goes from the core to the worker, every C and U entry is answered by the core (or the worker or a `system-request`-style thread) without waiting for a `UiReport`. The UI work of a U entry is a command or snapshot sent with no reply handle. The only wait of I rows on an interface result is the start-up gate (note 6), a scheduling rule kept by R11.

### 5.2 What the two streams change for a plugin

- A plugin that awaits each call before the next sees nothing: the second call is sent after the first reply.
- A plugin that sends several calls without waiting gets each reply on its own promise, matched by id, as today. Only the arrival order between a UI-bound and a UI-independent call can differ.
- Today's behaviour that goes away: "a request sent after a UI-bound request, without awaiting it, is handled after the interface change is in place". It holds today only because both are handled in the same tick. Whether any existing plugin depends on it is checked before the core moves (§7, K1).

## 6. What a UI stall costs, before and after

### 6.1 The table

| While the UI thread is held | Today | With the core thread | With the independent video layer as well |
| --- | --- | --- | --- |
| Windows repaint, clicks answered | No | No | No |
| Plugin requests that do not need the interface | Stop; time out after 10 s in the SDK's log | Continue | Continue |
| Playback control from plugins; `source.state` to plugins | Stop | Continue | Continue |
| Triggers, the `time` atom, wakes, standby timers | Late | On time | On time |
| Package operations, shared runner heartbeat | Stop (the heartbeat tolerates it) | Continue | Continue |
| Requests that need the interface (`ui.*`, settings, video area) | Wait | Wait | Wait |
| Video picture | Frozen | Frozen | Continues |
| After the stall | The backlog is worked through at 128 events or 4 ms per tick | The interface jumps to the latest state | Same |

The design does not make the interface stall less. It limits what a stall takes with it. Making the interface stall less is a separate, open-ended list (keep-alive, default adapter, upstream fixes, compile cache); each item removes one known cause, and new causes can always appear. This design is what makes a new cause cost a frozen window instead of a broken session.

### 6.2 The independent video layer

The same idea applied to video: video gets its own swap chain and render thread and is composed under the interface (DirectComposition on Windows), so a held UI thread no longer freezes the picture. It is a separate project with its own unknowns (whether the Slint interface can be a transparent layer; platform equivalents on macOS and Linux) and its own synchronisation concern: on a resize, or when a panel pushes the video area, the video layer and the interface layer are resized by different threads and can differ for a frame or two. It is not part of this design and not needed for it.

## 7. Risks and open points

| No. | Risk or open point | How it is handled |
| --- | --- | --- |
| K1 | A plugin sends a UI-bound request without awaiting it and then depends on it having taken effect (for example a read of state that the interface change produces) | A read-only scan of all built-in plugins and the plugin repository before the core moves (plan S0). Each finding is fixed in the plugin, or the pair is brought to the user  **Scan result (S0.3, 2026-11-15):** no K1 case in the 14 built-in plugins, the 8 store plugins and the 11 SDK examples; the SDK awaits every UI-bound reply. The scan's three read-after-await cases became rule R13 (handoff)  **User ruling (2026-11-15):** findings are fixed in the plugins; no ordering exception is kept for them. Third-party authors are told in the SDK docs (order across the two kinds is not guaranteed; await the first reply when order matters). TestHost is changed to model the two streams only after the core thread work is finished and stable, as a later SDK release; nothing in the SDK changes during S0–S3 |
| K2 | The 15 UI-bound method names (§5.1) need a round trip; their replies no longer leave in the same turn as the request | They get an explicit request/reply seam while still on one thread (plan S2d), so the move itself changes only the thread |
| K3 | `stop_run` (stopping a plugin) reaches the interface: views, mounts, OSD buttons, the floating window, shell properties (P:host/lifecycle.rs:282-445) | Splits into a registry part on the core and a "revoke this owner" message to the UI. During a stall the interface part is late; the process is stopped on time |
| K4 | The `time` trigger atom runs from the 16 ms UI tick | It moves with `on_demand_tick` to the core. [events-and-triggers.md §4.4](events-and-triggers.md) says "later only while the UI thread is busy"; that sentence is updated when S3 merges |
| K5 | `window_facts` is written by the interface and read by conditions, triggers and `window.state` | Rule R9. `window.state` is then broadcast once per snapshot received instead of once per tick |
| K6 | `mounts` is needed by the core (standby recording, rekey, revoke, replay) and by the interface (building panels) | The registry is core data; the interface gets mount changes as messages and reports `MountApplied`. The 32 ms batching window and the compile stay on the UI side |
| K7 | `settings`: the registry of pages is core, the open options window is interface; `settings.register` is answered after a compile check | The check thread already exists; its result goes to the core; the open window is updated by message |
| K8 | `input`, `ui_state`, `images`, `policy`, `drops` are used by both sides | Split one at a time (plan S2c). `ui_state` is one file with a window part and a core part; one writer keeps writing it, fed by both |
| K9 | Device loss and the restart path (P:host/restart.rs): the UI thread may be stuck in teardown while the core is alive | The restart decision and the plugin shutdown deadline move with the core; the existing "exit now" backstop stays. Re-checked in plan S3a against [supervisor.md §9](supervisor.md#9-quit-and-device-loss)  **User ruling (2026-11-15):** this work keeps the whole-player restart. Once the core thread is stable, the coordinator verifies whether the interface alone can be rebuilt after a device loss (windows closed, new device, windows reopened, core, plugins and playback untouched). If the verification shows the work is small and contained, it goes ahead without asking; if it would need another round of core changes, it is dropped and reported |
| K10 | Thread-affine system calls | `power` (`SetThreadExecutionState` is per calling thread) moves to the core thread and stays there. File dialogs and the confirmation dialog stay on the UI side. The clipboard leaves it (coordinator decision 2026-11-15, §5.1 note 10): `clipboard.*` runs on one serial clipboard thread on every platform, so its replies never wait for the UI and an unawaited write followed by a read keeps its order. Plan S3a checks on macOS that `NSPasteboard` works off the main thread; if it does not, the fallback is that `clipboard.*` is UI-bound on macOS only |
| K11 | Snapshot cost | The command registry and binding table are cloned per revision (hundreds of entries, changed at start, on plugin load and on user edits). Measured in plan S2; if it shows in a profile, the snapshot becomes shared and immutable (`Arc`) instead of copied |
| K12 | macOS and Linux | The design is platform-neutral. On macOS the UI thread must be the process's main thread (winit's rule), which this design already satisfies: the core is the thread that is created. The stall classes differ per platform; the stall test (§8) is platform-neutral because it stalls the UI thread artificially |
| K13 | The 23 "touches the interface, reply independent" method names (§5.1) answer before the interface has caught up | Already true today across processes for the plugin's own view of time; stated in the method table of §5.1 when S0 fixes it  **User ruling (2026-11-15):** the core answers these at once (R5); they never wait for the interface |
| K14 | Faster replies after R10's second step could expose a timing assumption in a plugin | It is a separate merge with its own measurement, and can be reverted alone |

## 8. Verification

| Check | What it proves | Notes |
| --- | --- | --- |
| V1 static gates | No test or golden trace was changed to pass | `src/scripts/verify-gates.mjs`, as in the events plan |
| V2 runtime differential | Registries, standby classification and parking, recordings, console output are identical | `src/scripts/verify-runtime.mjs`, four offscreen scenarios. It does **not** see request or event order, or timing (study §4.4) |
| Wire trace (new) | Order and content of every line between the host and each plugin | A test-only host switch that logs each line with a per-plugin sequence; a diff tool compares two runs per plugin with ids and times normalised. From S3 on the tool knows rule R4: a reorder between the two streams of one plugin is allowed and reported; any other difference fails. Not a wire change |
| UI stall test (new) | Rules R1 and R7, and §6.1's middle column | Offscreen. A test-only switch makes the UI turn sleep for N seconds while a test plugin calls `source.state` every 50 ms, a plugin with a short idle time is due to park, and a `time` trigger is due. Asserts: request latency p99 under 50 ms during the stall, the park on time, the trigger within one core turn. **Expected to fail on today's code**; recorded as the baseline, marked "expected to change" |
| Names-only rule | Stage S2 (split on one thread) changes structure and nothing else | Any difference in V2, the wire trace or `ci:local` fails the merge; no test or test host is edited to pass |
| `Send` check | The core part really is free of interface objects | When the core struct is not `Send`, the compiler rejects moving it to a thread. A compile-time assertion is added in S2 so the boundary is checked before S3 |
| `ci:local` | Everything else | At the end of each stage |
| Real windows | That the visible behaviour is right on a real desktop | Done by the user by hand, with `watch.ps1` recording stalls and stacks. Scripted mouse input cannot reproduce fast human dragging and is not used |

## 9. Glossary

| Word | Meaning here |
| --- | --- |
| Core | The host's state machine for everything that is not interface: routing, supervisor, triggers, playback control, packages, registries |
| UI thread | The thread that runs Slint's event loop and owns the windows |
| Tick | Today: one run of the 16 ms timer callback. After the change: one turn of the core loop |
| Stall | A period in which the UI thread runs none of our code because something else holds it |
| UI-bound request | A request whose reply depends on a result from the interface (the 15 B rows of §5.1) |
| UI-independent request | Any other request |
| Stream | The sequence of one plugin's requests of one kind; order is kept inside a stream only |
| Snapshot | An immutable copy of a table or value, sent whole; the receiver replaces what it had |
| Last value wins | Of several snapshots waiting, only the newest is applied |
| Cross-adapter | Rendering on one GPU and showing on a screen driven by another |
| Head-of-line blocking | One stuck item at the front of an ordered queue holding back everything behind it |
| `Send` | Rust's marker for a type that may move to another thread; checked by the compiler |
